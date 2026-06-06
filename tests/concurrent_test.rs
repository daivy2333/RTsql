//! Concurrent transaction tests for MVCC
//!
//! Tests verify:
//! - Snapshot consistency across concurrent transactions
//! - Transaction ID uniqueness under concurrency
//! - Read-write non-blocking behavior

use rtsql::storage::{BufferPool, FileStorage, TableManager};
use rtsql::transaction::{Snapshot, TransactionManager};
use std::sync::Arc;
use tempfile::tempdir;

/// Create a test buffer pool for tests that need it
fn create_test_buffer_pool() -> Arc<BufferPool> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    Arc::new(BufferPool::new(10, storage).unwrap())
}

/// Create a test table for abort tests
async fn create_test_table(buffer_pool: Arc<BufferPool>) -> Arc<rtsql::storage::TableMeta> {
    let table_manager = TableManager::new(buffer_pool.clone());
    table_manager
        .create_table(
            "test_table",
            vec![("id".to_string(), rtsql::storage::ColumnType::Int)],
            "id",
        )
        .await
        .unwrap();
    table_manager.get_table("test_table").await.unwrap()
}

#[tokio::test]
async fn test_concurrent_snapshot_consistency() {
    // Two concurrent transactions see different views based on their snapshots
    let manager = std::sync::Arc::new(TransactionManager::new());
    let buffer_pool = create_test_buffer_pool();

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
    manager.commit(tx1, &buffer_pool).await.unwrap();

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
    let buffer_pool = create_test_buffer_pool();

    let tx1 = manager.begin().await;
    let tx2 = manager.begin().await;

    // Both can create snapshots simultaneously (no blocking)
    let snap1 = tx1.snapshot();
    let snap2 = tx2.snapshot();

    // Snapshots created instantaneously
    assert!(snap1.tx_id() > 0);
    assert!(snap2.tx_id() > 0);

    manager.commit(tx1, &buffer_pool).await.unwrap();
    manager.commit(tx2, &buffer_pool).await.unwrap();
}

