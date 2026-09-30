//! Integration tests for MS21 Iteration 004 (T4-a/T4-b): TRUNCATE TABLE
//! （delta spec `truncate-table` R1–R2，design D11 checkpoint-first 序）。
//!
//! Scenario mapping (requirement → scenario → test):
//!   R1-S1 清空后查询空集且 schema 不变      truncate_empties_and_preserves_schema
//!   R1-S1+ TRUNCATE 与 TRUNCATE TABLE 等价  truncate_without_table_keyword
//!   R1-S2 物理释放与重开持久                truncate_persists_across_reopen
//!   R1-S3 显式事务内拒绝                    truncate_inside_explicit_transaction_rejected
//!   R1-S4 多表与目标边界拒绝                truncate_boundary_rejections
//!   R1-S5 空表幂等                          truncate_empty_table_idempotent
//!   R2-S1 崩溃后行不复活（drop 不 close）    truncate_consistent_across_crash_recovery

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

async fn open_db() -> (Database, TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    (db, dir)
}

async fn exec_ok(db: &Database, sql: &str, what: &str) {
    let resp = db.execute_sql(sql).await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{what} failed: {sql:?} -> {resp:?}"
    );
}

async fn count_rows(db: &Database, sql: &str, what: &str) -> Vec<Vec<serde_json::Value>> {
    match db.execute_sql(sql).await {
        Response::QueryResult { rows } => rows,
        other => panic!("{what}: expected QueryResult, got {other:?}"),
    }
}

fn expect_error(resp: Response, what: &str, needle: &str) {
    match resp {
        Response::Error { message, .. } => assert!(
            message.contains(needle),
            "{what}: error must name {needle:?}, got: {message}"
        ),
        other => panic!("{what}: expected Error naming {needle:?}, got {other:?}"),
    }
}

/// 种子：`t(id INT PRIMARY KEY, v INT)` 含三行 + 二级索引 `idx_v ON t(v)`。
async fn seed_with_secondary(db: &Database) {
    exec_ok(db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)", "create t").await;
    exec_ok(db, "CREATE INDEX idx_v ON t(v)", "create index").await;
    for (id, v) in [(1, 10), (2, 20), (3, 30)] {
        exec_ok(db, &format!("INSERT INTO t VALUES ({id}, {v})"), "insert").await;
    }
}

// ===========================================================================
// R1-S1: 清空后查询空集且 schema 不变
// ===========================================================================

/// R1-S1：TRUNCATE 后 COUNT 为 0、二级索引列等值查询空集、表定义保留、
/// 新写入正常且可经 PK 点查与二级索引列查询可达。
#[tokio::test]
async fn truncate_empties_and_preserves_schema() {
    let (db, _dir) = open_db().await;
    seed_with_secondary(&db).await;
    assert_eq!(
        count_rows(&db, "SELECT COUNT(*) FROM t", "pre").await.len(),
        1
    );
    let n: i64 = count_rows(&db, "SELECT COUNT(*) FROM t", "pre").await[0][0]
        .as_i64()
        .unwrap();
    assert_eq!(n, 3);

    exec_ok(&db, "TRUNCATE TABLE t", "truncate").await;

    // 清空：COUNT 0；二级索引列等值（IndexScanAll 路由）空集
    let n = count_rows(&db, "SELECT COUNT(*) FROM t", "post count").await;
    assert_eq!(n[0][0].as_i64().unwrap(), 0, "table must be empty");
    let by_v = count_rows(&db, "SELECT * FROM t WHERE v = 10", "post secondary").await;
    assert!(by_v.is_empty(), "secondary-index lookup must be empty");

    // schema 面不变：TableMeta 列结构完整保留（名称/类型/序）
    let meta = db.table_manager.get_table("t").await.unwrap();
    let cols: Vec<String> = meta.columns.iter().map(|(n, _)| n.clone()).collect();
    assert_eq!(cols, vec!["id", "v"], "column definitions must survive");

    // 新写入正常：PK 点查与二级索引列查询均可达（空树重建后条目可插可查）
    exec_ok(&db, "INSERT INTO t VALUES (5, 50)", "post insert").await;
    let by_id = count_rows(&db, "SELECT * FROM t WHERE id = 5", "post pk").await;
    assert_eq!(by_id.len(), 1, "PK lookup must reach the new row");
    let by_v = count_rows(&db, "SELECT * FROM t WHERE v = 50", "post secondary").await;
    assert_eq!(
        by_v.len(),
        1,
        "secondary-index lookup must reach the new row"
    );
}

/// R1-S1+：`TRUNCATE <t>`（无 TABLE 关键字）与 `TRUNCATE TABLE <t>` 等价。
#[tokio::test]
async fn truncate_without_table_keyword() {
    let (db, _dir) = open_db().await;
    seed_with_secondary(&db).await;

    exec_ok(&db, "TRUNCATE t", "truncate bare").await;
    let n = count_rows(&db, "SELECT COUNT(*) FROM t", "post").await;
    assert_eq!(n[0][0].as_i64().unwrap(), 0);
}

// ===========================================================================
// R1-S2: 物理释放与重开持久
// ===========================================================================

