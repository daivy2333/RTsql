# sql-table-alias-resolution Specification

## Purpose
约束 FROM 表别名的注册与限定名解析：别名 → 表映射对 SELECT / JOIN（含自连接多别名）/ WHERE / GROUP BY / HAVING / 聚合参数 / ON / ORDER BY 的列引用解析统一生效；限定列引用的限定符经单点归一化（别名命中 → 真实表键，未命中 → 原样）后沿既有通道解析；两段附加 FROM 项无显式别名时尾段作为隐式关联名注册（短名限定列引用与全限定同达）；未知限定符计划期以 `TableNotFound`（42P01）点名，终止「静默丢弃限定符」与「别名被当表名」两类误导错误；三段限定仅 attach 复合键可达，本地形态统一点名拒绝；别名不改变输出表头、`OutputColumn` 真实表名语义或计划形状；子查询（IN / EXISTS / 标量）内层 FROM 项的声明限定符（别名优先、声明别名时基表名并列保留）参与内外层引用分类，内层别名限定引用 SHALL NOT 被误判为相关外层引用产生幻影参数。来源：change `2026-09-30-catalog-durability-and-cli-surface` Iteration 001（P1 表别名限定引用家族失效收口）；附加 FROM 项隐式关联名来源 MS20-T02（change `2026-10-02-ms20-cross-db-dml` Iteration 002）；子查询内层声明限定符分类来源 change `2026-10-15-subquery-alias-binding-and-error-prefix`（T1，ISS16–ISS18）。

## Requirements

### Requirement: FROM 表别名注册

FROM 子句中的表别名（显式 `AS` 或隐式）SHALL 注册为该表在本语句作用域内的限定符：别名 → 表的映射对 SELECT / JOIN（含自连接多别名）/ WHERE / GROUP BY / HAVING / 聚合参数 / ON / ORDER BY 的列引用解析统一生效。两段附加 FROM 项（`alias.table`）在未写显式别名时，其尾段 SHALL 作为该项的隐式关联名注册（标准 SQL 隐式别名语义，SQLite 同型），使短名限定列引用与全限定、显式别名形态同达；主库表与字面含点表名 SHALL NOT 误登记隐式关联名。CTE 与视图基座的有效名（引用别名或定义名）沿既有行为保持。别名（显式或隐式关联名）SHALL NOT 改变输出的表头、`OutputColumn` 的真实表名语义或计划形状（路由、投影索引按真实形状解析）。

#### Scenario: 自连接两别名全子句面可用

- **GIVEN** 表 `people(id, name, team_id)`
- **WHEN** `SELECT a.name, b.name FROM people a JOIN people b ON a.id = b.id WHERE a.team_id = 10 ORDER BY b.id`
- **THEN** 查询成功，两列分别为左右别名对应表的 name 值，排序按右侧别名的 id

#### Scenario: JOIN 三表混用别名与真实名

- **GIVEN** 三表 JOIN（部分臂用别名、部分用真实表名）
- **WHEN** SELECT/ON/WHERE 混合使用别名限定列与真实表名限定列
- **THEN** 全部解析正确，同名裸名歧义仍按 `AmbiguousColumn` 点名

#### Scenario: 别名不改变表头与计划形状

- **GIVEN** `FROM people a JOIN team t ON a.team_id = t.id`
- **WHEN** `SELECT a.name` 执行并 `EXPLAIN`
- **THEN** 表头仍为 `name`；计划形状与真实表名书写等价（同路由同投影）

#### Scenario: 附加 FROM 项短名限定列可达

- **GIVEN** 主库表 `t(id)` 与附加库 `o` 的表 `u(id, tag)`
- **WHEN** `ATTACH 'o' AS o; SELECT u.tag FROM t JOIN o.u ON t.id = u.id`
- **THEN** 查询成功，`u.tag` 解析到附加 `o.u`（JOIN 投影/ON/WHERE/GROUP BY/ORDER BY 各消费点同达）；`SELECT u.tag FROM o.u`（单表基座）同达

#### Scenario: 显式别名优先且字面含点不误登记

- **GIVEN** 主库表 `t`、附加 `o.u`，主库另有字面含点表 `"o.u"`
- **WHEN** `SELECT x.tag FROM t JOIN o.u x`（显式别名）与对字面含点表的短名形态 `SELECT u.tag FROM "o.u"`
- **THEN** 显式别名形态与既有行为零变化；字面含点表名不注册 `u` 关联名，短名形态沿既有未知限定符面点名

### Requirement: 限定列解析认别名（全子句面）

