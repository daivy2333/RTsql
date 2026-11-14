# result-consumption Specification

## Purpose

规定四个公共入口如何按批交回查询行，以及 `execute_sql` 四个整段入口仍一次交回完整结果。

## Requirements

### Requirement: 公共行游标分批交回

库 SHALL 提供四个公共入口打开行游标：`open_rows`、`open_rows_with_params`、`open_rows_in_tx`、`open_rows_in_tx_with_params`。游标 SHALL 暴露计划已经解析出的列名和声明类型，并在任何一次拉取之前就可读。声明类型不可得的列 SHALL 保持未知，游标 SHALL NOT 为了推导类型而收集全部行。

`next_batch(max)` SHALL 至多交回 `max` 行，行序和行值与同一条语句经对应整段入口执行时一致。`max == 0` SHALL 交回空批且不推进。结果耗尽后再次拉取 SHALL 交回空批。

#### Scenario: 分批拼接等于整段结果

- **GIVEN** 表中有至少三行已知整数
- **WHEN** `open_rows` 执行对应的 `SELECT`，并以 2 为批量拉到空批
- **THEN** 各批拼接后的行序和行值与 `execute_sql` 相同
- **AND** 列名和声明类型在第一次拉取之前已经可读，且拉取不改变它们

#### Scenario: 空结果仍有列元数据

- **GIVEN** 表存在，查询条件排除全部行
- **WHEN** 打开游标并拉取一次
- **THEN** 列名和声明类型可读
- **AND** 该次拉取为空批

#### Scenario: 带参与事务内可见

- **GIVEN** 一条带 `?` 的查询，以及一个已插入未提交行的显式事务
- **WHEN** 分别用 `open_rows_with_params` 和 `open_rows_in_tx` 拉取
- **THEN** 带参结果与 `execute_with_params` 相同
- **AND** 事务内游标读到该未提交行
- **AND** 丢弃游标不提交也不回滚该事务

#### Scenario: 批量大小为零不推进

- **GIVEN** 表中至少一行
- **WHEN** 先 `next_batch(0)` 再 `next_batch(1)`
- **THEN** 第一次为空批
- **AND** 第二次交回的是第一行

### Requirement: 非行语句在副作用之前拒绝

游标只接受今天会进入查询臂并交回行的语句。INSERT、UPDATE、DELETE、UPSERT、CREATE、DROP、ALTER、TRUNCATE、CTAS 和 INSERT SELECT SHALL 在打开时拒绝，错误文本为 `streaming consumption requires a row-producing statement`，SQLSTATE 为空，且 SHALL NOT 产生该语句的副作用。

只读会话上的写计划 SHALL 先按既有只读规则拒绝，SQLSTATE 为 `25006`，不使用上面这条文本。解析、计划、参数个数错误 SHALL 与对应的整段入口同源。

#### Scenario: 可写会话中的插入不落行

- **GIVEN** 表中行数已知，会话可写
- **WHEN** 对一条 INSERT 调用 `open_rows`
- **THEN** 返回上述固定错误文本，SQLSTATE 为空
- **AND** 表中行数不变

#### Scenario: 只读会话仍是只读拒绝

- **GIVEN** 只读打开的库
- **WHEN** 对 CREATE TABLE 调用 `open_rows`
- **THEN** 拒绝的 SQLSTATE 为 `25006`
- **AND** 没有新建表

### Requirement: 结果交接处的背压

一次 `next_batch(max)` SHALL NOT 向根执行器索取超过 `max` 行，也 SHALL NOT 预取下一批。调用方停止拉取或丢弃游标后，根执行器 SHALL NOT 再产出行。排序、哈希聚合、哈希连接构建和 DISTINCT 的既有驻留不在本要求中取消。

无 WHERE、无排序、无聚合、无连接的投影扫描中，后面行的求值失败 SHALL NOT 出现在尚未索取到该行的批次里。失败批次的错误文本和 SQLSTATE SHALL 与同一条语句的 `execute_sql` 错误相同。已经交回的批次保持已交回。此后的拉取 SHALL 返回同一个错误，不再产出行。

#### Scenario: 未索取的行不会提前失败

- **GIVEN** 新建表按插入顺序有三行短字符串，前两行是十进制整数文本，第三行不是；查询是 `SELECT CAST(s AS INTEGER) FROM t`，没有 WHERE、排序、聚合或连接
- **WHEN** 每次拉取 1 行
- **THEN** 前两批各自是对应整数
- **AND** 第三批失败，错误文本和 SQLSTATE 与这条语句的 `execute_sql` 错误相同
- **AND** 再拉取一次仍是这个错误

#### Scenario: 提前丢弃不改变表

- **GIVEN** 表中有多行
- **WHEN** 只拉取一批后丢弃游标，再经 `execute_sql` 查询全表
- **THEN** 表内容与打开游标前一致
- **AND** 整段查询返回全部行

### Requirement: 整段入口与诊断口径保持

`execute_sql`、`execute_with_params`、`execute_in_tx` 和 `execute_in_tx_with_params` SHALL 继续一次交回完整 `SqlOutcome`。查询失败时 SHALL NOT 交回部分行。游标路径 SHALL NOT 调用 `note_first_delivered_row` 或 `finish_delivered_rows`。既有语句的首行等待、交回行数、吞吐和阶段归属 SHALL 保持原定义。查询内存上限对算子驻留的计费和「最终交回调用方的结果行不计」SHALL 保持。

#### Scenario: 整段失败不交回部分行

- **GIVEN** 上一要求中的三行 CAST 查询
- **WHEN** 经 `execute_sql` 执行
- **THEN** 返回错误，没有结果行

#### Scenario: 直接全表扫描仍不受结果大小拒绝

- **GIVEN** 查询内存上限为 16 字节，表中有多行整数，查询是无 WHERE 的 `SELECT *`
- **WHEN** 经 `execute_sql` 和 `open_rows` 分别执行
- **THEN** 两条路径都成功交回全部行
