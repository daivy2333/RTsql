# 会话级后台服务的状态归属与退出

对应路线：MS43。来源：I012 里实际存在的生产消费关系（WAL 缓冲、提交通知、checkpoint、缓冲池）。约束：M19、M20（本 goal 的当次变体见下）、M21、M22。

Gate 1 为 PASS。批准来自本次 goal 指令对 tasks.md 里 MS43 的 Outcome、Scope、Non-goals、Stable result、Split signals 的授权，不是跳过。本文件不更新全局里程碑状态。

## Why

MS43 要求：把 WAL、提交通知、checkpoint 和缓冲池协调里实际的生产消费关系逐处说清状态所有者、请求完成和后台任务退出，复用已有通知与队列，保留语句、事务和会话级资源的既有边界。必要诊断随改造补齐，并衔接 M22 的报告接口。

当前代码里，只有 WAL 缓冲有后台消费者。`Database::open` 在读写模式下 `WALBuffer::start_flush_loop`（`src/database.rs` 构造 WAL 处），`flush_loop` 要等 `shutdown` 标志才退出。`Database::close` 只调用 `checkpoint()`，生产路径没有调用 `WALBuffer::shutdown`。后台任务持有 `Arc<WALBuffer>`，会话 `close` 返回后循环仍在，定时器仍可能把内存缓冲写进已经截断的 WAL。

提交通知已经在 `WALBuffer` 内部：`append_commit_and_wait` 登记 `Notify`，`do_flush` 唤醒。`CheckpointManager` 没有后台任务，由 `Database::checkpoint` / `maybe_auto_checkpoint` / `close` 调用。`BufferPool` 没有后台消费者；脏页写由 `flush_all`、`flush_pages` 和驱逐在调用方上完成。

## What Changes

- WAL 刷盘在同一时刻只有一个执行者。取走某一批缓冲和 `pending_commits` 的那次 `do_flush` 负责把该批交给 `WalWriter::write_batch`，并唤醒这批里登记过的提交等待者。
- `WALBuffer::shutdown` 保持现有合同：刷出剩余缓冲、停止循环、join 后台任务。循环已经停止之后，`append_commit_and_wait` 由调用方在同一把刷盘闸门下同步完成，返回 `Ok(())`，不挂起。
- `write_batch` 失败时的处置保持现状：`do_flush` 仍唤醒已经取走的等待者，`append_commit_and_wait` 仍返回 `Ok(())`。不把这条路径改成 `Err`。
- 会话级互斥：`TransactionManager::commit` 在写入 `CommitTxn` 和 `append_commit_and_wait` 返回之间持有 WAL 会话闸门；`Database::checkpoint` 与 `Database::close` 在自己的协作序列期间持有同一把闸门。后到的一方等待。不新增错误类型。
- `checkpoint`（含自动 checkpoint）在进入 `CheckpointManager::checkpoint` 之前：等进行中的 `do_flush` 结束，再执行一次 `do_flush`，然后暂停后台循环。九步期间循环不运行。九步返回后恢复循环。
- `close` 使用同一段暂停和九步，九步返回后调用 `shutdown`（再刷一次并 join），不恢复循环。预先刷出的记录落在捕获的 LSN 之前，随既有截断进入前缀。九步期间新进入缓冲的记录由 `shutdown` 的那次刷盘写到截断后的文件，避免被丢掉。安静关闭（九步期间没有新的 `append`）的最终 WAL 仍只有既有 checkpoint 后缀。
- 语句取消不暂停、不 join WAL 循环，不拆除缓冲池。缓冲池不新增后台任务，不引入 `watch`。
- 不新增 `rtsql::diagnostics`，不新增 `eprintln!` / `println!`。本变更没有新的用户可见失败文案。

## Capabilities

### New Capabilities

- `session-service-lifecycle`：读写会话上 WAL 刷盘循环、提交等待、checkpoint 协作和 `close` 退出的责任；以及这些路径不改变的提交结果、取消和缓冲池边界。

### Modified Capabilities

无。既有提交成功、checkpoint 九步、只读/内存关闭、语句取消的规格文本不改。新规格只增加生命周期责任，并写明上述结果保持。

## Impact

