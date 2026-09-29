# Iteration 001 / Cycle 000: 输出行代数——DISTINCT / 集合操作 / 聚合扩展

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2（SELECT DISTINCT）、T3（集合操作）、T4（聚合扩展）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `select-distinct` R1–R3、`sql-set-operations` R1–R4、`sql-aggregate-extensions` R1–R4 全部场景；design D3（Distinct 节点与三分支链位）、D4（SetOperation 展开与执行器）、D7（聚合 DISTINCT 消费与 group_concat）；proposal 决策记录（SQLite 对齐基准，用户批准 2026-09-26）；change 级 Gate 2（2026-09-26，含本 Iteration 计划）
- Excluded scope: T1（Iteration 000 已 accepted）；T5/T6（Iteration 002）、T7（Iteration 003）；INTERSECT ALL/EXCEPT ALL 与 ByName 族（点名拒绝）；DISTINCT × 标量子查询项/× 复合语句（点名拒绝）；递归 CTE；窗口函数；跨引用共享物化

**Objective**

SELECT DISTINCT 三分支链位端到端（修复 `query.distinct` 静默忽略——重复行照常返回的缺陷）；UNION [ALL]/INTERSECT/EXCEPT 复合查询端到端（替换既有 `UnsupportedStatement("SET OPERATION")` 全量拒绝面）；`COUNT/SUM/AVG(DISTINCT …)` 真实去重（修复聚合路径 `distinct` 量化词静默忽略缺陷）与 group_concat 可达；既有查询与聚合面零回归；全量串行零回归。

**Investigation Facts**

- Current Baseline: Iteration 000 最终 Act Response（本会话，accepted Review 采信）——OUTER JOIN 三态 + WHERE×JOIN 解禁落地（`NestedLoopJoinType::{Inner,Left,Full}`、RIGHT=Left 交换重写、`ConstantPredicate`、D2 组合布局 `Filter(Join 全列)` + Sort 投影三态归属），全量串行 1301 passed / 0 failed / 2 ignored（基线 1285 + 新增 16）；`planner_test` 87、`cli_test` 111/2i 零校准。Rust 代码面：本 Iteration 目标文件中 `aggregate.rs`（executor 与 planner 两处）、`parser/ast.rs`、`executor/value.rs` 自基线验证后零变化（git diff 核实）。
- Current-State Evidence:
  - **DISTINCT 静默面**：`Query.distinct: Option<Distinct>`（sqlparser 0.44，`Distinct::{Distinct, On(Vec<Expr>)}`）在 planner 全无消费（grep 零匹配）——`SELECT DISTINCT` 静默不去重。`SELECT ALL` 同样零消费（语义恰为不去重，接受即无操作）。
  - **集合操作现状**：`src/parser/ast.rs:25` `extract_select_body` 只放行 `SetExpr::Select`，其余 → `UnsupportedStatement("SET OPERATION")`；`build_query`（query.rs:517）在 :523 调用它——复合语句的统一拒绝点。`SetExpr::Query(Box<Query>)` = 括号化分支。sqlparser 左结合嵌套：`a UNION b INTERSECT c` 解析为 `SetOperation{Union, left: a, right: SetOperation{Intersect, b, c}}`（INTERSECT 嵌套更深 = 更高优先级，计划形状测试见证）。
  - **聚合扩展面**（两文件 Iteration 000 未触碰，行号有效）：`src/executor/aggregate.rs:14` `AggregateFunc{CountStar,Count,Sum,Avg,Min,Max}`、:31 `AggregateState`（累积器枚举）、:48 `result_column_name()`（`count_{col}` 等输出名）、:62 `new`/:84 `update`/:149 `finalize`、:266 `extract_value`（按列名查 `column_indices`）；:178 `AggregateExecutor`（HashMap 分桶 + `output_rows` 物化）。`src/parser/planner/aggregate.rs:179` `is_aggregate_expr`（名单 COUNT/SUM/AVG/MIN/MAX——GROUP_CONCAT 不在名单，现走标量函数路径报未知名）、:187 `extract_aggregate_func`（**不读 `f.distinct`**——`COUNT(DISTINCT x)` 静默按 `COUNT(x)` 计算的缺陷点；`COUNT` 分支只认 `*` 与单列 `Expr`，多参数落 `InvalidAggregateArgument`）。
  - **链装配锚点（Iteration 000 后重定位）**：`build_query` query.rs:517；子查询探测 :537；聚合检测 :623；T4a 严格投影解析 :858；`proj_or_empty` :882；WHERE 臂 :893（D2 组合布局 :898+）；ORDER BY :1299（T6 校验 :1340、`alias_columns` :1351）；LIMIT :1381；SubqueryEval 包装 :1420；顶层 Projection 包装 :1439。链序：`base → [Sort] → [Limit] → [SubqueryEval] → [Projection]`（各段条件装配）。
  - **输出形状事实（链位规则依据）**：纯列清单无 ORDER BY → per-node 投影（DataScan/Filter/IndexScan `projection`）已裁剪为输出形状；有 ORDER BY → Sort 持投影（D10）→ Sort 之上为输出形状；表达式项 → 顶层 Projection 求值裁剪 → Projection 之上为输出形状；聚合 → Aggregate/Having 输出形状（混合投影经条件包装 Projection）。**Distinct SHALL 插在「输出形状已就绪」的位置**，LIMIT 须在其上（去重后行数截断）。
  - **去重键基础**：`Value`（`src/executor/value.rs:49-93`）`PartialEq + Eq + 手动 Hash`（discriminant 区分变体、`Float` 按 `to_bits`、`Null == Null` 为真且 Hash 一致）——`HashSet<Vec<Value>>` 直接可用，NULL 相等折叠无需新语义。
  - **管道构造**：`src/pipeline.rs` `create_executor_from_plan`（新节点需新臂，DerivedScan 臂 :832+ 为「构造期急切物化」先例）；`get_plan_output_columns`（query.rs:46）需为新节点加臂；`is_cacheable`（pipeline.rs:1212）仅 `Statement::Query` ——复合语句整体是 `Statement::Query`，可缓存（计划含全部侧臂，无跨执行状态）。
  - **执行器先例**：`DistinctExecutor`/`SetOperationExecutor` 均为纯内存物化型（同 `DerivedScanExecutor`/`SortExecutor` 形态：`next()` 首调消费输入 → 物化 → 按序产出），无存储访问（M17 不触及）；`ExecResult::Row` 流约定与 `NestedLoopJoinExecutor` 右输入物化同型。
  - **测试入口**：`tests/outer_join_test.rs`（Iteration 000 形态先例：TempDir + `Database::open` + `query_rows`/`error_msg`/`plan_of` helpers）；`tests/planner_test.rs`（87，计划形状断言）；`tests/cli_test.rs`（111/2i）；聚合既有面 `tests/group_by_expr_test.rs`（12）、`tests/expression_e2e_test.rs`（聚合矩阵）、`tests/scalar_function_test.rs`（32）。
