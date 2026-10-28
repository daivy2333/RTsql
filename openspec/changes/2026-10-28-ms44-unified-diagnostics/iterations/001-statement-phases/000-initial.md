# Iteration 001 / Cycle 000: 语句阶段

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS44 采集口径中「语句阶段与父子耗时」的真实管道部分、「成功失败取消都结束记录」、「旧计时出口保持」。行为以本文件的 Task Definition 为准。
- Excluded scope: 等待类别、并发归属、文件页 I/O、三种开销测量（T3）；全量回归（T4）；告警迁移；子指标接入诊断；`execute_in_tx` / 预编译语句路径的阶段摘要。

**Objective**

经 `Database::execute_sql` 执行的语句在采集打开时产出带真实 `parse` / `plan` / `execute` 阶段的摘要；隐式 DML 的 `commit` 是 `execute` 的子阶段；失败与被放弃的语句各自结束记录；`RTSQL_PROFILING` 旧表行为不变；SQL 结果、SQLSTATE 与取消后行可见性不变。

**Background**

Iteration 000 已 accepted（`iterations/000-diagnostics-facade/000-initial.md` 的 Plan Review，Review Result `accepted`）：门面、会话槽、三项配置、双上限与父子耗时口径已交付并验证（GREEN 10/10）。本轮把真实语句管道接到该门面上。

**Investigation Facts**

- Starting point: HEAD 1081a5f。000 最终 Act Response 的改动为 `src/diagnostics.rs`（门面）、`src/database.rs`（私有 `diagnostics: Arc<Diagnostics>` 槽 + `set_diagnostics` / `diagnostic_summaries`）、`src/lib.rs` 导出、`tests/diagnostics_facade_test.rs`；验证结论 `cargo test --offline --test diagnostics_facade_test` 退出码 0、10 passed / 0 failed，覆盖未变化，直接沿用。门面现有公开面：`DiagnosticsConfig`（默认 event_limit 64、summary_limit 32、collect 全关）、`StatementScope`（`end_ok` / `end_err` / 析构未结束按 Dropped）、`record_pre_measured`、`StageSample` 树、stats/trace 聚合（独占 = 包含 − 直接子阶段包含之和）。
- Current-State Evidence:
  - `src/pipeline.rs:115` `execute_with_params`：`is_profiling_enabled()` 为真时以 `with_profiling_scope` 包住 `execute_inner`，否则直调；无参入口 `execute`（`Database::execute_sql` 的实现体，`src/database.rs:541` 委托 `pipeline::execute`）与参数化入口都经过它——绑定点单一。
  - `src/pipeline.rs:1411` `execute_inner`：缓存查找（`cache_hit_check` 计时）→ 命中则直接 `execute_stage`（无 parse/plan）→ 未命中 `parse_stage` → 单语句与占位符校验 → `plan_stage` → 参数物化 → `execute_stage`；每段只在 profiling 为真时取 `Instant` 并 `record_time`，结束 `print_timings(total)`。parse 失败返回 `SqlError { message: "Parse error: {msg}", sqlstate: Some("42601") }`；plan 失败返回 `PlanFailure` 的文本与 SQLSTATE。环境变量未设置时 `record_time` 不可调用（`src/profiling.rs:21` 的 task-local `with` 在 scope 外 panic）。
  - `plan_stage` / `execute_stage` 的 `profiling: bool` 只控制子指标计时（`table_metadata_lookup`、`executor_creation`、`executor_execution`）。
  - DML 臂（`src/pipeline.rs:545` 起）：先 `reclaim_abandoned_implicit_transactions`，再 begin + `implicit_tx_guard`，执行后成功臂收敛到 `target_db.commit(tx)`（`src/pipeline.rs:660`）→ `Database::commit`（`src/database.rs:578`：`TransactionManager::commit` 后 `maybe_auto_checkpoint`）；失败臂 abort。CTAS 的提交走 `TransactionManager::commit` 直调（`src/pipeline.rs:1090`），不经 `Database::commit`。
  - 显式事务 COMMIT 由 session/CLI 调 `db.commit(tx)`（`src/transaction/session.rs:67`、`src/cli/mod.rs:933`），不在语句管道任务内。
  - `execute_in_tx`（`src/pipeline.rs:1184`）与预编译语句路径（`src/database.rs:1054`/`1073` 直调 `execute_stage*`）不经 `execute_inner`。
  - 放弃 future 的确定性锚点先例：`tests/statement_cancellation_test.rs` 的 `anchor_tx`（`db.transaction_manager.active_transactions()` + `get_tx_versions(tx_id).len() >= 2`，均为公开 API）配 `tokio::select! { biased; … }` 丢弃语句 future；下一条写语句的 DML 臂开头触发回收。
  - stderr 断言先例：`tests/recovery_bounded_read_test.rs` 用 `env!("CARGO_BIN_EXE_rtsql")` 子进程 + `Stdio::piped()` + `RTSQL_HOME`（隔离集中存储）+ `env_remove`（键环境清理）断言子进程 stderr 文本；`cli` 在 default features 内，`cargo test --offline --test <名>` 可用该宏。CLI 主命令形态为 `rtsql <db路径> "<sql>"`。
  - 测试目录没有既有 `RTSQL_PROFILING` 用例；旧表断言是本轮新测试。
  - `SqlOutcome` 为 `Rows { columns, rows }` / `AffectedRows(u64)`（`src/result.rs:25`）。
