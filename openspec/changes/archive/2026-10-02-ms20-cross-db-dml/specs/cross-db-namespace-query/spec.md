## MODIFIED Requirements

### Requirement: 附加库只读边界

附加库写入边界 SHALL 按语句种类及会话打开模式判定：可写会话的 INSERT/UPSERT/REPLACE、UPDATE、DELETE SHALL 按 `cross-db-dml` 路由独立提交；只读会话 SHALL 按 `readonly-open-mode` 拒绝写入。一切附加库 DDL（含 CREATE/DROP TABLE、ALTER、TRUNCATE、INDEX 与 VIEW）SHALL 继续计划期点名拒绝且零副作用。CREATE TABLE AS SELECT 目标表仍限主库。名称解析和跨库 SELECT 组合 SHALL 保持既有语义。

#### Scenario: 可写会话允许附加库 DML

- **GIVEN** 可写会话已 ATTACH 附加库 o
- **WHEN** 对 o.u 执行已支持的 INSERT/UPDATE/DELETE
- **THEN** 按 cross-db-dml 执行于附加库，不误写主库同名表

#### Scenario: 附加库 DML 拒绝

- **GIVEN** 会话以只读模式打开并已注册附加库 o，或主库显式事务处于活跃状态
- **WHEN** 对 o.u 执行 INSERT/UPDATE/DELETE
- **THEN** 只读会话按 25006 拒绝；可写的显式事务按 0A000 附加目标限制拒绝；附加库不变

#### Scenario: 附加库 DDL 拒绝

- **GIVEN** 会话已 ATTACH 附加库 o
- **WHEN** 执行 CREATE TABLE o.newt 或 DROP TABLE o.u（ALTER/TRUNCATE/INDEX/VIEW 同边界）
- **THEN** 计划期点名拒绝，附加库 schema 与数据不变

#### Scenario: CTAS 附加目标继续拒绝

- **WHEN** 执行 CREATE TABLE o.newt AS SELECT ...
- **THEN** 沿用 create-table-as-select 的主库目标限制，附加库不建表
