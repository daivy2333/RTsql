# Iteration 003 / Cycle 000: planner 读面

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4, T5（执行序 T4 → T5）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R4（`join-output-shape-resolution` ADDED + `sql-views` MODIFIED）、R5（`join-output-shape-resolution` ADDED）；design D7/D8
- Excluded scope: Iteration 004 的文案与文档（T6/T7）；IN/EXISTS/标量表达式子查询内的集合操作与表达式项（维持抑制）；JOIN 基座 WHERE 子查询解锁；CTE 通道改动

**Objective**

派生表和视图的内层别名列、表达式列、内层 DISTINCT，外层可以引用，并且与 CTE 通道一致。`IN` / `EXISTS` 基座上，`ORDER BY` 可以按未出现在 SELECT 清单里的左表列排序，输出行仍只有 SELECT 清单。e2e 的 E11 两则 ignore 解除后转绿。

**Background**

Iteration 002 已 accepted。DDL 落盘不再占用本轮。本轮只改 planner 读面：先放行派生/视图内层输出列，再把 SemiJoin/AntiJoin 改成全宽承载，避免两处同时改 `query.rs` 的同一段分类门。

**Investigation Facts**

- Current Baseline: Iteration 002 Act Response（2026-10-13）全量集成 1566 passed、`--lib` 398 passed、`--bins` 0 failed。e2e 16 passed / 6 ignored。余下 ignore 为 `e11_inner_columns_outer_visible`（ISS07）、`e11_semijoin_order_by_non_output_column`（ISS09）、E12 的 ISS10–ISS13 四则。产品代码里的 DDL 落盘与索引修复保持不动。
- Current-State Evidence:
  - FROM 派生表已在 `query.rs:416-428` 把 `building_from_derived` 置真，并在 `build_set_operation`（`query.rs:985`）用 `building_subquery && !building_from_derived` 放行集合操作。
  - 视图展开 `resolve_view_reference`（`query.rs:1296-1304`）把 `building_from_derived` 置**假**。视图定义因此仍走表达式子查询抑制。
  - DISTINCT 是否生效只看 `!building_subquery`（`query.rs:1365`）。派生上下文里的 `SELECT DISTINCT` 不建 Distinct 节点。
  - SELECT 项分类的五道门仍用 `!building_subquery` 才放行表达式/别名路由：`query.rs:1572`、`:1585`、`:1614`、`:1626`、`:1677`（`alias_reroutable`）。派生上下文里 `age AS v` 和 `CASE … AS tier` 被当成普通列名，外层按源列名查找。
  - 标量子查询投影位（`query.rs:1409-1424`）、`subquery.rs:39-52`（IN）、`:94-107`（EXISTS）、`:167-180` 在进入内层前把 `building_from_derived` 置假。这些复位保持。
  - SemiJoin/AntiJoin 的 `output_columns` 来自 `build_output_columns_for_table(table_name, projection_columns)`（`subquery.rs:63-64` 与 `:110-111`）。`SemiJoinExecutorV2::build_output_row`（`semi_join.rs:142-146`）和 AntiJoin 同型函数（`anti_join.rs:143`）只按这份清单从左行取列。
  - `get_plan_output_columns` 的 SemiJoin/AntiJoin 臂（`query.rs:118-127`）直接返回 `output_columns` 的列名。清单变全宽后，表头会跟着变全宽，除非下游裁剪。
  - `sort_projection`（`query.rs:2671-2700`）只覆盖聚合/表达式项、扫描链、WHERE×JOIN 和裸 JOIN。SemiJoin/AntiJoin 落到最后的空 `Vec`，Sort 不裁剪。
- Code and Critical Path: `src/parser/planner/query.rs` 的视图展开、DISTINCT 门、五项分类门、`sort_projection`；`src/parser/planner/subquery.rs` 的 IN/EXISTS 输出列；`src/executor/semi_join.rs` 与 `src/executor/anti_join.rs` 的 `build_output_row`。CTE 走顶层 `build_query`，不经过这些抑制门。

