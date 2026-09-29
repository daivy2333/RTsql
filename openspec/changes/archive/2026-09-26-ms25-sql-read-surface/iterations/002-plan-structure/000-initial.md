# Iteration 002 / Cycle 000: 计划结构消费——CTE 与 EXPLAIN

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T5（CTE）、T6（EXPLAIN）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `sql-common-table-expressions` R1–R4、`explain-plan-output` R1–R3 全部场景；design D5（CTE 注册表 + DerivedScan 内联复用）、D6（EXPLAIN 包装节点 + 纯函数渲染）；proposal 决策记录（用户批准 2026-09-26）；change 级 Gate 2（2026-09-26）覆盖本 Iteration 计划；Iteration 000/001 已交付面（外连接、WHERE×JOIN D2 组合布局、Distinct/SetOperation 节点）作为组合场景输入
- Excluded scope: T7（Iteration 003）、T8/T9（Iteration 004）；递归 CTE（`RECURSIVE` 关键字与内容一律点名拒绝）；子查询内 WITH 解禁；跨引用共享物化（improvement 候选）；EXPLAIN ANALYZE/VERBOSE/FORMAT 语义；窗口函数；LATERAL

**Objective**

顶层 `WITH` 端到端可达（修复 `query.with` 静默丢弃缺陷：CTE 被丢弃 / 同名时静默按基表查询）；`EXPLAIN <SELECT>` 计划树文本可达且不执行内层语句（新增能力面）；DML/DDL/事务语句、递归形态、自引用、子查询内 WITH 等拒绝面计划期点名；`tests/cte_test.rs`（含 WITH 静默面 RED→GREEN 见证）+ `tests/explain_test.rs` 全绿，既有套件零校准，全量串行零回归。

**Investigation Facts**

- Current Baseline: Iteration 001 最终 Act Response（accepted Review 采信，2026-09-27）——DISTINCT/集合操作/聚合扩展落地，全量串行 1340 passed / 0 failed / 2 ignored（基线 1301 + 新增 39）；`planner_test` 87、`cli_test` 111/2i 零校准。本 Iteration 目标文件中 `src/executor/plan.rs`、`src/pipeline.rs`、`src/parser/planner/mod.rs` 自该验证后零变化（git 只读核实）；`query.rs` 最后改动为 Iteration 001（本 Review 已核对现态）。
- Current-State Evidence:
  - **WITH 现状**：`Query.with: Option<With>` 在 planner 全无消费——顶层 `WITH c AS (…) SELECT * FROM c` 报 `Table 'c' does not exist`（`validate_table`，mod.rs:238-247，文案 "Table '{}' does not exist"）；CTE 名与基表同名时**静默按基表查询**。sqlparser 0.44 形状（design 实测）：`With{recursive: bool, cte_tables: Vec<Cte>}`；`Cte{alias: TableAlias, query: Box<Query>}`；`TableAlias{name, columns: Vec<Ident>}`。
  - **PlanBuilder 生命周期与字段**（mod.rs）：每语句新建（`pipeline.rs:104` `PlanBuilder::new()`，DDL 例外走裸 builder :95——`Statement::Explain` 走 `_` 分支带表注册，正确）；既有字段 `tables: HashMap<String, Vec<String>>`（:103）、`building_subquery: bool`（:129）、`join_column_layout`（:134）；`register_table(&mut self, name, columns, pk)`（:153）；CTE 注册表为本 Iteration 加性字段，语句级生命周期天然隔离。
  - **基座解析臂**（query.rs `build_from_clause_with_projection`）：`TableFactor::Table` 臂（:281-291）——`object_name_to_table_name(name)` 归一化（去引号+lowercase，I039）→ `validate_table` → `self.tables.get` → `ScanNode`。**CTE 查找钩子插在 `validate_table` 之前**。`TableFactor::Derived` 臂（:292-320）为「子查询计划 → `get_plan_output_columns` → `register_table(alias, columns, "")` → `DerivedScanNode{subquery, alias, columns}`」先例，CTE 引用复用同构（差异：定义计划已构建，引用时克隆）。
  - **JOIN 右臂**（:352-359）：`extract_join_table_name`（ast.rs:312，Derived 拒绝）→ `validate_table` → `ScanNode`——CTE 钩子同位（`validate_table` 前）。
  - **子查询上下文置位点**：subquery.rs:36（WHERE IN/EXISTS）、subquery.rs:85（标量子查询）、query.rs:293（派生表臂）、query.rs:785（标量子查询项）。子查询内 WITH 拒绝门 = `build_query` 顶部 `query.with.is_some() && building_subquery`。
  - **body 分派序**（`build_query`，:533-549）：`SetExpr::Select` 放行 → `SetOperation` 走复合构建 → `Query` 括号化拒绝。**WITH 消费须在分派之前**（design D4：语句级 `query.with` 先于侧臂构建消费——`WITH c AS (…) SELECT … UNION SELECT …` 两臂引用 c 可达，Iteration 001 Deviation 11 的 `Table 'c' not found` 随本 Iteration 消除）。
  - **定义构建上下文裁定（D5「定义内 ORDER BY/LIMIT/集合操作经合成 Query 全量复用」的推论）**：CTE 定义计划以**顶层上下文**构建（`building_subquery` 保持 false）——子查询上下文会（a）拒绝定义内集合操作（`build_set_operation` 的 `building_subquery` 检查），（b）使定义内 DISTINCT 落入零消费静默面（Iteration 001 契约 Forbidden 项），（c）抑制定义内表达式项路由。顶层上下文下定义计划与普通顶层查询同构（可含 Projection/Distinct/SetOperation/Sort/Limit），引用点经 `DerivedScanNode` 消费（pipeline 构造期急切物化，`DerivedScanExecutor::new(rows)` 静态内存表）——任意计划形状均可消费，无形状约束冲突。定义内 DISTINCT 因此**被真实消费**（去重生效，无静默面）。
  - **DerivedScan 消费面**：`get_plan_output_columns(DerivedScan)` 返回 `node.columns`（注册列名）；多引用 = 定义计划逐引用克隆（`PhysicalPlan: Clone`），每引用独立急切物化——同一语句快照（`create_executor_from_plan` 每臂 `snapshot.clone()`）下行集跨引用一致；N 次引用 N 次求值（improvement 候选，非验收面）。
  - **EXPLAIN 现状**：`build_plan` 分派（mod.rs:194-245）无 `Statement::Explain` 臂——落入 `_ => UnsupportedStatement(unsupported_statement_name(stmt))`（:243）。sqlparser 0.44：`Statement::Explain{describe_alias: bool, analyze: bool, verbose: bool, statement: Box<Statement>, format: Option<…>}`（design 实测 registry）。plan cache：`is_cacheable` 仅 `Statement::Query`（pipeline.rs:1262-1264）——EXPLAIN 语句天然不缓存，每次重建。
  - **渲染面**：`PhysicalPlan` 现含 Distinct/SetOperation（Iteration 001）；渲染器为新 `src/executor/plan_format.rs` 纯函数，逐变体一行「节点名 + 关键参数」（表名/谓词文本/投影清单/JOIN 类型/排序键/LIMIT），覆盖全部变体含 000/001 新节点；`Response::QueryResult{rows}` 单列行集经 CLI 四格式渲染（既有通路，无新渲染面）。
  - **测试入口**：`tests/select_distinct_test.rs` 等 Iteration 001 文件形态（TempDir + `Database::open` + `query_rows`/`error_msg` helpers）；计划形状断言 `tests/planner_test.rs`（87）；CLI e2e `tests/cli_test.rs`（111/2i）。
