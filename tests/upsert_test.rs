//! MS24 Iteration 001 — UPSERT 与 REPLACE INTO 执行面
//!
//! change: 2026-09-25-ms24-write-surface-completion
//!
//! 判定全部走 lib API（`Database::open` + `execute_sql` → `Response`），
//! 与 `constraint_enforcement_test` 同模式；受影响计数按 SQLite 语义断言。
//! 恢复两态与显式事务面在文件末段（2.5）。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use std::path::PathBuf;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

async fn expect_error(db: &Database, sql: &str, needle: &str) {
    match db.execute_sql(sql).await {
        Response::Error { message } => assert!(
            message.contains(needle),
            "expected error containing {needle:?}, got: {message}"
        ),
        other => panic!("Expected Error containing {needle:?}, got {:?}", other),
    }
}

async fn expect_affected(db: &Database, sql: &str, count: u64) {
    match db.execute_sql(sql).await {
        Response::AffectedRows { count: c } => assert_eq!(c, count, "{sql}"),
        other => panic!("Expected AffectedRows({count}) for {sql}, got {:?}", other),
    }
}

async fn expect_count(db: &Database, table: &str, count: usize) {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    match db.execute_sql(&sql).await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(count), "{sql}");
        }
        other => panic!("Expected QueryResult for {sql}, got {:?}", other),
    }
}

async fn expect_where_count(db: &Database, sql: &str, count: usize) {
    match db.execute_sql(sql).await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(count), "{sql}");
        }
        other => panic!("Expected QueryResult for {sql}, got {:?}", other),
    }
}

async fn expect_cell(db: &Database, sql: &str, column: usize, expected: serde_json::Value) {
    match db.execute_sql(sql).await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][column], expected, "{sql}");
        }
        other => panic!("Expected QueryResult for {sql}, got {:?}", other),
    }
}

/// 建夹具表：id INT PK / name VARCHAR（DEFAULT 'anon'）/ code INT UNIQUE。
async fn setup(db: &Database) {
    db.execute_sql(
        "CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR(100) DEFAULT 'anon', code INT UNIQUE)",
    )
    .await;
    db.checkpoint().await.unwrap();
}

// ===========================================================================
// 2.2 — 仲裁搜索、无冲突路径与 DO NOTHING
// ===========================================================================

/// 无冲突行插入与普通 INSERT 逐字节等价（含子集清单 + DEFAULT 填充）。
#[tokio::test]
async fn upsert_without_conflict_inserts_like_plain_insert() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(
        &db,
        "INSERT INTO users (id, name, code) VALUES (1, 'Alice', 100) ON CONFLICT DO NOTHING",
        1,
    )
    .await;
    expect_affected(
        &db,
        "INSERT INTO users (id, code) VALUES (2, 200) ON CONFLICT DO NOTHING",
        1,
    )
    .await;

    // 省略 name → 声明 DEFAULT 'anon'（Iteration 000 通道经 upsert 路径消费）
    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 2",
        0,
        serde_json::json!("anon"),
    )
    .await;
    expect_count(&db, "users", 2).await;

    db.wal_buffer.shutdown().await;
}

/// DO NOTHING：PK 冲突行跳过，不计入受影响行数。
#[tokio::test]
async fn do_nothing_skips_pk_conflict_row() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(
        &db,
        "INSERT INTO users VALUES (1, 'Bob', 999) ON CONFLICT DO NOTHING",
        0,
    )
    .await;

    expect_count(&db, "users", 1).await;
    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 1",
        0,
        serde_json::json!("Alice"),
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// DO NOTHING：唯一索引冲突（非 PK 目标）同样跳过。
#[tokio::test]
async fn do_nothing_skips_unique_conflict_row() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(
        &db,
        "INSERT INTO users VALUES (2, 'Bob', 100) ON CONFLICT DO NOTHING",
        0,
    )
    .await;

    expect_count(&db, "users", 1).await;
    expect_where_count(&db, "SELECT COUNT(*) FROM users WHERE id = 2", 0).await;

    db.wal_buffer.shutdown().await;
}

