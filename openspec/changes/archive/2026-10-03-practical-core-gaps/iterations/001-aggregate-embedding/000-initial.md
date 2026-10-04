# Iteration 001 / Cycle 000: 聚合表达式嵌入（SELECT/HAVING/ORDER BY + 常量混排 + 拒绝面收口）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4、T5、T6、T7
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R2 全部 scenario（proposal S6–S12）；design D4；集中决策 3（标准 SQL 面）
- Excluded scope: 聚合进入 WHERE/JOIN ON、嵌套聚合解锁、窗口函数、UPDATE 表达式（Iteration 002）、文档同步（T10）

**Objective**

聚合调用可嵌套于 SELECT 清单值表达式、HAVING 条件表达式、聚合查询 ORDER BY 键；纯常量项与聚合混排合法（SELECT 与 INSERT SELECT 源同语义）；`DISTINCT` 量化词在表达式中保持去重；WHERE/ON 聚合、嵌套聚合、无 GROUP BY 非聚合列引用三个拒绝面 e2e 固化；既有聚合输出形状与套件零回归。

**Investigation Facts**

- Current Baseline: Iteration 000 收尾后工作区（HEAD 9a103c0 + 未提交实施面）；全量 **1809 tests / 0 failures / 2 ignored**（Iteration 000 Act Response M19 全量，Plan Review accepted 采信）；fmt/clippy 干净。Iteration 000 未触碰聚合面文件（其修改面为 table_manager/ddl_dml 建表段/lifecycle），本 Iteration 代码基线与调查时一致。
- Current-State Evidence:
  - **顶层判定不递归**：`src/parser/planner/aggregate.rs` `is_aggregate_expr(expr)` = `matches!(expr, Expr::Function(f) if 名 ∈ {COUNT,SUM,AVG,MIN,MAX,GROUP_CONCAT})`——`MAX(id)+1` 判 false，`Expr::Value` 常量亦非 plain column，二者均落 Expression 角色后在聚合路径被拒（探针：`SELECT MAX(id)+1` → `Unsupported expression type: MAX`，来自 `build_expression` 对未注册聚合名的 `UnsupportedExpression(name)` 臂；`SELECT MAX(id), 1` 的 `1` → `Non-aggregated column '1' must appear in GROUP BY`）。
  - **SELECT 项分类循环**：`src/parser/planner/query.rs:1341-1460`——逐项产出 `SelectItemRole::{Aggregate(name), Column, Expression}`；共享收集器 `aggregates: Vec<AggregateFunc>`、`aggregate_args: Vec<Option<Expr>>`（COUNT(*) → None）、`agg_output_columns`（结果列名 = `result_column_name()` 或别名）、`non_agg_columns`、`has_expression_items`；子查询项跳过（subquery_evals 按索引）；`alias_reroutable` 段仅非聚合基座生效。
  - **聚合计划装配**：`query.rs:1872` `plan_with_aggregate`；`:2148` `agg_plan = AggregateNode { input, group_by, group_key_exprs, aggregates, aggregate_arg_indices, output_columns, table_name, column_indices }`（`plan.rs:585` 字段定义）；其后 `having_pred` → `HavingNode` 包装；再后 `wrap_projection: Option<(Vec<ProjectionItem>, Vec<String>)>` → 条件 `ProjectionNode`（:2171）——混合投影/表达式项在聚合输出行上求值的既有通道（MS13 Iter001 + MS25 T8），`None` = 直出。
  - **HAVING 编译器**：`aggregate.rs` `build_having_expression(expr, output_columns) -> ExpressionRef`——现有臂：Identifier（含 NULL 常量）/ Function（MS25 D7 DISTINCT 感知命名 `count_distinct_x` 型 + 五聚合 `result_column_name` 映射到 output_columns 索引）/ Value / UnaryOp-Minus；`_ => Err(UnsupportedExpression("EXPRESSION"))`（:154）即 `HAVING MAX(id)+1>2` 的撞点。`build_having`（:156+）顶部比较经比较谓词臂、`COMPARISON OPERATOR`/`HAVING PREDICATE` 拒绝臂在 :189/:202。
  - **聚合查询 ORDER BY**：`query.rs:2186` 分支——键经 `extract_column_name` 只编译列名键（`OrderByColumn { column, asc, expr: None }`）；`MAX(id)+1` 为 BinaryOp 无列名通路；校验集 `sort_columns`（聚合查询 = `projection_columns`）∪ `alias_columns`，输出别名键经 MS25 T7 机制回查 `Expr` 并 `build_expression` 编译置 `OrderByColumn.expr: Some`（`plan.rs:11` 注释：`SortExecutor` decorate-sort-undecorate 在未投影输入行上求值）。**装配顺序约束**：ORDER BY 分支（:2186）在 `agg_plan`（:2148）之后——ORDER BY 键引入的**新**聚合调用（如 `ORDER BY MAX(v)` 而 SELECT 无 MAX(v)）须在 `agg_plan` 装配前进入共享 `aggregates` 清单，否则聚合输出行不含该结果列。
  - **执行器**：`src/executor/aggregate.rs`（515 行）——`group_key_exprs` 逐行求值分桶（MS13），聚合按 `aggregates` 序计算，输出行 = `[group_cols..., agg_results...]`；`AggregateFunc` 载荷 `result_column_name()`（含 `count_star`/`sum_x`/`max_x`/`count_distinct_x`/`group_concat_x` 形态）。`SortExecutor`（`sort.rs`）对 `expr: Some` 键在输入行（= 聚合输出行）上求值排序。
  - **拒绝面通道**：WHERE/JOIN ON 内聚合——聚合名不在 `function.rs REGISTRY`，`build_expression`/WHERE 编译器既有 `UnsupportedExpression(name)` 点名（探针同源文案）；嵌套聚合 `MAX(MIN(x))`——提取不进入聚合参数内部，`extract_single_column_arg` 既有 `InvalidAggregateArgument` 点名；非聚合列引用——`NonAggregatedColumn` 既有文案（query.rs :201-312 段）。
