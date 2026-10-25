# Iteration 001 / Cycle 000: 调用方可见的三种结果与收尾回归

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: 2.1、2.2、2.3、2.4、2.5
- Acceptance gaps: None
- Repair items: None
- Inherited scope: Iteration 000 的稳定结果——库属隐式事务的**放弃即登记 + 三点回收**机制（`src/database.rs` 的 `ImplicitTxGuard` / `implicit_transactions` 登记表 / `reclaim_abandoned_implicit_transactions` / `reclaim_implicit_tx`；`src/pipeline.rs` 的 DML 臂与 CTAS 路径；`src/executor/alter_table.rs` 的 ALTER TABLE DROP COLUMN 臂；`src/transaction/manager.rs` 的 `abort_by_id` / `discard_tx_version_tables`）、`tests/statement_cancellation_test.rs` 的四个既有用例与其夹具、design D1-D6、规格 `statement-cancellation-outcome` 的全部 requirement、change `tasks.md` 的 Iteration Map（000 已 `accepted`）、Iteration 000 的最终 Act Response 与 accepted Review 的结论。
- Excluded scope: 协作式取消上下文与 deadline 公共 API（命中 MS34 拆分信号，留 I096）、完整分级诊断（I096）、真实断电与 `kill -9` 语义、WAL 记录格式与 checkpoint 九步顺序、CLI 信号停机行为、语句并发模型的改造（不引入全局执行闸门）、被放弃的 ALTER TABLE DROP COLUMN 的数据面语义（目录已窄化 + 数据面混合宽度——change 范围外的实质问题，只报告不登记）、MS35 / MS36。

**Objective**

调用方能从库入口判断出三种结果——提交前被取消则未提交、提交已完成但未送达则已提交、显式事务由调用方继续还是回滚；超时是对语句的放弃而不是对事务的回滚；语句被放弃之后会话与后台 I/O 仍按会话级生命周期工作；整个 change 一次 M19 分批串行全量回归 0 failed。

**Background**

Iteration 000（`iterations/000-abandoned-implicit-transaction-reclaim/`）的 rework Cycle `001-rework.md` 已获 `accepted`：登记表语义从「库自己开的事务」改为「持有它的语句 future 已被放弃」，并发中的非取消语句不再被另一条语句的回收中止，第三处库属隐式事务站点（ALTER TABLE DROP COLUMN）补齐回归用例。该 Review 同时留下一条**必须在本 Iteration 承接的事实**：`TransactionManager::commit` 的内部顺序是「写 WAL `CommitTxn` + fsync → `commit_mark_versions` → 移出活跃集 → 清 `tx_versions`」，因此存在一个极窄子窗口（`commit_mark_versions` 完成后、`active.remove` 之前有一个 await 点）——future 在此被放弃时，守卫会登记该 tx id，而该事务**仍在活跃集**内，后续回收会中止它。该窗口在 Iteration 000 的父实现下更宽（父实现自 `begin()` 起即登记），返工是收窄而非引入；它正落在本 Iteration 的场景 3（提交已完成但未送达）范围内，本 Iteration 必须给出可观察结论。

**Investigation Facts**

- Starting point: 修订 `0a08b14`（MS33 收尾入库），`master` 比 `origin/master` 超前 9。工作区携带 Iteration 000 已接受但未提交的成果（不回退、不重做）：`src/database.rs`（`ImplicitTxGuard`（`pub(crate)`）、`Database::implicit_transactions: Arc<Mutex<HashSet<u64>>>`、`Database::implicit_tx_guard(tx_id)`、`reclaim_abandoned_implicit_transactions()`、`reclaim_implicit_tx()`（含「不在活跃集 → 跳过」守卫））、`src/pipeline.rs`（DML 臂 `:560` 回收 / `:575` 建守卫 / `:605`、`:638`、`:652` 解除；CTAS 路径 `:884` 回收 / `:1009` 建守卫 / `:1020`、`:1061`、`:1088` 解除）、`src/executor/alter_table.rs`（`:87` 回收 / `:100` 建守卫 / `:119`、`:135` 解除）、`src/transaction/manager.rs`（`abort_by_id` `:160`、`discard_tx_version_tables`）、`tests/statement_cancellation_test.rs`（四个用例全绿）。`.agents/docs/tasks.md` 的未提交修改（MS34–MS36 转 ready）**不属于本 Cycle**。
  - 沿用未失效的既有结论：变更前基线 21 / 3 / 4 / 6（`explicit_tx_test` / `mvcc_abort_test` / `mvcc_commit_test` / `dml_tx_id_test`）与 MS33 收尾的全量终态 2155 / 0 / 2（`.agents/docs/SNAPSHOT.md`）；Iteration 000 在其实现改动之后已重跑事务类基线与受影响面 9 目标（101 passed / 0 failed）并退出码 0（来源：Iteration 000 的 `001-rework.md` Act Response Verification Evidence）。
- Current-State Evidence:
  - **显式事务的语句路径（任务 2.2 / 2.3 的行为基础）**：`Database::execute_in_tx`（`src/database.rs:738`）→ `pipeline::execute_in_tx`（`src/pipeline.rs:1170`）→ `execute_in_tx_with_params`（`src/pipeline.rs:1183`）→ `execute_stage_in_tx`（`src/pipeline.rs:1248`）→ `execute_stage_in_tx_routed`（`src/pipeline.rs:1258`）。该路径**不** `begin()`、**不** `abort()`、**不** `commit()`、**不**触发回收、**不**创建守卫；DML 消费调用方的 `tx_id`（`create_executor_from_plan(..., Some(tx_id), ...)`）。语句返回错误时**不**代替调用方收尾——函数文档注释明写「A failed statement returns an error response and leaves the transaction alive for the caller to commit or roll back」。语句 future 被放弃时同样不触碰事务状态（future 析构只释放执行器与页锁，不产生任何事务副作用）。→ **「库不代替调用方提交或回滚显式事务」当前已经成立**，2.2 / 2.3 缺的是可观察证据，不是行为修复。
  - **显式事务的读可见性（2.2 / 2.3 的断言基础）**：`execute_stage_in_tx_routed` 的 `_` 臂取 `database.statement_snapshot(Some(tx_id))`——事务自身的写经 `is_visible_self` 在事务内可见，事务外的读看不到未提交的写。→ 「行仍在事务内」与「事务外不可见」都是库入口可观察状态。
  - **提交路径与「提交已完成但未送达」窗口（任务 2.1 的全部技术难点）**：`Database::commit`（`src/database.rs:547`）= `transaction_manager.commit(tx, &buffer_pool).await?` → `maybe_auto_checkpoint().await`（`src/database.rs:654`）。`TransactionManager::commit`（`src/transaction/manager.rs:108`）的顺序是：写 WAL `CommitTxn` + `append_commit_and_wait`（fsync）→ `commit_mark_versions`（`:292`）→ `active.remove(&tx_id)` → 清 `tx_versions`。**因此「tx id 离开 `active_transactions()`」等价于「提交已原子完成」**（提交帧已 fsync、版本已标 committed）。
    - `maybe_auto_checkpoint` 的尾段：`reclaim_abandoned_implicit_transactions()`（登记表为空时**无 await 点**）→ 阈值判定 → R3 顺延判定（`active_transactions()` 为空才继续）→ `self.checkpoint().await`（`src/database.rs:849`）→ `CheckpointManager::checkpoint` 的刷页与 WAL 重写截断（多个 await）。
    - **窗口默认不可达**：阈值为 0 时 `maybe_auto_checkpoint` 立即返回，`active.remove` 与 `Database::commit` 返回之间没有 await 点，放弃动作无法落在其中。
    - **窗口可被工程化拉宽**：阈值由 `Database::set_checkpoint_bytes_threshold(bytes)`（`src/database.rs:684`，`pub`）控制；提交帧本身会 `bytes_appended.fetch_add`（`src/wal/writer.rs:97`），因此**在提交前把阈值设为 1**，提交后 `bytes_appended() >= 1` 必然成立、活跃集为空（R3 不顺延）→ 自动 checkpoint 在 `Database::commit` 内触发 → 尾段变成一次真 checkpoint（多 await、含 fsync），「提交已完成但结果未送达」的窗口稳定可达。
    - 计数器语义：`bytes_appended` 只在 `CheckpointManager::checkpoint` 成功收尾时 `reset_bytes_appended()`（`src/wal/checkpoint.rs:229`）；默认阈值 `DEFAULT_CHECKPOINT_BYTES = 16MB`（`src/wal/writer.rs:19`）——准备阶段（2000 行插入）可能触发自动 checkpoint 并把计数清零，因此阈值必须在**提交之前**设置。
  - **显式事务不会被回收误伤（2.1 的安全性前提）**：登记表的唯一来源是语句作用域守卫的 `Drop`，守卫只在三处库属隐式事务点创建（`src/pipeline.rs:575`、`src/pipeline.rs:1009`、`src/executor/alter_table.rs:100`）；`Database::begin()`（`src/database.rs:536`）与 `TransactionSession::begin`（`src/transaction/session.rs:52`）是调用方持有的显式事务，**从不**产生登记项。因此「已提交但结果未送达」的显式事务不会被任何回收触发点中止；`reclaim_implicit_tx` 的「不在活跃集 → 跳过」守卫（`src/database.rs` 内 `reclaim_implicit_tx`）作为第二道保险继续有效。
  - **回滚路径（2.2 的另一半）**：`Database::rollback`（`src/database.rs:705`）按 `tx_version_tables(tx_id)` 解析该事务触及的表 → `get_table` 取 `TableMeta` → `transaction_manager.abort(tx, &buffer_pool, &tables)` → 既有中止路径（WAL `AbortTxn` → `abort_cleanup_versions` 还原索引条目 → 移出活跃集 → 清 `tx_versions`）。→ 调用方回滚可撤销被取消语句已写入的部分。
  - **库外的完整性检查（2.4 的断言）**：`rtsql::storage::integrity::check_integrity(&db.buffer_pool, &db.table_manager)`（`src/storage/integrity.rs:87`，`pub mod integrity`；`Database::buffer_pool` / `Database::table_manager` 为 `pub` 字段）返回 `IntegrityReport`，严重级 `FindingSeverity::{Error, Info}`（`src/storage/integrity.rs:43`）。
  - **夹具（Iteration 000 产出，可直接复用）**：`tests/statement_cancellation_test.rs` 的 `setup(&dir)`（`t(id INT PRIMARY KEY, v INT)` 预置 2000 行、`t2(id INT PRIMARY KEY)`）、`anchor_tx(&db)`（返回「活跃且 `get_tx_versions(tx_id).len() >= 2`」的 tx id）、`abandon_statement(&db, sql)`（`Box::pin` + `tokio::select!`（`biased`，锚点支在前）+ `drop(fut)`）、`expect_affected` / `expect_rows`。`ANCHOR_TIMEOUT = 10s` 只作夹具失控兜底（触发即失败，不是阈值判定）。锚点轮询用 `tokio::task::yield_now()` 让出执行权。
  - **`Transaction` 不是 `Clone`**（`src/transaction/manager.rs:24-28`：字段 `id` / `snapshot` / `state`）→ 显式事务句柄不能移入 `tokio::spawn`；需要并发观察时只克隆 `Database`（`Database: Clone`，内部字段全是 `Arc`）。
