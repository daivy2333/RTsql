//! Integration tests for MS20 T1 (`sql-attach-session` R1–R3): ATTACH session
//! registration, the rejection matrix, and lib-path session-only rejection.
//!
//! Scope note: the routing surface (`cross-db-namespace-query` R1/R2 — reading
//! `alias.table`) belongs to Iteration 001, so this file witnesses the
//! *registration* surface only: an alias registered by `ATTACH` is observable
//! through the session (the rest of the one-shot call still runs) and across
//! the process boundary (the attached file's lock is held for the session and
//! released at exit).
//!
//! Scenario mapping (requirement → scenario → test):
//!   R1/S1 register + same-call visibility  attach_registers_database_and_next_statement_runs_in_same_session
//!   R1/S2 bare-name path resolution        attach_resolves_bare_name_under_rtsql_home
//!   R1/S3 missing path (no file created)  attach_missing_path_rejected_without_creating_file
//!   R1/S4 lock conflict (exit 4)          attach_locked_database_rejected_with_exit_4
//!   R2/S1 duplicate alias                 attach_duplicate_alias_rejected
//!   R2/S2 attach the main db itself       attach_main_database_itself_rejected_with_exit_4
//!   R2   invalid key (exit 5)             attach_invalid_key_rejected_with_exit_5
//!   D2   active transaction is orthogonal attach_allowed_inside_active_transaction
//!   D1   lock released at process exit    attached_database_lock_released_at_process_exit
//!   R3/S1 lib path rejected               execute_sql_rejects_attach_without_touching_plan_cache
//!         (and inside a transaction)      execute_in_tx_rejects_attach_and_transaction_stays_usable

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::transaction::IsolationLevel;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

// ---------------------------------------------------------------------------
// CLI e2e section — real binary, one-shot calls.
// ---------------------------------------------------------------------------

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 运行 rtsql 二进制并等待退出；60s 未退出则 kill 并 panic
/// （夹具模式与 tests/tx_statement_test.rs 一致：独立 TempDir CWD + RTSQL_HOME，
/// stdout/stderr 均为管道 → 默认 json 格式）。
fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    run_cli_env(dir, args, &[])
}

/// T4（D8 校准）：锁冲突断言子进程用例的显式短预算注入
/// （`RTSQL_BUSY_TIMEOUT=0` ＝ 仅 ~8ms 下限窗口；断言本身零变化）。
fn run_cli_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> CliOutput {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtsql"));
    cmd.args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_BUSY_TIMEOUT");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait().expect("poll rtsql status") {
            Some(status) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                child
                    .stdout
                    .take()
                    .unwrap()
                    .read_to_string(&mut stdout)
                    .unwrap();
                child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                return CliOutput {
                    code: status.code(),
                    stdout,
                    stderr,
                };
            }
            None => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rtsql did not exit within 60s (one-shot CLI must terminate)");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// 每个测试独立的仓库现场：TempDir + 预建 `db/` 集中存储目录
fn cli_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    dir
}

/// 以裸名执行一条 auto-commit 语句，非零退出即 panic（建库/种子用）。
fn run_cli_ok(dir: &Path, db: &str, sql: &str, what: &str) {
    let out = run_cli(dir, &[db, sql]);
    assert_eq!(out.code, Some(0), "{what} failed: {}", out.stderr);
}

/// 建两个库：主库 `app`（表 t）与附加库候选 `other`（表 u）。
/// 二者都在 `$RTSQL_HOME/db/` 下，CWD 同时指向 fixture 根（裸名解析命中的是
/// RTSQL_HOME 路径，不是 CWD 下的同名文件）。
fn two_db_fixture() -> TempDir {
    let dir = cli_fixture();
    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建两个库（断言语义不变）。
    for name in ["app", "other"] {
        let out = run_cli(dir.path(), &["new", name]);
        assert_eq!(out.code, Some(0), "new {name} failed: {}", out.stderr);
    }
    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE t (id INT); INSERT INTO t VALUES (7)",
        "seed main db",
    );
    run_cli_ok(
        dir.path(),
        "other",
        "CREATE TABLE u (id INT); INSERT INTO u VALUES (9)",
        "seed attach candidate db",
    );
    dir
}

fn assert_failure(out: &CliOutput, code: i32, needle: &str, what: &str) {
    assert_eq!(out.code, Some(code), "{what}: stderr was {}", out.stderr);
    assert!(
        out.stderr.contains(needle),
        "{what}: expected stderr containing {needle:?}, got {:?}",
        out.stderr
    );
}

