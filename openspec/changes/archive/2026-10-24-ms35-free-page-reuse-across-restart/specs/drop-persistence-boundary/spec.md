## MODIFIED Requirements

### Requirement: checkpoint 前崩溃保留 DROP 前的已提交状态

`DROP TABLE`、`DROP INDEX` 或 `DROP VIEW` 已经成功返回，且此后没有成功完成 checkpoint、也没有页分配复用其释放的页、也没有其它 DDL 语句成功返回时，进程结束并重新打开数据库，SHALL 仍能看到该语句执行前的对象和已提交内容。DDL 语句的边界有序落盘会把该语句的提交点与其它脏页一并写入数据文件，先前语句的目录删除因而可能在其后任一 DDL 成功返回时随该次落盘生效——该形态下旧对象在重开后不再存在，不属于本要求的崩溃点。同会话页分配复用被释放页后、在分配方语句的 catalog 落盘之前崩溃的窗口不属于本要求的崩溃模型，其结果由 `free-page-lifecycle` 域规定：未完成持久化的释放所涉及的页在任何时刻都不是可分配页，因此该窗口不再以「旧对象数据被清零且不可恢复」的形式出现。`DROP TABLE` 的原行可查询。`DROP INDEX` 的索引仍可用于等值查询，结果与崩溃前一致。`DROP VIEW` 仍可查询出基表的已提交行。同一库中未被该语句修改的其他表，其已提交行 SHALL 保持可查询。`check_integrity` 的 error 级发现数 SHALL 为 0。该崩溃模型是进程内丢弃句柄且不调用 `close`，不是真实断电。

#### Scenario: DROP TABLE 后崩溃

- **GIVEN** 表 `t` 有已提交行，表 `other` 有已提交行
- **WHEN** `DROP TABLE t` 成功返回，不 checkpoint 即丢弃数据库句柄，然后重新打开
- **THEN** `t` 与 `other` 的原行都可查询，且 `check_integrity` 的 error 级发现数为 0

#### Scenario: DROP INDEX 后崩溃

- **GIVEN** 表上存在二级索引，且按该索引列的等值查询有确定行集
- **WHEN** `DROP INDEX` 成功返回，不 checkpoint 即丢弃数据库句柄，然后重新打开
- **THEN** 同一等值查询的行集与崩溃前相同，且 `check_integrity` 的 error 级发现数为 0

#### Scenario: DROP VIEW 后崩溃

- **GIVEN** 视图查询返回基表的已提交行
- **WHEN** `DROP VIEW` 成功返回，不 checkpoint 即丢弃数据库句柄，然后重新打开
- **THEN** 该视图查询仍返回这些行

#### Scenario: 释放未持久时的分配不摧毁旧表内容

- **GIVEN** 表 `t` 有已提交行（已落盘且 WAL 已截断）
- **WHEN** `DROP TABLE t` 成功返回（未 checkpoint），此后没有其它 DDL 语句成功返回，再丢弃句柄崩溃（不调用 `close`）
- **THEN** 重新打开后 `t` 仍存在且原行可查询
- **AND** 未完成持久化的释放所涉及的页不是可分配页（重开后的新表不占用 `t` 的数据页与索引树页，`t` 的磁盘页内容保持原样）
- **AND** `check_integrity` 的 error 级发现数为 0
