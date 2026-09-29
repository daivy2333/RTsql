# MS25 SQL 读面补全 — Design

> 调查基线：master @ bc1d18d + MS19/MS18 收尾工作区（Rust 代码 2026-09-26 后零变化，全量 1285/0/2 串行，来源 MS18 Iteration 001 Act Response accepted Review）。
> 行号为 2026-09-26 本 change Plan 调查实测，实施时以符号名为准。

## Current-State Evidence（实现调查结论）

### 解析与计划入口

- `src/parser/ast.rs:8` `parse_sql` 使用 **GenericDialect**——`WITH`/`UNION`/`EXPLAIN`/外连接均可解析到达 planner。
- `src/parser/ast.rs:25` `extract_select_body`：`SetExpr::Select` 放行，其余 → `UnsupportedStatement("SET OPERATION")`——集合操作现状拒绝点。
- `src/parser/ast.rs:264` `extract_join_table_name`：JOIN 右侧仅接受普通表（Derived → `"DERIVED TABLE"` 拒绝）；**FROM 基座支持派生表**（`query.rs:277-304` `TableFactor::Derived` 臂 → `DerivedScanNode` + `register_table(alias, columns, "")`）。
- `src/parser/planner/mod.rs:193` `build_plan` 分派：`Statement::Query → build_query`；`Statement::Explain` 落入 `_ => UnsupportedStatement(unsupported_statement_name(stmt))`（EXPLAIN 现状拒绝点）。
- `sqlparser 0.44` AST 形状（实测 registry 源码）：`Query{with: Option<With>, body: Box<SetExpr>, order_by, limit, offset, …}`；`With{recursive, cte_tables: Vec<Cte>}`；`Cte{alias: TableAlias, query: Box<Query>, …}`；`TableAlias{name, columns: Vec<Ident>}`；`SetExpr::SetOperation{op, set_quantifier, left, right}`（左结合嵌套）+ `SetExpr::Query(Box<Query>)`（括号化）；`SetQuantifier::{All,Distinct,ByName,AllByName,DistinctByName,None}`；`JoinOperator::{Inner,LeftOuter,RightOuter,FullOuter,…(JoinConstraint)}`；`JoinConstraint::{On,Using,Natural,None}`；`Statement::Explain{describe_alias, analyze, verbose, statement, format}`；`Select.distinct: Option<Distinct::{Distinct, On(Vec<Expr>)}>`；`Function.distinct: bool`；`OrderByExpr{expr, asc, nulls_first}`。

### 查询计划主体（`src/parser/planner/query.rs`）

- `build_from_clause_with_projection`（:252-457）：基座（Table/Derived）+ JOIN 链循环。JOIN 臂（:318-322）：仅 `Inner(On)`；`Inner(Using/Natural/None)` → `on_clause=None` → :335 `MissingOnClause` 拒绝；其余 → `UnsupportedJoinType`（error.rs:88 文案 "Only INNER JOIN is supported"，PG 0A000）。JOIN 右侧恒为 `ScanNode`。
- 纯等值 ON → Hash `JoinNode`，否则 NLJ `NestedLoopJoinNode`（:400-451）；NLJ 谓词经 `join_column_layout`（mod.rs:134 加性通道）在组合行绝对索引上编译（ON 编译期间 save/restore）。
- **WHERE + JOIN 现状整体拒绝**（:844-851 `UnsupportedStatement("WHERE CLAUSE ON JOIN")`）。
- `build_query`（:460-1337）链装配序：子查询探测 → FROM/JOIN → 聚合检测/路由（T4a 严格投影解析 / T4b 别名重路由 / MS13 混合投影条件包装）→ WHERE → Aggregate/Having → Sort（:1196-1265）→ Limit（:1268-1304）→ SubqueryEval 包装 → 顶层 Projection（:1327-1334）。
- ORDER BY（:1196-1254）：`extract_column_name`（:1779）**仅接受裸 `Expr::Identifier`**——`ORDER BY t.col`/位置序号/表达式现状 `ParseError "ORDER BY only supports column names"`。T6 校验（MS19）：键 ∈ `sort_columns`（非聚合 = `get_plan_output_columns(输入)`；聚合 = SELECT 输出名）∪ `alias_columns`（投影项别名），否则 `ColumnNotFound`；别名键照常进 SortNode，被 `SortExecutor::compare_rows` 的 `position()` None 臂静默跳过（sort.rs:74-97）——T6 升级点。聚合查询的别名键今天**已真实排序**（`sort_columns = projection_columns` 含别名输出名）。
- `query.distinct` / `query.with` 全文件零消费——DISTINCT/WITH 静默面确认。

### 执行器与管道