限定列引用 `q.c` 的解析 SHALL 先将限定符 `q` 归一化（别名映射命中 → 真实表键；未命中 → 原样），再沿既有通道解析（单表注册表 / JOIN 组合布局 / attach 复合键 / 相关外层引用）。SHALL 覆盖：JOIN 投影（含纯列清单严格校验与过滤）、ON 腿（Hash 等值臂与 NLJ 布局臂）、WHERE、GROUP BY 与聚合参数（`aggregate_arg_indices` 通道）、ORDER BY（见限定名排序 Requirement）。归一化后仍不可达的限定符 SHALL 以携带该限定符的错误点名（`TableNotFound`，42P01 语义 = FROM 中无该关系），SHALL NOT 误报为列缺失或静默求值。

#### Scenario: JOIN 投影别名限定列

- **GIVEN** `FROM people a JOIN people b ON a.id = b.id`
- **WHEN** `SELECT a.name FROM …`（历史缺陷形态）
- **THEN** 返回左别名侧 name 列，无 `Column not found` 误报

#### Scenario: ON 与 WHERE 别名限定不再被当作表名

- **GIVEN** `FROM people p JOIN team t ON t.id = p.team_id`
- **WHEN** 执行该查询（历史缺陷形态：`Parse error: Table 't' does not exist` / `Table not found: t`）
- **THEN** 成功返回；`SELECT * FROM … WHERE p.team_id = 10` 同样成功

#### Scenario: 聚合与 GROUP BY 别名限定（JOIN 基座）

- **GIVEN** 自连接两别名
- **WHEN** `SELECT a.team_id, COUNT(*) FROM … GROUP BY a.team_id`（历史缺陷形态：`Table 'a' does not exist`）
- **THEN** 分组与聚合参数按左别名侧列正确解析

#### Scenario: 未知限定符点名

- **GIVEN** `FROM people`
- **WHEN** `SELECT x.name FROM people`（x 未声明）
- **THEN** 计划期点名拒绝，错误携带 `x`（不再静默丢弃限定符求值）

### Requirement: 单表限定列与 JOIN 统一

单表扫描基座的限定列解析 SHALL 与 JOIN 基座同规则（经限定符归一化），SHALL NOT 保留「静默丢弃限定符」的解析路径。两段限定的输出列名/表头职责不变（取末段）；已声明别名的既有用法经别名映射正常工作。

#### Scenario: 单表别名投影保持可用

- **GIVEN** `FROM people p`
- **WHEN** `SELECT p.name FROM people p WHERE p.id = 1`（历史缺陷：投影可用但 WHERE 报 `Table 'p' not found`）
- **THEN** 投影与 WHERE 均按别名解析成功，表头 `name`

#### Scenario: 单表未知限定符拒绝

- **GIVEN** `FROM people`（无别名）
- **WHEN** `SELECT p.name FROM people`
- **THEN** 计划期点名拒绝（与 JOIN 形态同语义）

### Requirement: 限定形态边界

三段限定列 `a.b.c` SHALL 仅当 `a.b` 作为整体命中表注册键（attach 复合键 `alias.table`）时可达；本地库形态（如 `db.table.column`）SHALL 计划期点名拒绝，不再出现「单表静默成功 / JOIN 失败」的分裂行为。表引用语境的两段名 attach-first 既有语义不变。attach 复合键通道的既有测试形态零回归。

#### Scenario: 本地三段限定统一拒绝

- **GIVEN** 本地库表 `people`
- **WHEN** `SELECT demo.people.name FROM people`（历史：单表静默成功、JOIN 失败）
- **THEN** 计划期点名拒绝，不再分裂

#### Scenario: ATTACH 三段限定保持可用

- **GIVEN** `ATTACH 'other' AS o` 且 `o` 中有表 `dept_map`
- **WHEN** `SELECT o.dept_map.dept FROM people JOIN o.dept_map ON people.team_id = o.dept_map.team_id`
- **THEN** 行为与修复前一致（既有通道零回归）

### Requirement: 限定名排序与既有语义零回归

ORDER BY SHALL 接受两段限定键：限定符经归一化后按可见列解析排序，输出别名优先语义、既有裸键排序、未知键拒绝语义 SHALL 保持不变；`order-by-alias-sorting` 中「`ORDER BY t.col` 维持 ParseError」的边界注记终止。限定键在**无 WHERE 无聚合的 JOIN 基座**上 SHALL 按组合行布局解析并真实排序（该基座的输出形状 SHALL 因 ORDER BY 存在而承载排序所需列，Sort 承担投影裁剪）；自连接同名列 SHALL 按限定符精确解析到各自表的列（布局索引解析，非裸名首匹配）。全部既有读面（单表/带 WHERE JOIN/DISTINCT/集合操作尾 ORDER BY/CTE/视图/派生基座/聚合形状/外连接）SHALL 零回归。

