// Test for IndexManager async API (Task 6)
use rtsql::storage::{btree::IndexManager, page_format::RowId, BufferPool, FileStorage};
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_index_manager_basic_ops() {
    // Setup: create storage and buffer pool
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    // Create IndexManager inside spawn_blocking (BTree::new uses block_on internally)
    let buffer_pool_clone = buffer_pool.clone();
    let index = tokio::task::spawn_blocking(move || IndexManager::new(buffer_pool_clone).unwrap())
        .await
        .unwrap();

    // Test insert and search
    index.insert(b"key1", RowId::new(1, 0)).await.unwrap();
    let result = index.search(b"key1").await.unwrap();
    assert_eq!(result, Some(RowId::new(1, 0)));

    // Test non-existent key
    let result = index.search(b"key_not_found").await.unwrap();
    assert_eq!(result, None);

    // Test multiple inserts
    index.insert(b"key2", RowId::new(2, 1)).await.unwrap();
    index.insert(b"key3", RowId::new(3, 2)).await.unwrap();

    let result = index.search(b"key2").await.unwrap();
    assert_eq!(result, Some(RowId::new(2, 1)));

    let result = index.search(b"key3").await.unwrap();
    assert_eq!(result, Some(RowId::new(3, 2)));
}

#[tokio::test]
async fn test_index_manager_delete() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    // Create IndexManager inside spawn_blocking
    let buffer_pool_clone = buffer_pool.clone();
    let index = tokio::task::spawn_blocking(move || IndexManager::new(buffer_pool_clone).unwrap())
        .await
        .unwrap();

    // Insert a key
    index.insert(b"key1", RowId::new(1, 0)).await.unwrap();
    let result = index.search(b"key1").await.unwrap();
    assert_eq!(result, Some(RowId::new(1, 0)));

    // Delete the key
    index.delete(b"key1").await.unwrap();
    let result = index.search(b"key1").await.unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn test_index_manager_update() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    // Create IndexManager inside spawn_blocking
    let buffer_pool_clone = buffer_pool.clone();
    let index = tokio::task::spawn_blocking(move || IndexManager::new(buffer_pool_clone).unwrap())
        .await
        .unwrap();

    // Insert a key
    index.insert(b"key1", RowId::new(1, 0)).await.unwrap();
    let result = index.search(b"key1").await.unwrap();
    assert_eq!(result, Some(RowId::new(1, 0)));

    // Update the RowId
    index.update(b"key1", RowId::new(2, 5)).await.unwrap();
    let result = index.search(b"key1").await.unwrap();
    assert_eq!(result, Some(RowId::new(2, 5)));

    // Update non-existent key should fail
    let result = index.update(b"key_not_found", RowId::new(3, 0)).await;
    assert!(result.is_err());
}
