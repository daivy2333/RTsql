# Iteration 000 / Cycle 000: MS07-T04 显式事务（公开 API + 事务内执行 + 多表回滚）

## Plan Context

- Status: ready
- Iteration: 000-initial
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3, T4（`tasks.md` §Iteration 000）
- Depends on: None（MS06 事务唯一 WAL 源已完成；MS07-T01/T02/T03 已归档）
- Stable baseline: `Database::{begin,commit,rollback,execute_in_tx}` 可用；多语句 DML 在一个事务内可原子提交/回滚；无显式事务时 `execute_sql` 行为不变
- Verification boundary: `cargo build`/`cargo clippy -D warnings`/`cargo fmt --check` 全 0；`cargo test --all` 0 failures（≥542）；新增显式事务集成测试全绿
- Diagnostic boundary: `src/transaction/manager.rs`、`src/database.rs`、`src/pipeline.rs`、`src/executor/{insert,update}.rs`
- Deferred tasks: Iteration 001（T05 Checkpoint）、Iteration 002（T06 下推）；T07 不在本 change

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 完整 T04 范围（R1 全部场景；不含隔离级别/快照语义变更，对齐 MS09）
- Excluded scope: 任何隔离级别/读可见性变更（扫描保持 `snapshot: None`）；`wal/*`、`planner/*`、网络；T05/T06/T07；SQL 方言

**Objective**

公开 `Database` 层显式事务 API（`begin`/`commit`/`rollback`/`execute_in_tx`），允许把多条 DML/DDL 放进同一事务以获得原子提交/回滚；`TransactionManager` 版本记录按表标记以支持显式多表事务回滚；无显式事务时 `execute_sql` 保持现有隐式自动提交。可见性语义（扫描 `snapshot: None`）本轮不变。

**Background**

- MS07-T04（`tasks.md`）定位"显式事务 API 可用"，验收含"DDL/DML 显式事务可单测"。
- 当前每个 DML 在 `execute_stage`（`src/pipeline.rs:116-183`）自动 `begin→execute→commit`，多语句无法共享一个事务，缺跨语句原子性。
- `TransactionManager::{begin,commit,abort}` 与 `Transaction`（已公开导出，`src/transaction/mod.rs:13`）已存在，缺 `Database` 层 API 与用户事务执行路径。
- 用户决策：MS07 剩余（T04/T05/T06）合并为一个 change；T04 API 采用"属有事务句柄 + 显式 in-tx 执行"。

**Current Baseline**

- Revision: `dc662d4`（HEAD；工作区干净）
- 测试基线：542 tests pass（SNAPSHOT；本 Cycle 以实际运行为准）
- `Database` 无事务公开 API（`src/database.rs:26-110`）
- `TransactionManager` 版本记录 `record_version(tx_id, row_id)` + `tx_versions: HashMap<u64, HashSet<RowId>>`（manager.rs:171,54）；`abort_cleanup_versions(tx, bp, table_meta)` 单表（manager.rs:230）
- `execute_stage` DML 无条件隐式包裹（pipeline.rs:116-183）；`create_executor_from_plan` 已线程化 `tx_id: Option<u64>`（:358-601）；扫描执行器构造 `snapshot: None`（:374/383/393/403）

**Current-State Evidence**

- `src/transaction/mod.rs:13` `pub use manager::{Transaction, TransactionManager, TransactionState};` — `Transaction` 已公开，类 `id()/snapshot()/state()`（manager.rs:32-42），字段私有，`Send` 非常 `Clone`。
- `src/transaction/manager.rs:54` `tx_versions: RwLock<HashMap<u64, HashSet<RowId>>>`；:171 `record_version`；:230 `abort_cleanup_versions`（单 `TableMeta`，索引回退 `update`/`delete`）；:101 `commit`（WAL `append_commit_and_wait` + `commit_mark_versions`）；:134 `abort`（WAL `append` + `abort_cleanup_versions`）。
- `src/executor/insert.rs`/`update.rs` 调用 `record_version(tx_id, row_id)`（见 `grep record_version`）— Task1 需带表名。
- `src/pipeline.rs:96` `execute_stage`（DDL 直包 / DML begin→executor→commit/abort / Query `create_executor(None)`）；:358 `create_executor_from_plan(plan, database, tx_id)`；Insert/Update/Delete `expect("...requires a transaction id")`（:415/:429/:443）。
- `src/pipeline.rs:42/56` `parse_stage`/`plan_stage`（MS06-T04 拆出，`execute_in_tx` 可复用）。
- `src/database.rs:93` `execute_sql` → `pipeline::execute`；`Database` 字段含 `transaction_manager: Arc<TransactionManager>`、`buffer_pool`、`table_manager`（:17-22）。

