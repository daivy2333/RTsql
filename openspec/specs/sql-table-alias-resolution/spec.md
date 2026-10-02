# sql-table-alias-resolution Specification

## Purpose
约束 FROM 表别名的注册与限定名解析：别名 → 表映射对 SELECT / JOIN（含自连接多别名）/ WHERE / GROUP BY / HAVING / 聚合参数 / ON / ORDER BY 的列引用解析统一生效；限定列引用的限定符经单点归一化（别名命中 → 真实表键，未命中 → 原样）后沿既有通道解析；两段附加 FROM 项无显式别名时尾段作为隐式关联名注册（短名限定列引用与全限定同达）；未知限定符计划期以 `TableNotFound`（42P01）点名，终止「静默丢弃限定符」与「别名被当表名」两类误导错误；三段限定仅 attach 复合键可达，本地形态统一点名拒绝；别名不改变输出表头、`OutputColumn` 真实表名语义或计划形状。来源：change `2026-09-30-catalog-durability-and-cli-surface` Iteration 001（P1 表别名限定引用家族失效收口）；附加 FROM 项隐式关联名来源 MS20-T02（change `2026-10-02-ms20-cross-db-dml` Iteration 002）。

## Requirements

### Requirement: FROM 表别名注册

FROM 子句中的表别名（显式 `AS` 或隐式）SHALL 注册为该表在本语句作用域内的限定符：别名 → 表的映射对 SELECT / JOIN（含自连接多别名）/ WHERE / GROUP BY / HAVING / 聚合参数 / ON / ORDER BY 的列引用解析统一生效。两段附加 FROM 项（`alias.table`）在未写显式别名时，其尾段 SHALL 作为该项的隐式关联名注册（标准 SQL 隐式别名语义，SQLite 同型），使短名限定列引用与全限定、显式别名形态同达；主库表与字面含点表名 SHALL NOT 误登记隐式关联名。CTE 与视图基座的有效名（引用别名或定义名）沿既有行为保持。别名（显式或隐式关联名）SHALL NOT 改变输出的表头、`OutputColumn` 的真实表名语义或计划形状（路由、投影索引按真实形状解析）。

#### Scenario: 自连接两别名全子句面可用

- **GIVEN** 表 `people(id, name, team_id)`
- **WHEN** `SELECT a.name, b.name FROM people a JOIN people b ON a.id = b.id WHERE a.team_id = 10 ORDER BY b.id`
- **THEN** 查询成功，两列分别为左右别名对应表的 name 值，排序按右侧别名的 id

#### Scenario: JOIN 三表混用别名与真实名

- **GIVEN** 三表 JOIN（部分臂用别名、部分用真实表名）
- **WHEN** SELECT/ON/WHERE 混合使用别名限定列与真实表名限定列
- **THEN** 全部解析正确，同名裸名歧义仍按 `AmbiguousColumn` 点名

#### Scenario: 别名不改变表头与计划形状

- **GIVEN** `FROM people a JOIN team t ON a.team_id = t.id`
- **WHEN** `SELECT a.name` 执行并 `EXPLAIN`
- **THEN** 表头仍为 `name`；计划形状与真实表名书写等价（同路由同投影）

#### Scenario: 附加 FROM 项短名限定列可达

- **GIVEN** 主库表 `t(id)` 与附加库 `o` 的表 `u(id, tag)`
- **WHEN** `ATTACH 'o' AS o; SELECT u.tag FROM t JOIN o.u ON t.id = u.id`
- **THEN** 查询成功，`u.tag` 解析到附加 `o.u`（JOIN 投影/ON/WHERE/GROUP BY/ORDER BY 各消费点同达）；`SELECT u.tag FROM o.u`（单表基座）同达

#### Scenario: 显式别名优先且字面含点不误登记

- **GIVEN** 主库表 `t`、附加 `o.u`，主库另有字面含点表 `"o.u"`
- **WHEN** `SELECT x.tag FROM t JOIN o.u x`（显式别名）与对字面含点表的短名形态 `SELECT u.tag FROM "o.u"`
- **THEN** 显式别名形态与既有行为零变化；字面含点表名不注册 `u` 关联名，短名形态沿既有未知限定符面点名

### Requirement: 限定列解析认别名（全子句面）

限定列引用 `q.c` 的解析 SHALL 先将限定符 `q` 归一化（别名映射命中 → 真实表键；未命中 → 原样），再沿既有通道解析（单表注册表 / JOIN 组合布局 / attach 复合键 / 相关外层引用）。SHALL 覆盖：JOIN 投影（含纯列清单严格校验与过滤）、ON 腿（Hash 等值臂与 NLJ 布局臂）、WHERE、GROUP BY 与聚合参数（`aggregate_arg_indices` 通道）、ORDER BY（见限定名排序 Requirement）。归一化后仍不可达的限定符 SHALL 以携带该限定符的错误点名（`TableNotFound`，42P01 语义 = FROM 中无该关系），SHALL NOT 误报为列缺失或静默求值。

