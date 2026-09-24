# Iteration 001 / Cycle 000: NLJ 与启发式切换（T10-T14）

## Plan Context

- Status: ready
- Iteration: 001-nlj
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T10-T14（tasks.md Iteration 001「NLJ 与启发式切换」）
- Depends on: None（tasks.md Iteration Plan：与 000 无代码耦合，按执行序后行；快照穿线面由 Iteration 000 提供，本 Iteration 不消费）
- Stable baseline: 纯等值 ON Hash 形状与结果逐字节保持；非等值/混合/字面量腿 ON 经 NLJ 产出语义连接结果；计划期 `Unsupported expression type` 拒绝面被该能力取代；计划期启发式可经 plan 形状断言；默认配置全量零回归
- Verification boundary: `tests/nested_loop_join_test.rs` 全绿 + `tests/join_test` 保持 + 全量回归零修改 + clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: planner join 分类（`ddl_dml.rs`/`query.rs`）、NLJ 执行器（新 `nested_loop_join.rs`）、pipeline 构造臂与注册面（`pipeline.rs`/`correlated.rs`/`query.rs` 匹配点）
- Deferred tasks: T20-T22（Iteration 002 关联子查询结果缓存）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: join-executor-selection 全部 Requirement（R1-R5）；design D5 + D5a 处方；Iteration 000 的全量绿面作为回归基线（本 Iteration 不触碰事务可见性域）
- Excluded scope: WHERE + JOIN（两种 join 节点，维持拒绝）；SELECT 表达式项 + JOIN（维持拒绝）；LEFT/RIGHT/FULL JOIN；SMJ；代价模型与运行时统计；算术表达式（`build_expression` 无算术臂，维持既有拒绝）；3+ 表链中 NLJ 左输入为投影后 Join 的形状错位（预存 Hash 同源边界）；关联子查询缓存（T20-T22）

**Objective**

非等值/混合/字面量腿 ON 从计划期 `Unsupported expression type` 拒绝改为经新增 `NestedLoopJoin` 计划节点与执行器产出正确 INNER 连接结果（三值语义、空输入、NULL 排除、ORDER BY、关联 ON 注入全部可达）；纯等值 ON 的 Hash 路径计划形状与结果逐字节保持；Hash/NLJ 选择为计划期结构启发式（可 plan 形状断言）；既有全量测试零修改通过。

**Background**

需求来源：tasks.md MS09-T02（I015 NLJ 部分）+ 用户裁定 2026-09-13「非等值 JOIN 一并解锁」（proposal 决策记录 2）。现状：`extract_join_conditions`（`ddl_dml.rs:29-76`）仅接受 AND 组合的列=列等值腿，其余计划期 `PlanError::UnsupportedExpression`（"Unsupported expression type"，`error.rs:17/:67`）响亮拒绝——非等值 JOIN 不可达。本 Cycle 是 Iteration 001 的首个执行 Cycle，按 design D5 + D5a（2026-09-14 Plan 调查闭合）实施。

**Investigation Facts**

- Current Baseline: HEAD `e51c4a3` + 工作区（40 个已跟踪修改文件——Iteration 000 三轮 Cycle 实施与校准 + docs；代码面自 002-rework 收尾后未变化，最新 src/tests mtime 2026-09-14 11:51 均为 002-rework 文件）。全量 **919 passed / 0 failed / 2 ignored**（002-rework Act 运行 + Plan Review 独立复跑决定性套件，采信）；`cargo test --test join_test` **7 passed / 0 failed**（本会话新鲜复跑）。
- Current Baseline probes（本会话新鲜运行，CLI one-shot）：
  - 非等值 ON `SELECT r.a, s.b FROM r JOIN s ON r.a < s.b` → `statement 1 of 1 failed: Plan error: Unsupported expression type`，exit 3。
  - 字面量腿 `SELECT r.a FROM r JOIN s ON r.a = 5` → 同上（`resolve_column_ref` 对 Value 侧失败，`expression.rs:77`）。
  - WHERE + JOIN `... ON r.a = s.b WHERE r.a > 0` → `Plan error: Unsupported statement type`，exit 3（预存拒绝面，本 Iteration 维持）。
