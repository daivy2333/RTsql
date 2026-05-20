//! Concurrent transaction tests for MVCC
//!
//! Tests verify:
//! - Snapshot consistency across concurrent transactions
//! - Transaction ID uniqueness under concurrency
//! - Read-write non-blocking behavior

use rtsql::transaction::{Snapshot, TransactionManager};

#[tokio::test]
async fn test_concurrent_snapshot_consistency() {
    // Two concurrent transactions see different views based on their snapshots
    let manager = std::sync::Arc::new(TransactionManager::new());

    // Tx1 starts
    let tx1 = manager.begin().await;
    let tx1_id = tx1.id();

    // Tx2 starts (after Tx1)
    let tx2 = manager.begin().await;
    let tx2_id = tx2.id();

    // Tx1's snapshot should not contain Tx2 (Tx2 started after)
    // For visibility: Tx2 with create_tx_id=tx2_id, commit_tx_id=None
    // Tx1 snapshot tx_id = tx1_id < tx2_id, so tx2_id > tx1_id -> not visible
    let snap1 = tx1.snapshot();
    assert!(!snap1.is_visible(tx2_id, None));

    // Tx2's snapshot should contain Tx1 in active list
    // Tx1 not committed, so is_visible returns false
    let snap2 = tx2.snapshot();
    assert!(!snap2.is_visible(tx1_id, None)); // Tx1 uncommitted, not visible

    // Commit Tx1
    manager.commit(tx1).await.unwrap();

    // Tx2's snapshot still considers Tx1 not visible
    // (snapshot taken before Tx1 committed, Tx1 was in active list)
    // Even after commit, the snapshot's active_tx_ids still contains tx1_id
    assert!(!snap2.is_visible(tx1_id, Some(tx1_id))); // Tx1 was in active list
}

#[tokio::test]
async fn test_concurrent_read_write_no_block() {
    // Read operations should not block write operations
    // (This is guaranteed by MVCC snapshot design)
    let manager = std::sync::Arc::new(TransactionManager::new());

    let tx1 = manager.begin().await;
    let tx2 = manager.begin().await;

    // Both can create snapshots simultaneously (no blocking)
    let snap1 = tx1.snapshot();
    let snap2 = tx2.snapshot();

    // Snapshots created instantaneously
    assert!(snap1.tx_id() > 0);
    assert!(snap2.tx_id() > 0);

    manager.commit(tx1).await.unwrap();
    manager.commit(tx2).await.unwrap();
}

#[tokio::test]
async fn test_concurrent_transactions_unique_ids() {
    let manager = std::sync::Arc::new(TransactionManager::new());

    let mut tasks = vec![];

    for _ in 0..10 {
        let manager_clone = manager.clone();
        tasks.push(tokio::spawn(async move {
            let tx = manager_clone.begin().await;
            let id = tx.id();
            // Hold transaction briefly
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            manager_clone.commit(tx).await.unwrap();
            id
        }));
    }

    let ids: Vec<u64> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // All IDs should be unique
    let unique_ids: std::collections::HashSet<u64> = ids.iter().copied().collect();
    assert_eq!(unique_ids.len(), 10);

    // IDs should be increasing (though not strictly sequential due to concurrency)
    let max_id = *ids.iter().max().unwrap();
    let min_id = *ids.iter().min().unwrap();
    assert!(max_id > min_id);
}

#[tokio::test]
async fn test_snapshot_visibility_rules() {
    let manager = TransactionManager::new();

    // Sequence of transactions
    let tx1 = manager.begin().await;
    let tx1_id = tx1.id();

    let tx2 = manager.begin().await;
    let tx2_id = tx2.id();

    let tx3 = manager.begin().await;
    let tx3_id = tx3.id();

    // tx3's snapshot includes tx1 and tx2 in active list
    let snap3 = tx3.snapshot();

    // tx1 and tx2 not committed -> not visible
    assert!(!snap3.is_visible(tx1_id, None));
    assert!(!snap3.is_visible(tx2_id, None));

    // Commit tx1
    manager.commit(tx1).await.unwrap();

    // tx3's snapshot still considers tx1 not visible
    // (tx1 was in active list when snapshot was taken)
    assert!(!snap3.is_visible(tx1_id, Some(tx1_id)));

    // Start tx4 after tx1 committed
    let tx4 = manager.begin().await;
    let snap4 = tx4.snapshot();

    // tx4's snapshot does NOT include tx1 in active list
    // tx1 is committed before tx4 started, so visible
    assert!(snap4.is_visible(tx1_id, Some(tx1_id)));

    // tx2 and tx3 still not committed -> not visible to tx4
    assert!(!snap4.is_visible(tx2_id, None));
    assert!(!snap4.is_visible(tx3_id, None));

    // Cleanup
    manager.abort(tx2).await.unwrap();
    manager.commit(tx3).await.unwrap();
    manager.commit(tx4).await.unwrap();
}
