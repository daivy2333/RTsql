# Tasks

> Change: 2026-10-15-antijoin-correlated-param-hardening
> Status: approved（2026-10-15 用户批准并指令实施；Gate 2 PASS；Iteration 000 Cycle 000-initial Review Result: `accepted`，2026-10-15 Plan Review，无 Acceptance Gap）

## Task List

| Task | 内容 | Requirement | Iteration |
|---|---|---|---|
| T1 | `AntiJoinExecutor::extract_param_values` `filter_map` 静默丢弃改 `Result` 点名报错（同款消息 `correlated parameter column not found in outer row: {列名}`，调用点 `?` 上浆）+ `anti_join.rs` 内联先行单测（直构执行器，外层缺列断言报错点名；RED 先行） | R1 | 000 |

Requirement 权威文本：`specs/correlated-subquery-cache/spec.md`（R1 MODIFIED「关联参数外层绑定失败显式报错」——执行器枚举扩至四含 AntiJoin）。

## Iteration Plan

### Iteration 000: AntiJoin 绑定期加固

- Status: completed（Cycle 000-initial Act Response: `reported`；Plan Review 2026-10-15：`accepted`，无 Acceptance Gap）
- Tasks: T1（completed，Gate 4 验证满足）
- Depends on: None
- Stable baseline: 四执行器绑定期错误面同模（AntiJoin 补入）；`extract_param_values` 全库收口；既有 NOT EXISTS/NOT IN 套件零回归；全量 `cargo test` 零回归（2029 基线 + 新增单测）
- Verification boundary: 全量 `cargo test` 一次，于 T1 落地后执行（Iteration Gate）；任务内定向验证随时进行（`cargo test --lib executor`）
- Diagnostic boundary: `src/executor/anti_join.rs`、`src/executor/mod.rs`（StubRowExecutor 复用）、`tests/subquery_alias_binding_test.rs`（NOT EXISTS/NOT IN 零回归面）
- Non-goals: proposal Non-goals 全项（inject_correlated_values/表达式注入面、计划器、缓存键结构、SQLSTATE、文档）

### 平衡审计

单 Iteration 单任务：一处执行器加固 + 一个先行单测 + 一域 spec MODIFIED，合成一个可独立验收的阶段成果「AntiJoin 绑定期错误面收口（四执行器同模）」。无多个独立验收结果混装，亦无可合并的相邻在批工作；体量最小但不可再拆（拆出测试不成结果）。符合平衡审计。