- Code and Critical Path: 库入口 `Database::execute_in_tx`（`src/database.rs:738`）→ `pipeline::execute_in_tx`（`src/pipeline.rs:1170`）→ `execute_in_tx_with_params`（`:1183`）→ `execute_stage_in_tx`（`:1248`）→ `execute_stage_in_tx_routed`（`:1258`）→ `execute_executor(executor).await`——**放弃点落在此处**；此后事务状态不变。调用方收尾：`Database::commit`（`src/database.rs:547`）→ `TransactionManager::commit`（`src/transaction/manager.rs:108`）→ `maybe_auto_checkpoint`（`src/database.rs:654`）→ `Database::checkpoint`（`:849`）→ `CheckpointManager::checkpoint`；或 `Database::rollback`（`:705`）→ `TransactionManager::abort`（`src/transaction/manager.rs:142`）→ `abort_by_id`（`:160`）。会话收尾：`Database::close`（`src/database.rs:821`）→ `checkpoint()`。

**Implementation Guidance**

- 2.2 / 2.3 / 2.4 是**表征用例**（characterization）：按上面的 Current-State Evidence，显式事务路径本就不代替调用方提交或回滚，会话级对象本就不随语句生死。这三个用例**不存在修复前 RED**，按 TDD 的「重构 / 表征」分支处理——先观察 GREEN、记录基线，**不得为制造 RED 改产品代码**；用例的价值是把「库不代替调用方收尾」这一约定从代码注释变成可观察证据。
- **2.1 是本 Iteration 唯一可能出现 RED 的任务**，且它有明确的判据（见 Task Definition 的 First test）。关键工程手法有三件，缺一不可：
  1. **拉宽窗口**：提交前调用 `db.set_checkpoint_bytes_threshold(1)`，使 `Database::commit` 的尾段变成一次真 checkpoint（多个 await + fsync），「提交已完成但结果未送达」的窗口才稳定可达；不设阈值时该窗口在 `active.remove` 与 `Database::commit` 返回之间**没有 await 点**，不可达。
  2. **锚点**：以「tx id 离开 `active_transactions()`」为「提交已原子完成」的判据（依据 `TransactionManager::commit` 的内部顺序：提交帧 fsync → 版本标 committed → 移出活跃集）。
  3. **判别位**：用一个测试内的 `AtomicBool` 记录「commit future 已返回」。锚点命中时若该位为真，说明放弃动作落在提交**完全返回之后**、场景未被复现——此时用例必须**显式失败**，不得静默通过（这正是「若放弃点实际落在提交完成之前，该用例必须失败而不是误判为通过」的反向面：落在提交完成之后同样必须失败）。
- 2.2 / 2.3 的放弃点复用 Iteration 000 的 `anchor_tx`（活跃 tx 且版本数 ≥2）。2.3 的超时只是**取消触发源**（规格场景原文即「调用方用超时包住该语句且超时到期」），一切断言落在可观察状态上；**不得**以耗时阈值判定成败，也**不得**把超时时长当成通过条件。2.3 另需一个并发锚点观察任务确认「超时到期前语句已推进」（≥2 个版本），否则显式失败（夹具规模不对，不是产品结论）。
- 2.4 复用 `setup` 夹具：放弃一条语句 → 同会话继续写入 → `close()` → 重开 → 后续写入完整、被放弃语句的行不存在、`check_integrity` 无 Error 级发现。
- 2.5 是整个 change 唯一一次全量回归，**必须**按 M19 分批串行执行（见 Task Definition）。不得用一次无目标过滤的全量调用替代分批，也不得在批内使用默认并行度。

**Task Definitions**

### 2.1: 提交已完成但结果未送达时语句已提交

- Requirement/Scenario: 提交已完成但调用方未收到结果时语句已提交 / 放弃已完成的提交后修改仍在
- Depends on: None（Iteration 000 的稳定结果已在起点上）
- Targets: `tests/statement_cancellation_test.rs`（新增用例 `committed_statement_survives_undelivered_result`）
- Current behavior: 显式事务不经登记表、从不被回收；`Database::commit` 的原子效果（提交帧 fsync + 版本标 committed + 移出活跃集）先于其返回。因此「提交已完成但结果未送达」时修改应已持久且可见。**当前没有用例证明这一点**，且该窗口在 `active.remove` 与 `Database::commit` 返回之间默认没有 await 点（阈值 0 时 `maybe_auto_checkpoint` 立即返回），放弃动作无法落在其中——需要用 WAL 字节阈值把尾段拉宽成一个真 checkpoint 才能稳定复现。
- Required behavior: 新增用例 `committed_statement_survives_undelivered_result`：显式事务内插入一行 → 提交前把 WAL 字节阈值设为 1 → 发起 `Database::commit` 并在其**提交已原子完成、future 尚未返回**时放弃该 future → 断言：(a) 该行在**同一会话**读得到；(b) `close()` 后重开该行仍在；(c) 活跃事务集合为空（无悬空事务）；(d) 放弃之后的后续写语句正常成功（不报 `Reclaim failed`）。用例必须在「放弃点未落在提交完成之后」时**显式失败**，不得静默通过。
- Preserve: 显式事务的生死仍由调用方决定；`Database::commit` / `Database::rollback` 的既有语义与错误面不变；checkpoint 九步顺序、WAL 帧与位点布局不变；`set_checkpoint_bytes_threshold` 的既有语义不变（只是本用例把它设成一个更小的值）；回收机制与登记表语义不变。
- Forbidden: 为让用例通过而改 `Database::commit` / `TransactionManager::commit` 的顺序或语义；改 checkpoint 步骤或 WAL 格式；引入协作式取消上下文或 deadline 公共 API；用哈希、指纹、run-id、位点 pin、manifest、时间顺序之类判定代替行为观察；在项目测试框架之外建判定层；用「提交结果是否为 Ok」代替「行是否已提交」作为判据。
- First test: 在 `tests/statement_cancellation_test.rs` 新增用例 `committed_statement_survives_undelivered_result`。夹具：`setup(&dir)`（`t` 2000 行、`t2`）；`let tx = db.begin().await.unwrap();` → `db.execute_in_tx("INSERT INTO t2 VALUES (1)", &tx).await.unwrap();` → `db.set_checkpoint_bytes_threshold(1);` → 用 `Box::pin` 持有 `db.commit(tx)` 的 future，用 `tokio::select!`（`biased`，锚点支在前）在锚点命中时 `drop(fut)`。锚点 = 轮询 `db.transaction_manager.active_transactions()` 直到**不含**该 tx id（轮询用 `yield_now`，上限 10s 只作失控兜底）。判别位：用一个 `AtomicBool` 记录 commit future 是否已返回；锚点命中时若该位为真 → `panic!("the commit future already returned: the undelivered-result scenario was not reproduced")`；若 `tokio::select!` 的提交支先返回 → 同样 `panic!`（场景未被复现，不是产品结论）。命令：`cargo test --test statement_cancellation_test committed_statement_survives_undelivered_result -- --test-threads=1`。
  - **RED 判据**：该用例必须先观察到 RED 才能算进入 GREEN 修复——失败信息指向「行在重开后不存在」或「行在同一会话读不到」这类**目标行为**未满足，而不是编译错误、夹具错误、锚点超时或「场景未被复现」的 panic。**若只得到「场景未被复现」的 panic**，说明窗口工程（阈值 / 锚点 / 判别位）还没到位，按 Stop when 处理，不得改产品代码去制造 RED，也不得把该 panic 当作通过。
- GREEN condition: 该用例通过；`cargo test --test statement_cancellation_test -- --test-threads=1` 退出码 0（五个用例全部通过，failed 计数为 0）。
- Verification: 上列两条命令的退出码都是 0；本用例的失败原因与 1.1、1.3、T1-R1、T1-R2 用例互不相同（它看的是「提交已完成但结果未送达」后的行是否仍在，与活跃集是否清空、是否随 checkpoint 落盘、是否被并发回收中止、回收是否报错都不同）。
- Stop when: 复现该窗口必须改 `TransactionManager::commit` 的内部顺序或 `Database::commit` 的收尾结构——那是范围扩大，返回 Plan；或让该用例通过必须改 checkpoint 步骤 / WAL 格式——返回 Plan；或 `close()` 后重开失败、行丢失、或出现 `Reclaim failed`——说明中断 checkpoint 的收尾或提交边界有实质缺陷，返回 Plan（不得改 checkpoint 顺序或 WAL 格式来迁就）。

### 2.2: 显式事务内的语句被放弃后事务仍然活跃

- Requirement/Scenario: 显式事务继续还是回滚由调用方决定 / 显式事务内的语句被放弃后事务仍然活跃、调用方回滚撤销被取消语句的效果、调用方提交保留被取消语句的效果
- Depends on: 2.1
- Targets: `tests/statement_cancellation_test.rs`（新增用例 `abandoned_statement_in_explicit_tx_keeps_tx_open`）
- Current behavior: 显式事务的语句路径（`src/pipeline.rs:1258`）不 abort、不 commit、不建守卫、不触发回收——语句 future 被放弃时事务状态不变，已写入的部分留在事务内。**当前没有用例证明这一点**。
- Required behavior: 新增用例 `abandoned_statement_in_explicit_tx_keeps_tx_open`，覆盖规格的三个场景：
  1. **事务仍然活跃且可用**：`BEGIN` → 在显式事务内执行一条会写很多行的语句 → 锚点命中（事务版本数 ≥2）即 `drop(fut)` 放弃语句 future → 断言该事务**仍在** `active_transactions()` 中；该事务**仍可用于后续语句**（在同事务内再执行一条写语句成功）；库**没有**代替调用方提交或回滚（事务外读不到该语句已写入的行，事务内读得到）。
  2. **调用方回滚撤销被取消语句的效果**：在上述现场上调用 `Database::rollback(tx)` → 断言该行在事务外不存在、事务结束（活跃集为空）。
  3. **调用方提交保留被取消语句的效果**：另起一个显式事务重复「放弃 → 提交」→ 断言该行持久存在，`close()` 后重开仍在。
