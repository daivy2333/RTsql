# IN (SELECT … JOIN …) 子查询计划期误报 Subquery returns multiple columns——get_subquery_first_column 无 Join 形态臂

- Status: open
- Filed: 2026-09-14
- Source: `openspec/changes/2026-09-13-ms09-engine-mvcc-closeout/iterations/001-nlj/000-initial.md`（Act Response Experience Candidates + Blocker Handoff 缺口 1 + Plan Review F1/F5）；Plan Review 本会话独立探针与读码复核
- Environment: Linux x86_64（WSL2）、Rust/Cargo；revision HEAD `e51c4a3`（`src/parser/planner/subquery.rs` 不在 change `2026-09-13-ms09-engine-mvcc-closeout` 改动集内，工作区内容与 HEAD 一致——缺陷在 e51c4a3 即预存，非 Iteration 001 实施引入）

## 缺陷描述

对已有代码的指控（预期 / 实际 / 位置）：

- **预期**：`IN (SELECT <单列> FROM … JOIN …)` 的子查询 SELECT 清单恰为一列时，子查询输出形状是单列，`get_subquery_first_column` 应能从 Join 形态计划（`PhysicalPlan::Join` / `NestedLoopJoin`）提取该列（或至少给出与事实相符的错误）；`PhysicalPlan::SemiJoin`/`AntiJoin`/`Aggregate` 均已有形态臂，Join 族缺臂不是有意拒绝面的证据。
- **实际**：`src/parser/planner/subquery.rs::get_subquery_first_column`（:386-438）的 match 臂覆盖 Scan/DataScan/Filter/Aggregate/SemiJoin/AntiJoin，Join 与 NestedLoopJoin 形态落入 `:437` `_ => Err(PlanError::SubqueryReturnsMultipleColumns)`——单列子查询被误报为「多列」。CLI 实测（本会话新鲜探针，二进制含 Iteration 001 实施）：`SELECT o.x FROM o WHERE o.x IN (SELECT r.a FROM r JOIN s ON r.a = s.b)` → `Plan error: Subquery returns multiple columns (IN subquery requires single column)`，exit 3——**纯等值形态今天即被拒**，非等值形态（Iteration 001 解锁的 NLJ）仅是先行暴露同型缺口。
- 机制分类：确认（Confirmed）——根因经 Plan Review 独立读码核实（`:437` fallback 臂），并经等值形态 CLI 探针行为复现；非推断。

关联面（同域边界，非本 Issue 独立指控）：`extract_correlated_params`/`collect_outer_column_refs`（`subquery.rs:156-240`）仅遍历子查询 WHERE（`select.selection`），ON 子句中的外层引用永不注册 CorrelatedParam；且 WHERE + JOIN（任一 join 节点）被 `query.rs:507` 计划期拒绝——三面叠加使「子查询内 JOIN + 关联」当前无任何可达 SQL 形态。缺口 1 修复后缺口 2 才会成为用户可达面（届时未注入的 `ParameterExpression` 求值为 Null，静默错）。

## 影响

- **当前**：无错误结果、无数据影响——拒绝发生在计划期（fail-fast）。损失是能力面 + 诊断面：结构合法的单列 IN×JOIN 子查询不可达，且错误文案与事实不符（子查询确实只返回一列，报错却称「requires single column」），误导排障方向。
- **潜在**：MS09 Iteration 001 已解锁非等值 JOIN，用户写出 `IN (SELECT r.a FROM r JOIN s ON r.a < s.b)` 时撞同一误报；若未来直接修缺口 1 而不同步处理关联面（缺口 2），关联 ON 形态将产生静默错误结果——两缺口须一并裁定。

## 事件记录

None（未爆发——计划期 fail-fast 拒绝，未引发错误结果或故障；无时间线可记）

## 处置

- 未排期（2026-09-14 用户指令落账）。是否支持 `IN (SELECT … JOIN …)`（需 get_subquery_first_column 形态臂 + 关联参数 ON 遍历两面一并裁定，波及等值 Hash 共享路径的能力解锁）属独立 change 决策，留用户提议时规划；错误文案与事实不符的修正是可独立执行的小项。来源裁定：Iteration 001 000-initial Plan Review F5 + 用户选项 A（见证改形 rework，2026-09-14）——两缺口维持预存边界不入 MS09 范围。

## 证据

- Act Response Verification Evidence 末行（用例 8 探针：等值 IN×JOIN exit 3）+ Blocker Handoff 缺口 1/2/3 机理链：`openspec/changes/2026-09-13-ms09-engine-mvcc-closeout/iterations/001-nlj/000-initial.md`
- Plan Review F1（独立读码 + 探针复现）、F5（候选定性）：同上文件 Plan Review 节
- 代码现场：`src/parser/planner/subquery.rs:386-438`（`:437` fallback）、`:156-240`（WHERE-only 遍历）、`src/parser/planner/query.rs:507`（WHERE+JOIN 拒绝面）
