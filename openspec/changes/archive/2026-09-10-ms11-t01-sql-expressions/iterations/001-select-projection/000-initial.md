# Iteration 001 / Cycle 000-initial: SELECT 派生列（投影表达式机制）

## Plan Context

- Status: ready
- Iteration: 001-select-projection
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T7, T8, T9
- Depends on: Iteration 000（accepted，2026-09-10——值表达式 `CaseExpression`/`CoalesceExpression`/`CastExpression` 已实现且经 E2E 见证）
- Stable baseline: 非聚合 SELECT 列表支持值表达式项 + AS 别名 + Display 列名；纯列查询 plan 逐字节不变；CLI 四格式渲染兼容；既有测试零修改
- Verification boundary: `tests/projection_expression_test.rs` + cli_test 追加全绿；全量 `cargo test` / clippy / fmt / `openspec validate --all` 通过
- Diagnostic boundary: `src/executor/{plan,projection}.rs`、`src/pipeline.rs`、`src/parser/planner/query.rs` SELECT 路由、`src/parser/ast.rs`
- Deferred tasks: None（change 最后一个 Iteration）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: design D4；Iteration 000 Review 裁定的 R3 S1-S4 SELECT 形态断言归属本 Iteration（T9 见证）；proposal 默认假设 4/5/6（列名规则、ORDER BY 限制、聚合报错保持）
- Excluded scope: 标量子查询项与表达式项混用；`SELECT *, expr` 通配混用；聚合查询表达式项；算术运算；HAVING 新表达式；I034/I035/I036（已登记独立 I 项）

**Objective**

`SELECT <值表达式> [AS alias] [, ...] FROM t` 在非聚合查询下逐行求值输出；`SELECT 42 FROM t` 类常量项输出单列（修正恒等回退怪癖）；纯列查询 plan 与行为逐字节不变。

**Background**

路线图 MS11-T01 Outcome "WHERE/**SELECT** 支持"；Iteration 000 已交付谓词面与值表达式实现（Review accepted）。SELECT 侧当前投影是纯列索引裁剪，表达式项不可达——本 Iteration 建立投影表达式机制并接通 CLI 表头。

**Current Baseline**

- revision `a5b0a5f` + 未提交 MS10-T05 现场 + 本 change Iteration 000 实施（未 commit，叠加工作树）。
- 全量 747 passed / 0 failed / 2 ignored（Iteration 000 Review 独立复跑 2026-09-10）。
- 已知基线 flaky（非本 change 面）：`cli::resolve::tests::test_db_dir_env_cases`（env 竞态，Iteration 000 Review Finding 6）——遇其偶发失败重跑即可，不计入本 change 失败。

**Current-State Evidence**

投影表示与携带节点（`src/executor/plan.rs`）：
- `PhysicalPlan` enum :18-57（**19 变体**；本 Iteration 新增 `Projection` → 20，M01 节点清单由 docs-maintainer 在 change 收尾同步——change 收尾义务，非 Act 代码义务）。
- `projection: Vec<usize>` 字段（空 = 恒等）：`ScanNode` :61-69、`DataScanNode` :77-91（谓词求值后应用）、`IndexScanNode` :95-104、`IndexScanAllNode` :108-117、`FilterNode` :121-131（谓词后应用）、`SortNode` :261-270（**输入行形状上比较排序键、产出时裁剪**——design D10：排序键允许不在投影内）。`columns: Vec<String>` 为各节点输出表头。
- `SubqueryEvalNode` :400-411：标量子查询逐行求值并**追加一列**（`result_column_index`）——其输出形状 = 输入 + 1，故与表达式项混用被本 Iteration 拒绝。

SELECT 路由（`src/parser/planner/query.rs`）：
- `build_query` SELECT 处理 :280-389：`extract_columns`/`extract_qualified_columns`（`ast.rs`）在 :290-300 **先于**聚合检测 :313-359 调用（JOIN 输出过滤与投影列收集用）；聚合检测循环 :321-359 非聚合项经 `expr_to_column_name`（`expression.rs:242-253`，非列/字面量 → `InvalidAggregateArgument`）。
- 投影解析 :363-389：`base_schema` 仅 `Scan` 节点取（:369-372）；`has_aggregates || !subquery_evals.is_empty()` → `None`（:374-376）；否则 `resolve_projection_indices(&projection_columns, schema)`（:830-851，**名字不在基 schema → None → 恒等回退**——`SELECT 42 FROM t` 返回全 schema 行的怪癖根源，:367-368 注释自认 "alias / expression"）；`sort_due`（:373）时 `proj_or_empty` 置空、Sort 拥有裁剪（:381-389）。
- `get_plan_output_columns` :23-83：穷尽 match（无 `_` 臂），Filter/Sort 按非空 `projection` 裁剪描述输出形状。CLI 调用点 `cli/mod.rs:286`。
- `is_base_scan_chain` :815-826（Sort 裁剪门）。