- Preserve: 显式事务的生死仍由调用方决定；`Database::rollback` 的既有语义与错误面不变；`TransactionManager::abort` / `abort_by_id` 的既有语义不变；回收机制与登记表语义不变（显式事务永远不进登记表）。
- Forbidden: 为了让用例通过而让库在取消路径上提交或回滚显式事务；改 `execute_stage_in_tx_routed` 的语句失败语义；新增取消或 deadline 公共 API；用文件字节比较代替库入口的可观察结果；用超时阈值或固定 sleep 判定「放弃已发生」。
- First test: 在 `tests/statement_cancellation_test.rs` 新增用例 `abandoned_statement_in_explicit_tx_keeps_tx_open`。夹具：`setup(&dir)`；`let tx = db.begin().await.unwrap();` → `Box::pin(db.execute_in_tx("INSERT INTO t2 SELECT id FROM t", &tx))` → 用 `tokio::select!`（`biased`，锚点支在前）在 `anchor_tx(&db)` 命中时 `drop(fut)`。命令：`cargo test --test statement_cancellation_test abandoned_statement_in_explicit_tx_keeps_tx_open -- --test-threads=1`。
  - **本任务是表征用例**（行为当前已成立，见 Current-State Evidence），因此**不要求修复前 RED**。必须在 Act Response 中记录「首次运行即通过」的基线并说明理由（覆盖缺口分支，非新功能分支）——不得为制造 RED 改产品代码。
- GREEN condition: 该用例通过；`cargo test --test statement_cancellation_test -- --test-threads=1` 退出码 0（六个用例全部通过）。
- Verification: 同一条命令；本用例的失败原因与既有用例互不相同（它看的是「事务是否仍活跃可用」与「回滚 / 提交对已取消语句效果的影响」，不是活跃集是否被回收清空、不是落盘、不是并发、不是回收报错）。
- Stop when: 该用例的通过需要库在取消路径上代替调用方收尾显式事务——那是与 requirement 相反的行为，返回 Plan；或需要改 `Database::rollback` / `abort` 的既有语义——返回 Plan。

### 2.3: 超时不代替回滚

- Requirement/Scenario: 超时不代替回滚 / 显式事务内的语句超时后事务仍然活跃；取消触发集合是语句 future 被放弃 / 丢弃、超时与断开是同一类事件（超时支）
- Depends on: 2.2
- Targets: `tests/statement_cancellation_test.rs`（新增用例 `timeout_in_explicit_tx_is_not_a_rollback`）
- Current behavior: 超时是语句 future 被放弃的另一种来源（规格原文：丢弃、超时与断开在库入口是同一类事件）。显式事务路径不因语句结束而收尾事务，因此超时到期后事务应仍然活跃、行仍在事务内。**当前没有用例证明这一点**。
- Required behavior: 新增用例 `timeout_in_explicit_tx_is_not_a_rollback`：`BEGIN` → 用 `tokio::time::timeout` 包住显式事务内的一条写语句并让超时到期 → 断言：(a) 该语句被放弃（超时返回 `Elapsed`）；(b) 该事务**仍然活跃**；(c) 该行**仍在事务内**（事务内读得到、事务外读不到）；(d) 只有调用方显式回滚才撤销它（回滚后事务外读不到、事务结束）；(e) 调用方显式提交则该效果持久（`close()` 后重开仍在）。
- Preserve: 超时只是对语句的放弃，库在任何取消路径上都不代替调用方提交或回滚显式事务；`Database::rollback` / `commit` 的既有语义与错误面不变；库入口不新增取消或 deadline 公共 API。
- Forbidden: 把超时时长当作通过条件或判据（超时只是**取消触发源**，所有断言必须落在可观察状态上）；用固定 sleep 阈值判定「放弃已发生」；让库在超时路径上代替调用方收尾显式事务；新增库内 timeout / deadline 公共 API；用耗时对比作断言。
- First test: 在 `tests/statement_cancellation_test.rs` 新增用例 `timeout_in_explicit_tx_is_not_a_rollback`。夹具：`setup(&dir)`；`let tx = db.begin().await.unwrap();` → `tokio::time::timeout(<时长>, db.execute_in_tx("INSERT INTO t2 SELECT id FROM t", &tx))`。另起一个 `tokio::spawn` 的锚点观察任务（只克隆 `Database`，`Transaction` 不是 `Clone` 故不进任务），轮询 `anchor_tx(&db)` 命中后置一个 `AtomicBool` 并停止。超时时长取「远小于该语句完成所需时间、且远大于语句推进到第 2 个版本所需时间」的值（语句约需数秒，锚点在毫秒级命中——时长按此量级选取，**不作为判据**）。断言超时返回 `Elapsed`、锚点观察任务的标志为真（**超时到期前语句已推进**，否则显式失败——夹具规模不对，不是产品结论）、以及 (b)-(e) 四条。命令：`cargo test --test statement_cancellation_test timeout_in_explicit_tx_is_not_a_rollback -- --test-threads=1`。
  - **本任务是表征用例**（行为当前已成立），**不要求修复前 RED**；必须在 Act Response 记录首次运行即通过的基线与理由，不得为制造 RED 改产品代码。
- GREEN condition: 该用例通过；`cargo test --test statement_cancellation_test -- --test-threads=1` 退出码 0（七个用例全部通过）。
- Verification: 同一条命令；本用例的失败原因与既有用例互不相同（它看的是「超时这一取消来源与事务存活的因果」，与放弃来源的用例在断言面上不重叠）。
- Stop when: 该用例的通过需要库在超时路径上代替调用方收尾显式事务——与 requirement 相反，返回 Plan；或需要新增库内 timeout / deadline 公共 API——命中 MS34 拆分信号，返回 Plan。

### 2.4: 被放弃的语句之后会话继续可用

- Requirement/Scenario: 取消之后后台 I/O 按会话级生命周期收尾 / 被放弃的语句之后会话继续可用
- Depends on: 2.3
- Targets: `tests/statement_cancellation_test.rs`（新增用例 `session_keeps_working_after_abandoned_statement`）
- Current behavior: 会话级对象（WAL writer、WAL buffer、缓冲池）由 `open_internal` 构造，不随语句生死；语句级只读预取任务自然终止。放弃一条语句后，同一会话应继续正常写入并提交，`close` 后重开数据完整。**当前没有用例把「放弃之后的会话可用性」与「后台 I/O 的会话级生命周期」合在一起证明**。
- Required behavior: 新增用例 `session_keeps_working_after_abandoned_statement`：`setup(&dir)` → 放弃一条隐式语句（沿用既有 `abandon_statement` 夹具）→ **同一会话**继续写入若干行并全部提交成功 → `close()` → 重开同一个库 → 断言：后续写入的行完整可读；被放弃语句写入的行**不存在**；`rtsql::storage::integrity::check_integrity(&db.buffer_pool, &db.table_manager)` 无 `FindingSeverity::Error` 级发现。
- Preserve: 会话级对象的生命周期不变（语句取消不拆 WAL writer / WAL buffer / 缓冲池）；checkpoint 九步与 WAL 语义不变；`close()` 的既有语义不变。
- Forbidden: 为让用例通过而新增任何「取消后收尾」逻辑（design D5 明确本轮不新增后台收尾逻辑）；改 checkpoint 让它跳过某些脏页；改 WAL 格式或位点布局；用文件字节比较代替库入口的可观察结果；把「后台 I/O 收尾」写成需要探针或内部钩子才能断言的形式。
- First test: 在 `tests/statement_cancellation_test.rs` 新增用例 `session_keeps_working_after_abandoned_statement`。夹具：`setup(&dir)`；`abandon_statement(&db, ABANDONED_SQL).await;` → 继续写入 → `db.close().await.unwrap();` → `drop(db);` → 重开 → 断言。命令：`cargo test --test statement_cancellation_test session_keeps_working_after_abandoned_statement -- --test-threads=1`。
  - **本任务是表征用例**，**不要求修复前 RED**；必须在 Act Response 记录首次运行即通过的基线与理由。
- GREEN condition: 该用例通过；`cargo test --test statement_cancellation_test -- --test-threads=1` 退出码 0（八个用例全部通过）。
- Verification: 同一条命令；本用例的失败原因与既有用例互不相同（它看的是「放弃之后的会话可用性与重开后的完整性」，1.3 看的是「未提交数据不落盘」）。
- Stop when: 该用例的通过需要新增取消后的后台收尾逻辑或改动 checkpoint / WAL——返回 Plan。

### 2.5: M19 分批串行全量回归（收尾验证，整个 change 一次）

- Requirement/Scenario: 不直接映射 requirement——它是整个 change 的收尾验证（Iteration 001 的 Stable result 末项「全量分批回归 0 failed」）
- Depends on: 2.4
- Targets: 仓库内全部测试目标（`src/` 的单元测试 + `tests/*.rs` 的集成测试）
- Current behavior: MS33 收尾的全量终态是 2155 / 0 / 2（`.agents/docs/SNAPSHOT.md`）——**变更前基线**。本次改动覆盖 `src/database.rs` / `src/pipeline.rs` / `src/executor/alter_table.rs` / `src/transaction/manager.rs` 的执行面，全量结论**不因定向目标全绿而成立**，必须在最终代码状态下重跑。
- Required behavior: 按 M19 配方做全量分批回归，**严格串行**：
  1. `--lib --bins` 一批：`cargo test --lib --bins -- --test-threads=1`。
  2. `tests/*.rs` 按**文件名字典序**分组，每批 **8–12 个测试目标**；每批一次独立调用：`cargo test --test <f1> ... --test <fn> -- --test-threads=1`。
  3. **批间顺序执行，同一时刻只有一批在跑**（不得并行起多批）。
  4. 每批退出码为 **0** 且输出中 `test result:` 的 **failed 计数为 0**。
  5. 记录**每批**的 passed / failed 计数，与「变更前的目标清单」做覆盖对账（**不重不漏**：每个 `tests/*.rs` 恰好落在一个批次里；`--lib`/`--bins` 单独一批）。
  6. 与变更前基线 2155 / 0 / 2 对账：新总数 = 2155 + 本次新增用例数（`tests/statement_cancellation_test.rs` 的用例数），failed 仍为 0，ignored 数与基线一致。
