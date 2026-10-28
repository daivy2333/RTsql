//! change 2026-10-03-practical-core-gaps T9：UPDATE SET 值表达式与多列赋值
//! 执行语义——逐命中行旧行快照求值全部表达式右值（S14 同快照语义）；门序
//! 保持（NOT NULL → 键列类型门 → coerce → 唯一碰撞预检 → 类型门 FLOAT 升
//! 格）；PK rekey 三分支 / 唯一条目 / 次级索引既有维护路径；语句失败零副
//! 作用（预检前置，隐式事务回滚既有语义）。
//!
//! 对照 proposal 场景：S12–S17。

#![cfg(feature = "network")]
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use serde_json::json;
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("test")
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
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

fn error_msg(resp: Response) -> String {
    match resp {
        Response::Error { message, .. } => message,
        other => panic!("Expected Error, got {other:?}"),
    }
}

fn affected(resp: Response) -> u64 {
    match resp {
        Response::AffectedRows { count } => count,
        other => panic!("Expected AffectedRows, got {other:?}"),
    }
}

/// 夹具：t(id PK, v INT, name STRING(20), f FLOAT)——两行。
async fn seed_t(dir: &TempDir) -> Database {
    let db = Database::open(&db_path(dir)).await.unwrap();
    exec_ok(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, v INT, name STRING(20), f FLOAT)",
    )
    .await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 10, 'ada', 0.5)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 20, 'bob', 1.5)").await;
    db
}

// ===========================================================================
// 字面量基线（T8 后已可达——既有单字面量形态逐字节）
// ===========================================================================

