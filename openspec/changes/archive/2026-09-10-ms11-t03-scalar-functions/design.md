# Design — MS11-T03 标量函数库第一批

## 当前行为与目标行为

当前：`build_expression` 的 `Expr::Function` 臂只放行 COALESCE（`src/parser/planner/expression.rs:272-294`），其余函数名报 `PlanError::UnsupportedExpression`。SELECT 表达式项路由、WHERE 谓词构建、投影执行、表头派生均已就绪（MS11-T01），函数只缺分派与语义。

目标：`upper/lower/length/substr/replace/trim/abs/round/floor/ceil` 十个标量函数在 WHERE 与 SELECT 双侧可用；OVER/DISTINCT/FILTER/命名参数/通配符参数/arity 错误 plan 期点名拒绝；NULL 传播、严格类型、SQLite 边缘语义按 spec；未知名维持既有文案。

## 关键选择

### D1 统一 `FunctionExpression` + 静态分派（不建 10 个具体 struct）

新类型 `FunctionExpression { name: String, args: Vec<ExpressionRef> }` 实现 `Expression`；求值时按 `name` 分派到注册表。备选的每函数一个 struct（CASE/CAST 式）被否：10 个函数 10 个类型使 planner/executor/mod.rs re-export 面膨胀，且注册表仍需存在。单类型 + 分派使加函数只改一个模块，`Debug`/`set_parameter_value`/`evaluate_ref` 样板只写一次。

### D2 注册表单点：`src/executor/function.rs`

新模块承载三件事：(1) 函数元数据（名称、arity 范围），(2) planner 校验入口（`pub fn resolve_scalar_function(name: &str, argc: usize) -> Result<(), PlanError>` 语义——校验失败给点名文案），(3) 求值分派（`match name` → 纯同步实现）。planner `Expr::Function` 臂改为：COALESCE 保持既有构造（不动既有路径）→ 其余名称先查 `is_aggregate` 同名冲突不适用（聚合在 SELECT 路由更早分流，但 `build_expression` 也被 HAVING 外的谓词/值位置调用——聚合名直接走既有 `UnsupportedExpression`，与现状一致）→ 注册表校验 → 构造 `FunctionExpression`。名单只有一处，planner 与 executor 不漂移。

### D3 求值顺序与 NULL 语义

按序求值全部参数 → 参数求值错误先传播（不被 NULL 吞没）→ 任一参数 NULL → 结果 NULL（此时不做类型校验）→ 否则类型校验 → 计算。纯函数无副作用，先求值全部再查 NULL 与短路观察等价，且保证 `upper(CAST(NULL AS INT))` 输出 NULL 而非类型错误。

### D4 错误面映射（文案契约）

- plan 期 arity 不符 / AST 拒绝面（OVER、DISTINCT、FILTER、命名参数、通配符、零参）→ `PlanError::ParseError`，文案点名函数名与构造（COALESCE "requires at least one argument" 同风格）。
- 未注册名 → `PlanError::UnsupportedExpression`（"Unsupported expression type"，既有文案逐字节不变）。
- 执行期类型不符 → `ValueError::TypeMismatch`（"Type mismatch"），经执行器包装（如 ProjectionExecutor 的 `Expression evaluation error: ...`）。
- 错误文案是 Act 契约，测试断言点名关键词，不得改写。

### D5 `evaluate_ref` 与参数注入照抄三先例

`evaluate_ref` 物化 owned 行求值 → Copy 变体（Int/Float/Bool/Null）回借、String 结果显式报错（CASE/COALESCE/CAST 同模式，`src/executor/predicate.rs:478-495` 模板）。`set_parameter_value` 递归 `args`——关联子查询参数注入经 `correlated.rs:61-65` 对投影项调用此方法，漏实现会静默返回错误值。

### D6 表头命名零改动

`ProjectionItem.name = expr.to_string()`（`src/parser/planner/query.rs:427-440`）按书写形态回放（探针实证：`COALESCE(name, 'x')` 输出原样）。AS 别名走既有 `ExprWithAlias` 臂。渲染层零改动。

### D7 WHERE 路由零改动

`contains_or` 已递归扫描函数参数（`query.rs:1009-1013`）→ OR 组合正确保留 Filter。PK 提取只认 `Expr::Value` 右侧（`extract_pk_from_where`）→ `WHERE id = abs(5)` 落普通扫描。无 OR 非 PK 函数谓词进 DataScan 行内下推，行级求值时为全形状（投影裁剪在谓词后，MS10-T01 语义）。三条路径均无新接线。

## 变更面与责任边界

| 文件 | 责任 | 变化 |
|---|---|---|
| `src/executor/function.rs`（新） | 注册表、分派、十函数实现、单测 | 新建 |
| `src/executor/mod.rs` | re-export | +2 行（模块声明 + `FunctionExpression` 等） |
| `src/parser/planner/expression.rs` | `Expr::Function` 臂 | COALESCE 保持；其余改为查注册表 → 构造 `FunctionExpression` 或点名拒绝 |
| `tests/scalar_function_test.rs`（新） | R1-R5 e2e | 新建 |
| `tests/cli_test.rs` | 函数表头 CLI 用例 | 追加 |
| `src/pipeline.rs`、`src/cli/`、`src/executor/projection.rs`、`src/executor/aggregate.rs` | — | 禁止修改 |

## 实现顺序

T1 机制与 planner 臂（先 upper/lower 打通最小可观察）→ T2 string 六函数语义补全 → T3 R1/R2/R4 e2e →（Iteration 001）T4 math 四函数 → T5 R5 调用面 e2e + CLI 表头 → T6 全量回归。依赖原因：T2-T5 全部依赖 T1 的注册表与 `FunctionExpression`；T5 的调用面用例需要函数可用；T6 收尾必须在全部行为面落地后。

## 风险

- `substr` 负 start/负 len 的 SQLite 精确语义以 spec 场景为准（`substr('abc',0,2)='a'`、`substr('abc',2,-1)='a'`、`substr('abc',-2)='bc'`）；实现按字符（char）计数。多字节边界若与 SQLite 字节行为冲突，以 spec 场景断言为准（记录于 Plan Review，不阻塞）。
- `round` 用 `f64::round`（半数远离零）+ `10^digits` 乘除；浮点表示误差导致 `round(2.675, 2)` 类边界与 SQLite 输出差异的可能存在——spec 场景只锁定 `3.14159→3.14`、`2.5→3.0` 等可精确判定值。
- ORDER BY 表达式别名静默语义（sort.rs:82-95 未命中即 Equal）为既有行为，本 change 只锁定不修复；若 Review 判定需改进，另立 improvement。
