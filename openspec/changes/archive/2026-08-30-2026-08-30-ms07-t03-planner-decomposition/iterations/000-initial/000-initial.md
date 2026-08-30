# Iteration 000 / Cycle 000: MS07-T03 planner 模块化拆分

> _Plan Context 与 Act Response 与 Plan Review 同文件：Plan Context（draft）→ Act Response（reported）→ Plan Review（accepted）。_

## Plan Context

- Status: ready
- Authorization: 用户批准 Gate 1 与 Gate 2 并指示开始（原话：「同意，开始吧」）。决策 6 模块 + A(pub(crate) 字段) + A(测试随函数迁移)。Gate 2 Readiness 表 7 项全 PASS。
- Iteration: 000-initial
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3, T4（`tasks.md` §1-§4）
- Depends on: None（MS07-T01/T02 已合并；HEAD = `50ef820`，工作区干净）
- Stable baseline: `src/parser/planner.rs` 拆为 `src/parser/planner/` 目录 6 模块；`PlanBuilder` 公共 API 与 SQL 计划输出零变化；12 个单测随函数迁移且全绿
- Verification boundary: `cargo build` 0 warning；`cargo clippy -D warnings` 0 warning；`cargo fmt --check` 0 diff；`cargo test --all` 0 failures（542 tests，含迁移后 12 个 planner 单测）；`tests/planner_test.rs` 与 `tests/executor_test.rs` 不修改逻辑仍全绿
- Diagnostic boundary: `src/parser/planner/` 目录 + `src/parser/mod.rs`
- Deferred tasks: None（本 change 完成 MS07-T03 全部子项；MS07-T04/T05/T06/T07 留独立 change）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 完整 MS07-T03 范围（仅 planner 模块化纯搬移重构；不含显式事务 API、Checkpoint、谓词下推、消息传递重构）
- Excluded scope: 任何 SQL 逻辑重建/重构；`ast.rs`/`error.rs`/`value.rs`；代码清理/注释润色（属搬移必需 `mod`/import 调整除外）；MS07-T04/T05/T06/T07

**Objective**

把单文件 `src/parser/planner.rs`（2266 行）按职责拆为 `src/parser/planner/` 目录下 `mod.rs` + `query.rs` + `expression.rs` + `aggregate.rs` + `subquery.rs` + `ddl_dml.rs` 六个模块，函数正文逐字搬移不改逻辑。`PlanBuilder` 公共 API（`new`/`register_table`/`build_plan`/`Default`）与 `rtsql::parser::PlanBuilder`、`rtsql::PlanBuilder` re-export 路径不变；`PhysicalPlan` 输出结构不变。三个字段改 `pub(crate)` 支持跨模块 `impl PlanBuilder` 块。12 个内联单元测试随被测函数迁移到对应子模块 `#[cfg(test)]`，实现"任意子模块可独立单测"。全量回归 542 tests 全绿、clippy/fmt 归零。

**Background**

- MS07-T03（`tasks.md:128-139`）定位"基础能力建设 / planner.rs 2266 → 按 build_* 拆分到 4-6 个模块"，验收含"planner 任意子模块可独立单测"。
- 当前 `src/parser/planner.rs` 2266 行单文件，12 个内联测试集中在 `planner.rs:2064-2266`，全部 `super::*` 依赖整个 impl 块，子模块不可独立单测。
- 后续 MS07-T06 谓词/LIMIT 下推需要 query/expression 模块落点——本拆分是前置重构。
- 用户决策（6 + A + A）：
  - 模块数量：6 个（按职责内聚，subquery 独立）
  - 字段可见性：A → 三字段 `pub(crate)`（最小改动、方法签名不变、不对外泄漏）
  - 测试归属：A → 随函数迁移到各子模块

**Current Baseline**

- Revision: `50ef820`（master @ 2026-08-30；文档与 MS07-T02 同步）
- 工作区干净（`git status --porcelain` 空）
- 测试基线：542 tests pass（SNAPSHOT 记录；本 Cycle 未重跑，Act 以实际运行为准）
- `src/parser/planner.rs` 2266 行；`PlanBuilder` struct（行 24-32）；须迁移方法见 `design.md` 模块归属
- `impl Default for PlanBuilder`（行 2058-2062）；`#[cfg(test)] mod tests`（行 2064-2266，12 个测试）
- `inner_table_names` 已在 `build_query`(416-425)、`build_expression`(1037)、`try_build_where_subquery`(1158-1211) 读写——跨模块访问需求确认

**Current-State Evidence**

