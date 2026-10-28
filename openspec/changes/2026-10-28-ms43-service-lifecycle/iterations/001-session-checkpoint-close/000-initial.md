# Iteration 001 / Cycle 000: 会话 checkpoint 与关闭

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 需求 3、4、5；spec `session-service-lifecycle` 的「刷盘执行者与提交等待」里会话闸门那一段、「checkpoint 与关闭对刷盘循环的责任」「取消与缓冲池边界保持」。design 决策 3、4、5、6 中属于 `commit` / `checkpoint` / `close` 接线的部分。tasks.md 第 2 节的任务合同。
- Excluded scope: 全量（T3）；改 `CheckpointManager::checkpoint` 的九步、位点格式和错误类型；改 `WalWriter`；改 `BufferPool` 的任务模型或脏位交还；把 `append_commit_and_wait` 改成返回 `Err`；改 `WALBuffer` 的刷盘、暂停和停止后等待；预建缺陷修复轮。

**Objective**

读写会话的 `checkpoint` 在九步期间暂停刷盘循环，返回后循环继续按间隔写未达容量的记录。`close` 走同一段预刷和九步，返回后循环已 join，未达容量的 `append` 不再被定时器写走。`commit` 在 `CommitTxn` 到 `append_commit_and_wait` 返回之间持有会话闸门，和 `checkpoint` / `close` 的这段序列互斥。已提交行、未提交行、取消、只读和内存路径保持。

**Investigation Facts**

- Starting point: 分支 `changes/2026-10-28-ms43-service-lifecycle`。Iteration 000 已把 `WALBuffer` 的单飞刷盘、暂停/恢复和停止后等待落地，该轮 Review Result 为 `accepted`。本 Cycle 开始时 `src/database.rs` 与 `src/transaction/manager.rs` 还没有接上这些方法。000 的定向测试已通过，见该轮 Act Response：`cargo test --offline --lib wal::buffer` 为 `7 passed`，`cargo test --offline --test wal_buffer_test` 为 `5 passed`。本轮不重跑这两条，除非又改了 `src/wal/buffer.rs` 的刷盘路径。
- Current-State Evidence:
  - 读写打开在 `Database::open_internal` 里执行 `WALBuffer::new(wal_writer, 100, 100)` 并 `start_flush_loop`。容量 100，刷盘间隔 100 毫秒。只读和内存会话的 `wal_buffer`、`wal_writer`、`checkpoint_manager` 都是 `None`。`Database` 派生 `Clone`，克隆共享这些 `Arc`。
  - `WALBuffer` 已提供的调用，本轮只使用、不改语义：
    - `session_gate(&self) -> &tokio::sync::Mutex<()>`。`quiesce_for_checkpoint`、`resume_after_checkpoint`、`shutdown`、`do_flush`、`append`、`append_commit_and_wait` 都不锁它。
    - `quiesce_for_checkpoint`：已经停止则直接返回且不 `spawn`。否则把暂停设为真，`state_notify.notify_waiters()`，取得刷盘闸门后 `flush_locked`，然后释放。
    - `resume_after_checkpoint`：已经停止则直接返回且不 `spawn`。否则清除暂停并 `notify_waiters`。
    - `shutdown`：持有刷盘闸门时把停止标志设为真并刷出剩余缓冲，释放后再唤醒循环并 join。第二次调用在 handle 已取出后直接返回。
    - 停止之后的 `append_commit_and_wait` 在闸门内当场刷盘，释放后 `notified().await`，返回 `Ok(())`。
  - `TransactionManager::commit` 在 `wal_buffer` 为 `Some` 时先 `append(CommitTxn)`，再 `append_commit_and_wait`，然后才 `commit_mark_versions`。`begin` 和 `abort_by_id` 只 `append`，不等待。`commit_by_id` 不写 WAL。这些函数今天都不取会话闸门。
  - `Database::commit` 先调用 `transaction_manager.commit`，成功后再 `maybe_auto_checkpoint`。`maybe_auto_checkpoint` 在字节水位达到且没有其他活跃事务时调用公开的 `checkpoint()`。水位未到则在回收之后返回，不进入九步。
  - `Database::checkpoint` 在 `checkpoint_manager` 为 `None` 时立刻 `Ok(())`，不回收、不发布窄化。为 `Some` 时顺序是 `reclaim_abandoned_implicit_transactions`、`publish_committed_narrowings`、`CheckpointManager::checkpoint`。错误映射是 `StorageError::WalError(e.to_string())`。公开签名保持 `(self) -> Result<()>`。
  - `Database::close` 今天的函数体只有 `self.checkpoint().await`。生产路径没有 `WALBuffer::shutdown`。`close` 返回后循环仍在，间隔 100 毫秒的定时器仍会把之后 `append` 的未达容量记录写入 WAL。
  - `CheckpointManager::checkpoint` 先做零工作早退（打开时没有未决 WAL、写入计数为 0、缓冲池没有脏页则返回 `Ok(0)`），否则执行 LSN 捕获、水位读取、`flush_all`、`fsync`、写位点、追加 Checkpoint 记录、`rewrite_truncate`、位点置 0、重置计数。本轮不改这个函数。`fresh_db_zero_work_close_creates_no_site` 说明新建库、不执行语句就 `close`，会走到这个早退。
  - `BufferPool::flush_all` 写失败时从失败页起把脏位交还。生产路径没有后台任务。本轮不改这个文件。
  - 调用公开 `checkpoint()` 的还有 `maybe_auto_checkpoint`、`src/executor/truncate.rs`、`src/cli/mod.rs`、`src/cli/lifecycle.rs`、`src/storage/integrity.rs`。这些调用方应继续得到「九步之后循环恢复」的行为，本轮不改这些文件。
  - 集成测试在 crate 外，可以调用公开的 `append`、`shutdown`、`WalReader`，不能调用 `pub(crate)` 的 `session_gate` / `quiesce_for_checkpoint` / `resume_after_checkpoint`。`Database::wal_buffer` 是公开字段。WAL 路径是数据库路径的 `with_extension("wal")`。
  - `tests/statement_cancellation_test.rs` 以 `#![cfg(feature = "network")]` 编译。默认 features 含 `network`。同文件的 `setup` 与 `abandon_statement` 能在语句提交前丢掉 future。`tests/checkpoint_skip_test.rs` 的 `dml_session_close_keeps_full_checkpoint` 已经检查提交一行后 `close` 再打开，行还在。`r3_readonly_session_writes_nothing` 与 `t3_no_wal_or_checkpoint_channels` 覆盖只读和内存的 `checkpoint` / `close`。