- Code and Critical Path: T5——`PlanBuilder` 加性 `cte_plans: Vec<CteDef{name, columns, plan}>` + `pending_cte_names: Vec<String>`（mod.rs）；`build_query` 顶部 WITH 消费（拒绝门 + 逐定义构建 + 注册，query.rs :533 前）；基座臂与 JOIN 右臂查找钩子（query.rs :281-291/:352-359）；引用 = `DerivedScanNode` + `register_table`（遮蔽基表：注册表先于 `validate_table`）。T6——`build_plan` 增 `Statement::Explain` 臂（mod.rs :243 前）；`PhysicalPlan::Explain(ExplainNode{input})` + 新 `ExplainExecutor`（静态行迭代）+ 新 `plan_format.rs` 渲染器；`get_plan_output_columns` Explain 臂 → `["plan"]`；pipeline Explain 臂（**不构造内层执行器**——DerivedScan 急切物化不触发、零副作用）。

**Implementation Guidance**

实施顺序（TDD，建议 T6 先行——独立最小面先行，且渲染器先立可即时见证 000/001 全部计划形状；等价顺序不阻塞）：① `tests/explain_test.rs` 先行——全部用例现状 `UnsupportedStatement` RED；② `ExplainNode`/`ExplainExecutor`/`plan_format.rs` 渲染器/`build_plan` 臂/pipeline 臂/`get_plan_output_columns` 臂；③ `tests/cte_test.rs`——`basic_cte`（现状 RED：`Table 'c' does not exist`）、`cte_shadows_base_table`（现状 RED：静默按基表查询）等；④ `PlanBuilder` 加性字段 + WITH 消费段 + 两查找钩子 + 列别名校验；⑤ 定向套件 + 全量串行。

关键实施细节：

- WITH 消费段（伪序）：`if let Some(with) = &query.with` → `building_subquery` 置位则点名拒绝；`with.recursive` 点名拒绝（含非递归内容）；逐 `Cte`：`pending_cte_names.push(归一化名)` → 合成 `Query{body: cte.query.body 克隆, order_by: cte.query.order_by 克隆, limit: cte.query.limit 克隆, with: None, …}`（嵌套 WITH 随 `with: None` 拒绝，非静默——嵌套定义内 WITH 落 `TableNotFound` 或以定义内同名解析，实现时以「嵌套 WITH 点名拒绝」用例锁定具体文案）→ `build_query` 构建定义计划 → 列名 = `alias.columns` 非空时校验数量后采用（不符点名拒绝），否则 `get_plan_output_columns(定义计划)` → `pending_cte_names` 出栈 → 注册 `cte_plans`。
- 查找钩子共享 helper：`resolve_cte(name) -> Option<&CteDef>`——先查 `pending_cte_names`（命中 → 自引用/环点名拒绝），再查 `cte_plans`（命中 → 克隆定义计划构造 `DerivedScanNode` + `register_table(name, columns, "")`）；未命中 → 既有 `validate_table` 路径（反向引用经此报 `Table 'x' does not exist`）。名称归一化沿用 `object_name_to_table_name` 同款 lowercase 纪律。
- 渲染器逐变体一行，参数取舍（非实质留给 Act）：Scan/DataScan（表名+谓词有无）、Filter（谓词文本）、Join/NestedLoopJoin（类型+表名）、Aggregate（函数清单+分组键）、Sort（键）、Limit（值）、Distinct/SetOperation（op+all）、DerivedScan（别名）、Projection（列名）、Explain（"EXPLAIN" 根行）等。
- 零工作 doc 勘误（Iteration 001 Review 指派）：`src/parser/planner/query.rs:2058` `OuterJoinKind` doc 「`Right` 经左右交换重写后按 FULL 承载」→「按 `Left` 承载」（与 `:484-485` 勘误后注释及 `:490-494` 代码对齐）——本 change 遗留注释残留，零行为面，随本 Cycle 对 query.rs 的触碰顺带完成。

**Task Contracts**

### T5: 非递归 CTE（`sql-common-table-expressions` R1–R4）