列提取（`src/parser/ast.rs`）：`extract_columns` :36-95（`Expr::Function` 臂 :48-86 未知名 → `UnsupportedStatement`）；`extract_qualified_columns` :100-164 同构。**当前 SELECT 表达式 RED 基线（2026-09-10 二进制探针实测）**：`SELECT CASE...` → `Plan error: Invalid aggregate argument: Expected column name`；`SELECT COALESCE(...)` / `SELECT CAST(...)` → `Plan error: Unsupported statement type`。**T8 必须扩展两函数遍历 `Case`/`Cast`/`Coalesce(Function)`/`InList`/`Between`/`Like`/`IsNull` 并放行 COALESCE**，否则表达式项在聚合检测前就被拒。

执行器构造与输出（`src/pipeline.rs`）：`create_executor_from_plan` :433+ 穷尽 match（Filter 臂 :445-450 为 `with_projection` 透传样板）；`execute_executor` :405-426（`Row → value_to_json`）；`value_to_json` :711-729（五变体全支持，NaN/Inf→Null）。

相关子查询注入（`src/executor/correlated.rs`）：`inject_correlated_values(plan, params)` :14 走计划树（Filter/Having/SemiJoin/AntiJoin/SubqueryEval/DataScan/Sort/Limit/Aggregate/Join 臂 :16-52+）——新增 `Projection` 需透传臂（递归 input），否则相关子查询外层 `SELECT ... , CASE ...` 注入断裂。

CLI（`src/cli/mod.rs`）：`run_sql` :270-311——`get_plan_output_columns` :286 → `execute_stage` :288 → 渲染 :288-307；`render.rs` 全程 serde_json，表达式产值为既有 `Value` 变体零改动。

Iteration 000 交付的硬约束（Act Remaining Issues 3，Review Finding 确认）：`CaseExpression`/`CoalesceExpression`/`CastExpression` 均实现 owned `evaluate`；其 `evaluate_ref` 对 String 结果**显式报错**（`predicate.rs` 注释）。当前无 `evaluate_ref` 外部消费者；**`ProjectionExecutor` 必须调用 owned `Expression::evaluate(&[Value])` 路径**，不得走 `evaluate_ref`。

**Relevant Code**

- `src/executor/plan.rs` — 新增 `ProjectionNode`/`ProjectionItem` 变体
- `src/executor/projection.rs`（新）— `ProjectionExecutor`
- `src/executor/mod.rs` — re-export（additive）
- `src/pipeline.rs` — `create_executor_from_plan` 新臂
- `src/executor/correlated.rs` — `inject_correlated_values` 透传臂
- `src/parser/planner/query.rs` — SELECT 表达式项检测与顶层包装、`get_plan_output_columns` 新臂
- `src/parser/ast.rs` — `extract_columns`/`extract_qualified_columns` 遍历扩展
- `tests/projection_expression_test.rs`（新）、`tests/cli_test.rs`（追加）、`tests/predicate_test.rs`（如需表达式在 Projection 内的组合单测，可不放）

**Critical Path**

`SELECT` 列表 → `build_query`：检测任一项非普通列标识符（表达式/字面量/带别名表达式）→ 既有 per-node 裁剪全部置空（`proj_or_empty=Vec::new()`、Sort 不裁剪）→ 最终 plan 顶层包 `Projection(ProjectionNode{input, items, columns})` → `create_executor_from_plan` 构造 `ProjectionExecutor` → 逐行对**全形状输入行**求值 `items.map(eval)`（列项 = `ColumnExpression`，表达式项 = Iter 000 实现，owned `evaluate`）→ `execute_executor` → CLI 表头经 `get_plan_output_columns(Projection) = node.columns`。纯列查询：检测不触发，plan 逐字节同现状。

**Implementation Guidance**

顺序：T7（节点 + 执行器 + 三处接线）→ T8（planner 路由 + ast.rs 扩展）→ T9（见证收口）。TDD：先写 `tests/projection_expression_test.rs`（运行时 RED——当前报 Invalid aggregate argument / Unsupported statement type）。

