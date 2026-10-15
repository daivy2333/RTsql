//! MS26 Iteration 000 / T1: 只读打开模式（`readonly-open-mode` R1/R2/R3/R6）。
//!
//! 见证面：
//! - R1 入口与共享锁：双只读并发打开（LOCK_SH ×2）、写者拒只读者、只读者拒写者、
//!   缺失文件不创建、加密库只读打开 + 错误密钥 `InvalidKey`；
//! - R2 WAL 未决拒绝：checkpoint-clean 放行、未决点名拒绝且文件未动、
//!   拒绝后读写恢复一次即可只读打开；
//! - R3 零写入：只读会话（含显式 `checkpoint()` / `close()`）前后主文件、
//!   `.wal`、`.checkpoint` 字节 checksum 一致；
//! - R6 既有打开语义零回归：默认读写打开在本文件内的形态保持。
//!
//! T2 的只读拒绝面（25006 / `--readonly` / ATTACH 只读传递）由本文件后续分组
//! 见证。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::storage::StorageError;
use rtsql::transaction::IsolationLevel;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::{tempdir, TempDir};

const KEY: &str = "pw";

/// T4（D8 校准）：锁冲突断言用例的显式短预算——`Some(0)` ＝ 仅 ~8ms 可见性
/// 下限窗口即拒绝（MS28 前默认语义的显式表达），断言本身零变化。
const ZERO_WAIT: Option<Duration> = Some(Duration::from_millis(0));

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("ro.db")
}

fn wal_path(db: &Path) -> PathBuf {
    db.with_extension("wal")
}

fn site_path(db: &Path) -> PathBuf {
    db.with_extension("checkpoint")
}

/// unwrap_err 的等价形式：不要求 Ok 侧类型实现 Debug。
fn expect_err<T>(result: Result<T, StorageError>) -> StorageError {
    match result {
        Ok(_) => panic!("expected an open error, got Ok"),
        Err(e) => e,
    }
}

fn assert_locked(err: &StorageError) {
    assert!(
        matches!(err, StorageError::DatabaseLocked(_)),
        "expected StorageError::DatabaseLocked, got: {err:?}"
    );
}

fn rows(resp: Response, what: &str) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        Response::Error { message, .. } => panic!("{what} failed: {message}"),
        other => panic!("{what}: unexpected response {other:?}"),
    }
}

fn assert_ok(resp: Response, what: &str) {
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{what} unexpectedly failed: {resp:?}"
    );
}

/// 建库 + 写入两行 + close（checkpoint-clean 形态：`.wal` 仅含 Checkpoint 标记）。
async fn seed(dir: &TempDir) -> PathBuf {
    let path = db_path(dir);
    let db = Database::open(&path).await.unwrap();
    assert_ok(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
            .await,
        "create table",
    );
    assert_ok(
        db.execute_sql("INSERT INTO t VALUES (1, 10), (2, 20)")
            .await,
        "insert rows",
    );
    db.close().await.unwrap();
    path
}

/// ---------------------------------------------------------------- R1
/// R1-S1：两只读会话并发打开同库并各自查询，结果一致完整；关闭其一不影响另一。
#[tokio::test]
async fn r1_two_readonly_sessions_open_and_query() {
    let dir = tempdir().unwrap();
    let path = seed(&dir).await;

    let a = Database::open_readonly(&path).await.unwrap();
    let b = Database::open_readonly(&path).await.unwrap();

    let expected = vec![vec![serde_json::json!(1), serde_json::json!(10)]];
    for (db, tag) in [(&a, "a"), (&b, "b")] {
        let got = rows(db.execute_sql("SELECT * FROM t WHERE id = 1").await, tag);
        assert_eq!(got, expected, "session {tag} row mismatch");
    }
    let all = rows(a.execute_sql("SELECT * FROM t").await, "full scan");
    assert_eq!(all.len(), 2, "full scan should see both committed rows");

    drop(a);
    let after = rows(b.execute_sql("SELECT * FROM t").await, "b after a closed");
    assert_eq!(
        after.len(),
        2,
        "closing one session must not affect the other"
    );
}