- Code and Critical Path: 修改 `src/diagnostics.rs`（阶段上报与任务内绑定扩展，模块仍不得依赖 database/pipeline/wal/storage；`tokio` 为既有依赖）、`src/pipeline.rs`（`execute_with_params` 绑定、`execute_inner` 根作用域与阶段上报、Result 确定后的渲染写出）、`src/database.rs`（`Database::commit` 上报 `commit` 子阶段）、新建 `tests/diagnostics_statement_test.rs`。`src/profiling.rs` 零改动。

**Implementation Guidance**

- 阶段归属沿用 000 固定树的先例：`parse` / `plan` / `execute` / `cache_hit_check` 记 `Module::Pipeline`，`commit` 记 `Module::Wal`。
- 每个阶段只测一次：`execute_inner` 现有 `Instant` 模式扩展为「旧开关或会话采集打开即取」，同一次测量同时喂 `record_time`（仅旧开关）与诊断阶段上报；旧开关关闭时不调用 `record_time`。
- 当前语句经任务内绑定（`tokio::task_local`，`profiling.rs` 同款机制）对深层代码可见：`Database::commit` 在绑定存在时把 `commit` 阶段记为当前最内阶段的子阶段（隐式 DML 提交时最内阶段是 `execute`）；无绑定的任务（显式 COMMIT 的 session/CLI 调用、checkpoint 触发点）为 no-op，不创建根、不产生摘要。
- 根作用域生命周期：`execute_inner` 进入时开根（含 SQL 文本），`Ok` 结束 `end_ok`、`Err` 结束 `end_err`，语句 future 被放弃时随 future 析构按 Dropped 记录——沿用 000 `StatementScope` 的三态结束语义。
- 渲染写出（design D2）：Result 确定后、返回调用方前，对本语句摘要按会话 `Detail` 渲染，非空则写一次 stderr。默认配置（采集全关）无摘要、`Detail::Off` 渲染为空，均结构性不写。诊断投递在语句执行过程中不写 stderr。
- 缓存命中路径摘要记 `cache_hit_check` 与 `execute`；未命中路径记 `parse` / `plan` / `execute`（`cache_hit_check` 在未命中路径不作为诊断阶段，仍只由旧开关记录）——按 design D4。
- 放弃测试的夹具在默认配置下建好后才 `set_diagnostics`，避免 2000 行 setup 刷满摘要上限。

**Task Definitions**

### T2: 语句阶段接入诊断摘要

- Requirement/Scenario: 需求 2（parse/plan/execute、commit 子阶段、总耗时口径、缓存命中不记解析规划）；需求 5（成功、失败、放弃都结束记录，错误文本与 SQLSTATE 不变）；需求 6 的 T2 部分（执行过程中不写 stderr、结果行不含阶段名）；需求 8（旧表保持）。场景 S3b、S4、S5、S6、S11、S12。
- Depends on: T1（Iteration 000 accepted）。
- Targets: `src/diagnostics.rs`（阶段上报与任务内绑定扩展）、`src/pipeline.rs::execute_with_params` / `execute_inner`、`src/database.rs::Database::commit`、`tests/diagnostics_statement_test.rs`（新建）。
- Current behavior: 诊断未接管道——真实语句不产生摘要（000 的 `default_session_execute_has_empty_summaries` 已证明默认空）；阶段计时只存在于 `RTSQL_PROFILING` 打开时的 stderr 旧表；失败与取消无摘要。
- Required behavior:
  - 会话 stats 打开时，经 `Database::execute_sql` 执行的未命中缓存语句，其摘要含 `parse`、`plan`、`execute` 阶段汇总（`Module::Pipeline`），语句 `total` 为根阶段包含耗时。
  - 隐式 DML（INSERT/UPSERT/UPDATE/DELETE 经 `execute_sql`）实际调用 `Database::commit` 时，摘要含 `commit` 阶段（`Module::Wal`）且为 `execute` 的子阶段：单次插入下 `commit.inclusive_total <= execute.inclusive_total`，且 `execute.exclusive_total + commit.inclusive_total == execute.inclusive_total`。
  - 不写数据的查询（SELECT）摘要没有 `commit` 阶段。
  - 同一 SELECT 文本第二次执行（缓存命中）的摘要没有 `parse`、`plan`，有 `cache_hit_check` 与 `execute`。
  - 无法解析的语句：摘要 severity 为 `Error`；其错误文本与 SQLSTATE 与关闭诊断执行同一语句逐字节相同。
  - 语句 future 在提交前被放弃：摘要 severity 为 `Warn`（Dropped）；下一条写语句触发回收后，被放弃语句的行不存在——同一键可再次插入成功，目标表行数只含已完成语句的行。
  - 成功结束的语句摘要 severity 为 `Info`。
  - `RTSQL_PROFILING` 已设置且会话保持默认配置：CLI 真实入口（`CARGO_BIN_EXE_rtsql` 子进程执行 SQL）的 stderr 含 parse、plan、execute 三段计时表；该变量未设置时同一入口 stderr 无该表。
  - Result 确定后按会话 `Detail` 渲染本语句摘要，非空写一次 stderr；默认配置与 `Detail::Off` 不写。
  - 默认配置（采集全关）：`execute_sql` 仍成功且不产生摘要（000 已覆盖，保持）。
  - 关闭诊断插入一行、`Detail::Full` + trace 打开插入另一行后重开库，两行都可见，行集与逐列值一致。
