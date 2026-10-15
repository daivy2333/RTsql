# Tasks

> Change: 2026-10-15-subquery-alias-binding-and-error-prefix
> Status: approved（2026-10-15 用户批准；Gate 2 PASS；Iteration 000 Cycle 000-initial Review Result: `accepted`，2026-10-15 Plan Review）

## Task List

| Task | 内容 | Requirement | Iteration |
|---|---|---|---|
| T1 | `extract_subquery_table_names` 声明限定符化（别名/JOIN 臂/派生表）+ 先行集成测试 `tests/subquery_alias_binding_test.rs` **形状覆盖矩阵**（六算子 × 三内层形态 × 谓词列位置 × 相关性 × IN 索引面 + 零回归锚点；含审计新增 NOT EXISTS/NOT IN/JOIN 内层/Derived 内层四形状的判别式期望，RED 先行） | R1 | 000 |
| T2 | 三执行器 `extract_param_values` 未命中点名报错（替代 `unwrap_or(0)`）+ 执行器级先行单测 | R2 | 000 |
| T3 | `StorageError::ExecutionError` 去 thiserror 前缀 + `tests/subquery_test.rs:632` 锚点校准（RED 先行） | R3 | 000 |
| T4 | `tests/e2e_test.rs` 真实 CLI 见证：ISS16/ISS17/ISS18 台账形状 + 审计新增形状（NOT EXISTS 别名、EXISTS JOIN 双臂别名），各带无别名对照 | R1, R2, R3 | 000 |

Requirement 权威文本：`specs/sql-table-alias-resolution/spec.md`（R1 ADDED）、`specs/correlated-subquery-cache/spec.md`（R2 ADDED）、`specs/error-reporting/spec.md`（R3 MODIFIED「错误消息前缀单层」）。

## Iteration Plan

### Iteration 000: 子查询别名绑定修复（ISS16–ISS18）与错误前缀单层化（ISS19）

- Status: completed（Act Response: `reported`；Plan Review 2026-10-15：`accepted`，无 Acceptance Gap；非阻塞 Issue 候选一项——AntiJoin `extract_param_values` 静默丢弃未加固，见 Cycle Plan Review Findings 1）
- Tasks: T1, T2, T3, T4（均 completed）
- Depends on: None
- Stable baseline: 形状覆盖矩阵全 GREEN（含四审计新形状与零回归锚点）；三执行器绑参失败显式报错；CLI 错误文案单层；e2e 台账 + 审计形状见证落地；全量 `cargo test` 零回归（1999 基线 + 新增）
- Verification boundary: 全量 `cargo test` 一次，于 T1–T4 全部落地后执行（Iteration Gate）；任务内定向验证随时进行（`cargo test --test subquery_alias_binding_test`、`--test subquery_test`、`--test e2e_test -- --test-threads=1` 定向函数）
- Diagnostic boundary: `src/parser/planner/subquery.rs`、`src/executor/{semi_join,scalar_subquery_filter,subquery_eval}.rs`、`src/storage/error.rs`、`src/pipeline.rs`、`tests/subquery_alias_binding_test.rs`、`tests/subquery_test.rs`、`tests/e2e_test.rs`
- Non-goals: proposal Non-goals 全项（多级相关/JOIN 形 IN 维持拒绝、遮蔽语义、ast.rs 其他别名点、文档变更）

### 平衡审计

单 Iteration 承载四任务：T1 是唯一实质计划器变更（单函数 + 形状覆盖矩阵先行测试——矩阵是本 change 覆盖缺口的修复载体，与核心修复同故障域、不可拆分验收），T2/T3 是小面加固与文案（各一处主改动 + 先行测试），T4 是端到端见证（台账 + 审计形状）。四者共享单一故障域（子查询别名绑定 + 错误文案漏斗）与同一验证边界（全量一次），合成一个可独立验收的阶段成果「三 issue 修复 + 覆盖矩阵补强 + 错误面收敛」；无多个独立验收结果混装，无单项不成结果的过碎拆分。符合平衡审计。
