//! M10 Phase 4: Test version chain traversal
//!
//! These tests verify that:
//! - Version chains are correctly traversed
//! - Visibility rules work across multiple versions
//! - Repeatable Read isolation level is maintained

use rtsql::executor::{Executor, InsertExecutor, UpdateExecutor, Value};
use rtsql::storage::{
    data::TableManager, page_format::ColumnType, BufferPool, FileStorage, Result,
};
use rtsql::transaction::{Snapshot, TransactionManager};
use std::sync::Arc;
use tempfile::tempdir;

/// Test comprehensive version chain traversal with multiple updates
///
/// Scenario:
/// 1. Tx1 inserts v1 (value=10), commits
/// 2. Tx2 updates to v2 (value=20), commits
/// 3. Tx3 updates to v3 (value=30), does NOT commit
/// 4. Tx4 (snapshot before Tx3) should see v2 (value=20)
/// 5. Tx3 commits
/// 6. Tx4 still sees v2 (Repeatable Read)
/// 7. Tx5 (new tx) sees v3 (value=30)
#[tokio::test]
async fn test_version_chain_traversal() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let tx_manager = Arc::new(TransactionManager::new());

    // Step 1: Tx1 inserts v1 (value=10), commits
    let tx1 = tx_manager.begin().await;
    let tx1_id = tx1.id();

    let values = vec![vec![Value::Int(10)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        tx1_id,
        None,
    );
    insert_executor.next().await?;

    // Commit Tx1
    tx_manager.commit(tx1, &buffer_pool).await?;

    // Verify Tx1's version is committed
    let key = 10i64.to_be_bytes();
    let row_id_v1 = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("v1 should exist");
    let version_header_v1 = buffer_pool.read_version_header(row_id_v1).await?;
    assert_eq!(
        version_header_v1.commit_tx_id(),
        Some(tx1_id),
        "v1 should have commit_tx_id set"
    );

    // Step 2: Tx2 updates to v2 (value=20), commits
    let tx2 = tx_manager.begin().await;
    let tx2_id = tx2.id();

    let mut update_executor = UpdateExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        key.to_vec(),
        "id".to_string(),
        Value::Int(20),
        tx2_id,
        None,
    );
    update_executor.next().await?;

    // Get new row_id for v2
    let row_id_v2 = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("v2 should exist");
    assert_ne!(row_id_v1, row_id_v2, "v2 should have different row_id");

    // Verify version chain: v2 -> v1
    let version_header_v2 = buffer_pool.read_version_header(row_id_v2).await?;
    assert_eq!(
        version_header_v2.next_version(),
        Some(row_id_v1),
        "v2 should point to v1"
    );

    // Commit Tx2
    tx_manager.commit(tx2, &buffer_pool).await?;

    // Verify Tx2's version is committed
    let version_header_v2 = buffer_pool.read_version_header(row_id_v2).await?;
    assert_eq!(
        version_header_v2.commit_tx_id(),
        Some(tx2_id),
        "v2 should have commit_tx_id set"
    );

    // Step 3: Tx3 updates to v3 (value=30), does NOT commit yet
    let tx3 = tx_manager.begin().await;
    let tx3_id = tx3.id();

    let mut update_executor = UpdateExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        key.to_vec(),
        "id".to_string(),
        Value::Int(30),
        tx3_id,
        None,
    );
    update_executor.next().await?;

    // Get new row_id for v3
    let row_id_v3 = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("v3 should exist");
    assert_ne!(row_id_v2, row_id_v3, "v3 should have different row_id");

    // Verify version chain: v3 -> v2 -> v1
    let version_header_v3 = buffer_pool.read_version_header(row_id_v3).await?;
    assert_eq!(
        version_header_v3.next_version(),
        Some(row_id_v2),
        "v3 should point to v2"
    );

    // Tx3 is NOT committed yet

    // Step 4: Tx4 (snapshot before Tx3 commits) should see v2 (value=20)
    let tx4 = tx_manager.begin().await;
    let tx4_snapshot = tx4.snapshot();

    // Tx4 should see v2 (committed by Tx2), not v3 (uncommitted by Tx3)
    let visible_tuple = buffer_pool
        .find_visible_version(row_id_v3, &tx4_snapshot)
        .await?;
    assert!(visible_tuple.is_some(), "Tx4 should see a visible version");

    // Deserialize and check value
    let values = rtsql::storage::page_format::deserialize_tuple(
        &visible_tuple.unwrap(),
        &table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect::<Vec<_>>(),
    )?;
    assert_eq!(values[0], Value::Int(20), "Tx4 should see value=20 (v2)");

    // Step 5: Tx3 commits
    tx_manager.commit(tx3, &buffer_pool).await?;

    // Verify Tx3's version is committed
    let version_header_v3 = buffer_pool.read_version_header(row_id_v3).await?;
    assert_eq!(
        version_header_v3.commit_tx_id(),
        Some(tx3_id),
        "v3 should have commit_tx_id set after Tx3 commits"
    );

    // Step 6: Tx4 still sees v2 (Repeatable Read)
    // Tx4's snapshot was taken before Tx3 committed, so it still sees v2
    let visible_tuple_after = buffer_pool
        .find_visible_version(row_id_v3, &tx4_snapshot)
        .await?;
    assert!(
        visible_tuple_after.is_some(),
        "Tx4 should still see a visible version"
    );

    let values_after = rtsql::storage::page_format::deserialize_tuple(
        &visible_tuple_after.unwrap(),
        &table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect::<Vec<_>>(),
    )?;
    assert_eq!(
        values_after[0],
        Value::Int(20),
        "Tx4 should still see value=20 (v2) - Repeatable Read"
    );

    // Step 7: Tx5 (new tx) sees v3 (value=30)
    let tx5 = tx_manager.begin().await;
    let tx5_snapshot = tx5.snapshot();

    let visible_tuple_tx5 = buffer_pool
        .find_visible_version(row_id_v3, &tx5_snapshot)
        .await?;
    assert!(
        visible_tuple_tx5.is_some(),
        "Tx5 should see a visible version"
    );

    let values_tx5 = rtsql::storage::page_format::deserialize_tuple(
        &visible_tuple_tx5.unwrap(),
        &table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect::<Vec<_>>(),
    )?;
    assert_eq!(
        values_tx5[0],
        Value::Int(30),
        "Tx5 should see value=30 (v3)"
    );

    // Cleanup
    tx_manager.commit(tx4, &buffer_pool).await?;
    tx_manager.commit(tx5, &buffer_pool).await?;

    Ok(())
}

