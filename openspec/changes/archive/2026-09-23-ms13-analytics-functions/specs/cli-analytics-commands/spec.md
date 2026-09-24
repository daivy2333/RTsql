# cli-analytics-commands Specification

## Purpose

约束 stats/sample/profile 三个分析薄命令的输出契约：三命令 SHALL 经既有查询路径拉数后 CLI 侧计算（引擎零改动），输出 SHALL 遵循 --format 四态，错误面 SHALL 遵循既有退出码分类。来源：MS13-T02 CLI 侧 / R18 主题 7（change `2026-09-23-ms13-analytics-functions`，决策 4）。

## ADDED Requirements

### Requirement: stats 命令输出契约

`rtsql stats <db> <table>` SHALL 输出：总行数 + 每列 null 率、distinct 计数、min、max、p50/p90/p99（分位数最近邻秩，p50 偶数双值平均；仅数值列输出分位数，非数值列该格 null）；min/max 支持全可比类型（Int/Float/String/Bool/Date/Timestamp）；distinct 精确计数。

#### Scenario: 数值列统计

- **GIVEN** 数值列含已知分布（含 NULL 若干）
- **WHEN** `rtsql stats db t`
- **THEN** 行数、null 率、distinct、min/max、三分位数正确

#### Scenario: 日期列统计

- **GIVEN** DATE 列数据
- **WHEN** `rtsql stats db t`
- **THEN** min/max 为日期形态；分位数格 null（非数值列）

#### Scenario: 空表边界

- **GIVEN** 空表
- **WHEN** `rtsql stats db t`
- **THEN** 行数 0、null 率 100%（或既有列形态）、min/max/分位数 null（零除保护，不崩溃）

### Requirement: sample 命令输出契约

`rtsql sample <db> <table> [N]` SHALL 输出随机 N 行（全量拉取后 reservoir sampling；N 默认 10）；N=0 或非法 SHALL 用法错（exit 2）；行形状与 `SELECT *` 一致。

#### Scenario: 抽样行数与列形状

- **GIVEN** 表 M 行（M > N）
- **WHEN** `rtsql sample db t 5`
- **THEN** 恰 5 行、列与 SELECT * 一致、每次执行行集可不同（随机性）

#### Scenario: N 边界

- **GIVEN** `rtsql sample db t 0`、`rtsql sample db t notanumber`
- **WHEN** 执行
- **THEN** 均用法错（exit 2）

### Requirement: profile 命令输出契约

`rtsql profile <db> <table>` SHALL 输出每列：类型、min、max、top-k 高频值（k 默认 5 上限 20，仅 String 列，频次并列取字典序最前者稳定输出）。

#### Scenario: 每列画像

- **GIVEN** 混合类型列表（String/Int/Date）
- **WHEN** `rtsql profile db t`
- **THEN** 每列类型/min/max 正确；String 列 top-5 值+频次；数值列无 top-k 格

#### Scenario: top-k 并列稳定

- **GIVEN** String 列两值频次相同
- **WHEN** profile
- **THEN** 输出顺序确定（并列按字典序），多次执行一致

### Requirement: 格式四态与错误面

三命令输出 SHALL 遵循 `--format table|json|csv|tsv`（TTY/非 TTY 默认沿用）；表不存在 SHALL 沿用既有错误文案与退出码（exit 3）；库锁冲突 exit 4 沿用。

#### Scenario: 格式四态

- **GIVEN** 任一命令
- **WHEN** 分别以 table/json/csv/tsv 执行
- **THEN** 四态输出列形状一致（既有 render 契约）

#### Scenario: 表不存在

- **GIVEN** `rtsql stats db no_such_table`
- **WHEN** 执行
- **THEN** 既有表不存在错误（exit 3）