- Code and Critical Path: 新增 `src/executor/distinct.rs`（`DistinctNode` 进 `plan.rs` + `DistinctExecutor`）、`src/executor/set_operation.rs`（`SetOperationNode` + 执行器，`SetOpKind{Union,Intersect,Except}`）；planner：`query.rs::build_query` 顶部 body 分派改造（:523 处 `extract_select_body` 前置 `SetExpr::SetOperation`/`Query` 分流）+ DISTINCT 链位装配（:1381 LIMIT 段与 :1439 Projection 段重构）+ `get_plan_output_columns` 两新臂；`parser/aggregate.rs`（planner）DISTINCT/GROUP_CONCAT 消费；`executor/aggregate.rs` 新变体全套；`pipeline.rs` 两构造臂；`executor/mod.rs` 导出。测试新文件 `tests/select_distinct_test.rs`、`tests/set_operations_test.rs`、`tests/aggregate_extension_test.rs`。

**Implementation Guidance**

实施顺序（TDD，建议 T4 → T2 → T3——T4 独立最小先行，T3 最后组合复用；等价顺序不阻塞）：① `tests/aggregate_extension_test.rs` 先行——`count_distinct_basic`/`sum_distinct`（现状静默按普通聚合计算，RED）与 group_concat（现状未知名错误，RED）；② planner `extract_aggregate_func` 消费 `f.distinct`（COUNT/SUM/AVG → 新变体；MIN/MAX → 丢弃 flag 映射普通形态；`COUNT(DISTINCT a,b)` 多参数 → `InvalidAggregateArgument` 点名）+ GROUP_CONCAT 注册（1-2 参数、第二参数须 `Expr::Value(String)` 字面量、`is_aggregate_expr` 名单扩充）+ executor 新 `AggregateFunc`/`AggregateState` 变体（HashSet 去重累积器跳过 NULL；GroupConcat 累积 String 部件、全 NULL 组 finalize → `Null`；`result_column_name`：`count_distinct_{col}`/`sum_distinct_{col}`/`avg_distinct_{col}`/`group_concat_{col}`）；③ `tests/select_distinct_test.rs`——`distinct_single_col`（现状静默 3 行，RED）等；④ `DistinctNode`/`DistinctExecutor` + 链位装配（三分支规则见 T2 契约）+ `get_plan_output_columns`/pipeline 臂；⑤ `tests/set_operations_test.rs`；⑥ `build_query` body 分派 + `build_set_operation` 左结合展平（侧臂经合成 `Query{body: SetExpr::Select(leaf), order_by: [], limit: None, …}` 复用 `build_query`——侧臂可含 JOIN/聚合/表达式项/外连接，全部既有装配复用）+ `SetOperationNode`/执行器 + 复合 ORDER BY/LIMIT（Sort 键 = 左侧列名，既有裸名限制）+ 拒绝面（列数/ByName 族/INTERSECT ALL/EXCEPT ALL/括号化侧臂/顶层 `query.distinct` × 复合/子查询上下文复合）；⑦ 定向套件 + 全量串行。

测试夹具要点：DISTINCT 矩阵（单列/多列元组/NULL 折叠/表达式求值后去重/×ORDER BY 输出项/×ORDER BY 非输出项拒绝/DISTINCT ON 拒绝/ALL 恒等/×JOIN 组合〔Iteration 000 面〕）；集合操作矩阵（UNION 去重/UNION ALL 保序保重/NULL 相等/INTERSECT/EXCEPT/EXCEPT NULL 匹配/链式三分支/INTERSECT 优先级计划形状/列数不符/INTERSECT ALL 与括号化拒绝/复合 ORDER BY/×WHERE×JOIN 侧臂）；聚合矩阵（COUNT/SUM/AVG DISTINCT + 全 NULL/多参数拒绝/MIN MAX DISTINCT 等价/group_concat 四形态/GROUP BY 组合/既有聚合零校准）。

**Task Contracts**

### T2: SELECT DISTINCT（`select-distinct` R1–R3）

- Requirement/Scenario: R1 去重四场景（单列/多列元组/NULL 折叠/表达式求值后）、R2 组合两场景（×ORDER BY 输出项/非输出项拒绝）、R3 拒绝与零回归三场景（DISTINCT ON/ALL 恒等/既有零回归）
- Depends on: None（Iteration 000 已交付的 JOIN/组合形状为组合场景输入）
- Targets: `src/executor/plan.rs`（`DistinctNode`）、`src/executor/distinct.rs`（新，`DistinctExecutor`）、`src/executor/mod.rs`（导出）、`src/parser/planner/query.rs`（`build_query` DISTINCT 消费 + 链位装配 + `get_plan_output_columns` Distinct 臂）、`src/pipeline.rs`（构造臂）、`tests/select_distinct_test.rs`（新）
- Current behavior: `query.distinct` 全无消费——`SELECT DISTINCT` 静默返回重复行（`select_distinct_single_col` 现状 RED：3 行）；`SELECT ALL` 同样静默（语义恰为既有行为）；`DISTINCT ON(...)` 落入既有表达式拒绝兜底
- Required behavior: ① `SELECT DISTINCT <投影项>` 按输出行去重（完全相同输出行保留首现行；NULL 彼此相等折叠；表达式项按求值后值去重）；② 链位三分支——纯列清单（无表达式/子查询项）：`base → [Sort] → Distinct → Limit`；表达式项（含别名重路由）：`base → [Sort] → Projection → Distinct → Limit`（Limit 上移至 Distinct 之上）；标量子查询项 × DISTINCT：计划期点名拒绝；③ DISTINCT 查询 ORDER BY 键 SHALL ∈ 输出列集（输出名/别名），非输出项计划期点名拒绝（Sort 在 Distinct 之下时键解析宽形状，须额外校验保证「先去重后排序」等价）；④ `DISTINCT ON(...)` → 计划期点名拒绝；`SELECT ALL` → 显式无操作（等价既有不去重）；no-FORM SELECT + DISTINCT → 无操作接受（单行去重恒等，SQLite 对齐）；⑤ DISTINCT × 聚合（含混合投影包装）按输出行去重可达
- Preserve: 无 DISTINCT 查询全路径逐字节不变（`query.distinct == None` 时零新逻辑）；plan cache 键语义不变（SQL 文本自带 DISTINCT）；`Value` Eq/Hash 语义零变化；既有错误面文案不变；M17/M19
- Forbidden: 不在 Distinct 节点下推谓词或改写扫描；不实现 DISTINCT ON；不为去重新增身份型机制（HashSet<Value> 既有语义消费）；不修改既有测试断言（全部新用例）；子查询上下文（`building_subquery`）内 DISTINCT 行为保持现状（零消费→静默不去重）——仅顶层语句消费（子查询 DISTINCT 语义随需要另行裁定，超范围静默不变即无回归）
- Test witness: `tests/select_distinct_test.rs` 先于实现运行——`distinct_single_col`/`distinct_multi_col`/`distinct_null_folds`/`distinct_expression_dedup` 现状 RED（重复行照常返回），实现后 GREEN；其余拒绝面用例现状亦 RED（兜底文案不同于点名文案），按点名文案断言
- GREEN condition: 新测试全绿；`planner_test`(87)/`cli_test`(111)/`expression_e2e_test`/`group_by_expr_test` 零校准通过；全量串行 0 failures
- Verification: ①定向 `cargo test --test select_distinct_test --test planner_test`（退出码 0）；②全量 `cargo test -- --test-threads=1`（M19，退出码 0）；③`openspec validate 2026-09-26-ms25-sql-read-surface --strict`（退出码 0）
- Stop when: 三分支链位与既有 SubqueryEval/顶层 Projection 装配序冲突且无等价装配点（返回 Plan 裁定链位）；DISTINCT × D2 `Filter(Join 全列)` 组合出现行形状不一致（返回 Plan）；需要修改既有测试断言才能通过（返回 Plan）