- Requirement/Scenario: R1 可达五场景（basic_cte RED→GREEN / cte_shadows_base_table / multi_def_forward_ref / column_alias_list / reverse_ref_rejected）、R2 引用一致两场景（multi_ref_consistent_rows / two_scalar_refs_same_rows）、R3 拒绝面三场景（recursive_keyword_rejected / self_reference_rejected / with_in_subquery_rejected）、R4 零回归两场景（no_with_queries_unchanged / 全量回归）
- Depends on: None（集合操作/外连接/表达式项装配为既有复用面）
- Targets: `src/parser/planner/mod.rs::PlanBuilder`（加性 `cte_plans` + `pending_cte_names` 字段与注册/查找 helper）、`src/parser/planner/query.rs::build_query`（顶部 WITH 消费段，body 分派之前）、`src/parser/planner/query.rs::build_from_clause_with_projection`（基座臂 :281-291 与 JOIN 右臂 :352-359 的 `validate_table` 前 CTE 钩子）、`src/executor/plan.rs`（零改动——引用复用 `DerivedScanNode`）、`tests/cte_test.rs`（新）
- Current behavior: `query.with` 全无消费——`WITH c AS (…) SELECT * FROM c` 报 `Table 'c' does not exist`；CTE 名与基表同名时静默按基表查询；`WITH RECURSIVE` 同静默丢弃
- Required behavior: ① 顶层 `WITH c AS (…) SELECT …` 端到端可达：定义计划经合成 Query 构建（顶层上下文，定义内 ORDER BY/LIMIT/集合操作/表达式项/DISTINCT 全量复用既有装配），引用点克隆为 `DerivedScanNode`（急切物化）+ `register_table`（供 WHERE/ORDER BY 引用）；② 单/多定义与前向引用（后定义引用先定义，注册表 [0..i] 可见）；③ CTE 名遮蔽同名基表（注册表先于 `validate_table` 消费）；④ 列别名列表 `WITH c(a,b) AS (…)`：数量与定义输出不符点名拒绝，相符时引用列名 = 别名列表；⑤ 引用行集一致：同语句多引用（含自 JOIN 两侧引用与标量子查询引用）行集一致（同语句快照 + 逐引用克隆物化）；⑥ 拒绝面：`RECURSIVE` 关键字（含非递归内容）、定义内自引用/成环（`pending_cte_names` 命中）、反向引用（前定义引后定义 → 既有 `Table '…' does not exist`）、子查询内 WITH（`building_subquery` 置位 + `query.with` → 点名拒绝）、嵌套定义内 WITH 点名拒绝
- Preserve: 无 WITH 语句全路径逐字节不变（`query.with == None` 时零新逻辑，含 body 分派对 `SetExpr::Select` 的零变化）；CTE 未定义引用维持既有 `Table '…' does not exist` 文案；既有派生表臂、子查询计划形状、plan cache 键语义（SQL 文本自带 WITH）不变；M17/M19
- Forbidden: 不实现递归 CTE 求值；不实现跨引用共享物化；不解禁子查询内 WITH；不触碰 `validate_table` 既有文案与 `Derived` 臂既有行为；不修改既有测试断言（全部新用例）
- Test witness: `tests/cte_test.rs` 先于实现运行——`basic_cte` 现状 RED（`Table 'c' does not exist`）、`cte_shadows_base_table` 现状 RED（静默基表行）、`recursive_keyword_rejected`/`self_reference_rejected`/`with_in_subquery_rejected` 现状 RED（兜底文案不同），实现后 GREEN；`reverse_ref_rejected` 现状 GREEN（行为锁定）
- GREEN condition: 新测试全绿；`planner_test`(87)/`cli_test`(111)/`subquery_test`(32)/`set_operations_test`(14) 零校准通过；全量串行 0 failures
- Verification: ①定向 `cargo test --test cte_test --test planner_test`（退出码 0）；②全量 `cargo test -- --test-threads=1`（M19，退出码 0）；③`openspec validate 2026-09-26-ms25-sql-read-surface --strict`（退出码 0）
- Stop when: CTE 定义计划的顶层上下文构建与既有子查询机制产生形状冲突（如 DerivedScan 克隆消费不含某节点臂——返回 Plan）；WITH 消费点与 body 分派/`building_subquery` 快照的装配序冲突（返回 Plan）；需要修改既有测试断言才能通过（返回 Plan）

### T6: EXPLAIN 最小计划树（`explain-plan-output` R1–R3）

- Requirement/Scenario: R1 计划树四场景（explain_simple_tree / explain_compound_nodes〔含外连接/DISTINCT/集合操作节点渲染〕/ explain_no_execution / explain_plan_error_not_exempt）、R2 拒绝面两场景（explain_dml_rejected / explain_ddl_rejected）、R3 零回归两场景（non_explain_unchanged / explain_repeatable）
- Depends on: None（渲染 000/001 交付节点的臂为既有形状的只读文本投影）
- Targets: `src/parser/planner/mod.rs::build_plan`（`Statement::Explain` 臂，`_` 兜底臂之前）、`src/executor/plan.rs`（`PhysicalPlan::Explain` + `ExplainNode`）、`src/executor/plan_format.rs`（新，`render_plan_tree` 纯函数）、`src/executor/explain.rs`（新，`ExplainExecutor` 静态行迭代）+ `src/executor/mod.rs` 导出、`src/pipeline.rs`（Explain 构造臂——不构造内层执行器）、`src/parser/planner/query.rs::get_plan_output_columns`（Explain 臂 → `["plan"]`）、`tests/explain_test.rs`（新）
- Current behavior: `Statement::Explain` 落 `UnsupportedStatement(unsupported_statement_name(stmt))`（mod.rs:243）——EXPLAIN 全形态不可达
- Required behavior: ① `EXPLAIN <SELECT>` 构建内层计划后包 `Explain` 节点，产出单列（表头 `plan`）计划树文本（树形缩进，节点名 + 关键参数），**不执行内层语句**（pipeline Explain 臂不构造内层执行器——DerivedScan 急切物化与任何执行器副作用不触发）；② 内层计划期错误不豁免（`EXPLAIN SELECT * FROM missing` → 既有 `Table … does not exist`）；③ 渲染覆盖全部 `PhysicalPlan` 变体（含 Iteration 000/001 的 NestedLoopJoin/Distinct/SetOperation 与本 Iteration 无依赖的其他既有节点）；④ 拒绝面：`EXPLAIN <DML/DDL/事务语句>` 点名拒绝（「EXPLAIN supports SELECT statements only」语义）；`ANALYZE`/`VERBOSE`/`FORMAT` 修饰点名拒绝；`DESC`/`DESCRIBE`（describe_alias）同语义接受；⑤ 两次执行同一 EXPLAIN 输出一致（无缓存、每次重建，确定性渲染）
- Preserve: 非 EXPLAIN 语句全路径零变化（新臂仅 `Statement::Explain` 可达）；plan cache 语义不变（EXPLAIN 不入缓存，`is_cacheable` 既有判定零修改）；内层 SELECT 的计划形状与执行路径逐字节不变（Explain 节点仅包装不改造）；M17/M19
- Forbidden: 不实现 EXPLAIN ANALYZE（执行统计）；不做格式化选项；渲染文本不引入身份型证据机制（节点字段直出的确定性投影）；不修改既有测试断言（全部新用例）
- Test witness: `tests/explain_test.rs` 先于实现运行——全部用例现状 RED（`Unsupported statement type: EXPLAIN`），实现后 GREEN；`explain_no_execution` 以「内层 DML 表无行变化/内层 SELECT 侧副作用」断言见证不执行
- GREEN condition: 新测试全绿；`planner_test`(87)/`cli_test`(111) 零校准通过；全量串行 0 failures
- Verification: ①定向 `cargo test --test explain_test --test planner_test`（退出码 0）；②CLI e2e `cargo test --test explain_test --test cli_test`（EXPLAIN 四格式渲染）；③全量 `cargo test -- --test-threads=1`（M19，退出码 0）；④`openspec validate 2026-09-26-ms25-sql-read-surface --strict`（退出码 0）
- Stop when: 渲染器变体覆盖需要改既有节点结构（返回 Plan）；EXPLAIN 与 pipeline 事务路径（`execute_stage_in_tx`）交互产生副作用面（返回 Plan）

