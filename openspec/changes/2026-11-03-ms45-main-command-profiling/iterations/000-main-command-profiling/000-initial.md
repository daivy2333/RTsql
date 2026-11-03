# Iteration 000 / Cycle 000: 主命令三段表

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `pipeline-stage-decomposition` 的 Requirement「三段顶层计时观测」。本变更的 delta 在 `specs/pipeline-stage-decomposition/spec.md`。未设置 `RTSQL_PROFILING` 时的主命令结果、错误文本、SQLSTATE 和退出码保持。import 的旧表输出时机、统一诊断默认配置下的 stderr 保持。
- Excluded scope: T2（收尾全量，Iteration 001，只写在 `tasks.md`，本 Cycle 不跑）。预编译语句入口。`openspec/specs/` 的权威正文。SNAPSHOT、tasks、issues。

**Objective**

`RTSQL_PROFILING` 已设置时，主命令执行含索引扫描的语句不再以进程中止结束。调用方得到查询结果，或得到与未设置该变量时相同的 SQL 错误。进入执行阶段的语句在 stderr 输出旧的三段计时表。未设置该变量时不输出这张表。

**Background**

本次 goal 的需求批准已经裁定：主命令要输出这张表，并与 import 已有的表、以及「三段顶层计时观测」对齐。预编译语句不在验收内。本 Cycle 按这个裁定实施，不再把打表与否留作选择。

**Investigation Facts**

- Starting point: 产品代码与 `eb84384` 相比，`src/cli/mod.rs`、`src/pipeline.rs`、`src/profiling.rs`、`src/cli/lifecycle.rs`、`src/executor/index_scan.rs`、`src/executor/index_scan_all.rs`、`src/database.rs`、`tests/diagnostics_statement_test.rs` 无差异。行号对应 `b1cec3c`。本计划没有重跑测试。import 旧表的通过记录在已归档变更 `2026-10-30-diagnostics-legacy-table-error-path`。改 `run_sql` 之前，Act 在本工作区把 `diagnostics_statement_test` 跑一次，作为当前基线；那次若失败，停止，不要先改产品代码。
- Current-State Evidence:
  - `is_profiling_enabled`（`src/profiling.rs:55-57`）只看变量是否存在。`record_time`（21–25 行）在 scope 未设置时访问 task-local，所在任务会 panic。`print_timings`（33–52 行）向 stderr 打表，表头含 `Time (µs)`，行按耗时降序，末行 `Total` 用调用方传入的耗时。`with_profiling_scope`（60–65 行）包住一个 future。
  - 全仓库只有 `execute_with_params`（`src/pipeline.rs:116-141`）在变量已设置时建立 scope。`print_timings` 的调用只有 `execute_staged` 的 1540 行和 1662 行，都在执行阶段被触达之后。
  - `run_sql`（`src/cli/mod.rs:846`）是主命令路径。调用点只有 545 行的主命令闭包。它先 `parse_stage`（854 行）。解析失败返回 `Parse error:`，SQLSTATE 为 `None`（858 行），不套 `statement k of n`。ATTACH 与 BEGIN / COMMIT / ROLLBACK 不进入 plan / execute。其余语句用 `plan_stage_routed(..., false, ...)`（964–972 行）；事务内执行是 `execute_stage_in_tx_routed`（990 行，无 profiling 参数）；事务外是 `execute_stage_routed(..., false, ...)`（991 行）。
  - `IndexScanExecutor::next`（`src/executor/index_scan.rs:69-74`）在变量已设置且 `search` 成功返回后调用 `record_time("index_manager_search", ...)`。`IndexScanAllExecutor::next`（`src/executor/index_scan_all.rs:86-90`）对应 `index_manager_search_all`。两次都在 `next` 里、`await` 之后，与 `run_sql` 同一个任务。`search` 返回错误时还没调用 `record_time`。
  - 整数主键简单等值由 `src/parser/planner/query.rs:2266-2290` 建成 `IndexScan`。二级索引等值由 2302–2312 行建成 `IndexScanAll`。无 WHERE 的 `SELECT COUNT(*)` 不走这两处。
  - `PreparedStatement::execute`（`src/database.rs:1074-1082`）把 `is_profiling_enabled()` 传给 `execute_stage`，不建 scope。`execute_in_tx`（1086–1100 行）调用 `execute_stage_in_tx`。主命令不经过这里。
  - import 的建表与插入（`src/cli/lifecycle.rs:840`、891 行）走 `execute_sql`，已经有旧表。诊断摘要在 `execute_inner`（1425 行起）渲染，前缀是 `diagnostics `。主命令不进入 `execute_inner`。
  - 重复主键的执行期文案是 `Execution error: Duplicate key`（`execute_executor`，1827–1830 行），无 SQLSTATE。不存在的表是 `Plan error: Table not found: ...`，SQLSTATE `42P01`。JSON 错误对象由 `emit_error_status` 在 `run_sql` 返回之后写入 stderr。
  - 现有 CLI 子进程先例：`tests/cli_test.rs` 的 `spawn_cli`（独立 TempDir、`RTSQL_HOME`、移除 `RTSQL_KEY`、预建 `db/`、60 秒超时），`tests/attach_session_test.rs` 的 `ATTACH 'other' AS o` 在两个库都 `new` 之后退出码 0，`tests/diagnostics_statement_test.rs` 在子进程上设置或移除 `RTSQL_PROFILING`，并用 `Time (µs)` 识别旧表。
