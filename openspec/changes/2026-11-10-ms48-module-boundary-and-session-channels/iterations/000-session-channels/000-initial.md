# Iteration 000 / Cycle 000: 两条会话队列改为通道

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 需求 1 至 4；`session-service-lifecycle` 与 `unified-diagnostics` 的现有结果；M19 对定向测试的 `--test-threads=1`；M21 的六层归属
- Excluded scope: T2 全量；拆 crate；替换 `Notify`；检查点或缓冲池的新通道；I093、I095、诊断上报、SQL 语义与性能

**Objective**

`WALBuffer` 的记录队列和待提交事务号队列改为无界 `mpsc`。容量、关闭、停止后的提交等待、checkpoint 与 `close` 的现有结果保持。包仍然只有一个。

**Investigation Facts**

- Starting point: 产品代码与 `56da53b` 相同。本计划的文档提交不改产品代码。改前已实际运行：`cargo test --offline --lib wal::buffer::tests -- --test-threads=1` 为 7 passed、0 failed；`cargo test --offline --test wal_buffer_test -- --test-threads=1` 为 5 passed、0 failed；`cargo test --offline --test diagnostics_wait_test commit_wait_classes_present -- --test-threads=1` 为 1 passed、0 failed；`cargo test --offline --test session_lifecycle_test -- --test-threads=1` 为 7 passed、0 failed；`cargo test --offline --test statement_cancellation_test cancel_leaves_flush_loop_running -- --test-threads=1` 为 1 passed、0 failed。这些结论覆盖的代码在本 Cycle 实施前没有变化，实施前不必再跑一遍。
- Current-State Evidence:
  - `src/wal/buffer.rs` 的 `WALBuffer::buffer` 是 `Mutex<Vec<(u64, WalRecord)>>`，`pending_commits` 是 `Mutex<Vec<u64>>`。`append` 在缓冲锁内 `push`，长度达到 `capacity` 后调用 `flush_after_pause`。`append_commit_and_wait` 先拿 `flush_gate`，登记 `CommitWaiter`（`Arc<Notify>` 加诊断字段），再 `push` 事务号；已 `shutdown` 时在同一把闸门里调用 `flush_locked`。等待是 `notified().await`，然后返回 `Ok(())`。
  - `flush_locked` 在调用方已持有 `flush_gate` 时取走两段 `Vec`，有记录才 `write_batch`。`write_batch` 返回 `Err` 时分支为空，仍然 `notify_one`。缓冲为空但取到了事务号时也唤醒。
  - `flush_loop` 用 `select` 等 `flush_notify`、`state_notify` 和定时器，然后拿 `flush_gate` 并调用 `flush_locked`。`do_flush`、`quiesce_for_checkpoint`、`shutdown`、`flush_after_pause` 也走 `flush_locked`。
  - `src/transaction/manager.rs` 的 `commit` 在 `append(CommitTxn)` 和 `append_commit_and_wait` 之间持有 `session_gate`。`src/database.rs` 的 `checkpoint_under_session_gate` 持有同一把闸门并调用 `quiesce_for_checkpoint`。`checkpoint` 之后 `resume_after_checkpoint`。`close` 在同一段前序之后 `shutdown`，不恢复循环。
  - `src/wal/checkpoint.rs` 没有后台队列。`src/storage/buffer_pool.rs` 没有生产者-消费者队列。`Cargo.toml` 只有一个 `[package]`，feature 为 `cli`、`network`、`crypto`。`tokio` 1.52.3 的 `UnboundedReceiver::len` 可用。
  - 执行器里 13 个文件持有 `Database`，子查询执行器调用 `pipeline::create_executor_from_plan`，`transaction/session.rs` 引用 `Database`，存储使用 `Value` 和 `VersionHeader`。这些环使六层不能各自构建。cli、network、crypto 的裁剪已经由 feature 完成。`AsyncStorage` 已在包内提供文件存储和内存存储。没有第二套实现等待单独的 crate。
- Code and Critical Path: 生产者是 `append` 和 `append_commit_and_wait`。消费者是已持有 `flush_gate` 的 `flush_locked`。数据从队列进入 `WalWriter::write_batch`，完成信号从 `CommitWaiter.notify` 回到 `append_commit_and_wait`。`commit`、`checkpoint`、`close` 的调用顺序不改。

**Implementation Guidance**

