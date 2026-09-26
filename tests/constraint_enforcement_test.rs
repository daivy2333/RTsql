//! MS23 Iteration 000 — 数据完整性约束执行面（诚实化 + NOT NULL 强制）
//!
//! change: 2026-09-24-ms23-constraint-enforcement
//!
//! 缺陷现场（Plan Context 调查）：NOT NULL/UNIQUE 经 `extract_column_constraints`
//! 解析并持久化进 catalog，但执行器零消费——「DDL 静默接受、运行期永不生效」。
//! 本文件 Iteration 000 部分见证：
//!
//! - R1（NOT NULL 强制）：INSERT/UPDATE 对 NOT NULL 列的 NULL 写入以
//!   `NullConstraintViolation { column }` 点名拒绝且零副作用；未声明列
//!   （含未声明 NOT NULL 的 PK 键位 keyless 语义）行为逐字节不变。
//! - UPDATE 面用例（1.5）与 UNIQUE 面/恢复两态用例（Iteration 001）随后追加。
//!
//! 判定全部走 lib API（`Database::open` + `execute_sql` → `Response`），
//! 错误文本断言点名列名与约束名。

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
    let sql = format!("SELECT COUNT(*) FROM {}", table);
    match db.execute_sql(&sql).await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(count), "{sql}");
        }
        other => panic!("Expected QueryResult for {sql}, got {:?}", other),
    }
}

// ===========================================================================
// 1.4 — INSERT NOT NULL 零副作用强制
// ===========================================================================

/// (a) INSERT NULL → 拒绝且错误文本点名列名
#[tokio::test]
async fn insert_null_rejected_naming_column() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE users (id INT PRIMARY KEY, email VARCHAR(100) NOT NULL, name VARCHAR(100))")
        .await;

    expect_error(
        &db,
        "INSERT INTO users VALUES (1, NULL, 'x')",
        "NOT NULL constraint violation: column 'email'",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// (b) 拒绝零副作用——行数不变、表仍可写
#[tokio::test]
async fn insert_null_rejection_is_side_effect_free() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE users (id INT PRIMARY KEY, email VARCHAR(100) NOT NULL)")
        .await;

    expect_error(
        &db,
        "INSERT INTO users VALUES (1, NULL)",
        "NOT NULL constraint violation",
    )
    .await;
    expect_count(&db, "users", 0).await;

    // 拒绝后同库仍可正常写入
    expect_affected(&db, "INSERT INTO users VALUES (2, 'a@b.c')", 1).await;
    expect_count(&db, "users", 1).await;

    db.wal_buffer.shutdown().await;
}

/// (c) 非 NULL 值插入成功
#[tokio::test]
async fn insert_non_null_succeeds() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE users (id INT PRIMARY KEY, email VARCHAR(100) NOT NULL)")
        .await;

    expect_affected(&db, "INSERT INTO users VALUES (1, 'a@b.c')", 1).await;
    expect_count(&db, "users", 1).await;

    db.wal_buffer.shutdown().await;
}

/// (d) `PRIMARY KEY NOT NULL` 组合：键位 NULL 走 NullConstraintViolation
/// （先于键位类型预检/无键行落库路径），非无键行
#[tokio::test]
async fn insert_null_into_pk_not_null_combo_rejected() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY NOT NULL, v INT)")
        .await;

    expect_error(
        &db,
        "INSERT INTO t VALUES (NULL, 5)",
        "NOT NULL constraint violation: column 'id'",
    )
    .await;
    expect_count(&db, "t", 0).await;

    db.wal_buffer.shutdown().await;
}

/// (e) 未声明 NOT NULL 的列保持既有语义：普通列 NULL 照常落库；未声明
/// NOT NULL 的 PK 键位 NULL 保持无键行落库不入索引
#[tokio::test]
async fn undeclared_columns_keep_existing_semantics() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;

    // 普通列 NULL：既有语义，成功
    expect_affected(&db, "INSERT INTO t VALUES (1, NULL)", 1).await;

    // 未声明 NOT NULL 的 PK 键位 NULL：无键行落库不入索引（既有语义）
    expect_affected(&db, "INSERT INTO t VALUES (NULL, 5)", 1).await;
    expect_count(&db, "t", 2).await;

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// 1.5 — UPDATE SET NOT NULL 强制
// ===========================================================================