- Code and Critical Path: 主命令闭包 → `run_sql` → `parse_stage` → 每条普通语句 `plan_stage_routed` → `execute_stage_routed` 或 `execute_stage_in_tx_routed` → 索引执行器 `next` → `record_time`。旧表的数据在 task-local map 里，由 `print_timings` 写出。变量未设置时，执行器不调用 `record_time`，主命令也不打表。

**Implementation Guidance**

先写测试，确认 RED 来自新行为，再改 `run_sql`。scope 包住该语句 plan 与 execute 的 `.await`，不要 `spawn`。`index_manager_search` 是 `index_manager_search_all` 的前缀，主键点查的断言要同时排除更长的名字。准备步骤不要设置变量。只对改过的 Rust 文件跑 `rustfmt`。

**Task Definitions**

### T1: 主命令在开关打开时返回结果或既有错误，并输出旧三段表

- Requirement/Scenario: 「三段顶层计时观测」的主命令场景与「导入路径仍输出三段表」。proposal 场景 S1–S11。
- Depends on: None
- Targets: `src/cli/mod.rs::run_sql`；新增 `tests/cli_main_profiling_test.rs`
- Current behavior: 变量已设置时，主键简单等值与二级索引等值在 `record_time` 处中止进程，stdout 没有查询结果。没有索引点查的语句能结束，stderr 没有旧表。变量未设置时，同样的主键等值退出码 0、有结果、没有旧表。解析失败、规划失败、单独的 ATTACH 今天也不打表。import 在变量已设置时已经打表。
- Required behavior:
  1. 在 `run_sql` 开头读取一次 `is_profiling_enabled()`。变量未设置时：不调用 `Instant::now`，不进入 `with_profiling_scope`，不调用 `print_timings`；`plan_stage_routed` 与事务外的 `execute_stage_routed` 仍传入 `false`。
  2. 变量已设置时，整段 SQL 仍只调用一次 `parse_stage`，并用 `Instant` 记下这次解析的耗时。解析失败时返回现有的 `Parse error:` 与 `None` SQLSTATE，不建 scope，不打表。
  3. ATTACH 与 BEGIN / COMMIT / ROLLBACK 保持现在的分派，不建 scope，不打表。
  4. 其余语句在变量已设置时进入一个新的 `with_profiling_scope`，scope 包住该语句的 plan 与 execute。`plan_stage_routed` 传入 `true`。事务外的 `execute_stage_routed` 传入 `true`。事务内仍调用现有的 `execute_stage_in_tx_routed`，不改签名、不加参数。
  5. plan 返回 `Err` 时，不调用 `print_timings`，仍走现有的 `sql_failure_status`，错误文本和 SQLSTATE 不变。
  6. execute 返回之后，无论 `Ok` 还是 `Err`，在同一个 scope 里先 `record_time`：`parse` 用第 2 步那一次解析的耗时，`plan` 用该语句 `plan_stage_routed` 的耗时，`execute` 用该语句 execute 调用的耗时。然后 `print_timings`。传给它的 `Total` = 解析耗时 + 从该语句 `plan_stage_routed` 调用前到 execute 返回的墙钟。表的列、排序和百分比仍完全由现有的 `print_timings` 产生。然后再走现有的 stdout 渲染或 `sql_failure_status`。
  7. 每条这样的语句各建一个 scope。下一条语句的表里不出现只有上一条才会记下的子指标。子指标的名字仍由现有的 `record_time` 调用写入，主命令不新增指标名。
  8. 不调用 `execute_with_params` 或 `execute_inner`，不打开诊断语句 scope，不写 `diagnostics ` 前缀。