- `src/parser/planner.rs:24-32` `PlanBuilder` struct（三字段：`tables`/`primary_keys` private，`inner_table_names` pub(crate)）
- `src/parser/planner.rs:34-90` `new`/`register_table`/`build_plan` dispatcher（`Statement::Query/Insert/Update/Delete/CreateTable/Drop` 六个分支 → 对应 build_*）
- `src/parser/planner.rs:93-104` `validate_table`
- `src/parser/planner.rs:1070` 附近 `collect_outer_column_refs` 调用面；`inner_table_names` 写点 416/425/1158/1166/1203/1211
- `src/parser/mod.rs:12` `pub use planner::PlanBuilder;`
- `src/lib.rs:16` `pub use parser::{parse_sql, PlanBuilder, PlanError};`
- `src/pipeline.rs:63,64,72,78` `PlanBuilder::new().build_plan` 与 `register_table` 调用（不修改）
- `tests/planner_test.rs`（624 行，仅公共 API）；`tests/executor_test.rs` 4 处 `PlanBuilder` 用例（1642-1726，仅公共 API）
- 12 个内联测试分布（`planner.rs:2069/2076/2086/2095/2114/2135/2157/2178/2197/2216/2227/2243`）

**Relevant Code**

| 文件 | 符号 | 职责 |
|---|---|---|
| `src/parser/planner.rs` → `src/parser/planner/mod.rs` | `PlanBuilder` struct, `new`/`Default`/`register_table`/`build_plan`/`validate_table`, 子模块声明 | 核心 facade |
| `src/parser/planner/query.rs`（新增） | `build_query` 等 10 个 SELECT/scan/join/PK 方法 + 2 自由函数 | SELECT 计划 |
| `src/parser/planner/expression.rs`（新增） | `build_expression`/`build_where`/`convert_comparison_op`/`resolve_column_ref`/`expr_to_column_name` | 表达式与谓词 |
| `src/parser/planner/aggregate.rs`（新增） | `build_having`/`build_having_expression` + 3 自由函数 | 聚合/HAVING |
| `src/parser/planner/subquery.rs`（新增） | `try_build_where_subquery` 等 7 方法 | 子查询/相关列 |
| `src/parser/planner/ddl_dml.rs`（新增） | DML + DDL 11 方法 | DDL/DML |
| `src/parser/mod.rs` | `pub mod planner;` + re-export | 入口（仅确认，不改逻辑） |

**Critical Path**

```
src/parser/planner.rs  (原单文件)
   │ git mv → src/parser/planner/mod.rs
   ├─ mod.rs:  struct + core(4) + validate_table + mod query/expression/aggregate/subquery/ddl_dml
   ├─ query.rs:   SELECT/scan/join/PK-opt (impl PlanBuilder)
   ├─ expression.rs: build_expression/build_where/resolve/conversion
   ├─ aggregate.rs:  HAVING + aggregate fn helpers
   ├─ subquery.rs:   correlated/subquery helpers
   └─ ddl_dml.rs:    INSERT/UPDATE/DELETE + CREATE/DROP
           ↓ 从原 impl 块逐字搬移（保留 impl PlanBuilder 块外成员）
   cargo build → clippy → fmt → cargo test --all (542) 全绿
```

**Implementation Guidance**

- 用 `git mv` 保留历史；子模块建为 private `mod`（不 `pub use`），`parser::planner` 只暴露 `PlanBuilder`。
- 各子模块 `impl PlanBuilder { ... }` 块内方法直接读 `self.tables`/`self.primary_keys`/`self.inner_table_names`（三字段 `pub(crate)` 后子模块可访问）。
- 需要的 crate 内类型：按需 `use crate::executor::{...}`、`use crate::parser::error::PlanError`、`use crate::parser::value::value_from_sqlparser`、`use sqlparser::ast::...`；`ast::*` 符号（如 `extract_join_table_name`）在调用处用完整路径 `crate::parser::ast::extract_join_table_name` 显式引用（参考原 `build_from_clause_with_projection` 内 `use crate::parser::ast::extract_join_table_name;` 的局部 use 模式，保留原风格）。
- 自由函数（`parse_limit_value`/`parse_offset_value`/`expr_to_column_name`/`is_aggregate_expr`/`extract_aggregate_func`/`extract_single_column_arg`/`extract_column_name`）按 design 归属迁入对应模块，保持 `fn`（非方法）。
- 12 个测试迁移到对应模块 `#[cfg(test)] mod tests { use super::*; ... }`，断言逐字保留。
- 每步后跑 `cargo check` 缩小定位；完成后跑 `cargo build`/`cargo clippy -D warnings`/`cargo fmt --check`/`cargo test --all`。
- `cargo fmt` 自动修复产生的格式 diff 可执行；但不改任何函数正文逻辑。
- 若 `pub(crate)` 字段在某个子模块实际未使用，编译器会报 `unused` 警告——确认字段确实跨模块使用或按"最少可见性"再评估；但当前三个字段（尤其 `inner_table_names` 经 expression.rs/subquery.rs/query.rs 跨模块读写）都有跨模块需求，预期全 `pub(crate)` 即有使用方。

**Behavioral Change**

- 当前行为：`src/parser/planner.rs` 单文件，`PlanBuilder` 私有字段 + 全部方法 + 12 内联测试，`parser::planner` 模块只有一个文件。
- 目标行为：`src/parser/planner/` 目录 6 模块；三字段 `pub(crate)`；`impl PlanBuilder` 分布在子模块；12 测试随函数迁移；**输出 `PhysicalPlan` 结构、方法签名、re-export 路径、行为逐字节不变**。
- 接口变化：三字段 private → `pub(crate)`（lib crate 内部可见，不对外）；无公共符号增删。
- 错误语义：无变化。