- **ProjectionNode 形态**：`ProjectionNode { input: Box<PhysicalPlan>, items: Vec<ProjectionItem>, columns: Vec<String> }`；`ProjectionItem { expr: ExpressionRef, name: String }`。列引用项统一表达为 `ColumnExpression{column_name, column_index}`（全 schema 索引）——与谓词同源语义。`Debug`/`Clone` 派生对齐既有节点。
- **顶层放置**：包在最终 plan 最外层（`Limit` 之上）。含 ORDER BY + 表达式项时 Sort 的 `projection` 保持空（全行流过，排序键可达），顶层统一裁剪 + 求值——继承 design D10 精神；无表达式项的纯列查询完全走既有路径（兼容关键：MS10-T01 的 `projection_test.rs`/`pushdown_test.rs` plan 断言不动）。
- **列名规则**：`ExprWithAlias{alias}` → `alias.value`；`UnnamedExpr` → sqlparser `Expr` Display 文本。`SELECT 42` 列名 `42`（怪癖修正：不再 `_42`/恒等回退）。注意 ast.rs 提取列与命名分离：列收集（JOIN 过滤）与输出命名是两回事。
- **拒绝面**（`ParseError` additive 文案）：表达式项与 SubqueryEval 项混用（子查询列追加移位输出形状）；`SELECT *, expr` 通配混用。二者单独出现维持现状。
- **聚合边界**：聚合检测循环（:321-359）不改判定顺序——聚合查询中的非聚合表达式项维持 `InvalidAggregateArgument` 报错（spec R4/S3 后半）；检测先行于 T8 的表达式路由，表达式路由仅 `!has_aggregates && subquery_evals.is_empty()` 时接管。
- **ast.rs 扩展**：`extract_columns`/`extract_qualified_columns` 对 `Case`（conditions/results/else/operand）、`Cast`（inner）、`Function`（COALESCE 参数——其余函数名维持 `UnsupportedStatement` 不变）、`InList`/`Between`/`Like`（expr/list/pattern）、`IsNull/IsNotNull`（inner）遍历收集列引用（JOIN 输出过滤与投影列收集依赖）。
- **`get_plan_output_columns`**：`Projection` 臂 → `node.columns`（穷尽 match 编译器强制）。
- **`inject_correlated_values`**：`Projection` 透传臂递归 input（相关子查询场景 `SELECT outer.x, CASE WHEN ... END FROM ... (SELECT ...)`）。

**Behavioral Change**

- 当前：SELECT 表达式项报错（CASE → `InvalidAggregateArgument`；COALESCE/CAST → `UnsupportedStatement`）；`SELECT 42 FROM t` 恒等回退返回全 schema 行。
- 目标：表达式项逐行求值输出、AS 别名/Display 列名、常量项单列输出；派生列值经 `value_to_json` 走既有四格式渲染。
- 不变：纯列查询 plan 逐字节不变；`SELECT *` 不变；聚合查询报错不变；错误文案 additive。

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T7 | R4/S1-S2、R4/S4 | `plan.rs`（enum :18-57）、新 `projection.rs`、`pipeline.rs::create_executor_from_plan` :433+、`correlated.rs::inject_correlated_values` :14、`query.rs::get_plan_output_columns` :23-83 | 无 Projection 变体；穷尽 match 需新臂 | 新增 `ProjectionNode`/`ProjectionItem`/`ProjectionExecutor`（owned `evaluate`）+ 三处接线臂 |
| T8 | R4/S1-S3 | `query.rs` :280-389、`ast.rs::extract_columns`/`extract_qualified_columns` :36-164 | 表达式项不可达；列提取不认新变体 | 表达式项检测 → 顶层包装 + 列名规则 + 混用拒绝 + ast.rs 遍历扩展（放行 COALESCE） |
| T9 | R4/S1-S4、R3/S1-S4（SELECT 形态）、R6/S1 | `tests/projection_expression_test.rs`（新）、`tests/cli_test.rs`（追加） | 无见证 | 场景矩阵 + CLI 渲染 + 全量回归 |

**Task Contracts**

### T7: ProjectionNode 节点、执行器与三处接线

- Requirement/Scenario: R4/S1-S2、R4/S4
- Depends on: None（依赖 Iter 000 的值表达式类型，已就绪）
- Targets: `src/executor/plan.rs`、`src/executor/projection.rs`（新）、`src/pipeline.rs::create_executor_from_plan`、`src/executor/correlated.rs::inject_correlated_values`、`src/parser/planner/query.rs::get_plan_output_columns`
- Current behavior: 无 `Projection` 变体；穷尽 match 编译失败即未接线
- Required behavior: `ProjectionExecutor` 对全形状输入行逐项求值（`Expression::evaluate` owned 路径）产出 `items.map(eval)` 行；`get_plan_output_columns(Projection)` 返回 `node.columns`；`inject_correlated_values` 透传 input
- Required changes: 节点与 item 类型（Debug/Clone）；新执行器文件；pipeline/correlated/get_plan_output_columns 三臂
- Preserve: 既有 19 变体与 6 节点 `projection: Vec<usize>` 字段、执行器 `with_projection` 零修改；既有测试零修改
- Forbidden: 调用新表达式类型的 `evaluate_ref`（String 结果报错——硬约束）；修改 `value_to_json`/render
- Test witness: 编译 RED（match 穷尽）→ 单元级 GREEN 由 T8 路由接通后 E2E 覆盖；`cargo test --test predicate_test --test planner_test` 保持全绿（未破坏既有）
- GREEN condition: 既有套件全绿 + 新变体三臂接通（T8 E2E 见证端到端）
- Verification: `cargo test`（全量）
- Stop when: 穷尽 match 暴露未预期的第三处消费点且语义不明——返回 Plan

### T8: planner SELECT 表达式路由 + ast.rs 列提取扩展