- Preserve: `SqlError` 文本与 SQLSTATE；取消后的行可见性与 MS34 回收语义；`record_time` 在旧 scope 外 panic；`print_timings` 的列与 `Total` 行算法（`src/profiling.rs` 零改动）；`plan_stage` / `execute_stage` 的 `profiling: bool` 参数语义（仍只表示旧开关）；34 个 stderr 调用点原文；`Database::open` / `execute_sql` / `commit` 签名；WAL 刷盘单飞与 `Ok(())` 合同。
- Forbidden: 修改 `src/profiling.rs`、`src/wal/`、`src/transaction/`、`src/storage/`、`src/executor/`；迁移既有告警；新增 crate、CLI 参数或 SQL 语法；给公开记录或摘要加操作标识、运行标识或内容哈希字段；把子指标（`table_metadata_lookup`、`executor_creation`、`executor_execution`、`index_manager_search(_all)`）或等待类别接入诊断；给 `execute_in_tx` / 预编译路径接摘要；跑全量 `cargo test`；修改 `tests/statement_cancellation_test.rs` 与 `tests/recovery_bounded_read_test.rs`。
- First test: `tests/diagnostics_statement_test.rs` 的 `insert_commit_is_child_of_execute`。先只加这个测试。RED 命令 `cargo test --offline --test diagnostics_statement_test insert_commit_is_child_of_execute -- --test-threads=1`，预期编译失败（阶段上报/绑定 API 尚不存在）或断言失败（摘要无阶段）。看到该失败后再写实现。
- GREEN condition: `cargo test --offline --test diagnostics_statement_test -- --test-threads=1` 退出码 0 且 failed 为 0，文件内测试为：`insert_commit_is_child_of_execute`、`query_has_no_commit_stage`、`cache_hit_skips_parse_and_plan`、`parse_error_keeps_error_text_and_summary_fails`、`abandoned_future_summary_is_cancelled`、`legacy_profiling_table_follows_env_var`、`inserts_visible_with_and_without_diagnostics`。
- Verification: 上一条 GREEN 命令，批内单线程（文件内用例经子进程与夹具操纵进程级环境，沿用已批准 Iteration Plan 的单线程纪律）。`legacy_profiling_table_follows_env_var` 为 CLI 子进程真实入口：沿用 `tests/recovery_bounded_read_test.rs` 的 `spawn_cli` 形态（`CARGO_BIN_EXE_rtsql`、`RTSQL_HOME` 指向临时目录、`env_remove` 清键环境；`RTSQL_PROFILING` 只在子进程环境设置/移除），spawn 前释放进程内 `Database` 句柄。`abandoned_future_summary_is_cancelled` 沿用 `tests/statement_cancellation_test.rs` 的锚点技术（`t` 预置 2000 行、被放弃语句 `INSERT INTO t2 SELECT id FROM t`、`anchor_tx` 以活跃事务 + 版本数 ≥ 2 判定、`tokio::select! { biased; }` 丢弃），可见性判据用同一 oracle：回收后 `INSERT INTO t2 VALUES (1)` 成功且 `SELECT COUNT(*) FROM t2` 只含完成语句的行。其余测试断言阶段汇总的精确关系（见 Required behavior）。
- Stop when: 接入阶段必须改变 `SqlError` 文本、SQLSTATE 或取消后行可见性才能完成；或必须修改 `src/profiling.rs` / `wal` / `transaction` / `storage` / `executor` 才能记录阶段或渲染；或必须给公开记录加标识字段；或同一阶段无法用一次测量同时喂旧表与新摘要。

