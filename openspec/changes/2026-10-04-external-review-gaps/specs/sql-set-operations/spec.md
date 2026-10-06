## ADDED Requirements

### Requirement: FROM 派生表内的集合操作

`FROM (SELECT … UNION [ALL] / INTERSECT / EXCEPT SELECT …)` SHALL 可作为派生表源：派生表列名取左臂输出列名，外层投影、WHERE、聚合、ORDER BY 叠加 SHALL 照常生效；执行语义与顶层集合操作同源（UNION 去重 / UNION ALL 保序保重 / INTERSECT / EXCEPT，NULL 相等语义既有）。表达式子查询（IN / EXISTS / 标量比较）内的集合操作 SHALL 维持计划期点名拒绝（范围收口）；括号化操作数（`(SELECT …) UNION (SELECT …)`）的既有拒绝 SHALL 零变化。

#### Scenario: 派生表内 UNION 可达

- **GIVEN** dept(id) 与 emp(dept_id) 表
- **WHEN** `SELECT COUNT(*) FROM (SELECT id FROM dept UNION SELECT dept_id FROM emp)`
- **THEN** 可执行并返回去重后计数（失败边界——修复前：`Set operations are not supported inside subqueries` exit 3）

#### Scenario: 外层子句叠加

- **GIVEN** 同上
- **WHEN** 外层 WHERE 过滤、列投影、ORDER BY 叠加于 UNION 派生表
- **THEN** 照常生效，列名取左臂

#### Scenario: 表达式子查询内维持拒绝

- **GIVEN** 同上
- **WHEN** `WHERE id IN (SELECT a FROM t UNION SELECT b FROM t)`
- **THEN** 点名拒绝（范围收口，修复前后一致）
