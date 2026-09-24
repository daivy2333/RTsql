# in-subquery-join-rejection Specification

## Purpose

约束 `IN (SELECT …)` 子查询计划含 JOIN 节点时的计划期拒绝语义：JOIN 形态 SHALL 在计划期显式拒绝且错误信息点名 JOIN 不被支持，SHALL NOT 误报为「子查询返回多列」；非 JOIN 形态既有行为逐字节保持。来源：MS17-T02（ISS02 最小诚实化处置——用户裁定 2026-09-23 维持计划期拒绝、错误文案与事实相符，JOIN 形态取列与 ON 关联参数注册的能力解锁另立 improvement 候选；change `2026-09-23-ms17-t02-defect-closeout`）。

## Requirements

### Requirement: IN 子查询 JOIN 形态诚实拒绝

`IN (SELECT …)` 子查询的计划内含 JOIN 节点（`PhysicalPlan::Join` 或 `NestedLoopJoin`，无论 SELECT 清单列数）SHALL 在计划期显式拒绝为 SQL 错误，且错误信息 SHALL 点名 JOIN 不被支持（`IN subquery with JOIN is not supported`），SHALL NOT 误报为「子查询返回多列」。拒绝 SHALL 发生在计划期（fail-fast，不执行、无副作用），退出码沿用既有 SQL 错误分类。子查询计划不含 JOIN 节点时的既有行为 SHALL 逐字节保持：单列子查询可达性、`WHERE + JOIN` 的既有拒绝文案、`_` fallback 臂的既有文案均不变。

#### Scenario: 单列 JOIN 子查询点名 JOIN 拒绝（修复对象）

- **GIVEN** 表 `o(x INT)`、`r(a INT)`、`s(b INT)` 各含至少一行
- **WHEN** `SELECT o.x FROM o WHERE o.x IN (SELECT r.a FROM r JOIN s ON r.a = s.b)`
- **THEN** 报 SQL 错误（exit 3），错误信息为 `IN subquery with JOIN is not supported`，SHALL NOT 出现 `Subquery returns multiple columns`

#### Scenario: 多列 JOIN 子查询同文案

- **GIVEN** 同上表结构
- **WHEN** `SELECT o.x FROM o WHERE o.x IN (SELECT r.a, s.b FROM r JOIN s ON r.a = s.b)`
- **THEN** 报 SQL 错误，错误信息点名 JOIN 不被支持（主因优先，不报多列）

#### Scenario: WHERE + JOIN 维持既有拒绝

- **GIVEN** 同上表结构
- **WHEN** `SELECT o.x FROM o WHERE o.x IN (SELECT r.a FROM r JOIN s ON r.a = s.b WHERE r.a > 0)`
- **THEN** 报 SQL 错误，错误信息为既有 `Unsupported statement type`（逐字节不变）

### Requirement: 既有 IN 子查询语义零回归

除 JOIN 形态的错误文案变化外，IN 子查询既有行为 SHALL 逐字节保持：非 JOIN 单列 IN 子查询（相关与非相关）可达性与结果集不变；`IN (SELECT …)` 非 JOIN 形态的计划形状（SemiJoin/AntiJoin/Filter 等）不变；`get_subquery_first_column` 既有各形态臂（Scan/DataScan/Filter/Aggregate/SemiJoin/AntiJoin）与 `_` fallback 臂的行为与文案不变。既有子查询测试套件 SHALL 零修改通过。

#### Scenario: 非 JOIN 单列 IN 子查询保持可达

- **GIVEN** 表 `o(x INT)` 含行 `(5)`、`r(a INT)` 含行 `(1),(5)`
- **WHEN** `SELECT o.x FROM o WHERE o.x IN (SELECT r.a FROM r)`
- **THEN** 返回 `[[5]]`，与 change 前逐字节一致

#### Scenario: 既有子查询套件零修改

- **GIVEN** 既有 `tests/subquery_test.rs`（28 用例）与关联子查询缓存等价见证
- **WHEN** 全量测试运行
- **THEN** 全部零修改通过
