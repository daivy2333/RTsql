# design — MS11-T01 SQL 表达式四件套与值表达式

> 采集 revision：`a5b0a5f` + 未提交 MS10-T05 工作树（2026-09-10）。行号以该现场为准。

## D0 行为差异总览

| 维度 | 当前行为 | 目标行为 |
|---|---|---|
| WHERE 表达式 | 仅六比较 + AND/OR + Nested | + [NOT] IN 列表 / [NOT] BETWEEN / [NOT] LIKE / IS [NOT] NULL / NOT |
| NULL 语义 | 两值（比较遇 NULL → Ok(false)，AND/OR 布尔短路） | 内部三值 True/False/Unknown；行选择折叠 Unknown → 不匹配；既有形态可观察结果不变 |
| 值表达式 | 无（build_expression 只认列/字面量/负数字面量） | CASE（searched/simple）/COALESCE/CAST 可作谓词操作数与 SELECT 投影项 |
| SELECT 投影 | 纯列索引裁剪（6 节点 `projection: Vec<usize>`）；非常量名回退恒等（`SELECT 42 FROM t` 返回全 schema 行——无测试锁定，修正为单列输出） | 含表达式项时外层包 `ProjectionNode` 逐项求值；纯列查询 plan 逐字节不变 |
| INSERT 字面量 | `UnaryOp::Minus` → `UnsupportedValue`（`ddl_dml.rs:119`） | 负数字面量折叠入库 |

## D1 三值求值内核（executor/predicate.rs）

- 新增 `pub enum Ternary { True, False, Unknown }` 与 `fn fold(t: Ternary) -> bool`（Unknown → False）。
- `Predicate` trait 新增 `evaluate_ternary(&self, row: &[Value]) -> Result<Ternary, ...>`，**默认实现** = `self.evaluate(row).map(|b| if b { True } else { False })`——既有实现（ParameterExpression 等）零修改即正确。
- `ComparisonPredicate::evaluate_ternary`：任一操作数求值为 NULL → `Unknown`；否则比较映射 True/False。`evaluate()` 重写为 `evaluate_ternary().map(fold)`——与现 `predicate.rs:82-85` 的 NULL→`Ok(false)` 行为逐字节等价（直接 `evaluate()` 调用者 Filter/DataScan/HAVING 无感知）。
- `LogicalPredicate::evaluate_ternary`：AND = 有 False 则 False，否则有 Unknown 则 Unknown，否则 True；OR 对偶。`evaluate()` 同样改走 fold。现布尔短路改为双侧求值——求值为纯函数，可观察结果与错误传播不变。
- 新谓词（`predicate.rs` 内新增，同职责域）：
  - `LikePredicate { expr: ExpressionRef, pattern: ExpressionRef }`：两侧求值为 String 后执行 `%`/`_` 通配匹配（贪婪回溯，模式为常量或表达式均可，逐行求值）；任一侧非 String → `Err`（经 "Predicate evaluation error" 包装）；expr 为 NULL → Unknown（模式为 NULL → Unknown）。
  - `IsNullPredicate { expr: ExpressionRef }`：求值为 NULL → True，否则 False（永 Unknown）。
  - `NotPredicate { inner: PredicateRef }`：`evaluate_ternary` 三值取反。
- 否定形式统一在 planner 侧脱糖为 `NotPredicate` 包装（negation 单点实现），新增谓词类型本身不带 negated 字段。

## D2 IN / BETWEEN 的 planner 脱糖（零新执行器类型）

- `x IN (a, b, c)` → `OR(Eq(x,a), OR(Eq(x,b), Eq(x,c)))`（既有 `LogicalPredicate`/`ComparisonPredicate` 组合）；`NOT IN` → `Not(OR 链)`。三值语义自动正确（`v NOT IN (2, NULL)` = Not(False∨Unknown) = Unknown → 排除；`v IN (2, NULL)` v=1 → Unknown → 排除）。
- `x BETWEEN low AND high` → `AND(Ge(x,low), Le(x,high))`；`NOT BETWEEN` → `Not(AND)`。
- 交叉验证：`equals`（`executor/value.rs:118-137`）跨类型 → False、同类型比较直给——脱糖后与既有等值语义同源；NULL 短路在 `ComparisonPredicate::evaluate_ternary` 层拦截（list 项 NULL → Unknown，不走 `equals` 的 NULL==NULL→true 分支）。
- 下推路由自动成立：BETWEEN 脱糖为纯 AND → `contains_or`=false、`has_pk_equality`=false → 装入 `DataScanNode.predicate`（`query.rs:470-489` pushdown 臂）；IN 脱糖含 OR → 保留 Filter（`query.rs:451-469`）——语义正确，优化留给后续。新变体天然不匹配 `is_simple_pk_equality`/`has_pk_equality`（二者只认 `BinaryOp::Eq` 形态，`query.rs:675-740`），不会落入 I036 的索引不可达陷阱。
- `contains_or`（`query.rs:859-870`）扩展：`InList` → **直接 true**（脱糖构造即含 OR，保守基线，IN 全路径落 Filter）；`Between{expr,low,high}`、`Like{expr,pattern}`、`IsNull/IsNotNull(inner)`、`Case{operand,conditions,results,else_result}`、`Cast(inner)`、`Function` 参数 → 遍历子表达式（这些脱糖/求值树仅当子项含 OR 才含 OR）。`UnaryOp` 臂已存在（:866）覆盖 NOT。**不同步扩展会让 CASE 条件内的 OR 被误下推（`_ => false`），属正确性缺陷。**

