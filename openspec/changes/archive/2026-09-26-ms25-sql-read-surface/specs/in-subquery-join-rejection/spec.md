## MODIFIED Requirements

### Requirement: IN 子查询 JOIN 形态诚实拒绝

`IN (SELECT …)` 子查询的计划内含 JOIN 节点（`PhysicalPlan::Join` 或 `NestedLoopJoin`，无论 SELECT 清单列数、无论子查询内是否含 WHERE）SHALL 在计划期显式拒绝为 SQL 错误，且错误信息 SHALL 统一点名 JOIN 不被支持（`IN subquery with JOIN is not supported`），SHALL NOT 误报为「子查询返回多列」。拒绝 SHALL 发生在计划期（fail-fast，不执行、无副作用），退出码沿用既有 SQL 错误分类（PG 0A000）。子查询计划不含 JOIN 节点时的既有行为 SHALL 逐字节保持：单列子查询可达性与 `_` fallback 臂的既有文案均不变。修订说明：MS25 change `2026-09-26-ms25-sql-read-surface` 解除顶层 WHERE×JOIN 拒绝（D2）后，子查询内 `WHERE + JOIN` 的计划构建不再提前命中 `WHERE CLAUSE ON JOIN` 拒绝臂，改由 `get_subquery_first_column` JOIN 臂拒绝——原「`Unsupported statement type` 逐字节不变」的文案锁定随之终止（该锁定为 D2 前臂序的偶然产物）；拒绝语义本身不变，且与无 WHERE 的 JOIN 形态文案统一（MS17-T02 点名拒绝本意）。

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
- **THEN** 报 SQL 错误，错误信息为 `IN subquery with JOIN is not supported`（与无 WHERE 的 JOIN 形态统一；拒绝语义与 MS25 变更前一致，仅文案由 `Unsupported statement type` 校准为点名错误——场景名保留承载语料连续性，原「`Unsupported statement type` 逐字节不变」锁定由 change `2026-09-26-ms25-sql-read-surface` replan 裁定终止，2026-09-27 用户裁定方案 (a)）
