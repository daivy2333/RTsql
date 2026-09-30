# error-reporting Specification

## Purpose

定义计划错误与列解析错误面的可操作化语义：`PlanError::UnsupportedStatement`/`UnsupportedExpression` 携带被拒绝能力名，CLI 错误信息直接点名；PG 协议错误响应按类别携带 SQLSTATE（能力拒绝 0A000、解析 42601、表不存在 42P01、列不存在 42703、歧义列 42702，其余 58000 兜底）；计划期列解析静默面显式化——纯列查询未知投影列与未知排序键计划期 `ColumnNotFound` 点名拒绝（不再静默 identity 回落 / 静默不排序）、纯非聚合查询 `col AS alias` 别名表头生效、值表达式与 WHERE 列缺失与多表路径同模建模 `ColumnNotFound`；既有退出码分类与错误文案零回归（白名单限制）。来源：MS19（change `2026-09-26-ms19-cli-admin-hardening`，2026-09-26 收尾）。

## Requirements

### Requirement: 计划错误携带能力名

`PlanError::UnsupportedStatement` 与 `PlanError::UnsupportedExpression` SHALL 携带被拒绝能力的名称（语句种类或表达式种类），CLI 错误信息 SHALL 直接点名不支持的能力（如 `MERGE`、`WINDOW`），使排错无需读源码或试错。错误文案框架与既有点名拒绝变体（`UnsupportedConstraint`、`InSubqueryJoinUnsupported`、`TransactionStatement` 等）的语义 SHALL 保持不变。

**MS25 Iteration 004 二次 replan（T13，2026-09-28 用户指令并入）修改**：限定通配投影项（`SELECT t.*`）的拒绝文案由泛化的 `SELECT ITEM` 修订为能力点名——修订前该形态在单表面/派生表/JOIN 三基座均以 `Unsupported statement type: SELECT ITEM` 拒绝（诚实但不可操作）。

#### Scenario: 不支持语句点名能力

- **WHEN** `rtsql db "MERGE INTO t USING s ON t.id = s.id WHEN MATCHED THEN UPDATE SET v = s.v"`
- **THEN** 退出码 3，stderr 错误信息点名 `MERGE`（不再是无上下文的 "Unsupported statement type"）

#### Scenario: 不支持表达式点名能力

- **WHEN** 对既有库执行携带未支持表达式种类的语句（如窗口函数形态）
- **THEN** 退出码 3，stderr 错误信息点名该表达式能力

#### Scenario: 显式拒绝变体语义不变

- **WHEN** 触发既有显式拒绝面（CHECK/FK 约束、IN 子查询 JOIN、会话外事务语句）
- **THEN** 错误信息与本变更前逐字节一致（本变更只补 `UnsupportedStatement`/`UnsupportedExpression` 两变体的能力名）

#### Scenario: 限定通配点名拒绝（T13 新增）

- **GIVEN** 任意库（单表面、派生表、JOIN 基座同）
- **WHEN** `rtsql db "SELECT t.* FROM t"`（或 `SELECT p.* FROM p JOIN q ON …`）
- **THEN** 退出码 3，错误信息点名限定通配能力（如 `Unsupported statement type: QUALIFIED WILDCARD`，不再是无上下文的 "SELECT ITEM"）

### Requirement: PG 协议错误码分类

PG 协议错误响应 SHALL 按错误类别携带区分的 SQLSTATE 错误码，SHALL NOT 再对全部错误统一返回 58000。分类 SHALL 至少覆盖：计划期能力面拒绝（不支持语句/表达式/约束/JOIN 类型/子查询位置等）→ `0A000`（feature_not_supported）；解析错误 → `42601`（syntax_error）；表不存在 → `42P01`（undefined_table）；列不存在 → `42703`（undefined_column）；列名歧义 → `42702`（ambiguous_column）；只读状态下的写操作拒绝（`readonly-open-mode` 域）→ `25006`（read_only_sql_transaction）；其余未分类错误 SHALL 维持 `58000` 兜底。完整变体映射表随 design 定稿并保持可枚举。

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

