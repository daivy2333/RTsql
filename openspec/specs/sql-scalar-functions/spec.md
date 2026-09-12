# sql-scalar-functions Specification

## Purpose
SQL 标量函数能力（第一批，string 六件 + math 四件）。十个标量函数在 WHERE 谓词与 SELECT 派生列双侧可用：string 六件（`upper`/`lower`/`length`/`substr`/`replace`/`trim`——严格字符串类型无隐式转换、Unicode 字符计数、substr 边缘对齐 SQLite（start=0 幻影位/负 start 尾数/负 len 前取）、replace 空 from 原样、trim 仅剥 U+0020）与 math 四件（`abs` 同型返回、`round` 半数远离零 + digits 整数位舍入/Float 向零截断、`floor`/`ceil` 返回 Float）。函数经单点注册表（`src/executor/function.rs`）驱动 plan 期校验与执行期分派：名称大小写不敏感、arity plan 期校验，OVER/DISTINCT/FILTER/命名参数/通配符/零参显式点名拒绝；未注册名维持既有拒绝文案，聚合五名不进标量分派。任一参数 NULL → NULL 且跳过类型校验，参数求值错误先于 NULL 检查传播，参数接受任意值表达式（嵌套函数/COALESCE/CAST/负数字面量）。WHERE 下推/Filter/OR 组合三路径一致，聚合混用与 HAVING 标量引用保持既有拒绝，ORDER BY 表达式别名静默保持输入序（既有语义文档化）。来源：MS11-T03（change `2026-09-10-ms11-t03-scalar-functions`，2026-09-11 归档）。

## Requirements

### Requirement: 函数注册与分派机制

标量函数调用 SHALL 经统一的注册与分派机制处理：函数名匹配 SHALL 大小写不敏感；参数个数 SHALL 在 plan 期校验，不符时报 SQL 错误且错误信息点名函数名与期望参数个数。以下形态 SHALL 在 plan 期显式拒绝为 SQL 错误且错误信息点名不被支持的构造，SHALL NOT 静默降级或部分执行：`OVER` 窗口子句、`DISTINCT` 限定、`FILTER (WHERE ...)` 子句、命名参数（`name => value`）、通配符参数（`*`）、零参调用。未注册的函数名 SHALL 维持既有拒绝行为且文案逐字节不变：SELECT 投影位置为既有 `Unsupported statement type`（`ast.rs` extract_columns 放行门先于 planner 函数臂），谓词与值表达式位置为既有 `Unsupported expression type`。注册表 SHALL 与聚合五函数（COUNT/SUM/AVG/MIN/MAX）互斥：聚合名继续走聚合路径，SHALL NOT 进入标量分派。

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

### Requirement: string 函数六件

`upper(s)`/`lower(s)` SHALL 返回 ASCII 大写/小写形式；`length(s)` SHALL 返回字符数（按 Unicode scalar 计数，非字节数）；`substr(s, start[, len])` SHALL 按 1-based 字符索引取子串，边缘参数对齐 SQLite：`start=0` 与 `start=1` 等价但结果长度少一（`substr('abc',0,2)='a'`）、`start<0` 从尾部倒数（`substr('abc',-2)='bc'`）、`len<0` 返回从第 `start` 个字符往前的 `|len|` 个字符（`substr('abc',2,-1)='a'`）、省略 `len` 取到串尾；`replace(s, from, to)` SHALL 把 `s` 中所有 `from` 子串替换为 `to`（`from` 为空串时原样返回）；`trim(s)` SHALL 仅剥离首尾空格（U+0020），TAB、换行保留。六个函数 SHALL 接受恰好声明的参数个数（substr 2 或 3 参，replace 3 参，其余 1 参）。入参类型 SHALL 严格校验为字符串（列、字面量或表达式求值结果），非字符串入参 SHALL 报运行时类型错误，SHALL NOT 隐式转换。SELECT 列表中的函数项 SHALL 以书写形态回放为默认列名（如 `upper(name)`），`AS 别名` SHALL 覆盖默认列名。

#### Scenario: upper/lower 派生列与默认表头

- **GIVEN** 表 `t(name VARCHAR)` 含一行 `name='AbC'`
- **WHEN** `SELECT upper(name), lower(name) FROM t`
- **THEN** 输出一行两列：`ABC`、`abc`；列名分别为 `upper(name)`、`lower(name)`（按书写形态回放）

#### Scenario: AS 别名列名

- **GIVEN** 表 `t(name VARCHAR)` 含一行 `name='abc'`
- **WHEN** `SELECT upper(name) AS u FROM t`
- **THEN** 输出一行 `ABC`，列名为 `u`

#### Scenario: length 字符计数 + WHERE 过滤