### T3: 集合操作（`sql-set-operations` R1–R4）

- Requirement/Scenario: R1 UNION 三场景、R2 INTERSECT/EXCEPT 三场景、R3 结构三场景（链式/优先级/拒绝）、R4 排序与零回归三场景
- Depends on: T2（共享去重内核与构造臂接线模式；DISTINCT × 复合拒绝在复合分派中实现）
- Targets: `src/executor/plan.rs`（`SetOperationNode` + `SetOpKind`）、`src/executor/set_operation.rs`（新，执行器）、`src/executor/mod.rs`（导出）、`src/parser/planner/query.rs`（`build_query` body 分派 + `build_set_operation` + `get_plan_output_columns` 臂）、`src/pipeline.rs`（构造臂）、`tests/set_operations_test.rs`（新）
- Current behavior: 复合语句统一 `UnsupportedStatement("SET OPERATION")`（ast.rs:25 `extract_select_body`）；侧臂括号化同拒绝
- Required behavior: ① `UNION [ALL]`/`INTERSECT`/`EXCEPT` 端到端：UNION 去重并集（NULL 相等）/UNION ALL 保留重复与序/INTERSECT 去重交集/EXCEPT 去重差集（NULL 参与匹配）；② 左结合同级、INTERSECT 优先级更高（计划形状见证 sqlparser 嵌套结构）；③ 列数不符计划期点名拒绝；`INTERSECT ALL`/`EXCEPT ALL`/`SetQuantifier::{ByName,AllByName,DistinctByName}`/括号化侧臂（`SetExpr::Query`）计划期点名拒绝；④ 顶层 `query.distinct` × 复合 → 计划期点名拒绝；复合语句尾 ORDER BY（键 = 左侧列名，裸名）与 LIMIT 作用于复合输出；⑤ 侧臂为完整 SELECT（可含 JOIN/外连接/聚合/表达式项/WHERE——经 D2 组合布局）；子查询上下文（`building_subquery`）内复合 → 计划期点名拒绝（范围外，不静默）
- Preserve: 单查询（`SetExpr::Select`）路径逐字节不变（分派改造对 Select 臂零行为变化）；`UnsupportedStatement("SET OPERATION")` 对 `SetExpr::Values` 等非复合形态维持；既有排序 NULL 语义与错误面不变；plan cache 键语义不变；M17/M19
- Forbidden: 不支持 INTERSECT ALL/EXCEPT ALL/ByName 族/括号化侧臂/子查询内复合（全部点名拒绝）；不做跨分支类型强转（行按 Value 流动）；不实现侧臂级 ORDER BY/LIMIT（合成 Query 置空，sqlparser 层侧臂带 LIMIT 落 `SetExpr::Query` 括号化拒绝路径）；不修改既有测试断言
- Test witness: `tests/set_operations_test.rs` 先于实现运行——全部用例现状 RED（`SET OPERATION` 拒绝），实现后 GREEN；`union_dedup`/`intersect_precedence`（计划形状）等按点名断言
- GREEN condition: 新测试全绿；`planner_test`(87)/`cli_test`(111)/`subquery_test`(32) 零校准通过；全量串行 0 failures
- Verification: ①定向 `cargo test --test set_operations_test --test planner_test`（退出码 0）；②全量 `cargo test -- --test-threads=1`（M19，退出码 0）；③`openspec validate 2026-09-26-ms25-sql-read-surface --strict`（退出码 0）
- Stop when: 左结合展平遇 sqlparser 嵌套结构与设计假设不符（如 INTERSECT 优先级解析差异——返回 Plan）；侧臂合成 Query 构建触发 `building_subquery` 上下文歧义（返回 Plan）；复合行形状与 `get_subquery_first_column` 消费面冲突超出拒绝方案（返回 Plan）

### T4: 聚合扩展（`sql-aggregate-extensions` R1–R4）

