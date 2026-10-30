# 统一分级诊断与性能归因

对应路线：MS44。Gate 1 为 PASS，批准来自本次 goal 对 MS44 范围的授权，不是跳过。本文件不更新全局里程碑状态。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。
- 000：已完成——`iterations/000-diagnostics-facade/000-initial.md` 的 Review Result 为 `accepted`（Plan Review 已记录结论与后继）。
- 001：已完成——`iterations/001-statement-phases/000-initial.md` 的 Review Result 为 `accepted`（Plan Review 已记录结论、Minor finding 与两个 Issue 候选的判定）。
- 002：已完成——`iterations/002-wait-attribution/000-initial.md` 的 Review Result 为 `accepted`（Plan Review 已记录结论、三条偏差的归类与 Minor finding）。
- 003：当前 Cycle 是 `iterations/003-final-regression/000-initial.md`。Plan Context 为 `ready`，等待 Act。003 是本 change 的最后一轮（收尾全量）。
- Persisted Evidence：none。
- 无跳过项。

本 goal 覆盖 M20 / M23 的缺陷承载：实施中新发现的实质缺陷登记为 Issue（ISS42 / ISS43，已在主仓库登记，均属变更范围外的既有行为）。ISS42 的修复需产品裁定（与 Iteration 001 已批准的 Preserve 项「`record_time` 在旧 scope 外仍 panic」相抵触），按 M23 停机回报用户决定；ISS43 是注释面，已在主分支以轻量修改修正。两者都不在本 change 内承载，003 之后不再排轮。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 门面、三项配置、上限和父子耗时可在不改语句结果的前提下读回 | accepted |
| T2 | 001 | 000 accepted | 语句阶段、旧计时表、失败和取消接上；SQL 结果不变 | accepted（Cycle 000 Review Result accepted） |
| T3 | 002 | 001 accepted | 等待类别、并发归属、文件页 I/O、三种开销；`[recovery]` 仍在 | accepted（Cycle 000 Review Result accepted） |
| T4 | 003 | 002 accepted | M19 分批全量一次，每批 failed 为 0 | 未开始（Cycle 000 Plan Context ready，等待 Act） |

## 1. 诊断门面（Iteration 000）

- [x] 1.1 T1：按 `iterations/000-diagnostics-facade/000-initial.md` 的 Task Definition 执行。先写 `tests/diagnostics_facade_test.rs` 的 `stats_without_trace_records_totals` 并看到 RED，再实现。通过条件是该 Cycle 的 GREEN 命令。不改语句管道、WAL、存储和旧计时表。

## 2. 语句阶段（Iteration 001）

- [x] 2.1 T2：在 000 的门面之上，把 `pipeline::execute_with_params` / `execute_inner` 和 `Database::commit` 接上阶段记录。（GREEN：`cargo test --offline --test diagnostics_statement_test -- --test-threads=1` 7 passed / 0 failed，见 `iterations/001-statement-phases/000-initial.md` 的 Act Response。）

当前行为：只有 `RTSQL_PROFILING` 存在时，`execute_inner` 向 stderr 打 parse/plan/execute。失败和取消没有诊断摘要。`is_profiling_enabled` 只看该变量。

完成后：未命中缓存的语句摘要有 `parse`、`plan`、`execute`。实际调用 `Database::commit` 时多一个 `commit` 子阶段，其包含耗时不大于 `execute`，且 `execute` 的独占耗时加上 `commit` 的包含耗时等于 `execute` 的包含耗时。不写数据的查询没有 `commit`。缓存命中的那次执行没有 `parse`/`plan`。无法解析的语句摘要为失败，错误文本和 SQLSTATE 与关闭诊断时相同。隐式插入的 future 在提交前被放弃后，摘要为取消，下一条写语句开始前该行不可见。`RTSQL_PROFILING` 已设置且会话配置保持默认时，stderr 仍含 parse、plan、execute。该变量未设置时不打这张旧表。

必须保持：`record_time` 在旧 scope 外仍 panic。`print_timings` 的列和 `Total` 行算法不改。SQL 结果、SQLSTATE、取消后的行可见性不改。

禁止：改 `wal/buffer.rs`、`transaction/manager.rs` 的等待语义，改 `FileStorage::read_page`，迁移既有告警，新增操作标识字段，跑全量。