- Code and Critical Path:
  - 入口：`TransactionManager::commit`、`Database::checkpoint`、`Database::close`、`Database::maybe_auto_checkpoint`（只通过公开 `checkpoint`）。
  - 数据：`CommitTxn` 与九步前已经在缓冲里的记录由 `quiesce_for_checkpoint` 写入。九步期间新 `append` 留在缓冲里。`close` 的 `shutdown` 把这些记录写入截断后的 WAL。`checkpoint` 的 `resume_after_checkpoint` 让定时器继续写它们。
  - 状态：会话闸门由 `commit`、`checkpoint`、`close` 在外部持有。刷盘闸门仍只由 `WALBuffer` 内部持有。`close` 之后停止标志为真，循环已 join。
  - 错误：`CheckpointManager::checkpoint` 的 `Err` 仍映射成原来的 `StorageError::WalError`。`maybe_auto_checkpoint` 的 `checkpoint failed after commit` 文案不变。`quiesce` 已经执行之后如果九步返回错误，`checkpoint` 必须先 `resume_after_checkpoint` 再把错误返回，`close` 必须先 `shutdown` 再把错误返回。循环不能停在暂停上。
  - 并发：`commit` 持有会话闸门并等待循环时，循环只需要刷盘闸门。`close` 先拿会话闸门，再在 `quiesce` / `shutdown` 里短暂拿刷盘闸门。同一任务不能对会话闸门加锁两次。

**Implementation Guidance**

`checkpoint` 与 `close` 共用这一段，且 `close` 不调用公开的 `checkpoint()`：`checkpoint_manager` 为 `None` 时立即 `Ok(())`；否则持有 `wal_buffer` 的会话闸门，依次回收被放弃的隐式事务、发布已提交窄化、`quiesce_for_checkpoint`、`CheckpointManager::checkpoint`。公开 `checkpoint` 在这段调用返回后执行 `resume_after_checkpoint`。`close` 在这段调用返回后执行 `shutdown`，不恢复循环。九步返回 `Err` 时也按各自的收尾执行，再返回原来的错误。

