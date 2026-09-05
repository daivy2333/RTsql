# 任务清单：ms07-rest-explicit-tx-checkpoint-pushdown

> 关联里程碑：**MS07**（基础能力建设 / T04 显式事务 + T05 Checkpoint + T06 谓词/LIMIT 下推）
> 关联 design：`design.md`
> 关联 proposal：`proposal.md`
> 关联 spec：`specs/ms07-rest-tx-checkpoint-pushdown/spec.md`

## Iteration Plan

### Iteration 000: T04 显式事务（公开 API + 事务内执行 + 多表回滚）

- Tasks: T1, T2, T3, T4
- Depends on: None（MS06 事务唯一 WAL 源已完成；MS07-T01/T02/T03 已合并，HEAD = `dc662d4`）
- Stable baseline: `Database::{begin,commit,rollback,execute_in_tx}` 可用；多语句 DML 在一个事务内可原子提交/回滚；无显式事务时 `execute_sql` 行为不变
- Verification boundary: `cargo build` 0 warning；`cargo clippy -D warnings` 0 warning；`cargo fmt --check` 0 diff；`cargo test --all` 0 failures（≥542）；新增显式事务集成测试全绿
- Diagnostic boundary: `src/transaction/manager.rs`、`src/database.rs`、`src/pipeline.rs`、`src/executor/{insert,update}.rs`
- Deferred tasks: Iteration 001（T05）、Iteration 002（T06）；T07 不在本 change

### Iteration 001: T05 Checkpoint（恢复消费位点 + 截断 WAL + 静默错显式化）

- Tasks: T1, T2, T3, T4
- Depends on: None
- Stable baseline: 重启后 `redo_count` 随 checkpoint 收敛；WAL 有界；恢复静默吞错显式化为 `Database::open` 可见错误；无 checkpoint 时仍全量恢复
- Verification boundary: 4 项质量命令全绿；restart-e2e 证明 redo 下降 + 数据完整；损坏场景显式报错
- Diagnostic boundary: `src/wal/{checkpoint,recovery,reader,writer}.rs`、`src/database.rs`
- Deferred tasks: Iteration 002（T06）；T07 视实现暴露的并发协调点另开 change

### Iteration 002: T06 谓词/LIMIT 下推（扫描层过滤 + 提前终止）

- Tasks: T1, T2, T3
- Depends on: None
- Stable baseline: 非 PK WHERE 下沿到 DataScan/IndexScan* 行内过滤；无 Sort LIMIT 下推进扫描提前终止；查询结果与改造前完全一致
- Verification boundary: 4 项质量命令全绿；等价性查询测试（NULL/边界/Sort+Limit）全绿；PK 路径不退化
- Diagnostic boundary: `src/parser/planner/{query,expression}.rs`、`src/executor/{data_scan,index_scan,index_scan_all,scan,filter,limit}.rs`
- Deferred tasks: 无（本 Iteration 完成后本 change 全部交付；T07 与 MS09 另论）

## Iteration 000: T04 显式事务（当前展开 Iteration）

### Task 1: TransactionManager 版本记录按表标记，支持多表回滚

- [x] 1.1 `record_version(tx_id, row_id)` → `record_version(tx_id, table_name: &str, row_id)`；`tx_versions` 由 `HashMap<u64, HashSet<RowId>>` 改为按表聚合（如 `HashMap<u64, HashMap<String, HashSet<RowId>>>`）
- [x] 1.2 `abort_cleanup_versions` 改为遍历每个 `(table, row_id)`，按该表 `TableMeta` 做索引回退（保留「有 prev → update 到 prev；无 prev → delete」逻辑）
- [x] 1.3 更新 `InsertExecutor`/`UpdateExecutor` 的 `record_version` 调用点携带表名
- [x] 1.4 `cargo test --lib transaction` 单测更新：单表回归 + 新增多表复用测试
- [x] 1.5 `cargo check` / `cargo clippy -D warnings` 通过

### Task 2: Database 公开事务 API（begin / commit / rollback / execute_in_tx）

