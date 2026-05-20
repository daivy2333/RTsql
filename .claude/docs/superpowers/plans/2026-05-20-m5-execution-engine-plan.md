# M5: 异步执行引擎实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 PhysicalPlan 的异步执行引擎，包含 ExecResult、Executor trait 和 5 种 Executor 实现

**Architecture:** 采用 trait + 多 Executor 设计，每种 PhysicalPlan 节点对应独立 Executor 结构。M5 仅实现索引层执行，返回 RowId，数据存储层推迟到 M6。

**Tech Stack:** Rust + Tokio (async runtime) + 现有 storage/executor 模块

---

## 文件结构

### 新增文件

| 文件 | 职责 |
|------|------|
| `src/executor/result.rs` | ExecResult enum（执行结果类型） |
| `src/executor/executor_trait.rs` | Executor trait（异步迭代器接口） |
| `src/executor/scan.rs` | ScanExecutor（返回 NotImplemented） |
| `src/executor/index_scan.rs` | IndexScanExecutor（主键索引查找） |
| `src/executor/insert.rs` | InsertExecutor（批量插入） |
| `src/executor/update.rs` | UpdateExecutor（单行更新） |
| `src/executor/delete.rs` | DeleteExecutor（单行删除） |
| `tests/executor_test.rs` | 单元测试（每种 Executor） |
| `tests/plan_exec_test.rs` | 集成测试（完整流程） |

### 修改文件

| 文件 | 改动 |
|------|------|
| `src/executor/mod.rs` | 导出新模块和类型 |
| `src/lib.rs` | 导出 executor 公共接口 |

---

## Phase 1: 基础结构

### Task 1: ExecResult enum

**Files:**
- Create: `src/executor/result.rs`
- Modify: `src/executor/mod.rs`

- [ ] **Step 1: 写 ExecResult 类型**

```rust
//! Execution result types

use crate::storage::page_format::RowId;

/// 执行结果类型
#[derive(Debug, Clone, PartialEq)]
pub enum ExecResult {
    /// 查询返回 RowId（IndexScan）
    RowId(RowId),
    /// 写操作返回影响计数（Insert/Update/Delete）
    AffectedRows(u64),
    /// Scan 暂不实现（M6 补数据层）
    NotImplemented,
}
```

- [ ] **Step 2: 更新 mod.rs 导出**

```rust
//! Execution engine - Physical plan execution, async iterator
//!
//! M5: Implement async fn next() -> Result<Option<ExecResult>>

mod plan;
mod value;
mod result;

pub use plan::{DeleteNode, IndexScanNode, InsertNode, PhysicalPlan, ScanNode, UpdateNode};
pub use value::Value;
pub use result::ExecResult;
```

- [ ] **Step 3: 验证编译通过**

Run: `cargo build`
Expected: 编译成功，无错误

- [ ] **Step 4: Commit**

```bash
git add src/executor/result.rs src/executor/mod.rs
git commit -m "feat(m5): add ExecResult enum for execution results"
```

---

### Task 2: Executor trait

**Files:**
- Create: `src/executor/executor_trait.rs`
- Modify: `src/executor/mod.rs`

- [ ] **Step 1: 写 Executor trait**

```rust
//! Executor trait - async iterator interface

use crate::executor::ExecResult;
use crate::storage::Result;

/// Executor trait - 异步迭代器接口
pub trait Executor {
    /// 执行一次迭代，返回结果
    /// None 表示迭代结束（无更多结果）
    async fn next(&mut self) -> Result<Option<ExecResult>>;
}
```

- [ ] **Step 2: 更新 mod.rs 导出**

```rust
//! Execution engine - Physical plan execution, async iterator
//!
//! M5: Implement async fn next() -> Result<Option<ExecResult>>

mod plan;
mod value;
mod result;
mod executor_trait;

pub use plan::{DeleteNode, IndexScanNode, InsertNode, PhysicalPlan, ScanNode, UpdateNode};
pub use value::Value;
pub use result::ExecResult;
pub use executor_trait::Executor;
```

