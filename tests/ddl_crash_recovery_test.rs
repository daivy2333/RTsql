//! change 2026-10-13-e2e-verification-and-defect-closure / Iteration 002 T3
//! （R3 / S7–S8）：DDL 语句边界持久化崩溃矩阵。
//!
//! 崩溃模拟沿用 recovery_replay_convergence_test.rs 先例：WAL shutdown
//! （不落 checkpoint）后 drop（不 close）→ 脏页丢失 → 重开走恢复。
//! pre-fix 模型 DDL catalog 写只留在脏页，重开即丢失（或 WAL redo 因
//! catalog 缺表硬失败）；post-fix（D4/D5）DDL 成功返回前有序落盘，
//! 重开对象仍在且 `check` 无 error 级发现。
//!
//! 矩阵：CREATE TABLE / CREATE INDEX / CREATE VIEW / ALTER ADD COLUMN /
//! CTAS 各形态建后即崩恢复 + 既有表普通写入崩溃恢复对照。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::storage::integrity::check_integrity;
use std::path::PathBuf;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

/// 执行单条 SQL 并断言成功。
async fn ok(db: &Database, sql: &str) {
    let resp = db.execute_sql(sql).await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{sql} must succeed: {resp:?}"
    );
}

/// 断言查询结果行集。
async fn rows(db: &Database, sql: &str) -> Vec<Vec<serde_json::Value>> {
    match db.execute_sql(sql).await {
        Response::QueryResult { rows } => rows,
        other => panic!("{sql} must return rows: {other:?}"),
    }
}

/// 断言重开会话结构干净（`check` error 级发现为零）。
async fn assert_check_clean(db: &Database) {
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(
        report.error_count(),
        0,
        "no error-level findings after crash recovery: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.severity == rtsql::storage::integrity::FindingSeverity::Error)
            .collect::<Vec<_>>()
    );
}

/// 崩溃模拟：WAL 落盘后 drop（不 close，脏页不刷）。
async fn crash(db: Database) {
    db.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(db);
}

/// CREATE TABLE 建后即崩：表（含 UNIQUE / NOT NULL / DEFAULT 列）重开可达，
/// check 干净。pre-fix：catalog 行仅脏页持有 → 重开表丢失 → RED。
#[tokio::test]
async fn create_table_survives_crash() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();
    ok(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, n INT NOT NULL, u INT UNIQUE, pad STRING DEFAULT 'x')",
    )
    .await;
    crash(db).await;

    let db2 = Database::open(&path).await.unwrap();
    ok(&db2, "INSERT INTO t VALUES (1, 10, 100, DEFAULT)").await;
    assert_eq!(
        rows(&db2, "SELECT n, u, pad FROM t WHERE id = 1").await,
        vec![vec![
            serde_json::json!(10),
            serde_json::json!(100),
            serde_json::json!("x"),
        ]],
    );
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}

/// CREATE INDEX 建后即崩：索引登记与树重开可达，等值查询正确，
/// check 干净（完整性校验会自 catalog 登记段遍历二级树）。
#[tokio::test]
async fn create_index_survives_crash() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();
    ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    for i in 1..=200 {
        ok(&db, &format!("INSERT INTO t VALUES ({i}, {})", i % 10)).await;
    }
    ok(&db, "CREATE INDEX idx_n ON t (n)").await;
    crash(db).await;

    let db2 = Database::open(&path).await.unwrap();
    assert_eq!(
        rows(&db2, "SELECT COUNT(*) FROM t WHERE n = 7").await,
        vec![vec![serde_json::json!(20)]],
    );
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}

/// CREATE VIEW 建后即崩：视图定义重开可达且可查询，check 干净。
#[tokio::test]
async fn create_view_survives_crash() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();
    ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    ok(&db, "CREATE VIEW v AS SELECT id, n FROM t").await;
    crash(db).await;

    let db2 = Database::open(&path).await.unwrap();
    assert!(
        db2.views.read().await.contains_key("v"),
        "view registration must survive crash"
    );
    assert_eq!(
        rows(&db2, "SELECT n FROM v WHERE id = 1").await,
        vec![vec![serde_json::json!(10)]],
    );
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}

/// ALTER TABLE ADD COLUMN 建后即崩：新列（含 DEFAULT）重开可见，
/// 存量行按 DEFAULT 回填，check 干净。pre-fix：列行/表行更新仅脏页持有
/// → 重开列丢失 → RED。
#[tokio::test]
async fn alter_add_column_survives_crash() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();
    ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    ok(&db, "ALTER TABLE t ADD COLUMN pad STRING DEFAULT 'dft'").await;
    ok(&db, "INSERT INTO t (id, n, pad) VALUES (2, 20, 'z')").await;
    crash(db).await;

    let db2 = Database::open(&path).await.unwrap();
    assert_eq!(
        rows(&db2, "SELECT pad FROM t WHERE id = 1").await,
        vec![vec![serde_json::json!("dft")]],
    );
    assert_eq!(
        rows(&db2, "SELECT pad FROM t WHERE id = 2").await,
        vec![vec![serde_json::json!("z")]],
    );
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}

/// CTAS 建后即崩：CTAS 沿 create 通道继承有序落盘，目标表与数据重开可达。
#[tokio::test]
async fn ctas_survives_crash() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();
    ok(&db, "CREATE TABLE src (id INT PRIMARY KEY, n INT)").await;
    ok(&db, "INSERT INTO src VALUES (1, 10), (2, 20)").await;
    ok(&db, "CREATE TABLE dst AS SELECT * FROM src").await;
    crash(db).await;

    let db2 = Database::open(&path).await.unwrap();
    assert_eq!(
        rows(&db2, "SELECT COUNT(*) FROM dst").await,
        vec![vec![serde_json::json!(2)]],
    );
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}

/// 对照：既有表普通写入（WAL 记录、无 DDL）崩溃恢复既有面不回归。
#[tokio::test]
async fn control_plain_insert_on_existing_table_recovers() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db1 = Database::open(&path).await.unwrap();
    ok(&db1, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    db1.close().await.unwrap();
    // catalog_durability_test.rs 先例：close(&self) 不释放 advisory 锁，
    // 显式 drop 后方可重开。
    drop(db1);

    let db2 = Database::open(&path).await.unwrap();
    ok(&db2, "INSERT INTO t VALUES (1, 10)").await;
    crash(db2).await;

    let db2 = Database::open(&path).await.unwrap();
    assert_eq!(
        rows(&db2, "SELECT n FROM t WHERE id = 1").await,
        vec![vec![serde_json::json!(10)]],
    );
    assert_check_clean(&db2).await;
    db2.close().await.unwrap();
}