**Invariants**

- `query.with == None` 与非 EXPLAIN 语句路径零变化——本 Iteration 全部新逻辑挂在新语句/新子句形态之下。
- CTE 引用 = 既有 `DerivedScanNode` 消费机制复用，不引入新执行器/节点（D5）；引用计划为定义计划克隆，无跨语句状态（PlanBuilder 语句级生命周期）。
- EXPLAIN 是只读观测面：不执行内层、零副作用、确定性渲染；渲染文本为节点字段直出，非身份型证据机制。
- 拒绝面全部计划期点名（fail-fast），不静默降级。
- 输出行形状不变量：CTE 引用输出列名 = 定义输出列名（或别名列表），表头经 `get_plan_output_columns` 与行宽恒一致（I034 纪律）；EXPLAIN 输出恒单列 `plan`。

**Non-goals**

- 递归 CTE；子查询内 WITH 解禁；跨引用共享物化（improvement 候选）；EXPLAIN ANALYZE/VERBOSE/FORMAT 语义；CTE 内 DML（定义恒为 Query——`Statement::Explain` 同款拒绝语义）；T7/T8/T9。

**Acceptance**

`sql-common-table-expressions` R1–R4、`explain-plan-output` R1–R3 全部场景可观察满足（映射见 tasks.md RTM 与各契约 GREEN condition）；WITH 静默面修复有 RED→GREEN 见证；白名单外既有测试零校准；全量串行零回归。Scenario→Task→Test 链路：RTM `sql-common-table-expressions` 四行 + `explain-plan-output` 三行。

**Verification**

- CTE：`basic_cte`（RED→GREEN）/`cte_shadows_base_table`/`multi_def_forward_ref`/`column_alias_list`/`reverse_ref_rejected`/`multi_ref_consistent_rows`/`two_scalar_refs_same_rows`/`recursive_keyword_rejected`/`self_reference_rejected`/`with_in_subquery_rejected`/`no_with_queries_unchanged`。
- EXPLAIN：`explain_simple_tree`/`explain_compound_nodes`/`explain_no_execution`/`explain_plan_error_not_exempt`/`explain_dml_rejected`/`explain_ddl_rejected`/`non_explain_unchanged`/`explain_repeatable`。
- 全量：`cargo test -- --test-threads=1` 退出码 0（串行，M19）。
- 判定均为测试自身断言 + cargo 退出码；无身份型证据工程。

**Gate 2 Readiness**

PASS——无 Missing requirement（RTM 全 Covered）；调查完整（WITH/EXPLAIN 现状拒绝点、PlanBuilder 生命周期、基座/JOIN 右臂钩子位、DerivedScan 消费先例、body 分派序、子查询上下文置位点全部 file:line 实证，行号为 Iteration 001 后现态）；设计闭合（D5 注册表机制 + 定义构建上下文裁定〔定义内集合操作可达 ⇒ 顶层上下文〕、D6 渲染与拒绝面明确）；任务契约自包含（Act 无需回读外部资料）；验证直接判定（测试断言 + 退出码，无身份型证据工程）；无实质未知项。change 级 Gate 2（2026-09-26 用户批准）覆盖本 Iteration 计划；Iteration 000/001 新增面作为组合场景输入已纳入契约。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果；无一次性环境、无不可复现结论。

**Risks and Notes**

- CTE 定义以顶层上下文构建是 D5「定义内 ORDER BY/LIMIT/集合操作」要求的直接推论——若定义内 DISTINCT/表达式项因此可达而与引用点消费冲突，由 DerivedScan 急切物化的任意形状消费性兜底（调查已核实）；该裁定为 design 级结论，Act 无需再裁。
- CTE 名与保留名（`__tables`/`__columns`）同名：注册表先于 `validate_table`，CTE 可遮蔽保留系统表名——与「CTE 遮蔽基表」同规则，无需特判（系统表不经本 planner 消费路径注册进 `self.tables`）。
- `WITH` × 复合（Iteration 001 Deviation 11 的 `Table 'c' not found`）随本 Iteration 消除——`set_operations_test.rs` 无该形态断言，无需校准；`compound_arms_with_join_and_where` 等既有用例零修改。
- 嵌套定义内 WITH（`WITH a AS (WITH b AS … SELECT …) SELECT …`）按「子查询内 WITH 同款点名拒绝」处理，具体文案为非实质选择。
- EXPLAIN 渲染格式细节（缩进宽度、参数取舍）为非实质选择留给 Act；`explain_compound_nodes` 只断言节点名行存在与树形可达，不锁全文本逐字节。
- 零工作 doc 勘误（query.rs:2058，Iteration 001 Review 指派）随本 Cycle 完成——唯一允许的契约外触碰，纯注释零行为。
- 非实质选择留给 Act：`CteDef` 结构形态、渲染器内部组织、`ExplainExecutor` 文件位置（新文件或并入现有模块）。