- Current-State Evidence（Plan 直接读码核实，file:line 为当前工作区现状）：
  - **ON 提取**：`extract_join_conditions`（`ddl_dml.rs:29-76`）——AND 递归分解；单腿要求 `BinaryOp::Eq` 且两侧经 `resolve_column_ref` 解析为列引用（左腿对 `left_tables`、右腿对 `[right_table]`，反序交换）；否则 `:74` `Err(PlanError::UnsupportedExpression)`。JOIN 类型面：`query.rs:171-175` 仅 `Inner(On)`，其余 `UnsupportedJoinType`。
  - **Join 构建**：`build_from_clause_with_projection`（`query.rs:105-260`）——右表仅支持 `TableFactor::Table`（`extract_join_table_name`，`:178`）；conditions 经 `:189-190` 提取；`output_columns` = `current_tables` 全列 ++ 右表全列按 `qualified_columns` 过滤（`:193-246`，`SELECT *` → 全列）；节点组装 `:249-254`；`current_tables.push(right_table)`（`:256`）。
  - **Join 下游匹配点**：`query.rs:345`（base_plan 为 Join 时 table_name="join_result"）；`:424`（SELECT 表达式项 + JOIN 显式拒绝）；`:507-509`（WHERE + JOIN 显式拒绝——probe3 实证文案 "Unsupported statement type"）；`query.rs:1271`（`get_plan_output_columns` Join 臂单测）。`get_plan_output_columns` Join 臂 `:77-84`（output_columns 列名，不递归）。
  - **计划节点**：`PhysicalPlan` 枚举 20 变体（`plan.rs:18-59`）；`JoinNode { left, right, conditions: Vec<JoinCondition>, output_columns: Vec<OutputColumn> }`（`plan.rs:315-324`）；`OutputColumn { table, column, table_alias, column_index }`（`plan.rs:301-311`）；`FilterNode { input, predicate: PredicateRef, table_name, projection }`（`plan.rs:123-133`）。`PhysicalPlan` derive `Debug, Clone`。
  - **谓词机制**：`PredicateRef = Arc<dyn Predicate>`（`predicate.rs:47`）；trait `evaluate`/`evaluate_ternary`/`inject_parameters`（`predicate.rs:30-43`）。`build_where`（`expression.rs:435-586`）：AND/OR/比较/IN/BETWEEN/LIKE/IS NULL/NOT/Nested 臂齐备（MS11-T01 面）；`build_expression`（`expression.rs:170-432`）：Identifier/CompoundIdentifier 经 `self.tables[table_name]` 解析为**表内索引**的 `ColumnExpression`；CompoundIdentifier 先经 `inner_table_names` 关联检查产生 `ParameterExpression`（`:206-214`）；**无算术 BinaryOp 臂**（`:430` fallback UnsupportedExpression——delta spec R2-S2 原算术场景不可编译，已按 D5a 修订场景）。
  - **执行器**：`JoinExecutor`（`join.rs:19-201`）纯等值 Hash——NULL 键不匹配（`:84-87`/:106-109）；`build_output_row` 按 `col.table_alias == left_table_name` 取 `left_row[col.column_index]` 否则 `right_row[col.column_index]`（`:113-124`）；三阶段 Volcano。`FilterExecutor` 求值形态：`predicate.evaluate(&values)` → Ok(true) 产出 / Ok(false) 跳过 / Err 传播（`filter.rs:43-58`）——`evaluate()` 为三值 fold（Unknown→false），INNER 语义下与 `evaluate_ternary` 判 `Ternary::True` 可观察等价。`Executor` trait（`executor_trait.rs:10-14`）`async fn next() -> Result<Option<ExecResult>>`；`ExecResult::Row(Vec<Value>)`（`result.rs:14`）。
  - **pipeline 构造**：`create_executor_from_plan` Join 臂（`pipeline.rs:617-644`）——`extract_column_indices` 取左右（索引映射 + 表名）→ 递归构造左右执行器 → `JoinExecutor::new(JoinConfig{..})`。`extract_column_indices`（`pipeline.rs:784+`）：Scan/DataScan → 列名 lowercase→idx + table_name；Join/SemiJoin/AntiJoin → output_columns 索引 + 首条件左表名（`:805-852`）；其余递归/直取。
  - **关联注入**：`inject_correlated_values`（`correlated.rs:14-80`）——Join 臂递归左右（`:52-55`）；DataScan 下推谓词 `predicate.inject_parameters`（`:36-42`）；谓词内 `ParameterExpression` 经注入求值。
  - **PlanBuilder**（`mod.rs:99-117`）：`tables`/`primary_keys`/`primary_key_types`/`inner_table_names`/`building_subquery`——`inner_table_names` 的 save/restore + 叶解析臂优先消费是本 Iteration 布局覆盖字段的同型先例（`query.rs:293-306` 用法）。
  - **测试入口**：e2e 模式 = `Database::open` + `execute_sql` → `Response::QueryResult { rows }`（`tests/keyless_eq_routing_test.rs:29-38` 先例）；plan 形状断言 = `pipeline::{parse_stage, plan_stage}` → match `PhysicalPlan` 节点（`tests/keyless_eq_routing_test.rs:41-46/:144-178` 先例）；`is_base_scan_chain`（`query.rs:1033-1042`）对未知变体默认 false（Sort 投影归属安全，无需改动）。
  - **既有 join 测试**：`tests/join_test.rs` 7 用例全为直连执行器等值用例（不触 planner），无非等值拒绝锁定（D0 调查结论，本会话 7/7 复跑确认）——非等值解锁零校准。
- Code and Critical Path: planner 路由 = `ddl_dml.rs::extract_join_conditions`（不动）+ 新结构探测 + `query.rs::build_from_clause_with_projection` NLJ 分支（组合行布局编译经 `expression.rs::build_where`/`build_expression` 布局覆盖字段）；执行 = 新 `src/executor/nested_loop_join.rs::NestedLoopJoinExecutor`（组合行 `left_row ++ right_row` 上谓词求值）+ `pipeline.rs::create_executor_from_plan` 新臂；注册面 = `plan.rs` 枚举/节点、`mod.rs` 导出、`query.rs::get_plan_output_columns`、`pipeline.rs::extract_column_indices`、`correlated.rs::inject_correlated_values`、`query.rs:345/:424/:507` 匹配点扩展。数据流：SQL ON → 结构分类 → （NLJ）谓词按组合布局编译 → 计划节点 → 执行器逐组合求值 → output_columns 投影产出。

**Implementation Guidance**

实施顺序：T10（RED 见证先行）→ T11（节点 + 执行器 + pipeline 臂 + 导出）→ T12（planner 分类 + NLJ 分支 + 布局覆盖编译）→ T13（注册面）→ T14（收尾门）。T12/T13 间顺序非实质（编译闭合两者都必需）。关键事实：(1) NLJ 分支复用 `:193-246` 的 output_columns 构造（两路由共享，`SELECT *` 与列过滤行为与 Hash 一致）；(2) 布局覆盖字段只在 NLJ 分支的 `build_where` 调用外 save/restore，单表路径（None）逐字节不变；(3) 执行器右侧行集物化一次、左侧行流式逐行 × 右侧行逐组合求值（或双侧物化——非实质选择留 Act，与 JoinExecutor 三阶段形态同族）；(4) 谓词求值用 `predicate.evaluate(&combined_row)`（与 FilterExecutor 同源，Ok(true) 产出 / Ok(false) 或 Unknown 折叠跳过 / Err 传播为 `StorageError::ExecutionError` 同型）；(5) 行序不约定——新增用例对结果行排序后断言；(6) 测试表用非 PK 表（规避键路由/键类型强制与本 Iteration 无关的交互面）。

**Behavioral Change**

- 当前：非等值/混合/字面量腿 ON → 计划期 `Plan error: Unsupported expression type`（exit 3），非等值 JOIN 不可达。
- 目标：上述形态生成 `NestedLoopJoin` 计划节点并执行——对左输入每行 × 右输入每行的组合行（左 0..n ++ 右 n..n+m）求值完整 ON 谓词，谓词为真（三值语义下非 Unknown 非假）的组合按 `output_columns` 产出；纯等值 ON 维持既有 Hash 路径（计划形状、结果、错误面逐字节不变）；Hash/NLJ 选择为计划期结构启发式；WHERE + JOIN 与 SELECT 表达式项 + JOIN 对两种 join 节点维持既有拒绝。
- 接口/错误/状态语义：`PhysicalPlan` 新变体（additive）；`PlanBuilder` 新加性字段（additive）；`PlanError` 面不变；页格式/WAL/快照语义零触及；无新增 SQL 语句面。

**Task Contracts**

### T10: RED 测试见证——非等值 JOIN 目标行为锁定