- Preserve: 批内 `--test-threads=1`（禁止默认并行度）；每批一次独立调用（禁止把多批合并成一次无目标过滤的全量调用）；不修改任何被测代码来让全量通过。
- Forbidden: 一次无目标过滤的 `cargo test` 全量调用；批内使用默认并行度；多批并行执行；跳过覆盖对账；用「大部分目标全绿」代替「每批 failed 计数为 0」的逐批结论；为通过全量而改产品代码或放宽既有断言。
- First test: 不适用（收尾验证任务，无先行测试）。其判据即 Required behavior 的 6 条。
- GREEN condition: 每一批退出码 0 且 failed 计数 0；覆盖对账显示无重无漏；新总数与「变更前基线 + 本次新增用例数」吻合。
- Verification: 逐批记录「批次号 / 目标清单 / 退出码 / passed / failed」；在 Act Response 给出对账表与总和。若某批 failed 非 0 → 该批必须单独定位并按 Stop when 处理，**不得**用其他批次的通过掩盖。
- Stop when: 某批出现 failed 且根因在本次改动范围内 → 按 Gate 5 处理（同一问题三次失败则反思）；根因在本次改动范围外 → 按 Act Response 的 Issue 候选规则**只报告不登记**；批次划分因目标数不足 8 个或超过 12 个需要调整时，在 Act Response 记录实际划分与理由（最后一不足 8 个的尾批除外），**不得**因此合并成无目标过滤的全量调用。

**Invariants**

- 显式事务的生死仍由调用方决定：任何取消路径（丢弃、超时、断开）都不提交、不回滚调用方的事务；显式事务不进登记表、也不被回收。
- 取消触发集合是语句 future 被放弃：丢弃、超时与断开在库入口是同一类事件，库不按触发来源给出不同的事务结果。
- 未发生取消的语句行为不变（含同一句柄上的并发语句）。
- 库入口不新增公共 API（`Executor` 接口不变；不引入协作式取消上下文或 deadline 公共 API）。
- WAL 记录格式、位点布局、checkpoint 九步顺序不变。
- 提交前被取消的语句不提交；被放弃事务永远不会以「已提交」的面貌出现在恢复期（没有提交帧的事务在恢复期仍被标记为中止）。
- 「已提交但调用方未收到结果」的事务不被回收误伤（`reclaim_implicit_tx` 的「不在活跃集 → 跳过」守卫保留）。

**Non-goals**

- 协作式取消上下文与 deadline 公共 API、完整分级诊断（I096）、真实断电与 `kill -9` 语义、WAL 格式与 checkpoint 九步、CLI 信号停机行为。
- 语句并发模型的改造（不引入全局执行闸门，不改变同一句柄上语句的并发语义）。
- 被放弃的 ALTER TABLE DROP COLUMN 的**数据面语义**（目录已窄化 + 数据面混合宽度的后果）——change 范围外的实质问题，只报告不登记。
- Iteration 000 遗留的四条非阻塞 Minor（三处触发点的回收错误面不一致、`abort_by_id` / `discard_tx_version_tables` 的 `pub` 可见性、守卫 `Drop` 的 `Mutex::lock().unwrap()` 中毒面、checkpoint 只回收主库登记表）——本 Iteration 不返工。
- MS35 / MS36。

**Acceptance**

- 2.1：`committed_statement_survives_undelivered_result` 通过——提交已完成但结果未送达时，该行在同一会话读得到、`close()` 后重开仍在、活跃集为空、后续写语句正常成功；用例在「放弃点未落在提交完成之后」时显式失败（不得静默通过）。
- 2.2：`abandoned_statement_in_explicit_tx_keeps_tx_open` 通过——被放弃的显式事务内语句之后，事务仍活跃且可用、库未代替调用方收尾、调用方回滚撤销其效果、调用方提交保留其效果。
- 2.3：`timeout_in_explicit_tx_is_not_a_rollback` 通过——超时是对语句的放弃，事务仍活跃、行仍在事务内，只有显式回滚才撤销它。
- 2.4：`session_keeps_working_after_abandoned_statement` 通过——放弃之后同一会话继续写入并提交、`close` 后重开数据完整、被放弃语句的行不存在、完整性检查无 Error 级发现。
- 2.5：M19 分批串行全量回归每批退出码 0、failed 计数 0，覆盖对账无重无漏，总数与「变更前基线 2155 + 本次新增用例数」吻合。
- `cargo test --test statement_cancellation_test -- --test-threads=1` 退出码 0（八个用例全部通过，failed 计数 0），且八个用例的失败原因互不相同、无身份证明机制、无项目测试框架外的判定层。

**Verification**

全部从库入口观察（`Database::open` / `execute_sql` / `execute_in_tx` / `begin` / `commit` / `rollback` / `close` / 重开）与公开的 `TransactionManager::active_transactions()`、`get_tx_versions()`、`rtsql::storage::integrity::check_integrity`；并发与放弃的观察点为事务集合、事务版本数与语句结果。不使用哈希、指纹、run-id、位点 pin、manifest、时间顺序等身份证明机制，不在本项目测试框架之外建判定层。锚点轮询用 `yield_now` 让出执行权，`tokio::time::timeout` 只作夹具失控兜底（触发即判失败，不是阈值判定）；2.3 的超时是规格场景给定的**取消触发源**，不作为任何断言的判据。**本 Cycle 跑全量回归**——高开销验证按 change 预算，整个 change 一次，作为收尾验证落在任务 2.5（M19 分批串行）。

**Gate 2 Readiness**

PASS。范围、requirement、依赖、验收边界与验证方式均已确定；五个 Task Definition 的 Targets、Current/Required behavior、Preserve/Forbidden、First test、GREEN condition、Verification 与 Stop when 齐备；RTM 无 `Missing`、无 `Simplified`；Persisted Evidence 模式为 `none`（全部验证均为可低成本重跑的定向命令，收尾全量按 M19 分批执行，无一次性环境即将消失的现场，无摘要会丢失决定性结构的长输出）。用户批准依据：2026-10-24 用户原话「/openspec-autopilot 把已经规划的三个ms逐步完成，要求遵守各种规范，尤其是全量测试时候的多批次串行」与补充「因为这个项目有点大，三个ms也是串行实施就好」——goal 指令构成目标范围内阶段推进的显式授权，本 Iteration 在该授权范围内且不扩大 change 范围。无 SKIPPED、无 BLOCKED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- **2.1 的窗口工程是本 Iteration 的主要技术风险**：阈值设为 0 时「提交已完成但结果未送达」窗口在 `active.remove` 与 `Database::commit` 返回之间**没有 await 点**，不可达；必须用 `set_checkpoint_bytes_threshold(1)` 把尾段拉宽成一个真 checkpoint。若提交后自动 checkpoint 因 R3 顺延（存在其他活跃事务）而未触发，窗口同样收窄——用例的显式事务场景里提交后活跃集为空，不受顺延影响。
- **2.1 的放弃动作会中断一次进行中的 checkpoint**：`close()` 随后必须能完成一次完整 checkpoint 并保持数据完整（提交帧在放弃点之前已 fsync，恢复期语义由 WAL 兜住）。若重开后该行缺失或 `close()` 报错，按 Stop when 返回 Plan——不得改 checkpoint 步骤顺序或 WAL 格式来迁就。
- **Iteration 000 Review 的 F8（窄提交中断窗口）在本 Iteration 承接**：`TransactionManager::commit` 中 `commit_mark_versions` 完成后、`active.remove` 之前有一个 await 点，future 在此被放弃时守卫会登记该 tx id，而该事务**仍在活跃集**内，后续回收会中止它。对**显式事务**该窗口不构成风险（显式事务从不进登记表）；对**隐式事务**而言，该窗口对应「提交已原子完成但结果未送达」——此时 tx 已离开活跃集，回收的「不在活跃集 → 跳过」守卫会跳过它，因此 2.1 的显式事务场景不会被它影响。本 Iteration 不为隐式事务的该窗口新增用例（不在 change 的 requirement 范围内），但若 2.1 的实现过程中发现它影响可观察结论，须在 Act Response 报告。
- **表征用例的 RED 纪律**：2.2 / 2.3 / 2.4 预期首次运行即通过（行为当前已成立，见 Current-State Evidence）。这是 TDD 的「重构 / 表征」分支，不是跳过先行测试——先行测试先行存在、结论以实跑记录为准；**不得**为制造 RED 改产品代码。
- **2.3 的超时时长不是判据**：它只是让取消发生的触发源。断言全部落在可观察状态（超时返回 `Elapsed`、事务仍活跃、行在事务内、回滚才撤销）。锚点观察任务用于确认「超时到期前语句已推进」，避免把「语句根本没跑起来就超时了」误判为通过。
- **夹具成本**：既有 `setup()` 的 2000 行单行插入约 6 秒/用例；本 Iteration 新增四个用例（2.1/2.2/2.3/2.4），合计准备成本约 24 秒量级，加上 2.5 的分批全量，本 Iteration 的验证预算以全量批次为主。可接受，不要求优化。
- **2.5 的执行纪律是用户显式强调项**：分批、串行、批内 `--test-threads=1`、逐批退出码与 failed 计数、覆盖对账——任何一条缺失都不构成完成声明。
- 工作区里已有的 `.agents/docs/tasks.md` 修改（MS34–MS36 转 ready）不属于本 Cycle，予以保留不改。

## Act Response

- Status: reported

### 实际改动

唯一改动文件：`tests/statement_cancellation_test.rs`（**产品代码零改动**——`src/` 未被本 Cycle 修改）。

| 符号 | 性质 | 归属 |
|---|---|---|
| `setup_with_isolation(&TempDir, IsolationLevel)` | 新增夹具 | 2.2 / 2.3 |
| `setup(&TempDir)` | 改为 `setup_with_isolation(.., RepeatableRead)` 的等价包装（`Database::open` 与 `open_with_isolation(RepeatableRead)` 同义，`src/database.rs:192`） | 2.2 / 2.3 |
| `expect_affected_in_tx` / `expect_rows_in_tx` | 新增夹具 | 2.2 / 2.3 |
| `await_anchor_tx(&Database)` | 新增夹具（复用 `anchor_tx` 判据 + `yield_now` 轮询 + 10s 失控兜底） | 2.2 / 2.3 |
| `TIMEOUT_CANCEL_WINDOW`（20ms） | 新增常量（取消触发源，非判据） | 2.3 |
| `committed_statement_survives_undelivered_result` | 新增用例 | 2.1 |
| `abandoned_statement_in_explicit_tx_keeps_tx_open` | 新增用例 | 2.2 |
| `timeout_in_explicit_tx_is_not_a_rollback` | 新增用例 | 2.3 |
| `session_keeps_working_after_abandoned_statement` | 新增用例 | 2.4 |
| 模块文档头 | 补记 Iteration 001 的四个用例与 RC 夹具说明 | 2.1–2.4 |

### 验证命令与实际结果

