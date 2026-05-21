//! M10 Phase 2: Test tx_versions recording by InsertExecutor/UpdateExecutor
//!
//! These tests verify that when InsertExecutor and UpdateExecutor create new versions,
//! they correctly record them in TransactionManager's tx_versions map.

use rtsql::executor::{ExecResult, Executor, InsertExecutor, UpdateExecutor, Value};
use rtsql::storage::{
    data::TableManager, page_format::ColumnType, BufferPool, FileStorage, Result,
};
use rtsql::transaction::TransactionManager;
use std::sync::Arc;
use tempfile::tempdir;

/// Test that InsertExecutor records versions in tx_versions
#[tokio::test]
async fn test_insert_records_version() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let tx_manager = Arc::new(TransactionManager::new());

    // Insert a row with tx_id = 5
    let values = vec![vec![Value::Int(1)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        5,
    );
    insert_executor.next().await?;

    // Verify tx_versions has entry for tx_id = 5
    let versions = tx_manager.get_tx_versions(5).await;
    assert_eq!(versions.len(), 1, "tx_id=5 should have exactly 1 version recorded");

    // Verify the recorded row_id is correct
    let key = 1i64.to_be_bytes();
    let row_id = table_meta.index_manager.search(&key).await?.expect("row should exist");
    assert!(versions.contains(&row_id), "tx_versions should contain the row_id");

    Ok(())
}

/// Test that UpdateExecutor records versions in tx_versions
#[tokio::test]
async fn test_update_records_version() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table(
            "test",
            vec![
                ("id".to_string(), ColumnType::Int),
                ("name".to_string(), ColumnType::String(100)),
            ],
            "id",
        )
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let tx_manager = Arc::new(TransactionManager::new());

    // Insert a row with tx_id = 1
    let values = vec![vec![Value::Int(1), Value::String("alice".to_string())]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        1,
    );
    insert_executor.next().await?;

    // Verify tx_id = 1 has 1 version
    let versions_tx1 = tx_manager.get_tx_versions(1).await;
    assert_eq!(versions_tx1.len(), 1);

    // Update the row with tx_id = 2
    let key = 1i64.to_be_bytes();
    let mut update_executor = UpdateExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        key.to_vec(),
        "name".to_string(),
        Value::String("bob".to_string()),
        2,
    );
    update_executor.next().await?;

    // Verify tx_id = 2 has 1 version recorded
    let versions_tx2 = tx_manager.get_tx_versions(2).await;
    assert_eq!(versions_tx2.len(), 1, "tx_id=2 should have exactly 1 version recorded");

    // Verify the new row_id is in tx_id=2's versions
    let new_row_id = table_meta.index_manager.search(&key).await?.expect("row should exist");
    assert!(versions_tx2.contains(&new_row_id), "tx_versions should contain the new row_id");

    // tx_id = 1 should still have its version (old row)
    let versions_tx1_after = tx_manager.get_tx_versions(1).await;
    assert_eq!(versions_tx1_after.len(), 1, "tx_id=1 should still have its version");

    Ok(())
}

/// Test multiple inserts in same transaction record multiple versions
#[tokio::test]
async fn test_multiple_inserts_multiple_versions() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let tx_manager = Arc::new(TransactionManager::new());

    // Insert 3 rows in the same transaction (tx_id = 10)
    let values = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
    ];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        10,
    );
    insert_executor.next().await?;

    // Verify tx_id = 10 has 3 versions
    let versions = tx_manager.get_tx_versions(10).await;
    assert_eq!(versions.len(), 3, "tx_id=10 should have 3 versions recorded");

    // Verify all row_ids are correct
    for i in 1i64..=3 {
        let key = i.to_be_bytes();
        let row_id = table_meta.index_manager.search(&key).await?.expect("row should exist");
        assert!(versions.contains(&row_id), "tx_versions should contain row_id for key {}", i);
    }

    Ok(())
}

/// Test batch insert records all versions
#[tokio::test]
async fn test_batch_insert_records_all_versions() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let tx_manager = Arc::new(TransactionManager::new());

    // Batch insert 5 rows with tx_id = 100
    let values = vec![
        vec![Value::Int(10)],
        vec![Value::Int(20)],
        vec![Value::Int(30)],
        vec![Value::Int(40)],
        vec![Value::Int(50)],
    ];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        100,
    );
    let result = insert_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(5)));

    // Verify tx_id = 100 has 5 versions
    let versions = tx_manager.get_tx_versions(100).await;
    assert_eq!(versions.len(), 5, "tx_id=100 should have 5 versions recorded");

    Ok(())
}

/// Test that different transactions have separate version tracking
#[tokio::test]
async fn test_different_tx_separate_versions() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let tx_manager = Arc::new(TransactionManager::new());

    // Insert row 1 with tx_id = 1
    let mut insert1 = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        vec![vec![Value::Int(1)]],
        1,
    );
    insert1.next().await?;

    // Insert row 2 with tx_id = 2
    let mut insert2 = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        vec![vec![Value::Int(2)]],
        2,
    );
    insert2.next().await?;

    // Insert row 3 with tx_id = 3
    let mut insert3 = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        vec![vec![Value::Int(3)]],
        3,
    );
    insert3.next().await?;

    // Verify each tx_id has its own version entry
    let versions_tx1 = tx_manager.get_tx_versions(1).await;
    let versions_tx2 = tx_manager.get_tx_versions(2).await;
    let versions_tx3 = tx_manager.get_tx_versions(3).await;

    assert_eq!(versions_tx1.len(), 1);
    assert_eq!(versions_tx2.len(), 1);
    assert_eq!(versions_tx3.len(), 1);

    // Verify all row_ids are correct
    let row_id1 = table_meta.index_manager.search(&1i64.to_be_bytes()).await?.expect("row 1");
    let row_id2 = table_meta.index_manager.search(&2i64.to_be_bytes()).await?.expect("row 2");
    let row_id3 = table_meta.index_manager.search(&3i64.to_be_bytes()).await?.expect("row 3");

    assert!(versions_tx1.contains(&row_id1));
    assert!(versions_tx2.contains(&row_id2));
    assert!(versions_tx3.contains(&row_id3));

    // tx_versions map should have 3 entries
    let all_versions = tx_manager.tx_versions().await;
    assert_eq!(all_versions.len(), 3, "tx_versions should have entries for 3 transactions");

    Ok(())
}