/// DO NOTHING 多行 VALUES：冲突行跳过、非冲突行正常插入，计数为非冲突行数。
#[tokio::test]
async fn do_nothing_mixed_rows_inserts_only_non_conflicting() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(
        &db,
        "INSERT INTO users VALUES (1, 'dup', 999), (2, 'Bob', 200), (3, 'Cid', 300) \
         ON CONFLICT DO NOTHING",
        2,
    )
    .await;

    expect_count(&db, "users", 3).await;
    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 1",
        0,
        serde_json::json!("Alice"),
    )
    .await;
    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 2",
        0,
        serde_json::json!("Bob"),
    )
    .await;
    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 3",
        0,
        serde_json::json!("Cid"),
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// 显式单列目标命中唯一索引列（spec R4：`(u)` 目标的冲突被仲裁跳过）。
#[tokio::test]
async fn explicit_unique_target_skips_unique_conflict() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(
        &db,
        "INSERT INTO users VALUES (2, 'Bob', 100) ON CONFLICT (code) DO NOTHING",
        0,
    )
    .await;
    expect_count(&db, "users", 1).await;
    expect_where_count(&db, "SELECT COUNT(*) FROM users WHERE id = 2", 0).await;

    db.wal_buffer.shutdown().await;
}

/// 显式单列目标命中主键列（spec R4：`(id)` 目标的冲突被仲裁跳过）。
#[tokio::test]
async fn explicit_pk_target_skips_pk_conflict() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(
        &db,
        "INSERT INTO users VALUES (1, 'Bob', 200) ON CONFLICT (id) DO NOTHING",
        0,
    )
    .await;
    expect_count(&db, "users", 1).await;
    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 1",
        0,
        serde_json::json!("Alice"),
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// 显式单列目标只仲裁该列：目标外的约束冲突不触发 DO NOTHING。
#[tokio::test]
async fn explicit_target_only_arbitrates_that_column() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    // code 冲突但目标为 (id)：仲裁不命中 → 走插入序列 → 唯一门 DuplicateKey
    expect_error(
        &db,
        "INSERT INTO users VALUES (2, 'Bob', 100) ON CONFLICT (id) DO NOTHING",
        "Duplicate key",
    )
    .await;
    expect_count(&db, "users", 1).await;

    db.wal_buffer.shutdown().await;
}

/// 显式唯一列目标 + 主键冲突（DO NOTHING 臂）：仲裁不命中 → 无冲突插入路径
/// 的 PK 重复预检以既有 DuplicateKey 零副作用拒绝（spec R3：显式目标仅仲裁
/// 该约束，仲裁外的 PK 约束 SHALL 以既有 DuplicateKey 拒绝）。
#[tokio::test]
async fn explicit_unique_target_with_pk_conflict_rejected() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    // code 不冲突（200）、目标为 (code) → 仲裁不命中；PK 冲突须被插入路径拒绝
    expect_error(
        &db,
        "INSERT INTO users VALUES (1, 'Bob', 200) ON CONFLICT (code) DO NOTHING",
        "Duplicate key",
    )
    .await;
    expect_count(&db, "users", 1).await;
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        0,
        serde_json::json!("Alice"),
    )
    .await;
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        1,
        serde_json::json!(100),
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// 显式唯一列目标 + 主键冲突（DO UPDATE 臂）：同型拒绝，行数与原行值不变。
#[tokio::test]
async fn explicit_unique_target_with_pk_conflict_do_update_rejected() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_error(
        &db,
        "INSERT INTO users VALUES (1, 'Bob', 200) ON CONFLICT (code) DO UPDATE SET name = 'Zed'",
        "Duplicate key",
    )
    .await;
    expect_count(&db, "users", 1).await;
    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 1",
        0,
        serde_json::json!("Alice"),
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// 仲裁序：PK 先于唯一列（两列同时冲突时按 PK 命中处理，行为等价）。
#[tokio::test]
async fn arbitration_prefers_pk_over_unique() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(
        &db,
        "INSERT INTO users VALUES (1, 'Bob', 100) ON CONFLICT DO NOTHING",
        0,
    )
    .await;
    expect_count(&db, "users", 1).await;

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// 2.3 — DO UPDATE 原位更新
// ===========================================================================

