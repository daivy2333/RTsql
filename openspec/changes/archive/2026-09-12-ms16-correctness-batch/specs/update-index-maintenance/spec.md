# update-index-maintenance Specification

## Purpose

约束 UPDATE 执行器对 PK 索引的维护语义：索引条目 SHALL 始终反映行当前键位的可键控性，与插入侧（键位不可键控行落库不入索引，MS10-T05 001-rework）及恢复侧（索引重建，MS10-T02 R7/R8）一致。来源：MS15-Rest（change `2026-09-12-ms15-rest-correctness-batch`，improvements I037）；MS16 扩展 rekey 索引一致性（change `2026-09-12-ms16-correctness-batch`，improvements I047）。

## ADDED Requirements

### Requirement: 键位 rekey 后索引条目一致

UPDATE 将键列 SET 为另一可键控 Int 值（新旧键值均可 `to_key()` 且不等）时，执行器 SHALL 在任何写入（数据页 / WAL / 索引）之前以新键查询索引：命中已有条目 SHALL 以 `DuplicateKey` 拒绝且零副作用（行集、索引条目、WAL 不变）；未命中 SHALL 删除旧键索引条目并将新键条目指向新版本。此后：新键点查 SHALL 返回该行；旧键点查 SHALL 为空集；对旧键值的 INSERT SHALL NOT 被误拒；崩溃恢复（WAL 重放 + 索引重建）后上述可达性 SHALL 与运行期一致（两态一致）。键列 SET 为原值（新键 == 旧键）SHALL 保持既有 update 路径。

**既有测试校准**（Plan Review BH-3 裁定，2026-09-13）：M10 时代直连执行器单测 `tests/gc_test.rs`（3 用例）、`tests/version_chain_test.rs`（2 用例）、`tests/plan_exec_test.rs::test_insert_update_scan_flow` 以「SET 键列 = 另一 Int 值」建立版本链，并按**旧键** search/IndexScan 定位 rekey 后版本或寻址后续 UPDATE——依赖本 Requirement 修复的缺陷行为（旧键条目残留指向新版本），本 Requirement 实施后该 6 用例确定性失败。按 key-column-type-conformance「BH-1 校准」先例校准：测试主题（GC 清理计数与最新版本可达 / 版本链遍历与可见性 / 插改扫流）与断言语义保持，寻址与定位一律改用**行当前键**（rekey 后的新值键）；同键更新用例与非键列更新用例不受影响，受影响面经 UpdateExecutor 全部测试用法排查闭合。校准后全量回归零失败。

#### Scenario: rekey 新键点查可达

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 含行 `(5, 100)`
- **WHEN** `UPDATE t SET id = 7 WHERE id = 5`
- **THEN** `SELECT * FROM t WHERE id = 7` 返回 `(7, 100)`（修复前新键无索引条目，静默空集）

#### Scenario: rekey 旧键条目清理

- **GIVEN** 同上，`UPDATE t SET id = 7 WHERE id = 5` 已成功
- **WHEN** `SELECT * FROM t WHERE id = 5` 与 `INSERT INTO t VALUES (5, 200)`
- **THEN** 旧键点查为空集；旧键 INSERT 成功（修复前旧键条目残留：点查经残留条目返回、INSERT 被 `DuplicateKey` 误拒）

#### Scenario: rekey 新键撞已有行写入前拒绝

- **GIVEN** 表含行 `(5, 100)` 与 `(7, 200)`
- **WHEN** `UPDATE t SET id = 7 WHERE id = 5`
- **THEN** 报错 `DuplicateKey`（exit 3）；两行行集与索引条目 5、7 均保持原状（零副作用，拒绝发生在任何写入之前）

#### Scenario: rekey 崩溃恢复两态一致

- **GIVEN** rekey 成功后进程未 close 退出（崩溃模拟：WAL 重放 + 索引重建）
- **WHEN** 重新打开数据库
- **THEN** 新键 7 点查可达、旧键 5 点查空集且 `INSERT (5, …)` 成功——与运行期行为一致

#### Scenario: 同键原值更新保持

- **GIVEN** 表含行 `(5, 100)`
- **WHEN** `UPDATE t SET id = 5 WHERE id = 5`
- **THEN** 既有行为保持（既有 `tests/update_index_maintenance_test.rs::key_column_same_value_update_keeps_entry` 锚点）
