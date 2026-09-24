# group-by-expression Specification

## Purpose

约束 GROUP BY 表达式/别名/位置引用的分桶语义：聚合查询的分组键除列名外 SHALL 接受 SELECT 别名、与 SELECT 项等价的表达式和 1-based 位置引用，分桶语义三者与底层表达式求值 SHALL 等价，既有列名分组 SHALL 零回归。来源：MS13-T02（change `2026-09-23-ms13-analytics-functions`，决策 3——date_trunc 分桶主用法解锁）。

## Requirements

### Requirement: GROUP BY 别名与表达式引用

聚合查询的 GROUP BY 项 SHALL 依次按以下顺序解析：SELECT 列名（既有语义优先）→ SELECT 别名 → 与 SELECT 投影项文本等价的表达式 → 位置引用；分桶键 SHALL 按匹配的投影项表达式对输入行求值（分组键 = 求值结果）。

#### Scenario: 别名分桶

- **GIVEN** 事件表 ts 列含跨日时间戳，`SELECT date_trunc('day', ts) AS day, COUNT(*) FROM t GROUP BY day`
- **WHEN** 执行
- **THEN** 按截断日分桶计数（每桶行数正确）

#### Scenario: 表达式分桶等价

- **GIVEN** 同数据
- **WHEN** `GROUP BY date_trunc('day', ts)`（表达式文本形态）
- **THEN** 与别名形态结果逐字节一致

#### Scenario: 位置引用分桶等价

- **GIVEN** 同数据
- **WHEN** `GROUP BY 1`
- **THEN** 与别名/表达式形态结果逐字节一致

### Requirement: 匹配失败与既有约束保持

GROUP BY 项无法按上述顺序解析时 SHALL 显式错误（点名不匹配项）；非聚合列检查、`GROUP BY ALL`、HAVING 既有语义 SHALL 保持；表达式分组键经 Date/Timestamp/数值 Hash 分桶 SHALL 正确（含 NULL 键归并）。

**校准（Iteration 001 Review 记录，BH-1 同型先例）**：`tests/projection_expression_test.rs::aggregate_expression_error_preserved` 与 `tests/scalar_function_test.rs::aggregate_mixed_with_scalar_function_rejected` 原锁定「聚合 × 未分组表达式项」经旧通道 `Invalid aggregate argument` 拒绝——R1 将混合投影合法化后该拒绝通道被本 Requirement 的 `NonAggregatedColumn`（点名未分组项）取代；两处断言校准为新通道，拒绝语义（未分组表达式项 SHALL NOT 静默通过）不变。此校准为 R1 合法化的必然结果，不计入零回归破例。

#### Scenario: 不匹配表达式显式错误

- **GIVEN** `SELECT COUNT(*) FROM t GROUP BY upper(nonexistent)`
- **WHEN** 执行
- **THEN** 显式错误（不静默回退或空结果）

#### Scenario: NULL 分组键归并

- **GIVEN** 分组表达式对部分行求值为 NULL
- **WHEN** 分组
- **THEN** NULL 归并为单一分组（SQL 语义）

#### Scenario: 既有列名分组零回归

- **GIVEN** 既有聚合/GROUP BY 测试全量
- **WHEN** 实施后运行
- **THEN** 零修改通过
