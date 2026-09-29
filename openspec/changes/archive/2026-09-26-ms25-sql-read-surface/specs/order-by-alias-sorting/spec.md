## ADDED Requirements

### Requirement: ORDER BY 输出别名真实排序

ORDER BY 排序键引用顶层投影项输出别名（表达式项别名与普通列别名）时 SHALL 按该输出列对结果集真实排序（升序默认 / DESC 降序），SHALL NOT 再保持输入序。基座范围：Scan 与 DerivedScan（CTE / FROM 子查询，MS25 Iteration 004 replan T10 扩展）；JOIN 基座的别名可达与排序由 `join-output-shape-resolution` 能力承载。解析序为输出名优先（SQLite 对齐）：键先与顶层投影输出名（`ExprWithAlias` 别名与 Display 列名，大小写不敏感）匹配，命中即按输出列排序；未命中再按输入列解析。本能力升级语料 `error-reporting` R7 与 `sql-scalar-functions` R5 锁定的「别名键静默保持输入序」既有语义（修订随本 change 合并）。

#### Scenario: 表达式项别名排序

- **GIVEN** 表 `t(name VARCHAR)` 按 INSERT 顺序含 'b','a'
- **WHEN** `SELECT upper(name) AS u FROM t ORDER BY u`
- **THEN** 输出 'A','B'（按输出列排序；现状 RED：保持输入序 'B','A'）

#### Scenario: 普通列别名排序（DESC）

- **GIVEN** 表 `t(id INT, name VARCHAR)` 含 (1,'x'),(2,'y')
- **WHEN** `SELECT id, name AS n FROM t ORDER BY n DESC`
- **THEN** 输出 (2,'y'),(1,'x')

#### Scenario: CTE 基座普通列别名排序（T10 扩展）

- **GIVEN** 表 `t(v VARCHAR)` 按 INSERT 顺序含 'b','a'；`WITH c AS (SELECT v FROM t)`
- **WHEN** `WITH c AS (SELECT v FROM t) SELECT v AS val FROM c ORDER BY val`
- **THEN** 表头 `["val"]`，输出 'a','b'（按输出列排序；现状 RED：别名丢失表头 `["v"]` 且保持输入序）

#### Scenario: 多键混合排序

- **GIVEN** 表 `t(a INT, b INT)` 含 (2,1),(1,2),(1,1)
- **WHEN** `SELECT a, b AS second FROM t ORDER BY a, second`
- **THEN** 基础列 a 与别名键混合排序：(1,1),(1,2),(2,1)

### Requirement: 输出名优先遮蔽语义

排序键与输出别名、输入列均可匹配时 SHALL 按输出别名列排序（输出名优先，SQLite 对齐）；别名与源列同名（`SELECT name AS name ... ORDER BY name`）SHALL 按输出列排序。仅输入列可匹配的键 SHALL 维持既有按输入列排序语义。

#### Scenario: 别名遮蔽源列

- **GIVEN** 表 `t(x INT)` 含 (5),(3),(20)（按 INSERT 序）
- **WHEN** `SELECT 10 - x AS x FROM t ORDER BY x`
- **THEN** 按输出列升序输出 `10-x` 值 -10,5,7（对应 x=(20),(5),(3)）；若按源列 x 排序则为 (3),(5),(20)（输出值 7,5,-10），两序可区分（输出名优先；示例经 Plan Review 2026-09-28 校正——原 `x % 2` 示例引用本引擎不支持的 `%` 算子，见证测试 `alias_shadows_source_column` 采用同一非单调形态）

#### Scenario: 输入列键维持既有解析

- **GIVEN** 表 `t(a INT, b INT)` 含 (1,3),(2,1)
- **WHEN** `SELECT a, b FROM t ORDER BY b`
- **THEN** 按输入列 b 排序（既有非别名键语义不变；注：SQL 级 `ORDER BY <表达式>` 与 `ORDER BY t.col` 限定名现状即 `ParseError`「ORDER BY only supports column names」，本能力不改其范围）

### Requirement: 排序键校验与组合面零回归

未知排序键 SHALL 维持 `ColumnNotFound` 计划期点名拒绝（`error-reporting` R7 既有语义，CLI exit 3 / PG 42703）；DISTINCT 查询与聚合查询的别名排序 SHALL 与 `select-distinct` 能力的组合语义一致（DISTINCT 下 ORDER BY 项须在输出列集——别名键天然满足）；基础列排序、聚合投影列名排序等既有可达形态 SHALL 零回归；既有测试零修改通过，**校准白名单**：`tests/scalar_function_test.rs` R5/S5 用例（锁定旧「静默保持输入序」语义）按新语义校准期望——与语料 `sql-scalar-functions` R5 的同步修订互为镜像。

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
