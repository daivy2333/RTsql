//! MS16 Iteration 000（001-replan）：INSERT 列清单映射与校验（design D7）
//!
//! change: 2026-09-12-ms16-correctness-batch / task T7
//!
//! 缺陷（BH-2，Plan Review 2026-09-12 独立探针实证；Act 探针证实错位形态）：
//! `build_insert` 将列清单装入 `InsertNode.columns` 后全链路无消费点，值按
//! 表列序位置解释——乱序清单静默错位落库、部分清单触发 `compute_tuple_size`
//! 断言 panic（tuple.rs:38，exit 101）、未知列被静默忽略（affected 1）、
//! 无清单数量不符同源 panic。
//!
//! 目标语义（spec insert-column-list-mapping）：列清单恰为表列排列时值按清
//! 单映射重排落位（键位校验作用于重排后键位值）；非法清单（未知列/重复列/
//! 数量不符）与无清单行长度不符在计划期明确拒绝（exit 3，零副作用，无
//! panic）；清单与表列序一致行为逐字节保持。
//!
//! MS24 Iteration 000 校准（change 2026-09-25-ms24-write-surface-completion）：
//! 列清单放宽为表列任意子集（R1），S3 部分清单用例按新语义重写（省略列取
//! DEFAULT/NULL）；未知列/重复列/无清单行长度拒绝意图由本文件其余用例与
//! planner_test 子集矩阵承接。
//!
//! RED 预测（修复前）：S1 行集断言失败（错位落库）、S2 无错误（affected 1）、
//! S3/S5 panic（exit 101）、S4 无错误（affected 1）、S7（重复列）无错误；
//! S6 一致锚点恒 GREEN。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use std::path::PathBuf;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

fn expect_error(resp: Response, what: &str) -> String {
    match resp {
        Response::Error { message } => message,
        other => panic!("{what} 应被拒绝为 Error，实际 {other:?}"),
    }
}

fn expect_affected(resp: Response, what: &str) {
    match resp {
        Response::AffectedRows { .. } => {}
        other => panic!("{what} 应成功，实际 {other:?}"),
    }
}

fn query_rows(resp: Response, what: &str) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("{what} 应返回 QueryResult，实际 {other:?}"),
    }
}

async fn assert_count(db: &Database, sql: &str, expected: serde_json::Value, what: &str) {
    match db.execute_sql(sql).await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], expected, "{what}");
        }
        other => panic!("{what} 应返回 QueryResult，实际 {other:?}"),
    }
}

async fn setup_p(db: &Database) {
    expect_affected(
        db.execute_sql("CREATE TABLE p (id INT PRIMARY KEY, v INT)")
            .await,
        "建表",
    );
}

/// R6-S1：乱序清单值按映射落位——`(v, id) VALUES (1, 2)` 落库 `(2, 1)`。
///
/// RED（修复前实测）：列清单被忽略，值按表列序错位落库为 `(1, 2)`。
#[tokio::test]
async fn out_of_order_list_maps_values_to_named_columns() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup_p(&db).await;

    expect_affected(
        db.execute_sql("INSERT INTO p (v, id) VALUES (1, 2)").await,
        "乱序清单 INSERT",
    );

    let rows = query_rows(db.execute_sql("SELECT id, v FROM p").await, "行集查询");
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(2), serde_json::json!(1)]],
        "值必须按列清单映射落位（id=2、v=1），实际: {rows:?}"
    );

    db.wal_buffer.shutdown().await;
}

