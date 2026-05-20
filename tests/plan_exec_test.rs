//! Integration tests: PhysicalPlan -> Executor -> Result

use rtsql::executor::{
    DeleteExecutor, ExecResult, Executor, IndexScanExecutor, InsertExecutor, Value,
};
use rtsql::storage::{btree::IndexManager, BufferPool, FileStorage, Result};
use std::sync::Arc;
use tempfile::tempdir;

/// Test full flow: Insert -> IndexScan (confirm) -> Delete -> IndexScan (confirm deleted)
#[tokio::test]
async fn test_full_flow_insert_find_delete() -> Result<()> {
    // Setup storage
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    // Create IndexManager inside spawn_blocking (BTree::new uses block_on internally)
    let buffer_pool_clone = buffer_pool.clone();
    let index_manager = tokio::task::spawn_blocking(move || {
        Arc::new(IndexManager::new(buffer_pool_clone).unwrap())
    })
    .await
    .unwrap();

    // 1. Insert a row
    let key_bytes = 100i64.to_be_bytes();
    let values = vec![vec![Value::Int(100)]];
    let mut insert_executor = InsertExecutor::new(index_manager.clone(), values);
    let result = insert_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    // 2. IndexScan - confirm row exists
    let mut scan_executor = IndexScanExecutor::new(index_manager.clone(), key_bytes.to_vec());
    let result = scan_executor.next().await?;
    assert!(
        matches!(result, Some(ExecResult::RowId(_))),
        "Expected RowId but got {:?}",
        result
    );

    // 3. Delete the row
    let mut delete_executor = DeleteExecutor::new(index_manager.clone(), key_bytes.to_vec());
    let result = delete_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    // 4. IndexScan - confirm deleted
    let mut scan_executor = IndexScanExecutor::new(index_manager, key_bytes.to_vec());
    let result = scan_executor.next().await?;
    assert_eq!(
        result, None,
        "Expected None after deletion but got {:?}",
        result
    );

    Ok(())
}

/// Test: Insert -> IndexScan with multiple rows
#[tokio::test]
async fn test_insert_then_index_scan() -> Result<()> {
    // Setup storage
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    // Create IndexManager inside spawn_blocking
    let buffer_pool_clone = buffer_pool.clone();
    let index_manager = tokio::task::spawn_blocking(move || {
        Arc::new(IndexManager::new(buffer_pool_clone).unwrap())
    })
    .await
    .unwrap();

    // Insert multiple rows
    let values = vec![
        vec![Value::Int(42)],
        vec![Value::Int(100)],
        vec![Value::Int(200)],
    ];
    let mut insert_executor = InsertExecutor::new(index_manager.clone(), values);
    let result = insert_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(3)));

    // IndexScan for key 42
    let key_42 = 42i64.to_be_bytes();
    let mut scan_executor = IndexScanExecutor::new(index_manager.clone(), key_42.to_vec());
    let result = scan_executor.next().await?;
    assert!(matches!(result, Some(ExecResult::RowId(_))));

    // IndexScan for key 100
    let key_100 = 100i64.to_be_bytes();
    let mut scan_executor = IndexScanExecutor::new(index_manager.clone(), key_100.to_vec());
    let result = scan_executor.next().await?;
    assert!(matches!(result, Some(ExecResult::RowId(_))));

    // IndexScan for non-existent key
    let key_999 = 999i64.to_be_bytes();
    let mut scan_executor = IndexScanExecutor::new(index_manager, key_999.to_vec());
    let result = scan_executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}

/// Test: Insert -> Update -> IndexScan (confirm update)
#[tokio::test]
async fn test_insert_update_scan_flow() -> Result<()> {
    use rtsql::executor::UpdateExecutor;

    // Setup storage
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    // Create IndexManager inside spawn_blocking
    let buffer_pool_clone = buffer_pool.clone();
    let index_manager = tokio::task::spawn_blocking(move || {
        Arc::new(IndexManager::new(buffer_pool_clone).unwrap())
    })
    .await
    .unwrap();

    // 1. Insert a row
    let key_bytes = 1i64.to_be_bytes();
    let values = vec![vec![Value::Int(1)]];
    let mut insert_executor = InsertExecutor::new(index_manager.clone(), values);
    let result = insert_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    // 2. Update the row
    let new_value = Value::Int(1000);
    let mut update_executor =
        UpdateExecutor::new(index_manager.clone(), key_bytes.to_vec(), new_value);
    let result = update_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    // 3. IndexScan - confirm row still exists (update doesn't change key)
    let mut scan_executor = IndexScanExecutor::new(index_manager, key_bytes.to_vec());
    let result = scan_executor.next().await?;
    assert!(matches!(result, Some(ExecResult::RowId(_))));

    Ok(())
}

/// Test: Multiple operations in sequence
#[tokio::test]
async fn test_multiple_operations_sequence() -> Result<()> {
    // Setup storage
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    // Create IndexManager inside spawn_blocking
    let buffer_pool_clone = buffer_pool.clone();
    let index_manager = tokio::task::spawn_blocking(move || {
        Arc::new(IndexManager::new(buffer_pool_clone).unwrap())
    })
    .await
    .unwrap();

    // Insert rows 1, 2, 3
    for i in 1i64..=3 {
        let values = vec![vec![Value::Int(i)]];
        let mut insert_executor = InsertExecutor::new(index_manager.clone(), values);
        let result = insert_executor.next().await?;
        assert_eq!(result, Some(ExecResult::AffectedRows(1)));
    }

    // Verify all exist
    for i in 1i64..=3 {
        let key = i.to_be_bytes().to_vec();
        let mut scan_executor = IndexScanExecutor::new(index_manager.clone(), key);
        let result = scan_executor.next().await?;
        assert!(
            matches!(result, Some(ExecResult::RowId(_))),
            "Row {} should exist",
            i
        );
    }

    // Delete row 2
    let key_2 = 2i64.to_be_bytes().to_vec();
    let mut delete_executor = DeleteExecutor::new(index_manager.clone(), key_2);
    let result = delete_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    // Verify 2 is deleted, 1 and 3 still exist
    let mut scan_2 = IndexScanExecutor::new(index_manager.clone(), 2i64.to_be_bytes().to_vec());
    assert_eq!(scan_2.next().await?, None, "Row 2 should be deleted");

    let mut scan_1 = IndexScanExecutor::new(index_manager.clone(), 1i64.to_be_bytes().to_vec());
    assert!(matches!(scan_1.next().await?, Some(ExecResult::RowId(_))));

    let mut scan_3 = IndexScanExecutor::new(index_manager, 3i64.to_be_bytes().to_vec());
    assert!(matches!(scan_3.next().await?, Some(ExecResult::RowId(_))));

    Ok(())
}
