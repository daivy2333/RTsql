# sql-aggregate-extensions Specification

## Purpose
约束聚合扩展语义：`COUNT/SUM/AVG(DISTINCT …)` 真实去重计算（MIN/MAX DISTINCT 与普通形态等价）、`group_concat` 的分隔符与 NULL 跳过语义、聚合调用嵌套值表达式（SELECT 清单项/HAVING 条件/聚合查询 ORDER BY 键，常量项混排合法）与聚合输出引用按注册输出正确定位（显示名重名去歧义），多参数与不支持形态点名拒绝，既有聚合面零回归。来源：MS25（change `2026-09-26-ms25-sql-read-surface`，SQLite 语义对齐）；change `2026-10-03-practical-core-gaps`（聚合表达式嵌入 + 重名解析）。

## Requirements

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

### Requirement: 聚合表达式嵌入值表达式

聚合查询中，聚合函数调用 SHALL 可嵌套于值表达式内部：SELECT 清单项值表达式（如 `SELECT MAX(id) + 1`、`SELECT COALESCE(MAX(id), 0) + 1`）、HAVING 条件表达式（如 `HAVING MAX(id) + 1 > 2`）、以及聚合查询的 ORDER BY 键（如 GROUP BY 查询 `ORDER BY MAX(v)`）SHALL 按标准 SQL 语义求值——每个唯一聚合调用在分桶阶段恰计算一次，表达式基于聚合结果求值。不含聚合且不含非聚合列引用的常量项 SHALL 与聚合项混排合法（`SELECT MAX(id), 1`；INSERT SELECT 源同语义）。SELECT 清单表达式项的表头 SHALL 沿既有表达式项命名语义（显式别名优先，否则表达式 Display 文本）。聚合调用参数面沿既有语义：`DISTINCT` 量化词（`SUM(DISTINCT v) + 1`）SHALL 保持去重语义，聚合参数表达式（`SUM(a + b)`）SHALL 保持可达。

以下拒绝面 SHALL 维持计划期点名拒绝：WHERE 与 JOIN ON 内的聚合调用（聚合在行过滤前无定义）、聚合嵌套聚合（`MAX(MIN(x))`）、无 GROUP BY 时 SELECT 清单中的非聚合列引用（既有 Non-aggregated 语义保持，消息点名列名）。

聚合输出引用 SHALL 按注册输出正确定位（Iteration 001 Plan Review 补强）：聚合项显示名重名（重复别名，或别名与其他聚合调用的 `result_column_name` 相同）时，条件包装的聚合项定位、SELECT 嵌入表达式内的聚合引用与 HAVING 内的聚合引用 SHALL 各自解析到正确的注册聚合输出列；同一聚合调用多次出现仍共享一个输出列（去重语义保持）。

#### Scenario: SELECT 清单嵌套聚合

- **GIVEN** 表 `t(id INTEGER PRIMARY KEY, v INTEGER)` 含三行 (1,10),(2,20),(3,30)
- **WHEN** `SELECT MAX(id) + 1 FROM t`
- **THEN** 单行结果 `4`（失败边界——修复前：`Unsupported expression type: MAX`）；`SELECT COALESCE(MAX(id), 0) + 1 FROM t` 同型返回 `4`

#### Scenario: HAVING 条件嵌套聚合

- **GIVEN** 同上表
- **WHEN** `SELECT v, MAX(id) FROM t GROUP BY v HAVING MAX(id) + 1 > 2`
- **THEN** 组内 max+1 > 2 的组返回，过滤语义正确（失败边界——修复前：`Unsupported expression type: EXPRESSION`）

#### Scenario: ORDER BY 聚合表达式键

- **GIVEN** 同上表多组数据
- **WHEN** `SELECT v, MAX(id) FROM t GROUP BY v ORDER BY MAX(id) + 1`
- **THEN** 输出按聚合表达式值排序生效

#### Scenario: 常量项与聚合混排

- **GIVEN** 同上表；目标表 `t2` 两列
- **WHEN** `SELECT MAX(id), 1 FROM t`；`INSERT INTO t2 SELECT MAX(id), 1 FROM t`
- **THEN** 常量项合法返回/写入（失败边界——修复前：`Non-aggregated column '1' must appear in GROUP BY`）

#### Scenario: DISTINCT 聚合在表达式内保持去重

- **GIVEN** 表含重复 v 值
- **WHEN** `SELECT SUM(DISTINCT v) + 1 FROM t`
- **THEN** 去重求和后加 1（DISTINCT 语义不因嵌套失效）

#### Scenario: 拒绝面保持

- **GIVEN** 同上表
- **WHEN** `SELECT * FROM t WHERE MAX(id) > 1`；JOIN ON 内聚合；`SELECT MAX(MIN(id)) FROM t`；`SELECT id, MAX(v) FROM t`
- **THEN** 四形态全部计划期点名拒绝（WHERE/ON 聚合、嵌套聚合、非聚合列引用各携既有文案通道）

#### Scenario: 既有聚合输出形状零回归

- **GIVEN** 既有聚合测试面（直出形状、混合投影包装、表头命名、别名排序）
- **WHEN** 既有聚合套件回归
- **THEN** 全部零回归

#### Scenario: 聚合显示名重名时解析正确

- **GIVEN** 表 `t(g, a, b)` 多组数据（g=1 组 max(a)=20、max(b)=200；g=2 组 max(a)=30、max(b)=300）
- **WHEN** ① `SELECT MAX(a) AS m, g, MAX(b) AS m FROM t GROUP BY g`（重复别名，组键非首项走包装路径）；② `SELECT MAX(a) AS max_b, g, MAX(b) FROM t GROUP BY g`（别名撞 `result_column_name`）；③ 同②形态加 `HAVING MAX(b) > 150`；④ 同②形态加嵌入项 `MAX(b)+1`
- **THEN** 四形态各自解析到正确的注册聚合输出：①第三列为 max(b)（200/300）；②第三列为 max(b)；③按 max(b) 过滤（两组均保留）；④嵌入项为 max(b)+1（201/301）。同一调用多次出现（如 `SELECT MAX(id), MAX(id)+1`）仍共享一个输出列（失败边界——修复前：四形态均按显示名首匹配解析到第一个同名注册项，静默取错值/错过滤）

### Requirement: 既有聚合语义零回归

本能力 SHALL NOT 改变既有行为：既有五聚合函数（COUNT/COUNT(*)/SUM/AVG/MIN/MAX）无 DISTINCT 形态结果逐字节不变；聚合×表达式、GROUP BY 解析序、HAVING、聚合混用拒绝面（`NonAggregatedColumn` 等）零回归；全部既有测试零修改通过。

#### Scenario: 既有聚合零回归

- **WHEN** 执行既有聚合形态（含 `tests/group_by_expr_test.rs`、`tests/expression_e2e_test.rs` 聚合矩阵）
- **THEN** 结果与计划形状与本变更前一致

#### Scenario: 全量回归

- **WHEN** `cargo test -- --test-threads=1`
- **THEN** 0 failed，测试总数只增不减
