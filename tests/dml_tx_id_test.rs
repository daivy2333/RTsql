//! Regression tests for MS06-T01: DML `tx_id=0` 占位注入修复
//!
//! 验证 `Database::execute_sql` 发出的 INSERT / UPDATE / DELETE 走完整的
//! `TransactionManager::begin → executor → commit/abort` 生命周期，写入的数据行
//! 带正确的 `create_tx_id > 0` 且 `commit_tx_id` 在 commit 后被设置。
//!
//! 修复前：所有 DML 写入的 `create_tx_id = 0`，`commit_tx_id` 永为 None，
//!         `active_transactions()` 永为 0。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::storage::RowId;
use tempfile::tempdir;

/// Helper: open an in-memory database for one test, create a simple table.
async fn open_db_with_table(table_sql: &str) -> (Database, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db"))
        .await
        .expect("Failed to open database");
    let resp = db.execute_sql(table_sql).await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "CREATE TABLE failed: {:?}",
        resp
    );
    (db, dir)
}

/// Helper: read the version header of the row pointed to by `key` (PK = id INT).
async fn read_vh(
    db: &Database,
    table: &str,
    key: i64,
) -> rtsql::storage::Result<rtsql::transaction::VersionHeader> {
    let table_meta = db.table_manager.get_table(table).await.unwrap();
    let key_bytes = key.to_be_bytes();
    let row_id = table_meta
        .index_manager
        .search(&key_bytes)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("row {} not found in {}", key, table));
    db.buffer_pool.read_version_header(row_id).await
}

/// R1/S1: INSERT through `Database::execute_sql` writes a real `create_tx_id > 0`
/// and a matching `commit_tx_id` after commit.
#[tokio::test]
async fn test_insert_writes_real_create_tx_id() {
    let (db, _dir) = open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;

    let resp = db
        .execute_sql("INSERT INTO t (id, name) VALUES (1, 'alice')")
        .await;
    assert!(
        matches!(resp, Response::AffectedRows { count: 1 }),
        "{:?}",
        resp
    );

    let vh = read_vh(&db, "t", 1).await.unwrap();
    assert!(
        vh.create_tx_id() > 0,
        "create_tx_id must be > 0 (real allocation), got {}",
        vh.create_tx_id()
    );
    assert_eq!(
        vh.commit_tx_id(),
        Some(vh.create_tx_id()),
        "commit_tx_id must equal create_tx_id after successful commit"
    );
    assert!(!vh.is_deleted(), "row should not be deleted");
}

/// R1/S2: UPDATE through `Database::execute_sql` writes a new version with real
/// `create_tx_id` and `commit_tx_id`.
#[tokio::test]
async fn test_update_writes_real_create_tx_id() {
    let (db, _dir) = open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;

    db.execute_sql("INSERT INTO t (id, name) VALUES (1, 'alice')")
        .await;
    let resp = db
        .execute_sql("UPDATE t SET name = 'bob' WHERE id = 1")
        .await;
    assert!(
        matches!(resp, Response::AffectedRows { count: 1 }),
        "{:?}",
        resp
    );

    // New version lives on a different RowId (version chain).
    let table_meta = db.table_manager.get_table("t").await.unwrap();
    let mut current: Option<RowId> = table_meta
        .index_manager
        .search(&1i64.to_be_bytes())
        .await
        .unwrap();
    let mut found_new_version = false;
    let mut iterations = 0;
    while let Some(rid) = current {
        iterations += 1;
        if iterations > 8 {
            panic!("version chain too long — likely a cycle");
        }
        let vh = db.buffer_pool.read_version_header(rid).await.unwrap();
        if vh.is_deleted() {
            current = vh.next_version();
            continue;
        }
        if vh.create_tx_id() > 0 && vh.commit_tx_id() == Some(vh.create_tx_id()) {
            found_new_version = true;
            break;
        }
        current = vh.next_version();
    }
    assert!(
        found_new_version,
        "expected a committed version with create_tx_id > 0 and matching commit_tx_id"
    );
}

