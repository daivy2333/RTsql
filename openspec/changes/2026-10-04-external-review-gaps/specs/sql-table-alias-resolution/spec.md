## ADDED Requirements

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

## MODIFIED Requirements

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