**Implementation Guidance**

T4 把「维持抑制」统一成 `building_subquery && !building_from_derived`。视图展开改为与 FROM 派生一样置 `building_from_derived = true`。IN/EXISTS/标量上下文的复位不动。

T5 在 T4 之后做。SemiJoin/AntiJoin 的输出列改为左表全部列，行内容等于左行。SELECT 清单的裁剪放到已有的 Sort / 投影下游，ORDER BY 仍能看见未入清单的列。字面量 IN 和相关形态的结果行集保持原样。

**Task Contracts**

### T4: 派生表/视图内层输出列外层可见（R4）

- Requirement/Scenario: R4 / S10–S11 + E11 的 ISS07
- Depends on: T1；Iteration 002 已 accepted，本任务不改 catalog/btree
- Targets: `src/parser/planner/query.rs` 的 `resolve_view_reference`、DISTINCT 门（`:1365`）、SELECT 项分类（`:1572`、`:1585`、`:1614`、`:1626`、`:1677`）
- Current behavior: 派生表和视图的内层 `col AS alias`、表达式列、`SELECT DISTINCT` 对外层不可见或静默不去重。CTE 通道已经可见。IN/EXISTS/标量子查询内的集合操作与表达式项被拒绝。
- Required behavior: 派生表、视图、CTE 三个通道上，外层可以引用内层别名列和表达式列；内层 DISTINCT 去重。IN/EXISTS/标量上下文的拒绝文案逐字节不变。
- Preserve: CTE 构建路径不改；`subquery.rs` 与标量投影位对 `building_from_derived` 的复位不改；集合操作在 IN/EXISTS/标量内仍拒绝。
- Forbidden: 改 Iteration 004 的 CLI 文案；放宽 IN/EXISTS 内的集合操作或表达式项；改 JOIN 基座的 WHERE 子查询拒绝。
- First test: 先解除 `e11_inner_columns_outer_visible` 的 ignore，运行 `cargo test --test e2e_test e11_inner_columns_outer_visible -- --ignored --test-threads=1`，预期失败。不要改这则夹具的 SQL。
- GREEN condition: 该 e2e 去掉 ignore 后通过。`derived_scan_alias_test` 与 `view_test` 覆盖别名、表达式、DISTINCT × 派生表/视图/CTE。IN/EXISTS 内集合操作与表达式项仍拒绝。
- Verification: `cargo test --test e2e_test -- --test-threads=1`；`derived_scan_alias_test`、`view_test`、`cte_test`、`subquery_test`、`set_operations_test` 单目标 0 failed。全量留到本 Iteration 末尾，与 T5 合用一次预算。
- Stop when: 放行派生上下文后，IN 或 EXISTS 内的集合操作或表达式项开始成功；或 CTE 既有用例失败且不能用「外层可见」的新语义解释。出现这些情况时停，带回 Plan。

### T5: SemiJoin/AntiJoin 全宽承载与排序键（R5）

