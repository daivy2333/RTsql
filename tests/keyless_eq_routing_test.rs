//! MS15-T01 Iteration 000：键位等值路由对无键行可达（I036）
//!
//! change: 2026-09-12-ms15-t01-keyless-eq-routing
//!
//! 病灶（I036，MS10-T05 001-rework 后遗留）：键位等值腿含不可键控字面量
//! （String/Float/Bool/NULL，`to_key()==None`）时，`extract_pk_from_where`
//! 返回 None、`has_pk_equality` 结构判定为真 → `Filter(Scan)` 索引遍历；
//! 无键行（落库不入索引）不可达，静默空集 exit 0。
//!
//! 修复语义（I036 方向 A）：不可键控字面量腿禁用索引路由，回退数据页行内
//! 求值（简单/AND → 谓词下推 `DataScan`；含 OR → `Filter(DataScan)`）；
//! 可键控 Int 字面量的既有路由形状（`IndexScan` / `Filter(Scan)`）逐字节
//! 不变（由 `pushdown_test` 两 PK 形状用例锁定，本文件 R3-S1 锁结果）。

use rtsql::database::Database;
use rtsql::executor::PhysicalPlan;
use rtsql::network::protocol::Response;
use rtsql::pipeline::{parse_stage, plan_stage};
use serde_json::json;
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("test")
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = db.execute_sql(sql).await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "setup statement failed: {sql:?} -> {resp:?}"
    );
}

fn query_rows(resp: Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("Expected QueryResult, got {other:?}"),
    }
}

async fn plan_of(db: &Database, sql: &str) -> PhysicalPlan {
    let stmts = parse_stage(sql).await.expect("parse should succeed");
    let stmt = stmts.first().expect("one statement");
    plan_stage(db, sql, stmt, false)
        .await
        .expect("plan should succeed")
}

/// R1-S1：String 隐式键列简单等值必须命中无键行。
///
/// RED（修复前实测）：`Filter(Scan)` 索引遍历，无键行不入索引 → 空集。
#[tokio::test]
async fn string_implicit_key_simple_equality_reaches_keyless_rows() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE t1 (s STRING, n INT)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES ('x', 1)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES ('y', 2)").await;

    let rows = query_rows(db.execute_sql("SELECT * FROM t1 WHERE s = 'x'").await);
    assert_eq!(
        rows,
        vec![vec![json!("x"), json!(1)]],
        "键位 String 字面量等值必须经数据页求值命中无键行"
    );

    db.wal_buffer.shutdown().await;
}

/// R1-S2：String 隐式键列 AND 组合等值必须命中无键行。
///
/// RED（修复前实测）：AND 内键位等值腿触发 has_pk_eq 分支 → `Filter(Scan)`
/// 空集。
#[tokio::test]
async fn string_implicit_key_and_combined_equality_reaches_keyless_rows() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE t1 (s STRING, n INT)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES ('x', 1)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES ('y', 2)").await;

    let rows = query_rows(
        db.execute_sql("SELECT * FROM t1 WHERE s = 'x' AND n = 1")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!("x"), json!(1)]],
        "AND 组合的键位 String 字面量等值必须命中无键行"
    );

    db.wal_buffer.shutdown().await;
}

/// R1-S3：Float 隐式键列 Float 字面量等值必须命中无键行。
///
/// RED（修复前实测）：Float 字面量 `to_key()` None → `Filter(Scan)` 空集
/// （5.0=5.0 行内本应相等）。
#[tokio::test]
async fn float_key_float_literal_equality_reaches_keyless_rows() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE t2 (f FLOAT, n INT)").await;
    exec_ok(&db, "INSERT INTO t2 VALUES (5.0, 1)").await;

    let rows = query_rows(db.execute_sql("SELECT * FROM t2 WHERE f = 5.0").await);
    assert_eq!(
        rows,
        vec![vec![json!(5.0), json!(1)]],
        "Float 键位 Float 字面量等值必须命中无键行"
    );

    db.wal_buffer.shutdown().await;
}

