# Iteration 002 / Cycle 000: 等待、归属和开销

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 需求 3（等待类别）、需求 4（同一次语句的异步样本归属）、需求 7（三种开销被实测且不设数值门槛）、需求 8 的 `[recovery]` 部分。Iteration 000 的门面与 Iteration 001 的阶段树（`parse` / `plan` / `execute` / `commit`、父子耗时口径、任务内绑定、旧计时表保持）在本轮直接继承使用，不重做、不改口径。行为以本文件的 Task Definition 为准。
- Excluded scope: 既有告警迁移；子指标接入诊断；CLI 主命令接入旧计时表（Issue 候选 1）与无主键建表的键列语义（Issue 候选 2）；`execute_in_tx` / 预编译路径的阶段摘要；CLI / SQL 界面变化；全量回归（T4）；数值开销门槛。

**Objective**

在 Iteration 001 的阶段树之上，把 design D5 列出的等待点按类别记入对应语句：提交等待分 Lock / Queue / Io / Schedule，文件页读取记 Io，其余阶段时间记 Compute；同一次刷盘的等待样本只进入被该次提交释放的那条语句的摘要，并发语句不串摘要；关闭 / 只开性能统计 / 详细追踪三种配置下同一负载的 SQL 结果相同；默认配置下 `[recovery]` 告警原文仍在。

**Background**

Iteration 000（诊断门面）与 Iteration 001（语句阶段）已 accepted（各自的 Plan Review 结论为 `accepted`，见 `iterations/000-diagnostics-facade/000-initial.md` 与 `iterations/001-statement-phases/000-initial.md`）。001 交付了阶段树、任务内绑定与父子耗时口径，但等待点尚未接入：提交等待与文件读页仍不产生任何等待类别，刷盘任务上的样本也不进入任何语句的摘要。本轮在既有阶段树上补这两块，并实测三种配置的开销。

**Investigation Facts**

