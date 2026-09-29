## ADDED Requirements

> 夹具注记：本项目建表未声明主键时以首列为隐式主键，Int 首列的重复值被 `DuplicateKey` 拒绝（`src/executor/create_table.rs`）；涉重复值的 GIVEN 表以显式 `k INT PRIMARY KEY` 锚列承载行（与见证测试夹具一致）。

### Requirement: UNION 与 UNION ALL

`<查询A> UNION [ALL] <查询B>` SHALL 计划期接受并端到端可达：UNION 输出两分支行的并集并去重（NULL 视为彼此相等）；UNION ALL SHALL 保留重复行。分支为合法 SELECT 查询；两分支输出列数 SHALL 相等。既有 `UnsupportedStatement("SET OPERATION")` 拒绝面 SHALL 对支持的形态终止。

#### Scenario: UNION 去重并集

- **GIVEN** 表 `t1(v INT)` 含 (1),(2)；表 `t2(v INT)` 含 (2),(3)
- **WHEN** `SELECT v FROM t1 UNION SELECT v FROM t2`
- **THEN** 输出 1,2,3 各一行（重复 2 折叠）

#### Scenario: UNION ALL 保留重复

- **GIVEN** 同上种子
- **WHEN** `SELECT v FROM t1 UNION ALL SELECT v FROM t2`
- **THEN** 输出 4 行：1,2,2,3（含重复与序）

#### Scenario: UNION NULL 相等折叠

- **GIVEN** 表 `t1(v INT)` 含 (NULL)；表 `t2(v INT)` 含 (NULL)
- **WHEN** `SELECT v FROM t1 UNION SELECT v FROM t2`
- **THEN** 输出一行 NULL

### Requirement: INTERSECT 与 EXCEPT

`<查询A> INTERSECT <查询B>` SHALL 输出两分支行集的去重交集；`<查询A> EXCEPT <查询B>` SHALL 输出在 A 且不在 B 的去重行集。匹配与差集判定 SHALL 使用 NULL 相等语义（NULL 与 NULL 匹配）。分支列数 SHALL 相等，否则计划期点名拒绝。

#### Scenario: INTERSECT 去重交集

- **GIVEN** 表 `t1(k INT PRIMARY KEY, v INT)` 含 (1,1),(2,2),(3,2)；表 `t2(k INT PRIMARY KEY, v INT)` 含 (1,2),(2,3)
- **WHEN** `SELECT v FROM t1 INTERSECT SELECT v FROM t2`
- **THEN** 输出一行 2

#### Scenario: EXCEPT 差集

- **GIVEN** 表 `t1(k INT PRIMARY KEY, v INT)` 含 (1,1),(2,2),(3,2)；表 `t2(k INT PRIMARY KEY, v INT)` 含 (1,2)
- **WHEN** `SELECT v FROM t1 EXCEPT SELECT v FROM t2`
- **THEN** 输出一行 1

#### Scenario: EXCEPT 中 NULL 匹配语义

- **GIVEN** 表 `t1(v INT)` 含 (NULL),(1)；表 `t2(v INT)` 含 (NULL)
- **WHEN** `SELECT v FROM t1 EXCEPT SELECT v FROM t2`
- **THEN** 输出一行 1（NULL 行被差集移除）

### Requirement: 复合结构与优先级

同级集合操作 SHALL 左结合；`INTERSECT` 的结合优先级 SHALL 高于 `UNION`/`EXCEPT`（`a UNION b INTERSECT c` 等价 `a UNION (b INTERSECT c)`）。三分支以上的链式复合 SHALL 支持。`INTERSECT ALL` 与 `EXCEPT ALL` SHALL 点名拒绝（SQLite 对齐）；分支括号化 `(SELECT ...) UNION (SELECT ...)` SHALL 点名拒绝。

#### Scenario: 链式三分支左结合

- **GIVEN** 表 `t1(v INT)` 含 (1),(2)；表 `t2(v INT)` 含 (2),(3)；表 `t3(v INT)` 含 (3)
- **WHEN** `SELECT v FROM t1 UNION SELECT v FROM t2 UNION SELECT v FROM t3`
- **THEN** 输出 1,2,3 各一行

#### Scenario: INTERSECT 优先级高于 UNION

- **GIVEN** 表 `a(v INT)` 含 (1),(2)；表 `b(v INT)` 含 (2),(3)；表 `c(v INT)` 含 (3)
- **WHEN** `SELECT v FROM a UNION SELECT v FROM b INTERSECT SELECT v FROM c`
- **THEN** 输出 1,2,3（先算 b∩c={3}，再 a∪{3}）

#### Scenario: INTERSECT ALL 与括号化拒绝

- **WHEN** 执行 `SELECT v FROM t1 INTERSECT ALL SELECT v FROM t2` 或 `(SELECT v FROM t1) UNION (SELECT v FROM t2)`
- **THEN** 计划期点名拒绝

### Requirement: 复合结果排序与既有语义零回归

复合语句尾部的 ORDER BY / LIMIT SHALL 作用于复合输出结果集（排序键引用输出列名或列位置以外的形态随调查按 SQLite 对齐定稿并记入 design）。列数不匹配 SHALL 点名拒绝。集合操作之外的全部既有查询面 SHALL 零回归；全部既有测试零修改通过。

#### Scenario: 复合结果 ORDER BY

- **GIVEN** 表 `t1(v INT)` 含 (2),(1)；表 `t2(v INT)` 含 (3)
- **WHEN** `SELECT v FROM t1 UNION SELECT v FROM t2 ORDER BY v DESC`
- **THEN** 输出 3,2,1

#### Scenario: 列数不匹配拒绝

- **GIVEN** 表 `t1(a INT, b INT)`、`t2(v INT)` 存在
- **WHEN** `SELECT a, b FROM t1 UNION SELECT v FROM t2`
- **THEN** 计划期点名拒绝

#### Scenario: 既有查询零回归

- **WHEN** 执行既有单查询形态
- **THEN** 结果与计划形状与本变更前一致
