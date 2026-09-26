## ADDED Requirements

### Requirement: 回滚后墓碑行的索引条目还原（PK 与唯一）

删除者事务 ROLLBACK 后，系统 SHALL 使索引状态与「该删除未发生」一致：被删除行在删除前最新存活版本的 PK 索引条目与各唯一索引条目 SHALL 全部还原（键值取自该存活版本，回滚后按该键的等值点查 SHALL 命中该行，按该唯一值的插入 SHALL 以 `DuplicateKey` 拒绝）。若同事务内该行还产生了本事务自身的新版本（先 UPDATE 后 DELETE、或 DELETE 后同键 INSERT / REPLACE），条目 SHALL 指向回滚后实际存活的版本，且同键上的残留条目 SHALL NOT 抹除该还原结果——处理顺序 SHALL 与事务内版本集合的迭代顺序无关。存活版本的键值无法读取（数据页 slot 缺失）时 SHALL 跳过该行的还原且 SHALL NOT 报错。既有 INSERT / UPDATE 回滚的条目移除与回退语义 SHALL 保持不变。崩溃恢复后的索引状态 SHALL 与运行期一致（既有 `redo_count > 0` 索引去信任重建通道承载）。

#### Scenario: DELETE 回滚后 PK 等值点查可达

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 存在行 `(1, 10)`
- **WHEN** 显式事务内 `DELETE FROM t WHERE id = 1` 后 ROLLBACK
- **THEN** `SELECT * FROM t` 产出 `(1, 10)`，且 `SELECT * FROM t WHERE id = 1` 同样产出该行（回滚前该点查漏行）

#### Scenario: DELETE 回滚后唯一值仍被复现行占用

- **GIVEN** 表 `t(id INT PRIMARY KEY, code INT UNIQUE)` 存在行 `(1, 100)`
- **WHEN** 显式事务内 `DELETE FROM t WHERE id = 1` 后 ROLLBACK
- **THEN** 自动提交插入 `(2, 100)` 以 `DuplicateKey` 拒绝（回滚后不存在两条存活行共享 `code = 100`）

#### Scenario: REPLACE 回滚后原行完整复现

- **GIVEN** 表 `t(id INT PRIMARY KEY, code INT UNIQUE)` 存在行 `(1, 'Alice', 100)`
- **WHEN** 显式事务内 `REPLACE INTO t VALUES (1, 'Zed', 300)` 后 ROLLBACK
- **THEN** 按 `id = 1` 点查产出 `(1, 'Alice', 100)`；按 `code = 100` 的唯一性检查判定其仍被占用，按 `code = 300` 的插入不被该回滚行阻断

#### Scenario: 同事务内先 UPDATE 后 DELETE 再回滚

- **GIVEN** 表 `t(id INT PRIMARY KEY, v INT)` 存在行 `(1, 10)`
- **WHEN** 同一显式事务内 `UPDATE t SET v = 20 WHERE id = 1` 后 `DELETE FROM t WHERE id = 1`，再 ROLLBACK
- **THEN** 按 `id = 1` 点查产出 `(1, 10)`（条目指向更新前版本，非被中性化的更新版本）

#### Scenario: 失败语句回滚不留索引残留

- **GIVEN** 表 `t(id INT PRIMARY KEY, n INT)` 存在行 `(1, 10)`
- **WHEN** 执行写入值类型不匹配的 `REPLACE INTO t VALUES (1, 'abc')`（删除已发生、插入段类型门拒绝、语句级回滚）
- **THEN** 按 `id = 1` 点查仍产出 `(1, 10)`，全表扫描亦为单行（失败的写语句不留下索引或数据差异）

#### Scenario: 回滚后干净重开两态一致

- **GIVEN** 上一场景的库已关闭
- **WHEN** 重新打开并查询
- **THEN** 按 `id = 1` 点查仍产出 `(1, 10)`，唯一性检查结论与关闭前一致

#### Scenario: 既有回滚语义零回归

- **GIVEN** 既有回滚用例集（INSERT 回滚同值可重插、UPDATE 回滚原值恢复、`mvcc-tombstone-visibility` 墓碑中性化扫描恢复）
- **WHEN** 重复执行
- **THEN** 行为与修复前逐字节一致
