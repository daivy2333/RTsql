# 主命令 profiling：进程不再中止，并输出旧三段表

来源：MS45，承接 ISS42。约束：M19、M22、M23。需求与场景在 `proposal.md`，做法在 `design.md`，规格增量在 `specs/pipeline-stage-decomposition/spec.md`。

Gate 1 为 PASS，来自本次 goal 的需求批准。Gate 2 的结论写在 `iterations/000-main-command-profiling/000-initial.md` 的 Plan Context。本文件不更新全局里程碑状态。

## 当前批次状态

- 需求与规格增量：已写入。delta 已通过 `openspec validate --strict`（Gate 1 提交时）。
- 000：已完成。`iterations/000-main-command-profiling/000-initial.md` 的 Review Result 为 `accepted`。
- 001：已完成。`iterations/001-full-suite/000-initial.md` 的 Review Result 为 `accepted`。
- 跳过项：无。

000 的任务级执行依据以 Cycle 里的 Task Definition 为准。本文件与它冲突时，以 Cycle 为准。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 变量已设置时，主命令的索引扫描不再中止，进入执行阶段的语句输出旧三段表；未设置变量、未进入执行阶段、import 与统一诊断输出口径保持 | 完成（Review Result accepted） |
| T2 | 001 | 000 accepted | M19 分批串行跑一次全量，每批退出码 0 且 failed 为 0 | 完成（Review Result accepted；第 1 批多跑一次为非阻塞 Minor） |

## 1. 主命令三段表（Iteration 000）

- [x] 1.1 T1：按 `iterations/000-main-command-profiling/000-initial.md` 的 T1 Task Definition 执行。先写 `tests/cli_main_profiling_test.rs` 并看到 RED，再改 `src/cli/mod.rs` 的 `run_sql`。规格 delta 已在本变更里，实施时不改 `openspec/specs/` 的权威正文。

当前行为：变量已设置时，主键等值与二级索引等值会在 `record_time` 处中止进程；没有索引点查的语句能结束，但没有旧表。变量未设置时不打表，结果与现在相同。import 已有旧表。

完成后：proposal 场景 S1–S11 的可观察结果成立。错误文本和 SQLSTATE 与改前相同。预编译语句的代码路径不动。

必须保持：`src/profiling.rs`、`src/pipeline.rs` 的打印点、`src/database.rs` 的预编译入口、两个索引执行器、`execute_stage_in_tx_routed` 的签名、import、统一诊断渲染。

禁止：把主命令改道到 `execute_with_params`；让 `record_time` 在没有 scope 时变成空操作；为预编译语句建 scope；新增身份证明机制或判定层；在本轮跑全量。

先行测试：`tests/cli_main_profiling_test.rs`。RED 是其中的新行为用例，在改 `run_sql` 之前失败。GREEN 是该文件与 `tests/diagnostics_statement_test.rs` 都退出码 0、failed 为 0。

Verification：

```text
cargo test --offline --test cli_main_profiling_test -- --test-threads=1
cargo test --offline --test diagnostics_statement_test -- --test-threads=1
```

这两条都不是全量。全量是 T2。

停止条件：见 Cycle 的 T1 `Stop when`。

## 2. 收尾全量（Iteration 001）

- [x] 2.1 T2：T1 的 Iteration Review Result 为 `accepted` 之后，按下面的分批方式跑一次全量。这是整个变更的唯一一次全量。不改产品代码。失败时停止，不在本轮里修。展开 001 时，把本节规程写入该 Iteration 的 `000-initial.md`，不改这套分批规则。规程已写入 `iterations/001-full-suite/000-initial.md` 的 T2，分批条文与本节一致。

分批是通过条件的一部分：

