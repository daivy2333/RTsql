## ADDED Requirements

### Requirement: EXPLAIN SELECT 计划树文本输出

`EXPLAIN <SELECT 查询>` SHALL 计划期构造目标查询的执行计划并以文本形式输出计划树，SHALL NOT 执行目标查询（无数据读取副作用、无 MVCC 快照行流）。输出 SHALL 为单列行集，每行对应计划树一个节点，按树形缩进呈现：节点名（PhysicalPlan 变体）+ 关键参数（扫描表名与索引形态、过滤谓词文本、投影列清单、JOIN 类型、排序键、LIMIT 值等，随节点类型而异）。输出 SHALL 经既有 CLI 四格式渲染面（table/json/csv/tsv）呈现。目标查询的表缺失、列缺失等计划期错误 SHALL 按既有错误面拒绝（EXPLAIN 不豁免计划期校验）。

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

### Requirement: EXPLAIN 拒绝面

`EXPLAIN` 作用于非 SELECT 族语句（INSERT/UPDATE/DELETE/CREATE TABLE/DROP TABLE/事务语句等）SHALL 计划期点名拒绝；嵌套 `EXPLAIN EXPLAIN` SHALL 点名拒绝。

#### Scenario: DML 拒绝

- **WHEN** `EXPLAIN INSERT INTO t VALUES (1)`
- **THEN** 计划期点名拒绝

#### Scenario: DDL 拒绝

- **WHEN** `EXPLAIN CREATE TABLE t2(id INT)`
- **THEN** 计划期点名拒绝

### Requirement: 既有语义零回归

本能力 SHALL NOT 改变既有行为：非 EXPLAIN 前缀的语句解析与执行零变化；`SELECT` 关键字作为列名/别名的既有解析不受影响（`SELECT explain FROM t` 类形态行为保持）；plan cache 键语义零变化；全部既有测试零修改通过。

#### Scenario: 非 EXPLAIN 语句零回归

- **WHEN** 执行既有查询与 DML/DDL 形态
- **THEN** 结果与计划形状与本变更前一致

#### Scenario: EXPLAIN 语句自身缓存行为无害

- **WHEN** 同一 `EXPLAIN <SELECT>` 连续执行两次（经 plan cache 命中路径）
- **THEN** 两次输出一致
