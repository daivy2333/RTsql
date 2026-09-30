# join-output-shape-resolution Specification

## Purpose
约束 JOIN 基座的名字解析与输出形状：聚合参数与分组键在完整组合列布局上精确解析（限定名按表、参数列为计划期索引），同名裸名歧义 `AmbiguousColumn` 点名、未知列 `ColumnNotFound` 点名，纯列引用别名项可达且表头为别名，任意表达式项 × JOIN 维持拒绝；派生基座（FROM 子查询/CTE/视图）的外层纯列清单按清单裁剪行与表头。来源：MS25（change `2026-09-26-ms25-sql-read-surface`，SQLite 语义对齐）+ MS26 T8（change `2026-09-30-ms26-embedded-runtime`，派生基座投影收口）。

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

JOIN 查询的 SELECT 投影 SHALL 逐项解析：未知列 SHALL 以 `ColumnNotFound` 计划期点名拒绝（现状零宽行静默产出的缺陷终止）；纯列引用的别名项（`t.c AS alias`）SHALL 可达且表头为别名（MS19 T4b 机制对 JOIN 基座的扩展）；任意表达式项 × JOIN 维持既有显式拒绝。

#### Scenario: 未知列点名拒绝

- **GIVEN** 同上种子
- **WHEN** `SELECT nosuch FROM p JOIN q ON p.g < q.h`
- **THEN** 计划期 `ColumnNotFound` 点名拒绝（现状 RED：零列零宽行静默产出）

#### Scenario: 纯列引用别名项可达

- **GIVEN** 同上种子
- **WHEN** `SELECT p.g AS pg FROM p JOIN q ON p.g < q.h`
- **THEN** 输出 pg 列含各组合行的 p.g 值，表头为 `pg`（现状 RED：零宽行）

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
