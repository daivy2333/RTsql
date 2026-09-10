# Iteration 000 / Cycle 000-initial: WHERE 侧表达式能力（四件套 + 三值内核 + 值表达式操作数 + I040）

## Plan Context

- Status: ready
- Iteration: 000-where-expressions
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3, T4, T5, T6
- Depends on: None
- Stable baseline: 三值求值内核与新谓词可用；WHERE 支持 [NOT] IN/[NOT] BETWEEN/[NOT] LIKE/IS [NOT] NULL/NOT 与 CASE/COALESCE/CAST 谓词操作数并可正确路由（下推/Filter）；I040 负数字面量入库；既有 704 测试零修改通过
- Verification boundary: `tests/expression_e2e_test.rs` + predicate 单测 + pushdown 追加用例全绿；全量 `cargo test` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` / `openspec validate --all` 通过
- Diagnostic boundary: `src/executor/predicate.rs`、`src/parser/planner/{expression,query}.rs`、`src/parser/planner/ddl_dml.rs`、`src/parser/error.rs`
- Deferred tasks: T7, T8, T9（Iteration 001 SELECT 派生列）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 全部默认假设（Gate 1 审计中）；design D1-D3/D5/D6
- Excluded scope: SELECT 投影表达式（T7-T9）；HAVING 新表达式；算术运算；I035/I034

**Objective**

非聚合查询的 WHERE 子句支持 spec `sql-expression-evaluation` R1/R2/R3/R5 全部形态，行选择语义符合 SQL 三值标准且既有形态可观察行为不变；INSERT 负数字面量可达。

**Background**

路线图 MS11-T01（R18 主题 7：agent 写 SQL 的日常件）。现状 `build_where`/`build_expression` 仅支持六比较 + AND/OR + 字面量，IN/LIKE/BETWEEN/IS NULL/CASE/COALESCE/CAST 全部落 `UnsupportedExpression`。I040 为同域 planned I 项（MS10-T05 登记），经用户决策默认并入。

**Current Baseline**

- revision `a5b0a5f` + 未提交 MS10-T05 工作树（实施与 docs sync 未 commit，基线现场）。
- 测试基线：704 pass / 0 failed / 2 ignored（SNAPSHOT:74，2026-09-09 复跑 2 次）。
- 本日烟雾验证：`cargo test --test planner_test --test predicate_test` → 29+12 全绿（2026-09-10）。
- 工作区与本 Cycle 规划期间一致（git status 复核 2026-09-10）。

**Current-State Evidence**

求值器（`src/executor/predicate.rs`）：
- `Predicate` trait `evaluate(&self, row: &[Value]) -> Result<bool>` :10-17；`PredicateRef` :20；`Expression` trait（`evaluate` 默认实现走 `evaluate_ref`→`to_value`）:23-46；`ExpressionRef` :49。
- `ComparisonOp{Eq,Ne,Gt,Lt,Ge,Le}` :53-66；`ComparisonPredicate{left,op,right}` :70-97，**NULL 拦截**：任一侧 is_null → `Ok(false)`（:82-85）；`<>` = `!equals`（:89）。
- `LogicalOp{And,Or}` :109-114；`LogicalPredicate` :118-144，布尔短路求值。
- `ColumnExpression{column_name,column_index}` :154-157（求值 = row[column_index]）；`ConstantExpression` :178-180；`ParameterExpression` :193-196（相关子查询参数）。
- 消费点共享同一 `evaluate`：Filter `filter.rs:45`（错误包装 :53-58 "Predicate evaluation error"）、DataScan `data_scan.rs:169-187` `filter_row`（主循环 :480，与 Filter 同求值同文案）、HAVING `having.rs:27`。JOIN ON 不走此体系（哈希键，`join.rs:70-110`）。

值类型（`src/executor/value.rs`）：`Value{Int,String,Null,Float,Bool}` :46-57；`equals` :118-137（NULL==NULL→true；跨类型→false 不报错；Int↔Float 转换）；`gt/lt/ge/le` :140-217（NULL→`NullComparison`；不兼容→`TypeMismatch`）；`Display` :267-278（**String 带引号**——CAST AS STRING 不得使用）；`is_null` :93、`as_float` :98、`as_bool` :108。

planner 转换（`src/parser/planner/expression.rs`）：
- `build_where` :191-238：`BinaryOp`{And :201-209 / Or :210-218 / 六比较经 `convert_comparison_op` :219-231}、`Nested` :235、其余 → `UnsupportedExpression` :236。
- `build_expression` :97-188：`Identifier`（裸 NULL hack :106-108）:103-127、两段 `CompoundIdentifier`（外层引用 → `ParameterExpression` :133-141）:128-160、`Value` :161-165、`UnaryOp::Minus` 仅数字字面量 :167-185、默认 `UnsupportedExpression` :186。
- `expr_to_column_name` :242-253（Iter 001 面，本 Iteration 不改）。

WHERE 路由（`src/parser/planner/query.rs`）：`build_query` WHERE 分支 :392-495——JOIN+WHERE 拒绝 :394-396；IN 子查询/EXISTS `try_build_where_subquery` :399；简单 PK 等值 → IndexScan :406-421（`is_simple_pk_equality` :675-707）；复杂 PK → Filter :422-431；`has_pk_equality` → Filter :432-441（:716-740，**只认 `BinaryOp::Eq` 形态**）；`contains_or` → Filter+DataScan 包装 :442-469；否则谓词装入 DataScan :470-489；无 WHERE → DataScan :490-495。`contains_or` :853-870（**已有 `UnaryOp` 遍历臂 :866**；BinaryOp/Nested 之外 `_ => false`）。`resolve_projection_indices` :830-851（本 Iteration 不改）。

字面量与错误：`value_from_sqlparser`（`src/parser/value.rs:8-30`，Number/SingleQuotedString/Null/Boolean）；`PlanError` 变体（`src/parser/error.rs:7-56`：`ParseError(String)`/`UnsupportedExpression`/`UnsupportedValue` 等；无 UnknownFunction 变体）。

I040 修复点：`ddl_dml.rs::extract_insert_values` :107-131（`Expr::Value`/NULL 标识符臂，`_ => UnsupportedValue` 在 :119 附近）。

sqlparser 0.44.0 形态（registry 源码核实，`ast/mod.rs`）：`InList{expr: Box<Expr>, list: Vec<Expr>, negated: bool}` :409；`Between{expr, negated, low, high}` :427；`Like{negated, expr, pattern, escape_char: Option<char>}` :440；`IsNull(Box<Expr>)`/`IsNotNull(Box<Expr>)` :397-399；`Cast{expr, data_type: DataType, format: Option<CastFormat>}` :497；`Case{operand: Option<Box<Expr>>, conditions: Vec<Expr>, results: Vec<Expr>, else_result: Option<Box<Expr>>}` :633；**COALESCE 无专用变体**（按 `Expr::Function` 解析）；`DataType` 四族映射先例 `convert_data_type`（`ddl_dml.rs:133-160`，未知类型默认 String——CAST 用严格版拒绝而非兜底）。

测试入口：`tests/predicate_test.rs`（12，Predicate 系统单测模式）、`tests/planner_test.rs`（29，plan 形态断言模式）、`tests/pushdown_test.rs`（15，DataScan/Filter 等价 + `DataScanNode.predicate` 断言模式）、`tests/aggregate_test.rs`（E2E 建表-插入-查询模式）。

**Relevant Code**

- `src/executor/predicate.rs` — 求值器内核（T1/T2/T4 主战场）
- `src/parser/planner/expression.rs` — WHERE/操作数转换（T3/T4）
- `src/parser/planner/query.rs` — 路由与 `contains_or`（T3）
- `src/parser/planner/ddl_dml.rs` — I040（T5）
- `src/parser/error.rs` — 错误变体（如需 additive 消息用 `ParseError(String)`）

**Critical Path**

SQL 文本 → parse_stage（sqlparser AST，目标变体解析已可用）→ `build_query` WHERE 分支（`query.rs:392`）→ `build_where`/`build_expression`（新增臂，`expression.rs`）→ `PredicateRef`/`ExpressionRef` 树（`predicate.rs` 新类型 + 三值内核）→ 路由判定（`contains_or`/`has_pk_equality`，`query.rs`）→ DataScan 装入或 Filter 包装 → `Predicate::evaluate` 行过滤（`data_scan.rs:480` / `filter.rs:45`）→ CLI exit 3 或行输出。INSERT 路径：`extract_insert_values`（`ddl_dml.rs`）→ `InsertNode.values`。

**Implementation Guidance**

顺序按依赖：T1（内核）→ T2/T4（新谓词与值表达式，均只依赖 T1）→ T3（planner 臂，依赖 T1/T2/T4 的类型）→ T5（独立）→ T6（见证收口）。TDD：先写 `tests/expression_e2e_test.rs`（运行时 RED——今日这些 SQL 报 `Plan error: Unsupported expression type`），再实现转 GREEN。

- T1 内核：`pub enum Ternary{True,False,Unknown}` + `fold`（Unknown→False）；trait 新增 `evaluate_ternary` 默认实现 = `evaluate().map(bool→True/False)`；`ComparisonPredicate::evaluate_ternary` NULL→Unknown（替代 ：82-85 的 Ok(false) 路径），`evaluate()` 重写为 fold——与旧行为逐字节等价；`LogicalPredicate::evaluate_ternary` 三值表（AND：有 False→False，否则有 Unknown→Unknown，否则 True；OR 对偶），`evaluate()` 改走 fold（双侧求值替代短路——求值纯函数，结果与错误传播不变）。
- T2 新谓词：`LikePredicate{expr,pattern: ExpressionRef}`（两侧求值 String 后 `%`/`_` 贪婪回溯匹配；任一侧非 String→Err；expr 或 pattern 为 NULL→Unknown）；`IsNullPredicate{expr}`（NULL→True，否则 False）；`NotPredicate{inner: PredicateRef}`（三值取反）。三者均实现 `evaluate`（fold）+ `evaluate_ternary`。
- T3 planner：`build_where` 新臂——`InList` → OR 链 `ComparisonPredicate(Eq)`（项经 `build_expression`），`negated` → `NotPredicate` 包装；`Between` → `AND(Ge, Le)`，negated 同上；`Like` → `LikePredicate`（`escape_char.is_some()` → `ParseError("LIKE ESCAPE clause is not supported")`；ILIKE/RLIKE/SimilarTo 落默认臂维持拒绝）；`IsNull` → `IsNullPredicate`、`IsNotNull` → `Not(IsNull)`；`UnaryOp::Not` → `NotPredicate(build_where(inner))`。`contains_or` 扩展遍历：`InList{expr,list}`、`Between{expr,low,high}`、`Like{expr,pattern}`、`IsNull/IsNotNull(inner)`、`Case{operand,conditions,results,else_result}`、`Cast(inner)`、`Function` 参数（args 遍历）——防 CASE 条件内 OR 被误下推。
- T4 值表达式（`predicate.rs` 内或相邻新文件，同职责域）：`CaseExpression{whens: Vec<(PredicateRef, ExpressionRef)>, else_: Option<ExpressionRef>}`——searched 直接构建（条件经 `build_where`，True 取结果/Unknown+False 继续）；simple 形态 planner 脱糖为 `Eq(operand, when)` 条件（operand NULL→Unknown→不命中）；缺省 ELSE→`Value::Null`。`CoalesceExpression{args}`——`build_expression` 的 `Expr::Function` 臂名匹配 `COALESCE`（不区分大小写），其余函数名维持 `UnsupportedExpression`；首个非 NULL 参数，全 NULL→Null。`CastExpression{expr,target: CastType}`——`CastType{Int,Float,String,Bool}` 严格映射（复用 `convert_data_type` 四族 match 逻辑但未知 DataType → `ParseError`；`format.is_some()` / TryCast → 拒绝）；转换矩阵：恒等、Int↔Float（`as`，Float→Int 截断向零、越界饱和）、String→Int/Float/Bool 解析失败→Err、数值/Bool→String 值格式化（**禁用 `Value::Display`**——String 加引号）、Bool↔数值拒绝、NULL 短路→Null。三实现为 `Expression` impl，作比较操作数时自动获得三值 NULL 传播。
- T5：`extract_insert_values` 加 `Expr::UnaryOp{op: Minus, expr: Expr::Value(v)}` 臂——`value_from_sqlparser` 后 Int/Float 取负，其余 `UnsupportedValue`（与 `build_expression:167-185` 同构）。
- 错误面：不支持形态维持 `UnsupportedExpression`；显式拒绝用 `ParseError(<具体说明>)`（additive）。

**Behavioral Change**

- 当前：目标 SQL 形态全部 `Plan error: Unsupported expression type`（exit 3）；INSERT 负数 `UnsupportedValue`。
- 目标：上述形态按 spec 语义求值（R1/R2/R3/R5 场景矩阵）；新错误文案仅 additive（ESCAPE/TRY_CAST/未知 DataType 等 `ParseError`）。
- 不变：无 NOT 的既有比较/AND/OR 行选择结果逐字节不变（fold 等价性）；错误文案不变；plan cache 仅缓存 Query；19 plan 节点集合不变；JOIN ON 等值限制不变。

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R2/S1-S4 | `src/executor/predicate.rs`（trait :10-17、ComparisonPredicate :70-97、LogicalPredicate :118-144） | 两值求值 | 新增 `Ternary`/`evaluate_ternary`，两实现三值重写，`evaluate()` 走 fold |
| T2 | R1/S3-S5、R2/S4 | `src/executor/predicate.rs` | 无（新类型） | 新增 `LikePredicate`/`IsNullPredicate`/`NotPredicate` |
| T3 | R1/S1-S7、R2/S1 | `src/parser/planner/expression.rs::build_where`、`query.rs::contains_or` :853-870 | 仅比较/AND/OR；遍历不含新变体 | 新增 IN/BETWEEN/LIKE/IS NULL/NOT 臂（脱糖）；`contains_or` 扩展遍历 |
| T4 | R3/S1-S5 | `src/executor/predicate.rs`（或相邻新文件）、`expression.rs::build_expression` :97-188 | 无（新类型）；`Expr::Function`/`Cast`/`Case` 无臂 | 新增 `CaseExpression`/`CoalesceExpression`/`CastExpression` + 转换臂 |
| T5 | R5/S1-S2 | `src/parser/planner/ddl_dml.rs::extract_insert_values` :107-131 | `UnaryOp` → `UnsupportedValue` | 数字字面量取负折叠 |
| T6 | R1-R3/R5/R6 | `tests/expression_e2e_test.rs`（新）、`tests/predicate_test.rs`（追加）、`tests/pushdown_test.rs`（追加） | 无 | 测试见证（RED→GREEN）+ 回归门 |

**Task Contracts**

### T1: 三值求值内核——既有谓词三值重写，可观察行为逐字节不变

- Requirement/Scenario: R2/S1-S4
- Depends on: None
- Targets: `src/executor/predicate.rs`（`Predicate` trait、`ComparisonPredicate`、`LogicalPredicate`）
- Current behavior: `Predicate::evaluate → bool`；比较 NULL→`Ok(false)`（:82-85）；AND/OR 布尔短路（:124-144）
- Required behavior: `evaluate_ternary → Ternary`（默认实现映射 `evaluate`）；两实现提供三值求值；`evaluate()` = fold（Unknown→False）
- Required changes: 新增 `Ternary` enum + fold；trait 新增 `evaluate_ternary` 默认方法；`ComparisonPredicate`（NULL 操作数→Unknown；比较映射 True/False）与 `LogicalPredicate`（三值 AND/OR 表）override；两实现 `evaluate()` 重写为 fold
- Preserve: 直接 `evaluate()` 调用者（Filter `filter.rs:45`、DataScan `data_scan.rs:480`、HAVING `having.rs:27`、相关子查询注入路径）零修改且结果不变；`ParameterExpression` 等其余实现零修改（走默认实现）；`equals`/`gt` 等 Value 语义零修改
- Forbidden: 修改 `evaluate()` 签名；修改 Filter/DataScan/HAVING 调用点；引入 async
- Test witness: `tests/predicate_test.rs` 追加——三值表单测（Unknown AND True=Unknown、Unknown OR True=True、NOT 语义经 T2 后补）+ 既有 12 测试零修改 GREEN（变更前基线 GREEN 已验证，2026-09-10 烟雾）
- GREEN condition: predicate_test 全绿；`cargo test --test predicate_test` 退出码 0
- Verification: `cargo test --test predicate_test --test value_test`（求值语义邻接）全绿
- Stop when: 发现 fold 等价性不成立（存在既有形态行为变化）——实质语义问题，返回 Plan

### T2: 新谓词 LikePredicate / IsNullPredicate / NotPredicate

- Requirement/Scenario: R1/S3-S5、R2/S4
- Depends on: T1
- Targets: `src/executor/predicate.rs`
- Current behavior: 无（LIKE/IS NULL/NOT 不可构建）
- Required behavior: 三新 `Predicate` 实现，`evaluate`（fold）+ `evaluate_ternary` 双实现；LIKE 双 String 通配匹配、非 String→Err、NULL→Unknown；IS NULL 永不 Unknown；NOT 三值取反
- Required changes: 三个新类型 + `%`/`_` 匹配函数（贪婪回溯，模式内 `\0` 等字面量按原样）
- Preserve: 与既有谓词同一错误包装路径（"Predicate evaluation error: {e}"）；不引入新依赖
- Forbidden: 在新类型上携带 negated 字段（否定统一经 `NotPredicate` 组合）；修改既有类型
- Test witness: `tests/predicate_test.rs` 追加单测（通配矩阵：`%`/`_`/混合/无通配精确匹配；IS NULL 三态；NOT 三值取反表）——编译 RED（类型不存在）→ 实现 GREEN
- GREEN condition: 追加单测全绿
- Verification: `cargo test --test predicate_test`
- Stop when: 匹配语义需超越 String 操作数（如隐式类型转换）——契约外语义扩展，返回 Plan

### T3: planner WHERE 转换臂（IN/BETWEEN/LIKE/IS NULL/NOT）+ contains_or 扩展

- Requirement/Scenario: R1/S1-S7、R2/S1
- Depends on: T1, T2, T4（`CASE`/`COALESCE` 作为 IN 项/操作数时）
- Targets: `src/parser/planner/expression.rs::build_where` :191-238、`src/parser/planner/query.rs::contains_or` :853-870
- Current behavior: 目标变体落 `_ => UnsupportedExpression`；`contains_or` 对新变体返回 false（误下推风险）
- Required behavior: InList→OR 链（negated→Not 包装）、Between→AND(Ge,Le)（negated→Not）、Like→`LikePredicate`（ESCAPE→`ParseError`）、IsNull/IsNotNull→`IsNullPredicate`/`Not` 包装、`UnaryOp::Not`→`NotPredicate`；`contains_or` 遍历 InList/Between/Like/IsNull(IsNotNull)/Case/Cast/Function 参数
- Required changes: `build_where` 六个新臂（脱糖到 T1/T2 类型 + `build_expression` 项）；`contains_or` 扩展——`InList` 直接 true（脱糖构造即含 OR）；`Between`/`Like`/`IsNull`/`IsNotNull`/`Case`/`Cast`/`Function` 遍历子表达式
- Preserve: 既有臂（And/Or/比较/Nested）行为与代码路径不变；路由矩阵不变（BETWEEN 纯 AND → DataScan 装入 :470-489；IN 含 OR → Filter :442-469；新形态不匹配 `has_pk_equality`/`is_simple_pk_equality`）；JOIN+WHERE 拒绝不变；ILIKE/RLIKE/SimilarTo 维持 `UnsupportedExpression`
- Forbidden: 修改 `has_pk_equality`/`is_simple_pk_equality`/路由臂结构；为 IN 增加索引路由
- Test witness: `tests/expression_e2e_test.rs`（运行时 RED：今日报 Unsupported expression）+ `tests/planner_test.rs` 追加 plan 形态断言（BETWEEN → `DataScanNode.predicate=Some`、IN → FilterNode、NOT IN → Filter）+ pushdown_test 追加 BETWEEN DataScan/Filter 等价用例
- GREEN condition: 新增测试全绿 + pushdown 既有 15 用例零修改全绿
- Verification: `cargo test --test expression_e2e_test --test planner_test --test pushdown_test`
- Stop when: 脱糖导致路由矩阵意外（如 BETWEEN 被判 contains_or）且无法在不改既有路由结构的前提下修正——返回 Plan

### T4: 值表达式 CaseExpression / CoalesceExpression / CastExpression

- Requirement/Scenario: R3/S1-S5
- Depends on: T1（NULL 传播经操作数）；T3 同 Iteration 交付
- Targets: `src/executor/predicate.rs`（或相邻新文件）、`src/parser/planner/expression.rs::build_expression` :97-188
- Current behavior: `Expr::Cast`/`Expr::Case`/`Expr::Function` 落 `_ => UnsupportedExpression`（:186）
- Required behavior: 三 `Expression` 实现（`evaluate → Value`）；simple CASE 脱糖为 Eq 比较；COALESCE 经 `Expr::Function` 名匹配；CAST 严格四族映射 + 转换矩阵（design D3）；`build_expression` 三新臂
- Required changes: 类型实现 + `build_expression` `Expr::Case`/`Expr::Cast`/`Expr::Function`(COALESCE) 臂（CASE 条件经 `build_where` 递归）
- Preserve: 其余 `Expr::Function` 名维持 `UnsupportedExpression`（T03 面）；`value_from_sqlparser` 不变；Display 不用于 CAST 值格式化
- Forbidden: TRY_CAST/SAFE_CAST/未知 DataType 静默兜底；Bool↔数值转换；为 CAST 引入 plan-time 类型校验（执行期报错即满足 spec）
- Test witness: `tests/expression_e2e_test.rs` searched/simple CASE、COALESCE 三行、CAST 矩阵（含 'abc'→Int 执行期错误断言、`CAST(1.7 AS INT)`=1、NULL 短路）+ `WHERE CAST(f AS INT) >= 60` 操作数场景——运行时 RED → GREEN
- GREEN condition: 全部场景绿
- Verification: `cargo test --test expression_e2e_test`
- Stop when: CAST 语义需类型系统扩展（如 DATE 类型）或 sqlparser `Expr::Case` 字段形态与 :633 描述不符——返回 Plan

### T5: I040 INSERT 负数字面量

- Requirement/Scenario: R5/S1-S2
- Depends on: None
- Targets: `src/parser/planner/ddl_dml.rs::extract_insert_values` :107-131
- Current behavior: `Expr::UnaryOp` → `UnsupportedValue`（:119 附近）
- Required behavior: `UnaryOp{Minus, Value}` 数字字面量折叠负值；非字面量取负维持拒绝
- Required changes: 单臂新增（与 `build_expression:167-185` 同构）
- Preserve: `Expr::Value`/NULL 标识符臂与 `_ => UnsupportedValue` 兜底不变；CLI import/restore 路径自动受益（同一函数），无额外接线
- Test witness: `tests/expression_e2e_test.rs`：INSERT -1 → SELECT 验证 + 重开库持久验证；planner 断言 `-v`（列引用）仍 `UnsupportedValue`——运行时 RED → GREEN
- GREEN condition: 全绿
- Verification: `cargo test --test expression_e2e_test --test planner_test`
- Stop when: None

### T6: Iteration 000 测试见证与回归收口

- Requirement/Scenario: R1-R3/R5/R6 全场景
- Depends on: T1-T5
- Targets: `tests/expression_e2e_test.rs`（新）、`tests/predicate_test.rs`、`tests/pushdown_test.rs`（均追加）
- Current behavior: 无见证
- Required behavior: spec 六 Requirement 中本 Iteration 全部场景有断言；回归门全绿
- Required changes: 按 spec 场景矩阵组织测试（GIVEN/WHEN/THEN 注释对应）；`NotInWithNull` 等 error.rs 既有变体不误用
- Preserve: 既有测试文件零修改（只追加）；704 基线零失败
- Forbidden: 修改既有断言；为通过而放宽既有断言
- Test witness: 全量 `cargo test`（预期 ≥704+新增 全绿、0 failed、2 ignored）
- GREEN condition: `cargo test` 退出码 0；`cargo clippy --all-targets -- -D warnings` 0；`cargo fmt --check` 0 diff；`openspec validate --all` PASS
- Verification: 四命令输出（各 ≤20 行决定性片段）写入 Act Response
- Stop when: 任一既有测试失败且根因不在本 change 文件——BASELINE-CHANGED，Blocker Handoff

**Invariants**

- 既有全量测试零修改通过（704/0/2 基线）；错误文案 additive；19 plan 节点不变；plan cache 仅 Query；JOIN ON 等值限制；M13 异步原则（谓词求值为同步纯函数，不引入 await/锁）；M15 命名规范。
- `Predicate::evaluate` 直接消费方（Filter/DataScan/HAVING/相关注入）零修改。

**Non-goals**

SELECT 投影表达式（T7-T9，Iteration 001）；HAVING 新表达式（现状报错不变）；算术运算；I035/I034；IN 索引路由优化；LIKE 隐式类型转换。

**Acceptance**

spec `sql-expression-evaluation` R1/S1-S7、R2/S1-S4、R3/S1-S5、R5/S1-S2、R6/S1-S2 全部可观察并通过测试断言；映射见 change `tasks.md` RTM（Iteration 000 行）。

**Verification**

1. `cargo test`（全量，预期 ≥704+新增 全绿 / 0 failed / 2 ignored）
2. `cargo test --test expression_e2e_test --test predicate_test --test planner_test --test pushdown_test`（目标面）
3. `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check`
4. `openspec validate --all`
全部命令与决定性输出（≤20 行/项）记入 Act Response。

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | 本 Plan Context Current-State Evidence（file:line 全覆盖，2026-09-10 现场核实）+ Explorer 三代理交叉验证 + 本日烟雾基线 |
| Design | PASS | design.md D0-D8（行为差异/接口/错误语义/选择理由闭合；三值 fold 等价性论证） |
| Iteration Plan | PASS | change tasks.md Iteration Plan + 平衡审计（两 Iteration 各自内聚、依赖有序） |
| Cycle Scope | PASS | initial 范围 = T1-T6；Excluded scope 明确 |
| Task Contracts | PASS | 六任务契约含位置/行为/见证/停止条件；Act 只读本 Cycle 即可建立测试见证 |
| Traceability | PASS | RTM 六 Requirement 全 Covered，无 Simplified/Missing |
| Verification | PASS | 验证命令与通过条件明确；无身份型证据工程 |

**Persisted Evidence**

- Mode: none

Act Response（命令、决定性输出、退出码）足以承载验证结果；无不可复现结果。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- `LogicalPredicate::evaluate()` 由短路改为双侧求值：求值为纯函数（无副作用/异步），结果与错误传播顺序不变——若 Act 发现求值存在副作用路径（当前没有），停止并返回 Plan。
- LIKE 模式含 `%`/`_` 以外字符按字面量匹配；模式实现用贪婪回溯小函数，不引入 regex 依赖（Scope Control：优先平台原生能力，正则语义与 SQL LIKE 不同，不得用 regex crate）。
- Float→String 格式化用 Rust `{}`（`1.0` → `"1"`、`1.5` → `"1.5"`）；Float→Int 越界按 Rust `as` 饱和——均已文档化，非阻塞。
- sqlparser `Like.escape_char` 拒绝、`Cast.format` 忽略前先判 `Some` 拒绝——防静默语义偏移。
- 工作树含未提交 MS10-T05 现场：Act 在同一工作树上叠加修改，commit 边界由用户决定；Act Response 的 diff 审查范围以本 change 文件为限。

## Act Response

- Status: reported

**Implemented**

T1-T6 全部完成，TDD 见证齐全（E2E 全矩阵 24 测试运行时 RED → GREEN；predicate/planner/pushdown 追加先 RED 后 GREEN）：

- **T1 三值内核**：`Ternary{True,False,Unknown}` + `fold`（Unknown→False）；`Predicate::evaluate_ternary` 默认实现映射 `evaluate`；`ComparisonPredicate`（NULL→Unknown）与 `LogicalPredicate`（三值 AND/OR 表）override，两者 `evaluate()` 改走 fold。既有形态可观察行为逐字节不变（含错误传播——见 Deviations 1）。
- **T2 新谓词**：`LikePredicate`（%/_ 贪婪回溯匹配、双 String 校验、NULL→Unknown）、`IsNullPredicate`（永 Unknown）、`NotPredicate`（三值取反）；三者均 `evaluate`(fold)+`evaluate_ternary` 双实现 + `inject_parameters` 透传；否定统一经 `NotPredicate` 组合，无 negated 字段。
- **T3 planner 转换臂**：`build_where` 七个新臂——InList→Eq OR 链（negated→Not 包装，空 list 显式拒绝）、Between→AND(Ge,Le)、Like（ESCAPE→`ParseError`）、IsNull、IsNotNull→Not 包装、`UnaryOp::Not`→Not 递归；`contains_or` 扩展（InList 恒 true；Between/Like/IsNull(IsNotNull)/Cast/Case/Function 遍历子表达式）。路由矩阵与 `has_pk_equality`/`is_simple_pk_equality` 零修改。
- **T4 值表达式**：`CaseExpression`（searched 直构 + simple 脱糖为 Eq 条件，缺省 ELSE→Null，条件经三值判定 True 才命中）、`CoalesceExpression`（`Expr::Function` 名匹配 COALESCE，≥1 参数，首个非 NULL）、`CastExpression`（`CastType` 四族严格映射 `convert_cast_data_type`——未知 DataType/FORMAT/TryCast 计划期拒绝；转换矩阵：恒等、Int↔Float、String 解析失败执行期 `ValueError::TypeMismatch`、数值/Bool→String 值格式化（未用 `Value::Display`）、Bool↔数值拒绝、NULL 短路）；`build_expression` 四个新臂；三者 `set_parameter_value` 向内传播（相关子查询参数可达 CAST/CASE/COALESCE 操作数）。
- **T5 (I040)**：`extract_insert_values` 加 `UnaryOp{Minus, Expr::Value}` 臂——Int/Float 取负折叠，其余维持 `UnsupportedValue`；CLI import/restore 路径同函数自动受益。
- **T6 测试见证**：新增 `tests/expression_e2e_test.rs` 24 测试（R1×7/R2×4/R3×11/R5×2，GIVEN/WHEN/THEN 对应 spec 场景）；predicate_test +11（三值表/新谓词/NOT 组合）；planner_test +7（5 路由形态 + I040 正反 2 断言）；pushdown_test +1（BETWEEN 双路径等价 + plan 形态）。既有测试文件零修改（仅追加）。

**Changed Files and Symbols**

| File | Change |
|---|---|
| `src/executor/predicate.rs` | +`Ternary`/`fold`/`evaluate_ternary`(trait 默认)；`ComparisonPredicate`/`LogicalPredicate` 三值重写；+`LikePredicate`/`IsNullPredicate`/`NotPredicate`/`like_match`；+`CastType`/`CaseExpression`/`CoalesceExpression`/`CastExpression`（含 `set_parameter_value` 传播）；`Predicate::evaluate` 签名与既有实现零修改 |
| `src/executor/mod.rs` | predicate re-exports +8（additive） |
| `src/parser/planner/expression.rs` | `build_where` +7 臂、`build_expression` +4 臂、+`convert_cast_data_type` |
| `src/parser/planner/query.rs` | `contains_or` 扩展 7 变体；+2 import |
| `src/parser/planner/ddl_dml.rs` | `extract_insert_values` +`UnaryOp::Minus` 臂（I040） |
| `tests/expression_e2e_test.rs` | 新建，24 测试 |
| `tests/predicate_test.rs` | +11 测试（追加段，既有 12 零修改） |
| `tests/planner_test.rs` | +7 测试（追加段，既有 29 零修改） |
| `tests/pushdown_test.rs` | +1 测试（追加段，既有 15 零修改） |

不变量核查：19 plan 节点集合不变（本 Iteration 无新节点）；plan cache 仅 Query 不变；JOIN ON 等值限制不变；Filter/DataScan/HAVING 调用点零修改；错误文案 additive（新增 5 条 ParseError 文案，既有文案未动）；无新增依赖；无哈希/指纹。

**Deviations from Plan**

1. **LogicalPredicate 采用短路保持式三值求值**（Plan Guidance 写"双侧求值替代短路"）：False 支配 AND / True 支配 OR 时跳过右侧。双侧求值会在既有形态上引入新的错误暴露——如 `WHERE id > 10 AND name > 5`（右側 String>Int 本会 TypeMismatch）在左侧为 false 时，旧行为短路排除该行，双侧求值会改为报错，违反契约 Preserve 条款「错误传播不变」。短路保持式三值表与双侧求值在三值语义上完全等价（AND 有 False 即 False；OR 对偶），并使 fold 等价性精确成立。契约要求（evaluate()=fold、三值表、可观察行为不变）全部满足。
2. **R3 spec 场景 S1-S4（SELECT 投影形态）以 WHERE 谓词操作数等价形式见证**：Cycle Non-goals 明确排除 SELECT 投影（T7-T9），且现状聚合检测循环对非列 SELECT 项直接报 `InvalidAggregateArgument`（`query.rs:337`），SELECT 形态在 Iteration 000 不可达。E2E 以谓词操作数形式覆盖同一语义全分支（searched 分支/缺省 ELSE→NULL/simple operand NULL→ELSE/COALESCE 首个非 NULL+兜底/CAST 矩阵与错误面）。SELECT 形态场景断言随 Iteration 001 交付。
3. **spec R1/S3 THEN 第三项疑似笔误**：`NOT LIKE '%o%'` 对 ('Alice','Bob','Carol') 的标准语义结果为 Alice（Bob/Carol 含 o 被 LIKE 命中而排除），spec 写 Bob。测试按标准语义断言 Alice，留 Plan Review 裁定 spec 勘误。
4. **pushdown 等价断言改用 OR 组合 Filter 路径**：spec S6 描述的"与 PK 等值 AND 组合"本身限制行集，与纯 BETWEEN 行集天然不等、无法构成等价断言；等价改用 `v BETWEEN 10 AND 20 OR v = 30`（contains_or → Filter，同谓词比较经共享内核求值，行集与下推路径相等）；PK 等值组合路径另断言落 Filter 且行集正确。spec 语义（两路径产出等价、plan 断言双落点）满足。
5. **Act 自身测试勘误 2 处**（实现正确、测试期望错误，GREEN 前修正）：`A_ce` 对 "Alice" 期望应为 false（`_` 恰一字符）；`not_compound_predicate` 夹具未声明 PK 时首列 a 成为隐式键位致 a=1 重复被拒，改显式 `id INT PRIMARY KEY` 夹具。
6. **clippy collapsible_match 修复**：`contains_or` 的 Named 函数参数臂折叠为模式直匹配（fmt/clippy 收口，语义不变）。

**Blocker Handoff**

None

**Blocker Resolution**

None

**Self-Review**

- Plan compliance: 六任务契约逐项满足；Preserve/Forbidden 全项核查通过（既有测试零修改、路由矩阵/`has_pk_equality`/`is_simple_pk_equality`/JOIN 拒绝未动、无 negated 字段、无 regex 依赖、TRY_CAST/未知类型/FORMAT 无静默兜底、`Value::Display` 未用于 CAST）；Invariants 全部保持。
- Full diff reviewed: 是——9 文件完整 diff 逐块审查（含 fmt 后终态重读 predicate.rs 全文）；diffstat 中 `src/cli|resolve|create_table|insert|table_manager|recovery|cli_test|keyless_row` 的改动为未 commit 的 MS10-T05 现场基线，不属于本 change 变更面。
- Critical findings unresolved: None
- Important findings unresolved: None
- Minor findings unresolved: 见 Deviations 3（spec S3 笔误待 Plan Review 裁定）与 Deviations 2（R3 SELECT 场景断言归属 Iteration 001，需 Review 确认映射口径）。

**Verification Evidence**

| 验证项 | 命令 | 输出摘录 | 结论 |
|---|---|---|---|
| 全量测试 | `cargo test` | `passed: 747, failed: 0, ignored: 2`（704 基线 + 43 新增；2 ignored 为既有信号标定项） | PASS |
| 目标面 | `cargo test --test expression_e2e_test --test predicate_test --test planner_test --test pushdown_test` | 24 + 23 + 36 + 16 全部 passed / 0 failed | PASS |
| RED 见证 | 实施前 `cargo test --test expression_e2e_test` | `20 failed`（全部 `Plan error: Unsupported expression type`）+ 4 既有行为守卫 passed；predicate/planner/pushdown 追加各自编译/运行 RED 在案 | PASS |
| Lint | `cargo clippy --all-targets -- -D warnings` | 0 error 0 warning（退出码 0） | PASS |
| 格式 | `cargo fmt --check` | `FMT CLEAN`（退出码 0；diff 全部位于本 change 触及文件） | PASS |
| OpenSpec | `openspec validate --all` | `Totals: 20 passed, 0 failed (20 items)` | PASS |

**Persisted Evidence**

None required（Plan Mode: none；验证结果均可低成本复现，Act Response 承载）。

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook / Incident | None | | |

**Remaining Issues**

1. spec R1/S3 第三项 THEN（Bob）与标准 LIKE 语义矛盾（应为 Alice）——待 Plan Review 裁定 spec 勘误（Deviations 3）。
2. R3 场景 S1-S4 的 SELECT 形态断言归属 Iteration 001（T9 witness）——待 Plan Review 确认 RTM 映射口径（Deviations 2）。
3. Iteration 001（T7-T9 SELECT 派生列）依赖本 Iteration 的值表达式实现；其 ProjectionExecutor 求值应走 owned `evaluate` 路径（新表达式的 `evaluate_ref` 零拷贝路径对 String 结果显式报错，见 predicate.rs 注释）。

**Commit or Diff Reference**

未 commit（工作树叠加于未 commit 的 MS10-T05 现场之上，基线 `a5b0a5f`；commit 边界由用户决定）。本 change 变更面：`src/executor/{mod,predicate}.rs`、`src/parser/planner/{expression,query,ddl_dml}.rs`、`tests/{predicate,planner,pushdown}_test.rs`（追加）、`tests/expression_e2e_test.rs`（新建）。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（不依赖 Act Self-Review）全部通过：

1. **验证复现**：全量 `cargo test` 747 passed / 0 failed / 2 ignored（与 Act 声称一致）；`cargo clippy --all-targets -- -D warnings` 0、`cargo fmt --check` CLEAN、`openspec validate --all` 20 PASS（2026-09-10 Plan 新鲜复跑）。
2. **Diff 逐块审查**：三值内核（`Ternary`/`fold`/trait 默认 `evaluate_ternary`/Comparison 与 Logical 三值重写、`evaluate()`=fold）、`LikePredicate`/`IsNullPredicate`/`NotPredicate`（含 `inject_parameters` 透传）、planner 七个 WHERE 臂 + 四个 expression 臂、`contains_or` 扩展（InList ⇒ true 口径与 Plan 一致）、I040 臂——与 Task Contract 逐条一致。三个测试文件 **0 删除**（零修改成立）；路由矩阵/`has_pk_equality`/`is_simple_pk_equality`/JOIN 拒绝未动；`src/cli|resolve|create_table|insert|table_manager|recovery` 等改动确认为未 commit 的 MS10-T05 现场基线，不属本 change。
3. **二进制语义探针**（独立于测试套件，temp 库实跑）：`IN (1,3)`、`NOT IN (2, NULL)`（0 行，三值正确）、`BETWEEN 2 AND 3`、`IS NULL`、`NOT LIKE '%o%'`、负数字面量 INSERT + 非 PK 谓词可达（`WHERE v = -7` → `["neg",-7]`）全部符合 spec。
4. **SELECT 面现状确认**（Iteration 001 的 RED 基线）：`SELECT CASE...` → `Invalid aggregate argument: Expected column name`；`SELECT COALESCE(...)`/`SELECT CAST(...)` → `Unsupported statement type`（`ast.rs::extract_columns` Function 臂先于聚合检测）。符合 Non-goals 边界。
5. **既有登记项现象复现（非本 change 回归）**：探针观察到 I034（裸 DataScan 子集投影 CLI 表头返回全 schema）与 I036（String 首列隐式 PK 表 `WHERE name = 'neg'` 等值不可达、非 PK 谓词可达）——均为已登记 planned I 项。
6. **基线 flaky（非阻塞，非本 change 变更面）**：`cli::resolve::tests::test_db_dir_env_cases`（`src/cli/resolve.rs:105`，MS10-T05 基线文件）6 次复跑 1 次失败——进程全局 env 变量（`HOME`/`RTSQL_HOME`）在并行测试线程下竞态；完整套件重跑 747/0/2 全绿。建议经 docs-maintainer 登记 I 项（测试稳定性）或后续 change 修复，不阻塞本 Cycle。
7. **spec R1/S3 笔误核实成立**：`NOT LIKE '%o%'` 对 ('Alice','Bob','Carol') 标准语义为 Alice（Bob/Carol 含 `o`）——Plan 已勘误 spec（Bob → Alice），Act 测试断言正确。

**Deviation Classification**

- **PLAN-OMISSION ×1（非阻塞）**：R3 S1-S4 的 SELECT 投影形态场景在 RTM 中映射到 Iteration 000，但该形态依赖 Iteration 001 的投影机制，Iter 000 内不可达。Act 以 WHERE 谓词操作数等价形式完成语义见证并将 SELECT 形态断言移交 T9。Plan 已修正 RTM 口径（R3 行注记），验收语义无损失。
- **PLAN-INVALID ×1（非阻塞）**：spec R1/S3 THEN 笔误（Bob 应为 Alice）。Plan 已勘误。
- **ACT-DEVIATION ×4（全部接受，非实质局部差异）**：① LogicalPredicate 短路保持式三值求值（替代 Guidance 的双侧求值）——三值语义等价且精确保留既有错误传播（`WHERE id > 10 AND name > 5` 左假短路不暴露右侧 TypeMismatch），优于原 Guidance；② pushdown 等价断言改用 OR 组合 Filter 路径（PK-AND 组合限制行集、无法构成等价断言）——plan 双落点断言与行集等价齐全，spec 语义满足；③ 2 处测试期望勘误（`A_ce` 对 Alice、夹具显式 Int PK）——实现正确、测试作者笔误；④ clippy collapsible_match 折叠——fmt/clippy 收口。

**Acceptance Gaps**

None——R1/S1-S7、R2/S1-S4、R3/S1-S5（SELECT 形态断言按修正口径归 Iter 001）、R5/S1-S2、R6/S1-S2 全部有证据满足。

**Convergence**

N/A（首次 Review；以本 Cycle Plan Context 为基线）

**Evidence**

- `cargo test` → `747 passed; 0 failed; 2 ignored`（Plan 独立复跑 2026-09-10）
- `cargo clippy --all-targets -- -D warnings` → 0；`cargo fmt --check` → CLEAN；`openspec validate --all` → `20 passed, 0 failed`
- 探针输出（temp 库）：NOT IN 含 NULL 0 行 / IN、BETWEEN、IS NULL、NOT LIKE 行集正确 / 负数行非 PK 谓词可达 / SELECT 面两态错误文案
- flaky 复现：`for i in 1..6; cargo test --lib` → run 3 `test_db_dir_env_cases ... FAILED`，其余 5 次 195 全绿
- diff 审查：`git diff --numstat` 测试文件 0 删除；`query.rs` diff 仅 contains_or + imports

**Follow-up Decision**

接受（accepted）。理由：Acceptance 全满足；4 项 ACT-DEVIATION 均为非实质局部差异且记录完整；2 项 Plan 侧问题（RTM 口径、spec 笔误）已由 Plan 在本 Review 内修正（change 自有产物，非 Act 范围）。Act Remaining Issues 1/2 随本 Review 关闭（spec 已勘误、RTM 口径已修正）；Remaining Issues 3（ProjectionExecutor 须走 owned `evaluate` 路径，新表达式 `evaluate_ref` 对 String 结果报错）已写入 Iteration 001 Plan Context 作为硬约束。

**Iteration Plan Update**

None（Iteration Map 不变；R3 映射口径为 RTM 注记修正，非范围/验证契约变化）

**Next Cycle**

None

**Next Iteration**

`iterations/001-select-projection/000-initial.md`（已展开，Status: ready）