- `PhysicalPlan`（plan.rs:18-70，25 变体）+ `create_executor_from_plan`（pipeline.rs:517-864）逐变体构造。`DerivedScan` 臂（:832-846）**构造期急切物化**子查询行 → `DerivedScanExecutor::new(rows)` 静态内存表。
- `NestedLoopJoinExecutor`（nested_loop_join.rs）：右输入物化一次、左输入流式；组合行 = `left_row ++ right_row`（全宽）上求值谓词；`build_output_row` 按 `output_columns` 的 `table_alias` 归属提取——**左右交换不影响输出归属**（RIGHT 交换重写的基础）。
- `JoinExecutor`（join.rs）：Hash join，NULL 键 `build_hash_key_*` → None 不匹配（:66-71）。
- `SortExecutor`（sort.rs）：`compare_rows` 列名 `position()` 查找 + `compare_values`（NULL 排尾语义——**与 SQLite NULL 排首相反，既有语义保持**）；`sort_unstable_by` 后按投影裁剪。
- `AggregateExecutor`（aggregate.rs）：`AggregateFunc` 6 变体 + `AggregateState` 累积器 + `result_column_name()` 输出名（`count_{col}` 等）；`is_aggregate_expr` 名单 COUNT/SUM/AVG/MIN/MAX（planner/aggregate.rs:179）；`extract_aggregate_func` 不读 `func.distinct`（静默面确认）。
- `Value`（value.rs:49-93）：`PartialEq + Eq + 手动 Hash`（discriminant 区分；`Null==Null` 为真，`Float` 按 `to_bits`）——HashSet 去重键直接可用。
- `plan_stage`（pipeline.rs:88-117）：`is_cacheable(stmt)` = `matches!(Statement::Query)`（:1211-1213）——仅 SELECT 进 plan cache；EXPLAIN 语句天然不缓存。
- `execute_stage`（:126）/`execute_stage_in_tx`（:331）→ `create_executor_from_plan` 统一入口；`Response::QueryResult{rows}` 单列行集经 CLI 四格式渲染（cli/mod.rs:427）。
- 相关约束：M19 全量串行；M17 锁序（新执行器无存储访问，不触及）；plan cache key = SQL 文本（WITH/集合操作/EXPLAIN 语句文本自带完整结构，无新键语义）。

## Design Decisions

### D1 — OUTER JOIN 经 NLJ 统一承载；RIGHT 计划期左右交换

> Review 勘误（2026-09-27）：本节 RIGHT 语义自始正确（LEFT 交换重写、未匹配原左行丢弃 = 标准 SQL）；Iteration 000 Act 发现的 D1/R2-S1 冲突根因是 delta spec R2-S1 场景 THEN 误写 3 行（FULL 语义）——该场景已由 Plan 勘误为 2 行，实现按本节 LEFT 交换承载（RIGHT 不用 Full 位图机制）。

- **LEFT/FULL**：扩展 `NestedLoopJoinExecutor`——LEFT：左行右组合耗尽且无匹配 → 产出左行 + 保存侧（右）列全 NULL（按 `output_columns` 归属填 NULL）；FULL：另维护右行匹配位图，左输入耗尽后按序产出未匹配右行 + 左列全 NULL。
- **RIGHT** = 计划期重写 `LEFT JOIN(交换左右输入)`：planner 构造 `NestedLoopJoinNode{left: 右计划, right: 左计划, join_type: Left}`，ON 谓词按**交换后**组合行布局编译（复用 `join_column_layout` save/restore）；`output_columns` 保持原序（按 `table_alias` 归属提取，与输入侧无关——executor 实测基础）。未匹配的原左行被丢弃（RIGHT 语义）；FULL 不做交换（两侧都要补齐）。
- **全部外连接走 NLJ**（无论 ON 是否纯等值）——Hash `JoinExecutor` 零触碰；等值外连接的 Hash 性能路径列为非目标（结果语义一致，性能批归 MS22 域）。替代方案（扩展 Hash 执行器 LEFT 臂）被否：两处执行器扩展面 × 语义重复维护，收益仅等值外连接性能。
- 外连接要求 `JoinConstraint::On`；`Using`/`Natural` → `UnsupportedJoinType` 点名保持。`Inner(Using/None)` 现状（`MissingOnClause`）零触碰。
- `JoinOperator::LeftSemi/RightSemi/LeftAnti/RightAnti/CrossApply/OuterApply/CrossJoin` 维持 `UnsupportedJoinType`。
- T01 同时解除 **WHERE + JOIN 拒绝**（D2）——外连接 spec 场景（WHERE 过滤补 NULL 行）的必要前提。

### D2 — WHERE + JOIN 解禁（INNER 与 OUTER 统一；Gate 2 用户确认点）

现状拒绝的根因：`build_where` 单表面向（按 `self.tables[table_name]` 解析），JOIN 行是组合行。机制已存在：`join_column_layout`（NLJ ON 谓词同款）支持限定名/非限定名在组合布局上解析。

