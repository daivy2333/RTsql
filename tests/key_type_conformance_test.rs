//! MS16 Iteration 000：Int 键列写入类型强制（design D3）
//!
//! change: 2026-09-12-ms16-correctness-batch / tasks T3-T4
//!
//! 缺陷（2026-09-12 探针新发现，根因收口并入本 change、不登记 improvement）：
//! INSERT/UPDATE 不校验值类型与列声明类型——Float 值写入 Int 键列静默落库
//! （序列化按值打 tag），该行键位为 Float、成为无键行，键位等值点查静默
//! 空集（I046 同族漏行，方向 B 路由修复修不到存储值面）。
//!
//! 强制语义（design D3）：键列声明类型 Int 且键位值非 Int/NULL 时，在任何
//! 写入（数据页/WAL/索引）之前以新增加性错误 `StorageError::KeyTypeMismatch`
//! 拒绝（exit 3，零副作用）；INSERT 校验先于 DuplicateKey 预检（非法类型
//! 无需访问索引），UPDATE 校验位于 Step 1 之后（KeyNotFound 优先保持）。
//! 非 Int 键列不新增拒绝（Float 键列收 Int 值等宽松行为保持）；NULL 保持
//! 既有无键行语义，`UPDATE SET id = NULL` 继续 I037 删旧键分支。

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

async fn assert_count(db: &Database, sql: &str, expected: serde_json::Value, what: &str) {
    match db.execute_sql(sql).await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], expected, "{what}");
        }
        other => panic!("{what} 应返回 QueryResult，实际 {other:?}"),
    }
}

/// R3-S1：Int 键列 INSERT Float 值被拒绝，零副作用。
///
/// RED（修复前实测）：`INSERT INTO t VALUES (5.0, 1)` 静默落库（探针实证
/// 行 (5.0, 1)，键位 Float 成无键行）。
#[tokio::test]
async fn insert_int_key_rejects_float_value_with_zero_side_effects() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
            .await,
        "建表",
    );

    let message = expect_error(
        db.execute_sql("INSERT INTO t VALUES (5.0, 1)").await,
        "Int 键列 INSERT Float 值",
    );
    assert!(
        message.contains("key column 'id'") && message.contains("INT"),
        "错误文案必须点名键列与 INT 期望，实际: {message}"
    );

    // 零副作用：拒绝前无数据页/WAL/索引写入
    assert_count(
        &db,
        "SELECT COUNT(*) FROM t",
        serde_json::json!(0),
        "被拒绝的 INSERT 不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R3-S2：Int 键列 INSERT String/Bool 值同样被拒绝（拒绝矩阵补全），零副作用。
///
/// RED（修复前预测）：String/Bool 值与 Float 同机制静默落库为无键行。
#[tokio::test]
async fn insert_int_key_rejects_string_and_bool_values() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
            .await,
        "建表",
    );

    expect_error(
        db.execute_sql("INSERT INTO t VALUES ('x', 1)").await,
        "Int 键列 INSERT String 值",
    );
    expect_error(
        db.execute_sql("INSERT INTO t VALUES (true, 1)").await,
        "Int 键列 INSERT Bool 值",
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM t",
        serde_json::json!(0),
        "被拒绝的 INSERT 不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R3-S3：UPDATE SET 键列为 Float 被拒绝，原行零副作用。
///
/// RED（修复前实测）：`UPDATE t SET id = 5.0 WHERE id = 5` 成功走 I037
/// 分支，行静默转无键。
#[tokio::test]
async fn update_int_key_rejects_float_value_with_zero_side_effects() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
            .await,
        "建表",
    );
    expect_affected(
        db.execute_sql("INSERT INTO t VALUES (5, 100)").await,
        "首次 INSERT",
    );

    let message = expect_error(
        db.execute_sql("UPDATE t SET id = 5.0 WHERE id = 5").await,
        "UPDATE SET 键列为 Float",
    );
    assert!(
        message.contains("key column 'id'") && message.contains("INT"),
        "错误文案必须点名键列与 INT 期望，实际: {message}"
    );

    // 零副作用：原行 (5, 100) 经点查逐字节保持
    match db.execute_sql("SELECT * FROM t WHERE id = 5").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(5), serde_json::json!(100)]],
                "被拒绝的 UPDATE 不得改动原行"
            );
        }
        other => panic!("Expected QueryResult，实际 {other:?}"),
    }
    assert_count(
        &db,
        "SELECT COUNT(*) FROM t",
        serde_json::json!(1),
        "被拒绝的 UPDATE 不得增删行",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R3-S4：UPDATE SET 键列为 String 被拒绝（拒绝矩阵补全）。
///
/// RED（修复前预测）：String 值同机制被接受、行静默转无键。
#[tokio::test]
async fn update_int_key_rejects_string_value() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
            .await,
        "建表",
    );
    expect_affected(
        db.execute_sql("INSERT INTO t VALUES (5, 100)").await,
        "首次 INSERT",
    );

    expect_error(
        db.execute_sql("UPDATE t SET id = 'x' WHERE id = 5").await,
        "UPDATE SET 键列为 String",
    );

    match db.execute_sql("SELECT * FROM t WHERE id = 5").await {
        Response::QueryResult { rows } => {
            assert_eq!(
                rows,
                vec![vec![serde_json::json!(5), serde_json::json!(100)]],
                "被拒绝的 UPDATE 不得改动原行"
            );
        }
        other => panic!("Expected QueryResult，实际 {other:?}"),
    }

    db.wal_buffer.shutdown().await;
}