- Requirement/Scenario: R1 COUNT DISTINCT 三场景（含静默面 RED→GREEN）、R2 SUM/AVG/MIN/MAX DISTINCT 四场景、R3 group_concat 四场景、R4 零回归两场景
- Depends on: None
- Targets: `src/parser/planner/aggregate.rs`（`extract_aggregate_func` DISTINCT 消费 + GROUP_CONCAT 注册 + `is_aggregate_expr` 名单）、`src/executor/aggregate.rs`（`AggregateFunc`/`AggregateState` 新变体 + `result_column_name`/`extract_value`/`update`/`finalize` 臂）、`tests/aggregate_extension_test.rs`（新）
- Current behavior: `extract_aggregate_func` 不读 `f.distinct`——`COUNT(DISTINCT v)` 静默按 `COUNT(v)` 计算（`count_distinct_basic` 现状 RED：输出 3 而非 2）；`SUM/AVG(DISTINCT …)` 同静默；`MIN/MAX(DISTINCT …)` 静默等价普通形态（恰为 SQLite 语义，接受）；`group_concat` 不在聚合名单 → 标量函数路径报未知名；`COUNT(DISTINCT a,b)` 落既有 `InvalidAggregateArgument`
- Required behavior: ① `COUNT(DISTINCT col)` 去重非 NULL 计数；全 NULL → 0；多参数 `COUNT(DISTINCT a,b)` → `InvalidAggregateArgument` 点名；② `SUM/AVG(DISTINCT col)` 去重非 NULL 计算；`MIN/MAX(DISTINCT col)` 接受并等价普通形态（flag 丢弃）；③ `group_concat(col)` 逗号连接组内非 NULL 值、`group_concat(col, '<字面量>')` 自定义分隔符、NULL 跳过、全 NULL 组 → `Null`、非字面量分隔符计划期点名拒绝；与 GROUP BY 组合按分组语义；④ 新输出名 `count_distinct_{col}`/`sum_distinct_{col}`/`avg_distinct_{col}`/`group_concat_{col}`（ORDER BY 引用聚合输出名的既有机制自然可达）；⑤ 既有五聚合无 DISTINCT 形态逐字节不变
- Preserve: 既有聚合计划形状与输出名（`count_{col}` 等）不变；空集语义（COUNT→0/其余→NULL）不变；聚合×表达式混合投影拒绝面与 `NonAggregatedColumn` 语义不变；`result_column_name` 既有六变体文本不变；M17/M19
- Forbidden: 不改既有 `AggregateState` 变体布局（纯新增）；不支持多参数 DISTINCT 聚合与 `GROUP_CONCAT(DISTINCT …)`（后者现状标量拒绝路径自然保持——`func.distinct` 消费仅限新增四变体分支）；分隔符不做表达式求值（仅字面量）；不修改既有测试断言
- Test witness: `tests/aggregate_extension_test.rs` 先于实现运行——`count_distinct_basic`（现状 3 ≠ 2）/`sum_distinct`/`avg_distinct`/group_concat 四用例现状 RED，实现后 GREEN；`min_distinct_eq_plain` 现状 GREEN（行为锁定）
- GREEN condition: 新测试全绿；`group_by_expr_test`(12)/`expression_e2e_test`/`scalar_function_test`(32) 零校准通过；全量串行 0 failures
- Verification: ①定向 `cargo test --test aggregate_extension_test --test group_by_expr_test`（退出码 0）；②全量 `cargo test -- --test-threads=1`（M19，退出码 0）；③`openspec validate 2026-09-26-ms25-sql-read-surface --strict`（退出码 0）
- Stop when: DISTINCT 量化词消费与 `is_plain_column_expr`/混合投影校验产生交互冲突（返回 Plan）；`AggregateState` 新变体需要改动既有变体布局（返回 Plan）

**Invariants**

- `query.distinct == None` 与单查询 `SetExpr::Select` 路径零变化——本 Iteration 全部新逻辑挂在新量化词/新 body 形态之下。
- 去重语义单一来源：`HashSet<Vec<Value>>`（Value 既有 Eq/Hash，NULL 相等）——DISTINCT 与集合操作共用同一语义，不出现两套 NULL 处理。
- 静默面修复以 RED 见证为锚：DISTINCT 静默不去重、COUNT/SUM/AVG DISTINCT 静默错算——实现后用例 GREEN 且断言新语义。
- 拒绝面全部计划期点名（fail-fast），不静默降级。
- 输出行形状不变量：Distinct/SetOperation 产出行与其输入/列声明形状一致，表头经 `get_plan_output_columns` 与行宽恒一致（I034 纪律）。

**Non-goals**

- INTERSECT ALL/EXCEPT ALL/ByName 族；DISTINCT ON；DISTINCT × 标量子查询项/× 复合（点名拒绝）；子查询内复合与子查询 DISTINCT 语义升级；CTE/EXPLAIN（Iteration 002）；ORDER BY 别名排序（Iteration 003——本 Iteration 的 DISTINCT×ORDER BY 用例用输出列名形态）；跨分支类型强转；集合操作 Hash 性能路径。

**Acceptance**

`select-distinct` R1–R3、`sql-set-operations` R1–R4、`sql-aggregate-extensions` R1–R4 全部场景可观察满足（映射见 tasks.md RTM 与各契约 GREEN condition）；两个静默面修复各有 RED→GREEN 见证；白名单外既有测试零校准；全量串行零回归。Scenario→Task→Test 链路：RTM 三 capability 行。

**Verification**

- DISTINCT：`distinct_single_col`（RED→GREEN）/`distinct_multi_col`/`distinct_null_folds`/`distinct_expression_dedup`/`distinct_order_by_output`/`distinct_order_by_non_output_rejected`/`distinct_on_rejected`/`all_quantifier_identity`/×JOIN 组合。
- 集合操作：`union_dedup`/`union_all_preserves`/`union_null_folds`/`intersect_basic`/`except_basic`/`except_null_matching`/`chained_three_way`/`intersect_precedence`/`intersect_all_and_parens_rejected`/`compound_order_by`/`column_count_mismatch_rejected`。
- 聚合：`count_distinct_basic`（RED→GREEN）/`count_distinct_all_null_zero`/`count_distinct_multi_arg_rejected`/`sum_distinct`/`avg_distinct`/`min_distinct_eq_plain`/`distinct_with_group_by`/group_concat 四用例。
- 全量：`cargo test -- --test-threads=1` 退出码 0（串行，M19）。
- 判定均为测试自身断言 + cargo 退出码；无身份型证据工程。

**Gate 2 Readiness**

PASS——RTM 全 Covered 无 Missing；调查完整（DISTINCT/WITH 量化词消费点、`extract_select_body` 分派点、聚合两文件全套、链装配锚点 Iteration 000 后重定位、Value Eq/Hash 基础全部 file:line 实证）；设计闭合（D3 三分支链位、D4 展开与执行器、D7 变体全套、拒绝面清单明确）；任务契约自包含（Act 无需回读外部资料）；验证直接判定；无实质未知项。change 级 Gate 2（2026-09-26 用户批准）覆盖本 Iteration 计划；Iteration 000 新增面（D2 `Filter(Join 全列)`、外连接形状）作为组合场景输入已纳入契约（Status: ready 依据同 MS18 Iteration 001 先例）。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果；无一次性环境、无不可复现结论。

