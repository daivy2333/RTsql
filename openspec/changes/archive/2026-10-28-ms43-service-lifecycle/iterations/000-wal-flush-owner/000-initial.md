# Iteration 000 / Cycle 000: WAL 刷盘所有者

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 需求 1 和 2；spec `session-service-lifecycle` 的「刷盘执行者与提交等待」「shutdown 与写盘失败的完成方式」。design 决策 1、2、3 中属于 `WALBuffer` 的部分。
- Excluded scope: `Database::commit` / `checkpoint` / `close` 的接线（T2）；全量（T3）；缓冲池、`WalWriter`、checkpoint 九步、诊断门面、把写盘失败改成 `Err`。

**Objective**

`WALBuffer` 在同一时刻只有一个 `do_flush` 写盘。`quiesce_for_checkpoint` 先刷出当前缓冲再暂停定时器。`resume_after_checkpoint` 恢复定时器。`shutdown` 仍刷出剩余缓冲并 join。循环退出之后，再 `append` 的未达容量记录不会被定时器写走，随后的 `append_commit_and_wait` 在 2 秒内返回 `Ok(())` 并把该记录写进 WAL。

**Investigation Facts**

- Starting point: 分支 `changes/2026-10-28-ms43-service-lifecycle`，基点 master `60e107965a4f728b0dcbe157bf39c59f8c9dbb0f`。本 Cycle 开始时产品代码未改。既有 `tests/wal_buffer_test.rs` 的四个用例（`test_wal_buffer_append_returns_lsn`、`test_wal_buffer_flush_on_capacity`、`test_group_commit_multiple_txns`、`test_wal_buffer_shutdown_flushes`）是变更前的保持面；实现前先跑一次，通过则沿用为改前 GREEN。
- Current-State Evidence:
  - `WALBuffer`（`src/wal/buffer.rs`）的后台循环由 `start_flush_loop` 以 `tokio::spawn` 启动，任务持有 `Arc<WALBuffer>`。`flush_loop` 在 `flush_notify` 或 `flush_interval_ms` 上 `select`，看到 `shutdown` 标志为真就退出，退出前不再 `do_flush`。
  - `do_flush` 的调用者有三处：`append` 在 `buffer.len() >= capacity` 时在调用方上调用；`flush_loop`；`shutdown`。三处都是先取走 `buffer` 和 `pending_commits`，再在锁外 `write_batch`。没有共同的在飞闸门。
  - `append_commit_and_wait` 登记 `Arc<Notify>`、把 `tx_id` 推入 `pending_commits`、`flush_notify.notify_one()`，然后 `notified().await`，最后 `Ok(())`。它不检查停止标志。
  - `do_flush` 在 `records` 非空时调用 `write_batch`。`write_batch` 返回 `Err` 时分支为空，不把错误传出。无论写盘是否执行，只要 `committed_tx_ids` 非空，就 `remove` 对应 `Notify` 并 `notify_one`。缓冲为空但 pending 非空时也会唤醒。
  - `shutdown` 先把 `shutdown` 原子量设为真，再 `do_flush`，再 `notify_one`，然后 `take` `JoinHandle` 并 `await`。第二次调用时 handle 已是 `None`，函数返回。生产路径的 `Database::close` 不调用它；许多集成测试在丢弃库之前调用它来刷盘并停循环。
  - `Database::open_internal` 只在 `OpenMode::ReadWrite` 时 `WALBuffer::new(writer, 100, 100)` 并 `start_flush_loop`。`TransactionManager` 经 `set_wal_buffer` 持有同一 `Arc`。内存和只读会话的 `wal_buffer` 为 `None`。
  - 只 `append`、不等待的调用者：`TransactionManager::begin` / `abort`；`insert` / `update` / `delete` / `upsert` 执行器；`table_manager` 的删除、插入和列收窄。本 Cycle 不改这些调用点。
  - `CheckpointManager::checkpoint` 与 `BufferPool::flush_all` 没有后台任务。本 Cycle 不改这两个文件。
  - `WalWriter::write_batch` 失败不能靠现有 `IoFailPoint::WalFsync` 打进组提交：该注入点只在 `WalWriter::fsync`。`write_batch` 自己 `sync_all`。