- Starting point: HEAD `5d0d212`（Iteration 001 accepted）。001 的最终改动为 `src/diagnostics.rs`（任务内阶段绑定 `STATEMENT_STAGES` + `StageStack` + `enter_statement`）、`src/pipeline.rs`（`execute_with_params` 绑定点、`execute_inner` 拆出 `execute_staged`、每阶段一次测量同时喂旧表与新摘要）、`src/database.rs`（`Database::commit` 上报 `commit` 子阶段）、`tests/diagnostics_statement_test.rs`（新建）。验证结论 `cargo test --offline --test diagnostics_statement_test -- --test-threads=1` 退出码 0、7 passed / 0 failed，`cargo test --offline --test diagnostics_facade_test` 退出码 0、10 passed / 0 failed；覆盖面自产生后未变化，直接沿用。
- Current-State Evidence:
  - 阶段树现状（`src/diagnostics.rs`）：task_local `STATEMENT_STAGES` 持有 `Arc<Mutex<StageStack>>`；`StageStack` = `root_children: Vec<StageSample>` + `open: Vec<OpenStage>`；`OpenStage` = {module, name, children}（无等待字段）；`StageSample` = {module, name, inclusive, children}（无等待字段）；写入口只有 `push_completed` / `open_stage` / `close_stage` / `finish`；`close_stage` 以调用方单次测量把阶段挂到父级，`finish` 取走根的已完成子阶段并弃置仍 open 的阶段。`collect_stats` / `collect_samples` 从根的子阶段聚合（根自身不占阶段名、不受类别过滤）；独占 = `inclusive.saturating_sub(直接子阶段 inclusive 之和)`。
  - 公开类型现状：`Summary { sql, severity, total, stages, samples, events, dropped }`；`StageStat` 与 `TraceSample` 的字段是 {module, name, count, inclusive_total, exclusive_total}，没有等待类别字段；`Category { Compute, Lock, Io, Queue, Schedule }` 已存在（事件类别口径，与 design D1 的 `WaitClass` 同一集合，本轮复用，不新增第二套类别名）。`Record` 恰好四字段（severity / category / message / data），无操作标识、运行标识或内容哈希字段。
  - 等待点（design D5 的表，逐个核对实际代码）：
    - `src/transaction/manager.rs:110` `TransactionManager::commit` → `:119` `wal.session_gate().lock().await`——Lock 之一；在语句任务内执行（`Database::commit` → `transaction_manager.commit`）。
    - `src/wal/buffer.rs:105` `append_commit_and_wait` → `:107` `self.flush_gate.lock().await`（Lock 之二）；`:108-115` 登记等待者（`commit_waiters: Mutex<HashMap<u64, Arc<Notify>>>`、`pending_commits: Mutex<Vec<u64>>`）；`:124` `self.flush_notify.notify_one()`；`:127` `waiter.notified().await`——Queue 的测量窗口。
    - `src/wal/buffer.rs:263` `flush_locked`（调用方必须已持 `flush_gate`）：`:287-295` 那次 `wal_writer.write_batch(...)`（Io）；`:298-305` 唤醒本批等待者。`flush_loop`（`:145`）在 `:157-161` 的 select 中被 `flush_notify` 唤醒，`:167` 取 `flush_gate` 后 `:174` 调 `flush_locked`。`write_batch` 失败时仍唤醒等待者且 `append_commit_and_wait` 仍返回 `Ok(())`（既有合同）。
    - `src/storage/file_storage.rs:514` `FileStorage::read_page`（`AsyncStorage` 实现；计时须包在 `crypto` 分支之外，不复制函数体）；`src/storage/memory_storage.rs:69` `MemoryStorage::read_page` 不记 Io；`src/storage/buffer_pool.rs:130` 的 `self.storage.read_page(page_id).await?` 在调用方任务内执行，语句任务可见 task-local。
  - 跨任务边界：刷盘任务没有语句的 task-local。等待者登记发生在语句任务内（`append_commit_and_wait`），因此登记处能取得当前语句的阶段树句柄；`flush_locked` 运行在刷盘任务内，只能通过登记时随等待者一起保存的句柄回写。
  - 先后关系（保证归属正确）：`flush_locked` 先 `write_batch`（Io）再唤醒等待者；唤醒发生在语句任务从 `notified().await` 返回之前，因此回写的 Io / Schedule 在语句侧读取时可见。同一批被释放的每个等待者都拿到同一段 Io（组提交口径，design D5 风险条目已登记该放大效应）。
  - 阶段形态现状：`Database::commit`（`src/database.rs:578`）用 `stage_instant()` + `record_stage()` 把 `commit` 记为**已完成**阶段——不是 open 阶段，等待样本无处可挂。`execute_staged`（`src/pipeline.rs:1487`）中 `parse` / `plan` 以 `record_stage` 记为已完成阶段，`execute` 以 `open_stage` / `close_stage` 记；隐式 DML 的提交点 `src/pipeline.rs:674`（`target_db.commit(tx)`，在 `execute_stage_routed` 内）。
  - 既有测试与基准入口：`tests/diagnostics_statement_test.rs` 的 `stats_config()`（只开 stats）与 `Detail::Full` + 全采集配置可直接沿用为等待用例的配置写法；`Database::open` + `execute_sql` 的 tempdir 文件库是语句行为的真实入口；`db.close().await` 后重开同一路径得到缓冲池为空的 `Database`（缓冲池是 `Database` 的私有字段，不跨打开保留）。`benches/pipeline_stages_bench.rs` 与 `benches/common/mod.rs` 的负载形态是 `CREATE TABLE bench (id INT PRIMARY KEY, name STRING, value INT)` + 100 行 + `SELECT * FROM bench WHERE id = 42`；判定放在 `cargo test`，不改 criterion 门槛。`tests/recovery_bounded_read_test.rs`（`#![cfg(all(feature = "network", feature = "cli"))]`）持有 `[recovery]` 的 stderr 断言。`tests/diagnostics_facade_test.rs` 只用字面量构造 `Record`（四字段），不字面量构造 `StageStat` / `TraceSample` / `Summary`——给后两者增加字段不破坏既有测试。