/// 字面量赋值：冲突行原位更新；未赋值列保持旧值（DO UPDATE 只消费 SET 清单）。
#[tokio::test]
async fn do_update_literal_assignment_updates_in_place() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(
        &db,
        "INSERT INTO users VALUES (1, 'Bob', 200) ON CONFLICT (id) DO UPDATE SET name = 'Bob'",
        1,
    )
    .await;

    expect_count(&db, "users", 1).await;
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        0,
        serde_json::json!("Bob"),
    )
    .await;
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        1,
        serde_json::json!(100),
    )
    .await;
    // code 未被赋值 → 仍由本行占用
    expect_error(
        &db,
        "INSERT INTO users VALUES (2, 'Carl', 100)",
        "Duplicate key",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// `excluded.col` 赋值取本行待插值。
#[tokio::test]
async fn do_update_excluded_column_uses_inserted_value() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(
        &db,
        "INSERT INTO users VALUES (1, 'Zed', 300) \
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, code = excluded.code",
        1,
    )
    .await;

    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        0,
        serde_json::json!("Zed"),
    )
    .await;
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        1,
        serde_json::json!(300),
    )
    .await;
    expect_count(&db, "users", 1).await;
    // 唯一条目随赋值迁移：旧值 100 释放、新值 300 占用
    expect_affected(&db, "INSERT INTO users VALUES (2, 'Carl', 100)", 1).await;
    expect_error(
        &db,
        "INSERT INTO users VALUES (3, 'Dee', 300)",
        "Duplicate key",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// 裸列名赋值取冲突行旧值。
#[tokio::test]
async fn do_update_bare_column_uses_old_row_value() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(
        &db,
        "INSERT INTO users VALUES (1, 'Zed', 300) ON CONFLICT (id) DO UPDATE SET name = name",
        1,
    )
    .await;

    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 1",
        0,
        serde_json::json!("Alice"),
    )
    .await;
    // 待插行的 code 未被赋值（excluded 语义不自动落列），旧值保持
    expect_cell(
        &db,
        "SELECT code FROM users WHERE id = 1",
        0,
        serde_json::json!(100),
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// 多行 DO UPDATE 逐行独立判定（spec R3「多行语句逐行独立」）。
#[tokio::test]
async fn do_update_multi_row_evaluates_each_row() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    // 第 1 行冲突 → 原位更新；第 2 行新键 → 插入
    expect_affected(
        &db,
        "INSERT INTO users VALUES (1, 'Zed', 300), (2, 'Bob', 200) \
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, code = excluded.code",
        2,
    )
    .await;

    expect_count(&db, "users", 2).await;
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        0,
        serde_json::json!("Zed"),
    )
    .await;
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 2",
        0,
        serde_json::json!("Bob"),
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// 裸列名赋值作用于唯一列：取旧行值，不引入新唯一冲突（spec R3 DO UPDATE 场景）。
#[tokio::test]
async fn do_update_bare_column_on_unique_column_keeps_old_value() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(
        &db,
        "INSERT INTO users VALUES (1, 'Zed', 100) ON CONFLICT (id) DO UPDATE SET name = excluded.name, code = code",
        1,
    )
    .await;

    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        0,
        serde_json::json!("Zed"),
    )
    .await;
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        1,
        serde_json::json!(100),
    )
    .await;
    // 唯一列仍归本行，旧唯一值未被释放
    expect_error(
        &db,
        "INSERT INTO users VALUES (2, 'Bob', 100)",
        "Duplicate key",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// `DEFAULT` 赋值取该列声明默认值。
#[tokio::test]
async fn do_update_default_assignment_uses_declared_default() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(
        &db,
        "INSERT INTO users VALUES (1, 'Zed', 300) ON CONFLICT (id) DO UPDATE SET name = DEFAULT",
        1,
    )
    .await;

    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 1",
        0,
        serde_json::json!("anon"),
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// 唯一列碰撞预检零副作用：新键命中即拒，两行数据与索引不变。
#[tokio::test]
async fn do_update_collision_precheck_is_side_effect_free() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(&db, "INSERT INTO users VALUES (2, 'Bob', 200)", 1).await;

    // code=200 已被 id=2 占用 → 碰撞预检拒绝，id=1 行原样
    expect_error(
        &db,
        "INSERT INTO users VALUES (1, 'Zed', 200) ON CONFLICT (id) DO UPDATE SET code = 200",
        "Duplicate key",
    )
    .await;

    expect_count(&db, "users", 2).await;
    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 1",
        0,
        serde_json::json!("Alice"),
    )
    .await;
    expect_cell(
        &db,
        "SELECT code FROM users WHERE id = 1",
        0,
        serde_json::json!(100),
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// 键位 rekey 碰撞预检零副作用：新 PK 命中即拒。
#[tokio::test]
async fn do_update_rekey_collision_is_side_effect_free() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(&db, "INSERT INTO users VALUES (2, 'Bob', 200)", 1).await;

    expect_error(
        &db,
        "INSERT INTO users VALUES (1, 'Zed', 300) ON CONFLICT (id) DO UPDATE SET id = 2",
        "Duplicate key",
    )
    .await;
    expect_count(&db, "users", 2).await;
    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 1",
        0,
        serde_json::json!("Alice"),
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// 键位 rekey 成功：旧条目清理、新条目建立，PK 等值点查随之迁移。
#[tokio::test]
async fn do_update_rekey_maintains_pk_entry() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(
        &db,
        "INSERT INTO users VALUES (1, 'Zed', 300) ON CONFLICT (id) DO UPDATE SET id = 7",
        1,
    )
    .await;

    expect_count(&db, "users", 1).await;
    // 仅 SET 清单被赋值——name 取冲突行旧值，不取待插行值
    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 7",
        0,
        serde_json::json!("Alice"),
    )
    .await;
    expect_where_count(&db, "SELECT COUNT(*) FROM users WHERE id = 1", 0).await;
    // 旧键位 1 已释放
    expect_affected(&db, "INSERT INTO users VALUES (1, 'Ann', 400)", 1).await;

    db.wal_buffer.shutdown().await;
}