/// UPDATE 置 NULL → 拒绝点名 + 原值保持
#[tokio::test]
async fn update_set_null_on_not_null_column_rejected_keeps_old_value() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE users (id INT PRIMARY KEY, email VARCHAR(100) NOT NULL)")
        .await;
    expect_affected(&db, "INSERT INTO users VALUES (1, 'a@b.c')", 1).await;

    expect_error(
        &db,
        "UPDATE users SET email = NULL WHERE id = 1",
        "NOT NULL constraint violation: column 'email'",
    )
    .await;

    // 原值保持
    match db.execute_sql("SELECT email FROM users WHERE id = 1").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!("a@b.c"), "原值必须保持");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// SET 非 NULL 值成功
#[tokio::test]
async fn update_set_non_null_succeeds() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE users (id INT PRIMARY KEY, email VARCHAR(100) NOT NULL)")
        .await;
    expect_affected(&db, "INSERT INTO users VALUES (1, 'a@b.c')", 1).await;

    expect_affected(&db, "UPDATE users SET email = 'x@y.z' WHERE id = 1", 1).await;
    match db.execute_sql("SELECT email FROM users WHERE id = 1").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!("x@y.z"));
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// 目标行不存在时 KeyNotFound 优先于 NOT NULL 校验（Step 1 先行）
#[tokio::test]
async fn update_key_not_found_takes_priority() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE users (id INT PRIMARY KEY, email VARCHAR(100) NOT NULL)")
        .await;
    expect_affected(&db, "INSERT INTO users VALUES (1, 'a@b.c')", 1).await;

    expect_error(&db, "UPDATE users SET email = NULL WHERE id = 999", "Key not found").await;

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// 2.5 — INSERT UNIQUE 强制（预检零副作用 + 落位后条目）
// ===========================================================================

/// (a) INT UNIQUE 列重复值 → DuplicateKey 拒绝且零副作用，随后可插不同值
#[tokio::test]
async fn insert_duplicate_unique_value_rejected_side_effect_free() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE items (id INT PRIMARY KEY, code INT UNIQUE)")
        .await;
    expect_affected(&db, "INSERT INTO items VALUES (1, 100)", 1).await;

    expect_error(&db, "INSERT INTO items VALUES (2, 100)", "Duplicate key").await;

    // 零副作用：行数不变、索引无残留（同值仍占位），随后可插不同值
    expect_count(&db, "items", 1).await;
    expect_affected(&db, "INSERT INTO items VALUES (2, 200)", 1).await;
    expect_count(&db, "items", 2).await;

    db.wal_buffer.shutdown().await;
}

/// (b) 不同值插入成功（强制不误伤）
#[tokio::test]
async fn insert_distinct_unique_values_succeed() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE items (id INT PRIMARY KEY, code INT UNIQUE)")
        .await;
    expect_affected(&db, "INSERT INTO items VALUES (1, 100)", 1).await;
    expect_affected(&db, "INSERT INTO items VALUES (2, 200)", 1).await;
    expect_count(&db, "items", 2).await;

    db.wal_buffer.shutdown().await;
}

/// (c) 多行 NULL 全部成功（NULL 豁免——不入唯一索引、互不冲突）
#[tokio::test]
async fn insert_multiple_nulls_into_unique_column_succeed() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE items (id INT PRIMARY KEY, code INT UNIQUE)")
        .await;
    expect_affected(&db, "INSERT INTO items VALUES (1, NULL)", 1).await;
    expect_affected(&db, "INSERT INTO items VALUES (2, NULL)", 1).await;
    expect_count(&db, "items", 2).await;

    db.wal_buffer.shutdown().await;
}

/// (d) 两个 INT UNIQUE 列各自独立强制
#[tokio::test]
async fn two_unique_columns_enforced_independently() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, a INT UNIQUE, b INT UNIQUE)")
        .await;
    expect_affected(&db, "INSERT INTO t VALUES (1, 10, 20)", 1).await;

    // a 冲突
    expect_error(&db, "INSERT INTO t VALUES (2, 10, 99)", "Duplicate key").await;
    // b 冲突
    expect_error(&db, "INSERT INTO t VALUES (3, 99, 20)", "Duplicate key").await;
    // 两列均不同 → 成功
    expect_affected(&db, "INSERT INTO t VALUES (4, 11, 21)", 1).await;
    expect_count(&db, "t", 2).await;

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// 2.6 — UPDATE 唯一维护四分支
// ===========================================================================

