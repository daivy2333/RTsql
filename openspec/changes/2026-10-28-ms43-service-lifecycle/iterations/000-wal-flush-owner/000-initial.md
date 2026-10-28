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

- Status: 待 Act 填写

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
