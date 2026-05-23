# IndexScanAllExecutor 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Executor 层支持非唯一索引查询，返回所有匹配键的行（完整端到端功能：IndexManager → Executor → SQL层）。

**Architecture:** 扩展 IndexManager 新增 search_all 方法；新增 IndexScanAllExecutor 复用 MVCC 逻辑逐行返回；SQL层新增 PhysicalPlan::IndexScanAll 节点，Pipeline 创建 executor。

**Tech Stack:** Rust, Tokio async/await, Arc<BufferPool>, MVCC Snapshot, Executor trait

---

## File Structure

| 文件 | 责任 | 创建/修改 |
|------|------|----------|
| `src/storage/btree/index_manager.rs` | IndexManager::search_all 方法 | 修改（新增方法） |
| `src/executor/index_scan_all.rs` | IndexScanAllExecutor 实现 | 创建（新文件） |
| `src/executor/mod.rs` | 导出 IndexScanAllExecutor | 修改（新增 pub mod） |
| `tests/executor_test.rs` | 非唯一索引 executor 测试 | 修改（新增测试） |
| `src/planner/mod.rs` | PhysicalPlan::IndexScanAll 节点 | 修改（新增 enum variant） |
| `src/pipeline.rs` | IndexScanAllExecutor 创建逻辑 | 修改（新增 match 分支） |

---

## Task 1: IndexManager::search_all 方法

**Files:**
- Modify: `src/storage/btree/index_manager.rs:43-78`（在 search 方法后新增 search_all）
- Test: `tests/storage_test.rs`（暂不新增，由 Executor 测试覆盖）

**依赖**：BTree::search_all 已存在（src/storage/btree/btree.rs:437）

---

### Step 1: 在 IndexManager 新增 search_all 公开方法

在 `src/storage/btree/index_manager.rs` 的 `search` 方法（第 43-47 行）后，新增 `search_all` 方法：

```rust
/// Async search_all — find all RowIds matching a key (for non-unique indexes)
pub async fn search_all(&self, key: &[u8]) -> Result<Vec<RowId>> {
    let root_page_id = PageId(self.root_page_id.load(Ordering::Acquire));
    let key_obj = Key::new(key);

    self.search_all_from_page_async(root_page_id, &key_obj).await
}
```

**位置**：在 `search` 方法后（约第 48 行）

---

### Step 2: 实现 search_all_from_page_async 递归逻辑

在 `search_all` 方法后，新增 `search_all_from_page_async` 方法：

```rust
/// Recursive async search_all from a page
#[allow(clippy::only_used_in_recursion)]
fn search_all_from_page_async<'a>(
    &'a self,
    page_id: PageId,
    key: &'a Key,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<RowId>>> + Send + 'a>> {
    Box::pin(async move {
        let child_page_ids = {
            let guard = self.async_loader.load_page(page_id).await?;
            let data_guard = guard.page_data();

            if data_guard[0] == LEAF_NODE {
                // Leaf: collect all matching RowIds
                let leaf = LeafNodeRef::new(&data_guard);
                let matches = leaf.find_all_matches(key);
                let mut row_ids = Vec::new();
                for idx in matches {
                    if let Some(rid) = leaf.get_row_id(idx) {
                        row_ids.push(rid);
                    }
                }
                return Ok(row_ids);
            } else {
                // Internal: check if key matches any separator (need to search both subtrees)
                let internal = InternalNodeRef::new(&data_guard);
                let count = internal.key_count();

                let mut child_page_ids = Vec::new();
                for i in 0..count {
                    if let Some(sep_key) = internal.get_key(i) {
                        if sep_key == *key {
                            // Key matches separator: search both left and right subtrees
                            let left_child = if i == 0 {
                                internal.leftmost_child()
                            } else {
                                internal.get_child_page_id(i - 1)
                            };
                            let right_child = internal.get_child_page_id(i);
                            child_page_ids.push(PageId(left_child as u64));
                            child_page_ids.push(PageId(right_child as u64));
                        }
                    }
                }

                // If no separator match, follow normal routing
                if child_page_ids.is_empty() {
                    let child_page_id = internal.find_child_page_id_binary(key);
                    child_page_ids.push(PageId(child_page_id as u64));
                }

                child_page_ids
            }
        }; // guard and data_guard dropped

        // Recursively search all child subtrees
        let mut results = Vec::new();
        for child_page_id in child_page_ids {
            let child_results = self.search_all_from_page_async(child_page_id, key).await?;
            results.extend(child_results);
        }
        Ok(results)
    })
}
```