- Requirement/Scenario: join-executor-selection R2-S1/S2/S3、R3-S1、R4-S1、R5-S1（目标行为面）
- Depends on: None
- Targets: 新 `tests/nested_loop_join_test.rs`
- Current behavior: 非等值/混合/字面量腿 ON 计划期 `Unsupported expression type`（baseline probe1/probe2 实证）
- Required behavior: 测试文件定义目标行为——先于实现运行观察 RED（失败原因为计划期拒绝或 NestedLoopJoin 形状缺失）
- Required changes: 新建测试文件，含以下用例（e2e 经 `Database::open` + `execute_sql`；plan 形状经 `parse_stage`/`plan_stage`；表用非 PK 列定义；结果行排序后断言）：
  1. `inequality_join_produces_semantic_result`（R2-S1）：r{1,2} × s{2,3}，`ON r.a < s.b` → {(1,2),(1,3),(2,3)}
  2. `mixed_legs_multi_and_evaluated`（R2-S2）：`ON r.a < s.b AND r.a >= 2` → {(2,3)}
  3. `null_side_combination_excluded`（R3-S1）：一侧含 NULL 行，谓词涉及 NULL 侧的组合不产出
  4. `empty_input_yields_empty_set`（R2-S3）：任一侧空表 → 空集不报错
  5. `non_equi_on_routes_to_nested_loop_join`（R4）：plan 形状 = `PhysicalPlan::NestedLoopJoin`
  6. `equi_on_keeps_hash_join_shape`（R1/R4）：`ON r.a = s.b` plan 形状 = 既有 `PhysicalPlan::Join`
  7. `order_by_over_non_equi_join`（R5）：非等值 JOIN + ORDER BY → 排序正确
  8. `correlated_on_injection`（R5/T13）：IN 子查询内 JOIN 的 ON 含外层引用（如 `SELECT o.x FROM o WHERE o.x IN (SELECT r.a FROM r JOIN s ON r.a < s.b AND r.a > o.y)`）→ 关联语义正确
- Preserve: 既有测试文件零修改；用例不依赖执行顺序
- Forbidden: 为观察执行次数引入计数 hooks；修改既有 join_test
- Test witness: 本任务交付即见证——实现前运行 `cargo test --test nested_loop_join_test` 观察 RED（用例 1-4/7/8 因计划期拒绝失败；用例 5 因变体不存在编译失败——编译失败本身计入 RED 观察，实现后转 GREEN）
- GREEN condition: 由 T14 收口（本任务只建立 RED）
- Verification: `cargo test --test nested_loop_join_test`（RED 输出记录进 Act Response）
- Stop when: 目标行为断言与 delta spec 场景矛盾（返回 Plan）

### T11: NestedLoopJoinNode + NestedLoopJoinExecutor + pipeline 构造臂

- Requirement/Scenario: join-executor-selection R2/R3（执行机制承载）
- Depends on: T10（RED 在位）
- Targets: `src/executor/plan.rs`（`PhysicalPlan` 新变体 `NestedLoopJoin(NestedLoopJoinNode)` + 节点结构 `{ left: Box<PhysicalPlan>, right: Box<PhysicalPlan>, predicate: PredicateRef, output_columns: Vec<OutputColumn> }`）；新 `src/executor/nested_loop_join.rs`（`NestedLoopJoinExecutor`）；`src/executor/mod.rs`（导出）；`src/pipeline.rs::create_executor_from_plan` 新臂（`:617-644` Join 臂邻接——`extract_column_indices` 取左右表名、递归构造左右执行器、传入 predicate/output_columns）
- Current behavior: 变体不存在；非等值 ON 无执行承载
- Required behavior: 执行器对左输入每行 × 右输入每行组合（`left_row ++ right_row`）求值谓词，`Ok(true)` 组合按 `output_columns` 投影产出（`table_alias == left_table_name → left_row[column_index]` 否则 `right_row[column_index]`，与 `join.rs:113-124` 同型）；流式 Volcano 形态（`Executor` trait，`async fn next`）
- Required changes: 节点结构 + 执行器（含 NULL/Unknown 经谓词求值自然排除——`evaluate()` fold 语义）+ 构造臂 + 导出
- Preserve: `JoinNode`/`JoinExecutor`/`JoinConfig` 零改动；`Executor` trait 零改动；既有执行器文件零改动
- Forbidden: 修改 Hash Join 路径任何行为；为 NLJ 引入条件列表或哈希机制（谓词是唯一判定面）
- Test witness: T10 用例 5 的 plan 形状断言 + 用例 1 的执行结果（实现后转绿）
- GREEN condition: 由 T14 收口
- Verification: `cargo test --test nested_loop_join_test`（T10 用例 1/5 转绿即本契约见证）
- Stop when: 组合行求值需要谓词之外的判定面（返回 Plan）

### T12: planner ON 分类启发式 + NLJ 分支 + 组合行布局编译

- Requirement/Scenario: join-executor-selection R1/R2/R4（路由判定面）
- Depends on: T11（节点存在）
- Targets: `src/parser/planner/ddl_dml.rs`（新结构性探测 helper，AND 分解镜像 `extract_join_conditions` 的递归形状）；`src/parser/planner/mod.rs`（`PlanBuilder` 加性字段 `join_column_layout: Option<Vec<(String, Vec<String>)>>`）；`src/parser/planner/expression.rs`（`build_expression` Identifier 臂 `:176-200` 与 CompoundIdentifier 臂 `:201-233` 布局覆盖消费——限定名在 `inner_table_names` 关联检查之后、`self.tables` 之前；非限定名布局全表搜索 0→ColumnNotFound / >1→AmbiguousColumn / 1→偏移+位置）；`src/parser/planner/query.rs::build_from_clause_with_projection`（`:188-190` 处分流：探测纯等值 → 既有 Hash 路径原样；否则 NLJ 分支——output_columns 构造复用 `:193-246`，`build_where` 编译 ON 于布局覆盖 save/restore 内，组装 `NestedLoopJoinNode`，`current_tables.push` 保持）
- Current behavior: 任一非等值/字面量腿 → `:74` `UnsupportedExpression`
- Required behavior: 结构探测判定路由；NLJ 分支产出正确谓词（组合行绝对索引：左表偏移 0..n、右表偏移 n..n+m）
- Required changes: 探测 helper + 布局字段 + 两个解析臂覆盖 + NLJ 分支
- Preserve: `extract_join_conditions` 本体与 Hash 分支行为逐字节不变（等值形态的 ColumnNotFound/AmbiguousColumn/UnsupportedExpression 语义错误原样保留）；`build_where`/`build_expression` 在布局字段 None 时逐字节不变；JOIN 类型面（`:171-175`）不变；output_columns 构造对两路由行为一致
- Forbidden: 语义解析进探测（探测仅结构判定）；修改 `extract_join_conditions`；算术表达式支持（`s.b - r.a` 维持拒绝）；`self.tables` 全局状态污染（布局覆盖必须 save/restore）
- Test witness: T10 用例 1/2（结果）+ 用例 5/6（plan 形状）
- GREEN condition: 由 T14 收口
- Verification: `cargo test --test nested_loop_join_test`（用例 1/2/5/6 转绿）
- Stop when: 分类判定需要运行时信息或代价估算（违反 R4，返回 Plan）