- [ ] **Step 3: 验证编译通过**

Run: `cargo build`
Expected: 编译成功，无错误

- [ ] **Step 4: Commit**

```bash
git add src/executor/executor_trait.rs src/executor/mod.rs
git commit -m "feat(m5): add Executor trait with async next() interface"
```

---

## Phase 2: Executor 实现

### Task 3: ScanExecutor（最简单，验证接口）

**Files:**
- Create: `src/executor/scan.rs`
- Create: `tests/executor_test.rs`（单元测试文件）
- Modify: `src/executor/mod.rs`

- [ ] **Step 1: 写 ScanExecutor 测试**

```rust
//! Executor unit tests

use crate::executor::{ExecResult, Executor, ScanExecutor};
use crate::storage::Result;

#[tokio::test]
async fn test_scan_executor_returns_not_implemented() -> Result<()> {
    let mut executor = ScanExecutor::new();

    // 第一次 next 返回 NotImplemented
    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::NotImplemented));

    // 第二次 next 返回 None（迭代结束）
    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test test_scan_executor_returns_not_implemented`
Expected: FAIL - ScanExecutor 未定义

- [ ] **Step 3: 实现 ScanExecutor**

```rust
//! Scan executor - full table scan (M5: NotImplemented)

use crate::executor::{ExecResult, Executor};
use crate::storage::Result;

/// ScanExecutor - 全表扫描执行器
/// M5: 暂不实现，返回 NotImplemented
pub struct ScanExecutor {
    executed: bool,
}

impl ScanExecutor {
    pub fn new() -> Self {
        Self { executed: false }
    }
}

impl Default for ScanExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor for ScanExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;
        Ok(Some(ExecResult::NotImplemented))
    }
}
```

- [ ] **Step 4: 更新 mod.rs 导出**

在 `src/executor/mod.rs` 添加：

```rust
mod scan;
pub use scan::ScanExecutor;
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test test_scan_executor_returns_not_implemented`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/executor/scan.rs src/executor/mod.rs tests/executor_test.rs
git commit -m "feat(m5): implement ScanExecutor (returns NotImplemented)"
```

---

### Task 4: IndexScanExecutor

**Files:**
- Create: `src/executor/index_scan.rs`
- Modify: `src/executor/mod.rs`
- Modify: `tests/executor_test.rs`

- [ ] **Step 1: 写 IndexScanExecutor 测试**

在 `tests/executor_test.rs` 添加：

```rust
use crate::executor::{ExecResult, Executor, IndexScanExecutor};
use crate::storage::{btree::IndexManager, BufferPool, page_format::{Key, RowId}};
use std::sync::Arc;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_index_scan_executor_found() {
    // 创建临时文件和 IndexManager
    let temp_file = NamedTempFile::new().unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, temp_file.path()).unwrap());
    let index_manager = Arc::new(IndexManager::new(buffer_pool).unwrap());

    // 先插入一条数据
    let key = Key::new(&1i64.to_be_bytes());
    let row_id = RowId::new(0, 1);
    index_manager.insert(&key, row_id).await.unwrap();

    // 创建 IndexScanExecutor
    let mut executor = IndexScanExecutor::new(index_manager, key, vec!["id".to_string()]);

    // 第一次 next 返回 RowId
    let result = executor.next().await.unwrap();
    assert_eq!(result, Some(ExecResult::RowId(row_id)));

    // 第二次 next 返回 None（迭代结束）
    let result = executor.next().await.unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn test_index_scan_executor_not_found() {
    let temp_file = NamedTempFile::new().unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, temp_file.path()).unwrap());
    let index_manager = Arc::new(IndexManager::new(buffer_pool).unwrap());

    // 不插入任何数据，直接查找
    let key = Key::new(&1i64.to_be_bytes());
    let mut executor = IndexScanExecutor::new(index_manager, key, vec!["id".to_string()]);

    // next 返回 None（未找到）
    let result = executor.next().await.unwrap();
    assert_eq!(result, None);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test test_index_scan`
Expected: FAIL - IndexScanExecutor 未定义

- [ ] **Step 3: 实现 IndexScanExecutor**

```rust
//! Index scan executor - primary key lookup

