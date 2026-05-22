# M14 Phase 2 T2: IndexManager Search Optimization Design

> Created: 2026-05-22
> Status: Approved
> Target: PK 查询从 ~34µs → ~5-8µs (5-6x 提速)

## Executive Summary

通过重构 IndexManager 架构，启用 async search 路径，消除 spawn_blocking 调度开销和 RwLock 锁争用，实现 PK 查询 5-6x 性能提升。

**核心瓶颈**（Profiling 数据）：
- IndexManager.search: 51µs (81% of total)
- spawn_blocking + SyncPageLoader 调度开销: ~25µs
- std::sync::RwLock 锁争用: ~5µs
- 实际 BTree.search 计算: ~21µs

**优化策略**：
1. 启用 async search 路径（消除 ~25µs 调度开销）
2. 移除 RwLock 包装，改用 AtomicPageId 无锁设计（消除 ~5µs 锁争用）
3. 保持写操作 sync 路径（insert/update/delete 不是瓶颈）

## Architecture Changes

### Current Architecture

```rust
pub struct IndexManager {
    btree: Arc<std::sync::RwLock<BTree>>,  // RwLock 包装
    async_loader: AsyncPageLoader,
    row_to_key: RwLock<HashMap<RowId, Vec<u8>>>,
}

pub async fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
    let btree = self.btree.clone();
    let key_vec = key.to_vec();
    tokio::task::spawn_blocking(move || {
        let btree_guard = btree.read().unwrap();
        btree_guard.search(&key_vec)
    })
    .await?
}
```

**问题**：
- spawn_blocking 调度开销 (~25µs)
- RwLock::read() 锁争用 (~5µs)
- SyncPageLoader::block_on 包装 BufferPool

### New Architecture

```rust
pub struct IndexManager {
    root_page_id: AtomicU64,              // 无锁访问根页
    sync_loader: Arc<SyncPageLoader>,     // 写操作仍用 sync
    async_loader: AsyncPageLoader,        // 读操作用 async
    row_to_key: RwLock<HashMap<RowId, Vec<u8>>>,
}

impl IndexManager {
    pub async fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
        let root_page_id = PageId(self.root_page_id.load(Ordering::Acquire));
        let key_obj = Key::new(key);
        self.search_from_page_async(root_page_id, &key_obj).await
    }

    pub async fn scan_all(&self) -> Result<Vec<(Vec<u8>, RowId)>> {
        let root_page_id = PageId(self.root_page_id.load(Ordering::Acquire));
        self.scan_all_async_from_root(root_page_id).await
    }

    // 写操作保持 sync 路径（不是瓶颈）
    pub async fn insert(&self, key: &[u8], row_id: RowId) -> Result<()> {
        let root_page_id = self.root_page_id.load(Ordering::Acquire);
        let sync_loader = self.sync_loader.clone();
        let key_vec = key.to_vec();

        tokio::task::spawn_blocking(move || {
            let btree = BTree::from_root(PageId(root_page_id), sync_loader);
            btree.insert(&key_vec, row_id)
        }).await??;

        self.row_to_key.write().await.insert(row_id, key.to_vec());
        Ok(())
    }

    pub async fn update(&self, key: &[u8], new_row_id: RowId) -> Result<()> {
        let root_page_id = self.root_page_id.load(Ordering::Acquire);
        let sync_loader = self.sync_loader.clone();
        let key_vec = key.to_vec();

        tokio::task::spawn_blocking(move || {
            let btree = BTree::from_root(PageId(root_page_id), sync_loader);
            btree.update(&key_vec, new_row_id)
        }).await??;

        self.row_to_key.write().await.insert(new_row_id, key.to_vec());
        Ok(())
    }

    pub async fn delete(&self, key: &[u8]) -> Result<()> {
        if let Some(row_id) = self.search(key).await? {
            self.row_to_key.write().await.remove(&row_id);
        }

        let root_page_id = self.root_page_id.load(Ordering::Acquire);
        let sync_loader = self.sync_loader.clone();
        let key_vec = key.to_vec();

        tokio::task::spawn_blocking(move || {
            let btree = BTree::from_root(PageId(root_page_id), sync_loader);
            btree.delete(&key_vec)
        }).await?
    }
}
```

**改进**：
- 读操作无锁访问 root_page_id（AtomicU64::load）
- 直接 async 调用 BufferPool（无 spawn_blocking）
- 写操作保持 sync 路径（insert/update/delete 不是瓶颈）

## Implementation Details

### Task 1: Add AtomicPageId to IndexManager

**改动文件**: `src/storage/btree/index_manager.rs`

**步骤**：
1. 移除 `Arc<std::sync::RwLock<BTree>>` 字段
2. 添加 `root_page_id: AtomicU64` 字段
3. 保留 `sync_loader: Arc<SyncPageLoader>` 和 `async_loader: AsyncPageLoader`
4. 修改 `IndexManager::new()` 初始化逻辑