### T13: 注册面——新节点的全匹配点接入

- Requirement/Scenario: join-executor-selection R4-S1（计划观测面）、R5-S1（周边能力保持）
- Depends on: T11
- Targets: `src/parser/planner/query.rs`——`get_plan_output_columns` 新臂（`:77-84` Join 臂同型：output_columns 列名）+ `:345` table_name 匹配扩展（NestedLoopJoin → "join_result"）+ `:424`/`:507` `matches!` 扩展（SELECT 表达式项 + JOIN 拒绝 / WHERE + JOIN 拒绝对新节点同语义生效）；`src/pipeline.rs::extract_column_indices` 新臂（`:805-820` Join 臂同型：output_columns 索引 + 首列 table_alias）；`src/executor/correlated.rs::inject_correlated_values` 新臂（`:52-55` 同型：递归左右 + `node.predicate.inject_parameters(param_values)`）
- Current behavior: 新变体使上述匹配点落入 fallback 或漏接（`:507` 漏接时 WHERE + NLJ 将以 table_name="unknown" 走单表 WHERE 路径——错误行为而非既有拒绝）
- Required behavior: 新节点在全部 Join 匹配点与 Join 节点同语义；ORDER BY/聚合经 `get_plan_output_columns` 正常消费 NLJ 输出形状；关联 ON 参数可注入
- Required changes: 五处注册臂/匹配扩展
- Preserve: 既有 Join/SemiJoin/AntiJoin 臂行为逐字节不变；`is_base_scan_chain`（`query.rs:1033-1042`）默认 false 语义不变（无需改动）
- Forbidden: 遗漏任一匹配点（以 `grep -n "PhysicalPlan::Join" src/` 全量核对收口）；放宽 WHERE + JOIN / 表达式项 + JOIN 拒绝面
- Test witness: T10 用例 7（ORDER BY）、用例 8（关联注入）；WHERE + NLJ 拒绝面由全量回归锁定（既有拒绝用例面）——如需显式见证可加 1 用例断言 `ON r.a < s.b WHERE r.a > 0` 维持 `Unsupported statement type`
- GREEN condition: 由 T14 收口
- Verification: `cargo test --test nested_loop_join_test`（用例 7/8 转绿）+ grep 核对清单写入 Act Response
- Stop when: 出现 Join 匹配点之外的新节点消费面（返回 Plan）

### T14: GREEN 收尾

- Requirement/Scenario: join-executor-selection R1/R5（零回归类）
- Depends on: T10-T13
- Targets: 全局
- Current behavior: 全量 919/0/2（Iteration 000 收尾基线，材料未变化采信）
- Required behavior: nested_loop_join_test 8/8 + join_test 7/7 + 全量回归零修改 + clippy/fmt/validate 全 0/PASS
- Required changes: 如全量暴露依赖旧拒绝行为的既有测试，按 D7 在 delta spec 记录校准后实施（调查预判为零——无非等值拒绝锁定）；否则仅验证
- Preserve: 校准不放宽断言语义
- Forbidden: 静默放宽既有断言
- Test witness: 全量输出决定性片段
- GREEN condition: 全量 0 failed / 2 ignored，passed ≥ 927（919 基线 + 8 新增）
- Verification: `cargo test --test nested_loop_join_test` / `cargo test --test join_test` / `cargo test` 全量 / `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` / `openspec validate --specs --changes`
- Stop when: 全量出现非校准可解失败——返回 Plan

**Invariants**

- 页格式（22B VersionHeader）、WAL 记录格式、文件格式版本、快照/隔离语义零触及。
- 纯等值 ON 的 Hash 路径：计划形状（`PhysicalPlan::Join` + conditions）、执行结果、错误面逐字节不变——既有 `join_test` 7 用例与全量等值 JOIN 面零修改锚点。
- JOIN 类型面保持仅 INNER（`UnsupportedJoinType` 原样）；WHERE + JOIN 与 SELECT 表达式项 + JOIN 拒绝面对两种 join 节点一致。
- 布局覆盖字段 save/restore 严格配对；None 路径（单表 WHERE/投影/谓词）行为逐字节不变。
- 无身份型证据工程；无 test-only 计数 hooks。
- RR/RC 扫描语义不受影响（join 两侧经既有扫描执行器构造，快照穿线沿用 Iteration 000 面零改动）。

**Non-goals**

- WHERE + JOIN 能力（预存拒绝，未来独立 change）；SELECT 表达式项 + JOIN；LEFT/RIGHT/FULL OUTER；SMJ；代价模型、运行时统计与 Join 重排（D-candidates）；算术/函数表达式在 ON 中的新增支持面（既有 `build_expression` 拒绝面维持）；3+ 表链中 NLJ 左输入为投影后 Join 的形状错位（与既有 Hash `build_output_row` 同源的预存边界，不扩大不修复，若用例暴露按 Issue 候选报告）；关联子查询缓存（Iteration 002）；性能优化。

**Acceptance**

1. `tests/nested_loop_join_test.rs` 8/8：非等值语义结果（R2-S1）、混合腿全评估（R2-S2）、NULL 排除（R3-S1）、空输入（R2-S3）、NLJ plan 形状（R4）、等值 Hash 形状保持（R1/R4）、ORDER BY（R5）、关联 ON 注入（R5）——映射 join-executor-selection R1-R4 + R5 部分。
2. `tests/join_test.rs` 7/7 保持（既有 Hash 直连面）——映射 R1/R5。
3. 全量回归零修改通过（0 failed / 2 ignored，passed ≥ 927）——映射 R1/R5 零回归类。
4. clippy/fmt/validate 全 0/PASS。
Acceptance 与 requirement/scenario/design/task/代码/测试映射见 tasks.md RTM（join-executor-selection 5 Requirement 全 Covered；design 依据 D5 + D5a）。

**Verification**

- `cargo test --test nested_loop_join_test`（目标套件，RED→GREEN）
- `cargo test --test join_test`（Hash 面保持）
- `cargo test`（全量，零回归门）
- `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` / `openspec validate --specs --changes`
- 决定性输出各取 ≤20 行写入 Act Response。

**Gate 2 Readiness**