use crate::executor::{ExecResult, Executor};
use crate::storage::{btree::IndexManager, page_format::Key, Result};
use std::sync::Arc;

/// IndexScanExecutor - 主键索引扫描执行器
pub struct IndexScanExecutor {
    index_manager: Arc<IndexManager>,
    key: Key,
    columns: Vec<String>,
    executed: bool,
}

impl IndexScanExecutor {
    pub fn new(index_manager: Arc<IndexManager>, key: Key, columns: Vec<String>) -> Self {
        Self {
            index_manager,
            key,
            columns,
            executed: false,
        }
    }
}

impl Executor for IndexScanExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        let row_id = self.index_manager.search(&self.key).await?;

        match row_id {
            Some(id) => Ok(Some(ExecResult::RowId(id))),
            None => Ok(None),
        }
    }
}
```

- [ ] **Step 4: 更新 mod.rs 导出**

在 `src/executor/mod.rs` 添加：

```rust
mod index_scan;
pub use index_scan::IndexScanExecutor;
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test test_index_scan`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/executor/index_scan.rs src/executor/mod.rs tests/executor_test.rs
git commit -m "feat(m5): implement IndexScanExecutor with async search"
```

---

### Task 5: InsertExecutor

**Files:**
- Create: `src/executor/insert.rs`
- Modify: `src/executor/mod.rs`
- Modify: `tests/executor_test.rs`

- [ ] **Step 1: 写 InsertExecutor 测试**

在 `tests/executor_test.rs` 添加：

```rust
use crate::executor::{ExecResult, Executor, InsertExecutor, Value};
use crate::storage::{btree::IndexManager, BufferPool, page_format::Key};
use std::sync::Arc;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_insert_executor_single_row() {
    let temp_file = NamedTempFile::new().unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, temp_file.path()).unwrap());
    let index_manager = Arc::new(IndexManager::new(buffer_pool).unwrap());

    // 单行插入
    let values = vec![vec![Value::Int(1)]];
    let columns = vec!["id".to_string()];
    let mut executor = InsertExecutor::new(index_manager, columns, values);

    // 第一次 next 返回 AffectedRows(1)
    let result = executor.next().await.unwrap();
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    // 第二次 next 返回 None
    let result = executor.next().await.unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn test_insert_executor_batch() {
    let temp_file = NamedTempFile::new().unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, temp_file.path()).unwrap());
    let index_manager = Arc::new(IndexManager::new(buffer_pool).unwrap());

    // 批量插入 3 行
    let values = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
    ];
    let columns = vec!["id".to_string()];
    let mut executor = InsertExecutor::new(index_manager, columns, values);

    // 第一次 next 返回 AffectedRows(3)
    let result = executor.next().await.unwrap();
    assert_eq!(result, Some(ExecResult::AffectedRows(3)));

    // 第二次 next 返回 None
    let result = executor.next().await.unwrap();
    assert_eq!(result, None);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test test_insert_executor`
Expected: FAIL - InsertExecutor 未定义

- [ ] **Step 3: 实现 InsertExecutor**

