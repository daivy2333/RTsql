// BTree split and non-unique index tests (M17)
use std::sync::Arc;
use tempfile::tempdir;

use rtsql::storage::page_format::{Key, RowId};
use rtsql::storage::{
    btree::{BTree, SyncPageLoader, LeafNodeRef},
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
        let mut btree = BTree::new(loader).unwrap();

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
        let mut btree = BTree::new(loader).unwrap();

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
        let mut btree = BTree::new(loader).unwrap();

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
        let mut btree = BTree::new(loader).unwrap();

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
        let mut btree = BTree::new(loader).unwrap();

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

// ============================================================
// B-Tree Split Tests (Tasks 5-8)
// ============================================================

/// Helper: format a number as a 32-byte left-padded key string.
/// Produces strings like "00000000000000000000000000000001", "00000000000000000000000000000002", etc.
/// These are exactly 32 bytes (MAX_KEY_LEN) and sort lexicographically in numeric order.
fn make_key(i: u32) -> Vec<u8> {
    format!("{:032}", i).into_bytes()
}

#[tokio::test]
async fn test_leaf_split_first_time() {
    // Insert more entries than a leaf node can hold (~97) to trigger the first split.
    // Verify all entries are searchable and scan_all returns the correct count.
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        let initial_root = btree.root_page_id();
        let total: u32 = 120; // exceeds leaf capacity (~97)

        for i in 0..total {
            let key = make_key(i);
            let row_id = RowId::new(i, 0);
            btree.insert(&key, row_id).unwrap();
        }

        // Verify all entries are searchable
        for i in 0..total {
            let key = make_key(i);
            let result = btree.search(&key).unwrap();
            assert!(
                result.is_some(),
                "Key {} should be found after split",
                i
            );
            assert_eq!(
                result.unwrap(),
                RowId::new(i, 0),
                "RowId mismatch for key {}",
                i
            );
        }

        // Verify scan_all returns the correct count
        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), total as usize, "scan_all should return {} entries", total);

        // Root page_id should have changed if root split occurred
        // (With 120 entries, root definitely split)
        assert_ne!(
            btree.root_page_id(),
            initial_root,
            "Root page_id should change after split"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_non_unique_key_split() {
    // Insert many entries with the same key to trigger a split within a non-unique key set.
    // Verify search_all returns all matching rows, and delete_by_key removes them all.
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        let key = b"same_key";
        let total: u32 = 120; // exceeds leaf capacity

        // Insert many entries with the same key but different RowIds
        for i in 0..total {
            let row_id = RowId::new(i, (i % 100) as u16);
            btree.insert(key, row_id).unwrap();
        }

        // Verify search_all returns all matching RowIds
        let results = btree.search_all(key).unwrap();
        assert_eq!(results.len(), total as usize, "search_all should return all {} entries", total);

        // Verify each RowId is present
        for i in 0..total {
            let expected_rid = RowId::new(i, (i % 100) as u16);
            assert!(
                results.contains(&expected_rid),
                "RowId {} should be in search_all results",
                expected_rid
            );
        }

        // Delete all entries with this key
        let deleted = btree.delete_by_key(key).unwrap();
        assert_eq!(deleted, total as usize, "delete_by_key should delete all {} entries", total);

        // Verify all are gone
        let after_delete = btree.search_all(key).unwrap();
        assert_eq!(after_delete.len(), 0, "No entries should remain after delete_by_key");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_search_after_split() {
    // Insert entries that trigger a split, then search for various keys:
    // - Keys at the split boundary
    // - Non-existent keys
    // - The first and last keys
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        let total: u32 = 150; // well beyond leaf capacity
        for i in 0..total {
            let key = make_key(i);
            let row_id = RowId::new(i, 0);
            btree.insert(&key, row_id).unwrap();
        }

        // Search for keys at the split boundary (around index ~48 and ~97)
        for boundary in [48u32, 49, 96, 97, 98] {
            let key = make_key(boundary);
            let result = btree.search(&key).unwrap();
            assert!(
                result.is_some(),
                "Boundary key {} should be found",
                boundary
            );
            assert_eq!(
                result.unwrap(),
                RowId::new(boundary, 0),
                "RowId mismatch for boundary key {}",
                boundary
            );
        }

        // Search for non-existent keys (outside the range)
        let missing_key = make_key(9999);
        assert!(
            btree.search(&missing_key).unwrap().is_none(),
            "Non-existent key should return None"
        );
        let missing_key2 = make_key(total + 100);
        assert!(
            btree.search(&missing_key2).unwrap().is_none(),
            "Key beyond range should return None"
        );

        // Search for the first and last keys
        let first_key = make_key(0);
        assert_eq!(
            btree.search(&first_key).unwrap(),
            Some(RowId::new(0, 0)),
            "First key should be found"
        );
        let last_key = make_key(total - 1);
        assert_eq!(
            btree.search(&last_key).unwrap(),
            Some(RowId::new(total - 1, 0)),
            "Last key should be found"
        );

        // Search for a key in the middle of the range
        let mid_key = make_key(total / 2);
        assert!(
            btree.search(&mid_key).unwrap().is_some(),
            "Middle key should be found"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_delete_after_split() {
    // Insert entries triggering a split, then delete some entries.
    // Verify deleted keys return None and remaining keys are still findable.
    // Also test delete_exact after split.
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        let total: u32 = 120;
        for i in 0..total {
            let key = make_key(i);
            let row_id = RowId::new(i, 0);
            btree.insert(&key, row_id).unwrap();
        }

        // Delete keys in the first half
        for i in 0..30u32 {
            let key = make_key(i);
            let deleted = btree.delete_by_key(&key).unwrap();
            assert_eq!(deleted, 1, "Should delete exactly 1 entry for key {}", i);
        }

        // Verify deleted keys return None via search
        for i in 0..30u32 {
            let key = make_key(i);
            assert!(
                btree.search(&key).unwrap().is_none(),
                "Deleted key {} should not be found",
                i
            );
        }

        // Verify remaining keys are still searchable
        for i in 30..total {
            let key = make_key(i);
            let result = btree.search(&key).unwrap();
            assert!(
                result.is_some(),
                "Remaining key {} should still be found",
                i
            );
        }

        // Verify scan_all has the correct count
        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), (total - 30) as usize, "scan_all should return {} entries", total - 30);

        // Test delete_exact after split
        // Insert duplicate keys, then use delete_exact to remove one
        let dup_key = make_key(200);
        btree.insert(&dup_key, RowId::new(200, 0)).unwrap();
        btree.insert(&dup_key, RowId::new(201, 1)).unwrap();
        btree.insert(&dup_key, RowId::new(202, 2)).unwrap();

        // Delete the middle one exactly
        btree.delete_exact(&dup_key, RowId::new(201, 1)).unwrap();

        // Verify only the right ones remain
        let remaining = btree.search_all(&dup_key).unwrap();
        assert_eq!(remaining.len(), 2, "Should have 2 remaining entries after delete_exact");
        assert!(remaining.contains(&RowId::new(200, 0)));
        assert!(remaining.contains(&RowId::new(202, 2)));
        assert!(!remaining.contains(&RowId::new(201, 1)));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_leaf_chain_after_split() {
    // Insert entries triggering a split.
    // Verify scan_all returns all entries in key order (leaf chain maintained).
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        let total: u32 = 200; // triggers multiple leaf splits
        for i in 0..total {
            let key = make_key(i);
            let row_id = RowId::new(i, 0);
            btree.insert(&key, row_id).unwrap();
        }

        // Verify scan_all returns all entries
        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), total as usize, "scan_all should return all {} entries", total);

        // Verify entries are in sorted key order
        for i in 1..all.len() {
            assert!(
                all[i - 1].0 <= all[i].0,
                "Entries should be in key order: {:?} > {:?} at index {}",
                all[i - 1].0.as_bytes(),
                all[i].0.as_bytes(),
                i
            );
        }

        // Verify every expected key+RowId pair is present
        for i in 0..total {
            let expected_key = Key::new(&make_key(i));
            let expected_rid = RowId::new(i, 0);
            let found = all.iter().any(|(k, r)| *k == expected_key && *r == expected_rid);
            assert!(found, "Entry (key={}, RowId={}) should be in scan_all results", i, expected_rid);
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_massive_insert_multiple_splits() {
    // Insert 2000+ entries to trigger many splits (leaf and internal).
    // Verify all entries can be found and scan_all returns the correct count.
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        let total: u32 = 2000;
        for i in 0..total {
            let key = make_key(i);
            let row_id = RowId::new(i, (i % 1000) as u16);
            btree.insert(&key, row_id).unwrap();
        }

        // Verify scan_all returns the correct count
        let all = btree.scan_all().unwrap();
        assert_eq!(all.len(), total as usize, "scan_all should return {} entries", total);

        // Spot-check search for various keys across the range
        for i in [0u32, 1, 50, 100, 500, 999, 1000, 1500, 1999] {
            let key = make_key(i);
            let result = btree.search(&key).unwrap();
            assert!(
                result.is_some(),
                "Key {} should be found",
                i
            );
            assert_eq!(
                result.unwrap(),
                RowId::new(i, (i % 1000) as u16),
                "RowId mismatch for key {}",
                i
            );
        }

        // Verify scan_all entries are in sorted key order
        for i in 1..all.len() {
            assert!(
                all[i - 1].0 <= all[i].0,
                "Entries should be in key order at index {}",
                i
            );
        }

        // Verify non-existent key returns None
        let missing = make_key(99999);
        assert!(
            btree.search(&missing).unwrap().is_none(),
            "Non-existent key should return None"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_root_split() {
    // Insert entries that trigger a root split.
    // Verify BTree::insert returns Some(new_root_page_id) and root_page_id() is updated.
    let pool = create_test_pool();
    let pool_clone = pool.clone();

    tokio::task::spawn_blocking(move || {
        let loader = Arc::new(SyncPageLoader::new(pool_clone));
        let mut btree = BTree::new(loader).unwrap();

        let initial_root = btree.root_page_id();
        let mut root_split_occurred = false;

        // Insert until a root split happens (first leaf split IS a root split since root is a leaf)
        let total: u32 = 120;
        for i in 0..total {
            let key = make_key(i);
            let row_id = RowId::new(i, 0);
            let result = btree.insert(&key, row_id).unwrap();

            if let Some(new_root_page_id) = result {
                root_split_occurred = true;
                // Verify the returned page_id matches the new root
                assert_eq!(
                    new_root_page_id,
                    btree.root_page_id(),
                    "Returned new_root_page_id should match btree.root_page_id()"
                );
                // Verify root_page_id has changed from the initial
                assert_ne!(
                    new_root_page_id,
                    initial_root,
                    "New root should be a different page from the initial root"
                );
            }
        }

        assert!(
            root_split_occurred,
            "At least one root split should have occurred with {} entries",
            total
        );

        // After root split, root_page_id should no longer be the initial leaf
        assert_ne!(
            btree.root_page_id(),
            initial_root,
            "root_page_id should have changed after root split"
        );

        // Verify search still routes correctly after root split
        for i in 0..total {
            let key = make_key(i);
            let result = btree.search(&key).unwrap();
            assert!(
                result.is_some(),
                "Key {} should still be found after root split",
                i
            );
            assert_eq!(
                result.unwrap(),
                RowId::new(i, 0),
                "RowId mismatch for key {} after root split",
                i
            );
        }

        // Verify the root is now an internal node (has children)
        // by checking that it routes keys correctly to different subtrees
        let low_key = make_key(0);
        let high_key = make_key(total - 1);
        assert!(
            btree.search(&low_key).unwrap().is_some(),
            "Low key should be found via internal root"
        );
        assert!(
            btree.search(&high_key).unwrap().is_some(),
            "High key should be found via internal root"
        );
    })
    .await
    .unwrap();
}