- Code and Critical Path:
  - 入口：`WALBuffer::append`、`append_commit_and_wait`、`do_flush`、`start_flush_loop`、`shutdown`。
  - 数据：记录先进 `buffer`，提交等待者进 `commit_waiters` 与 `pending_commits`。`do_flush` 取走这两份，非空批次交给 `wal_writer.write_batch`，再按取走的 `tx_id` 唤醒。
  - 状态：`shutdown: AtomicBool` 今天只表示循环应退出。本 Cycle 增加暂停状态和刷盘闸门；停止后不得再 `spawn`。
  - 错误：`write_batch` 的 `Err` 继续被 `do_flush` 吃掉，等待者仍被唤醒，`append_commit_and_wait` 仍返回 `Ok(())`。
  - 并发：刷盘闸门把写盘串行化。会话闸门本 Cycle 只添加 `pub(crate)` 访问，不在这些方法内部获取。T2 的 `commit` 会在外部持有它并调用 `append` 与 `append_commit_and_wait`。

**Implementation Guidance**

方法名使用 `quiesce_for_checkpoint`、`resume_after_checkpoint`，可见性 `pub(crate)`，不从 `lib.rs` 重导出。会话闸门是 `pub(crate)` 方法返回的 `&tokio::sync::Mutex<()>`，本 Cycle 的上述方法不得获取它。

刷盘闸门包住「取走缓冲和 pending、调用 `write_batch`、唤醒取走的等待者」这一段。`notified().await` 之前必须释放。暂停等待期间也必须释放，否则 `shutdown` 无法进入刷盘。

`shutdown` 在持有刷盘闸门时把停止标志设为真并 `do_flush`，释放后再唤醒循环并 join。`append_commit_and_wait` 在刷盘闸门内登记并入队；已经停止则当场 `do_flush`，否则释放并 `notify` 循环。两种都在释放之后 `await`。

`quiesce_for_checkpoint` 把状态设为暂停，唤醒循环，获取刷盘闸门（等到进行中的 `do_flush` 离开），再 `do_flush` 一次，释放。循环暂停时不刷盘，停在可被恢复或停止唤醒的点上。已经停止时，`quiesce_for_checkpoint` 在 2 秒内返回，不 `spawn`。`resume_after_checkpoint` 在已经停止时不得 `spawn`。

`append` 因容量触发的 `do_flush` 在暂停且未停止时等待恢复或停止，再刷。不得把 `get_current_lsn()` 失败时的 `unwrap_or(0)` 改成错误返回。

**Task Definitions**

### T1: WALBuffer 的刷盘、暂停和停止后等待

- Requirement/Scenario: spec「刷盘执行者与提交等待」的两个场景；「shutdown 与写盘失败的完成方式」的两个场景。proposal S5、S6、S7。
- Depends on: None
- Targets: `src/wal/buffer.rs::WALBuffer`；`src/wal/buffer.rs` 内 `#[cfg(test)]` 模块；`tests/wal_buffer_test.rs`
- Current behavior: 三处 `do_flush` 可以交错进入写盘。`shutdown` 会刷盘并 join，join 之后新的 `append_commit_and_wait` 登记的等待者没有循环再去 `do_flush`，`notified()` 不返回。没有暂停/恢复方法。写盘失败时等待者仍被唤醒，等待函数返回 `Ok(())`。
- Required behavior:
  - 同一时刻只有一个 `do_flush` 执行 `write_batch`。取走的 `pending_commits` 在该次写盘尝试返回之后被唤醒，包括缓冲为空和 `write_batch` 返回 `Err`。`append_commit_and_wait` 仍返回 `Ok(())`。
  - `quiesce_for_checkpoint` 把调用前已经在缓冲中的记录写入 WAL，然后定时器不再写新的未达容量记录。`resume_after_checkpoint` 之后定时器恢复写这些记录。
  - `shutdown` 仍把当时缓冲写入 WAL 并 join。join 之后未达容量的 `append` 不会在刷盘间隔之后出现在 WAL 中。此后 `append_commit_and_wait` 在 2 秒内返回 `Ok(())`，并且把 `shutdown` 之后 `append` 的那条记录写进可解析的 WAL。
  - 第二次 `shutdown` 返回，不 panic。已经停止后再 `quiesce_for_checkpoint` 或 `resume_after_checkpoint`，不启动新任务，未达容量的 `append` 仍不会被定时器写走。
  - 刷盘闸门不跨 `notified().await` 持有，暂停等待也不持有它。`session_gate()` 返回会话闸门的共享引用。`quiesce_for_checkpoint`、`resume_after_checkpoint`、`shutdown`、`do_flush`、`append`、`append_commit_and_wait` 不获取这把闸门。当前任务已经持有它时，`quiesce_for_checkpoint` 仍在 2 秒内返回。
