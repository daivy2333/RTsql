# M18 优化项目与技术债清理实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 分4个 phases 处理所有记录的优化点和技术债，实现代码质量清理 + 性能提升

**Architecture:**
- Phase1: 架构warnings清理（务实策略，平衡成本）
- Phase2: Executor层非唯一索引测试覆盖（新增 IndexScanAllExecutor）
- Phase3: WAL集成 + Group Commit（INSERT 性能优化）
- Phase4: B-Tree Merge（删除后页合并）

**Tech Stack:** Rust, Tokio async, Clippy, criterion.rs benchmark

---

## File Structure

### Phase1: 架构Warnings清理

**创建文件**：
- `src/executor/join_config.rs` - JoinConfig struct（解决 too_many_arguments）

**修改文件**：
- `src/executor/mod.rs` - 添加 ExecutorFuture type alias + 导出 JoinConfig
- `src/executor/join.rs` - 使用 JoinConfig 重构 Executor::new
- `src/executor/anti_join.rs` - 使用 JoinConfig 重构 Executor::new
- `src/executor/semi_join.rs` - 使用 JoinConfig 重构 Executor::new
- `src/executor/pipeline.rs` - 使用 ExecutorFuture type alias
- `src/storage/btree/mod.rs` - 评估 module_inception 是否重命名
- `src/storage/buffer_pool.rs` - #[allow] await_holding_lock + 注释说明
- `src/executor/aggregate.rs` - #[allow] dead_code output_columns + 注释说明
- `src/executor/delete.rs` - #[allow] dead_code tx_id + 注释说明

**测试文件**：
- 现有测试无需修改（重构保持功能一致性）

---

### Phase2: Executor层非唯一索引测试覆盖

**创建文件**：
- `src/executor/index_scan_all.rs` - IndexScanAllExecutor 实现
- `tests/executor/index_scan_all_test.rs` - IndexScanAllExecutor 测试

**修改文件**：
- `src/executor/mod.rs` - 导出 IndexScanAllExecutor
- `src/pipeline/mod.rs` - create_executor_from_plan 支持 IndexScanAllExecutor

---

### Phase3: WAL集成 + Group Commit

**创建文件**：
- `src/storage/wal/mod.rs` - WAL 系统模块
- `src/storage/wal/wal_record.rs` - WALRecord 结构
- `src/storage/wal/wal_writer.rs` - WALWriter 实现
- `src/storage/wal/wal_file.rs` - WALFile 管理
- `tests/storage/wal_test.rs` - WAL 单元测试
- `benches/wal_benchmark.rs` - WAL 性能基准测试

**修改文件**：
- `src/storage/mod.rs` - 导出 WAL 模块
- `src/executor/insert.rs` - 集成 WAL 写入
- `src/executor/update.rs` - 集成 WAL 写入
- `src/executor/delete.rs` - 集成 WAL 写入

---

### Phase4: B-Tree Merge

**创建文件**：
- `tests/storage/btree_merge_test.rs` - B-Tree Merge 测试

**修改文件**：
- `src/storage/btree/leaf_node.rs` - 实现 LeafNode::merge
- `src/storage/btree/internal_node.rs` - 实现 InternalNode::merge
- `src/storage/btree/btree.rs` - delete 递归 merge回传
- `src/storage/buffer_pool.rs` - free_page 集成

---

## Phase1: 架构Warnings清理

### Task P1-1: 创建 JoinConfig struct

**Files:**
- Create: `src/executor/join_config.rs`
- Modify: `src/executor/mod.rs` (导出 JoinConfig)

- [ ] **Step 1: 创建 join_config.rs 文件**

创建文件 `src/executor/join_config.rs`：

```rust
use crate::executor::Executor;

/// Join executor 配置参数集合
///
/// 用于解决 too_many_arguments warning，将 8-9 个参数组织为单一结构体
pub struct JoinConfig {
    pub left_source: Executor,
    pub right_source: Executor,
    pub left_key_column: usize,
    pub right_key_column: usize,
    pub output_columns: Vec<usize>,
    pub left_alias: Option<String>,
    pub right_alias: Option<String>,
}
```

- [ ] **Step 2: 在 mod.rs 导出 JoinConfig**

修改 `src/executor/mod.rs`，添加：

```rust
mod join_config;

pub use join_config::JoinConfig;
```