- Requirement/Scenario: R5 / S12 + E11 的 ISS09
- Depends on: T4（同一 Iteration，后做，避免与 T4 同时改 SELECT 项分类）
- Targets: `src/parser/planner/subquery.rs:63-64` 与 `:110-111`；`src/executor/semi_join.rs::build_output_row`；`src/executor/anti_join.rs::build_output_row`；`query.rs` 的 `get_plan_output_columns` SemiJoin/AntiJoin 臂、`sort_projection`（`:2671-2700`）、排序键校验（`:2703` 起）
- Current behavior: SemiJoin/AntiJoin 只输出 SELECT 清单里的列。`ORDER BY` 一个没出现在清单里的左表列，在这个基座上排不对或被静默丢掉。
- Required behavior: 节点承载左表全列，输出行在裁剪前等于左行。有 ORDER BY 时，排序能使用未出现在 SELECT 清单中的左表列，最终输出列序和列集仍是 SELECT 清单。没有 ORDER BY 时，输出形状与现在的投影结果一致。字面量 IN、EXISTS、相关子查询的行集不回归。
- Preserve: 字面量 IN 的谓词下推路径；相关参数按左行取值；`get_subquery_first_column` 对空输出列的拒绝。
- Forbidden: 为了让排序通过而把非输出列留在最终结果里；改页格式或 DDL 落盘；清理与本任务无关的死代码。
- First test: T4 已绿之后，解除 `e11_semijoin_order_by_non_output_column` 的 ignore，运行 `cargo test --test e2e_test e11_semijoin_order_by_non_output_column -- --ignored --test-threads=1`，预期失败。不要改这则夹具的 SQL。
- GREEN condition: 该 e2e 去掉 ignore 后通过。`subquery_test` 覆盖 ORDER BY 非输出列 × IN/EXISTS/相关，以及投影形状。形状断言若按全宽中间态校准，逐条记在 Act Response。
- Verification: `cargo test --test e2e_test -- --test-threads=1`；`subquery_test` 与 `cli_test` 里受表头影响的目标 0 failed。全量按名字典序分批一次（本 Iteration 预算），各批 0 failed。
- Stop when: 全宽之后，无 ORDER BY 的 IN/EXISTS 结果比 SELECT 清单多列，且现有裁剪臂盖不住；或相关子查询的参数列被裁掉导致结果变了。出现这些情况时停，带回 Plan，不要放宽 e2e 的输出列断言。

**Invariants**

- `building_from_derived` 的 save/restore 仍然成对。
- 先看到对应 e2e ignore 用例失败，再改分类门或输出列。

**Non-goals**

见 Excluded scope。

**Acceptance**

- R4：S10/S11 由 `derived_scan_alias_test` / `view_test` 扩展和 E11 ISS07 覆盖。
- R5：S12 由 `subquery_test` 扩展和 E11 ISS09 覆盖。
- RTM：R4 → T4 → 上述测试（Covered）；R5 → T5 → 上述测试（Covered）。

**Verification**

E11 两则走 CLI 子进程。单测断言行集和表头。不另做判定层。

**Gate 2 Readiness**

PASS——T4/T5 与 design D7/D8 已在 2026-10-13 的变更计划里获批。本文件只把调查落到当前行号：视图展开仍清掉派生标志，五道分类门和 DISTINCT 仍只看 `building_subquery`，SemiJoin 输出列仍是投影清单，`sort_projection` 没有 SemiJoin 臂。无 TBD。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 投影形状变化会碰到既有 `subquery_test` / `cli_test` 的表头或列数断言。按新语义改断言时逐条记录，不改 SQL 语义。
- design D7 写的行号 1365/1572/1585/1614/1626/1677 与当前文件一致；视图展开在 `:1299-1304`，比 design 草稿的 1296 起点晚三行。以本文件行号为准。

## Act Response

- Status: reported

**Changes**

T4（R4）：

- `resolve_view_reference`（query.rs:1318-1327）：视图展开与 FROM 派生同型置位 `building_from_derived = true`（原复位假），save/restore 严格配对——内层别名/表达式列与 DISTINCT 经分类门放行（R4 三通道一致）。
- `build_query`（query.rs:1347-1353）：新增 `inner_suppressed = building_subquery && !building_from_derived`；DISTINCT 门（`:1391`）与五道 SELECT 项分类门（聚合嵌入 `:1596`/`:1650`、表达式项 `:1609`/`:1650`、`alias_reroutable` `:1697` 附近——以实际行为 `!inner_suppressed` 判定）统一消费；IN/EXISTS/标量三入口复位逐字节保持（query.rs 标量投影位 `:1438-1451` + subquery.rs IN `:44-57`/EXISTS `:104-117`/标量绑定 `:180-193`）。
- 测试：`derived_scan_alias_test` +5（S1 别名/S2 表达式/S3 内层 DISTINCT 去重/S4 派生-视图-CTE 三通道逐字节一致/S5 表达式子查询拒绝面逐字节）+ `view_test` +4（别名/表达式/DISTINCT/嵌套视图）；e2e `e11_inner_columns_outer_visible` 解除 ignore（夹具 SQL 未动）。

