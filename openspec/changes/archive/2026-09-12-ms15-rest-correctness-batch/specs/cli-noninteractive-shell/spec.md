# cli-noninteractive-shell Specification

## Purpose

约束 `rtsql` 非交互命令面的入口、解析、渲染与生命周期子命令行为。本 delta 修改「扫描执行器真投影」Requirement：CLI 表头提取与投影行形状的一致性面。来源：MS15-Rest（change `2026-09-12-ms15-rest-correctness-batch`，improvements I034）。

## MODIFIED Requirements

### Requirement: 扫描执行器真投影（Iteration 001）

`SELECT` 的投影列表 SHALL 决定扫描路径返回行的形状：四个扫描执行器（Scan / DataScan / IndexScan / IndexScanAll）SHALL 按投影裁剪产出行，plan 节点的 `columns` 元数据与行形状一致。谓词求值（WHERE / 下推谓词 / MVCC 可见性）SHALL 在全 schema 行上先行完成，投影只发生在行产出最后一步。`SELECT *` 的投影等于全 schema，行为不变。

CLI 表头 SHALL 与行形状一致：`get_plan_output_columns` 对携带投影的 plan 节点 SHALL 返回投影后的列名（含 DataScan / Scan / IndexScanAll 节点自身的 `projection` 裁剪，与 Filter / Sort 臂既有模式一致）；任何查询路径下 CLI 输出的表头列数 SHALL 等于每行字段数（table / json / csv / tsv 各格式同契约）。

#### Scenario: 子集投影在全部扫描路径返回投影列

- **GIVEN** 表 `s(id INT PRIMARY KEY, name STRING)` 含一行 `(1, 'Alice')`
- **WHEN** 分别执行 `SELECT name FROM s`（DataScan 路径）与 `SELECT name FROM s WHERE id = 1`（IndexScan 路径）
- **THEN** 两条查询都返回单列：表头 `["name"]`、行 `[["Alice"]]`
- **AND** 表头列数与每行字段数一致（任何路径无错位）

#### Scenario: 裸 DataScan 子集投影 CLI 表头按投影裁剪

- **GIVEN** 表 `s(id INT PRIMARY KEY, name STRING)` 含一行 `(1, 'Alice')`（修复前表头为 `["id","name"]`、行 `[["Alice"]]`）
- **WHEN** `rtsql <db> "SELECT name FROM s"`（`--format json`）
- **THEN** 输出 `{"columns":["name"],"rows":[["Alice"]]}`（修复前 `columns` 为 `["id","name"]`，与 `rows` 字段数不一致）

#### Scenario: 下推 DataScan 子集投影 CLI 表头按投影裁剪

- **GIVEN** 表 `t(id INT, n INT, s STRING)` 含行 `(1, 10, 'a')`
- **WHEN** `rtsql <db> "SELECT s FROM t WHERE n > 5"`（谓词下推 DataScan 路径，`--format json`）
- **THEN** 输出 `{"columns":["s"],"rows":[["a"]]}`，表头列数与行字段数一致

#### Scenario: PK 点查聚合返回正确值

- **GIVEN** 表 `s(id INT PRIMARY KEY, price INT)` 含行 `(1,10), (2,20)`
- **WHEN** `SELECT SUM(price) FROM s WHERE id = 2`
- **THEN** 返回 `20`（而非 `null`）
- **AND** 聚合输入的列映射与投影后的行形状一致（无静默 Null 兜底路径）

#### Scenario: 投影外排序键正确排序

- **GIVEN** 表 `s(id INT PRIMARY KEY, name STRING)` 含多行
- **WHEN** `SELECT id FROM s WHERE price > 15 ORDER BY name DESC`（排序键 `name` 不在投影内）
- **THEN** 输出行按 `name` 降序排列（而非静默保持原序）

#### Scenario: SELECT 与全 schema 行为不变

- **GIVEN** 任意含数据的表
- **WHEN** `SELECT * FROM t` 或投影覆盖全部列
- **THEN** 返回行与投影改造前的全 schema 行完全一致（旧行为保留）

#### Scenario: 聚合与表达式路径表头零回归

- **GIVEN** 聚合查询（`SELECT COUNT(*) AS cnt FROM t`）与表达式投影查询（`SELECT COALESCE(n, 0) AS x FROM t`；表达式项支持面见 `sql-expression-evaluation`——二元算术不在 SELECT 表达式项之列，本场景以受支持的 Projection 定形形态为锚）
- **WHEN** 渲染结果
- **THEN** 表头分别来自 Aggregate `output_columns` 与 Projection 节点 `columns`，与本 change 前一致（聚合查询 scan 输入投影恒为空、表达式路径由顶层 Projection 节点定形，本 change 的 scan 臂投影裁剪不触及）

#### Scenario: 既有测试按投影语义校准

- **GIVEN** 既有测试套件中假设"子集投影返回全 schema 行"的断言
- **WHEN** 本 Requirement 落地
- **THEN** 受影响断言按投影语义校准（只改行形状期望，不改测试意图），校准清单记录于 Act Response
- **AND** `cargo test --all` 全绿