- Code and Critical Path: 修改 `src/diagnostics.rs`（等待类别累计、阶段等待字段、跨任务槽句柄、公开等待汇总类型；模块仍不得依赖 database / pipeline / wal / storage / transaction 各层，M21）、`src/pipeline.rs`（`execute_staged` 的 parse / plan / execute 改为 open 阶段，承载等待归属）、`src/database.rs`（`Database::commit` 的 `commit` 改为 open 阶段）、`src/transaction/manager.rs`（会话闸门 Lock 计时）、`src/wal/buffer.rs`（刷盘闸门 Lock、`notified().await` 的 Queue、等待者登记携带槽与通知时刻、`flush_locked` 回写 Io / Schedule）、`src/storage/file_storage.rs`（读页 Io 与页数），新建 `tests/diagnostics_wait_test.rs`。`src/profiling.rs` 零改动。

**Implementation Guidance**

- 等待类别复用既有 `Category`（Compute / Lock / Io / Queue / Schedule），不引入第二套类别名，也不新增依赖。
- 等待样本的归属点是「当时最内层 open 阶段」。因此 `parse` / `plan` / `execute` / `commit` 都要以 open 阶段形式打开并在同一次测量后关闭；闭合后的结构应与 Iteration 001 等价——`parse`、`plan`、`execute` 仍是根的直接子阶段，`commit` 仍是 `execute` 的子阶段（`commit` 仍在 `execute_stage` 内、隐式提交点 `src/pipeline.rs:674` 之后闭合）。
- 跨任务槽：刷盘任务没有语句的 task-local。登记等待者时（语句任务内）把当前语句的阶段树句柄放入等待表，并在发出 `notify_one` 时把通知时刻一并记录到该等待者；`flush_locked` 只对本次释放的等待者回写 Io 与 Schedule。句柄类型由 `src/diagnostics.rs` 提供（诊断模块解释它，`src/wal/buffer.rs` 只持有与回写，不解释语义）。
- Queue 的算法：`notified().await` 的实测耗时减去本次已记入该语句的 Io 与 Schedule，小于 0 记 0。Lock 只含两次 `.lock().await`，不含 Queue。
- Schedule 的算法：从该等待者 `notify_one` 的时刻到刷盘任务开始处理该批的时刻的间隔；通知到达时刷盘循环不在空等（正忙）时记 0。
- Io 与页数：`FileStorage::read_page` 的整次调用记 Io，并使该阶段的页数加 1；`MemoryStorage::read_page` 不记；缓冲池已持有的页不经过 `read_page`，不记 Io。
- Compute 的算法：阶段独占耗时减去该阶段自身的等待类别耗时之和（不含子阶段的等待——内层等待不重复计入外层阶段）。
- 每个阶段只测一次：同一次 `Instant` 测量继续同时喂旧表（仅旧开关打开时调用 `record_time`）与诊断阶段上报；等待计时是额外的等待点测量，不与阶段测量重复。
- 采集全关时不建立绑定、不做等待计时、不写 stderr；默认配置下既有 stderr 告警（含 `[recovery]`）原文不动。

**Task Definitions**

### T3: 等待类别、跨任务归属与三种开销