/// NOT NULL 赋值零副作用拒绝。
#[tokio::test]
async fn do_update_not_null_violation_is_side_effect_free() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, req VARCHAR(100) NOT NULL)")
        .await;
    db.checkpoint().await.unwrap();
    expect_affected(&db, "INSERT INTO t VALUES (1, 'keep')", 1).await;

    expect_error(
        &db,
        "INSERT INTO t VALUES (1, 'x') ON CONFLICT (id) DO UPDATE SET req = NULL",
        "NOT NULL constraint violation: column 'req'",
    )
    .await;
    expect_cell(
        &db,
        "SELECT req FROM t WHERE id = 1",
        0,
        serde_json::json!("keep"),
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// 赋值列类型门：跨类型值点名拒绝且原行保持。
#[tokio::test]
async fn do_update_type_gate_rejects_cross_type_value() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_error(
        &db,
        "INSERT INTO users VALUES (1, 'Zed', 300) ON CONFLICT (id) DO UPDATE SET code = 'abc'",
        "expects INT",
    )
    .await;
    expect_cell(
        &db,
        "SELECT code FROM users WHERE id = 1",
        0,
        serde_json::json!(100),
    )
    .await;

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// 2.4 — REPLACE INTO
// ===========================================================================

/// REPLACE：PK 冲突行删除后重插，计 1 行。
#[tokio::test]
async fn replace_into_replaces_conflicting_row() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(&db, "REPLACE INTO users VALUES (1, 'Zed', 300)", 1).await;

    expect_count(&db, "users", 1).await;
    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 1",
        0,
        serde_json::json!("Zed"),
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// REPLACE：唯一索引冲突行同样被替换。
#[tokio::test]
async fn replace_into_replaces_unique_conflicting_row() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(&db, "REPLACE INTO users VALUES (2, 'Zed', 100)", 1).await;

    expect_count(&db, "users", 1).await;
    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 2",
        0,
        serde_json::json!("Zed"),
    )
    .await;
    // 替换行自身占用 code=100；被替换行的 PK 键位 1 已释放
    expect_error(
        &db,
        "INSERT INTO users VALUES (3, 'Carl', 100)",
        "Duplicate key",
    )
    .await;
    expect_affected(&db, "INSERT INTO users VALUES (1, 'Ann', 300)", 1).await;

    db.wal_buffer.shutdown().await;
}

