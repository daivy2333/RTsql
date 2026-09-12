# Iteration 000 / Cycle 000-initial: 注册机制与 string 函数

## Plan Context

- Status: draft
- Iteration: 000-mechanism-strings
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3
- Depends on: None
- Stable baseline: 注册机制可扩展；string 六函数（upper/lower/length/substr/replace/trim）在 WHERE 与 SELECT 双侧全语义可用；R1/R2/R4 验收关闭；math 函数保持拒绝（未注册）
- Verification boundary: T3 e2e 全绿 + function.rs 单测绿 + 全量回归 0 failed + clippy/fmt 0 + validate PASS
- Diagnostic boundary: `src/executor/function.rs`、`src/parser/planner/expression.rs`、`src/executor/mod.rs`、`tests/scalar_function_test.rs`
- Deferred tasks: T4, T5, T6（Iteration 001）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: spec `sql-scalar-functions` R1/R2/R4 全部场景；design D1-D7
- Excluded scope: math 四函数、R5 调用面场景、CLI 表头用例、窗口函数/UDF/日期时间、ORDER BY 别名排序能力

**Objective**

`SELECT upper(name) FROM t`、`SELECT upper(name) AS u`、`WHERE upper(name) = 'X'`、`WHERE length(d) > 3` 等 string 函数形态全语义可用；未知名/OVER/DISTINCT/arity 拒绝面按文案契约生效；NULL 传播与嵌套参数语义落地。

**Background**

tasks MS11-T03 + proposal.md（用户 2026-09-10 裁定 4 项语义 + 2 项默认假设）；R18 主题 7。MS11-T01 已就位的表达式内核（CASE/COALESCE/CAST 三先例）是本 Cycle 的直接模板。

**Current Baseline**

- revision 179228b（master，工作区 clean），797 tests pass / 0 failed / 2 ignored（2026-09-10 MS11-T02 收尾独立复跑，SNAPSHOT 记录）。
- 当前非聚合非 COALESCE 函数调用全部 `PlanError::UnsupportedExpression`（"Unsupported expression type"）。
- 表头实证：函数项默认列名按书写形态回放（`SELECT COALESCE(name, 'x')` → 列名 `COALESCE(name, 'x')`，二进制探针 2026-09-10）。
- ORDER BY 表达式别名实证：非空表静默保持输入序、exit 0（二进制探针；sort.rs:82-95 未命中排序列即 Equal）。

**Current-State Evidence**

- `Expr::Function` 唯一现有臂：`src/parser/planner/expression.rs:272-294`——COALESCE 构造 `CoalesceExpression`，其余 `PlanError::UnsupportedExpression`；注释点名 MS11-T03。函数参数经 `FunctionArg::Unnamed(FunctionArgExpr::Expr(e))` 解包后递归 `build_expression`。
- Expression trait 双入口：`src/executor/predicate.rs:50-73`（owned `evaluate` 默认委托 `evaluate_ref`）。三个新值表达式先例的 `evaluate_ref` 模式：物化 owned 行 → Copy 变体回借 → String 结果显式报错（predicate.rs:478-495 CaseExpression 为模板）。
- `set_parameter_value` 递归先例：predicate.rs:497-507；关联子查询注入调用点 `src/executor/correlated.rs:61-65`（对投影项 `item.expr.set_parameter_value(...)`）——漏实现静默返回 false。
- ProjectionExecutor 只走 owned `evaluate`：`src/executor/projection.rs:26-45`（模块注释为 MS11-T01 Iter000 Plan Review 硬约束：新值表达式禁止依赖 evaluate_ref 的 String 结果）。
- NULL 语义先例：`CastExpression::cast_value` NULL 短路（predicate.rs:561-563）；严格类型 `ValueError::TypeMismatch`（LIKE，predicate.rs:231-237）。
- SELECT 表达式项路由：query.rs:343-385 检测（`is_aggregate_expr` 先行、`is_plain_column_expr` 排除列引用）→ query.rs:390-451 裁决（聚合混用/子查询混用/JOIN/通配符四拒绝）→ `ProjectionItem { expr, name: expr.to_string() }` → 顶层 ProjectionNode（query.rs:768-778）。
- WHERE 路由零改动依据：`contains_or` 递归扫描函数参数（query.rs:1009-1013）；`extract_pk_from_where` 只认 `Expr::Value` 右侧（query.rs，本 change 补查）→ 函数比较落普通扫描；无 OR 非 PK 谓词进 DataScan 行内下推（MS07-T06），行级求值时全形状（投影裁剪在谓词后，MS10-T01）。
- 聚合名分派先例：`f.name.to_string().to_uppercase()` match（planner/aggregate.rs:49-89、179-184）；`is_aggregate_expr` 在 SELECT 路由早于表达式项检测，聚合名不会到达标量臂的注册表查询（HAVING 的 `build_having_expression` 也只认聚合名，标量名维持其既有 `UnsupportedExpression`）。
- 负数字面量参数：`Expr::UnaryOp { Minus, Value }` 臂在 `build_expression` 递归中命中 → ConstantExpression（expression.rs:296-314，Int/Float 均折叠，I040）。
- executor re-export 模式：`src/executor/mod.rs:36-46` 按模块 `pub use`，新模块追加两行。
- e2e 模板：`tests/expression_e2e_test.rs` 头部 helpers（`open_db/exec_ok/query_rows/error_message`，24 测试）+ `tests/projection_expression_test.rs`（16 测试）；单元测试放 `src/executor/function.rs` 内 `#[cfg(test)]`。
- 验证基线命令：`cargo test`（≥797 且 0 failed）、`cargo clippy -- -D warnings`（0）、`cargo fmt --check`（0 diff）、`openspec validate`（PASS）。

