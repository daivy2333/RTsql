## REMOVED Requirements

### Requirement: ORDER BY 排序键计划期校验

**Reason**: T06（I075，用户批准 2026-09-26）升级排序键解析语义——原 Requirement 锁定「输出别名键静默保持输入序、SHALL NOT 实现按别名排序」，该语义由 ADDED Requirement「ORDER BY 排序键解析与别名排序」替换；未知键计划期拒绝与既有可达排序零回归两项语义由新 Requirement 完整承载，行为承诺不变。

## ADDED Requirements

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

## MODIFIED Requirements

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

## MODIFIED Requirements

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