1. 执行时枚举，不把规划时的文件数写进通过条件。集成测试文件来自 `ls tests/*.rs | sort`。规划时这组文件是 162 个；T1 会新增 `tests/cli_main_profiling_test.rs`。这两个数字都不是通过条件。
2. 第一批单独执行：`cargo test --offline --lib --bins -- --test-threads=1`。
3. 其余文件按该排序每 12 个一批，最后一批可以少于 12。12 取 M19 经验值 8–12 的上沿。文件名去掉目录和 `.rs` 得到 `--test` 的名字。每批一条命令：`cargo test --offline --test <名> --test <名> ... -- --test-threads=1`。
4. 批间一条命令结束后再开始下一条。禁止一次不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`。禁止全量批次使用默认并行度。
5. 对账：全部 `--test` 名字的集合等于枚举集合，无缺失、无多余、无重复；`--lib` 和 `--bins` 只出现在第一批。
6. 每一批把退出码和 failed 计数写入 Act Response。退出码 0 且 failed 为 0 才算该批通过。任一批未跑，不得声明全量通过。
7. 某一批失败时，只把该批再跑一次。第二次仍失败则本任务失败，不改并行度，不为了凑通过而重跑其余已通过的批。

## Iteration Plan

### Iteration 000: 主命令输出三段表且索引扫描不再中止

- Tasks: T1
- Depends on: None
- 状态：已完成。`iterations/000-main-command-profiling/000-initial.md` 的 Review Result 为 `accepted`。
- Stable result: `RTSQL_PROFILING` 已设置时，主命令的索引扫描返回结果或既有 SQL 错误，进入执行阶段的语句向 stderr 输出旧三段表；未设置该变量时主命令不打这张表。import 的旧表和统一诊断的默认输出保持。
- Verification boundary: 上面两条定向 `cargo test`。不跑全量。全量预算在 001，整个变更一次。
- Diagnostic boundary: `src/cli/mod.rs::run_sql` 的 scope 与打表，以及 `tests/cli_main_profiling_test.rs` 的子进程断言。import 用例失败时再看 `src/pipeline.rs` 的既有打印点有没有被碰到。错误文本或 SQLSTATE 变化不属于「补上 scope」能解释的失败。
- Non-goals: 预编译语句、统一诊断输出口径、import 的打表时机、全量回归、批量写入与 GC。

### Iteration 001: 收尾全量

- Tasks: T2
- Depends on: 000 accepted
- 状态：已完成。`iterations/001-full-suite/000-initial.md` 的 Review Result 为 `accepted`。
- Stable result: 默认构建上按 M19 分批串行的一次全量，每批退出码 0 且 failed 为 0，目标对账无缺无重。
- Verification boundary: 本节 T2 的分批方式。只此一次。失败批只整批复核一次。
- Diagnostic boundary: 按失败测试所在的批和目标文件定位。不把全量失败当成修改分批规则的理由。
- Non-goals: 新行为、补跑已通过的批、无过滤全量、默认并行度全量。

## 平衡审计

000 只有一个结果：主命令在开关打开时不再中止，并按 import 同一张表的规则输出。主键点查、二级等值、无索引语句、错误路径、显式事务、多语句和 ATTACH 都靠同一次 `run_sql` 改动成立，拆开会得到不能单独交付的半截行为。001 只有全量，和「行为失败」分开排障。两个 Iteration 都不是按文件数或固定任务数切的。没有预建「修复尚未发现的缺陷」的 Iteration。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 三段顶层计时观测（主键等值扫描返回结果并输出三段表） | T1 | `main_command_pk_scan_returns_row_and_legacy_table` | Covered |
| 三段顶层计时观测（二级索引等值扫描返回结果并输出三段表） | T1 | `main_command_secondary_scan_returns_row_and_legacy_table` | Covered |
| 三段顶层计时观测（无索引点查的语句也输出三段表） | T1 | `main_command_count_without_index_lookup_prints_legacy_table` | Covered |
| 三段顶层计时观测（未设置变量时不输出三段表） | T1 | `main_command_without_profiling_omits_legacy_table` | Covered |
| 三段顶层计时观测（解析失败不输出三段表） | T1 | `main_command_parse_error_omits_legacy_table` | Covered |
| 三段顶层计时观测（规划失败不输出三段表） | T1 | `main_command_missing_table_omits_legacy_table` | Covered |
| 三段顶层计时观测（执行失败仍返回既有错误并输出三段表） | T1 | `main_command_duplicate_key_keeps_error_and_prints_table` | Covered |
| 三段顶层计时观测（显式事务内的索引扫描只输出一张表） | T1 | `main_command_transaction_pk_scan_prints_one_table` | Covered |
| 三段顶层计时观测（多条语句各自输出且子指标不串） | T1 | `main_command_two_statements_keep_separate_tables` | Covered |
| 三段顶层计时观测（只执行 ATTACH 时不输出三段表） | T1 | `main_command_attach_only_omits_legacy_table` | Covered |
| 三段顶层计时观测（导入路径仍输出三段表；原场景「profiling 输出三段耗时」） | T1 | `legacy_profiling_table_follows_env_var` | Covered |
| 收尾全量（M19 分批预算） | T2 | M19 分批命令 | Covered |