- Requirement/Scenario: R4/S1-S3
- Depends on: T7
- Targets: `src/parser/planner/query.rs`（:280-389 SELECT 处理）、`src/parser/ast.rs`（:36-164）
- Current behavior: 表达式项 → `InvalidAggregateArgument`/`UnsupportedStatement`；`SELECT 42` 恒等回退
- Required behavior: 非聚合、无标量子查询项的 SELECT 含表达式项 → 顶层 `Projection` 包装（列项 = ColumnExpression、表达式项 = Iter 000 类型、命名 = alias 或 Display）；`SELECT 42` 单列；通配/子查询混用 → `ParseError`；聚合查询表达式项报错保持
- Required changes: SELECT 项分类检测（普通列 vs 表达式）、包装时机（`proj_or_empty` 置空 + 顶层包）、列名生成、ast.rs 两函数遍历扩展（Case/Cast/Coalesce/InList/Between/Like/IsNull，COALESCE 放行、其余函数名维持拒绝）
- Preserve: 纯列查询 plan 逐字节不变（既有 plan 断言测试零修改通过）；聚合检测循环顺序不变；`resolve_projection_indices` 既有语义不变（表达式路径绕过而非修改）；JOIN 查询行为不变（表达式项 + JOIN → 现状 `UnsupportedStatement` 或明确 ParseError，不得静默变形）
- Forbidden: 修改 6 节点 `projection` 字段类型或执行器 `with_projection`；为聚合查询增加表达式求值
- Test witness: `tests/projection_expression_test.rs`（运行时 RED：当前两态错误文案见 Current-State Evidence）→ GREEN
- GREEN condition: 新增测试全绿 + `projection_test.rs`/`pushdown_test.rs`/`planner_test.rs` 既有断言零修改全绿
- Verification: `cargo test --test projection_expression_test --test projection_test --test planner_test`
- Stop when: 表达式项与 JOIN/聚合/子查询的组合出现契约未预料的可行路径分歧——返回 Plan

### T9: Iteration 001 测试见证与回归收口

- Requirement/Scenario: R4/S1-S4、R3/S1-S4（SELECT 形态断言，Review 裁定归属）、R6/S1
- Depends on: T7, T8
- Targets: `tests/projection_expression_test.rs`（新）、`tests/cli_test.rs`（追加）
- Current behavior: 无见证
- Required behavior: spec R4 四场景 + R3 SELECT 形态（searched/simple CASE、COALESCE、CAST 出现在 SELECT 列表）+ CLI 渲染兼容断言；回归门全绿
- Required changes: 场景矩阵组织（GIVEN/WHEN/THEN 注释对应 spec）
- Preserve: 既有测试文件零修改（只追加）；747 基线零失败
- Forbidden: 修改既有断言
- Test witness: 全量 `cargo test`（≥747+新增 / 0 failed / 2 ignored）
- GREEN condition: `cargo test` 0 failed；clippy/fmt/validate 全 0/PASS
- Verification: 四命令决定性输出写入 Act Response
- Stop when: 既有测试失败且根因不在本 change 文件——BASELINE-CHANGED，Blocker Handoff（`test_db_dir_env_cases` env 竞态 flaky 除外：重跑通过即不计失败，记录即可）

**Invariants**

- 既有全量测试零修改（747/0/2 基线）；`projection: Vec<usize>` 与 `with_projection` 六处零修改；错误文案 additive；plan cache 仅 Query；JOIN ON 等值限制；M13 异步原则（ProjectionExecutor 求值同步纯函数）；M15 命名规范。
- `PhysicalPlan` 19 → 20 变体为唯一节点集合变化；M01 模型条目由 docs-maintainer 在 change 收尾同步（change 收尾义务清单，非 Act 代码义务）。

**Non-goals**

标量子查询与表达式项混用；`SELECT *, expr`；聚合查询表达式项；算术运算；ORDER BY 引用派生列/别名；HAVING 新表达式；I034/I035/I036。

**Acceptance**

spec R4/S1-S4 + R3/S1-S4（SELECT 形态）+ R6/S1-S2 可观察并通过测试断言；RTM 见 change `tasks.md`（R4 行 + R3 行注记）。

**Verification**

1. `cargo test`（全量，≥747+新增 / 0 failed / 2 ignored）
2. `cargo test --test projection_expression_test --test cli_test --test projection_test --test planner_test`
3. `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check`
4. `openspec validate --all`

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | 本 Plan Context Current-State Evidence（投影链 file:line 本会话两轮核实 + Iter 000 探针实测 RED 文案）；ProjectionNode 相关结论经 Iter 000 Review 确认 |
| Design | PASS | design.md D4（节点形态/放置/列名/拒绝面/接线点闭合）；硬约束（owned evaluate）经 Iter 000 Review 传递 |
| Iteration Plan | PASS | change tasks.md Iteration Plan + 平衡审计（用户批准 2026-09-10） |
| Cycle Scope | PASS | initial 范围 = T7-T9；Excluded scope 明确 |
| Task Contracts | PASS | 三任务契约含位置/行为/见证/停止条件；Act 只读本 Cycle 即可实施 |
| Traceability | PASS | RTM R4 + R3 注记 + R6 全 Covered |
| Verification | PASS | 验证命令与通过条件明确；无身份型证据工程 |