**Relevant Code**

- `src/parser/planner/expression.rs` — `PlanBuilder::build_expression`（函数臂所在）、`convert_cast_data_type`（严格映射风格参照）。
- `src/executor/function.rs`（新建）— 注册表、校验入口、`FunctionExpression`、十函数中本 Cycle 六个、单测。
- `src/executor/mod.rs` — re-export。
- `src/executor/value.rs` — `Value`/`ValueError`（错误来源）。
- `tests/scalar_function_test.rs`（新建）— R1/R2/R4 e2e。

**Critical Path**

`SELECT/WHERE` → `PlanBuilder::build_select`（query.rs）→ `build_expression`/`build_where` 的 `Expr::Function` 臂 →（COALESCE 既有路径 | 注册表校验 → `FunctionExpression` 构造）→ PhysicalPlan::Projection/DataScan predicate/Filter predicate → 执行器逐行 `Expression::evaluate`（owned）→ `FunctionExpression` 分派 → `Value`。错误路径：plan 期 `PlanError::ParseError`/`UnsupportedExpression`（CLI exit 3 文案透传）；执行期 `ValueError::TypeMismatch` 经执行器包装为 `ExecutionError`。

**Implementation Guidance**

- D2 单点注册表：`src/executor/function.rs` 同时承载 planner 校验入口（如 `pub fn check_scalar_function(name: &str, argc: usize) -> Result<(), String>`——Err 为点名文案，planner 包成 `PlanError::ParseError`）与求值分派，名单不漂移。未注册名由 planner 侧维持 `PlanError::UnsupportedExpression`（不经校验入口，保证既有文案逐字节不变）。
- planner 臂顺序：COALESCE 既有构造保持 → 聚合五名不进标量（保持既有 `UnsupportedExpression`，与 change 前一致——聚合名在值位置本就报此错）→ `over.is_some()/distinct/filter.is_some()/null_treatment.is_some()/!order_by.is_empty()` → 点名 `PlanError::ParseError` → 参数解包（Named/Wildcard 点名拒绝）→ `check_scalar_function(name, argc)` → 构造 `FunctionExpression`。
- D3 求值顺序（契约）：按序求值全部参数（错误传播）→ 任一 NULL → NULL（跳过类型校验）→ 类型校验（TypeMismatch）→ 计算。`upper/lower/trim` 结果的 `ValueRef` 不可回借（无 backing storage）——`evaluate_ref` 沿三先例报错；`length`/`abs`(Int) 产出 Copy 变体可回借。
- substr 按字符（`s.chars()`）计数；SQLite 边缘按 spec R2/S4 五个断言值实现；`replace` 的 `from` 为空串原样返回；`trim` 仅剥 `' '`（`trim_matches(' ')`）。
- CLI 错误断言用关键词匹配（点名构造的文案），与 D4 文案契约一致；不锁全文。

**Behavioral Change**