**位置**：在 `search_all` 方法后（约第 49-100 行）

**关键逻辑**：
- Leaf node：调用 `LeafNodeRef::find_all_matches` 收集所有匹配 row_id
- Internal node：检查 separator 是否匹配 key，匹配时搜索左右子树
- 递归搜索所有子树，合并结果

---

### Step 3: 编译验证

运行：`cargo build`
预期：编译成功，无错误

---

### Step 4: Commit

```bash
git add src/storage/btree/index_manager.rs
git commit -m "feat(M18-T1): add IndexManager::search_all for non-unique indexes"
```

---

## Task 2: IndexScanAllExecutor 实现

**Files:**
- Create: `src/executor/index_scan_all.rs`（新文件）
- Modify: `src/executor/mod.rs:1-10`（新增 pub mod index_scan_all）

---

### Step 1: 创建 IndexScanAllExecutor 文件

创建新文件 `src/executor/index_scan_all.rs`，完整内容：

```rust
//! Index scan executor for non-unique indexes - returns all matching rows

use crate::executor::{ExecResult, Executor};
use crate::profiling::{is_profiling_enabled, record_time};
use crate::storage::page_format::{deserialize_tuple, ColumnType};
use crate::storage::{read_tuple_from_data_page, BufferPool, Result, TableMeta};
use crate::transaction::Snapshot;
use std::sync::Arc;
use std::time::Instant;

pub struct IndexScanAllExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    key: Vec<u8>,
    schema: Vec<ColumnType>,
    snapshot: Option<Snapshot>,
    row_ids: Vec<RowId>,
    current_idx: usize,
    initialized: bool,
}

impl IndexScanAllExecutor {
    pub fn new(
        table_meta: Arc<TableMeta>,
        buffer_pool: Arc<BufferPool>,
        key: Vec<u8>,
        snapshot: Option<Snapshot>,
    ) -> Self {
        let schema: Vec<ColumnType> = table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect();
        Self {
            table_meta,
            buffer_pool,
            key,
            schema,
            snapshot,
            row_ids: Vec::new(),
            current_idx: 0,
            initialized: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for IndexScanAllExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        // Lazy initialization: execute search_all on first call
        if !self.initialized {
            let profiling = is_profiling_enabled();
            if profiling {
                let t0 = Instant::now();
                self.row_ids = self.table_meta.index_manager.search_all(&self.key).await?;
                record_time("index_manager_search_all", t0.elapsed());
            } else {
                self.row_ids = self.table_meta.index_manager.search_all(&self.key).await?;
            }
            self.initialized = true;
        }

        // Iterate through all RowIds, returning visible versions
        while self.current_idx < self.row_ids.len() {
            let row_id = self.row_ids[self.current_idx];
            self.current_idx += 1;

            // MVCC visibility check (reuse IndexScanExecutor logic)
            if let Some(ref snapshot) = self.snapshot {
                let tuple_bytes = self
                    .buffer_pool
                    .find_visible_version(row_id, snapshot)
                    .await?;

                match tuple_bytes {
                    Some(data) => {
                        let values = deserialize_tuple(&data, &self.schema)?;
                        return Ok(Some(ExecResult::Row(values)));
                    }
                    None => {
                        // All versions invisible: skip to next RowId
                        continue;
                    }
                }
            } else {
                // No snapshot: read latest version (backward compat)
                let (_, tuple_bytes) = read_tuple_from_data_page(&self.buffer_pool, row_id).await?;
                let values = deserialize_tuple(&data, &self.schema)?;
                return Ok(Some(ExecResult::Row(values)));
            }
        }

        Ok(None) // All RowIds processed
    }
}
```