- [ ] **Step 3: 验证编译**

Run: `cargo build --lib`
Expected: 编译成功，无新增 errors

- [ ] **Step 4: Commit**

```bash
git add src/executor/join_config.rs src/executor/mod.rs
git commit -m "feat(P1): add JoinConfig struct for parameter organization"
```

---

### Task P1-2: 重构 JoinExecutor 使用 JoinConfig

**Files:**
- Modify: `src/executor/join.rs`

- [ ] **Step 1: 读取 join.rs 当前 Executor::new 签名**

Run: `grep -n "pub fn new" src/executor/join.rs | head -1`
Expected: 找到 `pub fn new` 函数签名（确认参数数量）

- [ ] **Step 2: 重构 Executor::new 使用 JoinConfig**

修改 `src/executor/join.rs`，替换 Executor::new 签名：

```rust
// 原签名（假设）：
// pub fn new(
//     left_source: Executor,
//     right_source: Executor,
//     left_key_column: usize,
//     right_key_column: usize,
//     output_columns: Vec<usize>,
//     left_alias: Option<String>,
//     right_alias: Option<String>,
// ) -> Self

// 新签名：
pub fn new(config: JoinConfig) -> Self {
    Self {
        left_source: config.left_source,
        right_source: config.right_source,
        left_key_column: config.left_key_column,
        right_key_column: config.right_key_column,
        output_columns: config.output_columns,
        left_alias: config.left_alias,
        right_alias: config.right_alias,
        // ... 其他字段初始化
    }
}
```

- [ ] **Step 3: 更新 Pipeline 调用点**

修改 `src/pipeline/mod.rs` 中的 `create_executor_from_plan`，构造 JoinConfig：

```rust
// 原调用（假设）：
// let executor = JoinExecutor::new(
//     left, right, left_key, right_key, output_cols, left_alias, right_alias
// );

// 新调用：
let config = JoinConfig {
    left_source: left,
    right_source: right,
    left_key_column: left_key,
    right_key_column: right_key,
    output_columns: output_cols,
    left_alias,
    right_alias,
};
let executor = JoinExecutor::new(config);
```

- [ ] **Step 4: 验证编译和测试**

Run: `cargo test --lib executor::join_test`
Expected: 所有测试通过

- [ ] **Step 5: Commit**

```bash
git add src/executor/join.rs src/pipeline/mod.rs
git commit -m "refactor(P1): JoinExecutor use JoinConfig to reduce parameters"
```

---

### Task P1-3: 重构 AntiJoinExecutor 和 SemiJoinExecutor

**Files:**
- Modify: `src/executor/anti_join.rs`
- Modify: `src/executor/semi_join.rs`
- Modify: `src/pipeline/mod.rs` (调用点)

- [ ] **Step 1: 重构 AntiJoinExecutor::new**

修改 `src/executor/anti_join.rs`，使用 JoinConfig：

```rust
pub fn new(config: JoinConfig) -> Self {
    Self {
        left_source: config.left_source,
        right_source: config.right_source,
        left_key_column: config.left_key_column,
        right_key_column: config.right_key_column,
        output_columns: config.output_columns,
        left_alias: config.left_alias,
        right_alias: config.right_alias,
        // ... AntiJoin 特有字段
    }
}
```

- [ ] **Step 2: 重构 SemiJoinExecutor::new**

修改 `src/executor/semi_join.rs`，使用 JoinConfig：

```rust
pub fn new(config: JoinConfig) -> Self {
    Self {
        left_source: config.left_source,
        right_source: config.right_source,
        left_key_column: config.left_key_column,
        right_key_column: config.right_key_column,
        output_columns: config.output_columns,
        left_alias: config.left_alias,
        right_alias: config.right_alias,
        // ... SemiJoin 特有字段
    }
}
```

- [ ] **Step 3: 更新 Pipeline 调用点**

修改 `src/pipeline/mod.rs` 中的 AntiJoin 和 SemiJoin 调用：

```rust
// AntiJoin
let config = JoinConfig { /* ... */ };
let executor = AntiJoinExecutor::new(config);

// SemiJoin
let config = JoinConfig { /* ... */ };
let executor = SemiJoinExecutor::new(config);
```

- [ ] **Step 4: 验证编译和测试**

Run: `cargo test --lib`
Expected: 所有测试通过

- [ ] **Step 5: Commit**