- 当前：十个目标函数 + 任何非聚合非 COALESCE 函数名 → `Unsupported expression type`。
- 目标：六个 string 函数双侧可用（语义按 spec R2）；未知名文案不变；OVER/DISTINCT/FILTER/命名参数/通配符/零参/arity → plan 期点名 SQL 错误；NULL 入参出 NULL；非 String 入参执行期 TypeMismatch。
- 接口：新增 `src/executor/function.rs` 公共项（`FunctionExpression` + 校验入口）；`PlanError`/`ValueError` 无新变体；PhysicalPlan/ProjectionItem/渲染/plan cache 键零变化。

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R1/S1-S4、R2/S1-S2(upper/lower) | `src/executor/function.rs`(新)、`src/executor/mod.rs`、`src/parser/planner/expression.rs::build_expression` | 臂内仅 COALESCE、其余拒绝 | 新建注册表模块 + 臂扩展（校验/拒绝面/构造）+ re-export |
| T2 | R2/S1-S6、R4/S1-S3 | `src/executor/function.rs`（string 段） | —（T1 后仅 upper/lower） | length/substr/replace/trim 语义 + NULL 层 + 严格类型 + 单测 |
| T3 | R1/S1-S4、R2/S1-S6、R4/S1-S3 | `tests/scalar_function_test.rs`(新) | — | e2e 全场景（先建 RED 见证，T1/T2 后转绿） |

**Task Contracts**

### T1: 注册机制与 planner 臂（upper/lower 打通）

- Requirement/Scenario: R1/S1（未知名文案不变）、S2（OVER）、S3（DISTINCT）、S4（arity）；R2/S1-S2（upper/lower 表头与别名）
- Depends on: None
- Targets: `src/executor/function.rs`（新）、`src/executor/mod.rs`、`src/parser/planner/expression.rs::build_expression` 的 `Expr::Function` 臂
- Current behavior: 非聚合非 COALESCE 函数名 → `PlanError::UnsupportedExpression`；`SELECT COALESCE(...)` 可用
- Required behavior: `upper/lower` 单参可用（SELECT 默认表头按书写形态、AS 别名生效、WHERE 比较谓词中可用）；未知名文案逐字节不变；OVER/DISTINCT/FILTER/命名参数/通配符/零参/arity 不符 → `PlanError::ParseError` 点名文案（含函数名与构造名）
- Required changes: 新模块（元数据表 + 校验入口 + `FunctionExpression` + upper/lower 实现 + 单测）；臂扩展按 Implementation Guidance 顺序；`mod.rs` re-export
- Preserve: COALESCE 分支行为与构造逐字节保持；聚合五名在标量臂维持既有 `UnsupportedExpression`；`ProjectionItem`/`ProjectionNode`/渲染/plan cache 键不动；Ternary/fold 语义不动
- Forbidden: 修改 `src/pipeline.rs`、`src/cli/`、`src/executor/projection.rs`、`src/executor/aggregate.rs`、`src/parser/planner/query.rs`（臂以外的路由）；新增哈希/校验和
- Test witness: `tests/scalar_function_test.rs` R1/R2 段先建（RED：全部 `Unsupported expression type`，命令 `cargo test --test scalar_function_test`，实施前实测记录）；`function.rs` 单测随模块建立
- GREEN condition: R1/S1-S4 + R2/S1-S2 e2e 绿；function.rs 单测绿
- Verification: `cargo test --test scalar_function_test`、`cargo test --lib`（0 failed）；`cargo clippy -- -D warnings` 0
- Stop when: 臂扩展需要动 SELECT 聚合检测路由或 `ProjectionItem` 结构；注册表与既有 `is_aggregate_expr` 出现职责冲突

### T2: string 六函数语义补全

- Requirement/Scenario: R2/S1-S6（length/substr/replace/trim 全语义、严格类型）、R4/S1-S3（NULL 传播、短路优先、嵌套参数）
- Depends on: T1
- Targets: `src/executor/function.rs`（string 函数段 + NULL 层）
- Current behavior:（T1 后）仅 upper/lower 可用；length/substr/replace/trim 未注册
- Required behavior: `length` 字符计数（`你好`=2）；`substr` 五个 spec 断言值（R2/S4）；`replace` 全替换、空 `from` 原样；`trim` 仅剥空格；非 String 入参执行期 TypeMismatch；任一参数 NULL → NULL 且跳过类型校验（`upper(CAST(NULL AS INT))` → NULL）；参数求值错误先传播
- Required changes: 四函数实现 + D3 求值顺序落地（全部参数求值 → NULL 短路 → 类型校验）+ 单测（含 NULL/类型/边缘矩阵）
- Preserve: D3 顺序契约；不做隐式转换；CAST 既有矩阵不动
- Forbidden: 修改 `CastExpression`/`CoalesceExpression`/`CaseExpression`；引入 String → 数值隐式解析
- Test witness: `function.rs` 单测先行（RED：函数未注册断言失败）；`tests/scalar_function_test.rs` R2/S4-S6、R4 段（RED 依赖 T3 文件已建，随 T1/T2 推进转绿）
- GREEN condition: 单测 + 对应 e2e 全绿
- Verification: `cargo test --lib`、`cargo test --test scalar_function_test`
- Stop when: SQLite 边缘语义与 spec 场景断言冲突无法同解（返回 Plan 裁定）