## D3 值表达式（Expression trait 实现，谓词操作数与投影共用）

- `CaseExpression { whens: Vec<(PredicateRef, ExpressionRef)>, else_: Option<ExpressionRef> }`：
  - searched 形态直接构建（条件经 `build_where` → `PredicateRef`，三值判定：True 取该结果，Unknown/False 继续）。
  - simple 形态（`CASE operand WHEN v ...`）在 planner 脱糖为 searched：条件 = `ComparisonPredicate(Eq(operand, v))`——operand NULL → Unknown → 不命中（spec 场景），复用同一比较语义。
  - 缺省 ELSE → `Value::Null`；结果表达式求值即返回（不做类型统一）。
- `CoalesceExpression { args: Vec<ExpressionRef> }`：首个非 NULL 参数值；全 NULL → Null；参数错误传播。sqlparser 0.44 无专用变体——`Expr::Function` 且名为 `COALESCE`（大小写不敏感）时进入；其余函数名维持 `UnsupportedExpression`（T03 面）。
- `CastExpression { expr: ExpressionRef, target: CastType }`，`CastType ∈ {Int, Float, String, Bool}`：
  - 类型映射用**严格版** `convert_data_type` 逻辑（`ddl_dml.rs:133-160` 的四族 match），不认ColumnType::String 默认兜底——未知 DataType（DATE 等）规划期报 `ParseError`；TRY_CAST/SAFE_CAST 显式拒绝。
  - 转换矩阵：恒等（同型原样）；Int↔Float（`as` 转换，Float→Int 截断向零、越界饱和——Rust `as` 语义，文档化）；String→{Int,Float,Bool} 解析失败 → `Err(ValueError)`（执行期，"Execution error" 包装）；{Int,Float,Bool}→String 用值格式化（**不得用 `Value::Display`**——它给 String 加引号，`value.rs:267-278`）；Bool↔数值拒绝。
  - CAST NULL → NULL（expr 求值为 Null 直接短路）。
- 全部实现 `Expression::evaluate`（`predicate.rs:23-46` 的 trait），作为 `ComparisonPredicate`/`LogicalPredicate` 的操作数自动获得三值 NULL 传播（操作数 NULL → Unknown）。
- `build_expression` 新臂：`Expr::Cast`、`Expr::Case`、`Expr::Function`(COALESCE)。`Expr::InList/Between/Like/IsNull` 不是值表达式——若出现在比较操作数位置落 `_ => UnsupportedExpression`（合理：SQL 亦不允许）。

## D4 SELECT 投影表达式机制（Iteration 001，新增 ProjectionNode）

- 新 plan 节点 `ProjectionNode { input: Box<PhysicalPlan>, items: Vec<ProjectionItem>, columns: Vec<String> }`，`ProjectionItem { expr: ExpressionRef, name: String }`。列引用项统一表达为既有 `ColumnExpression{column_name, column_index}`（求值 = row[index]）。
- 触发条件：SELECT 列表**任一**项不是普通列标识符（表达式/字面量/带别名的表达式）→ 最终 plan 外层包 `ProjectionNode`（最外层，LIMIT 之上），所有项按表达式求值。全部为普通列 → 走既有 `resolve_projection_indices` 路径，plan 逐字节不变（既有测试断言 `node.projection` 的兼容性关键）。
- 既有 6 节点的 `projection: Vec<usize>` 字段与执行器**零修改**；含 ORDER BY + 表达式项时 `proj_or_empty` 置空（全行流过 Sort），投影在顶层 ProjectionNode 统一裁剪+求值——排序键（基础列）始终可达，符合 design D10 精神。
- 限制（显式报错，不静默）：表达式项与标量子查询项（SubqueryEval）混用 → 拒绝（子查询列追加会移位输出形状）；`SELECT *, expr` 通配混用 → 拒绝。子查询单独出现维持现状。
- 接线点（编译器穷尽 match 引导）：
  - `pipeline.rs::create_executor_from_plan` 新增 `ProjectionNode` 臂 → 新 `ProjectionExecutor`（逐行 `items.map(eval(row))`；输入为全形状行，`ColumnExpression.column_index` 按 full-schema 解析）。
  - `pipeline.rs::get_plan_output_columns`（实际定义在 `planner/query.rs:23-83`，CLI 调用点 `cli/mod.rs:286`）新增臂 → `node.columns`。
  - `executor/correlated.rs::inject_correlated_values`（:14，walk 计划树注入外层参数）新增透传臂（递归 input）。
  - `plan.rs` 枚举新增变体（19 → 20；M01 的节点清单由 docs-maintainer 在 change 收尾同步，非本 change 代码义务）。
