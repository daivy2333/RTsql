# datetime-functions Specification

## Purpose

约束日期/时间标量函数族与 INTERVAL 算术的语义：函数经既有标量函数注册机制分派（大小写不敏感/arity/NULL 短路/严格类型沿用），INTERVAL 为表达式构造不可存储，既有十函数零回归。来源：MS13-T02 引擎侧（change `2026-09-23-ms13-analytics-functions`，决策 3）。

## ADDED Requirements

### Requirement: 日期函数族注册与语义

函数族 SHALL 经 `function.rs` 单点注册表接入（planner 校验入口与运行时分派同源）：`now()`（零参，墙钟，返回 TIMESTAMP）、`date(x)`（Date 恒等 / Timestamp 截断到日）、`year(x)`/`month(x)`/`day(x)`（接受 Date/Timestamp）、`hour(x)`/`minute(x)`/`second(x)`（接受 Timestamp，Date 视为零点）、`date_trunc(unit, x)`（unit 为 String 字面量，单位 year/month/day/hour/minute/second 大小写不敏感；Date 支持 year/month/day，Timestamp 支持全单位）、`datediff(unit, a, b)`（返回 b−a 整单位数，截断）。

#### Scenario: 抽取函数正确

- **GIVEN** Date `2024-02-29` 与 Timestamp `2024-01-15 10:30:45.123456`
- **WHEN** `year(d)`/`month(d)`/`day(d)`、`hour(ts)`/`minute(ts)`/`second(ts)`、`date(ts)`
- **THEN** 分别得 2024/2/29、10/30/45、`2024-01-15`

#### Scenario: date_trunc 截断正确

- **GIVEN** Timestamp `2024-01-15 10:30:45`
- **WHEN** `date_trunc('day', ts)`、`date_trunc('hour', ts)`、`date_trunc('month', ts)`、`date_trunc('year', ts)`
- **THEN** 分别得 `2024-01-15 00:00:00`、`2024-01-15 10:00:00`、`2024-01-01 00:00:00`、`2024-01-01 00:00:00`

#### Scenario: now 返回当前时刻

- **GIVEN** 任意库
- **WHEN** 两次执行 `SELECT now()`
- **THEN** 均得 TIMESTAMP 且非递减（运行期求值，非 plan 期常量）

#### Scenario: 单位与类型错误面

- **GIVEN** `date_trunc('week', ts)`（未注册单位）、`year('abc')`（String 参数）、`hour(d)`+DA9 之外形态
- **WHEN** 执行
- **THEN** 分别显式拒绝（单位不识别点名文案）、运行期类型错误（严格类型）

#### Scenario: NULL 短路

- **GIVEN** 任一函数参数为 NULL（now 除外）
- **WHEN** 求值
- **THEN** 返回 NULL（既有 D3 求值顺序沿用）

### Requirement: INTERVAL 表达式算术

`INTERVAL '<n> <unit>'` 与 `INTERVAL <n> <unit>`（sqlparser `Expr::Interval`）SHALL 解析为表达式内部区间值（单位 year/month/day/hour/minute/second）；`Date ± INTERVAL` 与 `Timestamp ± INTERVAL` SHALL 产出同族值；month/year 算术 SHALL 同日锚定、溢出日截断月末（PostgreSQL 语义）；INTERVAL 作列类型或独立投影项 SHALL 显式拒绝（不可存储）。

#### Scenario: 日/时级算术

- **GIVEN** Date `2024-01-15`、Timestamp `2024-01-15 10:00:00`
- **WHEN** `d + INTERVAL '1 day'`、`ts - INTERVAL '90 minutes'`
- **THEN** 得 Date `2024-01-16`、Timestamp `2024-01-15 08:30:00`

#### Scenario: 月末锚定

- **GIVEN** Date `2024-01-31`
- **WHEN** `d + INTERVAL '1 month'`
- **THEN** 得 Date `2024-02-29`（闰年截月末）

#### Scenario: INTERVAL 拒绝面

- **GIVEN** `CREATE TABLE t (i INTERVAL)`、`SELECT INTERVAL '1 day'`
- **WHEN** 执行
- **THEN** 均显式拒绝（列类型不可用；独立投影项不可达，点名文案）

### Requirement: datediff 语义

`datediff(unit, a, b)` SHALL 返回 b−a 的整单位数（绝对值按单位截断，非四舍五入）；a、b 同族（Date 或 Timestamp）；跨单位换算按日历（month/year 取日历差）。

#### Scenario: 日差与月差

- **GIVEN** `datediff('day', DATE '2024-01-01', DATE '2024-01-31')`、`datediff('month', DATE '2024-01-15', DATE '2024-03-14')`
- **WHEN** 求值
- **THEN** 得 30、1（3-14 减 1-15 不足 2 整月，截断）

### Requirement: 既有标量函数零回归

string 六件与 math 四件的既有语义（含 MS11-T03 全部场景）SHALL 零回归；新函数注册 SHALL NOT 改变未注册名既有拒绝文案。

#### Scenario: 既有函数面零回归

- **GIVEN** 既有 `tests/scalar_function_test.rs` 全量
- **WHEN** 实施后运行
- **THEN** 零修改通过
