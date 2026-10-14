# sql-views Specification

## Purpose

约束只读视图（`CREATE VIEW` / `DROP VIEW`）语义：定义（规范化 SQL 文本 + 列名清单）经 catalog kind=View 行持久化、跨重启与跨会话可查；名称解析期经 CTE 通道展开为等价派生表（MS25 DerivedScan 同型，与内联 `FROM (<定义>) AS <视图名>` 逐字节一致，含组合面与错误面）；嵌套展开与循环引用结构性封死（创建期对尚不存在视图的引用即拒）；显式列清单覆盖输出列名；DML 穿透计划期只读拒绝、非基表 DDL 目标点名拒绝；定义变更后 plan cache 全清旧计划不复活；附加库引用创建期点名拒绝（可持久对象不得携带会话限定名）。视图非物化：无数据页、无索引树、无行级痕迹，不进 dump。来源：MS21（change `2026-09-29-ms21-ddl-evolution`，用户 2026-09-29 扩围裁定）。

## Requirements

### Requirement: CREATE VIEW 持久化只读视图

`CREATE VIEW <name> AS <SELECT ...>` SHALL 创建非物化只读视图并将定义（名称、查询文本或结构化定义、可选列名清单）经 catalog 持久化：视图 SHALL 跨重启与跨会话可查；视图列名 SHALL 取显式列清单（`CREATE VIEW v (a, b) AS ...`），缺省取查询输出表头。`SELECT ... FROM <view>` SHALL 在名称解析期将视图展开为等价派生表（MS25 DerivedScan 同型语义）：结果行集与将视图定义内联为 `FROM (<定义查询>) AS <视图名>` 的查询一致；视图内 ORDER BY/LIMIT/聚合/JOIN 按派生表既有语义。视图 SHALL 可引用基表与既有视图（嵌套展开），循环引用 SHALL 计划期点名拒绝。重名创建、定义引用不存在的表或列、`CREATE OR REPLACE`、`MATERIALIZED`、定义引用附加库（`alias.table`）均 SHALL 计划期点名拒绝且零副作用。

#### Scenario: 创建与查询视图

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 含若干行
- **WHEN** `CREATE VIEW big AS SELECT id, v FROM t WHERE v > 100` 后 `SELECT * FROM big`
- **THEN** 返回 `v > 100` 的行，列名 `id, v`；与内联查询 `SELECT id, v FROM (SELECT id, v FROM t WHERE v > 100) AS big` 结果一致

#### Scenario: 显式列名清单

- **WHEN** `CREATE VIEW vv (a, b) AS SELECT id, v FROM t` 后 `SELECT a, b FROM vv`
- **THEN** 输出列名为 `a, b`，行数据为 `id, v`

#### Scenario: 跨重启持久

- **GIVEN** 创建视图后显式 close
- **WHEN** 重新打开数据库后 `SELECT * FROM big`
- **THEN** 视图可查，定义与结果一致

#### Scenario: 嵌套视图展开

- **GIVEN** 已有视图 `big`（基于 `t`）
- **WHEN** `CREATE VIEW big2 AS SELECT a FROM big WHERE a > 10` 后 `SELECT * FROM big2`
- **THEN** 结果与两层内联展开一致

#### Scenario: 循环引用拒绝

- **WHEN** `CREATE VIEW v1 AS SELECT * FROM v2`（`v2` 不存在或经链回指 `v1`）
- **THEN** 计划期点名拒绝（不存在 / 循环引用），零副作用

#### Scenario: 定义错误与重名拒绝

- **WHEN** `CREATE VIEW v AS SELECT * FROM nope` / 对已存在的视图或表名再次 `CREATE VIEW ...`
- **THEN** 均点名拒绝，既有对象零变化

#### Scenario: OR REPLACE / MATERIALIZED / 附加库引用拒绝

- **WHEN** `CREATE OR REPLACE VIEW ...` / `CREATE MATERIALIZED VIEW ...` / 定义含 `SELECT * FROM att.t`（已 ATTACH）
- **THEN** 均计划期点名拒绝（点名能力）

### Requirement: 视图只读与 DML/DDL 拒绝

