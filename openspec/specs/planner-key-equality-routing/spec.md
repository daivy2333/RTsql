# planner-key-equality-routing Specification

## Purpose

约束单表 SELECT 键位等值过滤的 planner 路由正确性：索引路由（IndexScan 点查 / Scan 索引遍历）仅当其结果覆盖表中全部可见行时可用；键位不可键控的行（MS10-T05 001-rework 语义：落库不入索引）必须经数据页行内求值路径可达。来源：MS15-T01（change `2026-09-12-ms15-t01-keyless-eq-routing`，2026-09-12 归档，improvements I036）。

## Requirements

### Requirement: 键位等值过滤对无键行可达

WHERE 子句对键列的等值过滤 SHALL 经数据页行内求值路径覆盖键位不可键控的行（NULL 或非 Int 值，落库不入索引），SHALL NOT 因索引路由产生静默漏行。适用形态 SHALL 包括：简单等值（`WHERE s = 'x'`）、AND 组合（`WHERE s = 'x' AND n = 1`）；键列 SHALL 涵盖隐式首列主键与显式声明主键（含非 Int 类型声明，如 `TEXT PRIMARY KEY`）；不可键控字面量 SHALL 涵盖 String、Float、Bool 与 NULL 字面量。restart 后（WAL 恢复 + 索引重建）上述可达性 SHALL 保持。

#### Scenario: String 隐式键列简单等值

- **GIVEN** 表 `t1(s STRING, n INT)`（无声明 PK，隐式键列 = s）含行 `('x', 1)`、`('y', 2)`
- **WHEN** `SELECT * FROM t1 WHERE s = 'x'`
- **THEN** 返回行 `("x", 1)`（修复前为空集 exit 0）

#### Scenario: String 隐式键列 AND 组合等值

- **GIVEN** 同上表形与种子行
- **WHEN** `SELECT * FROM t1 WHERE s = 'x' AND n = 1`
- **THEN** 返回行 `("x", 1)`（修复前为空集）

#### Scenario: Float 键列 Float 字面量等值

- **GIVEN** 表 `t2(f FLOAT, n INT)`（隐式键列 = f）含行 `(5.0, 1)`
- **WHEN** `SELECT * FROM t2 WHERE f = 5.0`
- **THEN** 返回行 `(5.0, 1)`（修复前为空集）

#### Scenario: 声明 TEXT PRIMARY KEY 等值

- **GIVEN** 表 `t3(s TEXT PRIMARY KEY, n INT)` 含行 `('x', 1)`
- **WHEN** `SELECT * FROM t3 WHERE s = 'x'`
- **THEN** 返回行 `("x", 1)`（修复前为空集）

#### Scenario: restart 后等值可达性保持

- **GIVEN** Scenario 1 的库已完成写入并显式落盘关闭
- **WHEN** 重新打开数据库后执行 `SELECT * FROM t1 WHERE s = 'x'`
- **THEN** 返回行 `("x", 1)`，与 restart 前一致

### Requirement: 可键控字面量路由保持

键列等值字面量为 Int（可键控，`to_key()` 返回 `Some`）时 SHALL 保持既有索引路由与 plan 形状：顶层简单等值 SHALL 保持 `IndexScan` 点查；AND 组合 SHALL 保持 `Filter(Scan)`（索引遍历 + 行内过滤）。上述形态的查询结果 SHALL 与本 change 前一致。

#### Scenario: 简单 Int 等值保持 IndexScan

- **GIVEN** 表 `t(id INT PRIMARY KEY, a INT, b VARCHAR)` 存在
- **WHEN** `SELECT id FROM t WHERE id = 2`
- **THEN** plan 为 `IndexScan`，结果行集与本 change 前一致（既有 `tests/pushdown_test.rs::simple_pk_equality_still_index_scan` 见证保持通过）

#### Scenario: AND 组合 Int 等值保持 Filter(Scan)

- **GIVEN** 同上表形
- **WHEN** `SELECT id FROM t WHERE id = 2 AND a > 5`
- **THEN** plan 为 `Filter(Scan)`，结果行集与本 change 前一致（既有 `tests/pushdown_test.rs::complex_pk_equality_still_filter_over_scan` 见证保持通过）

### Requirement: 既有语义零回归

本 change SHALL NOT 改变既有可观察行为，除 R1 场景所列缺陷形态由错误空集变为正确行集外：Int 键列 + 非 Int 字面量的查询结果 SHALL 保持空集（键位等值按 `Value::equals` 跨类型为 false，修复仅改变求值路径）；OR 形态与非 Eq 运算符的既有 DataScan 路由 SHALL 保持；既有测试套件 SHALL 零修改通过（除既有注记的校准外）。

#### Scenario: Int 键列非 Int 字面量空结果不变

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 含行 `(1, 10)`
- **WHEN** `SELECT * FROM t WHERE id = 'abc'`
- **THEN** 返回空集（本 change 前经 `Filter(Scan)` 为空集，本 change 后经 DataScan 行内求值 Int=Text 为 false 仍为空集，可观察结果逐字节一致）

#### Scenario: 全量回归零修改

- **WHEN** 运行完整测试套件与静态检查（`cargo test`、`cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate`）
- **THEN** 全部通过，既有测试文件除本 change 新增见证外零修改
