## ADDED Requirements

> 夹具注记：本项目建表未声明主键时以首列为隐式主键，Int 首列的重复值被 `DuplicateKey` 拒绝（`src/executor/create_table.rs`）；涉重复值的 GIVEN 表以显式 `k INT PRIMARY KEY` 锚列承载行（与见证测试夹具一致）。

### Requirement: COUNT(DISTINCT X) 真实去重计数

`COUNT(DISTINCT <列>)` SHALL 输出去除重复且排除 NULL 后的计数。本能力修复既有缺陷——聚合路径当前不读取 `distinct` 量化词、`COUNT(DISTINCT x)` 静默按 `COUNT(x)` 计算返回含重复计数；该静默行为 SHALL 终止。多参数形态 `COUNT(DISTINCT a, b)` SHALL 点名拒绝。

#### Scenario: 去重计数

- **GIVEN** 表 `t(k INT PRIMARY KEY, v INT)` 含 (1,1),(2,1),(3,2),(4,NULL)
- **WHEN** `SELECT count(DISTINCT v) FROM t`
- **THEN** 输出 2（现状 RED：静默输出 3）

#### Scenario: 全 NULL 去重计数为 0

- **GIVEN** 表 `t(v INT)` 含 (NULL),(NULL)
- **WHEN** `SELECT count(DISTINCT v) FROM t`
- **THEN** 输出 0

#### Scenario: 多参数拒绝

- **WHEN** `SELECT count(DISTINCT a, b) FROM t`
- **THEN** 计划期点名拒绝

### Requirement: SUM/AVG/MIN/MAX 的 DISTINCT 量化词（SQLite 对齐）

`SUM(DISTINCT X)` 与 `AVG(DISTINCT X)` SHALL 按去重非 NULL 值计算；`MIN(DISTINCT X)` 与 `MAX(DISTINCT X)` SHALL 接受 DISTINCT 量化词并等价普通 `MIN(X)`/`MAX(X)`（SQLite 对齐）。上述形态与 GROUP BY 组合 SHALL 按分组语义工作。

#### Scenario: SUM(DISTINCT) 去重求和

- **GIVEN** 表 `t(k INT PRIMARY KEY, v INT)` 含 (1,1),(2,1),(3,2)
- **WHEN** `SELECT sum(DISTINCT v) FROM t`
- **THEN** 输出 3

#### Scenario: AVG(DISTINCT) 去重平均

- **GIVEN** 表 `t(k INT PRIMARY KEY, v INT)` 含 (1,1),(2,1),(3,3)
- **WHEN** `SELECT avg(DISTINCT v) FROM t`
- **THEN** 输出 2

#### Scenario: MIN(DISTINCT) 等价普通 MIN

- **GIVEN** 表 `t(k INT PRIMARY KEY, v INT)` 含 (1,3),(2,1),(3,1)
- **WHEN** `SELECT min(DISTINCT v) FROM t`
- **THEN** 输出 1（与 `min(v)` 一致）

#### Scenario: 与 GROUP BY 组合

- **GIVEN** 表 `t(g VARCHAR, v INT)` 含 ('a',1),('a',1),('a',2),('b',5)
- **WHEN** `SELECT g, count(DISTINCT v) FROM t GROUP BY g`
- **THEN** 输出 ('a',2)、('b',1)

### Requirement: group_concat

`group_concat(<列>)` SHALL 按组内非 NULL 值以逗号分隔连接为 String；`group_concat(<列>, <分隔符>)` SHALL 使用字面量分隔符。组内全部为 NULL 或组为空集时 SHALL 返回 NULL（SQLite 对齐）。组内连接顺序 SHALL 为输入行序（实现序文档化，与 SQLite「顺序未指定」兼容）。分隔符为非字面量表达式 SHALL 点名拒绝。与 GROUP BY 组合 SHALL 按分组语义工作。

#### Scenario: 默认逗号分隔

- **GIVEN** 表 `t(v VARCHAR)` 含 'a','b','c'
- **WHEN** `SELECT group_concat(v) FROM t`
- **THEN** 输出 'a,b,c'

#### Scenario: 自定义分隔符

- **GIVEN** 表 `t(v VARCHAR)` 含 'a','b'
- **WHEN** `SELECT group_concat(v, '|') FROM t`
- **THEN** 输出 'a|b'

#### Scenario: NULL 跳过与全 NULL 组

- **GIVEN** 表 `t(g VARCHAR, v VARCHAR)` 含 ('a',NULL),('a','x'),('b',NULL)
- **WHEN** `SELECT g, group_concat(v) FROM t GROUP BY g`
- **THEN** 输出 ('a','x') 与 ('b',NULL)

#### Scenario: 非字面量分隔符拒绝

- **WHEN** `SELECT group_concat(v, upper('|')) FROM t`
- **THEN** 计划期点名拒绝

### Requirement: 既有聚合语义零回归

本能力 SHALL NOT 改变既有行为：既有五聚合函数（COUNT/COUNT(*)/SUM/AVG/MIN/MAX）无 DISTINCT 形态结果逐字节不变；聚合×表达式、GROUP BY 解析序、HAVING、聚合混用拒绝面（`NonAggregatedColumn` 等）零回归；全部既有测试零修改通过。

#### Scenario: 既有聚合零回归

- **WHEN** 执行既有聚合形态（含 `tests/group_by_expr_test.rs`、`tests/expression_e2e_test.rs` 聚合矩阵）
- **THEN** 结果与计划形状与本变更前一致

#### Scenario: 全量回归

- **WHEN** `cargo test -- --test-threads=1`
- **THEN** 0 failed，测试总数只增不减
