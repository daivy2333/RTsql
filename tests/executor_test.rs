//! Executor unit tests

use rtsql::executor::{ExecResult, Executor, IndexScanExecutor, InsertExecutor, ScanExecutor, Value};
use rtsql::storage::{btree::IndexManager, page_format::RowId, BufferPool, FileStorage, Result};
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_scan_executor_returns_not_implemented() -> Result<()> {
    let mut executor = ScanExecutor::new();

    // 第一次 next 返回 NotImplemented
    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::NotImplemented));

    // 第二次 next 返回 None（迭代结束）
    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}

#[tokio::test]
async fn test_index_scan_executor_found() -> Result<()> {
    // 创建临时目录和 FileStorage
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

    // 先插入一条数据
    let key = b"key1";
    let row_id = RowId::new(0, 1);
    index_manager.insert(key, row_id).await.unwrap();

    // 创建 IndexScanExecutor
    let mut executor = IndexScanExecutor::new(index_manager, key.to_vec());

    // 第一次 next 返回 RowId
    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::RowId(row_id)));

    // 第二次 next 返回 None（迭代结束）
    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}

#[tokio::test]
async fn test_index_scan_executor_not_found() -> Result<()> {
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

    // 不插入任何数据，直接查找
    let key = b"key_not_found";
    let mut executor = IndexScanExecutor::new(index_manager, key.to_vec());

    // next 返回 None（未找到）
    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}

#[tokio::test]
async fn test_insert_executor_single_row() -> Result<()> {
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

    // 单行插入
    let values = vec![vec![Value::Int(1)]];
    let mut executor = InsertExecutor::new(index_manager, values);

    // 第一次 next 返回 AffectedRows(1)
    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    // 第二次 next 返回 None
    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}

#[tokio::test]
async fn test_insert_executor_batch() -> Result<()> {
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

    // 批量插入 3 行
    let values = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
    ];
    let mut executor = InsertExecutor::new(index_manager, values);

    // 第一次 next 返回 AffectedRows(3)
    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(3)));

    // 第二次 next 返回 None
    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}