**Persisted Evidence**

- Mode: none

Act Response 足以承载验证结果。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- 表达式项 + ORDER BY 组合：Sort 全行流过 + 顶层裁剪是既定方案；若 Act 发现 Sort 在表达式场景的列名解析（`extract_column_name` :873-880 只认列名）与顶层 Projection 交互异常，属契约内处理（ORDER BY 基础列始终全行可达）；ORDER BY 引用派生列本身是 Non-goal（显式报错）。
- `SELECT 42` 怪癖修正为**行为变化**（恒等回退 → 单列 `42`）：已 grep 核实无既有测试锁定；e2e 层面为新增断言。
- 基线 flaky `test_db_dir_env_cases` 见 Current Baseline——重跑通过不计本 change 失败。
- 工作树未 commit 现场叠加：Act diff 审查范围以本 change 文件为限。

## Act Response

- Status: reported

**Implemented**

T7-T9 全部完成，TDD 见证齐全（新套件 16 测试运行时 RED —— 实施前实测 14 failed / 2 passed，2 个通过项为现状守卫「SELECT * 不变」与「JOIN 已拒绝」——后 GREEN；T7 结构见证 = 穷尽 match 编译 RED，精确落在契约三处接线点 correlated.rs:15 / query.rs:24 / pipeline.rs create_executor_from_plan）：

- **T7 投影机制**：`ProjectionNode{input, items, columns}` + `ProjectionItem{expr, name}`（plan.rs，PhysicalPlan 19 → 20 变体）；新 `src/executor/projection.rs` `ProjectionExecutor`——对全形状输入行逐项求值，**走 owned `Expression::evaluate` 路径**（Iteration 000 Review 硬约束，未触碰 `evaluate_ref`）；三处接线：`create_executor_from_plan` Projection 臂、`get_plan_output_columns` → `node.columns`、`inject_correlated_values` 透传 input。
- **T8 路由**：`ast.rs` 两函数扩展七变体臂（Case/Cast/InList/Between/Like/IsNull/IsNotNull → Display 名）+ Function 臂 COALESCE 放行（其余函数名维持 `UnsupportedStatement`）；`query.rs` 聚合检测循环非聚合项分类（`is_plain_column_expr`：裸标识符非 NULL / 两段 CompoundIdentifier 为普通列，其余标记 `has_expression_items` 不再误报 InvalidAggregateArgument）；路由块四拒绝面（聚合+表达式 → InvalidAggregateArgument、子查询混用/JOIN/通配混用 → ParseError additive）；`projection_indices` 表达式查询强制 None（per-node 裁剪置空，Sort 排序键可达）；顶层 `Projection` 包装于 LIMIT/SubqueryEval 之上；列名 = alias 或 sqlparser Expr Display。
- **T9 见证**：`tests/projection_expression_test.rs` 16 测试（R4/S1-S4 + R3/S1-S4 SELECT 形态 + 怪癖修正 + 排除面守卫 + ORDER BY 基础列组合）；cli_test +6 渲染测试（别名/Display 表头、COALESCE 表头原文 `COALESCE(NULL, id)`、常量单列、csv/table 渲染）；全量回归 769/0/2。
- **子查询上下文抑制（R6 回归修复，见 Deviations 1）**：`PlanBuilder.building_subquery` 标志（mod.rs）+ 三个递归调用点 save/restore（subquery.rs IN/EXISTS 两处、query.rs 派生表与标量子查询两处）——子查询计划形状保持既有行为。

**Changed Files and Symbols**

| File | Change |
|---|---|
| `src/executor/plan.rs` | +`Projection(ProjectionNode)` 变体（第 20 个）；+`ProjectionItem`/`ProjectionNode`（Debug/Clone）；import `ExpressionRef` |
| `src/executor/projection.rs` | 新建：`ProjectionExecutor`（owned `evaluate` 逐项求值；非行结果透传；错误经 `StorageError::ExecutionError("Expression evaluation error: {e}")` additive 包装） |
| `src/executor/mod.rs` | +`mod projection`；re-export `ProjectionExecutor`；plan re-exports +`ProjectionItem`/`ProjectionNode` |
| `src/pipeline.rs` | `create_executor_from_plan` +Projection 臂（items 提取 expr 交执行器） |
| `src/executor/correlated.rs` | `inject_correlated_values` +Projection 透传臂（items 经 `set_parameter_value` 注入 + input 递归） |
| `src/parser/ast.rs` | `extract_columns`/`extract_qualified_columns` +七变体臂（Display 名）+ Function 臂 COALESCE 放行 |
| `src/parser/planner/mod.rs` | `PlanBuilder` +`building_subquery: bool` 字段 |
| `src/parser/planner/subquery.rs` | IN/EXISTS 两处递归 build_query 的 flag save/restore |
| `src/parser/planner/query.rs` | `get_plan_output_columns` +Projection 臂；`build_query` 顶部读 flag；聚合循环分类（`!building_subquery && !is_plain_column_expr` → 表达式标记）；路由块（四拒绝面 + items 构建）；`projection_indices` 表达式查询 → None；顶层 Projection 包装；+`is_plain_column_expr` |
| `tests/projection_expression_test.rs` | 新建，16 测试 |
| `tests/cli_test.rs` | +6 渲染测试（追加段，既有 48 零修改） |

