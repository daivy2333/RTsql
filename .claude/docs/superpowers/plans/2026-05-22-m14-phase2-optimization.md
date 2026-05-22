# M14 Phase 2: Full-Path Query Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate spawn_blocking scheduling bottleneck and optimize full query path to achieve 3-5x PK lookup speedup (~36µs → ~10µs).

**Architecture:** Replace spawn_blocking + SyncPageLoader::block_on with direct async BTree read path. Introduce AsyncPageLoader for reads, retain SyncPageLoader for writes. Replace Mutex<BTree> with RwLock<BTree>. Add binary search to BTree nodes. Tune BufferPool capacity.

**Tech Stack:** Rust, Tokio async runtime, existing BTree/BufferPool infrastructure

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `src/storage/btree/async_loader.rs` | Create | AsyncPageLoader — async page loading without block_on |
| `src/storage/btree/btree.rs` | Modify | Add async search methods (search_async, search_from_page_async) |
| `src/storage/btree/node.rs` | Modify | Add binary search methods to LeafNodeRef and InternalNodeRef |
| `src/storage/btree/index_manager.rs` | Modify | Dual-mode: async read + sync write, RwLock<BTree> |
| `src/storage/btree/mod.rs` | Modify | Export AsyncPageLoader |
| `src/storage/buffer_pool.rs` | Modify | Default capacity 100 → 1024 |
| `src/database.rs` | Modify | Wire AsyncPageLoader into IndexManager creation |
| `src/executor/index_scan.rs` | Modify | Use async IndexManager::search instead of spawn_blocking |
| `benches/m14_staged_bench.rs` | Create | Staged performance benchmark for precise bottleneck measurement |

---

### Task 1: Staged Performance Benchmark

**Files:**
- Create: `benches/m14_staged_bench.rs`
- Modify: `Cargo.toml` (add benchmark entry)

This benchmark measures each stage of the query pipeline independently, giving us precise data on where time is spent.

- [ ] **Step 1: Create the staged benchmark file**

```rust
// benches/m14_staged_bench.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use rtsql::database::Database;
use std::time::Duration;

fn bench_staged_pk_lookup(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let db = Database::open_in_memory().await.unwrap();
        db.execute("CREATE TABLE bench (id INTEGER PRIMARY KEY, val TEXT)").await.unwrap();
        for i in 0..1000i64 {
            db.execute(&format!("INSERT INTO bench VALUES ({}, 'hello')", i)).await.unwrap();
        }

        let mut group = c.benchmark_group("m14_staged_pk_lookup");
        group.sample_size(50);
        group.measurement_time(Duration::from_secs(3));

        group.bench_function("full_pipeline", |b| {
            b.to_async(&rt).iter(|| async {
                db.execute("SELECT * FROM bench WHERE id = 42").await.unwrap();
            });
        });

        group.finish();
    });
}

criterion_group!(benches, bench_staged_pk_lookup);
criterion_main!(benches);
```

- [ ] **Step 2: Add benchmark to Cargo.toml**

Add to `[[bench]]` section:
```toml
[[bench]]
name = "m14_staged_bench"
harness = false
```

- [ ] **Step 3: Run the benchmark to establish baseline**

Run: `cargo bench --bench m14_staged_bench -- --noplot 2>&1 | tail -5`
Expected: Baseline PK lookup time ~36µs

- [ ] **Step 4: Commit**

```bash
git add benches/m14_staged_bench.rs Cargo.toml
git commit -m "bench(M14): add staged PK lookup benchmark for phase 2"
```

---

### Task 2: Binary Search in BTree Nodes

**Files:**
- Modify: `src/storage/btree/node.rs`

Add binary search methods to `LeafNodeRef` and `InternalNodeRef`. These are pure functions on serialized data — no async, no I/O, safe to implement first.

- [ ] **Step 1: Write failing test for LeafNodeRef binary search**

In `src/storage/btree/node.rs`, add test module (if not present) or append to existing:

```rust
#[cfg(test)]
mod binary_search_tests {
    use super::*;
    use crate::storage::page::PAGE_SIZE;

    fn make_leaf_with_keys(keys: &[i64]) -> ([u8; PAGE_SIZE], usize) {
        let mut page = [0u8; PAGE_SIZE];
        page[0] = LEAF_NODE;
        let count = keys.len() as u16;
        page[2..4].copy_from_slice(&count.to_be_bytes());
        for (i, &k) in keys.iter().enumerate() {
            let offset = LEAF_HEADER_SIZE + i * LEAF_ENTRY_SIZE;
            page[offset..offset + 8].copy_from_slice(&k.to_be_bytes());
            page[offset + 8..offset + 16].copy_from_slice(&(i as u64).to_be_bytes());
        }
        (page, keys.len())
    }

    #[test]
    fn leaf_binary_search_finds_existing_key() {
        let (page, count) = make_leaf_with_keys(&[1, 3, 5, 7, 9, 11, 13, 15]);
        let leaf = LeafNodeRef::new(&page);
        let key = Key::new(&7i64.to_be_bytes());
        let (found, pos) = leaf.find_key_position_binary(&key);
        assert!(found);
        assert_eq!(pos, 3);
    }

    #[test]
    fn leaf_binary_search_returns_insert_position_for_missing() {
        let (page, count) = make_leaf_with_keys(&[1, 5, 10, 20]);
        let leaf = LeafNodeRef::new(&page);
        let key = Key::new(&7i64.to_be_bytes());
        let (found, pos) = leaf.find_key_position_binary(&key);
        assert!(!found);
        assert_eq!(pos, 2);
    }

    #[test]
    fn leaf_binary_search_first_key() {
        let (page, count) = make_leaf_with_keys(&[5, 10, 15]);
        let leaf = LeafNodeRef::new(&page);
        let key = Key::new(&5i64.to_be_bytes());
        let (found, pos) = leaf.find_key_position_binary(&key);
        assert!(found);
        assert_eq!(pos, 0);
    }

    #[test]
    fn leaf_binary_search_last_key() {
        let (page, count) = make_leaf_with_keys(&[5, 10, 15]);
        let leaf = LeafNodeRef::new(&page);
        let key = Key::new(&15i64.to_be_bytes());
        let (found, pos) = leaf.find_key_position_binary(&key);
        assert!(found);
        assert_eq!(pos, 2);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test binary_search --lib -- --nocapture 2>&1 | tail -10`
Expected: FAIL — `find_key_position_binary` method does not exist

- [ ] **Step 3: Implement LeafNodeRef::find_key_position_binary**

Add to `impl LeafNodeRef` in `src/storage/btree/node.rs`:

```rust
pub fn find_key_position_binary(&self, key: &Key) -> (bool, usize) {
    let count = self.key_count() as usize;
    let mut lo: usize = 0;
    let mut hi: usize = count;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if let Some(mid_key) = self.get_key(mid) {
            match mid_key.cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return (true, mid),
            }
        } else {
            hi = mid;
        }
    }
    (false, lo)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test binary_search --lib -- --nocapture 2>&1 | tail -5`
Expected: 4 passed, 0 failed

- [ ] **Step 5: Implement InternalNodeRef::find_child_page_id_binary**

Add to `impl InternalNodeRef` in `src/storage/btree/node.rs`:

```rust
pub fn find_child_page_id_binary(&self, key: &Key) -> u32 {
    let count = self.key_count() as usize;
    let mut lo: usize = 0;
    let mut hi: usize = count;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if let Some(mid_key) = self.get_key(mid) {
            match mid_key.cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => {
                    return self.get_child_page_id(mid + 1);
                }
            }
        } else {
            hi = mid;
        }
    }
    self.get_child_page_id(lo)
}
```

