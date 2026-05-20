# M5: 异步执行引擎设计规范

> 创建日期：2026-05-20
> 状态：设计完成，待用户 review

## 一、目标与范围

### 目标

实现 PhysicalPlan 的异步执行引擎，验证从 SQL 解析 → 物理计划 → 执行的完整流程。

### 范围界定

**M5 包含**：
- ExecResult 执行结果类型
- Executor trait 定义
- 5 种 Executor 实现（对应 5 种 PhysicalPlan 节点）
- 异步迭代器 `next() -> Result<Option<ExecResult>>`
- 单元测试 + 集成测试

**M5 不包含（推迟到 M6）**：
- 完整 Row 数据返回（仅返回 RowId）
- Scan 全表扫描（需要数据存储层）
- 事务整合（MVCC 可见性判断）
- 多表支持（M5 单表验证）
- 复杂查询节点（Filter/Join 等）

### 设计决策

| 决策 | 选项 | 理由 |
|------|------|------|
| 数据存储层 | 仅索引层执行 | M5 核心目标是执行引擎，数据层 M6 补 |
| 执行器接口 | trait + 多 Executor | 职责独立，便于扩展，符合 Rust trait 抽象 |
| 执行结果类型 | 统一 ExecResult enum | 一个类型覆盖所有操作，NotImplemented 标记未完成 |
| 事务整合 | M5 不整合 | MVCC 复杂度高，M6 网络层整合 |
| IndexManager 注入 | 构造时传入 Arc | 直接简单，测试方便 |
| 测试策略 | 单元 + 集成 | 验证每种 Executor + 完整流程 |

---

## 二、核心数据结构

### ExecResult（执行结果类型）

```rust
pub enum ExecResult {
    /// 查询返回 RowId（IndexScan）
    RowId(RowId),
    /// 写操作返回影响计数（Insert/Update/Delete）
    AffectedRows(u64),
    /// Scan 暂不实现（M6 补数据层）
    NotImplemented,
}
```

**设计说明**：
- 统一返回类型，适配不同 PhysicalPlan 节点
- RowId 用于 IndexScan（后续 M6 可扩展为完整 Row）
- AffectedRows 符合 SQL 语义（INSERT 返回插入行数）
- NotImplemented 明确标记未完成功能

### Executor trait

```rust
pub trait Executor {
    /// 执行一次迭代，返回结果
    /// None 表示迭代结束（无更多结果）
    async fn next(&mut self) -> Result<Option<ExecResult>>;
}
```

**设计说明**：
- 异步迭代器接口，支持流式返回
- `Option<ExecResult>` 区分"有结果"和"迭代结束"
- 使用 `Result` 包装错误（StorageError）

---

## 三、Executor 实现结构

### 文件结构

```
src/executor/
├── mod.rs           # 模块导出（更新）
├── value.rs         # Value（现有，M4）
├── plan.rs          # PhysicalPlan（现有，M4）
├── result.rs        # ExecResult（新增）
├── executor_trait.rs # Executor trait（新增）
├── index_scan.rs    # IndexScanExecutor（新增）
├── insert.rs        # InsertExecutor（新增）
├── update.rs        # UpdateExecutor（新增）
├── delete.rs        # DeleteExecutor（新增）
└── scan.rs          # ScanExecutor（新增）
```

### Executor 实现对照表

| Executor | 持有数据 | 执行逻辑 | 返回类型 | 复杂度 |
|----------|---------|---------|---------|--------|
| **IndexScanExecutor** | `Arc<IndexManager>, Key, columns` | `search(key)` | `RowId(RowId)` 或 `None` | 低 |
| **InsertExecutor** | `Arc<IndexManager>, Vec<Value>` | `insert(key, row_id)` | `AffectedRows(count)` | 中 |
| **UpdateExecutor** | `Arc<IndexManager>, Key, new_value` | `update(key, new_row_id)` | `AffectedRows(1)` | 低 |
| **DeleteExecutor** | `Arc<IndexManager>, Key` | `delete(key)` | `AffectedRows(1)` | 低 |
| **ScanExecutor** | 无 | 直接返回 `NotImplemented` | `NotImplemented` | 极低 |