T5（R5）：

- `subquery.rs`：`try_build_where_subquery` IN/EXISTS 两臂改 `build_full_output_columns_for_table(table_name)` 基表全列装配（原 `build_output_columns_for_table` 契约内等价改名；`projection_columns` 参数退役、调用方签名保持）；`SemiJoinNode`/`AntiJoinNode` 构造增 `projection: Vec::new()`。
- `plan.rs`：`SemiJoinNode`/`AntiJoinNode` 增 `projection: Vec<usize>` 裁剪位（空 = 恒等全宽）。
- `join_related_config.rs`：`JoinRelatedConfig.output_columns` → `projection`（两执行器共享构造面）。
- `semi_join.rs`/`anti_join.rs`：`build_output_row` 改 `apply_projection(&self.projection, left_row.to_vec())` 左行直通后裁剪（`apply_projection` 为 executor/mod.rs 既有共享原语，空表恒等）。
- `query.rs`：WHERE 子查询臂（`:2043-2056`）无 ORDER BY 时按 `projection_indices` 装节点 `projection`（ORDER BY 时留空恒等，裁剪归 Sort）；`sort_projection`（`:2719-2731`）增 SemiJoin/AntiJoin 臂（索引按基表 schema 序解析，`None` 恒等面 unwrap_or_default）；`get_plan_output_columns` SemiJoin/AntiJoin 两臂按 `projection` 裁剪列名（空投影 = 全宽）。
- `pipeline.rs`：两执行器构造臂改传 `node.projection`；`plan_output_names` SemiJoin/AntiJoin 两臂同款对齐（与 `get_plan_output_columns` 一致，I034 表头纪律）。
- `correlated.rs`：核对参数注入仅遍历 left/right 子节点、不重建节点——`projection` 天然保留（Gate 3 独立核对项）。
- 测试：`subquery_test` +6（R5-S1 非输出列 ASC/DESC 真实排序、S2 与无子查询/字面量 IN 三形态同序、S3 无 ORDER BY 投影形状零全宽泄漏 + 多列投影、S4 EXISTS 基座、S5 NOT IN AntiJoin 基座、S6 相关 IN）；`cli_test` +1 `test_subquery_semi_join_projection_header`（无 ORDER BY/ORDER BY 非输出列/多列投影三形态 columns 与 rows 逐位对齐）；e2e `e11_semijoin_order_by_non_output_column` 解除 ignore（夹具 SQL 未动）。

本会话（Act 恢复）补齐部分：T4/T5 代码面与 T4 测试面、e2e 双 ignore 解除由用户先行完成（工作区未暂存改动核对入账）；Act 补齐 T5 契约的 `subquery_test`/`cli_test` 扩展与全部验证。

**Deviations from Plan**

- `build_output_columns_for_table` → `build_full_output_columns_for_table` 等价局部改名 + `JoinRelatedConfig` 字段 `output_columns` → `projection`：计划以「节点承载全列 + 下游裁剪」表述，未点名配置结构体字段形态；语义与 D8 一致，属可直接定位的等价实现（契约内处理）。
- 无 ORDER BY 形状断言零校准：既有 `subquery_test`/`cli_test` 表头与列数断言在修复后仍全部成立（输出恒 SELECT 清单），Plan Context 预警的白名单校准未发生，空白名单记录于此。
- 验证流程注记：全量分批首跑脚本误将目标名作名字过滤器（缺 `--test` 标志）触发一次全目标构建即中止（无测试结果产出，机器承载保护性失败）；修正为逐目标显式 `--test` 后按 M19 前台串行重跑，批次划分不变。
- 既有缺陷确认流程注记：为裁定 Issue 候选的新旧归属执行过一次 `git stash push --keep-index` 对照（Iteration-002 accepted 状态复现同一错误），pop 冲突经 stash 树精确还原，还原后 worktree/index 与 stash 双向 diff 为空、受影响目标复跑全绿； stash 已清理。