回收或发布窄化在 `quiesce` 之前失败时，循环还没暂停，直接返回错误。闸门随函数返回释放。

`TransactionManager::commit` 只在 `wal_buffer` 为 `Some` 时取会话闸门，范围从 `append(CommitTxn)` 之前到 `append_commit_and_wait` 返回。释放闸门之后再 `commit_mark_versions`。`begin`、`abort_by_id` 和其他 `append` 不取闸门。

辅助函数的名字由实现选择。两条路径的回收、发布、预刷和九步顺序要保持同一段，避免 `close` 再包一层公开 `checkpoint`。

**Task Definitions**

### T2: 把提交、checkpoint 和 close 接到刷盘循环

- Requirement/Scenario: spec「checkpoint 与关闭对刷盘循环的责任」的五个场景；「取消与缓冲池边界保持」的取消场景；「刷盘执行者与提交等待」里会话闸门那段。proposal S2、S3、S4、S8、S9、S10、S11。
- Depends on: T1（Iteration 000 已 accepted）
- Targets: `src/database.rs::Database::checkpoint`；`src/database.rs::Database::close`；`src/transaction/manager.rs::TransactionManager::commit`；`tests/session_lifecycle_test.rs`；`tests/statement_cancellation_test.rs`
- Current behavior: `close` 只调用 `checkpoint()`。`checkpoint` 不暂停循环，返回后定时器仍在写。`commit` 在 `CommitTxn` 与 `append_commit_and_wait` 之间不持有会话闸门。取消一条语句不会停循环。只读和内存会话在 `checkpoint_manager` 为 `None` 时直接返回。
- Required behavior:
  - 读写会话 `close` 返回之后，经 `wal_buffer` `append` 一条未达容量的 `BeginTxn`，等待 1 秒，这条记录不在 WAL 中。`WalReader::read_all` 能读完该文件。已执行过提交的 `close`，以及打开后没有执行语句就 `close`，两条都满足。`close` 本身在 2 秒内返回。
  - `checkpoint` 返回且没有 `close` 时，同样的 `append` 加 1 秒等待，记录出现在 WAL 中。
  - `commit` 在 `wal_buffer` 为 `Some` 时，从 `append(CommitTxn)` 到 `append_commit_and_wait` 返回持有会话闸门，释放之后才标记版本。`begin` 与 `abort` 不持有。
  - 已提交的一行在 `close` 后重新打开仍然可见。显式事务 `begin` 后插入、未 `commit` 就 `close`，重新打开后该行不可见。
  - `close` 返回后，同一句柄上的 `commit` 在 2 秒内返回成功，对应 `CommitTxn` 能从 WAL 读出。`commit` 与 `close` 并发时，两边都在 2 秒内返回。`close` 返回后再调用两次公开的 `WALBuffer::shutdown`，两次都返回。
  - 语句 future 在提交前被放弃之后，不调用 `close` 或 `checkpoint`。随后 `begin` 的 `BeginTxn` 在 1 秒内出现在 WAL 中，同一会话再提交另一条插入能够成功。
  - `checkpoint_manager` 为 `None` 的只读和内存会话，`checkpoint` 与 `close` 仍立即成功，不创建 WAL 文件，不改写已有 WAL。
  - 九步函数返回 `Err` 时，公开 `checkpoint` 先恢复循环再返回原错误；`close` 先 `shutdown` 再返回原错误。`maybe_auto_checkpoint` 的失败文案仍是 `checkpoint failed after commit`。
