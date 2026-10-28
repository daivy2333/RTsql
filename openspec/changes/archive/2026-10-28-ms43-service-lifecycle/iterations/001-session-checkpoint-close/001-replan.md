# Iteration 001 / Cycle 001: 关闭后未提交行的验收不能在本轮改写

## Plan Context

- Status: draft
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T2
- Acceptance gaps: 父 Cycle `000-initial.md` 的「未提交行在 `close` 后不可见」
- Repair items: None
- Inherited scope: 父 Cycle 已经落地、并且不改变事务结果和持久化合同的接线。`commit` 在 `wal_buffer` 为 `Some` 时，从 `append(CommitTxn)` 到 `append_commit_and_wait` 返回持有会话闸门，释放之后才 `commit_mark_versions`。公开 `checkpoint` 在九步前 `quiesce_for_checkpoint`，返回时 `resume_after_checkpoint`，future 被丢掉时 `ResumeOnDrop` 补一次恢复。`close` 走同一段前序，然后 `shutdown`，不调用公开 `checkpoint()`。只读和内存会话在没有 checkpoint manager 时立即返回。
- Excluded scope: 让未提交行在 `close` 后重开不可见。在 `close` 里中止仍活跃的显式事务。改 `CheckpointManager::checkpoint` 的九步、位点格式和错误类型。改恢复规则，包括 `redo_count == 0` 时重建索引，以及无快照点查改看版本头。改 `WalWriter`、`BufferPool`、`WALBuffer` 的刷盘、暂停和停止后等待。把 `append_commit_and_wait` 改成返回 `Err`。全量（T3）。把失败用例改成期望该行可见，并据此把父 Cycle 的原验收标成通过。

**Objective**

本 Cycle 不产生新的可观察产品行为。父 Cycle 的生命周期接线保持。未提交行在 `close` 后重开仍然可见，与改前只调用公开 `checkpoint()` 的 `close` 相同。这个结果不在本 Cycle 里改掉，也不改写成新的规格句子。

**Background**

父 Cycle 是 `iterations/001-session-checkpoint-close/000-initial.md`，Review Result 为 `replan-required`。Act Response 为 `blocked`。阻塞点是 `uncommitted_row_absent_after_reopen`：显式事务插入后不提交就 `close`，重新打开后 `SELECT id FROM t WHERE id = 7` 得到 `[[Int(7)]]`。把 `close` 换回父 Cycle 开始时的实现，结果相同。

proposal 场景 S8 和规格场景「未提交缓冲不在关闭后可见」把「重开后该行不存在」写成了本变更的验收。里程碑排除项包含改变既有事务与持久化合同。本变更「不做」包含改变 checkpoint 九步和恢复规则，并且不新增错误类型。

**Investigation Facts**

