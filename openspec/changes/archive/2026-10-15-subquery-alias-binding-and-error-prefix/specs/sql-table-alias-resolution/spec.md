# Delta: sql-table-alias-resolution

## ADDED Requirements

### Requirement: 子查询内层声明限定符参与外层引用分类

子查询（IN / EXISTS / 标量）内层 FROM 项的可解析限定符集 SHALL 含每个 FROM 项的声明名：显式或隐式别名优先，无别名取基表名；声明了别名时 SHALL 同时保留基表名作为可解析限定符——本引擎顶层对「别名 + 基名限定」本就宽松，子查询层保持同一宽松度（「别名 + 基名限定」子查询形状由既有正确结果退化为错误不得发生）。JOIN 臂 SHALL 同规则；`TableFactor::Derived`（派生表）SHALL 取其别名。相关参数分类（`collect_outer_column_refs`/`extract_correlated_params`）与 `ParameterExpression` 改写（`qualified_column_expression`）SHALL 以该限定符集判断内外——内层别名限定引用（如 `FROM c x ... x.age`）SHALL 解析为内层表列，SHALL NOT 被误判为相关外层引用产生幻影参数。修复前：内层集丢弃别名，`x.age` 被改写为幻影 `ParameterExpression` 并走相关执行路径，产生恒空集 / 按内层存活行数截断外层 / 全放行 / 碰巧正确四种静默错误结果（ISS16/ISS17/ISS18）。

> 基表名并列保留一项由 2026-10-15 用户豁免裁定（Gate 5 Blocker Handoff 方案一），见 Cycle Act Response 的 Blocker Resolution；对应回归锚点 `a1_base_name_qualifier_inside_aliased_inner`。

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
