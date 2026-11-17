## MODIFIED Requirements

### Requirement: EXPLAIN SELECT 计划树文本输出

`EXPLAIN <SELECT 查询>` SHALL 计划期构造目标查询的执行计划并以文本形式输出计划树，SHALL NOT 执行目标查询（无数据读取副作用、无 MVCC 快照行流）。输出 SHALL 为单列行集，按树形缩进呈现：节点名（PhysicalPlan 变体）+ 关键参数（扫描表名与索引形态、过滤谓词文本、投影列清单、JOIN 类型、排序键、LIMIT 值等，随节点类型而异）。`query-cost-and-join-order` 要求输出连接顺序来源时，SHALL 在首行 `EXPLAIN` 之后额外输出一行，缩进两个空格，文本为 `JoinOrder estimated` 或 `JoinOrder written`。该行 SHALL NOT 替换任何计划节点，也 SHALL NOT 改写节点行的文本。没有该要求时，节点行 SHALL 与不输出这一行时逐字节一致。输出 SHALL 经既有 CLI 四格式渲染面（table/json/csv/tsv）呈现。目标查询的表缺失、列缺失等计划期错误 SHALL 按既有错误面拒绝（EXPLAIN 不豁免计划期校验）。

#### Scenario: 简单查询计划树

- **GIVEN** 表 `t(id INT PRIMARY KEY, name VARCHAR)` 存在
- **WHEN** `EXPLAIN SELECT name FROM t WHERE id = 1`
- **THEN** 输出计划树行集：可见扫描节点（表名 t）与谓词/投影参数，语句未执行（无行数据输出）

#### Scenario: 复合查询节点参数

- **GIVEN** 表 `l(id INT)`、`r(rid INT)` 各含数据
- **WHEN** `EXPLAIN SELECT l.id FROM l JOIN r ON l.id = r.rid WHERE l.id > 1 ORDER BY l.id LIMIT 5`
- **THEN** 计划树含 JOIN 节点（类型/等值键）、Filter 谓词文本、Sort 排序键、Limit 值节点

#### Scenario: EXPLAIN 不执行目标语句

- **GIVEN** 表 `t(id INT PRIMARY KEY)` 存在且含数据
- **WHEN** `EXPLAIN SELECT * FROM t`
- **THEN** 仅输出计划树；随后同一会话查询验证无副作用（数据未变、无锁残留）

#### Scenario: 计划期错误不豁免

- **WHEN** `EXPLAIN SELECT x FROM nosuch_table`
- **THEN** 按既有计划期错误拒绝（表缺失，与直接执行同错误面）

#### Scenario: 未收集统计时不增加 JoinOrder 行

- **GIVEN** 会话没有可用的表统计
- **WHEN** `EXPLAIN` 一条带 `ORDER BY` 的三表内连接
- **THEN** 输出中没有 `JoinOrder estimated`，也没有 `JoinOrder written`
- **AND** 该语句的节点行与统计功能引入前逐字节一致
- **AND** EXPLAIN 不读取这三张表的行

### Requirement: 既有语义零回归

本能力 SHALL NOT 改变这些既有行为：`SELECT explain FROM t` 一类把 explain 当作列名或别名的解析；EXPLAIN 的 plan cache 键仍只含语句文本；未收集表统计时，既有查询的 EXPLAIN 节点行与收集统计之前逐字节一致。`ANALYZE` 与连接叶序的行为由 `query-cost-and-join-order` 规定。全部既有测试在未收集表统计时 SHALL 零修改通过。

#### Scenario: 非 EXPLAIN 语句零回归

- **WHEN** 执行既有查询与 DML/DDL，且本会话没有成功收集过表统计
- **THEN** 结果与计划形状与本变更前一致

#### Scenario: EXPLAIN 语句自身缓存行为无害

- **WHEN** 同一 `EXPLAIN <SELECT>` 连续执行两次（经 plan cache 命中路径）
- **THEN** 两次输出一致
