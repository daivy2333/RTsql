## MODIFIED Requirements

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