- Preserve: `CheckpointManager::checkpoint` 的九步顺序、零工作早退、位点格式和 `Result` 类型。`flush_all` 失败时脏位交还。`append_commit_and_wait` 仍返回 `Ok(())`。`dml_session_close_keeps_full_checkpoint`、`fresh_db_zero_work_close_creates_no_site`、`zero_work_close_skips_site_and_wal`、`r3_readonly_session_writes_nothing`、`t3_no_wal_or_checkpoint_channels`，以及下面 Verification 列出的整组测试文件。
- Forbidden: 修改 `src/wal/checkpoint.rs` 的九步、`src/wal/writer.rs`、`src/storage/buffer_pool.rs`，以及 `src/wal/buffer.rs` 的刷盘、暂停和停止后等待。`close` 调用会执行 `resume_after_checkpoint` 的公开 `checkpoint()`。把 `append_commit_and_wait` 改成返回 `Err`。给 `BufferPool` 加任务或 `watch`。新增 `eprintln!` / `println!` 或 `diagnostics` 模块。把 `Notify` 换成 `oneshot`，或把缓冲换成 `mpsc`。
- First test:
  - 位置与命令：新建 `tests/session_lifecycle_test.rs`，增加 `close_stops_timer_flush`。`TempDir` 下用 `Database::open` 打开 `lifecycle.db`，`execute_sql` 建表并提交一行，对 `close` 使用 `tokio::time::timeout(Duration::from_secs(2), ...)`。`close` 返回后，经 `wal_buffer` `append` 一条 `WalRecord::BeginTxn { tx_id: 424242 }`，等待 1 秒，再用 `WalReader::read_all` 断言这条记录不在 WAL 中。断言完成之后才可以再调用 `shutdown`。
  - 改前命令：`cargo test --offline --test session_lifecycle_test close_stops_timer_flush`
  - 预期 RED：`close` 能返回，但 1 秒后记录出现在 WAL 中，断言失败。循环在 `close` 之后仍按 100 毫秒间隔刷盘。
  - 同文件增加 `checkpoint_resumes_timer_flush`：同样建表并提交一行，调用 `checkpoint` 而不是 `close`，再 `append` 同一形状的 `BeginTxn`（使用另一个 `tx_id`），等待 1 秒，记录在 WAL 中。断言之后再 `close`，避免测试进程留下循环。
  - 改前命令：`cargo test --offline --test session_lifecycle_test checkpoint_resumes_timer_flush`
  - 预期：改前退出码 0。今天 `checkpoint` 不停循环，定时器会把记录写出去。
- GREEN condition: 下面 Verification 的命令退出码都是 0。`close_stops_timer_flush` 在改后通过，记录不在 WAL 中。`checkpoint_resumes_timer_flush` 仍通过。既有 checkpoint、取消、只读、内存这组测试仍通过。
- Verification:
  - `cargo test --offline --test session_lifecycle_test`，默认并行度。退出码 0。失败表示 `close` 没停循环、`checkpoint` 没恢复、提交与 `close` 挂起，或重开后的可见性变了。
  - `cargo test --offline --test checkpoint_test --test checkpoint_skip_test --test checkpoint_redo_reduction_test --test statement_cancellation_test --test readonly_session_test --test memory_db_test`，默认并行度。退出码 0。失败表示九步、零工作关闭、取消、只读或内存路径回归。
  - 同一次实现里还要有这些用例，并随上面的命令通过。数据库路径使用以 `.db` 结尾的文件，WAL 用 `with_extension("wal")`。每个会留下循环的用例在断言之后 `close` 或 `shutdown`。
    - `fresh_close_stops_timer_flush`：`open` 之后不执行语句就 `close`，再 `append` 一条未达容量的 `BeginTxn` 并等待 1 秒，记录不在 WAL 中。`close` 使用 2 秒 `timeout`。
    - `committed_row_visible_after_reopen` 不单列。S2 由既有 `dml_session_close_keeps_full_checkpoint` 承担：提交一行，`close` 后重新打开，行还在。
    - `uncommitted_row_absent_after_reopen`：`begin` 后 `execute_in_tx` 插入一行，不 `commit` 就 `close`，丢掉句柄后重新打开，查询不到该行。
    - `commit_after_close_wait_returns`：`close` 返回后，在同一句柄上 `begin`、插入并 `commit`，整体放在 2 秒 `timeout` 里。返回成功，且 WAL 中有这次事务的 `CommitTxn`。
    - `commit_and_close_finish_within_2s`：同一 `Database` 上，一个任务执行已 `begin` 并插过一行的 `commit`，另一个任务执行 `close`。两边都在 2 秒内返回成功。
    - `second_shutdown_after_close_returns`：`close` 返回后再调用两次 `wal_buffer.shutdown()`，两次都返回。
    - `cancel_leaves_flush_loop_running`：放在 `tests/statement_cancellation_test.rs`，复用 `setup` 与 `abandon_statement`。放弃之后不调用 `close` 或 `checkpoint`。`begin` 记下 `tx.id()`，等待 1 秒，WAL 中出现这个 `BeginTxn`。随后 `INSERT INTO t VALUES (900001, 1)` 成功。
  - 记录应当不出现时，出现就是失败，不重跑。记录应当出现时，若 1 秒内没出现，只把该用例再跑一次，仍没有则失败。
  - 缓冲池脏位交还：本轮 diff 不包含 `src/storage/buffer_pool.rs`。不新造写失败注入。
