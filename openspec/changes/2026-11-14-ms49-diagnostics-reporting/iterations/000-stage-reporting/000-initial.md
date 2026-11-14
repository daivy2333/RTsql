# Iteration 000 / Cycle 000: 五个旧子指标进入阶段树

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 提案里已经锁定的上报口径。五个旧子指标用现有 `record_stage` 挂到当前语句阶段树；页读继续走 `record_page_read`；不新增 `diagnostics::report`；不用 `record_pre_measured` 另起摘要。
- Excluded scope: T3 的首行等待、交回行数、`segment_metrics`、`process_memory_peak_bytes`；T4 的全量；显式事务路径上的新测量；全局 tasks、SNAPSHOT、M/R/I 和 `openspec/specs/` 正文。

**Objective**

性能统计打开时，五段旧耗时出现在该语句的诊断摘要里。普通插入的执行独占加上 `executor_creation`、`executor_execution` 和 commit 的包含耗时，等于执行的包含耗时。页读和提交等待仍归当时开着的阶段。旧表仍打印这些阶段名。SQL 结果、错误文本和 SQLSTATE 不变。

**Background**

MS49 只补观察。解析、规划、执行、提交等待保持现在的 open/close。五个名字今天只在 `RTSQL_PROFILING` 的旧表里。比较指标不在本轮。

**Investigation Facts**

- Starting point: 工作树分支 `changes/2026-11-14-ms49-diagnostics-reporting`。产品代码与 `8779fdb` 相同，本 Cycle 之前没有产品改动。变更前不另跑全量。定向测试在改断言之前的现状由下面的代码路径说明：五段只有 `record_time`，没有 `record_stage`。
- Current-State Evidence:
  - 入口：`Database::execute_sql` → `pipeline::execute` → `execute_with_params`。采集打开时用 `with_statement_stages` 包住 `execute_inner`。`execute_inner` 调用 `diagnostics_session().enter_statement`。`execute_staged` 在 `profiling || statement_scope.is_some()` 时对 parse / plan / execute 各取一次测量，旧开关才 `record_time`，并 `open_stage` / `close_stage`。
  - `record_stage` 把完成节点挂到最内层 open 阶段，不改 open 栈。没有绑定时是空操作。`stage_instant()` 没有绑定时返回 `None`。
  - `plan_stage_impl` 的三处 `record_time("table_metadata_lookup")`：CTAS 分支、`CreateView` 分支、以及会调用 `register_table` 的 `_` 分支。`CREATE TABLE` / `DROP TABLE` 那个分支不记。调用发生在 `execute_staged` 已经 `open_stage(plan)` 之后、`close_stage(plan)` 之前。
  - `execute_stage_routed` 的 DML 臂和查询臂，以及 `execute_ctas_routed`，各有一处 `executor_creation` 和一处 `executor_execution`。它们发生在 `execute` 已经 open、尚未 close 的期间。DML 的 commit 在 `executor_execution` 记录点之后，由 `Database::commit` `open_stage` / `close_stage`。
  - `IndexScanExecutor::next` 与 `IndexScanAllExecutor::next` 只在 `is_profiling_enabled()` 时测量，并只调用 `record_time`。查找发生在 `execute_executor` 里面，因此当时最内层 open 阶段是 `execute`。
  - `execute_stage_in_tx_routed` 没有这五段的 `record_time`。
  - `StageNode::exclusive` 已是包含耗时减去直接子阶段之和，饱和为 0。`insert_commit_is_child_of_execute` 和 `row_insert_share_of_hundred_inserts` 仍把「执行独占 + 提交包含 = 执行包含」写成断言。
  - 页读：`file_storage` 的读路径使用 `PageReadTimer`，析构时 `record_page_read`。`file_page_read_records_io` 断言摘要页数至少为 1，且 parse 的 Io 为 0。查询语句是 `SELECT id FROM t WHERE id = 3`。
  - 旧表：`main_command_pk_scan_returns_row_and_legacy_table` 使用 `SELECT * FROM t WHERE id = 1`，stderr 含 `index_manager_search`，不含 `index_manager_search_all`。`main_command_secondary_scan_returns_row_and_legacy_table` 在 `CREATE INDEX idx_b ON t(b)` 之后执行 `SELECT * FROM t WHERE b = 10`，stderr 含 `index_manager_search_all`。表结构是 `CREATE TABLE t (id INT PRIMARY KEY, b INT)`，种子行 `(1, 10)`。
  - 不存在 `diagnostics::report`。`Summary` 还没有首行等待字段。本轮不要加。
- Code and Critical Path: 一次未命中缓存的语句依次 open/close parse、plan、execute。plan 开着时记 `table_metadata_lookup`。execute 开着时先记 `executor_creation`，再在 `execute_executor` 里可能记索引查找，返回后记 `executor_execution`，写入语句再在其中 open/close commit。摘要由 `StatementScope::end_ok` / `end_err` 形成。旧表只在执行阶段被触达的返回路径上 `print_timings`。

**Implementation Guidance**