/// R1-S2：写者持独占锁时只读打开被拒（`DatabaseLocked`），文件未被触碰。
#[tokio::test]
async fn r1_readonly_rejected_while_writer_holds_lock() {
    let dir = tempdir().unwrap();
    let path = seed(&dir).await;
    let before = std::fs::read(&path).unwrap();

    let _writer = Database::open(&path).await.unwrap();
    let err = expect_err(Database::open_readonly_with_lock_wait(&path, None, ZERO_WAIT).await);
    assert_locked(&err);

    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "rejected readonly open must not touch the database file"
    );
}

/// R1-S3：只读者持共享锁时读写打开被拒（写写互斥同型排除）。
#[tokio::test]
async fn r1_writer_rejected_while_readonly_holds_lock() {
    let dir = tempdir().unwrap();
    let path = seed(&dir).await;

    let reader = Database::open_readonly(&path).await.unwrap();
    let err = expect_err(
        Database::open_with_lock_wait(&path, IsolationLevel::RepeatableRead, None, ZERO_WAIT).await,
    );
    assert_locked(&err);

    drop(reader);
    assert!(
        Database::open(&path).await.is_ok(),
        "read-write open succeeds after the shared lock is released"
    );
}

/// R1-S4：只读打开缺失文件报错且不创建文件。
#[tokio::test]
async fn r1_readonly_missing_file_not_created() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("missing.db");

    let err = expect_err(Database::open_readonly(&path).await);
    let msg = err.to_string();
    assert!(
        msg.contains(&path.display().to_string()),
        "error should name the path, got: {msg}"
    );
    assert!(
        !path.exists(),
        "readonly open must not create the missing database file"
    );
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(
        entries.is_empty(),
        "directory must stay empty, got: {entries:?}"
    );
}

/// R1-S5：加密库只读打开可查；错误密钥沿既有 `InvalidKey` 拒绝。
#[tokio::test]
async fn r1_encrypted_database_readonly() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("enc.db");
    {
        let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some(KEY))
            .await
            .unwrap();
        assert_ok(
            db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
                .await,
            "create table",
        );
        assert_ok(
            db.execute_sql("INSERT INTO t VALUES (1, 7)").await,
            "insert",
        );
        db.close().await.unwrap();
    }

    let db = Database::open_readonly_with_key(&path, Some(KEY))
        .await
        .unwrap();
    let got = rows(
        db.execute_sql("SELECT n FROM t WHERE id = 1").await,
        "enc select",
    );
    assert_eq!(got, vec![vec![serde_json::json!(7)]]);

    drop(db);
    let err = expect_err(Database::open_readonly_with_key(&path, Some("wrong")).await);
    assert!(
        matches!(err, StorageError::InvalidKey(_)),
        "expected InvalidKey for wrong key, got: {err:?}"
    );
}

/// ---------------------------------------------------------------- R2
/// R2-S1：checkpoint-clean 库只读打开成功，数据为最近一次提交的完整状态。
#[tokio::test]
async fn r2_checkpoint_clean_database_opens_readonly() {
    let dir = tempdir().unwrap();
    let path = seed(&dir).await;

    let db = Database::open_readonly(&path).await.unwrap();
    let got = rows(
        db.execute_sql("SELECT n FROM t WHERE id = 2").await,
        "select",
    );
    assert_eq!(got, vec![vec![serde_json::json!(20)]]);
}

/// R2-S2：WAL 未决（崩溃遗留事务记录）时只读打开点名拒绝，三个文件字节未变。
#[tokio::test]
async fn r2_pending_wal_rejected_without_touching_files() {
    let dir = tempdir().unwrap();
    let path = seed(&dir).await;
    append_pending_wal_records(&path).await;

    let before: Vec<Vec<u8>> = [&path, &wal_path(&path), &site_path(&path)]
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();

    let err = expect_err(Database::open_readonly(&path).await);
    assert!(
        matches!(err, StorageError::WalPendingRecovery(_)),
        "expected WalPendingRecovery, got: {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("pending WAL") && msg.contains("read-write"),
        "error should name the pending WAL and the read-write recovery hint, got: {msg}"
    );

    for (p, bytes) in [&path, &wal_path(&path), &site_path(&path)]
        .iter()
        .zip(before.iter())
    {
        assert_eq!(
            &std::fs::read(p).unwrap(),
            bytes,
            "rejected readonly open must not modify {}",
            p.display()
        );
    }
}