**关键逻辑**：
- 惰性初始化：首次调用时执行 search_all
- MVCC 可见性：跳过不可见版本，继续下一个 row_id
- 逐行返回：符合 Executor 接口约定

---

### Step 2: 在 executor/mod.rs 导出新模块

在 `src/executor/mod.rs` 的模块声明区域（约第 1-10 行），新增：

```rust
pub mod index_scan_all;
```

并在 pub use 区域新增：

```rust
pub use index_scan_all::IndexScanAllExecutor;
```

**位置**：在现有 `pub use index_scan::IndexScanExecutor;` 后

---

### Step 3: 修复 RowId 导入缺失

在 `src/executor/index_scan_all.rs` 文件顶部，新增 RowId 导入：

```rust
use crate::storage::page_format::RowId;
```

**位置**：在现有 imports 区域（约第 5-10 行）

---

### Step 4: 编译验证

运行：`cargo build`
预期：编译成功，无错误

---

### Step 5: Commit

```bash
git add src/executor/index_scan_all.rs src/executor/mod.rs
git commit -m "feat(M18-T2): add IndexScanAllExecutor for non-unique indexes"
```

---

## Task 3: executor_test.rs 新增非唯一索引测试

**Files:**
- Modify: `tests/executor_test.rs:100+`（新增 4 个测试）

---

### Step 1: 新增基础功能测试

在 `tests/executor_test.rs` 文件末尾（约第 100 行后），新增测试：

```rust
#[tokio::test]
async fn test_index_scan_all_executor_basic() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("name".to_string(), ColumnType::String)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    // Insert 3 rows with the same key (non-unique index scenario)
    let values = vec![
        vec![Value::String("Alice".to_string())],
        vec![Value::String("Alice".to_string())],
        vec![Value::String("Alice".to_string())],
    ];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
    );
    insert_executor.next().await?;

    // Search for all rows with key "Alice"
    let key = b"Alice".to_vec();
    let mut executor = IndexScanAllExecutor::new(table_meta, buffer_pool, key, None);

    let mut row_count = 0;
    while let Some(result) = executor.next().await? {
        match result {
            ExecResult::Row(values) => {
                assert_eq!(values.len(), 1);
                assert_eq!(values[0], Value::String("Alice".to_string()));
                row_count += 1;
            }
            _ => panic!("Expected ExecResult::Row"),
        }
    }
    assert_eq!(row_count, 3);

    Ok(())
}
```

**验证点**：
- 插入 3 行相同键
- search_all 返回 3 行
- 每行值正确

---

### Step 2: 新增空结果测试

继续新增测试：

```rust
#[tokio::test]
async fn test_index_scan_all_executor_empty() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("name".to_string(), ColumnType::String)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;

    // Search for non-existent key
    let key = b"Bob".to_vec();
    let mut executor = IndexScanAllExecutor::new(table_meta, buffer_pool, key, None);

    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}
```

**验证点**：
- 查询不存在的键
- 立即返回 None

---

### Step 3: 新增单结果测试（回退场景）

继续新增测试：

```rust
#[tokio::test]
async fn test_index_scan_all_executor_single() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("name".to_string(), ColumnType::String)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    // Insert 1 row (unique key scenario, but search_all still works)
    let values = vec![vec![Value::String("Alice".to_string())]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
    );
    insert_executor.next().await?;

    // Search for all rows with key "Alice"
    let key = b"Alice".to_vec();
    let mut executor = IndexScanAllExecutor::new(table_meta, buffer_pool, key, None);

    let mut row_count = 0;
    while let Some(result) = executor.next().await? {
        match result {
            ExecResult::Row(values) => {
                assert_eq!(values.len(), 1);
                assert_eq!(values[0], Value::String("Alice".to_string()));
                row_count += 1;
            }
            _ => panic!("Expected ExecResult::Row"),
        }
    }
    assert_eq!(row_count, 1);

    // Verify no more results
    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}
```

**验证点**：
- search_all 对唯一键也有效（回退场景）
- 返回单行后立即结束

---

### Step 4: 运行测试验证功能

运行：`cargo test test_index_scan_all`
预期：3 个测试全部 PASS

---

### Step 5: Commit