```bash
git add src/executor/anti_join.rs src/executor/semi_join.rs src/pipeline/mod.rs
git commit -m "refactor(P1): AntiJoin/SemiJoin use JoinConfig to reduce parameters"
```

---

### Task P1-4: 定义 ExecutorFuture type alias

**Files:**
- Modify: `src/executor/mod.rs`
- Modify: `src/executor/pipeline.rs`

- [ ] **Step 1: 在 mod.rs 定义 ExecutorFuture type alias**

修改 `src/executor/mod.rs`，添加：

```rust
use std::pin::Pin;
use std::future::Future;
use crate::executor::result::{ExecutionError, Tuple};

/// Executor 异步执行返回类型别名
///
/// 用于解决 type_complexity warning，简化复杂返回类型签名
pub type ExecutorFuture = Pin<Box<dyn Future<Output = Result<Option<Tuple>, ExecutionError>> + Send>>;
```

- [ ] **Step 2: 更新 Executor trait 使用 ExecutorFuture**

修改 `src/executor/mod.rs` 中的 Executor trait 定义：

```rust
// 原定义（假设）：
// pub trait Executor {
//     fn execute(&mut self) -> Pin<Box<dyn Future<Output = Result<Option<Tuple>, ExecutionError>> + Send>>;
// }

// 新定义：
pub trait Executor {
    fn execute(&mut self) -> ExecutorFuture;
}
```

- [ ] **Step 3: 更新 pipeline.rs 使用 ExecutorFuture**

修改 `src/executor/pipeline.rs` 中的返回类型：

```rust
// 原返回类型（假设）：
// fn create_executor_chain(...) -> Pin<Box<dyn Future<Output = Result<Option<Tuple>, ExecutionError>> + Send>>

// 新返回类型：
fn create_executor_chain(...) -> ExecutorFuture
```

- [ ] **Step 4: 验证编译**

Run: `cargo build --lib`
Expected: 编译成功，type_complexity warning 减少

- [ ] **Step 5: Commit**

```bash
git add src/executor/mod.rs src/executor/pipeline.rs
git commit -m "refactor(P1): add ExecutorFuture type alias to reduce type complexity"
```

---

### Task P1-5: 评估 module_inception (btree/mod.rs)

**Files:**
- Modify: `src/storage/btree/mod.rs` (可能重命名)

- [ ] **Step 1: 检查当前 module_inception warning**

Run: `cargo clippy --message-format=short 2>&1 | grep "module_inception"`
Expected: 显示 `src/storage/btree/mod.rs` 的 warning

- [ ] **Step 2: 评估重命名必要性**

检查 `src/storage/btree/mod.rs` 的导出内容：

```bash
grep "pub mod" src/storage/btree/mod.rs
grep "pub use" src/storage/btree/mod.rs
```

分析：
- 若模块名 `btree` 与主要内容类型 `BTree` 重复 → 可重命名为 `btree_node`
- 若外部引用较少 → 重命名成本低，可执行
- 若外部引用多 → 重命名成本高，使用 #[allow]

- [ ] **Step 3: 选择方案并执行**

**方案A（重命名）**：若重命名成本低

```bash
# 重命名文件
mv src/storage/btree/mod.rs src/storage/btree.rs

# 或修改 mod.rs 内容（保留模块名）
```

**方案B（#[allow]）**：若重命名成本高

修改 `src/storage/btree/mod.rs`，添加：

```rust
// 保留模块名 btree，与 BTree 结构体名称一致（合理设计）
#[allow(clippy::module_inception)]
```

- [ ] **Step 4: 验证 Clippy**

Run: `cargo clippy --message-format=short 2>&1 | grep "module_inception"`
Expected: warning 消失

- [ ] **Step 5: Commit**

```bash
git add src/storage/btree/mod.rs
git commit -m "fix(P1): resolve module_inception warning in btree module"
```

---

### Task P1-6: #[allow] await_holding_lock (buffer_pool.rs)

**Files:**
- Modify: `src/storage/buffer_pool.rs`

- [ ] **Step 1: 检查 await_holding_lock warning**

Run: `cargo clippy --message-format=short 2>&1 | grep "await_holding_lock"`
Expected: 显示 `src/storage/buffer_pool.rs` 的 warning

- [ ] **Step 2: 读取 buffer_pool.rs MutexGuard 跨 await 的代码**