- 列名规则：`SelectItem::ExprWithAlias{alias}` → `alias.value`；`UnnamedExpr` → sqlparser `Expr` Display 文本（`expr_to_column_name` 不复用——其字面量 `_42` 前缀规则与 Display 规则不同；新规则下 `SELECT 42` 列名为 `42`）。
- `ast.rs::extract_columns`/`extract_qualified_columns`（:36-164，聚合检测前调用，`query.rs:290-300`）需扩展遍历新变体收集列引用（CASE 条件/COALESCE 参数/CAST 内的列参与 JOIN 输出过滤与投影解析），未知函数名报 `UnsupportedStatement` 的现状对 T03 前保持不变（COALESCE 需放行——它此刻是合法函数）。WHERE 侧无此需求（build_expression 直接解析列，JOIN+WHERE 本就拒绝）。
- 聚合查询含非聚合表达式项 → 保持现有 `InvalidAggregateArgument` 报错（`query.rs:326-359` 的检测循环不改判定顺序）。

## D5 INSERT 负数字面量（I040）

- `ddl_dml.rs::extract_insert_values`（:107-131）`Expr::UnaryOp { op: Minus, expr: Value }` 臂：`value_from_sqlparser` 后 Int 取负 / Float 取负，其余保持 `UnsupportedValue`；列引用取负维持拒绝。

## D6 错误面

- 规划期：新形态不识别维持 `UnsupportedExpression`（"Unsupported expression type"）；显式拒绝的形态（ESCAPE 子句、TRY_CAST/SAFE_CAST、ILIKE/RLIKE/SIMILAR TO、未知 DataType、通配/子查询混用）用 `PlanError::ParseError(<具体说明>)`——additive 文案，不改既有。
- 执行期：LIKE 非 String、CAST 解析失败 → `ValueError` 变体经既有谓词求值错误包装（Filter `filter.rs:53-58` / DataScan `data_scan.rs:169-187` 同文案模板）成为 `Execution error`，CLI exit 3。

## D7 实现顺序与依赖

1. 三值内核 + 新谓词（D1）——后续一切的地基，独立可测（predicate_test）。
2. planner WHERE 转换 + 脱糖 + contains_or 扩展（D2/D5 前半）——依赖 1；pushdown/planner 测试立即可断言 plan 形态。
3. 值表达式三实现 + build_expression 臂（D3）——依赖 1；WHERE 操作数场景（`WHERE CAST(f AS INT) >= 60`）打通。
4. I040（D5）——独立，随 Iteration 000 顺带。
5. ProjectionNode 机制（D4）——依赖 3 的表达式实现；独立 Iteration 交付 SELECT 面。

## D8 测试策略

- `tests/predicate_test.rs`：Ternary/新谓词/Not/三值表单测（pattern.rs 同模式）。
- `tests/planner_test.rs` 邻接新增或独立 `tests/expression_planner_test.rs`：plan 形态断言（BETWEEN → DataScan.predicate、IN → Filter、NOT 包装、simple CASE 脱糖、I040 值折叠）。
- 新增 `tests/expression_e2e_test.rs`：spec 六 Requirement 的 GIVEN/WHEN/THEN 全矩阵（Database::execute_sql 级，含 NULL 三值场景、错误面 exit 语义由 lib 错误断言覆盖）。
- `tests/pushdown_test.rs` 模式复用：新谓词 DataScan vs Filter 等价（不改既有 15 测试，追加）。
- Iteration 001：新增 `tests/projection_expression_test.rs`（派生列/别名/列名/混合/`SELECT 42` 怪癖修正/聚合报错保持/渲染兼容走 cli_test 追加）。
- 回归门：全量 `cargo test`（704 基线）零既有修改 + `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check` + `openspec validate --all`。