```bash
git add tests/executor_test.rs
git commit -m "test(M18-T3): add IndexScanAllExecutor basic tests"
```

---

## Task 4: SQL层集成（Planner + Pipeline）

**Files:**
- Modify: `src/planner/mod.rs:1-50`（新增 PhysicalPlan::IndexScanAll）
- Modify: `src/pipeline.rs:100+`（新增 IndexScanAllExecutor 创建分支）

---

### Step 1: 在 PhysicalPlan 新增 IndexScanAll 节点

在 `src/planner/mod.rs` 的 PhysicalPlan enum 定义中（约第 1-50 行），新增 variant：

```rust
pub enum PhysicalPlan {
    // ... existing variants ...
    IndexScan {
        table: String,
        key: Vec<u8>,
        snapshot: Option<Snapshot>,
    },
    IndexScanAll {  // 新增
        table: String,
        key: Vec<u8>,
        snapshot: Option<Snapshot>,
    },
    // ... other variants ...
}
```

**位置**：在现有 `IndexScan` variant 后

---

### Step 2: 在 Pipeline 新增 IndexScanAllExecutor 创建分支

在 `src/pipeline.rs` 的 PhysicalPlan match 分支中（约第 100+ 行），新增分支：

```rust
match plan {
    // ... existing branches ...
    PhysicalPlan::IndexScan { table, key, snapshot } => {
        // ... existing IndexScan logic ...
    }
    PhysicalPlan::IndexScanAll { table, key, snapshot } => {  // 新增
        let table_meta = self.table_mgr.get_table(&table).await?;
        Box::pin(async move {
            Ok(IndexScanAllExecutor::new(table_meta, self.buffer_pool.clone(), key, snapshot))
        })
    }
    // ... other branches ...
}
```

**位置**：在现有 `PhysicalPlan::IndexScan` 分支后

---

### Step 3: 编译验证

运行：`cargo build`
预期：编译成功，无错误

---

### Step 4: 运行完整测试套件

运行：`cargo test`
预期：所有测试 PASS（包括新增的 IndexScanAllExecutor 测试）

---

### Step 5: Commit

```bash
git add src/planner/mod.rs src/pipeline.rs
git commit -m "feat(M18-T4): integrate IndexScanAllExecutor into SQL layer"
```

---

## Final Verification

### Step 1: 运行完整测试套件

运行：`cargo test`
预期：所有测试 PASS，新增 3 个 IndexScanAllExecutor 测试

---

### Step 2: 运行 Clippy 检查

运行：`cargo clippy`
预期：无 warnings（Phase1 已清理）

---

### Step 3: 更新 tasks.md

更新 `.claude/docs/tasks.md`，标记 Phase2 任务完成：

```markdown
### Phase2: Executor层非唯一索引测试覆盖 ✅

- [x] T1: 新增 IndexManager::search_all 方法
- [x] T2: 实现 IndexScanAllExecutor::execute
- [x] T3: executor_test.rs 新增非唯一索引测试
- [x] T4: SQL层集成验证
```

---

### Step 4: Commit tasks.md

```bash
git add .claude/docs/tasks.md
git commit -m "docs(M18): mark Phase2 tasks complete"
```

---

## Self-Review Check

**✅ Spec coverage**: 检查设计文档所有需求，均有对应任务覆盖
- IndexManager::search_all → Task 1
- IndexScanAllExecutor → Task 2
- executor_test.rs → Task 3
- SQL层集成 → Task 4

**✅ Placeholder scan**: 无 TBD/TODO/不完整步骤，所有代码完整

**✅ Type consistency**: 类型签名一致
- RowId import 在 Task 2 已添加
- Key 类型使用 Vec<u8>，与现有 IndexScanExecutor 一致
- Snapshot 类型使用 Option<Snapshot>，与现有一致

---

## Execution Handoff

计划已保存到 `.claude/docs/superpowers/plans/2026-05-23-index-scan-all-executor.md`。

**两种执行选项**：

**1. Subagent-Driven（推荐）** - 每个任务分派全新 subagent，任务间审核，快速迭代

**2. Inline Execution** - 在当前会话使用 executing-plans 执行，批量执行带 checkpoint 审核

**你选择哪种方式？**