/// (a) SET 唯一列为已有值 → DuplicateKey 拒绝且原值保持（任何写入前预检）
#[tokio::test]
async fn update_set_unique_to_existing_value_rejected_keeps_old() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE items (id INT PRIMARY KEY, code INT UNIQUE)")
        .await;
    expect_affected(&db, "INSERT INTO items VALUES (1, 100)", 1).await;
    expect_affected(&db, "INSERT INTO items VALUES (2, 200)", 1).await;

    expect_error(&db, "UPDATE items SET code = 200 WHERE id = 1", "Duplicate key").await;

    // 原值保持：id=1 仍是 100
    match db.execute_sql("SELECT code FROM items WHERE id = 1").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(100), "原值必须保持");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
}

/// (b) SET 非唯一列 → 唯一性检查继续正确（分支 1：条目随行回归见证）
#[tokio::test]
async fn update_non_unique_column_keeps_uniqueness_enforced() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE items (id INT PRIMARY KEY, code INT UNIQUE, tag INT)")
        .await;
    expect_affected(&db, "INSERT INTO items VALUES (1, 100, 1)", 1).await;
    expect_affected(&db, "INSERT INTO items VALUES (2, 200, 2)", 1).await;

    expect_affected(&db, "UPDATE items SET tag = 9 WHERE id = 1", 1).await;

    // 更新他列后，code=100 仍被 id=1 占用——他人插入同值仍拒绝
    expect_error(&db, "INSERT INTO items VALUES (3, 100, 3)", "Duplicate key").await;
    expect_count(&db, "items", 2).await;

    db.wal_buffer.shutdown().await;
}

/// (c)+(e) SET 唯一列新值 → 成功、旧值腾出（可再插）、新值占用（再插拒绝）
#[tokio::test]
async fn update_set_unique_to_new_value_moves_occupancy() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE items (id INT PRIMARY KEY, code INT UNIQUE)")
        .await;
    expect_affected(&db, "INSERT INTO items VALUES (1, 100)", 1).await;
    expect_affected(&db, "INSERT INTO items VALUES (2, 200)", 1).await;

    expect_affected(&db, "UPDATE items SET code = 300 WHERE id = 1", 1).await;

    // 新值已被 id=1 占用
    expect_error(&db, "INSERT INTO items VALUES (3, 300)", "Duplicate key").await;
    // 旧值腾出——他行可插
    expect_affected(&db, "INSERT INTO items VALUES (4, 100)", 1).await;
    expect_count(&db, "items", 3).await;

    db.wal_buffer.shutdown().await;
}

/// (d) SET 唯一列 NULL → 成功且同值可重插（NULL 不入索引，镜像 I037 分支）
#[tokio::test]
async fn update_set_unique_to_null_frees_value() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE items (id INT PRIMARY KEY, code INT UNIQUE)")
        .await;
    expect_affected(&db, "INSERT INTO items VALUES (1, 100)", 1).await;

    expect_affected(&db, "UPDATE items SET code = NULL WHERE id = 1", 1).await;

    // 旧值腾出
    expect_affected(&db, "INSERT INTO items VALUES (2, 100)", 1).await;
    expect_count(&db, "items", 2).await;

    db.wal_buffer.shutdown().await;
}

/// (f) 多唯一列 × SET PK 列（rekey 改 row_id）：全部唯一条目随行，其后
/// 唯一强制继续正确（契约 Risks 点名勿漏的组合）
#[tokio::test]
async fn update_pk_rekey_follows_all_unique_entries() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, a INT UNIQUE, b INT UNIQUE)")
        .await;
    expect_affected(&db, "INSERT INTO t VALUES (1, 10, 20)", 1).await;
    expect_affected(&db, "INSERT INTO t VALUES (2, 11, 21)", 1).await;

    // PK rekey：1 → 100
    expect_affected(&db, "UPDATE t SET id = 100 WHERE id = 1", 1).await;

    // rekey 后唯一强制继续正确：a=10/b=20 仍被 id=100 占用
    expect_error(&db, "INSERT INTO t VALUES (3, 10, 99)", "Duplicate key").await;
    expect_error(&db, "INSERT INTO t VALUES (4, 99, 20)", "Duplicate key").await;
    expect_affected(&db, "INSERT INTO t VALUES (5, 12, 22)", 1).await;
    expect_count(&db, "t", 3).await;

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// 2.7 — DELETE 条目移除与回滚唯一修复
// ===========================================================================