- **GIVEN** 表 `t(name VARCHAR)` 含 `name='你好'` 与 `name='abc'` 两行
- **WHEN** `SELECT name, length(name) FROM t WHERE length(name) > 2`
- **THEN** 仅输出 `abc` 行（`你好` 字符数为 2，不满足 `>2`；`abc` 为 3）

#### Scenario: substr 常规与 SQLite 边缘

- **GIVEN** 表 `t(s VARCHAR)` 含一行 `s='abcdef'`
- **WHEN** 依次执行 `SELECT substr(s, 2) FROM t`、`SELECT substr(s, 2, 3) FROM t`、`SELECT substr(s, 0, 2) FROM t`、`SELECT substr(s, -2) FROM t`、`SELECT substr(s, 3, -1) FROM t`
- **THEN** 依次输出 `bcdef`、`bcd`、`a`、`ef`、`b`（省略 len 取到尾；start=0 少一；负 start 尾部倒数；负 len 往前取）

#### Scenario: replace 与 trim（仅空格）

- **GIVEN** 表 `t(s VARCHAR)` 含一行 `s='a-b-a'` 与另一行 `s='  x '`（首尾各空格）
- **WHEN** `SELECT replace(s, '-', '+') FROM t WHERE s = 'a-b-a'`
- **THEN** 输出 `a+b+a`
- **AND** `SELECT trim(s) FROM t WHERE s = '  x '` 输出 `x`（长度 1）；构造 `s` 含 TAB 时 TAB 保留

#### Scenario: 严格类型错误

- **GIVEN** 表 `t(id INT PRIMARY KEY)` 含一行 `id=123`
- **WHEN** `SELECT upper(id) FROM t`
- **THEN** 执行期报类型错误（列值为非字符串），错误信息表明类型不匹配，SHALL NOT 输出 `'123'`

### Requirement: math 函数四件

`abs(x)` SHALL 返回绝对值并保持入参类型（Int→Int、Float→Float）；`round(x[, digits])` SHALL 按半数远离零（half-away-from-zero）舍入并返回 Float（`round(3.7)=4.0`、`round(2.5)=3.0`、`round(-2.5)=-3.0`、`round(3.14159,2)=3.14`）；`digits` SHALL 接受 1 或 2 个参数形态，`digits<0` 按整数位舍入（`round(123.4,-1)=120.0`），`digits` 为 Float 时按向零截断取整后使用；`floor(x)`/`ceil(x)` SHALL 返回 Float（`floor(3.7)=3.0`、`ceil(3.2)=4.0`、`floor(-3.7)=-4.0`）。四个函数入参 SHALL 严格校验为 Int 或 Float，其他类型报运行时类型错误；`digits` 参数 SHALL 严格校验为 Int 或 Float。

#### Scenario: abs 同型返回

- **GIVEN** 表 `t(i INT, f FLOAT)` 含一行 `i=-5, f=-5.5`
- **WHEN** `SELECT abs(i), abs(f) FROM t`
- **THEN** 输出 `5`（Int）与 `5.5`（Float）

#### Scenario: round 舍入方向与 digits

- **GIVEN** 表 `t(f FLOAT)` 含一行 `f=3.14159`
- **WHEN** 依次执行 `SELECT round(3.7) FROM t`、`SELECT round(2.5) FROM t`、`SELECT round(-2.5) FROM t`、`SELECT round(f, 2) FROM t`、`SELECT round(123.4, -1) FROM t`
- **THEN** 依次输出 `4.0`、`3.0`（半数远离零）、`-3.0`、`3.14`、`120.0`（负 digits 整数位舍入）

#### Scenario: floor/ceil 返回 Float

- **GIVEN** 表 `t(f FLOAT)` 含一行 `f=3.2` 与一行 `f=-3.7`
- **WHEN** `SELECT floor(f), ceil(f) FROM t`
- **THEN** 输出 `3.0, 4.0` 与 `-4.0, -3.0`（均为 Float 形态）

### Requirement: NULL 语义与嵌套参数

任一参数求值为 NULL 时标量函数 SHALL 返回 NULL，且 SHALL NOT 对 NULL 结果再做类型校验或报错。参数求值本身发生的错误 SHALL 先于 NULL 检查传播（求值错误不被 NULL 吞没）。参数 SHALL 接受任意值表达式：列引用、字面量（含负数字面量）、嵌套标量函数、CASE/COALESCE/CAST。行级求值 SHALL 与既有三值折叠组合：比较谓词中的函数结果为 NULL 时该行为 Unknown、不匹配（既有 `fold` 语义，SHALL NOT 改变）。

#### Scenario: NULL 传播

