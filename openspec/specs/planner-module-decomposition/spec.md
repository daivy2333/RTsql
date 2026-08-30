# planner-module-decomposition Specification

## Purpose
TBD - created by archiving change 2026-08-30-ms07-t03-planner-decomposition. Update Purpose after archive.
## Requirements
### Requirement: 六个职责模块

`src/parser/planner.rs` SHALL 被拆分为 `src/parser/planner/` 目录下六个模块：`mod.rs`、`query.rs`、`expression.rs`、`aggregate.rs`、`subquery.rs`、`ddl_dml.rs`。每个模块 SHALL 承载一个内聚职责且可独立编译、独立单测。

#### Scenario: 核心 facade 在 mod.rs

- **GIVEN** 原 `src/parser/planner.rs`（2266 行，含 `impl PlanBuilder` 及 12 个内联测试）
- **WHEN** 拆分完成
- **THEN** `mod.rs` SHALL 包含 `PlanBuilder` struct 声明、`new`、`Default`、`register_table`、`build_plan`（dispatcher）、`validate_table`，且 `parser::planner::PlanBuilder` 公共 API 保持可访问
- **AND** `parser::planner` 对外只暴露 `PlanBuilder`，不暴露 `query`/`expression`/`aggregate`/`subquery`/`ddl_dml` 的内部符号

#### Scenario: 各子模块承载对应职责

- **GIVEN** 拆分后的 `src/parser/planner/` 目录
- **THEN** `query.rs` SHALL 包含 `build_query`/`build_from_clause_with_projection`/`build_output_columns_for_table`/`get_plan_output_columns`/`extract_column_name`(ORDER BY)/`parse_limit_value`/`parse_offset_value`/`is_simple_pk_equality`/`has_pk_equality`/`extract_pk_from_where`
- **AND** `expression.rs` SHALL 包含 `build_expression`/`build_where`/`convert_comparison_op`/`resolve_column_ref`/`expr_to_column_name`
- **AND** `aggregate.rs` SHALL 包含 `build_having`/`build_having_expression`/`is_aggregate_expr`/`extract_aggregate_func`/`extract_single_column_arg`
- **AND** `subquery.rs` SHALL 包含 `try_build_where_subquery`/`extract_subquery_table_names`/`extract_correlated_params`/`collect_outer_column_refs`/`has_outer_refs_outside`/`resolve_column_in_plan`/`get_subquery_first_column`
- **AND** `ddl_dml.rs` SHALL 包含 `build_insert`/`extract_insert_values`/`build_update`/`build_delete`/`extract_join_conditions`/`build_create_table`/`build_drop_table`/`convert_data_type`/`extract_column_constraints`/`extract_default_value`/`extract_primary_key`

### Requirement: 多 impl 块 + pub(crate) 字段

`PlanBuilder` 的三个字段（`tables`、`primary_keys`、`inner_table_names`）SHALL 改为 `pub(crate)`，使各子模块的 `impl PlanBuilder` 块能访问。方法 SHALL 以多个 `impl PlanBuilder` 块分布在各子模块，不改变任何方法签名。

#### Scenario: 子模块可访问状态字段

- **GIVEN** 拆分后的 `src/parser/planner/` 目录，`PlanBuilder` 定义在 `mod.rs`
- **WHEN** `query.rs`/`expression.rs`/`aggregate.rs`/`subquery.rs`/`ddl_dml.rs` 中的 `impl PlanBuilder` 方法读写 `self.tables` / `self.primary_keys` / `self.inner_table_names`
- **THEN** 编译通过（三字段可见性为 `pub(crate)`）
- **AND** 这些字段 SHALL NOT 在 crate 外部可见（`PlanBuilder` 是 `pub` struct，但字段 `pub(crate)` 不对外导出）

#### Scenario: 方法签名不变

- **GIVEN** 原 `src/parser/planner.rs` 的所有 `plan` 相关方法（`new`/`register_table`/`build_plan`/`build_query`/`build_expression`/`build_where`/`build_having`/子查询相关/DDL/DML）
- **WHEN** 拆分到各子模块的 `impl PlanBuilder` 块
- **THEN** 每个方法的参数、返回类型、async 标记、可见性 SHALL 与拆分前逐字相同