**Relevant Code**

| 文件 | 符号 | 职责 |
|---|---|---|
| `src/transaction/manager.rs` | `Transaction`, `TransactionManager::{begin,commit,abort,record_version,abort_cleanup_versions}`, `tx_versions` | 事务生命周期 + 版本记录/回滚 |
| `src/database.rs` | `Database::{open,execute_sql,close,…}` | 新增 `begin/commit/rollback/execute_in_tx` |
| `src/pipeline.rs` | `execute`, `parse_stage`, `plan_stage`, `execute_stage`, `create_executor_from_plan` | 用户事务执行路径 |
| `src/executor/{insert,update}.rs` | `record_version` 调用点 | 版本记录带表名 |

**Critical Path**

```
Database::begin ──► transaction_manager.begin() ──► Transaction (owned)
Database::execute_in_tx(sql, &tx) ──► parse_stage → plan_stage
              └─► 用户事务执行路径：DML 用 tx.id()、不隐式 commit；SELECT 复用 tx_id（snapshot 语义不变）
Database::commit(tx,…) ──► transaction_manager.commit（WAL + mark committed + 清 active）
Database::rollback(tx,…) ──► transaction_manager.abort（WAL abort + 按表清理版本 + 索引回退）
                                              ▲ tx_versions[tx_id] 按 (table,row_id) 聚合（Task1）
```

**Implementation Guidance**

- Task1 先行：改 `tx_versions` 结构 + `record_version` 签名 + `abort_cleanup_versions` 多表（这是 T2/T3 的前提）。
- Task2：`Database::begin` 一行委托；`commit`/`rollback` 委托子方法（rollback 需能拿到事务涉及各表的 `TableMeta`——从 `tx_versions[tx_id]` 的每个 table 名 `table_manager.get_table(table)`）。
- Task3：`execute_in_tx` 复用 `parse_stage`/`plan_stage`；DML 分支复用 `tx.id()`、跳过既有自动 begin/commit/abort；SELECT 分支传 `Some(tx.id())`（可见性语义仍 `snapshot: None`，保持现状）。
- 保持 `execute_stage`（隐式 `execute`）行为零变化：新增路径是**旁路**，不触碰隐式分支。
- `Transaction` 非 `Clone`：`execute_in_tx` 取 `&Transaction`（读 `id()`），`commit`/`rollback` 取 `Transaction`（消费）。

**Behavioral Change**

- 当前行为：无显式事务 API；每条 DML 在 `execute_stage` 隐式自动提交；版本记录无表名、回滚单表。
- 目标行为：`Database::begin/commit/rollback/execute_in_tx` 可用；多语句 DML 一事务内原子提交/回滚；版本按表记录、多表回滚索引一致；无显式事务时 `execute_sql` 行为不变。
- 接口变化：新增公开方法（`Database::transaction_manager` 仍为字段）；`TransactionManager::record_version` 签名变化（内部调用点同步）。
- 错误语义：重复 commit/rollback 复用 `AlreadyCommitted`/`AlreadyAborted`（manager.rs:120/154）。

**Change Surface**

| Task | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R1/S1.6,S1.2 | `transaction/manager.rs::{record_version,tx_versions,abort_cleanup_versions}`; `executor/{insert,update}.rs` | 单表版本记录/回滚 | 按表聚合 + 多表回滚 |
| T2 | R1/S1.1,S1.2,S1.3,S1.4 | `database.rs::{begin,commit,rollback,execute_in_tx}` | 无事务 API | 新增公开事务 API |
| T3 | R1/S1.5,S1.6 | `pipeline.rs` | 隐式 execute | 用户事务执行路径（旁路） |
| T4 | R1/R5 | 全工作区 | — | 全量回归 |

