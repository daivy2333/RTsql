# query-cost-and-join-order Specification

## Purpose
让一次打开的数据库记录基表在当前快照下的行数和每列不同值个数，并据此估计左深内连接的代价。只有估计严格更省时才更换叶序，输出列序和行的多重集合保持与书写顺序相同。

## Requirements

### Requirement: 会话内基表统计

系统 SHALL 接受 `ANALYZE TABLE <表>`，也 SHALL 接受在表名之后只增加 `COMPUTE STATISTICS` 的同一语句。两者对同一快照 SHALL 交回相同的行。统计对象 SHALL 是主库基表。可见行集合 SHALL 与同一快照上的 `SELECT * FROM <表>` 相同。

结果 SHALL 是行集。列名依次为 `column`、`row_count`、`null_count`、`ndv`。每个结果列对应基表的一列，顺序与 `SELECT *` 的表头相同。`row_count` 在每一行上都是可见行数。`null_count` 是该列的 NULL 个数。`ndv` 是该列非 NULL 值按 `Value` 的 `==` 计的不同值个数。`Float(-0.0)` 与 `Float(0.0)` SHALL 算作一个不同值。按规范形相等的 DECIMAL SHALL 算作一个不同值。NULL SHALL NOT 增加 `ndv`。

成功后 SHALL 把这组数字保存在当前 `Database` 会话中，供随后的计划读取。克隆共享的 `Database` SHALL 看见同一份统计。统计 SHALL NOT 写入数据页、目录、WAL 或检查点文件。关闭后再打开 SHALL 看不到这次统计。`:memory:` 与只读打开 SHALL 可以统计。只读统计 SHALL NOT 创建库文件。

空表 SHALL 仍为每一列交回一行，三个计数都是 0，并且 SHALL 保存这组数字。

INSERT、UPDATE、DELETE SHALL NOT 使统计失效，也 SHALL NOT 改写已保存的数字。之后的计划可以继续使用统计当时的数字。结果行仍按执行时的可见行计算。

#### Scenario: 可见行与三个计数

- **GIVEN** 基表 `t(id INT, name STRING)` 在当前快照有两行 `(1, 'a')` 与 `(2, NULL)`
- **WHEN** 执行 `ANALYZE TABLE t`
- **THEN** 结果两行，顺序与 `SELECT *` 的表头相同
- **AND** 两行的 `row_count` 都是 2
- **AND** `id` 的 `null_count` 是 0、`ndv` 是 2
- **AND** `name` 的 `null_count` 是 1、`ndv` 是 1

#### Scenario: COMPUTE STATISTICS 同义

- **GIVEN** 上一场景的表和行
- **WHEN** 执行 `ANALYZE TABLE t COMPUTE STATISTICS`
- **THEN** 结果行与 `ANALYZE TABLE t` 相同

#### Scenario: 负零只算一个不同值

- **GIVEN** 一列里只有 `Float(-0.0)` 与 `Float(0.0)` 两个非 NULL 值
- **WHEN** 计算该列的 `ndv`
- **THEN** `ndv` 是 1

#### Scenario: 空表也保存零计数

- **GIVEN** 基表存在且 `SELECT *` 没有行
- **WHEN** 执行 `ANALYZE TABLE t`
- **THEN** 每一列都有一行结果，三个计数都是 0
- **AND** 随后的计划把这张表视为已有统计

#### Scenario: 不写入库文件

- **GIVEN** 文件库已经记下主文件与 WAL 的长度
- **WHEN** 成功执行 `ANALYZE TABLE t`
- **THEN** 两个长度都不变
- **AND** 随后的 `SELECT * FROM t` 仍返回统计前的行

#### Scenario: 重新打开后统计消失

- **GIVEN** `ANALYZE TABLE` 已成功
- **WHEN** 关闭数据库，再按默认方式打开，并对原来那条可重排查询执行 `EXPLAIN`
- **THEN** 计划文本没有 `JoinOrder` 行
- **AND** 左深叶序与从未统计时相同

#### Scenario: 只读会话可以统计

