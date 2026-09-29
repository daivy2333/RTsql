# MS25 SQL 读面补全 — Tasks

> 来源：MS25（`.agents/docs/tasks.md`）T01–T06 全量 + I075 promoted；用户批准 2026-09-26 单 change 四 Iteration；2026-09-27 replan 增补 T8/Iteration 004（JOIN 名字解析收口，audit 实证静默缺陷经用户裁定并入）；2026-09-27 用户裁定增补 T9/并入 Iteration 004（聚合输出形状一致性收口，Iteration 001 Act Issue 候选经 Plan Review 核实后不单独落账）；2026-09-28 用户指令 replan——Iteration 004 并入 T10（DerivedScan 基座别名应用收口）与 T11（信号注册时序硬化），来源 Iteration 003 Act/Plan Review 的两项 Issue 候选，design D11 承载；2026-09-28 二次 replan——Cycle 001 交付 T8 主体/T9/T10/T11 并通过 Plan Review（残留 T8 参数面收口项），用户指令并入 T12（聚合参数计划期解析统一，吸收 T8 残留项）与 T13（限定通配文案可操作化），来源 Cycle 001 Plan Review 实测缺口与 Act Issue 候选，design D12 承载。
> 设计依据：`design.md` D1–D10；范围与决策记录见 proposal。
> Gate 1：已批准（2026-09-26，需求 + 默认假设 + 语料修订建模）；T8/T9 范围增补获用户批准（2026-09-27）。

## Task List

| Task | 内容 | Requirement | Iteration |
|---|---|---|---|
| T1 | OUTER JOIN 三态（D1：NLJ 统一承载 LEFT/FULL、RIGHT 计划期左右交换；Hash 执行器零触碰）+ WHERE×JOIN 解禁（D2，含 INNER——Gate 2 统一裁定）+ 拒绝面保持 + 子查询文案统一（`in-subquery-join-rejection` MODIFIED，replan 裁定 2026-09-27 方案 (a)） | `sql-outer-join` R1–R4 + `in-subquery-join-rejection` R1 MODIFIED | 000 |
| T2 | SELECT DISTINCT（D3：Distinct 节点/执行器 + 三分支链位 + 输出列集排序校验 + DISTINCT ON/×子查询项/×复合拒绝） | `select-distinct` R1–R3 | 001 |
| T3 | 集合操作（D4：SetOperation 节点/执行器 + 左结合展开 + 侧臂合成构建 + 列数/quantifier 拒绝面 + 复合 ORDER BY/LIMIT） | `sql-set-operations` R1–R4 | 001 |
| T4 | 聚合扩展（D7：COUNT/SUM/AVG DISTINCT 真实去重 + MIN/MAX DISTINCT 等价 + group_concat 字面量分隔符 + 多参数拒绝） | `sql-aggregate-extensions` R1–R4 | 001 |
| T5 | CTE（D5：planner 注册表 + DerivedScan 内联 + 前向引用/遮蔽/列别名 + RECURSIVE/自引用/子查询内 WITH 拒绝） | `sql-common-table-expressions` R1–R4 | 002 |
| T6 | EXPLAIN（D6：Explain 包装节点 + plan_format 渲染器 + 不执行内层 + DML/DDL/ANALYZE/VERBOSE 拒绝） | `explain-plan-output` R1–R3 | 002 |
| T7 | ORDER BY 输出别名排序（D8：OrderByColumn 表达式键 + 别名优先遮蔽 + decorate-sort-undecorate + scalar_function_test R5/S5 校准） | `order-by-alias-sorting` R1–R3 + `error-reporting` REMOVED/ADDED + `sql-scalar-functions` MODIFIED | 003 |
| T8 | JOIN 输出形状与名字解析收口（D9：聚合×JOIN 全形状解析 + 布局感知歧义拒绝 + 聚合参数 plan 期索引解析 + 非聚合 JOIN 严格解析与别名列可达——audit 实证三面静默缺陷，用户 2026-09-27 裁定并入） | `join-output-shape-resolution` R1–R4 | 004 |
| T9 | 聚合输出形状一致性收口（D10：直出安全判定收窄〔键 ⊆ SELECT ∧ 键序一致〕+ 形状不安全改走既有包装路径 + HAVING/排序绑定零变化——行/表头错配，用户 2026-09-27 裁定不落账 Issue 并入） | `aggregate-output-shape-conformance` R1–R2 | 004 |
| T10 | DerivedScan 基座（CTE / FROM 子查询）普通列别名应用收口（D11：`alias_reroutable` 门放宽 + 表头 = 别名 + ORDER BY 别名键真实排序；与 T8 同一门编辑点落地、验收分离；delta `error-reporting` MODIFIED R5 + `order-by-alias-sorting` R1 场景扩展） | Iteration 003 Act/Plan Review Issue 候选（MS19 T4b 既有静默面），2026-09-28 用户指令并入 | 004 |
| T11 | 信号处理器注册时序硬化（D11：注册先于 open 路径锁获取，消除「锁持有先于注册」窗口；无 corpus delta——既有 130/143 语义意图内的实现修正，固定次数压测见证） | Iteration 003 全量首跑 `test_sigint_during_open_130` 偶发（Issue 候选），2026-09-28 用户指令并入 | 004 |
| T12 | 聚合参数计划期解析统一（D12：JOIN 布局与单表面 `column_indices` 两基座逐参计划期解析——未知列/别名引用 `ColumnNotFound` 点名、JOIN 裸名歧义 `AmbiguousColumn` 保持；吸收 Cycle 001 Review 的 JOIN 面残留收口项；delta `error-reporting` MODIFIED R6） | Cycle 001 Plan Review 实测缺口（Finding 1）+ 单表面聚合未知名参数静默 NULL（Issue 候选，Plan Risk 留档），2026-09-28 用户指令并入 | 004 |
| T13 | 限定通配（`t.*`）拒绝文案可操作化（D12：ast 两处兜底臂 `SELECT ITEM` → 能力名点名，跟随 `error-reporting` R1 框架；拒绝语义零变化——不实现 `t.*` 投影；delta `error-reporting` MODIFIED R1） | Act Issue 候选（`SELECT t1.*` 拒绝文案泛化），2026-09-28 用户指令并入 | 004 |

