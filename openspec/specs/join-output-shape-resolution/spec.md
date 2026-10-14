# join-output-shape-resolution Specification

## Purpose
约束 JOIN 基座的名字解析与输出形状：聚合参数与分组键在完整组合列布局上精确解析（限定名按表、参数列为计划期索引），同名裸名歧义 `AmbiguousColumn` 点名、未知列 `ColumnNotFound` 点名，纯列引用别名项可达且表头为别名，任意表达式项 × JOIN 维持拒绝；派生基座（FROM 子查询/CTE/视图）的外层纯列清单按清单裁剪行与表头。来源：MS25（change `2026-09-26-ms25-sql-read-surface`，SQLite 语义对齐）+ MS26 T8（change `2026-09-30-ms26-embedded-runtime`，派生基座投影收口）+ 本 change `2026-09-30-catalog-durability-and-cli-surface` Iteration 001（投影严格解析纳入表别名限定符归一化）。

## Requirements

### Requirement: 聚合×JOIN 完整形状解析

聚合查询以 JOIN（含外连接、含 WHERE 组合）为基座时，聚合参数与 GROUP BY 分组键 SHALL 在 JOIN 的完整组合列布局上解析：限定名（`t.c`）SHALL 精确解析到所指表的列；聚合参数 SHALL 在计划期解析为输入行索引（SHALL NOT 依赖 SELECT 清单是否包含该列）——参数列不在 SELECT 清单时求值 SHALL 正确（现状静默返回 NULL 的缺陷终止）。单表聚合的既有解析语义不变。

#### Scenario: 聚合参数在 SELECT 清单外可达

- **GIVEN** 表 `p(g INT PRIMARY KEY, x INT)` 含 (1,100),(2,100)；表 `q(h INT PRIMARY KEY, x INT)` 含 (5,200),(6,300)
- **WHEN** `SELECT p.g, sum(p.x) FROM p JOIN q ON p.g < q.h GROUP BY p.g`
- **THEN** 输出 (1,200)、(2,200)（每组按 p.x 求和：各 p 行匹配两个 q 行，100+100；现状 RED：静默 NULL）

#### Scenario: 限定名分组键在完整形状下精确解析

- **GIVEN** 同上种子
- **WHEN** `SELECT p.x, count(*) FROM p JOIN q ON p.g < q.h GROUP BY p.x`
- **THEN** 输出 (100,4)（限定名键精确指向 p.x；本场景现状即可达，作为全形状化改造的保持见证）

### Requirement: 同名歧义点名拒绝

JOIN 查询中裸名（非限定）列引用命中多张被连接表的同名卷时 SHALL 计划期以 `AmbiguousColumn` 点名拒绝（SQLite 对齐），SHALL NOT 静默解析到任一侧（现状 `column_indices` HashMap 末位胜出的缺陷终止）。适用面：GROUP BY 分组键、聚合函数参数、非聚合 SELECT 投影清单。限定名引用不受影响。

#### Scenario: GROUP BY 裸名歧义拒绝

- **GIVEN** 同上种子
- **WHEN** `SELECT x, count(*) FROM p JOIN q ON p.g < q.h GROUP BY x`
- **THEN** 计划期 `AmbiguousColumn` 点名拒绝（现状 RED：静默按后出现表分组，非等值 ON 下错误分组 [[200,2],[300,2]]）

#### Scenario: 聚合参数裸名歧义拒绝

- **GIVEN** 同上种子
- **WHEN** `SELECT p.g, sum(x) FROM p JOIN q ON p.g < q.h GROUP BY p.g`
- **THEN** 计划期 `AmbiguousColumn` 点名拒绝（现状静默取末位表列）

#### Scenario: 非聚合投影裸名歧义拒绝

- **GIVEN** 同上种子
- **WHEN** `SELECT x FROM p JOIN q ON p.g < q.h`
- **THEN** 计划期 `AmbiguousColumn` 点名拒绝（现状输出两列同名 `x`）

### Requirement: JOIN 基座投影严格解析与别名列可达

JOIN 查询的 SELECT 投影 SHALL 逐项解析：未知列 SHALL 以 `ColumnNotFound` 计划期点名拒绝（现状零宽行静默产出的缺陷终止）；纯列引用的别名项（`t.c AS alias`）SHALL 可达且表头为别名（MS19 T4b 机制对 JOIN 基座的扩展）；限定列引用的限定符 SHALL 先经表别名归一化（`sql-table-alias-resolution` 域）再匹配组合形状的真实表名——归一化后不可达的限定符 SHALL 点名拒绝而非计入列缺失；任意表达式项 × JOIN 维持既有显式拒绝。纯列清单的严格校验与过滤（命中计数、歧义点名）在归一化后的限定符上执行，判定规则不变。

#### Scenario: 未知列点名拒绝

- **GIVEN** 同上种子
- **WHEN** `SELECT nosuch FROM p JOIN q ON p.g < q.h`
- **THEN** 计划期 `ColumnNotFound` 点名拒绝（现状 RED：零列零宽行静默产出）