- Preserve: 既有四个 `wal_buffer_test` 用例的断言。`shutdown` 被测试用来「刷盘并停循环」的合同，包括 `src/cli/mod.rs` 里恢复夹具对 `wal_buffer.shutdown()` 的调用，调用点不改但行为必须仍成立。组提交仍是多个等待者由一次取走唤醒。容量触发仍会刷盘。
- Forbidden: 修改 `src/database.rs`、`src/transaction/manager.rs`、`src/wal/checkpoint.rs`、`src/wal/writer.rs`、`src/storage/buffer_pool.rs`。把 `append_commit_and_wait` 改成返回 `Err`。把 `WalFsync` 接进 `write_batch`。把缓冲换成 `mpsc` 或把 `Notify` 换成 `oneshot`。在 `lib.rs` 重导出新方法。新增 `eprintln!` / `println!` 或 `diagnostics` 模块。
- First test:
  - 位置与命令：先在 `tests/wal_buffer_test.rs` 增加 `commit_wait_after_shutdown_returns_ok`。构造容量 100、间隔 10 秒的缓冲，`start_flush_loop`，`shutdown` 完成后再 `append` 一条 `BeginTxn`，对 `append_commit_and_wait` 使用 `tokio::time::timeout(Duration::from_secs(2), ...)`。断言超时结果为完成，且 `WalReader` 能读出这条 `BeginTxn`。
  - 改前命令：`cargo test --offline --test wal_buffer_test commit_wait_after_shutdown_returns_ok`
  - 预期 RED：2 秒超时，断言失败。循环已经 join，新的等待不会被唤醒。
  - 然后在 `src/wal/buffer.rs` 的测试模块增加 `quiesce_flushes_then_pauses_timer`：间隔 50 毫秒、容量 100，`start_flush_loop` 之后 `append` 一条记录，再 `quiesce_for_checkpoint`。WAL 中能读出第一条。再 `append` 另一条，等待 1 秒，第二条不在 WAL 中。这条断言完成之后才 `shutdown`，避免把第二条提前刷出，也避免测试进程留下循环。
  - 同模块增加 `quiesce_does_not_take_session_gate`：`start_flush_loop` 后在当前任务持有 `session_gate()`，对 `quiesce_for_checkpoint` 做 2 秒 `timeout`。超时失败。
  - 改前命令：`cargo test --offline --lib quiesce_flushes_then_pauses_timer`
  - 预期 RED：方法不存在，编译失败。`quiesce_does_not_take_session_gate` 同样因方法不存在而编译失败。
