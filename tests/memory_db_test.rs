//! MS26 Iteration 001 / T3+T4: `:memory:` 库形态（`in-memory-database` R1–R4）。
//!
//! 见证面：
//! - R1（lib，`Database::open_in_memory`）：全 CRUD/DDL 链路、二级索引等值路由、
//!   视图/ALTER/TRUNCATE、CTAS 物化进内存主库、约束拒绝面与文件库同型；
//! - R1（CLI，`rtsql :memory:`）：多语句计算管道、进程退出即弃、零文件；
//! - R2：不运行恢复、事务回滚语义（行消失 / 唯一键占用还原 / 可重插）、
//!   `checkpoint()`/`close()` 无文件 I/O（无 WAL / checkpoint 通道）；
//! - R3：六生命周期子命令与 `--key` / `--readonly` 组合对 `:memory:` 点名拒绝
//!   exit 2 且零文件；
//! - R4：内存会话 ATTACH 文件库跨查 + CTAS 组合，附加库按会话模式零特殊化打开。
//!
//! 零文件见证分工：lib 侧断言「无 `:memory:` 命名文件 + 无 WAL/checkpoint 通道」
//! （lib 不经 `RTSQL_HOME`，进程 CWD 不可安全改写）；CLI 侧以 CWD 与
//! `RTSQL_HOME` 同指独立 TempDir 断言目录零新文件——那是真实的落盘面。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tempfile::tempdir;

// ===========================================================================
// 断言辅助
// ===========================================================================

fn assert_ok(resp: Response, what: &str) {
    if let Response::Error { message, .. } = &resp {
        panic!("{what} unexpectedly failed: {message}");
    }
}

fn expect_err(resp: Response, what: &str) -> String {
    match resp {
        Response::Error { message, .. } => message,
        other => panic!("{what}: expected an error, got {other:?}"),
    }
}

fn rows(resp: Response, what: &str) -> Vec<Vec<Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        Response::Error { message, .. } => panic!("{what} failed: {message}"),
        other => panic!("{what}: unexpected response {other:?}"),
    }
}

/// 单列标量查询结果（无别名时表头即表达式文本）。
fn scalar(resp: Response, what: &str) -> Value {
    let mut got = rows(resp, what);
    assert_eq!(
        got.len(),
        1,
        "{what}: expected exactly one row, got {got:?}"
    );
    assert_eq!(
        got[0].len(),
        1,
        "{what}: expected a single column, got {got:?}"
    );
    got.remove(0).remove(0)
}

fn ints(resp: Response, what: &str) -> Vec<i64> {
    rows(resp, what)
        .into_iter()
        .map(|row| {
            assert_eq!(
                row.len(),
                1,
                "{what}: expected single-column rows, got {row:?}"
            );
            row[0]
                .as_i64()
                .unwrap_or_else(|| panic!("{what}: non-integer cell {:?}", row[0]))
        })
        .collect()
}

async fn exec_ok(db: &Database, sql: &str, what: &str) {
    assert_ok(db.execute_sql(sql).await, what);
}

/// CWD 下是否存在以 `prefix` 开头的条目（`:memory:` 落盘回归见证）。
fn cwd_entries_starting_with(prefix: &str) -> Vec<PathBuf> {
    let cwd = std::env::current_dir().expect("current dir");
    std::fs::read_dir(&cwd)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(prefix))
                })
                .collect()
        })
        .unwrap_or_default()
}

// ===========================================================================
// T3: lib 内存库面
// ===========================================================================

/// R1-S1: `open_in_memory` 全 CRUD 链路 + 与文件库同型的错误面。
#[tokio::test]
async fn t3_crud_chain_in_memory_db() {
    let db = Database::open_in_memory().await.unwrap();

    exec_ok(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, n INT, name VARCHAR)",
        "create table",
    )
    .await;
    exec_ok(
        &db,
        "INSERT INTO t VALUES (1, 10, 'a'), (2, 20, 'b'), (3, 30, 'c')",
        "insert rows",
    )
    .await;

    assert_eq!(
        ints(db.execute_sql("SELECT n FROM t").await, "select all").len(),
        3
    );
    assert_eq!(
        ints(
            db.execute_sql("SELECT n FROM t WHERE n > 15").await,
            "filtered select"
        ),
        vec![20, 30]
    );

    exec_ok(&db, "UPDATE t SET n = 99 WHERE id = 2", "update row").await;
    assert_eq!(
        scalar(
            db.execute_sql("SELECT n FROM t WHERE id = 2").await,
            "select updated row"
        ),
        Value::from(99)
    );

    exec_ok(&db, "DELETE FROM t WHERE id = 3", "delete row").await;
    assert_eq!(
        ints(
            db.execute_sql("SELECT n FROM t").await,
            "select after delete"
        )
        .len(),
        2
    );

    // 内存库可写（`is_readonly()` 为假），写路径经 `Option<WALBuffer>` 降级。
    assert!(!db.is_readonly(), "memory sessions are writable");
    db.close().await.unwrap();
}

