# session-service-lifecycle Specification

## Purpose
规定读写会话上 WAL 刷盘循环、提交等待、checkpoint 与 `close` 的状态归属和任务退出。已提交结果、checkpoint 九步、语句取消、只读和内存会话保持既有合同。

## Requirements

### Requirement: 刷盘执行者与提交等待

`WALBuffer` SHALL 保证同一时刻只有一个 `do_flush` 执行写盘。取走某一批缓冲记录和 `pending_commits` 的那一次 `do_flush` SHALL 在该次 `WalWriter::write_batch` 返回之后，唤醒这批中已经登记的提交等待者。成功时 `append_commit_and_wait` SHALL 返回 `Ok(())`。`TransactionManager::commit` SHALL 在该等待返回成功之后才标记版本已提交。

`TransactionManager::commit` SHALL 在追加 `CommitTxn` 和 `append_commit_and_wait` 返回之间持有与 `checkpoint` / `close` 相同的会话闸门。`begin`、数据 `append` 和 `AbortTxn` 的 `append` SHALL NOT 持有这把闸门。

#### Scenario: 并发提交等待都返回

- **GIVEN** 一个已启动刷盘循环的 `WALBuffer`，容量足以放下两条未提交记录
- **WHEN** 两个任务各自 `append` 一条 `BeginTxn` 后并发 `append_commit_and_wait`
- **THEN** 两个等待都返回 `Ok(())`，WAL 文件可被 `WalReader` 完整读出

#### Scenario: 重叠刷盘不写坏记录

- **GIVEN** 缓冲中已有记录
- **WHEN** 两个任务同时调用 `do_flush`
- **THEN** 两次调用都返回，WAL 可被完整读出，记录条数与刷盘前缓冲中的记录数一致

### Requirement: shutdown 与写盘失败的完成方式

`WALBuffer::shutdown` SHALL 把当时缓冲中的记录写入 WAL，使后台循环退出，并 join 该任务。join 返回之后，低于容量的 `append` SHALL NOT 再被后台定时器写盘。

循环已经退出之后，`append_commit_and_wait` SHALL 由调用方同步完成刷盘，SHALL 在 2 秒内返回 `Ok(())`，且该次等待对应的记录 SHALL 可从 WAL 读出。

`write_batch` 返回错误时，已经取走该批的 `do_flush` SHALL 仍唤醒这批等待者，`append_commit_and_wait` SHALL 仍返回 `Ok(())`。本需求 SHALL NOT 把该路径改成 `Err`。

#### Scenario: shutdown 刷出缓冲并停掉定时器

- **GIVEN** 缓冲中有未达容量的记录，WAL 文件仍为空
- **WHEN** `shutdown` 返回，然后再 `append` 一条未达容量的记录并等待超过刷盘间隔
- **THEN** `shutdown` 返回时先前的记录已经在 WAL 中；后追加的那条在等待结束时仍不在 WAL 中

#### Scenario: 循环停止后的提交等待返回成功

- **GIVEN** `shutdown` 已经返回，之后又 `append` 了一条未达容量的记录
- **WHEN** 调用 `append_commit_and_wait`
- **THEN** 调用在 2 秒内返回 `Ok(())`，该记录出现在 WAL 中，且 WAL 可被 `WalReader` 完整读出

### Requirement: checkpoint 与关闭对刷盘循环的责任

`Database::checkpoint` 与 `Database::close` SHALL 在调用 `CheckpointManager::checkpoint` 之前持有会话闸门，等待进行中的 `do_flush` 结束，再执行一次 `do_flush`，然后暂停后台循环。九步执行期间后台循环 SHALL NOT 运行。`CheckpointManager::checkpoint` 的九步顺序、位点格式和错误返回 SHALL 保持不变。

`Database::checkpoint`（含 `maybe_auto_checkpoint` 触发的同一次调用）SHALL 在九步返回后恢复后台循环。恢复之后，未达容量的 `append` SHALL 仍由该循环按刷盘间隔写盘。