- GREEN condition: 下面 Verification 的两条命令退出码都是 0。`commit_wait_after_shutdown_returns_ok` 在 2 秒内通过。`quiesce_flushes_then_pauses_timer` 通过。既有四个集成用例仍通过。
- Verification:
  - `cargo test --offline --test wal_buffer_test`，默认并行度。退出码 0。失败表示公开的追加、容量刷盘、组提交、`shutdown` 刷盘或停止后的等待回归。
  - `cargo test --offline --lib wal::buffer`，默认并行度。退出码 0。失败表示暂停、恢复或停止后的定时器行为不对。
  - 同文件还要有这些用例，并随上面两条命令一起通过。每个会启动循环的用例都先 `start_flush_loop`。
    - `resume_lets_timer_flush`：`quiesce_for_checkpoint` 之后 `resume_after_checkpoint`，间隔 50 毫秒，1 秒内记录出现。若 1 秒内没出现，只把该用例再跑一次，仍没有则失败。断言之后 `shutdown`。
    - `shutdown_after_quiesce_flushes_paused_records`：暂停后 `append` 一条未达容量的记录，对 `shutdown` 做 2 秒 `timeout`。必须完成，且该记录在 WAL 中。
    - `resume_after_shutdown_does_not_flush_by_timer`：`shutdown` 之后调用 `resume_after_checkpoint`，再 `append` 一条未达容量的记录并等待 1 秒，记录不在 WAL 中。
    - `second_shutdown_returns`：连续两次 `shutdown` 都返回。
    - `quiesce_does_not_take_session_gate`：见 First test。2 秒内完成。
  - 暂停用例里记录被定时器写走就是失败，不用重跑把它判成通过。
  - 重叠 `do_flush`：两个任务同时 `do_flush`，两侧都返回，`WalReader` 能读完，条数与刷盘前一致。改前可能已经通过。它是保持用例，不是本任务的 RED。
  - 写盘失败仍返回 `Ok(())`：不新造注入。实现后 `do_flush` 里 `write_batch` 的 `Err` 分支仍唤醒已取走的等待者，`append_commit_and_wait` 的末尾仍是 `Ok(())`。Act Response 写明这两处还在。
- Stop when: 要让等待返回 `Err` 才能通过测试；要改 `Database::close` 或 `WalWriter` 才能停循环；刷盘闸门必须跨 `await` 持有才会死锁；发现产品代码里第二次 `start_flush_loop` 会漏掉前一个任务需要在本轮改调用方。这些返回 Plan。

**Invariants**

- 成功的 `append_commit_and_wait` 表示这次取走的批次已经从 `write_batch` 返回，函数值是 `Ok(())`。T2 才会在这之后标记版本；本 Cycle 不改 `commit`。
- 已取走的等待者一定被唤醒，包括写盘失败和缓冲已空。
- `shutdown` 之后进程内不再有这条缓冲的 `flush_loop`。
- 新方法不获取会话闸门，避免 T2 在外部持有它时死锁。

**Non-goals**

- 不把 `checkpoint` 或 `close` 接到 `quiesce_for_checkpoint`。不改取消路径。不改缓冲池。不跑全量。

**Acceptance**

- `commit_wait_after_shutdown_returns_ok` 在改后 2 秒内通过，WAL 含 `shutdown` 之后追加的那条记录，且文件可解析。对应 spec 的停止后等待场景。
- `quiesce_flushes_then_pauses_timer` 与 `resume_lets_timer_flush` 通过。对应预刷和暂停/恢复。
- `test_wal_buffer_shutdown_flushes` 与另外三个既有用例通过。对应 `shutdown` 刷盘和组提交。
- `second_shutdown_returns`、`shutdown_after_quiesce_flushes_paused_records`、`resume_after_shutdown_does_not_flush_by_timer` 通过。
- `do_flush` 的写盘失败分支仍唤醒等待者，等待函数仍返回 `Ok(())`。由 Act Response 指到代码，Plan Review 读 diff。
- 本 Cycle 的 diff 不包含 Forbidden 列出的文件。

**Requirements Traceability Matrix**

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 刷盘执行者与提交等待 | T1 | `quiesce_flushes_then_pauses_timer` | Covered |
| shutdown 与写盘失败的完成方式 | T1 | `commit_wait_after_shutdown_returns_ok` | Covered |