**Change Surface**

| Task | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R1/S1.1,R2/S2.1 | `src/parser/planner.rs` → `src/parser/planner/mod.rs` | 单文件承载全部 | 拆为目录 mod.rs + 5 子模块；struct + core 保留；三字段 pub(crate) |
| T2 | R1/S1.2,R5/S5.1 | `src/parser/planner/{query,expression,aggregate}.rs`（新增） | — | 迁入对应方法/自由函数到 impl 块 |
| T3 | R1/S1.2,R4/S4.1 | `src/parser/planner/{subquery,ddl_dml}.rs`（新增）+ 独立测试 | — | 迁入方法 + 迁移 12 测试 |
| T4 | R3/S3.1,R5/S5.1 | 全量回归 | 工作区 | 验证 build/clippy/fmt/test 全绿 + 公共 API/行为零变化 |

**Task Contracts**

### T1: 建立 planner 目录骨架 + mod.rs core + 字段 pub(crate)

- Requirement/Scenario: R1/S1.1, R2/S2.1, R3/S3.1
- Depends on: None
- Targets: `src/parser/planner.rs` → `src/parser/planner/mod.rs`；`src/parser/planner/{query,expression,aggregate,subquery,ddl_dml}.rs`（建空）
- Current behavior: `src/parser/planner.rs` 单文件，`PlanBuilder` 私有字段
- Required behavior: `mod.rs` 含 struct + `new`/`Default`/`register_table`/`build_plan`/`validate_table` + `mod query; mod expression; mod aggregate; mod subquery; mod ddl_dml;`（private）；三字段 `pub(crate)`
- Required changes: `git mv`；建子模块文件；改字段可见性；`build_plan` dispatcher 保留在 mod.rs
- Preserve: `PlanBuilder` 为 `pub`；`new`/`register_table`/`build_plan`/`Default` 签名逐字；`parser::planner` 对外只暴露 `PlanBuilder`
- Forbidden: 不改 build_plan dispatcher 的分支逻辑；不 `pub use` 子模块；不动 `parse_sql`/ast/error/value
- Test witness: 现有 `test_plan_builder_new`/`test_register_table`/`test_validate_table` 迁移到 mod.rs `#[cfg(test)]`；变更前这些测试 GREEN
- GREEN condition: `cargo check` + `cargo test --lib` 中 mod.rs 3 个 core 测试通过
- Verification: `cargo check`；结算见 T4 全量回归
- Stop when: `build_plan` dispatcher 分支逻辑被改动，或 `parser::planner` 意外导出子模块，或公共 API 签名变化

### T2: 迁移 query + expression + aggregate 方法

- Requirement/Scenario: R1/S1.2, R5/S5.1
- Depends on: T1
- Targets: `src/parser/planner/query.rs`、`expression.rs`、`aggregate.rs`
- Current behavior: 方法在单文件 `impl PlanBuilder` 块内（`build_query`@394、`build_expression`@1005、`build_where`@1095、`build_having`@958、`build_having_expression`@853、`convert_comparison_op`@835、`resolve_column_ref`@106、`expr_to_column_name`@2045、`parse_limit_value`/`parse_offset_value`@1946/1956、`extract_column_name`@1936）
- Required behavior: 方法以 `impl PlanBuilder` 块迁入对应子模块；自由函数以 `fn` 迁入；函数正文逐字
- Required changes: 按 design 归属搬移；补子模块 imports；`inner_table_names` 跨模块读写（expression.rs:1037）已验证可用（pub(crate)）
- Preserve: 方法签名、async 标记、可见性、返回值逐字；`build_from_clause_with_projection` 的局部 `use crate::parser::ast::extract_join_table_name;` 保留
- Forbidden: 改逻辑/命名/控制流；public 化原是 private 的方法
- Test witness: `test_build_query_scan`/`test_build_query_index_scan`/`test_extract_pk_from_where_reversed`/`test_nonexistent_table`/`test_unsupported_where` 迁移到 query.rs；变更前 GREEN
- GREEN condition: `cargo check` + `cargo test --lib` 通过
- Verification: `cargo check`；结算见 T4
- Stop when: 方法签名变化，或函数正文被改动（非搬移）

### T3: 迁移 subquery + ddl_dml 方法并迁移全部测试

