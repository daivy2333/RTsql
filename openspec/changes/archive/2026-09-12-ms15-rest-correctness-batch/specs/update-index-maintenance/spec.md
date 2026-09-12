# update-index-maintenance Specification

## Purpose

约束 UPDATE 执行器对 PK 索引的维护语义：索引条目 SHALL 始终反映行当前键位的可键控性，与插入侧（键位不可键控行落库不入索引，MS10-T05 001-rework）及恢复侧（索引重建，MS10-T02 R7/R8）一致。来源：MS15-Rest（change `2026-09-12-ms15-rest-correctness-batch`，improvements I037）。

## ADDED Requirements

### Requirement: 键位置更新为无键值后旧键索引条目清理

UPDATE 将键列（`table_meta.pk_column`）更新为不可键控值（`Value::to_key()` 返回 `None`，即 NULL 或非 Int 值）时，执行器 SHALL 删除旧键的索引条目（`IndexManager::delete`）SHALL NOT 保留指向键位已无键新版本的条目。此后：

- 对旧键值的 INSERT SHALL NOT 因残留索引条目被误拒（`DuplicateKey`）；
- 对旧键值的索引点查 SHALL 返回空结果（SHALL NOT 经残留条目返回键位已为无键值的行）；
- 崩溃恢复（WAL 重放 + 索引重建，无键版本不入重建索引）后，上述可达性 SHALL 与运行期一致（两态一致）。

#### Scenario: 键位置 NULL 后旧键 INSERT 不再误拒

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 含行 `(5, 100)`
- **WHEN** `UPDATE t SET id = NULL WHERE id = 5` 成功后执行 `INSERT INTO t VALUES (5, 200)`
- **THEN** INSERT 成功（修复前被 `DuplicateKey` 误拒），表含 `(NULL, 100)` 与 `(5, 200)` 两行

#### Scenario: 键位置 NULL 后旧键点查返回空集

- **GIVEN** 同上，`UPDATE t SET id = NULL WHERE id = 5` 已执行
- **WHEN** `SELECT * FROM t WHERE id = 5`
- **THEN** 返回空集（修复前经残留索引条目返回键位为 NULL 的行 `(NULL, 100)`）

#### Scenario: 键位置 NULL 后崩溃恢复两态一致

- **GIVEN** `UPDATE t SET id = NULL WHERE id = 5` 已执行且未显式 close（进程崩溃 / drop 退出）
- **WHEN** 重新打开数据库（WAL 重放 + 索引重建）
- **THEN** 行 `(NULL, 100)` 可经非键谓词或全扫描可达，`WHERE id = 5` 点查为空集，INSERT `(5, 200)` 成功——与运行期行为一致

#### Scenario: 键位无键行对键位等值 UPDATE 不可达

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 已执行 `UPDATE t SET id = NULL WHERE id = 5`（行键位已为 NULL，索引无键 5 条目）
- **WHEN** `UPDATE t SET v = 42 WHERE id = 5`
- **THEN** 报错 `Key not found`（键位等值 WHERE 经索引定位失败——无键行不入索引，对键位等值 UPDATE 不可达，与恢复侧重建语义一致）

#### Scenario: 非键列更新不影响索引条目

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 含行 `(5, 100)`
- **WHEN** `UPDATE t SET v = 200 WHERE id = 5`
- **THEN** 索引条目 `5` 仍指向该行（`WHERE id = 5` 点查返回 `(5, 200)`），行为与本 change 前一致

### Requirement: 既有 UPDATE 语义零回归

UPDATE 对键列更新为可键控值、非键列任意更新、目标键不存在（`KeyNotFound`）、多列 SET 拒绝、非 PK WHERE 拒绝等既有语义 SHALL 保持与本 change 前一致；MVCC 版本链写入、WAL Update 记录、事务包裹语义不变。既有测试套件 SHALL 通过，其中既有 `keyless_row_test::keyless_row_update_recovery_after_crash` SHALL 按 R1 语义校准——其原「无键行 UPDATE 链」第二次 UPDATE 依赖的恰是 I037 缺陷的残留索引条目（old_tuple 无键 Update 记录的唯一运行期生产路径），修复后 `KeyNotFound` 即正确语义（R1 不可达场景）；校准内容 SHALL 记录于 Act Response，其余既有测试 SHALL 零修改。

#### Scenario: 键列原值更新条目保持

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 含行 `(5, 100)`
- **WHEN** `UPDATE t SET id = 5 WHERE id = 5`（键列 SET 为原可键控值）
- **THEN** `WHERE id = 5` 点查仍返回该行，索引条目保持

#### Scenario: 既有 UPDATE 行为零回归

- **WHEN** 运行 `tests/keyless_row_test.rs`（UPDATE 链与崩溃恢复，T8-R2 按 R1 语义校准）、显式事务套件与完整测试套件
- **THEN** 全部通过；除本 change 新增见证与 T8-R2 语义校准（R2 Requirement 文本所列）外既有测试零修改；`cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate` 全部通过