/// R6-S2：乱序清单键位越界被键位类型校验拒绝——`(v, id) VALUES (1, 5.0)`
/// 重排后 id 收 5.0（Float），`KeyTypeMismatch` 点名键列，零副作用。
///
/// RED（修复前实测）：5.0 错位落非键列 v、id 收 1，静默成功 affected 1。
#[tokio::test]
async fn out_of_order_list_key_violation_rejected() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup_p(&db).await;

    let message = expect_error(
        db.execute_sql("INSERT INTO p (v, id) VALUES (1, 5.0)")
            .await,
        "乱序清单键位越界 INSERT",
    );
    assert!(
        message.contains("key column 'id'") && message.contains("INT"),
        "错误文案必须点名键列与 INT 期望，实际: {message}"
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM p",
        serde_json::json!(0),
        "被拒绝的 INSERT 不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R6-S3 → MS24 校准：部分清单在子集语义下合法——省略列（无声明 DEFAULT）
/// 取 NULL，值按清单映射落位，无 panic。原防回归意图（计划期明确拒绝、
/// 无 panic）由本文件未知列/重复列/无清单行长度用例与 planner_test 子集
/// 矩阵承接；本用例转为见证子集合法语义与映射正确性。
#[tokio::test]
async fn partial_list_inserts_omitted_columns_no_panic() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup_p(&db).await;

    expect_affected(
        db.execute_sql("INSERT INTO p (v) VALUES (9)").await,
        "部分清单 INSERT（子集语义）",
    );

    // 省略列 id 取 NULL → 无键行（既有 NULL 键语义落库）；v=9 经清单映射落位
    assert_count(
        &db,
        "SELECT COUNT(*) FROM p WHERE v = 9",
        serde_json::json!(1),
        "子集 INSERT 行必须落库",
    )
    .await;
    match db.execute_sql("SELECT id, v FROM p").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(null), serde_json::json!(9)]],
                "省略位必须填 NULL、显式位按清单落位，实际: {rows:?}"
            );
        }
        other => panic!("Expected QueryResult，实际 {other:?}"),
    }

    db.wal_buffer.shutdown().await;
}

/// R6-S4：未知列计划期拒绝——`(id, zz) VALUES (7, 1)` 报点名错误，
/// 不静默忽略未知列。
///
/// RED（修复前实测）：未知列被忽略、affected 1 静默成功。
#[tokio::test]
async fn unknown_column_rejected_at_plan_time() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup_p(&db).await;

    let message = expect_error(
        db.execute_sql("INSERT INTO p (id, zz) VALUES (7, 1)").await,
        "未知列 INSERT",
    );
    assert!(
        message.contains("zz") && message.contains("not found"),
        "错误文案必须点名未知列，实际: {message}"
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM p",
        serde_json::json!(0),
        "被拒绝的 INSERT 不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R6-S5：无清单但值数不符计划期拒绝（panic 消除）——`VALUES (1)` 报
/// 数量不符明确错误。
///
/// RED（修复前实测）：同源 panic（tuple.rs:38，exit 101）。
#[tokio::test]
async fn no_list_row_length_mismatch_rejected_at_plan_time() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup_p(&db).await;

    let message = expect_error(
        db.execute_sql("INSERT INTO p VALUES (1)").await,
        "无清单数量不符 INSERT",
    );
    assert!(
        message.contains("expects 2") && message.contains("got 1"),
        "错误文案必须点名期望与实际值数，实际: {message}"
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM p",
        serde_json::json!(0),
        "被拒绝的 INSERT 不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R6-S6 锚点：清单与表列序一致行为保持——`(id, v) VALUES (5, 100)` 落库
/// `(5, 100)`，键位点查可达（变更前后恒 GREEN）。
#[tokio::test]
async fn in_order_list_behavior_unchanged() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup_p(&db).await;

    expect_affected(
        db.execute_sql("INSERT INTO p (id, v) VALUES (5, 100)")
            .await,
        "一致清单 INSERT",
    );

    let rows = query_rows(
        db.execute_sql("SELECT id, v FROM p WHERE id = 5").await,
        "键位点查",
    );
    assert_eq!(
        rows,
        vec![vec![serde_json::json!(5), serde_json::json!(100)]],
        "一致清单行为必须保持"
    );

    db.wal_buffer.shutdown().await;
}

/// R6 Requirement SHALL（重复列拒绝，编号场景之外的需求面覆盖）：清单
/// 含重复列 `(id, id)` 在计划期点名拒绝。
///
/// RED（修复前预测）：重复列被忽略，值按表列序落库 affected 1。
#[tokio::test]
async fn duplicate_column_in_list_rejected_at_plan_time() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup_p(&db).await;

    let message = expect_error(
        db.execute_sql("INSERT INTO p (id, id) VALUES (1, 2)").await,
        "重复列 INSERT",
    );
    assert!(
        message.contains("Duplicate") && message.contains("id"),
        "错误文案必须点名重复列，实际: {message}"
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM p",
        serde_json::json!(0),
        "被拒绝的 INSERT 不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}