## Iteration Plan

### Iteration 000: OUTER JOIN 三态与 WHERE×JOIN 解禁

- Tasks: T1
- Depends on: None
- Stable baseline: LEFT/RIGHT/FULL OUTER JOIN 端到端可达（匹配组合 + 未匹配 NULL 补齐 + ON 三值语义 + 非等值 ON）；外连接结果与 WHERE/ORDER BY/LIMIT/聚合组合可达；INNER JOIN 无 WHERE 路径计划形状与结果逐字节不变；WHERE+JOIN（含 INNER）按 D2 组合布局编译可达；NATURAL/CROSS APPLY 等维持点名拒绝
- Verification boundary: `tests/outer_join_test.rs` 矩阵（三态 × 匹配/未匹配/NULL 键/组合面）+ `tests/planner_test.rs` 计划形状与拒绝矩阵 + `tests/nested_loop_join_test.rs` 零校准 + WHERE×JOIN 矩阵 + 全量串行零回归
- Diagnostic boundary: `src/parser/planner/query.rs`（JOIN 臂 + WHERE 臂 + D2 组合布局）、`src/executor/nested_loop_join.rs`（LEFT/FULL 扩展）、`src/executor/plan.rs`（NestedLoopJoinNode 加性字段）、`src/pipeline.rs`（构造臂适配）
- Non-goals: 等值外连接 Hash 性能路径；`Inner(Using/None)` 现状语义变更；Derived 表作 JOIN 右侧；SMJ；表达式项 × JOIN 解禁

### Iteration 001: 输出行代数——DISTINCT / 集合操作 / 聚合扩展