## Act Response

- Status: reported

### 实施结果

T5 / T6 全部 Required behavior、Preserve、Forbidden 与 GREEN condition 已交付；两处静默面（`query.with` 无消费、CTE 名同名时静默按基表查询）具 RED→GREEN 见证，EXPLAIN 形态从全量拒绝转为可达（另拒绝面全点名、零静默降级）。

**产品代码（7 个文件改动 + 2 个新文件）**

- `src/executor/plan.rs` — `PhysicalPlan::Explain(ExplainNode{input})`（仅包装被观测计划）。
- `src/executor/plan_format.rs`（新）— `render_plan_tree` 纯函数：首行 `EXPLAIN`，其后逐节点一行（每层两空格缩进），覆盖 `PhysicalPlan` 全部 25 个变体（含计划期已点名拒绝的 DML/DDL 变体——新增变体时编译器强制补齐渲染臂）；参数为节点字段直出（表名、键有效字节十六进制、列清单、JOIN 类型/等值键、排序键、LIMIT 值、聚合清单）。
- `src/executor/explain.rs`（新）— `ExplainExecutor`：构造期一次性渲染，之后每行产出一个单值行；**不持有内层执行器**（内层计划从不执行）。
- `src/executor/mod.rs` — 两新模块与 `ExplainNode` / `ExplainExecutor` 导出。
- `src/executor/correlated.rs` — `PhysicalPlan::Explain` 臂（匹配穷尽性；被观测计划不参与参数注入）。
- `src/parser/ast.rs` — `table_factor_alias`（CTE 引用按引用处别名取名）。
- `src/parser/planner/mod.rs` — ① `build_plan` 增 `Statement::Explain` 臂（`ANALYZE` / `VERBOSE` / `FORMAT` 修饰逐项点名拒绝；内层非 `Statement::Query` → 点名拒绝；`describe_alias` 的 `DESC` / `DESCRIBE` 同语义接受）；② `PlanBuilder` 加性 `cte_plans: Vec<CteDef>` + `pending_cte_names: Vec<String>`（语句级生命周期，无跨语句状态）。
- `src/parser/planner/query.rs` — ① `build_query` 顶部 WITH 消费段（`consume_with`：子查询内 WITH / `RECURSIVE` / 嵌套定义内 WITH 点名拒绝；逐定义以顶层上下文构建 + `pending_cte_names` push/pop 严格配对；列别名列表数量校验）；② `resolve_cte_reference` 查找钩子（FROM 基座臂 + JOIN 右臂各一，置于 `validate_table` 之前 → 遮蔽基表；引用处别名或 CTE 名为注册名；引用处列别名列表点名拒绝）；③ `get_plan_output_columns` 增 Explain 臂（`["plan"]`）；④ 零工作 doc 勘误（`OuterJoinKind`「`Right` … 按 FULL 承载」→「按 `Left` 承载」，Iteration 001 Review 指派）。
- `src/pipeline.rs` — ① `create_executor_from_plan` 增 Explain 臂（**不构造内层执行器**）；② `extract_all_table_names` 增 `Statement::Explain` 递归臂；③ `extract_all_query_table_names` 抽 `extract_query_body_table_names` 并增 WITH 感知（定义体贡献自身表名；WITH 定义名按「逐定义可见序」遮蔽同名基表，不进目录查找）。
- `src/executor/plan_format.rs` 单测 2 + `src/executor/explain.rs` 单测 1。

**测试（2 个新文件，24 用例，零校准既有套件）**

- `tests/cte_test.rs`（14）：`basic_cte`(RED→GREEN，含 plan cache 命中路径二次执行)/`cte_with_compound_query`/`cte_shadows_base_table`(RED→GREEN)/`multi_def_forward_ref`/`column_alias_list`(+ 列数不符拒绝)/`column_alias_list_headers_via_cli`（CLI 真表头 n、s）/`reverse_ref_rejected`/`multi_ref_consistent_rows`/`two_scalar_refs_same_rows`/`recursive_keyword_rejected`/`self_reference_rejected`/`with_in_subquery_rejected`/`nested_with_and_reference_alias_list_rejected`/`no_with_queries_unchanged`。
- `tests/explain_test.rs`（10）：`explain_simple_tree`/`explain_compound_nodes`（JOIN + Filter + Sort + Limit，另含外连接 NestedLoopJoin[Left]、Distinct、SetOperation[UNION] 渲染臂与树形缩进断言）/`explain_no_execution`（无数据行输出 + 事后同库数据完好）/`explain_plan_error_not_exempt`/`explain_dml_rejected`/`explain_ddl_rejected`（含零副作用：t2 未被创建）/`explain_modifiers_rejected_and_desc_accepted`（ANALYZE/VERBOSE/FORMAT 拒绝 + 嵌套 EXPLAIN 拒绝 + `DESC` 输出与 `EXPLAIN` 逐行相同）/`non_explain_unchanged`（含 `explain` 作列名）/`explain_repeatable`/`explain_cli_four_formats`（json `columns == ["plan"]`、行宽恒 1，table/csv/tsv 表头 `plan`）。

- 部分完成：None。
- 未开始：None（T7 属 Iteration 003，T8/T9 属 Iteration 004）。

### Deviations from Plan