| 任务 | 命令 | 实际结果 | 类型 |
|---|---|---|---|
| 2.1 | `cargo test --test statement_cancellation_test committed_statement_survives_undelivered_result -- --test-threads=1` | `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 4.81s` | **首次行为运行即 GREEN**（表征型，理由见 Deviations 1） |
| 2.2 | `cargo test --test statement_cancellation_test abandoned_statement_in_explicit_tx_keeps_tx_open -- --test-threads=1` | `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 5.67s` | 表征型 GREEN（首次运行是 RED，缺陷 E2，见 Deviations 3 与自检 2） |
| 2.3 | `cargo test --test statement_cancellation_test timeout_in_explicit_tx_is_not_a_rollback -- --test-threads=1` | `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 4.33s` | 表征型 GREEN（同一命令另跑 7 次全绿，合计 8/8） |
| 2.4 | `cargo test --test statement_cancellation_test session_keeps_working_after_abandoned_statement -- --test-threads=1` | `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 4.93s` | **表征型首次 GREEN** |
| 2.1–2.4 GREEN | `cargo test --test statement_cancellation_test -- --test-threads=1` | `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 39.54s` | 八个用例全通过（2.1–2.4 的 GREEN condition：退出码 0、failed 计数 0） |
| 2.5 | M19 分批串行（见下表），13 批 | 每批退出码 0、failed 计数 0；合计 **2163 passed / 0 failed / 2 ignored** | 收尾全量回归 |

**2.5 分批明细**（`--lib --bins` 单独一批；`tests/*.rs` 按文件名字典序、每批 11 个目标（M19 的 8–12 区间内），批间顺序执行、同一时刻只有一批在跑、批内 `--test-threads=1`；每批一次独立调用）：

| 批次 | 目标（文件名字典序） | 测试二进制数 | passed | failed | ignored | 退出码 |
|---|---|---|---|---|---|---|
| 00 | `--lib --bins` | 2 | 420 | 0 | 0 | 0 |
| 01 | aggregate_extension_test, aggregate_output_shape_test, aggregate_test, alter_table_test, attach_session_test, btree_merge_test, btree_scale_test, btree_split_test, btree_test, busy_timeout_test, cache_perf_test | 11 | 149 | 0 | 0 | 0 |
| 02 | catalog_durability_test, chain_join_test, check_command_test, checkpoint_command_test, checkpoint_redo_reduction_test, checkpoint_skip_test, checkpoint_test, cli_test, concurrent_test, connection_limit_test, constraint_enforcement_test | 11 | 210 | 0 | **2** | 0 |
| 03 | cross_db_dml_test, cross_db_query_test, ctas_test, cte_test, database_file_lock_test, datetime_function_test, datetime_type_test, ddl_crash_recovery_test, default_declaration_test, default_precheck_test, derived_scan_alias_test | 11 | 163 | 0 | 0 | 0 |
| 04 | dml_tx_id_test, drop_persistence_test, drop_table_free_test, e2e_test, encryption_test, executor_test, explain_test, explicit_tx_test, expression_e2e_test, file_header_test, file_storage_io_test | 11 | 169 | 0 | 0 | 0 |
| 05 | full_table_dml_test, gc_test, group_by_expr_test, import_input_surface_test, index_manager_test, insert_column_list_test, insert_select_test, isolation_level_test, join_output_resolution_test, join_test, key_type_conformance_test | 11 | 103 | 0 | 0 | 0 |
| 06 | keyless_eq_routing_test, keyless_row_test, limit_test, memory_db_test, mvcc_abort_test, mvcc_commit_test, mvcc_record_test, mvcc_tombstone_visibility_test, nested_loop_join_test, network_protocol_test, network_server_test | 11 | 75 | 0 | 0 | 0 |
| 07 | no_from_select_test, order_by_alias_sort_test, outer_join_test, overflow_page_test, param_binding_test, parser_test, pg_integration_test, pg_messages_test, pg_protocol_test, pipeline_test, plan_cache_test | 11 | 143 | 0 | 0 | 0 |
| 08 | plan_exec_test, planner_test, predicate_test, prefetch_test, prepared_statement_test, projection_expression_test, projection_test, pushdown_test, query_memory_test, readonly_session_test, recovery_e2e_test | 11 | 222 | 0 | 0 | 0 |
| 09 | recovery_error_surface_test, recovery_replay_convergence_test, recovery_tail_persistence_test, recovery_test, recovery_wal_convergence_test, runtime_test, scalar_function_test, scalar_subquery_comparison_test, schema_persistence_test, secondary_index_equality_test, secondary_index_test | 11 | 107 | 0 | 0 | 0 |
| 10 | select_distinct_test, self_join_test, set_operations_test, sort_test, sql_differential_test, **statement_cancellation_test**, storage_test, structural_property_test, subquery_alias_binding_test, subquery_test, subset_insert_test | 11 | 168 | 0 | 0 | 0 |
| 11 | sync_loader_test, table_alias_resolution_test, table_manager_test, truncate_test, tx_statement_test, typed_api_test, update_expression_test, update_index_maintenance_test, upsert_test, value_test, version_chain_test | 11 | 151 | 0 | 0 | 0 |
| 12 | view_test, visibility_test, wal_auto_checkpoint_test, wal_buffer_test, wal_handle_test, wal_integration_test, wal_record_test, wal_recovery_large_test, wal_writer_test, write_type_conformance_test | 10 | 83 | 0 | 0 | 0 |
| **合计** | **13 批（1 + 11×11 + 10）** | **132** | **2163** | **0** | **2** | **全 0** |

- **覆盖对账（不重不漏）**：变更前 `tests/` 下 130 个已跟踪测试文件 + 本 change 新增的 `tests/statement_cancellation_test.rs` = 131 个目标；分批清单逐项比对结果为 **missing 0 / extra 0 / duplicate 0**（每个文件恰好落在一个批次），`--lib --bins` 单独成批。每批的 `test result:` 行数 = 该批目标数（批 00 为 lib + bin 两个二进制），无目标被 cargo 中止跳过。
- **与变更前基线对账**：MS33 收尾终态 **2155 / 0 / 2**（`.agents/docs/SNAPSHOT.md`）；本次新增总数 **2163 / 0 / 2**。差额 **+8** = 本 change 新增文件 `tests/statement_cancellation_test.rs` 的 8 个用例（Iteration 000 的 4 个 + 本 Iteration 的 4 个）——与 Task Definition 2.5 第 6 条的公式「2155 + 该文件用例数」一致。failed 仍为 0；ignored 仍为 2（`cli_test.rs` 的长期 `#[ignore]` 用例，与基线一致，批次 02）。
- 批次划分理由：131 个 `tests/*.rs` 目标按 11 个一批得 12 批（11×11 + 10），全部落在 M19 的 8–12 区间内，无尾批不足 8 个；加 `--lib --bins` 一批共 13 批（目标数比 MS33 实例的 131 多 1，按 Runbook「目标数变化后按同一分组规则重算批数」处理）。

### Deviations from Plan

1. **2.1 的 RED 判据未成立——首次行为运行即 GREEN（Plan 错误）**。Task Definition 要求「必须先观察到 RED（行在重开后不存在 / 行在同一会话读不到）」，但同一份 Plan Context 的 Current-State Evidence 已写明「`Database::commit` 的原子效果（提交帧 fsync + 版本标 committed + 移出活跃集）先于其返回 → 目标行为当前已成立」，两者互斥。实测：窗口工程三件（阈值 1 / 锚点 / 判别位）全部生效——锚点在 commit future **尚未返回**时命中（判别位 `commit_returned` 为假，否则用例会 panic），放弃点落在自动 checkpoint 的步骤 1（`CheckpointManager::checkpoint` 的 `get_current_lsn()` 挂起点，`src/wal/checkpoint.rs:185`），此前提交帧已 fsync、版本已标 committed、tx 已出活跃集；该行在同一会话读得到、`close()` 后重开仍在、活跃集为空、后续写语句正常成功。**未改任何产品代码去制造 RED**（按 Task Definition 的禁止项与 AGENTS.md 的表征分支处理）。分类：**Plan 错误**（RED 判据与自身 Current-State Evidence 冲突）。
2. **2.2 / 2.3 的夹具以 `ReadCommitted` 打开，而不是 `setup(&dir)` 的 RepeatableRead**。Plan 的断言基础「事务自身的写经 `is_visible_self` 在事务内可见，事务外的读看不到未提交的写」只在 RC 下可观察：RR 的 `Database::statement_snapshot` 返回 `None`（`src/database.rs:477-489`），扫描路径不做可见性过滤（`src/executor/data_scan.rs:485-499` 的可见性门只在 `snapshot` 为 `Some` 时生效），未提交行在事务内外都读得到。夹具改为 `Database::open_with_isolation(path, IsolationLevel::ReadCommitted)`（公开 API，无产品改动）；其余断言与 Plan 一致。
3. **2.2 场景 3 与 2.3 的 (e) 用第二个写入目标 `t3`**（场景形状不变：显式事务内放弃一条多行 INSERT…SELECT → 提交 → 效果持久 → `close()` 后重开仍在）。原因：场景 2 的回滚之后，被取消语句写入的键（**第一个键之后**）仍留在主键索引的正向条目里，同表同键的第二次放弃/超时会撞 `DuplicateKey`（实测证据见 Experience Candidates E2）。该残留与本场景要证明的「提交保留被取消语句的效果」无关，故换目标表，不依赖缺陷路径。
4. **2.3 的超时窗口取 20ms**（Plan 要求「按量级选取，不作为判据」）。实测 100ms 窗口（取消前写入约 523 行）会命中 Experience Candidates **E1（进程崩溃）**；20ms 窗口取消前写入约 100 行，同一命令 8 次运行全部通过。窗口只决定取消何时发生，全部断言落在可观察状态上（超时返回 `Elapsed`、锚点观察任务确认「超时到期前语句已推进」、事务仍活跃、行在事务内、回滚才撤销、提交则持久）。
5. **2.4 的后续写入键取 9001 / 9002 / 9003**（落在被放弃语句写入的 1..2000 之外），原因同偏差 3（E2 的残留条目使同键再插撞 `DuplicateKey`）。

### 自检发现、已修复内容与遗留 Minor

| # | 发现 | 处理 |
|---|---|---|
| 1 | 2.1 首次编译失败：锚点 future 借用 `db`（`drop(db)` 时借用未释放） | 用例内修正（放弃 commit future 前 `drop(anchor)`），非产品代码问题；非行为 RED |
| 2 | 2.2 首次运行 RED：`the statement finished before the abandon anchor was reached: Err(Duplicate key)` | 定位为缺陷 E2（回滚后残留主键索引条目），不是用例写错；按偏差 3 换目标表后 GREEN |
| 3 | 2.3 首次运行失败于「the statement must have advanced before the timeout fired」 | 锚点观察任务未置位 `advanced` 标志（用例自身缺陷）→ 修正观察任务后 GREEN |
| 4 | 2.3 在 100ms 窗口下进程崩溃（stack overflow → SIGABRT） | 定位为缺陷 E1（索引层），本 Cycle 不修（范围外）；按偏差 4 缩窗口规避并如实报告 |
| 5 | 2.4 首跑即 GREEN | 表征型用例的预期基线，无修复 |