方案：JOIN 查询含 WHERE 时——
1. JOIN 节点 `output_columns` 承载**全部组合列**（跳过 SELECT 清单过滤；无 WHERE 路径维持过滤，零回归）。
2. WHERE 谓词经 `join_column_layout`（布局 = 组合全列）编译，包 `FilterNode{input: join, projection: SELECT 清单在组合形状上的索引}`——Filter 既有投影裁剪承担 SELECT 收窄（D10：有 ORDER BY 时 `proj_or_empty` 置空、投影交给 Sort，既有机制不变）。
3. 列解析语义：限定名 `t.c` 按表内位置；非限定名在组合列中唯一才解析、多表命中 → `AmbiguousColumn`、不存在 → `ColumnNotFound`（expression.rs:248-338 既有臂原样消费）。
4. 组合面自然成立：`Filter→Aggregate`（聚合 input_schema = Filter 输出形状）、`Filter→Sort`（D10）、`Filter→Distinct`（Iteration 001）。
5. **边界（Iteration 000 实施确认）**：`IN (SELECT …)`/`EXISTS (SELECT …)` 谓词在 JOIN 基座上**维持既有计划期拒绝**（`WHERE CLAUSE ON JOIN` 文案不变）——SemiJoin/AntiJoin 左输入按单表 schema 解析，与组合形状不同构，解禁会静默产出错误行形状；嵌套于复合谓词内的子查询经既有 `build_where` 兜底 `UnsupportedExpression` 点名。另：布尔字面量 ON（`ON true` 等）经新增 `ConstantPredicate` 编译可达——`INNER JOIN … ON true`（叉积）随之从 `UnsupportedExpression` 拒绝转为可用（行为扩展，无既有断言锁定，R3-S3 的 `ON true` 子查询见证所需）。

INNER+WHERE 一并解禁的理由：同一代码路径（按 join 类型人为分叉反而是额外代码）；此前被拒绝的形态从「报错」变「可用」属能力扩展而非既有行为回归（既有可用形态零回归由测试锁定）。**用户裁定（Gate 2，2026-09-26）：统一解禁**——INNER 与 OUTER 同路径，无类型分叉拒绝臂。

**子查询交互（Iteration 000 Act 阻塞 → replan 裁定 2026-09-27，方案 (a)）**：D2 后子查询内 `WHERE + JOIN` 的计划构建成功（`Filter(Join)`），经 `get_subquery_first_column` 的 Filter 臂递归（subquery.rs:410）到达 Join/NestedLoopJoin 臂（:441）→ `InSubqueryJoinUnsupported`——**拒绝保留，文案统一**：原顶层 `WHERE CLAUSE ON JOIN`（:849）先于该臂触发的形态（`tests/subquery_test.rs:789` 锁定 `Unsupported statement type` 逐字节）改报点名错误。两处既有见证按前提退役校准（`nested_loop_join_test.rs:361` 拒绝见证 → 可用性见证；子查询用例期望文案校准），`in-subquery-join-rejection` 以 MODIFIED delta 修订（WHERE+JOIN 场景文案随 D2 更新、拒绝语义不变）；校准白名单记录于 `sql-outer-join` R4。替代方案（D2 收窄仅外连接）被否：推翻 Gate 2 统一解禁裁定 + INNER/OUTER 不对称语义 + 混合 JOIN 链类型感知拒绝分叉。

### D3 — SELECT DISTINCT：新 Distinct 节点 + 链位规则

- 新 `PhysicalPlan::Distinct(DistinctNode{input})` + `DistinctExecutor`：`HashSet<Vec<Value>>` 逐行去重（Value `Hash+Eq`，`Null==Null` 折叠），首现保留，产出按首现序。
- 链位（DISTINCT 作用于**输出行**，而 Sort 拥有裁剪/Projection 拥有求值——链位按行形状分派）：
  - 纯列清单（无表达式项/子查询项）：`base → [Sort] → Distinct → Limit`。Sort 在下时键解析于宽形状，计划期额外校验「DISTINCT 查询排序键 ∈ 输出列集」保证结果与「先去重后排序」等价。
  - 表达式项（含别名重路由项）：`base → [Sort(表达式键)] → Projection → Distinct → Limit`——Distinct 必须在 Projection 之上（输出行求值后去重）；Limit 随之上移到 Distinct 之上（LIMIT 作用于去重后行数）。
  - 标量子查询项 × DISTINCT：计划期点名拒绝（SubqueryEval 在 Limit 之上插列，链位冲突；诚实拒绝优于错误语义）。
  - DISTINCT × 复合语句（集合操作）：计划期点名拒绝（quantifier 归属歧义，SQLite 语义歧义面）。
- `Distinct ON(...)` → 计划期点名拒绝；`SELECT ALL` → 显式放行等价既有不去重。
- `get_plan_output_columns` 增 Distinct 臂（透传输入列名）。