### Requirement: 公共 API 零变化

`PlanBuilder` 的对外 API 与 `parser`/`lib` 的 re-export SHALL 在拆分前后完全一致，使 `src/pipeline.rs`、`tests/planner_test.rs`、`tests/executor_test.rs` 无需修改或仅需路径不变地继续编译通过。

#### Scenario: parser::PlanBuilder re-export 不变

- **GIVEN** `src/parser/mod.rs:12` `pub use planner::PlanBuilder;` 和 `src/lib.rs:16` `pub use parser::{parse_sql, PlanBuilder, PlanError};`
- **WHEN** DDL、DML、SELECT 的公共调用路径（`src/pipeline.rs:63,64,72,78`）
- **THEN** `rtsql::parser::PlanBuilder` 与 `rtsql::PlanBuilder` SHALL 继续以相同签名导出（`new`/`register_table`/`build_plan`/`Default`）

#### Scenario: 行为零变化

- **GIVEN** 同一组 SQL 语句
- **WHEN** 拆分前后各执行一次 `PlanBuilder::build_plan`
- **THEN** 两次生成的 `PhysicalPlan` SHALL 结构等价（同样节点顺序、字段值、别名、谓词语义）

### Requirement: 测试随函数迁移且子模块可独立单测

原 `src/parser/planner.rs` 中 12 个 `#[cfg(test)]` 内联单元测试 SHALL 随被测函数迁移到对应子模块的 `#[cfg(test)]`，断言逻辑不变。每个子模块 SHALL 能被 `cargo test --lib` 独立执行。

#### Scenario: 测试迁移到对应模块

- **GIVEN** 原内联测试 `test_plan_builder_new`/`test_register_table`/`test_validate_table`/`test_build_query_scan`/`test_build_query_index_scan`/`test_build_insert`/`test_build_update`/`test_build_delete`/`test_extract_pk_from_where_reversed`/`test_nonexistent_table`/`test_unsupported_where`/`test_insert_multiple_rows`
- **WHEN** 拆分完成
- **THEN** 每个测试 SHALL 放置在与其被测函数相同模块的 `#[cfg(test)]` 内
- **AND** 全部测试断言 SHALL 与拆分前相同，`cargo test --lib` SHALL 仍为 12 个单测全绿

#### Scenario: 模块独立编译单元

- **GIVEN** `src/parser/planner/query.rs` 等子模块
- **WHEN** 运行 `cargo check` / `cargo test --lib`
- **THEN** 每个子模块 SHALL 是独立的编译单元
- **AND** 各子模块的 `#[cfg(test)]` SHALL 可通过 `cargo test --lib` 单独运行对应测试函数

### Requirement: 编译与回归零告警

拆分后 `cargo build`、`cargo clippy -D warnings`、`cargo fmt --check` 和 `cargo test --all` SHALL 全部归零报警/全绿，不产生因字段 `pub(crate)` 导致的 `dead_code`/`unused` 警告。

#### Scenario: 全量回归通过

- **GIVEN** 拆分完成后的工作区
- **WHEN** 运行 `cargo build`、`cargo clippy -D warnings`、`cargo fmt --check`、`cargo test --all`
- **THEN** `cargo build` SHALL 0 warning；`cargo clippy -D warnings` SHALL 0 warning；`cargo fmt --check` SHALL 0 diff；`cargo test --all` SHALL 0 failures（542 tests，含迁移后的 12 个 planner 单测）
- **AND** 新增/修改文件 SHALL NOT 引入 `#[allow(dead_code)]` 等压制

#### Scenario: 无障碍路径改动

- **GIVEN** `src/pipeline.rs`、`tests/planner_test.rs`、`tests/executor_test.rs`、`src/lib.rs`、`src/parser/mod.rs`
- **WHEN** 拆分后完整编译
- **THEN** 这些文件 SHALL 无需任何逻辑修改即可编译通过（仅路径/`mod` 声明按需微调，`pub use` 暴露面保持）