- 无 Missing requirement（RTM 全 Covered）：PASS（tasks.md RTM join-executor-selection 5 Requirement 全 Covered；T10-T14 映射不变）
- 无 Simplified 未批准：PASS（无需求裁剪；R2-S2 场景修订为设计闭合而非裁剪——场景意图不变，见 design D5a，随本计划交用户批准）
- 调查完整（入口/调用链/匹配点/谓词机制/测试入口均有 file:line 证据；基线 919/0/2 材料未变化采信 + join_test 7/7 与三个 CLI probe 本会话新鲜复跑）：PASS
- 设计闭合（分类处方、布局编译机制、注册面枚举、边界与场景修订均已定稿，无 TBD）：PASS（design D5a）
- 任务可执行（T10-T14 各有 Targets/行为变化/测试见证/停止条件）：PASS
- 分轮合理（单 Iteration 单故障域 planner/执行器 Join 面，T10-T14 一 Cycle 承载）：PASS
- 追踪完整（requirement-scenario-design-task-code-test 链路见 Acceptance 与 RTM）：PASS
- 验证充分（目标套件 RED→GREEN + Hash 保持 + 全量零回归 + 工具链门）：PASS
- 无身份型证据工程：PASS（执行次数由等价性与代码审查承载，不建计数机制——D6 同源纪律）
- 无实质未知项留 Act：PASS（非实质留白见 Risks）
- tasks/specs/design/Iteration/Cycle 一致：PASS（delta spec R2-S2 已修订；design D5a 补全；tasks.md RTM 无需改动）
- Persisted Evidence 模式明确：PASS（none，见下）
- 用户批准计划：PASS（2026-09-14 用户批准「批准」——含 delta spec R2-S2 场景修订一并批准，Plan 交付摘要随批准提交）

**Persisted Evidence**

- Mode: none

理由：全部验收由可重复运行的测试与工具链命令承载；RED 见证（计划期拒绝）一条命令可低成本复现，命令、决定性输出、退出码写入 Act Response 即满足公共规则 › 验证。

- Budget: 不适用（none）

**Risks and Notes**

- 非限定名 ON 引用（如 `ON a < b`）在布局搜索命中多表时计划期 `AmbiguousColumn` 拒绝——与 `resolve_column_ref` 既有歧义语义同型，确定性错误面，非实质。
- `evaluate()`（fold，Unknown→false）与 `evaluate_ternary()` 判 `Ternary::True` 在 INNER 语义下可观察等价——执行器取哪种留 Act（非实质）。
- 右侧行集物化 vs 双侧流式——非实质选择留 Act（Investigation Guidance (3)）。
- 关联 ON 注入用例（T10 用例 8）依赖 `inner_table_names` 关联检查先于布局消费（D5a 机制序）；若 e2e 形态下关联参数未达 ParameterExpression（如 IN 子查询右计划构建路径差异），按实际形态记录并返回 Plan——不虚构 GREEN。
- 3+ 表链混合路由（等值链 + 非等值链）可达但超出 spec 场景覆盖：布局基线取全 schema，左输入为投影后 Join 时与既有 Hash `build_output_row` 形状错位同源——Non-goals 已列，若新增用例暴露按 Issue 候选报告，不扩大本 Cycle。
- delta spec R2-S2 场景修订（算术 → 纯比较混合腿）为本次设计闭合的 spec 变更，已随 Gate 2 交用户批准。

## Act Response

- Status: blocked

**Implemented**

- **T10 completed**（RED 见证确立）：新 `tests/nested_loop_join_test.rs` 9 用例（契约 8 用例 + T13 预授权的 WHERE+NLJ 拒绝见证）。实现前运行观察 RED：`error[E0599]: no variant or associated item named NestedLoopJoin found for enum PhysicalPlan`（契约明文计入 RED 观察）；变体落地后（T12 前）用例 1-4/7/8 RED 于计划期拒绝（`Unsupported expression type` / 用例 8 `Subquery returns multiple columns`）。
- **T11 completed**：`PhysicalPlan::NestedLoopJoin(NestedLoopJoinNode)` 新变体 + 节点结构 `{left, right, predicate: PredicateRef, output_columns}`（plan.rs，JoinNode 邻接）；新 `src/executor/nested_loop_join.rs::NestedLoopJoinExecutor`——右输入物化一次、左输入流式逐行 × 右行逐组合（`left_row ++ right_row`）求值 `predicate.evaluate`（与 FilterExecutor 同源 fold 语义，Ok(true) 产出 / Ok(false) 跳过 / Err → `StorageError::ExecutionError` 同型文案），`build_output_row` 与 `join.rs:113-124` 同型；`create_executor_from_plan` 新臂（Join 臂邻接，快照参数照常穿线左右递归）；executor mod 导出。
- **T12 completed**：`ddl_dml.rs` 新增 `is_pure_equi_join_on` + `is_structural_column_ref`（AND 分解镜像 `extract_join_conditions` 递归形状，纯结构判定零语义解析；探测真集恰为 `resolve_column_ref` 的接受面——Hash 路径输入集与行为逐字节不变）；`PlanBuilder` 加性字段 `join_column_layout: Option<Vec<(String, Vec<String>)>>`；`expression.rs` 两列解析臂布局覆盖消费（Identifier 臂 NULL 检查后、self.tables 前：全表搜索 0→ColumnNotFound / >1→AmbiguousColumn / 1→偏移+位置；CompoundIdentifier 臂 inner_table_names 关联检查后、self.tables 前：表偏移+表内位置，表不在布局 → TableNotFound）；`query.rs::build_from_clause_with_projection` 分流——output_columns 构造前移为两路由共享（构造无失败路径，错误序无可观察变化），探测真 → 既有 Hash 臂原样，否则 NLJ 分支（布局 save/restore 严格配对含错误路径，`build_where(&current_tables[0], on_clause)` 于布局覆盖内编译）。
- **T13 completed**（五处注册面 + grep 全量核对）：`get_plan_output_columns` 新臂（Join 臂同型 output_columns 列名）；`:345` table_name 匹配扩展（NestedLoopJoin → "join_result"）；`:424` SELECT 表达式项 + JOIN 拒绝扩展；`:507` WHERE + JOIN 拒绝扩展；`extract_column_indices` 新臂（output_columns 索引 + 首列 table_alias——NLJ 无 conditions）；`correlated.rs::inject_correlated_values` 新臂（`predicate.inject_parameters` + 递归左右）。grep `PhysicalPlan::Join` src/ 全量核对：correlated.rs / query.rs ×4 / pipeline.rs 构造臂 + extract_column_indices——全部接入；`is_base_scan_chain` 默认 false 语义不变（NLJ 落 Sort projection 空臂，与 Join 同型）；`get_subquery_first_column` 落入 `_` 拒绝臂属 **Join 匹配点之外的消费面**（T13 停止条件命中，见 Blocker Handoff，未擅自扩臂）。
- **T14 completed-except-case-8**：nested_loop_join_test 8/9 通过（唯一失败 = 用例 8，契约外缺口）；join_test 7/7；全量 `--no-fail-fast` 927 passed / 1 failed（= 用例 8）/ 2 ignored——既有测试零修改零回归；clippy 0 / fmt 0 / validate 28 PASS。