字段使用 `records_tx`、`records_rx`、`pending_tx`、`pending_rx`。接收端的互斥用 `std::sync::Mutex`，只包住 `try_recv` 或 `len`，不跨越 `.await`。`send` 失败时 panic。后台循环不调用 `recv`。两条通道保持分开。容量判断用发送之后的 `records_rx.len()`，不用有界 `send().await`。

**Task Definitions**

### T1: 两条队列改为无界通道

- Requirement/Scenario: proposal 需求 1 至 4，场景 S1 至 S9、S11、S12。S10 属于 T2，本任务不执行。
- Depends on: None
- Targets: `src/wal/buffer.rs` 的 `WALBuffer`、`append`、`append_commit_and_wait`、`flush_locked`；同文件 `mod tests` 的 `append_enqueues_on_the_record_channel` 与 `commit_id_is_queued_on_the_channel`
- Current behavior: 记录和待提交事务号放在两段 `Mutex<Vec<_>>` 里。`flush_locked` 用 `mem::take` 取走。提交完成是 `Notify`。`append` 与 `append_commit_and_wait` 的签名是 `u64` 和 `Result<(), WalError>`。改前的定向测试已通过，见 Investigation Facts。
- Required behavior:
  - `records_tx` 发送 `(u64, WalRecord)`，`pending_tx` 发送 `u64`。两者都是 `tokio::sync::mpsc` 无界通道。接收端放在 `std::sync::Mutex` 中。
  - `append` 先用现有的 `fetch_add` 分配 LSN，再 `send`。然后在接收端锁内读取 `len()`。长度大于等于 `capacity` 时，放开锁，调用现有 `flush_after_pause`。`send` 的 `Err` 直接 panic。
  - `append_commit_and_wait` 仍先取得 `flush_gate`，仍先登记 `CommitWaiter`，再 `pending_tx.send`。已 `shutdown` 时仍在这把闸门内调用 `flush_locked`。未停止时仍 `flush_notify.notify_one()`，然后 `notified().await`。返回值仍是 `Ok(())`。Lock、Queue、Io、Schedule 的记时点保持在现在的语句上。
  - `flush_locked` 仍要求调用方已持有 `flush_gate`。它先取空记录通道，再取空待提交通道，放开接收端锁，然后沿用现在的文件偏移、`write_batch`、诊断回写和 `notify_one`。`write_batch` 返回 `Err` 时仍然唤醒，不把 `append_commit_and_wait` 改成 `Err`。没有记录但取到事务号时也唤醒。
  - `flush_loop` 仍 `select` 三个等待，然后拿闸门并调用 `flush_locked`。它不从通道 `recv`。
  - `Cargo.toml` 保持一个 `[package]`，没有 `[workspace]`，feature 文本不变。
- Preserve: `append` 与 `append_commit_and_wait` 的签名；`session_gate` 的持有者；`quiesce_for_checkpoint`、`resume_after_checkpoint`、`shutdown` 的对外结果；`flush_gate` 保证同时只有一个 `flush_locked` 写盘；诊断四类的记时位置；磁盘上的 WAL 记录格式。
- Forbidden: 修改 `Cargo.toml`、`src/database.rs`、`src/transaction/manager.rs`、`src/wal/checkpoint.rs`、`src/wal/writer.rs`、`src/storage/buffer_pool.rs`。新增 crate 或 workspace 成员。把 `Notify` 换成 `oneshot`。使用有界 `send().await` 表达容量。把记录和事务号合成一条通道消息。让后台循环阻塞在 `recv()`。给缓冲池或检查点加通道、`watch` 或任务。新增 `WalError` 变体。新增诊断类别。新增注入器来制造 `write_batch` 失败。跑全量，或使用不带 `--test-threads=1` 的测试命令。
- First test: 在 `src/wal/buffer.rs` 的 `mod tests` 增加下面两个测试，先不改队列实现。`append_enqueues_on_the_record_channel`：用现有 `open_buffer(100, 10_000)`，不启动刷盘循环；`append` 一条 `BeginTxn { tx_id: 7 }`；从 `records_rx` `try_recv` 得到这条记录后立刻放开接收端锁；此时 `read_records` 不包含 tx 7；然后 `shutdown`。`commit_id_is_queued_on_the_channel`：同样不启动循环；`spawn` `append_commit_and_wait(7)`；在 2 秒内等到 `pending_rx.len() == 1`，且该 `JoinHandle` 尚未结束；读长度时放开接收端锁；不要 `try_recv` 掉这个事务号；再 `do_flush`；`JoinHandle` 在 2 秒内结束且内层结果是 `Ok(())`。然后执行 `cargo test --offline --lib append_enqueues_on_the_record_channel -- --test-threads=1`。预期：编译失败，错误指向尚不存在的 `records_rx` 或 `pending_rx`。这是 RED。看到这个编译失败之前不改队列实现。
- GREEN condition: 队列按 Required behavior 改完后，下面五条命令按顺序各执行一次，都退出码 0，且各自的 failed 为 0。
  1. `cargo test --offline --lib wal::buffer::tests -- --test-threads=1`
  2. `cargo test --offline --test wal_buffer_test -- --test-threads=1`
  3. `cargo test --offline --test session_lifecycle_test -- --test-threads=1`
  4. `cargo test --offline --test diagnostics_wait_test commit_wait_classes_present -- --test-threads=1`
  5. `cargo test --offline --test statement_cancellation_test cancel_leaves_flush_loop_running -- --test-threads=1`