/// R2-S3：未决拒绝不妨碍恢复路径——读写打开执行既有恢复并 close 后只读可开。
#[tokio::test]
async fn r2_readonly_available_after_read_write_recovery() {
    let dir = tempdir().unwrap();
    let path = seed(&dir).await;
    append_pending_wal_records(&path).await;

    assert!(expect_err(Database::open_readonly(&path).await)
        .to_string()
        .contains("pending WAL"));

    {
        let db = Database::open(&path).await.unwrap();
        assert_ok(
            db.execute_sql("INSERT INTO t VALUES (3, 30)").await,
            "insert after recovery",
        );
        db.close().await.unwrap();
    }

    let db = Database::open_readonly(&path).await.unwrap();
    let got = rows(
        db.execute_sql("SELECT n FROM t WHERE id = 3").await,
        "select",
    );
    assert_eq!(got, vec![vec![serde_json::json!(30)]]);
}

/// ---------------------------------------------------------------- R3
/// R3-S1：只读会话（查询 + 显式 checkpoint + close）前后三文件字节一致。
#[tokio::test]
async fn r3_readonly_session_writes_nothing() {
    let dir = tempdir().unwrap();
    let path = seed(&dir).await;
    let files = [path.clone(), wal_path(&path), site_path(&path)];
    let before: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let before_entries = dir_entry_names(dir.path());

    {
        let db = Database::open_readonly(&path).await.unwrap();
        assert_ok(db.execute_sql("SELECT COUNT(*) FROM t").await, "aggregate");
        assert_ok(
            db.execute_sql("SELECT * FROM t WHERE id = 1").await,
            "indexed point",
        );
        db.checkpoint().await.unwrap();
        db.close().await.unwrap();
    }

    for (p, bytes) in files.iter().zip(before.iter()) {
        assert_eq!(
            &std::fs::read(p).unwrap(),
            bytes,
            "readonly session must not write {}",
            p.display()
        );
    }
    assert_eq!(
        dir_entry_names(dir.path()),
        before_entries,
        "readonly session must not create or remove companion files"
    );
}

fn dir_entry_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// 未决 WAL 夹具：用库自身序列化器（`WalWriter`）向 `.wal` 追加一笔未决
/// 事务（`BeginTxn` + `CommitTxn`，无数据记录）——谓词只读记录种类，判定
/// 不依赖崩溃时序，而帧本身是可被恢复路径消费的真实形态。
async fn append_pending_wal_records(db: &Path) {
    let writer = rtsql::wal::WalWriter::open(db).unwrap();
    writer
        .write_record(rtsql::wal::WalRecord::BeginTxn { tx_id: 9001 })
        .await
        .unwrap();
    writer
        .write_record(rtsql::wal::WalRecord::CommitTxn {
            tx_id: 9001,
            timestamp: 0,
        })
        .await
        .unwrap();
    writer.fsync().await.unwrap();
}

// =============================================================================
// T2：只读拒绝面与 CLI（`readonly-open-mode` R4/R5 + `error-reporting` R2）
// =============================================================================

use rtsql::transaction::Transaction;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Instant;

