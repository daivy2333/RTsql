## Purpose

规定本次变更如何用一次测量决定类型化批量写入入口的去留，以及入口交付时与逐行插入一致的写入、拒绝和整批回滚行为。

## ADDED Requirements

### Requirement: 入口去留由一次测量决定

本变更 SHALL 按下列口径测量一次逐行插入耗时占端到端写入耗时的比例，并把两条耗时和「建入口」或「不建入口」写入变更产物。

口径：

- 表为 `CREATE TABLE bench (id INT PRIMARY KEY, name STRING, value INT)`。
- 建表完成后再打开会话诊断。诊断只采集性能统计，`summary_limit` 至少为 1000。
- 随后逐条执行 1000 条 `INSERT INTO bench VALUES ({i}, 'user_{i}', {i * 10})`，`i` 从 0 到 999。每条经 `Database::execute_sql`。
- 测量进程使用 release profile（`opt-level = 3`，`lto = true`），与 `cargo bench` 继承的 release 配置一致。只跑这一遍，不重复采样。
- 端到端写入耗时是这 1000 条 INSERT 诊断摘要的语句总耗时之和。语句总耗时是根阶段包含耗时。
- 逐行插入耗时是这 1000 条摘要中执行阶段独占耗时之和。执行阶段独占耗时等于执行阶段包含耗时减去其提交子阶段的包含耗时。
- 比例达到 10% 的判定是 `逐行插入耗时的纳秒数 * 10 >= 端到端写入耗时的纳秒数`。端到端写入耗时为 0 时，本次测量无效。

比例达到 10% 时，变更完成态 SHALL 提供 `Database::insert_batch`，并满足「批量入口与逐行插入一致」。比例低于 10% 时，变更完成态 SHALL NOT 新增批量写入公共方法。两种完成态下，经 `Database::execute_sql` 与 `Database::execute_in_tx` 发出的 SQL `INSERT` SHALL 保持变更前的结果、错误文本和 SQLSTATE。

无效测量 SHALL NOT 被当成低于 10%，也 SHALL NOT 据此交付入口。

#### Scenario: 测量达到门槛后存在入口

- **GIVEN** 按本要求测得的比例达到 10%
- **WHEN** 本变更完成
- **THEN** `Database::insert_batch` 存在，并满足「批量入口与逐行插入一致」
- **AND** 同一库上原有的 SQL `INSERT` 仍返回变更前的结果、错误文本和 SQLSTATE

#### Scenario: 测量未达门槛则不新增入口

- **GIVEN** 按本要求测得的比例低于 10%
- **WHEN** 本变更完成
- **THEN** `Database` 上不存在新的批量写入方法
- **AND** 同一库上的 SQL `INSERT` 仍返回变更前的结果、错误文本和 SQLSTATE

#### Scenario: 测量无效时不交付入口

- **GIVEN** 1000 条 INSERT 的诊断摘要没有全部留下，或缺少解析、规划、执行、提交阶段，或端到端写入耗时为 0
- **WHEN** 本变更核对测量
- **THEN** 测量记为无效
- **AND** 不交付批量写入入口

### Requirement: 批量入口与逐行插入一致

本要求只在 `Database::insert_batch` 已交付时生效。未交付时，下列场景不适用。

`insert_batch` 接受表名和一批行。每一行是按该表列顺序排列的 `Value` 序列。成功时返回 `SqlOutcome::AffectedRows`。方法 SHALL 使用与隐式多行 `INSERT` 相同的一次事务：全部行成功才提交，任一行失败则整批回滚。SHALL NOT 在同一调用里提交一部分行并返回另一部分的失败。

已交付的入口 SHALL 让下列结果与等价的 SQL 写入相同：成功后的受影响行数和随后 `SELECT` 读到的行；类型不符、NOT NULL、主键冲突、UNIQUE 冲突、非法日期时的错误文本、SQLSTATE，以及失败后的表内容。等价 SQL 是对同一张表、同一批值的多行 `INSERT INTO ... VALUES (...), (...)`。

