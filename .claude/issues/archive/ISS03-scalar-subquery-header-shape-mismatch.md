# 标量子查询 select-list 输出表头与行形状不一致——get_plan_output_columns SubqueryEval 臂未计入插入列

- Status: closed
- Filed: 2026-09-14
- Source: `openspec/changes/2026-09-13-ms09-engine-mvcc-closeout/iterations/002-subquery-cache/000-initial.md`（Act Response Experience Candidates + Deviation 1 + Plan Review F2/F4）；Plan Review 本会话独立读码复核
- Environment: Linux x86_64（WSL2）、Rust/Cargo；revision HEAD `e51c4a3`（`src/parser/planner/query.rs` 的 SubqueryEval 臂不在 change `2026-09-13-ms09-engine-mvcc-closeout` 改动集内——git diff 无该臂 hunk，缺陷在 e51c4a3 即预存，非 Iteration 002 实施引入）

## 缺陷描述

对已有代码的指控（预期 / 实际 / 位置）：

- **预期**：SELECT 清单含标量子查询项（`SELECT col, (SELECT …) AS alias FROM t` 形态）时，输出表头应与行形状一致——`SubqueryEvalExecutor` 将标量值插入基表行的 `result_column_index` 位置，表头应在对应位置携带该标量列名（alias）。依据：I034 修复先例（spec `cli-noninteractive-shell`「扫描执行器真投影」——CLI 表头与行形状一致）+ `query.rs:476-477` 既有注释自身记载「标量子查询项会追加一列（SubqueryEval 移位输出形状）」。
- **实际**：`src/parser/planner/query.rs::get_plan_output_columns`（:104）`PhysicalPlan::SubqueryEval(node) => self.get_plan_output_columns(&node.input)`——只返回输入计划的列名，未计入执行器在 `result_column_index` 插入的标量列（插入点 `src/executor/subquery_eval.rs:188-192`）。输出：表头 = 基表 N 列，行 = N+1 值（标量位于其 select-list 位置，如 index 1），CLI JSON 与库 Response 同形，关联与非关联标量子查询同形。Act 探针（Act Response 记载）：`{"columns":["id","name","dept","salary"],"rows":[[1,"East","Alice",10,50000],...]}`——表头 4 列对 5 值行，标量值（dept.region）位于行 index 1，列名（alias）不出现在任何表头位置；单列投影（`SELECT emp.name`）则正常裁剪不受影响。
- **机制分类**：确认（Confirmed）——Plan Review F4 独立读码核实 + git diff 核实该臂非本 change 改动面 + 既有用例（`tests/subquery_test.rs::test_correlated_scalar_subquery` 等 20 用例）按此形状断言通过（预存形状在生产测试面即存在），非推断。

范围说明：I034（promoted，MS15）修复的「CLI 表头与行形状一致」覆盖 `get_plan_output_columns` 的 Scan/DataScan/IndexScan 臂（`projected_columns` 面），SubqueryEval 臂在其修复范围外——本 Issue 为 I034 同族缺口，非回归。

## 影响

- **当前**：无错误结果、无数据影响——行值本身正确，损失是形状描述面：CLI json 的 `columns` 数与 `rows` 每行值数不一致，table/csv/tsv 表头与值错位，标量列名（alias）完全丢失；按表头解析行的脚本/集成消费者将错位或失败。
- **潜在**：任何以 `columns` 驱动行解析的外部消费者（json/csv 下游、未来分析薄命令）持续错位；MS13 `stats`/`profile` 域若以标量子查询形态产出即暴露同一缺口。

## 事件记录

None（未爆发——形状失真持续存在但未引发故障或数据影响；唯一波及为 MS09 Iteration 002 T22 初版见证按「两列理想形状」断言失败，Act 经 CLI/库探针复核确认预存形状后按直执行参照校准（Act Response Deviation 1），无恢复动作）

## 处置

- 未排期（2026-09-14 用户指令落账）。修复方向候选（独立小 change，留用户提议时规划）：`get_plan_output_columns` SubqueryEval 臂按 `node.output_column` + `node.result_column_index` 在输入列名向量对应位置插入标量列名（`SubqueryEvalNode` 已携带两字段，plan 期信息完备）；实施时须与 I034 先例（`projected_columns` 面）及「表达式项与标量子查询项混用」拒绝面（`query.rs:476-483`）一并核对，不改执行器行产出；改后既有按 N 列头断言的用例需同步校准（校准不放宽断言语义）。
- 关联：I034（同族，promoted）；spec `cli-noninteractive-shell` R6「扫描执行器真投影」覆盖面扩展候选。
- 2026-09-23 scheduled → **MS17-T02**（用户裁定并入初版分发收口缺陷清账；修复方向候选〔SubqueryEval 臂按 `result_column_index` 插列名〕随 change 调查定稿，I034 先例核对与既有用例校准要求原样携带；`.claude/docs/tasks.md` MS17 条目「消耗 ISS」已引用）
- 2026-09-23 **fixed → closed**（同 change，Iteration 000 T3，Plan Review accepted）：按处置方向候选实施——`get_plan_output_columns` SubqueryEval 臂在 `min(result_column_index, len)` 处插入 `node.output_column`（镜像执行器 `row.insert`/`push` 语义），执行器行产出与 `SubqueryEvalNode` 字段零改动；CLI json `columns` 长度与 `rows` 每行值数一致，table/csv/tsv 表头对齐，标量列名（alias）在标量位携带。I034 先例核对与既有用例校准要求满足：`tests/subquery_test.rs` 行值断言与 I034 既有表头用例零修改通过。过程记录：Act 发现 delta spec S1 THEN 穷举列名与行形状 Preserve 自相矛盾（按 spec 字面会重现本缺陷），经 Iteration 000 Plan Review 修正为实际行为穷举（F1，`PLAN-INVALID` spec 文字缺陷，非代码缺陷）；Act 测试断言按 Cycle Risks 预授权路径以实际探针校准，断言语义未放宽。spec `cli-noninteractive-shell` 新增 Requirement「标量子查询输出列的表头形状」（3 场景）；测试见证 cli_test +2（先 RED：4 列表头对 5 值行）。

## 证据

- Act Response「Deviation 1」（直执行参照形状与校准经过）、「Experience Candidates」（候选本体 + 探针 JSON）、「Verification Evidence」：`openspec/changes/2026-09-13-ms09-engine-mvcc-closeout/iterations/002-subquery-cache/000-initial.md`
- Plan Review F2（预存性核实）/F4（Issue 候选独立核实成立、定性留 Recorder/用户）：同上文件 Plan Review 节
- 代码现场（Recorder 2026-09-14 只读复认，与 Act/Review 记载一致）：`src/parser/planner/query.rs:104`（SubqueryEval 臂）、`:476-477`（预存形状注释）、`:479-483`（混用拒绝面）；`src/executor/subquery_eval.rs:188-192`（标量插入位置）；既有形状锚点 `tests/subquery_test.rs::test_correlated_scalar_subquery`