- Preserve:
  - 未设置变量时的 stdout、stderr、退出码、错误文本和 SQLSTATE。
  - 变量已设置时，既有 SQL 错误的文本和 SQLSTATE。解析失败仍然没有 SQLSTATE。`Execution error: Duplicate key` 仍然没有 SQLSTATE。`Table not found` 仍然是 `42P01`。
  - `src/profiling.rs`、`src/pipeline.rs`、`src/database.rs`、`src/executor/index_scan.rs`、`src/executor/index_scan_all.rs`、`src/cli/lifecycle.rs`、`src/diagnostics.rs` 的行为。
  - `execute_stage_in_tx_routed` 的签名和库调用方。
  - import 的旧表，以及默认诊断配置下不新写诊断 stderr。
  - 成功的 `BEGIN` / `COMMIT` 不在 stderr 留下 `uncommitted transaction was rolled back at exit`。
- Forbidden:
  - 修改 `record_time`，使它在没有 scope 时不再 panic。
  - 修改 `PreparedStatement::execute` 或 `execute_in_tx`，或为它们建立 scope。
  - 把主命令改道到 `execute_with_params`。
  - 为了打出子指标去改两个索引执行器或计划器。
  - 给 `execute_stage_in_tx_routed` 增加 profiling 参数。
  - 改 `openspec/specs/` 权威正文，改 SNAPSHOT、tasks、issues。
  - 新增操作标识、运行标识、内容哈希、判定层、专用 CLI 或审计器。
  - 编辑 `tests/diagnostics_statement_test.rs`。本 Cycle 跑全量。
- First test: 新增 `tests/cli_main_profiling_test.rs`，`#![cfg(feature = "cli")]`。文件内不打开进程内 `Database`。每个子进程：`CARGO_BIN_EXE_rtsql`，工作目录和 `RTSQL_HOME` 都是独立 TempDir，TempDir 下先建 `db/`，移除 `RTSQL_KEY` 和 `RTSQL_BUSY_TIMEOUT`；观察步骤按用例设置 `RTSQL_PROFILING=1` 或显式 `env_remove`。60 秒未退出则杀掉进程并让测试失败。SQL 观察步骤带 `--format json`。准备步骤（`new`、建表、插入、建索引、`new other`）不设置该变量。

  库名用裸名 `app`。表：`CREATE TABLE t (id INT PRIMARY KEY, b INT)`，再 `INSERT INTO t VALUES (1, 10)`。二级索引用例再 `CREATE INDEX idx_b ON t(b)`。

  用例与断言：

  1. `main_command_pk_scan_returns_row_and_legacy_table`：变量已设置，`SELECT * FROM t WHERE id = 1`。退出码 0。stdout JSON 的行含 `1` 和 `10`。stderr 含 `Time (µs)`、`parse`、`plan`、`execute`、`Total`、`index_manager_search`，不含 `index_manager_search_all`，不含 `diagnostics `，不含 `panicked`。
  2. `main_command_secondary_scan_returns_row_and_legacy_table`：变量已设置，`SELECT * FROM t WHERE b = 10`。退出码 0。stdout 的行含 `1` 和 `10`。stderr 含 `Time (µs)`、`parse`、`plan`、`execute`、`index_manager_search_all`。
  3. `main_command_count_without_index_lookup_prints_legacy_table`：变量已设置，`SELECT COUNT(*) FROM t`。退出码 0。stdout JSON 的 `rows` 含 `1`。stderr 含 `Time (µs)`、`parse`、`plan`、`execute`、`Total`。
  4. `main_command_without_profiling_omits_legacy_table`：变量已移除，`SELECT * FROM t WHERE id = 1`。退出码 0。stdout 的行含 `1` 和 `10`。stderr 不含 `Time (µs)`。
  5. `main_command_parse_error_omits_legacy_table`：变量已设置，`SELEC 1`。退出码 3。stderr 含 `Parse error:`，不含 `sqlstate`，不含 `Time (µs)`。
  6. `main_command_missing_table_omits_legacy_table`：变量已设置，`SELECT * FROM missing WHERE id = 1`。退出码 3。stderr 含 `Table not found` 与 `42P01`，不含 `Time (µs)`。
  7. `main_command_duplicate_key_keeps_error_and_prints_table`：先在变量未设置时插入 `(1, 10)`，再在变量已设置时执行 `INSERT INTO t VALUES (1, 10)`。退出码 3。stdout 为空。stderr 含 `Execution error: Duplicate key` 与 `Time (µs)`。含 `"error"` 的那一行不含 `sqlstate`。
  8. `main_command_transaction_pk_scan_prints_one_table`：变量已设置，一次执行 `BEGIN; SELECT * FROM t WHERE id = 1; COMMIT`。退出码 0。stdout 含该行的 `1`。stderr 里 `Time (µs)` 恰好出现一次，该表含 `parse`、`plan`、`execute`、`index_manager_search`，不含 `index_manager_search_all`，不含 `uncommitted transaction was rolled back at exit`。
  9. `main_command_two_statements_keep_separate_tables`：变量已设置，一次执行 `SELECT * FROM t WHERE id = 1; SELECT COUNT(*) FROM t`。退出码 0。stderr 里 `Time (µs)` 恰好两次。按出现次序切成两段：两段都含 `parse`、`plan`、`execute`；第二段不含 `index_manager_search`。
  10. `main_command_attach_only_omits_legacy_table`：`new app` 与 `new other` 都不设置变量。变量已设置时只执行 `ATTACH 'other' AS o`。退出码 0。stderr 不含 `Time (µs)`。

  RED：改 `run_sql` 之前运行下面的新文件命令，命令失败。失败用例应是 1、2、3、7、8、9。1、2、8、9 今天在索引扫描的 `record_time` 处中止（退出码 101，stderr 含 task-local 未设置）。3 今天退出码 0，但 stderr 没有 `Time (µs)`。7 今天退出码 3，stderr 有 `Execution error: Duplicate key`，同样没有 `Time (µs)`。4、5、6、10 在改之前就通过。`rtsql new`、建表或插入若失败，那是夹具错误，不是这个 RED。

  同一时刻、在改产品代码之前，运行下面的 `diagnostics_statement_test` 命令，预期退出码 0、failed 为 0。这是本工作区的当前基线，本计划没有代跑。
