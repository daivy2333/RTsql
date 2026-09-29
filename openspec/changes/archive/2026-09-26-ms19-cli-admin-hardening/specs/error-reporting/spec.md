## ADDED Requirements

### Requirement: 计划错误携带能力名

`PlanError::UnsupportedStatement` 与 `PlanError::UnsupportedExpression` SHALL 携带被拒绝能力的名称（语句种类或表达式种类），CLI 错误信息 SHALL 直接点名不支持的能力（如 `MERGE`、`WINDOW`），使排错无需读源码或试错。错误文案框架与既有点名拒绝变体（`UnsupportedConstraint`、`InSubqueryJoinUnsupported`、`TransactionStatement` 等）的语义 SHALL 保持不变。

#### Scenario: 不支持语句点名能力

- **WHEN** `rtsql db "MERGE INTO t USING s ON t.id = s.id WHEN MATCHED THEN UPDATE SET v = s.v"`
- **THEN** 退出码 3，stderr 错误信息点名 `MERGE`（不再是无上下文的 "Unsupported statement type"）

#### Scenario: 不支持表达式点名能力

- **WHEN** 对既有库执行携带未支持表达式种类的语句（如窗口函数形态）
- **THEN** 退出码 3，stderr 错误信息点名该表达式能力

#### Scenario: 显式拒绝变体语义不变

- **WHEN** 触发既有显式拒绝面（CHECK/FK 约束、IN 子查询 JOIN、会话外事务语句）
- **THEN** 错误信息与本变更前逐字节一致（本变更只补 `UnsupportedStatement`/`UnsupportedExpression` 两变体的能力名）

### Requirement: PG 协议错误码分类

PG 协议错误响应 SHALL 按错误类别携带区分的 SQLSTATE 错误码，SHALL NOT 再对全部错误统一返回 58000。分类 SHALL 至少覆盖：计划期能力面拒绝（不支持语句/表达式/约束/JOIN 类型/子查询位置等）→ `0A000`（feature_not_supported）；解析错误 → `42601`（syntax_error）；表不存在 → `42P01`（undefined_table）；列不存在 → `42703`（undefined_column）；列名歧义 → `42702`（ambiguous_column）；其余未分类错误 SHALL 维持 `58000` 兜底。完整变体映射表随 design 定稿并保持可枚举。

#### Scenario: 能力拒绝返回 0A000

- **GIVEN** 经 PG 协议连接的客户端
- **WHEN** 发送不被支持的语句（如 `MERGE`）
- **THEN** 错误响应携带 SQLSTATE `0A000`（feature_not_supported）且消息点名能力

#### Scenario: 解析错误返回 42601

- **GIVEN** 经 PG 协议连接的客户端
- **WHEN** 发送语法错误的语句
- **THEN** 错误响应携带 SQLSTATE `42601`（syntax_error）

#### Scenario: 名字解析错误按类返回

- **WHEN** 分别发送引用不存在表与不存在列的语句
- **THEN** 错误响应分别携带 `42P01` / `42703`

#### Scenario: 未分类错误维持兜底

- **WHEN** 触发未纳入分类表的内部错误
- **THEN** 错误响应维持 `58000`，不因分类引入新的错误形态

### Requirement: 既有错误语义零回归

CLI 退出码分类（0/1/2/3/4/5）、错误信息的触发条件与优先级、以及各 change 已点名的错误文案 SHALL 保持不变；本变更对错误信息的改变 SHALL 仅为以下白名单：`UnsupportedStatement`/`UnsupportedExpression` 文案追加能力名（既有测试断言按预期校准，不改变任何拒绝行为）；值表达式列缺失文案由 `Parse error: Column 'x' not found in table 't'` 变为 `Column not found: x`（重分类，见「列缺失错误分类一致」）。新增的显式拒绝面（见「投影列解析显式化」「ORDER BY 排序键计划期校验」）SHALL NOT 影响白名单外任何既有可达形态。

#### Scenario: 退出码矩阵不变

- **WHEN** 重复执行既有错误面测试集（SQL 错误 3、锁冲突 4、用法错误 2、库缺失 1、密钥错误 5）
- **THEN** 全部退出码与本变更前一致

#### Scenario: 文案仅追加能力名与白名单重分类

- **WHEN** 对比既有 planner/CLI 错误面用例的断言
- **THEN** 除 `UnsupportedStatement`/`UnsupportedExpression` 两变体追加能力名与列缺失重分类文案外，其余断言无需修改

### Requirement: 投影列解析显式化

纯列查询（无聚合、无表达式项、无标量子查询项）的投影名 SHALL 在计划期逐项解析到基表列：未知裸列、未知限定列（含与已知列混合的投影清单）SHALL 以 `ColumnNotFound` 计划期点名拒绝（CLI exit 3，PG SQLSTATE 42703），SHALL NOT 再静默回落 identity 投影返回全列全数据。`*` 通配与既有可解析形态（子集、重排、重复列、限定有效列）SHALL 逐字节保持。

#### Scenario: 未知裸列拒绝