```rust
//! Insert executor - batch insertion

use crate::executor::{ExecResult, Executor, Value};
use crate::storage::{btree::IndexManager, page_format::RowId, Result};
use std::sync::Arc;

/// InsertExecutor - 批量插入执行器
pub struct InsertExecutor {
    index_manager: Arc<IndexManager>,
    columns: Vec<String>,
    values: Vec<Vec<Value>>,
    executed: bool,
}

impl InsertExecutor {
    pub fn new(index_manager: Arc<IndexManager>, columns: Vec<String>, values: Vec<Vec<Value>>) -> Self {
        Self {
            index_manager,
            columns,
            values,
            executed: false,
        }
    }
}

impl Executor for InsertExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        let mut count = 0u64;
        for (slot_id, row_values) in self.values.iter().enumerate() {
            // 取第一列作为 key（假设主键在第一列）
            if let Some(first_value) = row_values.first() {
                if let Some(key) = first_value.to_key() {
                    // M5: 使用测试占位 RowId（page_id=0, slot_id 递增）
                    let row_id = RowId::new(0, slot_id as u16);
                    self.index_manager.insert(&key, row_id).await?;
                    count += 1;
                }
            }
        }

        Ok(Some(ExecResult::AffectedRows(count)))
    }
}
```

- [ ] **Step 4: 更新 mod.rs 导出**

在 `src/executor/mod.rs` 添加：

```rust
mod insert;
pub use insert::InsertExecutor;
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test test_insert_executor`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/executor/insert.rs src/executor/mod.rs tests/executor_test.rs
git commit -m "feat(m5): implement InsertExecutor with batch insertion"
```

---

### Task 6: UpdateExecutor

**Files:**
- Create: `src/executor/update.rs`
- Modify: `src/executor/mod.rs`
- Modify: `tests/executor_test.rs`

- [ ] **Step 1: 写 UpdateExecutor 测试**

在 `tests/executor_test.rs` 添加：

```rust
use crate::executor::{ExecResult, Executor, UpdateExecutor, Value};
use crate::storage::{btree::IndexManager, BufferPool, page_format::{Key, RowId}};
use std::sync::Arc;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_update_executor() {
    let temp_file = NamedTempFile::new().unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, temp_file.path()).unwrap());
    let index_manager = Arc::new(IndexManager::new(buffer_pool).unwrap());

    // 先插入一条数据
    let key = Key::new(&1i64.to_be_bytes());
    let row_id = RowId::new(0, 1);
    index_manager.insert(&key, row_id).await.unwrap();

    // 创建 UpdateExecutor
    let new_value = Value::Int(100);
    let mut executor = UpdateExecutor::new(index_manager, key.clone(), "id".to_string(), new_value);

    // 第一次 next 返回 AffectedRows(1)
    let result = executor.next().await.unwrap();
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    // 第二次 next 返回 None
    let result = executor.next().await.unwrap();
    assert_eq!(result, None);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test test_update_executor`
Expected: FAIL - UpdateExecutor 未定义

- [ ] **Step 3: 实现 UpdateExecutor**

```rust
//! Update executor - single row update

use crate::executor::{ExecResult, Executor, Value};
use crate::storage::{btree::IndexManager, page_format::{Key, RowId}, Result};
use std::sync::Arc;

/// UpdateExecutor - 单行更新执行器
pub struct UpdateExecutor {
    index_manager: Arc<IndexManager>,
    key: Key,
    column: String,
    new_value: Value,
    executed: bool,
}

impl UpdateExecutor {
    pub fn new(index_manager: Arc<IndexManager>, key: Key, column: String, new_value: Value) -> Self {
        Self {
            index_manager,
            key,
            column,
            new_value,
            executed: false,
        }
    }
}

impl Executor for UpdateExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        // M5: 仅更新索引层 RowId（使用新占位值）
        // M6: 数据层实现时需要更新实际数据页
        let new_row_id = RowId::new(0, 999);
        self.index_manager.update(&self.key, new_row_id).await?;

        Ok(Some(ExecResult::AffectedRows(1)))
    }
}
```

- [ ] **Step 4: 更新 mod.rs 导出**

在 `src/executor/mod.rs` 添加：

```rust
mod update;
pub use update::UpdateExecutor;
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test test_update_executor`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/executor/update.rs src/executor/mod.rs tests/executor_test.rs
git commit -m "feat(m5): implement UpdateExecutor (updates RowId in index)"
```

