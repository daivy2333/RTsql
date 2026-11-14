# 诊断上报与比较指标

对应路线：MS49。Gate 1：PASS，结论在 `proposal.md` 的 Gate 1 节。批准原文是「很好，按你推荐的来就行，默认全局最优」，加上提案里已锁定的六项口径。不是跳过。本文件不更新全局里程碑状态。

Gate 2：PASS。无 BLOCKED，无 SKIPPED。Iteration 000 的结论在 `iterations/000-stage-reporting/000-initial.md`。Iteration 001 的结论在 `iterations/001-comparison-metrics/000-initial.md`。这两个 Cycle 的 Plan Context 都为 ready。

M23 只覆盖实施中新发现的实质缺陷。本计划不预建那一轮。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。规格增量在 `specs/unified-diagnostics/spec.md`。
- 000：执行依据是 `iterations/000-stage-reporting/000-initial.md`。Plan Context 为 ready。Act Response 为 reported。Review Result 为 accepted。
- 001：执行依据是 `iterations/001-comparison-metrics/000-initial.md`。Plan Context 为 ready。尚未实施。
- 002：只列在下面的 Iteration Plan 里，不建目录。
- Persisted Evidence：none。
- 跳过项：无。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 三个流水线子指标进入阶段树；插入的执行独占加上 creation、execution、commit 等于执行包含耗时；SQL 结果不变 | 完成 |
| T2 | 000 | T1 | 两个索引子指标进入阶段树；页读和提交等待的归属不变；旧表仍打印索引子指标 | 完成 |
| T3 | 001 | 000 accepted | 首行等待、交回行数、吞吐、P95、进程峰值可从保留摘要和进程读出；上限 0 仍不计预算字节 | 未开始 |
| T4 | 002 | 001 accepted | M19 分批全量一次，每批退出码 0 且 failed 为 0 | 未开始 |

## 1. 流水线子指标（Iteration 000）

- [x] 1.1 T1：按 `iterations/000-stage-reporting/000-initial.md` 的 T1 执行。先写会失败的断言，再改 `plan_stage_impl`、`execute_stage_routed` 和 `execute_ctas_routed` 里已有的测量点。

## 2. 索引子指标（Iteration 000）

- [x] 2.1 T2：按同一 Cycle 的 T2 执行。先写会失败的断言，再改两个索引扫描执行器。不新开阶段。

## 3. 比较指标（Iteration 001）

- [ ] 3.1 T3：000 被接受之后才展开本轮并实施。本文件给出读出口径，展开时把下列内容写入该轮的 Task Definition，不重新选择公式。

当前行为：`Summary` 没有开始时刻、首行等待和交回行数。没有 `segment_metrics` 或 `process_memory_peak_bytes`。`execute_executor` 把行收进 `SqlOutcome::Rows`。`QueryBudget::charge` 在上限为 0 时直接返回。

完成后：

- `Summary` 增加 `started_at`、`first_row`、`rows_delivered`。成功且至少交回一行时，`first_row` 不大于 `total`，`rows_delivered` 等于结果行数。插入、空结果、出错、被放弃的语句中 `first_row` 为 `None`，`rows_delivered` 为 0。
- 只在 `execute_executor` 收到 `ExecResult::Row` 时记录。`materialize_insert_select_source`、CTAS 源循环、派生表物化不记录。
- `segment_metrics` 按提案和 design 的墙钟与 P95 公式计算。`process_memory_peak_bytes` 在 Linux 上返回 `ru_maxrss * 1024`，失败或其他操作系统返回 `None`。
- `render` 不追加这四项。查询上限为 0 的语句仍返回原来的行，不报预算超限。

必须保持：语句总耗时仍是根包含耗时。错误文本、SQLSTATE、SQL 结果不变。五段子指标的接线不在本任务里返工。

禁止：把指标写入结果列；读 `QueryBudget` 或分配器统计；解析 `VmHWM`；新建场景对象；跑全量。

