## ADDED Requirements

### Requirement: WHERE 标量子查询比较

谓词比较腿（`= != < > <= >=`，任一侧）中的标量子查询 SHALL 支持：非相关子查询每语句求值一次；相关子查询逐行注入外层参数并按参数值语句级缓存（复用既有相关机制）。含标量子查询的谓词 SHALL NOT 下推进扫描节点（由 Filter 型计划节点承载）。子查询结果恰一行取其首列值参与比较；零行 SHALL 折叠为 NULL 并按三值语义求值（比较 Unknown → 行不匹配）；多行 SHALL 点名拒绝（`SubqueryReturnsMultipleRow` 既有错误面）。JOIN 基座 WHERE 内的标量子查询 SHALL 维持计划期点名拒绝（与 IN/EXISTS 同边界）。含标量子查询的谓词 SHALL NOT 触发既有索引等值路由（值非计划期字面量）。

#### Scenario: 非相关标量比较

- **GIVEN** emp 表含多行
- **WHEN** `SELECT name FROM emp WHERE salary = (SELECT MAX(salary) FROM emp)`；`>` 变体
- **THEN** 返回最高薪行（失败边界——修复前：`Unsupported expression type: EXPRESSION` exit 3）

#### Scenario: 相关标量比较

- **GIVEN** emp(dept_id, salary)
- **WHEN** `SELECT name FROM emp e WHERE salary = (SELECT MAX(salary) FROM emp e2 WHERE e2.dept_id = e.dept_id)`
- **THEN** 逐行按部门最高薪匹配（同部门重复最高薪全部返回）

#### Scenario: 空子查询三值语义

- **GIVEN** 任意表
- **WHEN** `SELECT … WHERE v = (SELECT v FROM t WHERE 1 = 0)`
- **THEN** 零行（NULL 比较为 Unknown，行不匹配）

#### Scenario: 多行标量点名拒绝

- **GIVEN** 子查询返回多行
- **WHEN** `SELECT … WHERE v = (SELECT v FROM t)`
- **THEN** `SubqueryReturnsMultipleRow` 点名拒绝，零副作用

#### Scenario: 拒绝边界保持

- **GIVEN** JOIN 基座
- **WHEN** `… JOIN … WHERE e.salary = (SELECT …)`；`WHERE (SELECT …)` 裸子查询谓词
- **THEN** 计划期点名拒绝（与既有 WHERE CLAUSE ON JOIN / PREDICATE 边界一致）

### Requirement: SELECT 投影位标量子查询输出形状

SELECT 清单项中的标量子查询 SHALL 与其余纯列清单项共同构成输出形状：投影裁剪 SHALL 作用于输入行（未请求的基表列 SHALL NOT 泄漏），标量值 SHALL 按其在 SELECT 清单中的位次插入，表头 SHALL 与行逐位对齐（`__subquery` 或别名命名既有语义保持）。DISTINCT×子查询、混合聚合×子查询、表达式项×子查询的既有拒绝面 SHALL 零变化。

#### Scenario: 投影位形状与请求一致

- **GIVEN** 表 `q6(id, name, dept_id, salary)`
- **WHEN** `SELECT name, (SELECT MAX(salary) FROM q6) FROM q6`
- **THEN** 表头 `["name","__subquery"]`、每行 `[name值, 标量值]`（失败边界——修复前：表头 `["id","__subquery","name","dept_id","salary"]` 全表列泄漏且位次错乱）