/// R1/S1: `ATTACH 'other' AS o` 在会话内注册成功，渲染 Affected(0)，同次调用
/// 后续语句继续执行（fail-fast 语义下后句能跑完即证明未中断）。
#[test]
fn attach_registers_database_and_next_statement_runs_in_same_session() {
    let dir = two_db_fixture();

    let out = run_cli(
        dir.path(),
        &["app", "ATTACH 'other' AS o; SELECT id FROM t"],
    );

    assert_eq!(out.code, Some(0), "stderr was {}", out.stderr);
    assert!(
        out.stdout.contains(r#"{"affected_rows":0}"#),
        "ATTACH must render Affected(0), stdout was {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains(r#""rows":[[7]]"#),
        "the second statement must still run in the same call, stdout was {:?}",
        out.stdout
    );
}

/// R1/S2: 裸名 `other` 按 `resolve_db_path` 解析到 `$RTSQL_HOME/db/other.db`。
/// 成功本身即见证解析面——CWD 下没有 `other.db`，非 RTSQL_HOME 路径必失败。
#[test]
fn attach_resolves_bare_name_under_rtsql_home() {
    let dir = two_db_fixture();
    assert!(
        !dir.path().join("other.db").exists(),
        "precondition: no same-named file under CWD"
    );

    let out = run_cli(dir.path(), &["app", "ATTACH 'other' AS o"]);

    assert_eq!(out.code, Some(0), "stderr was {}", out.stderr);
}

/// R1/S3: 路径不存在时点名拒绝（exit 3）且不静默创建 0 字节新库。
#[test]
fn attach_missing_path_rejected_without_creating_file() {
    let dir = two_db_fixture();

    let out = run_cli(dir.path(), &["app", "ATTACH 'missing' AS o"]);

    assert_failure(&out, 3, "does not exist", "ATTACH of a missing path");
    assert!(
        !dir.path().join("db").join("missing.db").exists(),
        "a rejected ATTACH must not create the database file"
    );
}

/// R1/S4: 目标库被另一持有者打开时，flock 冲突以 `DatabaseLocked` 同型拒绝
/// （exit 4）。本进程经 lib 打开 `other.db` 持锁，CLI 进程再 attach。
#[test]
fn attach_locked_database_rejected_with_exit_4() {
    let dir = two_db_fixture();
    let held = std::thread::spawn({
        let path = dir.path().join("db").join("other.db");
        move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let db = Database::open(&path)
                    .await
                    .expect("first open holds the lock");
                // 持锁直到本测试结束：CLI 子进程必须拿到 exit 4。
                std::thread::sleep(Duration::from_secs(3));
                drop(db);
            });
        }
    });
    // 等待子线程真正完成 open（避免竞态：CLI 先于持锁者启动）。
    std::thread::sleep(Duration::from_millis(500));

    let out = run_cli_env(
        dir.path(),
        &["app", "ATTACH 'other' AS o"],
        &[("RTSQL_BUSY_TIMEOUT", "0")],
    );

    assert_failure(&out, 4, "database is locked", "ATTACH of a locked database");
    held.join().unwrap();
}

/// R2/S1: 重复别名点名拒绝（exit 3），注册表零变化。
#[test]
fn attach_duplicate_alias_rejected() {
    let dir = two_db_fixture();
    // T10 机械校准：先经 new 通道创建第三个库（断言语义不变）。
    let new_out = run_cli(dir.path(), &["new", "second"]);
    assert_eq!(
        new_out.code,
        Some(0),
        "new second failed: {}",
        new_out.stderr
    );
    run_cli_ok(
        dir.path(),
        "second",
        "CREATE TABLE v (id INT)",
        "seed second attach candidate db",
    );

    let out = run_cli(
        dir.path(),
        &["app", "ATTACH 'other' AS o; ATTACH 'second' AS o"],
    );

    assert_failure(&out, 3, "already", "duplicate alias");
}

/// R2/S2: attach 主库自身由 flock 按 inode 拒绝（exit 4），不引入独立路径比较。
#[test]
fn attach_main_database_itself_rejected_with_exit_4() {
    let dir = two_db_fixture();

    let out = run_cli_env(
        dir.path(),
        &["app", "ATTACH 'app' AS m"],
        &[("RTSQL_BUSY_TIMEOUT", "0")],
    );

    assert_failure(&out, 4, "database is locked", "self-attach");
}