#### Scenario: JOIN 投影别名限定列

- **GIVEN** `FROM people a JOIN people b ON a.id = b.id`
- **WHEN** `SELECT a.name FROM …`（历史缺陷形态）
- **THEN** 返回左别名侧 name 列，无 `Column not found` 误报

#### Scenario: ON 与 WHERE 别名限定不再被当作表名

- **GIVEN** `FROM people p JOIN team t ON t.id = p.team_id`
- **WHEN** 执行该查询（历史缺陷形态：`Parse error: Table 't' does not exist` / `Table not found: t`）
- **THEN** 成功返回；`SELECT * FROM … WHERE p.team_id = 10` 同样成功

#### Scenario: 聚合与 GROUP BY 别名限定（JOIN 基座）

- **GIVEN** 自连接两别名
- **WHEN** `SELECT a.team_id, COUNT(*) FROM … GROUP BY a.team_id`（历史缺陷形态：`Table 'a' does not exist`）
- **THEN** 分组与聚合参数按左别名侧列正确解析

#### Scenario: 未知限定符点名

- **GIVEN** `FROM people`
- **WHEN** `SELECT x.name FROM people`（x 未声明）
- **THEN** 计划期点名拒绝，错误携带 `x`（不再静默丢弃限定符求值）

### Requirement: 单表限定列与 JOIN 统一

单表扫描基座的限定列解析 SHALL 与 JOIN 基座同规则（经限定符归一化），SHALL NOT 保留「静默丢弃限定符」的解析路径。两段限定的输出列名/表头职责不变（取末段）；已声明别名的既有用法经别名映射正常工作。

#### Scenario: 单表别名投影保持可用

- **GIVEN** `FROM people p`
- **WHEN** `SELECT p.name FROM people p WHERE p.id = 1`（历史缺陷：投影可用但 WHERE 报 `Table 'p' not found`）
- **THEN** 投影与 WHERE 均按别名解析成功，表头 `name`

#### Scenario: 单表未知限定符拒绝

- **GIVEN** `FROM people`（无别名）
- **WHEN** `SELECT p.name FROM people`
- **THEN** 计划期点名拒绝（与 JOIN 形态同语义）

### Requirement: 限定形态边界

三段限定列 `a.b.c` SHALL 仅当 `a.b` 作为整体命中表注册键（attach 复合键 `alias.table`）时可达；本地库形态（如 `db.table.column`）SHALL 计划期点名拒绝，不再出现「单表静默成功 / JOIN 失败」的分裂行为。表引用语境的两段名 attach-first 既有语义不变。attach 复合键通道的既有测试形态零回归。

#### Scenario: 本地三段限定统一拒绝

- **GIVEN** 本地库表 `people`
- **WHEN** `SELECT demo.people.name FROM people`（历史：单表静默成功、JOIN 失败）
- **THEN** 计划期点名拒绝，不再分裂

#### Scenario: ATTACH 三段限定保持可用

- **GIVEN** `ATTACH 'other' AS o` 且 `o` 中有表 `dept_map`
- **WHEN** `SELECT o.dept_map.dept FROM people JOIN o.dept_map ON people.team_id = o.dept_map.team_id`
- **THEN** 行为与修复前一致（既有通道零回归）

### Requirement: 限定名排序与既有语义零回归

ORDER BY SHALL 接受两段限定键：限定符经归一化后按可见列解析排序，输出别名优先语义、既有裸键排序、未知键拒绝语义 SHALL 保持不变；`order-by-alias-sorting` 中「`ORDER BY t.col` 维持 ParseError」的边界注记终止。全部既有读面（DISTINCT/集合操作/CTE/视图/派生基座/聚合形状/外连接）SHALL 零回归。

#### Scenario: ORDER BY 限定键真实排序

- **GIVEN** `FROM people p JOIN team t ON p.team_id = t.id`
- **WHEN** `SELECT p.name FROM … ORDER BY t.id`（历史：`ORDER BY only supports column names`）
- **THEN** 按右侧表 id 真实排序；`ORDER BY p.id`（单表）同理

#### Scenario: 输出别名优先与未知键拒绝不变

- **GIVEN** `SELECT id AS k FROM people p`
- **WHEN** `ORDER BY k` / `ORDER BY p.zzz`
- **THEN** 别名键按既有语义排序；未知限定键计划期点名拒绝