- Verification: 五条 GREEN 命令覆盖 S1 至 S9 和 S12 的可运行部分。S11 不新造注入器；`flush_locked` 里 `write_batch` 的 `Err` 分支必须仍然执行到 `notify_one`，由本任务的 diff 核对。另两条观察：`git diff -- Cargo.toml` 无输出；`git diff -- src/database.rs src/transaction/manager.rs src/wal/checkpoint.rs src/wal/writer.rs src/storage/buffer_pool.rs` 无输出。任一条 GREEN 命令失败，或这两条 diff 出现输出，本任务未完成。
- Stop when: 为了通过测试必须把 `append_commit_and_wait` 改成返回 `Err`，必须改 Forbidden 列出的文件，必须把两条通道合成一条，必须让发送方堵在有界 `send` 上，或必须新增 crate。出现这些情况时停止并回到计划，不继续改。

**Invariants**

- 同时只有一个 `flush_locked` 执行 `write_batch`。
- `commit` 在 `CommitTxn` 与等待返回之间持有会话闸门。`checkpoint` 与 `close` 在各自的前序里持有同一把闸门。
- `close` 结束后循环不再被定时器唤醒。`checkpoint` 返回后循环仍会按间隔写未达容量的记录。
- 已提交行在 `close` 后重开仍在。显式事务未提交就 `close` 时，默认重开仍能查到该行。
- 一次成功插入的提交子阶段仍有 Lock、Queue、Io，Schedule 不超过该阶段的等待合计。
- 库保持一个 package。

**Non-goals**

本 Cycle 不跑全量，不拆包，不替换 `Notify`，不给检查点或缓冲池增加通道。

**Acceptance**

- S1：`append_enqueues_on_the_record_channel` 通过，记录在刷盘前可从通道取出且不在 WAL 文件中。
- S2：`commit_id_is_queued_on_the_channel` 通过。
- S3 至 S7：`wal_buffer_test` 与 `wal::buffer::tests` 全部通过。其中容量、关闭、停止后等待、暂停恢复和重叠 `do_flush` 保持改前结果。
- S8：`session_lifecycle_test` 全部通过。
- S9：`commit_wait_classes_present` 通过。
- S11：`flush_locked` 的 `write_batch` 错误分支仍唤醒等待者，`append_commit_and_wait` 仍返回 `Ok(())`。
- S12：`cancel_leaves_flush_loop_running` 通过。缓冲池文件的 diff 为空。
- 需求 1：`Cargo.toml` 的 diff 为空，文件中仍是一个 `[package]`，没有 `[workspace]`。

**Verification**

按 T1 的 GREEN 顺序执行五条命令。每条都带 `--test-threads=1`。再看两条 `git diff`。S10 不在本 Cycle。不新增判定脚本。

**Gate 2 Readiness**

Gate 2：PASS。审查项全部通过，无 BLOCKED，无 SKIPPED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 发送和读取 `len()` 之间若另一次 `flush_locked` 取走了记录，本次 `append` 看到的长度可以小于容量，因而不再自己刷。写盘责任在已经取走的那次 `flush_locked`。
- 并发追加的持久化顺序是通道接受顺序。现有测试断言条数和指定事务号，不断言跨任务的锁取得顺序。
- 改前的四组定向测试结果来自本计划的实际运行。产品代码未改时，实施前不必重跑。新测试的编译失败必须在改队列之前看到。