先行测试：`tests/diagnostics_statement_test.rs` 的 `insert_commit_is_child_of_execute`。RED 是该测试编译失败或断言失败（阶段尚未接入）。命令见 Verification。GREEN 是该文件全部测试退出码 0、failed 为 0。文件内还要覆盖：查询无 commit、缓存命中不记 parse、解析失败、放弃 future、旧开关的三段 stderr、关闭诊断与打开详细追踪各插入一行后重开都可见。

Verification：`cargo test --offline --test diagnostics_statement_test -- --test-threads=1`。该文件会设置进程级 `RTSQL_PROFILING`，所以批内单线程。这不是全量。全量仍在 T4。

停止条件：接入阶段必须改变 `SqlError` 文本、SQLSTATE 或取消后的行可见性才能完成时，停止并回到计划。

## 3. 等待、归属和开销（Iteration 002）

- [x] 3.1 T3：把设计里列出的等待点接到当前语句的槽上，并测量三种配置。（GREEN：`cargo test --offline --test diagnostics_wait_test` 6 passed / 0 failed；`cargo test --offline --test recovery_bounded_read_test` 6 passed / 0 failed，见 `iterations/002-wait-attribution/000-initial.md` 的 Act Response。）

当前行为：提交等待和文件读页不产生等待类别。刷盘任务的样本不会进某条语句的摘要。

完成后：详细追踪下，成功插入的提交子阶段给出 Lock、Queue、Io、Schedule（Schedule 允许为 0）。解析阶段有 Compute 且没有 Io。重新打开的文件库上，缓冲池尚未持有目标页时，查询摘要有 Io，`pages` 至少为 1。只开性能统计时等待以汇总数出现，没有逐条样本。两条 SQL 文本不同的并发插入各自只含自己的文本，摘要没有操作标识字段，两行都可见。同一条 `SELECT * FROM bench WHERE id = 42`（各自新库、100 行）在关闭、只开统计、详细追踪下的行集相同；三次都在 30 秒内返回；不比较三者差值。默认配置下，既有恢复测试的 stderr 仍含 `[recovery]`。

必须保持：`append_commit_and_wait` 成功和 `write_batch` 失败时都返回 `Ok(())`。刷盘仍单飞。`MemoryStorage::read_page` 不记 Io。`[recovery]` 原文不改。

禁止：给未列出的锁加计时，新建后台写出任务，把 Io 跨语句相加写成断言，用操作标识判定通过，修改 `tests/recovery_bounded_read_test.rs` 的告警断言，跑全量，修改 criterion 门槛。

先行测试：`tests/diagnostics_wait_test.rs` 的 `commit_wait_classes_present`。RED 是等待类别尚不存在。GREEN 是该文件退出码 0、failed 为 0，并且 `cargo test --offline --test recovery_bounded_read_test` 退出码 0、failed 为 0。

Verification：上面两条命令，默认并行度。不设置 `RTSQL_PROFILING`。30 秒只判定挂起。

停止条件：某个已列等待点无法在保持 `Result` 的前提下包住，或并发摘要必须靠操作标识才能区分时，停止并回到计划。

## 4. 收尾全量（Iteration 003）

- [ ] 4.1 T4：T3 通过后，按下面的分批方式跑一次全量。这是整个 change 的唯一一次全量。不改产品代码。失败时停止，不在本轮里修。

分批是通过条件的一部分：

