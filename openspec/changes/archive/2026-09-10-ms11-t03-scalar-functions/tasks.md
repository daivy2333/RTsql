# tasks — MS11-T03 标量函数库第一批

> 规划：openspec-plan 2026-09-10；Gate 1 已批准（需求 + 场景 + 范围，用户裁定 4 项语义 + 2 项默认假设）。
> 设计：design.md D1-D7（Gate 2 审查后 ready）。
> 实施：Iteration 000（T1-T3）已完成（2026-09-11，Act Response 见 `iterations/000-mechanism-strings/000-initial.md`；827 tests pass / 0 failed / 2 ignored，Plan Review accepted——7 项偏差含 spec R1/S1 文案勘误与 ast.rs 放行门补齐）。
> 实施：Iteration 001（T4-T6）已完成（2026-09-11，Act Response 见 `iterations/001-math-boundaries/000-initial.md`；845 tests pass / 0 failed / 2 ignored，Plan Review accepted——CEIL/FLOOR 独立 sqlparser 变体接线按 TRIM 先例补救）。

## Task List

| Task | 状态 | 目标 | 关键产出 | Iteration |
|---|---|---|---|---|
| T1 | **completed**（2026-09-11） | 函数注册机制 + planner 臂扩展 + `FunctionExpression` | `src/executor/function.rs`（元数据 + 校验入口 + 分派骨架 + upper/lower 实现 + 单测）；`src/parser/planner/expression.rs` `Expr::Function` 臂扩展（COALESCE 保持、注册表查找、AST 拒绝面、arity 校验）；`src/executor/mod.rs` re-export | 000 |
| T2 | **completed**（2026-09-11） | string 六函数语义补全 | `length/substr/replace/trim` 实现与单测（substr SQLite 边缘、trim 仅空格、严格类型、NULL 传播） | 000 |
| T3 | **completed**（2026-09-11） | R1/R2/R4 e2e 集成测试 | `tests/scalar_function_test.rs`：未知名文案、OVER/DISTINCT/arity 拒绝、upper/lower 表头与别名、length+WHERE、substr 边缘、replace/trim、严格类型错误、NULL 传播与嵌套 | 000 |
| T4 | **completed**（2026-09-11） | math 四函数 | `abs/round/floor/ceil` 实现与单测（同型 abs、half-away-from-zero、负 digits 整数位、Float 返回）；CEIL/FLOOR 经 `Expr::Ceil/Floor` 独立变体臂接线（偏差 2） | 001 |
| T5 | **completed**（2026-09-11） | R5 调用面 e2e + CLI 表头 | `tests/scalar_function_test.rs` 追加：WHERE 下推/OR 双路径、聚合混用拒绝、HAVING 拒绝、ORDER BY 别名静默锁定、PK+函数比较；`tests/cli_test.rs` 追加 2 函数表头用例 | 001 |
| T6 | **completed**（2026-09-11） | 全量回归清扫 | `cargo test` 845 passed / 0 failed / 2 ignored（R6）、clippy/fmt 0、`openspec validate` PASS | 001 |

## Iteration Plan

### Iteration 000: 注册机制与 string 函数（双侧可用）

- Tasks: T1, T2, T3
- Depends on: None
- Stable baseline: 注册机制可扩展（加函数只改 `src/executor/function.rs`）；string 六函数在 WHERE 与 SELECT 双侧全语义可用；R1/R2/R4 验收关闭；math 函数仍拒绝（未注册）
- Verification boundary: T3 e2e 全绿 + function.rs 单测绿；全量回归 0 failed；clippy/fmt 0、validate PASS
- Diagnostic boundary: `src/executor/function.rs`、`src/parser/planner/expression.rs`、`src/executor/mod.rs`、`tests/scalar_function_test.rs`
- Non-goals: math 四函数（T4）、R5 调用面场景（T5）、CLI 用例（T5）

### Iteration 001: math 函数与调用面收尾

- Tasks: T4, T5, T6
- Depends on: Iteration 000（注册机制与 `FunctionExpression`）
- Stable baseline: MS11-T03 全部 Acceptance 关闭（R1-R6）
- Verification boundary: T5 e2e 全绿 + T6 全量基线（测试总数只增不减）+ clippy/fmt 0 + validate PASS
- Diagnostic boundary: `src/executor/function.rs`（math 段）+ `tests/scalar_function_test.rs`、`tests/cli_test.rs`
- Non-goals: 窗口函数、UDF、日期/时间、聚合扩展、ORDER BY 别名排序能力

### 平衡审计

- Iteration 000 收敛于"string 函数全语义双侧可用"内聚结果：机制（T1）+ 语义（T2）+ 验收（T3）同一故障域（求值内核与 planner 接线），可独立验证与排障。
- Iteration 001 收敛于 math 语义 + 调用面边界 + 全量验收；调用面用例（聚合混用/HAVING/ORDER BY/PK 路径）需要全部函数注册后才可书写，依赖顺序成立。
- 单 Iteration 承载全部任务会混合求值内核与集成边界两个验证面，且 math 场景依赖 string 场景建立的模式——按两轮拆分；不按行数/task 数切分。
