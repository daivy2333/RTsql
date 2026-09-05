# Design: MS07 剩余工作（显式事务 / Checkpoint / 谓词与 LIMIT 下推）

## 概述

把 MS07 剩余三项可交付基础能力（T04 / T05 / T06）合并为一个 OpenSpec change，按依赖与职责拆为 3 个逻辑 Iteration，各自独立验收、独立提交。T07（消息传递重构）不纳入（见下 T04 API 决策与 T05 收尾评估）。

目标目录结构不变（均在既有模块内扩展）。

## Iteration 规划

| Iteration | 能力 | 目标 | 依赖 |
|---|---|---|---|
| 000 | T04 显式事务 | 公开 API + 事务内执行 + 多表回滚 + 隐式兼容 | MS06（已完成） |
| 001 | T05 Checkpoint | 恢复消费位点 + 截断 WAL + 静默吞错显式化 | 无 |
| 002 | T06 谓词/LIMIT 下推 | 扫描层过滤 + 提前终止 + 结果等价 | 无 |

顺序原则：T04/T05/T06 彼此无强依赖，任一可独立提交；按任务书既有编号 T04→T05→T06 编排便于 MS 进度核对。T07 不在本轮，见各迭代风险评估。

---

## Iteration 000：T04 显式事务

### 当前行为

- `TransactionManager::{begin,commit,abort}` 已实现且 `Transaction` 已公开导出（`crate::transaction::Transaction`，`src/transaction/mod.rs:13`）。`Transaction` 是属有型、`Send`、**非 `Clone`**，被 `commit`/`abort` 消费（`src/transaction/manager.rs:17-43,101,134`）。
- `Database` 无事务公开 API（`src/database.rs:26-110`）。
- `execute_stage`（`src/pipeline.rs:116-183`）对 Insert/Update/Delete **无条件隐式包裹**：`begin → create_executor(Some(tx_id)) → commit`，出错 abort。Query 路径 `create_executor_from_plan(plan, db, None)`；扫描执行器构造传 `snapshot: None`（pipeline.rs:374/383/393/403）。
- `create_executor_from_plan` 已把 `tx_id: Option<u64>` 穿透整棵计划树（pipeline.rs:358-601）；Insert/Update/Delete 分别 `expect("... requires a transaction id")`（:415/:429/:443）。
- MVCC 版本记录 `TransactionManager::record_version(tx_id, row_id)` 仅记 `HashSet<RowId>`（manager.rs:171），**无表名**；`abort_cleanup_versions(tx_id, buffer_pool, table_meta)` 单表清理（manager.rs:230，需单个 `TableMeta` 做索引回退）。

### 设计决策

1. **API 形状：属有事务句柄 + 显式 in-tx 执行**（非 task-local "当前事务" cell，避免引入共享可变状态 → 不触发 T07）。
   - `Database::begin(&self) -> Result<Transaction>`：委托 `transaction_manager.begin()`。
   - `Database::execute_in_tx(&self, sql, tx: &Transaction) -> Response`：内部走一个"用户事务"执行路径（复用 `tx.id()`、**不** begin/commit）。
   - `Database::commit(&self, tx, buffer_pool…)` / `rollback(&self, tx, …)`：委托 `transaction_manager.commit/abort`。
   - 弃用的 `execute_inner`/`execute_stage` 拆解在 `execute_in_tx` 与既有 `execute` 之间复用 parse/plan/execute 三阶段（MS06-T04 已把 `execute_inner` 拆成 `parse_stage`/`plan_stage`/`execute_stage`）。
2. **retdot 表标记以支持多表回滚**：
   - `record_version(tx_id, row_id)` → `record_version(tx_id, table_name, row_id)`；`tx_versions: HashMap<u64, HashSet<RowId>>` → 按表聚合（`HashMap<u64, HashMap<String, HashSet<RowId>>>` 或等价）。
   - `abort` 对 `tx_versions[tx_id]` 的每个 `(table, row_id)` 用该表的 `TableMeta` 做索引回退，从而支持显式多表事务回滚。
   - 受影响调用点：`InsertExecutor`/`UpdateExecutor`（`record_version` 调用处需带表名）。