**Risks and Notes**

- DISTINCT 链位与既有 SubqueryEval/顶层 Projection 装配序的交互是本 Iteration 最重的设计落点——表达式项 + DISTINCT 要求 Limit 上移（`Projection → Distinct → Limit`），Limit 装配段（:1381）需按 `distinct_due && has_expression_items` 分支重构；标量子查询项 × DISTINCT 直接点名拒绝以避开 `:1420` 包装段冲突。
- DISTINCT × D2 `Filter(Join 全列)`：WHERE+JOIN 无 ORDER BY 时 Filter 持 SELECT 投影（输出形状）→ Distinct 直接在其上；×ORDER BY 时 Sort 持投影 → Distinct 在 Sort 上。两形态由「输出形状已就绪」规则覆盖，组合用例见证。
- 集合操作侧臂经合成 Query 构建：侧臂的表达式项路由顶层 Projection（`building_subquery=false` 语义）——侧臂计划自带形状裁剪，复合节点消费的即输出形状；子查询上下文复合同步点名拒绝（`building_subquery` 检查）。
- 聚合 DISTINCT 的 `HashSet<Value>` 状态内存上界 = 组内 distinct 值数（与 GROUP BY 分桶同量级）；`GroupConcat` 部件列表同量级。
- `SELECT ALL` 与 no-FORM + DISTINCT 的「无操作接受」为 SQLite 对齐的显式裁定（非静默面：结果与语义一致）。
- 非实质选择留给 Act：`DistinctExecutor`/`SetOperationExecutor` 内部物化结构、`SetOpKind` 命名与位置、拒绝文案的具体措辞（点名能力即可）、新测试文件的 helper 复用形态。

## Act Response

- Status: reported

### 实施结果

T2 / T3 / T4 全部 Required behavior、Preserve、Forbidden 与 GREEN condition 已交付；三个静默面修复（`query.distinct` 无消费、聚合 `DISTINCT` 量化词无消费、`group_concat` 未注册）各具 RED→GREEN 见证。

**产品代码（8 个文件改动 + 2 个新文件）**

- `src/executor/plan.rs` — `PhysicalPlan::Distinct(DistinctNode{input})`；`SetOperationNode{left,right,op,all,columns}` + `SetOpKind{Union,Intersect,Except}`（D3/D4）。
- `src/executor/distinct.rs`（新）— `DistinctExecutor`：首调 `next()` 物化输入，`HashSet<Vec<Value>>` 折叠重复，保留首现序产出（`AggregateExecutor` 同型物化形态）。
- `src/executor/set_operation.rs`（新）— `SetOperationExecutor`：两侧物化后按 op 组合（Union 拼接/去重由 `all` 切换、Intersect 取左侧 ∈ 右集、Except 取左侧 ∉ 右集，均去重并保留左侧首现序）；匹配与去重复用 `DistinctExecutor` 的同一 `HashSet<Vec<Value>>` 内核（NULL 相等单一来源）。
- `src/executor/aggregate.rs` — `AggregateFunc` 增 `CountDistinct/SumDistinct/AvgDistinct/GroupConcat{col,separator}`；`AggregateState` 增四个累积器（`update` 跳 NULL、首次出现才累积；GroupConcat 累积文本部件、全空 → `Null`）；`result_column_name` 增 `count_distinct_{col}`/`sum_distinct_{col}`/`avg_distinct_{col}`/`group_concat_{col}`；`AggregateFunc::column()` 抽出（`extract_value` 的五臂合并 match，既有查找语义不变）；`concat_text` 值→文本渲染。
- `src/parser/planner/aggregate.rs` — `is_aggregate_expr` 名单增 `GROUP_CONCAT`；`extract_aggregate_func` 消费 `f.distinct`（COUNT 多参数/`*` × DISTINCT 点名拒绝，SUM/AVG → 去重变体，MIN/MAX 丢弃 flag 等价普通形态）；`extract_group_concat_args`（1-2 参、次参须 String 字面量）；`build_having_expression` 增新聚合输出名解析（既有五聚合 match 逐字不变）。
- `src/parser/ast.rs` — `extract_columns` / `extract_qualified_columns` 两面扩 DISTINCT 感知命名 + `GROUP_CONCAT` 首参列（新增 `extract_first_col_static`）。
- `src/parser/planner/query.rs` — ① `build_query` body 分派（`SetOperation` → 复合构建、`Query` 括号化点名拒绝、其余维持既有 `SET OPERATION`）；② `build_set_operation` / `build_set_operation_node` / `build_set_operand`（侧臂合成零 ORDER BY/LIMIT 的 `Query` 复用 `build_query`，嵌套复合递归，列数校验，复合尾 ORDER BY（键 ∈ 左列名）/LIMIT）；③ `set_operand_has_distinct` 分支树探测；④ DISTINCT 量化词消费（`!building_subquery` 门、`DISTINCT ON` 点名拒绝、标量子查询项 × DISTINCT 点名拒绝、ORDER BY 键 ∈ 输出列集校验）；⑤ 链位重排 `Sort → [Projection] → Distinct → Limit`；⑥ `get_plan_output_columns` 增 Distinct / SetOperation 两臂。
- `src/pipeline.rs` — Distinct / SetOperation 构造臂；`extract_all_query_table_names` 扩复合 body（抽出 `extract_select_table_names` 共享 helper + `extract_all_table_names_from_set_expr` 递归）。
- `src/executor/correlated.rs` — Distinct / SetOperation 的相关参数注入递归臂。
- `src/executor/mod.rs` — 两新模块与类型的导出。

**测试（3 个新文件，39 用例，全部零校准既有套件）**