本 Cycle 不覆盖 checkpoint/关闭、取消与缓冲池、全量。它们属于 T2、T3。

**Verification**

- 停止后等待：`cargo test --offline --test wal_buffer_test commit_wait_after_shutdown_returns_ok`。2 秒内退出码 0，输出里该用例 passed。超时即失败。
- 暂停与恢复：`cargo test --offline --lib wal::buffer`。退出码 0。`quiesce_flushes_then_pauses_timer` 用 WAL 里有没有第二条记录判定，不用日志。
- 既有缓冲行为：`cargo test --offline --test wal_buffer_test`。退出码 0。
- 写盘失败分支：读 `do_flush` 与 `append_commit_and_wait` 的 diff，不跑新的失败注入。
- 不在本 Cycle 跑不带目标过滤的 `cargo test`，也不跑 `--test-threads=1` 的全量批。

**Gate 2 Readiness**

PASS。审查计划 11 项通过，RTM 无 Missing，无未批准的裁剪。无 BLOCKED。无 SKIPPED。计划批准来自本次 goal 指令，不是跳过。

**Persisted Evidence**

- Mode: none

定向 `cargo test` 可以重跑。命令、退出码和失败分支的代码位置写入 Act Response 即可。

**Risks and Notes**

- 重叠 `do_flush` 不一定在改前失败。单飞以刷盘闸门为准，并发用例只要求改后可解析。
- 写盘失败没有现成注入点。不扩展 `WalFsync`。
- `resume_lets_timer_flush` 依赖定时器。通过条件里允许该用例失败后只复跑一次。
- 会话闸门本 Cycle 不接入 `commit`。T2 才能验证它和 `close` 的互斥。
- `get_current_lsn()` 的 `unwrap_or(0)` 保持原样。

## Act Response

- Status: reported

**实际改动**

- `src/wal/buffer.rs` 的 `WALBuffer` 增加 `paused`、`flush_gate`、`session_gate`、`state_notify`。`do_flush` 取得 `flush_gate` 后调用 `flush_locked`。`flush_locked` 取走缓冲和 `pending_commits`，调用 `write_batch`，再唤醒这次取走的等待者。`append` 在容量达到时走 `flush_after_pause`。`append_commit_and_wait` 在闸门内登记并入队；已经停止则当场 `flush_locked`，否则释放闸门后 `notify` 循环。两种路径都在释放闸门之后 `await`。`shutdown` 持有闸门时把停止标志设为真并 `flush_locked`，释放后再唤醒循环并 join。`start_flush_loop` 看到停止标志就返回，不 `spawn`。
- 新增 `pub(crate)` 方法：`quiesce_for_checkpoint`、`resume_after_checkpoint`、`session_gate`。未从 `lib.rs` 重导出。
- `tests/wal_buffer_test.rs` 增加 `commit_wait_after_shutdown_returns_ok`。
- `src/wal/buffer.rs` 的测试模块增加 `quiesce_flushes_then_pauses_timer`、`quiesce_does_not_take_session_gate`、`resume_lets_timer_flush`、`shutdown_after_quiesce_flushes_paused_records`、`resume_after_shutdown_does_not_flush_by_timer`、`second_shutdown_returns`、`overlapping_do_flush_preserves_record_count`。

**与计划的偏差**

- `tokio::sync::Mutex` 不可重入。`shutdown`、`quiesce_for_checkpoint` 和已停止的 `append_commit_and_wait` 在已经持有 `flush_gate` 时调用 `flush_locked`，不再次进入 `do_flush`。取走缓冲、`write_batch` 和唤醒仍在同一次持锁内。
- 容量刷盘走 `flush_after_pause`：先在闸门外等待恢复或停止，拿到闸门后如果仍是暂停且未停止，就放开闸门继续等。暂停若插在等待和写盘之间，这一批不会被写出去。
- 暂停、恢复和停止使用 `state_notify.notify_waiters`。`flush_notify` 仍只通知刷盘循环。两处等待若共用一把 `Notify`，`notify_one` 只能唤醒其中一个。
- `session_gate` 字段，以及 `session_gate`、`quiesce_for_checkpoint`、`resume_after_checkpoint`，加了 `#[allow(dead_code)]`。提交和 checkpoint 还没有调用它们。同 crate 的单元测试会调用。不加的话，集成测试编译库时出现 dead_code。