- Requirement/Scenario: R1/S1.2, R4/S4.1, R5/S5.1
- Depends on: T2
- Targets: `src/parser/planner/subquery.rs`、`ddl_dml.rs`；所有 `#[cfg(test)]`
- Current behavior: 方法在单文件（`try_build_where_subquery`@1143、`extract_subquery_table_names`@1244、`extract_correlated_params`@1272、`collect_outer_column_refs`@1294、`has_outer_refs_outside`@1413、`resolve_column_in_plan`@1475、`get_subquery_first_column`@1502、`build_output_columns_for_table`@1556、`build_insert`@1580、`extract_insert_values`@1604、`convert_data_type`@1643、`extract_column_constraints`@1687、`extract_default_value`@1717、`extract_primary_key`@1748、`build_create_table`@1790、`build_drop_table`@1830、`build_update`@1851、`build_delete`@1908、`extract_join_conditions`@166）
- Required behavior: 方法迁入 ddl_dml.rs；DDL 归类 + DML 归类；12 测试迁移到对应模块（core→mod.rs、query→query.rs、ddl_dml→ddl_dml.rs）
- Required changes: 编组归属（`build_output_columns_for_table`→query.rs；其余 DML/DDL→ddl_dml.rs）；测试随函数迁移
- Preserve: 断言逐字；`inner_table_names` 读写（subquery.rs 1158-1211）
- Forbidden: 新增/删除测试断言；改逻辑
- Test witness: `test_build_insert`/`test_build_update`/`test_build_delete`/`test_insert_multiple_rows` 迁移到 ddl_dml.rs；全部 12 测试迁移后 `cargo test --lib` 全绿
- GREEN condition: `cargo test --lib` 12 单测全绿；`cargo test --all` 0 failures
- Verification: `cargo build` 0 warning、`cargo clippy -D warnings` 0 warning、`cargo fmt --check` 0 diff、`cargo test --all`
- Stop when: 测试断言被改，或迁移后公共 API/re-export 变化

### T4: 全量回归与验证

- Requirement/Scenario: R3/S3.1, R5/S5.1
- Depends on: T3
- Targets: 全工作区
- Current behavior: 无（T3 已完成拆分）
- Required behavior: `cargo build`/`cargo clippy -D warnings`/`cargo fmt --check`/`cargo test --all` 全绿；`tests/planner_test.rs`/`tests/executor_test.rs` 不修改仍全绿
- Required changes: 验证（无代码改动）
- Preserve: 公共 API 与 re-export 路径
- Forbidden: 为过 clippy 引入 `#[allow]` 压制；修改 `tests/planner_test.rs`/`tests/executor_test.rs` 逻辑
- Test witness: 全量 `cargo test --all`（542 基线）
- GREEN condition: 4 项检查全绿
- Verification: `cargo test --all` 0 failures；`cargo clippy -D warnings` 0 warning；`cargo build` 0 warning；`cargo fmt --check` 0 diff
- Stop when: 任何 check 失败需返工；或发现公共 API/re-export 变化

**Invariants**

- `PlanBuilder` 公共 API（`new`/`register_table`/`build_plan`/`Default`）签名不变。
- `rtsql::parser::PlanBuilder`、`rtsql::PlanBuilder` re-export 路径不变。
- `PhysicalPlan` 输出结构不变；SQL 语义逐字节不变。
- 12 个 planner 单测的断言逐字保留，仅迁移位置。
- `parser/planner` 子模块保持 private（不对外导出）；`parser::planner` 只暴露 `PlanBuilder`。

**Non-goals**

- 任何 SQL 逻辑重建/重构；不改函数正文。
- `ast.rs`/`error.rs`/`value.rs`、`pipeline.rs`、`lib.rs` 逻辑模块。
- MS07-T04/T05/T06/T07。
- 注释润色/代码清理/文档完善（纯搬移 `mod` 声明与 import 调整除外）。

**Acceptance**

| Acceptance | 验证 |
|---|---|
| R1 六职责模块 | T2/T3 目录结构 + `mod.rs` 只暴露 `PlanBuilder`；无子模块符号泄漏 |
| R2 多 impl 块 + pub(crate) 字段 | T1.3 三字段 `pub(crate)`；`cargo build` 通过 |
| R3 公共 API 零变化 | T4 `tests/planner_test.rs`+`executor_test.rs` 不改仍全绿；re-export 路径不变 |
| R4 测试随迁移独立单测 | T3.3 12 单测迁移；`cargo test --lib` 全绿 |
| R5 编译与回归零告警 | T4 全量回归 + clippy/fmt 归零 |

**Verification**

- `cargo build`（0 warning）
- `cargo test --all`（542 tests，0 failures；含迁移后 12 个 planner 单测）
- `cargo clippy -D warnings`（0 warning）
- `cargo fmt --check`（0 diff）
- `tests/planner_test.rs`（624 行）与 `tests/executor_test.rs` 4 处 `PlanBuilder` 用例逻辑零修改仍全绿

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | 单文件结构、imgpl 边界、六模块归属、字段 pub(crate) 需要（inner_table_names 跨 3 子模块读写）、公共 API/测试依赖面已确认（本 Cycle Current-State Evidence） |
| Design | PASS | 6+pub(crate)+测试迁移方案；`impl PlanBuilder` 多块 + re-export 不变（design.md） |
| Iteration Plan | PASS | 单 Iteration（T1-T4）依赖有序；稳定基线/验证/诊断边界明确 |
| Cycle Scope | PASS | initial；T1-T4 覆盖 R1-R5 全部 requirement |
| Task Contracts | PASS | 每个 Task 有 Targets/Current/Required/Preserve/Forbidden/Test witness/GREEN/Verification/Stop |
| Traceability | PASS | tasks.md RTM R1-R5 全 Covered |
| Verification | PASS | cargo build/test/clippy/fmt 4 项通过条件清晰 |