/// D2: 打开错误分类——加密附加库在无密钥会话中 attach 以 exit 5 拒绝
/// （主库为明文且不带密钥，故失败只可能来自附加库打开面）。
#[tokio::test]
async fn attach_invalid_key_rejected_with_exit_5() {
    let dir = two_db_fixture();
    {
        let path = dir.path().join("db").join("sec.db");
        let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some("k1"))
            .await
            .unwrap();
        db.execute_sql("CREATE TABLE u (id INT)").await;
        db.close().await.unwrap();
    }

    let out = run_cli(dir.path(), &["app", "ATTACH 'sec' AS s"]);

    assert_failure(&out, 5, "invalid key", "ATTACH of an encrypted database");
}

/// D2/假设 11: attach 与事务正交——活跃事务内 ATTACH 允许并可提交。
#[test]
fn attach_allowed_inside_active_transaction() {
    let dir = two_db_fixture();

    let out = run_cli(dir.path(), &["app", "BEGIN; ATTACH 'other' AS o; COMMIT"]);

    assert_eq!(out.code, Some(0), "stderr was {}", out.stderr);
}

/// D1: 附加库锁从 ATTACH 持有到进程退出；退出后可重新打开。
#[tokio::test]
async fn attached_database_lock_released_at_process_exit() {
    let dir = two_db_fixture();

    let out = run_cli(dir.path(), &["app", "ATTACH 'other' AS o"]);
    assert_eq!(out.code, Some(0), "stderr was {}", out.stderr);

    let reopened = Database::open(&dir.path().join("db").join("other.db")).await;
    assert!(
        reopened.is_ok(),
        "attached database lock must be released when the session process exits: {:?}",
        reopened.as_ref().err()
    );
}

// ---------------------------------------------------------------------------
// lib section (R3) — non-session paths.
// ---------------------------------------------------------------------------

fn expect_error_contains(resp: Response, needle: &str, what: &str) {
    match resp {
        Response::Error { message, .. } => assert!(
            message.contains(needle),
            "{what}: expected message containing {needle:?}, got {message:?}"
        ),
        other => panic!("{what}: expected Response::Error, got {:?}", other),
    }
}

/// R3/S1: `execute_sql("ATTACH ...")` 计划期拒绝并点名 session-only，
/// 且不触碰 plan cache。
#[tokio::test]
async fn execute_sql_rejects_attach_without_touching_plan_cache() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("lib.db")).await.unwrap();
    assert!(db
        .execute_sql("CREATE TABLE t (id INT)")
        .await
        .is_ok_or_success());
    assert!(db.execute_sql("SELECT id FROM t").await.is_ok_or_success());
    let len_before = db.plan_cache_len();
    assert!(len_before > 0, "SELECT should populate the plan cache");

    let resp = db.execute_sql("ATTACH 'other.db' AS o").await;
    expect_error_contains(
        resp,
        "only supported in an rtsql CLI session",
        "execute_sql(ATTACH)",
    );
    assert_eq!(
        db.plan_cache_len(),
        len_before,
        "a rejected ATTACH must not change the plan cache"
    );
}

/// R3 续: 事务内拒绝不终止事务——拒绝后同事务 DML 仍可用。
#[tokio::test]
async fn execute_in_tx_rejects_attach_and_transaction_stays_usable() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("lib.db")).await.unwrap();
    assert!(db
        .execute_sql("CREATE TABLE t (id INT)")
        .await
        .is_ok_or_success());
    let tx = db.begin().await.unwrap();

    let resp = db.execute_in_tx("ATTACH 'other.db' AS o", &tx).await;
    expect_error_contains(
        resp,
        "only supported in an rtsql CLI session",
        "execute_in_tx(ATTACH)",
    );

    assert!(
        db.execute_in_tx("INSERT INTO t VALUES (1)", &tx)
            .await
            .is_ok_or_success(),
        "in-tx insert after rejection"
    );
    db.commit(tx).await.unwrap();

    match db.execute_sql("SELECT id FROM t").await {
        Response::QueryResult { rows } => assert_eq!(rows.len(), 1, "committed write visible"),
        other => panic!("expected QueryResult, got {:?}", other),
    }
}

/// 小工具：把「非 Error 即成功」写成语义直白的断言（避免匹配未来新增变体）。
trait ResponseExt {
    fn is_ok_or_success(&self) -> bool;
}

impl ResponseExt for Response {
    fn is_ok_or_success(&self) -> bool {
        !matches!(self, Response::Error { .. })
    }
}