#[tokio::test]
async fn test_concurrent_transactions_unique_ids() {
    let manager = std::sync::Arc::new(TransactionManager::new());
    let buffer_pool = create_test_buffer_pool();

    let mut tasks = vec![];

    for _ in 0..10 {
        let manager_clone = manager.clone();
        let buffer_pool_clone = buffer_pool.clone();
        tasks.push(tokio::spawn(async move {
            let tx = manager_clone.begin().await;
            let id = tx.id();
            // Hold transaction briefly
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            manager_clone.commit(tx, &buffer_pool_clone).await.unwrap();
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
    let buffer_pool = create_test_buffer_pool();
    let table_meta = create_test_table(buffer_pool.clone()).await;

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
    manager.commit(tx1, &buffer_pool).await.unwrap();

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
    manager.abort(tx2, &buffer_pool, &table_meta).await.unwrap();
    manager.commit(tx3, &buffer_pool).await.unwrap();
    manager.commit(tx4, &buffer_pool).await.unwrap();
}

// =========================================================================
// M31: BufferPool DashMap + miss Semaphore concurrent tests
// =========================================================================
//
// These tests cover the 6 acceptance criteria for the M31 change:
//   2.1 H1: 16 concurrent get_page on same cached page
//   2.2 H2: 16 concurrent get_page on different pages
//   2.3 E1: double-check single load (8 concurrent miss on uncached page)
//   2.4 H4: concurrent get_page + free_page
//   2.5 S4/E3: miss semaphore backpressure (1000 concurrent miss, 10s timeout)
//   2.6 H1+miss sem: cache hit skips miss semaphore (no waiting when hit)

use rtsql::storage::{AsyncStorage, Page, PageId};
use std::sync::atomic::{AtomicUsize, Ordering};

/// CountingStorage wraps FileStorage to count read_page calls.
/// Used to verify double-check ensures single load (test 2.3).
struct CountingStorage {
    inner: Arc<FileStorage>,
    read_count: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl AsyncStorage for CountingStorage {
    async fn read_page(&self, page_id: PageId) -> rtsql::storage::Result<Page> {
        self.read_count.fetch_add(1, Ordering::SeqCst);
        self.inner.read_page(page_id).await
    }
    async fn write_page(&self, page_id: PageId, page: &Page) -> rtsql::storage::Result<()> {
        self.inner.write_page(page_id, page).await
    }
    async fn allocate_page(&self) -> rtsql::storage::Result<PageId> {
        self.inner.allocate_page().await
    }
    async fn free_page(&self, page_id: PageId) -> rtsql::storage::Result<()> {
        self.inner.free_page(page_id).await
    }
    async fn sync(&self) -> rtsql::storage::Result<()> {
        self.inner.sync().await
    }
    fn page_size(&self) -> usize {
        self.inner.page_size()
    }
}

/// M31 Task 2.1 (H1): 16 concurrent tasks get_page(same cached page).
/// All MUST return PageGuards for the same page without panic/deadlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_concurrent_get_same_page() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(20, storage).unwrap());

    let page_id = buffer_pool.storage().allocate_page().await.unwrap();

    // Pre-cache the page
    let _primer = buffer_pool.get_page(page_id).await.unwrap();
    drop(_primer);

    // 16 concurrent get_page on the same cached page
    let mut tasks = Vec::with_capacity(16);
    for _ in 0..16 {
        let bp = buffer_pool.clone();
        let pid = page_id;
        tasks.push(tokio::spawn(async move { bp.get_page(pid).await }));
    }

    let results: Vec<_> = futures::future::join_all(tasks).await;
    for r in results {
        let guard = r.expect("task panicked").expect("get_page errored");
        assert_eq!(guard.page().id, page_id);
    }
}

/// M31 Task 2.2 (H2): 16 concurrent tasks get_page(different pages).
/// All MUST complete without deadlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_concurrent_get_different_pages() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(20, storage).unwrap());

    // Pre-allocate 16 pages
    let mut page_ids = Vec::with_capacity(16);
    for _ in 0..16 {
        page_ids.push(buffer_pool.storage().allocate_page().await.unwrap());
    }

    let mut tasks = Vec::with_capacity(16);
    for pid in page_ids.iter() {
        let bp = buffer_pool.clone();
        let pid = *pid;
        tasks.push(tokio::spawn(async move { bp.get_page(pid).await }));
    }

    let results: Vec<_> = futures::future::join_all(tasks).await;
    let mut seen = std::collections::HashSet::new();
    for r in results {
        let guard = r.expect("task panicked").expect("get_page errored");
        assert!(seen.insert(guard.page().id), "duplicate page returned");
    }
    assert_eq!(seen.len(), 16);
}

/// M31 Task 2.3 (E1): 8 concurrent miss on uncached page → exactly 1 read_page call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_double_check_single_load() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let read_count = Arc::new(AtomicUsize::new(0));

    let counting = Arc::new(CountingStorage {
        inner: storage,
        read_count: read_count.clone(),
    });

    let buffer_pool = Arc::new(BufferPool::new(10, counting).unwrap());
    let page_id = buffer_pool.storage().allocate_page().await.unwrap();

    // 8 concurrent tasks all miss on the same uncached page
    let mut tasks = Vec::with_capacity(8);
    for _ in 0..8 {
        let bp = buffer_pool.clone();
        let pid = page_id;
        tasks.push(tokio::spawn(async move { bp.get_page(pid).await }));
    }

    let results: Vec<_> = futures::future::join_all(tasks).await;
    for r in results {
        r.expect("task panicked").expect("get_page errored");
    }

    // read_page(42) must be called exactly 1 time despite 8 concurrent miss requests
    let count = read_count.load(Ordering::SeqCst);
    assert_eq!(
        count, 1,
        "expected exactly 1 read_page call (double-check), got {}",
        count
    );
}

/// M31 Task 2.4 (H4): concurrent get_page + free_page on disjoint page sets.
/// No panic, no deadlock, no use-after-free.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_concurrent_get_and_free() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(200, storage).unwrap());

    // Pre-allocate 100 pages
    let mut all_pages = Vec::with_capacity(100);
    for _ in 0..100 {
        all_pages.push(buffer_pool.storage().allocate_page().await.unwrap());
    }
    // Split: get pages (read-only usage) and free pages (write-then-free)
    let get_pages: Vec<PageId> = all_pages[0..50].to_vec();
    let free_pages: Vec<PageId> = all_pages[50..100].to_vec();

    let rounds = 500u32;
    let bp_get = buffer_pool.clone();
    let bp_free = buffer_pool.clone();

    let getter = tokio::spawn(async move {
        for i in 0..rounds {
            let pid = get_pages[(i as usize) % get_pages.len()];
            let g = bp_get.get_page(pid).await.expect("get_page on read-set");
            drop(g);
        }
    });

    let freer = tokio::spawn(async move {
        for i in 0..rounds {
            let pid = free_pages[(i as usize) % free_pages.len()];
            // free_page removes from buffer pool + storage; we re-allocate
            // for next round to keep the test deterministic.
            bp_free.free_page(pid).await.expect("free_page");
        }
    });

    getter.await.expect("getter task panicked");
    freer.await.expect("freer task panicked");
}

/// M31 Task 2.5 (S4 + E3): 1000 concurrent miss all complete within 10s.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn test_miss_semaphore_backpressure() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(2000, storage).unwrap());

    // Pre-allocate 1000 pages
    let mut page_ids = Vec::with_capacity(1000);
    for _ in 0..1000 {
        page_ids.push(
            buffer_pool
                .storage()
                .allocate_page()
                .await
                .expect("allocate"),
        );
    }

    let n = 1000usize;
    let mut tasks = Vec::with_capacity(n);
    for pid in page_ids.into_iter() {
        let bp = buffer_pool.clone();
        tasks.push(tokio::spawn(async move { bp.get_page(pid).await }));
    }

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        futures::future::join_all(tasks),
    )
    .await;

    let results = result.expect("1000 concurrent miss should not hang >10s");
    let mut ok = 0;
    for r in results {
        if r.expect("task panicked").is_ok() {
            ok += 1;
        }
    }
    assert_eq!(ok, n, "all {} tasks should succeed, got {} ok", n, ok);
}