#### Scenario: ORDER BY 限定键真实排序

- **GIVEN** `FROM people p JOIN team t ON p.team_id = t.id`
- **WHEN** `SELECT p.name FROM … ORDER BY t.id`（历史：`ORDER BY only supports column names`）
- **THEN** 按右侧表 id 真实排序；`ORDER BY p.id`（单表）同理

#### Scenario: JOIN 基座限定键真实排序（无 WHERE）

- **GIVEN** `FROM emp e JOIN dept d ON e.dept_id = d.id`（无 WHERE、无聚合）
- **WHEN** `SELECT e.name FROM … ORDER BY e.id`；`ORDER BY e.salary`（无歧义列）；`ORDER BY emp.id`（全表名）
- **THEN** 各按键真实排序（失败边界——修复前：`Column not found: id` exit 3）

#### Scenario: 自连接同名列按限定符精确解析

- **GIVEN** `FROM emp e1 JOIN emp e2 ON e1.id = e2.manager_id`（salary 两表同名）
- **WHEN** `ORDER BY e1.salary` / `ORDER BY e2.salary`
- **THEN** 各按限定符对应表的列排序（非裸名首匹配）

#### Scenario: 输出别名优先与未知键拒绝不变

- **GIVEN** `SELECT id AS k FROM people p`
- **WHEN** `ORDER BY k` / `ORDER BY p.zzz`
- **THEN** 别名键按既有语义排序；未知限定键计划期点名拒绝

### Requirement: 自连接侧身份与输出归属正确性

自连接（同一表以不同别名出现于 JOIN 两侧）的等值 JOIN SHALL 正确配对与投影：ON 条件两侧的列 SHALL 按书写限定符与 FROM 声明限定符（别名原文 / 归一命中 / 隐式短名）匹配判定侧归属，交换仅发生在实际反序书写形态；两腿同侧 SHALL 计划期点名拒绝（`InvalidQuery`）零副作用；裸名腿 SHALL 维持按位置解析的既有语义；未知限定符 SHALL 维持 `TableNotFound`（42P01）。Hash 与 Nested-Loop 两执行器的输出行与外连接补齐行 SHALL 按计划期显式侧标志归属——自连接两侧行值不同时，各输出列 SHALL 读其限定符对应侧的行值。非自连接全部形态（正向/反向书写、裸名、别名、附加库、内/外连接）SHALL 零回归。三表及以上链式 JOIN 的既有边界（计划期失败）SHALL 不因本要求改变。

#### Scenario: Hash 自连接正向书写配对正确

- **GIVEN** `emp2(id, name, manager_id)`：`(1,'m1',NULL),(2,'m2',NULL),(10,'r1',1),(11,'r2',2)`
- **WHEN** `SELECT e1.name, e2.name FROM emp2 e1 JOIN emp2 e2 ON e1.id = e2.manager_id`
- **THEN** `[["m1","r1"],["m2","r2"]]`（失败边界——修复前 `[["r1","r1"],["r2","r2"]]`）

#### Scenario: 反序书写交换按限定符判定

- **GIVEN** 同上
- **WHEN** `SELECT e1.name, e2.name FROM emp2 e1 JOIN emp2 e2 ON e2.manager_id = e1.id`
- **THEN** 同正向结果 `[["m1","r1"],["m2","r2"]]`（失败边界——修复前 `[["m1","m1"],["m2","m2"]]`）

#### Scenario: 自连接外连接配对与 NULL 补齐

- **GIVEN** 同上
- **WHEN** `SELECT e1.name, e2.name FROM emp2 e1 LEFT JOIN emp2 e2 ON e1.id = e2.manager_id`
- **THEN** `[["m1","r1"],["m2","r2"],["r1",null],["r2",null]]`（失败边界——修复前全部 `[x,x]`；FULL 与非等值 ON 形态同归属语义）

#### Scenario: 双腿同侧计划期拒绝

- **GIVEN** 同上
- **WHEN** `... FROM emp2 e1 JOIN emp2 e2 ON e1.id = e1.manager_id`
- **THEN** 计划期点名拒绝、零副作用（失败边界——修复前静默误判执行）

