## MODIFIED Requirements

### Requirement: 纯等值 JOIN Hash 路径保持

ON 条件 AND 分解后全部腿均为可解析的列=列等值（既有 `extract_join_conditions` 接受面），且该连接步不满足「整数主键基表扫描走排序合并」时，SHALL 继续路由既有 Hash Join 执行器。两侧都是基表扫描且每个连接键都是该表写出的整数 `PRIMARY KEY` 的内连接，不由本 Requirement 路由。没有写出 `PRIMARY KEY` 的整数首列由本 Requirement 路由到哈希。本 Requirement 不覆盖混合腿（等值 + 非等值）——混合腿按既有规则路由嵌套循环连接。结果的多重集合 SHALL 与变更前一致，但连接键中的 `Float(-0.0)` 与 `Float(0.0)` 按 `PartialEq` 为同一键：两侧以此配对时 SHALL 命中，输出单元格保留输入的原始值。其余键的命中与变更前一致。有 `ORDER BY` 时，行序 SHALL 由 `ORDER BY` 决定。无 `ORDER BY` 时：两侧输入行数相等，或较小构建侧接线已撤回时，行序 SHALL 与变更前一致（构建右输入、探测左输入）；两侧行数不等且接线保留时，行序 SHALL 以行数较多的一侧为探测顺序，同一探测行上的匹配行保持构建侧进入顺序。

#### Scenario: 等值 INNER JOIN 形状与结果保持

- **GIVEN** 表 r(a INT)、s(b INT)，列定义里没有写出 `PRIMARY KEY`。目录仍把各自的首列登记为整数键列
- **WHEN** `SELECT ... FROM r JOIN s ON r.a = s.b`（含 AND 复合纯等值）
- **THEN** 计划选择 Hash Join，多重集合与变更前一致

#### Scenario: 重新打开后隐式首列仍走哈希

- **GIVEN** 表 r(a INT)、s(b INT) 已经建好并关闭，列定义里没有写出 `PRIMARY KEY`
- **WHEN** 重新打开后执行 `r JOIN s ON r.a = s.b`
- **THEN** 计划选择 Hash Join

#### Scenario: 负零与正零在哈希连接中命中

- **GIVEN** 留在哈希路径上的纯等值内连接，一侧键为 `Float(-0.0)`，另一侧键为 `Float(0.0)`
- **WHEN** 执行该哈希连接
- **THEN** 这两行命中，输出中的两个浮点分别保持各自的原始位模式

#### Scenario: 非主键列仍走哈希

- **GIVEN** `customers(id INT PRIMARY KEY, name STRING)` 与 `orders(id INT PRIMARY KEY, customer_id INT, amount INT)`
- **WHEN** `orders.customer_id = customers.id`
- **THEN** 计划选择 Hash Join

#### Scenario: 字符串主键仍走哈希

- **GIVEN** 两侧基表的连接键列都是已声明的字符串主键
- **WHEN** 以该列做等值内连接
- **THEN** 计划选择 Hash Join

#### Scenario: 连接结果再连接仍走哈希

- **GIVEN** 三张基表都以整数主键等值相连
- **WHEN** 执行这条三表内连接
- **THEN** 第一步两个基表扫描走排序合并，第二步的左输入已是连接结果，该步选择 Hash Join

#### Scenario: 行数相等时行序不变

- **GIVEN** 留在哈希路径上的纯等值内连接，两侧输入行数相等，且没有 `ORDER BY`
- **WHEN** 执行该语句
- **THEN** 行序与变更前的左输入探测序一致

#### Scenario: 右输入更大且接线保留

- **GIVEN** 留在哈希路径上的纯等值连接，左输入行数少于右输入，没有 `ORDER BY`，较小构建侧接线仍保留
- **WHEN** 执行该哈希连接
- **THEN** 多重集合与构建右输入时相同，行序按右输入逐行探测

#### Scenario: 时间门未过

- **GIVEN** 左表 1000 行、右表 100000 行、连接键不是两侧整数主键的等值连接基准，其平均时间没有比「构建右输入」基线至少低 10%
- **WHEN** 结束构建侧这一轮
- **THEN** 生产路径仍构建右输入，无 `ORDER BY` 的行序与变更前一致

## ADDED Requirements

### Requirement: 整数主键基表扫描走排序合并

内连接的 ON 条件是列=列等值，左右输入在该连接步都是基表扫描，且每一条等值键列都是该表写出的整数 `PRIMARY KEY` 时，计划 SHALL 选择排序合并连接。写出指建表语句里的列级或表级 `PRIMARY KEY`，并且该列的声明类型为整数。目录把隐式首列登记成整数键列，不构成写出。字符串主键、没有写出 `PRIMARY KEY` 的整数首列、只作为普通列出现的整数列，都不满足。多一条非主键等值腿时，这一步 SHALL 仍选择哈希连接。只写出一侧时，这一步 SHALL 仍选择哈希连接。CTE、视图、派生表以及上一层连接的输出都不是基表扫描。`ORDER BY` SHALL NOT 参与这个选择，也 SHALL NOT 因此去掉连接之上的 Sort。无 `ORDER BY` 时，本 Requirement 不规定行序。关闭数据库再打开后，建表时写入的写出与否 SHALL 保持。缺这个记录的旧目录行 SHALL 不满足本 Requirement。

#### Scenario: 两侧整数主键

- **GIVEN** `customers(id INT PRIMARY KEY)` 与 `orders(id INT PRIMARY KEY)`，两侧都是基表扫描
- **WHEN** `customers.id = orders.id`
- **THEN** 该连接节点是排序合并，输出多重集合与同一输入上的哈希连接相同