- [x] 2.1 `Database::begin(&self) -> Result<Transaction>`：委托 `transaction_manager.begin()`
- [x] 2.2 `Database::commit(&self, tx, buffer_pool) -> Result` 与 `rollback(&self, tx, buffer_pool, table_meta…)`：委托对应子，多表回滚按 Task1 的按表版本
- [x] 2.3 `Database::execute_in_tx(&self, sql, tx:&Transaction) -> Response`：复用 parse/plan（MS06-T04 的 `parse_stage`/`plan_stage`）+ 用户事务执行路径
- [x] 2.4 `execute_in_tx` 对 DML 用 `tx.id()` 且**不**隐式 begin/commit；对 SELECT 亦以事务 id 线程化（可见性语义保持现状）
- [x] 2.5 集成测试：begin→多条 INSERT→commit 一次性生效；rollback 撤销；错误语句后事务仍可用；重复 commit/rollback 显式错误
- [x] 2.6 `cargo test --all` 中既有（隐式）用例不变且全绿

### Task 3: pipeline 用户事务执行路径（不复用隐式 auto-commit）

- [x] 3.1 在 `pipeline.rs` 增加用户事务入口（复用 `parse_stage`/`plan_stage`/`create_executor_from_plan`），DML 分支复用显式 `tx_id`、跳过既有自动 begin/commit/abort 包裹
- [x] 3.2 保持 `execute_stage`/`execute`（隐式路径）行为零变化（无显式事务时）
- [x] 3.3 事务 ID 复用断言：事务内多条语句写出的 `create_tx_id` 均为 `tx.id()` 且非 0
- [x] 3.4 `cargo clippy -D warnings` / `cargo fmt --check` 通过

### Task 4: 全量回归与验证

- [x] 4.1 `cargo test --all` 0 failures（≥542；含新增显式事务测试）
- [x] 4.2 `cargo build` / `cargo clippy -D warnings` / `cargo fmt --check` 全 0
- [x] 4.3 确认隐式 `execute_sql`、`tests/` 既有用例零逻辑修改仍全绿
- [x] 4.4 `openspec validate --all` 通过

## 验收

| Acceptance | 验证 |
|---|---|
| R1 显式事务生命周期 | T2.5 集成测试：begin/commit/rollback + 错误语句可用 + 重复提交错误 |
| R1 原子性 | T2.5 / T4：begin→多 DML→commit 一次性生效；rollback 无残留 |
| R1 隐式兼容 | T2.6/T4.3：隐式 `execute_sql` 行为不变、既有用例全绿 |
| R1 事务 ID 复用 | T3.3：事务内 `create_tx_id` 均为 `tx.id()` 且非 0 |
| R1 多表回滚 | T1.4：多表事务回滚索引一致 |
| R5 质量门 | T4.1/T4.2/T4.4 |

## Requirements Traceability Matrix

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 显式事务 | S1.1-S1.6 | T04 决策 1-5 | T1,T2,T3,T4 | 000 | `database.rs::begin/commit/rollback/execute_in_tx`, `pipeline.rs`, `transaction/manager.rs`, `executor/{insert,update}.rs` | `tests/explicit_tx_test.rs`（新增） | None | Covered |
| R2 Checkpoint | S2.1-S2.4 | T05 决策 1-5 | Iter001 T1-T4 | 001 | `wal/{checkpoint,recovery,reader,writer}.rs`, `database.rs` | `tests/checkpoint_redo_reduction_test.rs`（新增） | None | Covered |
| R3 下推 | S3.1-S3.5 | T06 决策 1-3 | Iter002 T1-T3 | 002 | `planner/{query,expression}.rs`, `executor/{data_scan,index_scan,index_scan_all,scan,filter,limit}.rs` | `tests/pushdown_test.rs`（新增） | None | Covered |

## 与 OpenSpec Changes/迭代同步

- 本 change 一个 proposal + 一个 delta spec（`ms07-rest-tx-checkpoint-pushdown`），内含 R1/R2/R3 三个 Requirement。
- 每 Iteration 一个独立提交；Iteration 000 的 Cycle 目录为 `iterations/000-initial/000-initial.md`。
- 后续 Iteration（001/002）完成再逐个展开新 Cycle；T07 与 MS09 属后续 change。