# order-by-alias-sorting Delta — 限定名排序键支持

## MODIFIED Requirements

### Requirement: 输出名优先遮蔽语义

排序键与输出别名、输入列均可匹配时 SHALL 按输出别名列排序（输出名优先，SQLite 对齐）；别名与源列同名（`SELECT name AS name ... ORDER BY name`）SHALL 按输出列排序。仅输入列可匹配的键 SHALL 维持既有按输入列排序语义。两段限定排序键（`t.col` / `alias.col`）SHALL 经限定符归一化（`sql-table-alias-resolution` 域）后按输入列解析并真实排序——原「`ORDER BY t.col` 现状即 ParseError、本能力不改其范围」的边界注记自本 change 起终止；SQL 级 `ORDER BY <表达式>` 维持既有拒绝面。

#### Scenario: 别名遮蔽源列

- **GIVEN** 表 `t(x INT)` 含 (5),(3),(20)（按 INSERT 序）
- **WHEN** `SELECT 10 - x AS x FROM t ORDER BY x`
- **THEN** 按输出列升序输出 `10-x` 值 -10,5,7（对应 x=(20),(5),(3)）；若按源列 x 排序则为 (3),(5),(20)（输出值 7,5,-10），两序可区分（输出名优先）

#### Scenario: 输入列键维持既有解析

- **GIVEN** 表 `t(a INT, b INT)` 含 (1,3),(2,1)
- **WHEN** `SELECT a, b FROM t ORDER BY b`
- **THEN** 按输入列 b 排序（既有非别名键语义不变）

#### Scenario: 限定名排序键真实排序

- **GIVEN** 表 `people(id INT, team_id INT)` 含 (1,20),(2,10)
- **WHEN** `SELECT id FROM people p ORDER BY p.id`
- **THEN** 按 p 的输入列 id 排序输出 1,2（现状：`Parse error: ORDER BY only supports column names`）

### Requirement: 排序键校验与组合面零回归

未知排序键 SHALL 维持计划期点名拒绝（裸键 `ColumnNotFound` / 限定键限定符归一化后不可达按 `sql-table-alias-resolution` 域点名，CLI exit 3）；DISTINCT 查询与聚合查询的别名排序 SHALL 与 `select-distinct` 能力的组合语义一致（DISTINCT 下 ORDER BY 项须在输出列集——别名键天然满足）；基础列排序、聚合投影列名排序、限定名排序等既有可达形态 SHALL 零回归；既有测试零修改通过，**校准白名单**：`tests/scalar_function_test.rs` R5/S5 用例（锁定旧「静默保持输入序」语义）按新语义校准期望——与语料 `sql-scalar-functions` R5 的同步修订互为镜像（历史白名单，本 change 无新增校准面；限定键为纯新增可达形态）。

#### Scenario: 未知键拒绝保持

- **WHEN** `SELECT id FROM t ORDER BY nosuch`
- **THEN** exit 3 点名 `nosuch`（PG 42703），与变更前一致

#### Scenario: DISTINCT 查询别名排序

- **GIVEN** 表 `t(v VARCHAR)` 含 'b','a','b'
- **WHEN** `SELECT DISTINCT v AS val FROM t ORDER BY val`
- **THEN** 去重后按 val 排序输出 'a','b'

#### Scenario: 既有可达排序零回归

- **WHEN** 执行 `SELECT id FROM t ORDER BY name DESC`、`SELECT name, COUNT(*) FROM t GROUP BY name ORDER BY count_star` 等既有形态
- **THEN** 排序结果与本变更前一致