- GREEN condition: 改完并 `rustfmt` 之后，下面两条命令都是退出码 0、failed 为 0。用例 1 的 stdout 仍含 `1` 和 `10`，stderr 含 `index_manager_search` 且不含 `index_manager_search_all`。用例 7 仍含 `Execution error: Duplicate key`，且含 `"error"` 的那一行仍没有 `sqlstate`。用例 4 的 stderr 仍没有 `Time (µs)`。
- Verification: 两条命令都带 `--test-threads=1`。这是定向测试，不是全量。新文件失败表示主命令的结局或旧表不对。诊断文件失败表示 import 旧表或诊断用例被碰到了；`run_sql` 的改动解释不了它时，停止并回到计划。

  ```text
  cargo test --offline --test cli_main_profiling_test -- --test-threads=1
  cargo test --offline --test diagnostics_statement_test -- --test-threads=1
  ```
- Stop when:
  - 不修改 `record_time` 的 scope 外行为，或不动 `PreparedStatement`，就无法让主命令的索引扫描返回结果。
  - 不把主命令改道到 `execute_with_params`，就无法打出含 parse、plan、execute 的表。
  - 主键等值那张表没有 `index_manager_search`，或在排除 `index_manager_search_all` 之后仍然没有这个更短的名字。二级等值那张表没有 `index_manager_search_all`。
  - `SELECT COUNT(*)` 那张表出现 `index_manager_search`。这时不要改计划器来消掉它。
  - 改完后，用例 5、6、7 的错误文本或 SQLSTATE 与改前不同。
  - 未设置变量的主键等值查询打出了 `Time (µs)`，或退出码不是 0。
  - `record_time` 实际跑在另一个 task 上，`run_sql` 里的 scope 盖不住。

**Invariants**