先行测试：`tests/diagnostics_comparison_test.rs` 的 `nonempty_query_records_first_row` 与 `p95_uses_nearest_rank_upper_bound`。改前 RED。`abandoned_future_summary_is_cancelled` 追加「首行等待为空、交回行数为 0」，可见性和再次插入的既有断言保持。

Verification：

```text
cargo test --offline --test diagnostics_comparison_test -- --test-threads=1
cargo test --offline --test diagnostics_statement_test abandoned_future_summary_is_cancelled -- --test-threads=1
cargo test --offline --test diagnostics_facade_test -- --test-threads=1
```

三条都退出码 0、failed 为 0。这不是全量。

停止条件：要得到首行等待就必须改错误文本、SQLSTATE 或结果列时，停止并回到计划。

## 4. 收尾全量（Iteration 002）

- [ ] 4.1 T4：001 被接受之后才展开本轮。按下面的分批方式跑一次全量。这是整个变更的唯一一次全量。本轮不改产品代码。不写「通过后再跑一次全量」。

分批是通过条件的一部分：

1. 任何测试调用都带 `--test-threads=1`。禁止默认并行度。
2. 执行时重新枚举，不把规划时的文件数写死。集成测试来自 `ls tests/*.rs | sort`，数量为 N。
3. 第一批：`cargo test --offline --lib --bins -- --test-threads=1`。
4. B 是不小于 N/12 的最小整数，K = B×12−N。K 小于等于 B 时，按该排序前 K 批各 11 个文件，其余各 12 个。文件名去掉 `.rs` 得到 `--test` 的名字。每批 `cargo test --offline --test <名> ... -- --test-threads=1`。
5. K 大于 B，或任一集成测试批的文件数不在 8 到 12 之间时，停止并回到计划。不另造切分，也不改成一次无过滤的 `cargo test`。
6. 批间一条命令结束后再开始下一条。禁止一次不带 `--lib`、`--bins` 或 `--test` 过滤的 `cargo test`。
7. 对账：全部 `--test` 名字的集合等于枚举集合，无缺失、无多余、无重复；`--lib` 和 `--bins` 只出现在第一批。
8. 每一批把退出码和 failed 计数写入 Act Response。退出码 0 且 failed 为 0 才算该批通过。任一批未跑，不得声明全量通过。
9. 某一批失败时只把该批再跑一次。第二次仍失败则本任务失败。不重跑已通过的批，不再跑一遍全量。
10. 诊断输出不得当正确性判据。通过与否只看该批测试进程的退出码和 failed 计数。

## Iteration Plan

### Iteration 000: 五个旧子指标进入阶段树

- Tasks: T1, T2
- Depends on: None
- 状态：Plan Context 为 ready。Act Response 为 reported。Review Result 为 accepted。目录已展开。执行依据是 `iterations/000-stage-reporting/000-initial.md`。
- Stable result: 性能统计打开时，五段旧耗时出现在当前语句摘要里；普通插入的执行独占加上 creation、execution、commit 等于执行包含耗时；页读和提交等待仍归当时开着的阶段；旧表仍打印这些名字；SQL 结果、错误文本和 SQLSTATE 不变。
- Verification boundary: 只跑本轮 Cycle 里的定向命令，每条都带 `--test-threads=1`，按任务顺序执行。不跑全量。全量预算在 Iteration 002，整个变更一次。
- Diagnostic boundary: T1 看 `plan_stage_impl`、`execute_stage_routed`、`execute_ctas_routed` 的现有测量点，以及两条独占等式测试。T2 看两个索引扫描执行器、`file_page_read_records_io` 和 `cli_main_profiling_test`。`diagnostics.rs` 的独占公式、`profiling.rs` 的打印、`execute_stage_in_tx_routed` 不在本轮修改范围。
- Non-goals: 比较指标、摘要新字段、进程峰值、全量、显式事务路径上的新测量。

平衡：五段是同一种接线，拆成两个任务只为把流水线测量点和索引执行器分成两个故障域。合成一轮，是因为它们共同证明「旧子指标已进门面且等待归属没变」。再拆开会得到不能单独交付的半截上报。比较指标使用另一组字段和公式，不放进本轮。

