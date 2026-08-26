// SyncPageLoader tests
use std::sync::Arc;
use tempfile::tempdir;

use rtsql::storage::{AsyncStorage, BufferPool, FileStorage, SyncPageLoader};

#[tokio::test]
async fn test_sync_page_loader_load_page() {
    // Setup: Create storage and buffer pool
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());

    // Allocate a page first
    let page_id = storage.allocate_page().await.unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    // Create SyncPageLoader inside Tokio runtime context
    let loader = Arc::new(SyncPageLoader::new(buffer_pool));

    // Test: Load the allocated page synchronously within spawn_blocking
    let loader_clone = loader.clone();
    let guard = tokio::task::spawn_blocking(move || loader_clone.load_page(page_id).unwrap())
        .await
        .unwrap();

    assert_eq!(guard.page().id, page_id);
}

#[tokio::test]
async fn test_sync_page_loader_allocate_page() {
    // Setup
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    // Create SyncPageLoader inside Tokio runtime context
    let loader = Arc::new(SyncPageLoader::new(buffer_pool));

    // Test: Allocate page synchronously within spawn_blocking
    let loader_clone = loader.clone();
    let page_id = tokio::task::spawn_blocking(move || loader_clone.allocate_page().unwrap())
        .await
        .unwrap();

    // Verify the allocated page can be loaded
    let loader_clone2 = loader.clone();
    let guard = tokio::task::spawn_blocking(move || loader_clone2.load_page(page_id).unwrap())
        .await
        .unwrap();

    assert_eq!(guard.page().id, page_id);
}
