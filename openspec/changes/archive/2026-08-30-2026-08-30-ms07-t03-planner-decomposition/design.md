# Design: MS07-T03 planner 模块化拆分

## 概述

把单文件 `src/parser/planner.rs`（2266 行）按职责拆为 `src/parser/planner/` 目录下 6 个模块的纯搬移重构。公共 API `PlanBuilder::{new, register_table, build_plan}` 与 `Default` 行为零变化；`PhysicalPlan` 输出结构不变；SQL 语义逐字节不变。这是为 MS07-T06（谓词下推）等后续扩展提供模块落点的基础重构。

## 目标目录结构

```text
src/parser/
├── mod.rs          # 原 13 行；`pub mod planner;` + re-export（不变）
├── ast.rs / error.rs / value.rs   # 不动
└── planner/                        # 原 src/parser/planner.rs
    ├── mod.rs      # PlanBuilder struct + new/Default/register_table/build_plan/validate_table + 子模块声明
    ├── query.rs    # SELECT: build_query, build_from_clause_with_projection, build_output_columns_for_table,
    │               #   get_plan_output_columns, extract_column_name(ORDER BY), parse_limit_value,
    │               #   parse_offset_value, is_simple_pk_equality, has_pk_equality, extract_pk_from_where
    ├── expression.rs  # build_expression, build_where, convert_comparison_op, resolve_column_ref, expr_to_column_name
    ├── aggregate.rs   # build_having, build_having_expression, is_aggregate_expr, extract_aggregate_func,
    │                  #   extract_single_column_arg
    ├── subquery.rs    # try_build_where_subquery, extract_subquery_table_names, extract_correlated_params,
    │                  #   collect_outer_column_refs, has_outer_refs_outside, resolve_column_in_plan,
    │                  #   get_subquery_first_column
    └── ddl_dml.rs     # DML: build_insert, extract_insert_values, build_update, build_delete,
                       #   extract_join_conditions; DDL: build_create_table, build_drop_table,
                       #   convert_data_type, extract_column_constraints, extract_default_value,
                       #   extract_primary_key
```

## 关键处理

### 1. 字段可见性（决策 A：pub(crate)）

```rust
// mod.rs
#[derive(Debug, Clone)]
pub struct PlanBuilder {
    pub(crate) tables: HashMap<String, Vec<String>>,
    pub(crate) primary_keys: HashMap<String, String>,
    pub(crate) inner_table_names: Option<Vec<String>>,
}
```

- `inner_table_names` 已是 `pub(crate)`（planner.rs:31）——不变
- `tables`/`primary_keys` 原 private → `pub(crate)`
- `PlanBuilder` 仍是 `pub` struct，但 `pub(crate)` 字段不会对外导出；`lib.rs` re-export 只暴露类型与方法

### 2. 使用完整路径访问同 crate 类型

各子模块 `use crate::executor::{...}`（按需取用的符号）、`use crate::parser::error::PlanError`、`use crate::parser::value::value_from_sqlparser`、`use sqlparser::ast::...`。`ast::*`（`extract_join_table_name` 等）在调用处用完整路径或显式 import。

### 3. parser::planner 的全局导出

`mod.rs` 声明 `mod query; mod expression; mod aggregate; mod subquery; mod ddl_dml;`（private，保持对内隐藏），并**不** `pub use` 它们，避免 `parser::planner::query` 泄漏。`PlanBuilder` 的方法因是 `impl PlanBuilder`，调用方只需 `use crate::parser::planner::PlanBuilder` 即可调用所有方法（方法解析不依赖调用方看到 impl 块所在模块）。

### 4. 测试迁移

每个子模块末尾 `#[cfg(test)] mod tests { use super::*; ... }`。迁移时：
- 保留断言不变。
- `use super::*` 引入本模块方法与 `PlanBuilder`/字段。
- 需构造 `PlanBuilder` 则 `PlanBuilder::new()` + `register_table(...)`。
- 12 个测试分布：mod.rs(core: test_plan_builder_new/test_register_table/test_validate_table)、query.rs(test_build_query_scan/test_build_query_index_scan/test_extract_pk_from_where_reversed/test_nonexistent_table/test_unsupported_where)、ddl_dml.rs(test_build_insert/test_build_update/test_build_delete/test_insert_multiple_rows)。

### 5. doc comment / 模块注解

- `plan/` 各模块顶部保留原 `//!` 或补充一句内聚职责说明。
- 不改动函数正文。

### 6. 为 MS07-T06 预留

`query.rs`（扫描路径 + LIMIT 辅助）与 `expression.rs`（谓词）是下推的天然落点；本轮不实现，仅因拆分产生清晰模块边界。

## 序列（流程）

纯重构无运行时流程变化；Move + 编译 + 回归即完成：

```
1. git mv src/parser/planner.rs src/parser/planner/mod.rs
2. 按模块归属拆分 impl 块 + 自由函数到 5 个子模块
3. 调整字段可见性为 pub(crate)
4. 迁移 12 个测试到对应模块
5. cargo build / clippy / fmt / test 全部归零
```

## 备选方案对比

| 方案 | 选择 |
|---|---|
| **6 模块**（按职责内聚，subquery 独立） | ✅ 采用 |
| 5 模块（pkscan 合并入 query/ddl_dml） | ❌ subquery 仍偏大且可测性差 |
| 4 模块（subquery/expression/aggregate 合一） | ❌ 聚合无独立测试单元 |
| **A** 字段 pub(crate) | ✅ 采用（最小改动，方法签名不变） |
| B 字段 getter 访问器 | ❌ 侵入大，与"仅拆分"不符 |
| **A** 测试随函数迁移 | ✅ 采用（中子模块独立单测 = 验收） |
| B 测试收拢在 mod.rs | ❌ 子模块无法独立单测，与验收冲突 |

## 不需要修改的文件

- `src/parser/ast.rs`、`error.rs`、`value.rs`
- `src/lib.rs`（re-export `parser::{parse_sql, PlanBuilder, PlanError}` 路径不变）
- `src/parser/mod.rs`（`pub mod planner;` 不变）
- `src/pipeline.rs`
- `tests/planner_test.rs`、`tests/executor_test.rs`

## 风险与缓解

| 风险 | 严重度 | 缓解 |
|---|---|---|
| 字段 pub(crate) 触发 dead_code/unused 警告 | 低 | `cargo build`/`cargo clippy -D warnings` 验证归零 |
| 跨模块同 crate 方法调用 | 低 | 方法解析不依赖模块；字段可见即可编译 |
| parser::planner 子模块错误导出 | 低 | 子模块声明为 private `mod`，不 `pub use`；`parser::planner` 只暴露 `PlanBuilder` |
| 内部 `use crate::parser::ast::*` 符号缺失 | 低 | `extract_join_table_name` 等在调用处显式引用 |
| 行为漂移 | 低（关键） | 函数正文逐字搬移；GREEN 以 `tests/planner_test.rs`(624 行) + 12 单测 + 全量 542 回归证明 |