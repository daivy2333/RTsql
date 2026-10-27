# Tasks: ADD COLUMN 目录面（ISS39、ISS40）

**Change**: `2026-10-26-ms40-add-column-catalog`
**状态**：`pending`。完整状态以当前 Cycle 的 Act Response 为准。
**来源**: `.agents/issues/ISS39-add-column-races-inflight-narrowing.md`、`.agents/issues/ISS40-add-column-abandoned-during-column-count-flush.md`、`.agents/docs/tasks.md` 的 MS40。本次只规划修复，不改 Issue 的 Status。
**裁定**: `design.md` D1、D2、D3。

## 任务列表

| Task | 标题 | Iteration | 依赖 | 状态 |
|---|---|---|---|---|
| T1 | 列数刷盘期间被放弃的 ADD COLUMN 不留下已落盘的列数上调（ISS40） | 000 | None | reported |
| T2 | 在途窄化的发布不使用被加列改过的活布局，进程内读与重开一致（ISS39） | 001 | T1 | reported |
| T3 | 本变更唯一一次全量回归（M19 分批串行） | 002 | T2 | reported |

每个任务只归属一个 Iteration。T1 → 000，T2 → 001，T3 → 002。

- [x] T1 列数刷盘期间被放弃的 ADD COLUMN 不留下已落盘的列数上调（ISS40）
- [x] T2 在途窄化的发布不使用被加列改过的活布局，进程内读与重开一致（ISS39）
- [x] T3 本变更唯一一次全量回归（M19 分批串行）

## 任务要点

T1 的执行依据以 `iterations/000-add-column-flush-abandon/000-initial.md` 的 Task Definition 为准。T2 的执行依据以 `iterations/001-inflight-narrowing-publish-layout/000-initial.md` 的 Task Definition 为准。下面只保留验收与验证，避免和这些定义各写一套步骤。

- **T1（ISS40）**：`ddl-catalog-transactional`「被放弃的 ADD COLUMN 不留下已落盘的列数上调」。实现按 `design.md` D1。验收：新用例在列数刷盘返回成功之后、成功结果交回之前丢弃语句 future，处理结束后不 `close` 就崩溃再打开，列数与可见列定义回到语句前。既有 `failed_add_column_is_compensated` 与 `successful_add_column_still_adds_the_column` 保持通过。验证：`cargo test --test ddl_layout_publication_test -- --test-threads=1`，然后 `cargo test --test alter_table_test`，然后 `cargo test --test ddl_crash_recovery_test`。三次调用顺序执行，同一时刻只有一次。前一条是单目标，`--test-threads=1` 只用于稳住该文件里的暂停点，不是全量批次。后两条用默认并行度。
- **T2（ISS39）**：`ddl-layout-publication`「同表布局窄化 DDL 串行化」的两个新场景，外加既有「窄化令牌不约束 ADD COLUMN」与「同表并发的窄化 DDL 被显式拒绝」。实现按 `design.md` D2，等待包住 D1 的任务，不改 D1 的决策表。验收用例 `tests/ddl_layout_publication_test.rs::add_column_waits_out_inflight_narrowing_and_reopen_matches`：用现有 `arm_ddl_commit_window` 把删列停在提交完成到目录发布之间；同表加列不因令牌被拒绝；停住期间该表目录列数与列名不变；读该表仍被点名拒绝；另一张表的加列完成且不改这张表的目录；放行后两条都成功；进程内 `SELECT` 与关闭再打开的列集合、列数、行值相同。既有 `concurrent_same_table_ddl_is_refused` 保持通过。验证：`cargo test --test ddl_layout_publication_test -- --test-threads=1`，然后 `cargo test --test ddl_rewrite_consistency_test`。顺序执行。不跑全量。
- **T3**：不改产品代码，不改目录序列化，也不改 `legacy_db_write_upgrades_touched_table_row_to_current_layout` 与 `legacy_g2_db_opens_queries_and_writes` 的目录行长度断言（现为 `g1_tables_row_len + 10`，`tests/legacy_compat_test.rs` 442–444 行与 1089–1091 行）。通过标准仍是每一批退出码为 0，且该次每条 `test result:` 的 failed 计数为 0。缺一批、任一批 failed 非 0、或使用了无目标过滤的全量调用，本任务失败。`iterations/002-full-regression/000-initial.md` 已通过的第 00–06 批沿用，不重跑。第 07–13 批由 `iterations/002-full-regression/001-replan.md` 串行续跑。修正这两条断言的 Review 已是 `accepted`（`openspec/changes/2026-10-26-ms40-join-smj-and-build-side/iterations/003-current-catalog-row-length/000-initial.md`）。该 Cycle 的 Plan Context 为 `ready`。