#### Scenario: 重新打开后写出的整数主键仍走排序合并

- **GIVEN** `customers(id INT PRIMARY KEY)` 与 `orders(id INT PRIMARY KEY)` 已经建好并关闭
- **WHEN** 重新打开后执行 `customers.id = orders.id`
- **THEN** 该连接节点仍是排序合并

#### Scenario: 自连接

- **GIVEN** 一张写出了整数 `PRIMARY KEY` 的基表，以两个别名自连接
- **WHEN** 两个别名的主键列等值内连接
- **THEN** 计划选择排序合并

#### Scenario: ORDER BY 不改变选择

- **GIVEN** 两侧都写出了整数 `PRIMARY KEY` 的基表扫描等值内连接，并且带有 `ORDER BY`
- **WHEN** 构建计划并执行
- **THEN** 连接节点仍是排序合并，其上仍有 Sort，结果次序由 `ORDER BY` 决定

### Requirement: 排序合并连接的多重集合与哈希连接相同

对同一对输入和同一组列=列等值条件，排序合并连接 SHALL 产出与哈希连接相同的行多重集合。任一侧没有行时 SHALL 得到空结果且不报错。连接键任一列为 NULL 的行 SHALL 不进入内连接结果。键的相等 SHALL 与哈希连接使用的值相等一致，并且与 `Value` 的 `PartialEq` 一致：变体不同即不相等，因此整数与同数值的浮点数不匹配；`Float(-0.0)` 与 `Float(0.0)` 相等，两边都命中。同一键的重复行 SHALL 按两侧该键行数的乘积进入多重集合。组内行序不保证。输出单元格保留输入的原始值。

#### Scenario: 重复键的多重集合

- **GIVEN** 左输入在键 1 上有 2 行，右输入在键 1 上有 2 行
- **WHEN** 以该等值条件执行排序合并连接
- **THEN** 结果含 4 行匹配，与同一输入上的哈希连接多重集合相同

#### Scenario: NULL 键不匹配

- **GIVEN** 左右输入的连接键都含 NULL
- **WHEN** 执行排序合并内连接
- **THEN** NULL 行不出现在结果中，包括 NULL 与 NULL

#### Scenario: 负零与正零两边一致

- **GIVEN** 左键为 `Float(-0.0)`，右键为 `Float(0.0)`
- **WHEN** 对同一输入分别执行哈希连接和排序合并连接
- **THEN** 两边都产出这对行的匹配，多重集合相同，输出中的浮点保持输入的位模式

#### Scenario: 整数与浮点数不因数值相同而匹配

- **GIVEN** 左键为整数 2，右键为浮点数 2.0
- **WHEN** 执行排序合并连接
- **THEN** 这两行不匹配，与哈希连接一致

#### Scenario: 空输入

- **GIVEN** 任一侧输入没有行
- **WHEN** 执行排序合并连接
- **THEN** 空结果，不报错

### Requirement: 外连接与非等值仍走嵌套循环

左外连接、右外连接、全外连接，以及含非等值、字面量或表达式腿的 `ON`，SHALL 继续使用嵌套循环连接。排序合并连接 SHALL NOT 承接这些语句。两腿引用同一侧的 `ON` SHALL 继续在计划期拒绝。

#### Scenario: 不等式连接的计划不变

- **GIVEN** 表 r(a INT)、s(b INT)
- **WHEN** `SELECT r.a, s.b FROM r JOIN s ON r.a < s.b`
- **THEN** 计划为嵌套循环连接，结果与变更前一致

#### Scenario: 外连接的计划不变

- **GIVEN** 既有左外、右外或全外连接语句
- **WHEN** 构建计划并执行
- **THEN** 计划为嵌套循环连接，结果与变更前一致

### Requirement: 排序合并节点可被计划文本识别

计划树文本在渲染排序合并连接节点时 SHALL 包含 `SortMergeJoin`，并写出等值条件。没有被「整数主键基表扫描走排序合并」选中的语句 SHALL NOT 因此改变现有 `Join [Inner]` 或 `NestedLoopJoin` 文本。

#### Scenario: 节点文本

- **GIVEN** 一棵只含排序合并连接及其输入的计划
- **WHEN** 渲染计划树文本
- **THEN** 对应行包含 `SortMergeJoin` 和等值条件

#### Scenario: 语句文本

- **GIVEN** 一条被选中的整数主键基表等值内连接
- **WHEN** `EXPLAIN` 该语句
- **THEN** 计划文本含 `SortMergeJoin` 和等值条件

### Requirement: 排序合并连接遵守查询内存上限

排序合并连接驻留的输入行 SHALL 计入既有语句级查询内存预算。超过上限时 SHALL 整句失败，SQLSTATE 为 `53200`，且不返回部分结果行。未设置上限时 SHALL 与现在一样不因预算失败。

#### Scenario: 超过上限

- **GIVEN** 语句内存上限小于排序合并连接需要驻留的两侧输入
- **WHEN** 执行该连接
- **THEN** 语句失败，SQLSTATE 为 `53200`，没有部分结果行

### Requirement: IN 子查询继续拒绝连接形状

`IN` 子查询的内部计划若是排序合并连接，SHALL 与哈希连接一样被拒绝，错误为 `IN subquery with JOIN is not supported`。SHALL NOT 改报成子查询返回多列。

#### Scenario: 整数主键连接出现在 IN 子查询中

- **GIVEN** 子查询内部是两侧整数主键基表的等值内连接
- **WHEN** 外层 `IN` 使用这个子查询
- **THEN** 语句失败，错误含 `IN subquery with JOIN is not supported`