- Tasks: T2, T3, T4
- Depends on: Iteration 000（组合场景涉及 JOIN 基座；排序键校验消费 JOIN 输出形状）
- Stable baseline: SELECT DISTINCT 三分支链位端到端（纯列/表达式/排序组合 + NULL 折叠 + 拒绝面）；UNION[ALL]/INTERSECT/EXCEPT 端到端（去重/保序/NULL 相等/列数校验/优先级/链式/复合排序）；COUNT/SUM/AVG(DISTINCT) 真实去重（静默面 RED→GREEN）、MIN/MAX(DISTINCT) 等价、group_concat 四形态、多参数拒绝；既有聚合与查询面零回归
- Verification boundary: `tests/select_distinct_test.rs` + `tests/set_operations_test.rs` + `tests/aggregate_extension_test.rs`（各含 RED→GREEN 静默面见证）+ `tests/planner_test.rs` 计划形状/拒绝矩阵 + `tests/cli_test.rs` 错误面 e2e + 全量串行零回归
- Diagnostic boundary: `src/executor/{plan.rs,distinct.rs(新),set_operation.rs(新),aggregate.rs}`、`src/parser/planner/{query.rs,mod.rs,aggregate.rs}`、`src/pipeline.rs`（新节点构造臂 + get_plan_output_columns 臂）
- Non-goals: INTERSECT ALL/EXCEPT ALL；ByName 族；DISTINCT × 标量子查询项/×复合（点名拒绝）；CTE（Iteration 002）；EXPLAIN 渲染新节点（Iteration 002 补渲染臂）

### Iteration 002: 计划结构消费——CTE 与 EXPLAIN

- Tasks: T5, T6
- Depends on: Iteration 001（EXPLAIN 渲染 Distinct/SetOperation 节点臂；CTE×集合操作组合）
- Stable baseline: 顶层 WITH 端到端（单/多定义、前向引用、列别名列表、遮蔽基表、引用行集一致、定义内 ORDER BY/LIMIT/集合操作）；RECURSIVE/自引用/反向引用/子查询内 WITH/列数不符点名拒绝；`EXPLAIN <SELECT>` 计划树文本可达且不执行内层（含外连接/DISTINCT/集合操作/CTE 计划形状渲染）；DML/DDL/ANALYZE/VERBOSE/FORMAT 拒绝面
- Verification boundary: `tests/cte_test.rs`（含 WITH 静默面 RED→GREEN 见证）+ `tests/explain_test.rs` + `tests/planner_test.rs` 拒绝矩阵 + `tests/cli_test.rs` EXPLAIN 四格式 e2e + 全量串行零回归
- Diagnostic boundary: `src/parser/planner/{mod.rs,query.rs}`（注册表 + 消费钩子 + Explain 臂）、`src/executor/plan.rs`（ExplainNode）、`src/executor/plan_format.rs`（新渲染器）、`src/pipeline.rs`（Explain 臂）
- Non-goals: 跨引用共享物化（improvement 候选）；子查询内 WITH 解禁；EXPLAIN ANALYZE/VERBOSE/FORMAT 语义；递归 CTE

### Iteration 003: ORDER BY 输出别名排序与语料校准

- Tasks: T7
- Depends on: Iteration 000（JOIN 输出形状排序键）、Iteration 001（DISTINCT×别名排序组合场景）
- Stable baseline: 别名键按输出列真实排序（表达式项别名/普通列别名/DESC/多键混合）；输出名优先遮蔽语义；聚合查询既有别名排序零变化；未知键 `ColumnNotFound` 保持；`tests/scalar_function_test.rs` R5/S5 按白名单校准
- Verification boundary: `tests/order_by_alias_sort_test.rs`（含别名静默面 RED→GREEN 见证）+ `tests/cli_test.rs` 既有排序面零校准 + 全量串行零回归 + 语料 delta 与实现一致核对
- Diagnostic boundary: `src/parser/planner/query.rs`（排序键解析升级）、`src/executor/{plan.rs,sort.rs}`（OrderByColumn 加性字段 + decorate-sort-undecorate）
- Non-goals: SQL 级 `ORDER BY <表达式>`/`ORDER BY t.col`/位置序号解禁；NULLS FIRST/LAST；排序 NULL 语义变更

