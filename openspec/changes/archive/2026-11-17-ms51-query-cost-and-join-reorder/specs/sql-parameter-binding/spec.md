## MODIFIED Requirements

### Requirement: 可复用预编译语句

SHALL 提供 prepare 一次、多次带参执行的预编译语句；同一语句多次执行的结果与每次直接执行一致；schema 变化后的失效契约为可观察的定稿行为（默认透明重计划）；预编译语句 SHALL 与 `Database` 共享引用（`&self`）并发安全。

成功的 `ANALYZE` SHALL 清空计划缓存，并把与 DDL 相同的 `schema_version` 加一。此后预编译语句再次执行 SHALL 透明重计划，结果与当时的直接执行一致。`ANALYZE` 的这一次加一 SHALL NOT 丢弃其他表仍有效的统计。DDL 与 CTAS 已经通过 `bump_schema_version` 做的那一次递增，SHALL 同时使已收集的表统计不再被计划使用。

#### Scenario: 一次 prepare 多次执行

- **GIVEN** 预编译 `INSERT INTO t VALUES (?, ?)`
- **WHEN** 以不同参数连续执行多次
- **THEN** 各次写入与读回正确，计划不经每次执行重建（复用语义）

#### Scenario: schema 变化后失效契约

- **GIVEN** 预编译语句引用的表经 ALTER/DROP/建删索引后再次执行
- **WHEN** 执行
- **THEN** 透明重计划，结果与直接执行一致（schema 版本戳与 plan cache 失效同源递增，重计划完整重跑 parse 与计划）

#### Scenario: 显式事务内执行

- **GIVEN** 显式事务中执行参数化 DML
- **WHEN** 提交或回滚
- **THEN** 事务语义与既有 `execute_in_tx` 一致

#### Scenario: 并发共享安全

- **GIVEN** 同一预编译语句经多任务并发带参执行
- **WHEN** 并发执行
- **THEN** 各次执行结果正确，不因并发交错产生错误结果

#### Scenario: 统计替换后重计划

- **GIVEN** 一条可重排的参数化查询已经预编译，随后相关基表成功执行 `ANALYZE`
- **WHEN** 再执行该预编译语句，并直接执行同一文本和同一参数
- **THEN** 两次结果行相同
- **AND** `schema_version` 比 `ANALYZE` 之前大