- Code and Critical Path: T4 改 `aggregate.rs`（新增 `contains_aggregate_expr` 递归判定 + 提取-改写遍历）与 `query.rs` 分类循环（新角色路由）；T5 改 `aggregate.rs::build_having_expression`（表达式树臂扩展）；T6 改 `query.rs:2186` 分支（表达式键编译 + 校验豁免 + 装配顺序处理）；T7 纯测试收口。依赖链 T4 → T5 → T6 → T7 单向。

**Implementation Guidance**

- 提取-改写遍历（T4）：自顶向下递归；遇聚合 Function 节点（不进入其参数）→ `extract_aggregate_func` 规范化 → 按 `result_column_name()` 在共享 `agg_output_columns` 查重（同名 = 同一调用共享一个输出列，不重复注册 `aggregates`）→ 节点改写为 `ColumnExpression { column_name: 结果列名, column_index: 输出行索引 }`；其余节点按既有表达式构造器递归（BinaryOp 算术/比较、LogicalOp、Nested、CAST、CASE/COALESCE、标量函数、Value、负数字面量）。
- SELECT 项路由（T4）：顶层 `is_aggregate_expr` 命中维持既有路径逐字节不变；否则 `contains_aggregate_expr` 命中 → 新「聚合嵌入表达式项」：提取-改写后按 Expression 角色进入既有 `wrap_projection` 通道（输出名 = 别名优先否则表达式 Display 文本，沿 MS13 命名语义）；**纯常量项豁免**（无聚合 ∧ 无列引用——Identifier/QualifiedIdentifier 出现即非纯常量）→ Expression 角色合法进入聚合输出行求值；无 GROUP BY 的非聚合列引用维持 NonAggregated 拒绝（`non_agg_columns` 既有校验不动）。
- HAVING（T5）：把 `build_having_expression` 的 `_` 兜底替换为完整表达式树编译（聚合 Function 引用臂与 MS25 D7 命名臂原样保持）；可复用 `expression.rs` 既有构造器或原地扩展 match——等价实现选择由 Act 定。顶部比较谓词路径（`build_having`）同步容纳复合左右腿。
- ORDER BY（T6）：聚合查询的键在 `extract_column_name` 之前分流——非裸列键（含聚合调用或常量表达式）经 T5 同一编译器编译置 `OrderByColumn.expr: Some`；ORDER BY 引入的新聚合调用在 `agg_plan` 装配前并入共享清单（把键表达式预扫描并入分类阶段，或在装配点前单独提取——Act 定，行为契约：键中聚合调用在输出行可达）；校验循环对 `expr: Some` 键豁免 `sort_columns` 检查（别名键先例同型）；非聚合查询的 ORDER BY 行为零变化。
- 拒绝面 e2e（T7）：固化探针同源三形态的实际文案（WHERE `MAX(id)>1`、JOIN ON 聚合、`MAX(MIN(x))`、无 GROUP BY 列引用），断言既有文案通道不被本 change 改写。