1. **`extract_all_table_names` / `extract_all_query_table_names` 扩面**（Targets 未列这两个函数，但契约的「planner 侧引用点 + 计划期错误不豁免」必要）：调查事实「`Statement::Explain` 走 `_` 分支带表注册，正确」在**目录名提取层不成立**——`extract_all_table_names` 对 `Explain` 返回空、对 `Query` 不进入 `with` 定义体，故 `EXPLAIN SELECT … FROM t` 与全部 WITH 语句在 planner 之前即报 `Table '…' not found`（RED 实测）。修复为两个加性臂（Explain 递归 + WITH 感知过滤）；无 WITH、无 EXPLAIN 语句的提取结果逐值等价。
2. **R2-S2 见证改用可达等价形态**（带外层 `FROM c`）：delta spec `sql-common-table-expressions` R2-S2 的示例 SQL 无外层 FROM，而 SELECT 项中的标量子查询在无 FROM 形态下被既有 MS13 T9 拒绝面点名（实测 `SELECT (SELECT COUNT(*) FROM t) AS n1` 在本 change 前即报 `Unsupported expression type: EXPRESSION`，与 CTE 无关）。语义等价见证：两标量引用同一 CTE 读出同一行集（`(2,2)`）。属 spec 文本可达性问题，见下方 Spec 文本发现 1。
3. **引用处列别名列表点名拒绝**（契约 Required behavior 未列该形态）：`FROM cte AS c2(x)` 若照既有基表行为静默忽略会产出与用户书写不符的列名——按「拒绝面全部计划期点名（fail-fast），不静默降级」不变量改为拒绝并指向声明面 `WITH cte(cols) AS (…)`。非实质选择（拒绝文案为 Act 权限内）。
4. **嵌套定义内 WITH 点名拒绝**（Required behavior ⑥ 的一项，delta spec R3 未单列场景）：实现 + `nested_with_and_reference_alias_list_rejected` 见证。
5. **`ast.rs` 增 `table_factor_alias`**：JOIN 右臂需引用处别名，既有 `extract_join_table_name` 只返回表名（加性 helper，未改既有签名/行为）。
6. **`correlated.rs` 增 Explain 臂**：`PhysicalPlan` 匹配穷尽性要求（编译器拒绝缺臂）；语义为无操作（被观测计划不执行）。
7. **EXPLAIN 拒绝文案与承载层**：非 SELECT 族、嵌套 EXPLAIN、修饰面统一走既有 `PlanError::UnsupportedStatement`（载荷即被拒能力名，SQLSTATE 0A000，MS19 错误面零变更）。实测 `EXPLAIN EXPLAIN …` 由 sqlparser 在解析期点名拒绝（`Explain must be root of the plan`），planner 臂不可达——R2「嵌套 EXPLAIN 点名拒绝」由解析层收敛（见 Spec 文本发现 2）。
8. **渲染格式为非实质选择**（契约明文留给 Act）：谓词 / 参数化表达式无 `Display`，按 `Debug` 直出（如 `ComparisonPredicate { … }`），未新增 Display 实现（避免 10 类谓词 + 表达式族的展示层）；键按有效字节十六进制；`IndexScan` 直出构造期已收窄的 `columns`（I034 口径），`IndexScanAll` 按 `projection` 裁剪（与 `get_plan_output_columns` 同口径）。`explain_compound_nodes` 只断言节点名与关键参数存在，不锁全文本。
9. **`ExplainExecutor` 落在 `src/executor/explain.rs` 新文件**（契约明文留给 Act 的非实质选择；与 `distinct.rs` / `set_operation.rs` 同型）。
10. **零工作 doc 勘误**（Iteration 001 Review 指派，唯一允许的契约外触碰）：`src/parser/planner/query.rs` `OuterJoinKind` doc 一行，纯注释零行为。

### 自检发现（Gate 3）

- **已修复（实施期，5 项）**：① 渲染器初版对 `IndexScan` 施加 `projection` 裁剪 → 越界 panic（`len is 1 but the index is 1`）——I034 口径下 `IndexScan.columns` 已收窄、索引指向基表 schema，改为直出；② 连带修正 `IndexScanAll` 的形状口径（`columns` 为基表清单，须按 `projection` 裁剪）；③ 两条「因错误原因通过/失败」的测试假设——`explain_simple_tree` 原断言 `Filter` 节点，但 `WHERE id = 1` 走既有 PK 等值路由（谓词以扫描节点键参数呈现），改断言键参数；`with_in_subquery_rejected` 与 `self_reference_rejected` 在 RED 阶段因旧的全量丢弃/表缺失错误而通过，GREEN 后改断言点名文案（`nested WITH` / `column alias list` / 自引用文案）；④ 新单测的 `clippy::bool_assert_comparison`；⑤ `unused_mut` 告警（新测试夹具）。
- **已修复（格式/静态）**：`rustfmt --edition 2021 --config skip_children=true` 仅施加于本 Cycle 触及文件（`--check` 先行确认差异位置全在本 Cycle 新增代码内，未触碰 Iteration 000/001 未提交内容）；`cargo clippy --all-targets` 0 warning。
- **遗留 Minor 1**：谓词 / 表达式渲染为 `Debug` 投影（可读性一般，但为节点字段直出的确定性文本；新增 Display 层属计划外展示层投入）。EXPLAIN 文本的「人读友好度」在 MS22 性能批之外无排期。
- **遗留 Minor 2**：`FROM <基表> AS alias(x, y)`（基表引用处列别名列表）仍按既有行为静默忽略（基表路径未改，契约 Forbidden「不触碰既有派生表臂/既有行为」）；CTE 引用处同名形态为点名拒绝。两者不对称，基表侧属既有行为面。
- **遗留 Minor 3**：CTE 引用出现在 IN / EXISTS 子查询内时命中既有形状缺口——`get_subquery_first_column`（`subquery.rs:386+`）无 `DerivedScan` 臂，报 `Subquery returns multiple columns`。该缺口对派生表**同样存在且为变更前既有**（实测 `… WHERE v IN (SELECT v FROM (SELECT v FROM t) d)` 变更前后同文案），且是点名拒绝非静默错值；契约 R2 只列自 JOIN 两侧引用与标量子查询引用两种形态，本 Cycle 不扩张该面。
- **遗留 Minor 4**：`EXPLAIN` 走 `execute_stage` 的查询路径（`_` 臂）会调用 `database.statement_snapshot(None)`——默认 RR 下恒返回 `None`（`database.rs:149-161`）；RC 下构造一个语句快照但 Explain 执行器不消费任何行流，无数据读副作用。契约 Stop-when 的「与 `execute_stage_in_tx` 交互」已核实：会话事务内 EXPLAIN 同样只落 `_` 臂，无副作用面。

### Spec / design 文本发现（Act 只报告）