/// R1-S4：显式声明 TEXT PRIMARY KEY 的等值必须命中无键行。
///
/// RED（修复前实测）：声明主键不改变字面量可键控性 → `Filter(Scan)` 空集。
#[tokio::test]
async fn text_primary_key_equality_reaches_keyless_rows() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE t3 (s TEXT PRIMARY KEY, n INT)").await;
    exec_ok(&db, "INSERT INTO t3 VALUES ('x', 1)").await;

    let rows = query_rows(db.execute_sql("SELECT * FROM t3 WHERE s = 'x'").await);
    assert_eq!(
        rows,
        vec![vec![json!("x"), json!(1)]],
        "TEXT PRIMARY KEY 声明键列的 String 字面量等值必须命中无键行"
    );

    db.wal_buffer.shutdown().await;
}

/// R1 修复形态 plan 形状：简单非键控等值下推进 `DataScan` 行内求值。
///
/// RED（修复前实测）：实际为 `Filter(Scan)`（has_pk_eq 分支）。
#[tokio::test]
async fn simple_non_keyable_equality_plan_is_data_scan_with_predicate() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE t1 (s STRING, n INT)").await;

    let plan = plan_of(&db, "SELECT s FROM t1 WHERE s = 'x'").await;
    match plan {
        PhysicalPlan::DataScan(node) => {
            assert!(
                node.predicate.is_some(),
                "非键控字面量腿必须作为谓词下推进 DataScan 行内求值"
            );
        }
        other => panic!("简单非键控等值应为谓词下推 DataScan，实际 {other:?}"),
    }

    db.wal_buffer.shutdown().await;
}

/// R1 修复形态 plan 形状：非键控等值 + OR 保持 `Filter(DataScan)` 包装。
///
/// 修复前即 GREEN（`has_pk_equality` 对 OR 保守 false，走既有 contains_or
/// 臂）；本用例守卫 T2 分类收窄不得干扰 OR 臂路由。
#[tokio::test]
async fn non_keyable_equality_with_or_keeps_filter_over_data_scan() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE t1 (s STRING, n INT)").await;

    let plan = plan_of(&db, "SELECT s FROM t1 WHERE s = 'x' OR n = 2").await;
    match plan {
        PhysicalPlan::Filter(node) => match node.input.as_ref() {
            PhysicalPlan::DataScan(inner) => {
                assert!(inner.predicate.is_none(), "OR 谓词不得下推进 DataScan");
            }
            other => panic!("Expected Filter over DataScan, got {other:?}"),
        },
        other => panic!("非键控等值 + OR 应保持 Filter(DataScan)，实际 {other:?}"),
    }

    db.wal_buffer.shutdown().await;
}

/// R3-S1：Int 键列 + 非 Int 字面量的查询结果保持空集（变更前后 GREEN，
/// 锁结果不锁路径）。
///
/// 修复前：`Filter(Scan)` 索引遍历，行内 Int=Text 跨类型 equals false → 空集；
/// 修复后：谓词下推 `DataScan`，同一求值语义 → 空集。可观察结果逐字节一致。
#[tokio::test]
async fn int_key_non_int_literal_still_empty_result() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 10)").await;

    let rows = query_rows(db.execute_sql("SELECT * FROM t WHERE id = 'abc'").await);
    assert!(
        rows.is_empty(),
        "Int 键列 + String 字面量等值必须保持空集（跨类型 equals false）"
    );

    db.wal_buffer.shutdown().await;
}

/// R1-S5：restart（WAL 恢复 + 索引重建）后键位等值可达性保持。
///
/// 流程参照 keyless_row_test.rs 夹具先例：DDL 无 WAL 记录，catalog 经
/// buffer_pool 落盘；写入后 shutdown + drop，重开验证 Scenario 1 行集。
#[tokio::test]
async fn keyless_equality_reachability_survives_restart() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();

    exec_ok(&db, "CREATE TABLE t1 (s STRING, n INT)").await;
    db.buffer_pool.flush_all().await.unwrap();
    exec_ok(&db, "INSERT INTO t1 VALUES ('x', 1)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES ('y', 2)").await;

    db.wal_buffer.shutdown().await;
    drop(db);

    let db2 = Database::open(&path).await.unwrap();
    let rows = query_rows(db2.execute_sql("SELECT * FROM t1 WHERE s = 'x'").await);
    assert_eq!(
        rows,
        vec![vec![json!("x"), json!(1)]],
        "restart 后键位 String 字面量等值必须保持可达"
    );

    db2.wal_buffer.shutdown().await;
}
