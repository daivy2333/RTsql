# tasks — MS09 引擎能力与 MVCC 收尾

> Change: `2026-09-13-ms09-engine-mvcc-closeout`（tasks.md MS09：T01 Read Committed + I033 + I032 / T02 NLJ / T04 关联子查询缓存，单 change 聚合）
> 设计依据：`design.md` D0-D9（调查基线 2026-09-13，HEAD e51c4a3，直接代码调查）
> 需求基线：proposal.md（Gate 1 批准 2026-09-13）+ 4 delta specs

## Task List

### Iteration 000 — 事务可见性域收口（I033 + I032 + RC）

| Task | 内容 | 设计 | 关键落点 |
|---|---|---|---|
| T1 | RED 测试见证：I033 探针序列（库级 INSERT→UPDATE→DELETE 扫描空集 + restart 两态）、未提交删除扫描形态（→ pre-delete 版本）、I032 复活窗口、RC 脏读排除 | D9 | 新 `tests/mvcc_tombstone_visibility_test.rs`、`tests/isolation_level_test.rs` |
| T2 | 墓碑 slot 化：`DeleteExecutor` 写独立墓碑版本（create_tx=删除者、SENTINEL、next→被删 rid）；索引移除保持；`tx_versions` 记录墓碑 rid；WAL Delete 记录格式不变 | D1 | `src/executor/delete.rs` |
| T3 | 抑制判定重写：`superseder_suppresses` 按删除者提交状态（aborted 标记 create_tx=0 不抑制；活跃→不抑制；已提交→抑制）；`VersionHeader::mark_aborted`；DataScan 构造增加 `Option<Arc<TransactionManager>>` 并穿线 | D2 | `src/executor/data_scan.rs`、`src/transaction/version_chain.rs`、`src/pipeline.rs` |
| T4 | abort 中性化：`abort_cleanup_versions` 墓碑化改 `mark_aborted`（含 DELETE 墓碑 slot 情形） | D2 | `src/transaction/manager.rs` |
| T5 | 恢复侧：Delete redo 臂写墓碑 slot；`mark_uncommitted_aborted` 实施页链迭代标记（I032）；`BufferPool::mark_tx_aborted` no-op 移除 | D3 | `src/wal/recovery.rs`、`src/storage/buffer_pool.rs` |
| T6 | Read Committed：`IsolationLevel` + `Database::open_with_isolation`（open 委托 RR）；`create_executor_from_plan` 快照参数穿线（execute_stage 查询臂 / execute_stage_in_tx / 子查询与 Semi/Anti 重建 / DerivedScan）；DataScan 可见性 `is_visible ∨ is_visible_self`。**001-replan 修订（D10）**：Snapshot 自身身份/高水位分离（`statement_view` 构造器 + `is_visible` 规则 2 改用高水位）+ 恢复后分配器水位推进（`advance_past`，消除重启 id 复用）；T6 主体已按原契约实施（000-initial），修订面为 snapshot.rs / database.rs / tx_id.rs | D4/D10 | `src/transaction/mod.rs`（枚举）、`src/transaction/snapshot.rs`、`src/transaction/tx_id.rs`、`src/database.rs`、`src/pipeline.rs`、`src/executor/data_scan.rs` |
| T7 | GREEN 收尾：T1 全套转绿 + 全量回归零修改 + restart 两态一致性 | D7/D9 | 测试套件 |

### Iteration 001 — NLJ 与启发式切换（T02）

