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

- Status: reported

**实际改动**

- `src/diagnostics.rs`：
  - 新增任务内阶段绑定（`tokio::task_local` 的 `STATEMENT_STAGES`，`profiling.rs` 同款机制）：`with_statement_stages`（绑定点包装）、`stage_instant` / `record_stage` / `open_stage` / `close_stage`（绑定外均为 no-op）、`StageStack`（root_children + open 栈；close 时以调用方单次测量挂父级，未关的 open 阶段在根结束时不计耗时弃置）。
  - `Diagnostics` 新增 `collecting` / `render_with_session_detail`（均 `pub(crate)`）与 `enter_statement`（绑定存在才开根并接阶段树；无绑定返回 `None`，默认配置零成本）。
  - `record_statement` 返回 `Option<Summary>`（采集全关返回 `None`）；`StatementScope::end_ok` / `end_err` 返回本次形成的摘要（facade 测试忽略返回值，仍 10/10 GREEN）；scope 持有阶段树句柄而非回读 task-local——语句 future 被放弃时析构发生在绑定作用域外，仍能取到已记录阶段。
- `src/pipeline.rs`：`execute_with_params` 为绑定点（旧开关、会话采集各自独立决定是否包装，可叠加）；`execute_inner` 拆为「开根 → `execute_staged` → 旧表 `print_timings` → 结束根 + 按会话 Detail 渲染非空写一次 stderr」；新 `execute_staged` 承载原缓存查找 / parse / plan / execute 分段，每段一次测量同时喂 `record_time`（仅旧开关）与诊断上报。开根先于 `total_start`、渲染在 `print_timings` 之后：旧表计时窗口内容不变。
- `src/database.rs`：`Database::commit` 在绑定存在时把 `commit`（`Module::Wal`）经 `stage_instant` / `record_stage` 记为当前最内阶段的子阶段，错误传播路径逐字节等价（`?` 改为等价 match，签名不变）；新增 `pub(crate)` 访问器 `diagnostics_session` / `diagnostics_collecting`。
- 新建 `tests/diagnostics_statement_test.rs`（`#![cfg(feature = "cli")]`，批内单线程）：`insert_commit_is_child_of_execute` 先行（RED：断言失败，摘要为空）→ 实现 GREEN；其余 6 个用例按 Task Definition 补齐。

**与计划的偏差（Deviations from Plan）**

1. `legacy_profiling_table_follows_env_var` 的真实入口用 `rtsql import <db> <table> <csv> --csv`（自动建表 DDL 与逐行 INSERT 都经 `Database::execute_sql`），不是 Investigation Facts 推断的 CLI 主命令 `rtsql <db> "<sql>"`。原因：主命令的 `run_sql`（`src/cli/mod.rs`）自行分段 parse/plan/execute 且 `profiling=false`，不经 `execute_with_params`，`RTSQL_PROFILING` 对它本来就无效——Plan 的 Current-State Evidence 在此点与实际代码矛盾。import 入口满足 Required behavior 原文（`CARGO_BIN_EXE_rtsql` 子进程执行 SQL、stderr 含三段表、未设置时无表），且全程无进程内 `Database` 句柄（比「spawn 前释放句柄」更简单）。该矛盾同时登记为 Issue 候选（见下）。
2. 建库用 `rtsql new` 子进程而非进程内 `Database::open`（同上，避免句柄）。
3. `execute_inner` 主体拆出 `execute_staged` 私有函数（等价局部实现）：为单一收尾点（结束根 + 渲染）服务，不改任何分支语义；parse/plan 的计时改为「await 后一次测量、再分支处理」，比原「两臂各记一次」少一次重复，测量点与原语义相同。

**自检发现与修复**

