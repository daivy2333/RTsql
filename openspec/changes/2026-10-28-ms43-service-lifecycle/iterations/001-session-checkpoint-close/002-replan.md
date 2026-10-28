# Iteration 001 / Cycle 002: 撤回关闭后不可见并对齐重开查询

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T2 的剩余测试对齐。接线不重做。
- Acceptance gaps: None。父 Cycle 的「未提交行在 `close` 后不可见」已按本次裁定撤回，不在本 Cycle 关闭，也不记成已经满足。
- Repair items: None
- Inherited scope: 父 Cycle 已经落地、且不改变事务结果的接线。`commit` 在 `wal_buffer` 为 `Some` 时，从 `append(CommitTxn)` 到 `append_commit_and_wait` 返回持有会话闸门，释放之后才 `commit_mark_versions`。公开 `checkpoint` 在九步前 `quiesce_for_checkpoint`，返回时 `resume_after_checkpoint`，future 被丢掉时 `ResumeOnDrop` 补一次恢复。`close` 走同一段前序，然后 `shutdown`，不调用公开 `checkpoint()`。只读和内存会话在没有 checkpoint manager 时立即返回。`tests/session_lifecycle_test.rs` 里除 `uncommitted_row_absent_after_reopen` 以外的用例保持原断言。
- Excluded scope: 改产品代码来藏行。在 `close` 里回滚仍活跃的显式事务。改 `CheckpointManager::checkpoint` 的九步、位点格式和错误类型。改恢复规则，包括 `redo_count == 0` 时的索引路径，以及无快照点查。改 `WalWriter`、`BufferPool`、`WALBuffer`。把 `append_commit_and_wait` 改成返回 `Err`。修改 `tests/mvcc_tombstone_visibility_test.rs` 的 `flushed_uncommitted_insert_does_not_resurrect`。把现行可见结果写成覆盖该测试的新持久化承诺。把这条可见性登记成要在本变更里修的缺陷。全量（T3）。

**Objective**

`uncommitted_row_absent_after_reopen` 不再要求该行不存在。它断言显式事务 `begin` 后插入、不 `commit` 就 `close`，按默认 `Database::open` 重开后仍查到 `id = 7` 的那一行。`cargo test --offline --test session_lifecycle_test` 退出码 0，failed 为 0。产品代码保持现状。

**Background**

父 Cycle 是 `iterations/001-session-checkpoint-close/000-initial.md`。Act Response 为 `blocked`，Review Result 为 `replan-required`。`001-replan.md` 的 Plan Context 仍为 `draft`，Gate 2 为 BLOCKED。那份草稿写明不能把失败用例改成期望该行可见，也不能改 proposal 和规格。用户随后批准改计划。原话：「我注意到了问题，我觉得可以接受replan更改计划，继续吧」。

批准范围：接受修订计划；不接受新的事务错误语义，也不接受改变既有事务与持久化合同。上述「重开后该行不可见」从本变更的验收里拿掉。现行结果保持：这样 `close` 再打开，该行仍然可见，与改前只调用公开 `checkpoint()` 的 `close` 相同。规格只记录这个现行结果，并写明它不覆盖「刷盘后丢弃句柄、不调用 `close`」那条已有测试。不改 checkpoint 九步、恢复规则、无快照点查，也不在 `close` 里代替调用方回滚显式事务。父 Cycle 已落地且不改变事务结果的接线保留。不把这条登记成要在本变更里修掉的缺陷。

`001-replan.md` 的 Plan Context 不改为 `ready`。本文件是同一 Iteration 目录的后继 replan。

**Investigation Facts**