---

## 四、详细实现设计

### 4.1 IndexScanExecutor

**结构定义**：
```rust
pub struct IndexScanExecutor {
    index_manager: Arc<IndexManager>,
    key: Key,
    columns: Vec<String>,
    executed: bool, // 标记是否已执行
}
```

**执行流程**：
1. 构造时传入 IndexManager、Key、columns
2. 第一次 `next()` 调用 `index_manager.search(&key)`
3. 找到 → 返回 `Some(ExecResult::RowId(row_id))`，标记 `executed = true`
4. 未找到 → 返回 `None`
5. 后续 `next()` 直接返回 `None`（单次查询结束）

**状态管理**：
- `executed: bool` 标记执行状态
- 单次查询，非迭代式

### 4.2 InsertExecutor

**结构定义**：
```rust
pub struct InsertExecutor {
    index_manager: Arc<IndexManager>,
    values: Vec<Vec<Value>>, // 批量插入
    row_id_generator: u64,   // RowId 生成器（测试占位）
    executed: bool,
}
```

**执行流程**：
1. 构造时传入 IndexManager 和批量 values
2. 第一次 `next()` 遍历所有 values
3. 每个插入：
   - 生成 RowId（测试占位值：`RowId::new(0, slot_id)`）
   - 调用 `index_manager.insert(key, row_id)`
4. 返回 `Some(ExecResult::AffectedRows(count))`
5. 后续 `next()` 直接返回 `None`

**RowId 生成策略**：
- M5 使用测试占位值（`page_id = 0`, `slot_id` 递增）
- M6 数据存储层实现真实 RowId 分配

### 4.3 UpdateExecutor

**结构定义**：
```rust
pub struct UpdateExecutor {
    index_manager: Arc<IndexManager>,
    key: Key,
    new_value: Value,
    executed: bool,
}
```

**执行流程**：
1. 构造时传入 IndexManager、Key、new_value
2. 第一次 `next()`：
   - 生成新 RowId（测试占位）
   - 调用 `index_manager.update(&key, new_row_id)`
3. 返回 `Some(ExecResult::AffectedRows(1))`
4. 后续 `next()` 直接返回 `None`

**注意**：
- M5 仅更新索引层 RowId，不更新实际数据
- M6 数据层实现时需要更新数据页内容

### 4.4 DeleteExecutor

**结构定义**：
```rust
pub struct DeleteExecutor {
    index_manager: Arc<IndexManager>,
    key: Key,
    executed: bool,
}
```

**执行流程**：
1. 构造时传入 IndexManager、Key
2. 第一次 `next()` 调用 `index_manager.delete(&key)`
3. 返回 `Some(ExecResult::AffectedRows(1))`
4. 后续 `next()` 直接返回 `None`

### 4.5 ScanExecutor

**结构定义**：
```rust
pub struct ScanExecutor {
    // 无数据（暂不实现）
}
```

**执行流程**：
1. 构造时无数据
2. `next()` 直接返回 `Some(ExecResult::NotImplemented)`
3. 后续 `next()` 返回 `None`

**设计说明**：
- Scan 需要数据存储层遍历所有数据页
- M5 标记 NotImplemented，不阻塞其他 Executor 测试

---

## 五、测试策略

### 5.1 单元测试

**文件**：`tests/executor_test.rs`

| Executor | 测试数量 | 测试内容 |
|----------|---------|---------|
| IndexScanExecutor | 3 | 插入后查找、未找到返回 None、第二次 next 返回 None |
| InsertExecutor | 3 | 单行插入、批量插入、第二次 next 返回 None |
| UpdateExecutor | 2 | 更新成功、第二次 next 返回 None |
| DeleteExecutor | 2 | 删除成功、第二次 next 返回 None |
| ScanExecutor | 1 | 返回 NotImplemented |

