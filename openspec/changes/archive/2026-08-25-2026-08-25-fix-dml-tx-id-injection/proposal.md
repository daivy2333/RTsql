## Why

RTsql 的 SQL 执行入口 `src/pipeline.rs::execute_inner` 在为 INSERT / UPDATE / DELETE 三个 DML 执行器构造时，统一传入 `0` 作为 `tx_id` 占位（`src/pipeline.rs:336/350/363`），但同函数内**没有任何代码会"set"这个值**。这导致每个 DML 写入的数据行都带错误的 `create_tx_id = 0`，进而引发连锁的 MVCC / 事务管理 bug：

1. **MVCC snapshot 失效**：`TransactionManager::begin()` 从未被调用 → `active_tx_ids` 永远不增 → 其他事务的 snapshot 看不到真实的"未提交事务"集合
2. **已提交行不可见**：`TransactionManager::commit()` 从未被调用 → `commit_tx_id` 永不被写入 `VersionHeader` → `find_visible_version` 找不到任何已提交行
3. **abort 路径不存在**：`TransactionManager::abort()` 从未被调用 → 失败的 DML 无法回滚 index 与 version 链
4. **WAL 中所有事务 tx_id = 0**：每个 INSERT/UPDATE/DELETE 仍写 BeginTxn/CommitTxn WAL 记录，但 `tx_id` 字段全是 0；recovery 时无法区分多个事务

更糟的是 `InsertExecutor::next()` / `UpdateExecutor::next()` / `DeleteExecutor::next()` 各自又写了一遍 BeginTxn/CommitTxn WAL（与 `TransactionManager::begin/commit` 重复），且 `tx_id=0` 时这些记录是"伪事务"，recovery 看到 1000 条 BeginTxn{tx_id=0} + 1000 条 CommitTxn{tx_id=0}，无从下手。

这一 bug 的修复优先级在 `tasks.md` MS06-T01 列为"MS06 内最高优先级（影响每个 DML）"。

## What Changes

- **改 `src/pipeline.rs::execute_inner`**：在 DML 路径上用 `TransactionManager::begin() / commit() / abort()` 包裹 executor 构造与执行，把真实 `tx.id()` 传给 executor，替换 `0` 占位
- **改 `src/executor/insert.rs`**：删除内部 BeginTxn/CommitTxn 写入（WAL 唯一来源是 TransactionManager）；保留 Insert WAL record 与 `tx_manager.record_version()` 调用
- **改 `src/executor/update.rs`**：删除内部 BeginTxn/CommitTxn 写入；保留 Update WAL record 与 `tx_manager.record_version()` 调用
- **改 `src/executor/delete.rs`**：删除内部 BeginTxn/CommitTxn 写入；保留 Delete WAL record；**新增** `tx_manager.record_version()` 调用（删除操作的 abort 回滚依赖此记录）
- **改 `src/pipeline.rs::create_executor_from_plan`**：签名增加 `tx_id: Option<u64>` 参数，DML 节点传入真实 tx_id，SELECT 节点传 `None`
- **新增 `tests/dml_tx_id_test.rs`**：覆盖以下场景
  - 通过 `Database::execute_sql("INSERT ...")` 写入后，`VersionHeader.create_tx_id > 0` 且 `commit_tx_id` 已设置
  - UPDATE/DELETE 同样有正确的 `create_tx_id` / `commit_tx_id` / `DELETED_TX_ID`
  - DML 失败（重复 PK）后 `tx_manager.active_transactions()` 不再含该 tx
  - `TransactionManager::current_tx_id()` 在连续 DML 之间单调递增

## Capabilities

### Modified Capabilities

- `dml-transaction-lifecycle`：DML 自动事务的 begin/commit/abort 正确性
  - 改前：每个 DML 用 `tx_id=0` 占位，MVCC 与 abort 路径失效
  - 改后：每次 DML 由 `TransactionManager::begin()` 分配真实 tx_id，executor 完成后由 `TransactionManager::commit()` 标记已提交，失败时由 `TransactionManager::abort()` 回滚
  - 关联 M/K：`M01`（执行管道）、`M10`（MVCC 可见性）、`K07`（inner_column_index 设计教训 — 提醒"按列名/按名匹配更稳"）

## Impact

- **影响模块**：`src/pipeline.rs`（核心入口）、`src/executor/{insert,update,delete}.rs`（3 个 DML 执行器）、`tests/dml_tx_id_test.rs`（新增）
- **影响接口**：仅 `create_executor_from_plan` 内部签名变化（`pub(crate)`，不暴露 public API）
- **影响行为**：
  - 行为差异：DML 写入的数据行 `create_tx_id` 现在取自 `TransactionManager` 原子分配，而非 `0`
  - 行为差异：每次 DML 现在有 `BeginTxn/CommitTxn WAL record`（由 `tx_manager` 写入，**唯一**），而非 executor + tx_manager 各写一次
  - 行为差异：DML 失败时事务会被 abort，`active_tx_ids` 中移除，`tx_versions` 清空
- **兼容性**：
  - 已有 `tests/mvcc_commit_test.rs` / `tests/mvcc_abort_test.rs` / `tests/mvcc_record_test.rs` 直接构造 executor，**不经过** `pipeline.execute_inner`，因此与本 change 解耦
  - 已有 e2e 测试（`tests/e2e_test.rs` / `tests/pipeline_test.rs`）走 `Database::execute_sql`，**正是本 change 的目标场景**；预期全部通过且行为改进
  - WAL 文件格式：BeginTxn/CommitTxn 仍按原 schema 写入（仅 tx_id 字段从 0 变成真实值），旧 db 文件回放无破坏
- **风险**：
  - 中：影响所有 DML 路径；测试覆盖需全面（包括并发场景）
  - 低：MVCC 测试已隔离，不会被本 change 误伤
- **回退方案**：git revert 本 change 即可；Pipeline::execute_inner 旧实现已存在