- Starting point: 分支 `changes/2026-10-28-ms43-service-lifecycle`，提交 `29c6b0aa0d7e359f00a25ef969ac82f3158116bd`，工作区在本 Cycle 的文档修订之前是干净的。产品代码和 `tests/session_lifecycle_test.rs` 自 `454c87d723df76ac66bb1c6c637b7c70087693c9` 至该提交未改；其间 diff 只有 Cycle 文件和 `tasks.md`。父 Cycle Act Response 的定向结果因此仍有效，本 Cycle 不重跑那次失败。
- Current-State Evidence:
  - `tests/session_lifecycle_test.rs` 的 `uncommitted_row_absent_after_reopen`：`Database::open`，建表 `t (id INT PRIMARY KEY)`，`begin`，`execute_in_tx("INSERT INTO t VALUES (7)")`，不 `commit`，`close`，丢掉句柄，再 `Database::open`，执行 `SELECT id FROM t WHERE id = 7`。当前断言是 `rows.is_empty()`。
  - 父 Cycle Act Response：`cargo test --offline --test session_lifecycle_test` 退出码 101，`6 passed; 1 failed`。失败用例的查询结果是 `[[Int(7)]]`。把 `close` 临时换回只调用公开 `checkpoint()` 后，同一用例仍是退出码 101，查询结果仍是 `[[Int(7)]]`。同文件其余 6 条为 ok。
  - `Database::close`（`src/database.rs`）调用 `checkpoint_under_session_gate`，然后对 `wal_buffer` 调用 `shutdown`。公开 `checkpoint` 在同一前段之后 `resume_after_checkpoint`，并用 `ResumeOnDrop` 覆盖 future 被丢掉的情况。前段在有 checkpoint manager 时持有会话闸门，顺序是 `reclaim_abandoned_implicit_transactions`、`publish_committed_narrowings`、`quiesce_for_checkpoint`、`CheckpointManager::checkpoint`。
  - `reclaim_abandoned_implicit_transactions` 只清空 `implicit_transactions`。调用方仍持有的显式事务不在这个名单里。
  - `TransactionManager::commit` 在 `wal_buffer` 为 `Some` 时，于 `append(CommitTxn)` 之前取得会话闸门，`append_commit_and_wait` 返回后释放，然后才 `commit_mark_versions`。
  - `CheckpointManager::checkpoint` 在 `flush_all` 之后 `rewrite_truncate`。这是既有九步里的截断，本 Cycle 不改。
  - `statement_snapshot` 在 `RepeatableRead` 时返回 `None`。`IndexScan` 在 `snapshot` 为 `None` 时走 `read_tuple_from_data_page`，不按版本头过滤。
  - `Recovery::mark_uncommitted_aborted` 在未提交集合为空时直接返回。
  - `flushed_uncommitted_insert_does_not_resurrect` 在 `do_flush` 和 `flush_all` 之后 `drop(db)`，不调用 `close`。本 Cycle 不改这个用例，也不改规格 `mvcc-tombstone-visibility` 的场景「落盘未提交行重启后不复活」。
- Code and Critical Path:
  - 本 Cycle 的唯一修改入口：`tests/session_lifecycle_test.rs` 的 `uncommitted_row_absent_after_reopen`。
  - 查询入口是该用例里的 `Database::open` 与 `execute_sql`。产品路径 `Database::close`、`Database::checkpoint`、`TransactionManager::commit` 保持父 Cycle 落地后的形状。
  - 数据：插入的行在重开后被 `SELECT id FROM t WHERE id = 7` 读到，结果为 `[[Int(7)]]`。
  - 状态：`close` 不把该显式事务改成调用方的回滚。
  - 错误：本 Cycle 不新增错误，也不改变 `commit` 与 `close` 并发时双方都返回成功的结果。`commit_and_close_finish_within_2s` 的断言保持。

**Implementation Guidance**

只改 `uncommitted_row_absent_after_reopen` 的断言和失败文案。前置步骤保持：`begin`、`INSERT INTO t VALUES (7)`、不 `commit`、2 秒内 `close`、丢掉句柄、默认 `Database::open`、`SELECT id FROM t WHERE id = 7`。

把 `assert!(rows.is_empty(), ...)` 改成：

```rust
assert_eq!(rows, vec![vec![rtsql::Value::Int(7)]]);
```