不变量核查：既有 19 变体与 6 节点 `projection: Vec<usize>` 字段、执行器 `with_projection` 零修改；既有测试文件零修改（cli_test/predicate/planner/pushdown/projection 均只追加）；纯列查询 plan 逐字节不变（projection_test 6 + pushdown 16 + planner 36 零修改全绿）；`resolve_projection_indices`/聚合检测循环判定顺序未改（仅 else 分支分类）；plan cache 仅 Query 不变；错误文案 additive（+4 条 ParseError + 1 条 ExecutionError 包装模板）；无新增依赖。

**Deviations from Plan**

1. **子查询上下文抑制（Plan Context 未预见，R6 回归修复）**：首次实施后全量回归 `tests/subquery_test.rs` 的 `test_correlated_exists`/`test_correlated_not_exists` FAILED——`EXISTS (SELECT 1 FROM ...)` 类常量投影子查询被表达式路由包装为 Projection，破坏 SemiJoin 侧计划形状消费面（`get_subquery_first_column` 等）。修复：`building_subquery` 标志在三个递归 build_query 调用点 save/restore，子查询上下文内分类与路由全部抑制、子查询计划逐字节保持现状。属 T8 契约 Preserve 条款「既有全量测试零修改通过」的必要实现面，非范围扩展。
2. **correlated.rs Projection 臂含 items 参数注入**（契约 Required behavior 仅「透传 input」）：表达式项内 ParameterExpression（相关外层引用）若不注入将静默求值为 Null（`ParameterExpression` 未注入值时 evaluate → Null）。经 `Expression::set_parameter_value` 注入——与谓词操作数传播同链，且 Iteration 000 已为此交付 CASE/COALESCE/CAST 的向内传播。属新臂作用域内的非实质局部差异。
3. **聚合+表达式项的错误消息按 D4 目标态断言**：实施前 `SELECT COUNT(*), CASE...` 因 extract_columns 先拒报 "Unsupported statement type"（Plan Context Current-State Evidence 记录的 InvalidAggregateArgument 实为 ExprWithAlias 形态经聚合循环产生）；实施后 extract_columns 放行、聚合循环统一报 `InvalidAggregateArgument("Expected column name")`——design D4 明示该报错为保持目标（"保持现有 InvalidAggregateArgument 报错"），spec R4/S3 语义（非聚合项不可与聚合混用）不变。
4. **Act 自身测试断言勘误 2 处**（实现正确、测试作者大小写笔误）：`subquery_mixed_with_expression_rejected`/`join_with_expression_rejected` 断言 `contains("expression")`，实际 additive 文案为大写 "Expression"——改用大小写不敏感匹配。
5. **分类边界解释性记录**（Non-goal 保持现状）：裸 `NULL` 标识符按字面量路由为常量项（D0 怪癖修正同类，无测试锁定）；`SELECT id AS x`（纯列带别名）按普通列走既有路径（纯列 plan 逐字节不变 Preserve 条款，别名怪癖非本 change 范围）。

**Blocker Handoff**

None

**Blocker Resolution**

None

**Self-Review**

- Plan compliance: 三任务契约逐项满足；Preserve/Forbidden 全项核查通过（19 变体外零节点改动、`with_projection`/6 节点 projection 字段未动、既有测试零修改只追加、聚合检测循环判定顺序未改、`resolve_projection_indices` 未改、JOIN 拒绝不静默变形、`value_to_json`/render 未动、ProjectionExecutor 走 owned evaluate）；Invariants 全部保持。
- Full diff reviewed: 是——本 Cycle 11 文件逐块终态复读（含 fmt 后 query.rs 路由块重读）；git diff --numstat 中其余文件（predicate/expression/ddl_dml/cli/recovery 等）为未 commit 的 MS10-T05 与 Iteration 000 现场基线，不属本 Cycle 变更面（与 Iteration 000 Act Response 变更面清单交叉核对一致）。
- Critical findings unresolved: None
- Important findings unresolved: None
- Minor findings unresolved: items 构建的防御性 `_ => ParseError("Unsupported projection item")` 臂实践中不可达（通配已被上方拒绝、QualifiedWildcard/长 CompoundIdentifier 在 extract_columns 即拒）——保留为防御性收口，非问题。

**Verification Evidence**