- Starting point: 分支 `changes/2026-10-28-ms43-service-lifecycle`，提交 `454c87d723df76ac66bb1c6c637b7c70087693c9`。父 Cycle 的定向结果沿用其 Act Response，本 Cycle 不重跑已通过的命令。`cargo test --offline --test session_lifecycle_test` 退出码 101，`6 passed; 1 failed`，失败用例是 `uncommitted_row_absent_after_reopen`。保持面命令在 `ResumeOnDrop` 之后退出码 0。
- Current-State Evidence:
  - `Database::close` 调用 `checkpoint_under_session_gate`，然后对 `wal_buffer` 调用 `shutdown`。公开 `checkpoint` 在同一前段之后调用 `resume_after_checkpoint`。前段在有 checkpoint manager 时持有会话闸门，顺序是回收被放弃的隐式事务、发布已提交窄化、`quiesce_for_checkpoint`、`CheckpointManager::checkpoint`。
  - 回收只覆盖已放弃的隐式事务。仍由调用方持有的显式事务不在回收名单里。`statement-cancellation-outcome` 规定这种事务的回滚由调用方决定。
  - `CheckpointManager::checkpoint` 在 `flush_all` 之后 `rewrite_truncate`。`quiesce_for_checkpoint` 已经把 `BeginTxn` 和插入记录写入 WAL，这些记录落在捕获的 LSN 之前，随截断离开文件。脏页留在数据文件里。
  - 恢复只从剩余 WAL 收集未提交事务。没有 `BeginTxn` 时未提交集合为空，`mark_uncommitted_aborted` 不改页。`redo_count == 0` 时不重建主键索引。
  - 默认打开是 `RepeatableRead`，`statement_snapshot` 返回 `None`。无快照主键点查不读版本头。无快照扫描只跳过已删除槽。因此该行在重开后仍被查到。
  - 既有「落盘未提交行重启后不复活」的测试在刷盘后丢弃句柄，不调用 `close`。那条路径的 WAL 仍含 `BeginTxn`，恢复能标中止。它不覆盖「`close` 完成截断之后再打开」。
  - `TransactionManager::commit` 与 `close` 用同一把会话闸门排队。`commit_and_close_finish_within_2s` 已经通过：两边都在 2 秒内返回成功。`close` 可以先拿到闸门，此时事务还没有 `CommitTxn`。
- Code and Critical Path:
  - 入口：`Database::close`、`Database::checkpoint`、`TransactionManager::commit`。失败用例入口是 `tests/session_lifecycle_test.rs` 的 `uncommitted_row_absent_after_reopen`。
  - 数据：未提交插入的页在九步里被 `flush_all` 写入。对应 WAL 记录被截断。重开后的查询读到页上的槽。
  - 状态：`close` 返回后刷盘循环已 join。显式事务的活跃身份没有被 `close` 改成中止。
  - 错误：要让该行消失，又要让先等到闸门的 `commit` 仍然成功，当前代码没有这样的路径。中止会使 `commit` 返回错误。保留页并只补 WAL 标记，点查仍能读到槽。

**Implementation Guidance**

没有可以交给实施的改动。范围内的接线已经在 `454c87d`。下一步如果是产品改动，只会走到下面两条被排除的路之一，因此本 Cycle 停在草稿。

**Task Definitions**

### 无任务

- Requirement/Scenario: 父 Cycle 的「未提交缓冲不在关闭后可见」不能映射到一条仍落在本里程碑范围内的任务。
- Depends on: 父 Cycle 的接线保持，不在这里重做。
- Targets: 无。
- Current behavior: `close` 后重新打开，未提交插入仍能被无快照查询读到。已提交行、循环停止、`checkpoint` 恢复、取消、只读和内存路径按父 Cycle 已通过的用例保持。
- Required behavior: 本 Cycle 不定义新的产品行为。不把「重开后不可见」写成必须实现的结果，也不把「重开后可见」写成新的持久化合同。
- Preserve: 父 Cycle 已落地的会话闸门、暂停、恢复、`shutdown` 和 `ResumeOnDrop`。`commit` 与 `close` 并发时两边仍返回成功。已提交行在 `close` 后仍在。
- Forbidden: 修改 `src/database.rs`、`src/transaction/manager.rs`、`src/wal/checkpoint.rs`、`src/wal/recovery.rs`、`src/executor/index_scan.rs` 来隐藏该行。删除或改写 `uncommitted_row_absent_after_reopen` 的断言来让 `session_lifecycle_test` 退出码变成 0，并据此宣布父 Cycle 原验收已满足。修改 proposal、规格或 `tasks.md` 里的任务合同，把 S8 改成另一种可见性承诺。
- First test: 不新增，也不修改现有失败用例。父 Cycle 的失败结果就是当前状态。
- GREEN condition: 无。Gate 2 未通过，没有 GREEN。
- Verification: 无命令。不重跑父 Cycle 已通过的定向测试。
- Stop when: 要让该行在 `close` 后不可见；要让 `commit` 在与 `close` 并发时返回错误；要改九步或恢复规则。这些都返回需求裁定，不在本 Cycle 实施。

