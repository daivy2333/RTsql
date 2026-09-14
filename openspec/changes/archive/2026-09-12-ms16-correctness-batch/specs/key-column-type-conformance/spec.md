# key-column-type-conformance Specification

## Purpose

约束键列写入的类型一致性：Int 类型键列 SHALL 只存储 Int 或 NULL 键位值，越界类型 SHALL 在任何存储副作用前显式拒绝。该约束保证「Int 键列的键位等值 IndexScan 路由可靠」（键列中不存在因类型越界而落库不入索引的可匹配行），与 `planner-key-equality-routing` 的路由正确性互为前提。来源：MS16（change `2026-09-12-ms16-correctness-batch`）调查探针实证（2026-09-12，revision d8a244f：INSERT 不校验列类型，Float 值可落 Int 键列成为无键行，`WHERE id = 5` 静默空集；用户裁定并入本 change 直接修复，不登记 improvement）。

## ADDED Requirements

### Requirement: Int 键列写入类型强制

键列声明类型为 Int 时，INSERT 的键位值与 UPDATE SET 键列的新值 SHALL 只接受 Int 或 NULL；其他类型（Float / String / Bool）SHALL 在写入数据页、WAL 或索引之前被拒绝（`StorageError::KeyTypeMismatch`，错误文案点名键列与期望/实际类型），表行集、索引条目与 WAL SHALL 零变化，CLI 映射 exit 3。NULL 键位值 SHALL 保持既有无键行语义（落库不入索引，MS10-T05 001-rework）。显式列序 INSERT 的键位值按列清单映射后的键位取值校验（映射语义见 `insert-column-list-mapping`）。键列声明类型为非 Int（Float / String / Bool）时 SHALL NOT 新增类型拒绝（既有行为保持）。

**既有测试校准**（Plan Review BH-1 裁定，2026-09-12）：既有 `tests/expression_e2e_test.rs::negative_number_literal_persists`（MS11-T01 R5/S1，I040 负数字面量）原依赖「Int 隐式键列收负 Float 字面量」形态，本 Requirement 实施后该 INSERT 被拒绝——按 update-index-maintenance R2「T8-R2 校准」先例校准：负 Int 行（`INSERT INTO t VALUES (-1, 'x')`）保持原表原断言逐字节不动；负 Float 行移入 Float 键列表 `tf(f FLOAT, s STRING)`（`INSERT INTO tf VALUES (-1.5, 'y')`），重开断言两表各自行集。I040 的负 Int/负 Float 字面量折叠覆盖完整保留（负 Float 落 Float 键列同时补足非 Int 键列持久化覆盖），其余既有测试零修改。

#### Scenario: INSERT Float 值入 Int 键列被拒绝

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 为空表
- **WHEN** `INSERT INTO t VALUES (5.0, 1)`
- **THEN** 报错 `KeyTypeMismatch`（exit 3）；随后 `SELECT COUNT(*)` 为 0，索引与 WAL 零副作用

#### Scenario: INSERT NULL 键位保持无键行语义

- **GIVEN** 同上表形
- **WHEN** `INSERT INTO t VALUES (NULL, 1)`
- **THEN** 成功；行 `(NULL, 1)` 落库不入索引，经非键谓词可达

#### Scenario: 显式列序 INSERT 按映射后键位校验

- **GIVEN** 同上表形
- **WHEN** `INSERT INTO t (v, id) VALUES (1, 5.0)`
- **THEN** 报错 `KeyTypeMismatch`（列清单按 `insert-column-list-mapping` 重排后 id 收 5.0；修复前列清单无消费点，5.0 错位落非键列 v、id 收 1 静默成功）

#### Scenario: UPDATE SET 键列为 Float 被拒绝

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 含行 `(5, 100)`
- **WHEN** `UPDATE t SET id = 5.0 WHERE id = 5`
- **THEN** 报错 `KeyTypeMismatch`；行 `(5, 100)` 与索引条目 5 保持不变（零副作用）

#### Scenario: UPDATE SET 键列为 NULL 保持既有语义

- **GIVEN** 同上表形含行 `(5, 100)`
- **WHEN** `UPDATE t SET id = NULL WHERE id = 5`
- **THEN** 成功，旧键条目清理（`update-index-maintenance` R1 语义不受影响）

#### Scenario: 非 Int 键列不新增拒绝

- **GIVEN** 表 `t2(f FLOAT PRIMARY KEY, n INT)`
- **WHEN** `INSERT INTO t2 VALUES (5, 1)`（Int 值入 Float 键列）
- **THEN** 行为与本 change 前一致（成功落库，行为保持锚点）

#### Scenario: import 与 restore 合规面不受影响

- **WHEN** `import --csv` 与既有 dump/restore 往返套件运行
- **THEN** 全部通过（import `csv_value` 按列声明类型转换；dump/restore 往返既有测试零修改）