/// R1-S2: 非 PK 列二级索引的等值路由可达（`IndexScanAll`），会话内结果等价。
#[tokio::test]
async fn t3_secondary_index_eq_routing_in_memory_db() {
    let db = Database::open_in_memory().await.unwrap();
    exec_ok(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, age INT, name VARCHAR)",
        "create table",
    )
    .await;
    exec_ok(
        &db,
        "INSERT INTO t VALUES (1, 20, 'a'), (2, 30, 'b'), (3, 20, 'c')",
        "insert rows",
    )
    .await;

    let before = ints(
        db.execute_sql("SELECT id FROM t WHERE age = 20").await,
        "scan before index",
    );
    assert_eq!(before.len(), 2, "duplicate age rows must both match");

    exec_ok(&db, "CREATE INDEX idx_age ON t(age)", "create index").await;

    let after = ints(
        db.execute_sql("SELECT id FROM t WHERE age = 20").await,
        "indexed eq query",
    );
    assert_eq!(after.len(), 2, "indexed eq query must keep both rows");
    for id in &before {
        assert!(
            after.contains(id),
            "indexed result must match scan result: {after:?}"
        );
    }
    // 索引写入路径随 DML 维护。
    exec_ok(
        &db,
        "INSERT INTO t VALUES (4, 20, 'd')",
        "insert indexed row",
    )
    .await;
    assert_eq!(
        ints(
            db.execute_sql("SELECT id FROM t WHERE age = 20").await,
            "eq query after insert"
        )
        .len(),
        3
    );
    db.close().await.unwrap();
}

/// R1-S3: 会话内 DDL 面——CREATE VIEW / ALTER TABLE ADD COLUMN / TRUNCATE。
#[tokio::test]
async fn t3_view_alter_truncate_in_memory_db() {
    let db = Database::open_in_memory().await.unwrap();
    exec_ok(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, n INT)",
        "create table",
    )
    .await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 10), (2, 20)", "insert rows").await;

    // 视图：catalog kind=View 行写入内存 catalog，会话内可查。
    exec_ok(&db, "CREATE VIEW v AS SELECT id, n FROM t", "create view").await;
    assert_eq!(
        rows(db.execute_sql("SELECT * FROM v").await, "select from view").len(),
        2
    );
    // 视图非物化：基表写入后视图读到新值。
    exec_ok(&db, "INSERT INTO t VALUES (3, 30)", "insert after view").await;
    assert_eq!(
        rows(db.execute_sql("SELECT * FROM v").await, "view sees new row").len(),
        3
    );

    // 加列：存量行按 NULL 读回。
    exec_ok(&db, "ALTER TABLE t ADD COLUMN tag INT", "add column").await;
    assert_eq!(
        scalar(
            db.execute_sql("SELECT tag FROM t WHERE id = 1").await,
            "read added column"
        ),
        Value::Null
    );

    // TRUNCATE：数据清空、表定义与索引定义保留。
    exec_ok(&db, "CREATE INDEX idx_n ON t(n)", "index before truncate").await;
    exec_ok(&db, "TRUNCATE TABLE t", "truncate").await;
    assert!(rows(
        db.execute_sql("SELECT n FROM t").await,
        "select after truncate"
    )
    .is_empty());
    exec_ok(
        &db,
        "INSERT INTO t VALUES (9, 90, 99)",
        "insert after truncate (table + index survive)",
    )
    .await;
    assert_eq!(
        ints(
            db.execute_sql("SELECT n FROM t WHERE n = 90").await,
            "indexed eq after truncate"
        ),
        vec![90]
    );
    db.close().await.unwrap();
}