### Iteration 004: JOIN 输出形状、名字解析、聚合形状与别名应用收口（2026-09-27 增补；2026-09-28 两次 replan 并入 T10/T11 与 T12/T13）

- Tasks: T8, T9, T10, T11, T12, T13
- Depends on: Iteration 000（JOIN 面与 D2 组合布局）；与 001–003 无耦合；T10/T11 与 T12/T13 来源 Issue 候选（用户指令两次 replan 并入）
- 交付状态：Cycle 001 已交付 T8 主体 + T9 + T10 + T11（Act Response + Plan Review 核对通过，全量 1401/0/2）——残留 T8 参数面收口项并入 T12；Cycle 002 已交付 T12 + T13（Act Response + Plan Review accepted；全量串行 1408/0/2、clippy/fmt/openspec validate 干净）
- Stable baseline: 聚合×JOIN 参数与分组键在完整组合列布局上精确解析（聚合参数静默 NULL 与裸名末位胜出消除；限定名精确指向所指表列）；同名裸名歧义 `AmbiguousColumn` 计划期点名拒绝（GROUP BY/聚合参数/非聚合投影三面）；JOIN 基座未知列 `ColumnNotFound` 点名拒绝（零宽行消除）；纯列引用别名项在 JOIN 与 DerivedScan 基座均可达（表头=别名，T8+T10）；任意表达式项×JOIN 维持拒绝；CTE/FROM 子查询基座普通列别名表头生效且 ORDER BY 别名键真实排序（T10——静默回落源列名与输入序消除）；聚合查询输出行与表头恒为 SELECT 清单形状（键未入选不进输出、键序按 SELECT 序）；HAVING/排序绑定零变化；信号处理器注册先于 open 路径锁获取（T11——子进程持锁后 SIGINT 恒 exit 130，无默认处置死亡）；**T12——聚合参数在 JOIN 与单表面均计划期解析：未知列与别名引用 `ColumnNotFound` 点名（静默 0/NULL 终止）、JOIN 裸名歧义 `AmbiguousColumn` 保持、既有聚合形态零回归**；**T13——`SELECT t.*` 三基座拒绝文案点名限定通配能力（泛化 `SELECT ITEM` 终止）**；无歧义有效查询与形状安全的直出形态逐字节不变
- Verification boundary: `tests/join_output_resolution_test.rs`（T8 三面 RED→GREEN 见证 + T12 JOIN 面未知参数 2 场景）+ `tests/aggregate_output_shape_test.rs`（T9 两类错配 RED→GREEN 见证）+ `tests/derived_scan_alias_test.rs`（T10 别名应用 RED→GREEN 见证）+ `tests/planner_test.rs` 形状与拒绝矩阵（含 T12 单表面 2 场景 + T13 文案 2 场景）+ 既有 join/aggregate 套件零校准（T9 白名单已定稿：`distinct_aggregate_output` + `groupby_order_interleave_aligned_to_select_order`）+ T11 信号 e2e 按计划判定方式的固定次数压测全 130/143 + 全量串行零回归
- Diagnostic boundary: `src/parser/planner/query.rs`（聚合分支 `column_indices` 构造与 `resolve_group_by_item` + JOIN 投影解析 + `alias_reroutable` 扩展 + `direct` 判定与包装路由 + ORDER BY 别名装配 + `aggregate_arg_indices` 构造）、`src/executor/aggregate.rs`（`extract_value` 索引消费）、`src/executor/plan.rs`（`AggregateNode` 加性字段）、`src/parser/ast.rs`（兜底臂文案 + 如需聚合参数列提取）、`src/cli/mod.rs`（T11 信号 future 创建与 select 臂序）
- Non-goals: 表达式项×JOIN 解禁；`SELECT t.*` 限定通配投影能力本身（T13 只修文案，能力支持为独立方向）；GROUP BY 分组键列缺失的措辞变更（维持既有 `NonAggregatedColumn` 终态）；有效查询输出形状改变（T9 修复的错配形态除外）；信号语义扩展；子查询内别名；MS21-T03；代价模型

