# sql-parameter-binding Specification

## Purpose

SQL 文本位置式占位符 `?` 的类型化参数执行面：parse → 占位符编号（AST 单遍按 SQL 文本序）→ 个数校验 → 计划面编译（表达式位编译为 `ParameterExpression`、DML 值位登记槽位）→ 注入/物化 → 执行；参数个数与类型错误点名、NULL 三值语义、TEXT 真值求值；不可达位（DML 定位键位、标识位、DDL 内占位符）计划期点名拒绝且零副作用；以及可复用预编译语句面——prepare 一次、多次带参执行，计划自持复用，schema 变化（DDL 八类 + CTAS）后按版本戳透明重计划，显式事务内执行，同一条预编译语句的并发执行经闸门串行化（并发机制定稿见归档 change 的 design D7）。来源：MS31-T02 / I089（change `2026-10-21-ms31-typed-api-param-binding` Iterations 001-002，2026-10-23 收尾）。

## Requirements

### Requirement: SQL 占位符与类型化参数执行

SQL 文本 SHALL 支持位置式占位符 `?`；执行入口接受同序类型化参数（原生 `Value`），占位符按参数真实值求值（不以字符串替换实现绑定）。参数个数与语句占位符数不符 SHALL 在执行前点名错误；参数类型与消费点类型门语义与字面量一致；NULL 参数按三值语义。

#### Scenario: 占位符等值过滤

- **GIVEN** 表含已知行、语句 `SELECT … WHERE a = ?` 与参数 `[Int(1)]`
- **WHEN** 带参执行
- **THEN** 返回与字面量 `a = 1` 相同的结果

#### Scenario: INSERT/UPDATE 值位绑定

- **GIVEN** `INSERT INTO t VALUES (?, ?)` 或 `UPDATE t SET b = ? WHERE a = 1`
- **WHEN** 带参执行
- **THEN** 写入与读回值和等价字面量语句一致（类型门、NOT NULL、键列门、唯一与次级索引维护照常）

#### Scenario: DML 定位键位占位符点名拒绝

- **GIVEN** `UPDATE t SET b = 1 WHERE a = ?` 或 `DELETE FROM t WHERE a = ?`
- **WHEN** 计划
- **THEN** 计划期点名拒绝（定位键为计划期物化、参数执行期才可得——Phase 2 调查定稿的不可达位），零副作用

#### Scenario: 参数个数不符

- **GIVEN** 语句含 2 个占位符、提供 1 个参数
- **WHEN** 执行
- **THEN** 执行前点名错误（期望与实际个数），零副作用

#### Scenario: 参数类型不符

- **GIVEN** TEXT 列语句 `INSERT INTO t(b) VALUES (?)`、参数 `[Int(1)]`
- **WHEN** 执行
- **THEN** 拒绝语义与字面量类型门一致（`ColumnTypeMismatch` 同源），零副作用

#### Scenario: NULL 参数三值

- **GIVEN** `WHERE a = ?` 参数 `[Null]`，及 `WHERE b IS NULL` 形态
- **WHEN** 带参执行
- **THEN** 等值比较按 Unknown 折叠不返回行；IS NULL 判定按参数真实 NULL 生效

#### Scenario: 不可达位置点名拒绝

- **GIVEN** 占位符出现在标识位（表名/列名）或 DDL 语句中
- **WHEN** 计划
- **THEN** 计划期点名拒绝（携带能力名），不静默忽略

### Requirement: 可复用预编译语句

SHALL 提供 prepare 一次、多次带参执行的预编译语句；同一语句多次执行的结果与每次直接执行一致；schema 变化后的失效契约为可观察的定稿行为（默认透明重计划）；预编译语句 SHALL 与 `Database` 共享引用（`&self`）并发安全。

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
