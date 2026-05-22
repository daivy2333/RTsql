# M14 Phase 2: Full-Path Query Optimization Design

> Date: 2026-05-22
> Status: Approved (auto mode)
> Milestone: M14 (Query Path Optimization)

## Goal

PK lookup from ~36µs to as low as possible (target ~10µs, 3-5x improvement over baseline ~49µs).

## Current Bottleneck Analysis

| Bottleneck | Cost | Root Cause |
|------------|------|------------|
| spawn_blocking + block_on | ~25µs | Async→sync→async thread switching chain |
| Mutex<BTree> | ~5µs | Global exclusive lock for all BTree ops |
| Linear search in BTree | ~5µs | O(n) scan instead of O(log n) binary search |
| BufferPool capacity=100 | varies | High eviction rate, frequent disk I/O |
| PageGuard Mutex per access | ~1µs | std::sync::Mutex lock/unlock on every page access |

## Architecture: Async BTree Read Path

### Core Change

BTree read path moves from `spawn_blocking + SyncPageLoader::block_on` to direct async execution.

```
Current path (~36µs):
  async → spawn_blocking → Mutex<BTree>::lock → SyncPageLoader::block_on → BufferPool::get_page → compute

Optimized path (target ~10µs):
  async → RwLock<BTree>::read → async BufferPool::get_page → compute
```

### Key Design Decisions

1. **Read path async**: `BTree::search` / `get` → async methods, execute directly in tokio context
2. **Write path preserved**: `insert` / `delete` / `update` keep spawn_blocking, avoid write path complexity
3. **RwLock replaces Mutex**: `Arc<Mutex<BTree>>` → `Arc<RwLock<BTree>>`, read ops use read lock
4. **AsyncPageLoader replaces SyncPageLoader** for read path; SyncPageLoader retained for write path
5. **Binary search**: `LeafNodeRef::find_key_position` and `InternalNodeRef::find_child_page_id` → O(log n)
6. **BufferPool capacity**: 100 → 1024 pages

## Component Design

### AsyncPageLoader (new)

```rust
pub struct AsyncPageLoader {
    buffer_pool: Arc<BufferPool>,
}

impl AsyncPageLoader {
    pub async fn load_page(&self, page_id: PageId) -> Result<PageGuard> {
        self.buffer_pool.get_page(page_id).await
    }
}
```

No `block_on`, no `Handle`, no thread switching. Direct async call chain.

### BTree Async Read Path

```rust
pub async fn search_async(&self, key: &[u8], loader: &AsyncPageLoader) -> Result<Option<RowId>> {
    let key_obj = Key::new(key);
    self.search_from_page_async(self.root_page_id, &key_obj, loader).await
}

async fn search_from_page_async(&self, page_id: PageId, key: &Key, loader: &AsyncPageLoader) -> Result<Option<RowId>> {
    let guard = loader.load_page(page_id).await?;
    let data_guard = guard.page_data();
    if data_guard[0] == LEAF_NODE {
        let leaf = LeafNodeRef::new(&data_guard);
        let (found, pos) = leaf.find_key_position_binary(key);  // binary search
        if found { Ok(leaf.get_row_id(pos)) } else { Ok(None) }
    } else {
        let internal = InternalNodeRef::new(&data_guard);
        let child_page_id = internal.find_child_page_id_binary(key);
        drop(data_guard);
        drop(guard);
        self.search_from_page_async(PageId(child_page_id as u64), key, loader).await
    }
}
```

### IndexManager Dual-Mode

```rust
pub struct IndexManager {
    btree: Arc<RwLock<BTree>>,        // RwLock replaces Mutex
    async_loader: AsyncPageLoader,     // read path
    sync_loader: SyncPageLoader,       // write path (preserved)
    buffer_pool: Arc<BufferPool>,
}

// Read ops → async (no spawn_blocking)
pub async fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
    let btree = self.btree.read().await;
    btree.search_async(key, &self.async_loader).await
}

pub async fn scan_all(&self) -> Result<Vec<(Key, RowId)>> {
    let btree = self.btree.read().await;
    btree.scan_all_async(&self.async_loader).await
}

// Write ops → spawn_blocking (preserved)
pub async fn insert(...) -> Result<()> {
    let btree = self.btree.clone();
    let sync_loader = self.sync_loader.clone();
    spawn_blocking(move || {
        let btree = btree.write().unwrap();
        btree.insert_with_loader(&key, row_id, &sync_loader)
    }).await?
}
```

### Binary Search in LeafNodeRef / InternalNodeRef

```rust
// LeafNodeRef::find_key_position_binary
pub fn find_key_position_binary(&self, key: &Key) -> (bool, usize) {
    let count = self.key_count();
    let mut lo = 0;
    let mut hi = count;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if let Some(mid_key) = self.get_key(mid) {
            match mid_key.cmp(key) {
                Ordering::Less => lo = mid + 1,
                Ordering::Greater => hi = mid,
                Ordering::Equal => return (true, mid),
            }
        } else {
            hi = mid;
        }
    }
    (false, lo)
}
```

### BufferPool Capacity Tuning

- Capacity: 100 → 1024 pages (4MB cache, reasonable for embedded DB)
- This reduces eviction frequency, especially for multi-table scenarios

## Data Flow (Optimized)

```
SELECT * FROM bench WHERE id = 42

Optimized path:
  pipeline::execute (async)
    → plan_cache hit (skip parse+plan)
    → IndexScanExecutor::next()
      → IndexManager::search (async, no spawn_blocking)
        → RwLock<BTree>::read() (async read lock, concurrent readers OK)
        → BTree::search_async (async)
          → AsyncPageLoader::load_page (direct await BufferPool::get_page)
          → binary search (O(log n))
          → return RowId
      → read_tuple_from_data_page (async BufferPool::get_page)
```

## Error Handling

- async BTree operation failure → StorageError, consistent with existing error system
- RwLock write lock contention → write ops wait, read ops unaffected
- BufferPool cache miss → normal disk I/O path (async FileStorage)

## Testing Strategy

1. **Precision performance test**: staged timing of parse/plan/BTree search/BufferPool per stage
2. **async BTree correctness**: verify search_async results match sync version
3. **Concurrent read test**: multiple coroutines concurrent read RwLock<BTree>
4. **Regression test**: full lib + integration tests pass
5. **Benchmark comparison**: PK lookup latency before vs after optimization

## BDD Default Assumptions

- MVCC visibility logic unchanged (async BTree only affects index lookup, not version chain)
- WAL write path unchanged (write ops retain spawn_blocking)
- DDL cache clearing unchanged (CREATE/DROP TABLE still clears plan_cache + rebuilds IndexManager)
- Transaction isolation levels unchanged (Repeatable Read still works)

## Implementation Steps (Outline)

1. T1: Precision performance test (staged timing)
2. T2: Add AsyncPageLoader + async BTree read methods
3. T3: IndexManager dual-mode (async read + sync write)
4. T4: RwLock<BTree> replaces Mutex<BTree>
5. T5: Binary search in LeafNodeRef + InternalNodeRef
6. T6: BufferPool capacity tuning (100 → 1024)
7. T7: Pipeline + Executor integration (IndexScanExecutor uses async search)
8. T8: Benchmark validation + regression test

## Risk Assessment

| Risk | Mitigation |
|------|------------|
| async BTree recursive search may stack overflow | BTree depth typically 1-3, safe |
| RwLock<BTree> write starvation | Write ops are infrequent vs reads |
| Binary search on serialized keys | Keys are fixed 32 bytes, comparable |
| BufferPool capacity increase memory usage | 1024 pages = 4MB, acceptable for embedded DB |