**Task Contracts**

### T1: TransactionManager 版本按表标记 + 多表回滚

- Requirement/Scenario: R1/S1.2, R1/S1.6
- Depends on: None
- Targets: `src/transaction/manager.rs::{tx_versions, record_version, abort_cleanup_versions, abort}`; `src/executor/{insert,update}.rs`（`record_version` 调用点）
- Current behavior: `record_version(tx_id, row_id)` 记 `HashSet<RowId>`；`abort_cleanup_versions` 需单个 `TableMeta` 索引回退
- Required behavior: 版本按表聚合；`abort` 遍历每表 `TableMeta` 回退回退每个 `(table,row_id)`；索引"有 prev→update 到 prev；无 prev→delete"
- Required changes: `tx_versions: HashMap<u64, HashMap<String, HashSet<RowId>>>`（或等价，保持 `get_tx_versions`/`tx_versions()` 测试接口可用）；`record_version` 增 `table_name`；`abort_cleanup_versions` 改双循环
- Preserve: `begin`/`commit`/`abort` 对外签名与 WAL 语义；`get_tx_versions(tx_id)`/`tx_versions()` 的公开契约（若改签名需同步测试）
- Forbidden: 改 SQL/隔离级别；碰 WAL 记录格式；破坏 `commit_mark_versions`
- Test witness: `src/transaction/manager.rs` 内既有单测更新为带表名 + 新增多表回滚单测（RED→GREEN）
- GREEN condition: `cargo test --lib transaction` 全绿；`cargo build`/`cargo clippy -D warnings` 0
- Verification: `cargo check`；`cargo clippy -D warnings`
- Stop when: 需改 `Transaction` 公开字段、WAL 记录格式、或 `commit_mark_versions` 语义

### T2: Database 公开事务 API（begin / commit / rollback / execute_in_tx）

- Requirement/Scenario: R1/S1.1, R1/S1.2, R1/S1.3, R1/S1.4
- Depends on: T1
- Targets: `src/database.rs`（新增 4 方法）
- Current behavior: `Database` 无事务 API；`execute_sql` 隐式自动提交
- Required behavior: `begin()->Result<Transaction>`；`commit(tx,…)`/`rollback(tx,…)` 委托子方法（rollback 按 T1 多表）；`execute_in_tx(sql, &tx)->Response` 走用户事务路径
- Required changes: 4 个公开 async 方法；`commit` 委托 `transaction_manager.commit(tx, &buffer_pool)`；`rollback` 需各表 `TableMeta`（由 `tx_versions[tx_id]` 表名 `table_manager.get_table`）；`execute_in_tx` 调 pipeline 用户事务入口
- Preserve: `execute_sql`/`close` 签名；`Database` 字段
- Forbidden: 引入共享"当前事务"可变状态（task-local/全局 cell）——用属有句柄
- Test witness: 新增 `tests/explicit_tx_test.rs`：begin→多 INSERT→commit 生效；rollback 无残留；错误语句后事务仍 Active 可用；重复 commit/rollback 显式错误
- GREEN condition: `cargo test --test explicit_tx_test` 全绿；`cargo build`/`cargo clippy` 0
- Verification: `cargo check`；`cargo test --test explicit_tx_test`
- Stop when: API 形状被迫改为共享可变状态，或需改变 `Transaction` 所有权语义

### T3: pipeline 用户事务执行路径（不复用隐式 auto-commit）