## Iteration Plan

### Iteration 000: 列数刷盘期间放弃加列的目录回退

- Tasks: T1
- Depends on: None
- Stable result: 列数刷盘已经成功的 `ADD COLUMN` 被放弃后，不 `close` 的崩溃重开看不到新列，列数回到语句前。刷盘失败的补偿仍不自己刷盘。成功加列仍持久。
- Verification boundary: T1 的三条定向命令。不跑全量。本变更的全量预算在 Iteration 002。
- Diagnostic boundary: `AlterTableExecutor` 的加列臂、`alter_add_column`、`compensate_column_add`，以及只被加列使用的暂停点。删列发布、令牌拒绝、WAL 意图不在本 Iteration 的改动范围内。
- Non-goals: ISS39。其他目录面。实施中才发现的缺陷不在本 Iteration 里修。

### Iteration 001: 在途窄化与同表加列的发布布局

- Tasks: T2
- Depends on: 000
- Stable result: 正常路径发布安装的是取得令牌时捕获的窄化布局。同表加列在令牌期间不改目录，等待后追加，不被拒绝。两条都完成后，进程内读取与重开一致。在途读拒绝和第二条同表删列的立即拒绝保持。
- Verification boundary: T2 的两条定向命令。不跑全量。
- Diagnostic boundary: `alter_drop_column` 捕获的布局、`PendingColumnDrop`、`apply_column_drop` 的活布局安装、加列任务取得写锁之前的等待。恢复期 `publish_column_narrowing` 的推导保持从打开时的布局出发。
- Non-goals: 重做 D1 的放弃决策表。Join。全量回归。

### Iteration 002: 本变更的全量回归

- Tasks: T3
- Depends on: 001
- Stable result: 同一次全量的每一批退出码为 0，且 failed 计数为 0。第 00–06 批已经通过并沿用。第 07–13 批在目录行长度断言修正的 Review 为 `accepted` 之后串行跑完。
- Verification boundary: 下方「收尾验证」。通过标准不降低。已通过的第 00–06 批不重跑。第 07 批按原来的 11 个目标整批重跑，不把未跑到的 8 个目标拆出去。缺一批即未通过。该 Review 已是 `accepted`，后继 Cycle 的 Plan Context 为 `ready`。
- Diagnostic boundary: 失败批次里的目标。定向用例已在 000 与 001 通过时，不再把整次全量当成加列逻辑的定位手段。目录行多出的 `explicit_primary_key` 字节不在本 Iteration 里改。与本变更故障域相同的新缺陷不在本 Iteration 里改产品代码，留给后续 Iteration；其他故障域的新缺陷仍留在 MS40，不新建里程碑。
- Non-goals: 新功能。为失败批次改验收标准。把另一变更的目录尾字节或那两条长度断言修进本变更。预建尚未出现的缺陷的 Iteration。

## 收尾验证

整个变更的全量回归只在 Iteration 002 执行一次。规划阶段不跑。

`000-initial` 启动前按下面 1–6 步当场分批，不用计划里的历史个数代替当时的现场。那次已经分完并停在第 07 批。后继 Cycle 不再重算 `N`。

1. 集成测试目标是 `tests/*.rs` 的文件名去掉 `.rs`，按 `LC_ALL=C` 字典序。
2. 第 00 批单独一次：`cargo test --lib --bins -- --test-threads=1`。
3. 设 `N` 为集成测试目标个数，`B = ceil(N / 12)`。大小为 11 的批次数是 `B * 12 - N`，大小为 12 的批次数是 `N - 11 * B`。任一个为负，或任一批的目标数不在 8 到 12 之间，停止并回到计划，不另选分法。计划里曾经写下的 `N = 148` 只是历史个数：按这个式子会是前 8 批各 11 个、后 5 批各 12 个。`000-initial` 当场数到 `N = 149`，因此是前 7 批各 11 个、后 6 批各 12 个。这一步的重算已经用过，后继 Cycle 不再做。
4. 每个集成测试批次一次 `cargo test --test <名> --test <名> … -- --test-threads=1`。批内目标连续取自第 2 步的字典序，先排完大小为 11 的批，再排大小为 12 的批。每个目标恰好出现一次。
5. 第 00 批与各集成测试批次严格顺序执行，同一时刻只有一批。禁止一次无目标过滤的 `cargo test`。禁止这些批次使用默认并行度。
6. 通过条件是每一批退出码为 0 且 failed 计数为 0。少跑一批就是未通过。

