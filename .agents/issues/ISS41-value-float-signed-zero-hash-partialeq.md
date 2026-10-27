# `Float(-0.0)` 与 `Float(0.0)`：`PartialEq` 相等，`Hash` 不同

- Status: closed（2026-10-27 fixed → `openspec/changes/archive/2026-10-26-ms40-join-smj-and-build-side`）
- Filed: 2026-10-26
- Source: Plan Review 的 Issue 候选，`openspec/changes/2026-10-26-ms40-join-smj-and-build-side/iterations/000-smj-executor/000-initial.md`（Review Result `accepted`）
- Environment: 该 Cycle 的 Plan Context 写工作区相对 `e9cb9d3` 有未提交产品改动。审查写明所读文件的修改时间早于本 Cycle 文件写入，其后没有新的提交改写它们。审查未记录平台与工具链版本。

## 缺陷描述

- 预期：`PartialEq` 下相等的连接键应当命中。本 Cycle 的不变量是连接键相等与 `Value` 的 `PartialEq` 一致。`Float(-0.0)` 与 `Float(0.0)` 在该相等关系下是同一键，哈希连接与排序合并都应命中。
- 实际（审查记载）：`Value` 派生 `PartialEq`，`Float(-0.0)` 与 `Float(0.0)` 相等。`Hash` 使用 `to_bits()`，二者哈希不同。
- 后果（审查写「可能」，本 Cycle 未实跑）：哈希连接经 `HashMap` 查找时，负零可能对不上正零。排序合并按 `PartialEq` 会合并这对键。
- 位置：`Value` 的派生 `PartialEq` 与 `Hash`（`to_bits()`）。审查没有给出 `Value` 的文件路径。哈希表在 `JoinExecutor::next`，类型是 `HashMap<Vec<Value>, Vec<Vec<Value>>>`（Plan Context）。排序合并是 `SortMergeJoinExecutor`（`src/executor/sort_merge_join.rs`）；审查 Evidence 写明只有键向量 `==` 才做乘积。`src/executor/join.rs` 本轮未改。
- 根因：`PartialEq` 与 `Hash` 的差异由审查按已读实现写出，标 Confirmed。`HashMap` 因此漏匹配标 Unconfirmed。
- 边界：同一审查把浮点 NaN 的排序比较记为非阻塞 Minor。该条留在审查记录里。

## 影响

本 Cycle 没有观测到这对值的故障。四个多重集合场景没有 `Float(-0.0)` 与 `Float(0.0)`。纯等值 SQL 仍发出哈希连接，不发出排序合并节点。

潜在：哈希查找可能把这对相等的浮点当成不同键。排序合并会把它们当成同一键。两条执行路径对这对键的命中可能不一致。纯等值 SQL 已经走哈希连接，漏匹配若发生，会落在现有 SQL 路径上。排序合并要等计划发出该节点后，才会在 SQL 结果里出现。

## 事件记录

None

## 处置

用户 2026-10-26 授权把实施过程中新登记的缺陷纳入当前里程碑（MS40）的后续工作，由计划安排承载它的 Iteration。

来源审查写明不在本 Cycle 修改 `Value` 或 `JoinExecutor`。

- 2026-10-26 登记。Status 为 open。任务列表尚未指向本条。
- 2026-10-27 关闭。原因：哈希漏匹配已实测。关闭依据是归档变更 `openspec/changes/archive/2026-10-26-ms40-join-smj-and-build-side` Iteration 002 的 accepted 审查：改前过滤测试退出码 101，排序合并有这对行、哈希连接没有；改后过滤测试和整份 `sort_merge_join_test` 退出码 0。处置指针：fixed → `openspec/changes/archive/2026-10-26-ms40-join-smj-and-build-side`。

## 证据

- `openspec/changes/2026-10-26-ms40-join-smj-and-build-side/iterations/000-smj-executor/000-initial.md`：Plan Review「Issue 候选」；Review Result `accepted`。同文件 Evidence 写明排序合并只有键向量 `==` 才乘积，且 `src/executor/join.rs` 未改。Plan Context 写 `Value` 派生 `PartialEq`，以及 `JoinExecutor::next` 使用 `HashMap<Vec<Value>, Vec<Vec<Value>>>`。Act Response 的四个多重集合场景未包含这对值。
- 未知：没有失败用例。`Hash` 的文件路径与行号审查未给出。哈希查找漏匹配未经本 Cycle 实跑。
- 2026-10-27 关闭依据：`openspec/changes/archive/2026-10-26-ms40-join-smj-and-build-side/iterations/002-signed-zero-match/000-initial.md` 的 Plan Review 为 `accepted`。审查沿用 Act Response：改前 `cargo test --test sort_merge_join_test signed_zero_matches_across_hash_and_sort_merge` 退出码 101，排序合并有这对行，哈希连接没有；改后该过滤命令与 `cargo test --test sort_merge_join_test` 退出码 0。