### Iteration 001: 比较指标

- Tasks: T3
- Depends on: Iteration 000 accepted
- 状态：已展开。Plan Context 为 ready。执行依据是 `iterations/001-comparison-metrics/000-initial.md`。尚未实施。
- Stable result: 调用方从 `diagnostic_summaries` 和 `segment_metrics` / `process_memory_peak_bytes` 读到首行等待、吞吐、P95 和 Linux 进程峰值。上限为 0 的查询仍返回原行。指标不在结果列里。
- Verification boundary: 本节 T3 的三条定向命令，均带 `--test-threads=1`。不跑全量。
- Diagnostic boundary: `Summary` 的三个新字段、`execute_executor` 的行收集、`segment_metrics` 与 `process_memory_peak_bytes`。五段 `record_stage` 接线不在本轮重做。
- Non-goals: 第二套计时、缓冲池计数、全量、改 SQL 结果列。

平衡：首行、吞吐、P95 和峰值是一组读出。拆开后，分段公式没有语句上的行数就无法用真实查询验收。内存峰值的断言和上限为 0 的查询放在同一次读出测试里，避免另开一轮只跑一个系统调用。

### Iteration 002: 收尾全量

- Tasks: T4
- Depends on: Iteration 001 accepted
- 状态：未展开。
- Stable result: 默认构建上按 M19 分批串行的一次全量，每批退出码 0 且 failed 为 0，目标对账无缺无重。
- Verification boundary: 本节 T4 的分批方式。只此一次。失败批只整批复核一次。
- Diagnostic boundary: 按失败测试所在的批和目标文件定位。不把全量失败当成修改分批规则的理由。分批条件不满足时回到计划，不在本轮里改公式。
- Non-goals: 第二次全量、改产品代码、非默认 feature 组合。

平衡：全量不并进 000 或 001。前两轮的定向测试已经能判断上报和比较指标。全量成本高，单独成轮。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 语句总耗时仍是根包含耗时；提交仍是执行的子阶段；独占不再假定提交是唯一子阶段 | T1 | `insert_commit_is_child_of_execute` | Covered |
| 三个流水线子指标进入摘要，插入等式成立，插入的行可见 | T1 | `insert_records_pipeline_submetrics` | Covered |
| 主键等值记入 `index_manager_search`，结果行不变 | T2 | `primary_key_lookup_records_index_search` | Covered |
| 二级索引等值记入 `index_manager_search_all` | T2 | `secondary_index_lookup_records_search_all` | Covered |
| 页读仍计入当时开着的阶段，解析阶段没有 Io | T2 | `file_page_read_records_io` | Covered |
| 排除 Index 或 Pipeline 时对应子指标不出现 | T2 | `index_stages_hidden_when_module_excludes_index` | Covered |
| 旧表仍含 `index_manager_search`，结果行仍在 | T2 | `main_command_pk_scan_returns_row_and_legacy_table` | Covered |
| 非空查询有首行等待，交回行数等于结果行数 | T3 | `nonempty_query_records_first_row` | Covered |
| 插入、空结果、出错没有首行等待，错误文本与 SQLSTATE 不变 | T3 | `insert_empty_and_error_omit_first_row` | Covered |
| 被放弃的语句没有首行等待，该键可再次插入 | T3 | `abandoned_future_summary_is_cancelled` | Covered |
| 吞吐和 P95 只用保留摘要，P95 取最近秩上界 | T3 | `p95_uses_nearest_rank_upper_bound` | Covered |
| 墙钟为 0 或行数为 0 时的吞吐 | T3 | `throughput_handles_zero_wall_and_zero_rows` | Covered |
| Linux 进程峰值等于 `ru_maxrss * 1024`，上限 0 仍返回原行 | T3 | `linux_memory_peak_matches_ru_maxrss` | Covered |
| 比较指标不进入 SQL 结果 | T3 | `comparison_metrics_absent_from_sql_rows` | Covered |
| 全量只跑一次且按 M19 分批 | T4 | T4 的分批命令 | Covered |

没有 Simplified。没有 Missing。
