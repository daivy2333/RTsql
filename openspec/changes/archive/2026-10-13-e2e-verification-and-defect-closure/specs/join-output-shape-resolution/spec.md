## ADDED Requirements

### Requirement: 派生表与视图内层输出列外层可见

FROM 派生表与视图通道的内层 SELECT 输出列——列别名（`age AS v`）与表达式（计算）列（`CASE … END AS tier`）——SHALL 对外层查询可引用（内层顶层 Projection 路由在 FROM 派生上下文放行，`building_from_derived` 判式）。内层 `SELECT DISTINCT` 在派生上下文 SHALL 真实去重（不再静默跳过）。三通道（FROM 派生表 / 视图 / CTE）行为 SHALL 一致（`sql-views`「CTE 通道展开与内联派生表逐字节一致」声明成立）。裸列名形态、CTE 通道、外层投影裁剪（`join-output-shape-resolution` 既有 R5）与派生表内集合操作（T5 先例）SHALL 零回归。

#### Scenario: 派生表内层别名外层引用

- **GIVEN** people 表 3 行
- **WHEN** `SELECT v FROM (SELECT age AS v FROM people) t`
- **THEN** 正常返回 v 列值（失败边界——修复前：42703 `Column not found: v`）

#### Scenario: 派生表内层表达式列外层引用

- **GIVEN** 同上
- **WHEN** `SELECT tier FROM (SELECT CASE … END AS tier FROM people) t`（限定/裸名两形态）
- **THEN** 正常返回计算列（失败边界——修复前：58000 `Invalid aggregate argument: Expected column name`——纯 SELECT 收到聚合错误文案）

#### Scenario: 视图定义含别名/表达式列可查询

- **GIVEN** `CREATE VIEW vw AS SELECT age AS v, CASE … END AS tier FROM people`
- **WHEN** `SELECT v FROM vw`；`SELECT tier FROM vw`
- **THEN** 正常返回（失败边界——修复前：42703——视图通道同样中招）

#### Scenario: 三通道一致与 CTE 对照零回归

- **GIVEN** 同一内层查询三种包装
- **WHEN** `FROM (…)` 派生表 / 视图 / `WITH x AS (…) SELECT … FROM x`
- **THEN** 三者输出一致；CTE 通道行为与修复前一致（本就正确）

#### Scenario: 内层 DISTINCT 真实去重

- **GIVEN** 含重复值的列
- **WHEN** `SELECT d FROM (SELECT DISTINCT x AS d FROM t) s`
- **THEN** 去重结果（失败边界——修复前：派生上下文 DISTINCT 被静默跳过）

#### Scenario: 表达式子查询上下文维持既有抑制

- **GIVEN** 同上
- **WHEN** `WHERE id IN (SELECT …)`、`EXISTS (…)`、标量子查询内使用表达式项/别名形态
- **THEN** 既有错误面逐字节不变（SemiJoin/SubqueryEval/ScalarSubqueryFilter 内层形状契约不变）

### Requirement: SemiJoin/AntiJoin 基座全宽承载与排序键

WHERE 带 IN/EXISTS 子查询（SemiJoin/AntiJoin 路径）时，基座 SHALL 承载左输入基表全宽行（`output_columns` = 基表全列，输出行不裁剪），SELECT 清单裁剪与 ORDER BY 排序由下游承载——排序键计划期校验的可解析集与 Sort/Projection 裁剪序 SHALL 与无子查询基座一致：`ORDER BY <非输出列>`（表内列）SHALL 可用且按该列真实排序。字面量 IN（谓词下推路径）、投影形状（表头与行对齐）、EXISTS 形态、相关子查询形态、AntiJoin（NOT IN）形态 SHALL 零回归。

#### Scenario: IN 子查询按非输出列排序

- **GIVEN** people(id, name)/dept(did) 两表
- **WHEN** `SELECT name FROM people WHERE id IN (SELECT did FROM dept) ORDER BY id`
- **THEN** 按 id 排序输出 name（失败边界——修复前：42703 `Column not found: id`）

#### Scenario: 排序语义与无子查询基座一致

- **GIVEN** 同上
- **WHEN** 同查询去子查询对照、`ORDER BY <输出列>`、`WHERE id IN (1,3)` 字面量形态
- **THEN** 各形态排序结果一致；字面量路径零回归

#### Scenario: 投影与子查询形态零回归

- **GIVEN** 同上
- **WHEN** 多列投影、`NOT IN`（AntiJoin）、EXISTS、相关子查询（`IN (SELECT … WHERE …)` 引用外层列）组合
- **THEN** 表头与行对齐、行集正确，与修复前一致（形状断言按全宽基座语义校准的点逐条白名单记录）
