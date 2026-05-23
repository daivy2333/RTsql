// BTree split and non-unique index tests (M17)
use std::sync::Arc;
use tempfile::tempdir;

use rtsql::storage::page_format::RowId;
use rtsql::storage::{
    btree::{BTree, SyncPageLoader, LEAF_NODE},
    BufferPool, FileStorage, PageId,
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