/// R2-S1: 显式事务回滚语义与文件库同型——行消失、唯一键占用还原、可重插。
#[tokio::test]
async fn t3_explicit_tx_rollback_restores_unique_occupancy() {
    let db = Database::open_in_memory().await.unwrap();
    exec_ok(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, u INT UNIQUE, v INT)",
        "create table",
    )
    .await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 100, 1)", "seed row").await;

    let tx = db.begin().await.unwrap();
    // 唯一冲突在事务内被拒绝，事务保持活跃。
    expect_err(
        db.execute_in_tx("INSERT INTO t VALUES (2, 100, 2)", &tx)
            .await,
        "duplicate unique in tx",
    );
    assert_ok(
        db.execute_in_tx("INSERT INTO t VALUES (2, 200, 2)", &tx)
            .await,
        "insert in tx",
    );
    db.rollback(tx).await.unwrap();

    // 回滚后事务内写入的行消失。
    assert_eq!(
        ints(
            db.execute_sql("SELECT id FROM t").await,
            "select after rollback"
        ),
        vec![1]
    );
    // 唯一键占用还原：回滚事务插入过的唯一值可再次插入。
    assert_ok(
        db.execute_sql("INSERT INTO t VALUES (3, 200, 3)").await,
        "reinsert released unique value",
    );

    // 提交路径同样成立。
    let tx = db.begin().await.unwrap();
    assert_ok(
        db.execute_in_tx("INSERT INTO t VALUES (4, 400, 4)", &tx)
            .await,
        "insert for commit",
    );
    db.commit(tx).await.unwrap();
    assert_eq!(
        ints(
            db.execute_sql("SELECT id FROM t").await,
            "select after commit"
        )
        .len(),
        3
    );
    db.close().await.unwrap();
}

/// R2-S2: 内存会话无 WAL / checkpoint 通道，`checkpoint()`/`close()` 无文件 I/O。
#[tokio::test]
async fn t3_no_wal_or_checkpoint_channels() {
    let db = Database::open_in_memory().await.unwrap();

    assert!(
        db.wal_writer.is_none(),
        "memory sessions hold no WAL writer"
    );
    assert!(
        db.wal_buffer.is_none(),
        "memory sessions hold no WAL buffer"
    );
    assert!(
        db.checkpoint_manager.is_none(),
        "memory sessions hold no checkpoint manager"
    );

    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)", "create table").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)", "insert row").await;

    // 结构性 no-op：脏页存在时也不进入 MS18 守卫与九步 checkpoint 路径。
    db.checkpoint().await.unwrap();
    db.checkpoint().await.unwrap();
    db.close().await.unwrap();
    // 显式 checkpoint 之后数据仍在会话内可读（no-op 不破坏内存态）。
    assert_eq!(
        ints(
            db.execute_sql("SELECT id FROM t").await,
            "select after checkpoint"
        ),
        vec![1]
    );
}

/// R1-S4: 进程/会话退出即弃——新内存会话是空库，无任何恢复语义。
#[tokio::test]
async fn t3_fresh_session_is_empty() {
    {
        let db = Database::open_in_memory().await.unwrap();
        exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)", "create table").await;
        exec_ok(&db, "INSERT INTO t VALUES (1)", "insert row").await;
        db.close().await.unwrap();
    }

    let reopened = Database::open_in_memory().await.unwrap();
    let message = expect_err(
        reopened.execute_sql("SELECT id FROM t").await,
        "table from a discarded session",
    );
    assert!(
        message.contains("not found"),
        "expected a not-found error for a discarded table, got: {message}"
    );
    // 空库仍可正常建表写入。
    exec_ok(
        &reopened,
        "CREATE TABLE t (id INT PRIMARY KEY)",
        "create table in fresh session",
    )
    .await;
    reopened.close().await.unwrap();
}

/// R1-S5: 约束拒绝面与文件库同型（NOT NULL / UNIQUE / 键列类型）。
#[tokio::test]
async fn t3_constraint_rejections_match_file_db() {
    let db = Database::open_in_memory().await.unwrap();
    exec_ok(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, u INT UNIQUE, req INT NOT NULL)",
        "create table",
    )
    .await;

    let null_msg = expect_err(
        db.execute_sql("INSERT INTO t (id, u) VALUES (1, 1)").await,
        "NOT NULL violation",
    );
    assert!(
        null_msg.contains("req"),
        "NOT NULL rejection must name the column, got: {null_msg}"
    );

    exec_ok(&db, "INSERT INTO t VALUES (1, 1, 1)", "seed row").await;
    let dup_msg = expect_err(
        db.execute_sql("INSERT INTO t VALUES (2, 1, 2)").await,
        "UNIQUE violation",
    );
    assert!(
        dup_msg.to_lowercase().contains("duplicate"),
        "UNIQUE rejection must report a duplicate key, got: {dup_msg}"
    );

    let type_msg = expect_err(
        db.execute_sql("INSERT INTO t VALUES ('x', 3, 3)").await,
        "key column type mismatch",
    );
    assert!(
        type_msg.contains("id") || type_msg.contains("INT"),
        "key type rejection must name the column/type, got: {type_msg}"
    );

    // 被拒绝的写入零副作用。
    assert_eq!(
        ints(
            db.execute_sql("SELECT id FROM t").await,
            "select after rejections"
        ),
        vec![1]
    );
    db.close().await.unwrap();
}