- Stop when: 要让 `append_commit_and_wait` 返回 `Err` 才能通过；要改九步顺序或 `flush_all` 的脏位交还才能通过；`quiesce_for_checkpoint` 或 `shutdown` 必须再取会话闸门才会死锁；零工作 `close` 一旦 `shutdown` 就会改写既有零工作测试所断言的位点或 WAL 字节。这些返回 Plan。

**Invariants**

- 成功的 `append_commit_and_wait` 返回之后才 `commit_mark_versions`。会话闸门在标记版本之前释放。
- `close` 返回后，这个 `WALBuffer` 上不再有 `flush_loop`。公开 `checkpoint` 返回后循环仍在，除非进入 `checkpoint` 时循环已经停过。
- 只读和内存会话不取得会话闸门，也不创建 WAL 缓冲。
- 九步期间新进入缓冲的记录留给 `close` 的 `shutdown` 写入截断后的文件。公开 `checkpoint` 则把它们留给恢复后的定时器。

**Non-goals**

- 不跑全量。不改取消的回收语义。不改自动 checkpoint 的水位条件。不实现诊断输出。

**Acceptance**

- `close_stops_timer_flush` 与 `fresh_close_stops_timer_flush` 在改后通过：`close` 2 秒内返回，之后追加的未达容量记录在 1 秒后仍不在可解析的 WAL 中。对应停止循环。
- `checkpoint_resumes_timer_flush` 通过。对应九步之后循环恢复。
- `dml_session_close_keeps_full_checkpoint` 通过。对应已提交行在 `close` 后仍在。
- `uncommitted_row_absent_after_reopen` 通过。对应未提交行在 `close` 后不可见。
- `commit_after_close_wait_returns`、`commit_and_close_finish_within_2s`、`second_shutdown_after_close_returns` 通过。对应闸门互斥和停止后的再次 `shutdown`。
- `cancel_leaves_flush_loop_running` 通过。对应取消不停循环。
- `r3_readonly_session_writes_nothing` 与 `t3_no_wal_or_checkpoint_channels` 通过。对应只读和内存早退。
- diff 不包含 Forbidden 列出的行为文件。`checkpoint failed after commit` 文案仍在 `maybe_auto_checkpoint`。

**Requirements Traceability Matrix**

| Requirement | Task | First Test | Status |
|---|---|---|---|
| checkpoint 与关闭对刷盘循环的责任 | T2 | `close_stops_timer_flush` | Covered |
| 提交路径持有会话闸门 | T2 | `commit_and_close_finish_within_2s` | Covered |
| 取消与缓冲池边界保持 | T2 | `cancel_leaves_flush_loop_running` | Covered |

本 Cycle 不覆盖全量。全量属于 T3。

**Verification**

- 关闭后定时器停止：`cargo test --offline --test session_lifecycle_test close_stops_timer_flush`。改前记录出现、断言失败。改后 2 秒内 `close` 返回，且 1 秒后记录不在 WAL 中。
- checkpoint 后定时器仍在：`cargo test --offline --test session_lifecycle_test checkpoint_resumes_timer_flush`。改前和改后退出码都是 0。用 WAL 里有没有这条 `BeginTxn` 判定。
- 新库级用例与保持面：`cargo test --offline --test session_lifecycle_test`，以及 `cargo test --offline --test checkpoint_test --test checkpoint_skip_test --test checkpoint_redo_reduction_test --test statement_cancellation_test --test readonly_session_test --test memory_db_test`。两条退出码都是 0。
- 缓冲池与九步：读 diff，确认没有修改 Forbidden 文件。不新造失败注入。
- 不在本 Cycle 跑不带目标过滤的 `cargo test`，也不跑 `--test-threads=1` 的全量批。

**Gate 2 Readiness**