3. **可见性语义保持现状**：扫描仍 `snapshot: None`（全见）。显式事务内的 SELECT 也保持该语义，**不在本轮引入**读自己写/快照隔离（列 MS09）。T04 只提供生命周期与原子性，验收限定 DDL/DML 显式事务可单测。
4. **隐式兼容**：`execute_sql` 不加参数、不感知显式事务时行为不变（每条 DML 隐式自动提交）。`execute_in_tx` 是新增路径，二者互不干扰。
5. **重复提交/回滚**：复用 `transaction_manager.commit`（已返回 `AlreadyCommitted`）与 `abort`（`AlreadyAborted`）错误语义；事务对象被消费后再次调用由调用方持有的已失效句柄返回显式错误（对已 use-after-move 的属有型，由 `TransactionState`/活动集校验）。

### 边界与风险

- **多表回滚正确性**：`tx_versions` 按表聚合 + 每表 `TableMeta` 回退；需保证 `INSERT`/`UPDATE` 与 `record_version` 表名一致（含派生/子查询写入归属最外层表）。
- **非实质实现选择**：`execute_in_tx` 复用 `parse_stage`/`plan_stage`/`execute_stage_in_tx` 的具体拆分由 Act 决定，但契约要求"事务内 DML 不自动 begin/commit、SELECT 复用 tx_id"。

---

## Iteration 001：T05 Checkpoint

### 当前行为

- `CheckpointManager::checkpoint()`（`src/wal/checkpoint.rs:83-110`）：取 `current_lsn` → `buffer_pool.flush_all()` → `write_checkpoint_site(lsn, ts)`（16B 位点文件）→ 写 `WalRecord::Checkpoint` → `reset_write_count`。**不截断 WAL**。
- `RecoveryManager::full_recover`（`src/wal/recovery.rs:60-131`）：从 WAL 头 `WalReader::read_all` 全量分类 + 重放全部 committed；**不调用 `read_checkpoint_site`**。
- 静默吞错（K05）：recovery.rs:116（`is_ok()`）、:148/:165（表缺失 `return Ok(())`）、:179（索引 delete `let _=`）、:193（mark_aborted `let _=`）。
- `Database::open`（`src/database.rs:26-78`）未接线 `CheckpointManager`；checkpoint 目前仅能被显式调用（若有）。

### 设计决策

1. **恢复消费位点**：`full_recover` 先读 checkpoint 位点（`read_checkpoint_site`），把 WAL reader 定位/裁剪到该 LSN 之后，只重放 checkpoint 之后的记录；「此前记录已因 `flush_all` 落盘」保证不丢已提交数据。
2. **WAL 截断**：checkpoint 后可把 WAL 头部已落盘区域截断（`set_len` 或重写），使 WAL 文件有界。定位 + 截断的选择由 Act 依 `WalReader`/`WalWriter` 现有 IO 能力落地，契约要求"重启 redo 数量随 checkpoint 收敛 + 无丢/无重"。
3. **静默吞错显式化**：恢复路径把 `Ok(())`-吞错改为 `Err(...)` 传播到 `full_recover` 返回值并进一步到 `Database::open`；损坏/表缺失时启动报错（可由调用方决定中止）。«保持幂等：已提交记录仍可安全重放，不因已存在版本而报错»。
4. **接线**：`Database::open` 构造 `CheckpointManager` 并存于 `Database`；在 `close()` 或适当写循环触发 checkpoint。
5. **位点损坏退化**：位点文件缺失/不足 16B → 从头全量重放（等价现状），不 panic。

### 边界与风险

- **重放幂等**：redo 必须幂等（重复写同 tuple 不报错/不重复）；显式化错误不得破坏幂等已提交重放。
- **截断时机安全**：截断前必须确认该前缀已 `flush_all` 且位点已持久化（`write_checkpoint_site` 已 `sync_all`）。
- **T07 触发点**：若实现中发现 `reset_write_count`/LSN 捕获与 WAL flush actor（`buffer.rs:107-140`）并发竞态需 actor 化，在本 change 内以最小方式规避记录之，**另开 change 评估 T07**，不扩大本 Iteration。

---

## Iteration 002：T06 谓词/LIMIT 下推

### 当前行为