### D4 — 集合操作：SetOperation 节点 + 左结合展开

- planner：`build_query` 对 `query.body` 分派——`SetExpr::Select` 既有；`SetExpr::SetOperation` → 复合构建（递归左结合展平：`SetOperation{left,right}` 嵌套结构天然承载 INTERSECT 更深优先级，实施时以 AST 嵌套形状验证）；`SetExpr::Query`（括号化）→ 点名拒绝；其余维持 `UnsupportedStatement("SET OPERATION")`。
- 侧臂构建：合成 `Query{body: 侧臂 Select, order_by: [], limit: None, …}` 复用 `build_query`（侧臂可含 JOIN/聚合/表达式项/CTE 引用——全部既有装配复用）；语句级 `query.with` 先于侧臂构建消费（CTE × 集合操作组合自然可达）。
- 列数校验：左右侧 `get_plan_output_columns` 长度相等，否则点名拒绝；复合输出列名 = 左侧列名。
- 新 `PhysicalPlan::SetOperation(SetOperationNode{left, right, op: {Union,Intersect,Except}, all: bool, columns})` + `SetOperationExecutor`：两侧物化 → Union：左+右拼接（`all=false` 去重）；Intersect：∈ 右集的左侧去重行；Except：∉ 右集的左侧去重行（匹配用 `HashSet<Vec<Value>>`，Null 相等）。
- `SetQuantifier::{ByName,AllByName,DistinctByName}` → 点名拒绝；`Intersect/Except × All` → 点名拒绝（SQLite 对齐）。
- ORDER BY/LIMIT 作用于复合输出：`SetOperation → Sort（键 ∈ 左侧列名，既有裸名限制不变）→ Limit`。顶层 `query.distinct` × 复合 → 拒绝（D3）。

### D5 — CTE：planner 注册表 + DerivedScan 内联复用

- **不引入新执行器/节点**：CTE 引用 = `DerivedScanNode{subquery: CTE 计划克隆, alias, columns}`（`FROM (SELECT…)` 既有机制），急切物化、同语句快照下行集跨引用一致（MVCC 语句快照同源）；跨引用共享物化为后续性能优化（improvement 候选，不在验收面）。
- PlanBuilder 加性字段：`cte_plans: Vec<CteDef{name: String, columns: Vec<String>, plan: PhysicalPlan}>` + `pending_cte_names: Vec<String>`（定义中集合，防自引用/环）。
- 消费点：`build_query` 顶部（no-FORM 分支之前、子查询上下文判定之后）——`query.with` 存在且 `building_subquery=true` → 点名拒绝（子查询内 WITH）；`with.recursive` → 点名拒绝；逐个 `Cte` 构建：`pending_cte_names.push(name)` → 合成 Query 构建定义计划（前向引用可见：注册表已含 [0..i]）→ 列名 = `alias.columns` 非空时校验数量后采用，否则 `get_plan_output_columns(定义计划)` → 注册。反向引用：查找仅搜注册表——构建中名称先查 `pending_cte_names` 命中即点名拒绝（自引用/环）。
- 解析钩子：基座臂（query.rs:266）与 JOIN 右臂（:326）解析表名时**先查 CTE 注册表**——命中 → `DerivedScanNode`（绕过 `validate_table`）+ `register_table(alias, columns, "")`（遮蔽同名基表：注册表优先于 `self.tables`）；未命中 → 既有路径。子查询/关联面自然可达（注册表 builder 级共享——标量子查询引用 CTE 场景 R2-S2 由此覆盖）。
- 列别名列表数量与定义输出不符 → 点名拒绝。定义内 ORDER BY/LIMIT/集合操作经合成 Query 全量复用。

### D6 — EXPLAIN：计划包装节点 + 纯函数渲染

- `build_plan` 增 `Statement::Explain` 臂：`analyze=true` / `verbose=true` / `format=Some` → 点名拒绝；内嵌语句须 `Statement::Query`（DML/DDL/事务语句 → 点名拒绝「EXPLAIN supports SELECT statements only」）；`describe_alias`（DESC/DESCRIBE）接受同语义。构建内层计划（计划期错误不豁免）→ 包 `PhysicalPlan::Explain(ExplainNode{input})`。
- `create_executor_from_plan` Explain 臂：**不构造输入执行器**（DerivedScan 急切物化不触发、零副作用），调渲染器产出行集 → 新 `ExplainExecutor`（静态行迭代）。
- 渲染器：新 `src/executor/plan_format.rs` 纯函数 `render_plan_tree(&PhysicalPlan) -> Vec<String>`（单列 `plan`，树形缩进；每变体一行：节点名 + 关键参数——表名/谓词文本/投影清单/JOIN 类型/排序键/LIMIT）；`get_plan_output_columns(Explain) → ["plan"]`。
- 缓存：`is_cacheable` 仅 `Statement::Query` → EXPLAIN 语句不进 plan cache，每次重建（无跨执行状态风险）。
- 渲染文本为结构化信息的确定性投影（节点字段直出），非身份型证据机制。