/// R4-S1: CTAS 物化进内存主库，会话内可查。
#[tokio::test]
async fn t3_ctas_materializes_into_memory_main_db() {
    let db = Database::open_in_memory().await.unwrap();
    exec_ok(
        &db,
        "CREATE TABLE src (id INT PRIMARY KEY, n INT)",
        "create source",
    )
    .await;
    exec_ok(
        &db,
        "INSERT INTO src VALUES (1, 10), (2, 20)",
        "insert source rows",
    )
    .await;

    exec_ok(
        &db,
        "CREATE TABLE copy AS SELECT id, n FROM src WHERE n >= 20",
        "ctas",
    )
    .await;
    assert_eq!(
        ints(
            db.execute_sql("SELECT n FROM copy").await,
            "select materialized"
        ),
        vec![20]
    );
    // 物化结果是独立表，源表后续写入不影响它。
    exec_ok(&db, "INSERT INTO src VALUES (3, 30)", "insert after ctas").await;
    assert_eq!(
        ints(
            db.execute_sql("SELECT n FROM copy").await,
            "copy is independent"
        )
        .len(),
        1
    );
    db.close().await.unwrap();
}

/// R1/R2: 内存会话不产生任何 `:memory:` 命名文件（含 `.wal` / `.checkpoint`）。
///
/// 断言「会话期间没有新增 `:memory:` 命名文件」而非「目录本就为空」——工作区
/// 可能残留更早测试运行产生的同类 artifact，零文件不变量只约束本会话。
#[tokio::test]
async fn t3_creates_no_memory_named_files() {
    let before = cwd_entries_starting_with(":memory:");

    {
        let db = Database::open_in_memory().await.unwrap();
        exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)", "create table").await;
        exec_ok(&db, "INSERT INTO t VALUES (1)", "insert row").await;
        db.checkpoint().await.unwrap();
        db.close().await.unwrap();
    }

    let after = cwd_entries_starting_with(":memory:");
    assert_eq!(
        before.len(),
        after.len(),
        "memory sessions must not create :memory: files in the CWD; before={before:?} after={after:?}"
    );
}

// ===========================================================================
// T4: CLI `:memory:` 特判与拒绝面
// ===========================================================================

struct CliOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

/// CWD 与 `RTSQL_HOME` 同指独立 TempDir——`:memory:` 落盘的唯一真实面。
fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary");
    let deadline = Duration::from_secs(60);
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None if deadline.is_zero() => {
                let _ = child.kill();
                panic!("rtsql did not exit within 60s: {args:?}");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let output = child.wait_with_output().expect("wait with output");
    CliOutput {
        code: output.status.code().expect("exit code"),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

/// 目录下零条目（`:memory:` 会话不产生任何伴生文件）。
fn assert_dir_empty(dir: &Path, what: &str) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{what}: read dir: {e}"))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    assert!(
        entries.is_empty(),
        "{what}: expected no files, found {entries:?}"
    );
}

fn assert_contains(haystack: &str, needle: &str, what: &str) {
    assert!(
        haystack.contains(needle),
        "{what}: expected output to contain {needle:?}, got stdout={haystack:?}"
    );
}