- planner `build_query`（`src/parser/planner/query.rs:195`）：PK 等值 → `IndexScan`（:294）；复杂 WHERE+PK → `Filter`（:301-302）；非 PK WHERE → `Filter(Scan→DataScan)`（:311-327）；无 WHERE → `DataScan`（:334-337）。谓词经 `expression.rs::build_where` 生成 `PredicateRef`。
- LIMIT/offset：`query.rs:490-498` 顶层 `PhysicalPlan::Limit`；`parse_limit_value`/`parse_offset_value`（query.rs:679/689）。
- 执行器：`filter.rs` 在 scan 之上逐行过滤；`data_scan.rs`/`index_scan*.rs` 不携带谓词；`limit.rs` 顶层封顶。

### 设计决策

1. **谓词下推**：给 DataScan / IndexScan / IndexScanAll 执行器增加可选谓词字段（`Option<PredicateRef>`）；planner 在构建对应 scan 节点时把 WHERE 谓词一并装入，扫描行迭代内直接 `predicate.eval`，跳过不匹配行，不生成独立 `Filter` 节点。Filter 语义（含 NULL/类型/错误）必须逐字继承现有 `filter.rs`。
   - 无法行内评估的复杂谓词（如 OR 分支 / 需外部状态）→ 保留原 `Filter` 节点（扫描不带该谓词），结果不变。
   - PK 等值路径保持 IndexScan（下推不等价于退化为全表）。
2. **LIMIT 下推**：仅当计划无 Sort/OrderBy 时，把 `LimitNode` 的 `limit+offset` 下推进扫描，扫描产出达到 `limit+offset` 后停止迭代；保留顶层 Limit（安全封顶）。含 `ORDER BY` 时 LIMIT 保持在 Sort 之上（不提前终止）。
3. **等价性验收**：下推后行集/行序 / NULL 边界与改造前完全一致，由既有与新增查询测试证明。

### 边界与风险

- **下推等价性**：谓词求值顺序、NULL/类型错误路径必须与 `filter.rs` 一致；LIMIT+Sort 组合禁止错误提前终止。
- **非实质实现选择**：谓词以 `Option<PredicateRef>` 字段还是闭包注入，由 Act 决定，但必须保持 `predicate.eval` 语义。
- 划分 `query.rs`/`expression.rs`（MS07-T03 已拆）作为天然落点。

---

## 总体影响

- **公开接口**：T04 新增 `Database::{begin,commit,rollback,execute_in_tx}`；`execute_sql` 不变。T05/T06 无公共接口变化。
- **行为**：显式事务原子性活；重启 redo 随 checkpoint 收敛且静默错显式化；查询结果不变、扫描物化减少。
- **兼容**：隐式 `execute_sql`、网络协议、SQL 方言、`tests/` 既有用例全绿。
- **回退**：每 Iteration 独立提交，可独立 revert。

## 不需要修改的文件

- `src/parser/ast.rs`、`value.rs`、`error.rs`（T03 拆分后不动）；`src/network/`。
- 既有 `tests/*` 只新增不删改（除非签名变更必需）。

## 关键实现顺序与原因

1. T04（Iteration 000）：先铺显式事务 API，为后续多语句原子性提供基础；`transaction/manager.rs` 版本表标记是多表回滚前置，独立可测。
2. T05（Iteration 001）：恢复消费位点 + 截断依赖 `WalReader` 能力，独立于事务 API。
3. T06（Iteration 002）：纯 planner/executor 优化，独立，最易验收。

## 风险与缓解

| 风险 | 严重度 | 缓解 |
|---|---|---|
| T04 多表回滚版本表重构 | 中 | `tx_versions` 按表聚合 + 调用点带表名；单表交易回归测试证明兼容 |
| T05 redo 幂等与位点消费 | 中 | 幂等重放保持；位点损坏退化；无 checkpoint 全量恢复 |
| T05 静默错显式化 | 中 | 显式化不破坏已提交幂等重放；`Database::open` 暴露错误 |
| T06 下推等价性 | 中 | 谓词/LIMIT 逐字继承 filter/limit 语义；Sort+Limit 不提前终止 |
| T07 被 T04/T05 意外触发 | 低 | T04 用属有句柄规避；T05 若暴露 actor 竞态则另开 change |