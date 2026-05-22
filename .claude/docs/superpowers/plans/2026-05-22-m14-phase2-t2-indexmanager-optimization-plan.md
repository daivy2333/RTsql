# M14 Phase 2 T2: IndexManager Async Search Optimization Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 重构 IndexManager 架构，启用 async search 路径，消除 spawn_blocking 调度开销和 RwLock 锁争用，实现 PK 查询 5-6x 性能提升（34µs → 5-8µs）。

**Architecture:** 移除 RwLock<BTree> 包装，改用 AtomicPageId 存储根页 ID。读操作（search/scan_all）无锁访问 root_page_id，直接 async 调用 BufferPool。写操作（insert/update/delete）保持 sync 路径，使用临时 BTree 实例。

**Tech Stack:** Rust + Tokio async runtime + AtomicU64 + AsyncPageLoader + SyncPageLoader

---

## File Structure

**Modified files:**
- `src/storage/btree/index_manager.rs` — 架构调整，新增 async search/scan_all
- `src/storage/btree/btree.rs` — 新增 from_root() helper

**Test files:**
- `tests/index_manager_test.rs` — 新增 async search/scan_all 测试
- `tests/executor_test.rs` — 验证 IndexScanExecutor 使用优化后的 search

**Profiling validation:**
- `examples/bench_minimal.rs` — RTSQL_PROFILING=1 验证性能改进

---

## Implementation Tasks

### Task 1: Add AtomicPageId to IndexManager

**Files:**
- Modify: `src/storage/btree/index_manager.rs`（移除 RwLock<BTree>，添加 AtomicPageId）

**Goal:** 重构 IndexManager 结构，移除 RwLock 包装，改用 AtomicPageId 无锁访问根页。

- [ ] **Step 1: 修改 IndexManager 结构体**

修改 `src/storage/btree/index_manager.rs`：

```rust
use std::sync::atomic::{AtomicU64, Ordering};

pub struct IndexManager {
    root_page_id: AtomicU64,              // 替换 Arc<std::sync::RwLock<BTree>>
    sync_loader: Arc<SyncPageLoader>,     // 写操作仍用 sync
    async_loader: AsyncPageLoader,        // 读操作用 async
    row_to_key: RwLock<HashMap<RowId, Vec<u8>>>,
}
```

- [ ] **Step 2: 修改 IndexManager::new() 初始化逻辑**

修改 `src/storage/btree/index_manager.rs`：