- `tests/aggregate_extension_test.rs`（13）：`count_distinct_basic`(RED→GREEN)/`count_distinct_all_null_zero`/`count_distinct_multi_arg_rejected`/`sum_distinct`/`avg_distinct`/`min_distinct_eq_plain`/`distinct_with_group_by`/`group_concat_default_sep`/`group_concat_custom_sep`/`group_concat_null_skip_and_all_null_group`/`group_concat_non_literal_sep_rejected`/`new_aggregate_output_names`（CLI 真表头）/`existing_aggregates_unchanged`。
- `tests/select_distinct_test.rs`（12）：`distinct_single_col`(RED→GREEN)/`distinct_multi_col`/`distinct_null_folds`/`distinct_expression_dedup`/`distinct_order_by_output`/`distinct_order_by_non_output_rejected`/`distinct_aggregate_output`/`distinct_limit_above_dedup`/`distinct_over_join_result`/`distinct_on_rejected`/`all_quantifier_identity`/`non_distinct_queries_unchanged`。
- `tests/set_operations_test.rs`（14）：`union_dedup`/`union_all_preserves`/`union_null_folds`/`intersect_basic`/`except_basic`/`except_null_matching`/`chained_three_way`/`intersect_precedence`/`intersect_all_and_parens_rejected`/`compound_other_forms_rejected`/`compound_order_by`/`column_count_mismatch_rejected`/`single_query_unchanged`/`compound_arms_with_join_and_where`。

- 部分完成：None。
- 未开始：None（T5/T6 属 Iteration 002、T7 属 Iteration 003）。

### Deviations from Plan

1. **夹具改用显式 PK 锚（delta spec 种子在本项目不可插入）**。spec 三份 delta 的 GIVEN 表写作 `t(v INT)` / `t(g VARCHAR, v INT)`，但本项目建表未声明 PK 时**以首列为隐式 PK**（`src/executor/create_table.rs:74-82`，`tests/keyless_row_test.rs:134` 已记录）——Int 首列的重复值被 `DuplicateKey` 拒绝，测试在 setup 阶段即失败。全部新夹具改用 `k INT PRIMARY KEY` 锚 + 被测列（与 Iteration 000 R1-S2 的夹具校正同款做法）。语义要求（重复值 + NULL 的去重计数/连接）不变。
2. **`extract_all_query_table_names` 扩复合 body**（Targets 未列 `pipeline.rs` 的表名提取函数，但契约的 pipeline 改动面必要）：不改则复合语句两侧表全部未注册 → 全量 `TableNotFound`（实测）。同时把原 `SetExpr::Select` 臂抽为 `extract_select_table_names` 共享函数（单查询路径逐值等价）。
3. **`ast.rs` 两个聚合名提取面扩面**（Targets 只列 planner/executor 的 aggregate 文件）：不改则 `GROUP_CONCAT` 在 `extract_columns` 先被 `UnsupportedStatement("GROUP_CONCAT")` 拒绝（RED 实测），且新聚合的输出名与执行器 `result_column_name` 不同源（表头/ORDER BY 键错位）。
4. **`correlated.rs` 增两臂**：`PhysicalPlan` 匹配穷尽性要求（编译器拒绝缺臂）。
5. **子查询上下文 DISTINCT 加 `!building_subquery` 门**（契约 Forbidden 项的落地手段）：不加门时 Distinct 节点进入子查询计划，而 `get_subquery_first_column`（`subquery.rs:386+`）无 Distinct 臂 → 既有可用形态 `IN (SELECT DISTINCT v FROM t2)` 由「成功（去重被静默忽略）」变为报错 `Subquery returns multiple columns`（实测）。加门后子查询内 DISTINCT 维持变更前行为。
6. **`build_having_expression` 补新聚合输出名解析**（Targets 未列）：否则 `HAVING count(DISTINCT v) > 1` / `HAVING group_concat(v) …` 在输出名查找处报 `HavingNonAggregatedReference`（既有五聚合路径逐字不变，仅新增前置分支）。
7. **`AggregateFunc::column()` 抽取**：`extract_value` 由「五变体按 `col` 合并 match」改为「除 `CountStar` 外经 `column()` 查表」，既有查找语义（`column_indices` 小写键、未命中 → `Value::Null`）不变。
8. **`group_concat` 值→文本渲染**（契约未指定非 String 列的文本形态）：String 原样、Int/Float/Bool 用 `Display`、Date/Timestamp 复用既有 `executor::datetime::{format_date,format_timestamp}`（DA5 形态，与 CLI `value_to_text` 同型）。
9. **复合尾 Sort 的 `table_name: "set_operation"`**：合成值；`SortExecutor` 不消费该字段（与既有 JOIN 路径的 `"join_result"` 同型），非实质选择。
10. **`distinct_aggregate_output` 见证改写**：聚合执行器输出行恒为「分组键 ++ 聚合值」，不同分组的输出行天然不同 → 该组合下的折叠不可达。用例改为断言「DISTINCT 行流 == 非 DISTINCT 行流的去重结果（含行形状不变）」，不虚构不可达场景（见下方 Spec 文本发现 2）。
11. **`WITH` × 复合本轮保持不可达**：语句级 `query.with` 由 T5（Iteration 002）在 `build_query` 顶部消费；本轮合成侧臂 Query 置 `with: None`，`WITH c AS (…) SELECT … UNION SELECT …` 报 `Table 'c' not found`（实测，非静默错值）。design D4 的「CTE × 集合操作自然可达」随 T5 落地。

### 自检发现（Gate 3）

- **已修复（首轮实现期，4 项）**：① 复合语句表名注册遗漏（`extract_all_query_table_names` 对非 `Select` body 返回空 → 所有复合查询 `TableNotFound`，10/13 用例红）；② `DISTINCT × 复合` 的分支树探测缺口（初版只查顶层两分支，中间/嵌套分支的 `SELECT DISTINCT` 被静默去重——实测 `SELECT v FROM t1 UNION SELECT DISTINCT v FROM t3 UNION SELECT v FROM t1` 返回去重结果而非点名拒绝）→ 改递归 `set_operand_has_distinct` 并补用例；③ 子查询上下文 DISTINCT 的既有形态回归（见 Deviation 5）；④ 两个「因其他原因报错而假通过」的拒绝面断言（列数不匹配、`INTERSECT ALL`/括号化——实施前它们因旧的 `SET OPERATION` 全量拒绝而通过）→ 改为断言点名文案（`column count` / `INTERSECT ALL` / `parenthesized`），并补 `BY NAME`、DISTINCT × 复合、嵌套分支 DISTINCT、子查询内复合四个拒绝面。
- **已修复（单测自检）**：`rustfmt` 施加于本 Cycle 触及文件（`--config skip_children=true`，避免 Iteration 000 曾遇的递归重排未触碰文件）。
- **遗留 Minor 1**：`group_concat(v, 5)`（数值字面量分隔符）按「仅字面量」契约拒绝，文案为 `separator must be a string literal`；SQLite 接受数值分隔符。属对齐面范围外。
- **遗留 Minor 2**：复合结果行序文档化为「左侧首现序」（SQLite 声明顺序未指定，兼容）；`UNION ALL` 为左支全行 + 右支全行。
- **遗留 Minor 3**：`set_operand_has_distinct` 先于括号化检查触发，`(SELECT DISTINCT …) UNION …` 报 DISTINCT 而非括号化文案（两者均为点名拒绝，仅文案优先级差异）。