- Requirement/Scenario: R1/S1.5, R1/S1.6
- Depends on: T2
- Targets: `src/pipeline.rs`
- Current behavior: DML 在 `execute_stage` 无条件隐式 begin/commit/abort
- Required behavior: 用户事务入口复用 `parse_stage`/`plan_stage`；DML 用 `tx.id()` 且**不** begin/commit/abort；SELECT 传 `Some(tx.id())`（snapshot 语义不变）
- Required changes: 在 pipeline 增加 `execute_in_tx`（或等价，如 `execute_stage_in_tx`），对 DML 分支复用 `Some(tx_id)`、跳过自动包裹；保持 `execute_stage` 隐式路径零变化
- Preserve: 隐式 `execute_stage`/`execute` 行为逐字；`create_executor_from_plan` 签名
- Forbidden: 修改隐式 DML 分支；改扫描 `snapshot: None`
- Test witness: `tests/explicit_tx_test.rs` 事务内多语句 `create_tx_id` 复用断言；既有隐式 DML 测试不变仍删绿
- GREEN condition: `cargo check`；`cargo test --test explicit_tx_test`（含事务 ID 复用）+ 既有 DML 集成测试全绿
- Verification: `cargo clippy -D warnings`；`cargo fmt --check`
- Stop when: 需改 `create_executor_from_plan` 签名或扫描快照语义

### T4: 全量回归与验证

- Requirement/Scenario: R1（全场景）, R5
- Depends on: T3
- Targets: 全工作区
- Current behavior: 无（T3 已完成）
- Required behavior: `cargo test --all` 0 failures（≥542）；3 项质量命令 0；隐式 `execute_sql`/既有用例全绿
- Required changes: 验证（无代码改动）
- Preserve: 隐式行为、公共 SQL/网络接口
- Forbidden: 为过 clippy 引入 `#[allow]`；改既有测试逻辑
- Test witness: `cargo test --all`
- GREEN condition: `cargo test --all` 0 failures；`cargo build`/`cargo clippy -D warnings`/`cargo fmt --check` 全 0
- Verification: `cargo test --all`；`cargo clippy -D warnings`；`cargo fmt --check`；`openspec validate --all`
- Stop when: 任何 check 失败需返工；或公共行为变化

**Invariants**

- `Transaction` 保持属有、非 `Clone`；`begin` 分配唯一非 0 `tx_id`；事务内所有写出版本 `create_tx_id == tx.id()`。
- 无显式事务时 `execute_sql` 隐式自动提交行为逐字不变。
- 扫描可见性语义不变（`snapshot: None`，全见）；本 Cycle 不引入隔离级别。
- `Database` 事务 API 不引入共享"当前事务"可变状态。
- WAL 记录格式、网络协议、SQL 方言不变。

**Non-goals**

- 隔离级别 / 读自己写 / 快照隔离（MS09）。
- T05 Checkpoint、T06 下推、T07 消息传递。
- 任何 `wal/*`、`planner/*`、网络改动。
- 改变隐式 `execute_sql` 的自动提交行为。

**Acceptance**

| Acceptance | 验证 |
|---|---|
| R1 显式事务生命周期 | T2 集成测试：begin/commit/rollback/错误后可用/重复提交错误 |
| R1 原子性 | T2: begin→多 DML→commit 一次性生效；rollback 无残留 |
| R1 隐式兼容 | T3/T4: 隐式 `execute_sql` 行为不变、既有用例全绿 |
| R1 事务 ID 复用 | T3: `create_tx_id == tx.id()` 且非 0 |
| R1 多表回滚 | T1: 多表事务回滚索引一致 |
| R5 质量门 | T4: 4 项命令全绿 |

**Verification**

- `cargo build`（0 warning）
- `cargo clippy --all-targets -- -D warnings`（0 warning）
- `cargo fmt --check`（0 diff）
- `cargo test --all`（≥542 tests，0 failures）
- `cargo test --test explicit_tx_test`（新增）
- `openspec validate --all`（变更后应含本 spec）

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | 事务/API/pipeline/执行器现状已核实（Current-State Evidence）；`snapshot: None` 语义、单表回滚限制确认 |
| Design | PASS | API 属有句柄；`tx_versions` 按表聚合并支持多表回滚；隐式路径零变化（design.md T04） |
| Iteration Plan | PASS | Iteration 000 单一职责 T1-T4，依赖有序；稳定基线/验证/诊断边界明确 |
| Cycle Scope | PASS | initial；T1-T4 覆盖 R1 全部 scenario（S1.1-S1.6） |
| Task Contracts | PASS | 每 Task 有 Targets/Current/Required/Preserve/Forbidden/Test witness/GREEN/Verification/Stop |
| Traceability | PASS | tasks.md RTM R1 全 Covered |
| Verification | PASS | 4 项质量命令 + explicit_tx_test 通过条件明确 |