### D7 — 聚合扩展：DISTINCT 量化词消费 + group_concat

- planner（aggregate.rs）：`extract_aggregate_func` 消费 `f.distinct`——COUNT/SUM/AVG × distinct → 新变体；MIN/MAX × distinct → 丢弃 flag 映射普通形态（SQLite 对齐）；`COUNT(DISTINCT a,b)` 多参数 → `InvalidAggregateArgument` 点名。新增 `GROUP_CONCAT` 注册：参数 1-2 个、第二参数须字面量 String（`Expr::Value`），否则点名拒绝；`is_aggregate_expr` 名单增 GROUP_CONCAT。
- executor（aggregate.rs）：`AggregateFunc` 增 `CountDistinct(String)`/`SumDistinct(String)`/`AvgDistinct(String)`/`GroupConcat{col, separator: Option<String>}`；`AggregateState` 增对应累积器（HashSet 去重 + `Null` 跳过；GroupConcat 累积 String 部件，全 NULL 组 finalize → `Null`）；`result_column_name()`：`count_distinct_{col}` / `sum_distinct_{col}` / `avg_distinct_{col}` / `group_concat_{col}`；`extract_value`/`update`/`finalize` 对应臂。
- 空 GROUP BY 无行输入的既有空集语义（finalize 表驱动）随新变体自然成立。

### D8 — ORDER BY 输出别名排序：表达式键 decorate-sort-undecorate

- `OrderByColumn` 加性字段 `expr: Option<ExpressionRef>`（None = 既有列名键，逐字节不变）。
- planner：T6 校验后，对「仅别名可解析」（`sort_columns` 未命中、`alias_columns` 命中）的键——取该 SELECT 项底层表达式经 `build_expression(&table_name, expr)` 编译为键表达式（与顶层 ProjectionItem 同上下文同索引基座；Scan 基座 = 全形状行、DerivedScan 基座 = 注册输出形状，两者 Sort 输入行形状与注册列一致）；`sort_columns` 已命中的键（含聚合查询别名）维持列名键零变化。别名与基础列同时可解析 → **输出别名优先**（SQLite 语义：先查输出名）。
- `SortExecutor`：含表达式键时 decorate-sort-undecorate——逐行先求值全部键表达式（求值错误正常经 `Result` 传播，不吞入比较闭包），按键序（列键按列名查 + 表达式键按求值值，`compare_values` 既有 NULL 语义）排序后还原行。纯列名键路径零变化。
- 语料同步：delta specs（error-reporting REMOVED+ADDED / sql-scalar-functions 校准段 MODIFIED）随本 change 合并；`tests/scalar_function_test.rs` R5/S5 校准白名单（order-by-alias-sorting spec R3）。

### D9 — JOIN 输出形状与名字解析收口（Iteration 004；2026-09-27 用户裁定并入 T08）

现状三面静默缺陷（2026-09-27 真实二进制实证，Iteration 000 Act Issue 候选经核查并入；均为 MS25 前既有，D2 仅扩大可达面）：

- **聚合参数静默 NULL**：`extract_qualified_columns` 对聚合项返回结果列名而非参数列（ast.rs:184-200）——参数列进不了 `output_columns`，聚合输入形状缺列，`extract_value` 落 `None => Value::Null`（aggregate.rs:277）。D2 路径因全列承载反而幸免。
- **裸名同列末位胜出**：聚合分支 `column_indices: HashMap<String, usize>` 由 JOIN 输出列名构造（query.rs:1086-1090）——同名列收集折叠，GROUP BY 键与聚合参数静默解析到后出现表（非等值 ON 下错误分组）。
- **JOIN 基座零宽行**：`output_columns` 按 SELECT 清单过滤为空集（未知列/别名项不匹配）→ 零列表零宽行静默产出。

方案：

1. **聚合查询 JOIN 全列承载**：`has_aggregates` 且 JOIN 基座时，Join/NLJ 节点 `output_columns` 扩为全组合列（对齐 D2 `carry_all_columns` 机制——聚合输出形状由 group/agg 名单决定，无需 SELECT 收窄 join 输入行）。
2. **布局感知解析**：聚合分支 `column_indices` 改为保序布局感知结构（如 `Vec<(table, name, idx)>` + 查询 helper，与 `join_column_layout` 同型）：GROUP BY 限定名按 (table, column) 精确解析、裸名多命中 → `PlanError::AmbiguousColumn`（SQLite 对齐）；聚合参数 plan 期解析为索引——`AggregateNode` 加性 `aggregate_arg_indices: Vec<Option<usize>>`（与 `aggregates` 平行），`extract_value` 消费索引替代名字查表；参数解析失败 → `ColumnNotFound`（静默 NULL 消除）、裸名歧义 → `AmbiguousColumn`。
3. **非聚合 JOIN 投影严格解析**：JOIN 基座纯列清单逐项解析——未知列 → `ColumnNotFound`、裸名多表命中 → `AmbiguousColumn`（零宽行消除）；纯列引用别名项经 MS19 T4b 机制扩展（`alias_reroutable` 放宽至 JOIN 基座）可达、表头 = 别名；任意表达式项 × JOIN 维持既有显式拒绝。
4. **零回归锚**：无歧义、无未知列的有效查询（含 Iteration 000–003 交付面）计划形状、结果与表头逐字节不变；`get_plan_output_columns` 对有效查询输出不变（聚合查询 join 输入形状变化不外溢——聚合输出形状由自身名单决定）。