- Requirement/Scenario: 需求 3（等待类别 Lock / Queue / Io / Schedule；文件页读取记 Io；其余阶段时间记 Compute；内层等待不重复计入外层；只开统计时等待以汇总数出现）、需求 4（同一次提交采集的等待样本只进入该语句的摘要；并发语句按各自 SQL 文本区分；公开摘要没有操作标识、运行标识或内容哈希）、需求 7（关闭 / 只开性能统计 / 详细追踪三种配置的耗时都被记录，SQL 结果相同，不设差值上限）、需求 8 的 `[recovery]` 部分。场景 S9、S10、S13，以及 change 内 spec 增量的「提交等待带出锁、队列和 I/O」「解析阶段记为计算」「文件页读取记为 I/O」「只开统计时不保留逐条样本」「三种配置结果相同且都有耗时记录」。
- Depends on: T2（Iteration 001 accepted）。
- Targets: `src/diagnostics.rs`（等待类别累计、阶段等待字段、跨任务槽句柄、公开等待汇总类型）、`src/pipeline.rs::execute_staged`（parse / plan / execute 的 open 阶段）、`src/database.rs::Database::commit`（`commit` 的 open 阶段）、`src/transaction/manager.rs::TransactionManager::commit`（会话闸门 Lock）、`src/wal/buffer.rs::append_commit_and_wait` 与 `flush_locked`（刷盘闸门 Lock、Queue、Io、Schedule、等待者登记携带槽与通知时刻）、`src/storage/file_storage.rs::FileStorage::read_page`（Io 与页数）、`tests/diagnostics_wait_test.rs`（新建）。
- Current behavior: 提交等待与文件读页不产生等待类别——阶段摘要只有 `parse` / `plan` / `execute` / `commit` 的耗时，没有 Lock / Queue / Io / Schedule，也没有页数；刷盘任务上的样本不进入任何语句的摘要；并发语句的摘要各自独立但没有等待归属这一层。
- Required behavior:
  - 详细追踪打开时，一次成功插入的提交子阶段给出 Lock、Queue、Io 和 Schedule 四类耗时（Schedule 允许为 0），且这四类只出现在该语句的摘要里。
  - 解析阶段有 Compute 且没有 Io。
  - 重新打开的文件库上（缓冲池尚未持有目标页）执行一次要读该页的查询，摘要含 Io 且页数至少为 1；缓冲池已持有的页不因被访问而记 Io。
  - 只开性能统计（详细追踪关闭）时，等待以类别汇总数出现在阶段汇总里，且摘要不含逐条追踪样本。
  - 同一会话上两条 SQL 文本不同的并发插入各自只含自己的 SQL 文本；两条都成功；两行都可见；公开摘要没有操作标识、运行标识或内容哈希字段。
  - 同一条已有单行查询负载（`SELECT * FROM bench WHERE id = 42`，各自新库、100 行）在诊断关闭、只开性能统计、详细追踪三种配置下的行集相同，三次都在 30 秒内返回。
  - 默认配置下，既有恢复测试的 stderr 仍含 `[recovery]`。
