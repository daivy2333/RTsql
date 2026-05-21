//! M10 Phase 3: Test commit visibility
//!
//! These tests verify that:
//! - Uncommitted versions are not visible to other transactions
//! - After commit, versions become visible

use rtsql::executor::{Executor, InsertExecutor, Value};
use rtsql::storage::{
    data::TableManager, page_format::ColumnType, BufferPool, FileStorage, Result,
};
use rtsql::transaction::{Snapshot, TransactionManager};
use std::sync::Arc;
use tempfile::tempdir;

/// Test that uncommitted version is not visible to other transactions
#[tokio::test]
async fn test_uncommitted_version_not_visible() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let tx_manager = Arc::new(TransactionManager::new());

    // Insert a row with tx_id = 1
    let values = vec![vec![Value::Int(1)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        1,
    );
    insert_executor.next().await?;

    // Get the row_id
    let key = 1i64.to_be_bytes();
    let row_id = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("row should exist");

    // Create a snapshot for tx_id = 2 (which started after tx_id = 1 began)
    // tx_id = 1 is in the active list, so it should not be visible to tx_id = 2
    let snapshot_tx2 = Snapshot::new(2, vec![1]);

    // Check visibility: tx_id = 2 should NOT see the uncommitted version from tx_id = 1
    // Note: commit_tx_id is None because tx_id = 1 has not committed
    let visible = snapshot_tx2.is_visible(1, None);
    assert!(
        !visible,
        "tx_id=2 should not see uncommitted version from tx_id=1"
    );

    // Also test with find_visible_version
    let visible_tuple = buffer_pool
        .find_visible_version(row_id, &snapshot_tx2)
        .await?;
    assert!(
        visible_tuple.is_none(),
        "find_visible_version should return None for uncommitted version"
    );

    Ok(())
}

/// Test that committed version becomes visible to other transactions
#[tokio::test]
async fn test_committed_version_visible() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let tx_manager = Arc::new(TransactionManager::new());

    // Begin tx_id = 1
    let tx1 = tx_manager.begin().await;
    let tx1_id = tx1.id();

    // Insert a row with tx_id = 1
    let values = vec![vec![Value::Int(100)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        tx1_id,
    );
    insert_executor.next().await?;

    // Get the row_id
    let key = 100i64.to_be_bytes();
    let row_id = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("row should exist");

    // Begin tx_id = 2 (after tx_id = 1, before tx_id = 1 commits)
    let tx2 = tx_manager.begin().await;
    let _tx2_id = tx2.id();

    // tx_id = 2's snapshot should see tx_id = 1 as active (uncommitted)
    let snapshot_tx2 = tx2.snapshot();

    // Commit tx_id = 1 (mark all versions as committed)
    tx_manager.commit(tx1, &buffer_pool).await?;

    // Now tx_id = 1's version should have commit_tx_id set
    let version_header = buffer_pool.read_version_header(row_id).await?;
    assert_eq!(
        version_header.commit_tx_id(),
        Some(tx1_id),
        "version should have commit_tx_id set after commit"
    );

    // Create a new snapshot for tx_id = 3 (which starts after tx_id = 1 committed)
    // Active list should not include tx_id = 1
    let active_txs = tx_manager.active_transactions().await;
    assert!(
        !active_txs.contains(&tx1_id),
        "tx_id=1 should not be in active list after commit"
    );

    let tx3 = tx_manager.begin().await;
    let snapshot_tx3 = tx3.snapshot();

    // tx_id = 3 should see tx_id = 1's committed version
    let visible = snapshot_tx3.is_visible(tx1_id, Some(tx1_id));
    assert!(visible, "tx_id=3 should see committed version from tx_id=1");

    // Also test with find_visible_version for tx_id = 3
    let visible_tuple = buffer_pool
        .find_visible_version(row_id, &snapshot_tx3)
        .await?;
    assert!(
        visible_tuple.is_some(),
        "find_visible_version should return the tuple for committed version"
    );

    // But tx_id = 2's snapshot (taken before tx_id=1 committed) should still not see it
    // because tx_id = 1 was in the active list when tx_id = 2's snapshot was taken
    let visible_for_tx2 = snapshot_tx2.is_visible(tx1_id, Some(tx1_id));
    assert!(
        !visible_for_tx2,
        "tx_id=2 should not see tx_id=1's version (snapshot taken before commit)"
    );

    tx_manager.commit(tx2, &buffer_pool).await?;
    tx_manager.commit(tx3, &buffer_pool).await?;

    Ok(())
}

/// Test that tx_versions is cleared after commit
#[tokio::test]
async fn test_tx_versions_cleared_after_commit() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let tx_manager = Arc::new(TransactionManager::new());

    // Begin a transaction
    let tx = tx_manager.begin().await;
    let tx_id = tx.id();

    // Insert a row
    let values = vec![vec![Value::Int(42)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        tx_id,
    );
    insert_executor.next().await?;

    // Verify tx_versions has the version
    let versions_before = tx_manager.get_tx_versions(tx_id).await;
    assert_eq!(
        versions_before.len(),
        1,
        "tx_versions should have 1 version before commit"
    );

    // Commit the transaction
    tx_manager.commit(tx, &buffer_pool).await?;

    // Verify tx_versions is cleared
    let versions_after = tx_manager.get_tx_versions(tx_id).await;
    assert!(
        versions_after.is_empty(),
        "tx_versions should be empty after commit"
    );

    Ok(())
}

/// Test self-visibility: a transaction can always see its own uncommitted versions
#[tokio::test]
async fn test_self_visibility() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let tx_manager = Arc::new(TransactionManager::new());

    // Begin tx_id = 5
    let tx = tx_manager.begin().await;
    let tx_id = tx.id();
    let snapshot = tx.snapshot();

    // Insert a row with this tx_id
    let values = vec![vec![Value::Int(999)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        tx_id,
    );
    insert_executor.next().await?;

    // Get the row_id
    let key = 999i64.to_be_bytes();
    let row_id = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("row should exist");

    // The transaction should be able to see its own uncommitted version
    let visible_self = snapshot.is_visible_self(tx_id, None);
    assert!(
        visible_self,
        "transaction should see its own uncommitted version"
    );

    // Also test with find_visible_version
    let visible_tuple = buffer_pool.find_visible_version(row_id, &snapshot).await?;
    assert!(
        visible_tuple.is_some(),
        "find_visible_version should return the tuple for self-created version"
    );

    // Commit
    tx_manager.commit(tx, &buffer_pool).await?;

    Ok(())
}