**Changed Files and Symbols**

- 新文件：`tests/nested_loop_join_test.rs`（9 用例）、`src/executor/nested_loop_join.rs`（`NestedLoopJoinExecutor::new/build_output_row` + `Executor` impl）。
- `src/executor/plan.rs`：`PhysicalPlan::NestedLoopJoin` 变体、`NestedLoopJoinNode` 结构。
- `src/executor/mod.rs`：`mod nested_loop_join`、`pub use nested_loop_join::NestedLoopJoinExecutor`、plan 导出列表加 `NestedLoopJoinNode`。
- `src/pipeline.rs`：import、`create_executor_from_plan` NLJ 臂、`extract_column_indices` NLJ 臂。
- `src/parser/planner/mod.rs`：`PlanBuilder::join_column_layout` 字段 + `new()` 初始化。
- `src/parser/planner/ddl_dml.rs`：`is_pure_equi_join_on` / `is_structural_column_ref`（自由函数）。
- `src/parser/planner/expression.rs`：`build_expression` Identifier 臂与 CompoundIdentifier 臂的布局覆盖块。
- `src/parser/planner/query.rs`：import、`build_from_clause_with_projection` 共享 output_columns 前移 + 启发式分流 + NLJ 分支、`get_plan_output_columns` NLJ 臂、`:345`/`:424`/`:507` 匹配扩展。
- `src/executor/correlated.rs`：`inject_correlated_values` NLJ 臂。

**Deviations from Plan**

1. **用例 7 ORDER BY 用裸列名**（`ORDER BY a`，契约未指定限定形态）：CLI 探针实证 `ORDER BY users.id`（限定名）在既有 Hash Join 上今天即 `ORDER BY only supports column names` exit 3——`extract_column_name` 只接受 Identifier，属两种 join 节点共享的预存拒绝面；按 R5「保持不扩大」NLJ 对齐同一接受面，非等值 + 裸列名 ORDER BY 转绿（8/9 中）。
2. **WHERE + 非等值 JOIN 拒绝文案变化**：改动前该形态在 ON 提取处拒绝（`Unsupported expression type`），改动后经 NLJ 路由成功、在 WHERE+JOIN 拒绝面拒绝（`Unsupported statement type`）——两者均为计划期 exit 3 拒绝，后者正是 T13 处方要求的面对（防漏接成 "unknown" 单表路径）；按 T13 预授权新增第 9 用例锁定。
3. **output_columns 构造前移**：原在 `extract_join_conditions` 之后，现前移至分流前（两路由共享）；构造无失败路径（expect 仅触达已 validate 的表），错误序无可观察差异。Hash 路径行为逐字节不变由 join_test 7/7 + 全量零修改锚定。
4. **pipeline NLJ 臂只取左表名**：契约写「取左右表名」；NLJ 执行器无哈希条件、右表名无消费点，未加 `_right_table_name` 死字段（JoinExecutor 的该字段为既有面不动）。
5. **括号包裹纯等值 ON**（`ON (r.a = s.b)`）改路由 NLJ 并产出正确结果（原 `Unsupported expression type` 拒绝）：结构探测按 D5a 处方「ON 整体非纯列等值 → NLJ」判定，属拒绝面被能力取代的正常范围；Hash 接受面不受影响（探测真集 ⊆ 且 == extract 接受面）。
6. `is_pure_equi_join_on`/`is_structural_column_ref` 落为 ddl_dml.rs 自由函数（契约写「新结构性探测 helper」于 ddl_dml.rs，未规定形态）；query.rs 经 `use super::ddl_dml::is_pure_equi_join_on` 消费。

**Blocker Handoff**

- **触发 task/Gate**：T10 用例 8（`correlated_on_injection`）→ T13 required behavior「关联 ON 参数可注入」的 e2e 见证不可达 → T14 GREEN 条件「目标套件全绿」与 Acceptance 1 第 8 项不满足。Gate 6：Task Contract 无法覆盖达到既有 Acceptance 所需的工作。
- **Plan 预期 vs 实际**：Plan 预期 T13 五处注册面接入后用例 8 转绿（T10 RED 归因「计划期拒绝」）。实际：用例 8 的 SQL 形态在 T11-T13 全部落地后仍计划期失败，RED 原因与契约预期不同，且转绿需要 **两个 T13 枚举清单之外的工作面**：
  1. **`subquery.rs::get_subquery_first_column` 无 Join/NLJ 臂**——Join 形态子查询计划落入 `_ => Err(SubqueryReturnsMultipleColumns)`。新鲜探针（本会话）：`SELECT o.x FROM o WHERE o.x IN (SELECT r.a FROM r JOIN s ON r.a = s.b)`（**纯等值**）→ `statement 1 of 1 failed: Plan error: Subquery returns multiple columns (IN subquery requires single column)`，exit 3——**等值/Hash 路径今天同样不可达，属预存共享拒绝**。为 NLJ 加臂 = 「Join 匹配点之外的新节点消费面」，命中 T13 自身停止条件。
  2. **`subquery.rs::extract_correlated_params`/`collect_outer_column_refs` 只扫描子查询 WHERE（`select.selection`）**——ON 子句（`select.from[].joins[].join_operator`）从不被遍历，ON 中的外层引用（`o.y`）不注册 CorrelatedParam。即使绕过缺口 1，NLJ 谓词中的 `ParameterExpression("o.y")` 永不接收注入值（未注入求值为 Null，静默错——correlated.rs 头注释记载的失败模式），产出静默错误结果。扩关联提取至 ON = 关联机制变更，超出 T13「注册面」范畴。
  3. **范围内无任何可达 SQL 形态**：关联参数非空要求子查询有 WHERE，WHERE + JOIN（base_plan 为任一 join 节点）被 `:507` 拒绝——故「子查询内 JOIN + 关联」今天（Hash）与改动后（NLJ）均不可达；用例 8 要求的能力超出 delta spec join-executor-selection R5（其仅要求既有注入路径**保持**），delta spec 场景集（R1-S1…R5-S1）不含关联 ON 场景。