- Preserve: `append_commit_and_wait` 在 `write_batch` 失败时仍返回 `Ok(())`；刷盘仍单飞；`MemoryStorage::read_page` 不记 Io；`[recovery]` 原文不改；SQL 结果、错误文本、SQLSTATE、取消后的行可见性与回收语义不变；`print_timings` 的列与 `Total` 行算法；`src/profiling.rs` 零改动；`src/diagnostics.rs` 不 import database / pipeline / wal / storage / transaction（M21）；默认配置不产生摘要、不写新的诊断 stderr；公开摘要没有操作标识、运行标识或内容哈希字段。
- Forbidden: 给未列出的锁加计时；给全部 await 加计时；新建后台写出任务；把 Io 跨语句相加写成断言；用操作标识、运行标识或内容哈希判定通过；修改 `tests/recovery_bounded_read_test.rs` 的告警断言；跑全量 `cargo test`；修改 criterion 的通过门槛；改 `src/executor/`；新增 crate、CLI 参数或 SQL 语法。
- First test: `tests/diagnostics_wait_test.rs` 的 `commit_wait_classes_present`。先只加这个测试。RED 命令 `cargo test --offline --test diagnostics_wait_test commit_wait_classes_present`，预期编译失败（等待类别字段尚不存在）或断言失败（阶段没有等待类别）。看到该失败后再写实现。
- GREEN condition: `cargo test --offline --test diagnostics_wait_test` 退出码 0 且 failed 为 0，文件内测试为：`commit_wait_classes_present`、`parse_stage_has_compute_without_io`、`file_page_read_records_io`、`stats_only_reports_wait_totals_without_samples`、`concurrent_inserts_keep_own_sql`、`three_modes_match_rows`；并且 `cargo test --offline --test recovery_bounded_read_test` 退出码 0 且 failed 为 0。
- Verification: 上面两条命令都用默认并行度（该文件不设置 `RTSQL_PROFILING`，无进程级环境竞争；design D7 的单线程纪律只适用于设置该变量的文件）。`commit_wait_classes_present` 断言提交子阶段的四类等待字段都存在，且 Lock / Queue / Io 非零、Schedule ≥ 0；`parse_stage_has_compute_without_io` 断言解析阶段 Compute > 0 且 Io == 0；`file_page_read_records_io` 用「建库 → 插入 → `db.close()` → 重开 → 查询」的真实入口断言摘要含 Io 且页数 ≥ 1，并断言解析阶段 Io == 0；`stats_only_reports_wait_totals_without_samples` 断言阶段汇总带等待类别数而 `samples` 为空；`concurrent_inserts_keep_own_sql` 用两条 SQL 文本不同的并发插入断言各自摘要只含自己的文本且两行都可见；`three_modes_match_rows` 用三个各自新建的 100 行文件库跑同一条查询，断言三次行集相同且每次在 30 秒内返回（`tokio::time::timeout`），不比较三者差值。断言只用等待类别数值、页数、摘要 SQL 文本、行集与子进程 stderr 文本。
- Stop when: 某个已列等待点无法在保持既有 `Result` 与合同（`Ok(())`、刷盘单飞、`[recovery]` 原文）的前提下包住；或并发摘要必须靠操作标识才能区分；或等待样本必须新建后台写出任务才能归属；或必须改变事务语义、SQL 结果、错误文本或取消后行可见性才能记录等待。

**Invariants**

- SQL 结果、错误文本、SQLSTATE、取消后的行可见性与回收语义不变。
- 等待只加在 design D5 列出的点上；不新增锁与 await 的计时。
- 内层等待不重复计入外层阶段；跨语句的 Io 不得相加当作进程级 I/O 总量。
- 默认配置不产生摘要、不写新的诊断 stderr；既有 stderr 告警（含 `[recovery]`）原文不变。
- `src/diagnostics.rs` 不 import database / pipeline / wal / storage / transaction（M21）。
- 公开摘要与公开记录没有操作标识、运行标识或内容哈希字段。
- 每个阶段只测一次：同一 `Instant` 的耗时喂旧表与新摘要两处。
- 采集全关时不做等待计时（默认配置零成本）。
- 跨任务槽只承载阶段树句柄与通知时刻，不承载身份信息。

**Non-goals**

- 给全部锁与全部 await 加计时；全库插桩；后台监控任务；远程采集；持久化诊断库或可视化界面。
- 把 CLI 主命令接入旧计时表（Issue 候选 1）；无显式主键建表的键列语义（Issue 候选 2）。
- 数值开销门槛；修改 criterion 门槛；任何实际性能优化（改执行计划、组提交、缓存策略或 I/O 路径）。
- 告警迁移；CLI / SQL 界面变化；`execute_in_tx` / 预编译路径的阶段摘要；全量回归。

**Acceptance**