**Persisted Evidence**

- Mode: none

`none` — 所有验证（build/clippy/`cargo test --all`/`cargo fmt --check`）可低成本在本地重跑，决定性输出（退出码、warning/failure 计数、测试数）写入 Act Response 即可；字段 pub(crate) 与 zero-dead-code 也可通过 read diff + 编译确认，无需持久化。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB；超限需要用户明确批准。本 Cycle 不需要任何证据文件。

**Risks and Notes**

- **低**：字段 `pub(crate)` 若在某子模块实际未使用 → clippy `unused` 警告；但三个字段（尤其 `inner_table_names`）经 expression.rs/subquery.rs/query.rs 读写，均有跨模块使用方，预期归零。
- **低**：`ast::*` 符号（`extract_join_table_name`/`extract_join_conditions` 等）需要显式 import；按原局部 `use` 风格处理。
- **低**：`parser::planner` 若意外 `pub use` 子模块会泄漏；保持 private `mod` 声明。
- **非实质**：测试具体安置到每个模块的确切 `#[cfg(test)]` 划分（mod.rs 3 / query.rs 5 / ddl_dml.rs 4）是 Act 可决定的局部选择，不影响契约语义；但必须每处断言逐字保留、总数恒为 12。
- **非实质**：`build_output_columns_for_table` 归属 query.rs（design 已定），无歧义。
- 本重构不涉及安全/数据/性能关键路径。

## Act Response

- Status: reported（含 Plan Review 触发的 `cargo fmt` 修复）
- Cycle type: initial
- Completion date: 2026-08-30

### 实际改动

按 T1→T2→T3→T4 顺序实施。**纯搬移重构** — 函数正文逐字保留，无逻辑重建；只调整可见性、模块归属与必要 import。

#### Plan Review 触发的 fmt 修复

`Plan Review`（`pending`）独立核实 R5 阻塞：原 Act Response 验证表声称 `cargo fmt --check` exit 0，与实际 5 处 diff / exit 1 不符。复跑确认（修复前实测 5 处 diff）：

1. `src/parser/planner/expression.rs:7` — `use crate::executor::{…}` 包裹行未按 rustfmt 收敛。
2. `src/parser/planner/query.rs:118` — `extract_join_conditions` 调用行应收敛为多行。
3. `src/parser/planner/query.rs:128` — `columns.iter().enumerate().map(...)` 闭包应收敛为单行。
4. `src/parser/planner/query.rs:201` — `subquery_evals: Vec<(...)>` 复合类型应展开为多行。
5. `src/parser/planner/query.rs:566` — `has_pk_equality` 参数签名应收敛为单行。

修复操作：直接执行 `cargo fmt`（`rustfmt` 5 处自动收敛 / 展开）；**未改动任何函数正文逻辑**；**未新增 `#[allow]` / `#[expect]` 压制**。修复后 `cargo fmt --check` 退出码 0、无 diff；`cargo build` 0 warning、`cargo clippy --all-targets -- -D warnings` 0 warning、`cargo test --all` 542 tests 0 failures；`tests/planner_test.rs`（29 passed）与 `tests/executor_test.rs`（39 passed）零修改仍全绿。

> 错误记录：原始 Act Response 验证表误判 `cargo fmt --check` exit 0。根因是 `bash` 管道 `cargo fmt --check 2>&1 | tail -3; echo "---FMT EXIT: $?---"` 中 `$?` 取的是 `tail` 的退出码而非 `cargo fmt --check` 的退出码（`PIPESTATUS[0]` 才是）。后续验证改用 `${PIPESTATUS[0]}` 与独立 `cargo fmt --check; echo $?` 复核。

- **T1**：git mv `src/parser/planner.rs` → `src/parser/planner/mod.rs`；建空子模块 `query.rs` / `expression.rs` / `aggregate.rs` / `subquery.rs` / `ddl_dml.rs`；mod.rs 加 `mod xxx;`（private）声明；字段 `tables` / `primary_keys` 由 `private` → `pub(crate)`（`inner_table_names` 已是 `pub(crate)`）；保留原 12 个 `#[cfg(test)]` 测试原地。
- **T2**：迁移 13 个方法 + 7 个自由函数到子模块，方法签名逐字保留，跨子模块自由函数通过 `super::xxx::name` 引用。
  - `query.rs`（10 方法 + 3 自由函数）：`build_query` / `build_from_clause_with_projection` / `get_plan_output_columns` / `build_output_columns_for_table` / `is_simple_pk_equality` / `has_pk_equality` / `extract_pk_from_where` + 自由函数 `extract_column_name`（ORDER BY 用） / `parse_limit_value` / `parse_offset_value`
  - `expression.rs`（4 方法 + 1 自由函数）：`resolve_column_ref` / `convert_comparison_op` / `build_expression` / `build_where` + 自由函数 `expr_to_column_name`
  - `aggregate.rs`（2 方法 + 3 自由函数）：`build_having_expression` / `build_having` + 自由函数 `is_aggregate_expr` / `extract_aggregate_func` / `extract_single_column_arg`