#### Scenario: 非自连接零回归

- **GIVEN** 独立表 JOIN 矩阵（正向/反向书写、裸名 ON、别名、ATTACH 附加库、INNER/LEFT/FULL）
- **WHEN** 各既有形态查询
- **THEN** 行为逐字节零变化

### Requirement: 子查询内层声明限定符参与外层引用分类

子查询（IN / EXISTS / 标量）内层 FROM 项的可解析限定符集 SHALL 含每个 FROM 项的声明名：显式或隐式别名优先，无别名取基表名；声明了别名时 SHALL 同时保留基表名作为可解析限定符——本引擎顶层对「别名 + 基名限定」本就宽松，子查询层保持同一宽松度（「别名 + 基名限定」子查询形状由既有正确结果退化为错误不得发生）。JOIN 臂 SHALL 同规则；`TableFactor::Derived`（派生表）SHALL 取其别名。相关参数分类（`collect_outer_column_refs`/`extract_correlated_params`）与 `ParameterExpression` 改写（`qualified_column_expression`）SHALL 以该限定符集判断内外——内层别名限定引用（如 `FROM c x ... x.age`）SHALL 解析为内层表列，SHALL NOT 被误判为相关外层引用产生幻影参数。修复前：内层集丢弃别名，`x.age` 被改写为幻影 `ParameterExpression` 并走相关执行路径，产生恒空集 / 按内层存活行数截断外层 / 全放行 / 碰巧正确四种静默错误结果（ISS16/ISS17/ISS18）。

> 基表名并列保留一项由 2026-10-15 用户豁免裁定（Gate 5 Blocker Handoff 方案一），见 change `2026-10-15-subquery-alias-binding-and-error-prefix` Cycle Act Response 的 Blocker Resolution；对应回归锚点 `a1_base_name_qualifier_inside_aliased_inner`。

#### Scenario: EXISTS 内层别名谓词非首列

- **GIVEN** 外层 `a(id)` 两行，内层 `t(id PRIMARY KEY, name)` 含 `name='Lin'`
- **WHEN** `SELECT id FROM a WHERE EXISTS (SELECT 1 FROM t x WHERE x.name = 'Lin')`
- **THEN** 返回两行（修复前：恒空集）

#### Scenario: EXISTS 内层别名谓词首列

- **GIVEN** 同上数据
- **WHEN** `SELECT id FROM a WHERE EXISTS (SELECT 1 FROM t x WHERE x.id = 1)`
- **THEN** 返回两行（EXISTS 判定与子查询行存在性相关，修复前：仅返回外层第一行）

#### Scenario: 相关 EXISTS 经外层非主键列

- **GIVEN** 外层 `a6(id PRIMARY KEY, ref INTEGER, ...)` 含 `(9,9)` 无内层匹配，内层 `t(id PRIMARY KEY, name)`
- **WHEN** `SELECT o.id FROM a6 o WHERE EXISTS (SELECT 1 FROM t x WHERE x.id = o.ref)`
- **THEN** 仅返回 ref 有匹配的行（修复前：全部外层行放行）

#### Scenario: IN 内层别名谓词走二级索引

- **GIVEN** 外层 `a(id)`，内层 `c(id PRIMARY KEY, age INTEGER)` 有 `age` 二级索引且存在 `age=41`
- **WHEN** `SELECT id FROM a WHERE a.id IN (SELECT x.id FROM c x WHERE x.age = 41)`
- **THEN** 返回匹配行（修复前：恒空集）；内层无别名的同形状结果不变

#### Scenario: WHERE 相关标量比较

- **GIVEN** 外层 `a(id)`，内层 `t(id PRIMARY KEY, name)`
- **WHEN** `SELECT a.id FROM a WHERE (SELECT COUNT(*) FROM t x WHERE x.id = a.id) = 1`
- **THEN** 返回每个有匹配内层行的外层行（修复前：静默空集）

#### Scenario: SELECT 投影位相关标量

- **GIVEN** 同上数据
- **WHEN** `SELECT a.id, (SELECT x.name FROM t x WHERE x.id = a.id) AS who FROM a`
- **THEN** 输出行数与外层一致，who 列为各行匹配名（修复前：塌缩为全部基列或 `Subquery returns multiple rows` 报错）

#### Scenario: 无别名与基名限定形状零回归

- **GIVEN** 上述各查询的无别名内层形态（`FROM t WHERE t.name = ...`）与既有相关子查询用例
- **WHEN** 执行
- **THEN** 结果与修复前一致，既有全量测试零修改通过