**Task Contracts**

### T4: 聚合提取基建 + SELECT 项嵌入与常量混排

- Requirement/Scenario: R2（S6 SELECT 嵌套 / S9 常量混排 / S11 DISTINCT 保持）
- Depends on: None
- Targets: `src/parser/planner/aggregate.rs`（新增 `contains_aggregate_expr` 与提取-改写遍历）、`src/parser/planner/query.rs:1341-1460` 分类循环（新角色路由）
- Current behavior: `SELECT MAX(id)+1`、`SELECT COALESCE(MAX(id),0)+1` 计划期 `Unsupported expression type: MAX` (exit 3)；`SELECT MAX(id), 1` → `Non-aggregated column '1'`
- Required behavior: 聚合嵌入表达式项经提取-改写在聚合输出行上求值——`SELECT MAX(id)+1 FROM t` 返回单行 max+1；`COALESCE(MAX(id),0)+1` 同型；`SUM(DISTINCT v)+1` 保持去重；纯常量项（`SELECT MAX(id), 1`）合法，INSERT SELECT 源同语义（`INSERT INTO t2 SELECT MAX(id), 1 FROM t` 两列目标可达）；表达式项表头 = 别名优先否则 Display 文本
- Preserve: 顶层聚合项路径逐字节不变（`aggregates`/`agg_output_columns` 既有装配、直出形状、混合投影包装、别名排序）；子查询项跳过逻辑不动；NonAggregated 拒绝文案不变；无 GROUP BY 列引用拒绝不变
- Forbidden: 不改 `executor/aggregate.rs` 求值内核；不改 `AggregateFunc` 定义与 `result_column_name` 命名；不解锁 WHERE/ON/嵌套聚合
- Test witness: `tests/aggregate_extension_test.rs` 新嵌套矩阵 RED（当前全部计划期拒绝）+ `tests/planner_test.rs` 单元（计划形状：表达式项经 wrap_projection 的 Projection 臂）
- GREEN condition: 新矩阵 GREEN + aggregate_extension_test 既有用例 + aggregate_output_shape_test + aggregate_output_shape 相关直出形状零回归
- Verification: `cargo test --test aggregate_extension_test --test aggregate_output_shape_test --test planner_test -- --test-threads=1`（0 failed）
- Stop when: 提取-改写无法在不改执行器内核的前提下表达（契约前提失效），或顶层聚合项输出形状出现字节级变化——返回 Plan

### T5: HAVING 完整表达式编译

- Requirement/Scenario: R2（S7 HAVING 条件嵌套）
- Depends on: T4（共享提取基建与编译器基座）
- Targets: `src/parser/planner/aggregate.rs::build_having_expression`（`_` 兜底替换为完整表达式树臂）+ `build_having` 比较谓词路径容纳复合腿
- Current behavior: `HAVING MAX(id)+1 > 2` → `Unsupported expression type: EXPRESSION`
- Required behavior: HAVING 条件表达式可嵌套聚合与算术——组过滤按 `max+1 > 2` 语义生效；AND/OR、CASE/COALESCE/CAST、标量函数腿可达
- Preserve: Identifier/聚合引用/常量/负数字面量既有臂语义与错误面不变；`ColumnNotFound`/`HavingNonAggregatedReference` 既有拒绝通道不变
- Forbidden: 不改 HavingNode/执行器；不把聚合引用语义改为重求值（仍为输出列引用）
- Test witness: `tests/aggregate_extension_test.rs` HAVING 矩阵 RED
- GREEN condition: 矩阵 GREEN + 既有 HAVING 用例零回归
- Verification: `cargo test --test aggregate_extension_test -- --test-threads=1`（0 failed）
- Stop when: 比较谓词臂无法容纳复合腿且需改执行器——返回 Plan

### T6: ORDER BY 聚合表达式键