- **GIVEN** 数据库以只读方式打开，基表已有可见行
- **WHEN** 执行 `ANALYZE TABLE t`
- **THEN** 交回的计数与同一快照的可见行一致
- **AND** 不创建新的库文件

#### Scenario: 插入不使统计失效

- **GIVEN** 三张基表已统计，一条可重排查询因此选用了非书写叶序
- **WHEN** 向其中一张表插入一行，再执行该查询
- **THEN** 结果包含这一新行
- **AND** 行的多重集合与同一数据上未重排的执行相同

### Requirement: 失败的统计不留下数字

下列情况 SHALL 拒绝，并且 SHALL NOT 替换该表已有统计，SHALL NOT 推进计划版本戳：

- 表不存在：`Plan error: Table not found: <表名>`，SQLSTATE `42P01`。
- 名字是视图：`Plan error: ANALYZE requires a base table: <表名>`，SQLSTATE `0A000`。
- 名字解析到已附加的表：`Plan error: ANALYZE does not support attached tables: <表名>`，SQLSTATE `0A000`。
- 出现 `NOSCAN`、`PARTITION`、`FOR COLUMNS` 或 `CACHE METADATA`：`Plan error: ANALYZE does not support <子句>`，SQLSTATE `0A000`。子句名分别为 `NOSCAN`、`PARTITION`、`FOR COLUMNS`、`CACHE METADATA`。
- 用户事务已经开始：`ANALYZE is not supported inside a transaction`，SQLSTATE `0A000`。这条消息不另加 `Plan error:` 或 `Execution error:` 前缀。拒绝发生在扫描之前。

查询内存上限大于 0，且为保存不同值而保留的值超过该上限时，SHALL 整句失败。消息 SHALL 为 `Execution error: query memory limit exceeded (limit N bytes)`，其中 `N` 是该上限的十进制字节数，SQLSTATE SHALL 为 `53200`。失败 SHALL NOT 替换统计，SHALL NOT 推进版本戳，SHALL NOT 交回部分计数行。上限为 0 时 SHALL NOT 因该预算失败。

扫描开始时记下的计划版本戳，到保存统计时已经变化的，SHALL 不保存，不为此次失败再推进版本戳。消息 SHALL 为 `ANALYZE aborted because the schema changed`，SQLSTATE `58000`，不另加前缀。

#### Scenario: 缺表

- **WHEN** 对不存在的表执行 `ANALYZE TABLE missing`
- **THEN** 错误文本为 `Plan error: Table not found: missing`
- **AND** SQLSTATE 为 `42P01`
- **AND** 计划版本戳不变

#### Scenario: 视图

- **GIVEN** 已有视图 `v`
- **WHEN** 执行 `ANALYZE TABLE v`
- **THEN** 错误文本为 `Plan error: ANALYZE requires a base table: v`
- **AND** SQLSTATE 为 `0A000`

#### Scenario: 附加表

- **GIVEN** 别名 `o` 已附加，其中有基表 `t`
- **WHEN** 执行 `ANALYZE TABLE o.t`
- **THEN** 错误文本为 `Plan error: ANALYZE does not support attached tables: o.t`
- **AND** SQLSTATE 为 `0A000`
- **AND** 主库与附加库的统计都不变

#### Scenario: NOSCAN 被拒绝

- **WHEN** 执行 `ANALYZE TABLE t NOSCAN`
- **THEN** 错误文本为 `Plan error: ANALYZE does not support NOSCAN`
- **AND** SQLSTATE 为 `0A000`
- **AND** 不读取该表的行来替换统计

#### Scenario: 显式事务

- **GIVEN** 用户事务已经 `begin` 且尚未提交或回滚
- **WHEN** 在该事务中执行 `ANALYZE TABLE t`
- **THEN** 错误文本为 `ANALYZE is not supported inside a transaction`
- **AND** SQLSTATE 为 `0A000`
- **AND** 计划版本戳不变

#### Scenario: 超过查询内存上限

