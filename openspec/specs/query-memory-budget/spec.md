# query-memory-budget Specification

## Purpose

每语句查询内存预算与查询执行资源模型：默认 0 不限制；`Database::set_query_memory_limit` 与 `RTSQL_QUERY_MEMORY_BYTES` 双通道配置（非法值打开前 exit 2 零库文件、只影响之后开始的语句）；驻留行数据按固定口径计费（非字符串值 16 字节、字符串 16 加字节长度，同一份值的每个驻留副本分别计费，单行在途数据与最终结果行不计），超限整句失败——消息 `Execution error: query memory limit exceeded (limit N bytes)` + SQLSTATE `53200`，不返回部分行、写语句零副作用；计数不跨语句。配套驻留收敛：等值哈希 JOIN 构建并驻留右输入后按左行逐行探测（不驻留整表左输入），`LIMIT` 直接约束排序时只驻留 `OFFSET + LIMIT` 行。来源：MS32-T02 / I082（change `2026-10-23-ms32-executor-batch-and-query-memory` Iterations 001–003，2026-10-23 收尾）。

## Requirements

### Requirement: 默认不限制

未设置查询内存上限，或上限为 0 时，语句 SHALL 按现有语义执行，不得因驻留字节被拒绝。环境变量未设置时上限为 0。`RTSQL_QUERY_MEMORY_BYTES=0` SHALL 同样表示不限制。

#### Scenario: 未配置时大查询仍成功

- **GIVEN** 打开库时没有设置查询内存上限
- **WHEN** 执行一条会排序或连接较大输入的查询
- **THEN** 结果与设置上限这个能力出现之前一致

### Requirement: 非法配置在打开前拒绝

`RTSQL_QUERY_MEMORY_BYTES` 不是非负整数时，会打开库的 CLI 命令 SHALL 以 exit 2 结束，stderr SHALL 包含该变量名，且不得创建数据库文件。

#### Scenario: 非法字节数

- **GIVEN** 环境变量值为 `nope`
- **WHEN** 执行 `rtsql <db> <sql>`
- **THEN** 退出码为 2，stderr 包含 `RTSQL_QUERY_MEMORY_BYTES`，目标路径上没有新的库文件

### Requirement: 驻留字节按固定口径计费

上限大于 0 时，语句 SHALL 只对同时驻留的行数据计费，单行在途数据和最终返回给调用者的结果行不计。每个非字符串值计 16 字节，每个字符串计 16 加其字节长度。同一份值的每个驻留副本分别计费。哈希键、分组键、去重集合中的值副本都算驻留。流式扫描本身不驻留整表。派生表、`INSERT SELECT` 和 `CTAS` 在写入或继续执行前持有的源行算驻留。语句结束后计数 SHALL 丢弃，下一条语句从 0 重新累计。

#### Scenario: 直接全表扫描不因结果大小被拒绝

- **GIVEN** 上限为 16 字节，表中有多行整数，查询是无 WHERE 的 `SELECT *`
- **WHEN** 执行该查询
- **THEN** 成功返回全部行

#### Scenario: 派生表按源行计费

- **GIVEN** 上限小于子查询全部源行的计费字节，且大于 0
- **WHEN** 执行 `SELECT * FROM (SELECT * FROM t)`
- **THEN** 语句失败，不返回行

#### Scenario: 下一条语句不继承计数

- **GIVEN** 上限足以容纳一次小排序
- **WHEN** 连续执行两次相同的排序查询
- **THEN** 两次都成功

### Requirement: 超限整句失败

计费超过上限时，公共执行入口（直接执行、显式事务内执行、预编译执行）SHALL 返回错误：消息包含 `query memory limit exceeded` 和该上限的十进制字节数，SQLSTATE 为 `53200`。SHALL NOT 返回部分结果行。写语句 SHALL 在产生该语句的写入副作用之前失败。

#### Scenario: 排序超过上限

- **GIVEN** 上限大于 0 且小于 `ORDER BY` 需要驻留的全部输入行
- **WHEN** 经公共入口执行该排序查询
- **THEN** 返回上述错误，没有结果行

#### Scenario: INSERT SELECT 零写入

- **GIVEN** 上限小于 `INSERT INTO dst SELECT * FROM src` 的源行计费
- **WHEN** 执行该语句
- **THEN** 返回上述错误，`dst` 的行数不变

### Requirement: 哈希 JOIN 不驻留整表左输入

等值哈希 JOIN SHALL 在构建并驻留右输入之后按左行探测，不得为了输出而再驻留全部左行。右输入和哈希键仍按驻留计费。

#### Scenario: 小右表加大左表

- **GIVEN** 上限容得下右表及其哈希键，但容不下左右表之和
- **WHEN** 执行等值 INNER JOIN
- **THEN** 语句成功，匹配行与不设上限时相同

#### Scenario: 右表本身超限

- **GIVEN** 上限容不下一个等值 JOIN 的右表
- **WHEN** 执行该 JOIN
- **THEN** 返回 53200，没有结果行

### Requirement: LIMIT 直接约束排序时只驻留有界行

`ORDER BY` 之后、`LIMIT` 之前若没有 `DISTINCT`、聚合或集合运算，排序 SHALL 只驻留 `OFFSET + LIMIT` 行，再按同一排序规则输出并跳过 OFFSET。排序键唯一时，输出 SHALL 与先全量排序再截取相同。`DISTINCT`、聚合或集合运算位于排序与 `LIMIT` 之间时，SHALL 继续全量排序。`LIMIT 0` SHALL 不读取排序输入。

#### Scenario: 唯一键的前缀

- **GIVEN** 表中排序键互不相同
- **WHEN** 执行 `ORDER BY key LIMIT n OFFSET m`
- **THEN** 结果等于全量排序后跳过 m 行再取 n 行

#### Scenario: 有界排序能通过小上限

- **GIVEN** 上限容得下 `LIMIT 1` 的那一行，但容不下整表排序
- **WHEN** 执行 `ORDER BY key LIMIT 1`
- **THEN** 语句成功，结果是排序后的第一行

#### Scenario: DISTINCT 仍全量排序

- **GIVEN** 上限容得下 `LIMIT 1` 的一行，但容不下去重前的全部输入
- **WHEN** 执行 `SELECT DISTINCT ... ORDER BY ... LIMIT 1`
- **THEN** 语句因驻留超限失败
