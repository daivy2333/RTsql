# aggregate-output-shape-conformance Specification

## Purpose
约束聚合查询的输出行与表头形状一致：输出恒为 SELECT 清单形状（未入选的分组键不进输出、键序按 SELECT 序呈现），HAVING 对分组键与聚合输出名的绑定零变化，形状安全的直出形态逐字节不变。来源：MS25（change `2026-09-26-ms25-sql-read-surface`，SQLite 语义对齐）。

## Requirements

### Requirement: 聚合查询输出行与表头形状一致

聚合查询（含 GROUP BY）的输出行 SHALL 与表头形状一致：表头 = SELECT 清单名序，行 = SELECT 清单形状。GROUP BY 分组键未被 SELECT 引用时 SHALL NOT 出现在输出行与表头中（仅可作为 HAVING 绑定依据）。SELECT 键项顺序与 GROUP BY 键序不同时，输出 SHALL 按 SELECT 清单序呈现。本能力修复既有缺陷——聚合直出路径表头取 SELECT 序而执行器行恒为「分组键 ++ 聚合值」，键未入选时行宽大于表头列数、键序不同时行/表头置换错位；该错配 SHALL 终止。

#### Scenario: 仅聚合项 + GROUP BY 非投影键

- **GIVEN** 表 `t(k INT PRIMARY KEY, g VARCHAR, v INT)` 含 (1,'a',1),(2,'a',2),(3,'b',3)
- **WHEN** `SELECT count(*) FROM t GROUP BY g`
- **THEN** 表头恰 1 列（`count_star`），每行恰 1 值（现状 RED：表头 1 列而行 2 值，行首携带未入选的分组键）

#### Scenario: SELECT 键序与 GROUP BY 键序不同

- **GIVEN** 表 `t(k INT PRIMARY KEY, a INT, b INT, v INT)` 含 (1,1,2,10)
- **WHEN** `SELECT b, a, count(*) FROM t GROUP BY a, b`
- **THEN** 表头与行均按 SELECT 序（`b`,`a`,`count_star`；现状 RED：行按 GROUP BY 序 `a`,`b` 置换错位）

### Requirement: HAVING 绑定与既有形态零回归

HAVING 对分组键与聚合输出名的绑定语义 SHALL 零变化；直出路径形状安全的既有聚合查询（分组键 ⊆ SELECT 键项、SELECT 键序 = GROUP BY 键序、键项在前聚合在后）计划形状与输出 SHALL 逐字节不变；包装路径既有形态（表达式键、交错序）零回归；全部既有测试除校准白名单外零修改通过。

#### Scenario: 既有直出形态零回归

- **GIVEN** 既有聚合查询形态（分组键在 SELECT 中且序一致，如 `SELECT g, count(*) FROM t GROUP BY g`）
- **THEN** 计划形状、输出行与表头与本变更前逐字节一致

#### Scenario: HAVING 引用未入选分组键

- **GIVEN** 表同上
- **WHEN** `SELECT count(*) FROM t GROUP BY g HAVING g = 'a'`
- **THEN** 分组键绑定可达（包装路径既有能力；直出形态下现状绑定失败面的行为扩展由 Cycle 调查确认无既有断言锁定）

#### Scenario: 全量回归

- **WHEN** `cargo test -- --test-threads=1`
- **THEN** 0 failed（`tests/select_distinct_test.rs::distinct_aggregate_output` 按新形状校准，白名单见 design D10）
