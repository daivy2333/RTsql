## ADDED Requirements

### Requirement: IN 子查询的索引扫描计划取首列

`IN (SELECT …)` 右支计划为 `IndexScan` 或 `IndexScanAll` 时，`get_subquery_first_column` SHALL 返回该计划的第一个输出列，列名与 `get_plan_output_columns` 的第一项相同，表名取节点上的表名。输出列为空时 SHALL 仍报 `Subquery returns multiple columns`。SHALL NOT 因计划节点是索引扫描而落入兜底臂误报多列。`IndexScan.columns` 已是收窄后的输出列，取 `columns[0]`。`IndexScanAll.columns` 是基表全列，输出列由 `projection` 决定：投影非空时取投影后的第一列，投影为空时取 `columns[0]`。JOIN 拒绝文案、限定名主键等值不走索引的路径、以及既有 Scan/DataScan/Filter 臂 SHALL 保持。

#### Scenario: 裸名主键等值的单列 IN 返回匹配行

- **GIVEN** `emp(id INT, name STRING, dept INT)` 与 `dept(id INT, region STRING)`，`dept` 含 `id = 10`
- **WHEN** `SELECT name FROM emp WHERE dept IN (SELECT id FROM dept WHERE id = 10)`
- **THEN** 返回 `dept = 10` 的姓名，不报 `Subquery returns multiple columns`

#### Scenario: 主键等值子查询的输出列不是表的第一列

- **GIVEN** 同上，`emp` 含一行 `dept = 10`
- **WHEN** `SELECT name FROM emp WHERE name IN (SELECT region FROM dept WHERE id = 10)`
- **THEN** 按 `region` 的值匹配 `name`，不按 `dept.id` 匹配

#### Scenario: 二级索引等值子查询按投影后的首列匹配

- **GIVEN** `emp` 的非主键整数列建有二级索引，该列不是表的第一列
- **WHEN** `SELECT name FROM emp WHERE name IN (SELECT region FROM dept WHERE <索引列> = <整数>)`
- **THEN** 按 `SELECT` 清单的第一列匹配，不按基表第一列匹配

#### Scenario: 限定名主键等值与 JOIN 拒绝保持

- **GIVEN** 同上
- **WHEN** `WHERE dept IN (SELECT id FROM dept WHERE dept.id = 10)`；以及既有 `IN (SELECT … JOIN …)`
- **THEN** 限定名形态仍返回匹配行；JOIN 形态仍报 `IN subquery with JOIN is not supported`