**自检**

- 发现：第一次 `cargo test --offline --test wal_buffer_test` 编译库时有 2 条 dead_code 警告。一条指向 `session_gate` 字段，另一条指向 `session_gate`、`quiesce_for_checkpoint`、`resume_after_checkpoint`。该次测试结果是 5 passed、0 failed，退出码 0。
- 已修复：加上 `#[allow(dead_code)]` 后重跑同一条命令。编译输出里没有这 2 条警告。
- 遗留 Minor：无。
- Gate 3：通过。单飞写盘、预刷后暂停、恢复、停止后 2 秒内返回、第二次 `shutdown`、调用方持有会话闸门时 `quiesce_for_checkpoint` 仍返回，都有对应用例。`flush_locked` 在 `write_batch` 返回 `Err` 时分支为空（`src/wal/buffer.rs` 287–295 行），唤醒在该分支之外（298–305 行），缓冲为空但 `committed_tx_ids` 非空时同样唤醒。`append_commit_and_wait` 的末尾仍是 `Ok(())`（128 行）。`get_current_lsn()` 的 `unwrap_or(0)` 仍在（277 行）。`notified().await` 和暂停等待都不持有 `flush_gate`。上述方法不获取 `session_gate`。diff 只有 `src/wal/buffer.rs` 和 `tests/wal_buffer_test.rs`，以及本轮的执行记录。
- Gate 4：通过。下面两条 Verification 命令的退出码都是 0，失败计数都是 0。
- Gate 5：未触发。
- 跳过：不带目标过滤的 `cargo test`。Verification 写明本 Cycle 不跑；全量属于后续 Iteration 002。

**验证**

- 改前保持面：`cargo test --offline --test wal_buffer_test -- --test-threads=8`。退出码 0。`4 passed; 0 failed`。命令比计划多了 `--test-threads=8`，四个既有用例都执行了。
- RED：`cargo test --offline --test wal_buffer_test commit_wait_after_shutdown_returns_ok`。退出码 101。`0 passed; 1 failed; 4 filtered out`，`finished in 2.00s`。失败断言是停止后的等待没有在 2 秒内返回。
- RED：`cargo test --offline --lib quiesce_flushes_then_pauses_timer`。退出码 101。编译失败，3 个 E0599：`quiesce_for_checkpoint` 与 `session_gate` 不存在。
- `cargo test --offline --lib wal::buffer`。退出码 0。`7 passed; 0 failed; 436 filtered out`，`finished in 1.06s`。`resume_lets_timer_flush` 第一次就通过，没有按计划复跑。这条命令在 `#[allow(dead_code)]` 和 rustfmt 之前。这两处不改变断言，也不改变刷盘路径。
- `cargo test --offline --test wal_buffer_test`（dead_code 修复之后）。退出码 0。`5 passed; 0 failed; 0 filtered out`，`finished in 0.28s`。`commit_wait_after_shutdown_returns_ok` 为 ok。编译输出没有 dead_code。

**Persisted Evidence**

None required。

**Experience Candidates**

None。

**未解决问题**

无。

## Plan Review

- Review Result: accepted

**Findings**

- 非阻塞。`shutdown`、`quiesce_for_checkpoint` 和已停止的 `append_commit_and_wait` 在已经持有 `flush_gate` 时调用 `flush_locked`，没有再次进入 `do_flush`。`tokio::sync::Mutex` 不可重入。同一次持锁内仍然完成取走缓冲、`write_batch` 和唤醒。调用形状和执行依据里的字面 `do_flush` 不同，闸门范围与 Acceptance 一致。分类：Act 偏离，不阻塞。
- 非阻塞。`session_gate`、`quiesce_for_checkpoint`、`resume_after_checkpoint` 带有 `#[allow(dead_code)]`。本轮没有提交或 checkpoint 调用这些符号，集成测试编译库时需要这条允许。下一轮接入之后可以去掉。不阻塞本轮验收。