- **GIVEN** 查询内存上限为 1 字节，基表有一个非 NULL 整数
- **WHEN** 执行 `ANALYZE TABLE t`
- **THEN** 错误文本为 `Execution error: query memory limit exceeded (limit 1 bytes)`
- **AND** SQLSTATE 为 `53200`
- **AND** 该表原有统计不变，计划版本戳不变

#### Scenario: 保存前模式已经变化

- **GIVEN** 扫描开始后、保存统计前，计划版本戳已经因 DDL 增加
- **WHEN** 统计尝试保存
- **THEN** 错误文本为 `ANALYZE aborted because the schema changed`
- **AND** SQLSTATE 为 `58000`
- **AND** 不写入这组新数字

### Requirement: 统计替换使已有计划失效

成功保存一张表的统计之后，SHALL 清空计划缓存，并把 `schema_version` 加一。加一 SHALL NOT 丢弃同一会话里其他表仍然有效的统计。随后的预编译语句 SHALL 重新计划，结果与当时的直接执行一致。

`bump_schema_version` 每次递增时，已保存的统计 SHALL 不再被计划使用。这包括既有 DDL 与 CTAS 已经在调用的那一次递增。统计列名与计划时该表的列名不一致时，该表 SHALL 视为没有可用统计。

没有可用统计时，计划 SHALL 保持书写叶序，EXPLAIN 的节点行 SHALL 与统计功能引入前的对应语句逐字节一致，并且 SHALL NOT 出现 `JoinOrder` 行。

#### Scenario: 成功统计清空计划缓存

- **GIVEN** 一条不含占位符的查询已经放入计划缓存，并记下当时的 `schema_version`
- **WHEN** 对一张基表成功执行 `ANALYZE TABLE`
- **THEN** 计划缓存为空
- **AND** `schema_version` 比统计前大 1

#### Scenario: 预编译语句随后与直接执行一致

- **GIVEN** 一条可重排查询已经预编译，然后三张基表都成功统计
- **WHEN** 执行该预编译语句，并在同一会话直接执行同一文本
- **THEN** 两次结果行相同
- **AND** 两次计划都使用统计之后的叶序

#### Scenario: DDL 之后统计不再参与计划

- **GIVEN** 三张基表已统计，可重排查询会输出 `JoinOrder estimated`
- **WHEN** 成功执行 `CREATE INDEX`，再 `EXPLAIN` 该查询
- **THEN** 计划文本没有 `JoinOrder` 行
- **AND** 第一条扫描是书写顺序的左表

### Requirement: 左深连接的代价

代价函数的输入是叶、等值边和书写叶序。每张叶有声明限定符、基表名、行数，以及每列的不同值个数。每条边连接两个限定符上的列。

一条左深叶序合法，当且仅当从第二张叶开始，每一张新叶都至少有一条边连到已经进入的叶。没有这样的边时，该顺序 SHALL NOT 被选中，也 SHALL NOT 因此产生笛卡尔积。

两张输入的输出基数，是左基数乘右基数，再按本步新用上的每一条边做一次整数除法。每一条边的除数是两端不同值个数的较大者，小于 1 时按 1。除法向零截断。乘法或加法溢出 `u128` 时，该候选顺序没有代价。

一步的代价是左基数乘 1，加上右基数乘 2，加上输出基数。第一步的两张叶都是基表扫描，并且既有「整数主键基表扫描走排序合并」规则会选中这一对时，这一步的代价改为左基数乘 2，加上右基数乘 2，加上输出基数。其后每一步的左输入都是中间结果，代价用哈希的 1 与 2。

基表不超过 8 张时，SHALL 在全部合法叶序里取代价最小的顺序。书写顺序的代价等于最小代价时，SHALL 保持书写顺序，并且决定仍是「已用估计」。有多个严格更小且代价相同的顺序时，SHALL 取声明限定符字典序最小的那个。决定为「已用估计」时，返回的叶序可以等于书写顺序。

基表超过 8 张，或书写顺序自己的代价溢出时，决定 SHALL 是「不足以重排」，并保持书写顺序。其他候选溢出时只丢掉该候选。计划器 SHALL 调用这一函数，不得另写一套表数上限或溢出规则。