**Invariants**

- 已提交行在 `close` 后重新打开仍然可见。
- `append_commit_and_wait` 仍返回 `Ok(())`。会话闸门在 `commit_mark_versions` 之前释放。
- 显式事务不被 `close` 代替调用方回滚。
- checkpoint 九步、位点格式和恢复对未提交集合的判定保持父 Cycle 开始时的行为。

**Non-goals**

- 不实现「未提交行在 `close` 后重开不可见」。
- 不把这条可见性登记进本变更的新合同。
- 不跑全量。不创建 Issue 文件。

**Acceptance**

本 Cycle 没有可以通过的验收。父 Cycle 里除「未提交行不可见」以外的接线结果保持，但不在这里被重新宣布为整个 T2 已经完成。

**Verification**

没有新的验证命令。

**Gate 2 Readiness**

BLOCKED。设计没有闭合：要满足父 Cycle 那条验收，就会改变持久化合同或增加错误语义；要删掉这条验收并让测试通过，就是把未满足的检查标成完成。两种写法都不能交给实施。

- BLOCKED：行为差异和错误语义。证据是父 Cycle Plan Review 的 Findings，以及本文件 Current-State Evidence。范围内的接线本身没有新的未决接口。

无 SKIPPED。计划批准不覆盖这个阻塞。本文件保持 `draft`。

**Persisted Evidence**

- Mode: none

没有要采集的新运行。父 Cycle 的失败结果可以重跑，但本 Cycle 不要求重跑。

**Risks and Notes**

- 规格句子「落盘未提交行重启后不复活」的正文提到 checkpoint。它的现行测试不调用 `close`。完成截断之后 WAL 里没有 `BeginTxn`，恢复无法沿那条测试的路径标中止。这是现状和那句规格正文之间的缝，不是本轮刷盘接线能单独补上的行为。
- 若以后的需求裁定选择「`close` 必须让该行不可见」，那一轮要单独写错误语义和恢复或九步的改动，并重新通过需求批准。那一轮不能从本文件的 `draft` 直接改成 `ready`。

## Act Response

- Status: 待 Act 填写

## Plan Review

- Review Result: replan-required

**Findings**

本 Cycle 没有 Act。Plan Context 里的 Gate 2 停在当时的停止条件：不能改 proposal 和规格，也不能把失败用例改成期望该行可见。用户原话「我注意到了问题，我觉得可以接受replan更改计划，继续吧」随后批准了修订计划。批准范围写在 `002-replan.md` 的 Background。本文件的 Plan Context 保持原文，状态保持 `draft`，不改为 `ready`。

**Acceptance Gaps**

本 Cycle 的 Plan Context 写明没有可以通过的验收。父 Cycle 的「未提交行在 `close` 后不可见」不由本文件宣布已经满足。该验收在 `002-replan.md` 按用户裁定撤回。

**Evidence**

- 用户原话见 `002-replan.md` 的 Background，以及 proposal 的「验收修订」。
- `454c87d723df76ac66bb1c6c637b7c70087693c9` 到 `29c6b0aa0d7e359f00a25ef969ac82f3158116bd` 的 diff 只有 Cycle 文件和 `tasks.md`。产品代码和 `tests/session_lifecycle_test.rs` 未改。

**Follow-up Decision**

后继执行依据是 `iterations/001-session-checkpoint-close/002-replan.md`。本文件不交给 Act。

**Iteration Plan Update**

验收边界按用户批准修订，见 proposal「验收修订」、规格场景「关闭后未提交插入仍被查到」、`tasks.md` 的 T2 与 Iteration 001。Map 仍是 000、001、002。001 未完成。全量仍在 Iteration 002。

**Next Cycle**

`iterations/001-session-checkpoint-close/002-replan.md`

**Next Iteration**

None
