## MODIFIED Requirements

### Requirement: UPDATE DELETE 保持已有语法边界

UPDATE/DELETE SHALL 保持现有单目标、主键等值条件、**省略 WHERE 的全表形态**以及 UPDATE 多列赋值与值表达式右值的支持范围。本基线没有 UPDATE/DELETE 一般谓词与子查询执行能力，跨库路由 SHALL NOT 将一般谓词条件或子查询条件/赋值变为可执行计划，也 SHALL NOT 静默忽略子查询后只执行部分条件。附加库目标的无 WHERE 全表形态 SHALL 经既有路由执行，行级语义与主库目标一致（per 行门序、回滚、索引维护），每文件独立提交语义不变。

#### Scenario: 子查询赋值与条件继续拒绝

- **GIVEN** 主库、附加库目标表与子查询源表存在
- **WHEN** 执行 `UPDATE o.t SET v = (SELECT v FROM t) WHERE id = 1` 或 `DELETE FROM o.t WHERE id IN (SELECT id FROM t)`
- **THEN** 按既有不支持值类型或主键等值条件要求拒绝，主库和附加库不变

#### Scenario: 附加库目标全表 DELETE

- **GIVEN** ATTACH 附加库目标表有行
- **WHEN** `DELETE FROM o.t`（无 WHERE）
- **THEN** 经路由全表删除，affected_rows=实际行数，主库不变
