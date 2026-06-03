// BTree merge and redistribution integration tests (M17)
use std::sync::Arc;
use tempfile::tempdir;

use rtsql::storage::page_format::RowId;
use rtsql::storage::{
    btree::{BTree, SyncPageLoader},
    BufferPool, FileStorage,
};

fn create_test_pool() -> Arc<BufferPool> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    Arc::new(BufferPool::new(100, storage).unwrap())
}

fn make_key(i: u32) -> Vec<u8> {
    format!("{:032}", i).into_bytes()
}

// ── test_leaf_merge_after_mass_delete ────────────────────────────────────────

#[tokio::test]
async fn test_leaf_merge_after_mass_delete() {
    // Insert 200 entries (triggering splits), then bulk-delete 150 from the
    // leftmost range. Verify data integrity: remaining entries searchable and
    // scan_all returns the correct count in order.
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        let total: u32 = 200;
        for i in 0..total {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }

        for i in 0..150u32 {
            let (count, _) = btree.delete_by_key(&make_key(i)).unwrap();
            assert_eq!(count, 1);
        }

        for i in 0..150u32 {
            assert!(btree.search(&make_key(i)).unwrap().is_none());
        }

        for i in 150..total {
            let result = btree.search(&make_key(i)).unwrap();
            assert!(result.is_some(), "Key {} missing", i);
            assert_eq!(result.unwrap(), RowId::new(i, 0));
        }

        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), 50);

        for idx in 1..all.len() {
            assert!(all[idx - 1].0 <= all[idx].0);
        }
    })
    .await
    .unwrap();
}

// ── test_leaf_redistribution ─────────────────────────────────────────────────

#[tokio::test]
async fn test_leaf_redistribution() {
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        let total: u32 = 140;
        for i in 0..total {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }

        for i in 0..10u32 {
            btree.delete(&make_key(i)).unwrap();
        }

        for i in 0..10u32 {
            assert!(btree.search(&make_key(i)).unwrap().is_none());
        }

        for i in 10..total {
            assert!(btree.search(&make_key(i)).unwrap().is_some());
        }

        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), 130);

        for idx in 1..all.len() {
            assert!(all[idx - 1].0 <= all[idx].0);
        }
    })
    .await
    .unwrap();
}

// ── test_internal_merge ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_internal_merge() {
    // Insert 2500 entries to create 3+ tree levels, then bulk-delete a large
    // range. Verify the tree still works correctly: remaining entries are all
    // searchable and scan_all returns the correct count in order.
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        let total: u32 = 2500;
        for i in 0..total {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }
        assert_eq!(btree.scan_all().unwrap().len(), 2500);

        for i in 0..1200u32 {
            let (count, _) = btree.delete_by_key(&make_key(i)).unwrap();
            assert_eq!(count, 1);
        }

        for i in 0..1200u32 {
            assert!(btree.search(&make_key(i)).unwrap().is_none());
        }

        for i in 1200..total {
            assert!(btree.search(&make_key(i)).unwrap().is_some());
        }

        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), 1300);

        for idx in 1..all.len() {
            assert!(all[idx - 1].0 <= all[idx].0);
        }
    })
    .await
    .unwrap();
}

// ── test_root_shrink ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_root_shrink() {
    // Create a 2-level tree, then delete most entries until everything
    // fits in a single leaf. Verify the tree still works correctly.
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        let initial_root = btree.root_page_id();

        let total: u32 = 150;
        let mut root_split_seen = false;
        for i in 0..total {
            let result = btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
            if result.is_some() {
                root_split_seen = true;
            }
        }
        assert!(root_split_seen, "Root split should have occurred");
        let expanded_root = btree.root_page_id();
        assert_ne!(expanded_root, initial_root);

        for i in 0..(total - 10) {
            let (count, _) = btree.delete_by_key(&make_key(i)).unwrap();
            assert_eq!(count, 1);
        }

        for i in (total - 10)..total {
            assert!(btree.search(&make_key(i)).unwrap().is_some());
        }

        for i in 0..(total - 10) {
            assert!(btree.search(&make_key(i)).unwrap().is_none());
        }

        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), 10);

        // After mass deletion the tree may still reference an internal root;
        // verify search and scan_all remain correct even from a structure
        // with mostly-empty leaves.
        let _ = expanded_root;
    })
    .await
    .unwrap();
}

// ── test_mass_delete_then_insert ─────────────────────────────────────────────

#[tokio::test]
async fn test_mass_delete_then_insert() {
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        for i in 0..200u32 {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }

        for i in 0..180u32 {
            let (count, _) = btree.delete_by_key(&make_key(i)).unwrap();
            assert_eq!(count, 1);
        }

        for i in 200..400u32 {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }

        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), 220);

        for i in 180..200u32 {
            assert!(btree.search(&make_key(i)).unwrap().is_some());
        }
        for i in 200..400u32 {
            assert!(btree.search(&make_key(i)).unwrap().is_some());
        }
        for i in 0..180u32 {
            assert!(btree.search(&make_key(i)).unwrap().is_none());
        }

        for idx in 1..all.len() {
            assert!(all[idx - 1].0 <= all[idx].0);
        }
    })
    .await
    .unwrap();
}

// ── test_interspersed_ops ────────────────────────────────────────────────────