`Database::close` SHALL 在九步返回后 `shutdown` 刷盘循环并 join，SHALL NOT 在 join 之后恢复循环。已提交的行 SHALL 在 `close` 之后重新打开时仍然可见。只读会话和内存会话没有 WAL 缓冲时，`close` 与 `checkpoint` SHALL 保持既有无 WAL 写入路径。

显式事务已经 `begin` 并插入一行、调用方没有 `commit` 就调用 `close` 时，库 SHALL NOT 代替调用方回滚该事务。按默认 `Database::open` 重新打开并查询时，该行仍被查到。本句记录的是改前 `close` 只调用公开 `checkpoint()` 时已经存在的结果，不新增持久化承诺，也不覆盖「刷盘后丢弃句柄、不调用 `close`」的既有测试 `tests/mvcc_tombstone_visibility_test.rs` 的 `flushed_uncommitted_insert_does_not_resurrect`（规格 `mvcc-tombstone-visibility` 的场景「落盘未提交行重启后不复活」）。

#### Scenario: 关闭后定时器不再写盘

- **GIVEN** 读写会话已经成功提交过数据并 `close`
- **WHEN** 经该会话的 `wal_buffer` 再 `append` 一条未达容量的记录，并等待超过刷盘间隔
- **THEN** 这条记录不在 WAL 中

#### Scenario: 关闭后已提交行仍在

- **GIVEN** 读写会话已经成功插入并提交一行
- **WHEN** `close` 后重新打开同一路径并查询
- **THEN** 该行存在

#### Scenario: checkpoint 之后循环仍在

- **GIVEN** 读写会话执行 `checkpoint` 且没有 `close`
- **WHEN** `append` 一条未达容量的记录并等待超过刷盘间隔
- **THEN** 该记录出现在 WAL 中

#### Scenario: 关闭后未提交插入仍被查到

- **GIVEN** 读写会话 `begin` 之后插入了一行但没有提交
- **WHEN** `close` 后按默认打开重新打开并查询该行
- **THEN** 该行仍被查到。这与改前 `close` 只调用公开 `checkpoint()` 的结果相同。本场景记录现行结果，不覆盖刷盘后丢弃句柄且不调用 `close` 的既有测试

#### Scenario: 只读与内存会话不拥有刷盘循环

- **GIVEN** 只读会话或内存会话
- **WHEN** 调用 `checkpoint` 和 `close`
- **THEN** 两者都返回成功，且不创建 WAL 文件（内存会话）或不改写已有 WAL（只读会话的既有无写入合同）

### Requirement: 取消与缓冲池边界保持

语句 future 在提交完成前被放弃时，库 SHALL NOT 因此暂停或 join WAL 刷盘循环，SHALL NOT 因此改变 `BufferPool` 的生命周期。取消之后，未达容量的 `append` SHALL 仍由原循环按间隔写盘。`BufferPool` SHALL NOT 为此新增后台任务。`flush_all` 写盘失败时 SHALL 继续把尚未写成功的页的脏位交还给缓冲池，错误仍返回给调用方。

#### Scenario: 取消后刷盘循环仍按间隔写盘

- **GIVEN** 一条尚未提交的隐式写语句的 future 被放弃，会话随后仍可执行
- **WHEN** 同一会话 `begin`（只追加 `BeginTxn`，不提交）并等待超过刷盘间隔
- **THEN** 该 `BeginTxn` 出现在 WAL 中，且会话可以继续提交另一条写入

### Requirement: 全量回归只跑一次且分批串行

本能力的收尾验证 SHALL 按 M19 执行一次全量，且只执行一次。测试目标 SHALL 按确定性顺序分批，每批一次独立的 `cargo test`，批内 `--test-threads=1`，批间顺序执行。SHALL NOT 使用一次无目标过滤的全量调用，SHALL NOT 使用默认并行度跑全量批次。任一批缺失、退出码非 0 或 failed 非 0 时，SHALL NOT 声明全量通过。单次目标测试 MAY 使用默认并行度。

#### Scenario: 分批全量全部通过

- **GIVEN** 本 change 的行为任务已经完成
- **WHEN** 按最后一轮 Iteration Plan 写明的分批方式跑完全部批次
- **THEN** 每一批退出码为 0 且 failed 为 0，批次目标相对枚举清单无缺失、无多余、无重复