- **T3**：迁移剩余 18 个方法 + 9 个测试到子模块。
  - `subquery.rs`（7 方法）：`try_build_where_subquery` / `extract_subquery_table_names` / `extract_correlated_params` / `collect_outer_column_refs` / `has_outer_refs_outside` / `resolve_column_in_plan` / `get_subquery_first_column`
  - `ddl_dml.rs`（11 方法 + 4 测试）：`extract_join_conditions`（per design，归属 ddl_dml）+ `build_insert` / `extract_insert_values` / `convert_data_type` / `extract_column_constraints` / `extract_default_value` / `extract_primary_key` / `build_create_table` / `build_drop_table` / `build_update` / `build_delete` + `#[cfg(test)]` 4 测试（test_build_insert / test_build_update / test_build_delete / test_insert_multiple_rows）
  - `query.rs` 加 `#[cfg(test)]` 5 测试（test_build_query_scan / test_build_query_index_scan / test_extract_pk_from_where_reversed / test_nonexistent_table / test_unsupported_where）
  - mod.rs `#[cfg(test)]` 保留 3 core 测试（test_plan_builder_new / test_register_table / test_validate_table）

### 文件和符号

**新增/移动文件**（行数含 12 单测）：

| 文件 | 行数 | 内容 |
|---|---|---|
| `src/parser/planner/mod.rs` | 146 | struct + 4 core 方法（new/register_table/build_plan/validate_table）+ impl Default + 3 单测 |
| `src/parser/planner/query.rs` | 791 | 10 方法 + 3 自由函数 + 5 单测 |
| `src/parser/planner/expression.rs` | 253 | 4 方法 + 1 自由函数 |
| `src/parser/planner/aggregate.rs` | 255 | 2 方法 + 3 自由函数 |
| `src/parser/planner/subquery.rs` | 433 | 7 方法 |
| `src/parser/planner/ddl_dml.rs` | 517 | 11 方法 + 4 单测 |
| 总计 | 2395 | （原单文件 2266 行） |

净增 129 行 = 12 测试代码 + 模块头注释 + 必要 import + impl 块结构。

**符号/路径**：

- `pub use planner::PlanBuilder;`（`src/parser/mod.rs:12`）不变
- `pub use parser::{parse_sql, PlanBuilder, PlanError};`（`src/lib.rs:16`）不变
- `PlanBuilder::{new, register_table, build_plan, Default}` 公共 API 签名逐字节不变
- `parser::planner` 只对外暴露 `PlanBuilder`（子模块 `mod xxx;` private，不 `pub use`）
- `tests/planner_test.rs`（624 行）与 `tests/executor_test.rs` 4 处 `PlanBuilder` 用例**逻辑零修改**仍全绿

### 与计划的偏差及原因

无实质偏差。三处非实质局部处理（按 plan 风险段已记录为 Act 可决定）：

1. **测试模块 imports 形式**：ddl_dml.rs 测试用 `use super::super::PlanBuilder;`（避免 `use super::*;` 触发 unused import 警告）— 仍在 `#[cfg(test)]` 内部，仅影响测试代码组织。
2. **`build_output_columns_for_table` 归属 query.rs**：与 design 一致。
3. **ddl_dml.rs 模块内包含 `extract_join_conditions`**：按 design.md 归属（虽然语义上更像 query，但 plan 显式归属 ddl_dml）。`build_from_clause_with_projection`（query.rs）通过 `self.extract_join_conditions(...)` 调用即可，跨模块 `impl PlanBuilder` 方法解析不依赖模块归属。

### Self-Review 检查结果

**全量 diff Review**（完成 T4 后统一审）：

- ✅ 仅修改目标：`src/parser/planner.rs` → `src/parser/planner/{mod,query,expression,aggregate,subquery,ddl_dml}.rs`；其他 542 tests 测试文件零修改
- ✅ 公共 API 零变化（re-export 路径、`PlanBuilder` 方法签名、字段可见性仅 lib crate 内部变化）
- ✅ 函数正文逐字保留（无逻辑、命名、控制流改动）
- ✅ 测试断言逐字保留（12 个测试全部断言原样）
- ✅ 0 `#[allow(dead_code)]` 等压制
- ✅ 0 clippy warning、0 fmt diff、0 cargo build warning
- ✅ 所有错误/边界/状态/资源生命周期继承原实现

**逐 Task Gate 4 复审**：

- T1：spec ✅（目录结构 + 字段 pub(crate) + 5 子模块 private mod）；code ✅（编译通过、12 测试不依赖 T1 改动）
- T2：spec ✅（query 10 + expression 5 + aggregate 5 方法/自由函数全部到位）；code ✅（cargo check 0 warning；tests 不依赖此步但保留 12 测试兼容）
- T3：spec ✅（subquery 7 + ddl_dml 11 方法全部到位；12 测试按 mod.rs 3 / query.rs 5 / ddl_dml.rs 4 分布）；code ✅（cargo test 12 全绿）
- T4：spec ✅（4 项验证全绿）；code ✅（回归 542 + 公共 API 路径不变）