- `commit_wait_classes_present` 证明提交等待分 Lock / Queue / Io / Schedule 且只在该语句摘要内（需求 3）。
- `parse_stage_has_compute_without_io` 证明其余阶段时间记 Compute、解析阶段无 Io（需求 3）。
- `file_page_read_records_io` 证明文件页读取记 Io 且页数 ≥ 1，缓冲池已持有的页不记 Io（需求 3、S9）。
- `stats_only_reports_wait_totals_without_samples` 证明只开统计时等待以类别汇总数出现、无逐条样本（需求 3）。
- `concurrent_inserts_keep_own_sql` 证明同一次提交的样本不串句、并发语句按各自 SQL 文本区分、无标识字段、两行都可见（需求 4、S10）。
- `three_modes_match_rows` 证明三种配置的耗时都被记录且 SQL 结果相同、无差值上限（需求 7）。
- `cargo test --offline --test recovery_bounded_read_test` 退出码 0、failed 为 0 证明 `[recovery]` 原文仍在（需求 8、S13）。
- Requirement Traceability：需求 3 → T3（`commit_wait_classes_present`、`parse_stage_has_compute_without_io`、`file_page_read_records_io`、`stats_only_reports_wait_totals_without_samples`）；需求 4 → T3（`concurrent_inserts_keep_own_sql`）；需求 7 → T3（`three_modes_match_rows`）；需求 8 的 `[recovery]` 部分 → T3（`recovery_bounded_read_test`）；均 Covered。

**Verification**

逐 scenario 的最简直接判定（自动命令、真实入口）：`cargo test --offline --test diagnostics_wait_test` 与 `cargo test --offline --test recovery_bounded_read_test`，两条命令退出码 0 且 failed 为 0 为通过。判定只用阶段等待类别数值、页数、摘要 SQL 文本、行集与子进程 stderr 文本；不使用诊断字段作为运行身份，不引入操作标识、运行标识或内容哈希作为通过条件，也不新增判定层。Iteration 000 与 001 的 GREEN 结论覆盖未变化，沿用；本轮不跑全量（全量预算在 Iteration 003，整个 change 一次）。

**Gate 2 Readiness**

PASS。检查项均通过，无 BLOCKED，无 SKIPPED。调查基于本会话对 `src/wal/buffer.rs`、`src/transaction/manager.rs`、`src/storage/file_storage.rs`、`src/storage/buffer_pool.rs`、`src/pipeline.rs`、`src/database.rs`、`src/diagnostics.rs` 与既有测试 / 基准先例的实际阅读；设计闭合（等待点、类别、归属机制、Queue / Schedule / Compute 的算法、公开承载形状已定，接口与错误语义不留 TBD）；RTM 无 Missing、无 Simplified；Persisted Evidence 为 none；批准来自本次 goal 对「把这个 change 做完」的授权（goal 注入授权，覆盖 Gate 1 / Gate 2 与计划批准，检查标准不降低）。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 等待点包住的方式不得改变 `Result` 与既有合同（`Ok(())`、刷盘单飞）；任一等待点无法包住时按 Stop when 回到计划。
- 刷盘任务回写 Io / Schedule 依赖「先回写、后唤醒」的顺序保证；实现必须保持该顺序，否则语句侧会在 Io 落账前计算 Queue。
- 组提交把整段 Io 写入每个被释放的等待者，跨语句相加会放大——不得写成断言，也不得把该和当作进程级 I/O。
- 阶段改为 open 阶段后，闭合结构应与 Iteration 001 等价（`parse` / `plan` / `execute` 是根的直接子阶段，`commit` 是 `execute` 的子阶段）；结构若变化先回计划。
- 没有 open 阶段时发生的等待样本（例如语句起止边界上的读页）记入语句根，不进入阶段汇总数——不得因此新增根阶段名。
- `three_modes_match_rows` 沿用 `benches/pipeline_stages_bench.rs` 的负载形态（`bench` 表 100 行 + `SELECT * FROM bench WHERE id = 42`），判定放在 `cargo test` 内；三次各自新建文件库，避免计划缓存把后两次变成另一种路径。
- `[recovery]` 断言面在 `tests/recovery_bounded_read_test.rs`，本轮只跑它作回归，不改其告警断言。
- 本轮不跑全量（全量预算在 Iteration 003，整个 change 一次，按 M19 分批串行）。
