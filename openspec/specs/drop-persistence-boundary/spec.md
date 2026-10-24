# drop-persistence-boundary Specification

## Purpose

DROP 类语句与受控持久化失败的可观察恢复结果。崩溃模型是进程内丢弃句柄且不调用 `close`（WAL shutdown + drop，脏页丢失），不是真实断电；受控失败经库测试开关注入（数据文件 `write_page` / WAL `fsync` 在系统调用前失败一次）或位点路径被目录占据，不产生半页写入。释放语义配套：页清零发生在空闲页被再次分配时，不在释放时——checkpoint 前崩溃后磁盘页保留原内容。来源：MS33-T01 / I086（change `2026-10-23-ms33-drop-persistence-failure`，两 Iteration accepted，2026-10-24 收尾）。

## Requirements

### Requirement: checkpoint 前崩溃保留 DROP 前的已提交状态

`DROP TABLE`、`DROP INDEX` 或 `DROP VIEW` 已经成功返回，且此后没有成功完成 checkpoint、也没有页分配复用其释放的页时，进程结束并重新打开数据库，SHALL 仍能看到该语句执行前的对象和已提交内容。同会话页分配复用被释放页后、在分配方语句的 catalog 落盘之前崩溃的窗口不属于本要求的崩溃模型，其结果由空闲页生命周期的后续能力规定。`DROP TABLE` 的原行可查询。`DROP INDEX` 的索引仍可用于等值查询，结果与崩溃前一致。`DROP VIEW` 仍可查询出基表的已提交行。同一库中未被该语句修改的其他表，其已提交行 SHALL 保持可查询。`check_integrity` 的 error 级发现数 SHALL 为 0。该崩溃模型是进程内丢弃句柄且不调用 `close`，不是真实断电。

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

### Requirement: 同会话再分配不读到已释放页的旧行

`DROP TABLE` 或 `DROP INDEX` 成功之后，同一打开会话中的下一次页分配 SHALL 得到空页。用该页建成的新表 SHALL 只包含新插入的行。

#### Scenario: 删表后立即建表

- **GIVEN** `DROP TABLE t` 已成功，会话尚未关闭
- **WHEN** 创建另一张表并插入一行，再查询该表
- **THEN** 结果只有这一行新数据

### Requirement: 删列成功后的崩溃保留删列结果

`ALTER TABLE DROP COLUMN` 成功返回后，不 checkpoint 即丢弃数据库句柄并重新打开，被删列 SHALL 不再存在，其余列的已提交值与该语句返回后的值一致，`check_integrity` 的 error 级发现数 SHALL 为 0。本要求不把 `DROP TABLE`、`DROP INDEX`、`DROP VIEW` 或 `TRUNCATE` 改成语句返回前落盘。

#### Scenario: 删列后崩溃

- **GIVEN** 表有两列已提交数据
- **WHEN** `ALTER TABLE DROP COLUMN` 成功返回，不 checkpoint 即丢弃数据库句柄，然后重新打开
- **THEN** 被删列不存在，剩余列的值与删列后一致，且 `check_integrity` 的 error 级发现数为 0

### Requirement: 加密库的语句崩溃结果与明文库同类

使用密钥打开的库上，`CREATE TABLE` 成功返回后不 checkpoint 即崩溃，用同一密钥重新打开，该表 SHALL 存在且可写入、读回。同一打开方式下的 `DROP TABLE` SHALL 满足「checkpoint 前崩溃保留 DROP 前的已提交状态」。本要求不加密 `.wal` 或 `.checkpoint`。

#### Scenario: 加密库建表后崩溃

- **GIVEN** 使用密钥创建并打开的库
- **WHEN** `CREATE TABLE` 成功返回，不 checkpoint 即丢弃句柄，再用同一密钥打开
- **THEN** 该表存在，插入的行可以读回

#### Scenario: 加密库删表后崩溃

- **GIVEN** 加密库中的表有已提交行
- **WHEN** `DROP TABLE` 成功返回，不 checkpoint 即丢弃句柄，再用同一密钥打开
- **THEN** 原行仍可查询，且 `check_integrity` 的 error 级发现数为 0

### Requirement: 受控 I/O 失败有两种恢复结果

测试可以让下一次数据文件写入或 WAL `sync` 在不修改目标文件字节的情况下失败一次。数据文件写入失败发生在 checkpoint 刷脏页（九步之第 2 步）之前，此时 checkpoint SHALL 返回错误，重新打开后的状态 SHALL 与「checkpoint 前崩溃保留 DROP 前的已提交状态」相同。数据文件 `sync` 无生产调用点，不设开关位。

WAL `sync` 失败与 `.checkpoint` 路径被一个目录占据都发生在脏页已经写完之后（九步之第 3 步起）。这两种情况下 checkpoint SHALL 返回错误；重新打开后，已成功的 `DROP TABLE` SHALL 已经生效，其他表的已提交行仍可查询，`check_integrity` 的 error 级发现数 SHALL 为 0。

上述结果都来自进程内的失败注入或文件系统替换，不代表真实断电。

#### Scenario: 写入在改文件之前失败

- **GIVEN** `DROP TABLE t` 已成功，且 `t` 的 catalog 删除还只在缓冲池中
- **WHEN** 下一次数据文件写入在不修改文件的情况下失败，并调用 checkpoint，然后重新打开
- **THEN** checkpoint 返回错误，`t` 的原行仍可查询

#### Scenario: 同步在不改变已写字节的情况下失败

- **GIVEN** `DROP TABLE t` 已成功，其他表有已提交行
- **WHEN** 下一次 WAL `sync` 不调用 `sync_all` 即失败，并调用 checkpoint，然后重新打开
- **THEN** checkpoint 返回错误，`t` 不存在，其他表的行仍可查询，且 `check_integrity` 的 error 级发现数为 0

#### Scenario: 位点文件无法创建

- **GIVEN** `DROP TABLE t` 已成功，其他表有已提交行
- **WHEN** `.checkpoint` 路径是一个目录，并调用 checkpoint，然后重新打开
- **THEN** checkpoint 返回错误，`t` 不存在，其他表的行仍可查询，且 `check_integrity` 的 error 级发现数为 0