/// 基线：单字面量 UPDATE 行为与 T8 前逐字节一致。
#[tokio::test]
async fn literal_baseline_preserved() {
    let dir = tempdir().unwrap();
    let db = seed_t(&dir).await;
    assert_eq!(
        affected(to_response(
            db.execute_sql("UPDATE t SET v = 99 WHERE id = 1").await
        )),
        1
    );
    let rows = query_rows(to_response(
        db.execute_sql("SELECT id, v, name, f FROM t ORDER BY id")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!(99), json!("ada"), json!(0.5)],
            vec![json!(2), json!(20), json!("bob"), json!(1.5)]
        ]
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// ===========================================================================
// 表达式右值（S12/S13）
// ===========================================================================

/// S12：列引用表达式——`SET v = v + 1` 按旧行求值；仅命中行变更。
#[tokio::test]
async fn column_reference_expression() {
    let dir = tempdir().unwrap();
    let db = seed_t(&dir).await;
    assert_eq!(
        affected(to_response(
            db.execute_sql("UPDATE t SET v = v + 1 WHERE id = 1").await
        )),
        1
    );
    let rows = query_rows(to_response(
        db.execute_sql("SELECT id, v FROM t ORDER BY id").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1), json!(11)], vec![json!(2), json!(20)]],
        "v = v + 1 必须按旧行求值且仅命中行变更"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S13：常量 / 标量函数 / CAST 右值逐行求值生效。
#[tokio::test]
async fn constant_function_cast_right_values() {
    let dir = tempdir().unwrap();
    let db = seed_t(&dir).await;
    exec_ok(&db, "UPDATE t SET v = 1 + 2 WHERE id = 1").await;
    exec_ok(&db, "UPDATE t SET name = upper('ada') WHERE id = 1").await;
    exec_ok(&db, "UPDATE t SET v = CAST('42' AS INTEGER) WHERE id = 2").await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT id, v, name FROM t ORDER BY id")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!(3), json!("ADA")],
            vec![json!(2), json!(42), json!("bob")]
        ],
        "常量/函数/CAST 右值必须逐行生效"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// ===========================================================================
// 多列同快照（S14）
// ===========================================================================

/// S14：多列混合赋值全部基于同一旧行快照（`b = a` 取旧 a，左列先行不影响
/// 右列）。
#[tokio::test]
async fn multi_column_same_row_snapshot() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t2 (id INT PRIMARY KEY, a INT, b INT)").await;
    exec_ok(&db, "INSERT INTO t2 VALUES (1, 5, 50)").await;

    exec_ok(&db, "UPDATE t2 SET a = a + 1, b = a WHERE id = 1").await;
    let rows = query_rows(to_response(db.execute_sql("SELECT a, b FROM t2").await));
    assert_eq!(
        rows,
        vec![vec![json!(6), json!(5)]],
        "多列赋值必须基于同一旧行快照（b = 旧 a = 5）"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// 多列语句的任一列门序失败 → 全语句零副作用（第一列的变更不落盘）。
#[tokio::test]
async fn multi_column_gate_failure_zero_side_effect() {
    let dir = tempdir().unwrap();
    let db = seed_t(&dir).await;
    // 第二列 name(STRING) 收 Int 表达式结果 → ColumnTypeMismatch，全语句失败。
    let msg = error_msg(to_response(
        db.execute_sql("UPDATE t SET v = v + 1, name = v WHERE id = 1")
            .await,
    ));
    assert!(
        msg.contains("v") || msg.contains("name"),
        "类型门必须点名失败列，实际: {msg}"
    );
    let rows = query_rows(to_response(
        db.execute_sql("SELECT id, v, name FROM t WHERE id = 1")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1), json!(10), json!("ada")]],
        "多列语句任一列失败必须全语句零副作用"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// ===========================================================================
// 门序保持（S15）
// ===========================================================================

/// S15：FLOAT 列 Int 表达式结果无损升格；日期族 coerce 生效。
#[tokio::test]
async fn float_upgrade_and_date_coerce() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE m (id INT PRIMARY KEY, f FLOAT, d DATE)").await;
    exec_ok(&db, "INSERT INTO m VALUES (1, 0.5, '2024-01-01')").await;

    // v 列引用 Int 值进 FLOAT 列 → 升格。
    exec_ok(&db, "UPDATE m SET f = 2 WHERE id = 1").await;
    // 标量函数 String 结果进 DATE 列 → coerce 强制解析。
    exec_ok(&db, "UPDATE m SET d = trim(' 2024-01-15 ') WHERE id = 1").await;
    let rows = query_rows(to_response(db.execute_sql("SELECT f, d FROM m").await));
    assert_eq!(
        rows,
        vec![vec![json!(2.0), json!("2024-01-15")]],
        "FLOAT 升格与日期 coerce 必须在表达式结果上生效"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S15：表达式结果为 NULL 落 NOT NULL 列 → 零副作用拒绝。
#[tokio::test]
async fn not_null_enforced_on_expression_result() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE n (id INT PRIMARY KEY, x INT NOT NULL)").await;
    exec_ok(&db, "INSERT INTO n VALUES (1, 7)").await;

    let msg = error_msg(to_response(
        db.execute_sql("UPDATE n SET x = COALESCE(NULL, NULL) WHERE id = 1")
            .await,
    ));
    assert!(
        msg.contains("NOT NULL"),
        "NOT NULL 必须对表达式结果强制，实际: {msg}"
    );
    let rows = query_rows(to_response(db.execute_sql("SELECT x FROM n").await));
    assert_eq!(rows, vec![vec![json!(7)]], "NOT NULL 拒绝必须零副作用");
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S15：表达式结果跨类型 → 写入类型门点名拒绝零副作用。
#[tokio::test]
async fn type_gate_rejects_cross_type_result() {
    let dir = tempdir().unwrap();
    let db = seed_t(&dir).await;
    let msg = error_msg(to_response(
        db.execute_sql("UPDATE t SET v = name WHERE id = 1").await,
    ));
    assert!(
        msg.contains("INT") && msg.contains("String"),
        "类型门必须点名期望与实际类型，实际: {msg}"
    );
    let rows = query_rows(to_response(
        db.execute_sql("SELECT v FROM t WHERE id = 1").await,
    ));
    assert_eq!(rows, vec![vec![json!(10)]], "类型门拒绝必须零副作用");
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// ===========================================================================
// 键列表达式 rekey 与碰撞（S16）
// ===========================================================================

/// S16：键列表达式 rekey——删旧键插新键，行在新键位可达。
#[tokio::test]
async fn key_column_expression_rekey() {
    let dir = tempdir().unwrap();
    let db = seed_t(&dir).await;
    assert_eq!(
        affected(to_response(
            db.execute_sql("UPDATE t SET id = id + 100 WHERE id = 1")
                .await
        )),
        1
    );
    let rows = query_rows(to_response(
        db.execute_sql("SELECT id, v FROM t WHERE id = 101").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(101), json!(10)]],
        "rekey 后行在新键位可达"
    );
    let rows = query_rows(to_response(
        db.execute_sql("SELECT id FROM t WHERE id = 1").await,
    ));
    assert!(rows.is_empty(), "旧键位不再可达");
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S16：rekey 碰撞预检——新键命中既有行 → DuplicateKey 零副作用。
#[tokio::test]
async fn key_column_rekey_collision_zero_side_effect() {
    let dir = tempdir().unwrap();
    let db = seed_t(&dir).await;
    let msg = error_msg(to_response(
        db.execute_sql("UPDATE t SET id = id + 1 WHERE id = 1")
            .await,
    ));
    assert!(
        msg.contains("Duplicate") || msg.contains("duplicate"),
        "rekey 碰撞必须拒绝，实际: {msg}"
    );
    let rows = query_rows(to_response(
        db.execute_sql("SELECT id, v FROM t ORDER BY id").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1), json!(10)], vec![json!(2), json!(20)]],
        "rekey 碰撞拒绝必须零副作用"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S16：键列表达式产生非 Int 结果 → KeyTypeMismatch 零副作用。
#[tokio::test]
async fn key_column_non_int_result_rejected() {
    let dir = tempdir().unwrap();
    let db = seed_t(&dir).await;
    let msg = error_msg(to_response(
        db.execute_sql("UPDATE t SET id = f WHERE id = 1").await,
    ));
    assert!(
        msg.contains("KeyTypeMismatch") || msg.contains("INT"),
        "键列类型门必须对表达式结果强制，实际: {msg}"
    );
    let rows = query_rows(to_response(
        db.execute_sql("SELECT id FROM t WHERE id = 1").await,
    ));
    assert_eq!(rows, vec![vec![json!(1)]], "键列类型门拒绝必须零副作用");
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// ===========================================================================
// 唯一列与次级索引（R3 索引维护面）
// ===========================================================================

/// 唯一列表达式更新：碰撞预检零副作用；正常更新条目迁移。
#[tokio::test]
async fn unique_column_expression_update() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE u (id INT PRIMARY KEY, code INT UNIQUE)").await;
    exec_ok(&db, "INSERT INTO u VALUES (1, 100)").await;
    exec_ok(&db, "INSERT INTO u VALUES (2, 200)").await;

    let msg = error_msg(to_response(
        db.execute_sql("UPDATE u SET code = code * 2 WHERE id = 1")
            .await,
    ));
    assert!(
        msg.contains("Duplicate") || msg.contains("duplicate"),
        "唯一列碰撞必须拒绝，实际: {msg}"
    );
    let rows = query_rows(to_response(
        db.execute_sql("SELECT code FROM u WHERE id = 1").await,
    ));
    assert_eq!(rows, vec![vec![json!(100)]], "唯一列碰撞拒绝必须零副作用");

    exec_ok(&db, "UPDATE u SET code = code + 1 WHERE id = 1").await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT code FROM u WHERE id = 1").await,
    ));
    assert_eq!(rows, vec![vec![json!(101)]], "唯一列表达式更新生效");
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// 次级索引条目随表达式更新迁移（等值路由可达新值、不可达旧值）。
#[tokio::test]
async fn secondary_index_maintained_on_expression_update() {
    let dir = tempdir().unwrap();
    let db = seed_t(&dir).await;
    exec_ok(&db, "CREATE INDEX idx_v ON t(v)").await;
    exec_ok(&db, "UPDATE t SET v = v + 1 WHERE id = 1").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT id FROM t WHERE v = 11").await,
    ));
    assert_eq!(rows, vec![vec![json!(1)]], "次级索引必须指向更新后的行");
    let rows = query_rows(to_response(
        db.execute_sql("SELECT id FROM t WHERE v = 10").await,
    ));
    assert!(rows.is_empty(), "次级索引旧键条目必须清除");
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// ===========================================================================
// 持久两态
// ===========================================================================

/// 表达式更新经 close 重开持久一致。
#[tokio::test]
async fn expression_update_persists_across_restart() {
    let dir = tempdir().unwrap();
    let db = seed_t(&dir).await;
    exec_ok(&db, "UPDATE t SET v = v * 2 WHERE id = 1").await;
    // 重开路径须走显式 close（checkpoint 落盘 catalog 脏页）——DDL 不写
    // WAL，wal_buffer.shutdown() 只刷 WAL，重开恢复会撞 T11 的缺失表形态。
    db.close().await.unwrap();
    drop(db);

    let db = Database::open(&db_path(&dir)).await.unwrap();
    let rows = query_rows(to_response(
        db.execute_sql("SELECT id, v FROM t WHERE id = 1").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1), json!(20)]],
        "表达式更新必须跨重启持久"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}