#### Scenario: 星型例子选择 fact 在左

- **GIVEN** `d1` 与 `d2` 的行数都是 2，`id` 的不同值个数都是 2。`fact` 的行数是 100，列 `d1` 与 `d2` 的不同值个数都是 2。边是 `d1.id = fact.d1` 与 `fact.d2 = d2.id`。第一步不走排序合并
- **WHEN** 计算书写顺序 `d1, fact, d2` 与其余合法顺序的代价
- **THEN** 书写顺序的代价是 506
- **AND** `fact, d1, d2` 与 `fact, d2, d1` 的代价都是 408
- **AND** 选中的叶序是 `fact, d1, d2`

#### Scenario: 排序合并的第一步更贵时改走哈希顺序

- **GIVEN** `a`、`b`、`c` 的行数都是 10。边 `a` 与 `b` 的主键等值不同值个数为 10，边 `b` 与 `c` 的非主键等值不同值个数为 10。只有两个基表主键等值的第一步使用排序合并代价
- **WHEN** 比较书写顺序 `a, b, c`
- **THEN** 该顺序的代价是 90
- **AND** `b, c, a` 的代价是 80
- **AND** 选中的叶序是 `b, c, a`

#### Scenario: 与书写顺序代价相同则保持书写顺序

- **GIVEN** 所有合法叶序的代价都等于书写顺序
- **WHEN** 计算决定
- **THEN** 叶序等于书写顺序
- **AND** 决定是「已用估计」

#### Scenario: 超过八张表不枚举

- **GIVEN** 一条链上有 9 张基表，统计齐全，边把它们连成一条链
- **WHEN** 计算决定
- **THEN** 决定是「不足以重排」
- **AND** 不返回另一条叶序

#### Scenario: 书写顺序溢出则不足以重排

- **GIVEN** 三张表的行数都是 18446744073709551615，连接键的不同值个数都是 1
- **WHEN** 计算书写顺序的第二步乘法
- **THEN** 该乘法溢出
- **AND** 决定是「不足以重排」

### Requirement: 估计只更换合格查询的叶序

计划器 SHALL 先按书写顺序完成现有的连接装配。该装配返回的错误 SHALL 保持原样。只有装配成功，并且下面每一条都成立时，才允许用代价函数的叶序替换这棵左深树：

- 当前构造不是子查询、不是派生表、不是 CTE 定义，也不是视图展开。
- `FROM` 只有一项，其中的连接步至少两步，所以至少三张叶。
- 每一张叶都是主库基表扫描。
- 每一步都是内连接，并且 ON 是纯列等值。
- 语句有 `ORDER BY`。
- 语句没有 `LIMIT`、`OFFSET`、`FETCH` 和 `LIMIT BY`。
- 链上每一张基表都有可用统计。

替换后的树 SHALL 仍是左深树，每一步的右子节点是一张基表扫描。SHALL NOT 再按行数交换任何一个连接节点的左右输入。选中叶序之后，既有 `integer_pk_base_scan_join` SHALL 原样决定该节点是排序合并还是哈希。

最终连接节点交给 WHERE、ORDER BY 和投影的列序、声明限定符序，SHALL 与书写顺序装配的结果相同。物理子树可以不同。输出列从实际持有该列的子行读取。

无 ORDER BY、只有两表、含外连接或非等值、含派生表、视图、CTE 或附加表，以及带 LIMIT 或 OFFSET 的语句，SHALL 保持书写叶序。这些语句的 EXPLAIN 节点行 SHALL NOT 因本需求改变。

合格并且每一张表都有可用统计时，EXPLAIN 在首行 `EXPLAIN` 之后 SHALL 有一行，缩进两个空格：

- 决定是「已用估计」时，该行是 `JoinOrder estimated`。
- 决定是「不足以重排」时，该行是 `JoinOrder written`，叶序保持书写顺序。

没有可用统计时 SHALL NOT 输出这两行之一。

对同一数据，重排后的结果行多重集合 SHALL 与书写叶序的执行相同。有 ORDER BY 时，结果序列 SHALL 满足该 ORDER BY。SHALL NOT 为了保住并列键的相对次序而把排序改成稳定排序。

