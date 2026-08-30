# 任务清单：ms07-t03-planner-decomposition

> 关联里程碑：**MS07-T03**（基础能力建设 / planner 模块化）
> 关联 design：`design.md`
> 关联 proposal：`proposal.md`
> 关联 spec：`specs/planner-module-decomposition/spec.md`
> 关联 Iteration：仅一个 Iteration 000（含 4 个 task）

## Iteration Plan

### Iteration 000: planner 模块化拆分（纯搬移重构）

- Tasks: T1, T2, T3, T4
- Depends on: None（MS07-T01/T02 已合并；工作区干净，HEAD = `50ef820`）
- Stable baseline: `src/parser/planner.rs` 拆为 `src/parser/planner/` 目录 6 模块；`PlanBuilder` 公共 API 与 SQL 计划输出零变化；12 个单测随函数迁移且全绿
- Verification boundary: `cargo build` 0 warning；`cargo clippy -D warnings` 0 warning；`cargo fmt --check` 0 diff；`cargo test --all` 0 failures（542 tests，含迁移后 12 个 planner 单测）；`tests/planner_test.rs` 与 `tests/executor_test.rs` 不修改逻辑仍全绿
- Diagnostic boundary: `src/parser/planner/` 目录 + `src/parser/mod.rs`
- Deferred tasks: None（本 change 完成 MS07-T03 全部子项；MS07-T04/T05/T06/T07 留独立 change）

## Task 1: 建立 planner 目录骨架并迁移结构

- [x] 1.1 `git mv src/parser/planner.rs src/parser/planner/mod.rs`
- [x] 1.2 新建 `src/parser/planner/{query,expression,aggregate,subquery,ddl_dml}.rs`
- [x] 1.3 `mod.rs` 保留 `PlanBuilder` struct（含 `pub(crate) tables`/`pub(crate) primary_keys`/`pub(crate) inner_table_names`）+ `new`/`Default`/`register_table`/`build_plan`/`validate_table`；声明 `mod query; mod expression; mod aggregate; mod subquery; mod ddl_dml;`（private）
- [x] 1.4 校验 `src/parser/mod.rs:12` `pub use planner::PlanBuilder;` 与 `src/lib.rs` re-export 路径引用仍有效
- [x] 1.5 `cargo check` 通过（预期当前会报缺符号——下游任务补齐）

## Task 2: 迁移 query + expression + aggregate 方法到子模块

- [x] 2.1 `query.rs`：`build_query`/`build_from_clause_with_projection`/`build_output_columns_for_table`/`get_plan_output_columns`/`extract_column_name`(ORDER BY 用)/`parse_limit_value`/`parse_offset_value`/`is_simple_pk_equality`/`has_pk_equality`/`extract_pk_from_where` 以 `impl PlanBuilder` 块迁入
- [x] 2.2 `expression.rs`：`build_expression`/`build_where`/`convert_comparison_op`/`resolve_column_ref` + 自由函数 `expr_to_column_name`
- [x] 2.3 `aggregate.rs`：`build_having`/`build_having_expression` + 自由函数 `is_aggregate_expr`/`extract_aggregate_func`/`extract_single_column_arg`
- [x] 2.4 每个子模块 `use crate::executor::{按需符号}`、`use crate::parser::error::PlanError`、`use crate::parser::value::value_from_sqlparser`、`use sqlparser::ast::...`；`ast::*` 符号调用处显式引用
- [x] 2.5 函数正文逐字保留，不做任何逻辑或命名改动
- [x] 2.6 `cargo check` 通过（缺符号由下游任务补）

## Task 3: 迁移 subquery + ddl_dml 方法并迁移测试

- [x] 3.1 `subquery.rs`：`try_build_where_subquery`/`extract_subquery_table_names`/`extract_correlated_params`/`collect_outer_column_refs`/`has_outer_refs_outside`/`resolve_column_in_plan`/`get_subquery_first_column`
- [x] 3.2 `ddl_dml.rs`：DML（`build_insert`/`extract_insert_values`/`build_update`/`build_delete`/`extract_join_conditions`）+ DDL（`build_create_table`/`build_drop_table`/`convert_data_type`/`extract_column_constraints`/`extract_default_value`/`extract_primary_key`）
- [x] 3.3 迁移 12 个内联测试：core(test_plan_builder_new/test_register_table/test_validate_table)→mod.rs；query(test_build_query_scan/test_build_query_index_scan/test_extract_pk_from_where_reversed/test_nonexistent_table/test_unsupported_where)→query.rs；ddl_dml(test_build_insert/test_build_update/test_build_delete/test_insert_multiple_rows)→ddl_dml.rs。断言逐字保留
- [x] 3.4 全量 `cargo build` 0 warning；`cargo test --lib` 含 12 个迁移单测全绿
- [x] 3.5 `cargo clippy -D warnings` 0 warning；`cargo fmt --check` 0 diff

## Task 4: 全量回归与验证

- [x] 4.1 `cargo test --all` 0 failures：基线 542 tests（含迁移后 12 个 planner 单测）
- [x] 4.2 `cargo clippy -D warnings` 0 warning；`cargo build` 0 warning
- [x] 4.3 `cargo fmt --check` 0 diff
- [x] 4.4 确认 `tests/planner_test.rs`（624 行）与 `tests/executor_test.rs` 4 处 `PlanBuilder` 用例未做任何逻辑修改仍全绿（仅公共 API）
- [x] 4.5 确认 `PlanBuilder` 公共 API（`new`/`register_table`/`build_plan`/`Default`）与 `rtsql::parser::PlanBuilder`、`rtsql::PlanBuilder` re-export 路径未变

## 验收

| Acceptance | 验证 |
|---|---|
| R1 六职责模块 | T2/T3 目录结构 + `mod.rs` 只暴露 `PlanBuilder`；`parser::planner` 无子模块符号泄漏 |
| R2 多 impl 块 + pub(crate) 字段 | T1.3 三字段 `pub(crate)`；`cargo build` 通过 |
| R3 公共 API 零变化 | T4.4/T4.5 `tests/planner_test.rs` + `executor_test.rs` 不动仍全绿；re-export 路径不变 |
| R4 测试随迁移且独立单测 | T3.3 12 单测迁移；`cargo test --lib` 全绿 |
| R5 编译与回归零告警 | T4.1-4.3 全量回归 + clippy/fmt 归零 |

## Requirements Traceability Matrix

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 六职责模块 | S1.1/S1.2 | 目录结构+模块归属 | T2,T3 | 000 | `src/parser/planner/{mod,query,expression,aggregate,subquery,ddl_dml}.rs` | `cargo check` + 模块成员断言 | None | Covered |
| R2 多 impl + pub(crate) 字段 | S2.1/S2.2 | 字段可见性 | T1.3,T2,T3 | 000 | `src/parser/planner/mod.rs::{tables,primary_keys,inner_table_names}` | `cargo build` | None | Covered |
| R3 公共 API 零变化 | S3.1/S3.2 | re-export + 行为不变 | T1.4,T4.4,T4.5 | 000 | `src/parser/mod.rs:12`, `src/lib.rs:16`, `src/pipeline.rs` | `tests/planner_test.rs`, `tests/executor_test.rs` | None | Covered |
| R4 测试随迁移独立单测 | S4.1/S4.2 | 测试迁移 | T3.3 | 000 | 各子模块 `#[cfg(test)] mod tests` | `cargo test --lib` | None | Covered |
| R5 编译与回归零告警 | S5.1/S5.2 | 全量回归 | T4 | 000 | 全模块 | `cargo test --all`/clippy/fmt | None | Covered |