### T3: R1/R2/R4 e2e 集成测试

- Requirement/Scenario: R1/S1-S4、R2/S1-S6、R4/S1-S3 全覆盖
- Depends on: T1, T2（GREEN 依赖）；测试文件建立先于 T1 实施（RED 见证）
- Targets: `tests/scalar_function_test.rs`（新）
- Current behavior: 目标形态全部 `Unsupported expression type`（change 前基线）
- Required behavior: 23 场景中本 Iteration 的 13 个（R1×4、R2×6、R4×3）断言按 spec 逐条落地；复用 `expression_e2e_test.rs` 的 helpers 模式（`open_db/exec_ok/query_rows/error_message`）
- Required changes: 新测试文件 + 头部 RED 记录注释（对齐 MS11-T01 惯例）
- Preserve: 既有测试文件零修改；不修改 helpers 来源文件
- Forbidden: 断言锁全文错误消息（用关键词）；为断言引入产品代码改动
- Test witness: 实施前 `cargo test --test scalar_function_test` RED 输出记录于 Act Response
- GREEN condition: 本文件全绿
- Verification: `cargo test --test scalar_function_test`（全绿、0 failed）
- Stop when: 某场景无法在不改产品契约的前提下断言（如错误消息在多层包装后丢失关键词）

**Invariants**

- 既有 797 测试零修改通过；测试总数只增不减。
- `COALESCE` 臂、聚合路径、Ternary/fold、plan cache 键、渲染层、pipeline/CLI 均不变。
- 错误文案契约（D4）：未知名逐字节不变；点名文案含函数名与构造名。
- 新值表达式的 `evaluate_ref` 遵守三先例模式（String 结果报错），`set_parameter_value` 递归 args。

**Non-goals**

- math 四函数、R5 调用面场景、CLI 用例（Iteration 001）
- 窗口函数、UDF、聚合扩展、日期/时间、ORDER BY 别名排序能力、no-FROM SELECT（I035）

**Acceptance**

- R1/S1-S4、R2/S1-S6、R4/S1-S3 全部可观察断言通过（`tests/scalar_function_test.rs` + `function.rs` 单测）。
- 全量 `cargo test` ≥797 且 0 failed；clippy 0；fmt 0；`openspec validate` PASS。
- RTM（精简，R1-R4 行）：

| R | Scenario | Design | Task | Iter | Code Surface | Test Witness | Status |
|---|---|---|---|---|---|---|---|
| R1 | S1-S4 | D2/D4 | T1 | 000 | `function.rs::check_scalar_function` + `expression.rs::build_expression` 函数臂 | `scalar_function_test` R1 段 + function 单测 | Covered |
| R2 | S1-S6 | D3/D4/D6 | T2,T3 | 000 | `function.rs` string 段 | `scalar_function_test` R2 段 | Covered |
| R4 | S1-S3 | D3/D5 | T2,T3 | 000 | `function.rs` NULL 层 | `scalar_function_test` R4 段 | Covered |
| R3 | S1-S3 | D3/D4 | T4 | 001 | `function.rs` math 段 | `scalar_function_test` R3 段 | Covered(001) |
| R5 | S1-S6 | D7 | T5 | 001 | 既有路由（零改动验证面） | `scalar_function_test` R5 段 + `cli_test` | Covered(001) |
| R6 | S1-S2 | — | T6 | 001 | 全仓库 | 全量命令 | Covered(001) |

**Verification**