/// R1-S6: CLI 多语句计算管道，退出即弃，零文件。
#[test]
fn t4_cli_memory_multi_statement_pipeline_and_zero_files() {
    let dir = tempdir().unwrap();
    let out = run_cli(
        dir.path(),
        &[
            ":memory:",
            "CREATE TABLE t (id INT PRIMARY KEY, v INT); \
             INSERT INTO t VALUES (1, 10), (2, 20); \
             SELECT sum(v) FROM t",
        ],
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_contains(&out.stdout, "30", "sum over the memory session");
    assert_dir_empty(dir.path(), "after a memory main-command session");
}

/// R1-S7: 重开 `:memory:` 是空库（进程退出即弃）。
#[test]
fn t4_cli_memory_session_is_discarded_on_exit() {
    let dir = tempdir().unwrap();
    let seeded = run_cli(
        dir.path(),
        &[
            ":memory:",
            "CREATE TABLE t (id INT PRIMARY KEY); INSERT INTO t VALUES (1)",
        ],
    );
    assert_eq!(seeded.code, 0, "stderr: {}", seeded.stderr);

    let reopened = run_cli(dir.path(), &[":memory:", "SELECT id FROM t"]);
    assert_eq!(
        reopened.code, 3,
        "a discarded table must fail as a SQL error; stdout={} stderr={}",
        reopened.stdout, reopened.stderr
    );
    assert_dir_empty(dir.path(), "after the discarded memory session");
}

/// R3-S1: `--key` / `RTSQL_KEY` 与 `:memory:` 组合点名拒绝 exit 2，零文件。
#[test]
fn t4_cli_memory_rejects_key_channels() {
    let dir = tempdir().unwrap();
    let by_flag = run_cli(dir.path(), &["--key", "pw", ":memory:", "SELECT 1"]);
    assert_eq!(
        by_flag.code, 2,
        "stdout={} stderr={}",
        by_flag.stdout, by_flag.stderr
    );
    assert_contains(
        &by_flag.stderr,
        ":memory:",
        "flag rejection names the target",
    );

    let by_env = run_cli(dir.path(), &[":memory:", "SELECT 1"]);
    assert_eq!(
        by_env.code, 0,
        "without a key the session runs; stderr: {}",
        by_env.stderr
    );

    let by_env_key = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args([":memory:", "SELECT 1"])
        .current_dir(dir.path())
        .env("RTSQL_HOME", dir.path())
        .env("RTSQL_KEY", "pw")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rtsql with RTSQL_KEY");
    assert_eq!(
        by_env_key.status.code(),
        Some(2),
        "RTSQL_KEY channel must be rejected too: {}",
        String::from_utf8_lossy(&by_env_key.stderr)
    );
    assert_dir_empty(dir.path(), "after rejected --key combinations");
}

/// R3-S2: `--readonly` 与 `:memory:` 组合点名拒绝 exit 2。
#[test]
fn t4_cli_memory_rejects_readonly() {
    let dir = tempdir().unwrap();
    let out = run_cli(dir.path(), &["--readonly", ":memory:", "SELECT 1"]);
    assert_eq!(out.code, 2, "stdout={} stderr={}", out.stdout, out.stderr);
    assert_contains(
        &out.stderr,
        ":memory:",
        "readonly rejection names the target",
    );
    assert_dir_empty(dir.path(), "after the rejected readonly combination");
}

/// R3-S3: 六生命周期子命令对 `:memory:` 实参点名拒绝 exit 2，零文件。
#[test]
fn t4_cli_lifecycle_subcommands_reject_memory_target() {
    let dir = tempdir().unwrap();
    let invocations: Vec<Vec<&str>> = vec![
        vec!["new", ":memory:"],
        vec!["schema", ":memory:"],
        vec!["dump", ":memory:"],
        vec!["restore", ":memory:", "-"],
        vec!["import", ":memory:", "t", "data.csv", "--csv"],
        vec!["delete", ":memory:"],
    ];
    for args in invocations {
        let out = run_cli(dir.path(), &args);
        assert_eq!(
            out.code,
            2,
            "`rtsql {}` must reject :memory: with usage exit 2; stdout={} stderr={}",
            args.join(" "),
            out.stdout,
            out.stderr
        );
        assert_contains(
            &out.stderr,
            ":memory:",
            &format!("`rtsql {}` rejection names the target", args.join(" ")),
        );
    }
    assert_dir_empty(dir.path(), "after rejected lifecycle invocations");
}

/// R4-S2: 内存会话 ATTACH 文件库跨查 + CTAS 物化，会话内可查。
#[test]
fn t4_cli_memory_session_attaches_file_db_and_ctas() {
    let dir = tempdir().unwrap();
    // CLI 不创建父目录（`new` 除外），裸名解析需要预建集中存储区。
    let db_dir = dir.path().join("db");
    std::fs::create_dir(&db_dir).unwrap();
    // 预置一个真实文件库（非 memory）。
    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建（断言语义不变）。
    let new_out = run_cli(dir.path(), &["new", "seed"]);
    assert_eq!(new_out.code, 0, "new seed failed: {}", new_out.stderr);
    let seed = run_cli(
        dir.path(),
        &[
            "seed",
            "CREATE TABLE t (id INT PRIMARY KEY, v INT); INSERT INTO t VALUES (7, 70)",
        ],
    );
    assert_eq!(seed.code, 0, "stderr: {}", seed.stderr);
    assert!(
        db_dir.join("seed.wal").exists(),
        "file-backed session must leave its WAL behind"
    );

    let out = run_cli(
        dir.path(),
        &[
            ":memory:",
            "ATTACH 'seed' AS f; \
             CREATE TABLE copy AS SELECT v FROM f.t; \
             INSERT INTO copy VALUES (80); \
             SELECT v FROM copy",
        ],
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_contains(&out.stdout, "70", "CTAS materialized the attached row");
    assert_contains(&out.stdout, "80", "the memory main table accepts writes");
}