失败文案说明重开后应查到该行。函数名可以保持。不新增第二条用例，也不为这个查询增加 WAL 内容断言。

**Task Definitions**

### T2: 未提交重开查询改为断言该行仍在

- Requirement/Scenario: 规格 `session-service-lifecycle` 的需求「checkpoint 与关闭对刷盘循环的责任」，场景「关闭后未提交插入仍被查到」。proposal 场景 S8。tasks.md 的 T2 剩余工作。
- Depends on: 父 Cycle 已落地的接线保持。本任务不重做接线。
- Targets: `tests/session_lifecycle_test.rs` 的 `uncommitted_row_absent_after_reopen`
- Current behavior: 该用例要求 `rows.is_empty()`。实际查询结果是 `[[Int(7)]]`，所以 `cargo test --offline --test session_lifecycle_test` 退出码 101，`6 passed; 1 failed`。
- Required behavior: 同一前置步骤下，断言 `rows == vec![vec![rtsql::Value::Int(7)]]`。该命令退出码 0，failed 为 0。产品上的查询结果保持 `[[Int(7)]]`。
- Preserve: 同文件其余 6 个用例的断言。父 Cycle 的会话闸门、`quiesce_for_checkpoint`、`resume_after_checkpoint`、`ResumeOnDrop` 和 `close` 之后的 `shutdown`。`commit` 与 `close` 并发时两边都在 2 秒内返回成功。已提交行在 `close` 后仍在。`flushed_uncommitted_insert_does_not_resurrect` 仍不调用 `close`。
- Forbidden: 修改 `src/` 来藏行或让该行消失。修改 `src/database.rs`、`src/transaction/manager.rs`、`src/wal/checkpoint.rs`、`src/wal/recovery.rs`、`src/executor/index_scan.rs`、`src/wal/buffer.rs`。在 `close` 里回滚显式事务。把 `append_commit_and_wait` 改成返回 `Err`。修改 `flushed_uncommitted_insert_does_not_resurrect` 或规格 `mvcc-tombstone-visibility`。把父 Cycle 的原验收写成已经满足。本 Cycle 跑全量。把「该行仍被查到」写成覆盖刷盘后丢句柄那条测试的新持久化承诺。
- First test: 变更前观察沿用父 Cycle Act Response，不重跑。命令 `cargo test --offline --test session_lifecycle_test`，退出码 101，`6 passed; 1 failed`，失败查询结果 `[[Int(7)]]`。来源是该 Act Response 的「验证」；覆盖的产品代码和本测试文件在 `454c87d723df76ac66bb1c6c637b7c70087693c9` 之后、本 Cycle 文档修订之前没有再改。
- GREEN condition: 只改该断言和失败文案后，`cargo test --offline --test session_lifecycle_test` 退出码 0，输出里 failed 为 0。
- Verification: 同一条命令。退出码 0 且 failed 为 0 即通过。退出码非 0 或 failed 非 0 即本任务失败。不使用 `--test-threads=1`，因为这是单次目标测试。不跑全量。
- Stop when: 要改产品代码才能让该命令通过；断言仍要求该行不存在；要改九步、恢复规则或无快照点查；要在 `close` 里回滚显式事务；要让并发 `commit` 返回错误；要改 `flushed_uncommitted_insert_does_not_resurrect`；要把结果写成新的持久化承诺。这些都返回计划，不在本 Cycle 实施。

**Invariants**

- 已提交行在 `close` 后重新打开仍然可见。
- `append_commit_and_wait` 仍返回 `Ok(())`。会话闸门在 `commit_mark_versions` 之前释放。
- 显式事务不被 `close` 代替调用方回滚。
- checkpoint 九步、位点格式和 `mark_uncommitted_aborted` 在未提交集合为空时直接返回，都保持现状。
- 默认 `RepeatableRead` 的 `statement_snapshot` 仍返回 `None`。无快照点查仍读页上的槽。
- `flushed_uncommitted_insert_does_not_resurrect` 的步骤和断言保持。

**Non-goals**