- Requirement/Scenario: R2（S8 ORDER BY 聚合表达式）
- Depends on: T5（同一编译器）
- Targets: `src/parser/planner/query.rs:2186` 聚合 ORDER BY 分支（键分流 + `OrderByColumn.expr` 装配 + 校验豁免 + 装配顺序处理）
- Current behavior: 聚合查询 `ORDER BY MAX(id)+1` / `ORDER BY MAX(v)` 无编译通路（计划期报错）
- Required behavior: 非裸列键经编译器置 `OrderByColumn.expr`，SortExecutor 在聚合输出行上求值排序生效；键中聚合调用（含 SELECT 未出现的新调用，如 `ORDER BY MAX(v)`）在输出行可达
- Preserve: 裸列键 / 别名键既有路径逐字节不变；非聚合查询 ORDER BY 零变化；`sort_columns` 校验对列名键保持
- Forbidden: 不改 SortExecutor；不改非聚合 ORDER BY 分支
- Test witness: `tests/aggregate_extension_test.rs` 排序矩阵 RED
- GREEN condition: 矩阵 GREEN（`ORDER BY MAX(id)+1` 与 `ORDER BY MAX(v)` 两形态）+ 既有排序用例（order_by_alias_sort_test）零回归
- Verification: `cargo test --test aggregate_extension_test --test order_by_alias_sort_test -- --test-threads=1`（0 failed）
- Stop when: ORDER BY 新聚合调用无法在 `agg_plan` 装配前进入共享清单（结构约束失效）——返回 Plan

### T7: 拒绝面保持与聚合 e2e 收口

- Requirement/Scenario: R2（S10 拒绝面 / S12 既有形状零回归）
- Depends on: T4、T5、T6
- Targets: `tests/aggregate_extension_test.rs`（拒绝面 e2e 固化）+ 既有聚合五套件回归（aggregate_extension / aggregate_output_shape / join_output_resolution / group_by_expr / order_by_alias_sort）
- Current behavior: WHERE/ON 聚合、嵌套聚合、NonAggregated 列引用各有既有拒绝通道（探针与代码定位见 Investigation Facts）
- Required behavior: 三拒绝面在新路由下保持——e2e 断言文案通道不被改写；既有聚合套件全 GREEN
- Preserve: 既有文案逐字节；直出/包装分形输出列名
- Forbidden: 不新增拒绝形态（本 change 只固化不扩展拒绝面）
- Test witness: 拒绝面用例先行 GREEN（固化现状），回归套件在 T4–T6 全部落地后零失败
- GREEN condition: 拒绝面用例 GREEN + 五套件 0 failed
- Verification: `cargo test --test aggregate_extension_test --test aggregate_output_shape_test --test join_output_resolution_test --test group_by_expr_test --test order_by_alias_sort_test -- --test-threads=1`（0 failed）+ fmt/clippy
- Stop when: 任何既有拒绝文案被新路由改写（Invariants 违反）——返回 Plan

**Invariants**

- 顶层聚合项、直出形状、混合投影包装、别名排序、`result_column_name` 命名全部逐字节保持。
- 拒绝面诚实化：WHERE/ON 聚合、嵌套聚合、NonAggregated 列引用的既有文案通道不改写。
- 聚合引用在 HAVING/表达式/排序键中均为「聚合输出行上的列引用」，不引入二次求值。
- 全量基线 1809 passed / 0 failed / 2 ignored 不回归（本 Iteration 净增用例）。

**Non-goals**

聚合进入 WHERE/JOIN ON；嵌套聚合解锁；窗口函数；递归 CTE；UPDATE 表达式（Iteration 002）；文档同步（T10）。

**Acceptance**

- S6：`SELECT MAX(id)+1` / `SELECT COALESCE(MAX(id),0)+1` 单行正确结果、表头命名语义正确。
- S7：`GROUP BY v HAVING MAX(id)+1 > 2` 过滤正确。
- S8：`GROUP BY v ORDER BY MAX(id)+1`（及 `ORDER BY MAX(v)`）排序生效。
- S9：`SELECT MAX(id), 1` 合法；INSERT SELECT 两列目标写入可达。
- S11：`SUM(DISTINCT v)+1` 去重保持。
- S10：WHERE/ON 聚合、嵌套聚合、NonAggregated 三拒绝面 e2e 固化（既有文案）。
- S12：既有聚合五套件零回归。
- 映射：R2→T4/T5/T6/T7（tasks.md RTM）；场景→design D4→Targets→测试文件链路闭合。

**Verification**

- S6/S9/S11：aggregate_extension_test 嵌套矩阵（自动）。
- S7：HAVING 矩阵（自动）。
- S8：排序矩阵（自动）。
- S10：拒绝面用例（自动，断言既有文案含能力名）。
- S12：五套件定向回归 0 failed + fmt/clippy 干净（自动）。
- Iteration 收尾：定向全量分批（lib + 受影响集成目标）0 failed。