PASS。审查计划 11 项通过，RTM 无 Missing，无未批准的裁剪。无 BLOCKED。无 SKIPPED。计划批准来自本次 goal 对已批准变更展开下一轮的授权，不是跳过。

**Persisted Evidence**

- Mode: none

定向 `cargo test` 可以重跑。命令、退出码和失败含义写入 Act Response 即可。

**Risks and Notes**

- 读写会话的刷盘间隔是打开时写死的 100 毫秒。1 秒等待覆盖多次定时器到期。「记录应当出现」的用例允许失败后只复跑一次。「记录不应当出现」的用例失败即失败。
- 公开 `checkpoint` 与 `close` 的共用前段叫什么名字，以及 `commit` 是先克隆 `Arc` 再取闸门还是在读锁内取闸门，都不改变可观察结果。
- `maybe_auto_checkpoint`、`truncate` 和 CLI 继续调用公开 `checkpoint()`，从而在九步后恢复循环。本轮不改这些调用点。
- 取消用例依赖 `statement_cancellation_test.rs` 里已有的 `abandon_statement`。默认 features 含 `network`，该文件才会编译进测试。
- 零工作 `close` 的 `shutdown` 在缓冲为空时不调用 `write_batch`。若因此改动了既有零工作测试所看的字节，按 Stop when 返回。

## Act Response

- Status: blocked

**实际改动**

- `src/database.rs` 的 `Database::close` 不再调用公开 `checkpoint()`。`Database::checkpoint` 与 `close` 共用 `checkpoint_under_session_gate`：没有 checkpoint manager 时立即 `Ok(())`；否则持有 `wal_buffer` 的会话闸门，依次回收被放弃的隐式事务、发布已提交窄化、`quiesce_for_checkpoint`、`CheckpointManager::checkpoint`。公开 `checkpoint` 在这段返回后调用 `resume_after_checkpoint`，九步的 `Err` 也先恢复再返回原来的 `StorageError::WalError`。`close` 在这段返回后调用 `shutdown`，不恢复循环。`maybe_auto_checkpoint` 的 `checkpoint failed after commit` 文案仍在 `src/database.rs` 704 行。
- `Database::checkpoint` 增加 `ResumeOnDrop`。future 在恢复之前被丢掉时，`Drop` 用当前 runtime `spawn` 一次 `resume_after_checkpoint`。正常返回先显式恢复，再解除守卫。
- `src/transaction/manager.rs` 的 `TransactionManager::commit` 在 `wal_buffer` 为 `Some` 时，从 `append(CommitTxn)` 之前到 `append_commit_and_wait` 返回持有会话闸门。闸门释放之后才 `commit_mark_versions`。`begin` 与 `abort_by_id` 不取闸门。
- 新建 `tests/session_lifecycle_test.rs`：`close_stops_timer_flush`、`checkpoint_resumes_timer_flush`、`fresh_close_stops_timer_flush`、`uncommitted_row_absent_after_reopen`、`commit_after_close_wait_returns`、`commit_and_close_finish_within_2s`、`second_shutdown_after_close_returns`。
- `tests/statement_cancellation_test.rs` 增加 `cancel_leaves_flush_loop_running`。

**与计划的偏差**

- `checkpoint` 的 future 可能在 `quiesce_for_checkpoint` 之后、`resume_after_checkpoint` 之前被丢掉。`committed_statement_survives_undelivered_result` 在事务离开活跃集后放弃 commit future，而阈值 1 的 `maybe_auto_checkpoint` 当时还在 `checkpoint` 里。丢掉 future 不会执行后面的恢复，循环停在暂停上，后续插入的提交等待不再返回。`ResumeOnDrop` 在 `Drop` 时把恢复排上运行时。`resume_after_checkpoint` 不取会话闸门。
- `second_shutdown_after_close_returns` 给两次 `shutdown` 加了 2 秒 `timeout`。计划只要求两次都返回。没有超时的话，挂起会让整条测试命令停住。
- `checkpoint_under_session_gate` 在 manager 为 `Some` 而 `wal_buffer` 为 `None` 时不取会话闸门，仍执行回收、发布和九步。计划写明这两种会话字段同时为 `None` 或同时为 `Some`，这条分支现有打开路径走不到。

**自检**

