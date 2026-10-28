# 会话级后台服务的状态归属与退出

对应路线：MS43。Gate 1 为 PASS，批准来自本次 goal 指令对 MS43 范围的授权，不是跳过。本文件不更新全局里程碑状态。

## 当前批次状态

- 需求与 BDD：completed（场景草图与默认假设见 proposal.md）。
- 000：当前轮。执行依据是 `iterations/000-wal-flush-owner/000-initial.md`。Plan Context 为 ready。Act Response 为 reported。Review Result 仍为 pending。
- 001：未展开。
- 002：未展开。
- Persisted Evidence：none。
- 无跳过项。

本 goal 覆盖 M20 的「不在当前 change 内扩范围修复」，仅限本 change 实施中新发现的实质缺陷。登记后由后续派发在当前 change 增加一轮 Iteration。本计划不预建那一轮。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | `WALBuffer` 单飞刷盘、暂停/恢复、`shutdown` 后等待返回 `Ok` | 已实施 |
| T2 | 001 | 000 accepted | `commit` / `checkpoint` / `close` 接到上述方法；取消与缓冲池行为保持 | 未开始 |
| T3 | 002 | 001 accepted | M19 分批全量一次，每批 failed 为 0 | 未开始 |

## 1. WAL 刷盘所有者（Iteration 000）

- [x] 1.1 T1：按 `iterations/000-wal-flush-owner/000-initial.md` 的 Task Definition 执行。先写失败测试并看到 RED，再改 `src/wal/buffer.rs`。通过条件是该文件列出的 GREEN 命令。不改 `database.rs`、`transaction/manager.rs`、`checkpoint.rs`、`writer.rs`、`buffer_pool.rs`。

## 2. 会话 checkpoint 与关闭（Iteration 001）

- [ ] 2.1 T2：在 000 的方法之上，让 `TransactionManager::commit` 在 `CommitTxn` 与 `append_commit_and_wait` 之间持有会话闸门；`Database::checkpoint` 在九步前 `quiesce_for_checkpoint`、九步后 `resume_after_checkpoint`；`Database::close` 走同一段九步前序列，然后 `shutdown`，不恢复循环。只读和内存会话保持早退。

先行测试与通过条件（本轮展开时写入该轮 Cycle，此处为任务合同）：

- RED：`close` 返回后，经 `wal_buffer` `append` 一条未达容量的记录，等待 1 秒，记录出现在 WAL 中。期望改成「不出现」。今天循环仍在，所以这条断言现在失败。
- 改前 GREEN，改后仍 GREEN：`checkpoint` 返回且未 `close` 时，同样的 `append` 加 1 秒等待，记录出现。`cargo test --offline --test checkpoint_test --test checkpoint_skip_test --test checkpoint_redo_reduction_test --test statement_cancellation_test --test readonly_session_test --test memory_db_test` 通过。
- GREEN：已提交一行，`close` 后重开仍在。未提交的 `begin` 写入，`close` 后重开不可见。取消语句之后 `begin` 的 `BeginTxn` 在 1 秒内被原循环刷出，随后另一条提交成功。`shutdown` 两次都返回。`commit` 与 `close` 在 2 秒内返回，不因闸门嵌套挂起。
- 禁止：改 `append_commit_and_wait` 的失败 Result；改九步；给 `BufferPool` 加任务或 `watch`；新增诊断输出；`close` 调用会恢复循环的公开 `checkpoint()`。

## 3. 收尾全量（Iteration 002）

- [ ] 3.1 T3：行为任务完成后，按下面的分批方式跑一次全量。这是整个 change 的唯一一次全量。

分批是通过条件的一部分：

