# 无键行版本链进入 GC

来源：MS46，承接 I038。约束：M10、M19、M23。配方：R48。需求与场景在 `proposal.md`，做法在 `design.md`，规格增量在 `specs/keyless-version-gc/spec.md`。

Gate 1 为 PASS，来自本次 goal 的需求批准。Gate 2 的结论写在 `iterations/000-keyless-gc/000-initial.md` 的 Plan Context。本文件不更新全局里程碑状态。

## 当前批次状态

- 需求与规格增量：已写入。
- 000：当前 Cycle 已展开。执行依据以该 Cycle 的 Task Definition 为准。本文件与它冲突时，以 Cycle 为准。
- 001：只列在下面的 Iteration Plan 里，尚未建目录。
- 跳过项：无。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | proposal 场景 S1–S11 成立；既有三个带键 GC 用例断言不改并通过 | 未开始 |
| T2 | 001 | 000 accepted | 按本节分批规程跑一次全量，每批退出码 0 且 failed 为 0 | 未开始 |

## 1. 无键链回收与带键合同（Iteration 000）

- [ ] 1.1 T1：按 `iterations/000-keyless-gc/000-initial.md` 的 T1 Task Definition 执行。先在 `tests/gc_test.rs` 与 `tests/overflow_page_test.rs` 写先行测试并看到 RED，再改 `src/storage/data/table_manager.rs` 的 `TableMeta::gc_table`。规格 delta 已在本变更里，实施时不改 `openspec/specs/` 的权威正文。

当前行为：GC 只走主键索引。无键链的旧版本留在数据页上。带键链按现有谓词回收。已删除的带键链不回收。

完成后：S1–S11 的可观察结果成立。`gc_table` 的签名仍是 `(&self, buffer_pool: &BufferPool) -> Result<usize>`。

必须保持：索引走查循环的判定、`delete_tuple_from_data_page`、快照函数、空闲页提升、WAL、索引维护。

禁止：用数据页走查替换索引走查；按快照留下旧版本；为无键链另写释放；新增 SQL 或后台 GC；在本轮跑全量。

先行测试与 GREEN 命令写在 Cycle 的 T1。这两条都不是全量。全量是 T2。

停止条件：见 Cycle 的 T1 `Stop when`。

## 2. 收尾全量（Iteration 001）

- [ ] 2.1 T2：T1 的 Iteration Review Result 为 `accepted` 之后，按下面的分批方式跑一次全量。这是整个变更的唯一一次全量。不改产品代码。失败时停止，不在本轮里修。展开 001 时，把本节规程写入该 Iteration 的 `000-initial.md`，不改这套分批规则。

分批是通过条件的一部分，依据 M19 与 R48 的等分规则：

1. 执行时枚举，不把规划时的文件数写进通过条件。集成测试文件来自 `ls tests/*.rs | sort`。规划时这组文件是 163 个。T1 的新用例写进已有的 `tests/gc_test.rs` 和 `tests/overflow_page_test.rs`，不新增集成测试文件。文件数若与 163 不同，以执行时的枚举为准。
2. 令 `N` 为该枚举的条数，`B = ceil(N / 12)`。文件按该顺序切片：前 `(B × 12 − N)` 批各 11 个，其余各 12 个。文件名去掉目录和 `.rs` 得到 `--test` 的名字。每批规模只为 11 或 12。
3. 第一批单独执行：`cargo test --offline --lib --bins -- --test-threads=1`。
4. 然后按上面的切片逐批执行：`cargo test --offline --test <名> ... -- --test-threads=1`。前一批的进程结束之后再启动下一批。同一时刻只有一批。
5. 禁止一次不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`。禁止全量批次去掉 `--test-threads=1` 或改用默认并行度。
6. 对账：全部 `--test` 名字的集合等于枚举集合，无缺失、无多余、无重复；`--lib` 和 `--bins` 只出现在第一批。对账基准是执行时重新枚举的文件集合。
7. 每一批把退出码和 `test result:` 行里的 passed / failed / ignored 写入 Act Response。计数只用一步 `grep -E "^test result:"` 读出。退出码 0 且 failed 为 0 才算该批通过。任一批未跑，不得声明全量通过。
8. 某一批失败时，只把该批再跑一次。第二次仍失败则本任务失败。不改并行度，不为了凑通过而重跑其余已通过的批。cargo 在该批第一个失败目标后不再跑后续目标，所以复核必须是整批。

`--offline` 只避免在验证时访问依赖源，不改变目标集合和并行度。

## Iteration Plan

### Iteration 000: 无键旧版本可回收且带键合同仍在

- Tasks: T1
- Depends on: None
- Stable result: `gc_table` 回收无键链上已提交的非链头版本，链头和未提交版本留下；带键索引走查、已删除带键链和回滚后的有效版本保持；溢出链仍按现有提升边界释放。
- Verification boundary: Cycle T1 的两条定向 `cargo test`。不跑全量。全量预算在 001，整个变更一次。
- Diagnostic boundary: `TableMeta::gc_table` 的索引走查与其后的无键收集、删除，以及 `tests/gc_test.rs`、`tests/overflow_page_test.rs` 里本次新增和原有的 GC 用例。`SELECT` 结果不对时再看执行器有没有被本轮改到；本轮不应改执行器。
- Non-goals: 快照可见性、空闲页提升规则、索引格式、WAL、刷盘、SQL GC、全量回归、MS47。

### Iteration 001: 收尾全量

- Tasks: T2
- Depends on: 000 accepted
- Stable result: 默认构建上按 M19 与 R48 分批串行的一次全量，每批退出码 0 且 failed 为 0，目标对账无缺无重。
- Verification boundary: 本节 T2 的分批方式。只此一次。失败批只整批复核一次。
- Diagnostic boundary: 失败测试所在的批和文件。本轮不改产品代码；失败要修时回到计划，不在本轮里改。
- Non-goals: 新的 GC 行为、重跑已通过的批、无过滤的全量命令、默认并行度。

## 平衡审计

000 的结果是一次 `gc_table` 调用同时满足无键回收和带键合同。拆成「先只找链头」和「再删除」两轮的话，中间没有可以单独交付的稳定行为。001 只做全量，故障域是整套测试，和 000 的 `gc_table` 定位分开。两轮都过同一条聚合标准：000 不是整个变更，001 也不是单条用例。高开销的全量只预算一次，放在 001。

## 需求跟踪

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 无键行的旧版本按既有 GC 规则回收 | T1 | `keyless_transition_collects_old_versions_and_selects_head` | Covered |
| 带键行的 GC 合同保持 | T1 | `deleted_keyed_chain_is_not_collected` | Covered |
| 走查不能结束时不删除无键版本 | T1 | `keyless_gc_stops_on_data_page_cycle_without_deleting` | Covered |

场景、设计和代码位置在 proposal、design 和 Cycle 的 T1 里。T2 不对应新的行为要求，它是整个变更的一次全量预算。