1. **`sql-common-table-expressions` R2-S2 的示例 SQL 在本引擎不可达**：无外层 FROM 的 SELECT 项标量子查询被既有 MS13 T9 拒绝面拦截（`Unsupported expression type: EXPRESSION`，与 CTE 无关，变更前即如此）。R2 的 requirement 文本（「各引用 SHALL 面对同一语句级数据视图……行集 SHALL 一致」）已由等价可达形态见证。建议 Plan 在合并语料时把场景 SQL 补上外层 FROM，或加已知边界注记（同 Iteration 001 对聚合夹具的处理先例）。
2. **`explain-plan-output` R2「嵌套 `EXPLAIN EXPLAIN` SHALL 点名拒绝」的承载层是 sqlparser 解析期**（`Explain must be root of the plan`），planner 的非 SELECT 族拒绝臂不可达该形态；requirement 语义满足（点名、非静默），但层次与文字暗示不同，建议语料合并时明确。
3. **`sql-common-table-expressions` R1「CTE 名 SHALL 遮蔽同目录下同名基表」**的实现面比 requirement 文字更宽：遮蔽范围是「整条语句的目录查找」——目录名提取按「逐定义可见序」过滤，定义体内引用同名的后定义 CTE 仍解析为基表（标准 SQL 可见性规则），主查询体则被 CTE 名遮蔽。文字无需修订，行为与文字不冲突。

### 验证

- RED 见证（T6，planner 未改）：`cargo test --test explain_test` → `1 passed; 9 failed`，失败即全形态不可达：`Unsupported statement type: EXPLAIN`（0A000）。退出码非 0。
- RED 见证（T5，planner 未改）：`cargo test --test cte_test` → `2 passed; 10 failed`，失败面：`Table 'cte' not found`（含 CTE 名遮蔽基表的静默面——现状返回基表行 9）。退出码非 0。
- GREEN（定向）：`cargo test --test cte_test --test explain_test` → `14 passed` / `10 passed`，0 failed，退出码 0。
- 单测（新增渲染器 + 执行器）：`cargo test --lib plan_format` → `2 passed`，退出码 0（`explain` 执行器单测在全量串行内同绿）。
- 零校准见证（受影响边界）：逐 suite `cargo test --test <name>` → `planner_test` 87 / `cli_test` 111 passed + 2 ignored / `subquery_test` 32 / `set_operations_test` 14 / `nested_loop_join_test` 9 / `select_distinct_test` 12 / `aggregate_extension_test` 13 / `outer_join_test` 16 / `aggregate_test` 13 / `projection_expression_test` 19，全部 0 failed，退出码 0（与契约 GREEN condition 声明的 `planner_test`(87)/`cli_test`(111)/`subquery_test`(32)/`set_operations_test`(14) 逐值一致，白名单外零校准）。
- 全量（M19 串行）：`cargo test -- --test-threads=1` → 各 test binary 均 `0 failed`，退出码 0；测试总数 `cargo test -- --list` 计数 1369（基线 1342 + 本 change 新增 27 = cte_test 14 + explain_test 10 + lib 单测 3），其中 2 ignored。
- `cargo clippy --all-targets` → 0 warning，退出码 0。
- `rustfmt --edition 2021 --check --config skip_children=true`（本 Cycle 触及的 11 个文件）→ 0 差异，退出码 0。
- `openspec validate 2026-09-26-ms25-sql-read-surface --strict` → `Change '2026-09-26-ms25-sql-read-surface' is valid`，退出码 0。
- 覆盖说明：全量串行在最后一次渲染面微调（`IndexScanAll` 形状口径，纯 EXPLAIN 文本面）之前运行；该改动后重跑 `explain_test`（10 passed）+ `plan_format` 单测（2 passed），其余测试面代码自全量运行后未变化（`git status` 核实改动面仅 `plan_format.rs`），故采信全量结论。

### Persisted Evidence

None required——全部结论由测试断言与命令退出码直接判定，可低成本重跑；无一次性环境、无不可复现结论。

### Experience Candidates

- Runbook：None。
- Issue 候选：None。核查过的三项候选面（Minor 2 基表引用处列别名静默忽略、Minor 3 IN 子查询 × `DerivedScan` 形状缺口、RC 下 EXPLAIN 构造语句快照）均为既有面且表现为点名拒绝或零数据副作用，不满足「实质缺陷（静默错误结果 / 正确性红线）」的 Issue 准入条件；留档于自检发现。

### 未解决问题

- Spec 文本发现 1–3 待 Plan 在合并语料时处置（场景 SQL 可达性注记、拒绝承载层文字、实现面与 requirement 文字的一致性）。
- 遗留 Minor 1–4 留档，均非阻塞。
- 本 Iteration（T5/T6）Acceptance 满足，等待 Plan 独立 Review 裁定 `Review Result`；Iteration 002 的 Iteration 状态与 `Review Result` 依流程由 Plan Review 更新（change `tasks.md` 无 per-task 状态字段，本 Cycle 无可更新的全局状态）。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读实际代码与测试核对 Act Response 全部声明，逐面一致，无阻塞 finding。偏差分类如下：