- 变量未设置时，主命令不打旧表，SQL 结果、错误文本、SQLSTATE 和退出码不变。
- 变量已设置时，这些字段也不变；stderr 对进入执行阶段的语句多出一张现有格式的旧表，进程不再中止。
- 旧表仍由 `print_timings` 渲染。主命令不新增另一套表，不新增 `eprintln!` 出口。
- import 的打表时机和统一诊断的默认 stderr 不变。
- 预编译语句的可观察行为不变，因为本 Cycle 不改它的代码。
- 不引入身份证明机制或判定层。

**Non-goals**

- T2 全量。
- 预编译语句。
- 改 `execute_stage_in_tx_routed` 的签名来补 `executor_creation` / `executor_execution`。事务内的表有 parse、plan、execute 即可；索引子指标仍由执行器写入。
- 批量写入、GC、新的 profiling API。

**Acceptance**

1. 主键等值（S1）：变量已设置时退出码 0，stdout 有该行，stderr 有三段表和 `index_manager_search`，没有 `index_manager_search_all`，没有 `diagnostics `。测试 `main_command_pk_scan_returns_row_and_legacy_table`。
2. 二级等值（S2）：退出码 0，stdout 有该行，表含 `index_manager_search_all`。测试 `main_command_secondary_scan_returns_row_and_legacy_table`。
3. 无索引点查（S3）：`SELECT COUNT(*)` 退出码 0，结果仍在，stderr 有三段表。测试 `main_command_count_without_index_lookup_prints_legacy_table`。
4. 变量未设置（S4）：主键等值退出码 0，有该行，没有 `Time (µs)`。测试 `main_command_without_profiling_omits_legacy_table`。
5. 解析失败（S5）：退出码 3，含 `Parse error:`，无 SQLSTATE，无表。测试 `main_command_parse_error_omits_legacy_table`。
6. 规划失败（S6）：退出码 3，含 `Table not found` 与 `42P01`，无表。测试 `main_command_missing_table_omits_legacy_table`。
7. 执行失败（S7）：退出码 3，含 `Execution error: Duplicate key`，错误行无 sqlstate，同时有表。测试 `main_command_duplicate_key_keeps_error_and_prints_table`。
8. 显式事务（S8）：退出码 0，有该行，恰好一张表，且不含未提交回滚提示。测试 `main_command_transaction_pk_scan_prints_one_table`。
9. 两句隔离（S9）：恰好两张表，后一张不含 `index_manager_search`。测试 `main_command_two_statements_keep_separate_tables`。
10. 只 ATTACH（S10）：退出码 0，无表。测试 `main_command_attach_only_omits_legacy_table`。
11. import（S11，以及原场景「profiling 输出三段耗时」）：`diagnostics_statement_test` 整文件通过，其中 `legacy_profiling_table_follows_env_var` 仍看到 parse、plan、execute。本 Cycle 不改这个文件。

**Verification**

| 场景 | 命令 | 通过依据 |
|---|---|---|
| S1–S10 | `cargo test --offline --test cli_main_profiling_test -- --test-threads=1` | 退出码 0，failed 为 0。断言看子进程的退出码、stdout JSON 和 stderr 原文 |
| S11 与 import 旧表的失败路径 | `cargo test --offline --test diagnostics_statement_test -- --test-threads=1` | 退出码 0，failed 为 0 |

两条都在改完 `run_sql` 之后各跑一次。改之前的新文件失败和诊断文件通过，用来确认 RED 和基线，不代替改后的通过。不跑无过滤的 `cargo test`，不用默认并行度跑全量。

**Gate 2 Readiness**

Gate 2: PASS

**Persisted Evidence**

- Mode: none

命令、退出码和 failed 计数写入 Act Response 即可。定向测试可以在同一工作区重跑。没有哪一项 Acceptance 必须靠留下来的日志才能判断。

**Risks and Notes**

- 规划时 `ls tests/*.rs` 是 162 个文件。T1 会增加一个。这个数字不进入 T2 的通过条件；T2 执行时重新枚举。
- 父进程若已设置 `RTSQL_PROFILING`，子进程必须显式设置或移除，不能只是不写 `.env`。
- 不断言微秒数或行的排序。`print_timings` 按耗时降序，耗时不稳定。
- `SELECT COUNT(*)` 的列标题不断言，只断言 `rows` 里有 `1`。
- ATTACH、解析失败、规划失败和未设置变量这四条，在改 `run_sql` 之前就应该通过。它们用来守住「不要整段函数打一张表」。

## Act Response

## Plan Review

- Review Result: pending