- **遗留 Critical/Important：无**（产品代码零改动；四个新用例的断言互不重叠：2.1 看「提交已完成但未送达」后的行是否仍在，2.2 看「事务是否仍活跃可用 + 回滚/提交对已取消语句效果的影响」，2.3 看「超时这一取消来源与事务存活的因果」，2.4 看「放弃之后的会话可用性与重开后的完整性」）。
- **遗留 Minor**：无（本 Cycle 未新增产品代码路径，无新增警告——`cargo fmt --check` 干净，`cargo clippy --tests` 在本文件无告警）。
- **Iteration 000 遗留事实的承接**：Plan 的 Risks 提到 `TransactionManager::commit` 里 `commit_mark_versions` 与 `active.remove` 之间的窄窗口。2.1 的锚点是「tx 离开活跃集」，命中点在该窗口**之后**（自动 checkpoint 的步骤 1），故该窗口不影响 2.1 的可观察结论，本 Iteration 不为其新增用例（不在 change 的 requirement 范围内）。

### Persisted Evidence

- None required：全部验证均为可低成本重跑的定向命令（2.1–2.4 单用例与整文件）与 M19 分批全量（逐批计数已在上面完整记录）；无一次性环境即将消失的现场，无摘要会丢失决定性结构的长输出，未创建 `evidence/` 目录。

### Experience Candidates

> 格式按用户本轮约定（「不要只做等级分类，必须给出可承载的完整信息」）。两条均为**本 change 范围外**的实质缺陷，只报告、不登记、不修复——登记与修复由 Recorder 与后续 change 承担。

**E1（严重：进程崩溃 / SIGABRT）**

- **一句话结论**：在显式事务里取消一条正在执行的多行 `INSERT … SELECT`，随后对该事务调用回滚，会让 B 树结构损坏——之后的一次索引插入沿「内部节点的子指针 = 0」无限下降，栈溢出使整个进程 abort。
- **根因位置**：`src/storage/btree/btree.rs:327`（`BTree::insert_into_page` 的内部节点下降递归）与 `src/storage/btree/btree.rs:243`（叶子/内部判定 `data_guard[0] == LEAF_NODE`）；损坏的入口是 `src/storage/btree/index_manager.rs:239-257`（`IndexManager::insert`：B 树条目在 `spawn_blocking` 内写入，反向映射 `row_to_key` 在其后的 await 之后才写）与 `src/storage/btree/index_manager.rs:261-282`（`IndexManager::delete`：回滚路径删条目时会合并/重分布叶子并可能换根）。被取消语句的索引写入在 `spawn_blocking` 里**不会被取消**（tokio 的 blocking 任务跑到底），它与回滚路径的索引删除/合并并发作用于同一棵 B 树，而 B 树的根页 id 与页内容没有跨写者的串行化。
- **可复现路径 / 已观察到的证据**：`cargo test --test statement_cancellation_test timeout_in_explicit_tx_is_not_a_rollback`（超时窗口 100ms 时连续两次复现）。gdb 现场：`thread 'tokio-rt-worker' has overflowed its stack` → SIGSEGV；栈上是 339+ 层 `BTree::insert_into_page`（`btree.rs:327`），帧内 `child_page_id_u32 = 0`（内部节点的子指针为 0 → 反复下降到第 0 页）；崩溃的那次插入的 `key = [0,0,0,0,0,0,0,111]`（即被取消语句写入的键 111）、`BTree::root_page_id = 75`，而该行的 `RowId.page_id` 也是 75（数据页）——根页 id 指向了一个不是 B 树节点的页。
- **影响面与触发条件**：触发条件 = 「一条会写多行的 INSERT…SELECT 在索引写入中被取消」+「该事务随后被回滚（回滚要删这些行的索引条目）」。用户可见后果是**整个进程崩溃**（不止当前语句失败），且崩溃点与取消无关、出现在之后的任意索引写入上——同一句柄上任何后续写语句都可能踩到。取消窗口越大越容易触发（100ms 窗口取消前约 523 行 → 稳定复现；20ms 窗口约 100 行 → 本机 8 次未复现）。同一根因也可能命中隐式事务的回收路径（回收同样调用 `abort_by_id` → 同一套删除/合并逻辑）。
- **与本 change 范围的关系**：超出本 change 范围——修复点落在存储/索引层的写者串行化（以及取消时 B 树根页 id 与反向映射的原子性），不在「取消/超时/事务收尾」的调用方语义面；Iteration 000/001 的 Plan 都明确不改 checkpoint/WAL/索引语义，Task Definition 2.2 的 Stop when 也规定「需要改 `Database::rollback` / `abort` 的既有语义 → 返回 Plan」。本 Cycle 按规则只报告不修。
- **建议修复方向**：① 给每个索引的**结构变更**（插入/删除/合并/换根）加同一条写者串行化路径（例如按 `IndexManager` 实例的写锁），使「B 树条目写入 + 反向映射写入 + 根页 id 更新」对其他写者原子可见；② 或让取消路径不留下「B 树已写、反向映射未写」的状态（把反向映射写入与 B 树写入放进同一次不可中断的临界区，并让根页 id 更新与 B 树变更同临界区）；③ 回归用例：取消一条多行 INSERT…SELECT → 回滚 → 之后同句柄上的任意写语句不得崩溃，且 `check_integrity` 无 Error 级发现。

**E2（中等：回滚后键仍被占用）**

- **一句话结论**：显式事务内一条多行 INSERT…SELECT 被取消后调用方回滚，被取消语句写入的键（**第一个键之后**）仍留在主键索引的正向条目里，同一个键再插入报 `DuplicateKey`。
- **根因位置**：`src/storage/btree/index_manager.rs:239-257`（`IndexManager::insert`：`btree.insert` 在 `spawn_blocking` 内完成，而反向映射 `row_to_key.insert` 在其后的 await 之后才写——取消落在两者之间时，正向条目 key→row_id 已在、反向映射 row_id→key 缺失）与 `src/transaction/manager.rs:344-355`（`abort_cleanup_versions` 的 A/B 分桶只查反向映射 `find_key_by_row_id`；反向映射缺失的行被误判为「墓碑」而跳过，其正向索引条目不被移除）。
- **可复现路径 / 已观察到的证据**：2.2 用例首版（同表同键重复「放弃 → 提交」）实测 RED：`the statement finished before the abandon anchor was reached: Err(SqlError { message: "Execution error: Duplicate key" })`。诊断现场（回滚前）：活跃事务的版本集合 = `{RowId{page 4, slot 0}, RowId{page 4, slot 1}, RowId{page 4, slot 2}}`；`find_key_by_row_id(slot 0) = Some(key 1)`、`find_key_by_row_id(slot 1) = None`、`find_key_by_row_id(slot 2) = Some(key 9999)`；索引里 `search(key 1) = Some(slot 0)`、`search(key 2) = Some(slot 1)`。回滚后：`search(key 1) = None`（已释放 ✓）、`search(key 2) = Some(slot 1)`（**残留 ✗**）→ 第二次插入键 2 撞 `DuplicateKey`。
- **影响面与触发条件**：触发条件 = 「显式事务内的 INSERT 被取消在索引写入中途」+「调用方回滚」。用户可见后果：被取消语句占用的键无法复用（`DuplicateKey`），且索引条目指向已中止的行。隐式事务的回收路径同样受影响——Iteration 000 的用例只复用了**第一个**键（`INSERT INTO t2 VALUES (1)`），所以没有暴露这一点。
- **与本 change 范围的关系**：超出本 change 范围——修复点在索引写入的原子性与中止路径的行分类，不在调用方语义面；且它同时是 Iteration 000 稳定结果「索引条目已还原、同键可再插」在**第一个键之后**不成立的证据，需要 Plan/后续 change 决定承载位置。
- **建议修复方向**：① 让 `IndexManager::insert` 的正向条目与反向映射成为不可分割的一步（同一临界区，取消不会留下半写状态）；② 让 `abort_cleanup_versions` 的分桶不只依赖反向映射（例如同时按正向条目反查该 row_id，或从元组派生 PK 键反查条目），使「有正向条目但反向映射缺失」的行仍走 A 趟（删除条目）；③ 回归用例：取消多行 INSERT → 回滚 → **全部**被取消行的键都能再次插入。

### 未解决问题与停止条件

- **E1 / E2 未修复**（范围外，见上）。2.3 用例的 20ms 窗口与 2.4 用例的 9001+ 键是**规避**而非修复：E1 的触发条件（取消窗口大到让回滚的索引删除与残留写入并发）在更大的窗口下依然成立，E2 的残留条目依然存在。
- **停止条件**：本 Cycle 的五个任务全部完成，无待办任务；未命中 Gate 5（2.1 的 RED 缺失是 Plan 自身的判据冲突，按 Deviations 1 记录，未改产品代码；2.2 / 2.3 的两处缺陷均在范围外且已按可承载格式报告）。若 Plan Review 认为 E1 / E2 应在本 change 内修复，则需要新执行依据（索引层写者串行化设计），按 rework / replan 处理。

## Plan Review

- Review Result: replan-required

**Findings**

**F1（阻塞规格准确性；分类：Plan 错误 + 新证据）—— 规格无条件断言的「索引条目已还原 / 同一个键可以再次插入成功」超出已验收范围。**

- 受影响处：`specs/statement-cancellation-outcome/spec.md` 的 requirement「提交前被取消的语句不提交」（枚举含「索引条目已还原」）、场景「隐式语句在执行中被放弃后不提交」与「被放弃语句的未提交数据不随 checkpoint 落盘」（各含一次「同一个键可以再次插入成功」）；同源措辞另见 change `tasks.md`（任务 1.3 的验收、Iteration 000 的 Stable result）与 `design.md`（D2 的「还原索引条目」、验证段的「同键再插入」）。
- 独立复现（本次 Review 的临时探针，已删除，工作区已复原）：显式事务内取消 `INSERT INTO t2 SELECT id FROM t` → 调用方回滚 → `INSERT INTO t2 VALUES (2)` 报 `Duplicate key`，键 1、3..10 均成功；回滚前该行的反向映射缺失（`find_key_by_row_id` → `None`）而正向条目在位。隐式回收路径同形（RR 与 RC 各跑一次，结果一致）。
- 结论：该保证只对被取消语句的**首个键**成立。Iteration 000 的用例只复用首个键（`INSERT INTO t2 VALUES (1)`），因此没有暴露；change 自己的迭代稳定结果与规格把它写成了无条件保证。
- 处置：该规格 delta 即将合并进 `openspec/specs/`；按原样合并会把未验收的行为写进权威规格库（AGENTS.md「规格库只保存验收过的行为」）。收窄 requirement 与验收边界属 `references/iteration-planning.md` Review 分类表的第 4 行 → `replan-required`。