## Act Response

- Status: reported

**实际改动**

- `src/wal/buffer.rs`
  - `WALBuffer` 的 `buffer`、`pending_commits` 换成 `records_tx`、`records_rx`、`pending_tx`、`pending_rx`。两条都是 `tokio::sync::mpsc` 无界通道，接收端放在 `std::sync::Mutex` 中。
  - `append`：先 `fetch_add` 分配 LSN，再 `records_tx.send`，`Err` 时 panic。然后在接收端锁内读 `len()`；长度大于等于 `capacity` 时放开锁，调用 `flush_after_pause`。
  - `append_commit_and_wait`：仍先取得 `flush_gate`，仍先登记 `CommitWaiter`，再 `pending_tx.send`（失败 panic）。已 `shutdown` 时仍在这把闸门内调用 `flush_locked`。未停止时仍 `flush_notify.notify_one()`，然后 `notified().await`，返回 `Ok(())`。Lock、Queue、Io、Schedule 的记时语句留在原处。
  - `flush_locked`：先取空记录通道，再取空待提交通道，放开接收端锁，然后沿用文件偏移、`write_batch`、诊断回写和 `notify_one`。`write_batch` 返回 `Err` 时分支仍为空，并执行到 `notify_one`。没有记录但取到事务号时也唤醒。
  - `flush_loop` 未改。它仍 `select` 三个等待，然后拿闸门并调用 `flush_locked`，不从通道 `recv`。
  - `mod tests` 新增 `append_enqueues_on_the_record_channel` 与 `commit_id_is_queued_on_the_channel`。
- `openspec/changes/2026-11-10-ms48-module-boundary-and-session-channels/tasks.md`：T1 标为完成。Iteration 000 仍待 Review。

**与计划的偏差**

无实质偏差。`append_enqueues_on_the_record_channel` 用 `&queued` 匹配取出的记录，失败信息仍能打印该值。判定仍是 `BeginTxn { tx_id: 7 }`，接收端锁在 `try_recv` 后放开。

**自检**

Gate 3 未发现 Critical 或 Important。diff 没有计划外的产品修改。接收端锁不跨越 `.await`。这次编译没有新增本文件警告。遗留 Minor：无。

**验证**

RED：`cargo test --offline --lib append_enqueues_on_the_record_channel -- --test-threads=1`，退出码 101。队列实现尚未修改。编译错误：

```
error[E0609]: no field `records_rx` on type `std::sync::Arc<WALBuffer>`
   --> src/wal/buffer.rs:588:33
error[E0609]: no field `pending_rx` on type `std::sync::Arc<WALBuffer>`
   --> src/wal/buffer.rs:616:37
error: could not compile `rtsql` (lib test) due to 2 previous errors
```

GREEN 按顺序各一次。退出码都是 0，failed 都是 0。

1. `cargo test --offline --lib wal::buffer::tests -- --test-threads=1`：退出码 0。`9 passed; 0 failed`。
2. `cargo test --offline --test wal_buffer_test -- --test-threads=1`：退出码 0。`5 passed; 0 failed`。
3. `cargo test --offline --test session_lifecycle_test -- --test-threads=1`：退出码 0。`7 passed; 0 failed`。
4. `cargo test --offline --test diagnostics_wait_test commit_wait_classes_present -- --test-threads=1`：退出码 0。`1 passed; 0 failed`。
5. `cargo test --offline --test statement_cancellation_test cancel_leaves_flush_loop_running -- --test-threads=1`：退出码 0。`1 passed; 0 failed`。

`git diff -- Cargo.toml`：无输出。
`git diff -- src/database.rs src/transaction/manager.rs src/wal/checkpoint.rs src/wal/writer.rs src/storage/buffer_pool.rs`：无输出。

S11 由 diff 核对：`flush_locked` 里 `write_batch` 的 `Err` 分支仍为空，随后 `notify_one`。`append_commit_and_wait` 仍返回 `Ok(())`。未新增注入器。

Gate 3：PASS。Gate 4：PASS。Gate 5：未命中。

**Persisted Evidence**

None required

**Experience Candidates**

None

**未解决问题**

无。T2 全量不在本 Cycle。

## Plan Review

- Review Result: pending

**Findings**

尚未 Review。

**Evidence**

尚未 Review。

**Follow-up Decision**

尚未 Review。

**Next Cycle**

None

**Next Iteration**

None