**Persisted Evidence**

- Mode: none

`none` — 所有验证（build/clippy/fmt/`cargo test --all`/explicit_tx_test）可低成本本地重跑，决定性输出（退出码、warning/failure 计数、测试数）写入 Act Response 即可；多表回滚与事务 ID 复用可读 diff + 测试断言确认，无需持久化。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超 256 KiB。本 Cycle 无 `required`，不创建 Evidence 目录。

**Risks and Notes**

- **中**：多表回滚依赖 `tx_versions` 结构重构——`get_tx_versions`/`tx_versions()` 测试接口签名变化需同步；保证单表行为回归。
- **中**：`rollback` 需要事务触及各表的 `TableMeta`——从 `tx_versions[tx_id]` 表名查 `table_manager.get_table`；表在该事务后已被 drop 的边界（S2 之外，本场景记录，不做特殊处理，`get_table` 失败即报错）。
- **低**：`execute_in_tx` 复用的三阶段拆分属 Act 可决定局部实现；契约只要求"事务内 DML 不自动 begin/commit + SELECT 复用 tx_id"。
- **非实质**：`execute_in_tx` 的命名/签名（`&Transaction` vs `tx_id`）由 Act 选，但必须保持属有句柄、不消费 `Transaction`（消费仅在 `commit`/`rollback`）。
- 本 Cycle 事务 API 不含安全/数据格式/性能关键路径入侵（隔离语义不变）。

## Act Response

- Status: reported

**Implemented**

Iteration 000 全部 4 个 Task 完成（2026-09-05，工作区未提交，待用户触发 commit）：

- **T1**：`TransactionManager.tx_versions` 改为 `HashMap<u64, HashMap<String, HashSet<RowId>>>` 按表聚合；`record_version(tx_id, table_name, row_id)` 签名带表名；`abort`/`abort_cleanup_versions` 收 `&HashMap<String, Arc<TableMeta>>`，按表遍历回退索引（保留「有 prev → update 到 prev；无 prev → delete」）。**新增墓碑步骤**：每个被回退版本写 `mark_deleted()`（`update_version_header_in_data_page`），否则快照无关扫描（DataScan `snapshot: None` 只跳过 deleted 行）会继续吐出已回滚行，S1.2「无残留」不可通过 `SELECT *` 验证。新增 `tx_version_tables(tx_id)` 访问器；`get_tx_versions`/`tx_versions()` 保持旧签名、语义为跨表并集（mvcc 测试契约不变）。
- **T2**：`Database::{begin, commit, rollback, execute_in_tx}` 四个公开 async 方法；`begin()->Result<Transaction>` 委托 manager；`commit(tx)`/`rollback(tx)` 消费句柄、内部解析 `buffer_pool`；`rollback` 通过 `tx_version_tables` + `table_manager.get_table` 解析各表 `TableMeta`，解析失败的表由 `abort` 显式报错（对齐 Risks 预案）。
- **T3**：`pipeline::execute_in_tx(db, sql, tx_id)` 复用 `parse_stage`/`plan_stage` + 新增 `execute_stage_in_tx`：DDL 立即执行并清 plan cache（镜像隐式路径）；其余节点统一 `create_executor_from_plan(plan, db, Some(tx_id))`——DML 节点消费 tx_id（MS06-T01 契约），query 侧节点忽略之（`snapshot: None` 不变）；全程无隐式 begin/commit/abort。隐式 `execute_stage`/`execute` 行为零变化。
- **T4**：全量回归 553 passed / 0 failed（基线 542 + 新增 8 集成 + 3 新单测）；build/clippy/fmt/openspec validate 全 0。

**Changed Files and Symbols**