**Gate 2 Readiness**

PASS——授权依据同 Iteration 000（用户原始指令 + Gate 1 四项集中决策；本 Cycle 契约为已批准计划 tasks 2.1–2.4 的展开，无新增用户面向选择）。Gate 2 检查：调查完整（本文件 Investigation Facts 含全部 file:line 锚点与装配顺序约束）、设计闭合（D4 无 TBD）、任务可执行（T4–T7 契约含变更面/见证/停止条件）、依赖单向、验证直接、Persisted Evidence none、无 BLOCKED/WAIVED。已知风险：与非实质实现选择（编译器复用形态、ORDER BY 预扫描落点）均已显式留 Act。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- ORDER BY 分支在 `agg_plan` 装配之后（:2186 vs :2148）——ORDER BY 引入新聚合调用时的装配顺序是本 Iteration 最可能触发 Stop-when 的结构点，Implementation Guidance 已给两条可行路径（分类期预扫描 / 装配点前提取）。
- `SELECT MAX(id), MAX(id)`（同一调用两次出现）经去重共享一个输出列——输出形状与今日（两同名输出列）可能有别，属新路由下的合理形态；既有套件无此形态用例，不构成回归。
- 常量豁免的判定（无列引用）对 `Expr::TypedString` 日期字面量同样适用（常量）——Act 在矩阵中补一例。
- 非实质留 Act：编译器内部函数形态、`SelectItemRole` 是否新增变体或复用 Expression、测试夹具形态。

## Act Response

- Status: reported

**Implemented**

- T4（R2，S6/S9/S11）`src/parser/planner/aggregate.rs`：新增 `contains_aggregate_expr` 递归判定（自顶向下、不进聚合参数、不下探子查询）与 `register_aggregate_calls` 提取-注册遍历（`extract_aggregate_func` 规范化、按结果列名在显示名清单去重共享输出列）；新增 `AggRowExprMode { Having, OutputRow }` 与 `PlanBuilder::compile_aggregate_row_expr` 聚合输出行编译器（算术/COALESCE/标量函数/CAST/CASE/TRIM/CEIL/FLOOR/TypedString/常量沿 `expression.rs` 既有构造语义递归，分支条件经 `build_having` 编译为聚合行谓词）；既有 Function 命名逻辑原样提取为 `aggregate_ref_result_column` 共享。`src/parser/planner/query.rs`：SELECT 项分类扩展「聚合嵌入表达式项」臂（注册嵌套聚合 + 项交 wrap 编译）与纯常量项臂（`is_pure_constant_expr`：无列引用/子查询/INTERVAL）；MS13 分组键绑定校验豁免两类项；`node_output_columns` 统一为「分组键名 ++ 注册序显示名」公式（`aggregate_display_names` 与 `aggregates` 平行；无 T4/T6 项时与既有两形态逐字节一致，直出形态经键序/子集强制条件证明等价）；wrap 装配 Aggregate 臂改按显示名定位（消除角色序≠注册序错位）、Expression 臂分流 T4 项编译；`aggregate_arg_expr` 提为 `pub(crate)` 共享。
- T5（R2，S7）`src/parser/planner/aggregate.rs`：`build_having_expression` 整体委托 `compile_aggregate_row_expr(Having)`——既有四臂（Identifier `ColumnNotFound`/聚合引用 `HavingNonAggregatedReference`/Value/负数字面量 `UnsupportedValue`）语义与拒绝通道逐字节承载，`_` 兜底（原 `UnsupportedExpression("EXPRESSION")`）替换为完整表达式树臂（算术、AND/OR 腿经既有 `build_having` 谓词层、Nested、CAST、CASE、COALESCE、TRIM/CEIL/FLOOR、标量函数、TypedString）。
- T6（R2，S8）`src/parser/planner/query.rs`：聚合查询 ORDER BY 键预扫描（`has_aggregates` 时键中聚合调用经 `register_aggregate_calls` 在 agg_plan 装配前进入共享清单——`ORDER BY MAX(v)` 而 SELECT 无 MAX(v) 可达）；ORDER BY 分支非裸列键（`extract_column_name` 失败）分流至 `compile_aggregate_row_expr(OutputRow)` 产出 `OrderByColumn.expr`（SortExecutor 既有 decorate-sort-undecorate 消费）；表达式键豁免列集校验与 DISTINCT 键收窄（`ob.expr.is_some()` 先例同型）；表达式排序键存在时条件包装延迟到 Sort 之上装配（`deferred_wrap`——Sort 落于聚合输出行，包装出 SELECT 清单形状），`sort_columns` 相应取聚合行名集；直出判定加「注册序显示名数 == SELECT 聚合角色数」强制条件（ORDER-BY-only 聚合与同调用去重使直出形状会泄漏未选列，强制包装裁剪）。非聚合查询 ORDER BY 全路径零变化。
- T7（R2，S10/S12）`tests/aggregate_extension_test.rs`：五拒绝面 e2e 固化（WHERE 聚合 / JOIN ON 聚合 → `Unsupported expression type: MAX`；嵌套聚合两形态 → `Invalid aggregate argument`；无 GROUP BY 非聚合列引用与表达式内嵌套非聚合列引用 → `Non-aggregated column`）。
- 新测试：`tests/aggregate_extension_test.rs` +21（T4 嵌套矩阵 7〔含 TypedString 常量、INSERT SELECT 源、SUM(DISTINCT) 表达式、分组嵌入〕+ T5 HAVING 矩阵 6 + T6 排序矩阵 3 + T7 拒绝面 5）；`tests/planner_test.rs` +3（wrap Projection 计划形状 + 输出名 Display/别名 + 常量混排名）。
- 语义校准（预期后果白名单，2 处既有测试，均带 change 注记）：`tests/projection_expression_test.rs::aggregate_expression_error_preserved` 输入改为含列引用的未分组表达式（原 `CASE WHEN 1 = 1 THEN 'x' END` 系纯常量，现按 R2/S9 混排合法——新行为由 `constant_mixing_with_aggregate` 见证），拒绝断言保持；`tests/scalar_function_test.rs::having_scalar_function_rejected` → `having_scalar_function_reachable`（T5 标量函数 HAVING 腿解锁的语义翻转见证，R5 改名承载连续性先例）。