- 直接观察：e2e 断言行集/列名/错误关键词；单测断言函数级输入输出。
- 命令：`cargo test`（≥797 pass / 0 failed）、`cargo clippy -- -D warnings`（0）、`cargo fmt --check`（0 diff）、`openspec validate`（PASS）。
- 无身份型证据工程：不建 manifest/hash/run-id；验证结果写 Act Response。

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | Current-State Evidence 全节（含探针实证与补查项） |
| Design | PASS | design.md D1-D7（行为差异/接口/错误语义/选择理由闭合，无 TBD） |
| Iteration Plan | PASS | tasks.md 两 Iteration + 平衡审计（任务全覆盖、依赖有序） |
| Cycle Scope | PASS | Cycle Scope 节（initial，无 gap/repair） |
| Task Contracts | PASS | T1-T3 契约（位置/行为/见证/停止条件完整） |
| Traceability | PASS | RTM 全行 Covered（R3/R5/R6 标注 001） |
| Verification | PASS | Verification 节（目标行为验证，无身份型工程） |

**Persisted Evidence**

- Mode: none

Act Response（命令、≤20 行决定性输出、退出码、修改文件与符号）足以承载验证结果；无不可低成本复现的产物。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- substr 多字节行为以 spec 断言为准（SQLite 字节级差异不阻塞；分歧记录 Plan Review）。
- round 浮点表示误差可能使极端值（如 `round(2.675,2)`）与 SQLite 输出不一致——spec 只锁定可精确判定值，不扩散断言。
- ORDER BY 别名静默语义为既有行为（MS10-T01 遗留），本 change 锁定不修复；如需改进另立 improvement（不自动登记）。
- 非实质选择留给 Act：`FunctionExpression` 内部字段命名、单测组织、错误文案的精确措辞（在 D4 关键词契约内）。

## Act Response

- Status: reported

**Implemented**

- T1 注册机制与 planner 臂：`src/executor/function.rs`（新）承载函数元数据表（Iteration 000 仅 string 六名；math 名单归属 001）、`is_scalar_function` / `check_scalar_function`（Err 为点名文案，planner 包 `PlanError::ParseError`）、统一 `FunctionExpression { name, args }`（D1）；求值 `eval_owned` 按 D3 顺序（全部参数求值 → 错误先传播 → 任一 NULL → NULL 不做类型校验 → 分派）；`evaluate` 直连 owned 路径（CaseExpression 模板），`evaluate_ref` 物化 owned 后仅 Copy 变体回借、String 结果显式报错（D5 三先例模式），`set_parameter_value` 递归 args。planner `Expr::Function` 臂扩展：COALESCE 内部逐字节保持 → 注册名走 AST 拒绝面（OVER/DISTINCT/FILTER/NULL treatment/ORDER BY）→ Named/通配符参数点名拒绝 → arity 校验 → 构造；未注册名维持既有 `UnsupportedExpression`。`src/executor/mod.rs` re-export 三项。
- T2 string 六函数语义：`UPPER/LOWER`（ASCII 大小写）、`LENGTH`（Unicode 字符计数）、`SUBSTR`（按 char、SQLite 边缘：start=0 幻影位、负 start 尾数、负 len 前取、省略 len 到尾）、`REPLACE`（全替换、空 from 原样）、`TRIM`（仅 U+0020）；严格类型（非 String/非 Int 入参 `ValueError::TypeMismatch`，无隐式转换）。
- T3 e2e：`tests/scalar_function_test.rs`（新，18 测试）= R1/S1-S4 + R2/S1-S6 + R4/S1-S3 共 13 场景 + 未知名双位置回归锁（2）+ R1 要求文本命名参数/通配符/零参拒绝锁（3）+ TRIM 规格化变体拒绝锁（1）。RED 见证：实施前 `cargo test --test scalar_function_test` → `FAILED. 2 passed; 15 failed`（15 个目标形态全部 `Unsupported statement type` / `Unsupported expression type`）；T1 后 `14 passed; 3 failed`（未实现四函数 unreachable）；T2 单测先行 `6 failed`（四函数 unreachable）后全绿。

**Changed Files and Symbols**

- `src/executor/function.rs`（新）：`REGISTRY`、`is_scalar_function`、`check_scalar_function`、`FunctionExpression`（evaluate/evaluate_ref/set_parameter_value）、`string_arg`/`int_arg`/`substr_chars`/`eval_scalar`、12 个 `#[cfg(test)]` 单测。
- `src/executor/mod.rs`：`mod function;` + `pub use function::{check_scalar_function, is_scalar_function, FunctionExpression};`。
- `src/parser/planner/expression.rs`：`build_expression` 的 `Expr::Function` 臂扩展 + 新增 `Expr::Trim` 臂；use 列表增三项。
- `src/parser/ast.rs`：`extract_columns` / `extract_qualified_columns` 各两处放行门（注册标量函数、`Expr::Trim`）。
- `tests/scalar_function_test.rs`（新）：18 测试。
- 禁止面核对：`src/pipeline.rs`、`src/cli/`、`src/executor/projection.rs`、`src/executor/aggregate.rs`、`src/parser/planner/query.rs` 零修改；无哈希/校验和。