/// R4：只读会话 13 个写计划变体逐一点名拒绝（SQLSTATE 25006），库与 schema
/// 无变化；拒绝先于任何写入生效。
#[tokio::test]
async fn r4_readonly_session_rejects_every_write_plan_shape() {
    let dir = tempdir().unwrap();
    let path = seed_with_index_and_view(&dir).await;

    let db = Database::open_readonly(&path).await.unwrap();
    let writes = [
        ("INSERT", "INSERT INTO t VALUES (99, 99)"),
        ("UPDATE", "UPDATE t SET n = 1 WHERE id = 1"),
        ("DELETE", "DELETE FROM t WHERE id = 1"),
        (
            "UPSERT",
            "INSERT INTO t VALUES (1, 5) ON CONFLICT DO NOTHING",
        ),
        ("REPLACE", "REPLACE INTO t VALUES (1, 5)"),
        ("CREATE TABLE", "CREATE TABLE u (id INT PRIMARY KEY)"),
        (
            "CREATE TABLE AS SELECT",
            "CREATE TABLE w AS SELECT * FROM t",
        ),
        ("CREATE INDEX", "CREATE INDEX idx2 ON t (n)"),
        ("ALTER TABLE", "ALTER TABLE t ADD COLUMN m INT"),
        ("DROP INDEX", "DROP INDEX idx_n"),
        ("TRUNCATE", "TRUNCATE TABLE t"),
        ("CREATE VIEW", "CREATE VIEW vv AS SELECT * FROM t"),
        ("DROP VIEW", "DROP VIEW v_seed"),
        ("DROP TABLE", "DROP TABLE t"),
    ];
    for (kind, sql) in writes {
        match db.execute_sql(sql).await {
            Response::Error { message, sqlstate } => {
                assert_eq!(
                    sqlstate.as_deref(),
                    Some("25006"),
                    "{kind}: expected SQLSTATE 25006 for `{sql}`"
                );
                assert!(
                    message.contains("read-only session"),
                    "{kind}: message should name the read-only session, got: {message}"
                );
            }
            other => panic!("{kind}: expected rejection for `{sql}`, got {other:?}"),
        }
    }

    // 库与 schema 无变化：两行原样、索引与视图仍可读。
    let got = rows(db.execute_sql("SELECT id, n FROM t").await, "scan");
    assert_eq!(got.len(), 2, "rows must be untouched");
    assert_eq!(got[0], vec![serde_json::json!(1), serde_json::json!(10)]);
    let via_view = rows(db.execute_sql("SELECT n FROM v_seed").await, "view");
    assert_eq!(via_view.len(), 2, "view must still expand over both rows");
    let via_index = rows(
        db.execute_sql("SELECT id FROM t WHERE n = 20").await,
        "index",
    );
    assert_eq!(via_index, vec![vec![serde_json::json!(2)]]);
}

/// R4：只读会话内只读语句不受限（SELECT 组合面、视图、显式事务包住 SELECT；
/// 事务内写语句被拒且事务保持活跃）。
#[tokio::test]
async fn r4_readonly_statements_are_not_restricted() {
    let dir = tempdir().unwrap();
    let path = seed_with_index_and_view(&dir).await;
    let db = Database::open_readonly(&path).await.unwrap();

    let one = |resp: Response, what: &str| {
        assert_eq!(rows(resp, what).len(), 2, "{what}");
    };
    one(
        db.execute_sql("SELECT id FROM t WHERE n > 5").await,
        "where",
    );
    one(
        db.execute_sql("SELECT t.id FROM t JOIN v_seed ON v_seed.id = t.id")
            .await,
        "join",
    );
    one(
        db.execute_sql("SELECT id FROM (SELECT * FROM t) AS sub")
            .await,
        "derived",
    );
    one(
        db.execute_sql("SELECT id FROM t WHERE id IN (SELECT id FROM t)")
            .await,
        "subquery",
    );

    let tx: Transaction = db.begin().await.unwrap();
    assert_ok(
        db.execute_in_tx("SELECT n FROM t WHERE id = 1", &tx).await,
        "in-tx select",
    );
    match db.execute_in_tx("INSERT INTO t VALUES (7, 7)", &tx).await {
        Response::Error { sqlstate, .. } => assert_eq!(sqlstate.as_deref(), Some("25006")),
        other => panic!("in-tx write should be rejected, got {other:?}"),
    }
    assert_ok(
        db.execute_in_tx("SELECT n FROM t WHERE id = 2", &tx).await,
        "in-tx select after rejection",
    );
    db.rollback(tx).await.unwrap();
}