---

### Task 7: DeleteExecutor

**Files:**
- Create: `src/executor/delete.rs`
- Modify: `src/executor/mod.rs`
- Modify: `tests/executor_test.rs`

- [ ] **Step 1: 写 DeleteExecutor 测试**

在 `tests/executor_test.rs` 添加：

```rust
use crate::executor::{ExecResult, Executor, DeleteExecutor};
use crate::storage::{btree::IndexManager, BufferPool, page_format::{Key, RowId}};
use std::sync::Arc;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_delete_executor() {
    let temp_file = NamedTempFile::new().unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, temp_file.path()).unwrap());
    let index_manager = Arc::new(IndexManager::new(buffer_pool).unwrap());

    // 先插入一条数据
    let key = Key::new(&1i64.to_be_bytes());
    let row_id = RowId::new(0, 1);
    index_manager.insert(&key, row_id).await.unwrap();

    // 创建 DeleteExecutor
    let mut executor = DeleteExecutor::new(index_manager, key.clone());

    // 第一次 next 返回 AffectedRows(1)
    let result = executor.next().await.unwrap();
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    // 第二次 next 返回 None
    let result = executor.next().await.unwrap();
    assert_eq!(result, None);

    // 验证已删除（再查找返回 None）
    let found = index_manager.search(&key).await.unwrap();
    assert_eq!(found, None);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test test_delete_executor`
Expected: FAIL - DeleteExecutor 未定义

- [ ] **Step 3: 实现 DeleteExecutor**

```rust
//! Delete executor - single row deletion

use crate::executor::{ExecResult, Executor};
use crate::storage::{btree::IndexManager, page_format::Key, Result};
use std::sync::Arc;

/// DeleteExecutor - 单行删除执行器
pub struct DeleteExecutor {
    index_manager: Arc<IndexManager>,
    key: Key,
    executed: bool,
}

impl DeleteExecutor {
    pub fn new(index_manager: Arc<IndexManager>, key: Key) -> Self {
        Self {
            index_manager,
            key,
            executed: false,
        }
    }
}

impl Executor for DeleteExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        self.index_manager.delete(&self.key).await?;

        Ok(Some(ExecResult::AffectedRows(1)))
    }
}
```

- [ ] **Step 4: 更新 mod.rs 导出**

在 `src/executor/mod.rs` 添加：

```rust
mod delete;
pub use delete::DeleteExecutor;
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test test_delete_executor`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/executor/delete.rs src/executor/mod.rs tests/executor_test.rs
git commit -m "feat(m5): implement DeleteExecutor"
```

---

## Phase 3: 集成测试

### Task 8: 集成测试（完整执行流程）

**Files:**
- Create: `tests/plan_exec_test.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: 更新 lib.rs 导出**

```rust
pub mod executor;
pub mod parser;
pub mod storage;
pub mod transaction;
```

- [ ] **Step 2: 写集成测试**