**Deviations from Plan**

- T4「提取-改写遍历」实现为「提取-注册遍历 + 聚合输出行编译器」：注册在分类期完成、wrap 阶段编译器直接消费原始 Expr（聚合 Function 节点原生解析为输出列引用），不构造中间改写树。等价实现（Implementation Guidance 明示内部形态留 Act）；「节点改写为 ColumnExpression」的契约语义由编译器的聚合引用臂承载。
- T5 测试初稿越界（HAVING 引用 SELECT 未注册的 `MIN(v)`/`MAX(id)`）→ 触发既有 `HavingNonAggregatedReference` 拒绝；按 Preserve 条款修正测试（聚合须在 SELECT 清单注册）。边界澄清：本 change 不解锁 HAVING 引入新聚合（与 T6 仅 ORDER BY 键预扫描注册形成对照），既有拒绝通道逐字节保持。
- T6 deferred wrap 装配顺序机制取 Guidance 两条路径的「分类期预扫描」臂；「直出强制条件」（注册数≠角色数）为实施中发现的结构点——计划未预见 ORDER-BY-only 聚合使直出形状泄漏未选列，处理属契约内（「键中聚合调用在输出行可达」+ 输出形状 = SELECT 清单不变量）的实现细化，非范围扩展。
- 瞬态工程态：T4 完成点 clippy 报 `AggRowExprMode::Having` dead-code（旧 build_having_expression 臂未拆除、T5 的 RED 观察所需）；T5 委托后消除，cycle 收尾 clippy 0 告警。

**Self-Review**

- 产品 diff 与 Task Contract 逐一对应；无计划外修改；无新警告、死代码、身份型证据机制。
- 遗留 Minor（记录不返工）：① 聚合查询中 `SELECT FOO('x'), MAX(id)` 类未注册函数常量形态的拒绝通道由 NonAggregated 变为 `UnsupportedExpression("FOO")`（无既有用例锚定，均计划期点名拒绝）；② 聚合查询 `ORDER BY NULL` 现为常量键 no-op 排序（原「ORDER BY only supports column names」拒绝；非聚合查询维持原拒绝）；③ 表达式排序键 × DISTINCT 组合的键不再要求 ∈ SELECT 清单（Sort 先于 Distinct 求值，未在验收矩阵）；④ CASE 分支条件内标识符经 HAVING 通道解析（`ColumnNotFound`）而非 OutputRow 通道（`NonAggregatedColumn`）——拒绝通道差异，均计划期点名。
- 无未解决 Critical/Important。

**Verification Evidence**

