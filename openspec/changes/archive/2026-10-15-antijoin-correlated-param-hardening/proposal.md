# Proposal: AntiJoin 关联参数绑定期加固（filter_map 静默丢弃终止）

- Date: 2026-10-15
- Status: draft（待 Gate 1 批准）
- 来源：change `2026-10-15-subquery-alias-binding-and-error-prefix` Cycle 000 Plan Review Finding 1（归档 carrier `iterations/000-initial/000-initial.md` Plan Review 节）+ 用户指令「我们继续解决这个」（2026-10-15，含不落账独立 Issue 的裁定——沿 2026-10-05 「范围外 Issue 候选并入修复、不落账」先例）

## Why

前一个 change 的 T2 把 SemiJoin/ScalarSubqueryFilter/SubqueryEval 三处 `extract_param_values` 的 `unwrap_or(0)` 静默回落改为点名报错，但 Plan Review 独立阅读发现第四处同构点漏网：`AntiJoinExecutor::extract_param_values`（`src/executor/anti_join.rs:151-161`）以 `filter_map` **静默丢弃**未命中的相关参数——比 `unwrap_or(0)` 更隐蔽：缺参的 `param_values` 注入右计划（未注入参数求值 NULL）且 `correlated_rows_cache` 键短项化（不同外层行可能误共享缓存项）。T1（前 change）后幻影参数源已消除，正常 SQL 不可触达，属同类防御纵深缺口；NOT IN / NOT EXISTS 路径对未来漏分类仍静默产出错结果。语料 `correlated-subquery-cache` 的「关联参数外层绑定失败显式报错」Requirement 当前枚举三执行器，实现与规格的一致性也要求收口第四处。

## What Changes

1. **T1 AntiJoin 绑定期加固**：`extract_param_values` 由 `filter_map` 静默丢弃改为 `Result` 点名报错（`StorageError::ExecutionError`，消息与前三执行器同模 `correlated parameter column not found in outer row: {列名}`，沿用 58000 兜底不新增 SQLSTATE 分类），调用点（`anti_join.rs:209`）`?` 上浆；命中路径 `(param_name, left_row[idx].clone())` 逐项与顺序逐字节不变。
2. **先行测试**：`anti_join.rs` 内联 `#[cfg(test)]` 单测，直构 `AntiJoinExecutor`（`JoinRelatedConfig` + 既有 `#[cfg(test)] StubRowExecutor`），correlated_params 引用外层行不存在的列，断言 `next()` 报错且消息点名该列、不产出数据行。当前代码下 RED（filter_map 丢弃 → 静默空集）。
3. **规格一致性**：`correlated-subquery-cache` delta MODIFIED「关联参数外层绑定失败显式报错」——执行器枚举扩展为四（含 AntiJoin）、修复前形态描述补 `filter_map` 丢弃、场景 GIVEN 含 AntiJoin。

## Non-goals

- `inject_correlated_values` 与各表达式 `inject_parameters` 的未注入参数求值 NULL 面（`src/executor/correlated.rs`）：同类防御面但签名连锁波及全部表达式类型，前 change T2 已审计为非目标，本 change 维持。
- 计划器/管道任何改动；缓存键结构；新 SQLSTATE 分类；文档（README/SKILL.md 无相关声明需变）。
- 幻影参数源修复本身（前 change T1 已完成，本 change 只收执行器侧最后退路）。

## 默认假设（集中决策项——已取默认值闭合，非 TBD；用户审计可调整）

- **A1** 错误消息、错误类型、调用点上浆手法与前 change T2 三处逐字一致（同款 `correlated parameter column not found in outer row: {列名}`），不引入新文案变体。
- **A2** 本项同属错误面收紧（仅防御路径可达），沿用 58000 兜底，不新增 SQLSTATE 分类。
- **A3** 命中路径 `left_row[idx]` 直接索引（无 `.get(idx)` 回退）保持原样——与前三执行器的差异是历史现状，不在本 change 统一。