**已修复发现**：

1. T2 阶段：`crate::parser::ast::*` 实际**不**re-export `sqlparser::ast::*` 类型（如 `Expr`/`Query`）— 改在 query/expression/aggregate/subquery 中直接 `use sqlparser::ast::{Expr, ...};`。
2. T2 阶段：mod.rs 残留大量 unused imports — 按 T2 实际保留的 `extract_join_conditions`（T3 前）+ dispatcher 所需精简。
3. T2 阶段：ddl_dml.rs 残留 `use super::*;` 触发 unused import — 改用 `use super::super::PlanBuilder;`。
4. T3 阶段：Python 脚本批量删除方法时残留前序方法的 doc comments（行 115-129、line 938-943、line 1000+ 的孤儿 `///`）和 `#[test]` 标记 — 手动 `edit` 修复。
5. T3 阶段：mod.rs 重复 `validate_table` 定义（脚本误保留两份）— 手动删除重复。

**遗留 Minor 问题**：

- 工具遗留警告：`/home/daivy/.cargo/config is deprecated in favor of config.toml` — 项目无关的环境配置提示，与本 change 无关。
- （已修正）原"遗留"段提及 `/// 提取 JOIN ON 条件` 在 T3 期间 mod.rs 残留 — 该 doc comment 实为 `ddl_dml.rs::extract_join_conditions` 的正确文档（design 归属 ddl_dml），并非残留。

**Plan Review 触发的 fmt 修复**（详见上文"Plan Review 触发的 fmt 修复"段）：

- 修复操作：`cargo fmt`（rustfmt 自动收敛/展开）— 仅 5 处格式 diff 受影响，0 行逻辑变更。
- 修复后 `cargo fmt --check` exit 0；其余 3 项验证仍 exit 0。
- 修复后 diff 仅触及 `src/parser/planner/{expression,query}.rs` 2 文件的格式；`src/parser/planner/{mod,aggregate,subquery,ddl_dml}.rs` 与已删除 `planner.rs` 在 git 视角下零新增格式改动（已合规）。
- 未创建新文件、未修改 openspec 元数据。

### 验证命令、输出和退出码

> **注**：原验证表误用 `$?` 捕获 `tail` 退出码（`bash` 管道语义）。本节验证改用 `${PIPESTATUS[0]}` 与独立 `cmd; echo $?` 复核，所有数字均为修复后实测。

| 验证项 | 命令 | 输出摘录 | 退出码 | 结论 |
|---|---|---|---|---|
| Build | `cargo build` | `Finished dev profile … in 1.09s`（无 warning） | 0 | PASS |
| Clippy | `cargo clippy --all-targets -- -D warnings` | `Finished dev profile … in 1.50s`（无 warning） | 0 | PASS |
| Fmt | `cargo fmt --check` | （无 diff） | 0 | PASS |
| Lib tests | `cargo test --lib parser::planner` | `test result: ok. 12 passed; 0 failed` | 0 | PASS |
| All tests | `cargo test --all` | `TOTAL: 542`（含 planner 12 + planner_test 29 + executor_test 39） | 0 | PASS |
| Planner test | `cargo test --test planner_test` | `test result: ok. 29 passed` | 0 | PASS |
| Executor test | `cargo test --test executor_test` | `test result: ok. 39 passed` | 0 | PASS |
| OpenSpec | `openspec validate --all` | `Totals: 12 passed, 0 failed (12 items)` | 0 | PASS |

**测试分布**（按 spec 验收）：

- `src/parser/planner/mod.rs::tests`：3 测试（test_plan_builder_new / test_register_table / test_validate_table）
- `src/parser/planner/query.rs::tests`：5 测试（test_build_query_scan / test_build_query_index_scan / test_extract_pk_from_where_reversed / test_nonexistent_table / test_unsupported_where）
- `src/parser/planner/ddl_dml.rs::tests`：4 测试（test_build_insert / test_build_update / test_build_delete / test_insert_multiple_rows）

### Persisted Evidence

- `None required` — `Plan Context: Persisted Evidence: none`；本 Cycle `none` 模式保持；所有验证可低成本重跑，决定性输出（退出码 0、warning/failure 计数 0、test 计数 542）已记录在 Act Response 表格。
- 未创建 `evidence/000-initial/000-initial/` 目录（符合 plan 设置）。

### Experience Candidates

- `None` — 实施过程无超出常规 TDD 模式的工程经验；纯搬移重构，未遇到 Runbook-worthy 重复操作或 Incident-worthy 故障。
- 提示：用户的 `pub(crate)` 字段可见性决策避免了大量 getter 样板代码；本 change 自身不构成 R 登记候选。

### 未解决问题

无。Plan Context 5 个 Requirement (R1-R5) 全部 Covered + Verified。

### 可选 commit / diff 引用

