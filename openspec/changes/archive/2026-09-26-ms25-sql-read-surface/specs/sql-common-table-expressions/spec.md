## ADDED Requirements

### Requirement: 顶层 WITH CTE 可达

顶层语句 `WITH <名> [ (列别名列表) ] AS ( <SELECT 查询> ) [, <名2> AS (...) ...] <主查询>` SHALL 计划期接受并端到端可达：主查询按名引用 CTE 时 SHALL 解析到对应 CTE 定义；多个 CTE SHALL 支持；后定义的 CTE SHALL 可引用先定义的 CTE（前向引用）；反向引用（先定义引用后定义）SHALL 点名拒绝。列别名列表 SHALL 重命名 CTE 输出列。CTE 名 SHALL 遮蔽同目录下同名基表（仅本语句作用域）。本能力修复既有缺陷——`query.with` 当前被 planner 静默丢弃（同名基表时静默按基表查询）；该静默行为 SHALL 终止。

#### Scenario: 基础 CTE 可达

- **GIVEN** 表 `t(v INT)` 含 (1),(2)
- **WHEN** `WITH c AS (SELECT v * 2 AS d FROM t) SELECT d FROM c`
- **THEN** 输出 2,4（现状 RED：WITH 被静默丢弃、按基表 c 查询报 TableNotFound 或命中同名基表）

#### Scenario: CTE 名遮蔽同名基表

- **GIVEN** 已有基表 `c(v INT)` 含 (9)
- **WHEN** `WITH c AS (SELECT 1 AS v) SELECT v FROM c`
- **THEN** 输出 1（CTE 遮蔽基表；现状 RED：静默输出 9）

#### Scenario: 多定义与前向引用

- **GIVEN** 表 `t(v INT)` 含 (1),(2)
- **WHEN** `WITH a AS (SELECT v FROM t), b AS (SELECT v + 10 AS w FROM a) SELECT w FROM b`
- **THEN** 输出 11,12

#### Scenario: 列别名列表

- **GIVEN** 表 `t(a INT, b VARCHAR)` 含 (1,'x')
- **WHEN** `WITH c(n, s) AS (SELECT a, b FROM t) SELECT n, s FROM c`
- **THEN** 表头为 n、s，输出 (1,'x')

#### Scenario: 反向引用拒绝

- **WHEN** `WITH a AS (SELECT * FROM b), b AS (SELECT 1 AS v) SELECT * FROM a`
- **THEN** 计划期点名拒绝

### Requirement: CTE 引用一致性

被引用 N 次（N ≥ 1）的 CTE，各引用 SHALL 面对同一语句级数据视图（同一 MVCC 语句快照）——跨引用读取的行集 SHALL 一致且等价于单次求值结果。当前实现按引用内联求值（每引用一次派生表物化，同一快照下行集一致）；跨引用共享物化为后续性能优化，不属本能力验收面。

#### Scenario: 多引用行集一致

- **GIVEN** 表 `t(v INT)` 含 (1),(2)
- **WHEN** `WITH c AS (SELECT v FROM t) SELECT a.v, b.v FROM c a JOIN c b ON a.v = b.v`
- **THEN** 输出 (1,1)、(2,2)（各引用面对同一行集）

#### Scenario: 物化视图一致性（多引用间无差异）

- **GIVEN** 表 `t(v INT)` 含 (1),(2)
- **WHEN** `WITH c AS (SELECT v FROM t) SELECT (SELECT count(*) FROM c) AS n1, (SELECT count(*) FROM c) AS n2`
- **THEN** 输出一行 (2,2)（两次标量引用读同一行集）

### Requirement: CTE 拒绝面

`WITH RECURSIVE`（含内容并非递归的形态）SHALL 计划期点名拒绝；CTE 定义内自引用（递归形态）SHALL 点名拒绝；子查询（WHERE/标量/FROM 派生表）内的 WITH 子句 SHALL 点名拒绝（本能力范围仅顶层语句，边界随实现显式化）。CTE 引用未定义名 SHALL 按既有表缺失错误路径拒绝。

#### Scenario: RECURSIVE 关键字拒绝

- **WHEN** `WITH RECURSIVE c AS (SELECT 1 AS v) SELECT v FROM c`
- **THEN** 计划期点名拒绝（递归 CTE 不支持）

#### Scenario: 自引用拒绝

- **WHEN** `WITH c AS (SELECT v FROM c) SELECT * FROM c`
- **THEN** 计划期点名拒绝

#### Scenario: 子查询内 WITH 拒绝

- **GIVEN** 表 `t(id INT PRIMARY KEY)` 存在
- **WHEN** `SELECT id FROM t WHERE id IN (WITH c AS (SELECT 1 AS x) SELECT x FROM c)`
- **THEN** 计划期点名拒绝（子查询内 WITH 不在范围）

### Requirement: 既有语义零回归

本能力 SHALL NOT 改变既有行为：无 WITH 的全部查询、同名基表查询（无 WITH 时）、plan cache 键语义、多语句与事务语句面零回归；全部既有测试零修改通过。

#### Scenario: 无 WITH 查询零回归

- **WHEN** 执行既有查询形态（含与 CTE 名同名的普通基表查询）
- **THEN** 结果与本变更前一致

#### Scenario: 全量回归

- **WHEN** `cargo test -- --test-threads=1`
- **THEN** 0 failed，测试总数只增不减