**Deviations from Plan**

1. **执行授权记录**：Act 启动时本 Cycle Plan Context 状态字段为 `draft`（Gate 2 Readiness 表全 PASS）。用户于 2026-09-11 明确指令「开始实施」，按 CLAUDE.md 用户豁免规则以该原话作为计划获批记录；Act 未改写 Plan Context。
2. **R1/S1 勘误（spec 文案引用错误）**：spec THEN 引用 `Unsupported expression type`「与 change 前逐字节一致」——实测 change 前 SELECT 位置的既有文案为 `Unsupported statement type`（ast.rs extract_columns 门先于 planner 函数臂，`PlanError::UnsupportedStatement`）；`Unsupported expression type` 是 WHERE 位置的既有文案（二进制探针 2026-09-11）。实现按「SHALL NOT 改变既有错误文案」以实测值逐字节锁定（双位置回归锁），spec 文字修正留 Plan Review。
3. **ast.rs 放行门（计划变更面遗漏）**：Plan Context 未记录 SELECT 项在到达 planner 函数臂之前必须先过 `extract_columns`/`extract_qualified_columns` 的函数名拒绝门（`_ => Err(UnsupportedStatement)`）——不扩则 R2 全部 SELECT 场景无法到达新臂。按 MS11-T01 Iter001 对 CASE/CAST 的同类放行先例扩展两函数（注册名与 `Expr::Trim` 放行，未知名维持既有拒绝）。计划内零行为回归。
4. **TRIM 是独立 AST 变体**：sqlparser 0.44 将 `trim(s)` 解析为 `Expr::Trim`（非 `Expr::Function`），DISTINCT/OVER 等参数级拒绝面不适用。新增 planner `Expr::Trim` 臂：仅接受纯 `trim(s)`（spec R2 唯一契约形态），`BOTH/LEADING/TRAILING` 与自定义字符变体 plan 期点名拒绝并加 e2e 锁；extract 两函数同门放行。
5. **R4/S1、R4/S3 的 abs 断言腿归属 Iteration 001**：两场景含 `abs(NULL)`/`abs(-5)`，而 math 四函数未注册是本 Iteration 稳定基线（Non-goal T4）；abs 腿随 T4 落地，本 Iteration 完成 R4 的 string 腿全部断言（upper/substr/CAST-NULL/嵌套 COALESCE）。RTM R4 行在 Iteration 001 收口。
6. **R2/S1-S2 表头文本断言归属 Iteration 001**：lib `Response::QueryResult` 不携带列名，表头文本断言按 MS11-T01 惯例由 cli_test 渲染承载（tasks T5 已规划 cli_test 追加）；本 Iteration 完成 R2/S1-S2 的值断言（含 AS 别名查询）。
7. **R2/S5 测试表形规避预存缺陷 A**：spec GIVEN 单列无 PK 表 `t(s VARCHAR)` 踩中预存缺陷 A（见 Remaining Issues）——字符串 Eq 落 `Filter(Scan)` 回退返回空结果，与函数无关。测试表加声明 PK 列（`id INT PRIMARY KEY, s STRING`）使等值走 DataScan 下推；THEN 断言与 spec 逐字一致，偏差注记于测试文件头与测试处。

**Blocker Handoff**

- None required（未发生 Gate 6 阻塞）。

**Blocker Resolution**

- None（未阻塞）。

**Self-Review**