/// R1-S2：TRUNCATE 后显式 close → 重开：表存在且空、行不复活、定义保留。
#[tokio::test]
async fn truncate_persists_across_reopen() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        seed_with_secondary(&db).await;
        exec_ok(&db, "TRUNCATE TABLE t", "truncate").await;
        db.close().await.unwrap();
    }
    let db = Database::open(&path).await.unwrap();
    // 表存在（get_table 可达）且空
    let meta = db.table_manager.get_table("t").await.unwrap();
    let cols: Vec<String> = meta.columns.iter().map(|(n, _)| n.clone()).collect();
    assert_eq!(cols, vec!["id", "v"]);
    let n = count_rows(&db, "SELECT COUNT(*) FROM t", "reopen").await;
    assert_eq!(n[0][0].as_i64().unwrap(), 0, "rows must not resurrect");
    // 重开后新写入可用
    exec_ok(&db, "INSERT INTO t VALUES (7, 70)", "reopen insert").await;
    let by_v = count_rows(&db, "SELECT * FROM t WHERE v = 70", "reopen secondary").await;
    assert_eq!(by_v.len(), 1);
}

// ===========================================================================
// R1-S3: 显式事务内拒绝
// ===========================================================================

/// R1-S3：显式事务内执行期点名拒绝（0A000）且零副作用，事务状态不变。
#[tokio::test]
async fn truncate_inside_explicit_transaction_rejected() {
    let (db, _dir) = open_db().await;
    seed_with_secondary(&db).await;
    let tx = db.begin().await.unwrap();
    match db.execute_in_tx("TRUNCATE TABLE t", &tx).await {
        Response::Error { sqlstate, .. } => {
            assert_eq!(
                sqlstate.as_deref(),
                Some("0A000"),
                "in-tx TRUNCATE must be rejected with 0A000"
            );
        }
        other => panic!("expected in-tx rejection, got {other:?}"),
    }
    db.rollback(tx).await.unwrap();
    // 零副作用：数据不变；会话事务结束后语句正常可用
    let n = count_rows(&db, "SELECT COUNT(*) FROM t", "after reject").await;
    assert_eq!(
        n[0][0].as_i64().unwrap(),
        3,
        "rejection must be side-effect free"
    );
    exec_ok(&db, "TRUNCATE TABLE t", "truncate after tx").await;
    let n = count_rows(&db, "SELECT COUNT(*) FROM t", "after truncate").await;
    assert_eq!(n[0][0].as_i64().unwrap(), 0);
}

// ===========================================================================
// R1-S4: 多表与目标边界拒绝
// ===========================================================================

/// R1-S4：多表（解析期失败——sqlparser 0.44 单 table_name 字段，错误返回
/// 即拒绝）、缺失表与系统保留名均拒绝。
#[tokio::test]
async fn truncate_boundary_rejections() {
    let (db, _dir) = open_db().await;
    seed_with_secondary(&db).await;

    // 多表：解析期即失败（无运行时多表臂），错误返回 = 拒绝
    let resp = db.execute_sql("TRUNCATE TABLE t, other").await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "multi-table TRUNCATE must be rejected, got {resp:?}"
    );

    // 缺失表：42P01 点名
    expect_error(
        db.execute_sql("TRUNCATE nope").await,
        "missing table",
        "not found",
    );

    // 系统保留名：非用户表 → 42P01 点名
    for target in ["__tables", "__columns"] {
        expect_error(
            db.execute_sql(&format!("TRUNCATE {target}")).await,
            target,
            "not found",
        );
    }

    // Hive 分区形态：点名拒绝
    expect_error(
        db.execute_sql("TRUNCATE TABLE t PARTITION (p0)").await,
        "partition",
        "TRUNCATE",
    );

    // 拒绝全部零副作用
    let n = count_rows(&db, "SELECT COUNT(*) FROM t", "after rejects").await;
    assert_eq!(n[0][0].as_i64().unwrap(), 3);
}

// ===========================================================================
// R1-S5: 空表幂等
// ===========================================================================

/// R1-S5：空表（刚建）TRUNCATE 成功，二次 TRUNCATE 幂等成功。
#[tokio::test]
async fn truncate_empty_table_idempotent() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)", "create").await;
    exec_ok(&db, "TRUNCATE TABLE t", "first truncate").await;
    exec_ok(&db, "TRUNCATE TABLE t", "idempotent truncate").await;
    let n = count_rows(&db, "SELECT COUNT(*) FROM t", "post").await;
    assert_eq!(n[0][0].as_i64().unwrap(), 0);
    // 幂等后仍可正常写入
    exec_ok(&db, "INSERT INTO t VALUES (1, 10)", "post insert").await;
}

// ===========================================================================
// R2-S1: 崩溃恢复两态（T4-b）
// ===========================================================================

/// R2：TRUNCATE 后崩溃（flush_all + WAL shutdown + drop 不 close，ALTER 崩溃
/// 见证同型）——重开表存在且空、行不复活、二级索引空树可用（新行可查）。
#[tokio::test]
async fn truncate_consistent_across_crash_recovery() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        seed_with_secondary(&db).await;
        exec_ok(&db, "TRUNCATE TABLE t", "truncate").await;
        // 模拟语句后崩溃：DDL 无行级 WAL，checkpoint 已截断——直接落盘退出
        db.buffer_pool.flush_all().await.unwrap();
        db.wal_buffer.shutdown().await;
        drop(db);
    }
    let db = Database::open(&path).await.unwrap();
    let n = count_rows(&db, "SELECT COUNT(*) FROM t", "recovered").await;
    assert_eq!(
        n[0][0].as_i64().unwrap(),
        0,
        "truncated rows must not resurrect"
    );
    // 二级索引空树可用：重开后的新写入经 PK 与二级索引列查询均可达
    exec_ok(&db, "INSERT INTO t VALUES (9, 90)", "post-crash insert").await;
    let by_id = count_rows(&db, "SELECT * FROM t WHERE id = 9", "post-crash pk").await;
    assert_eq!(by_id.len(), 1);
    let by_v = count_rows(&db, "SELECT * FROM t WHERE v = 90", "post-crash secondary").await;
    assert_eq!(by_v.len(), 1);
}
