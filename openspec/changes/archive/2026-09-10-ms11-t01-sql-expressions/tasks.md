# tasks — MS11-T01 SQL 表达式四件套与值表达式

> Change: `2026-09-10-ms11-t01-sql-expressions`
> 需求来源: tasks.md MS11-T01 + I040 并入（2026-09-10 决策默认，proposal Scope Decisions 供审计）

## Task List

| Task | 目标 | 关键文件 | 关联 |
|---|---|---|---|
| T1 | 三值求值内核：`Ternary` + `Predicate::evaluate_ternary`（默认实现映射既有 `evaluate`）+ `ComparisonPredicate`/`LogicalPredicate` 三值重写（`evaluate()` 走 fold，行为逐字节等价） | `src/executor/predicate.rs` | R2 |
| T2 | 新谓词：`LikePredicate`（%/_ 通配、String-only、NULL→Unknown）、`IsNullPredicate`、`NotPredicate`（三值取反）；否定统一 Not 包装 | `src/executor/predicate.rs` | R1/R2 |
| T3 | planner WHERE 转换：IN→OR 链、BETWEEN→AND 对、NOT IN/BETWEEN→Not 包装（脱糖）、Like/IsNull/Not 臂、`contains_or` 遍历扩展（Case/Cast/InList/Between/Like/IsNull/Function 参数）、显式拒绝（ESCAPE/ILIKE/RLIKE/SIMILAR TO） | `src/parser/planner/expression.rs`、`query.rs` | R1/R2 |
| T4 | 值表达式：`CaseExpression`（searched + simple 脱糖为比较条件）、`CoalesceExpression`（`Expr::Function` 名匹配）、`CastExpression`（严格四族映射 + 转换矩阵）；`build_expression` 新臂 | `src/executor/predicate.rs`（或相邻新文件）、`src/parser/planner/expression.rs` | R3 |
| T5 | I040：`extract_insert_values` 接受 `UnaryOp::Minus` 数字字面量折叠 | `src/parser/planner/ddl_dml.rs` | R5 |
| T6 | Iteration 000 测试见证：predicate 单测（Ternary/新谓词/三值表）、plan 形态断言、`tests/expression_e2e_test.rs` WHERE 矩阵、pushdown 追加等价用例、全量回归 | `tests/` | R1/R2/R3/R5/R6 |
| T7 | ProjectionNode 机制：新 plan 变体 + `ProjectionExecutor`（逐项表达式求值）+ `create_executor_from_plan`/`get_plan_output_columns`/`inject_correlated_values` 接线 | `src/executor/plan.rs`、新 `src/executor/projection.rs`、`src/pipeline.rs`、`src/executor/correlated.rs`、`planner/query.rs:23-83` | R4 |
| T8 | planner SELECT 路由：表达式项检测 → 顶层 ProjectionNode 包装（纯列查询 plan 不变）、别名/Display 列名、子查询/通配混用拒绝、聚合报错保持、`ast.rs::extract_columns(_qualified_columns)` 遍历扩展 | `src/parser/planner/query.rs`、`src/parser/ast.rs` | R4 |
| T9 | Iteration 001 测试见证：`tests/projection_expression_test.rs`（派生列/命名/混合/怪癖修正/限制）+ cli_test 渲染追加 + 全量回归 | `tests/` | R4/R6 |

## Iteration Plan

### Iteration 000: WHERE 侧表达式能力全绿（四件套 + 三值 + 值表达式操作数 + I040）

- Tasks: T1, T2, T3, T4, T5, T6
- Depends on: None
- Stable baseline: 谓词求值器三值内核与新谓词可用；WHERE 支持 spec R1/R2/R3/R5 全部形态并可下推；既有 704 测试零修改通过
- Verification boundary: `tests/expression_e2e_test.rs` WHERE 矩阵 + predicate 单测 + pushdown 追加用例全绿；全量 `cargo test` / clippy / fmt / `openspec validate --all` 通过
- Diagnostic boundary: `src/executor/predicate.rs`（求值器）、`src/parser/planner/{expression,query}.rs`（转换/路由）、`ddl_dml.rs`（I040）
- Non-goals: SELECT 投影表达式（T7-T9）；HAVING 新表达式；算术运算

### Iteration 001: SELECT 派生列（投影表达式机制）

- Tasks: T7, T8, T9
- Depends on: Iteration 000（值表达式实现是其求值内容）
- Stable baseline: 非聚合 SELECT 列表支持值表达式项 + AS 别名 + Display 列名；纯列查询 plan 逐字节不变；CLI 四格式渲染兼容
- Verification boundary: `tests/projection_expression_test.rs` + cli_test 追加全绿；全量回归零既有修改
- Diagnostic boundary: `src/executor/{plan,projection}.rs`、`src/pipeline.rs`、`planner/query.rs` SELECT 路由、`ast.rs` 列提取
- Non-goals: 标量子查询与表达式项混用；聚合查询表达式项；算术运算；ORDER BY 引用派生列

### 平衡审计

- Iteration 000 六任务共同形成"WHERE 表达式能力"单一可验收结果（拆开则 T3 无法独立验证）；T1/T2 是 T3 的地基、T4 复用 T1 内核，聚合成立。
- Iteration 001 三任务形成"SELECT 投影"独立结果，故障域（新执行器节点 + planner 路由）与 Iter 000（求值器语义）分离；不含无关验证。
- 两 Iteration 均非过碎（单项测试无法构成稳定基线）亦非过重（各自验收边界与诊断边界明确）。

## Requirements Traceability Matrix

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 四件套 | S1-S7 | D1/D2/D6 | T2,T3,T6 | 000 | `predicate.rs`、`planner/expression.rs::build_where`、`query.rs::contains_or` | predicate 单测 / expression_e2e / pushdown 追加 | None | Covered |
| R2 三值语义 | S1-S4 | D1/D2 | T1,T2,T3,T6 | 000 | `predicate.rs::evaluate_ternary` | predicate 单测 / expression_e2e NULL 矩阵 | None | Covered |
| R3 CASE/COALESCE/CAST | S1-S5 | D3 | T4,T6 | 000 | `predicate.rs`（或相邻）新 Expression 实现、`build_expression` | expression_e2e（WHERE 操作数等价形式；S1-S4 的 SELECT 投影形态断言归 Iteration 001 T9——Review 裁定口径） | None | Covered |
| R4 SELECT 派生列 | S1-S4 | D4 | T7,T8,T9 | 001 | `plan.rs::ProjectionNode`、`projection.rs`、`pipeline.rs`、`query.rs` SELECT 路由、`ast.rs` | projection_expression / cli_test 追加 | None | Covered |
| R5 INSERT 负数字面量 | S1-S2 | D5 | T5,T6 | 000 | `ddl_dml.rs::extract_insert_values` | planner 断言 + expression_e2e 往返 | None | Covered |
| R6 零回归 | S1-S2 | D0/D7 | T6,T9 | 000+001 | 全仓 | 全量 `cargo test` + clippy/fmt/validate | None | Covered |