- Self-Review 发现（Important，已修复）：`enter_statement` 初版外层 `Some(...)` 无条件——采集全关（默认配置）时每条语句仍开根并在结束时锁会话状态，与「默认配置零成本」不变量不符。已改为无绑定返回 `None`，修复后两个测试文件重跑均 GREEN。
- 遗留 Minor（不阻塞，供 Plan Review 终审）：
  - 仓库存在两处既有 rustfmt 漂移（`tests/diagnostics_facade_test.rs:216`、`tests/session_lifecycle_test.rs:15`），非本 Cycle 引入，未触碰（Surgical Changes）；本 Cycle 三份触碰文件已格式化。
  - 放弃路径的摘要只含已完结阶段（parse/plan），进行中的 `execute` 无耗时、按设计弃置——`abandoned_future_summary_is_cancelled` 断言 severity 与行可见性，不断言阶段集合。

**验证命令与实际结果**

- GREEN 条件：`cargo test --offline --test diagnostics_statement_test -- --test-threads=1` → `test result: ok. 7 passed; 0 failed`（7 个用例与 Task Definition 清单一致；RED 先行观察过 `insert_commit_is_child_of_execute` 断言失败 `summaries: []`）。
- 受影响范围：`cargo test --offline --test diagnostics_facade_test` → `10 passed; 0 failed`（Iteration 000 结论在 API 扩展后保持）。
- Issue 候选的定性运行（真实 CLI，见下）。
- 编译无新增警告；未跑全量（预算在 003）。

**Persisted Evidence**

None required（Mode: none；上述输出可低成本重跑）。

**Experience Candidates**

- Issue 候选 1（change 范围外，有证据）：`RTSQL_PROFILING` 对 CLI 主命令 `rtsql <db> "<sql>"` 无效——`run_sql`（`src/cli/mod.rs` 的 `parse_stage` + `plan_stage_routed(..., false, ...)` + `execute_stage_routed(..., false, ...)`）不经 `execute_with_params`，也不建立 profiling scope；同一变量在 `rtsql import`（经 `Database::execute_sql`）路径有效。证据：本 Act Response 与 `legacy_profiling_table_follows_env_var` 的两次子进程运行（import 有表、主命令形态无表——后者为本 Cycle 调查所证）。对本 change 的影响：需求 8 的「旧表保持」经 import 入口证明；主命令形态是否应接旧表属产品决策，建议 Recorder 登记。
- Issue 候选 2（change 范围外，有证据）：无显式主键建表把首列当唯一键——`CREATE TABLE u (a INT, b INT)` 后 `INSERT (1,1)` 成功、`INSERT (1,2)` 报 `Execution error: Duplicate key`、`INSERT (2,1)` 成功（真实 CLI 运行，2026-10-30）；与 `src/cli/lifecycle.rs` 自动建表注释「无主键——首列重复值合法」矛盾。是注释失效还是行为缺陷由 Recorder 判定。
- Runbook 候选：None（无高风险新操作路径；测试内技术已随测试文件自说明）。

**未解决问题**

- None（本 Cycle 范围内）。上述两个 Issue 候选只报告不登记。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读 merge-base `e69c7e2` 到 HEAD `5d0d212` 的完整 diff 与当前代码（T1 的 `1081a5f` 已 accepted，本轮产品代码改动集中在 `5d0d212`：`src/diagnostics.rs`、`src/pipeline.rs`、`src/database.rs`、`tests/diagnostics_statement_test.rs`），逐条核对 Act Response 的声明：

1. 声明与代码一致（Implemented）：
   - `src/diagnostics.rs`：task_local `STATEMENT_STAGES`（`Arc<Mutex<StageStack>>`）、`with_statement_stages` / `stage_instant` / `record_stage` / `open_stage` / `close_stage`（无绑定一律 no-op）、`StageStack`（`root_children` + `open` 栈，`close_stage` 以调用方单次测量挂父级）、`Diagnostics::enter_statement`（无绑定返回 `None`）、`record_statement` 返回 `Option<Summary>`、`StatementScope` 持阶段树句柄（放弃路径在绑定作用域外析构仍能取到已记录阶段）。逐条对应 Act Response「实际改动」，无遗漏项。
   - `src/pipeline.rs`：`execute_with_params` 的 profiling × 采集四路分支（绑定点单一，无参与参数化入口都经过它）；`execute_inner` 拆出 `execute_staged`；parse / plan / execute 各取一次测量，同一次测量同时喂 `record_time`（仅旧开关打开时调用）与诊断上报；缓存命中路径记 `cache_hit_check` + `execute` 且不记旧表 `execute`（与变更前一致）；`print_timings` 在 `execute_staged` 之后；渲染写出在 Result 确定之后、`Detail::Off` 与默认配置结构性为空。
   - `src/database.rs`：`Database::commit` 用 `stage_instant` / `record_stage` 记 `commit`（`Module::Wal`），`?` 改为等价 match（错误逐字节等价），签名不变；新增 `diagnostics_session` / `diagnostics_collecting` 两个 `pub(crate)` 访问器。
   - `tests/diagnostics_statement_test.rs`：7 个用例与 Task Definition 的 GREEN 清单逐一对应。