#### Scenario: 纯列引用别名项可达

- **GIVEN** 同上种子
- **WHEN** `SELECT p.g AS pg FROM p JOIN q ON p.g < q.h`
- **THEN** 输出 pg 列含各组合行的 p.g 值，表头为 `pg`

#### Scenario: 别名限定列可达

- **GIVEN** `FROM people a JOIN people b ON a.id = b.id`（两别名指向同表）
- **WHEN** `SELECT a.name FROM …`（历史缺陷形态：`Column not found: name`）
- **THEN** 返回左别名侧 name 列，表头 `name`

#### Scenario: 表达式项 × JOIN 维持拒绝

- **GIVEN** 同上种子
- **WHEN** `SELECT upper(p.x) FROM p JOIN q ON p.g < q.h`
- **THEN** 计划期点名拒绝（既有「表达式项 × JOIN」拒绝面不变）

### Requirement: 既有语义零回归

本能力 SHALL NOT 改变无歧义、无未知列的有效查询行为：JOIN 查询（含聚合、外连接、WHERE/ORDER BY 组合——Iteration 000–003 交付面）的计划形状、结果与表头逐字节不变；单表聚合解析语义不变；既有测试零修改通过，**校准白名单**由 Cycle 调查定稿（仅限依赖末位胜出或零宽行静默行为的既有用例——当前调查未见此类用例）。

#### Scenario: 有效查询零回归

- **WHEN** 执行既有 JOIN/聚合×JOIN 形态（无歧义列名，含 `tests/outer_join_test.rs`、`tests/nested_loop_join_test.rs`、`tests/group_by_expr_test.rs` 既有矩阵）
- **THEN** 结果、计划形状与表头与本变更前一致

#### Scenario: 全量回归

- **WHEN** `cargo test -- --test-threads=1`
- **THEN** 0 failed，测试总数只增不减（白名单外零校准）

### Requirement: 派生基座的外层投影裁剪

单表派生基座（`FROM (子查询) AS alias`、CTE 引用、视图展开——三者共用 `DerivedScan` 通道）上的纯列清单 SELECT SHALL 按外层 SELECT 清单裁剪输出行与表头：输出列恰为外层清单解析到的派生列，顺序与清单一致，表头与行形状一致。外层清单引用派生列集中不存在的列 SHALL 在计划期以 `ColumnNotFound`（42703）点名拒绝，SHALL NOT 静默返回多列（与基表 Scan 基座的严格投影解析同纪律）。WHERE（Filter 承载）与 ORDER BY（Sort 承载）组合 SHALL 保持同一裁剪语义；`CREATE TABLE AS SELECT` 以派生基座为源时列名 SHALL 取裁剪后的输出表头。`SELECT *`、聚合查询（输入保持全形状）、表达式与别名项（既有顶层 Projection 通路）SHALL 维持既有行为。

#### Scenario: 派生表外层投影裁剪

- **GIVEN** 表 `t(id, n)` 含两行
- **WHEN** `SELECT n FROM (SELECT id, n FROM t) AS d`
- **THEN** 输出一列 `["n"]`，行值仅含 n 列（表头与行形状一致）

#### Scenario: CTE 与视图通道等价

- **WHEN** `WITH c AS (SELECT id, n FROM t) SELECT n FROM c`；视图 `v AS SELECT id, n FROM t` 上执行 `SELECT n FROM v`
- **THEN** 两者与派生表形态同型，各输出一列 `["n"]`

#### Scenario: 未知外层列点名拒绝

- **WHEN** `SELECT zzz FROM (SELECT id, n FROM t) AS d`
- **THEN** 计划期 `ColumnNotFound`（42703）点名拒绝，不静默返回派生全集

#### Scenario: 组合面

- **WHEN** `SELECT n FROM (SELECT id, n FROM t) AS d WHERE id = 1`；`SELECT n FROM (SELECT id, n FROM t) AS d ORDER BY n`；`CREATE TABLE c AS SELECT n FROM (SELECT id, n FROM t) AS d`
- **THEN** 分别为：Filter 过滤后单列输出；排序后单列输出；CTAS 建单列表 `n` 并物化对应值

#### Scenario: 恒等面不回归

- **WHEN** `SELECT * FROM (SELECT id, n FROM t) AS d`；`SELECT COUNT(*) FROM (SELECT id, n FROM t) AS d`；`SELECT n AS x FROM (SELECT id, n FROM t) AS d`
- **THEN** 分别为：两列恒等输出；聚合正常计数；别名通路（`error-reporting` R5）输出 `["x"]` 单列——三者均与本 Requirement 增量前行为一致

### Requirement: JOIN 基座输出列序与请求一致