五个调用点使用同一个起点：`profiling` 为真时 `Some(Instant::now())`，否则 `stage_instant()`。得到时长后，仅当 `profiling` 为真才 `record_time`，并调用 `record_stage`。不要为门面再取一个 `Instant`。不要把这五段改成 `open_stage`。

`table_metadata_lookup`、`executor_creation`、`executor_execution` 使用 `Module::Pipeline`。两个索引查找使用 `Module::Index`。阶段名字符串保持现在的字面量。

提前返回、今天还没执行到 `record_time` 的路径，继续不记录。这包括执行器构造失败时、记录点之前就返回的分支。

**Task Definitions**

### T1: 三个流水线子指标进入当前语句摘要

- Requirement/Scenario: 「五个旧子指标进入当前语句阶段树」的插入场景；「语句阶段与父子耗时」里提交仍是执行子阶段、总耗时仍是根包含耗时。
- Depends on: None
- Targets: `src/pipeline.rs` 的 `plan_stage_impl`、`execute_stage_routed`、`execute_ctas_routed`；`tests/diagnostics_statement_test.rs`；`tests/typed_batch_write_test.rs`。
- Current behavior: 这三处只在 `profiling` 为真时 `record_time`。诊断摘要里没有 `table_metadata_lookup`、`executor_creation`、`executor_execution`。普通插入上，执行独占加 commit 的包含耗时等于执行包含耗时。
- Required behavior: 性能统计打开且未设置 `RTSQL_PROFILING` 时，成功的 `INSERT INTO t VALUES (1)` 的摘要含有这三个阶段，类别都是 Pipeline，各出现一次。execute 的独占耗时 + `executor_creation` 的包含耗时 + `executor_execution` 的包含耗时 + commit 的包含耗时 = execute 的包含耗时。commit 的包含耗时不大于 execute 的包含耗时。plan 的独占耗时 + `table_metadata_lookup` 的包含耗时 = plan 的包含耗时。随后 `SELECT` 能看见这一行。语句总耗时大于 0，且等于根阶段包含耗时（摘要的 `total`）。`RTSQL_PROFILING` 打开时，同一次测量仍进入旧表。
- Preserve: parse、plan、execute、commit 的 open/close 调用点和等待归属。旧表的打印时机、表头和 `Total` 行。错误文本与 SQLSTATE。`CREATE TABLE` / `DROP TABLE` 计划分支仍然不记 `table_metadata_lookup`。`execute_stage_in_tx_routed` 仍然没有这三段测量。
- Forbidden: 修改 `src/diagnostics.rs` 的独占公式或 `Summary` 字段；修改 `src/profiling.rs`；新增 `open_stage`；调用 `record_pre_measured` 或 `diagnostics::report`；改 SQL 结果列；跑全量。
- First test: 在 `tests/diagnostics_statement_test.rs` 增加 `insert_records_pipeline_submetrics`。库入口 `Database::execute_sql`，只开 stats。断言上面的阶段名、类别、次数、两条等式，以及随后查询看见插入的行。改产品代码前该测试 RED：摘要里没有这三个阶段名。同时把 `insert_commit_is_child_of_execute` 和 `row_insert_share_of_hundred_inserts` 里的「独占 + commit = 包含」改成三条子阶段的等式。改前这两条因此也失败。
- GREEN condition: `cargo test --offline --test diagnostics_statement_test -- --test-threads=1` 与 `cargo test --offline --test typed_batch_write_test -- --test-threads=1` 都是退出码 0、failed 为 0。
- Verification: 上列两条命令。失败表示三个子指标没有按同一次测量挂进 plan 或 execute，或者插入结果变了。`row_insert_share_of_hundred_inserts` 末尾的 `COUNT(*) = 100` 仍须成立。该测试里的 `println` 不是通过条件。
- Stop when: 不改独占公式、不开新阶段、不改错误文本就无法让等式成立时，停止并回到计划。不要把显式事务路径加进测量来凑阶段名。

### T2: 两个索引子指标进入当前语句摘要

- Requirement/Scenario: 主键等值、二级索引等值、页读仍记在当时开着的阶段、排除 Index、排除 Pipeline、旧表仍包含索引子指标。
- Depends on: T1
- Targets: `src/executor/index_scan.rs` 的 `IndexScanExecutor::next`；`src/executor/index_scan_all.rs` 的 `IndexScanAllExecutor::next`；`tests/diagnostics_statement_test.rs`；`tests/diagnostics_wait_test.rs` 的 `file_page_read_records_io`。
- Current behavior: 两处只在旧开关打开时 `record_time`。诊断摘要没有这两个阶段名。页读记在当时开着的阶段，parse 的 Io 为 0。CLI 在旧开关打开时，主键等值的 stderr 含 `index_manager_search`，二级索引等值含 `index_manager_search_all`，结果行仍在。
- Required behavior: 性能统计打开时，`CREATE TABLE t (id INT PRIMARY KEY, b INT)`、插入 `(1, 10)` 之后：
  - `SELECT * FROM t WHERE id = 1` 的摘要含有 `index_manager_search`（`Module::Index`，一次），不含 `index_manager_search_all`。返回行与诊断关闭时相同。
  - `CREATE INDEX idx_b ON t(b)` 之后，`SELECT * FROM t WHERE b = 10` 的摘要含有 `index_manager_search_all`（`Module::Index`）。返回行与诊断关闭时相同。
  - 观察类别只有 Pipeline 时，主键等值的摘要没有这两个索引名，仍有 execute，结果行相同。
  - 观察类别只有 Index 时，成功插入的摘要没有三个流水线子指标，插入的行可见。
  - `file_page_read_records_io` 在原有页数和 parse Io 断言之外，摘要含有 `index_manager_search`。查询结果仍是 `Value::Int(3)` 那一行。
  - 旧开关下的两个 CLI 用例仍看到原来的 stderr 阶段名和结果行。
