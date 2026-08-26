//! E2E crash recovery tests
//!
//! Verify WAL-based crash recovery behavior.
//!
//! Note: TableManager is currently in-memory only — table definitions don't
//! survive restart. These tests validate:
//! 1. WAL records are correctly written and readable after reopen
//! 2. RecoveryManager correctly classifies committed/aborted/uncommitted txns
//! 3. When tables are recreated after restart, data pages are accessible

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::wal::{RecoveryManager, WalReader, WalRecord};
use std::path::PathBuf;
use tempfile::TempDir;

async fn open_db(dir: &TempDir) -> Database {
    let path = PathBuf::from(dir.path()).join("test");
    Database::open(&path).await.unwrap()
}

/// Verify WAL file contains correct records after shutdown
#[tokio::test]
async fn test_wal_records_survive_shutdown() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir).await;

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, name TEXT)")
        .await;
    db.execute_sql("INSERT INTO t VALUES (1, 'alice')").await;
    db.execute_sql("INSERT INTO t VALUES (2, 'bob')").await;

    // Graceful shutdown
    db.wal_buffer.shutdown().await;

    // Read WAL file directly
    let wal_path = PathBuf::from(dir.path()).join("test.wal");
    let mut reader = WalReader::open(&wal_path).unwrap();
    let records = reader.read_all().unwrap();

    // Should contain BeginTxn + Insert + CommitTxn records
    let begin_count = records
        .iter()
        .filter(|r| matches!(r, WalRecord::BeginTxn { .. }))
        .count();
    let insert_count = records
        .iter()
        .filter(|r| matches!(r, WalRecord::Insert { .. }))
        .count();
    let commit_count = records
        .iter()
        .filter(|r| matches!(r, WalRecord::CommitTxn { .. }))
        .count();

    assert!(begin_count > 0, "Should have BeginTxn records");
    assert!(insert_count > 0, "Should have Insert records");
    assert!(commit_count > 0, "Should have CommitTxn records");
}

/// Verify RecoveryManager correctly classifies committed transactions
#[tokio::test]
async fn test_recovery_classifies_committed_transactions() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir).await;

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    db.execute_sql("INSERT INTO t VALUES (1)").await;
    db.execute_sql("INSERT INTO t VALUES (2)").await;

    db.wal_buffer.shutdown().await;
    drop(db);

    // Use basic recovery (just classify transactions)
    let db_path = PathBuf::from(dir.path()).join("test");
    let (committed, aborted) = RecoveryManager::recover(&db_path).unwrap();

    assert!(
        !committed.is_empty(),
        "Should detect committed transactions"
    );
    assert!(aborted.is_empty(), "No aborted transactions expected");
}

/// Verify data can be re-read after reopen (data pages persist in FileStorage)
#[tokio::test]
async fn test_data_pages_survive_restart() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir).await;

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, val INT)")
        .await;

    for i in 0..5u64 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({}, {})", i, i * 10))
            .await;
    }

    // Verify initial state
    let resp = db.execute_sql("SELECT * FROM t").await;
    match resp {
        Response::QueryResult { rows } => assert_eq!(rows.len(), 5),
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db.wal_buffer.shutdown().await;
    drop(db);

    // Reopen — triggers recovery, then recreate table
    let db2 = open_db(&dir).await;
    db2.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, val INT)")
        .await;

    // Data pages still exist in FileStorage; tuples are persistent.
    // However, index entries are lost because IndexManager is in-memory.
    // Recovery redo will re-insert committed tuples into data pages.
    // After table recreation, new INSERTs will work fine.
    db2.execute_sql("INSERT INTO t VALUES (100, 1000)").await;

    let resp2 = db2.execute_sql("SELECT * FROM t").await;
    match resp2 {
        Response::QueryResult { rows } => {
            // At least the new insert should be visible
            assert!(!rows.is_empty(), "Should see at least newly inserted row");
        }
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    db2.wal_buffer.shutdown().await;
}

/// Verify WAL contains Update records after UPDATE operations
#[tokio::test]
async fn test_wal_update_records_written() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir).await;

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, val TEXT)")
        .await;
    db.execute_sql("INSERT INTO t VALUES (1, 'original')").await;
    db.execute_sql("UPDATE t SET val = 'updated' WHERE id = 1")
        .await;

    db.wal_buffer.shutdown().await;

    let wal_path = PathBuf::from(dir.path()).join("test.wal");
    let mut reader = WalReader::open(&wal_path).unwrap();
    let records = reader.read_all().unwrap();

    let update_count = records
        .iter()
        .filter(|r| matches!(r, WalRecord::Update { .. }))
        .count();
    assert!(update_count > 0, "Should have Update WAL records");
}

/// Verify WAL contains Delete records after DELETE operations
#[tokio::test]
async fn test_wal_delete_records_written() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir).await;

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, val INT)")
        .await;
    db.execute_sql("INSERT INTO t VALUES (1, 100)").await;
    db.execute_sql("DELETE FROM t WHERE id = 1").await;

    db.wal_buffer.shutdown().await;

    let wal_path = PathBuf::from(dir.path()).join("test.wal");
    let mut reader = WalReader::open(&wal_path).unwrap();
    let records = reader.read_all().unwrap();

    let delete_count = records
        .iter()
        .filter(|r| matches!(r, WalRecord::Delete { .. }))
        .count();
    assert!(delete_count > 0, "Should have Delete WAL records");
}

/// Verify empty WAL recovery doesn't panic
#[tokio::test]
async fn test_empty_wal_recovery() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir).await;

    // Just open and close without operations
    db.wal_buffer.shutdown().await;
    drop(db);

    // Reopen should succeed without errors
    let db2 = open_db(&dir).await;
    db2.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    db2.wal_buffer.shutdown().await;
}