/// R5：只读会话的 ATTACH 随会话只读打开附加库（共享锁 + WAL 未决谓词）；
/// 失败零副作用。
#[tokio::test]
async fn r5_readonly_session_attaches_readonly() {
    let dir = tempdir().unwrap();
    let main = seed(&dir).await;
    let attached = dir.path().join("b.db");
    {
        let db = Database::open(&attached).await.unwrap();
        assert_ok(
            db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY)").await,
            "create t2",
        );
        assert_ok(
            db.execute_sql("INSERT INTO t2 VALUES (5)").await,
            "insert t2",
        );
        db.close().await.unwrap();
    }
    let attach_sql = format!("ATTACH '{}' AS b", attached.display());

    // 附加库被写者独占 → 只读会话 ATTACH 失败（锁冲突，零副作用）。
    {
        let _writer = Database::open(&attached).await.unwrap();
        let out = run_cli_env(
            dir.path(),
            &["--readonly", main.to_str().unwrap(), &attach_sql],
            &[("RTSQL_BUSY_TIMEOUT", "0")],
        );
        assert_eq!(out.code, Some(4), "locked attach: {}", out.stderr);
        assert!(
            out.stderr.contains("database is locked"),
            "locked attach stderr: {}",
            out.stderr
        );
    }

    // 写者退出后 ATTACH 成功，跨库只读查询可用。
    let ok = run_cli(
        dir.path(),
        &[
            "--readonly",
            main.to_str().unwrap(),
            &format!("{attach_sql}; SELECT id FROM b.t2"),
        ],
    );
    assert_eq!(ok.code, Some(0), "readonly attach+query: {}", ok.stderr);
    assert!(
        ok.stdout.contains('5'),
        "cross-database row should be visible, got: {}",
        ok.stdout
    );

    // 附加库 WAL 未决 → ATTACH 失败并点名未决 WAL。
    append_pending_wal_records(&attached).await;
    let out = run_cli(
        dir.path(),
        &[
            "--readonly",
            main.to_str().unwrap(),
            &format!("{attach_sql}; SELECT id FROM t"),
        ],
    );
    assert_eq!(out.code, Some(3), "pending-wal attach: {}", out.stderr);
    assert!(
        out.stderr.contains("pending WAL"),
        "pending-wal attach stderr: {}",
        out.stderr
    );
}

/// CLI `--readonly` 端到端：正常查询；WAL 未决 → exit 1 点名恢复指引；
/// 锁冲突 → exit 4；无 flag 的默认读写路径不受影响。
#[tokio::test]
async fn cli_readonly_open_modes() {
    let dir = tempdir().unwrap();
    let path = seed(&dir).await;

    let ok = run_cli(
        dir.path(),
        &[
            "--readonly",
            path.to_str().unwrap(),
            "SELECT n FROM t WHERE id = 2",
        ],
    );
    assert_eq!(ok.code, Some(0), "readonly select: {}", ok.stderr);
    assert!(ok.stdout.contains("20"), "stdout: {}", ok.stdout);

    // 锁冲突 → exit 4（锁先于伴生文件判定，同读写构造纪律）。
    {
        let _writer = Database::open(&path).await.unwrap();
        let locked = run_cli_env(
            dir.path(),
            &["--readonly", path.to_str().unwrap(), "SELECT n FROM t"],
            &[("RTSQL_BUSY_TIMEOUT", "0")],
        );
        assert_eq!(locked.code, Some(4), "locked readonly: {}", locked.stderr);
        assert!(
            locked.stderr.contains("database is locked"),
            "locked stderr: {}",
            locked.stderr
        );
    }

    // WAL 未决 → exit 1，文案点名恢复指引。
    append_pending_wal_records(&path).await;
    let pending = run_cli(
        dir.path(),
        &["--readonly", path.to_str().unwrap(), "SELECT n FROM t"],
    );
    assert_eq!(pending.code, Some(1), "pending wal: {}", pending.stderr);
    assert!(
        pending.stderr.contains("pending WAL") && pending.stderr.contains("read-write"),
        "pending-wal stderr should name the WAL and the read-write recovery hint: {}",
        pending.stderr
    );

    // 无 flag 的默认读写路径不受影响。
    let rw = run_cli(
        dir.path(),
        &[path.to_str().unwrap(), "INSERT INTO t VALUES (4, 40)"],
    );
    assert_eq!(rw.code, Some(0), "read-write default: {}", rw.stderr);
}