### Spec / design 文本发现（Act 只报告）

1. **三份 delta spec 的 GIVEN 表与项目键位语义冲突**：`sql-aggregate-extensions`（`t(v INT)`、`t(g VARCHAR, v INT)`）、`select-distinct`（`t(v VARCHAR)` 可插入但为无键行）、`sql-set-operations`（`t1(v INT)` 含重复）均按「无 PK 声明 = 无键表」的通用 SQL 直觉书写，与本项目「首列隐式 PK」实现不符，测试无法直接落库。建议 Plan 在合并语料时补 PK 锚列（Iteration 000 已有同款先例：R1-S2 场景夹具加 `k INT PRIMARY KEY`）。
2. **`select-distinct` R2 的「DISTINCT × 聚合」折叠不可达**：聚合输出行恒含分组键（既有 `AggregateNode` 输出装配形态），不同分组输出行必不同，该组合下无法见证行折叠。当前用例按「可达 + 按输出行去重」见证（见 Deviation 10）。若要真正的折叠见证需先修聚合输出装配（既有行/表头错配，见 Experience Candidates）。
3. **`sql-aggregate-extensions` R3 的「组内连接顺序 SHALL 为输入行序」**：DataScan 按数据页链序输出，插入序即产出序，实现成立并由 `group_concat_default_sep` 见证（`a,b,c`）。

### 验证

- RED 见证（T4，planner 未改）：`cargo test --test aggregate_extension_test -- --test-threads=1` → `4 passed; 8 failed`，失败即静默面与未注册面：`count_distinct_basic` 实际 3（期望 2）、`sum_distinct` 实际 4（期望 3）、`avg_distinct` 实际 1.667（期望 2.0）、`count(DISTINCT v) GROUP BY` a 组 3（期望 2）、`count(DISTINCT a, b)` 返回 `[[0]]`（期望点名拒绝）、`group_concat` ×3 报 `Unsupported statement type: GROUP_CONCAT`。退出码非 0。
- RED 见证（T2，planner 未改）：`cargo test --test select_distinct_test -- --test-threads=1` → `2 passed; 10 failed`，失败即静默面：`distinct_single_col` 返回 3 行、`distinct_null_folds` 返回 `[null,null,1]`、`distinct_limit_above_dedup` 返回 `[a,a]`、`DISTINCT ON` 无错误、`ORDER BY` 非输出项无错误。退出码非 0。
- RED 见证（T3，planner 未改）：`cargo test --test set_operations_test -- --test-threads=1` → `3 passed; 10 failed`，10 项均报 `Plan error: Unsupported statement type: SET OPERATION`。退出码非 0。
- GREEN（定向，M19 纪律）：`cargo test --test aggregate_extension_test --test select_distinct_test --test set_operations_test -- --test-threads=1` → `13 passed` / `12 passed` / `14 passed`，0 failed，退出码 0。
- 零校准见证（受影响边界）：`cargo test --test planner_test --test cli_test --test subquery_test --test group_by_expr_test --test scalar_function_test --test expression_e2e_test -- --test-threads=1` → `87` / `111 passed; 2 ignored` / `32` / `12` / `32` / `24` 全绿，退出码 0（T2/T3 落地后、T4 落地后各跑一次，覆盖范围未变化）。
- 全量（M19 串行）：`cargo test -- --test-threads=1` → 合计 `1340 passed; 0 failed; 2 ignored`（基线 1301 + 本 change 新增 39），退出码 0。合计值由逐 suite `test result` 求和得到。
- `cargo clippy --all-targets` → 无 warning，退出码 0。
- `rustfmt --edition 2021 --check --config skip_children=true`（本 Cycle 触及的 13 个文件）→ 0 差异，退出码 0。
- `openspec validate 2026-09-26-ms25-sql-read-surface --strict` → `Change '2026-09-26-ms25-sql-read-surface' is valid`，退出码 0。
- 采信（覆盖未变化）：Iteration 000 accepted 基线结论（`nested_loop_join_test` 9 / `outer_join_test` 16 / `subquery_test` 32 等）——本 Cycle 未触碰 JOIN 执行器与相关测试文件（`git diff` 核实本 Cycle 改动面不含 `nested_loop_join.rs` / `predicate.rs` / `tests/nested_loop_join_test.rs` / `tests/subquery_test.rs`）。

### Persisted Evidence

None required——全部结论由测试断言与命令退出码直接判定，可低成本重跑；无一次性环境。

### Experience Candidates

- **Issue 候选（无，未落账）**：聚合直出路径的**输出行/表头错配**——`SELECT count(*) FROM t GROUP BY g` 的 CLI JSON 为 `{"columns":["count_star"],"rows":[["b",1],["aa",1],["ab",1]]}`：表头 1 列而行 2 值（分组键在行内但不在 `AggregateNode.output_columns`，`query.rs` 直出路径 `node_output_columns = agg_output_columns`）。既有问题（无量化词时同样存在，与本 Cycle 的 DISTINCT/集合操作无关），影响面为「SELECT 仅聚合项 + GROUP BY 非投影键」的表头渲染。属 MS25 范围外（聚合输出装配面），由用户决定是否登记 Issue。证据：本 Response「Deviations 10」+ CLI 探测输出。
- Runbook：None。

### 未解决问题

- 上述 Issue 候选（聚合行/表头错配）待用户裁定是否登记；遗留 Minor 1-3 留档。
- `WITH` × 复合按计划待 T5（Iteration 002）消费 `query.with` 后自然可达。
- 本 Iteration（T2/T3/T4）Acceptance 满足，等待 Plan 独立 Review 裁定 `Review Result`。

## Plan Review

- Review Result: accepted

**Findings**

独立核查（diff 全量审读 + 源码逐面比对 + 新测试文件逐用例阅读 + 定向重跑）证实 Act Response 声明与代码一致，T2/T3/T4 全部 Required behavior、Preserve、Forbidden 落地。偏差分类：

