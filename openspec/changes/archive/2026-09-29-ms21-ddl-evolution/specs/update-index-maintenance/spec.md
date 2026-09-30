## ADDED Requirements

### Requirement: 回滚与冲突删除路径索引条目一致

事务回滚路径（`TransactionManager::abort_cleanup_versions`）对 rekey UPDATE（键列或唯一列 SET 为另一可键控 Int 值后回滚）的条目回退 SHALL 按 rekey 语义处理：旧键条目 SHALL 恢复指向 prev 版本（prev 版本 tuple 携带旧键），新键条目 SHALL NOT 残留。回滚后：旧键等值点查 SHALL 可达该行；新键等值点查 SHALL 为空集；对旧键与新键的 INSERT SHALL 行为与回滚前一致（不得因残留条目误拒）；干净重开与崩溃恢复后 SHALL 两态一致（PK 索引与唯一索引同型）。

唯一列 SET 为不可键控值（NULL）后回滚的，被删除的旧唯一键条目 SHALL 还原指向 prev 版本（占用语义恢复：同值 INSERT SHALL 重新被 UNIQUE 拒绝）；干净重开与崩溃恢复后 SHALL 两态一致。

UPSERT 冲突行删除（`DO UPDATE` 命中冲突行 / `REPLACE INTO`）在冲突索引条目指向的 slot 不可读（`SlotNotFound`，仅夹具/损坏态可达）时 SHALL 与 `DELETE` 执行器同形态对齐——仍按搜索键执行索引条目删除，SHALL NOT 静默保留悬空条目。既有 MS24 回滚还原矩阵（墓碑行 PK 与唯一索引条目还原）SHALL 保持零回归。

#### Scenario: PK rekey 回滚后旧键点查可达

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 含行 `(5, 100)`，CLI 会话 `BEGIN`
- **WHEN** `UPDATE t SET id = 7 WHERE id = 5` 后 `ROLLBACK`，再执行 `SELECT * FROM t WHERE id = 5`
- **THEN** 返回 `(5, 100)`（缺陷形态：旧键条目缺失、点查漏行）

#### Scenario: PK rekey 回滚后新键无残留

- **GIVEN** 同上回滚后
- **WHEN** `SELECT * FROM t WHERE id = 7` 与 `INSERT INTO t VALUES (7, 200)`
- **THEN** 点查为空集；INSERT 成功（SHALL NOT 经残留新键条目误拒或返回不可见行）

#### Scenario: 唯一列 rekey 回滚同型一致

- **GIVEN** 表 `t(id INT PRIMARY KEY, u INT UNIQUE)` 含行 `(1, 5)`，会话 `BEGIN`
- **WHEN** `UPDATE t SET u = 9 WHERE id = 1` 后 `ROLLBACK`
- **THEN** 唯一索引条目与存活版本一致：`WHERE u = 5` 可达该行、`WHERE u = 9` 空集、重插 `u = 9` 不被误拒

#### Scenario: 回滚后重开两态一致

- **GIVEN** rekey UPDATE 已回滚，进程未显式 close（崩溃模拟）
- **WHEN** 重新打开数据库（WAL 重放 + 索引重建）
- **THEN** 旧键点查可达、新键空集、双键 INSERT 可用——与运行期行为一致

#### Scenario: SET 唯一列为 NULL 的回滚还原旧值占用

- **GIVEN** 表 `t(id INT PRIMARY KEY, code INT UNIQUE)` 含行 `(1, 5)`，会话 `BEGIN`
- **WHEN** `UPDATE t SET code = NULL WHERE id = 1` 后 `ROLLBACK`
- **THEN** 唯一索引条目与存活版本一致：`WHERE code = 5` 扫描可达该行、重插 `(2, 5)` 被 UNIQUE 拒绝（占用恢复，缺陷形态：INSERT 成功、UNIQUE 静默失效）；干净重开与崩溃恢复后同型一致

#### Scenario: UPSERT 冲突删除夹具态条目对齐

- **GIVEN** 夹具构造：唯一/PK 索引条目指向不存在 slot 的表（损坏态）
- **WHEN** `INSERT ... ON CONFLICT DO UPDATE`（或 `REPLACE INTO`）命中该冲突条目
- **THEN** 与 `DELETE` 同形态：按搜索键执行 `index_manager.delete`（条目被清理），SHALL NOT 静默保留悬空条目

#### Scenario: 既有回滚还原零回归

- **WHEN** 运行 `tests/explicit_tx_test.rs`（MS24 R6 六矩阵）、`tests/upsert_test.rs`、`tests/mvcc_tombstone_visibility_test.rs` 与完整套件
- **THEN** 全部通过；除本 change 新增见证外既有测试零修改

### Requirement: DELETE 未命中键零副作用

`DELETE FROM t WHERE <PK> = <字面量>` 在索引 `search` 未命中时 SHALL 零副作用：返回 `AffectedRows(0)`，SHALL NOT 删除任何索引条目（含 B-Tree「首个 ≥」邻近条目），SHALL NOT 触碰数据页、墓碑、WAL 与唯一索引条目，SHALL NOT 报错（含键大于全部键时的 `KeyNotFound` 误报）。`search` 命中路径（含 slot 不可读容忍臂）SHALL 保持既有行为；索引删除原语（`LeafNode::delete`）对树中不存在的键 SHALL NOT 误删其他条目。2026-09-29 用户指令扩围（来源 Iter000 Act Response Issue 候选 1）。

#### Scenario: 未命中中间缺失键不误删邻近条目

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 含行 `(1,…) (2,…) (3,…)`
- **WHEN** `DELETE FROM t WHERE id = 0`
- **THEN** 返回 affected 0；`WHERE id = 1/2/3` 点查均可达；`INSERT (1, …)` 仍被 DuplicateKey 拒绝（缺陷形态：affected 1、id=1 条目被误删——点查空集、INSERT 不再拒绝）

#### Scenario: 未命中大于全部键不报错

- **GIVEN** 同上表
- **WHEN** `DELETE FROM t WHERE id = 999`（键大于全部现存键）
- **THEN** 返回 affected 0，SHALL NOT 报 `KeyNotFound`（缺陷形态：误报错误）

#### Scenario: 命中路径与容忍臂零回归

- **WHEN** 运行既有 DELETE 命中、删除恢复两态与 UPSERT 冲突删除夹具用例及完整套件
- **THEN** 全部通过；除本 change 新增见证外既有测试零修改