**Self-Review**

- Gate 3（T4）：分类门放行面与 design D7 清单一致（DISTINCT + 五门 + alias_reroutable）；三通道一致性有 S4 逐字节断言；IN/EXISTS/标量拒绝面由 S5 四形态文案断言锁定逐字节；CTE 通道零改动（顶层 build_query 不经抑制门，既有用例零回归实证）；save/restore 六处配对逐处核对。
- Gate 3（T5）：全宽承载与下游裁剪分工与 D8 一致（无 ORDER BY 节点裁剪/有 ORDER BY Sort 裁剪）；`get_subquery_first_column` 空清单拒绝、字面量 IN 谓词下推、相关参数按左行取值三个 Preserve 面均未触碰且有用例锁定（S2/S6 + 既有 32 例零回归）；Forbidden 面（非输出列留最终结果/页格式/DDL 落盘/死代码清理）无触碰——S3 宽度断言即裁决。
- 代码质量：`cargo check --tests` 干净；`cargo clippy --all-targets` 仅剩 Iteration 001 存量 1 条（`secondary_index_equality_test.rs:26` match→if let，上轮已记录、非本 Cycle 触及面）；`cargo fmt` 已过。
- 遗留 Minor（非阻塞）：① `try_build_where_subquery` 以 `let _ = projection_columns;` 退役未消费参数（保签名决策的显式占位）；② T5 相关 IN 内层 WHERE 若为裸名主键等值会触发下述 Issue 候选的既有缺陷面，S6 刻意用非 PK 列规避并注释。

**Verification**

- 先行测试采信：两则 e2e 的 pre-fix RED 由 Iteration 000 `--ignored` 10/10 见证（T1 Act Response 记录）；本 Cycle 解除 ignore 后 GREEN。
- `cargo test --test e2e_test -- --test-threads=1`：18 passed / 0 failed / 4 ignored（余 4 ignore 全部为 E12 ISS10–13，归 Iteration 004）。
- 新增/扩展目标：`subquery_test` 38/38（+6）、`cli_test` 121 passed / 2 ignored（+1 表头对照）、`derived_scan_alias_test` 14/14（+5）、`view_test` 23/23（+4）。
- 定向零回归：`cte_test` 21、`set_operations_test` 19、`join_output_resolution_test` 19、`explain_test` 10、`scalar_subquery_comparison_test` 14、`secondary_index_test` 27、`table_alias_resolution_test` 19——全部 0 failed。
- 全量（M19，本 Iteration 预算一次，前台串行）：`tests/*.rs` 名字典序 122 个集成目标 12 批（11×11 + 1）+ `--lib` + `--bins`，逐批 `--test-threads=1` 全部 0 failed；合计集成 1584 passed / 6 ignored（e2e 4 + cli 2）、lib 398 passed、bins 0。基线对账：Iteration 002 收尾 1566 + 398 → 净增 18 = e2e 解除 2 + subquery +6 + cli +1 + 用户 T4 测试面 +9（derived_scan_alias 5 + view_test 4）。
- `cargo fmt` + `cargo clippy --all-targets`：见 Self-Review。

**Persisted Evidence**

None required——上述命令均可复跑；分批脚本与批结果计数已并入本 Response。

**Experience Candidates**

- Issue 候选（当前 change 范围外、有证据的实质缺陷）：`PlanBuilder::get_subquery_first_column`（src/parser/planner/subquery.rs:633）缺 `PhysicalPlan::IndexScan`/`IndexScanAll` 臂——IN/EXISTS 子查询内层 WHERE 为裸名主键等值时（如 `WHERE dept IN (SELECT id FROM dept WHERE id = 10)`），内层计划走 PK IndexScan 路由（`EXPLAIN` 实证 `IndexScan dept (key=…, columns=[id])`），首列提取落入兜底臂误报 `Subquery returns multiple columns (IN subquery requires single column)`（58000）。限定名（`dept.id = 10`）与非 PK 列谓词均走 DataScan 不受影响。新旧归属已验证：Iteration-002 accepted 状态同现（stash 对照，exit 3），非 T4/T5 引入。证据：本 Response Self-Review Minor ② + `EXPLAIN` 输出 + 对照组（qualified/non-PK 均正常）。待用户裁定是否登记 Issue。