#### Scenario: 星型查询换到 fact 为左叶

- **GIVEN** `d1(id INT)` 有 1 和 2，`d2(id INT)` 有 1 和 2，`fact(id INT, d1 INT, d2 INT)` 有 100 行。`fact.id` 为 1 到 100，`fact.d1` 与 `fact.d2` 都是 `(id - 1) % 2 + 1`。三张表都已统计
- **WHEN** 执行 `SELECT fact.id, d1.id, d2.id FROM d1 JOIN fact ON d1.id = fact.d1 JOIN d2 ON fact.d2 = d2.id ORDER BY fact.id, d1.id, d2.id`
- **THEN** EXPLAIN 含有一行 `JoinOrder estimated`
- **AND** 第一条扫描行是 `fact`
- **AND** 结果行与同一数据上从未统计时的该语句相同

#### Scenario: 输出列序仍按书写 FROM

- **GIVEN** 上一场景的三张表都已统计
- **WHEN** 执行 `SELECT *` 加上与上一场景相同的 FROM、ON 和 `ORDER BY fact.id`
- **THEN** 结果表头的列序与从未统计时相同
- **AND** 按 `fact.id` 排好的行与从未统计时相同

#### Scenario: WHERE 仍按书写列序过滤

- **GIVEN** 上一场景的三张表都已统计
- **WHEN** 在该连接上增加 `WHERE fact.id = 1`，并 `ORDER BY fact.id`
- **THEN** 结果只有 `fact.id = 1` 的那一行
- **AND** 该行与从未统计时相同

#### Scenario: 聚合分组与未重排相同

- **GIVEN** 上一场景的三张表都已统计
- **WHEN** 执行 `SELECT fact.d1, COUNT(*) FROM d1 JOIN fact ON d1.id = fact.d1 JOIN d2 ON fact.d2 = d2.id GROUP BY fact.d1 ORDER BY fact.d1`
- **THEN** 分组和计数与从未统计时相同

#### Scenario: 没有 ORDER BY 时不重排

- **GIVEN** 三张表都已统计
- **WHEN** EXPLAIN 与执行去掉 ORDER BY 的同一内连接
- **THEN** 计划文本没有 `JoinOrder` 行
- **AND** 第一条扫描行仍是 `d1`
- **AND** 结果行序与从未统计时相同

#### Scenario: 两表连接不交换左右

- **GIVEN** `d1` 与 `fact` 都已统计
- **WHEN** EXPLAIN `SELECT fact.id FROM d1 JOIN fact ON d1.id = fact.d1 ORDER BY fact.id`
- **THEN** 计划文本没有 `JoinOrder` 行
- **AND** 第一条扫描行仍是 `d1`

#### Scenario: 外连接不重排

- **GIVEN** 三张表都已统计
- **WHEN** EXPLAIN `d1 LEFT JOIN fact ON d1.id = fact.d1 JOIN d2 ON fact.d2 = d2.id`，并带 `ORDER BY fact.id`
- **THEN** 计划含有 `NestedLoopJoin`
- **AND** 计划文本没有 `JoinOrder` 行
- **AND** 结果行与从未统计时相同

#### Scenario: LIMIT 不重排

- **GIVEN** 三张表都已统计
- **WHEN** 在星型内连接上使用 `ORDER BY fact.id LIMIT 1`
- **THEN** 计划文本没有 `JoinOrder` 行
- **AND** 返回的那一行与从未统计时相同

#### Scenario: 估计与书写顺序同样便宜

- **GIVEN** 合格查询的每一张表都有可用统计，并且最小代价等于书写顺序
- **WHEN** EXPLAIN 该查询
- **THEN** 有一行 `JoinOrder estimated`
- **AND** 第一条扫描行仍是书写顺序的左表

#### Scenario: 超过八张表时标明未重排

- **GIVEN** 一条合格链上有 9 张已统计的基表
- **WHEN** EXPLAIN 该查询
- **THEN** 有一行 `JoinOrder written`
- **AND** 叶序是书写顺序