- 不实现「未提交行在 `close` 后重开不可见」。
- 不把父 Cycle 的这条原验收记成已经满足。
- 不跑全量。不创建 Issue 文件。
- 不重做父 Cycle 已经落地的接线。

**Acceptance**

- `uncommitted_row_absent_after_reopen` 断言重开后的查询结果等于 `vec![vec![rtsql::Value::Int(7)]]`。
- `cargo test --offline --test session_lifecycle_test` 退出码 0，failed 为 0。
- 本 Cycle 的 diff 不改 `src/`，也不改 `flushed_uncommitted_insert_does_not_resurrect`。
- 父 Cycle 的「未提交行在 `close` 后不可见」保持撤回。本 Cycle 的通过不补上那条验收。

**Verification**

场景「关闭后未提交插入仍被查到」由 `uncommitted_row_absent_after_reopen` 走 `Database::open`、`close` 和 `execute_sql`。通过条件是上面的整文件命令。同文件其余用例一并留在这条命令里，用来看到已提交行、循环停止、`checkpoint` 恢复、关闭后提交和并发 `commit` / `close` 仍按原断言通过。

全量不在本 Cycle。它留在 Iteration 002，按 `tasks.md` 的 T3：分批串行，每批 `--test-threads=1`。

**Gate 2 Readiness**

PASS。无 BLOCKED。无 SKIPPED。

计划审查通过。RTM 无 Missing；撤回的验收已由用户原话批准。行为差异只在该测试断言，接口和错误语义不改。本 Cycle 与 `tasks.md`、规格场景「关闭后未提交插入仍被查到」、proposal 场景 S8、design 决策 7 一致。Persisted Evidence 为 `none`。用户已批准修订计划。

**Persisted Evidence**

- Mode: none

该测试可以重跑。退出码和 failed 计数写入 Act Response 即可。

**Risks and Notes**

- 函数名仍含 `absent`。改名不改变通过条件，本任务不要求改名。
- 同文件其余 6 条的上次通过结果来自父 Cycle，产品代码此后未改。本 Cycle 仍跑整文件一次，不把那 6 条单独再拆成一组保持面命令。
- 若断言改完后整文件命令仍失败，停止并返回计划。不改产品代码来凑退出码 0。

## Act Response

- Status: reported

**实际改动**

- `tests/session_lifecycle_test.rs` 的 `uncommitted_row_absent_after_reopen`：`assert!(rows.is_empty(), ...)` 改为 `assert_eq!(rows, vec![vec![rtsql::Value::Int(7)]], ...)`。失败文案改为重开后应查到该行。`begin`、`INSERT INTO t VALUES (7)`、不 `commit`、2 秒内 `close`、丢掉句柄、默认 `Database::open`、`SELECT id FROM t WHERE id = 7` 未改。函数名未改。
- `openspec/changes/2026-10-28-ms43-service-lifecycle/tasks.md`：T2 状态改为已实施，2.1 勾选。001 记为本 Cycle 的 Act Response `reported`、Review Result 仍为 `pending`。002 仍未展开。

**与计划的偏差**

无。

**自检**

- 发现：函数名仍是 `uncommitted_row_absent_after_reopen`。
- 已修复：无。计划写明改名不改变通过条件，本任务不要求改名。
- 遗留 Minor：函数名仍含 `absent`。
- Gate 3：通过。规格场景「关闭后未提交插入仍被查到」由该用例走 `Database::open`、`close` 和 `execute_sql`，断言重开后的结果等于 `vec![vec![rtsql::Value::Int(7)]]`。diff 不改 `src/`，也不改 `flushed_uncommitted_insert_does_not_resurrect`。同文件其余 6 条留在同一条命令里，结果为 ok。没有新增产品错误路径，也没有身份证明或判定层。
- Gate 4：通过。下面的命令退出码 0，failed 为 0。
- Gate 5：未触发。
- 跳过：不重跑父 Cycle 那条失败命令。不跑全量。全量留在 Iteration 002，按 T3 分批串行。