| 验证项 | 命令 | 输出摘录 | 结论 |
|---|---|---|---|
| 全量测试（fmt 后终态） | `cargo test` | `TOTAL passed:769 failed:0 ignored:2`（747 基线 + projection_expression 16 + cli_test 6；2 ignored 为既有信号标定项） | PASS |
| RED 见证 | 实施前 `cargo test --test projection_expression_test` | `14 failed; 2 passed`——aliased CASE → `Invalid aggregate argument`、UnnamedExpr CASE/COALESCE/CAST → `Unsupported statement type`（各形态错误面在案） | PASS |
| T7 结构见证 | 变体加入后 `cargo build` | `error[E0004]: non-exhaustive patterns: Projection(_) not covered` ×3（correlated.rs:15 / query.rs:24 / pipeline.rs）——接线后 build 0 error | PASS |
| 受影响边界 | `cargo test --test subquery_test --test projection_expression_test --test projection_test --test pushdown_test --test planner_test --test expression_e2e_test --test predicate_test` | 24 + 16 + 6 + 16 + 36 + 24 + 36 全部 0 failed | PASS |
| CLI 渲染 | `cargo test --test cli_test` | `54 passed; 0 failed; 2 ignored`（48 既有 + 6 新增） | PASS |
| Lint | `cargo clippy --all-targets -- -D warnings` | `Finished dev profile`（0 warning，退出码 0） | PASS |
| 格式 | `cargo fmt --check` | `FMT CLEAN`（退出码 0） | PASS |
| OpenSpec | `openspec validate --all` | `Totals: 20 passed, 0 failed (20 items)` | PASS |

**Persisted Evidence**

None required（Plan Mode: none；全部验证可低成本复现，Act Response 承载）。

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook / Incident | None | | |

**Remaining Issues**

1. 已知基线 flaky（非本 change 面，本两轮全量均未复现）：`cli::resolve::tests::test_db_dir_env_cases`（env 竞态，Iteration 000 Review Finding 6 建议）——登记 I 项待用户点名 docs-maintainer 处理。
2. M01 模型条目（PhysicalPlan 19 → 20 变体）与 SNAPSHOT 刷新为 docs-maintainer 在 change 收尾时的义务（本 Cycle Invariants 已注记）。

**Commit or Diff Reference**

未 commit（工作树叠加于未 commit 的 MS10-T05 与 Iteration 000 现场之上，基线 `a5b0a5f`；commit 边界由用户决定）。本 Cycle 变更面：`src/executor/{plan,mod,correlated,projection}.rs`、`src/pipeline.rs`、`src/parser/ast.rs`、`src/parser/planner/{mod,query,subquery}.rs`、`tests/projection_expression_test.rs`（新建）、`tests/cli_test.rs`（追加）。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（不依赖 Act Self-Review）全部通过：

1. **验证复现**：全量 `cargo test` 769 passed / 0 failed / 2 ignored（与 Act 声称一致）；`cargo clippy --all-targets -- -D warnings` 0、`cargo fmt --check` 退出码 0、`openspec validate --all` 20 passed / 0 failed（2026-09-10 Plan 新鲜复跑，含本次 spec 勘误后复验）。
2. **Diff 逐块审查**：本 Cycle 变更面 11 文件与 Act Response 表逐项一致——`Projection(ProjectionNode)` 为第 20 变体（plan.rs）、`ProjectionExecutor` 走 owned `Expression::evaluate`（projection.rs，硬约束遵守，`evaluate_ref` 未触碰）、pipeline/correlated/`get_plan_output_columns` 三臂接通、`ast.rs` 七变体臂 + COALESCE 放行、`query.rs` 聚合检测循环判定顺序未改（表达式分类在 `is_aggregate_expr` 之后的 else-if）、四拒绝面齐全、`projection_indices` 表达式查询置 None、顶层包装在 LIMIT 之上、`is_plain_column_expr` 裸 NULL/两段限定符口径与 Plan 一致。`with_projection` 六处与 6 节点 `projection: Vec<usize>` 字段零修改。
3. **子查询抑制覆盖面核查**：全仓 `build_query(` 递归调用点恰 4 处（subquery.rs:37 IN、:86 EXISTS；query.rs:117 派生表、:279 标量子查询），每处均 save/restore 且错误路径恢复——覆盖完整。Act Response 写"三个递归调用点"为计数笔误（实际 4 处），代码正确，仅记录不处理。
4. **测试文件零修改核查**：`git diff --numstat` 全部测试文件 0 删除（cli_test.rs 的 1 行"删除"为 import 行扩展 `Read` → `{Read, Write}`，属未 commit 的 MS10-T05 restore stdin 用例基线，非本 Cycle）；projection_expression_test 16 测试 + cli_test 追加 6 渲染测试与 Act 清单一致，R4/S1-S4 + R3/S1-S4（SELECT 形态）+ R6 场景映射完整。
5. **二进制语义探针**（独立于测试套件，temp 库实跑 17 项）：R4 别名/Display 表头与行值（P1-P3）、R3 SELECT 形态 searched 缺省 ELSE→NULL / simple operand NULL 不命中 / CAST 矩阵与 NULL 短路（P4-P7）、`SELECT 42` 单列怪癖修正（P8）、四拒绝面显式文案 exit 3（P9/P10/P12）、聚合+表达式 `InvalidAggregateArgument` 保持（P11）、`SELECT *` 与纯列查询逐字节现状（P13/P14）、ORDER BY 基础列组合排序正确（P15）、子查询上下文抑制保持既有报错（P17）——全部符合 spec 与契约。
6. **防御性臂不可达性核实**：`_ => ParseError("Unsupported projection item")` 确不可达——通配已在路由前置检查拒绝、`QualifiedWildcard` 在 `extract_columns` 无臂（`_ => UnsupportedStatement`）先行拒绝。与 Act Self-Review 记录一致。
7. **基线 flaky（非阻塞，非本 change 面）**：`cli::resolve::tests::test_db_dir_env_cases` 本次全量复跑未复现（769/0/2 一次通过）；延续 Iteration 000 Review Finding 6 的登记建议，待用户点名 docs-maintainer 处理。
8. **新增观察（非缺口）**：`WHERE id IN (子查询)`（SemiJoin 计划）+ 表达式项组合可正常工作（P16，行集与表头正确）——非聚合单表查询，符合 R4 requirement 文本，无静默变形；spec 场景矩阵未枚举该组合，不构成 Acceptance gap。