| 文件 | 变更 |
|---|---|
| `src/transaction/manager.rs` | `tx_versions` 结构、`record_version`、`abort`、`abort_cleanup_versions`（含墓碑）、新增 `tx_version_tables`；`get_tx_versions`/`tx_versions()`/`commit_mark_versions` 改为并集展平；单测 3 处 abort 调用点 + 新增 `test_record_version_multiple_tables`/`test_abort_cleanup_multi_table`/`test_abort_cleanup_missing_table_meta_errors` |
| `src/database.rs` | 新增 `begin/commit/rollback/execute_in_tx`；import `Transaction`/`HashMap` |
| `src/pipeline.rs` | 新增 `execute_in_tx`/`execute_stage_in_tx`；隐式 DML 分支 `table_meta_for_abort` → `abort_tables` 单表 map（T1 签名机械涟漪，行为等价） |
| `src/executor/{insert,update,delete}.rs` | `record_version` 调用点带表名（`table_meta.name`/`self.table_name`） |
| `tests/mvcc_abort_test.rs`、`tests/concurrent_test.rs` | 6 处 `record_version` + 4 处 `abort` 调用点机械同步，断言零逻辑修改 |
| `tests/explicit_tx_test.rs`（新增） | 8 集成测试覆盖 S1.1–S1.6 全场景 |

**Deviations from Plan**

1. **delete.rs 调用点**：Plan Context 仅列 `executor/{insert,update}.rs`，`record_version` 实际有 3 个调用点；`delete.rs:90` 同步带表名（grep 全量核对，机械变更）。
2. **abort 墓碑**（本 Cycle 最重要的超出契约字面的改动）：`abort_cleanup_versions` 在索引回退后对每个版本写 `mark_deleted()`。计划未列此步，但 DataScan `snapshot: None` 只跳过 deleted 行——无墓碑则回滚后的 INSERT 幽灵行对 `SELECT *` 可见，S1.2 的「表中不残留」无法满足。属同一 Acceptance 的实现细节，不改 WAL 格式、不改签名、不破坏 `commit_mark_versions`（Invariants/Forbidden 全部保持）；`tests/explicit_tx_test.rs::explicit_tx_rollback_leaves_no_insert_residue` + 单测墓碑断言见证。
3. **Database API 签名**：`commit(tx)`/`rollback(tx)` 省略契约行文中的 `buffer_pool`/`table_meta` 参数——`Database` 自持 `buffer_pool`，rollback 按计划内部解析各表 meta；`begin()->Result<Transaction>` 按契约保留 Result（manager.begin 当前无失败路径）。
4. **测试调用点机械同步**：`mvcc_abort_test.rs`/`concurrent_test.rs` 的 `abort`/`record_version` 调用点随 T1 签名同步（T4.3 的「零逻辑修改」指断言不变；调用点形状变更为 Task 1.3 预期的全量同步）。
5. **`tx_versions` 访问器语义**：`get_tx_versions`/`tx_versions()` 返回跨表并集（保留旧测试契约的扁平形状），新增 `tx_version_tables` 提供按表视图。
6. **execute_stage_in_tx 分支合并**：DML 与 Query 分支在用户事务路径下执行体相同（`tx_id` 仅被 DML 节点消费），合并为单一 `_` 分支；DDL 单独镜像隐式分支。

**Blocker Handoff**

None（正常完成）

**Blocker Resolution**

None（未恢复）

**Self-Review**

- Plan compliance: PASS——T1-T4 契约逐项核对（Targets/Preserve/Forbidden/Test witness/GREEN）；Invariants 全保持（owned 非 Clone 句柄、begin 非 0 id、隐式路径逐字不变、`snapshot: None` 不变、无共享"当前事务"状态、WAL/网络/方言不变）；S1.1–S1.6 全覆盖。
- Full diff reviewed: PASS——完整 diff 逐文件复核；无计划外修改（4 个运行时产物文件的删除为本会话开始前已存在的工作区状态，非本 Cycle 产生，未提交）。
- Critical findings unresolved: None
- Important findings unresolved: None
- Minor findings unresolved: 2 项，见 Remaining Issues。

**Verification Evidence**