- diff stat：
  ```
  src/parser/planner.rs     | 2266 ----------------
  src/parser/planner/mod.rs |  146 ++
  src/parser/planner/aggregate.rs   | (new, 255 lines)
  src/parser/planner/ddl_dml.rs     | (new, 517 lines)
  src/parser/planner/expression.rs  | (new, 253 lines)
  src/parser/planner/query.rs       | (new, 791 lines)
  src/parser/planner/subquery.rs    | (new, 433 lines)
  ```
- 暂未 commit（保留用户审计后触发）；git status 显示 5 文件 staged（openspec change 元数据）+ 1 git rename（planner.rs→mod.rs）+ 5 untracked（新子模块）

## Plan Review

- Review Result: **accepted**

**Findings**

Act 完成六模块搬移重构，并在上一轮 Plan Review（`pending`，ACT-DEVIATION：`cargo fmt --check` exit 1、5 处 diff）触发的当前 Cycle 修复中执行 `cargo fmt` 收敛了全部格式 diff。独立复审确认：

- 上一轮唯一阻塞项（R5 `cargo fmt --check` 未 0 diff）已被**完全关闭**：本轮独立重跑 `cargo fmt --check` **exit 0、无 diff**。
- R1-R5 全部满足：
  - **R1 六职责模块**：6 模块齐备（mod.rs + query/expression/aggregate/subquery/ddl_dml），35 个 spec 命名符号逐一落在指定模块，子模块全部 private `mod`（`parser::planner` 无符号泄漏）。
  - **R2 多 impl + pub(crate) 字段**：`tables`/`primary_keys`/`inner_table_names` 均 `pub(crate)`，`inner_table_names` 跨 mod/query/expression/subquery 使用处一致。
  - **R3 公共 API 零变化**：`parser::planner`/`lib` re-export 路径未变；`tests/planner_test.rs`(29 passed) 与 `tests/executor_test.rs`(39 passed) 零修改仍全绿。
  - **R4 测试随迁移**：12 单测分布 mod.rs 3 / query.rs 5 / ddl_dml.rs 4，断言逐字保留，全绿。
  - **R5 编译与回归零告警**：`cargo build` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` 全 0；`cargo test --all` 542 tests 0 failures。
- 修复保持外科式：仅 `src/parser/planner/{expression,query}.rs` 的格式变化；未新增 `#[allow]`/`#[expect]`（仍为 3 处原文件逐字迁移的 `#[allow(clippy::only_used_in_recursion)]`，经 `git show HEAD:planner.rs` 219/1293/1501 比对确认）；未改动函数正文逻辑；未修改 `tests/` 与 openspec 元数据。

非阻塞 Minor finding：

- 无。

**Deviation Classification**

ACT-DEVIATION（上一轮：Act 声称 `cargo fmt --check` PASS，实测 exit 1）— 已由当前 Cycle 修复闭环：Act 记录根因（`bash` 管道 `$?` 捕获 `tail` 的退出码而非 `cargo fmt --check` 的，需用 `${PIPESTATUS[0]}`），改用独立复核后复验四项全绿。本 change 当前无未解决偏差。

**Acceptance Gaps**

None — R1-R5 全部满足，上一轮 R5 fmt 缺口已关闭。

**Convergence**

reduced — 上一版当前 Cycle Review 存在 1 项 R5 fmt 缺口；本轮该缺口关闭，无任何 Acceptance gap，收敛状态为 `reduced`。

**Evidence**

独立复审（本 Review 时点，工作区 = 未 commit 的拆分后状态）：

| 验证项 | 命令 | 独立实测 | 结论 |
|---|---|---|---|
| Build | `cargo build` | Finished dev, 0 warning, exit 0 | PASS |
| Clippy | `cargo clippy --all-targets -- -D warnings` | Finished dev, 0 warning, exit 0 | PASS |
| Fmt | `cargo fmt --check` | exit **0**，无 diff（上轮 FAIL → 本轮 PASS） | PASS |
| All tests | `cargo test --all --no-fail-fast` | exit 0；total passed **542**，0 FAILED / 0 error / total_failed 0 | PASS |

结构/语义核验（与上一轮一致且 fmt 修复未触及逻辑，继续有效）：6 模块齐备；三字段 `pub(crate)`；spec 命名 35 符号归属正确；re-export 路径未变；12 单测分布 3/5/4；`#[allow]` 仅 3 处原迁移属性；`tests/` 零修改 — 全部相符。

**Follow-up Decision**

接受。当前 Cycle 修复完整关闭了上轮唯一阻塞缺口（R5 fmt），修复受原契约约束、无逻辑变化、无新增压制，且独立复验证实 build/clippy/fmt/test 四绿、542 tests 0 failures、公共 API/re-export 路径不变。Act Response 已覆盖为含 fmt 修复的最新完整快照（状态 `reported`）。无剩余阻塞 finding，无需 rework/replan，无需当前 Cycle 进一步修复。Iteration 000 达成 `accepted`，MS07-T03 全部范围交付。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None（Iteration 000 唯一 Cycle 已 accepted，change 所有任务完成，无剩余 Iteration）