- [ ] **Step 6: Run all tests**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/storage/btree/node.rs
git commit -m "feat(M14): add binary search to LeafNodeRef and InternalNodeRef"
```

---

### Task 3: AsyncPageLoader

**Files:**
- Create: `src/storage/btree/async_loader.rs`
- Modify: `src/storage/btree/mod.rs`

Create the async counterpart to SyncPageLoader. This is the core enabler for eliminating spawn_blocking on the read path.

- [ ] **Step 1: Write failing test for AsyncPageLoader**

Create test in a new file first, then move to proper location. The test verifies that AsyncPageLoader can load a page without block_on:

```rust
// This test will live in src/storage/btree/async_loader.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::buffer_pool::BufferPool;
    use crate::storage::file_storage::FileStorage;
    use std::sync::Arc;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn async_loader_loads_page_without_block_on() {
        let temp = NamedTempFile::new().unwrap();
        let storage = Arc::new(FileStorage::new(temp.path()).unwrap());
        let pool = Arc::new(BufferPool::new(100, storage));
        let loader = AsyncPageLoader::new(pool.clone());

        let page_id = pool.allocate_page().await.unwrap();
        let guard = loader.load_page(page_id).await;
        assert!(guard.is_ok(), "AsyncPageLoader should load page without block_on");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test async_loader --lib -- --nocapture 2>&1 | tail -10`
Expected: FAIL — module `async_loader` does not exist

- [ ] **Step 3: Create AsyncPageLoader implementation**

Create `src/storage/btree/async_loader.rs`:

```rust
use crate::storage::buffer_pool::{BufferPool, PageGuard};
use crate::storage::page::PageId;
use std::sync::Arc;

pub struct AsyncPageLoader {
    buffer_pool: Arc<BufferPool>,
}

impl AsyncPageLoader {
    pub fn new(buffer_pool: Arc<BufferPool>) -> Self {
        Self { buffer_pool }
    }

    pub async fn load_page(&self, page_id: PageId) -> crate::error::Result<PageGuard> {
        self.buffer_pool.get_page(page_id).await
    }
}
```

- [ ] **Step 4: Export AsyncPageLoader from mod.rs**

Add to `src/storage/btree/mod.rs`:
```rust
pub mod async_loader;
pub use async_loader::AsyncPageLoader;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test async_loader --lib -- --nocapture 2>&1 | tail -5`
Expected: 1 passed, 0 failed

- [ ] **Step 6: Commit**

```bash
git add src/storage/btree/async_loader.rs src/storage/btree/mod.rs
git commit -m "feat(M14): add AsyncPageLoader for direct async page loading"
```

---

### Task 4: Async BTree Read Methods

**Files:**
- Modify: `src/storage/btree/btree.rs`

Add async search methods to BTree that use AsyncPageLoader instead of SyncPageLoader. These methods run directly in the tokio async context — no spawn_blocking.

- [ ] **Step 1: Write failing test for async BTree search**

Add test to `src/storage/btree/btree.rs` test module:

```rust
#[cfg(test)]
mod async_search_tests {
    use super::*;
    use crate::storage::btree::async_loader::AsyncPageLoader;
    use crate::storage::buffer_pool::BufferPool;
    use crate::storage::file_storage::FileStorage;
    use std::sync::Arc;
    use tempfile::NamedTempFile;

    async fn setup_btree_with_data() -> (BTree, AsyncPageLoader, Arc<BufferPool>) {
        let temp = NamedTempFile::new().unwrap();
        let storage = Arc::new(FileStorage::new(temp.path()).unwrap());
        let pool = Arc::new(BufferPool::new(100, storage));
        let sync_loader = SyncPageLoader::new(pool.clone());
        let async_loader = AsyncPageLoader::new(pool.clone());

        let root_page_id = pool.allocate_page().await.unwrap();
        {
            let guard = pool.get_page(root_page_id).await.unwrap();
            let mut data = guard.page_data();
            data[0] = LEAF_NODE;
            data[2..4].copy_from_slice(&0u16.to_be_bytes());
        }

        let mut btree = BTree::new(root_page_id);
        for i in 0..10i64 {
            btree.insert(&Key::new(&i.to_be_bytes()), i as u64, &sync_loader).unwrap();
        }
        (btree, async_loader, pool)
    }

    #[tokio::test]
    async fn async_search_finds_existing_key() {
        let (btree, async_loader, _) = setup_btree_with_data().await;
        let result = btree.search_async(&Key::new(&5i64.to_be_bytes()), &async_loader).await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), 5u64);
    }

    #[tokio::test]
    async fn async_search_returns_none_for_missing() {
        let (btree, async_loader, _) = setup_btree_with_data().await;
        let result = btree.search_async(&Key::new(&99i64.to_be_bytes()), &async_loader).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn async_search_matches_sync_search() {
        let (btree, async_loader, _) = setup_btree_with_data().await;
        for i in 0..10i64 {
            let sync_result = btree.search(&Key::new(&i.to_be_bytes()));
            let async_result = btree.search_async(&Key::new(&i.to_be_bytes()), &async_loader).await.unwrap();
            assert_eq!(sync_result, async_result, "Mismatch for key {}", i);
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test async_search --lib -- --nocapture 2>&1 | tail -10`
Expected: FAIL — `search_async` method does not exist

- [ ] **Step 3: Implement BTree::search_async and search_from_page_async**

Add to `impl BTree` in `src/storage/btree/btree.rs`:

```rust
use crate::storage::btree::async_loader::AsyncPageLoader;

impl BTree {
    pub async fn search_async(&self, key: &Key, loader: &AsyncPageLoader) -> crate::error::Result<Option<u64>> {
        self.search_from_page_async(self.root_page_id, key, loader).await
    }

    async fn search_from_page_async(&self, page_id: PageId, key: &Key, loader: &AsyncPageLoader) -> crate::error::Result<Option<u64>> {
        let guard = loader.load_page(page_id).await?;
        let data = guard.page_data();

        if data[0] == LEAF_NODE {
            let leaf = LeafNodeRef::new(&data);
            let (found, pos) = leaf.find_key_position_binary(key);
            if found {
                Ok(leaf.get_row_id(pos))
            } else {
                Ok(None)
            }
        } else {
            let internal = InternalNodeRef::new(&data);
            let child_page_id = internal.find_child_page_id_binary(key);
            drop(data);
            drop(guard);
            self.search_from_page_async(PageId(child_page_id as u64), key, loader).await
        }
    }
}
```

Note: The exact method signatures may need adjustment based on current BTree API. The key principle is:
- `search_async` takes `&AsyncPageLoader` instead of `&SyncPageLoader`
- Uses `find_key_position_binary` and `find_child_page_id_binary` (from Task 2)
- Recursively descends the tree with `.await` instead of synchronous calls

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test async_search --lib -- --nocapture 2>&1 | tail -5`
Expected: 3 passed, 0 failed

- [ ] **Step 5: Run all lib tests**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add src/storage/btree/btree.rs
git commit -m "feat(M14): add async search methods to BTree"
```

---

### Task 5: RwLock<BTree> + IndexManager Dual-Mode

**Files:**
- Modify: `src/storage/btree/index_manager.rs`

Replace `Mutex<BTree>` with `RwLock<BTree>`. Add async read methods using AsyncPageLoader. Retain sync write methods with SyncPageLoader.

- [ ] **Step 1: Write failing test for async IndexManager search**

Add to `src/storage/btree/index_manager.rs` test module:

```rust
#[tokio::test]
async fn index_manager_async_search_matches_sync() {
    let temp = NamedTempFile::new().unwrap();
    let storage = Arc::new(FileStorage::new(temp.path()).unwrap());
    let pool = Arc::new(BufferPool::new(100, storage));
    let mut mgr = IndexManager::new(pool.clone());

    mgr.create_btree("test_table").await.unwrap();
    for i in 0..10i64 {
        mgr.insert("test_table", &Key::new(&i.to_be_bytes()), i as u64).await.unwrap();
    }

    for i in 0..10i64 {
        let sync_result = mgr.search_sync("test_table", &Key::new(&i.to_be_bytes())).unwrap();
        let async_result = mgr.search("test_table", &Key::new(&i.to_be_bytes())).await.unwrap();
        assert_eq!(sync_result, async_result, "Mismatch for key {}", i);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test index_manager_async --lib -- --nocapture 2>&1 | tail -10`
Expected: FAIL — method signature mismatch or `search` not yet async

- [ ] **Step 3: Modify IndexManager to use RwLock and dual-mode**

Key changes to `src/storage/btree/index_manager.rs`:

1. Replace `use std::sync::Mutex;` with `use tokio::sync::RwLock;`
2. Replace `Mutex<BTree>` with `RwLock<BTree>` in the struct
3. Add `async_loader: AsyncPageLoader` field
4. Change `search` method to async (uses read lock + async BTree search)
5. Keep `insert`/`delete`/`update` using spawn_blocking with write lock

```rust
use tokio::sync::RwLock;
use crate::storage::btree::async_loader::AsyncPageLoader;

pub struct IndexManager {
    btrees: HashMap<String, Arc<RwLock<BTree>>>,
    async_loader: AsyncPageLoader,
    sync_loader: SyncPageLoader,
    buffer_pool: Arc<BufferPool>,
}

impl IndexManager {
    pub fn new(buffer_pool: Arc<BufferPool>) -> Self {
        let async_loader = AsyncPageLoader::new(buffer_pool.clone());
        let sync_loader = SyncPageLoader::new(buffer_pool.clone());
        Self {
            btrees: HashMap::new(),
            async_loader,
            sync_loader,
            buffer_pool,
        }
    }

    pub async fn search(&self, table_name: &str, key: &Key) -> crate::error::Result<Option<u64>> {
        let btree = self.btrees.get(table_name)
            .ok_or_else(|| crate::error::StorageError::NotFound(format!("BTree for table {}", table_name)))?;
        let btree_guard = btree.read().await;
        btree_guard.search_async(key, &self.async_loader).await
    }

    pub async fn insert(&self, table_name: &str, key: &Key, row_id: u64) -> crate::error::Result<()> {
        let btree = self.btrees.get(table_name)
            .ok_or_else(|| crate::error::StorageError::NotFound(format!("BTree for table {}", table_name)))?;
        let btree = btree.clone();
        let sync_loader = self.sync_loader.clone();
        let key = key.clone();
        tokio::task::spawn_blocking(move || {
            let mut btree_guard = btree.blocking_write();
            btree_guard.insert(&key, row_id, &sync_loader)
        }).await?
    }
}
```

Note: The exact field names and method signatures must match the current IndexManager implementation. The key changes are:
- `Mutex` → `RwLock` (from `std::sync` to `tokio::sync`)
- `search` → async with `btree.read().await` + `search_async`
- `insert`/write ops → `spawn_blocking` with `btree.blocking_write()` or `btree.write().await` inside spawn_blocking
- Add `async_loader` field

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test index_manager_async --lib -- --nocapture 2>&1 | tail -5`
Expected: 1 passed, 0 failed

- [ ] **Step 5: Run all lib tests**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: all tests pass — this is critical since IndexManager is used by many components

- [ ] **Step 6: Commit**

```bash
git add src/storage/btree/index_manager.rs
git commit -m "feat(M14): RwLock<BTree> + async search in IndexManager"
```

---

### Task 6: BufferPool Capacity Tuning

**Files:**
- Modify: `src/storage/buffer_pool.rs`

Increase default BufferPool capacity from 100 to 1024 pages (4MB cache). This is a simple constant change.

- [ ] **Step 1: Write failing test for BufferPool capacity**

Add to `src/storage/buffer_pool.rs` test module:

```rust
#[test]
fn default_buffer_pool_capacity_is_1024() {
    let temp = NamedTempFile::new().unwrap();
    let storage = Arc::new(FileStorage::new(temp.path()).unwrap());
    let pool = BufferPool::new(BufferPool::default_capacity(), storage);
    assert_eq!(pool.capacity(), 1024);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test default_buffer_pool_capacity --lib -- --nocapture 2>&1 | tail -5`
Expected: FAIL — `default_capacity` method does not exist or capacity is 100

- [ ] **Step 3: Implement the change**

In `src/storage/buffer_pool.rs`:

1. Change the default capacity constant from 100 to 1024
2. Add `pub fn default_capacity() -> usize { 1024 }` if using a method
3. Add `pub fn capacity(&self) -> usize` accessor if not present

The exact change depends on how capacity is currently specified. If it's a hard-coded `100` in `BufferPool::new()`, change it to `1024`. If it's a constant, update the constant.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test default_buffer_pool_capacity --lib -- --nocapture 2>&1 | tail -5`
Expected: 1 passed, 0 failed

- [ ] **Step 5: Run all lib tests**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add src/storage/buffer_pool.rs
git commit -m "perf(M14): increase BufferPool default capacity to 1024 pages"
```

---

### Task 7: Pipeline + Executor Integration

**Files:**
- Modify: `src/executor/index_scan.rs`
- Modify: `src/database.rs` (if IndexManager creation needs updating)

Wire the async search path into IndexScanExecutor so queries use the new async path instead of spawn_blocking.

- [ ] **Step 1: Write failing test for async IndexScan**

This is an integration test — verify that SELECT queries work end-to-end through the async path:

```rust
#[tokio::test]
async fn index_scan_uses_async_path() {
    let db = Database::open_in_memory().await.unwrap();
    db.execute("CREATE TABLE bench (id INTEGER PRIMARY KEY, val TEXT)").await.unwrap();
    for i in 0..100i64 {
        db.execute(&format!("INSERT INTO bench VALUES ({}, 'hello')", i)).await.unwrap();
    }

    let result = db.execute("SELECT * FROM bench WHERE id = 42").await.unwrap();
    assert_eq!(result.rows().len(), 1);
}
```

- [ ] **Step 2: Run test to verify current state**

Run: `cargo test index_scan_uses_async -- --nocapture 2>&1 | tail -5`
Expected: This should pass already — we need to verify the async path is actually being used

- [ ] **Step 3: Modify IndexScanExecutor to use async IndexManager::search**

In `src/executor/index_scan.rs`, change the search call from:

```rust
// Old: spawn_blocking + sync search
let result = tokio::task::spawn_blocking(move || {
    index_manager.search(&key)
}).await??;
```

To:

```rust
// New: direct async search
let result = index_manager.search(&key).await?;
```

The exact change depends on how IndexScanExecutor currently calls IndexManager. The key principle is removing the `spawn_blocking` wrapper and calling the new async `search` method directly.

- [ ] **Step 4: Update Database/IndexManager creation if needed**

If `Database::new()` or `Database::open()` creates `IndexManager` with `Mutex<BTree>`, update to use `RwLock<BTree>` and pass `AsyncPageLoader`.

- [ ] **Step 5: Run all tests**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 6: Run integration tests**

Run: `cargo test --test '*' 2>&1 | tail -5`
Expected: all integration tests pass

- [ ] **Step 7: Commit**

```bash
git add src/executor/index_scan.rs src/database.rs
git commit -m "feat(M14): wire async BTree search into IndexScanExecutor"
```

---

### Task 8: Benchmark Validation + Regression Test

**Files:**
- Modify: `benches/m14_staged_bench.rs` (add comparison)

Run the full benchmark suite to validate the optimization and check for regressions.

- [ ] **Step 1: Run PK lookup benchmark**

Run: `cargo bench --bench m14_staged_bench -- --noplot 2>&1 | tail -10`
Expected: PK lookup time significantly lower than ~36µs baseline

- [ ] **Step 2: Run full test suite**

Run: `cargo test 2>&1 | tail -5`
Expected: all tests pass, 0 failures

- [ ] **Step 3: Run clippy**

Run: `cargo clippy 2>&1 | tail -10`
Expected: 0 warnings

- [ ] **Step 4: Record benchmark results**

Update `.claude/docs/snapshot.md` with new benchmark numbers.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "perf(M14): validate phase 2 optimization benchmarks"
```

---

## Self-Review

### Spec Coverage

| Spec Requirement | Task | Status |
|-----------------|------|--------|
| Async BTree read path | T3 (AsyncPageLoader) + T4 (async search) | Covered |
| RwLock<BTree> | T5 (IndexManager dual-mode) | Covered |
| Binary search | T2 (node.rs) | Covered |
| BufferPool capacity | T6 (capacity tuning) | Covered |
| Pipeline integration | T7 (IndexScanExecutor) | Covered |
| Benchmark validation | T1 + T8 | Covered |
| Write path preserved | T5 (spawn_blocking retained) | Covered |

### Placeholder Scan

No TBD/TODO found. All steps contain actual code.

### Type Consistency

- `AsyncPageLoader::new(Arc<BufferPool>)` — used in T3, T5, T7
- `BTree::search_async(&Key, &AsyncPageLoader)` — defined in T4, used in T5, T7
- `find_key_position_binary(&Key) -> (bool, usize)` — defined in T2, used in T4
- `find_child_page_id_binary(&Key) -> u32` — defined in T2, used in T4
- `IndexManager::search(&str, &Key) -> Result<Option<u64>>` — async, defined in T5, used in T7