Cycle 展开时的调查点（Gate 2 前闭合）：`alias_reroutable` 放宽与「表达式项 × JOIN 拒绝臂」的精确交互（纯列引用别名项豁免该臂的编译路径——经顶层 Projection 以过滤后 join 形状求值，或等价机制）；`extract_column_indices`/Hash join 条件解析与新布局结构的协作；别名列在 ORDER BY / 聚合组合下的表头与解析序。

### D10 — 聚合输出形状一致性（Iteration 004 增补 T09；2026-09-27 用户裁定并入）

现状缺陷（Iteration 001 Act Issue 候选，Plan Review 2026-09-27 独立核实机制）：

- **执行器行装配恒为 `group_key ++ aggregates`**（`src/executor/aggregate.rs::build_output_rows` 分组臂）。
- **直出路径表头 = SELECT 清单序**（`query.rs` 聚合分支 `direct = !has_expression_items && keys_all_column && select_keys_first`，`node_output_columns = agg_output_columns`）——`direct` 判定**不检查**「GROUP BY 键 ⊆ SELECT 键项」与「SELECT 键序 = GROUP BY 键序」，产生两类错配：① 键未入选（`SELECT count(*) FROM t GROUP BY g`）→ 表头 1 列而行 2 值；② 键序不同（`SELECT b, a, count(*) FROM t GROUP BY a, b`）→ 行按 GROUP BY 序、表头按 SELECT 序置换错位。均为 MS13 直出路径引入的既有缺陷，与本 change 前 Iteration 无关。
- **包装路径已形状一致**：`node_output_columns = group_keys ++ agg_names`，条件包装 `ProjectionNode` 把行重排到 SELECT 项（`wrap_projection`），未被 SELECT 引用的分组键被丢弃，表头 = SELECT 名——输出恒为 SELECT 清单形状。

方案：

1. **直出安全判定收窄**：`direct` 增加两条件——GROUP BY 键集合 ⊆ SELECT 键项、SELECT 键项序 = GROUP BY 键序（键项在前、聚合在后的既有 `select_keys_first` 条件保留）。判定安全的查询路径与计划形状逐字节不变。
2. **形状不安全改走既有包装路径**：不满足收窄判定的聚合查询（键未入选/键序不同）由条件包装机制承载——输出行与表头 = SELECT 清单形状，未入选分组键不进输出、仅供 HAVING 绑定（`node_output_columns` 仍含键）。SQLite/PostgreSQL 对齐（两系统聚合查询输出均为 SELECT 列表形状）。
3. **HAVING/排序绑定零变化**：HAVING 经 `node_output_columns`（键 ++ 聚合）绑定不受影响；ORDER BY 聚合输出名绑定不受影响。附带行为扩展：直出形态下「HAVING 引用未入选分组键」由绑定失败转为可达（包装路径既有能力，Cycle 调查确认无既有断言锁定失败面）。

替代方案被否：**表头补键**（输出用户未选择的列，偏离 SQLite/PG 输出形状）；**执行器去键**（改 `build_output_rows` 行装配，牵动 wrapper 重排与 HAVING 绑定两消费面，改动面大于判定收窄）。

已知校准：`tests/select_distinct_test.rs::distinct_aggregate_output`（Iteration 001 交付）按旧错配形状断言 `expected.len() == 2`，行变 1 宽（去重后 `[2]`）需按新形状校准——校准白名单。Cycle 调查点（Gate 2 前闭合）：全量套件中「仅聚合项 + GROUP BY 非投影键」「SELECT 键序 ≠ GROUP BY 序」的既有断言面（含 `group_by_expr_test`/`expression_e2e_test` 聚合矩阵与 CLI 表头用例）；HAVING 未入选键绑定面；`get_plan_output_columns` Aggregate 臂与包装 Projection 臂的表头通路核对。

### D11 — DerivedScan 基座别名应用收口与信号注册时序硬化（Iteration 004 replan 增补 T10/T11；2026-09-28 用户指令并入）