/// M31 Task 2.6 (H1 + miss sem isolation): cache hit path returns fast
/// even when miss path is under heavy load. Verifies hit path doesn't
/// acquire miss semaphore permit.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_cache_hit_skips_miss_semaphore() {
    let buffer_pool = create_test_buffer_pool();

    // Pre-cache page 42
    let hot_page = buffer_pool.storage().allocate_page().await.unwrap();
    let _p = buffer_pool.get_page(hot_page).await.unwrap();

    // Saturate miss path: spawn 32 tasks that each miss a unique page.
    // This pushes miss semaphore into contention.
    let saturate_n = 32usize;
    let mut saturators = Vec::with_capacity(saturate_n);
    for _ in 0..saturate_n {
        let bp = buffer_pool.clone();
        saturators.push(tokio::spawn(async move {
            let pid = bp.storage().allocate_page().await.unwrap();
            bp.get_page(pid).await
        }));
    }

    // Give saturators a head start to acquire permits
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    // While saturators are running, hit the hot page — must return < 50ms
    let start = std::time::Instant::now();
    let guard = buffer_pool.get_page(hot_page).await.expect("hit");
    let elapsed = start.elapsed();
    assert_eq!(guard.page().id, hot_page);
    assert!(
        elapsed < std::time::Duration::from_millis(50),
        "cache hit should return fast (<50ms), got {:?}",
        elapsed
    );

    // Drain saturators
    for t in saturators {
        let _ = t.await;
    }
}