/// REPLACE：无冲突时为普通插入。
#[tokio::test]
async fn replace_into_without_conflict_inserts() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "REPLACE INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(&db, "REPLACE INTO users VALUES (2, 'Bob', 200)", 1).await;
    expect_count(&db, "users", 2).await;

    db.wal_buffer.shutdown().await;
}

/// REPLACE：同一行被 PK 与唯一索引同时命中（id == code）时只删除一次。
#[tokio::test]
async fn replace_into_dedupes_row_hit_by_pk_and_unique() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 1)", 1).await;
    // PK(1) 与唯一索引(code=1) 命中同一行 → 去重后单次删除
    expect_affected(&db, "REPLACE INTO users VALUES (1, 'Zed', 1)", 1).await;

    // 单次删除语义：冲突行只被删一次（重复墓碑会使该行不可见或版本链错乱）
    expect_count(&db, "users", 1).await;
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        0,
        serde_json::json!("Zed"),
    )
    .await;
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        1,
        serde_json::json!(1),
    )
    .await;
    // 替换行自身占用 code=1
    expect_error(
        &db,
        "INSERT INTO users VALUES (2, 'Carl', 1)",
        "Duplicate key",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// REPLACE：两条不同冲突行（PK 命中一行 + 唯一命中另一行）全部删除后重插。
#[tokio::test]
async fn replace_into_removes_all_conflicting_rows() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(&db, "INSERT INTO users VALUES (2, 'Bob', 200)", 1).await;
    // id=1（PK 冲突）且 code=200（唯一冲突）→ 两条冲突行均删除
    expect_affected(&db, "REPLACE INTO users VALUES (1, 'Zed', 200)", 1).await;

    expect_count(&db, "users", 1).await;
    expect_cell(
        &db,
        "SELECT name FROM users WHERE id = 1",
        0,
        serde_json::json!("Zed"),
    )
    .await;
    expect_where_count(&db, "SELECT COUNT(*) FROM users WHERE id = 2", 0).await;
    // 两条被删行的唯一值 100 / 旧 PK 键位均已释放
    expect_affected(&db, "INSERT INTO users VALUES (3, 'Carl', 100)", 1).await;

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// 2.5 — 恢复两态与显式事务
// ===========================================================================

/// 干净重开（close → checkpoint → redo == 0 → catalog 根加载）后 DO UPDATE /
/// REPLACE / DO NOTHING 的数据与索引保持一致。
#[tokio::test]
async fn clean_reopen_keeps_upsert_state() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    {
        let db = Database::open(&path).await.unwrap();
        setup(&db).await;
        expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
        expect_affected(
            &db,
            "INSERT INTO users VALUES (1, 'Zed', 300) ON CONFLICT (id) DO UPDATE SET name = excluded.name, code = excluded.code",
            1,
        )
        .await;
        expect_affected(&db, "REPLACE INTO users VALUES (1, 'Zed2', 300)", 1).await;
        expect_affected(&db, "INSERT INTO users VALUES (2, 'x', 999)", 1).await;
        expect_affected(
            &db,
            "INSERT INTO users VALUES (2, 'y', 998) ON CONFLICT DO NOTHING",
            0,
        )
        .await;
        db.close().await.unwrap();
    }

    let db2 = Database::open(&path).await.unwrap();
    expect_count(&db2, "users", 2).await;
    expect_cell(
        &db2,
        "SELECT name, code FROM users WHERE id = 1",
        0,
        serde_json::json!("Zed2"),
    )
    .await;
    // 索引一致：唯一值仍被占用
    expect_error(
        &db2,
        "INSERT INTO users VALUES (3, 'z', 300)",
        "Duplicate key",
    )
    .await;
    expect_affected(
        &db2,
        "INSERT INTO users VALUES (1, 'w', 400) ON CONFLICT (id) DO UPDATE SET name = 'w'",
        1,
    )
    .await;
    expect_cell(
        &db2,
        "SELECT name FROM users WHERE id = 1",
        0,
        serde_json::json!("w"),
    )
    .await;
    db2.wal_buffer.shutdown().await;
}