- 代码：`src/wal/buffer.rs`（刷盘闸门、循环暂停/恢复、`shutdown` 之后的同步完成）、`src/transaction/manager.rs`（`commit` 在 `CommitTxn` 与等待之间持有会话闸门）、`src/database.rs`（`checkpoint` / `close` 的暂停、预刷、恢复或 `shutdown`）。
- 不改：`src/wal/checkpoint.rs` 九步顺序、`src/wal/writer.rs`、`src/storage/buffer_pool.rs`、执行器与 `table_manager` 的 `append` 调用点、错误类型、SQL 结果、磁盘格式。
- 测试：扩展 `tests/wal_buffer_test.rs`；在库级测试中覆盖 `close` 停循环、`checkpoint` 后循环仍在、取消后循环仍在。既有 `tests/checkpoint_*.rs`、`tests/statement_cancellation_test.rs`、`tests/readonly_session_test.rs`、`tests/memory_db_test.rs` 保持通过。
- 关联：MS43；I012；M19（全量只在最后一轮、分批串行）；M21（`wal_buffer` 仍归 `Database`，后台对象生命周期随会话）；M22（本变更不实现诊断门面）。

## 需求

1. 同一次刷盘取走的缓冲记录和提交等待由这一次 `do_flush` 完成：写盘尝试结束之后唤醒这些等待者。成功路径上，`append_commit_and_wait` 在对应批次 `write_batch` 返回之后返回 `Ok(())`，`TransactionManager::commit` 再标记版本。
2. `shutdown` 把剩余缓冲写入 WAL、停止后台循环并 join。循环停止之后的 `append_commit_and_wait` 在有限时间内返回 `Ok(())`，且对应记录可读。`write_batch` 失败时仍返回 `Ok(())` 并唤醒已取走的等待者。
3. `checkpoint` 与后台 `do_flush` 不并发。进入九步前先刷出当时已在缓冲中的记录。九步之后循环继续运行，后续提交仍可由该循环完成。
4. `close` 在九步之后停止并 join 循环。返回之后，低于容量的普通 `append` 不再被定时器写盘。已提交的行在 `close` 后重开仍在。只读和内存会话的 `close` / `checkpoint` 仍是无 WAL 任务的既有路径。
5. 语句取消不停止 WAL 循环，也不改变缓冲池的刷盘失败处置（失败写盘把脏位交还调用方）。
6. 整个 change 收尾时按 M19 分批串行跑一次全量。缺一批不得声明全量通过。

## 不做

- 把 `Mutex` 队列换成 `mpsc`，把 `Notify` 换成 `oneshot`，或给缓冲池加 `watch`。I012 的通道方案不执行。
- 所有锁改通道、全库 actor 化、组提交性能优化、节点级锁、批量 I/O、io_uring。
- 新增取消 API、跨进程读写。
- 把 `append_commit_and_wait` 在写盘失败时改成 `Err`，或改变已提交数据的可见性、checkpoint 九步、位点格式、恢复规则。
- 实现 M22 的诊断门面或迁移现有 `eprintln!`。
- 改索引写侧闸门、行锁、计划缓存。
- 预建「修复尚未发现的缺陷」的 Iteration。

## 场景草图

| # | 类别 | 前置 | 动作 | 可观察结果 | 失败边界 |
|---|---|---|---|---|---|
| S1 | Happy | 读写库，容量未满 | 两个事务提交 | 两者都返回成功；WAL 可解析；重开可见已提交行 | 等待不返回即失败 |
| S2 | Happy | 已提交一行 | `close` 后重开 | 行仍在 | 行丢失即破坏既有持久化 |
| S3 | Happy | `close` 已返回，循环应已停止 | 再 `append` 一条未达容量的记录并等待超过刷盘间隔 | WAL 中不出现这条记录 | 定时器仍写盘说明循环没退出 |
| S4 | Happy | `checkpoint` 已返回，会话未 `close` | 同上，等待超过间隔 | 记录出现在 WAL | 循环没恢复则后续提交只能靠调用方刷盘，与本需求不符 |
| S5 | Sad | `shutdown` 已返回 | `append_commit_and_wait` | 2 秒内返回 `Ok(())`，记录可读 | 超时即挂起 |
| S6 | Sad | `write_batch` 返回错误 | 已登记的等待者 | 仍被唤醒，等待返回 `Ok(())` | 改成 `Err` 即改变事务合同，本范围禁止 |
| S7 | Edge | 两次 `do_flush` 重叠 | 并发进入 | 记录可解析，等待者最终返回 | 帧损坏即刷盘执行者重叠 |
| S8 | Edge | 显式事务已 `begin` 但未提交，缓冲中有记录 | `close` 后重开 | 未提交行不可见；安静路径的最终 WAL 不含这条未提交记录 | 未提交行变成已提交即合同变化 |
| S9 | Edge | 只读或内存会话 | `close` / `checkpoint` | 成功返回，不创建 WAL 任务 | panic 或写出 WAL 即回归 |
| S10 | 组合 | 语句 future 在提交前被放弃 | 放弃后 `begin` 一条记录并等待超过间隔 | 循环仍把记录刷出；随后提交成功 | 取消把循环停掉即违反既有取消规格 |
| S11 | 组合 | 提交与 `close` 先后发生 | 先完成的一方结束后再跑另一方 | 先提交的行在 `close` 后仍在；`close` 之后的提交等待返回成功且记录可读 | 死锁或已提交行丢失 |
| S12 | 错误 | checkpoint 步骤失败 | 触发边界的提交 | 仍按既有规格点名失败，已提交行不回滚 | 本变更不改这条错误 |
| S13 | 兼容 | 默认构建 | M19 分批全量 | 每批退出码 0 且 failed 为 0，目标对账无缺无重 | 缺一批不得算通过 |