**代码示例**：
```rust
use std::sync::atomic::{AtomicU64, Ordering};

pub struct IndexManager {
    root_page_id: AtomicU64,
    sync_loader: Arc<SyncPageLoader>,
    async_loader: AsyncPageLoader,
    row_to_key: RwLock<HashMap<RowId, Vec<u8>>>,
}

impl IndexManager {
    pub fn new(buffer_pool: Arc<BufferPool>) -> Result<Self> {
        let sync_loader = Arc::new(SyncPageLoader::new(buffer_pool.clone()));
        let async_loader = AsyncPageLoader::new(buffer_pool.clone());

        // 创建 BTree 并获取 root_page_id
        let btree = BTree::new(sync_loader.clone())?;
        let root_page_id = btree.root_page_id().0;

        Ok(Self {
            root_page_id: AtomicU64::new(root_page_id),
            sync_loader,
            async_loader,
            row_to_key: RwLock::new(HashMap::new()),
        })
    }
}
```

**注意**：
- BTree::new() 需要在 spawn_blocking 或 sync 上下文调用（因为使用 SyncPageLoader）
- 初始化后不再持有 BTree 实例，仅存储 root_page_id

### Task 2: Implement async search path

**改动文件**: `src/storage/btree/index_manager.rs`

**新增方法**：
```rust
impl IndexManager {
    /// Async search — direct async path without spawn_blocking
    pub async fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
        let root_page_id = PageId(self.root_page_id.load(Ordering::Acquire));
        let key_obj = Key::new(key);

        self.search_from_page_async(root_page_id, &key_obj).await
    }

    /// Recursive async search from a page
    fn search_from_page_async(
        &self,
        page_id: PageId,
        key: &Key,
    ) -> impl Future<Output = Result<Option<RowId>>> + Send {
        async move {
            let child_page_id = {
                let guard = self.async_loader.load_page(page_id).await?;
                let data_guard = guard.page_data();

                if data_guard[0] == LEAF_NODE {
                    let leaf = LeafNodeRef::new(&data_guard);
                    let (found, pos) = leaf.find_key_position_binary(key);
                    if found {
                        return Ok(leaf.get_row_id(pos));
                    } else {
                        return Ok(None);
                    }
                } else {
                    let internal = InternalNodeRef::new(&data_guard);
                    internal.find_child_page_id_binary(key)
                }
            }; // guard and data_guard dropped here

            self.search_from_page_async(PageId(child_page_id as u64), key).await
        }
    }
}
```

**核心优化**：
- 无 spawn_blocking 调度开销
- 无 RwLock 锁争用
- 直接 async 调用 BufferPool

### Task 3: Implement async scan_all path

**改动文件**: `src/storage/btree/index_manager.rs`

**新增方法**：
```rust
impl IndexManager {
    /// Async scan all entries — direct async path
    pub async fn scan_all(&self) -> Result<Vec<(Vec<u8>, RowId)>> {
        let root_page_id = PageId(self.root_page_id.load(Ordering::Acquire));
        self.scan_all_async_from_root(root_page_id).await
    }

    async fn scan_all_async_from_root(&self, root_page_id: PageId) -> Result<Vec<(Vec<u8>, RowId)>> {
        let mut results = Vec::new();
        let mut page_id = root_page_id;

        while page_id.0 != 0 {
            let guard = self.async_loader.load_page(page_id).await?;
            let data_guard = guard.page_data();
            let leaf = LeafNodeRef::new(&data_guard);

            let count = leaf.key_count();
            let mut entries = Vec::with_capacity(count);
            for i in 0..count {
                if let (Some(key), Some(row_id)) = (leaf.get_key(i), leaf.get_row_id(i)) {
                    entries.push((key.as_bytes().to_vec(), row_id));
                }
            }

            let next_page_u32 = leaf.next_leaf_page_id();
            drop(data_guard);
            drop(guard);

            results.extend(entries);
            page_id = PageId(next_page_u32 as u64);
        }

        Ok(results)
    }
}
```

### Task 4: Keep write operations sync (insert/update/delete)

**改动文件**: `src/storage/btree/index_manager.rs`

**策略**：
- 写操作保持 spawn_blocking + SyncPageLoader 路径
- 需要重新创建临时 BTree 实例用于写操作
- 写操作完成后更新 root_page_id（如果 split 发生）

**代码示例**：
```rust
impl IndexManager {
    pub async fn insert(&self, key: &[u8], row_id: RowId) -> Result<()> {
        let root_page_id = self.root_page_id.load(Ordering::Acquire);
        let sync_loader = self.sync_loader.clone();
        let key_vec = key.to_vec();

        tokio::task::spawn_blocking(move || {
            // 创建临时 BTree 实例用于写操作
            let btree = BTree::from_root(root_page_id, sync_loader);
            btree.insert(&key_vec, row_id)?;

            // 如果 split 发生，root_page_id 可能改变（当前简化版无 split）
            Ok(())
        }).await??;

        self.row_to_key.write().await.insert(row_id, key.to_vec());
        Ok(())
    }
}
```

