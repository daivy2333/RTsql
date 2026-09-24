# sql-scalar-functions Specification（delta）

## MODIFIED Requirements

### Requirement: 函数注册与分派机制

标量函数调用 SHALL 经统一的注册与分派机制处理：函数名匹配 SHALL 大小写不敏感（大写/混合大小写调用形态与小写形态结果与错误面逐字节一致，SQL 层 e2e 见证锁定）；参数个数 SHALL 在 plan 期校验，不符时报 SQL 错误且错误信息点名函数名与期望参数个数。以下形态 SHALL 在 plan 期显式拒绝为 SQL 错误且错误信息点名不被支持的构造，SHALL NOT 静默降级或部分执行：`OVER` 窗口子句、`DISTINCT` 限定、`FILTER (WHERE ...)` 子句、命名参数（`name => value`）、通配符参数（`*`）、零参调用（`now` 等零参注册函数不在此列）。未注册的函数名 SHALL 维持既有拒绝行为且文案逐字节不变：SELECT 投影位置为既有 `Unsupported statement type`（`ast.rs` extract_columns 放行门先于 planner 函数臂），谓词与值表达式位置为既有 `Unsupported expression type`。注册表 SHALL 与聚合五函数（COUNT/SUM/AVG/MIN/MAX）互斥：聚合名继续走聚合路径，SHALL NOT 进入标量分派。

#### Scenario: 未知名维持既有文案

- **GIVEN** 表 `t(id INT PRIMARY KEY)` 存在
- **WHEN** `SELECT nonexistent_fn(id) FROM t`
- **THEN** 报 SQL 错误，错误信息为既有 `Unsupported statement type`（与 change 前逐字节一致）

#### Scenario: OVER 窗口子句拒绝

- **GIVEN** 表 `t(name VARCHAR)` 存在
- **WHEN** `SELECT upper(name) OVER () FROM t`
- **THEN** 报 SQL 错误，错误信息点名 `OVER`（窗口函数）不被支持

#### Scenario: DISTINCT 限定拒绝

- **GIVEN** 表 `t(name VARCHAR)` 存在
- **WHEN** `SELECT upper(DISTINCT name) FROM t`
- **THEN** 报 SQL 错误，错误信息点名 `DISTINCT` 不被支持

#### Scenario: arity 错误 plan 期拒绝

- **GIVEN** 表 `t(name VARCHAR)` 存在
- **WHEN** `SELECT substr(name) FROM t`
- **THEN** 报 SQL 错误（plan 期，不执行），错误信息点名 `substr` 与参数个数要求

#### Scenario: 大小写变体 SQL 层等价（I044）

- **GIVEN** 表 `t(name VARCHAR)` 含一行 `name='abc'`、`t2(i INT)` 含一行 `i=-5`
- **WHEN** `SELECT UPPER(name) FROM t`、`SELECT Abs(i) FROM t2`、`SELECT MiXeD_Length(name) FROM t` 等大写/混合形态
- **THEN** 与小写形态结果与错误面逐字节一致（表头回放书写形态的既有语义不受影响）

### Requirement: math 函数四件

`abs(x)` SHALL 返回绝对值并保持入参类型（Int→Int、Float→Float）；Int 入参为 `i64::MIN` 时 SHALL 报运行时溢出错误（显式拒绝，SHALL NOT panic 或回绕为负）；`round(x[, digits])` SHALL 按半数远离零（half-away-from-zero）舍入并返回 Float（`round(3.7)=4.0`、`round(2.5)=3.0`、`round(-2.5)=-3.0`、`round(3.14159,2)=3.14`）；`digits` SHALL 接受 1 或 2 个参数形态，`digits<0` 按整数位舍入（`round(123.4,-1)=120.0`），`digits` 为 Float 时按向零截断取整后使用；`digits` 超出 f64 数量级表示范围时 SHALL SQLite 对齐饱和：`digits` 正超界（如 1000）返回入参的 Float 形态（`round(1,1000)=1.0`），`digits` 负超界（如 -1000）返回 0.0，SHALL NOT 产出 inf/NaN。`floor(x)`/`ceil(x)` SHALL 返回 Float（`floor(3.7)=3.0`、`ceil(3.2)=4.0`、`floor(-3.7)=-4.0`）。四个函数入参 SHALL 严格校验为 Int 或 Float，其他类型报运行时类型错误；`digits` 参数 SHALL 严格校验为 Int 或 Float。

#### Scenario: abs 同型返回

- **GIVEN** 表 `t(i INT, f FLOAT)` 含一行 `i=-5, f=-5.5`
- **WHEN** `SELECT abs(i), abs(f) FROM t`
- **THEN** 输出 `5`（Int）与 `5.5`（Float）

#### Scenario: abs i64::MIN 溢出显式错误（I043）

- **GIVEN** 表 `t(i INT)` 含一行 `i=-9223372036854775808`
- **WHEN** `SELECT abs(i) FROM t`
- **THEN** 报运行时溢出错误（显式错误信息），SHALL NOT panic、SHALL NOT 回绕为负

#### Scenario: round 舍入方向与 digits

- **GIVEN** 表 `t(f FLOAT)` 含一行 `f=3.14159`
- **WHEN** 依次执行 `SELECT round(3.7) FROM t`、`SELECT round(2.5) FROM t`、`SELECT round(-2.5) FROM t`、`SELECT round(f, 2) FROM t`、`SELECT round(123.4, -1) FROM t`
- **THEN** 依次输出 `4.0`、`3.0`（半数远离零）、`-3.0`、`3.14`、`120.0`（负 digits 整数位舍入）

#### Scenario: round 极端 digits 饱和（I043）

- **GIVEN** 任意表
- **WHEN** `SELECT round(1, 1000)`、`SELECT round(1, -1000)`、`SELECT round(2.5, 400)`
- **THEN** 依次输出 `1.0`、`0.0`、`2.5`（SQLite 对齐饱和，无 inf/NaN）

#### Scenario: floor/ceil 返回 Float

- **GIVEN** 表 `t(f FLOAT)` 含一行 `f=3.2` 与一行 `f=-3.7`
- **WHEN** `SELECT floor(f), ceil(f) FROM t`
- **THEN** 输出 `3.0, 4.0` 与 `-4.0, -3.0`（均为 Float 形态）