/// 崩溃恢复（shutdown + drop 不 close → redo > 0 → 索引重建）后 DO UPDATE /
/// REPLACE 的数据与索引保持一致。
#[tokio::test]
async fn crash_recovery_keeps_upsert_state() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let db = Database::open(&path).await.unwrap();
    setup(&db).await;
    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(&db, "INSERT INTO users VALUES (2, 'Bob', 200)", 1).await;
    expect_affected(
        &db,
        "INSERT INTO users VALUES (1, 'Zed', 300) ON CONFLICT (id) DO UPDATE SET name = 'Zed', code = 300",
        1,
    )
    .await;
    expect_affected(&db, "REPLACE INTO users VALUES (2, 'Ann', 400)", 1).await;
    db.wal_buffer.shutdown().await;
    drop(db); // 无 close()——不 checkpoint，WAL 保留，重开走 redo + 重建

    let db2 = Database::open(&path).await.unwrap();
    expect_count(&db2, "users", 2).await;
    expect_cell(
        &db2,
        "SELECT name, code FROM users WHERE id = 1",
        0,
        serde_json::json!("Zed"),
    )
    .await;
    expect_cell(
        &db2,
        "SELECT name FROM users WHERE id = 2",
        0,
        serde_json::json!("Ann"),
    )
    .await;
    // 重建后的唯一索引仍生效：两个存活行的唯一值被占用
    expect_error(
        &db2,
        "INSERT INTO users VALUES (3, 'Cid', 300)",
        "Duplicate key",
    )
    .await;
    expect_error(
        &db2,
        "INSERT INTO users VALUES (3, 'Cid', 400)",
        "Duplicate key",
    )
    .await;
    // DO UPDATE 释放的旧唯一值（100）可复用，且 upsert 在恢复后仍可用
    expect_affected(&db2, "INSERT INTO users VALUES (3, 'Cid', 100)", 1).await;
    expect_affected(
        &db2,
        "INSERT INTO users VALUES (3, 'Cid2', 700) ON CONFLICT (id) DO UPDATE SET code = 700",
        1,
    )
    .await;
    expect_cell(
        &db2,
        "SELECT name, code FROM users WHERE id = 3",
        1,
        serde_json::json!(700),
    )
    .await;
    db2.wal_buffer.shutdown().await;
}