#### Scenario: 只读拒绝返回 25006

- **GIVEN** 只读会话
- **WHEN** 执行任意 DML/DDL 写语句
- **THEN** 错误响应 SQLSTATE 为 `25006`，错误文案点名只读状态

### Requirement: 既有错误语义零回归

CLI 退出码分类（0/1/2/3/4/5）、错误信息的触发条件与优先级、以及各 change 已点名的错误文案 SHALL 保持不变；本变更对错误信息的改变 SHALL 仅为以下白名单：`UnsupportedStatement`/`UnsupportedExpression` 文案追加能力名（既有测试断言按预期校准，不改变任何拒绝行为）；值表达式列缺失文案由 `Parse error: Column 'x' not found in table 't'` 变为 `Column not found: x`（重分类，见「列缺失错误分类一致」）。新增的显式拒绝面（见「投影列解析显式化」「ORDER BY 排序键解析与别名排序」）SHALL NOT 影响白名单外任何既有可达形态。

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

纯非聚合查询（无聚合、无标量子查询项）中 `col AS alias`（col 为可解析基表列或 DerivedScan 基座〔CTE / FROM 子查询〕注册列）SHALL 产出表头 `[alias]` 与该列投影行，SHALL NOT 静默回落 identity 投影（表头退化全 schema），SHALL NOT 丢失别名（表头回落源列名）。`未知列 AS alias` SHALL 以 `ColumnNotFound` 点名拒绝。聚合查询中的列别名（如 `SELECT col AS n, COUNT(*) ... GROUP BY col`）SHALL 保持既有输出（表头含 `n`）零回归。

**MS25 Iteration 004 replan（T10，2026-09-28 用户指令并入）修改**：基座范围由 Scan 扩展至 DerivedScan——修订前 CTE / FROM 子查询基座的普通列别名项表头回落源列名（`alias_reroutable` 门仅覆盖 Scan 基座的 MS19 T4b 保守遗漏），属静默错误结果同类，随本修订终止。

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

#### Scenario: CTE 基座别名表头（T10 新增）

- **GIVEN** 表 `t(v STRING)` 含行
- **WHEN** `rtsql db "WITH c AS (SELECT v FROM t) SELECT v AS val FROM c"`
- **THEN** exit 0，表头 `["val"]`，行含 v 列值（修订前表头回落 `["v"]`）

#### Scenario: FROM 子查询基座别名表头（T10 新增）

- **GIVEN** 表 `t(v STRING)` 含行
- **WHEN** `rtsql db "SELECT v AS val FROM (SELECT v FROM t) AS c"`
- **THEN** exit 0，表头 `["val"]`，行含 v 列值（修订前表头回落 `["v"]`）

### Requirement: 列缺失错误分类一致

值表达式、WHERE 路径与聚合函数参数的列缺失（裸名与两段限定名）SHALL 同模建模为 `PlanError::ColumnNotFound`：CLI 文案 `Column not found: {列名}`，PG SQLSTATE `42703`；SHALL NOT 再建模为 `ParseError`（42601）。JOIN 布局路径既有的 `ColumnNotFound` 42703 行为 SHALL 保持。

**MS25 Iteration 004 二次 replan（T12，2026-09-28 用户指令并入）修改**：聚合函数参数的列解析由「运行期名字查表、查无静默 NULL」修订为计划期解析——修订前 `SELECT count(nosuch) FROM t` 静默返回 `[[0]]`（单表面与 JOIN 基座同），聚合参数引用 SELECT 输出别名（如 `count(u)`，`u` 为别名）静默 0；修订后两类形态均计划期 `ColumnNotFound` 点名（聚合参数 SHALL 引用输入基列）。GROUP BY 分组键的列缺失维持既有解析链终态（`NonAggregatedColumn` 点名）不变。

#### Scenario: WHERE 列缺失分类

