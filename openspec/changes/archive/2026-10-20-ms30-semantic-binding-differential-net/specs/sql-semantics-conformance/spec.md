# Delta: sql-semantics-conformance

差分测试网（Iteration 000）分诊出的四类语义缺陷的清偿规格；来源 000 Plan Review Follow-up Decision 处置表 + 2026-10-20 用户裁定并入本 change（T03 / Iteration 003）。

## ADDED Requirements

### Requirement: 无键基表 JOIN 行可见性

PK 列类型非 INT 的无键表（keyless——行存储于数据页但不进入 PK B-Tree 索引）作为 JOIN / SemiJoin 基座输入时，其行 SHALL 完整可见（与单表 `DataScan` 通道同一可见性语义：快照可见版本、替代去重、宽度感知读回）；带 INT 主键表的既有索引枚举路径 SHALL 逐字节保持。

#### Scenario: 无键表自连接返回全行

- **GIVEN** 无主键（或 PK 列非 INT）基表含 N 行
- **WHEN** 该表自连接或作为等值 JOIN 输入
- **THEN** 枚举通道返回其可见行（差分形态：SQLite 一致，非空集）

#### Scenario: 无键表作为 IN 子查询外层基表

- **GIVEN** 无键基表作为 IN 子查询查询的外层 FROM
- **WHEN** SemiJoin 装配执行
- **THEN** 外层行完整参与半连接匹配（差分形态：SQLite 一致）

#### Scenario: 带主键表索引枚举零回归

- **GIVEN** INT 主键表作为 JOIN 基座
- **WHEN** 同一批既有 JOIN 查询执行
- **THEN** 行序、行值与既有索引枚举结果逐字节一致（既有 2046 基线零回归）

### Requirement: 相关参数文本值保真

相关子查询参数值为文本（String）时，含参数的表达式 / 谓词树 SHALL 求值得出真实文本值（非 NULL）；不含参数的表达式 / 谓词树 SHALL 维持既有零拷贝求值路径与结果逐字节一致。

#### Scenario: TEXT 相关参数等值匹配

- **GIVEN** 外层 TEXT 列经相关参数注入子查询谓词（`s2.carrier = s.carrier` 形态）
- **WHEN** 相关子查询执行
- **THEN** 参数按真实文本值参与比较，匹配行为与整型参数通道一致（差分形态：SQLite 一致，非恒空集）

#### Scenario: 非参数谓词零拷贝路径保持

- **GIVEN** 不含相关参数的任意谓词（含 TEXT 列比较）
- **WHEN** DataScan / Filter 求值
- **THEN** 求值路径与结果和既有实现一致（全量基线零回归）

### Requirement: NOT IN 的 NULL 三值折叠

`NOT IN` 子查询（AntiJoin 通道）SHALL 遵循三值语义：外层键为 NULL 的行不返回（Unknown 折叠）；子查询结果集含 NULL 键时全部不匹配行不返回（Unknown 折叠）；子查询空集时全行返回（TRUE）；既有 `IN` 通道（SemiJoin）与 `NOT EXISTS` 通道行为 SHALL 保持不变。

#### Scenario: 外层 NULL 键行被排除

- **GIVEN** 外层键列含 NULL 的行集与非空子查询结果
- **WHEN** `NOT IN` 执行
- **THEN** NULL 键行不出现在结果中（差分形态：SQLite 一致）

#### Scenario: 子查询含 NULL 时全折叠

- **GIVEN** 子查询结果集键列含 NULL
- **WHEN** 外层非 NULL 键行 `NOT IN` 执行
- **THEN** 不匹配行不返回（差分形态：SQLite 一致）

#### Scenario: IN 与 NOT EXISTS 通道不变

- **GIVEN** 同数据集的 `IN` 与 `NOT EXISTS` 查询
- **WHEN** 修复后执行
- **THEN** 两通道结果与修复前一致（既有测试零回归）

### Requirement: 聚合派生表输出列引用

聚合子查询作为 FROM 派生表时，外层限定列引用 SHALL 按 SELECT 清单输出名（别名生效）解析；既有聚合查询自身的 HAVING / ORDER BY / 表头通道与非聚合派生表通道 SHALL 保持不变。

#### Scenario: 聚合派生表别名键外层引用

- **GIVEN** `FROM (SELECT tag AS n, COUNT(*) AS c FROM t GROUP BY tag) d`
- **WHEN** 外层查询引用 `d.n` / `d.c`
- **THEN** 解析成功且值正确（差分 / 定向断言；修复前该形态计划期 `Column not found: n`）

#### Scenario: 聚合查询自身通道不变

- **GIVEN** 任意既有聚合查询（直出与包装形态、HAVING、聚合 ORDER BY）
- **WHEN** 修复后执行
- **THEN** 表头、行值、计划形状与修复前逐字节一致（全量基线零回归）