**F2（非阻塞；分类：Plan 错误）—— 2.1 无 RED 是 Plan 自身的判据冲突，Act 的处理正确；该用例的定位是表征/锁定用例，不是驱动用例。**

- Task Definition 2.1 要求「必须先观察到 RED（行在重开后不存在 / 行在同一会话读不到）」，而同一份 Plan Context 的 Current-State Evidence 已写明「`Database::commit` 的原子效果（提交帧 fsync + 版本标 committed + 移出活跃集）先于其返回 → 目标行为当前已成立」——两者互斥，RED 判据不可满足。
- 裁定：Act 的处理**正确**——未改任何产品代码去制造 RED，按 AGENTS.md 的表征分支记录基线。窗口工程三件（阈值 1 / 锚点 / 判别位）经 Act 实测生效（锚点在 commit future 尚未返回时命中，放弃点落在自动 checkpoint 的步骤 1），用例因而是有效的场景锁定，不是误判为通过。
- 后续产物的更正：`Plan Context` 不可改写，修正记录在本 Review 与 change `tasks.md` 的任务 2.6 / 边界注记中；2.1 的定位统一表述为「表征/锁定用例（行为在 Iteration 000 的实现之后已成立，Cycle 000 把它变成可观察证据）」。

**F3（非阻塞；偏差已说明且不削弱断言）—— 2.2 / 2.3 / 2.4 的绕行式偏差逐条裁定。**

| 偏差 | 裁定 | 依据 |
|---|---|---|
| 2.2 / 2.3 夹具用 `ReadCommitted` 打开 | **不是削弱，是 Plan 遗漏的必要补齐** | Plan 的断言基础（事务内可见 / 事务外不可见）只在 RC 下可观察：RR 下 `Database::statement_snapshot` 返回 `None`，扫描路径不套可见性过滤。Act 改用公开 API `open_with_isolation`，未动产品代码。 |
| 2.2 场景 3 与 2.3 的 (e) 改用第二张目标表 `t3` | **不削弱** | 规格的显式事务三个场景只断言「事务仍活跃可用」「该行不存在，事务结束」「该行持久存在，重开仍在」——没有「同键可再插」。2.2 的断言落在「事务外不可见 / 事务内可见 / 回滚后不可见 / 提交后持久」，与目标表无关。E2 影响的是**键的占用**而非**行的可见性**；换表把两个关注点解耦，是正确的隔离。 |
| 2.4 的后续写用键 9001–9003 | **不削弱** | 2.4 的目标行为是「会话与后台 I/O 在取消后仍按会话级生命周期工作」。用被取消语句的键会把「会话可用性」与「键占用」两个故障域塞进同一条断言；偏移键使 2.4 只测会话可用性（被放弃语句的行不存在由 1.3 / 2.4 的重开后断言覆盖）。 |
| 2.3 的超时窗口取 20 ms | **不削弱** | 2.3 的断言是「超时返回 `Elapsed`、锚点观察确认语句已推进、事务仍活跃、行仍在事务内、回滚才撤销、提交则持久」。窗口只决定取消何时发生；本次 Review 的探针在 100 ms 窗口下观测到事务内可见 422 行、20 ms 窗口下取消前写入约 100 行——「行仍在事务内」不是空断言。 |

- 但：三处规避**都没有修复** E1 / E2，只是把缺陷的暴露面缩小（2.3 的 20 ms 窗口直接避开了 E1 的触发条件）。缺陷不得靠用例规避结案，必须由 Issue 承载（见下面的 Issue 候选）。

**F4（阻塞 severity 认定；E1 成立）—— E1 的机制经独立核对成立，可复现性为竞态依赖。**

- 机制（本次 Review 独立读码确认）：`src/storage/btree/btree.rs:327` 的 `BTree::insert_into_page` 对内部节点的子指针**无条件递归下降**（`:319-324` 读 `find_child_page_id_binary`，`:243` 以 `data_guard[0] == LEAF_NODE` 判叶子／内部）；`src/storage/btree/index_manager.rs:239-257` 的 `IndexManager::insert` 把 B 树条目写放在 `spawn_blocking` 内（tokio 的 blocking 任务**不可取消**），反向映射 `row_to_key` 在其后的 await 才写；`index_manager.rs:261-282` 的 `IndexManager::delete` 走另一条 `spawn_blocking` 且会合并/重分布叶子并可能换根。两条写路径之间**没有写者串行化**，根页 id 的更新与树内容变更也不在同一临界区 → 子指针可能为 0 → 沿第 0 页无限下降 → 栈溢出 → 进程 abort。
- 复现尝试：本次 Review 用两份临时探针共试 6 次（4 次简化形 + 2 次贴近 2.3 原形的复刻，含事务内读、事务外读、回滚、第二个事务写入），**均未复现崩溃**。这不推翻 Act 的现场（gdb 回溯 339+ 递归帧、插入键 111、`root_page_id` 指向数据页是直接证据），也不构成「偶发即可接受」——结论是：**机制成立、可复现性竞态依赖**。
- 裁定：进程级 abort 属必须登记并修复的实质缺陷（非 Minor），修复点落在存储/索引层，超出本 change 的 Impact 与 design D1–D6。

**F5（新证据）—— 收窄后唯一缺直接断言的条款：「被放弃语句写入的行不可见」的同一会话形态。**

- 现有断言都在**重开后**形态（1.3、2.4）；场景 1 的 WHEN 是「随后调用方在同一个 `Database` 句柄上执行下一条写语句」，属同一会话形态。
- 本次 Review 的探针观测（RR 与 RC 各一次）：回收完成后同一会话 `SELECT id FROM t2` 只读到后续写入的那一行（行数 = 1）→ 该形态当前为真，但仓库里没有直接断言。→ 2.6 在 2.4 上补这一条断言（用例数保持 8）。

**F6（非阻塞）—— M19 分批回归记录的结构核对：成立。**

- 目标清单核对：`tests/` 下 130 个已跟踪文件 + 本 change 新增 1 个 = **131** 个 `tests/*.rs` 目标，与磁盘实际文件集合比对 **missing 0 / extra 0 / duplicate 0**；每批 11 个、尾批 10 个，全部落在 M19 的 8–12 区间；`--lib --bins` 单独成批 → 共 13 批、132 个测试二进制。
- 顺序核对：把分批清单摊平后**严格按文件名字典序**，批内各自有序，批间边界不倒序 → 符合「按文件名字典序分组」。
- 计数对账：各批 passed 加总 1743（149+210+163+169+103+75+143+222+107+168+151+83）+ `--lib --bins` 批 420 = **2163**；ignored 2（批次 02）；failed 全 0。与「MS33 收尾终态 2155 + 本 change 新增 8」一致。
- 未重跑全量的依据：2.5 的结论产生后被覆盖的代码未发生变化（`git status --short` 与 Review 进入时一致；`src/` 的四个文件仍是本 change 的既有改动，`tests/statement_cancellation_test.rs` 在 Review 期间未被修改）→ 按 AGENTS.md「覆盖范围未变化的验证结论直接沿用」处理，核对方式为覆盖性与结构核对，无需重跑。

**F7（非阻塞）—— 身份证明机制与判定层检查：无违规。**

- `tests/statement_cancellation_test.rs` 内没有 Hash / 指纹 / run-id / manifest / revision pin / peer-host pin / 时间顺序证明；判定全部落在 `cargo test` 内的 `assert!` / `assert_eq!`，读的是库入口返回的行集与公开的事务状态。
- `TIMEOUT_CANCEL_WINDOW`（20 ms）只作**取消触发源**；`ANCHOR_TIMEOUT`（10 s）只作夹具失控兜底（触发即 panic，非阈值判定）；`setup` 的 2000 行规模是夹具规模，不是判据。
- 无项目测试框架之外的判定层、无新包装命令、无专用 CLI。

**F8（非阻塞 Minor）—— 无。** 本 Cycle 未新增产品代码路径；命名、局部结构与重复实现方面无新增问题。

**Acceptance Gaps**

- 2.1–2.5 与整文件命令（`cargo test --test statement_cancellation_test -- --test-threads=1`）：**全部满足**，逐条映射见 F3 与 Evidence。
- **GAP-1（阻塞）**：change 级 requirement「提交前被取消的语句不提交」中的「索引条目被还原」与两个场景的「同一个键可以再次插入成功」**未满足**（只对被取消语句的首个键成立）。分类：Plan 错误（requirement 措辞超出已验收范围）+ 新证据（本次 Review 独立复现）。处置：由 replan Cycle 的任务 2.6 收窄规格并标注已知边界；缺陷本体由 Issue 承载，不在本 change 修（见 Follow-up Decision 的边界判断）。

**Evidence**

- 独立读取：`tests/statement_cancellation_test.rs` 全文（8 个用例 + 夹具）、父 Cycle `000-initial.md` 与 Iteration 000 的 `001-rework.md`、change `tasks.md` / `proposal.md` / `design.md` / `specs/statement-cancellation-outcome/spec.md`、`git diff`（`src/database.rs`、`src/pipeline.rs`、`src/executor/alter_table.rs`、`src/transaction/manager.rs`）、`src/storage/btree/index_manager.rs:225-300`、`src/storage/btree/btree.rs:228-348`、`src/transaction/manager.rs:310-380`。
- 定向审计（临时探针，两份，均已删除，工作区已复原为 Review 进入时的状态）：
  - E2 复现：`INSERT INTO t2 VALUES (2)` → `SqlError { message: "Execution error: Duplicate key" }`；键 1、3..10 成功；被取消行 `RowId{page 4, slot 1}` 的 `find_key_by_row_id` 为 `None`。
  - 隐式路径同形：RR 与 RC 下回收后 `INSERT INTO t2 VALUES (2)` 均报 `Duplicate key`，键 1 成功。
  - 同一会话形态：回收完成后同一会话 `SELECT id FROM t2` 行数 = 1（只有后续写入的行）——RR 与 RC 一致。
  - E1：6 次尝试（4 次简化形 + 2 次贴近 2.3 原形）**未复现**崩溃；100 ms 窗口下事务内可见 422 行（说明取消点确实落在多行写入中途）。
- 沿用未失效的既有结论（来源：父 Cycle Act Response）：2.1–2.4 的单用例与整文件运行（8 passed / 0 failed）、2.5 的 13 批 M19 分批回归（每批退出码 0、failed 0，合计 2163 / 0 / 2）。本次 Review 未重跑上述验证——覆盖范围未变化（见 F6）。
- 结构核对（不需重跑）：131 个目标 missing 0 / extra 0 / duplicate 0；摊平清单严格字典序；1743 + 420 = 2163、ignored 2、failed 0。