- **Plan Context Risks 预设条款命中**：「若 e2e 形态下关联参数未达 ParameterExpression（如 IN 子查询右计划构建路径差异），按实际形态记录并返回 Plan——不虚构 GREEN」——本 handoff 即该条款的执行。
- **已完成**：T10（9 用例 + RED 见证）、T11、T12、T13（五处注册面）；**部分完成**：T14（全部门禁绿，唯一失败即用例 8）；**未开始**：无。
- **工作区状态**：在 Iteration 000 + MS16 既有待提交改动之上叠加本 Cycle 改动（文件清单见 Changed Files）；无 plan 外修改；clippy 0 / fmt 0 / validate 28 PASS；全量 927/1/2（1 = 用例 8）。
- **恢复条件**（Plan 裁定，供参考非指示）：(i) replan——设计并授权两个缺口面（get_subquery_first_column NLJ/Join 臂 + 关联参数 ON 遍历；注意缺口 1 为 Hash 同源预存拒绝，属共享面能力决策）；或 (ii) 用例 8 重新定型/移出 Acceptance（delta spec R5 仅要求保持，注入机制臂已按 Join 对齐落地）；或 (iii) 维持现状收口本 Iteration 其余 Acceptance 并在 replan 中显式降级用例 8。任一方向均需 Plan Review 裁定，Act 不自行扩面。

**Blocker Resolution**

None（本 Cycle 未恢复过阻塞）。

**Self-Review**

- Plan compliance: T10-T13 契约逐条核对通过（Preserve/Forbidden 全遵守：`extract_join_conditions`/`build_where` None 路径/`JoinNode`/`JoinExecutor`/`JoinConfig`/Executor trait 零改动；无哈希机制进 NLJ；save/restore 配对；探测零语义解析）；T14 完成除用例 8 外全部条件。
- Full diff reviewed: 已按 Cycle 改动逐文件复查（含跨任务交互：output_columns 前移的错误序分析、布局覆盖与 inner_table_names 的机制序、快照穿线沿用、Sort/Limit/Aggregate 经注册臂消费 NLJ 输出形状）；fmt 重排复核为行合并无逻辑变化。
- Critical findings unresolved: 无。
- Important findings unresolved: 无（用例 8 为 Blocker Handoff 承载的 Acceptance gap，非 review finding）。
- Minor findings unresolved: 2 项记录在案——(a) Deviation 5（括号等值 ON 转 NLJ 可达）与 Deviation 2（拒绝文案变化）属拒绝面文案级观察，非阻塞；(b) `nested_loop_join.rs` 组合行每次分配 `combined` Vec，非热路径优化项（与既有执行器风格一致，性能域归 MS08）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T10 RED | `cargo test --test nested_loop_join_test`（实现前） | `error[E0599]: no variant ... NestedLoopJoin ... not found in PhysicalPlan`（1 compile error） | 变体缺失 = 契约明文 RED 形态 | RED 观察成立 |
| 目标套件 | `cargo test --test nested_loop_join_test`（实现后） | `test result: FAILED. 8 passed; 1 failed`；唯一失败 `correlated_on_injection`：`Expected QueryResult, got Error { message: "Plan error: Subquery returns multiple columns (IN subquery requires single column)" }` | join-executor-selection R1-R4 全场景 + R5 部分（8/9 绿） | PASS except 用例 8（Blocker） |
| Hash 面保持 | `cargo test --test join_test` | `7 passed; 0 failed` | 既有 Hash 直连执行器面 | PASS |
| 全量回归 | `cargo test --no-fail-fast` | `passed=927 failed=1 ignored=2`（失败即用例 8；既有套件零修改全绿） | 全局零回归门（919 基线 + 9 新增） | PASS except 用例 8 |
| 静态检查 | `cargo clippy --all-targets -- -D warnings` | exit 0（仅环境级 cargo config 弃用提示） | 全 targets | PASS |
| 格式 | `cargo fmt --check` | 0 diff | 全仓库 | PASS |
| OpenSpec | `openspec validate --specs --changes` | `Totals: 28 passed, 0 failed (28 items)` | change + specs 结构 | PASS |
| CLI e2e | `rtsql <db> "SELECT r.a, s.b FROM r JOIN s ON r.a < s.b"` | `{"columns":["a","b"],"rows":[[1,2],[1,3],[2,3]]}` exit 0 | NLJ 端到端渲染（表头=行形状，I034 面同型） | PASS |
| CLI 等值锚点 | `rtsql <db> --format table "... ON r.a = s.b"` | 表格渲染 `2 \| 2` exit 0 | Hash 路径行为不变 | PASS |
| 用例 8 探针 | `rtsql <db> "SELECT o.x FROM o WHERE o.x IN (SELECT r.a FROM r JOIN s ON r.a = s.b)"` | `Plan error: Subquery returns multiple columns` exit 3（**等值**形态，Blocker 缺口 1 实证） | IN 子查询 × JOIN 消费面 | 缺口实证 |

**Persisted Evidence**

None required——Plan Context mode: none；RED 见证一条命令可低成本复现（命令/输出/退出码已录入上表）。

**Experience Candidates**

- Issue 候选（change 范围外、有证据的预存缺陷）：「IN (SELECT … JOIN …) 子查询计划期被拒（`Subquery returns multiple columns`）——`get_subquery_first_column` 无 Join 形态臂，**等值/Hash 路径同源**，非等值 merely 先行暴露」。证据：本 Response 验证表末行 CLI 探针 + `src/parser/planner/subquery.rs::get_subquery_first_column` `_` 臂。普通拒绝 vs 缺陷的定性（IN 子查询含 JOIN 是否属应支持面）留 Recorder/用户裁定，Act 只登记候选。

**Remaining Issues**

- 用例 8 `correlated_on_injection` 保持 RED（按契约断言目标行为，不虚构 GREEN）——处置由 Plan Review 裁定（见 Blocker Handoff 恢复条件）。
- 长期方向观察（不阻塞）：限定名 ORDER BY over JOIN、算术 ON 腿、WHERE + JOIN 能力均为预存共享拒绝面，维持既有边界。

**Commit or Diff Reference**

未提交（工作区待用户统一触发）。本 Cycle 完整增量 = 上列 10 个文件；对照基线为 Iteration 000 收尾后工作区状态（40 个已跟踪修改文件 + 历史未跟踪文件，本 Cycle 未触碰）。

## Plan Review

- Review Result: rework-required

**Findings**