- Preserve: `record_page_read` 的归属。commit 的等待类别。T1 的插入等式。索引查找的 SQL 结果。不在查找失败或空结果时多记一次与今天 `record_time` 窗口不同的区间。
- Forbidden: 为了让索引查找变成 `executor_execution` 的子节点而 `open_stage`；改 `IndexManager::search` / `search_all` 的返回值；改旧表格式；改 `diagnostics.rs` 的等待槽；跑全量。
- First test: `tests/diagnostics_statement_test.rs` 的 `primary_key_lookup_records_index_search`。改产品代码前 RED：摘要没有 `index_manager_search`。同文件再增加 `secondary_index_lookup_records_search_all`、`index_stages_hidden_when_module_excludes_index`、`pipeline_submetrics_hidden_when_module_excludes_pipeline`。给 `file_page_read_records_io` 加上索引阶段存在的断言，这一断言在改执行器前 RED，原有页数断言保持。
- GREEN condition: 下列三条命令都是退出码 0、failed 为 0。
- Verification:
  - `cargo test --offline --test diagnostics_statement_test -- --test-threads=1`
  - `cargo test --offline --test diagnostics_wait_test -- --test-threads=1`
  - `cargo test --offline --test cli_main_profiling_test -- --test-threads=1`
  - 改执行器之前，后两条里除新增断言以外的既有用例应为 GREEN。改完后整文件仍为 GREEN。CLI 用例失败表示旧表丢了阶段名或结果行变了。等待用例失败表示页读或提交等待被挪走。
  - `diagnostics_wait_test.rs` 的文件头写着按默认并行度执行。本 Cycle 的命令覆盖那句，三条命令都必须带 `--test-threads=1`。
- Stop when: 要保持页读归属就必须不调用 `record_stage`，或必须新开阶段才能让索引名出现时，停止并回到计划。

**Invariants**

- 诊断只观察。本轮任何路径的 SQL 结果、错误文本、SQLSTATE 与改前相同。
- 五个名字各用一次测量。旧开关打开时旧表仍收到这个时长。
- 不新开阶段，因此 parse、plan、execute、commit 的等待归属不变。
- 页读仍只经 `record_page_read`。
- 语句总耗时仍是根阶段包含耗时，不是子阶段之和。
- 测试调用都带 `--test-threads=1`。本轮不跑全量。

**Non-goals**

首行等待、交回行数、吞吐、P95、进程峰值、`Summary` 新字段、渲染格式串、显式事务路径的新测量、全量。

**Acceptance**

- T1 的插入场景和更新后的父子耗时等式，由 `insert_records_pipeline_submetrics`、`insert_commit_is_child_of_execute`、`row_insert_share_of_hundred_inserts` 覆盖。
- T2 的五个场景由 `primary_key_lookup_records_index_search`、`secondary_index_lookup_records_search_all`、`index_stages_hidden_when_module_excludes_index`、`pipeline_submetrics_hidden_when_module_excludes_pipeline`、`file_page_read_records_io`、`main_command_pk_scan_returns_row_and_legacy_table`、`main_command_secondary_scan_returns_row_and_legacy_table` 覆盖。
- 正确性看 `SqlOutcome`、错误文本和 SQLSTATE。诊断摘要只证明观察结果。stderr 里的诊断文本不是 SQL 对错的判据。

**Verification**

按 T1、T2 的命令顺序执行。每条都是 `cargo test --offline --test <文件> -- --test-threads=1`。通过条件是退出码 0 且 failed 为 0。这些命令走库入口或 CLI 二进制，不是单独的判定脚本。

**Gate 2 Readiness**

Gate 2：PASS。无 BLOCKED。无 SKIPPED。

**Persisted Evidence**

- Mode: none

定向测试可以按上面的命令重跑。本轮不需要把输出存成文件。

**Risks and Notes**

- 索引查询上 execute 的独占可能饱和为 0。不要把 T1 的三阶段等式套到带索引查找的查询上。
- `row_insert_share_of_hundred_inserts` 打印的独占纳秒会变小。不要把它改成阈值断言。
- 阶段名变多后，诊断渲染会多几行阶段文本。不要新增「stderr 含某阶段名则 SQL 正确」这类断言。
- 辅助函数抽不抽取由实施时决定，只要每个调用点仍然只有一个起点。

## Act Response

Act 填写。本段在实施前没有结果。

## Plan Review

- Review Result: pending