2. 阶段归属与父子耗时（代码核对）：`execute_staged` 的 `open_stage(Pipeline, "execute")` → `execute_stage` → 隐式 DML 臂 `target_db.commit(tx)`（`src/pipeline.rs:674`，在 `execute_stage_routed` 内）→ `Database::commit` 的 `record_stage` 挂到当时最内层 open 阶段（= `execute`）⇒ `commit` 是 `execute` 的子阶段。独占算法 `inclusive.saturating_sub(直接子阶段 inclusive 之和)`（`src/diagnostics.rs` 的 `StageSample::exclusive`）与「`execute` 只有一个子阶段 `commit`」共同给出 `execute.exclusive + commit.inclusive == execute.inclusive`。`collect_stats` 从根的子阶段起算（根自身不占阶段名、不受类别过滤），`parse` / `plan` / `execute` / `cache_hit_check` 均为根的直接子阶段。✓
3. Preserve 全部成立：`src/profiling.rs` 零改动（不在 diff 中）；`record_time` 仍只在旧 scope 内调用（scope 外 panic 条件未变）；`print_timings` 的列与 `Total` 行算法未变；`plan_stage` / `execute_stage` 的 `profiling: bool` 语义未变（仍只表示旧开关）；`Database::open` / `execute_sql` / `commit` 签名未变；parse 失败仍是 `format!("Parse error: {message}")` + `PARSE_SQLSTATE`，plan 失败仍是 `failure.to_string()` + `failure.sqlstate`；WAL 刷盘单飞与 `append_commit_and_wait` 的 `Ok(())` 未触碰（`src/wal/` 不在 diff）。
4. Forbidden 全部成立：diff 只含 `src/diagnostics.rs`、`src/pipeline.rs`、`src/database.rs`、`src/lib.rs`、`tests/diagnostics_statement_test.rs` 与 change 文档；`src/profiling.rs`、`src/wal/`、`src/transaction/`、`src/storage/`、`src/executor/`、`tests/statement_cancellation_test.rs`、`tests/recovery_bounded_read_test.rs` 均未被触碰；`Cargo.toml` 未改（无新 crate）；无 CLI 参数、SQL 语法或协议字段变化；`Summary` / `StageStat` / `TraceSample` / `Record` 无操作标识、运行标识或内容哈希字段；子指标与等待类别未接入诊断；`execute_in_tx` / 预编译路径未接摘要；未跑全量。
5. Acceptance 逐条核对（代码 + 测试断言双向核对，断言均为精确值关系，无「因错误原因通过」的形态）：
   - `insert_commit_is_child_of_execute`：四个阶段各 `count == 1`，`commit.inclusive <= execute.inclusive`，`execute.exclusive + commit.inclusive == execute.inclusive`——与实现的父子挂载和独占算法一致。✓
   - `query_has_no_commit_stage`：查询摘要无 `commit`（查询不调 `Database::commit`）。✓
   - `cache_hit_skips_parse_and_plan`：首次未命中含 `parse`/`plan` 且无 `cache_hit_check`（未命中路径不作为诊断阶段，只由旧开关记录），第二次命中含 `cache_hit_check`/`execute` 且无 `parse`/`plan`——与 `execute_staged` 的条件上报一致。✓
   - `parse_error_keeps_error_text_and_summary_fails`：错误文本与 SQLSTATE 与关闭诊断时逐字节相同（同一语句两次执行取基线），摘要 severity 为 Error。✓
   - `abandoned_future_summary_is_cancelled`：锚点用公开 API（活跃事务集合 + 版本数 ≥ 2）+ `tokio::select! { biased; }` 丢弃语句 future；摘要 severity 为 Warn（Dropped）；回收后同键可再插入且 `t2` 只含完成语句的行——行可见性用行集 oracle 判定，未用摘要代替。✓
   - `legacy_profiling_table_follows_env_var`：真实 CLI 子进程（`CARGO_BIN_EXE_rtsql` + `RTSQL_HOME` 隔离 + `env_remove`），设置变量时 stderr 含 parse/plan/execute 与 `Total`，未设置时无 `Time (µs)` 表，默认会话不产生 `diagnostics ` 输出。✓
   - `inserts_visible_with_and_without_diagnostics`：诊断关闭与 `Detail::Full` + 全采集各插一行，重开后两行都可见且逐列值一致。✓