- 发现：按计划接线后，`uncommitted_row_absent_after_reopen` 失败。把 `src/database.rs` 临时恢复到本轮开始时的 `close`（只调用公开 `checkpoint()`）再跑同一用例，仍然失败，随后已把接线代码放回。失败不是这次暂停/停止引入的。
- 发现：第一次保持面命令里，`committed_statement_survives_undelivered_result` 超过 60 秒仍未结束。进程被停掉，该次没有退出码。原因见上面的偏差。`ResumeOnDrop` 之后同一条命令退出码 0。
- 已修复：取消 `checkpoint` future 后循环会恢复。
- 遗留：`uncommitted_row_absent_after_reopen` 未通过。没有在本轮里中止活跃显式事务，也没有改九步、扫描或恢复。
- Gate 3：刷盘循环的暂停、恢复、停止和提交闸门与 Task Definition 的接线顺序一致。未提交行重开不可见这一条没有实现，Gate 3 不通过。
- Gate 4：不通过。`session_lifecycle_test` 退出码 101。
- Gate 5：命中。见 Blocker Handoff。
- 跳过：不带目标过滤的 `cargo test`，以及 `--test-threads=1` 的全量。Verification 写明本 Cycle 不跑。

**验证**

- RED：`cargo test --offline --test session_lifecycle_test close_stops_timer_flush -- --exact`。退出码 101。`0 passed; 1 failed; 1 filtered out`，`finished in 1.02s`。`close` 已返回。WAL 中有 `Checkpoint { lsn: 105, ... }` 和 `BeginTxn { tx_id: 424242 }`。
- 改前：`cargo test --offline --test session_lifecycle_test checkpoint_resumes_timer_flush -- --exact`。退出码 0。`1 passed; 0 failed; 1 filtered out`，`finished in 1.05s`。
- 改前对照：把 `close` 恢复成只调用公开 `checkpoint()` 后，`cargo test --offline --test session_lifecycle_test uncommitted_row_absent_after_reopen -- --exact`。退出码 101。`0 passed; 1 failed; 6 filtered out`，`finished in 0.24s`。查询结果是 `[[Int(7)]]`。跑完已恢复接线代码。
- 保持面第一次：`cargo test --offline --test checkpoint_test --test checkpoint_skip_test --test checkpoint_redo_reduction_test --test statement_cancellation_test --test readonly_session_test --test memory_db_test`。`committed_statement_survives_undelivered_result` 超过 60 秒仍在运行，进程被停掉，没有退出码。其余已结束的二进制是 `checkpoint_redo_reduction_test` 9 passed、`checkpoint_skip_test` 6 passed、`checkpoint_test` 2 passed、`memory_db_test` 15 passed、`readonly_session_test` 15 passed，失败计数都是 0。`cancel_leaves_flush_loop_running` 在该次输出里为 ok。
- 保持面修复后：同一条命令。退出码 0。`checkpoint_redo_reduction_test` 9 passed、0 failed，`finished in 0.17s`。`checkpoint_skip_test` 6 passed、0 failed，`finished in 0.03s`。`checkpoint_test` 2 passed、0 failed，`finished in 0.01s`。`memory_db_test` 15 passed、0 failed，`finished in 0.11s`。`readonly_session_test` 15 passed、0 failed，`finished in 1.40s`。`statement_cancellation_test` 9 passed、0 failed，`finished in 7.76s`。`cancel_leaves_flush_loop_running` 与 `committed_statement_survives_undelivered_result` 都是 ok。`dml_session_close_keeps_full_checkpoint`、`fresh_db_zero_work_close_creates_no_site`、`zero_work_close_skips_site_and_wal`、`r3_readonly_session_writes_nothing`、`t3_no_wal_or_checkpoint_channels` 都在其中通过。记录应当出现的 `cancel_leaves_flush_loop_running` 第一次就通过，没有复跑。
- 接线与 `ResumeOnDrop` 都在之后：`cargo test --offline --test session_lifecycle_test`。退出码 101。`6 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out`，`finished in 1.05s`。失败用例是 `uncommitted_row_absent_after_reopen`，查询结果 `[[Int(7)]]`。`close_stops_timer_flush`、`fresh_close_stops_timer_flush`、`checkpoint_resumes_timer_flush`、`commit_after_close_wait_returns`、`commit_and_close_finish_within_2s`、`second_shutdown_after_close_returns` 为 ok。记录不应当出现的两个 close 用例失败即失败，没有重跑。