## 回滚

改动在 change 分支的 `src/wal/buffer.rs`、`src/transaction/manager.rs`、`src/database.rs` 和测试。回滚是还原或放弃该分支，主分支在合并前不被改写。无格式迁移，无新的磁盘字段。

## 默认假设

下列假设闭合场景缺口。它们不扩大 MS43 的 Scope，不把提交或 checkpoint 的成功/失败 Result 改成新合同，也不违反 Non-goals。

1. 不按 I012 原文替换通道。代码里只有 WAL 刷盘循环是后台消费者；提交通知已经是 `Notify`；checkpoint 和缓冲池没有后台消费者。依据：`src/wal/buffer.rs`、`src/wal/checkpoint.rs`、`src/storage/buffer_pool.rs`，以及 Non-goals「不把所有 Mutex 改成 channel」。
2. 写盘失败仍返回 `Ok(())`。依据：Non-goals 禁止改变既有事务合同。现有 `IoFailPoint::WalFsync` 只接在 `WalWriter::fsync`，组提交的 `write_batch` 不经过它；不把该注入点扩到 `write_batch`，以免改变 `tests/drop_persistence_test.rs` 的失败位置。S6 用任务约束和 diff 核对保持，不新造注入器。
3. `checkpoint` / `close` 在九步之前多做的那次 `do_flush`，把「定时器刚好在捕获 LSN 之前触发」收成稳定顺序。这些记录位于截断前缀，安静关闭的最终 WAL 仍是既有 checkpoint 后缀。依据：`wal-auto-checkpoint` 要求提交确认发生在 `sync_all` 之后且先于 checkpoint；已提交记录不依赖这次预刷。
4. `close` 在九步之后仍执行 `shutdown` 的刷盘，而不是丢掉九步期间新进入缓冲的记录。依据：丢掉后，同一 `Database` 上随后返回成功的提交可能找不到更早的 `append`。安静路径下这次刷盘为空。
5. 提交与 `close` / `checkpoint` 用会话闸门排队，不新增错误。依据：今天两边可以交错；闸门把交错收成一方完成再另一方开始，成功 Result 不变。`begin`、语句 `append`、`abort` 的 `append` 不取这把闸门。
6. 不建诊断门面，不新增标准输出。依据：M22 把用户可见 stderr 的有无变化当作行为变化；本变更没有新的用户可见失败。M22 的门面仍留给 MS44。
7. 语句取消保持 `statement-cancellation-outcome`：不拆除 WAL 循环和缓冲池。
8. 不同服务不拆成第二个 change。WAL 与提交通知是同一个 `WALBuffer`。checkpoint 的新增责任是相对这条循环的顺序。缓冲池没有独立的后台生命周期。三轮 Iteration 可以分开验证和排障。Split signals 里「无法在同一验证范围闭合才分开计划」的条件不成立。

## 后续 change

不创建。边界判定写在上一节假设 8。MS44 不在本变更里实施。

## 实施期缺陷

本 goal 覆盖 M20 的「不在当前 change 内扩范围修复」，仅限本 change 实施中新发现的实质缺陷。登记之后由后续派发在当前 change 增加一轮 Iteration。本计划不预建那一轮。

## 未自行批准的缺口

无。写盘失败改成 `Err`、新增取消 API、改持久化格式，都落在 Non-goals 里，本计划不采用。
