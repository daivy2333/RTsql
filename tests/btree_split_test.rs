// BTree split and non-unique index tests (M17)
use std::sync::Arc;
use tempfile::tempdir;

use rtsql::storage::page_format::{Key, RowId};
use rtsql::storage::{
    btree::{BTree, SyncPageLoader, LeafNodeRef, LEAF_NODE},
    BufferPool, FileStorage,
};

/// Helper: create a test BufferPool
fn create_test_pool() -> Arc<BufferPool> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    Arc::new(BufferPool::new(100, storage).unwrap())
}

#[tokio::test]
async fn test_non_unique_insert() {
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let btree = BTree::new(loader).unwrap();

        // Same key inserted multiple times (should succeed)
        let key = b"same_key";
        let row_id1 = RowId::new(1, 0);
        let row_id2 = RowId::new(2, 0);
        let row_id3 = RowId::new(3, 0);

        btree.insert(key, row_id1).unwrap();
        btree.insert(key, row_id2).unwrap();  // Would fail before, should succeed now
        btree.insert(key, row_id3).unwrap();  // Would fail before, should succeed now

        // Verify all insertions succeeded
        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_find_all_matches() {
    let pool = create_test_pool();
    let pool_clone1 = pool.clone();
    let pool_clone2 = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone1));
        let btree = BTree::new(loader).unwrap();

        // Insert duplicate keys
        let key = b"test_key";
        btree.insert(key, RowId::new(1, 0)).unwrap();
        btree.insert(key, RowId::new(2, 1)).unwrap();
        btree.insert(key, RowId::new(3, 2)).unwrap();

        // Insert different key
        btree.insert(b"other_key", RowId::new(4, 0)).unwrap();

        // Load root page and verify find_all_matches
        let root_id = btree.root_page_id();
        let loader2 = Arc::new(SyncPageLoader::new(pool_clone2));
        let guard = loader2.load_page(root_id).unwrap();
        let data = guard.page_data();
        let leaf_ref = LeafNodeRef::new(&data);

        // Find all matches for "test_key"
        let key_obj = Key::new(key);
        let matches = leaf_ref.find_all_matches(&key_obj);
        assert_eq!(matches.len(), 3);

        // Verify no matches for non-existent key
        let no_match = leaf_ref.find_all_matches(&Key::new(b"no_key"));
        assert_eq!(no_match.len(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_search_all_matches() {
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let btree = BTree::new(loader).unwrap();

        // Insert duplicate keys
        let key = b"multi_key";
        btree.insert(key, RowId::new(10, 0)).unwrap();
        btree.insert(key, RowId::new(20, 1)).unwrap();
        btree.insert(key, RowId::new(30, 2)).unwrap();

        // Insert other key
        btree.insert(b"single_key", RowId::new(40, 0)).unwrap();

        // Verify search_all returns all matching RowIds
        let results = btree.search_all(key).unwrap();
        assert_eq!(results.len(), 3);
        assert!(results.contains(&RowId::new(10, 0)));
        assert!(results.contains(&RowId::new(20, 1)));
        assert!(results.contains(&RowId::new(30, 2)));

        // Verify single key query
        let single = btree.search_all(b"single_key").unwrap();
        assert_eq!(single.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_delete_by_key() {
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let btree = BTree::new(loader).unwrap();

        // Insert duplicate keys
        let key = b"del_key";
        btree.insert(key, RowId::new(1, 0)).unwrap();
        btree.insert(key, RowId::new(2, 1)).unwrap();
        btree.insert(key, RowId::new(3, 2)).unwrap();

        // Verify insertion succeeded
        assert_eq!(btree.search_all(key).unwrap().len(), 3);

        // Delete all matches
        let deleted_count = btree.delete_by_key(key).unwrap();
        assert_eq!(deleted_count, 3);

        // Verify deleted
        let remaining = btree.search_all(key).unwrap();
        assert_eq!(remaining.len(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_delete_exact() {
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let btree = BTree::new(loader).unwrap();

        // Insert duplicate keys
        let key = b"exact_key";
        btree.insert(key, RowId::new(1, 0)).unwrap();
        btree.insert(key, RowId::new(2, 1)).unwrap();
        btree.insert(key, RowId::new(3, 2)).unwrap();

        // Exact delete middle one
        btree.delete_exact(key, RowId::new(2, 1)).unwrap();

        // Verify remaining two
        let remaining = btree.search_all(key).unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.contains(&RowId::new(1, 0)));
        assert!(remaining.contains(&RowId::new(3, 2)));
        assert!(!remaining.contains(&RowId::new(2, 1)));
    })
    .await
    .unwrap();
}