/// Test that version chain traversal skips invisible versions correctly
#[tokio::test]
async fn test_version_chain_skips_invisible() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let tx_manager = Arc::new(TransactionManager::new());

    // Tx1 inserts value=100
    let tx1 = tx_manager.begin().await;
    let tx1_id = tx1.id();

    let values = vec![vec![Value::Int(100)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        tx1_id,
        None,
    );
    insert_executor.next().await?;
    tx_manager.commit(tx1, &buffer_pool).await?;

    let key = 100i64.to_be_bytes();

    // Tx2 updates to value=200, but does NOT commit
    let tx2 = tx_manager.begin().await;
    let tx2_id = tx2.id();

    let mut update_executor = UpdateExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        key.to_vec(),
        "id".to_string(),
        Value::Int(200),
        tx2_id,
        None,
    );
    update_executor.next().await?;

    // Tx3 updates to value=300, commits
    let tx3 = tx_manager.begin().await;
    let tx3_id = tx3.id();

    let mut update_executor = UpdateExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        key.to_vec(),
        "id".to_string(),
        Value::Int(300),
        tx3_id,
        None,
    );
    update_executor.next().await?;
    tx_manager.commit(tx3, &buffer_pool).await?;

    // Get latest row_id (v3)
    let row_id_v3 = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("v3 should exist");

    // Tx4 (new transaction) should see v3 (value=300), not v2 (uncommitted)
    let tx4 = tx_manager.begin().await;
    let tx4_snapshot = tx4.snapshot();

    let visible_tuple = buffer_pool
        .find_visible_version(row_id_v3, &tx4_snapshot)
        .await?;
    assert!(visible_tuple.is_some(), "Tx4 should see a visible version");

    let values = rtsql::storage::page_format::deserialize_tuple(
        &visible_tuple.unwrap(),
        &table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect::<Vec<_>>(),
    )?;
    assert_eq!(
        values[0],
        Value::Int(300),
        "Tx4 should see value=300 (v3), not v2 (uncommitted)"
    );

    // Cleanup
    tx_manager.commit(tx4, &buffer_pool).await?;

    Ok(())
}

/// Test that all versions can be invisible (returns None)
#[tokio::test]
async fn test_all_versions_invisible() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let tx_manager = Arc::new(TransactionManager::new());

    // Tx1 inserts value=999, but does NOT commit
    let tx1 = tx_manager.begin().await;
    let tx1_id = tx1.id();

    let values = vec![vec![Value::Int(999)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        tx1_id,
        None,
    );
    insert_executor.next().await?;

    let key = 999i64.to_be_bytes();
    let row_id = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("row should exist");

    // Tx2 (different transaction) should see nothing
    let tx2 = tx_manager.begin().await;
    let tx2_snapshot = tx2.snapshot();

    let visible_tuple = buffer_pool
        .find_visible_version(row_id, &tx2_snapshot)
        .await?;
    assert!(
        visible_tuple.is_none(),
        "Tx2 should see nothing (all versions invisible)"
    );

    Ok(())
}