```rust
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

- [ ] **Step 3: 运行现有测试验证基本功能**

运行：`cargo test --test index_manager_test`
预期：编译失败（后续 Task 会修复方法签名）

- [ ] **Step 4: 提交架构调整**

```bash
git add src/storage/btree/index_manager.rs
git commit -m "refactor(M14-T2): replace RwLock<BTree> with AtomicPageId"
```

---

### Task 2: Implement async search path

**Files:**
- Modify: `src/storage/btree/index_manager.rs`（新增 async search 方法）

**Goal:** 实现无锁 async search，消除 spawn_blocking 调度开销。

- [ ] **Step 1: 实现 async search 方法**

修改 `src/storage/btree/index_manager.rs`，新增：

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

- [ ] **Step 2: 添加必要的 imports**

修改 `src/storage/btree/index_manager.rs`：

```rust
use crate::storage::{
    btree::node::{InternalNodeRef, LeafNodeRef, LEAF_NODE},
    page_format::{Key, RowId},
    PageId, Result,
};
use std::future::Future;
```

- [ ] **Step 3: 运行现有测试验证 search 功能**

运行：`cargo test --test index_manager_test`
预期：部分测试通过（search 相关）

- [ ] **Step 4: 提交 async search 实现**

```bash
git add src/storage/btree/index_manager.rs
git commit -m "feat(M14-T2): implement async search without spawn_blocking"
```

---

### Task 3: Implement async scan_all path

**Files:**
- Modify: `src/storage/btree/index_manager.rs`（新增 async scan_all 方法）

**Goal:** 实现无锁 async scan_all，遍历所有叶子页。

- [ ] **Step 1: 实现 async scan_all 方法**

修改 `src/storage/btree/index_manager.rs`，新增：

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

- [ ] **Step 2: 运行现有测试验证 scan_all 功能**

运行：`cargo test --test index_manager_test`
预期：所有测试通过（search + scan_all）

- [ ] **Step 3: 提交 async scan_all 实现**

```bash
git add src/storage/btree/index_manager.rs
git commit -m "feat(M14-T2): implement async scan_all without spawn_blocking"
```

---

### Task 4: Keep write operations sync

**Files:**
- Modify: `src/storage/btree/index_manager.rs`（重构 insert/update/delete）
- Modify: `src/storage/btree/btree.rs`（新增 from_root() helper，见 Task 5）

**Goal:** 保持写操作 sync 路径，使用临时 BTree 实例（需要先完成 Task 5 的 BTree::from_root()）。

**注意：此 Task 依赖 Task 5，应先完成 Task 5 再执行此 Task。**

- [ ] **Step 1: 等待 Task 5 完成**

Task 5 会添加 `BTree::from_root()` helper，此 Step 等待其完成后继续。

- [ ] **Step 2: 重构 insert 方法**

修改 `src/storage/btree/index_manager.rs`：

```rust
impl IndexManager {
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
}
```

- [ ] **Step 3: 重构 update 方法**

修改 `src/storage/btree/index_manager.rs`：

```rust
impl IndexManager {
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
}
```

- [ ] **Step 4: 重构 delete 方法**

修改 `src/storage/btree/index_manager.rs`：

```rust
impl IndexManager {
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

- [ ] **Step 5: 运行所有测试验证写操作**

运行：`cargo test --test index_manager_test`
预期：所有测试通过（search + scan_all + insert + update + delete）

- [ ] **Step 6: 提交写操作重构**

```bash
git add src/storage/btree/index_manager.rs
git commit -m "refactor(M14-T2): keep write operations sync with temporary BTree instance"
```

---

### Task 5: Add BTree::from_root() helper

**Files:**
- Modify: `src/storage/btree/btree.rs`（新增 from_root() 方法）

**Goal:** 添加 BTree::from_root() helper，用于写操作临时创建 BTree 实例。

- [ ] **Step 1: 实现 BTree::from_root() 方法**

修改 `src/storage/btree/btree.rs`，在 `impl BTree` 中新增：

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

- [ ] **Step 2: 验证编译通过**

运行：`cargo build`
预期：编译成功（from_root() 方法可用）

- [ ] **Step 3: 提交 BTree helper**

```bash
git add src/storage/btree/btree.rs
git commit -m "feat(M14-T2): add BTree::from_root() helper for write operations"
```

---

### Task 6: Validation and performance testing

**Files:**
- Run: `cargo test`（所有测试）
- Run: `RTSQL_PROFILING=1 cargo run --example bench_minimal`（性能验证）

**Goal:** 验证功能正确性 + 性能改进（预期 5-6x 提速）。

- [ ] **Step 1: 运行所有测试**

运行：`cargo test`
预期：所有测试通过（83 lib tests + 74 integration tests）

- [ ] **Step 2: 运行 profiling benchmark（优化后）**

运行：`RTSQL_PROFILING=1 cargo run --example bench_minimal`
预期输出：

```
Stage                    | Time (µs) | % Total
-------------------------|-----------|--------
executor_execution      |      10-12 |   70-80%
index_manager_search    |       5-8  |   35-50%
executor_creation       |       2-3  |   15-20%
cache_hit_check         |       0    |    0%
parse_and_plan          |       0    |    0%
-------------------------|-----------|--------
Total                   |      15-17 |  100.0%
```

**对比优化前**：

| Stage | 优化前 (µs) | 优化后 (µs) | 提速 |
|-------|------------|------------|------|
| index_manager_search | 51 | 5-8 | **6-10x** |
| executor_execution | 57 | 10-12 | **5-6x** |
| Total | 63 | 15-17 | **3-4x** |

- [ ] **Step 3: 验证性能稳定性**

多次运行（至少 3 次）验证性能稳定：
```bash
for i in {1..3}; do
    RTSQL_PROFILING=1 cargo run --example bench_minimal
done
```

预期：每次运行 index_manager_search 时间在 5-8µs 范围内。

- [ ] **Step 4: 提交验证记录**

```bash
git add .claude/docs/tasks.md .claude/docs/snapshot.md
git commit -m "docs(M14-T2): record validation results and performance improvement"
```

---

## Execution Order

**重要**：Task 4 依赖 Task 5，必须按以下顺序执行：

1. Task 1: Add AtomicPageId to IndexManager（架构调整）
2. Task 2: Implement async search path（核心优化）
3. Task 3: Implement async scan_all path（次要优化）
4. **Task 5: Add BTree::from_root() helper**（辅助方法，优先于 Task 4）
5. Task 4: Keep write operations sync（依赖 Task 5）
6. Task 6: Validation and performance testing（验证改进）

---

## Self-Review Checklist

**Spec Coverage:**
- ✅ Task 1-2-3: 覆盖设计文档"启用 async search 路径"
- ✅ Task 1: 覆盖设计文档"移除 RwLock 包装，改用 AtomicPageId"
- ✅ Task 4: 覆盖设计文档"保持写操作 sync 路径"
- ✅ Task 5: 覆盖设计文档"Add BTree::from_root() helper"
- ✅ Task 6: 覆盖设计文档"Validation + Performance Testing"

**Placeholder Scan:**
- ✅ 无 TBD/TODO/不完整部分
- ✅ 所有代码步骤包含完整代码
- ✅ 所有测试步骤包含具体命令和预期输出

**Type Consistency:**
- ✅ AtomicU64 在 Task 1 定义，后续 Task 使用一致
- ✅ PageId(PageId(...)) 类型转换在所有 Task 一致
- ✅ Ordering::Acquire 在所有 Task 一致
- ✅ Key::new(key) 在 Task 2/3/4 一致

**No Contradictions:**
- ✅ Execution Order 明确说明 Task 4 依赖 Task 5
- ✅ 所有 Task 目标与设计文档一致

---

## Success Criteria

1. **功能验证**：所有测试通过（83 lib + 74 integration）
2. **性能验证**：PK 查询从 ~34µs → ~5-8µs (5-6x 提速)
3. **稳定性验证**：多次运行性能稳定，无退化

---

**Plan complete.** 下一步：执行实施计划。