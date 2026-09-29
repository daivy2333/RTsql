## ADDED Requirements

> 夹具注记：本项目建表未声明主键时以首列为隐式主键，Int 首列的重复值被 `DuplicateKey` 拒绝（`src/executor/create_table.rs`）；涉重复值的 GIVEN 表以显式 `k INT PRIMARY KEY` 锚列承载行（与见证测试夹具一致）。

### Requirement: SELECT DISTINCT 输出去重

`SELECT DISTINCT <投影项> FROM ...` SHALL 按输出行去重后返回：完全相同的输出行只保留一行。去重 SHALL 作用于投影求值后的输出值（含表达式项与派生列）。NULL SHALL 视为彼此相等参与去重（一组全 NULL 折叠为一行）。本能力修复既有缺陷——`query.distinct` 量化词当前被 planner 静默忽略、重复行照常返回；该静默行为 SHALL 终止。

#### Scenario: 单列去重

- **GIVEN** 表 `t(v VARCHAR)` 含 'a','a','b'
- **WHEN** `SELECT DISTINCT v FROM t`
- **THEN** 输出 'a','b' 各一行（现状 RED：静默返回 3 行）

#### Scenario: 多列元组去重

- **GIVEN** 表 `t(k INT PRIMARY KEY, a INT, b VARCHAR)` 含 (1,1,'x'),(2,1,'x'),(3,1,'y')
- **WHEN** `SELECT DISTINCT a, b FROM t`
- **THEN** 输出 (1,'x')、(1,'y') 两行

#### Scenario: NULL 相等折叠

- **GIVEN** 表 `t(v INT)` 含 (NULL),(NULL),(1)
- **WHEN** `SELECT DISTINCT v FROM t`
- **THEN** 输出 NULL 与 1 各一行

#### Scenario: 表达式投影按求值后值去重

- **GIVEN** 表 `t(name VARCHAR)` 含 'ab','AB','cd'
- **WHEN** `SELECT DISTINCT upper(name) FROM t`
- **THEN** 输出 'AB'、'CD' 各一行（按求值后值折叠）

### Requirement: DISTINCT 与排序、聚合组合

DISTINCT 查询的 ORDER BY 项 SHALL 出现在输出列集（输出名或表达式文本）；引用非输出项的排序 SHALL 点名拒绝。DISTINCT 与聚合输出组合（如 `SELECT DISTINCT count(*) ...`）SHALL 按输出行去重语义工作。DISTINCT × 输出别名排序的组合语义由 `order-by-alias-sorting` 能力承载。

> 已知边界（Iteration 001 实施确认）：聚合执行器的输出行恒为「分组键 ++ 聚合值」，不同分组的输出行天然互异——聚合输出上不存在可折叠的重复行；该组合的去重见证为「DISTINCT 行流 = 非 DISTINCT 行流的去重结果（行形状不变）」。

#### Scenario: DISTINCT + ORDER BY 输出项

- **GIVEN** 表 `t(k INT PRIMARY KEY, v VARCHAR)` 含 (1,'b'),(2,'a'),(3,'a')
- **WHEN** `SELECT DISTINCT v FROM t ORDER BY v`
- **THEN** 输出排序去重后的 'a','b'

#### Scenario: DISTINCT + ORDER BY 非输出项拒绝

- **GIVEN** 表 `t(a INT, b INT)` 存在
- **WHEN** `SELECT DISTINCT a FROM t ORDER BY b`
- **THEN** 计划期点名拒绝（SQLite DISTINCT 约束对齐）

### Requirement: DISTINCT 拒绝面与既有语义零回归

`DISTINCT ON (...)`（PostgreSQL 形态）SHALL 点名拒绝；`ALL` 量化词 SHALL 显式接受并等价既有不去重行为。既有非 DISTINCT 查询、`SELECT *`、多语句与子查询面 SHALL 零回归；全部既有测试零修改通过。

#### Scenario: DISTINCT ON 拒绝

- **WHEN** `SELECT DISTINCT ON(a) a, b FROM t`
- **THEN** 计划期点名拒绝

#### Scenario: ALL 量化词等价既有行为

- **GIVEN** 表 `t(v VARCHAR)` 含 'a','a','b'
- **WHEN** `SELECT ALL v FROM t`
- **THEN** 输出 3 行（与 `SELECT v FROM t` 一致）

#### Scenario: 既有查询零回归

- **WHEN** 执行既有非 DISTINCT 查询形态（含子查询、聚合、no-FROM SELECT）
- **THEN** 结果与计划形状与本变更前一致