/// 显式事务内 upsert 失败 → 回滚后原值可重插（abort 清理通道含唯一条目）。
#[tokio::test]
async fn explicit_tx_upsert_failure_rolls_back_and_allows_reinsert() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;
    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;
    expect_affected(&db, "INSERT INTO users VALUES (2, 'Bob', 200)", 1).await;

    let tx = db.begin().await.unwrap();
    // 唯一碰撞 → 零副作用拒绝（拒绝语句不产生任何写入）
    let rejected = db
        .execute_in_tx(
            "INSERT INTO users VALUES (1, 'Zed', 200) ON CONFLICT (id) DO UPDATE SET code = 200",
            &tx,
        )
        .await;
    assert!(
        matches!(rejected, Response::Error { .. }),
        "事务内唯一碰撞必须报错，实际: {rejected:?}"
    );
    // 同事务内的成功 upsert 随后被回滚
    let applied = db
        .execute_in_tx(
            "INSERT INTO users VALUES (1, 'Zed', 300) ON CONFLICT (id) DO UPDATE SET name = 'Zed'",
            &tx,
        )
        .await;
    assert!(
        matches!(applied, Response::AffectedRows { count: 1 }),
        "同事务内合法 upsert 应成功，实际: {applied:?}"
    );
    db.rollback(tx).await.unwrap();

    // 回滚后原行原值恢复、唯一条目随之还原（code=100 仍归 id=1 所有）
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        0,
        serde_json::json!("Alice"),
    )
    .await;
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        1,
        serde_json::json!(100),
    )
    .await;
    expect_error(
        &db,
        "INSERT INTO users VALUES (3, 'Cid', 100)",
        "Duplicate key",
    )
    .await;
    // 事务内未落地的值无残留
    expect_affected(&db, "INSERT INTO users VALUES (3, 'Cid', 300)", 1).await;

    db.wal_buffer.shutdown().await;
}

/// 显式事务内 upsert 成功 → commit 后数据与索引一致。
#[tokio::test]
async fn explicit_tx_upsert_commit_persists() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let db = Database::open(&path).await.unwrap();
    setup(&db).await;
    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;

    let tx = db.begin().await.unwrap();
    match db
        .execute_in_tx(
            "INSERT INTO users VALUES (1, 'Zed', 300) ON CONFLICT (id) DO UPDATE SET name = 'Zed', code = 300",
            &tx,
        )
        .await
    {
        Response::AffectedRows { count } => assert_eq!(count, 1),
        other => panic!("Expected AffectedRows(1), got {:?}", other),
    }
    db.commit(tx).await.unwrap();

    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        0,
        serde_json::json!("Zed"),
    )
    .await;
    expect_affected(&db, "INSERT INTO users VALUES (2, 'Bob', 500)", 1).await;
    // 唯一条目随赋值迁移：旧值 100 释放、新值 300 占用
    expect_affected(&db, "INSERT INTO users VALUES (3, 'Cid', 100)", 1).await;
    expect_error(
        &db,
        "INSERT INTO users VALUES (4, 'Dee', 300)",
        "Duplicate key",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// 显式事务内 REPLACE 回滚：被替换行按 abort 通道恢复（墓碑中和 + 索引还原）。
#[tokio::test]
async fn explicit_tx_replace_rollback_restores_row() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;
    expect_affected(&db, "INSERT INTO users VALUES (1, 'Alice', 100)", 1).await;

    let tx = db.begin().await.unwrap();
    match db
        .execute_in_tx("REPLACE INTO users VALUES (1, 'Zed', 300)", &tx)
        .await
    {
        Response::AffectedRows { count } => assert_eq!(count, 1),
        other => panic!("Expected AffectedRows(1), got {:?}", other),
    }
    db.rollback(tx).await.unwrap();

    // 回滚后原行按 abort 通道复现（墓碑中和），且墓碑的 PK 与唯一索引条目
    // 经 replan 2.9 还原——点查可达（修复前丢失，见显式事务回滚矩阵）。
    match db.execute_sql("SELECT * FROM users").await {
        Response::QueryResult { rows } => assert_eq!(
            rows,
            vec![vec![
                serde_json::json!(1),
                serde_json::json!("Alice"),
                serde_json::json!(100)
            ]],
            "回滚后原行必须复现"
        ),
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        0,
        serde_json::json!("Alice"),
    )
    .await;
    expect_cell(
        &db,
        "SELECT name, code FROM users WHERE id = 1",
        1,
        serde_json::json!(100),
    )
    .await;
    // 唯一值 100 仍被复现的原行占用——修复前唯一条目随删除一并清理且回滚
    // 不还原，同值可再次插入（UNIQUE 静默失效）。
    expect_error(
        &db,
        "INSERT INTO users VALUES (2, 'Bob', 100)",
        "Duplicate key",
    )
    .await;

    db.wal_buffer.shutdown().await;
}