**Invariants**

- SQL 结果、SQLSTATE、取消后的行可见性与回收语义不变。
- `record_time` 的 panic 条件与 `print_timings` 的算法不变；旧开关关闭时不调用 `record_time` / `print_timings`。
- `src/diagnostics.rs` 不 import `database`、`pipeline`、`wal`、`storage`、`executor`（M21）。
- 默认配置不产生摘要、不写新 stderr；诊断投递在语句执行过程中不写 stderr；渲染写出只在 Result 确定后。
- 公开记录与摘要没有操作标识、运行标识或内容哈希字段。
- 每个阶段只测一次：同一 `Instant` 的耗时喂旧表与新摘要两处。
- 采集全关时不产生摘要（沿用 000 单点判定）。

**Non-goals**

- 等待类别（Lock/Queue/Io/Schedule）、并发语句归属、文件页 I/O 记录、三种开销测量（Iteration 002）。
- 子指标接入诊断摘要：无 requirement / scenario 映射，仍只由旧开关记录。
- `execute_in_tx` 与预编译语句路径的阶段摘要：targets 不含，无 scenario。
- 既有告警迁移、CLI / SQL 界面变化、全量回归。

**Acceptance**

- `insert_commit_is_child_of_execute` 证明 parse/plan/execute/commit 接入与父子耗时关系（需求 2、S3b）。
- `query_has_no_commit_stage` 证明不写数据的查询没有 commit（S11）。
- `cache_hit_skips_parse_and_plan` 证明缓存命中不记 parse/plan、记 cache_hit_check/execute（需求 2、S1）。
- `parse_error_keeps_error_text_and_summary_fails` 证明失败结束记录且错误文本/SQLSTATE 不变（需求 5、S5）。
- `abandoned_future_summary_is_cancelled` 证明取消结束记录且行可见性 oracle 不变（需求 5、S6）。
- `legacy_profiling_table_follows_env_var` 证明旧表仍由 `RTSQL_PROFILING` 决定、默认会话不关它（需求 8、S4）。
- `inserts_visible_with_and_without_diagnostics` 证明诊断不改变事务结果、重开后行集一致（S12）。
- Requirement Traceability：需求 2 → T2（`insert_commit_is_child_of_execute`、`cache_hit_skips_parse_and_plan`）；需求 5 → T2（`parse_error_keeps_error_text_and_summary_fails`、`abandoned_future_summary_is_cancelled`）；需求 8 → T2（`legacy_profiling_table_follows_env_var`）；需求 6 T2 部分 → T2（`inserts_visible_with_and_without_diagnostics` + 结构性保证）；均 Covered。

**Verification**

本轮场景经真实入口：语句行为经 `Database::open` + `execute_sql`（tempdir 文件库），旧表面经 CLI 子进程 stderr。判定只用阶段汇总的数值关系、severity、错误文本、SQLSTATE、行集与子进程 stderr 文本；不解析进程自身 stderr，不使用诊断字段作为运行身份。命令是 `cargo test --offline --test diagnostics_statement_test -- --test-threads=1`，退出码 0 且 failed 为 0 为通过。000 的 GREEN（`cargo test --offline --test diagnostics_facade_test`）覆盖未变化，沿用。

**Gate 2 Readiness**

PASS。检查项均通过，无 BLOCKED，无 SKIPPED。调查基于本会话对 pipeline/profiling/database/测试先例的实际阅读；设计闭合（阶段集合、模块归属、缓存命中口径、生命周期、渲染时机已定）；RTM 无 Missing、无 Simplified；Persisted Evidence 为 none；批准来自本次 goal 对「把这个 change 做完」的授权。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 放弃测试沿用 MS34 的 2000 行夹具：锚点确定性已被既有测试验证，setup 成本可接受；夹具建好后才打开采集。
- CLI 子进程用例须在 spawn 前释放进程内 `Database` 句柄（advisory 锁等待），并沿用 recovery 测试的 `RTSQL_HOME` / `env_remove` 形态。
- 渲染后写出（design D2）在默认配置与 `Detail::Off` 下结构性无输出，无专用断言，属代码评审覆盖项。
- 旧开关与新采集同时打开时 stderr 有两段输出（旧表 + `diagnostics ` 前缀新文本），测试按内容区分，不混合断言（design 风险条目）。

## Act Response

- Status: 尚未由 Act 填写

尚未执行。

## Plan Review

- Review Result: pending