```rust
//! Integration tests: PhysicalPlan → Executor → Result

use rtsql::{
    executor::{ExecResult, Executor, IndexScanExecutor, InsertExecutor, Value},
    storage::{btree::IndexManager, BufferPool, page_format::Key},
};
use std::sync::Arc;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_insert_then_index_scan() {
    let temp_file = NamedTempFile::new().unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, temp_file.path()).unwrap());
    let index_manager = Arc::new(IndexManager::new(buffer_pool).unwrap());

    // Insert
    let values = vec![vec![Value::Int(42)]];
    let mut insert_executor = InsertExecutor::new(
        index_manager.clone(),
        vec!["id".to_string()],
        values,
    );
    let result = insert_executor.next().await.unwrap();
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    // IndexScan
    let key = Key::new(&42i64.to_be_bytes());
    let mut scan_executor = IndexScanExecutor::new(
        index_manager,
        key,
        vec!["id".to_string()],
    );
    let result = scan_executor.next().await.unwrap();
    // 应返回 RowId（M5 占位值：RowId::new(0, 0)）
    assert!(matches!(result, Some(ExecResult::RowId(_))));
}

#[tokio::test]
async fn test_full_flow_insert_find_delete() {
    let temp_file = NamedTempFile::new().unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, temp_file.path()).unwrap());
    let index_manager = Arc::new(IndexManager::new(buffer_pool).unwrap());

    // 1. Insert
    let mut insert_executor = InsertExecutor::new(
        index_manager.clone(),
        vec!["id".to_string()],
        vec![vec![Value::Int(100)]],
    );
    insert_executor.next().await.unwrap();

    // 2. IndexScan - 确认存在
    let key = Key::new(&100i64.to_be_bytes());
    let mut scan_executor = IndexScanExecutor::new(
        index_manager.clone(),
        key.clone(),
        vec!["id".to_string()],
    );
    let result = scan_executor.next().await.unwrap();
    assert!(matches!(result, Some(ExecResult::RowId(_))));

    // 3. Delete
    use rtsql::executor::DeleteExecutor;
    let mut delete_executor = DeleteExecutor::new(index_manager.clone(), key.clone());
    delete_executor.next().await.unwrap();

    // 4. IndexScan - 确认已删除
    let mut scan_executor = IndexScanExecutor::new(
        index_manager,
        key,
        vec!["id".to_string()],
    );
    let result = scan_executor.next().await.unwrap();
    assert_eq!(result, None);
}
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test test_insert_then_index_scan`
Expected: FAIL - lib.rs 导出问题或模块路径

- [ ] **Step 4: 修复 lib.rs（如需要）**

检查 `src/lib.rs` 是否已导出 `executor` 模块，如未导出则添加。

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test test_insert_then_index_scan test_full_flow`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add tests/plan_exec_test.rs src/lib.rs
git commit -m "test(m5): add integration tests for full execution flow"
```

---

### Task 9: 最终验证

**Files:**
- 无新增/修改

- [ ] **Step 1: 运行全部测试**

Run: `cargo test`
Expected: 所有测试通过（executor_test + plan_exec_test）

- [ ] **Step 2: 运行 clippy**

Run: `cargo clippy`
Expected: 无 Critical warnings（允许 minor warnings）

- [ ] **Step 3: 运行 fmt**

Run: `cargo fmt --check`
Expected: 格式正确或执行 `cargo fmt`

- [ ] **Step 4: 最终 Commit（如有 fmt 修改）**

```bash
cargo fmt
git add .
git commit -m "style(m5): apply cargo fmt formatting"
```

---

## Self-Review 检查清单

### Spec Coverage

| Spec 章节 | 对应 Task |
|-----------|----------|
| ExecResult enum | Task 1 |
| Executor trait | Task 2 |
| ScanExecutor | Task 3 |
| IndexScanExecutor | Task 4 |
| InsertExecutor | Task 5 |
| UpdateExecutor | Task 6 |
| DeleteExecutor | Task 7 |
| 单元测试 | Task 3-7（嵌入式） |
| 集成测试 | Task 8 |

### Placeholder Scan

- ✅ 无 TBD/TODO
- ✅ 无 "Add appropriate error handling"
- ✅ 无 "Similar to Task N"
- ✅ 所有代码步骤包含完整代码

### Type Consistency

- ✅ ExecResult::RowId(RowId) - RowId 来自 storage::page_format
- ✅ Executor::next() 返回 Result<Option<ExecResult>> - Result 来自 storage
- ✅ IndexScanExecutor::new 参数：Arc<IndexManager>, Key, Vec<String>
- ✅ InsertExecutor::new 参数：Arc<IndexManager>, Vec<String>, Vec<Vec<Value>>
- ✅ Value::to_key() 返回 Option<Key> - Key 来自 storage::page_format

---

> Plan complete. Ready for execution.