**验证**

- 变更前观察沿用父 Cycle Act Response（`iterations/001-session-checkpoint-close/000-initial.md` 的「验证」）：`cargo test --offline --test session_lifecycle_test` 退出码 101，`6 passed; 1 failed`，失败查询结果 `[[Int(7)]]`。本 Cycle 未重跑。
- GREEN：`cargo test --offline --test session_lifecycle_test`。退出码 0。`7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`，`finished in 1.05s`。`uncommitted_row_absent_after_reopen` 为 ok。

**Persisted Evidence**

None required。

**Experience Candidates**

None。

**未解决问题**

无待修项。原验收「未提交行在 `close` 后不可见」保持撤回，这次通过不补上它。

## Plan Review

- Review Result: accepted

**Findings**

- 非阻塞 Minor。`uncommitted_row_absent_after_reopen` 这个名字仍含 `absent`。计划的 Risks and Notes 写明改名不改变通过条件，本任务不要求改名。断言是 `rows == vec![vec![rtsql::Value::Int(7)]]`，失败文案是重开后应查到该行。规格场景标题是「关闭后未提交插入仍被查到」。这个名字没有把验收读成「行必须不存在」。不为此返工。

没有阻塞 Acceptance 的发现。Act 的 diff 与 Task Definition 一致：只改了该断言和失败文案，并更新了 `tasks.md` 的 T2 状态。`begin`、`INSERT INTO t VALUES (7)`、不 `commit`、2 秒内 `close`、丢掉句柄、默认 `Database::open`、`SELECT id FROM t WHERE id = 7` 仍在。本提交没有改 `src/`，也没有改 `flushed_uncommitted_insert_does_not_resurrect`。

**Acceptance Gaps**

本 Cycle 的四条 Acceptance 都有对应结果。

「未提交行在 `close` 后不可见」不是本 Cycle 的未关闭缺口。用户原话「我注意到了问题，我觉得可以接受replan更改计划，继续吧」批准撤回它。父 Cycle 没有满足它。这次通过不把它记成已经满足。

**Evidence**

- 提交 `cfb6c076d802b6fb4b05829b9dff646dc25714a8` 的 diff 只有三处：本文件的 Act Response、`tasks.md` 的 T2 状态，以及 `tests/session_lifecycle_test.rs`。测试改动是把 `assert!(rows.is_empty(), ...)` 换成 `assert_eq!(rows, vec![vec![rtsql::Value::Int(7)]], ...)`，失败文案改为 `an uncommitted insert must still be found after close and reopen`。
- 该函数现文在 `tests/session_lifecycle_test.rs` 123–153 行。前置步骤没有 `commit`。查询走 `Database::open` 与 `execute_sql`。`query_rows` 取出 `SqlOutcome::Rows` 的 `rows`，断言比较的就是这个结果。
- 验证沿用本文件 Act Response 的「验证」一节，不重跑。`cargo test --offline --test session_lifecycle_test` 退出码 0，`7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`，`uncommitted_row_absent_after_reopen` 为 ok。工作区在审查前干净，HEAD 就是该提交，断言与报告一致，没有公共规则里的重跑情形。
- `tasks.md` 仍写「已撤回，不是已经满足」。规格场景「关闭后未提交插入仍被查到」仍记录现行可见结果，并写明不覆盖 `flushed_uncommitted_insert_does_not_resurrect`。
- Persisted Evidence 为 none。未因缺少 evidence 目录提出问题。Experience Candidates 为 None。

**Follow-up Decision**

本 Cycle 的 Acceptance 已经满足，没有要留在当前 Cycle 的修复，也没有需要新执行依据的缺口。Iteration Plan 保持不变。全量不并进本 Cycle。展开 Iteration 002 的目录和 `000-initial.md`。

**Iteration Plan Update**

无。Map 仍是 000、001、002。全量仍只在 002。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-10-28-ms43-service-lifecycle/iterations/002-batched-full/000-initial.md`