### 平衡审计

- Iteration 000 单任务单一故障域（JOIN 面：执行器扩展 + 计划路由 + WHERE 组合布局），形成完整可验收结果（三态矩阵 + 零回归），标准 TDD，不过碎。
- Iteration 001 三任务同属「输出行代数」主题：Distinct 与 SetOperation 共享去重内核（HashSet<Value> + NULL 相等）与 `get_plan_output_columns`/构造臂接线模式；聚合扩展是同一「结果集成型」面的量化词收口（静默面 RED→GREEN 同构）；拆开会重复同一批链位/校验基建。工作量 ≈ 000 的 1.5 倍，故障域同面（结果集成型），聚合为独立验收单元不过重。
- Iteration 002 两任务同属「计划结构消费」：CTE 生成新计划形状，EXPLAIN 恰好是其可观察面（渲染器同时补齐 000/001 新节点的展示）；T5 planner 级（无新执行器）+ T6 渲染纯函数，工作量适中，顺序耦合（EXPLAIN 臂需含 CTE 相关节点渲染完整性）。
- Iteration 003 单任务单故障域（排序键解析 + Sort 执行器），依赖 000/001 的组合场景齐备后一次校准语料与测试。
- Iteration 004 多任务同属「输出形状与名字解析收口」（正确性主题：静默错误结果与行/表头错配红线同类）：T8（JOIN 面名字解析）与 T9（聚合输出形状）共享 `query.rs` 聚合分支、`executor/aggregate.rs`、`plan.rs::AggregateNode` 诊断边界与调查面；T10（DerivedScan 别名应用）与 T8 同一 `alias_reroutable` 门编辑点；T12（聚合参数计划期解析统一）吸收 T8 的参数面残留收口项，与 T8/T9 同构造点同轮落地；T11（信号注册时序）与 T13（限定通配文案）各自为独立小改（≈10 行级）。Cycle 001 已交付 T8 主体 + T9 + T10 + T11（Plan Review 核对通过）；Cycle 002 执行 T12 + T13（含 T8 残留收口）。
- 五 Iteration 依赖边：000→001→002→003 线性，004 仅依赖 000（可并行于 001–003，建议序殿后），无环。