| Task | 内容 | 设计 | 关键落点 |
|---|---|---|---|
| T10 | RED 测试见证：非等值 ON（`r.a < s.b`、混合腿）当前 `Plan error: Unsupported expression type` | D9 | 新 `tests/nested_loop_join_test.rs` |
| T11 | `NestedLoopJoinNode` + `NestedLoopJoinExecutor`（组合行谓词求值、三值语义、output_columns 产出、流式 Volcano 形态） | D5 | `src/executor/plan.rs`、新 `src/executor/nested_loop_join.rs`、`src/pipeline.rs` 构造臂 |
| T12 | planner ON 分类：AND 分解全等值腿→Hash 保持；任一非等值腿→NLJ；ON 整体经 WHERE 谓词编译器在组合行布局（左 0..n/右 n..n+m）编译 | D5 | `src/parser/planner/ddl_dml.rs`、`query.rs` |
| T13 | 注册面：`get_plan_output_columns`、`inject_correlated_values`（递归左右 + 参数注入）、`extract_column_indices` 新臂 | D5 | `query.rs`、`correlated.rs`、`pipeline.rs` |
| T14 | GREEN 收尾：语义连接/NULL/空表/混合腿/plan 形状/关联注入用例转绿 + 全量回归零修改 | D9 | 测试套件 |

### Iteration 002 — 关联子查询结果缓存（T04）

| Task | 内容 | 设计 | 关键落点 |
|---|---|---|---|
| T20 | `SubqueryEvalExecutor` 关联臂语句级 `HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>` 缓存；错误不缓存传播；非关联 `cached_result` 保持 | D6 | `src/executor/subquery_eval.rs` |
| T21 | `SemiJoinExecutorV2`/`AntiJoinExecutor` 关联臂同型缓存 | D6 | `src/executor/semi_join.rs`、`anti_join.rs` |
| T22 | 测试：等价性（含 NULL 参数值/相异值不串结果）、跨语句新鲜度（语句间修改数据后按新数据求值）、错误传播面、既有子查询回归；GREEN 收尾全量 | D6/D9 | `tests/subquery_test.rs` 扩展或新文件 |

## Iteration Plan

### Iteration 000: 事务可见性域收口

- Tasks: T1-T7
- Depends on: None
- Stable baseline: I033 探针序列扫描空集（运行期 + restart）、未提交删除/回滚扫描语义正确、I032 未提交行不复活、RC 可配置且脏读排除；默认 RR 全量零回归
- Verification boundary: T1/T7 测试套件全绿 + 全量基线零修改 + clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: 版本链/扫描可见性族（data_scan、version_chain、delete、manager、recovery）+ isolation 接线面
- Non-goals: NLJ（001）、子查询缓存（002）、未提交删除点查/回滚索引时序边界（Issue 候选）、RR 真快照化、写写冲突检测

### Iteration 001: NLJ 与启发式切换

- Tasks: T10-T14
- Depends on: None（与 000 无代码耦合；按执行序后行）
- Stable baseline: 纯等值 ON Hash 形状与结果逐字节保持；非等值/混合 ON 经 NLJ 产出语义连接结果；计划期拒绝面消除
- Verification boundary: T10/T14 测试套件全绿 + 全量回归零修改
- Diagnostic boundary: planner join 分类 + NLJ 执行器 + 注册面
- Non-goals: SMJ、代价模型、JOIN 类型扩展（LEFT/RIGHT/FULL 维持拒绝）、Semi/AntiJoin 改造

### Iteration 002: 关联子查询结果缓存

- Tasks: T20-T22
- Depends on: Iteration 000（快照穿线面；执行序在 001 后）
- Stable baseline: 相同参数值不重复执行（缓存命中）、结果与直执行等价、跨语句无残留、错误语义不变
- Verification boundary: T22 测试全绿 + 全量回归零修改
- Diagnostic boundary: subquery_eval / semi_join / anti_join 缓存面
- Non-goals: 非关联子查询改造（已有 cached_result）、多层关联（I018 已归档）、跨语句缓存、test-only 计数 hooks

平衡审计：000 承载 T01 全部（同一故障域同一验证面，I033 为 RC 语义前提，拆开则互相触碰同一函数族）；001/002 各为单一可独立验收成果、不同故障域。无过碎/过重。