/// R3-S5 锚点：键位 INSERT NULL 保持既有无键行落库语义（变更前后恒 GREEN）。
#[tokio::test]
async fn insert_null_key_stays_accepted_keyless() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
            .await,
        "建表",
    );
    expect_affected(
        db.execute_sql("INSERT INTO t VALUES (NULL, 1)").await,
        "键位 NULL INSERT",
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM t",
        serde_json::json!(1),
        "键位 NULL 行必须落库",
    )
    .await;
    assert_count(
        &db,
        "SELECT COUNT(*) FROM t WHERE v = 1",
        serde_json::json!(1),
        "无键行必须经非键谓词可达",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R3-S6 锚点：UPDATE SET 键列为 NULL 保持 I037 接受语义（变更前后恒
/// GREEN；删旧键条目细节由 update_index_maintenance_test 锁定）。
#[tokio::test]
async fn update_set_key_null_stays_accepted() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
            .await,
        "建表",
    );
    expect_affected(
        db.execute_sql("INSERT INTO t VALUES (5, 100)").await,
        "首次 INSERT",
    );
    expect_affected(
        db.execute_sql("UPDATE t SET id = NULL WHERE id = 5").await,
        "UPDATE SET 键列为 NULL",
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM t WHERE v = 100",
        serde_json::json!(1),
        "键位置 NULL 后行必须经非键谓词可达",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R3-S7 锚点：非 Int 键列不新增拒绝——Float 键列收 Int 值保持既有宽松
/// 行为（变更前后恒 GREEN，spec 行为保持锚点）。
#[tokio::test]
async fn float_key_column_keeps_accepting_int_value() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE ft (f FLOAT, n INT)").await,
        "建表",
    );
    expect_affected(
        db.execute_sql("INSERT INTO ft VALUES (5, 1)").await,
        "Float 键列 INSERT Int 值",
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM ft WHERE n = 1",
        serde_json::json!(1),
        "Float 键列收 Int 值必须保持落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R3-S8：显式列序键位越界被拒绝——列清单 `(v, id)` 重排后 id 收 5.0
/// （Float），`KeyTypeMismatch` 点名键列，零副作用。
///
/// 依赖 T7 列清单映射（insert-column-list-mapping，001-replan）：修复前
/// 列清单无消费点，5.0 错位落非键列 v、id 收 1 静默成功（BH-2，场景因此
/// 在 000-initial 未写入——本场景见证「映射后键位校验」组合语义，T7 后
/// 直接 GREEN，写 RED 无意义）。
#[tokio::test]
async fn explicit_column_list_key_violation_rejected() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
            .await,
        "建表",
    );

    let message = expect_error(
        db.execute_sql("INSERT INTO t (v, id) VALUES (1, 5.0)")
            .await,
        "显式列序键位越界 INSERT",
    );
    assert!(
        message.contains("key column 'id'") && message.contains("INT"),
        "错误文案必须点名键列与 INT 期望，实际: {message}"
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM t",
        serde_json::json!(0),
        "被拒绝的 INSERT 不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}