6. **偏差 1 的归类：Plan 错误**（不是 Act 偏离）。Plan 的 Current-State Evidence 断言 CLI 主命令 `rtsql <db> "<sql>"` 经 `execute_with_params`，与实际代码矛盾：`src/cli/mod.rs:846` 的 `run_sql` 自行 `parse_stage` + `plan_stage_routed(..., false, ...)` + `execute_stage_routed(..., false, ...)`，从不建立 profiling scope。Act 发现后改用 `rtsql import`——该入口的建表 DDL 与逐行 INSERT 均经 `Database::execute_sql`（`src/cli/lifecycle.rs:838`、`:889`），仍满足 Required behavior 原文（真实 CLI 入口、stderr 含三段表、未设置时无表）与需求 8 / S4 的可观察结果。独立复核确认该 CLI 主命令形态在 T2 之前就没有旧计时表（`run_sql` 与 `with_profiling_scope` 的绑定关系未被本次改动触碰），因此不是本次改动引入的回归。
7. 偏差 2、3（`rtsql new` 建库以避免进程内句柄；`execute_inner` 主体拆出 `execute_staged`）属等价局部实现选择，为单一收尾点服务，不改变任何分支语义。

非阻塞 Minor finding（记录，不返工）：

1. **旧开关错误路径新增旧表输出（Act 偏离）**：变更前 `execute_inner` 在 parse 失败、plan 失败、多语句拒绝、占位符错误等早退路径上不调用 `print_timings`；改写后 `execute_inner` 对所有返回路径（含错误）统一调用 `print_timings`。因此 `RTSQL_PROFILING` 打开时，失败语句现在也会向 stderr 打一张只含已完成阶段的旧表。Act Response 的「旧表计时窗口内容不变」对该路径不成立。不阻塞：不影响 SQL 结果、错误文本与 SQLSTATE；不改变 `print_timings` 的列与算法（`src/profiling.rs` 零改动）；不违反任何已批准的 Preserve / Forbidden 项；仓库内无断言「错误路径无该表」的测试（`RTSQL_PROFILING` 只出现在本轮新测试文件中）。
2. **需求 5 的 spec 子句缺断言**：change 内 spec 增量的「解析失败留下失败摘要且错误不变」还要求摘要「含有解析阶段」。实现满足（`execute_staged` 在匹配错误结果之前先 `record_stage("parse")`），但 `parse_error_keeps_error_text_and_summary_fails` 只断言 severity 与错误文本/SQLSTATE，未锁定该子句。不阻塞（Cycle 的 Acceptance 以 Plan Context 为准，行为已满足 spec）；建议收尾合并规格时确认或补断言。
3. **Act Response 章节标题为中文变体**而非 Cycle 模板的英文名。语义齐全（Status/Implemented/Deviations/Self-Review/Verification Evidence/Persisted Evidence/Experience Candidates/Remaining Issues 均有对应内容），按公共规则 › 通用能力属平台适配，不构成偏差。
4. **两处既有 rustfmt 漂移**：`cargo fmt --check` 显示 `tests/diagnostics_facade_test.rs:216`（T1 引入，已 accepted）与 `tests/session_lifecycle_test.rs:15`（不在本 change 的 diff 内）。本轮触碰的三份文件无漂移；Act Response 的记录属实，属格式化基线问题，非本轮失败。