- Plan compliance: PASS（除上述 7 项已记录偏差；RTM R1/R2/R4 行的代码面与测试见证齐备，R4 的 abs 腿与 R2 表头按偏差 5/6 归属 001）。
- Full diff reviewed: PASS（3 修改 + 2 新建文件逐 hunk 审查：COALESCE 分支行为逐字节保持、聚合五名维持既有拒绝、渲染/plan cache/pipeline 零触碰、无计划外修改、无身份型证据工程）。
- Critical findings unresolved: 无。
- Important findings unresolved: 无。
- Minor findings resolved: ① 单测初版把「类型校验错误」误作 D3「参数求值错误」断言——修正为 `CAST('abc' AS INT)` 运行期求值错误见证错误先于 NULL 传播（类型校验本就该被 NULL 短路跳过）；② `set_parameter_value` 单测初版断言 String 参数求值结果——`ParameterExpression::evaluate_ref` 对 String 返回 Null 是 M36 既有限制（predicate.rs:426 `M37 TODO`），修正为断言递归布尔契约（漏实现静默返回 false 正是 D5 失效面）；③ clippy 4 项（可省略生命周期、`matches!(.., Err(_))`→`is_err()`×3）与 `useless format!` 已修。
- Minor findings open: 无。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| 单元测试 | `cargo test --lib executor::function` | `test result: ok. 12 passed; 0 failed` | 注册表/arity/六函数语义/NULL 层/D5 递归 | PASS |
| e2e（本 change） | `cargo test --test scalar_function_test` | `test result: ok. 18 passed; 0 failed` | R1/R2/R4 场景 + 拒绝锁 + 回归锁 | PASS |
| 全量回归 | `cargo test` | `TOTAL passed=827 failed=0 ignored=2` | 全仓库（基线 797 + 新增 30，只增不减，既有测试零修改） | PASS |
| Lint | `cargo clippy --all-targets -- -D warnings` | 0 error | 全仓库 | PASS |
| 格式 | `cargo fmt --check` | 0 diff | 全仓库 | PASS |
| OpenSpec | `openspec validate --specs` + `openspec validate 2026-09-10-ms11-t03-scalar-functions` | `Totals: 21 passed, 0 failed (21 items)`；`Change ... is valid` | 全部 capability specs + 本 change delta | PASS |

**Persisted Evidence**

- None required（Plan 设定 `Mode: none`；全部验证可低成本复现，命令与决定性输出已录入本 Response）。

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| — | None | — | 无满足 Runbook/Incident 门槛的可复用操作路径或故障现场；预存缺陷 A 的完整定性见 Remaining Issues，供 Plan/improvements 登记采信 |

**Remaining Issues**

1. **预存缺陷 A（本次定性、未修复、与函数无关）**：无声明 PK 的表，其首列被登记为隐式 PK（`TableMeta.pk_column` = 首列，探针实证无 PK 表 `WHERE a = 5` 走 IndexScan）。首列为字符串且 WHERE 为该列 Eq 等值时：`extract_pk_from_where` → `Value::String.to_key() = None` → `Ok(None)` 跳过 IndexScan 分支 → `has_pk_equality` 仍为 true → 落入 `Filter(Scan)` 回退（query.rs:537-545）→ Scan 路径产出空行集 → **错误空结果**。同列 Ne/Gt 与任何非隐式 PK 列的 Eq 均正确走 DataScan 下推。影响：无 PK 表首列字符串等值过滤静默丢行。定性在 pristine master（stash 后源码级探针 `_probe_tmp`：Eq→Filter(Scan)→rows:[]，Ne/Gt→DataScan 正常）+ CLI 探针双重实证。修复面涉及 query.rs 路由与 Scan 路径，超出本 change 边界（均为 T1 禁止面），建议登记 improvement。
2. **M36 既有限制**：`ParameterExpression::evaluate_ref` 对 String 参数值返回 Null（predicate.rs:426 `M37 TODO`）——关联子查询向标量函数注入外层字符串列会得到 NULL。既有行为，本 change 未触碰。
3. **Iteration 001 待办**：T4 math 四函数（含 R4 两场景的 abs 腿）、T5 R5 调用面 e2e + cli_test 表头断言（R2/S1-S2 表头文本、函数列名渲染）、T6 全量回归。

**Commit or Diff Reference**

- 未提交（工作区：3 modified + 2 untracked 新文件 + change 目录；commit 由用户触发）。对照基线 `179228b`（master）。

## Plan Review

- Review Result: accepted

**Findings**

独立审查（revision 基线 179228b + 工作区 3 modified + 2 新增；非采信 Act Self-Review）：