## Requirements Traceability Matrix

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| transaction-isolation-levels R1 | 默认零回归 / RC 可选打开 | D4 | T6 | 000 | `database.rs::open_with_isolation` | 全量零回归 + isolation_level_test | None | Covered |
| transaction-isolation-levels R2 | 脏读排除 / 语句间可见·消失 / 自身写可见 / auto-commit 等价 | D4/D10 | T1,T6 | 000 | `pipeline.rs::execute_stage/execute_stage_in_tx`、`database.rs::statement_snapshot`、`snapshot.rs`（001-replan 修订面） | isolation_level_test | None | Covered |
| transaction-isolation-levels R3 | 全量零修改 | D4/D7 | T7 | 000 | 全局 | 全量回归 | None | Covered |
| mvcc-tombstone-visibility R1 | 未提交删除扫描 / header 不改写 | D1/D2 | T1,T2,T3 | 000 | `delete.rs`、`data_scan.rs` | mvcc_tombstone_visibility_test | None | Covered |
| mvcc-tombstone-visibility R2 | I033 探针 / Z1/Z2 锚点 | D1/D2 | T1,T2,T3 | 000 | `data_scan.rs::superseder_suppresses` | mvcc_tombstone_visibility_test | None | Covered |
| mvcc-tombstone-visibility R3 | 回滚恢复 / 自身删除已删语义 | D2 | T1,T3,T4 | 000 | `manager.rs::abort_cleanup_versions` | mvcc_tombstone_visibility_test | None | Covered |
| mvcc-tombstone-visibility R4 | restart 两态 / 未提交 DELETE 不生效 / I032 不复活 | D3 | T1,T5 | 000 | `recovery.rs::redo_record/mark_uncommitted_aborted` | mvcc_tombstone_visibility_test | None | Covered |
| mvcc-tombstone-visibility R5 | 全量零修改 | D7 | T7 | 000 | 全局 | 全量回归 | None | Covered |
| join-executor-selection R1 | 纯等值 Hash 保持 | D5 | T12,T14 | 001 | `ddl_dml.rs::extract_join_conditions`、`query.rs` | join_test 既有 + nested_loop_join_test | None | Covered |
| join-executor-selection R2 | 非等值语义连接 / 混合腿 / 空输入 | D5 | T10,T11,T12,T14 | 001 | 新 `nested_loop_join.rs`、planner 分类 | nested_loop_join_test | None | Covered |
| join-executor-selection R3 | 三值 NULL / 类型面对齐 | D5 | T11,T14 | 001 | `nested_loop_join.rs` | nested_loop_join_test | None | Covered |
| join-executor-selection R4 | 计划期启发式可断言 | D5 | T12,T14 | 001 | planner 分类 | plan 形状断言 | None | Covered |
| join-executor-selection R5 | 既有 JOIN 面零回归 | D7 | T14 | 001 | 全局 | 全量回归 | None | Covered |
| correlated-subquery-cache R1 | 重复参数值复用 / 互异值全执行 | D6 | T20,T22 | 002 | `subquery_eval.rs` | subquery 缓存测试 | None（次数观测以等价性+审查承载，见 design D6） | Covered |
| correlated-subquery-cache R2 | 相异值不串 / NULL 键 | D6 | T20,T22 | 002 | `subquery_eval.rs` | 同上 | None | Covered |
| correlated-subquery-cache R3 | 等价 / 错误不缓存 | D6 | T20,T22 | 002 | `subquery_eval.rs` | 同上 | None | Covered |
| correlated-subquery-cache R4 | 跨语句不残留 | D6 | T22 | 002 | 构造面（每语句重建） | 跨语句新鲜度用例 | None | Covered |
| correlated-subquery-cache R5 | Semi/Anti 同型 + 既有零回归 | D6 | T21,T22 | 002 | `semi_join.rs`/`anti_join.rs` | 全量回归 + 缓存测试 | None | Covered |

Status 全部 Covered；无 Simplified、无 Missing。