/// (a)+(d) DELETE 唯一行后同值可重插（无残留条目）
#[tokio::test]
async fn delete_unique_row_then_reinsert_same_value_succeeds() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE items (id INT PRIMARY KEY, code INT UNIQUE)")
        .await;
    expect_affected(&db, "INSERT INTO items VALUES (1, 100)", 1).await;
    expect_affected(&db, "DELETE FROM items WHERE id = 1", 1).await;

    // 同值重插必须成功——DELETE 须移除唯一条目
    expect_affected(&db, "INSERT INTO items VALUES (2, 100)", 1).await;
    expect_count(&db, "items", 1).await;

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// 2.8 — 恢复重建唯一索引（两态一致）
// ===========================================================================

/// (a) 崩溃恢复（shutdown + drop 不 close → redo > 0 → 数据页重建）后
/// 唯一强制保持
#[tokio::test]
async fn crash_recovery_rebuilds_unique_enforcement() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let db = Database::open(&path).await.unwrap();
    db.execute_sql("CREATE TABLE items (id INT PRIMARY KEY, code INT UNIQUE)")
        .await;
    // 夹具先例（wal_recovery_large_test.rs）：DDL 后 checkpoint 落盘
    // catalog，再注入 DML——崩溃恢复面针对 redo + 重建，不针对 catalog。
    db.checkpoint().await.unwrap();
    for i in 1..=5 {
        let sql = format!("INSERT INTO items VALUES ({i}, {})", 1000 + i);
        expect_affected(&db, &sql, 1).await;
    }
    db.wal_buffer.shutdown().await;
    drop(db); // 无 close()——不 checkpoint，WAL 保留，重开走 redo + 重建

    let db2 = Database::open(&path).await.unwrap();
    // 恢复后 code=1003 仍被 id=3 占用
    expect_error(&db2, "INSERT INTO items VALUES (9, 1003)", "Duplicate key").await;
    expect_affected(&db2, "INSERT INTO items VALUES (9, 2000)", 1).await;
    expect_count(&db2, "items", 6).await;
    db2.wal_buffer.shutdown().await;
}

/// (b) 干净关闭重开（close → checkpoint → redo == 0 → catalog 根加载）后
/// 唯一强制保持
#[tokio::test]
async fn clean_reopen_keeps_unique_enforcement() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    {
        let db = Database::open(&path).await.unwrap();
        db.execute_sql("CREATE TABLE items (id INT PRIMARY KEY, code INT UNIQUE)")
            .await;
        expect_affected(&db, "INSERT INTO items VALUES (1, 100)", 1).await;
        db.close().await.unwrap();
    }

    let db2 = Database::open(&path).await.unwrap();
    expect_error(&db2, "INSERT INTO items VALUES (2, 100)", "Duplicate key").await;
    expect_affected(&db2, "INSERT INTO items VALUES (2, 200)", 1).await;
    db2.wal_buffer.shutdown().await;
}

/// (c) 跨链重复注入（直写数据页第二条同值 committed 链）→ 恢复以点名
/// 表与列的显式错误失败（K05 判重，镜像 PK 跨链重复）
#[tokio::test]
async fn cross_chain_duplicate_unique_value_fails_recovery_named() {
    use rtsql::storage::page_format::{compute_tuple_size, serialize_tuple, ColumnType};
    use rtsql::storage::write_tuple_to_data_page;
    use rtsql::transaction::VersionHeader;

    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    {
        let db = Database::open(&path).await.unwrap();
        db.execute_sql("CREATE TABLE items (id INT PRIMARY KEY, code INT UNIQUE)")
            .await;
        // 夹具先例：DDL 后 checkpoint 落盘 catalog（同 (a)）
        db.checkpoint().await.unwrap();
        expect_affected(&db, "INSERT INTO items VALUES (1, 100)", 1).await;

        // 注入：绕过执行器唯一预检，直写数据页第二条 committed 链 (2, 100)
        let meta = db.table_manager.get_table("items").await.unwrap();
        let schema: Vec<ColumnType> =
            meta.columns.iter().map(|(_, ct)| ct.clone()).collect();
        let values = vec![
            rtsql::executor::Value::Int(2),
            rtsql::executor::Value::Int(100),
        ];
        let size = compute_tuple_size(&values, &schema);
        let mut buf = vec![0u8; size];
        serialize_tuple(&values, &schema, &mut buf).unwrap();
        let rid = write_tuple_to_data_page(
            &db.buffer_pool,
            &meta,
            &VersionHeader::new(999, None),
            &buf,
        )
        .await
        .unwrap();
        db.buffer_pool.write_commit_tx_id(rid, 999).await.unwrap();
        // 注入槽无 WAL 记录——直写必须手动落盘，否则 drop 即丢失
        db.buffer_pool.flush_all().await.unwrap();

        db.wal_buffer.shutdown().await;
        drop(db);
    } // meta/db 全部 Arc 出作用域——advisory 锁释放后才能重开

    // 重开必须失败且点名表与列
    match Database::open(&path).await {
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("items") && msg.contains("UNIQUE") && msg.contains("code"),
                "recovery must fail naming table and UNIQUE column, got: {msg}"
            );
        }
        Ok(db) => {
            db.wal_buffer.shutdown().await;
            panic!("Expected recovery failure for cross-chain duplicate UNIQUE value");
        }
    }
}