表名 SHALL 按 catalog 与视图注册表中的存储名精确查找。视图名 SHALL 以与 SQL `INSERT` 写入该视图相同的错误文本和 SQLSTATE 拒绝。未知表 SHALL 以与 SQL `INSERT` 写入该表相同的错误文本和 SQLSTATE 拒绝。任一行的值个数与表列数不同 SHALL 以与 SQL `INSERT` 值个数不符相同的错误文本和 SQLSTATE 拒绝。上述拒绝以及只读会话拒绝都发生在本批任何行写入之前，已有表内容不变。只读会话上对已存在基表的非空批量 SHALL 以与只读会话执行 `INSERT` 相同的错误文本和 SQLSTATE 拒绝。

零行调用在表名解析为已存在的基表后 SHALL 返回受影响行数 0，不开启事务，不写入。调用方已经 `begin` 的显式事务 SHALL 仍由调用方结束。`insert_batch` 成功提交的行 SHALL NOT 随该显式事务的回滚消失。

调用 future 在写入过程中被丢弃时，本批已写入的键 SHALL 在下一次隐式写触发回收之后可以被再次插入。本批之前已提交的行仍在。

import 与 CSV SHALL NOT 在本变更中改道到该入口。

#### Scenario: 多行批量与多行 INSERT 读到相同结果

- **GIVEN** 入口已交付，基表已存在，两批输入的值相同且都会成功
- **WHEN** 一批用 `insert_batch` 写入，另一批在另一张同结构的空表上用多行 `INSERT` 写入
- **THEN** 两边的受影响行数相同
- **AND** 随后的 `SELECT` 行相同

#### Scenario: 后一行冲突时整批不可见

- **GIVEN** 入口已交付，批量的第一行可以写入，第二行与主键或 UNIQUE 冲突
- **WHEN** 调用 `insert_batch`
- **THEN** 返回的错误文本和 SQLSTATE 与等价多行 `INSERT` 相同
- **AND** 本批第一行和第二行都不可见
- **AND** 本批之前已提交的行仍在

#### Scenario: 类型与空约束在写入前按逐行规则拒绝

- **GIVEN** 入口已交付，某一行的值类型与列声明不符，或违反 NOT NULL，或日期文本不能按该列解析
- **WHEN** 调用 `insert_batch`
- **THEN** 返回的错误文本和 SQLSTATE 与等价多行 `INSERT` 相同
- **AND** 本批没有任何行可见

#### Scenario: 行宽、未知表和视图在写入前拒绝

- **GIVEN** 入口已交付
- **WHEN** 表名是未知表、表名是视图，或某一行的值个数与列数不同
- **THEN** 返回的错误文本和 SQLSTATE 与对同一表名发出的 SQL `INSERT` 相同
- **AND** 没有新行写入

#### Scenario: 只读会话拒绝非空批量

- **GIVEN** 入口已交付，会话以只读方式打开，目标基表已存在
- **WHEN** 调用非空的 `insert_batch`
- **THEN** 返回的错误文本和 SQLSTATE 与只读会话执行 `INSERT` 相同
- **AND** 表内容不变

#### Scenario: 空批不写

- **GIVEN** 入口已交付，表名是已存在的基表
- **WHEN** 调用 `insert_batch` 且行数为 0
- **THEN** 返回受影响行数 0
- **AND** 表内容不变
- **AND** 随后用 SQL `INSERT` 写入本可使用的键能够成功

#### Scenario: 不并入调用方的显式事务

- **GIVEN** 入口已交付，调用方已开始一个尚未结束的显式事务
- **WHEN** `insert_batch` 成功写入一行，然后调用方回滚该显式事务
- **THEN** 该行仍然可读
- **AND** 显式事务的回滚只撤销该事务自己写入的行

#### Scenario: 取消后本批的键可以再次插入

- **GIVEN** 入口已交付，索引写在结构变更之后、根页发布之前暂停
- **WHEN** 两行 `insert_batch` 在第二行到达该暂停点时被丢弃，随后另一条隐式写触发回收
- **THEN** 两行的键都可以再次插入
- **AND** 本批之前已提交的行仍在