对视图的 INSERT、UPDATE、DELETE SHALL 计划期点名拒绝（视图只读）；`DROP VIEW <name>` SHALL 删除视图定义（跨重启持久），视图不存在 SHALL 点名拒绝；`DROP TABLE` 对视图名 SHALL 按表不存在拒绝；对视图执行 `ALTER TABLE`/`TRUNCATE`/`CREATE INDEX` SHALL 点名拒绝（目标非基表）。`DROP`/`CREATE VIEW` 不得影响基表数据。

#### Scenario: DML 穿透视图拒绝

- **GIVEN** 已有视图 `big`
- **WHEN** `INSERT INTO big ...` / `UPDATE big SET ...` / `DELETE FROM big ...`
- **THEN** 均计划期点名拒绝（视图只读），基表零变化

#### Scenario: DROP VIEW 删除与不存在拒绝

- **WHEN** `DROP VIEW big` 后 `SELECT * FROM big`
- **THEN** 删除成功；此后查询视图点名拒绝（视图不存在），基表数据不受影响
- **WHEN** 再次 `DROP VIEW big` / `DROP VIEW nope`
- **THEN** 点名拒绝

#### Scenario: 视图非基表目标拒绝

- **GIVEN** 已有视图 `big`
- **WHEN** `TRUNCATE TABLE big` / `CREATE INDEX i ON big(c)` / `ALTER TABLE big ...` / `DROP TABLE big`
- **THEN** 均点名拒绝（目标非基表 / 表不存在）

### Requirement: 视图解析与计划一致性

引用视图的查询 SHALL 使用当前有效定义展开：`DROP VIEW` 后引用 SHALL 点名拒绝（SHALL NOT 复用旧展开结果），重建同名视图后 SHALL 按新定义展开；视图相关 DDL SHALL 沿用既有 DDL plan cache 失效面（proposal 假设 12），SHALL NOT 出现定义变更后旧计划复活。视图与 WHERE/JOIN/聚合/ORDER BY/子查询的组合 SHALL 按派生表既有组合语义零特殊化。

#### Scenario: 定义变更后解析一致

- **GIVEN** 视图 `big`（`v > 100`），同会话内 `DROP VIEW big` 后以不同定义重建（`v > 200`）
- **WHEN** `SELECT * FROM big`
- **THEN** 按新定义返回行集；`DROP VIEW big` 后未重建期间查询点名拒绝

#### Scenario: 组合面零特殊化

- **GIVEN** 视图 `big` 与基表 `u`
- **WHEN** `SELECT * FROM big JOIN u ON ... WHERE ... ORDER BY ...`（含子查询引用视图）
- **THEN** 行为与等价内联派生表查询一致（结果与错误面）

#### Scenario: 既有面零回归

- **WHEN** 运行完整测试套件
- **THEN** 除本 change 新增见证外既有测试零修改通过；`cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate` 通过

**e2e 验证与缺陷清偿 change（T4/ISS07，2026-10-14 收尾）收口**：三种通道均以派生表语义构建——视图定义内的别名列与表达式（计算）列 SHALL 与 FROM 派生表、CTE 通道产出一致的可查询输出列（内层顶层 Projection 路由在视图展开的派生上下文放行）。视图定义含 DISTINCT SHALL 真实去重。既有裸列名视图、显式列清单视图、嵌套视图、DML 只读拒绝面 SHALL 零回归。

#### Scenario: 视图与内联派生表逐字节一致（含别名/表达式列）

- **GIVEN** `CREATE VIEW vw AS SELECT age AS v, CASE … END AS tier FROM people` 与同定义内联派生表、同定义 CTE
- **WHEN** 三通道分别 `SELECT v, tier FROM …`
- **THEN** 输出逐字节一致（失败边界——修复前：CTE ✓ / 派生表 ✗ / 视图 ✗——与「逐字节一致」声明矛盾）

#### Scenario: 视图定义含 DISTINCT 真实去重

- **GIVEN** `CREATE VIEW dv AS SELECT DISTINCT dept FROM people`
- **WHEN** `SELECT * FROM dv`
- **THEN** 去重结果（失败边界——修复前：展开上下文 DISTINCT 被静默跳过）

#### Scenario: 既有视图面零回归

- **GIVEN** 裸列名视图、显式列清单视图、嵌套视图
- **WHEN** 查询与 DML 拒绝
- **THEN** 全部行为与修复前一致