**Acceptance Gaps**

None。Acceptance 列出的 7 条证明点与 RTM 的 4 条需求映射全部由代码与 GREEN 测试覆盖；Forbidden 项逐条核对成立；偏差 1 属 Plan 事实错误，已由等价真实入口覆盖同一可观察结果。

**Evidence**

- diff：`git diff e69c7e2..5d0d212`（merge-base 到 HEAD）。产品代码改动 = `src/diagnostics.rs`（+206）、`src/pipeline.rs`（+144/-55）、`src/database.rs`（+32）、`tests/diagnostics_statement_test.rs`（新建 509 行）。
- 验证沿用 Act Response（公共规则 › 验证）：GREEN `cargo test --offline --test diagnostics_statement_test -- --test-threads=1` → 退出码 0、`7 passed; 0 failed`；受影响范围 `cargo test --offline --test diagnostics_facade_test` → 退出码 0、`10 passed; 0 failed`。Act 提交后 HEAD 未再前进（`5d0d212` 即 Act 提交）、工作区 clean、覆盖范围未变化，无公共规则 › 验证 列出的重跑触发条件，Plan 未重跑已通过的验证，只做代码与 diff 的独立检查。
- Plan 独立执行的只读检查：`cargo fmt --check`（输出仅上述两处既有漂移）；`src/cli/mod.rs:846` `run_sql` 的执行路径；`src/cli/lifecycle.rs:838`/`:889` 的 import 入口；`src/pipeline.rs:674` 的隐式提交点；`src/storage/file_storage.rs:514`、`src/storage/memory_storage.rs:69` 的读页实现；`src/wal/buffer.rs:105`/`:263` 的刷盘路径。
- Mode: none，无 required Evidence；未因目录不存在提出问题。

**Follow-up Decision**

Acceptance 已满足且无阻塞项；四条 Minor finding 均不阻塞（其一为旧开关错误路径的新增输出，其二为 spec 子句的断言缺口但行为已满足，其三为格式适配，其四为既有格式化基线），按 iteration-planning › Review 分类判 `accepted`。Act Response 报告的两个 Issue 候选经本轮独立核对均为本 change 范围外的既有行为，不承载于本 change（判定见下），只报告不登记。Iteration 002（T3 等待、归属和开销）按 Map 展开。

**Issue 候选判定（只报告，不登记）**

- 候选 1（`RTSQL_PROFILING` 对 CLI 主命令 `rtsql <db> "<sql>"` 无效）：代码复核成立——`run_sql` 不经 `execute_with_params`、不建立 profiling scope，且该关系未被本次改动触碰，属变更前既有行为。本 change 的需求 8 只要求「旧计时出口保持」（已由真实 CLI 子进程入口证明），承载该修复需要新增「CLI 主命令接入旧计时表」的产品语义，超出 MS44 已批准范围，属产品决策。**不在本 change 承载**；建议由 Recorder 登记为 Issue（变更前既有缺口），若用户决定接线，另起 change。附带影响：`openspec/specs/pipeline-stage-decomposition/spec.md` 的「三段顶层计时观测」未限定入口，是否覆盖 CLI 主命令属既有规格解释问题，一并由 Recorder 判定。
- 候选 2（无显式主键建表把首列当唯一键）：Act 的真实 CLI 运行证据表明行为与 `src/cli/lifecycle.rs` 的注释矛盾，属键列语义面，与本 change 的诊断范围无关。**不在本 change 承载**；建议 Recorder 登记为 Issue。注意其对本轮测试的影响已被规避：`legacy_profiling_table_follows_env_var` 的第二次导入使用不同键（`2`），不依赖重复键行为。

**Iteration Plan Update**

None（accepted，无 replan）。

**Next Cycle**

None（当前 Cycle 无需修复，accepted 无返工）。

**Next Iteration**

`openspec/changes/2026-10-28-ms44-unified-diagnostics/iterations/002-wait-attribution/000-initial.md`