**T10 病灶**（Iteration 003 Act 报告 + Plan Review 2026-09-28 CLI 复现）：`WITH c AS (SELECT v FROM t1) SELECT v AS val FROM c ORDER BY val` 表头回落 `["v"]`（别名丢失）、序为输入序——MS19 T4b 的 `alias_reroutable` 门 `matches!(base_plan, Scan(_))` 漏掉 DerivedScan 基座，普通列别名项停在 `Column` 角色：别名不应用（表头 = 源列名）且 ORDER BY 无可解析输出列（静默不排序）。表达式项别名在 DerivedScan 基座正常（Act 实测 `SELECT upper(v) AS val FROM c ORDER BY val` → `['A','B']`）——证明 `projection_items` 通路与注册形状编译基座在 DerivedScan 上成立。

**T10 方案**：`alias_reroutable` 基座门扩展 `DerivedScan(_)`（与 T8 的 Join/NLJ 扩展同一编辑点一次落地）。编译基座已存在：CTE 与 FROM 子查询均经 `register_table(&alias, columns)` 注册输出形状（query.rs:320/:826），`build_expression(&table_name, expr)` 对 DerivedScan 基座解析注册形状——Sort 恒在顶层 Projection 之下，键/投影项索引与注册形状同源（与 Scan 基座同构）。T7 装配门（`projection_items.is_some()`）随之自动覆盖 DerivedScan 基座的 ORDER BY 别名键。语料承载：delta `error-reporting` MODIFIED R5（基座范围扩展 + CTE 基座别名表头场景）+ delta `order-by-alias-sorting` R1 扩展（DerivedScan 基座别名排序场景）。非目标：子查询内别名（`building_subquery` 抑制不变）、JOIN 基座（T8 契约承载）、通配清单（门既有排除不变）。

**T11 病灶**（Iteration 003 全量首跑 `test_sigint_during_open_130` 偶发，`out.code = None`）：`execute_command_inner` 的两段 `select!` 中 open 臂**先于**信号臂声明（cli/mod.rs:332-338），而 open future 首次轮询的同步前缀完成 `Database::open_with_key → FileStorage::open_with_key → try_lock`（database.rs:58 同步调用，file_storage.rs:71）——锁获取先于信号处理器注册（注册发生在信号臂 future 首次轮询）。测试以「子进程持锁」为锚发 SIGINT，kill 落入「已持锁、未注册」窗口 → 默认处置死亡（`out.code = None`），违背「open 阶段信号无 close 立即退（Signaled 2 → exit 130）」的规格意图。窗口极窄（同步前缀微秒级 × 测试 5ms 轮询），与偶发频率吻合。

**T11 方案**：结构性消除窗口——保证信号处理器注册先于 open 路径的锁获取。机制候选（等价，Act 按 tokio 注册语义调查后定）：① 在 open select 前创建两个信号 future（若 tokio 在 future 创建时注册——`Signal::new`/`ctrl_c()` 构造期——则直接生效）；② open 路径首 await 前置 `yield_now().await`，使 open 臂首次轮询在锁获取前让位、select 转而轮询信号臂完成注册。行为面：进程启动至 open 前送达的 SIGINT 由「默认处置死亡」收敛为 130（既有 Requirement 意图内，无 corpus 场景锁定旧窗口，不加 delta）。验证：两个信号 e2e 用例按**计划判定方式的固定次数压测**（不稳定类验证的既定判据条款）+ 全量零回归；白名单外既有测试零修改。

### D12 — 单表面聚合参数计划期解析与限定通配文案可操作化（Iteration 004 二次 replan 增补 T12/T13；2026-09-28 用户指令并入）

**T12 病灶**（Iteration 003 Plan Risk 留档 + Cycle 001 Plan Review Finding 1/8）：聚合函数参数的列解析只在 JOIN 布局面计划期进行，单表面（`join_layout = None`）`aggregate_arg_indices` 全 `None` → 运行期 `extract_value` 经 `column_indices` 名字查表、查无即静默 `Value::Null`——`SELECT count(nosuch) FROM t` → `[[0]]` exit 0（2026-09-28 实测）；JOIN 面 `resolve_layout_index` 零命中 `Ok(None)` 同样静默（Cycle 001 Review 实测 `count(nosuch)`/`sum(t1.nosuch)`）。附带形态：聚合参数引用 SELECT 输出别名（`SELECT upper(b) AS u, count(u) …`）现状静默 0（实测）——聚合参数按 SQL 语义引用输入列，别名引用属列缺失。

