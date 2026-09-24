# no-from-select Specification

## Purpose

约束无 FROM 子句 SELECT 的可达语义：常量表达式查询 SHALL 经虚拟单行输入产出单行结果，拒绝面 SHALL 显式点名，既有含 FROM 形态 SHALL 零回归。来源：I035 / MS13-T03（change `2026-09-23-ms13-analytics-functions`；MS10-T04 S5 见证原拟 SQL 曾因不可达修订）。

## ADDED Requirements

### Requirement: 常量表达式单行可达

`SELECT <表达式项列表>`（无 FROM）SHALL 对每项按既有表达式求值语义产出单行结果（常量、算术、标量函数、CASE/COALESCE/CAST、类型字面量组合可达）；表头 SHALL 为表达式文本或别名（既有 SELECT 表达式项表头语义沿用）；行数为恰 1。

#### Scenario: 常量算术

- **GIVEN** 任意库
- **WHEN** `SELECT 1+1`
- **THEN** 单行 `[[2]]`，表头 `1 + 1`（既有表达式表头语义）

#### Scenario: 标量函数探测

- **GIVEN** 任意库
- **WHEN** `SELECT now()`、`SELECT upper('a') AS u`
- **THEN** 单行结果；别名表头 `u`

#### Scenario: 无表探测组合

- **GIVEN** `SELECT CASE WHEN 1=1 THEN 'yes' ELSE 'no' END AS probe`
- **WHEN** 执行
- **THEN** 单行 `[['yes']]`

### Requirement: 拒绝面显式

no-FROM 形态下通配符（`SELECT *`）、WHERE、GROUP BY（含聚合项）、HAVING、ORDER BY、LIMIT SHALL 显式拒绝（点名文案，exit 3）；列引用 SHALL 维持列不存在类错误语义。

#### Scenario: 通配符拒绝

- **GIVEN** `SELECT * `（无 FROM）
- **WHEN** 执行
- **THEN** 显式拒绝文案（非既有 `Missing required field: FROM clause`）

#### Scenario: 谓词与聚合拒绝

- **GIVEN** `SELECT 1 WHERE 1=0`、`SELECT COUNT(*)`（无 FROM）
- **WHEN** 执行
- **THEN** 均显式拒绝（点名子句）

#### Scenario: 列引用错误语义

- **GIVEN** `SELECT nonexistent_col`（无 FROM）
- **WHEN** 执行
- **THEN** 列不存在类显式错误

### Requirement: 既有 FROM 形态零回归与算术表达式项解锁

含 FROM 的全部既有 SELECT 行为（路由/投影/JOIN/子查询/聚合/表达式项）SHALL 零回归；lib 两路径（execute_sql/execute_in_tx）与 CLI 面行为一致。与 no-FORM 同一放行通路，WITH-FORM 算术表达式项（当前 `SELECT 1 + 1` 含 FROM 形态亦报 `Unsupported statement type`——`ast.rs extract_columns` 放行清单不含 BinaryOp，探针实证）SHALL 随本能力解锁为可达（顶层 Projection 逐行求值，既有表达式项拒绝面同步生效）。

#### Scenario: WITH-FORM 算术表达式项解锁

- **GIVEN** 表 `t(id INT)` 含两行 `id=1, id=2`
- **WHEN** `SELECT id + 1 FROM t`
- **THEN** 输出 `2`、`3`（列名 `id + 1`，表达式项表头语义）

#### Scenario: 既有全量零回归

- **GIVEN** 既有全量测试基线
- **WHEN** 实施后运行
- **THEN** 零修改通过