- **F1（阻塞独立核实成立）**：用例 8（e2e 关联 ON 注入）在批准范围内不可达，三段机理链全部独立读码+探针证实——(1) 缺口 1：`subquery.rs::get_subquery_first_column`（:386-438）无 Join/NLJ 臂，Join 形态子查询计划落 `:437` `_ => Err(SubqueryReturnsMultipleColumns)`；本会话新鲜探针：**纯等值** `IN (SELECT r.a FROM r JOIN s ON r.a = s.b)` → exit 3（Hash 同源预存拒绝，非等值 merely 先行暴露）。(2) 缺口 2：`extract_correlated_params`/`collect_outer_column_refs`（`subquery.rs:156-240`）仅遍历子查询 `select.selection`（WHERE），ON 子句从不扫描——ON 外层引用永不注册 CorrelatedParam，`ParameterExpression` 未注入求值为 Null（静默错）。(3) 缺口 3：关联参数非空要求子查询有 WHERE，而 WHERE + JOIN（等值/非等值两形态均探针实测）被 `:507` 拒绝（`Unsupported statement type` exit 3，两种 join 节点一致）——范围内无任何可达 SQL 形态。
- **F2（Act 偏差全部核实为非实质且合规）**：Deviation 1（ORDER BY 用裸列名）——探针实证限定名 `ORDER BY r.a` 在既有 Hash Join 上今天即 `ORDER BY only supports column names` exit 3（`extract_column_name` 仅接受 Identifier），共享预存拒绝面，NLJ 对齐正确；Deviation 2（WHERE+非等值拒绝文案 `Unsupported expression type` → `Unsupported statement type`）——后者正是 T13 处方要求的拒绝面（防漏接成 "unknown" 单表路径），探针 P5/P6 证实两节点一致；Deviation 3（output_columns 构造前移两路由共享）——join_test 7/7 + 全量零修改锚定行为不变；Deviation 4（pipeline 臂不取右表名）——NLJ 无哈希条件，右表名无消费点，合理；Deviation 5（括号等值 ON `ON (r.a = s.b)` 转 NLJ 产出 {(2,2)}）——原形态在既有接受面之外（今天拒绝），属拒绝面被能力取代的正常范围，R1 不覆盖（其锚点为既有接受面）；Deviation 6（探测 helper 落自由函数）——契约未规定形态。
- **F3（实施面与验证核实）**：工作区增量与 Act 声明精确一致（40 文件基线 +4 已跟踪修改：`correlated.rs`/`executor mod.rs`/`plan.rs`/`expression.rs` + 2 新文件）；`extract_join_conditions` 本体完好（恰 1 处 `UnsupportedExpression`）；布局 save/restore 配对（`query.rs:297/301/305` 含错误路径臂）；注入臂在位（`correlated.rs:56`）；本会话独立复跑：nested_loop_join_test **8 passed / 1 failed**（唯一失败即用例 8，失败形态与 Blocker 记载一致）、join_test **7/7**、clippy 0 / fmt OK / validate 28 PASS；全量 927/1/2 采信 Act（材料未变化 + 决定性套件独立复跑）。
- **F4（根因归 Plan，Act 零偏离）**：PLAN-INVALID——T10 用例 8 见证处方（e2e 关联 ON）为 Plan 超出 delta spec 场景集的过度具体化：R5 仅要求注入路径「保持」，spec 无关联 ON 场景，该形态在批准范围内不可达， witnesses 永无法按契约转绿；PLAN-OMISSION——T13 注册面清点以 `grep PhysicalPlan::Join` 为界，漏 `subquery.rs` 对 Join 形态子查询的两处消费面（致无效处方未被 Gate 2 拦截）；漏接面本身亦为 Hash 同源预存边界，未破坏任何 spec 要求的 Acceptance。Act 在 T13 自身停止条件处正确停机、Preserve/Forbidden 零触碰、Handoff 完整——ACT-DEVIATION 为 None。
- **F5（非阻塞）**：Issue 候选（`IN (SELECT … JOIN …)` 等值形态预存计划期拒绝）证据齐全，定性留 Recorder/用户，不影响本裁定。

**Deviation Classification**

- PLAN-INVALID（用例 8 见证处方不可达——Plan 过度具体化）+ PLAN-OMISSION（注册面清点方法漏 subquery.rs 消费面）。Act 偏差：None。无 BASELINE-CHANGED、NEW-EVIDENCE。

**Acceptance Gaps**

- 父 Cycle Acceptance 1 第 8 项：`correlated_on_injection` e2e 见证保持 RED——按当前契约不可转绿（见证处方超出批准范围可达面）。其余 Acceptance 全部满足（8/9 + join 7/7 + 全量 927/1/2 唯一失败即用例 8 + 工具链门全过）。

**Convergence**

N/A（initial Cycle 首次 Review，无当前 Cycle 历史版本可比；gap 为新确立项而非已跟踪 gap 的恶化）

**Evidence**

- 代码核实：`subquery.rs:386-438`（`_` 臂 :437）、`subquery.rs:156-175`（WHERE-only）、`query.rs:507`（拒绝面）、`correlated.rs:56`（注入臂在位）、`query.rs:297/301/305`（save/restore 配对）、`ddl_dml.rs::extract_join_conditions`（本体完好）。
- 探针（本会话新鲜）：纯等值 IN×JOIN exit 3；WHERE+等值/非等值 JOIN 均 `Unsupported statement type` exit 3；限定名 ORDER BY over Hash JOIN exit 3（预存）；括号等值 ON → NLJ {(2,2)} exit 0；非等值 e2e {(1,2),(1,3),(2,3)} exit 0。
- 复跑：nested_loop_join_test 8/1/0（唯一失败 `correlated_on_injection`）、join_test 7/7、clippy 0、fmt OK、validate 28 passed/0 failed。
- 采信：Act 全量 927/1/2、CLI e2e 渲染、RED 序列（材料未变化 + 决定性套件独立复跑）。

**Follow-up Decision**

创建 rework Cycle 收口同一 Acceptance：用户 2026-09-14 裁定选项 A「见证改形 rework」——delta spec（需求权威）无关联 ON 场景、R5 仅要求注入路径保持，以可达见证（直接构造 NLJ 计划 + `inject_correlated_values` 行为断言）锁定 T13 注入臂，GREEN 9/9、全量 928/0/2，不改 spec、不加能力、产品代码零改动。repair item T10-R1 契约落于后继 Cycle。缺口 1/2 与 e2e 关联 ON × JOIN 能力维持预存边界（不扩面）；Issue 候选留 Recorder/用户裁定。需新执行契约（见证处方更换 + 收尾条件更新），故为 rework Cycle 而非当前 Cycle 修复。

**Iteration Plan Update**

None（rework 不修改 Iteration Map；T10-T14 分配与验收边界不变）

**Next Cycle**

`iterations/001-nlj/001-rework.md`（已创建，Plan Context ready，repair item T10-R1）

**Next Iteration**

None（Iteration 001 未完成；001-rework accepted 后按 tasks.md Map 展开 Iteration 002）