| 验证项 | 命令 | 输出摘录 | 结论 |
|---|---|---|---|
| T1 见证 RED | `cargo test --lib transaction`（改测试后、改实现前） | `error[E0061]: this method takes 2 arguments but 3 arguments were supplied`（record_version 新签名）等，exit 101 | 预期 RED ✅ |
| T1 GREEN | `cargo test --lib transaction` | `test result: ok. 30 passed; 0 failed`（含多表回滚/缺失 meta 报错/墓碑断言），exit 0 | PASS |
| T2/T3 见证 RED | `cargo test --test explicit_tx_test`（新建后） | `error[E0599]: no method named 'begin' found for struct 'Database'` 等，exit 101 | 预期 RED ✅ |
| T2/T3 GREEN | `cargo test --test explicit_tx_test` | `test result: ok. 8 passed; 0 failed`，exit 0 | PASS |
| 受影响边界 | `cargo test --test mvcc_abort_test --test mvcc_record_test --test mvcc_commit_test --test concurrent_test` | `10+3+4+5 passed; 0 failed`，exit 0 | PASS |
| 全量测试 | `cargo test --all` | `passed: 553 failed: 0`，exit 0（基线 542 + 11 新增） | PASS |
| 构建 | `cargo build` | 0 条 rustc warning（仅 cargo 配置弃用提示），exit 0 | PASS |
| 静态分析 | `cargo clippy --all-targets -- -D warnings` | 0 finding，exit 0 | PASS |
| 格式 | `cargo fmt --check` | clean，exit 0 | PASS |
| OpenSpec | `openspec validate --all` | `Totals: 13 passed, 0 failed (13 items)` | PASS |

**Persisted Evidence**

`None required`（mode: none）——全部验证可低成本本地重跑，决定性输出（退出码、通过/失败计数）已录入上表。

**Experience Candidates**

None

**Remaining Issues**

1. **既有存储层竞态（非本 change 引入，建议登记 Improvement）**：`FileStorage::allocate_page` 与随后并发 `get_page` 存在文件扩展竞态——末页读取偶发 `Io(UnexpectedEof)`。证据：`test_miss_semaphore_backpressure` 在本 Cycle 期间 ~15 次运行失败 3 次（999/1000 ok）；一次性探针（40 轮×1000 miss，用后已删）捕获 `ROUND N page #999 ERR: Io(UnexpectedEof)`，且在 pristine 基线 `dc662d4` 的独立 worktree 上同样复现（12/12 全绿为采样不足，探针 40 轮即中）。与本 change 代码路径无交集（BufferPool/FileStorage 未触碰）。Act 不跨范围修复，交由用户决定是否立 Ixx/新 change。
2. **语句级原子性与 DDL 非事务性（规格外已知边界）**：显式事务内一条多行 INSERT 中途失败时，已写入的前几行保留在事务中（无语句级 undo，随 commit 一起生效）；DDL 在事务内立即生效，`rollback` 不撤销 DDL。两者均在 R1 场景要求之外，S1.3「错误语句后事务仍可用」按规格满足。

**Commit or Diff Reference**

未提交（待用户触发 commit，沿用项目「未 commit（待用户触发）」惯例）。变更面：11 个跟踪文件（6 源码 + 2 既有测试机械同步 + 新增 1 测试文件 + 2 文档）+ change 目录。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（非 Act Self-Review 代替）结果：R1 全部场景有测试见证，全部验证门独立复跑通过，无阻塞 finding。2 项 Minor finding 不阻塞 Acceptance：

- Minor 1：`pipeline::execute_in_tx` 对 `plan_stage` 硬编码 `profiling: false`（pipeline.rs:245），而隐式路径传 `is_profiling_enabled()`（:356）——in-tx 路径 profiling 观测不可用。仅可观测性，不影响行为与验收。
- Minor 2：`Database::rollback` 对不可解析表在构建 map 时静默跳过（database.rs），缺失最终由 `abort` 的 missing-meta 检查显式报错——净语义与计划 Risks 预案一致（"get_table 失败即报错"），双层处理略绕，风格问题。

Act Response §Remaining Issues 的 2 项核实成立并维持：存储层 `allocate_page`/并发 `get_page` 竞态（pristine `dc662d4` 可复现，与本 change 无代码交集）是否立 Ixx 由用户决定；语句级原子性与 DDL 非事务性为 R1 场景之外的规格边界，已记录。

**Deviation Classification**

Act Response 记录的 6 项偏差逐一独立核对 diff 后分类：