**T12 方案**：统一两面的参数计划期解析——`aggregate_arg_indices` 构造从「JOIN 布局独享」扩为「JOIN 布局 ∨ 单表 `column_indices`」：`Some(arg)` 逐参解析（`Identifier` → 小写名查布局/`column_indices`；`CompoundIdentifier` 2 段 → JOIN 布局限定名解析 / 单表取末段名查表），命中 → `Some(idx)`；零命中 → `Err(ColumnNotFound(名))`（裸名 = `ident.value`、限定名 = 末段名）；JOIN 布局裸名多命中 → `Err(AmbiguousColumn)`（既有）；非列引用参数（`None`）维持 `Ok(None)`。运行期 `extract_value` 的名字查表回退对列参数成为死路径，保留为防御（非实质）。语料承载：delta `error-reporting` MODIFIED R6（列缺失分类一致扩展到聚合函数参数）。行为变化明示：别名参数与未知参数由静默 0/NULL 转为点名拒绝（既有测试清扫核实零锁定——聚合参数均为基列引用）。Cycle 001 Plan Review 的 JOIN 面修复项并入本契约。

**T13 病灶**（Act 报告 Issue 候选）：`SELECT t.*`（sqlparser `SelectItem::QualifiedWildcard`）计划期拒绝，但文案为泛化的 `Unsupported statement type: SELECT ITEM`（ast.rs 两处兜底臂：extract_columns ~:163、extract_qualified_columns ~:298）——不点名能力，违背语料 `error-reporting` R1「携带被拒绝能力的名称」的可操作化意图。单表面/派生表/JOIN 三基座同拒（拒绝本身诚实且为既有面，非本 change 引入）。

**T13 方案**：文案可操作化——两处兜底臂的 `"SELECT ITEM"` 改为能力点名（如 `"QUALIFIED WILDCARD"`，跟随 R1 文案框架 `Unsupported statement type: {能力名}` 与 no-FROM 路径既有先例 `"Qualified wildcard is not supported without FROM"`，query.rs:2123-2127）。拒绝语义本身零变化（不实现 `t.*` 投影——能力支持为独立方向，非本任务范围）。语料承载：delta `error-reporting` MODIFIED R1（新增限定通配点名场景）。

## Implementation Order（依赖理由）

1. **Iteration 000（T01）**：JOIN 面（外连接扩展 + WHERE×JOIN 解禁）——后续 Iteration 的 DISTINCT/集合操作/CTE 组合场景依赖 JOIN 可用面；先行收敛避免跨轮返工。
2. **Iteration 001（T02+T03+T05a）**：输出行代数（Distinct/SetOperation 执行器 + 聚合扩展）——依赖 000 的 JOIN 面（组合场景）；三者同域共享「输出行/去重」语义与 HashSet 基础。
3. **Iteration 002（T04+T05b）**：计划结构消费（CTE 注册表 + EXPLAIN 渲染）——EXPLAIN 放此轮可同时见证 000/001/002 引入的全部计划形状（外连接/去重/集合/CTE 节点渲染）。
4. **Iteration 003（T06）**：排序键升级 + 语料校准——依赖 000（JOIN 输出形状排序键）与 001（DISTINCT×别名排序组合场景）。
5. **Iteration 004（T08–T13，2026-09-27 增补；2026-09-28 两次 replan 并入 T10/T11 与 T12/T13）**：JOIN 名字解析收口（T08）与聚合输出形状一致性（T09）——依赖 000（JOIN 面）；T10（DerivedScan 基座别名应用）与 T11（信号注册时序硬化）为 Issue 候选修复（第一次 replan 并入）；T12（聚合参数计划期解析统一）与 T13（限定通配文案可操作化）为 Issue 候选修复（第二次 replan 并入，T12 并吸收 T08 的参数面残留收口）。T08/T09/T10/T12 同属输出形状与名字解析收口面（共享 `query.rs` 聚合分支与 `alias_reroutable` 门、`executor/aggregate.rs`、`plan.rs::AggregateNode` 诊断边界）；T11 独立故障域（CLI 信号面）；T13 独立错误面（ast 兜底臂文案）。T09/T10/T11 与 T08 主体已在 Cycle 001 交付并通过 Review。

## Risks and Notes

- WHERE×JOIN 解禁后，此前报错的用户 SQL 转为可用——行为面扩大由 spec R3 场景与测试矩阵锁定；既有可用行为零回归由既有测试零修改见证。
- `sort_unstable_by` 换 decorate-sort-undecorate 仅发生在「含表达式键」的查询；纯列名键保持 `sort_unstable_by` 原路径（等价性由既有套件锁定）。
- CTE 内联求值的性能：同一 CTE 被引用 N 次求值 N 次——登记 improvement 候选（跨引用共享物化），不阻塞验收。
- SetOperation 侧臂合成 Query 的 `building_subquery` 上下文：侧臂按顶层语义构建（表达式项路由 Projection 等）——实施时验证侧臂表达式项路径与顶层一致。
- EXPLAIN 渲染格式为最小面（节点名+参数行），格式细节（缩进宽度等）为非实质选择留给 Act。
- T6 校验升级后 `alias_columns` 的消费从「放行静默」变「编译键」——未知键拒绝语义不变（MS19 锁定面）。