**Deviation Classification**

- **PLAN-OMISSION ×1（非阻塞）**：T8 Change Surface 未预见子查询计划形状消费面被表达式路由破坏（`test_correlated_exists`/`test_correlated_not_exists` 回归暴露），Act 以 `building_subquery` 标志 + 4 处递归调用点 save/restore 修复——属 T8 Preserve 条款「既有全量测试零修改通过」的必要实现面，扩展面（planner/mod.rs、subquery.rs）经本 Review 逐点核实正确且必要，验收语义无损失。
- **PLAN-INVALID ×1（非阻塞，已修正）**：delta spec R6 原文「PhysicalPlan 节点集合保持 19 种」与本 change 自身 design D4（新增 Projection 为第 20 变体）矛盾。Plan 已在本 Review 内勘误 spec 原文（「除按 design D4 新增 `Projection`（第 20 种）外保持 19 种不变」），勘误后 `openspec validate --all` 复验 20/0 PASS。
- **ACT-DEVIATION ×4（全部接受，非实质局部差异）**：① correlated.rs Projection 臂含 items 参数注入（契约仅写"透传 input"）——ParameterExpression 未注入时静默求值 Null，注入与 Iter 000 的 `set_parameter_value` 传播同链，必要；② 聚合+表达式错误文案按 D4 目标态断言（实施前 extract_columns 先拒）——目标错误语义与 spec R4/S3 一致；③ 2 处测试断言大小写勘误（实现正确）；④ 分类边界记录（裸 NULL 常量项、`SELECT id AS x` 走既有路径）——Non-goal 保持现状，代码核实一致。

**Acceptance Gaps**

None——R4/S1-S4、R3/S1-S4（SELECT 形态断言，Iteration 000 Review 裁定归属）、R6/S1-S2 全部有测试断言与探针证据满足；四拒绝面与聚合报错保持经独立探针验证。

**Convergence**

N/A（首次 Review；以本 Cycle Plan Context 为基线）

**Evidence**

- `cargo test` → `TOTAL passed:769 failed:0 ignored:2`（Plan 独立复跑 2026-09-10）
- `cargo clippy --all-targets -- -D warnings` → 0；`cargo fmt --check` → 退出码 0；`openspec validate --all` → `20 passed, 0 failed`（spec 勘误后复验）
- 探针输出（temp 库 17 项）：别名/Display/COALESCE 表头、R3 SELECT 形态值语义、`SELECT 42` 单列、四拒绝面 exit 3、聚合报错保持、`SELECT *`/纯列查询现状、ORDER BY 组合、SemiJoin 组合、子查询抑制
- diff 审查：`git diff --numstat` 测试文件 0 删除（cli_test 1 行 import 扩展属 MS10-T05 基线）；`build_query(` 递归点 4/4 save/restore
- flaky：本次复跑未复现 `test_db_dir_env_cases`

**Follow-up Decision**

接受（accepted）。理由：Acceptance 全满足；1 项 PLAN-OMISSION 与 4 项 ACT-DEVIATION 均为非实质且记录完整、经独立核实；1 项 PLAN-INVALID（spec R6 文本）已由 Plan 在本 Review 内修正。无需当前 Cycle 修复，无 rework/replan Cycle。

**Iteration Plan Update**

None（Iteration Map 不变）

**Next Cycle**

None

**Next Iteration**

None（Iteration 001 为 Map 最后一个 Iteration；change 进入收尾——M01 同步 PhysicalPlan 19→20、SNAPSHOT 刷新、spec 合并由 docs-maintainer 在 change 收尾时执行）