**Persisted Evidence**

None required。失败用例可以重跑。

**Experience Candidates**

- Issue：显式事务 `begin` 后插入、不 `commit` 就 `close`，默认 `Database::open`（`RepeatableRead`）重新打开后该行仍在。改前只调用公开 `checkpoint()` 的 `close` 同样返回 `[[Int(7)]]`。证据是本 Response 的「验证」一节。本 Cycle 的 Acceptance 包含这条，所以它同时是下面的阻塞，不另建 Issue 文件，本轮也没有为它扩范围修改扫描、恢复或九步。

**未解决问题**

- `uncommitted_row_absent_after_reopen` 未通过。T2 未完成。

**Blocker Handoff**

- 位置：T2，Verification 的 `cargo test --offline --test session_lifecycle_test`，Gate 4。Gate 5 因此停止。
- 计划预期：显式事务插入后不 `commit` 就 `close`，丢掉句柄再打开，查询不到该行。
- 实际：`SELECT id FROM t WHERE id = 7` 得到 `[[Int(7)]]`。接线后的退出码 101，`6 passed; 1 failed`。把 `close` 换回本轮开始时的实现，同一断言同样失败。
- 影响：T2 的其余接线已经落地，保持面命令在 `ResumeOnDrop` 之后退出码 0。这一条 Acceptance 还没达到。继续做需要新的关闭语义，Task Definition 的共用前段没有覆盖。
- 现场：插入写的是 `VersionHeader::new(tx_id, None)`（`src/executor/insert.rs`）。默认打开是 `RepeatableRead`，`statement_snapshot` 返回 `None`，无快照的扫描会交出未删除槽。主键点查在无快照时走 `read_tuple_from_data_page`，不看 `is_deleted`。`close` 先 `quiesce_for_checkpoint` 把 `BeginTxn` 和插入记录写入 WAL，九步再 `flush_all` 并 `rewrite_truncate`，这些记录被截掉，脏页留在文件里。恢复只把 WAL 里仍有 `BeginTxn` 且没有提交或中止的事务放进 `uncommitted_tx_ids`。索引重建只在 `redo_count > 0` 时发生。
- 没有沿这两条继续改：在 `close` 持有会话闸门时中止活跃显式事务，行会从页和索引里去掉，但 `commit_and_close_finish_within_2s` 要求同一个尚未写 `CommitTxn` 的事务同时 `commit` 成功。`tokio::join` 先轮询 `commit`，它在拿到会话闸门之前就会让出 `wal_buffer` 读锁；`close` 的第一个等待点就是会话闸门，因此 `close` 可以先拿闸门。此时中止会使 `commit` 返回错误。只在截断后补一条 `BeginTxn`，事务能进入未提交集合，但 `redo_count` 仍是 0，主键索引不重建，无快照的点查仍能读到该槽。改九步或 `flush_all` 属于禁止项。
- 已完成：提交闸门、`checkpoint` 恢复、`close` 停止循环，以及除未提交重开以外的本轮用例。保持面在修复取消恢复之后通过。
- 部分完成：T2。
- 未开始：本 Cycle 没有其他任务。全量不在本 Cycle。
- 修改文件：`src/database.rs`、`src/transaction/manager.rs`、`tests/session_lifecycle_test.rs`、`tests/statement_cancellation_test.rs`，以及本 Cycle 文件。未改 `src/wal/checkpoint.rs`、`src/wal/writer.rs`、`src/storage/buffer_pool.rs`、`src/wal/buffer.rs`。
- 已通过的 Gate：Gate 5 之前，保持面命令和六条生命周期用例通过。Gate 3、Gate 4 未通过。
- Evidence：None required。
- 恢复条件：计划写明 `close` 遇到仍活跃的显式事务时怎样处理，并让「重开后查不到未提交行」和「并发 `commit` 与 `close` 都返回成功」同时成立。然后继续本 Cycle，或由计划另开 Cycle。不要在现有接线顺序上做第四次同类尝试。

## Plan Review

- Review Result: pending

**Findings**

尚未审查。

**Follow-up Decision**

尚未审查。

**Next Cycle**

None

**Next Iteration**

None