**Evidence**

- diff 范围：`git diff 60e107965a4f728b0dcbe157bf39c59f8c9dbb0f..d065a023ad31e412055ef37109a9a0c7e399dbe4` 的产品代码只有 `src/wal/buffer.rs` 和 `tests/wal_buffer_test.rs`。`src/database.rs`、`src/transaction/manager.rs`、`src/wal/checkpoint.rs`、`src/wal/writer.rs`、`src/storage/buffer_pool.rs`、`src/lib.rs` 不在其中。
- `flush_locked`（`src/wal/buffer.rs`）在 `write_batch` 返回 `Err` 时分支为空（287–295 行），唤醒在该分支之外（298–305 行）。缓冲为空但 `committed_tx_ids` 非空时同样唤醒。`append_commit_and_wait` 末尾是 `Ok(())`（128 行）。`get_current_lsn()` 的 `unwrap_or(0)` 仍在（277 行）。
- `notified().await`（127 行）发生在 `flush_gate` 释放之后。`wait_until_flush_allowed` 不取 `flush_gate`。`session_gate()` 只返回共享引用。`quiesce_for_checkpoint`、`resume_after_checkpoint`、`shutdown`、`do_flush`、`append`、`append_commit_and_wait` 不锁这把闸门。
- `quiesce_for_checkpoint` 与 `resume_after_checkpoint` 在停止标志已为真时直接返回，不调用 `start_flush_loop`。`start_flush_loop` 看到停止标志也不 `spawn`。
- `wait_until_flush_allowed` 先构造 `state_notify.notified()`，再读暂停和停止标志。tokio 1.52.3 的 `Notify::notified` 在创建时记下 `notify_waiters` 的调用次数；创建之后的 `notify_waiters` 会在 poll 时被看到（`tokio-1.52.3/src/sync/notify.rs` 中 `notified` 的说明，以及 `poll_notified` 对调用次数的比较）。暂停中的等待因此能在恢复或停止时返回。
- 验证沿用本文件 Act Response 的「验证」一节。`cargo test --offline --lib wal::buffer` 退出码 0，`7 passed; 0 failed`。`cargo test --offline --test wal_buffer_test` 在 `#[allow(dead_code)]` 之后退出码 0，`5 passed; 0 failed`，其中 `commit_wait_after_shutdown_returns_ok` 为 ok。这两条就是本 Cycle 的 GREEN 命令。属性与其后的格式调整不改变刷盘路径，不重跑。改前 RED，以及四个既有用例在改前的通过，也沿用该节。`resume_lets_timer_flush` 第一次通过，没有复跑。
- `tests/wal_buffer_test.rs` 的 `create_wal_buffer` 会 `start_flush_loop`。`commit_wait_after_shutdown_returns_ok` 因此是在循环已经启动并完成 `shutdown` 之后再等待。
- 本轮用例还包括 `quiesce_flushes_then_pauses_timer`、`resume_lets_timer_flush`、`shutdown_after_quiesce_flushes_paused_records`、`resume_after_shutdown_does_not_flush_by_timer`、`second_shutdown_returns`、`quiesce_does_not_take_session_gate`、`overlapping_do_flush_preserves_record_count`，以及既有的追加、容量刷盘、组提交和 `shutdown` 刷盘四个用例。组提交用例让两个 `append_commit_and_wait` 并发返回，并用 `WalReader::read_all` 读出记录。

**Follow-up Decision**

本轮 Acceptance 已经满足，没有要留在当前 Cycle 的修复，也没有需要新执行依据的缺口。Iteration Plan 保持不变。展开下一轮的目录和 `000-initial.md`。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-10-28-ms43-service-lifecycle/iterations/001-session-checkpoint-close/000-initial.md`