1. **[Minor · Act Response 文档数字误差]** Act Response 称渲染器覆盖「`PhysicalPlan` 全部 25 个变体」——枚举实为 **26** 个变体（plan.rs:18-75：Scan/DataScan/IndexScan/IndexScanAll/Filter/Insert/Upsert/Update/Delete/CreateTable/DropTable/Sort/Limit/Join/NestedLoopJoin/Aggregate/Having/SemiJoin/AntiJoin/SubqueryEval/DerivedScan/Projection/Distinct/SetOperation/Explain/SingleRow），渲染器 26 臂全覆盖（plan_format.rs:26-74 递归臂 15 + 叶臂 11，`describe` :78-255 同集）。代码完整无缺，仅 Response 数字误差，零行为面。
2. **[Plan 遗漏 · Act 正确补齐 · 非阻塞]** Deviation 1 属实且必要：Investigation Facts「`Statement::Explain` 走 `_` 分支带表注册，正确」在目录名提取层不成立——`extract_all_table_names` 对 Explain 返回空、对 Query 不进 `with` 定义体（pipeline.rs:1067-1089 现态），目录注册在 planner 之前失败。Act 的两个加性臂（Explain 递归 :1072 + `extract_all_query_table_names` WITH 可见性过滤 :1098-1114）修复正确；核实既有臂零触碰、新逻辑仅挂 `Statement::Explain` 与 `query.with` 存在形态之下（无 WITH/无 EXPLAIN 语句提取结果逐值等价），WITH 可见性过滤与 planner 遮蔽语义一致（逐定义可见序，即 Spec 发现 3 的标准 SQL 规则）。
3. **[Delta spec 文本问题 · 等价见证成立]** Deviation 2 / Spec 发现 1 核实：`sql-common-table-expressions` R2-S2 示例 SQL 无外层 FROM，被既有 MS13 T9 拒绝面拦截（与 CTE 无关、变更前即如此）；等价见证 `two_scalar_refs_same_rows`（tests/cte_test.rs:215，两标量引用读同一行集 (2,2)）满足 requirement 语义。语料合并时处置（见 Follow-up）。
4. **[契约不变量推论 · 非实质]** Deviation 3 核实：引用处列别名列表点名拒绝（query.rs:812-817 + tests/cte_test.rs:313-320）符合不变量「拒绝面全部计划期点名，不静默降级」，拒绝文案属 Act 权限。
5. **[承载层事实 · 语料合并事项]** Deviation 7 / Spec 发现 2 核实：`EXPLAIN EXPLAIN` 由 sqlparser 解析期点名拒绝，planner 臂不可达；测试断言用宽松 needle（tests/explain_test.rs:279-285，contains "explain"）与承载层事实一致。requirement 语义满足（点名、非静默），合并语料时明确承载层文字。
6. **[Act 自检遗留 Minor 1–4 · 非阻塞]** 核实 Issue 准入判断正确：Minor 2（基表引用处列别名静默忽略——既有行为面，契约 Forbidden 保护）、Minor 3（IN 子查询 × DerivedScan 形状缺口——**变更前既有**且为点名拒绝非静默错值）、Minor 4（RC 下 EXPLAIN 构造语句快照——零数据读副作用）均不满足实质缺陷准入（静默错误结果/正确性红线）；Minor 1（谓词/表达式 Debug 投影可读性）为非实质展示层。均留档，不落账 Issue、不建 Runbook。
7. **[实现面抽查核实]** WITH 消费段（query.rs:735-845）：子查询内/RECURSIVE/嵌套定义内 WITH 三拒绝、`pending_cte_names` push/pop 严格配对（错误路径 pop 先于 `?` :756-758）、列别名数量校验 :761-777；`resolve_cte_reference`（:791-831）自引用拒绝 + 引用处别名列表拒绝 + 遮蔽基表（基座臂 :285 与 JOIN 右臂 :365 均置 `validate_table` 前）；pipeline Explain 臂不构造内层执行器（:889-893）+ `ExplainExecutor` 静态行迭代（explain.rs，构造期渲染一次）；`build_plan` Explain 臂修饰面/非 SELECT 族拒绝（mod.rs:254-285）；`get_plan_output_columns` Explain 臂 `["plan"]`（query.rs:130）；`correlated.rs` Explain no-op 臂（:101）；`ast.rs::table_factor_alias`（:320）；零工作 doc 勘误到位（query.rs:2195「按 `Left` 承载」）。测试 14+10 用例断言实质（行值、点名文案、零副作用、plan cache 二次执行、四格式表头、树形缩进）与声明一致；测试数 14+10+3 单测 = 27，与 `--list` 计数 1369（基线 1342）自洽。

**Acceptance Gaps**

None——`sql-common-table-expressions` R1–R4 与 `explain-plan-output` R1–R3 全部场景有测试见证；两处静默面（`query.with` 无消费、CTE 名同名静默按基表查询）RED→GREEN 见证成立（Act 验证段：RED `cte_test` 2 passed / 10 failed，含静默基表行 9；GREEN 14 passed）；白名单外零校准（契约点名四套件逐值一致 + 更宽边界套件）；全量串行 0 failures。

**Evidence**

- 独立阅读：query.rs（WITH 消费/钩子/Explain 列臂/doc 勘误）、mod.rs（Explain 臂/CTE 字段）、pipeline.rs（Explain 臂/提取层）、plan.rs（ExplainNode/变体枚举）、plan_format.rs（全文）、explain.rs（全文）、correlated.rs、ast.rs、tests/cte_test.rs + tests/explain_test.rs（关键断言抽查 8 处）、executor/mod.rs 导出。
- 验证采信来源：Act Response「验证」段（2026-09-27 新鲜运行）——全量串行 0 failed、`--list` 1369（2 ignored）、定向 14+10 passed、零校准套件逐值、clippy 0 warning、`openspec validate --strict` 通过。最后一次渲染面微调（`IndexScanAll` 文本口径）后定向补跑 `explain_test` + `plan_format` 单测；本 Review 独立核实 `render_plan_tree` 消费面仅 EXPLAIN 路径（explain.rs:22 + plan_format 自身单测），补跑覆盖完整，其余测试面代码自全量运行后未变化——采信条件成立，不触发重跑。

**Follow-up Decision**

接受（accepted）：Acceptance 全部满足、无阻塞 finding；全部偏差为非实质（文档数字误差、Plan 遗漏已由 Act 正确补齐、spec 文本可达性）或契约内推论。**语料合并待办（change 收尾时由 Plan/Maintainer 执行）**：① `sql-common-table-expressions` R2-S2 场景 SQL 补外层 FROM 或加已知边界注记；② `explain-plan-output` R2 嵌套 EXPLAIN 承载层文字明确（sqlparser 解析期）；③ `sql-common-table-expressions` R1 遮蔽范围实现面注记（整条语句目录查找、逐定义可见序——行为与文字不冲突）。Iteration 002 完成——展开 Iteration 003（`003-order-by-alias`）。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-09-26-ms25-sql-read-surface/iterations/003-order-by-alias/000-initial.md`（已按 Iteration Map 展开，Status: ready——change 级计划 D8 与 T7 契约已于 2026-09-26 Gate 2 获用户批准；Current Baseline 引用本 Iteration 最终 Act Response 与本 accepted Review）。
