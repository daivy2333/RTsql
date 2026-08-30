## Why

`src/parser/planner.rs` 是 2266 行的单文件。它承载 PlanBuilder 的全部职责：SELECT/scan/join/projection、表达式与 WHERE 谓词、聚合与 HAVING、子查询与相关列、DDL、DML，以及索引/PK 扫描优化。`tasks.md:129`（MS07-T03）将该任务定位为"基础能力建设 / planner.rs 2266 → 按 build_* 拆分到 4-6 个模块"。

问题：

- **可测试性受阻**：12 个内联单元测试集中在文件尾部（`planner.rs:2064-2266`），`super::*` 依赖整个 impl 块；无法单独跑某个子系统的测试，也无法对单个模块做聚焦回归。
- **职责无边界**：聚合、子查询、DDL、DML 的 build_* 和辅助函数混在同一个 2280 行文件里，互相以私有方法调用，缺乏模块级隔离。
- **未来扩展风险**：MS07-T03 之后是 MS07-T06 谓词下推、MS08 性能项；单文件继续膨胀会放大回归面和排障难度。

## What Changes

按用户决策（**6 模块 + A(pub(crate) 字段) + A(测试随函数迁移)**），把 `src/parser/planner.rs` 拆成 `src/parser/planner/` 目录：

- `src/parser/planner.rs` → `src/parser/planner/mod.rs`：`PlanBuilder` struct + `new`/`Default`/`register_table`/`build_plan`(dispatcher)/`validate_table` + 对外 re-export 不变
- `src/parser/planner/query.rs`：SELECT 相关 — `build_query`/`build_from_clause_with_projection`/`build_output_columns_for_table`/`get_plan_output_columns`/`extract_column_name`(ORDER BY)/`parse_limit_value`/`parse_offset_value`/`is_simple_pk_equality`/`has_pk_equality`/`extract_pk_from_where`
- `src/parser/planner/expression.rs`：`build_expression`/`build_where`/`convert_comparison_op`/`resolve_column_ref`/`expr_to_column_name`
- `src/parser/planner/aggregate.rs`：`build_having`/`build_having_expression`/`is_aggregate_expr`/`extract_aggregate_func`/`extract_single_column_arg`/`extract_column_name`(聚合用)
- `src/parser/planner/subquery.rs`：`try_build_where_subquery`/`extract_subquery_table_names`/`extract_correlated_params`/`collect_outer_column_refs`/`has_outer_refs_outside`/`resolve_column_in_plan`/`get_subquery_first_column`
- `src/parser/planner/ddl_dml.rs`：`build_insert`/`extract_insert_values`/`build_update`/`build_delete`/`extract_join_conditions`/`build_create_table`/`build_drop_table`/`convert_data_type`/`extract_column_constraints`/`extract_default_value`/`extract_primary_key`

所有私有方法仍以 `impl PlanBuilder` 块分布在各子模块；三个字段改 `pub(crate)`：
- `tables: HashMap<String, Vec<String>>`（原 private → pub(crate)）
- `primary_keys: HashMap<String, String>`（原 private → pub(crate)）
- `inner_table_names: Option<Vec<String>>`（已是 pub(crate)，不变）

公共 API 与行为**零变化**：`PlanBuilder::new`/`register_table`/`build_plan`/`Default` 签名不变；`rtsql::parser::PlanBuilder` 与 `rtsql::PlanBuilder` re-export 不变。这是纯重构。

## Capabilities

### New Capabilities

- `planner-module-decomposition`：`src/parser/planner.rs` 按职责拆分为 `src/parser/planner/` 目录下的 `mod.rs` + `query.rs` + `expression.rs` + `aggregate.rs` + `subquery.rs` + `ddl_dml.rs` 六个模块；`PlanBuilder` 公共 API 与 SQL 计划语义完全不变；每个子模块可独立单测。
  - 改前：单文件 2266 行，私有字段 + 所有方法 + 12 内联测试混在一起，子模块不可独立单测。
  - 改后：6 个职责清晰的模块；`impl PlanBuilder` 分布在各子模块；三字段 `pub(crate)` 支持跨模块实现；内联测试随函数迁移到各子模块 `#[cfg(test)]`。
  - 关联 M/K：`M04`（SQL 解析 + PhysicalPlan）、`M07`（子查询）、`M08`（聚合）、`M09`（JOIN）。

### Out of Scope（本 change 不做）

- **MS07-T04 显式事务 API** `Database::begin/commit/rollback`：独立 change
- **MS07-T05 Checkpoint**：独立 change
- **MS07-T06 谓词/LIMIT 下推**：独立 change（拆分后为它在 `query.rs`/`expression.rs` 提供落点）
- **MS07-T07 消息传递重构**：独立 change
- **任何 SQL 逻辑重建或重构逻辑**：本 change 严格"搬移不改写"；每个函数正文逐字保留（删 `pub(crate)`/范围可见性调整之外不改动）
- **`ast.rs`/`error.rs`/`value.rs`**：不动
- **代码清理、注释润色、Doc 完善**：除非属于搬移必需（如 `mod` 声明），否则不做

## Impact

- **影响模块**：
  - `src/parser/planner.rs`（重命名）；`src/parser/mod.rs`（`pub mod planner;` 改为 `pub mod planner;` 指向新目录，不变或微调）；新增 `src/parser/planner/{mod,query,expression,aggregate,subquery,ddl_dml}.rs`
- **影响接口**：
  - `PlanBuilder` 三字段 private → `pub(crate)`（不对外泄漏；lib crate 内部可见）
  - 公共 API（`new`/`register_table`/`build_plan`/`Default`）与 re-export（`parser::PlanBuilder`、`lib::PlanBuilder`）不变
  - 无新增/删除公共符号
- **影响行为**：无——纯搬移重构，SQL 计划输出逐字节不变
- **兼容性**：
  - `src/pipeline.rs`（`PlanBuilder::new().build_plan` / `register_table` 两处）不变
  - `tests/planner_test.rs`（624 行，仅公共 API）不变且必须全绿
  - `tests/executor_test.rs` 4 个 `PlanBuilder` 用例（仅公共 API）不变且必须全绿
  - 12 个内联单元测试随函数迁移，逻辑不变、断言不变
- **风险**：
  - **低**：字段 `pub(crate)` 后编译器可能对"声明 `pub(crate)` 但跨模块未使用"的字段（如 `inner_table_names`）报 `dead_code` 或 `unused` 警告——需 `cargo build` / `cargo clippy -D warnings` 验证归零
  - **低**：模块间同 crate 方法调用（`build_query` 调 `build_expression` 等）——Rust 方法解析不依赖模块，只要字段可见即可编译
  - **低**：`mod.rs` 需 `pub use` 或 `#[doc(hidden)]` 处理子模块可见性；确保 `parser::planner` 只暴露 `PlanBuilder`
  - **低**：`use crate::parser::planner::...` 内部引用路径需同步调整
- **回退方案**：`git revert` 本 change 即可，planner 回到单文件

## 关联

- 关联里程碑：**MS07-T03**（基础能力建设 / planner 模块化）
- 关联 M/K：`M04`（SQL 解析 + PhysicalPlan）
- 后续依赖 change（不在本 change 范围）：MS07-T06 谓词下推（落到 query/expression 模块）、MS07-T04、MS07-T05、MS07-T07