- T4 RED：`cargo test --test planner_test -- --test-threads=1 aggregate_embedded aggregate_constant` → 2 FAILED，`UnsupportedExpression("MAX")`；`cargo test --test aggregate_extension_test -- --test-threads=1 nested_aggregate constant_mixing sum_distinct_inside grouped_nested` → 7 FAILED。
- T4 GREEN：`cargo test --test planner_test --test aggregate_extension_test -- --test-threads=1` → `20 passed; 0 failed` / `20 passed; 0 failed`；定向回归 projection_expression/scalar_function/subquery/insert_select/ctas 全 `0 failed`（projection_expression 1 处校准后 `17 passed`）。
- T5 RED：`cargo test --test aggregate_extension_test -- --test-threads=1 having_` → 6 FAILED，`Unsupported expression type: EXPRESSION`。
- T5 GREEN：同目标 → `6 passed`；scalar_function_test `32 passed`（校准用例翻转后）；aggregate_output_shape/group_by_expr/projection_expression/join_output_resolution `0 failed`。
- T6 RED：`cargo test --test aggregate_extension_test -- --test-threads=1 order_by_aggregate` → 3 FAILED，`Parse error: ORDER BY only supports column names`。
- T6 GREEN：aggregate_extension_test 全量 `29 passed`（首跑暴露直出形状泄漏未选列 1 失败，修复后全绿）；aggregate_output_shape/group_by_expr/order_by_alias_sort/planner_test `0 failed`。
- T7 拒绝面先行 GREEN（固化现状）：`-- --test-threads=1 rejected` → `7 passed`。
- Cycle 收尾（M19 全量分批：112 集成目标按名字典序 8 批 + `--lib` + `--bins`，串行 `--test-threads=1`，全部退出码 0）：集成 1439 passed + `--lib` `394 passed; 0 failed` + `--bins` ok，全量 **1833 tests / 0 failures / 2 ignored**（1809 基线 + 净增 24：aggregate_extension_test +21、planner_test +3；cli_test 2 ignored 不变）。
- `cargo fmt` 干净；`cargo clippy --all-targets` 0 警告；`openspec validate --changes 2026-10-03-practical-core-gaps` → `✓ 1 passed, 0 failed`。

**Persisted Evidence**

- None required（plan 模式 none；全部验证可低成本重跑，Act Response 命令 + 输出足以承载）。

**Experience Candidates**

- None（全部验证为既有 cargo 命令常轨；范围内结构点均为 change 内实现细节，不构成范围外缺陷）。

**Remaining Issues**

- 无未解决问题。已知边界（HAVING 不引入新聚合、表达式键 × DISTINCT、CompoundIdentifier 表达式键拒绝）由 Self-Review Minor 项与 design D4 承载，文档面归 Iteration 002 T10。

## Plan Review

- Review Result: accepted

**Findings**

1. **（实质、不阻塞 S6–S12 验收——Act 实现选择引入的边界回归，按用户指令并入 Iteration 002 以 T12 修复）** 聚合输出引用解析在显示名重名时静默取错值：T4 将条件包装的聚合项定位由旧位置序（`agg_seen` 计数）改为显示名查表（`query.rs` wrap Aggregate 臂 `aggregate_display_names.iter().position(eq_ignore_ascii_case)` 首匹配），`compile_aggregate_row_expr` 的聚合 Function 引用臂同按显示名 `position` 首匹配。显示名互异时两法等价（Review 核对：顶层聚合角色按 SELECT 序注册、role name 恒在 display names 中，`.expect` 不可达）；重名（重复别名或别名撞 `result_column_name`）时首匹配歧义。定向探针实测（`target/debug/rtsql` 含本 Cycle 实施面，2026-10-03）四形态取错值：① `SELECT MAX(a) AS m, g, MAX(b) AS m FROM t GROUP BY g` → 第三列 20/30（max(a) 值，应为 200/300）；② `SELECT MAX(a) AS max_b, g, MAX(b) FROM t GROUP BY g` → 第三列 20/30；③ 同② + `HAVING MAX(b) > 150` → 空结果（按 max(a) 过滤，应两组均保留）；④ 同② + 嵌入 `MAX(b)+1` → 21/31（应为 201/301）。异名对照形态（`AS m / AS mb`）正确。HEAD 位置序机制对四形态均正确——本 change 引入的回归，违反 T4 Preserve「混合投影包装不变」。分类：Plan 遗漏（调查与 Preserve 条款未枚举重名形态）× Act 实现选择（名字查表）叠加；修复面在本 Cycle 变更面内、机制明确（design D8），不需要新调查或新执行契约。用户 2026-10-03 指令「给出下一轮iter并把遗留issue记入一起解决」授权并轮修复（非返工往返）；已按该指令落入 Iteration 002 任务 T12（RTM 映射 R2/S24）。
2. **（Minor，记录不返工）** Act Self-Review 四项（未注册函数常量形态拒绝通道变化、聚合查询 `ORDER BY NULL` 变常量键 no-op、表达式排序键 × DISTINCT 键不受 SELECT 清单约束、CASE 分支条件经 HAVING 通道解析）核对属实，均计划期点名、无既有用例锚定，随 T10 文档面承载。
3. **（Minor，记录不返工）** T5 测试初稿越界（HAVING 引用 SELECT 未注册聚合）触发既有 `HavingNonAggregatedReference`，按 Preserve 修正测试——边界澄清正确：HAVING 不引入新聚合，与 T6 仅 ORDER BY 键预扫描注册形成对照。
4. **（Issue 候选处置记录）** Iteration 000 遗留的 drop 不 close 恢复错误面候选，经用户同一指令并入本 change 为 R6/T11（design D7、delta spec `wal-recovery-replay-integrity` ADDED Requirement）——不再以「范围外候选」报告。

