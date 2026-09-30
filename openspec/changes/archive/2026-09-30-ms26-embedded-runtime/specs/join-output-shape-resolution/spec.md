# join-output-shape-resolution Delta — MS26 T8 派生基座外层投影收口

## ADDED Requirements

### Requirement: 派生基座的外层投影裁剪

单表派生基座（`FROM (子查询) AS alias`、CTE 引用、视图展开——三者共用 `DerivedScan` 通道）上的纯列清单 SELECT SHALL 按外层 SELECT 清单裁剪输出行与表头：输出列恰为外层清单解析到的派生列，顺序与清单一致，表头与行形状一致。外层清单引用派生列集中不存在的列 SHALL 在计划期以 `ColumnNotFound`（42703）点名拒绝，SHALL NOT 静默返回多列（与基表 Scan 基座的严格投影解析同纪律）。WHERE（Filter 承载）与 ORDER BY（Sort 承载）组合 SHALL 保持同一裁剪语义；`CREATE TABLE AS SELECT` 以派生基座为源时列名 SHALL 取裁剪后的输出表头。`SELECT *`、聚合查询（输入保持全形状）、表达式与别名项（既有顶层 Projection 通路）SHALL 维持既有行为。

#### Scenario: 派生表外层投影裁剪

- **GIVEN** 表 `t(id, n)` 含两行
- **WHEN** `SELECT n FROM (SELECT id, n FROM t) AS d`
- **THEN** 输出一列 `["n"]`，行值仅含 n 列（表头与行形状一致）

#### Scenario: CTE 与视图通道等价

- **WHEN** `WITH c AS (SELECT id, n FROM t) SELECT n FROM c`；视图 `v AS SELECT id, n FROM t` 上执行 `SELECT n FROM v`
- **THEN** 两者与派生表形态同型，各输出一列 `["n"]`

#### Scenario: 未知外层列点名拒绝

- **WHEN** `SELECT zzz FROM (SELECT id, n FROM t) AS d`
- **THEN** 计划期 `ColumnNotFound`（42703）点名拒绝，不静默返回派生全集

#### Scenario: 组合面

- **WHEN** `SELECT n FROM (SELECT id, n FROM t) AS d WHERE id = 1`；`SELECT n FROM (SELECT id, n FROM t) AS d ORDER BY n`；`CREATE TABLE c AS SELECT n FROM (SELECT id, n FROM t) AS d`
- **THEN** 分别为：Filter 过滤后单列输出；排序后单列输出；CTAS 建单列表 `n` 并物化对应值

#### Scenario: 恒等面不回归

- **WHEN** `SELECT * FROM (SELECT id, n FROM t) AS d`；`SELECT COUNT(*) FROM (SELECT id, n FROM t) AS d`；`SELECT n AS x FROM (SELECT id, n FROM t) AS d`
- **THEN** 分别为：两列恒等输出；聚合正常计数；别名通路（`error-reporting` R5）输出 `["x"]` 单列——三者均与本 Requirement 增量前行为一致
