## MODIFIED Requirements

### Requirement: 视图解析与计划一致性

（既有主体保持——CTE 通道展开与内联派生表逐字节一致、plan cache 全清、组合面零特殊化。）本次将该一致性收口为「三种通道均以派生表语义构建」：视图定义内的别名列与表达式（计算）列 SHALL 与 FROM 派生表、CTE 通道产出一致的可查询输出列（内层顶层 Projection 路由在视图展开的派生上下文放行）。视图定义含 DISTINCT SHALL 真实去重。既有裸列名视图、显式列清单视图、嵌套视图、DML 只读拒绝面 SHALL 零回归。

#### Scenario: 视图与内联派生表逐字节一致（含别名/表达式列）

- **GIVEN** `CREATE VIEW vw AS SELECT age AS v, CASE … END AS tier FROM people` 与同定义内联派生表、同定义 CTE
- **WHEN** 三通道分别 `SELECT v, tier FROM …`
- **THEN** 输出逐字节一致（失败边界——修复前：CTE ✓ / 派生表 ✗ / 视图 ✗——与「逐字节一致」声明矛盾）

#### Scenario: 视图定义含 DISTINCT 真实去重

- **GIVEN** `CREATE VIEW dv AS SELECT DISTINCT dept FROM people`
- **WHEN** `SELECT * FROM dv`
- **THEN** 去重结果（失败边界——修复前：展开上下文 DISTINCT 被静默跳过）

#### Scenario: 既有视图面零回归

- **GIVEN** 裸列名视图、显式列清单视图、嵌套视图
- **WHEN** 查询与 DML 拒绝
- **THEN** 全部行为与修复前一致