- **Deviations 1–11 全部核实为必要且已正确落地**：① 夹具 PK 锚（`create_table.rs:74-82` 首列隐式 PK 实证，Int 首列重复值确被拒绝）；② `extract_all_query_table_names` 复合 body 扩面 + `extract_select_table_names` 抽取（单查询路径逐值等价）；③ `ast.rs` DISTINCT 感知命名与 `extract_first_col_static`（与执行器 `result_column_name` 同源）；④ `correlated.rs` 两臂（匹配穷尽性）；⑤ `!building_subquery` 门（不加门时 `IN (SELECT DISTINCT …)` 由成功变报错——Deviation 5 的实测回归成立，门是 Forbidden 项的正确落地）；⑥ `build_having_expression` 新名解析（既有五聚合 match 逐字未动）；⑦ `column()` 抽取（`extract_value` 既有查找语义不变：未命中 → `Value::Null`）；⑧ `concat_text` Date/Timestamp 走 DA5（与 CLI 渲染同型）；⑨ `"set_operation"` 合成表名（SortExecutor 不消费，同 `"join_result"` 先例）；⑩ `distinct_aggregate_output` 改写（聚合输出行恒含分组键、折叠不可达——见下方 Issue 候选，该改写是诚实的见证方式）；⑪ WITH × 复合留 T5（合成侧臂 `with: None`，实测 `Table 'c' not found` 非静默错值）。
- **Minor 1（非阻塞，纯注释；Act 偏离——Iteration 000 修复不完整残留）**：`src/parser/planner/query.rs:2058` `OuterJoinKind` doc 仍写「`Right` 经左右交换重写后按 FULL 承载」，与已 accepted 的修复（`Right → NestedLoopJoinType::Left`，承载点 `:490-494` 与内联注释 `:484-485` 均已勘误）矛盾——Iteration 000 Deviation 8 声称校正的三处 doc 中此处遗漏。零行为面。处置：一行 doc 勘误随 Iteration 002（T5 同文件触碰）顺带完成，记录于该 Cycle 计划。
- **Minor 2–4（非阻塞，Act 自检已留档）**：group_concat 数值字面量分隔符拒绝（SQLite 接受，对齐面范围外）；复合行序「左侧首现序」文档化；`set_operand_has_distinct` 先于括号化检查（两者均点名拒绝，仅文案优先级）。接受，无需处置。
- **Spec 文本发现（Act 报告，Plan 已处置）**：① 三份 delta spec 涉重复值的 GIVEN 表按「无 PK = 无键表」书写，与本项目「首列隐式 PK、Int 首列重复值被拒」不符——已按 Iteration 000 先例补 `k INT PRIMARY KEY` 锚列与夹具注记（`select-distinct` R1-S2/R2-S1、`sql-set-operations` R2-S1/R2-S2、`sql-aggregate-extensions` R1-S1/R2-S1/R2-S2/R2-S3；可插入的 GIVEN 保持原样）；② `select-distinct` R2 的聚合折叠不可达——已在 spec 增「已知边界」段（聚合输出行恒含分组键、折叠不可达，见证 = 行流等价）；③ group_concat 行序 spec 已自带实现序文档化，无处置需要。
- **Issue 候选处置（用户裁定 2026-09-27）**：聚合直出路径行/表头错配——机制经本 Review 独立核实成立（`query.rs` 直出路径 `direct` 判定缺「键 ⊆ SELECT ∧ 键序一致」检查 → `node_output_columns = agg_output_columns` 不含未入选键；`executor/aggregate.rs::build_output_rows` 分组臂恒产 `group_key ++ aggregates`；wrapper 路径经 `wrap_projection` 已形状一致）。经用户裁定**不单独落账 Issue**，以 **T9 + `aggregate-output-shape-conformance` delta 并入已规划 Iteration 004**（与 T8 同域：聚合输出装配/形状解析面），proposal 决策记录 7 / design D10 / tasks T9 / RTM 已入库。

**Acceptance Gaps**

无。三个静默面修复（`query.distinct` 无消费、聚合 DISTINCT 量化词无消费、group_concat 未注册）各有 RED→GREEN 见证（Act 报告 + 本 Review 在现树重跑 GREEN 态）；T2 三分支链位、T3 复合语义与拒绝矩阵、T4 变体全套与既有聚合零变化均由测试与代码双重核实；既有套件零校准（Act 报告 + 本 Review 定向抽查重跑一致）。

**Evidence**

- 代码核实：`src/executor/{distinct.rs,set_operation.rs}`（全文）、`plan.rs`（Distinct/SetOperation 节点）、`aggregate.rs`（executor/planner 两面 diff）、`ast.rs`/`correlated.rs`/`pipeline.rs`/`executor/mod.rs` diff、`query.rs` diff 与现态（body 分派 :533-549、DISTINCT 消费与链位 :551-580/:1628-1656、ORDER BY 输出列集校验 :1598-1613、复合构建 :521-750、`get_plan_output_columns` 两臂 :126-128、非 DISTINCT 路径 `projection_items.take()` 仅 distinct 分支消费）。
- 本会话新鲜重跑：`cargo test --test aggregate_extension_test --test select_distinct_test --test set_operations_test -- --test-threads=1` → `13/12/14 passed; 0 failed`，退出码 0；零校准抽查 `planner_test`(87)/`group_by_expr_test`(12)/`scalar_function_test`(32)/`expression_e2e_test`(24)/`subquery_test`(32)/`nested_loop_join_test`(9) 全绿，退出码 0；`openspec validate --strict` → valid，退出码 0（T9/spec 勘误后复跑仍 valid）。
- 采信（Act Response，覆盖未变化）：RED 见证三批（T4 4/8 failed、T2 2/10 failed、T3 3/10 failed，失败面即静默面）；全量串行 1340/0/2（本 Review 重跑后无代码变化，`git status` 只读核实）；`clippy`/`rustfmt`/`cargo check` 结论。
- Issue 候选机制核实：`query.rs:1407-1419`（direct 判定）/:1424-1435（node_output_columns 直出臂）+ `executor/aggregate.rs` `build_output_rows` 分组臂。

**Follow-up Decision**

`accepted`：Acceptance 全部满足，无阻塞 finding。三个 Issue 候选中两个已由用户裁定并入本 change（T8/T9），均不落账；Minor findings 留档。Iteration 001 完成——展开 Iteration 002（`002-plan-structure`，T5 CTE + T6 EXPLAIN，change 级 Gate 2 已于 2026-09-26 批准该 Iteration 计划）。

**Next Cycle**

None。

**Next Iteration**

`openspec/changes/2026-09-26-ms25-sql-read-surface/iterations/002-plan-structure/000-initial.md`（已按 Iteration Map 展开，Status: ready）。