**注意**：
- 当前 BTree 实现是简化版，不支持 split/merge
- 如果未来实现 split，需要更新 root_page_id
- 写操作不是瓶颈（profiling 数据未显示），保持 sync 路径合理

### Task 5: Add BTree::from_root() helper

**改动文件**: `src/storage/btree/btree.rs`

**新增方法**：
```rust
impl BTree {
    /// Create BTree from existing root page (for write operations)
    pub fn from_root(root_page_id: PageId, loader: Arc<SyncPageLoader>) -> Self {
        Self {
            loader,
            root_page_id,
        }
    }
}
```

**用途**：
- 写操作需要临时 BTree 实例
- 避免长期持有 RwLock<BTree>

## Testing Strategy

### Unit Tests

**文件**: `tests/index_manager_test.rs`

**新增测试**：
1. `test_async_search_basic()` — 基本 async search 功能
2. `test_async_scan_all()` — async scan_all 功能
3. `test_concurrent_reads()` — 并发读无锁争用验证
4. `test_write_operations_still_work()` — 写操作仍正常工作

**测试策略**：
- 使用 `RTSQL_PROFILING=1` 验证性能改进
- 对比优化前后 timing 数据
- 验证并发读性能（多线程同时 search）

### Integration Tests

**文件**: `tests/executor_test.rs`

**验证**：
- IndexScanExecutor 使用优化后的 IndexManager.search
- Pipeline 执行时间对比
- 缓存命中场景性能验证

### Performance Validation

**工具**: `examples/bench_minimal.rs`（RTSQL_PROFILING=1）

**验证步骤**：
1. 运行 `cargo run --example bench_minimal`（优化前）
2. 记录 index_manager_search 时间（预期 ~51µs）
3. 实施优化后运行相同命令
4. 记录优化后时间（预期 ~5-8µs）
5. 计算提速倍数（预期 5-6x）

**Profiling 输出对比**：

| Stage | 优化前 (µs) | 优化后 (µs) | 提速 |
|-------|------------|------------|------|
| index_manager_search | 51 | 5-8 | 6-10x |
| executor_execution | 57 | 10-12 | 5-6x |
| Total | 63 | 15-17 | 3-4x |

**注意**：
- 提速倍数可能因硬件和缓存状态略有差异
- 应验证多次运行稳定性

## Risk Analysis

### Risk 1: AtomicPageId 无锁设计可能引入 race condition

**场景**：写操作修改 root_page_id（split），读操作同时访问旧 root_page_id

**当前状态**：BTree 是简化版，不支持 split/merge，root_page_id 不改变

**未来风险**：如果实现 split，需要：
- 写操作使用 AtomicPageId::swap 更新 root_page_id
- 读操作使用 Ordering::Acquire 获取最新 root_page_id
- 可能读取到旧 root_page_id（但 BTree 页结构不变，仍可正常 search）

**缓解策略**：
- 当前简化版无风险
- 未来实现 split 时，需确保 root_page_id 更新是 atomic 操作

### Risk 2: 写操作临时 BTree 实例可能引入开销

**场景**：每次 insert/update/delete 都创建临时 BTree 实例

**当前影响**：写操作不是瓶颈（profiling 数据未显示）

**未来风险**：如果写操作频率增加，可能引入开销

**缓解策略**：
- 保持 spawn_blocking 包装（减少 Tokio 运行时影响）
- 监控写操作性能，如发现瓶颈再优化

### Risk 3: 测试覆盖可能不足

**场景**：新增 async search/scan_all 方法需要测试验证

**缓解策略**：
- 先运行现有测试验证基本功能
- 新增专门测试覆盖 async 路径
- 使用 profiling 验证性能改进

## Success Criteria

1. **功能验证**：
   - 所有现有测试通过
   - 新增 async search/scan_all 测试通过

2. **性能验证**：
   - PK 查询时间从 ~34µs → ~5-8µs (5-6x 提速)
   - index_manager_search 从 ~51µs → ~5-8µs (6-10x 提速)
   - Profiling 输出显示调度开销和锁争用消除

3. **稳定性验证**：
   - 多次运行性能稳定
   - 并发读无性能退化
   - 写操作仍正常工作

## Implementation Order

1. **Task 1**: Add AtomicPageId to IndexManager（架构调整）
2. **Task 2**: Implement async search path（核心优化）
3. **Task 3**: Implement async scan_all path（次要优化）
4. **Task 4**: Keep write operations sync（保持现状）
5. **Task 5**: Add BTree::from_root() helper（辅助方法）
6. **Validation**: Run tests + profiling（验证改进）

## Notes

- **Write operations not optimized**: 写操作不是瓶颈，保持 sync 路径合理
- **No split/merge support yet**: 当前 BTree 是简化版，root_page_id 不改变
- **AtomicPageId ordering**: 使用 Ordering::Acquire/Release 确保 memory ordering
- **Future optimization**: 如需进一步优化，可考虑 BTree search 算法优化（SIMD、key comparison reduction）