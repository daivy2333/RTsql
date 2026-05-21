//! Integration tests: PhysicalPlan -> Executor -> Result

use rtsql::executor::{
    DeleteExecutor, ExecResult, Executor, IndexScanExecutor, InsertExecutor, Value,
};
use rtsql::storage::{
    data::TableManager, page_format::ColumnType, BufferPool, FileStorage, Result,
};
use rtsql::transaction::TransactionManager;
use std::sync::Arc;
use tempfile::tempdir;

/// Test full flow: Insert -> IndexScan (confirm) -> Delete -> IndexScan (confirm deleted)
#[tokio::test]
async fn test_full_flow_insert_find_delete() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let index_manager = table_meta.index_manager.clone();

    let key_bytes = 100i64.to_be_bytes();
    let values = vec![vec![Value::Int(100)]];
    let mut insert_executor =
        InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), tx_manager, values, 0);
    let result = insert_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let mut scan_executor = IndexScanExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        key_bytes.to_vec(),
        None,
    );
    let result = scan_executor.next().await?;
    assert!(
        matches!(result, Some(ExecResult::Row(_))),
        "Expected Row but got {:?}",
        result
    );

    let mut delete_executor = DeleteExecutor::new(index_manager.clone(), key_bytes.to_vec(), 0);
    let result = delete_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let mut scan_executor = IndexScanExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        key_bytes.to_vec(),
        None,
    );
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
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![
        vec![Value::Int(42)],
        vec![Value::Int(100)],
        vec![Value::Int(200)],
    ];
    let mut insert_executor =
        InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), tx_manager, values, 0);
    let result = insert_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(3)));

    let key_42 = 42i64.to_be_bytes();
    let mut scan_executor = IndexScanExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        key_42.to_vec(),
        None,
    );
    let result = scan_executor.next().await?;
    assert!(matches!(result, Some(ExecResult::Row(_))));

    let key_100 = 100i64.to_be_bytes();
    let mut scan_executor = IndexScanExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        key_100.to_vec(),
        None,
    );
    let result = scan_executor.next().await?;
    assert!(matches!(result, Some(ExecResult::Row(_))));

    let key_999 = 999i64.to_be_bytes();
    let mut scan_executor =
        IndexScanExecutor::new(table_meta, buffer_pool.clone(), key_999.to_vec(), None);
    let result = scan_executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}

/// Test: Insert -> Update -> IndexScan (confirm update)
#[tokio::test]
async fn test_insert_update_scan_flow() -> Result<()> {
    use rtsql::executor::{IndexScanExecutor, UpdateExecutor};

    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let key_bytes = 1i64.to_be_bytes();
    let values = vec![vec![Value::Int(1)]];
    let mut insert_executor =
        InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), tx_manager.clone(), values, 0);
    let result = insert_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let new_value = Value::Int(1000);
    let mut update_executor = UpdateExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        key_bytes.to_vec(),
        "id".to_string(),
        new_value,
        0,
    );
    let result = update_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let mut scan = IndexScanExecutor::new(table_meta, buffer_pool, key_bytes.to_vec(), None);
    let result = scan.next().await?;
    assert!(
        matches!(result, Some(ExecResult::Row(ref values)) if values[0] == Value::Int(1000)),
        "Expected Row with Int(1000) after update, got {:?}",
        result
    );

    Ok(())
}

/// Test: Multiple operations in sequence
#[tokio::test]
async fn test_multiple_operations_sequence() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let index_manager = table_meta.index_manager.clone();

    for i in 1i64..=3 {
        let values = vec![vec![Value::Int(i)]];
        let mut insert_executor =
            InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), tx_manager.clone(), values, 0);
        let result = insert_executor.next().await?;
        assert_eq!(result, Some(ExecResult::AffectedRows(1)));
    }

    for i in 1i64..=3 {
        let key = i.to_be_bytes().to_vec();
        let mut scan_executor =
            IndexScanExecutor::new(table_meta.clone(), buffer_pool.clone(), key, None);
        let result = scan_executor.next().await?;
        assert!(
            matches!(result, Some(ExecResult::Row(_))),
            "Row {} should exist",
            i
        );
    }

    let key_2 = 2i64.to_be_bytes().to_vec();
    let mut delete_executor = DeleteExecutor::new(index_manager.clone(), key_2, 0);
    let result = delete_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let mut scan_2 = IndexScanExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        2i64.to_be_bytes().to_vec(),
        None,
    );
    assert_eq!(scan_2.next().await?, None, "Row 2 should be deleted");

    let mut scan_1 = IndexScanExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        1i64.to_be_bytes().to_vec(),
        None,
    );
    assert!(matches!(scan_1.next().await?, Some(ExecResult::Row(_))));

    let mut scan_3 = IndexScanExecutor::new(
        table_meta,
        buffer_pool.clone(),
        3i64.to_be_bytes().to_vec(),
        None,
    );
    assert!(matches!(scan_3.next().await?, Some(ExecResult::Row(_))));

    Ok(())
}