偏差分类：Finding 1 为 Plan 遗漏 × Act 实现选择（不阻塞已列验收场景）；其余无 Act 偏离、无基线变化。Act Response 声明与代码逐项核对一致：`aggregate.rs`（编译器/AggRowExprMode 模式化拒绝通道/提取-注册遍历/`aggregate_ref_result_column` 命名原样提取/`_` 兜底收窄为 EXPRESSION）、`query.rs`（display names 平行清单、agg_row_expr_items、T6 预扫描、分组键校验豁免、直出强制条件 `display.len() == role_agg_count`、统一输出列公式、wrap Aggregate 按名定位、deferred wrap、表达式键校验豁免、`aggregate_arg_expr` pub(crate)、`is_pure_constant_expr`）与声明逐一吻合；两处既有测试校准带 change 注记且行为断言未弱化（projection_expression 改用含列引用输入保持拒绝语义、scalar_function 语义翻转见证改名承载）。

**Acceptance Gaps**

无——S6–S12 全部有 GREEN 见证（Act Verification Evidence 2026-10-03，覆盖面未失效，采信；RED→GREEN 链完整：T4 嵌套/常量/混排 7、T5 HAVING 6、T6 排序 3、T7 拒绝面 5 先行固化）。全量 1833/0/2、fmt/clippy 干净、`openspec validate` 通过。Finding 1 形态不在已列验收场景与既有套件内，作为回归并轮修复（T12），不构成本 Iteration 验收缺口。

**Evidence**

- 独立审读完整 diff：`src/parser/planner/aggregate.rs`（690 行变更）、`src/parser/planner/query.rs`（309 行变更）、`tests/aggregate_extension_test.rs`（+21 用例计数核对）、`tests/planner_test.rs`（+3，另 1 例属 Iteration 000 T2）、两处校准文件。
- 定向探针（验证规则 › 疑点审计）：四形态错误值 + 异名对照正确 + 共享输出列形态，见 Finding 1。
- Act 测试运行结论采信（来源：Act Response Verification Evidence，2026-10-03；结论与 diff 一致、被覆盖表面在结论产生后未变化——Plan 探针仅新增 Finding 1 形态观察，不动摇已采信结论）。

**Follow-up Decision**

接受：S6–S12 验收全满足，无阻塞 finding。Finding 1 作为显式任务 T12 并入 Iteration 002（design D8、delta spec `sql-aggregate-extensions` 补强句与 S24 场景、RTM R2 行扩 Task），修复契约已就绪；不创建 rework Cycle——该形态不阻塞本 Iteration 已列验收，且用户指令授权并轮收口。Finding 4 Issue 候选经用户裁定并入 R6/T11。Iteration 001 完成。

**Iteration Plan Update**

（无——本 Iteration Map 不变；Iteration 002 Map 的 T11/T12 扩充属展开前修订，见 tasks.md 与 proposal 集中决策 5。）

**Next Cycle**

None

**Next Iteration**

`iterations/002-update-recovery-docs/000-initial.md`（已按修订后 Map 展开，Plan Context 就绪）