Run: `grep -n "MutexGuard" src/storage/buffer_pool.rs | head -10`
Expected: 找到 MutexGuard 使用位置

- [ ] **Step 3: 分析安全性并添加 #[allow]**

修改 `src/storage/buffer_pool.rs`，在相关函数上添加：

```rust
/// BufferPool 读操作
///
/// 注意：MutexGuard 跨 await 持有，但此设计安全：
/// - 读操作期间不修改 buffer_pool 状态
/// - PageGuard 仅提供读访问（page_data）
/// - 无并发写冲突风险
#[allow(clippy::await_holding_lock)]
pub async fn get_page(...) -> PageGuard {
    // ...
}
```

- [ ] **Step 4: 验证 Clippy**

Run: `cargo clippy --message-format=short 2>&1 | grep "await_holding_lock"`
Expected: warning 消失

- [ ] **Step 5: Commit**

```bash
git add src/storage/buffer_pool.rs
git commit -m "fix(P1): #[allow] await_holding_lock with safety explanation"
```

---

### Task P1-7: #[allow] dead_code 字段 (aggregate.rs, delete.rs)

**Files:**
- Modify: `src/executor/aggregate.rs`
- Modify: `src/executor/delete.rs`

- [ ] **Step 1: 检查 dead_code warnings**

Run: `cargo clippy --message-format=short 2>&1 | grep "dead_code"`
Expected: 显示 `output_columns` 和 `tx_id` 的 warnings

- [ ] **Step 2: #[allow] output_columns (aggregate.rs)**

修改 `src/executor/aggregate.rs`，在 AggregateExecutor 结构体上添加：

```rust
pub struct AggregateExecutor {
    // ...
    /// 聚合输出列名（未使用）
    ///
    /// 未来用途：投影优化，聚合后输出列名，避免重新计算列顺序
    #[allow(dead_code)]
    output_columns: Vec<String>,
}
```

- [ ] **Step 3: #[allow] tx_id (delete.rs)**

修改 `src/executor/delete.rs`，在 DeleteExecutor 结构体上添加：

```rust
pub struct DeleteExecutor {
    // ...
    /// 事务ID（未使用）
    ///
    /// 未来用途：MVCC 事务可见性检查，确保只删除当前事务可见的行
    #[allow(dead_code)]
    tx_id: u64,
}
```

- [ ] **Step 4: 验证 Clippy**

Run: `cargo clippy --message-format=short 2>&1 | grep "dead_code"`
Expected: warnings 消失

- [ ] **Step 5: Commit**

```bash
git add src/executor/aggregate.rs src/executor/delete.rs
git commit -m "fix(P1): #[allow] dead_code fields with future use explanation"
```

---

### Task P1-8: Phase1 验证和总结

- [ ] **Step 1: 验证 Clippy warnings 数量**

Run: `cargo clippy --message-format=short 2>&1 | grep "warning:" | wc -l`
Expected: warnings 从 6降至 0（或 #[allow] 已明确注释）

- [ ] **Step 2: 验证所有测试通过**

Run: `cargo test --lib`
Expected: 174+ tests pass, 0 failures

- [ ] **Step 3: 更新 architecture.md**

在 `.claude/docs/architecture.md` 的 ADR-005 中添加验证结果：

```markdown
**验证结果（Phase1完成）**：
- ✅ warnings 从 6降至 0
- ✅ JoinConfig 简化参数组织
- ✅ ExecutorFuture 简化类型签名
- ✅ #[allow] 合理设计有明确注释
```

- [ ] **Step 4: 更新 tasks.md**

标记 Phase1 所有任务为已完成：

```markdown
### Phase1: 架构Warnings清理 ✅

- [x] T1: 引入 JoinConfig struct
- [x] T2: 定义 ExecutorFuture type alias
- [x] T3: #[allow] await_holding_lock
- [x] T4: #[allow] dead_code 字段
- [x] T5: 解决 module_inception
- [x] T6: Clippy 验证
```

- [ ] **Step 5: Commit**

```bash
git add .claude/docs/architecture.md .claude/docs/tasks.md
git commit -m "docs(P1): update docs with Phase1 completion"
```

---

## Phase2-4: 待编写（后续添加）

Phase2、Phase3、Phase4 的详细任务将在 Phase1 完成后逐步添加到本计划文档中。