1. 执行时枚举，不把规划时的文件数写死。集成测试文件来自 `ls tests/*.rs | sort`。
2. 第一批单独执行：`cargo test --offline --lib --bins -- --test-threads=1`。
3. 其余文件按该排序每 12 个一批，最后一批可以少于 12。文件名去掉 `.rs` 得到 `--test` 的名字。每批一条命令：`cargo test --offline --test <名> --test <名> ... -- --test-threads=1`。
4. 批间一条命令结束后再开始下一条。禁止一次不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`。禁止全量批次使用默认并行度。
5. 对账：全部 `--test` 名字的集合等于枚举集合，无缺失、无多余、无重复；`--lib` 和 `--bins` 只出现在第一批。
6. 每一批把退出码和 failed 计数写入 Act Response。退出码 0 且 failed 为 0 才算该批通过。任一批未跑，不得声明全量通过。
7. 某一批失败时，只把该批再跑一次。第二次仍失败则本任务失败，不改并行度，不为了凑通过而重跑其余已通过的批。

## Iteration Plan

### Iteration 000：WAL 刷盘所有者

- Tasks：T1
- Depends on：None
- Stable result：`WALBuffer` 的刷盘单飞、暂停/恢复和停止后的提交等待可单独用缓冲测试验证。`Database` 尚未改关闭顺序。
- Verification boundary：`cargo test --offline --lib wal::buffer` 与 `cargo test --offline --test wal_buffer_test`，默认并行度。不跑全量。全量预算在 002，整个 change 一次。
- Diagnostic boundary：`src/wal/buffer.rs` 的闸门、暂停和 `shutdown`。`close` 之后定时器仍在写，不属于本轮失败。
- Non-goals：改 `commit` / `checkpoint` / `close` 的调用、缓冲池、诊断门面、全量回归。

### Iteration 001：会话 checkpoint 与关闭

- Tasks：T2
- Depends on：000 accepted
- Stable result：读写会话的 `checkpoint` 在九步期间暂停循环并在返回后恢复；`close` 在返回后循环已 join。已提交行、取消、只读和内存路径保持。
- Verification boundary：T2 的新测试加上 checkpoint、取消、只读、内存这组定向测试，默认并行度。不跑全量。
- Diagnostic boundary：`database.rs` 的暂停/恢复/`shutdown` 顺序，以及 `manager.rs` 的会话闸门范围。WAL 帧格式和缓冲池脏位不属于本轮改动。
- Non-goals：改九步、改写盘失败的 Result、全量回归、预建缺陷修复轮。

### Iteration 002：收尾全量

- Tasks：T3
- Depends on：001 accepted
- Stable result：默认构建上按 M19 分批串行的一次全量，每批退出码 0 且 failed 为 0，目标对账无缺无重。
- Verification boundary：本节 T3 的分批方式。只此一次。失败批只整批复核一次。
- Diagnostic boundary：按失败测试所在的批和目标文件定位。不把全量失败当成修改分批规则的理由。
- Non-goals：第二次全量、非默认 feature 组合、新平台、诊断门面。

## 平衡审计

- 000 只交付可独立测试的 `WALBuffer` 合同。并进 001 会让「循环没停」和「关闭顺序错了」落在同一轮。
- 001 是同一会话上的调用顺序，checkpoint 与 close 共用预刷和暂停。拆成两个 change 会让关闭测试依赖尚未存在的暂停方法，或者在 000 里提前改 `Database`。缓冲池没有后台任务，不单列一轮。
- 002 只有全量，没有新行为。它不并进 001：全量成本高，且 001 的定向测试已经能判断关闭顺序。
- 三轮都有稳定结果和单独的失败范围。没有再拆 change。后续边界写在 proposal 的「后续 change」。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 刷盘执行者与提交等待 | T1 | `quiesce_flushes_then_pauses_timer` | Covered |
| shutdown 与写盘失败的完成方式 | T1 | `commit_wait_after_shutdown_returns_ok` | Covered |
| checkpoint 与关闭对刷盘循环的责任 | T2 | `close_stops_timer_flush` | Covered |
| 取消与缓冲池边界保持 | T2 | `cancel_leaves_flush_loop_running` | Covered |
| 全量回归只跑一次且分批串行 | T3 | M19 分批命令（见 T3） | Covered |

写盘失败仍返回 `Ok(())` 写在第二条需求里，由 T1 的 Preserve 约束保住，不用新的失败注入当先行测试。重叠刷盘的可解析性是 T1 的保持用例，不作为那条需求的唯一 RED。