/// T9（MS26 Iteration 003）：`--readonly` × 生命周期子命令的组合拒绝由
/// 人工 CLI 实跑见证升级为自动化断言——Usage exit 2 且点名 flag 归属，
/// 不静默忽略。`new` 额外见证守卫早于父目录创建。
#[tokio::test]
async fn cli_readonly_rejected_with_lifecycle_subcommands() {
    let dir = tempdir().unwrap();
    let path = seed(&dir).await;
    let db = path.to_str().unwrap();

    for args in [
        vec!["--readonly", "schema", db],
        vec!["--readonly", "dump", db],
        vec!["--readonly", "delete", db],
        vec!["--readonly", "restore", db, "dump.sql"],
        vec!["--readonly", "import", db, "t", "rows.csv", "--csv"],
        // 分析子命令同走通用拒绝臂（此前零断言）
        vec!["--readonly", "stats", db, "t"],
        vec!["--readonly", "sample", db, "t"],
        vec!["--readonly", "profile", db, "t"],
    ] {
        let out = run_cli(dir.path(), &args);
        assert_eq!(
            out.code,
            Some(2),
            "lifecycle subcommand with --readonly must be a Usage rejection: {args:?} -> {:?}",
            out.stderr
        );
        assert!(
            out.stderr.contains("--readonly") && out.stderr.contains("main command"),
            "rejection must name the flag and its owning command: {args:?} -> {}",
            out.stderr
        );
    }

    // `new` 的守卫早于父目录创建：拒绝后不留下任何文件。
    let fresh = dir.path().join("created-by-new");
    let out = run_cli(dir.path(), &["--readonly", "new", fresh.to_str().unwrap()]);
    assert_eq!(out.code, Some(2), "new with --readonly: {}", out.stderr);
    assert!(
        !fresh.exists(),
        "rejection must precede parent directory creation"
    );
}

// ---------------------------------------------------------------------------
// T2 helpers
// ---------------------------------------------------------------------------

/// 含二级索引与视图的种子库（只为让 DROP INDEX / DROP VIEW 走到计划期）。
async fn seed_with_index_and_view(dir: &TempDir) -> PathBuf {
    let path = db_path(dir);
    let db = Database::open(&path).await.unwrap();
    assert_ok(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
            .await,
        "create t",
    );
    assert_ok(
        db.execute_sql("INSERT INTO t VALUES (1, 10), (2, 20)")
            .await,
        "insert t",
    );
    assert_ok(db.execute_sql("CREATE INDEX idx_n ON t (n)").await, "index");
    assert_ok(
        db.execute_sql("CREATE VIEW v_seed AS SELECT * FROM t")
            .await,
        "view",
    );
    db.close().await.unwrap();
    path
}

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

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
        .env_remove("RTSQL_KEY");
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

/// ---------------------------------------------------------------- R7 (T7)
/// S27：文本格式错误行尾携带 ` [SQLSTATE xxxx]`——`--readonly` 下 INSERT 的
/// 25006 拒绝（exit 3 与文案不变，stdout 纯净）。
/// S28：`--format json` 时同一错误以单行 JSON 对象发射 stderr（`error` 值为
/// 同一框架文本减 suffix，`sqlstate` 字段携带 25006）。
#[tokio::test]
async fn cli_error_output_carries_sqlstate() {
    let dir = tempdir().unwrap();
    let path = seed(&dir).await;
    let p = path.to_str().unwrap();

    // S27：文本格式（显式 table）——行尾 suffix，exit 3，错误不进 stdout。
    let out = run_cli(
        dir.path(),
        &[
            "--readonly",
            "--format",
            "table",
            p,
            "INSERT INTO t VALUES (3, 30)",
        ],
    );
    assert_eq!(
        out.code,
        Some(3),
        "readonly insert must fail: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("read-only session"),
        "message text preserved: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("[SQLSTATE 25006]"),
        "text error must end with the SQLSTATE suffix: {}",
        out.stderr
    );
    assert!(
        out.stdout.is_empty(),
        "error must not enter stdout: {}",
        out.stdout
    );

    // S28：--format json——单行 JSON 错误对象，含 sqlstate 字段、不含 suffix。
    let out = run_cli(
        dir.path(),
        &[
            "--readonly",
            "--format",
            "json",
            p,
            "INSERT INTO t VALUES (3, 30)",
        ],
    );
    assert_eq!(
        out.code,
        Some(3),
        "readonly insert must fail: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains(r#""sqlstate":"25006""#),
        "json error must carry the sqlstate field: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains(
            r#""error":"statement 1 of 1 failed: cannot execute INSERT in a read-only session"#
        ),
        "json error value must be the framed message without suffix: {}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("[SQLSTATE 25006]"),
        "json error value must not embed the text suffix: {}",
        out.stderr
    );
    assert!(
        out.stdout.is_empty(),
        "error must not enter stdout: {}",
        out.stdout
    );
}