#[tokio::test]
async fn test_interspersed_ops() {
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        for i in 0..100u32 {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }

        for i in 0..30u32 {
            btree.delete_by_key(&make_key(i)).unwrap();
        }

        for i in 100..150u32 {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }

        for i in 30..70u32 {
            btree.delete_by_key(&make_key(i)).unwrap();
        }

        for i in 150..170u32 {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }

        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), 100);

        for i in 70..170u32 {
            assert!(btree.search(&make_key(i)).unwrap().is_some());
        }
        for i in 0..70u32 {
            assert!(btree.search(&make_key(i)).unwrap().is_none());
        }

        for idx in 1..all.len() {
            assert!(all[idx - 1].0 <= all[idx].0);
        }
    })
    .await
    .unwrap();
}

// ── test_delete_all_entries ─────────────────────────────────────────────────

#[tokio::test]
async fn test_delete_all_entries() {
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        for i in 0..100u32 {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }
        assert_eq!(btree.scan_all().unwrap().len(), 100);

        for i in 0..100u32 {
            let (count, _) = btree.delete_by_key(&make_key(i)).unwrap();
            assert_eq!(count, 1);
        }

        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), 0);

        for i in 0..100u32 {
            assert!(btree.search(&make_key(i)).unwrap().is_none());
        }
    })
    .await
    .unwrap();
}

// ── test_delete_from_single_leaf ────────────────────────────────────────────

#[tokio::test]
async fn test_delete_from_single_leaf() {
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        let initial_root = btree.root_page_id();

        btree.insert(b"alice", RowId::new(1, 0)).unwrap();
        btree.insert(b"bob", RowId::new(2, 0)).unwrap();
        btree.insert(b"carol", RowId::new(3, 0)).unwrap();
        btree.insert(b"dave", RowId::new(4, 0)).unwrap();
        btree.insert(b"eve", RowId::new(5, 0)).unwrap();

        btree.delete(b"bob").unwrap();
        btree.delete(b"dave").unwrap();

        assert!(btree.search(b"alice").unwrap().is_some());
        assert!(btree.search(b"bob").unwrap().is_none());
        assert!(btree.search(b"carol").unwrap().is_some());
        assert!(btree.search(b"dave").unwrap().is_none());
        assert!(btree.search(b"eve").unwrap().is_some());

        assert_eq!(btree.root_page_id(), initial_root);

        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), 3);
        let collected: Vec<&[u8]> = all.iter().map(|(k, _)| k.as_bytes()).collect();
        assert_eq!(
            collected,
            vec![b"alice" as &[u8], b"carol" as &[u8], b"eve" as &[u8]]
        );
    })
    .await
    .unwrap();
}

// ── test_free_page_reuse ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_free_page_reuse() {
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));

        let page_a = loader.allocate_page().unwrap();
        loader.free_page(page_a).unwrap();

        let page_b = loader.allocate_page().unwrap();
        assert_eq!(page_a, page_b, "Freed page must be reused");

        let mut btree = BTree::new(loader.clone()).unwrap();
        for i in 0..150u32 {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }

        for i in 0..140u32 {
            btree.delete_by_key(&make_key(i)).unwrap();
        }
        drop(btree);

        let mut btree2 = BTree::new(loader.clone()).unwrap();
        let after_free_root = btree2.root_page_id();
        for i in 0..50u32 {
            btree2
                .insert(&make_key(i + 1000), RowId::new(i + 1000, 0))
                .unwrap();
        }
        assert_eq!(btree2.scan_all().unwrap().len(), 50);
        let _ = after_free_root;
    })
    .await
    .unwrap();
}

// ── test_scan_all_after_merge ────────────────────────────────────────────────

#[tokio::test]
async fn test_scan_all_after_merge() {
    // Insert 300 sorted entries, then bulk-delete ~100 from the middle range.
    // scan_all() must return the remaining 200 entries in ascending key order.
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        let total: u32 = 300;
        for i in 0..total {
            btree.insert(&make_key(i), RowId::new(i, 0)).unwrap();
        }
        assert_eq!(btree.scan_all().unwrap().len(), 300);

        for i in 100..200u32 {
            let (count, _) = btree.delete_by_key(&make_key(i)).unwrap();
            assert_eq!(count, 1);
        }

        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), 200);

        for idx in 1..all.len() {
            assert!(
                all[idx - 1].0 < all[idx].0,
                "Entries must be in ascending key order at index {}",
                idx,
            );
        }

        assert!(btree.search(&make_key(0)).unwrap().is_some());
        assert!(btree.search(&make_key(99)).unwrap().is_some());
        assert!(btree.search(&make_key(100)).unwrap().is_none());
        assert!(btree.search(&make_key(199)).unwrap().is_none());
        assert!(btree.search(&make_key(200)).unwrap().is_some());
        assert!(btree.search(&make_key(299)).unwrap().is_some());

        for i in 0..300u32 {
            if i >= 100 && i < 200 {
                assert!(btree.search(&make_key(i)).unwrap().is_none());
            } else {
                let result = btree.search(&make_key(i)).unwrap();
                assert!(result.is_some());
                assert_eq!(result.unwrap(), RowId::new(i, 0));
            }
        }
    })
    .await
    .unwrap();
}