**Issue 候选（按「可承载」方式报告；只报告不登记——登记由 Recorder 按用户指令执行）**

**E1（进程级 abort；必须登记并修复）**

- 一句话结论：在显式事务里取消一条正在执行的多行 `INSERT … SELECT`，随后对该事务回滚，会让 B 树结构损坏——之后的一次索引插入沿「内部节点的子指针 = 0」无限下降，栈溢出使整个进程 abort。
- 根因位置：`src/storage/btree/btree.rs:327`（`BTree::insert_into_page` 的内部节点下降递归）、`src/storage/btree/btree.rs:243`（叶子/内部判定）；损坏入口 `src/storage/btree/index_manager.rs:239-257`（`IndexManager::insert`：B 树条目在 `spawn_blocking` 内写入、不可取消，反向映射在其后的 await 才写）与 `src/storage/btree/index_manager.rs:261-282`（`IndexManager::delete`：回滚路径删条目时会合并/重分布叶子并可能换根）。两条写路径之间无写者串行化，根页 id 更新与树内容变更不在同一临界区。
- 已观察证据：Act 的 gdb 现场（`thread 'tokio-rt-worker' has overflowed its stack` → SIGABRT，栈上 339+ 层 `BTree::insert_into_page`，帧内 `child_page_id_u32 = 0`，插入键 111，`root_page_id = 75` 指向数据页）；本次 Review 的独立读码确认机制成立（F4），6 次复现尝试未命中 → 竞态依赖。
- 影响面与触发条件：触发条件 =「多行 `INSERT … SELECT` 在索引写入中被取消」+「该事务随后被回滚」；用户可见后果是**整个进程崩溃**，且崩溃点与取消无关、出现在之后的任意索引写入上。取消窗口越大越容易触发；隐式事务的回收路径（同样调 `abort_by_id`）可能同形命中。
- 与本 change 范围的关系：超出——修复点落在存储/索引层的写者串行化与取消时的原子性，不在「取消/超时/事务收尾」的调用方语义面；design D1–D6 与两个 Iteration 的 Plan 都明确不改索引语义，Task Definition 2.2 的 Stop when 也规定「需要改 `Database::rollback` / `abort` 的既有语义 → 返回 Plan」。
- 建议修复方向：① 按索引串行化结构变更（插入/删除/合并/换根）并让「B 树条目 + 反向映射 + 根页 id」对其他写者原子可见；或 ② 让取消路径不留下半写状态（把两次写放进同一不可中断的临界区，根更新同临界区）；③ 回归用例：取消多行 INSERT → 回滚 → 同句柄上的后续写语句不得崩溃且 `check_integrity` 无 Error。
- 承载方式建议：登记为 Issue（`.agents/issues/`），修复放在**存储/索引写侧**的后续里程碑（建议新开一段「索引写侧原子性与取消后一致性」，与 MS35「空闲页跨重启复用」相邻——E1 的现场涉及根页 id 指向被复用的数据页，两段在 `src/storage/` 的改动面可能重叠，须由 milestone-planner 统一排期）；不在 MS34 内修。

**E2（键仍被占用；必须登记并修复）**

- 一句话结论：显式事务内一条多行 `INSERT … SELECT` 被取消后调用方回滚，被取消语句写入的键（**首个键之外**）仍留在主键索引的正向条目里，同一个键再插入报 `DuplicateKey`；行本身不可见。
- 根因位置：`src/storage/btree/index_manager.rs:239-257`（`IndexManager::insert` 的两段写：正向条目在 `spawn_blocking` 内完成，反向映射 `row_to_key.insert` 在其后的 await 之后才写——取消落在两者之间时正向条目在位、反向映射缺失）与 `src/transaction/manager.rs:344-355`（`abort_cleanup_versions` 的 A/B 分桶只查反向映射 `find_key_by_row_id`，反向映射缺失的行被误判为「墓碑」而跳过，其正向条目不被移除）。
- 已观察证据：Act 的 2.2 首版 RED（`Err(SqlError { message: "Execution error: Duplicate key" })`）与其诊断现场；本次 Review 独立复现（显式路径与隐式路径、RR 与 RC 各一组：键 2 报 `Duplicate key`，键 1 与 3..10 成功；被取消行的 `find_key_by_row_id` 为 `None`）。
- 影响面与触发条件：触发条件 =「`INSERT` 被取消在索引写入中途」+「调用方回滚 / 回收」。用户可见后果：被取消语句占用的键无法复用，且索引条目指向已中止的行。隐式回收路径同样受影响——Iteration 000 的用例只复用了首个键，所以没有暴露。
- 与本 change 范围的关系：超出——修复点在索引写入的原子性与中止路径的行分类；它同时是 Iteration 000 稳定结果「索引条目已还原、同键可再插」在首个键之外不成立的证据，故本 Review 判 `replan-required` 收窄规格，而不是在本 change 修实现。
- 建议修复方向：① 让 `IndexManager::insert` 的正向条目与反向映射成为不可分割的一步（同一临界区，取消不留下半写状态）；② 让 `abort_cleanup_versions` 的分桶不只依赖反向映射（例如同时按正向条目反查该 row_id，或从元组派生 PK 键反查条目），使「有正向条目但反向映射缺失」的行仍走 A 趟；③ 回归用例：取消多行 INSERT → 回滚 → **全部**被取消行的键都能再次插入。① 与 E1 的修复面重叠，应与 E1 合并到同一后续 change。
- 承载方式建议：与 E1 合并为同一 Issue 主题（索引写侧原子性），由 Recorder 登记，修复落在上述后续存储/索引里程碑；本 change 以「规格收窄 + 已知边界注记」结清，键释放的行为规格随该后续 change 进入规格库。

**C1（数据面形态不一致；范围外实质问题）**

- 一句话结论：被放弃的 ALTER TABLE DROP COLUMN 之后，表处于「目录已窄化 + 数据面仍为混合宽度」的形态——回收只回滚数据面，目录侧的窄化不在事务内。
- 根因位置：`src/storage/data/table_manager.rs:793-802`（`alter_drop_column` 先改目录并 `flush_all`，窄化不在事务内）。
- 已观察证据：Iteration 000 与其 rework Cycle 的 Plan Review 均报告；本 change 的回收只覆盖数据面回滚。
- 影响面与触发条件：触发条件 =「ALTER TABLE DROP COLUMN 的语句 future 被放弃」；后果是目录与数据面的列宽不一致，后续查询按窄化后的目录解释旧数据。
- 与本 change 范围的关系：超出——DDL 目录变更的事务化不在「取消/超时/事务收尾」的调用方语义面；Iteration 000 / 001 的 Non-goals 已明确列为「change 范围外的实质问题，只报告不登记」。
- 建议修复方向：把 DDL 的目录变更纳入事务/回滚边界（要么窄化与数据面改写在同一可回滚单元内，要么窄化延后到语句确定完成之后）。
- 承载方式建议：登记为 Issue，修复放在 **DDL/目录事务化**的后续里程碑（与 MS36「历史库与 dump 兼容」相邻——两者都触及目录变更与存储格式的边界），由 milestone-planner 排期；不在 MS34 内修。

**Follow-up Decision**

- **为什么不是 `accepted`**：本 Iteration 的 Acceptance（2.1–2.5）已满足，但 change 级 requirement 的措辞超出已验收范围——规格 delta 即将合并进 `openspec/specs/`，按原样合并会把未验收的行为写进权威规格库。规格准确性是本 change 的交付物之一，不能带缺口收尾。
- **为什么不是 rework / 当前 Cycle 修复**：iteration-planning.md 的判断问题「该工作是否是达到当前 Iteration 原有 Acceptance 的必要条件？」——**否**。修 E1 / E2 不是达到 2.1–2.5 的必要条件；而且两者的修复点落在 `src/storage/btree/` 与索引写入路径，超出本 change 的 Impact（`src/database.rs` / `src/pipeline.rs` / `src/executor/alter_table.rs` / `src/transaction/manager.rs`）与 design D1–D6，需要新的执行依据与新的设计决策（写者串行化、原子性、页生命周期），不能以返工名义塞进当前 Cycle。
- **为什么是 `replan-required`**：requirement 的措辞与验收边界需要改变（收窄「索引条目被还原 / 同一个键可以再次插入成功」，改为已验收的可见性与回收契约 + 已知边界注记），落在 iteration-planning.md Review 分类表的第 4 行 → 更新 change 的 tasks / specs / design 与未完成的 Iteration Plan，再创建同目录的后继 replan Cycle。
- **收窄 vs 修实现**：用户本轮约定「实施当中发现的所有 issue 候选都直接寻找好的承载方式流程进行修复而不是等级」——承载方式 = Issue 登记 + 落到存储/索引侧的后续里程碑修复（见上面的「承载方式建议」）；「把索引写侧原子性纳入 MS34」超出 MS34 已批准的范围与 Impact，也超出 goal 授权的阶段推进范围，故不在本次选。
- **规格收窄的边界**：规格只写已验收行为——不断言键一定可复用，也不断言键一定被占用（后者会把缺陷写成契约）；已知边界只说明「行不可见 / 键可能仍占用」，修复方案不写进规格。键释放的行为规格随修复它的后续 change 进入规格库（AGENTS.md「可表达为行为要求的事实随下一个 change 进入规格库」）。
- **需要用户确认的一点（不阻塞）**：规格收窄是对已批准 requirement 的措辞收窄，本 Review 已按 goal 授权（阶段推进 + 「issue 候选要有承载方式」）执行并记录；若用户不接受收窄，替代路径是把索引写侧原子性纳入 MS34 范围——那是一次显著更大的范围与工作量变更，需重新规划。

**Iteration Plan Update**

- Iteration 001 的任务集合：2.1、2.2、2.3、2.4、2.5 → **2.1、2.2、2.3、2.4、2.5、2.6**（新增 2.6「规格收窄与边界回归」）。
- 验收边界：Stable result 追加「**行为规格只描述本 change 已验收的行为**（未验收的键释放能力标注为存储域边界，不写进权威规格库）」。
- 验证方式：2.6 只重跑受影响目标（`statement_cancellation_test` 单文件与 batch 10），**不重跑全量**——2.5 的其余 12 批覆盖范围未变，其结论沿用（AGENTS.md「覆盖范围未变化的验证结论直接沿用」）。
- 依赖与 Non-goals 不变；新增 Non-goal：索引写侧原子性 / 键释放的修复（E1、E2）。

**Next Cycle**

`iterations/001-caller-visible-outcomes-and-final-regression/001-replan.md`（Cycle Type: replan；任务 2.6；`Plan Context` 状态 `ready`，Gate 2 PASS）。

**Next Iteration**

None