### 5.2 集成测试

**文件**：`tests/plan_exec_test.rs`

| 测试 | 内容 |
|------|------|
| insert_then_index_scan | Insert → IndexScan 能找到插入的 RowId |
| update_then_index_scan | Update 后 IndexScan 返回新 RowId |
| delete_then_index_scan | Delete 后 IndexScan 返回 None |
| plan_to_executor | PhysicalPlan 转换为 Executor 并执行 |

---

## 六、错误处理

### 错误类型

沿用 M1 定义的 `StorageError`：

```rust
// executor 操作失败返回 StorageError
pub type Result<T> = std::result::Result<T, StorageError>;
```

### 错误场景

| Executor | 错误场景 | 错误类型 |
|----------|---------|---------|
| IndexScanExecutor | IndexManager.search 失败 | StorageError |
| InsertExecutor | IndexManager.insert 失败 | StorageError |
| UpdateExecutor | key 不存在 | StorageError（IndexManager 返回） |
| DeleteExecutor | key 不存在 | StorageError（IndexManager 返回） |

---

## 七、依赖关系

### 内部依赖

| 组件 | 来源 | 用途 |
|------|------|------|
| IndexManager | M2 | 索引操作异步 API |
| Key | M2 | 索引键（32 bytes） |
| RowId | M2 | 行定位符 |
| Value | M4 | SQL 值类型 |
| PhysicalPlan | M4 | 物理计划节点 |
| StorageError | M1 | 错误类型 |

### 外部依赖

| 依赖 | 版本 | 用途 |
|------|------|------|
| tokio | 1.x | 异步运行时 |
| anyhow | 1.x | 错误处理（可选） |

---

## 八、实现顺序

### Phase 1：基础结构

1. 实现 `result.rs`：ExecResult enum
2. 实现 `executor_trait.rs`：Executor trait
3. 更新 `mod.rs`：导出新模块

### Phase 2：Executor 实现

4. 实现 `scan.rs`：ScanExecutor（最简单，先验证接口）
5. 实现 `index_scan.rs`：IndexScanExecutor
6. 实现 `insert.rs`：InsertExecutor
7. 实现 `update.rs`：UpdateExecutor
8. 实现 `delete.rs`：DeleteExecutor

### Phase 3：测试

9. 单元测试：`tests/executor_test.rs`
10. 集成测试：`tests/plan_exec_test.rs`
11. 运行测试验证

---

## 九、后续演进（M6）

| M5 限制 | M6 改进 |
|---------|---------|
| 返回 RowId | 返回完整 Row（Vec<Value>） |
| Scan NotImplemented | 实现 Scan（遍历数据页） |
| 无事务整合 | Executor 持有 Transaction |
| 测试 RowId 占位 | 真实数据存储层 RowId |
| 单表索引操作 | 多表支持 |

---

## 十、风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| Executor trait 设计不合理 | 重构成本 | 参考 M0-M4 trait 设计模式 |
| 测试覆盖不足 | 隐藏 bug | 每种 Executor 至少 2 测试 |
| RowId 占位逻辑复杂 | M6 迁移困难 | 保持简单，M6 重构 |
| 异步状态管理错误 | 迭代器 bug | executed 标记明确状态 |

---

## 附录：代码示例

### IndexScanExecutor 实现示例

```rust
use crate::executor::{ExecResult, Executor, Result};
use crate::storage::{page_format::RowId, btree::IndexManager};
use std::sync::Arc;

pub struct IndexScanExecutor {
    index_manager: Arc<IndexManager>,
    key: crate::storage::page_format::Key,
    columns: Vec<String>,
    executed: bool,
}

impl IndexScanExecutor {
    pub fn new(
        index_manager: Arc<IndexManager>,
        key: crate::storage::page_format::Key,
        columns: Vec<String>,
    ) -> Self {
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

---

> 设计完成，待用户 review 后进入 Phase 2（PLAN）