**Unresolved**

- E12 四则 ignore（ISS10–ISS13）按计划归 Iteration 004（T6），非本 Cycle 遗留。

## Plan Review

- Review Result: accepted

**Findings**

- 非阻塞。`try_build_where_subquery` 用 `let _ = projection_columns;` 留下未使用参数（`src/parser/planner/subquery.rs:31`）。Act Response 已记录。签名保持是契约内选择。
- 非阻塞。`get_subquery_first_column` 与 `extract_column_indices` 的 SemiJoin/AntiJoin 臂仍按全宽 `output_columns` 取第 0 列和下标（`subquery.rs:667-683`，`pipeline.rs:2134-2164`），执行器在 `projection` 非空时已经裁剪行（`semi_join.rs:146-148`）。单列 `IN` 的比较值落在裁剪后行的下标 0，与全宽第 0 列的下标重合，现有行集断言仍然成立。不构成 R4/R5 的验收缺口。
- 非阻塞。`tests/recovery_error_surface_test.rs` 有一处数组字面量换行，行为不变，不属于本轮诊断面。
- 偏差分类：前两项是 Act 在契约内的局部实现；第三项是格式化带过的既有文件。没有 Plan 遗漏或阻塞性 Act 偏离。

**Acceptance Gaps**

无。R4 的别名、表达式列、DISTINCT、三通道一致，以及 IN/EXISTS/标量内拒绝，由 `derived_scan_alias_test` 的 `r4_*`、`view_test` 的 `r4_*` 和 `e11_inner_columns_outer_visible` 覆盖。R5 的非输出列排序、与无子查询/字面量 IN 同序、无 ORDER BY 行宽，由 `subquery_test` 的 `r5_*`、`cli_test::test_subquery_semi_join_projection_header` 和 `e11_semijoin_order_by_non_output_column` 覆盖。视图展开置 `building_from_derived = true`（`query.rs:1322-1327`）。分类门与 DISTINCT 使用 `inner_suppressed`（`query.rs:1353`、`:1392`、`:1599-1653`、`:1704`）。IN/EXISTS/标量三入口仍把该标志复位为假。

**Evidence**

- 独立阅读 `query.rs`、`subquery.rs`、`semi_join.rs`、`anti_join.rs`、`join_related_config.rs`、`plan.rs`、`pipeline.rs` 与对应测试 diff。`correlated.rs` 的参数注入不重建 SemiJoin 节点。E12 仍有 4 个 ignore，E11 两则已无 ignore。
- 验证采信 Iteration 003 Act Response：e2e 18 passed / 4 ignored；`derived_scan_alias_test` 14；`view_test` 23；`subquery_test` 38；`cli_test` 121 passed / 2 ignored；定向目标 0 failed；全量集成 1584 passed / 6 ignored、lib 398 passed、bins 0 failed。Persisted Evidence 为 none。本次审查没有重跑这些命令。

**Follow-up Decision**

R4 与 R5 的可观察结果已满足。Minor finding 不返工。范围外的索引扫描首列误报保持为 Issue 候选，不在本轮修复。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-10-13-e2e-verification-and-defect-closure/iterations/004-wording-and-docs/000-initial.md`

**Issue 候选**

`PlanBuilder::get_subquery_first_column`（`src/parser/planner/subquery.rs:633`）没有 `IndexScan`/`IndexScanAll` 臂，未命中时落入 `_`（`:702`）并报 `Subquery returns multiple columns`。内层 `WHERE` 为裸名主键等值时会走到这条臂。Act Response 已用 Iteration 002 的状态对照，说明不是本轮引入。不落账。