- 代码审阅：`function.rs` D1-D5 契约逐项落实（注册表单点、D3 求值顺序、`evaluate_ref` 三先例模式、`set_parameter_value` 递归、math 缺位臂 `unreachable!` 大声失败）；substr SQLite 边缘算法核对正确（phantom 位/负 start/负 len/越尾）。planner 臂：COALESCE 分支逐字节保持（仅条件提为变量）、六个拒绝面点名、未注册名 `UnsupportedExpression` 不变、`Expr::Trim` 独立臂仅放行纯 `trim(s)`。`ast.rs` 两放行门仅放行注册名与 `Expr::Trim`，未知名维持既有 `_ => Err(UnsupportedStatement)`。e2e 18 测试断言与 spec 逐字一致（substr 五断言、trim TAB 保留、NULL 传播、严格类型）。
- 独立复跑验证：`cargo test` 827 passed / 0 failed / 2 ignored（基线 797 + 30，只增不减）；`cargo clippy --all-targets -- -D warnings` exit 0；`cargo fmt --check` clean；`openspec validate --specs` 21 PASS。与 Act Response 报告一致。
- 探针复核：缺陷 A 独立复现成立——无 PK 表首列字符串 Eq 返回空行集（exit 0），Ne 对照正常返回（与函数无关的预存缺陷，Act 定性采信）；表头探针 `SELECT upper(s), trim(s) AS tr` → 列名 `upper(s)`/`tr`、值正确。
- 禁止面：`git diff --stat` 仅声明 3 修改 + 2 新增；`src/pipeline.rs`、`src/cli/`、`projection.rs`、`aggregate.rs`、`query.rs` 零触碰；无哈希/指纹。

非阻塞 finding：F1 计划遗漏 `ast.rs` 放行门（Act 按 MS11-T01 先例正确补齐）；F2 R4/S1、S3 的 abs 断言腿与 R2/S1-S2 表头文本断言不在本 Iteration 交付（归属 001，Iteration Plan 不变）；F3 预存缺陷 A 定性为 improvement 候选（登记由用户决定，Plan 不自动登记）；F4 spec R1 未知名文案引用错误（Plan 侧已修正 spec：SELECT 位置实为 `Unsupported statement type`，测试文件头注记与修正后 spec 一致）。

**Deviation Classification**

- PLAN-OMISSION：偏差 3（ast.rs 放行门）、偏差 5/6（abs 腿与表头断言归属）——Plan 对调用链前置门与场景断言归属的遗漏，Act 处理正确。
- PLAN-INVALID：偏差 2（R1/S1 spec 文案引用错误）——已由 Plan Review 修正 spec 文字。
- NEW-EVIDENCE：偏差 7（预存缺陷 A，与函数无关的新定性证据）。
- ACT-DEVIATION / BASELINE-CHANGED：None。偏差 1（draft 状态下用户「开始实施」指令）为用户豁免记录，不计入偏差分类。

**Acceptance Gaps**

- Iteration 000 验收内无未满足项：R1/S1-S4、R2/S1-S6（值语义）、R4/S1-S3（string 腿）e2e 全绿 + 全量基线达成。
- R4/S1、S3 的 abs 断言腿与 R2/S1-S2 表头文本断言按偏差 5/6 归属 Iteration 001（T4/T5 既有任务承接，RTM R3/R5 行与 R4 收口均在 001）——非本 Iteration Acceptance 缺口，Iteration Plan 不变。

**Convergence**

N/A（首次 Review，无比较项）。

**Evidence**

- 复跑：`cargo test` → passed=827 failed=0 ignored=2；clippy exit=0；fmt clean；validate 21 PASS（2026-09-11 独立复跑）。
- 探针：缺陷 A Eq→`rows:[]` vs Ne→`[["xyz"]]`；表头 `{"columns":["upper(s)","tr"]}`。
- 代码：`src/executor/function.rs`（REGISTRY/FunctionExpression/eval_scalar/12 单测）、`src/parser/planner/expression.rs` 函数臂 + Trim 臂 diff、`src/parser/ast.rs` 两放行门 diff、`tests/scalar_function_test.rs` 18 测试。

**Follow-up Decision**

接受（accepted）：全部 finding 非阻塞；Act 的 7 项偏差处理均正确且在授权范围内（用户豁免记录保留于 Act Response）；Iteration 000 Acceptance 达成，测试与验证证据齐备。预存缺陷 A 建议由 docs-maintainer 收尾时登记 improvement（不阻塞、不属于本 change 修复面）。

**Iteration Plan Update**

None（Iteration Map 不变）。

**Next Cycle**

None。

**Next Iteration**

`openspec/changes/2026-09-10-ms11-t03-scalar-functions/iterations/001-math-boundaries/000-initial.md`（T4/T5/T6；Current Baseline 引用本 Iteration Act Response 与本 Review）。