`000-initial` 已在第 07 批停止。该次 `N = 149`，名单已固定：前 7 个集成测试批次各 11 个目标，后 6 个各 12 个。第 00–06 批退出码 0、failed 为 0，沿用，不重跑，也不重新分批。第 07–13 批使用那次名单里的命令，写在 `iterations/002-full-regression/001-replan.md`。后来 `tests/*.rs` 增减，都不改这 7 条命令。开始条件已经满足：另一变更 `openspec/changes/2026-10-26-ms40-join-smj-and-build-side/iterations/003-current-catalog-row-length/000-initial.md` 的 Review Result 为 `accepted`。本变更不修改目录序列化，也不修改这两条断言。后继 Cycle `iterations/002-full-regression/001-replan.md` 的 Act Response 为 `reported`，Review Result 为 `accepted`。第 07–13 批退出码均为 0，failed 计数均为 0。第 00–06 批仍沿用，未重跑。没有后继 Iteration。

新增测试各自带独立 `TempDir`，不依赖执行顺序、并发度或批次边界。

## 平衡审计

| Iteration | 任务 | 独立可验证结果 | 诊断边界 | 结论 |
|---|---|---|---|---|
| 000 | T1 | 放弃已刷盘的加列后，崩溃重开列数回到语句前 | 加列任务的收尾，不含删列发布 | 保留。并入 001 会把刷盘放弃和布局发布混成一个失败面 |
| 001 | T2 | 发布布局与重开一致，加列不被令牌拒绝 | 令牌期间的等待和捕获布局的安装 | 保留。依赖 000 的任务边界，不能倒过来做 |
| 002 | T3 | 全部分批 0 failed | 失败的那一批 | 保留。并入 001 会把无关批次的失败写成 ISS39 的验收失败 |

000 没有承载整个变更。001 与 002 使用和 000 相同的聚合标准：一个可独立验证的结果，一个诊断边界。第 07 批之后的续跑仍在 002 里，不另开 Iteration。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 被放弃的 ADD COLUMN 不留下已落盘的列数上调 | T1 | `abandoned_add_column_during_count_flush_is_not_durable` | Covered |
| 失败的 ADD COLUMN 目录面被回退（不刷盘，保持） | T1 | `failed_add_column_is_compensated` | Covered |
| 在途窄化窗口内的同表 ADD COLUMN 不改写发布所用布局 | T2 | `add_column_waits_out_inflight_narrowing_and_reopen_matches` | Covered |
| 另一张表的 ADD COLUMN 不等待在途窄化 | T2 | `add_column_waits_out_inflight_narrowing_and_reopen_matches` | Covered |
| 窄化令牌不约束 ADD COLUMN（保持不被拒绝） | T2 | `add_column_waits_out_inflight_narrowing_and_reopen_matches` | Covered |
| 同表并发的窄化 DDL 被显式拒绝（保持） | T2 | `concurrent_same_table_ddl_is_refused` | Covered |
| 本变更全量回归（M19） | T3 | Iteration 002 的分批命令 | Covered |

没有 Simplified。没有 Missing。

## Iteration Map

| Iteration | 标题 | Tasks | 依赖 | 状态 |
|---|---|---|---|---|
| 000 | 列数刷盘期间放弃加列的目录回退 | T1 | None | accepted。Cycle：`iterations/000-add-column-flush-abandon/000-initial.md` |
| 001 | 在途窄化与同表加列的发布布局 | T2 | 000 | accepted。Cycle：`iterations/001-inflight-narrowing-publish-layout/000-initial.md` |
| 002 | 本变更的全量回归 | T3 | 001 | accepted。`000-initial.md` 的 Review Result 为 `replan-required`。后继 Cycle：`iterations/002-full-regression/001-replan.md`，Act Response 为 `reported`，Review Result 为 `accepted`。 |

Iteration 000、001、002 均已 accepted。`002-full-regression/000-initial.md` 已有后继，不再改写。没有未展开的 Iteration。