- **WHEN** `rtsql db "SELECT nosuchcol FROM t"`
- **THEN** exit 3，错误信息点名 `nosuchcol`；经 PG 协议同类语句错误响应携带 SQLSTATE `42703`

#### Scenario: 混合已知与未知列拒绝

- **WHEN** `rtsql db "SELECT id, nosuchcol FROM t"`
- **THEN** exit 3 点名 `nosuchcol`（SHALL NOT 部分投影或返回全列）

#### Scenario: 未知限定列拒绝

- **WHEN** `rtsql db "SELECT t.nosuch FROM t"`
- **THEN** exit 3 点名 `nosuch`

#### Scenario: 可解析形态零回归

- **WHEN** 执行 `SELECT *`、子集列、重排列、重复列、`SELECT t.id FROM t` 等既有可解析形态
- **THEN** 输出（表头、行、顺序）与本变更前逐字节一致

### Requirement: 纯查询列别名表头生效

纯非聚合查询（无聚合、无标量子查询项）中 `col AS alias`（col 为可解析基表列）SHALL 产出表头 `[alias]` 与该列投影行，SHALL NOT 静默回落 identity 投影（表头退化全 schema）。`未知列 AS alias` SHALL 以 `ColumnNotFound` 点名拒绝。聚合查询中的列别名（如 `SELECT col AS n, COUNT(*) ... GROUP BY col`）SHALL 保持既有输出（表头含 `n`）零回归。

#### Scenario: 别名表头与投影行

- **GIVEN** 表 `t(id INT, name STRING)` 含行
- **WHEN** `rtsql db "SELECT name AS n FROM t"`
- **THEN** exit 0，表头 `["n"]`，行仅含 name 列值

#### Scenario: 别名未知列拒绝

- **WHEN** `rtsql db "SELECT nosuch AS x FROM t"`
- **THEN** exit 3 点名 `nosuch`

#### Scenario: 聚合路径别名零回归

- **WHEN** `rtsql db "SELECT name AS n, COUNT(*) FROM t GROUP BY name"`
- **THEN** 表头 `["n","count_star"]` 与本变更前一致

### Requirement: 列缺失错误分类一致

值表达式与 WHERE 路径的列缺失（裸名与两段限定名）SHALL 与多表路径同模建模为 `PlanError::ColumnNotFound`：CLI 文案 `Column not found: {列名}`，PG SQLSTATE `42703`；SHALL NOT 再建模为 `ParseError`（42601）。JOIN 布局路径既有的 `ColumnNotFound` 42703 行为 SHALL 保持。

#### Scenario: WHERE 列缺失分类

- **WHEN** `rtsql db "SELECT id FROM t WHERE nosuch = 1"`（或经 PG 协议发送）
- **THEN** exit 3，文案 `Column not found: nosuch`；PG 错误响应携带 `42703`（SHALL NOT 为 42601）

#### Scenario: 表达式参数列缺失分类

- **WHEN** `rtsql db "SELECT abs(nosuch) FROM t"`
- **THEN** exit 3，文案 `Column not found: nosuch`

#### Scenario: JOIN 路径零回归

- **WHEN** `SELECT t.id FROM t JOIN r ON t.nosuch = r.a`
- **THEN** 行为与文案与本变更前一致（`ColumnNotFound` 42703）

### Requirement: ORDER BY 排序键计划期校验

ORDER BY 的每个排序键 SHALL 在计划期解析：键与 Sort 节点实际可见的排序输入列集或顶层投影项的输出名（`ExprWithAlias` 的别名，大小写不敏感）之一匹配时 SHALL 视为可解析；两者均不匹配的键 SHALL 以 `ColumnNotFound` 点名拒绝（CLI exit 3，PG SQLSTATE 42703），SHALL NOT 静默跳过该键（不排序不报错）。与顶层投影输出名匹配的键（输出别名键）SHALL 维持既有静默语义——不按别名排序、保持输入序、不报错（与已验收语料 `sql-scalar-functions` R5 一致），SHALL NOT 拒绝，SHALL NOT 实现按别名排序。既有可达排序形态 SHALL 零回归：基础扫描链按未投影基础列排序、表达式查询按基础列排序、聚合查询按投影列名排序、输出别名键保持输入序。

#### Scenario: 未知排序键拒绝

- **WHEN** `rtsql db "SELECT id FROM t ORDER BY nosuch"`（或 `ORDER BY nosuch, name DESC` 混合键形态）
- **THEN** exit 3，错误信息点名 `nosuch`；PG 同类语句错误响应携带 `42703`

#### Scenario: 输出别名键维持既有静默语义

- **WHEN** `rtsql db "SELECT upper(name) AS u FROM t ORDER BY u"`（或 `SELECT name AS n FROM t ORDER BY n`）
- **THEN** exit 0，不报错、不按别名排序、行保持输入序（与变更前一致）

#### Scenario: 既有可达排序零回归

- **WHEN** 执行 `SELECT id FROM t ORDER BY name DESC`、`SELECT id + 1 AS d FROM t ORDER BY name DESC`、`SELECT name, COUNT(*) FROM t GROUP BY name ORDER BY count_star` 等既有可达形态
- **THEN** 排序结果与本变更前一致