/// R1/S3: DELETE through `Database::execute_sql` marks the row as deleted.
#[tokio::test]
async fn test_delete_writes_real_create_tx_id() {
    let (db, _dir) = open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;

    db.execute_sql("INSERT INTO t (id, name) VALUES (1, 'alice')")
        .await;
    let resp = db.execute_sql("DELETE FROM t WHERE id = 1").await;
    assert!(
        matches!(resp, Response::AffectedRows { count: 1 }),
        "{:?}",
        resp
    );

    // After delete, the index entry should be gone (or row is_deleted).
    let table_meta = db.table_manager.get_table("t").await.unwrap();
    let key_bytes = 1i64.to_be_bytes();
    match table_meta.index_manager.search(&key_bytes).await.unwrap() {
        None => { /* index removed, expected */ }
        Some(rid) => {
            let vh = db.buffer_pool.read_version_header(rid).await.unwrap();
            assert!(
                vh.is_deleted(),
                "row {} should be marked deleted (commit_tx_id = DELETED_TX_ID)",
                1
            );
        }
    }
}

/// R1/S4: DML failure (duplicate PK) aborts the transaction so the failed tx
/// is no longer in `active_transactions()`.
#[tokio::test]
async fn test_insert_duplicate_pk_aborts_transaction() {
    let (db, _dir) = open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;

    db.execute_sql("INSERT INTO t (id, name) VALUES (1, 'alice')")
        .await;

    // Snapshot the current tx id (the successful INSERT committed; allocator advanced).
    let tx_id_before = db.transaction_manager.current_tx_id();

    // Duplicate PK should fail.
    let resp = db
        .execute_sql("INSERT INTO t (id, name) VALUES (1, 'bob')")
        .await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "duplicate PK should error, got {:?}",
        resp
    );

    // The failed tx must have been aborted (removed from active list).
    let active = db.transaction_manager.active_transactions().await;
    let new_ids: Vec<u64> = active.into_iter().filter(|id| *id > tx_id_before).collect();
    assert!(
        new_ids.is_empty(),
        "no transactions should remain active after abort, found: {:?}",
        new_ids
    );

    // Allocator must still have advanced (a tx_id was allocated then released).
    assert!(
        db.transaction_manager.current_tx_id() > tx_id_before,
        "current_tx_id must advance past the aborted tx"
    );
}

/// R1/S5: 10 consecutive DML statements produce strictly increasing tx_ids.
#[tokio::test]
async fn test_consecutive_dml_have_unique_tx_ids() {
    let (db, _dir) = open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;

    let initial = db.transaction_manager.current_tx_id();

    let mut prev = initial;
    for i in 0..10 {
        let resp = db
            .execute_sql(&format!(
                "INSERT INTO t (id, name) VALUES ({}, 'row{}')",
                i, i
            ))
            .await;
        assert!(
            matches!(resp, Response::AffectedRows { count: 1 }),
            "{:?}",
            resp
        );
        let now = db.transaction_manager.current_tx_id();
        assert!(
            now > prev,
            "tx_id must strictly increase: prev={}, now={}",
            prev,
            now
        );
        prev = now;
    }
}

/// R1/S6: an INSERT that successfully commits is visible to a subsequent SELECT.
#[tokio::test]
async fn test_insert_visible_after_commit() {
    let (db, _dir) = open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;

    db.execute_sql("INSERT INTO t (id, name) VALUES (1, 'alice')")
        .await;

    let resp = db.execute_sql("SELECT id, name FROM t WHERE id = 1").await;
    match resp {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1, "row must be visible after commit");
            // JSON shape: [1, "alice"]
            assert_eq!(rows[0][0], serde_json::json!(1));
            assert_eq!(rows[0][1], serde_json::json!("alice"));
        }
        other => panic!("expected QueryResult, got {:?}", other),
    }
}