纯列清单（裸路径与 WHERE / ORDER BY 裁剪路径）的 JOIN 基座输出列序 SHALL 按 SELECT 清单序（对齐 Scan 基座既有语义）；重复的 SELECT 项 SHALL 重复产出对应列；裸名多命中 SHALL 维持计划期 `AmbiguousColumn` 点名、未知列 SHALL 维持 `ColumnNotFound`；`SELECT *` 与 carry-all 全组合承载形态（聚合 / 派生 / CTAS 输入、Sort 承载全列裁剪）SHALL 既有语义零变化；表达式项的顶层 Projection 路径（既有 SELECT 序）SHALL 零回归；输出表头与行 SHALL 同源自洽（CTAS 列名 / INSERT SELECT 映射 / DerivedScan 列清单随输出序）。

#### Scenario: 裸纯列清单 SELECT 序

- **GIVEN** `emp3(id, ename, dept_id)` JOIN `dept(id, dname)` ON `e.dept_id = d.id`
- **WHEN** `SELECT d.dname, e.ename FROM emp3 e JOIN dept d ON e.dept_id = d.id`
- **THEN** 表头与行均为 `[dname, ename]` 序（失败边界——修复前表序 `[ename, dname]`）

#### Scenario: WHERE / ORDER BY 裁剪路径 SELECT 序

- **GIVEN** 同上
- **WHEN** `SELECT d.dname, e.ename … WHERE e.id = 1`；`SELECT d.dname, e.ename … ORDER BY e.id`
- **THEN** Filter / Sort 裁剪序 = SELECT 清单序（失败边界——修复前裁剪按表序）

#### Scenario: 重复项与全组合形态

- **GIVEN** 同上
- **WHEN** `SELECT e.ename, e.ename …`；`SELECT *`；聚合 / CTAS 消费 JOIN 输出
- **THEN** 重复项产出两列；`SELECT *` 表序不变；CTAS 列名取输出表头（与 Scan 基座 CTAS 一致）

### Requirement: 多表链式 JOIN 装配正确性

单 FROM ≥3 表因子的左深等值链 SHALL 端到端可达——hash+hash 纯等值链与 hash+NLJ 混合链（第三步非等值 ON）SHALL 产出正确配对行；每步等值 ON 条件键 SHALL 按（真实表, 列）限定解析，链上同名列 SHALL 不串列；中间 JOIN 步 SHALL 承载链前缀全组合列且输出列索引为左输入行绝对偏移；仅 SELECT 部分表 SHALL 正常裁剪零 panic；两表单步全形态（含 `SELECT *`、WHERE、ORDER BY、聚合、测试直构 `JoinConfig` 裸名键）SHALL 逐字节零回归；真未知列的错误面 SHALL 不变。

#### Scenario: hash+hash 三表链

- **GIVEN** `emp(id, name, dept_id)` / `dept(id, label)` / `sal(id, dept_id, amount)`，ON 链 `e.dept_id = d.id` ∧ `s.dept_id = d.id`
- **WHEN** 三表 SELECT 等值链查询
- **THEN** 端到端可达且每步配对正确（失败边界——修复前 `Column not found` 计划失败）

#### Scenario: hash+NLJ 混合链

- **GIVEN** 同上，第三步 ON 为非等值谓词
- **WHEN** 三表 SELECT
- **THEN** 可达且谓词作用于全组合行（失败边界——修复前计划失败）

#### Scenario: 仅前两表 SELECT 不 panic

- **GIVEN** 同上三表 FROM
- **WHEN** `SELECT e.name, d.label FROM … 三表链`
- **THEN** 正常输出前两表列（失败边界——修复前 `join.rs` 键查表 `.expect` panic exit 101）

#### Scenario: 链上同名列不串列

- **GIVEN** `t1(id, …)` / `t2(id, …)` / `t3(x, …)` 链，ON 条件跨链引用 `t2.id`
- **WHEN** 等值链查询
- **THEN** 条件键按（真实表, 列）解析、配对正确（失败边界——修复前裸列名查表静默串列或缺列 panic）

#### Scenario: 两表单步零回归

- **GIVEN** 既有两表 JOIN 矩阵
- **WHEN** 全形态查询与直构 `JoinConfig` 执行
- **THEN** 行为逐字节零变化

### Requirement: 派生表与视图内层输出列外层可见

FROM 派生表与视图通道的内层 SELECT 输出列——列别名（`age AS v`）与表达式（计算）列（`CASE … END AS tier`）——SHALL 对外层查询可引用（内层顶层 Projection 路由在 FROM 派生上下文放行，`building_from_derived` 判式）。内层 `SELECT DISTINCT` 在派生上下文 SHALL 真实去重（不再静默跳过）。三通道（FROM 派生表 / 视图 / CTE）行为 SHALL 一致（`sql-views`「CTE 通道展开与内联派生表逐字节一致」声明成立）。裸列名形态、CTE 通道、外层投影裁剪（本域既有「派生基座的外层投影裁剪」Requirement）与派生表内集合操作（T5 先例）SHALL 零回归。

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