/// 2.9(b) R5-S5：drop_table 释放唯一索引页——drop 后同进程新建表可写，
/// 且文件页数不增长（PK 树 + 唯一树 + 数据页全部入自由表复用）
#[tokio::test]
async fn drop_table_with_unique_index_releases_pages_for_reuse() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE big (id INT PRIMARY KEY, code INT UNIQUE)")
        .await;
    for i in 0..100u64 {
        let sql = format!("INSERT INTO big VALUES ({i}, {})", 10_000 + i);
        expect_affected(&db, &sql, 1).await;
    }
    let pages_before = db.buffer_pool.storage().page_count();

    match db.execute_sql("DROP TABLE big").await {
        Response::AffectedRows { .. } => {}
        other => panic!("Expected AffectedRows for DROP, got {:?}", other),
    }

    db.execute_sql("CREATE TABLE fresh (id INT PRIMARY KEY)").await;
    expect_affected(&db, "INSERT INTO fresh VALUES (1)", 1).await;
    expect_count(&db, "fresh", 1).await;

    let pages_after = db.buffer_pool.storage().page_count();
    assert!(
        pages_after <= pages_before,
        "drop must release data+pk+unique pages for reuse: before={pages_before} after={pages_after}"
    );

    db.wal_buffer.shutdown().await;
}

// ===========================================================================
// Plan Review F1 修复见证（当前 Cycle 修复）：UNIQUE 列非 Int 值写入
// KeyTypeMismatch 零副作用拒绝——修复前 INSERT 静默落库无条目、UPDATE
// 写入损坏值后维护区 unwrap panic
// ===========================================================================

/// INSERT：非 Int 字面量命中 INT UNIQUE 列 → KeyTypeMismatch 点名拒绝，
/// 零副作用（拒绝后行数不变、表可写）
#[tokio::test]
async fn insert_non_int_into_int_unique_column_rejected_key_type_mismatch() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE items (id INT PRIMARY KEY, code INT UNIQUE)")
        .await;

    expect_error(
        &db,
        "INSERT INTO items VALUES (1, 'abc')",
        "key column 'code' expects INT, got String",
    )
    .await;

    // 零副作用：行数不变，随后同 PK 合法值可正常插入
    expect_count(&db, "items", 0).await;
    expect_affected(&db, "INSERT INTO items VALUES (1, 100)", 1).await;
    expect_count(&db, "items", 1).await;

    db.wal_buffer.shutdown().await;
}

/// UPDATE：SET INT UNIQUE 列为非 Int 字面量 → 任何写入前 KeyTypeMismatch
/// 点名拒绝（修复前：Step 4-6 已写入损坏值后 new_key.unwrap() panic），
/// 原行保持、唯一条目完好、表可写
#[tokio::test]
async fn update_set_int_unique_column_to_non_int_rejected_keeps_old_value() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    db.execute_sql("CREATE TABLE items (id INT PRIMARY KEY, code INT UNIQUE)")
        .await;
    expect_affected(&db, "INSERT INTO items VALUES (1, 100)", 1).await;

    expect_error(
        &db,
        "UPDATE items SET code = 1.5 WHERE id = 1",
        "key column 'code' expects INT, got Float",
    )
    .await;

    // 原行保持：id=1 的 code 仍为 100
    match db.execute_sql("SELECT code FROM items WHERE id = 1").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], serde_json::json!(100), "原值必须保持");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }
    // 零副作用：code=100 的唯一条目仍归 id=1（他人插同值仍拒绝），随后他值可写
    expect_error(&db, "INSERT INTO items VALUES (2, 100)", "Duplicate key").await;
    expect_affected(&db, "INSERT INTO items VALUES (2, 200)", 1).await;
    expect_count(&db, "items", 2).await;

    db.wal_buffer.shutdown().await;
}
