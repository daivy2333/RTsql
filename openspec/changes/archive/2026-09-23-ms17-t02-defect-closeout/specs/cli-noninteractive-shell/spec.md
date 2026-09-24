# cli-noninteractive-shell Specification（delta）

## ADDED Requirements

### Requirement: 标量子查询输出列的表头形状

SELECT 清单含标量子查询项（`SELECT col, (SELECT …) AS alias FROM t` 形态）时，CLI 输出表头 SHALL 在该标量列的输出位置（`result_column_index`）携带其列名（别名或表达式名），SHALL NOT 返回未插入标量列名的输入计划列名。json 输出 `columns` 数组长度 SHALL 与 `rows` 每行值数一致；table/csv/tsv 表头 SHALL 与值列对齐。标量列名来源与既有执行器插入语义一致（`SubqueryEvalExecutor` 在 `result_column_index` 插入标量值，索引越界时追加）。不含标量子查询项的既有形态（含 I034 已锁定的扫描投影表头）SHALL 逐字节保持。

#### Scenario: 表头在标量位置携带列名

- **GIVEN** 表 `emp(id INT, name VARCHAR, salary INT)` 与 `dept(rid INT, region VARCHAR)` 存在关联数据（标量子查询可返回单行）
- **WHEN** `SELECT id, (SELECT region FROM dept WHERE dept.rid = emp.id) AS region FROM emp`，输出格式 json
- **THEN** `columns` 为 `["id", "region", "name", "salary"]`（别名在 index 1，输入计划列名保持原序列于其后），`columns` 长度等于每行 `rows` 值数，标量值位于行内 index 1

#### Scenario: 标量子查询位于中间位置时 table 表头对齐

- **GIVEN** 同上数据
- **WHEN** `SELECT id, name, (SELECT region FROM dept WHERE dept.rid = emp.id) AS region FROM emp`，输出格式 table
- **THEN** 表头为 `id | name | region | salary` 四列，与四值行对齐，标量列名位于标量值所在位置（index 2）；change 前表头为 `[id, name, salary]` 三列，对四值行错位且无 `region` 列名

#### Scenario: 非标量子查询形态零回归

- **GIVEN** 既有投影表头用例（I034 锁定的裸 DataScan/IndexScan 表头断言与 cli_test 全部既有用例）
- **WHEN** 全量测试运行
- **THEN** 全部零修改通过；不含标量子查询的查询表头行为逐字节不变
