# planner-key-equality-routing Specification

## Purpose

约束单表 SELECT 键位等值过滤的 planner 路由正确性：索引路由（IndexScan 点查 / Scan 索引遍历）仅当其结果覆盖表中全部可见行时可用；键位不可键控的行（MS10-T05 001-rework 语义：落库不入索引）必须经数据页行内求值路径可达。来源：MS15-T01（change `2026-09-12-ms15-t01-keyless-eq-routing`，improvements I036）；MS16 扩展键列类型感知路由（change `2026-09-12-ms16-correctness-batch`，improvements I046 形态 2 + 调查新发现）。

## MODIFIED Requirements

### Requirement: 可键控字面量路由保持

键列声明类型为 Int 且键列等值字面量为 Int（可键控，`to_key()` 返回 `Some`）时 SHALL 保持既有索引路由与 plan 形状：顶层简单等值 SHALL 保持 `IndexScan` 点查；AND 组合 SHALL 保持 `Filter(Scan)`（索引遍历 + 行内过滤）。上述形态的查询结果 SHALL 与本 change 前一致。键列声明类型非 Int 时的 Int 字面量等值不适用本 Requirement（由「键列类型感知路由」约束）。

#### Scenario: 简单 Int 等值保持 IndexScan

- **GIVEN** 表 `t(id INT PRIMARY KEY, a INT, b VARCHAR)` 存在
- **WHEN** `SELECT id FROM t WHERE id = 2`
- **THEN** plan 为 `IndexScan`，结果行集与本 change 前一致（既有 `tests/pushdown_test.rs::simple_pk_equality_still_index_scan` 见证保持通过）

#### Scenario: AND 组合 Int 等值保持 Filter(Scan)

- **GIVEN** 同上表形
- **WHEN** `SELECT id FROM t WHERE id = 2 AND a > 5`
- **THEN** plan 为 `Filter(Scan)`，结果行集与本 change 前一致（既有 `tests/pushdown_test.rs::complex_pk_equality_still_filter_over_scan` 见证保持通过）

## ADDED Requirements

### Requirement: 键列类型感知路由

键列声明类型为非 Int（Float / String / Bool，涵盖隐式首列主键与显式声明主键）时，对该键列的等值过滤 SHALL 统一经数据页行内求值路径（谓词下推 `DataScan`；含 OR 保持 `Filter(DataScan)`），SHALL NOT 路由为 `IndexScan` 或 `Filter(Scan)`。适用形态 SHALL 包括：简单等值（`WHERE f = 5`）、AND 组合（`WHERE f = 5 AND n = 1`）与反向书写（`WHERE 5 = f`）。行集 SHALL 按 `Value::equals` 语义正确（含 Int↔Float 隐式转换）。restart 后（WAL 恢复 + 索引重建）上述可达性与路由形状 SHALL 保持。

#### Scenario: Float 键列 Int 字面量简单等值可达

- **GIVEN** 表 `t2(f FLOAT, n INT)`（隐式键列 = f）含行 `(5.0, 1)`
- **WHEN** `SELECT * FROM t2 WHERE f = 5`
- **THEN** 返回行 `(5.0, 1)`，plan 为谓词下推 `DataScan`（修复前 IndexScan 点查空索引，静默空集 exit 0）

#### Scenario: Float 键列 Int 字面量 AND 组合可达

- **GIVEN** 同上表形与种子行
- **WHEN** `SELECT * FROM t2 WHERE f = 5 AND n = 1`
- **THEN** 返回行 `(5.0, 1)`（修复前 `Filter(Scan)` 索引遍历静默空集）

#### Scenario: 反向书写形态可达

- **GIVEN** 同上表形与种子行
- **WHEN** `SELECT * FROM t2 WHERE 5 = f`
- **THEN** 返回行 `(5.0, 1)`

#### Scenario: String 键列 Int 字面量结果不变

- **GIVEN** 表 `t4(s STRING, n INT)`（隐式键列 = s）含行 `('x', 1)`
- **WHEN** `SELECT * FROM t4 WHERE s = 5`
- **THEN** 返回空集（`Value::equals` 跨类型 false；路由由 `Filter(Scan)` 变为数据页求值，可观察结果逐字节一致）

#### Scenario: restart 后类型感知路由保持

- **GIVEN** Float 键列简单等值场景的库已完成写入，shutdown + drop 后重新打开（WAL 恢复 + 索引重建）
- **WHEN** `SELECT * FROM t2 WHERE f = 5`
- **THEN** 返回行 `(5.0, 1)`，与 restart 前一致