- **WHEN** `rtsql db "SELECT id FROM t WHERE nosuch = 1"`（或经 PG 协议发送）
- **THEN** exit 3，文案 `Column not found: nosuch`；PG 错误响应携带 `42703`（SHALL NOT 为 42601）

#### Scenario: 表达式参数列缺失分类

- **WHEN** `rtsql db "SELECT abs(nosuch) FROM t"`
- **THEN** exit 3，文案 `Column not found: nosuch`

#### Scenario: JOIN 路径零回归

- **WHEN** `SELECT t.id FROM t JOIN r ON t.nosuch = r.a`
- **THEN** 行为与文案与本变更前一致（`ColumnNotFound` 42703）

#### Scenario: 聚合参数列缺失点名（T12 新增）

- **WHEN** `rtsql db "SELECT count(nosuch) FROM t"`（单表面；JOIN 基座 `SELECT count(nosuch) FROM p JOIN q ON …` 同）
- **THEN** exit 3，文案 `Column not found: nosuch`；PG 错误响应携带 `42703`（修订前静默返回 `[[0]]`）

#### Scenario: 聚合参数限定名列缺失点名（T12 新增）

- **WHEN** `rtsql db "SELECT sum(t1.nosuch) FROM t1 JOIN t2 ON t1.k = t2.k"`
- **THEN** exit 3，文案 `Column not found: nosuch`（修订前静默返回 `[[null]]`）

#### Scenario: 聚合参数引用输出别名点名（T12 新增）

- **WHEN** `rtsql db "SELECT upper(b) AS u, count(u) FROM t GROUP BY u"`
- **THEN** exit 3，文案 `Column not found: u`（修订前静默返回 `[[0]]`——聚合参数引用 SELECT 输出名属列缺失，SHALL 引用输入基列）

#### Scenario: 既有聚合零回归（T12 新增）

- **WHEN** 执行 `SELECT count(*) FROM t`、`SELECT sum(v) FROM t`、`SELECT count(DISTINCT v) FROM t`、聚合 × GROUP BY/HAVING 既有形态
- **THEN** 结果与本变更前一致

### Requirement: ORDER BY 排序键解析与别名排序

ORDER BY 的每个排序键 SHALL 在计划期解析：键与顶层投影项的输出名（`ExprWithAlias` 的别名与 Display 列名，大小写不敏感）或 Sort 节点实际可见的排序输入列集之一匹配时 SHALL 视为可解析；两者均不匹配的键 SHALL 以 `ColumnNotFound` 点名拒绝（CLI exit 3，PG SQLSTATE 42703），SHALL NOT 静默跳过该键（不排序不报错）。与顶层投影输出名匹配的键（输出别名键）SHALL 按输出列真实排序（输出名优先遮蔽语义，详见 `order-by-alias-sorting` 能力）——原「不按别名排序、保持输入序、不报错」的静默语义终止。既有可达排序形态 SHALL 零回归：基础扫描链按未投影基础列排序、表达式查询按基础列排序、聚合查询按投影列名排序。

#### Scenario: 未知排序键拒绝

- **WHEN** `rtsql db "SELECT id FROM t ORDER BY nosuch"`（或 `ORDER BY nosuch, name DESC` 混合键形态）
- **THEN** exit 3，错误信息点名 `nosuch`；PG 同类语句错误响应携带 `42703`

#### Scenario: 输出别名键按输出列真实排序

- **WHEN** `rtsql db "SELECT upper(name) AS u FROM t ORDER BY u"`（或 `SELECT name AS n FROM t ORDER BY n`）
- **THEN** exit 0，按输出列真实排序（语义详见 `order-by-alias-sorting`）

#### Scenario: 既有可达排序零回归

- **WHEN** 执行 `SELECT id FROM t ORDER BY name DESC`、`SELECT id + 1 AS d FROM t ORDER BY name DESC`、`SELECT name, COUNT(*) FROM t GROUP BY name ORDER BY count_star` 等既有可达形态
- **THEN** 排序结果与本变更前一致
