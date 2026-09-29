# cross-db-namespace-query Specification

## Purpose

约束 ATTACH 会话内两段限定表名的跨库路由与只读边界：`alias.table` 在全部表名消费点先查注册表（命中→附加库、未命中→扁平名回落零回归）；SELECT 全形态跨库组合与三段列引用；附加库 DML/DDL 计划期点名拒绝（一期只读）。来源：MS20 一期（change `2026-09-29-ms20-attach-cross-db`，I059 用户方向；跨文件 DML 属 MS20 二期不在本域）。

## Requirements

### Requirement: 两段限定名路由

形如 `alias.table` 的两段限定表名 SHALL 在全部表名消费点先查 attach 注册表：首段命中已注册附加库 → 解析到该附加库的表；首段未命中 → 回落既有扁平名语义（`object_name_to_table_name` 归一化结果 `"alias.table"` 作为主库 catalog 表名，既有含点表名零回归），找不到仍以 `TableNotFound`（42P01）点名。裸名（单段）SHALL 恒解析到主库（`table-name-resolution` 既有语义零回归）。三段及以上限定名：首段命中别名 SHALL 计划期点名拒绝（无更深层级概念）；首段未命中 SHALL 按既有扁平名语义处理。

#### Scenario: 限定名命中附加库

- **GIVEN** 主库有表 `t`、附加库 `o` 有表 `u`，会话已 `ATTACH ... AS o`
- **WHEN** `SELECT * FROM o.u`
- **THEN** 返回附加库 `u` 的行

#### Scenario: 裸名恒主库

- **GIVEN** 主库与附加库 `o` 各有同名表 `t`（内容不同）
- **WHEN** `SELECT * FROM t`
- **THEN** 返回主库 `t` 的行（既有裸名语义不变）

#### Scenario: 未注册别名回落扁平名

- **GIVEN** 会话未注册别名 `z`，主库无表 `z.t`
- **WHEN** `SELECT * FROM z.t`
- **THEN** 按扁平名 `z.t` 查主库 catalog，找不到 → `TableNotFound`（42P01）点名（不静默按主库表 `t` 处理）

#### Scenario: 含点扁平表名零回归

- **GIVEN** 主库存在字面表名 `foo.bar`（既有合法形态），会话未注册别名 `foo`
- **WHEN** `SELECT * FROM foo.bar`
- **THEN** 返回主库表 `foo.bar` 的行（行为逐字节不变）

#### Scenario: 三段限定名且首段命中别名拒绝

- **GIVEN** 会话已 `ATTACH ... AS o`
- **WHEN** `SELECT * FROM o.b.t`
- **THEN** 计划期点名拒绝（无 schema 层级概念）

### Requirement: 只读跨库查询组合

SELECT 全形态（JOIN、WHERE、子查询、聚合、ORDER BY、DISTINCT、CTE、集合操作）SHALL 可自由组合主库表与一个或多个附加库表，语义与各能力单库形态一致。三段列引用 `alias.table.col` SHALL 在 `alias.table` 命中附加库路由时可用（既有两段 `t.col` 与三段拒绝面在路由未命中时零回归）。含附加库引用的语句 SHALL NOT 进入 plan cache（attach 身份不在 cache key 内）。

#### Scenario: 跨库 JOIN

- **GIVEN** 主库 `t(id, name)`、附加库 `o.u(id, tag)`
- **WHEN** `SELECT t.name, o.u.tag FROM t JOIN o.u ON t.id = o.u.id`
- **THEN** 返回组合结果（含 OUTER JOIN 形态同语义）

#### Scenario: 跨库子查询与聚合

- **WHEN** `SELECT count(*) FROM o.u WHERE id IN (SELECT id FROM t)`
- **THEN** 按单库同形态语义返回

### Requirement: 附加库只读边界

对附加库表的 INSERT/UPDATE/DELETE 与一切 DDL（含 `CREATE TABLE alias.t ...`、`DROP TABLE alias.t`）SHALL 计划期点名拒绝（错误按 MS19 错误面携带能力名），SHALL 零副作用。`CREATE TABLE AS SELECT` 目标表限主库（见 `create-table-as-select`）。

#### Scenario: 附加库 DML 拒绝

- **GIVEN** 会话已 `ATTACH ... AS o`
- **WHEN** `INSERT INTO o.u VALUES (1)` / `UPDATE o.u SET ...` / `DELETE FROM o.u`
- **THEN** 计划期点名拒绝（附加库只读），附加库无任何写入

#### Scenario: 附加库 DDL 拒绝

- **WHEN** `CREATE TABLE o.newt (a INT)` / `DROP TABLE o.u`
- **THEN** 计划期点名拒绝（附加库只读）
