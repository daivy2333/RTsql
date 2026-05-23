// BTree core logic tests (Task 5)
use std::sync::Arc;
use tempfile::tempdir;

use rtsql::storage::page_format::RowId;
use rtsql::storage::{
    btree::{BTree, LeafNode, SyncPageLoader, LEAF_NODE},
    BufferPool, FileStorage, StorageError,
};

/// Helper to create a test BTree with buffer pool inside spawn_blocking
fn create_test_btree() -> Arc<BufferPool> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    Arc::new(BufferPool::new(10, storage).unwrap())
}

#[tokio::test]
async fn test_btree_new_creates_empty_leaf_root() {
    let buffer_pool = create_test_btree();
    let buffer_pool_clone1 = buffer_pool.clone();
    let buffer_pool_clone2 = buffer_pool.clone();

    // Create BTree and verify root is empty LeafNode
    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(buffer_pool_clone1));
        let mut btree = BTree::new(loader).unwrap();
        let root_id = btree.root_page_id();

        // Load root page and verify it's LEAF_NODE
        let loader2 = Arc::new(SyncPageLoader::new(buffer_pool_clone2));
        let guard = loader2.load_page(root_id).unwrap();
        let page = guard.page();

        // Check first byte is LEAF_NODE
        assert_eq!(page.data[0], LEAF_NODE);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_btree_search_empty_tree_returns_none() {
    let buffer_pool = create_test_btree();
    let buffer_pool_clone = buffer_pool.clone();

    let result = tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(buffer_pool_clone));
        let mut btree = BTree::new(loader).unwrap();
        btree.search(b"key1")
    })
    .await
    .unwrap();

    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn test_btree_insert_and_search_single_key() {
    let buffer_pool = create_test_btree();
    let buffer_pool_clone = buffer_pool.clone();

    let result = tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(buffer_pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        // Insert key
        btree.insert(b"key1", RowId::new(1, 0)).unwrap();

        // Search for key
        btree.search(b"key1").unwrap()
    })
    .await
    .unwrap();

    assert_eq!(result, Some(RowId::new(1, 0)));
}

#[tokio::test]
async fn test_btree_insert_multiple_keys_ordered_search() {
    let buffer_pool = create_test_btree();
    let buffer_pool_clone = buffer_pool.clone();

    let results = tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(buffer_pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        // Insert keys in different order
        btree.insert(b"key3", RowId::new(3, 0)).unwrap();
        btree.insert(b"key1", RowId::new(1, 0)).unwrap();
        btree.insert(b"key2", RowId::new(2, 0)).unwrap();

        // Search all keys
        (
            btree.search(b"key1").unwrap(),
            btree.search(b"key2").unwrap(),
            btree.search(b"key3").unwrap(),
        )
    })
    .await
    .unwrap();

    assert_eq!(results.0, Some(RowId::new(1, 0)));
    assert_eq!(results.1, Some(RowId::new(2, 0)));
    assert_eq!(results.2, Some(RowId::new(3, 0)));
}

#[tokio::test]
async fn test_btree_insert_duplicate_key_returns_error() {
    let buffer_pool = create_test_btree();
    let buffer_pool_clone = buffer_pool.clone();

    let result = tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(buffer_pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        // Insert key
        btree.insert(b"key1", RowId::new(1, 0)).unwrap();

        // Insert duplicate
        btree.insert(b"key1", RowId::new(2, 0))
    })
    .await
    .unwrap();

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), StorageError::DuplicateKey));
}

#[tokio::test]
async fn test_btree_delete_existing_key() {
    let buffer_pool = create_test_btree();
    let buffer_pool_clone = buffer_pool.clone();

    let result = tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(buffer_pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        // Insert key
        btree.insert(b"key1", RowId::new(1, 0)).unwrap();

        // Delete key
        btree.delete(b"key1").unwrap();

        // Search should return None
        btree.search(b"key1").unwrap()
    })
    .await
    .unwrap();

    assert!(result.is_none());
}

#[tokio::test]
async fn test_btree_delete_nonexistent_key_returns_error() {
    let buffer_pool = create_test_btree();
    let buffer_pool_clone = buffer_pool.clone();

    let result = tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(buffer_pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        btree.delete(b"key1")
    })
    .await
    .unwrap();

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), StorageError::KeyNotFound));
}

#[tokio::test]
async fn test_btree_update_existing_key() {
    let buffer_pool = create_test_btree();
    let buffer_pool_clone = buffer_pool.clone();

    let result = tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(buffer_pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        // Insert key
        btree.insert(b"key1", RowId::new(1, 0)).unwrap();

        // Update key with new RowId
        btree.update(b"key1", RowId::new(5, 10)).unwrap();

        // Search should return new RowId
        btree.search(b"key1").unwrap()
    })
    .await
    .unwrap();

    assert_eq!(result, Some(RowId::new(5, 10)));
}

#[tokio::test]
async fn test_btree_update_nonexistent_key_returns_error() {
    let buffer_pool = create_test_btree();
    let buffer_pool_clone = buffer_pool.clone();

    let result = tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(buffer_pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        btree.update(b"key1", RowId::new(5, 10))
    })
    .await
    .unwrap();

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), StorageError::KeyNotFound));
}

#[tokio::test]
async fn test_btree_persists_changes_to_disk() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let storage = Arc::new(FileStorage::open(&db_path).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());
    let buffer_pool_clone1 = buffer_pool.clone();
    let buffer_pool_clone2 = buffer_pool.clone();

    // Insert key and get root_page_id
    let root_page_id = tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(buffer_pool_clone1));
        let mut btree = BTree::new(loader).unwrap();
        let root_id = btree.root_page_id();
        btree.insert(b"key1", RowId::new(1, 0)).unwrap();
        root_id
    })
    .await
    .unwrap();

    // Sync all pages to disk
    buffer_pool.flush_all().await.unwrap();

    // Create new BufferPool and verify data persisted in the original root page
    let storage2 = Arc::new(FileStorage::open(&db_path).unwrap());
    let buffer_pool2 = Arc::new(BufferPool::new(10, storage2).unwrap());
    let buffer_pool2_clone = buffer_pool2.clone();

    let result = tokio::task::spawn_blocking(move || {
        // Load the original root page directly
        let loader = Arc::new(SyncPageLoader::new(buffer_pool2_clone));
        let guard = loader.load_page(root_page_id).unwrap();

        // Create a LeafNode from the page
        guard.modify_page(|page_mut| {
            let leaf = LeafNode::from_page(page_mut).unwrap();

            // Search for the key
            let key = rtsql::storage::page_format::Key::new(b"key1");
            let pos = leaf.find_key_position(&key);

            if pos < leaf.key_count() {
                if let Some(existing_key) = leaf.get_key(pos) {
                    if existing_key == key {
                        return leaf.get_row_id(pos);
                    }
                }
            }
            None
        })
    })
    .await
    .unwrap();

    assert_eq!(result, Some(RowId::new(1, 0)));
}