1. `delete.rs` 调用点（Plan Context 仅列 insert/update）——`PLAN-OMISSION`，机械签名同步（grep 全量核对 3 处调用点），非实质。
2. abort 墓碑（`abort_cleanup_versions` 对每个回退版本写 `mark_deleted()`）——`ACT-DEVIATION`。核实为 S1.2「无残留」的必要实现细节（DataScan `snapshot: None` 只跳 deleted 行；机制核实：`VersionHeader::mark_deleted` 置 `DELETED_TX_ID` 哨兵（version_chain.rs:71-74），`update_version_header_in_data_page` 第 4 参为未使用占位、`&[]` 正确（data_page.rs:103-128））；不改 WAL 格式、签名、`commit_mark_versions`（Forbidden 全保持），测试见证完整（`explicit_tx_rollback_leaves_no_insert_residue` + 单测墓碑断言）。
3. `commit(tx)`/`rollback(tx)` 省略契约行文的 `buffer_pool`/`table_meta` 形参——`ACT-DEVIATION`，非实质（Implementation Guidance 本就要求内部解析各表 meta；`Database` 自持 `buffer_pool`）。
4. 测试调用点机械同步（`mvcc_abort_test.rs`/`concurrent_test.rs`）——`ACT-DEVIATION`，T1.3 预期的签名全量同步；diff 核对断言零逻辑修改。
5. `get_tx_versions`/`tx_versions()` 跨表并集 + 新增 `tx_version_tables`——非偏差，是 T1 契约 Preserve 条款（保持旧测试契约）的实现。
6. `execute_stage_in_tx` DML/Query 分支合并——非实质等价控制流（tx_id 仅被 DML 节点消费）。

全部不阻塞。

**Acceptance Gaps**

None。逐项核验：S1.1（`explicit_tx_commit_makes_multi_table_writes_visible`：无隐式 begin 以 `current_tx_id` 不变见证、无隐式 commit 以 `commit_tx_id` UNSET 见证）；S1.2（rollback 无残留：`SELECT *`/PK/index 三重视角 + UPDATE 回滚还原）；S1.3（parse 错误 + 约束失败后事务可用并提交）；S1.4（重建句柄后 `AlreadyCommitted`/`AlreadyAborted` 显式错误）；S1.5（隐式自动提交不变 + active 集空）；S1.6（`create_tx_id == tx.id()` 且非 0、commit 盖 `commit_tx_id` 章）；R5 质量门独立复跑全绿。

**Convergence**

N/A（首次 Review，无上一版 gap 可比较）

**Evidence**

独立复跑（2026-09-05，本工作区）：

- `cargo build` → 0 rustc warning（仅 `~/.cargo/config` 弃用提示），exit 0
- `cargo clippy --all-targets -- -D warnings` → 0 finding，exit 0
- `cargo fmt --check` → clean，exit 0
- `cargo test --all` → `passed=553 failed=0`，exit 0（其中 `tests/explicit_tx_test.rs` 8/8）
- `openspec validate --all` → `Totals: 13 passed, 0 failed (13 items)`

代码独立核对：`git diff` 逐文件复核 `manager.rs`（按表聚合/多表回滚/墓碑/访问器）、`database.rs`（四 API）、`pipeline.rs`（隐式路径仅 `abort_tables` 机械变换、`execute_stage_in_tx` 旁路）、3 个 executor 调用点、2 个既有测试同步、`tests/explicit_tx_test.rs`（S1.1–S1.6 映射）。Persisted Evidence mode `none`，无 Evidence 目录要求。

**Follow-up Decision**

接受（`accepted`）：全部 Acceptance 有测试见证且独立验证通过，6 项偏差均为非实质或必要实现细节，2 项 Minor finding 不构成返工理由（不创建修复，记录在案）。后续动作按职责分离：commit 由用户触发；存储层竞态是否立 Ixx 与 SNAPSHOT 刷新由用户调用 `openspec-docs-maintainer` 决定；本 Review 不修改产品代码与全局状态。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

`iterations/001-checkpoint/000-initial.md`（T05 Checkpoint，Plan Context 已展开，Gate 2 全 PASS，Status: ready）