- **GIVEN** 表 `t(name VARCHAR)` 含一行 `name=NULL`
- **WHEN** 依次执行 `SELECT upper(name) FROM t`、`SELECT abs(NULL) FROM t`、`SELECT substr(name, 1, 1) FROM t`
- **THEN** 三者均输出单列 NULL 行

#### Scenario: NULL 短路优先于类型校验

- **GIVEN** 表 `t(id INT PRIMARY KEY)` 含一行 `id=NULL` 不可能（主键），改用表达式 `NULL`
- **WHEN** `SELECT upper(NULL) FROM t`（no-FROM 不可用，经由表：`SELECT upper(CAST(NULL AS INT)) FROM t`，`t` 为任意单行表）
- **THEN** 输出 NULL（Int 型 NULL 不触发 string 函数类型错误）

#### Scenario: 嵌套表达式与负数字面量参数

- **GIVEN** 表 `t(name VARCHAR)` 含一行 `name=NULL` 与一行 `name='x'`
- **WHEN** `SELECT upper(COALESCE(name, 'empty')) FROM t`
- **THEN** 输出 `EMPTY` 与 `X`（嵌套 COALESCE 先求值）
- **AND** `SELECT abs(-5) FROM t` 输出 `5`（负数字面量常量参数）

### Requirement: 调用面与边界

标量函数 SHALL 在 WHERE 与 SELECT 双侧可用：WHERE 中的函数谓词无论走下推（无 OR、非简单 PK 等值）还是 Filter 包装（含 OR）路径，结果 SHALL 一致且正确。以下既有拒绝面 SHALL 保持：SELECT 列表中函数项与聚合混用报错；HAVING 中引用标量函数不支持。ORDER BY 引用表达式项别名或函数调用 SHALL 保持既有静默行为（排序列不可解析时所有行视为相等、保持输入序、不报错）——本文档化既有语义，SHALL NOT 改变。`WHERE id = abs(-5)` 等主键列与函数比较 SHALL 走普通扫描路径并返回正确结果（SHALL NOT 误入索引点查）。

#### Scenario: WHERE 下推路径结果正确

- **GIVEN** 表 `t(id INT PRIMARY KEY, name VARCHAR)` 含 `name='AbC'` 与 `name='xyz'`
- **WHEN** `SELECT id FROM t WHERE upper(name) = 'ABC'`
- **THEN** 仅输出 `name='AbC'` 行的 `id`

#### Scenario: WHERE OR 组合结果正确

- **GIVEN** 同上种子
- **WHEN** `SELECT id FROM t WHERE upper(name) = 'ABC' OR id = 2`
- **THEN** 输出两行 `id`（函数谓词与 OR 组合正确）

#### Scenario: 聚合混用保持拒绝

- **GIVEN** 表 `t(name VARCHAR)` 存在
- **WHEN** `SELECT count(*), upper(name) FROM t`
- **THEN** 报 SQL 错误（与 change 前一致）

#### Scenario: HAVING 中标量函数保持不支持

- **GIVEN** 表 `t(id INT PRIMARY KEY)` 存在
- **WHEN** `SELECT id, count(*) FROM t GROUP BY id HAVING upper(id) = 'X'`
- **THEN** 报 SQL 错误（HAVING 不支持标量函数，与 change 前一致）

#### Scenario: ORDER BY 表达式别名静默保持输入序（既有语义锁定）

- **GIVEN** 表 `t(name VARCHAR)` 按 INSERT 顺序含 `name='b'`、`name='a'`
- **WHEN** `SELECT upper(name) AS u FROM t ORDER BY u`
- **THEN** 退出码 0，输出顺序保持输入序（`B`、`A`），SHALL NOT 报错、SHALL NOT 按别名列排序

#### Scenario: 主键列与函数比较走普通路径

- **GIVEN** 表 `t(id INT PRIMARY KEY)` 含 `id=5`
- **WHEN** `SELECT id FROM t WHERE id = abs(5)`
- **THEN** 输出 `id=5`（普通扫描路径，结果正确）

### Requirement: 既有语义零回归

本 change SHALL NOT 改变既有行为：全部既有测试零修改通过；测试总数只增不减；`COALESCE`/`CASE`/`CAST`、聚合函数、多语句、事务语句、CLI 各面行为不变。

#### Scenario: 全量回归

- **WHEN** `cargo test`、`cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate`
- **THEN** 全部通过；测试总数 ≥ 797（change 前基线）且 0 failed

#### Scenario: 既有表达式测试零修改

- **WHEN** 运行 `tests/expression_e2e_test.rs`（24）、`tests/projection_expression_test.rs`（16）、`tests/predicate_test.rs`、`tests/planner_test.rs`
- **THEN** 全部零修改通过