1. 执行时枚举，不把规划时的文件数写死。集成测试文件来自 `ls tests/*.rs | sort`。
2. 第一批单独执行：`cargo test --offline --lib --bins -- --test-threads=1`。
3. 其余文件按该排序每 12 个一批，最后一批可以少于 12。文件名去掉 `.rs` 得到 `--test` 的名字。每批一条命令：`cargo test --offline --test <名> --test <名> ... -- --test-threads=1`。
4. 批间一条命令结束后再开始下一条。禁止一次不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`。禁止全量批次使用默认并行度。
5. 对账：全部 `--test` 名字的集合等于枚举集合，无缺失、无多余、无重复；`--lib` 和 `--bins` 只出现在第一批。
6. 每一批把退出码和 failed 计数写入 Act Response。退出码 0 且 failed 为 0 才算该批通过。任一批未跑，不得声明全量通过。
7. 某一批失败时，只把该批再跑一次。第二次仍失败则本任务失败，不改并行度，不为了凑通过而重跑其余已通过的批。

## Iteration Plan

### Iteration 000：诊断门面

- Tasks：T1
- Depends on：None
- Stable result：三项配置、事件上限、摘要上限和父子耗时可以不经 SQL 管道读回。默认会话执行 `SELECT 1` 仍成功，且没有新摘要。
- Verification boundary：`cargo test --offline --test diagnostics_facade_test`，默认并行度。不跑全量。全量预算在 003，整个 change 一次。
- Diagnostic boundary：`src/diagnostics.rs` 与 `Database` 的诊断槽。语句还没有阶段名称时，不属于本轮失败。
- Non-goals：改管道、WAL、文件读页、旧 stderr 表、既有告警、全量回归。

### Iteration 001：语句阶段

- Tasks：T2
- Depends on：000 accepted
- Stable result：解析、规划、执行和提交按设计挂到语句摘要上。失败与取消会结束记录。旧的三段计时表仍由 `RTSQL_PROFILING` 决定。
- Verification boundary：`cargo test --offline --test diagnostics_statement_test -- --test-threads=1`。不跑全量。
- Diagnostic boundary：`src/pipeline.rs`、`src/database.rs` 的 `commit` 接线、`src/profiling.rs` 的旧表。等待类别缺失不属于本轮失败。
- Non-goals：包锁和 I/O、并发刷盘归属、开销比较、迁移告警、全量回归。

### Iteration 002：等待、归属和开销

- Tasks：T3
- Depends on：001 accepted
- Stable result：设计所列等待点进入对应语句。并发语句不串摘要。三种配置的行集相同。`[recovery]` 仍在。
- Verification boundary：`cargo test --offline --test diagnostics_wait_test` 与 `cargo test --offline --test recovery_bounded_read_test`，默认并行度。不跑全量。
- Diagnostic boundary：`src/transaction/manager.rs` 的会话闸门、`src/wal/buffer.rs` 的登记和 `flush_locked`、`src/storage/file_storage.rs` 的 `read_page`。SQL 结果变化不属于「再加一个等待点」能解释的失败。
- Non-goals：全库插桩、后台监控、数值开销门槛、全量回归。

### Iteration 003：收尾全量

- Tasks：T4
- Depends on：002 accepted
- Stable result：默认构建上按 M19 分批串行的一次全量，每批退出码 0 且 failed 为 0，目标对账无缺无重。
- Verification boundary：本节 T4 的分批方式。只此一次。失败批只整批复核一次。
- Diagnostic boundary：按失败测试所在的批和目标文件定位。不把全量失败当成修改分批规则的理由。
- Non-goals：新行为、补跑已通过的批、无过滤全量、默认并行度全量。

## 平衡审计

000 只交付可独立测试的门面，形成 001 要依赖的记录口径。001 只交付语句阶段和旧表，失败时不用下到 WAL。002 把等待归属和开销放在同一轮，因为两者都要等阶段树存在，开销本身只有一组测量，单独成轮没有第二个故障域。003 只有全量，和 MS43 的收尾轮一样，避免行为失败和整库回归混在一次排障里。四个 Iteration 都不是按文件数切的。没有把尚未发现的缺陷预做成一轮。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 详细程度、观察类别和采集内容分开配置 | T1 | `stats_without_trace_records_totals` | Covered |
| 语句阶段与父子耗时 | T1 的固定耗时，T2 的真实插入 | `root_total_is_not_parent_plus_child` | Covered |
| 等待类别 | T3 | `commit_wait_classes_present` | Covered |
| 同一次语句的异步样本 | T3 | `concurrent_inserts_keep_own_sql` | Covered |
| 成功、失败和取消都结束记录 | T2 | `parse_error_keeps_error_text` | Covered |
| 输出限量与慢输出 | T1 的上限；T2 断言结果行不含阶段名 | `event_limit_increments_dropped` | Covered |
| 三种开销被实测且不设数值门槛 | T3 | `three_modes_match_rows` | Covered |
| 既有计时出口和 stderr 告警保持 | T2 的旧表，T3 的 `[recovery]` | `legacy_profiling_prints_three_stages` | Covered |
| 诊断不改变事务结果 | T2 | `insert_visible_with_and_without_diagnostics` | Covered |
| 收尾全量（proposal 需求 9） | T4 | M19 分批命令 | Covered |