## Requirements Traceability Matrix

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| `sql-outer-join` R1 LEFT 语义（3 场景） | T1 | `tests/outer_join_test.rs`：left_join_matched/left_join_null_key_no_match/left_join_non_equi_on | Covered |
| `sql-outer-join` R2 RIGHT/FULL（2 场景） | T1 | right_join_unmatched_padded / full_join_both_sides_padded | Covered |
| `sql-outer-join` R3 组合面（3 场景） | T1 | where_filters_padded_rows / order_by_on_outer_result / in_subquery_join_still_rejected | Covered |
| `sql-outer-join` R4 零回归（2 场景 + 校准白名单） | T1 | `tests/nested_loop_join_test.rs` 8 测试零修改 + `where_over_non_equi_join_stays_rejected` 改写为 `where_over_non_equi_join_reachable`（可用性见证，白名单①）+ inner_join_plan_shape（planner_test）+ natural_join_rejected | Covered |
| `in-subquery-join-rejection` R1 MODIFIED（3 场景，WHERE+JOIN 文案统一） | T1 | `tests/subquery_test.rs`：单列/多列 JOIN 拒绝两用例零修改 + `test_in_subquery_where_join_keeps_existing_rejection` 期望文案校准为 `InSubqueryJoinUnsupported`（白名单②，拒绝保留） | Covered |
| `select-distinct` R1 去重（4 场景） | T2 | `tests/select_distinct_test.rs`：distinct_single_col（RED→GREEN）/distinct_multi_col/distinct_null_folds/distinct_expression_dedup | Covered |
| `select-distinct` R2 组合（2 场景） | T2 | distinct_order_by_output / distinct_order_by_non_output_rejected | Covered |
| `select-distinct` R3 拒绝与零回归（3 场景） | T2 | distinct_on_rejected / all_quantifier_identity / 既有套件零修改 | Covered |
| `sql-set-operations` R1 UNION（3 场景） | T3 | `tests/set_operations_test.rs`：union_dedup / union_all_preserves / union_null_folds | Covered |
| `sql-set-operations` R2 INTERSECT/EXCEPT（3 场景） | T3 | intersect_basic / except_basic / except_null_matching | Covered |
| `sql-set-operations` R3 结构（3 场景） | T3 | chained_three_way / intersect_precedence / intersect_all_and_parens_rejected | Covered |
| `sql-set-operations` R4 排序与零回归（3 场景） | T3 | compound_order_by / column_count_mismatch_rejected / 既有套件零修改 | Covered |
| `sql-aggregate-extensions` R1 COUNT DISTINCT（3 场景） | T4 | `tests/aggregate_extension_test.rs`：count_distinct_basic（RED→GREEN）/count_distinct_all_null_zero/count_distinct_multi_arg_rejected | Covered |
| `sql-aggregate-extensions` R2 SUM/AVG/MIN/MAX DISTINCT（4 场景） | T4 | sum_distinct / avg_distinct / min_distinct_eq_plain / distinct_with_group_by | Covered |
| `sql-aggregate-extensions` R3 group_concat（4 场景） | T4 | group_concat_default_sep / custom_sep / null_skip_and_all_null_group / non_literal_sep_rejected | Covered |
| `sql-aggregate-extensions` R4 零回归（2 场景） | T4 | 既有聚合套件零修改（group_by_expr/expression_e2e）+ 全量回归 | Covered |
| `sql-common-table-expressions` R1 可达（5 场景） | T5 | `tests/cte_test.rs`：basic_cte（RED→GREEN）/cte_shadows_base_table/multi_def_forward_ref/column_alias_list/reverse_ref_rejected | Covered |
| `sql-common-table-expressions` R2 引用一致（2 场景） | T5 | multi_ref_consistent_rows / two_scalar_refs_same_rows | Covered |
| `sql-common-table-expressions` R3 拒绝面（3 场景） | T5 | recursive_keyword_rejected / self_reference_rejected / with_in_subquery_rejected | Covered |
| `sql-common-table-expressions` R4 零回归（2 场景） | T5 | no_with_queries_unchanged / 全量回归 | Covered |
| `explain-plan-output` R1 计划树（4 场景） | T6 | `tests/explain_test.rs`：explain_simple_tree / explain_compound_nodes / explain_no_execution / explain_plan_error_not_exempt | Covered |
| `explain-plan-output` R2 拒绝面（2 场景） | T6 | explain_dml_rejected / explain_ddl_rejected | Covered |
| `explain-plan-output` R3 零回归（2 场景） | T6 | non_explain_unchanged / explain_repeatable（两次执行一致） | Covered |
| `order-by-alias-sorting` R1 别名真实排序（3 场景） | T7 | `tests/order_by_alias_sort_test.rs`：alias_sorts_expression（RED→GREEN）/alias_sorts_plain_desc/mixed_keys | Covered |
| `order-by-alias-sorting` R2 遮蔽（2 场景） | T7 | alias_shadows_source_column / input_column_key_unchanged | Covered |
| `order-by-alias-sorting` R3 校验与组合（3 场景） | T7 | unknown_key_rejected（既有）/ distinct_alias_sort_combo / 既有排序套件零校准（scalar_function_test R5/S5 白名单校准除外） | Covered |
| `error-reporting` REMOVED+ADDED「ORDER BY 排序键解析与别名排序」（3 场景） | T7 | 与 order-by-alias-sorting R1/R3 同源见证（unknown_key_rejected / alias_sorts_* / 既有零回归） | Covered |
| `sql-scalar-functions` MODIFIED R5（6 场景） | T7 | 5 场景既有测试零修改 + R5/S5 用例校准（白名单）+ `tests/scalar_function_test.rs` 全文件其余用例零修改 | Covered |
| `join-output-shape-resolution` R1 完整形状解析（2 场景） | T8 | `tests/join_output_resolution_test.rs`：agg_arg_outside_select_reachable（RED：静默 NULL）/ qualified_group_key_precise（全形状化保持见证） | Covered |
| `join-output-shape-resolution` R2 歧义拒绝（3 场景） | T8 | bare_group_by_ambiguous_rejected（RED：末位胜出错误分组）/ bare_agg_arg_ambiguous_rejected / bare_projection_ambiguous_rejected | Covered |
| `join-output-shape-resolution` R3 严格解析与别名列（3 场景） | T8 | unknown_column_named_rejected（RED：零宽行）/ aliased_plain_column_reachable（RED：零宽行）/ expression_item_join_still_rejected | Covered |
| `join-output-shape-resolution` R4 零回归（2 场景） | T8 | 既有 join/aggregate 套件零校准（白名单由 Cycle 调查定稿）+ 全量串行零回归 | Covered |
| `aggregate-output-shape-conformance` R1 形状一致（2 场景） | T9 | `tests/aggregate_output_shape_test.rs`：agg_only_group_by_key_not_selected（RED：表头 1 列行 2 值）/ select_key_order_differs_from_group_by（RED：置换错位） | Covered |
| `aggregate-output-shape-conformance` R2 绑定与零回归（4 场景） | T9 | existing_keyed_forms_unchanged / having_unselected_key_bindable / distinct_aggregate_output 校准（白名单，design D10）+ groupby_order_interleave_aligned_to_select_order 校准（白名单，Plan Review 2026-09-28 调查定稿；原名 groupby_order_interleave_keeps_direct_path 经 Plan Risk 条授权改名）+ 既有聚合套件零校准 + 全量串行零回归 | Covered |
| `error-reporting` MODIFIED R5「纯查询列别名表头生效」DerivedScan 基座扩展（2 新增场景） | T10 | `tests/derived_scan_alias_test.rs`：cte_plain_column_alias_name（delta cte_base_alias_header）/ derived_plain_column_alias_name（delta from_subquery_alias_header）+ derived_qualified_alias_name / derived_alias_header_is_alias + 既有三场景零修改 | Covered |
| `order-by-alias-sorting` R1 扩展场景「CTE 基座普通列别名排序」（T10） | T10 | `tests/derived_scan_alias_test.rs`：cte_alias_sort_desc（delta cte_base_alias_sorts）+ derived_expression_item_unchanged（表达式项别名既有行为见证） | Covered |
| `error-reporting` MODIFIED R6「列缺失错误分类一致」聚合参数扩展（4 新增场景） | T12 | `tests/join_output_resolution_test.rs`：unknown_bare_agg_arg_rejected（JOIN 面裸名，delta 场景 1）/ unknown_qualified_agg_arg_rejected（delta 场景 2）+ `tests/planner_test.rs`：unknown_agg_arg_single_table_rejected（delta 场景 1 单表面）/ alias_agg_arg_rejected（delta 场景 3 别名引用）+ 既有聚合零回归场景 | Covered |
| `error-reporting` MODIFIED R1「计划错误携带能力名」限定通配点名（1 新增场景） | T13 | `tests/planner_test.rs`：qualified_wildcard_capability_named（单表面 + JOIN 基座两断言，delta 场景） | Covered |

T11（信号注册时序硬化）不引入行为 requirement——既有 130/143 语义意图内的实现修正，无 corpus delta；见证与判定方式见 Task Contract（`tests/cli_test.rs` 信号 e2e 零修改 + 固定次数压测）。

Scenario→Task→Test 链路完整，无 Simplified、无 Missing。
