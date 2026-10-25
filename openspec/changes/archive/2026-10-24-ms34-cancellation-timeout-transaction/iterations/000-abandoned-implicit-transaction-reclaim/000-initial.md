# Iteration 000 / Cycle 000: 被放弃的隐式事务回收

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: 1.1、1.2、1.3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 中已确认的三种结果约定与场景 1/2/6/7；设计 D1-D6；规格 `statement-cancellation-outcome` 的「取消触发集合」「提交前被取消的语句不提交」「取消不留下悬空事务」三条 requirement
- Excluded scope: 任务 2.1-2.5、场景 3（提交已完成但未送达）、场景 4/5（显式事务继续或回滚、超时不代替回滚）、后台 I/O 的会话级核对、公共取消或 deadline API、CLI 信号停机行为、WAL 格式与 checkpoint 九步

**Objective**

一条隐式语句在执行中被放弃之后：它的库属隐式事务不提交，在同一 `Database` 句柄的下一次写语句和下一次 checkpoint 之前完成回滚（活跃事务集合为空、索引条目已还原、同一个键可以再次插入）；它的未提交数据不随 checkpoint 落盘，`close` 后重开该行仍不存在。

**Investigation Facts**

- Starting point: 修订 `0a08b14`（MS33 收尾入库），`master` 比 `origin/master` 超前 9。工作区只有 `.agents/docs/tasks.md` 一处未提交修改（MS34/35/36 转 ready），本 Cycle 不改它。变更前基线已在本会话实跑：`cargo test --test explicit_tx_test --test mvcc_abort_test --test dml_tx_id_test --test mvcc_commit_test -- --test-threads=1` 退出码 0（`explicit_tx_test` 21 passed、`mvcc_abort_test` 3 passed、`mvcc_commit_test` 4 passed，均 0 failed）；`cargo test --test dml_tx_id_test -- --test-threads=1` 6 passed / 0 failed。MS33 收尾的全量终态是 2155/0/2（`.agents/docs/SNAPSHOT.md`），本 Cycle 未重跑全量。
- Current-State Evidence:
  - `src/pipeline.rs:559` 是 DML 臂的隐式事务起点（`target_db.transaction_manager.begin()`）。该臂只有两个出口：执行器返回错误时 `abort`（`src/pipeline.rs:604-617`），成功时 `target_db.commit(tx)`（`src/pipeline.rs:618-629`）。**没有丢弃路径**：`tx` 是局部变量，`Transaction` 只持 id + snapshot + state（`src/transaction/manager.rs:24-37`），析构不做任何事。
  - 语句 future 被放弃后，该 tx 永远留在 `TransactionManager::active_tx_ids`（`src/transaction/manager.rs:57-64`），其 `tx_versions` 条目也不清（`src/transaction/manager.rs:181-189` 登记）。
  - `Database::maybe_auto_checkpoint` 的 R3 顺延判定在活跃事务集合非空时直接返回（`src/database.rs:526-535`）→ 被放弃的事务让自动 checkpoint 无限顺延，WAL 不再被截断。
  - `InsertExecutor` 的重复键预检只查索引里有没有这个键，不查可见性（`src/executor/insert.rs:177-187`）；索引条目在语句执行期就写入（`src/executor/insert.rs:275-283` 先 `record_version` 再写索引）→ 被放弃的 INSERT 之后，同一个键再次插入会拿到 `DuplicateKey`。
  - `CheckpointManager::checkpoint` 先 `flush_all` 刷全部脏页，再重写截断 WAL（`src/wal/checkpoint.rs:190-213`）→ 被放弃事务的未提交页会被写盘，WAL 里的 `BeginTxn` 证据随截断消失。
  - 恢复期把 WAL 中没有提交/中止帧的事务标记为中止（`src/wal/recovery.rs:1420-1499`）→ WAL 是被放弃事务在恢复期的唯一权威，前提是 WAL 还没被截断。
  - CTAS 路径有同形的第二处隐式事务：`src/pipeline.rs:971` 的 `begin()`，出口在 `src/pipeline.rs:1010-1043`（失败 abort + drop_table；成功 commit + 自动 checkpoint）。
  - 行级写锁表 `RowLockTable`（`src/transaction/row_lock.rs:10-45`）在全仓没有任何执行器使用（只有定义与自身单测）→ 它不是取消面的一部分。取消时真正可能被持有的锁是缓冲池页锁（`BufferPool::with_page_data`，`src/storage/buffer_pool.rs:147`；页锁只在非 async 闭包内持有，见 `src/storage/buffer_pool.rs:331-333`，语句 future 不可能在闭包执行中被取消）与进程级文件锁（不受语句影响）。
  - `src/executor/executor_trait.rs:9-14` 的 `Executor` 接口不携带取消/超时上下文（I090 的原始证据，本次复核仍然成立）→ 协作式取消要改接口加全量执行器改造，属于完整取消系统。
  - 后台 I/O：WAL writer、WAL buffer、缓冲池是会话级对象（`src/database.rs:111-117`，由 `open_internal` 构造）→ 语句取消不涉及它们的生命周期。`src/executor/data_scan.rs:177-187` 的预取任务是只读、自终止的（`buffer_pool.get_page`），不写盘。`spawn_blocking` 里的写（WAL writer、`src/storage/btree/index_manager.rs`）不可取消，会在语句被放弃后完成，由 WAL 语义兜住（没有提交帧 → 恢复期标记中止）。
  - CLI 信号路径（`src/cli/mod.rs:698-729`）：`select!` 放弃 work future 后调用 `db.close()`（→ checkpoint）。`src/cli/mod.rs:607-609` 的注释称「执行阶段取消只丢弃未提交 DML（恢复期 uncommitted 清理）」——按当前代码这句只在 WAL 未被截断时成立（`kill -9` 路径），优雅关闭路径会先把未提交页写盘再截断 WAL。本 Cycle 修的是库侧收尾，不改 CLI 行为。
  - 网络路径（`src/network/connection.rs:18-35`）：连接处理器先 `await` 语句执行完再写响应 → 客户端在语句执行中断开时，语句照常跑完并提交，客户端只是没收到结果。这正是「提交已完成但调用方未收到结果」的自然来源，属 Iteration 001 的场景 3。
- Code and Critical Path: 库入口 `Database::execute_sql`（`src/database.rs:461-463`）→ `pipeline::execute` → `execute_with_params` → `execute_inner` → `execute_stage` → `execute_stage_routed`（DML 臂 `src/pipeline.rs:545-632`）。放弃点落在 `execute_executor(executor).await`（`src/pipeline.rs:598`）或其后的 `commit`（`src/pipeline.rs:623`）之间任一 await 点。回收要用的中止路径是 `TransactionManager::abort`（`src/transaction/manager.rs:142-169`）：写 `AbortTxn` → `abort_cleanup_versions`（墓碑化未提交版本、还原索引条目，`src/transaction/manager.rs:282-299` 起）→ 移出活跃集 → 清 `tx_versions`。按 id 中止已有先例：`TransactionManager::commit_by_id`（`src/transaction/manager.rs:242-250`）。

**Implementation Guidance**

登记表放在 `Database` 上（库属隐式事务 = 由库自己 begin、自己 commit/abort 的事务，与调用方持有的显式事务区分开）。DML 臂与 CTAS 路径在 `begin()` 之后登记 tx id、在 `commit`/`abort` 之后摘除；登记与摘除本身不做 I/O。

回收触发点三处，顺序固定：目标库下一次隐式写语句 `begin()` 之前、`Database::checkpoint()` 之前（覆盖 `close()`、显式 checkpoint 命令与自动 checkpoint）、`Database::maybe_auto_checkpoint()` 的活跃集判定之前。第三处必须在判定之前，否则被放弃的事务会让顺延条件永远为真。

回收按 `TransactionManager::tx_version_tables(tx_id)` 解析受影响表（与 `Database::rollback` 同源，`src/database.rs:564-586`）。表已不存在（被放弃之后被 DROP）时，先把该事务在这些表上的版本条目从管理器里摘掉，再中止其余部分——表的数据与索引已随 DROP 释放，没有可还原的东西，摘条目让中止能进行到底。中止本身失败（I/O 错误）时回收返回点名错误并保留登记项，由下一个触发点重试；触发方（下一次写语句 / checkpoint）收到该错误。

测试夹具（`tests/statement_cancellation_test.rs`，新文件）：独立 `TempDir` 现场；建表 `t(id INT PRIMARY KEY, v INT)` 与 `t2(id INT PRIMARY KEY, v INT)`；`t` 预置 2000 行（循环单行插入即可）；被放弃的语句是一条 `INSERT INTO t2 SELECT id FROM t`（一条语句、一个隐式事务、2000 行）。放弃点用库入口可观察的锚点确定，不靠定时器：`db.transaction_manager.active_transactions()` 出现该 tx id，且 `db.transaction_manager.get_tx_versions(tx_id)` 已有 ≥2 个版本——执行器逐行推进，第 1 行的索引条目先于第 2 行的 `record_version` 写入完成，所以 ≥2 个版本保证「第一个键的索引条目已在位」。观察循环用 `tokio::task::yield_now()` 让出执行权并带上限（例如 10 秒），不用固定 sleep 阈值判定。语句 future 用 `Box::pin` 持有，`tokio::select!` 在锚点命中后跳出，随后 `drop(fut)` 放弃它。

第一个键的确定性：`t` 的 id 是 1..2000，被放弃语句按 `SELECT id FROM t` 的顺序插入，`t2` 的第一个键恒为 1 → 回收前 `INSERT INTO t2 VALUES (1)` 必然撞上残留索引条目，回收后必然成功。

**Task Definitions**

### 1.1: 隐式语句被放弃后不提交的用例先失败

- Requirement/Scenario: 提交前被取消的语句不提交 / 隐式语句在执行中被放弃后不提交；取消不留下悬空事务 / 取消不留下悬空事务
- Depends on: None
- Targets: `tests/statement_cancellation_test.rs`（新建）
- Current behavior: 库入口没有丢弃路径。被放弃的隐式事务永远留在活跃事务集合，它的索引条目留在 B 树里；下一个写语句不会回收它，同一个键再次插入拿到 `DuplicateKey`。
- Required behavior: 用例 `abandoned_implicit_statement_is_not_committed` 存在，并且在 1.2 改动之前失败。失败断言必须是「下一条写语句执行完之后 `active_transactions()` 仍非空」或「同一个键再次插入拿到 `DuplicateKey`」，而不是编译错误、夹具错误或用例本身写错。
- Preserve: 不修改 `src/` 下任何文件（1.1 只加测试）。既有 `tests/explicit_tx_test.rs`、`tests/mvcc_abort_test.rs`、`tests/mvcc_commit_test.rs`、`tests/dml_tx_id_test.rs` 不动。
- Forbidden: 为让用例先通过而改产品代码；引入取消或 deadline 公共 API；用固定 sleep 阈值、哈希、指纹、run-id 之类判定代替行为观察；把子进程信号或 `kill -9` 写进这个用例。
- First test: 新增 `tests/statement_cancellation_test.rs` 的 `abandoned_implicit_statement_is_not_committed`。命令：`cargo test --test statement_cancellation_test abandoned_implicit_statement_is_not_committed -- --test-threads=1`。预期：失败，输出指向活跃事务集合非空或 `DuplicateKey`。
- GREEN condition: 本任务不要求 GREEN。GREEN 属于 1.2。
- Verification: 上列命令退出码非 0，且失败信息是上面两条断言之一。退出码非 0 但失败原因是「找不到测试」「编译失败」「锚点超时」时，本任务算未完成，按 Stop when 处理。
- Stop when: 锚点（活跃集出现 tx id 且版本数 ≥2）在语句完成之前无法命中——说明夹具选错了语句规模，改夹具而不是改产品代码；或用例在没有产品改动的情况下已经通过——说明静态结论与运行不符，回到计划。

### 1.2: 库属隐式事务登记与回收

- Requirement/Scenario: 提交前被取消的语句不提交 / 隐式语句在执行中被放弃后不提交；取消不留下悬空事务 / 取消不留下悬空事务；取消触发集合是语句 future 被放弃 / 丢弃、超时与断开是同一类事件
- Depends on: 1.1
- Targets: `src/database.rs`（隐式事务登记表、回收入口、三个触发点）；`src/pipeline.rs`（DML 臂 `src/pipeline.rs:559` 与 CTAS 路径 `src/pipeline.rs:971` 的登记与摘除）；`src/transaction/manager.rs`（按 id 中止的入口，形态照 `commit_by_id`；被放弃事务在已删表上的版本条目清理）
- Current behavior: 无登记表、无回收。被放弃的隐式事务留在活跃集，索引条目不还原，自动 checkpoint 被顺延。
- Required behavior: 库属隐式事务在 `begin()` 之后登记、在 `commit`/`abort` 之后摘除；语句被放弃时登记项留下；回收在三个触发点执行，走既有中止路径（写 `AbortTxn`、墓碑化未提交版本、还原索引条目、移出活跃集、清版本条目）。回收完成后：下一条写语句开始前活跃事务集合为空；同一个键可以再次插入成功；自动 checkpoint 的顺延判定不再因这条事务为真。表已不存在时先摘该事务在这些表上的版本条目再中止；中止本身失败时回收返回点名错误并保留登记项。
- Preserve: 既有 `commit`/`abort` 路径的行为不变（未发生取消时语句结果逐字节不变）；`active_transactions()`、`tx_versions()`、`get_tx_versions()` 的语义不变；WAL 记录格式与 checkpoint 九步不变；`Database::rollback` 的显式回滚语义不变。
- Forbidden: 新增公共 API（登记表与回收入口是库内部可见）；修改 `Executor` 接口；把回收放进 `Drop` 里 `spawn` 后台任务；改 `TransactionManager::abort` 既有调用的语义；改 WAL 帧、位点布局或 checkpoint 步骤顺序；动 CLI 信号路径。
- First test: 1.1 已经是 RED。命令：`cargo test --test statement_cancellation_test abandoned_implicit_statement_is_not_committed -- --test-threads=1`。
- GREEN condition: 1.1 通过；`cargo test --test explicit_tx_test --test mvcc_abort_test --test mvcc_commit_test --test dml_tx_id_test -- --test-threads=1` 退出码 0。
- Verification: 上面两条命令的退出码都是 0。
- Stop when: 回收必须改成同步才能进行（意味着要重写 WAL 追加或页写路径）；或回收必须靠新增公共 API 才能挂载；或既有事务类用例在改动后失败且修复要动 `TransactionManager::abort` 的既有语义。

### 1.3: 被放弃语句的未提交数据不随 checkpoint 落盘

- Requirement/Scenario: 提交前被取消的语句不提交 / 被放弃语句的未提交数据不随 checkpoint 落盘
- Depends on: 1.2
- Targets: `tests/statement_cancellation_test.rs`
- Current behavior: `close()` 直接进 `CheckpointManager::checkpoint`：先 `flush_all` 刷全部脏页（含被放弃事务的未提交版本与其索引条目），再重写截断 WAL。WAL 证据消失后恢复期无从判断这些版本属于未提交事务。
- Required behavior: 用例 `abandoned_statement_does_not_survive_reopen` 通过：制造一次放弃（同 1.1 的夹具）→ `close()` → 重新打开同一个库 → 被放弃语句写入的行不存在，且 `INSERT INTO t2 VALUES (1)` 成功（索引条目已还原并随 checkpoint 落盘为干净状态）。1.1 的用例仍然通过。
- Preserve: checkpoint 九步顺序、WAL 帧、位点布局不变；`close()` 的既有语义不变；不引入「checkpoint 跳过未提交页」这类新的刷盘策略。
- Forbidden: 改 checkpoint 让它跳过某些脏页；改 WAL 格式或位点布局；用文件字节比较代替库入口的可观察结果。
- First test: 新增 `abandoned_statement_does_not_survive_reopen`（与 1.1 同一个夹具函数）。命令：`cargo test --test statement_cancellation_test -- --test-threads=1`。在 1.2 之后该用例应通过；若它仍失败，按 Stop when 处理。
- GREEN condition: `cargo test --test statement_cancellation_test -- --test-threads=1` 退出码 0，两个用例都在输出里通过。
- Verification: 同一命令。1.1 与 1.3 的失败原因必须不同（1.1 看活跃集与同键再插，1.3 看重开后的同键再插），避免两个用例测的是同一件事。
- Stop when: 该用例只能靠改 checkpoint 步骤或 WAL 格式才通过；或重开后被放弃的行可见。

**Invariants**

- 未发生取消的语句行为不变：既有 DML 显式事务、CTAS、checkpoint、WAL 与恢复语义一律不动。
- 库入口不新增公共 API，`Executor` 接口不变。
- 显式事务的生死仍由调用方决定：任何取消路径都不提交、不回滚调用方的事务。
- WAL 记录格式、位点布局、checkpoint 九步顺序不变。
- 被放弃的事务永远不会以「已提交」的面貌出现在恢复期：没有提交帧的事务在恢复期仍被标记为中止。

**Non-goals**

场景 3（提交已完成但调用方未送达）、场景 4/5（显式事务继续或回滚、超时不代替回滚）、后台 I/O 的会话级核对、公共取消或 deadline API、CLI 信号停机行为、真实断电与 `kill -9`、WAL 格式与 checkpoint 九步。

**Acceptance**

- 1.1 在 1.2 改动之前观察到 RED，失败原因是活跃事务集合非空或 `DuplicateKey`。
- 1.2 之后 1.1 通过；`explicit_tx_test`、`mvcc_abort_test`、`mvcc_commit_test`、`dml_tx_id_test` 仍全绿。
- 1.3 通过：重开后被放弃的行不存在，同一个键可以再次插入。
- 回收在下一条写语句之前完成；自动 checkpoint 的顺延判定不被被放弃的事务卡住。

**Verification**

全部场景从库入口观察：`Database::open` / `execute_sql` / `execute_in_tx` / `begin` / `commit` / `rollback` / `close`，以及公开的 `TransactionManager::active_transactions()` 与 `get_tx_versions()`。不使用内部函数断言代替库入口行为，不使用哈希、指纹、run-id、时间顺序等身份证明机制，不在本项目测试框架之外建判定层。锚点轮询使用 `yield_now`（让出执行权）而不是固定时长阈值；等待上限只作为夹具失控的兜底，触发上限即判失败。本 Iteration 不跑全量回归（全量按 change 预算在 Iteration 001 的 2.5 一次）。

**Gate 2 Readiness**

PASS。2026-10-24 用户原话「/openspec-autopilot 把已经规划的三个ms逐步完成，要求遵守各种规范，尤其是全量测试时候的多批次串行」与补充「因为这个项目有点大，三个ms也是串行实施就好」构成 Gate 1 与 Gate 2 的用户批准依据。无 SKIPPED、无 BLOCKED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 1.1 的 RED 若不出现，说明「被放弃的隐式事务会泄漏」这一前提不成立，必须回到计划重做设计，不得改产品代码去制造 RED。
- 锚点轮询有 10 秒上限：上限触发即判失败，这是夹具失控，不是产品结论。
- 「表在放弃之后被 DROP」这条路径在 1.2 里按 D6 处理（摘版本条目再中止），本 Iteration 的用例不覆盖它；它是低频路径，覆盖与否留给 Plan Review 判断。
- 被放弃事务的 `tx_versions` 条目在回收时才清；在回收之前它们占着内存，且 `get_tx_versions` 仍能读到它们——这是登记项尚在的正常表现。
- 工作区里已有的 `.agents/docs/tasks.md` 修改不属于本 Cycle。
- Iteration 001 的场景 3 需要「提交完成之后、结果送达之前」这个窗口稳定出现；本 Cycle 不处理该窗口，若 Iteration 001 的锚点无法稳定命中，按其 Stop when 回到 Plan，不得用身份机制替代行为观察。

## Act Response

- Status: reported

**Implemented**

- **1.1 — 先行测试（RED 见证）**：`tests/statement_cancellation_test.rs`（新建）新增 `abandoned_implicit_statement_is_not_committed`。
  - 夹具 `setup()`：独立 `TempDir` → `Database::open` → 建 `t(id INT PRIMARY KEY, v INT)` 与 `t2(id INT PRIMARY KEY)` → `t` 循环单行插入 2000 行。
  - 放弃点 `abandon_statement()`：语句 future 用 `Box::pin(db.execute_sql(sql))` 持有；`tokio::select!`（`biased`，锚点支在前）在锚点命中时跳出，随后 `drop(fut)` 放弃它。锚点 = `db.transaction_manager.active_transactions()` 出现事务 id **且** `get_tx_versions(tx_id)` ≥ 2（第 1 行的索引条目先于第 2 行的 `record_version` 写入完成，≥2 个版本即保证第一个键的索引条目已在位）。观察循环用 `tokio::task::yield_now()` 让出执行权；`tokio::time::timeout(ANCHOR_TIMEOUT = 10s)` 只作夹具失控兜底，触发即判失败，不是产品结论。
  - RED 见证（1.2 改动之前实跑）：退出码 101，失败输出 `INSERT INTO t2 VALUES (1) must succeed with affected rows, got Error { message: "Execution error: Duplicate key", sqlstate: None }`——命中计划允许的两条断言之一（`DuplicateKey`），不是编译错误、夹具错误或锚点超时。
- **1.2 — 库属隐式事务登记与回收**：
  - `src/database.rs`：`Database` 新增 `implicit_transactions: Arc<Mutex<HashSet<u64>>>`（登记表）；新增 `register_implicit_tx`、`unregister_implicit_tx`、`reclaim_abandoned_implicit_transactions`、`reclaim_implicit_tx`（均 `pub(crate)`）。回收走既有中止路径：写 `AbortTxn` → `abort_cleanup_versions`（墓碑化未提交版本、还原索引条目）→ 移出活跃集 → 清版本条目。表已不存在时先 `discard_tx_version_tables` 摘该事务在这些表上的版本条目再中止；中止失败保留登记项并点名返回错误（`abandoned implicit transaction <id> reclaim failed: ...`），由下一个触发点重试。
  - `src/transaction/manager.rs`：`abort` 改为委托新增的 `abort_by_id(tx_id, buffer_pool, tables)`（同一路径、语义逐字节不变）；新增 `discard_tx_version_tables(tx_id, table_names)`。
  - `src/pipeline.rs`：DML 臂（`begin()` 前回收、`begin()` 后登记、abort/commit 成功后摘除）；CTAS 路径（函数入口回收、`begin()` 后登记、abort/commit 成功后摘除）。
  - `src/executor/alter_table.rs`：ALTER TABLE DROP COLUMN 臂同型登记/摘除/回收（第三处库属隐式事务，见 Deviations 第 2 条）。
  - 三个回收触发点（顺序固定）：目标库下一次隐式写语句 `begin()` 之前（DML 臂、CTAS 路径、ALTER TABLE 臂）、`Database::checkpoint()` 之前（覆盖 `close()`、显式 checkpoint 命令、自动 checkpoint）、`Database::maybe_auto_checkpoint()` 的活跃集判定之前。
- **1.3 — 被放弃语句的未提交数据不随 checkpoint 落盘**：同一测试文件新增 `abandoned_statement_does_not_survive_reopen`（复用同一夹具函数）：放弃 → `close()` → `drop(db)` → 重开 → `SELECT id FROM t2` 为空（被放弃语句写入的行不存在）、`SELECT id FROM t` 仍 2000 行（库未被写坏）、`INSERT INTO t2 VALUES (1)` 成功（索引条目已还原并随 checkpoint 落盘为干净状态）。

**Deviations from Plan**

1. **夹具表形状（Plan 内部不自洽，取自洽解）**：Plan 写 `t2(id INT PRIMARY KEY, v INT)`，但 Plan 自己的两条语句 `INSERT INTO t2 SELECT id FROM t` 与 `INSERT INTO t2 VALUES (1)` 都是单列——两列表会在计划期被 `source_width != expected` 拒绝（`insert_count_error`）。改为 `t2(id INT PRIMARY KEY)`：Plan 的两条语句逐字保留，`t(id INT PRIMARY KEY, v INT)` 不动，第一个键恒为 1 的论证不变。
2. **第三处库属隐式事务（Plan 遗漏，分类：Plan 遗漏）**：Task 1.2 的 Targets 只枚举 DML 臂与 CTAS 两处。实现时按「库属隐式事务 = 库自己 begin、自己 commit/abort」全仓核对，发现第三处同型构造——`src/executor/alter_table.rs:83` 的 ALTER TABLE DROP COLUMN 臂（库自己 begin/commit/abort）。规格 requirement「提交前被取消的语句不提交」对「库属隐式事务」与「隐式写语句」是通用表述（不限 INSERT），故同型登记与摘除一并实施：无新设计、无新公共 API、无接口改动，模式与另两处完全一致。分类为 Plan 遗漏，请 Plan Review 裁定。
3. **manager 层新入口的可见性**：`abort_by_id` 与 `discard_tx_version_tables` 为 `pub`，形态照既有 `commit_by_id`（同为 `pub`）。`Database` 侧未新增任何公共方法（登记表与回收入口为 `pub(crate)`），满足 Invariants「库入口不新增公共 API」。
4. **回收入口的错误面**：DML 臂与 CTAS 路径返回 `SqlError { message: "Reclaim failed: {}", sqlstate: None }`（与既有 "Abort failed: {}" / "Commit failed: {}" 同型）；ALTER TABLE 臂受执行器错误面约束（`crate::storage::Result`），返回 `StorageError::ExecutionError("Reclaim failed: ...")`，外层由 pipeline 统一渲染为 `Execution error: ...`。
5. **`Database::commit` 增加一次登记项摘除**：成功提交后、触发自动 checkpoint 判定之前摘除，否则回收会把自己刚提交的事务当成被放弃的那一条（会在 WAL 里写出多余的 `AbortTxn` 并让提交边界报错）。显式事务不在登记表内，摘除是空操作。

**Blocker Handoff**

None required — 本 Cycle 未命中 Gate 5。

**Blocker Resolution**

None — 本 Cycle 未曾阻塞。

**Self-Review**

- **Gate 3（Task Review）逐项结论**：
  - Spec compliance：PASS——requirement「提交前被取消的语句不提交」的三条要点（未提交版本墓碑化、索引条目还原、事务移出活跃集）由既有中止路径 `abort_by_id` 承载；回收时点覆盖「下一次隐式写语句开始之前」与「下一次 checkpoint 之前」；「取消不留下悬空事务」由第三触发点（`maybe_auto_checkpoint` 的活跃集判定之前）覆盖。计划列出的文件、符号、调用者与错误路径均已处理：不变量（未发生取消时语句行为不变）、兼容性（既有事务类用例全绿）、禁止修改项（`Executor` 接口、WAL 帧与位点布局、checkpoint 九步、CLI 信号路径）均未动。先行测试符合任务类型（1.1 新功能先 RED，1.3 在 1.2 之后 GREEN）。
  - Code quality：PASS——diff 无计划外修改（第三处隐式事务站点已按 Deviations 第 2 条显式报告）；错误、边界、状态与资源生命周期正确（中止失败保留登记项、表已不存在先摘版本条目、提交成功才摘登记项）；无新增编译告警与 clippy 告警；无身份证明机制与判定层；测试不会因错误原因通过（1.1 的 RED 原因与 1.3 的负控失败原因均已实跑见证）。
- **未解决的 Critical / Important 问题：无。**
- 自检中发现并已修复：
  1. 夹具的 `ANCHOR_TIMEOUT` 常量未被使用（编译告警）→ 接入 `tokio::time::timeout` 作为锚点观察上限，消除告警。
  2. 回收会误伤「自己刚提交的事务」：`Database::commit` 的成功提交后先摘登记项，并在回收入口加「已不在活跃集 → 跳过」守卫（残留登记项只摘不中止，不写 WAL、不碰版本）。
  3. 中止失败时登记项被误摘 → DML 臂、CTAS 路径、ALTER TABLE 臂三处 abort 路径改为「中止成功才摘除」，中止失败保留登记项由下一个触发点重试。
  4. 1.3 用例的判别力未验证 → 以临时负控（只在 `checkpoint()` 触发点停用回收）实跑见证：1.3 在「rows written by the abandoned statement must not survive the checkpoint, got [[1],[2]]」处失败，与 1.1 的失败原因不同；负控随即回退（`git diff` 核对无残留）。
- 遗留 Minor 问题（不阻塞，未修）：
  1. 回收入口的「已不在活跃集 → 跳过」依赖「每条提交路径都摘登记项」。当前三条路径都摘了；将来新增提交路径而漏摘时，回收会静默跳过该事务（不中止、不写 WAL）——行为安全但回收不彻底。
  2. ALTER TABLE DROP COLUMN 的登记路径本 Cycle 无专门用例覆盖（Plan 的用例只覆盖 INSERT 路径）。
  3. 夹具的 2000 行单行插入使每个用例的准备约 6 秒（两个用例合计约 12–15 秒）。
- 无身份证明机制（无 Hash/指纹/run-id/位点 pin/manifest/时间顺序证明），无项目测试框架之外的判定层：全部断言在 `cargo test` 内，观察点为 `active_transactions()`、`get_tx_versions()`、`execute_sql` 的可观察结果。
- 改动文件：`src/database.rs`、`src/pipeline.rs`、`src/transaction/manager.rs`、`src/executor/alter_table.rs`、`tests/statement_cancellation_test.rs`（新建）、change 的 `tasks.md`（1.1/1.2/1.3 置完成）与本 Cycle 的 Act Response。

**Verification Evidence**

- **Gate 4（Verified Completion）结论**：PASS——Task Definition 的 Verification 全部满足（1.1 的 RED 命令退出码非 0 且失败信息命中允许的两条断言之一；1.2 的两条 GREEN 命令退出码均为 0；1.3 的 GREEN 命令退出码为 0 且两个用例都在输出里通过）。完成声明基于下表的当次真实运行；变更前基线（21/3/4/6）沿用本会话已实跑且覆盖范围未变化的结论，未重跑。

| 验证 | 命令 | 实际结果 |
|---|---|---|
| 1.1 RED（1.2 改动前） | `cargo test --test statement_cancellation_test abandoned_implicit_statement_is_not_committed -- --test-threads=1` | 退出码 101；`test result: FAILED. 0 passed; 1 failed`；失败点为 `Duplicate key`（计划允许的两条断言之一） |
| 1.2 + 1.3 GREEN | `cargo test --test statement_cancellation_test -- --test-threads=1` | 退出码 0；`test result: ok. 2 passed; 0 failed`（两个用例都通过） |
| 1.2 GREEN condition | `cargo test --test explicit_tx_test --test mvcc_abort_test --test mvcc_commit_test --test dml_tx_id_test -- --test-threads=1` | 退出码 0；`explicit_tx_test` 21 passed / 0 failed、`mvcc_abort_test` 3 passed / 0 failed、`mvcc_commit_test` 4 passed / 0 failed、`dml_tx_id_test` 6 passed / 0 failed——与变更前基线 21/3/4/6 一致 |
| 受影响范围（定向，非全量） | `cargo test --test alter_table_test --test ctas_test --test insert_select_test --test checkpoint_test --test checkpoint_command_test --test wal_auto_checkpoint_test --test tx_statement_test --test drop_persistence_test --test truncate_test -- --test-threads=1` | 退出码 0；9 个目标合计 101 passed / 0 failed（ALTER TABLE / CTAS / INSERT SELECT / checkpoint / 自动 checkpoint / DROP 持久化 面） |
| 静态检查 | `cargo build`、`cargo clippy --all-targets` | 无编译错误、无新增告警（lib 的 14 条 clippy 告警全部落在 `async_storage.rs` / `protocol.rs` / `executor_trait.rs` / `cli/lifecycle.rs` / `parser/planner/ddl_dml.rs` / `manager.rs` 的既有代码上，与本次改动无关） |
| 1.3 判别力（负控，一次性） | 临时只在 `checkpoint()` 触发点停用回收后跑 `cargo test --test statement_cancellation_test abandoned_statement_does_not_survive_reopen -- --test-threads=1` | 退出码 101，失败于 `rows written by the abandoned statement must not survive the checkpoint, got [[Number(1)], [Number(2)]]`；负控随后回退并核对无残留 |

说明：`cargo fmt` 只改了本次新增代码的行宽（`src/pipeline.rs` 的 `if let` 与测试文件的 `.expect` 换行），不触语义。格式化后重跑了本表后四行中的全部 GREEN 命令，结果与上表一致——上表结论基于最终代码状态。`cargo fmt --check` 对本次改动的 5 个文件无 diff。

**Persisted Evidence**

None — 模式为 `none`。本 Cycle 的验证结论由上表的真实运行结果承载：全部命令可低成本重跑（定向目标、秒级到分钟级），没有一次性环境即将消失的现场，也没有摘要会丢失决定性结构的长输出。本 Cycle 未创建 `evidence/` 目录。

**Experience Candidates**

None — M19 分批全量回归配方已有 Runbook（R48）覆盖，本 Cycle 未新增可重复或高风险的操作路径；本 Cycle 发现的问题都在当前 change 范围内并已修复（第三处隐式事务站点属 Plan 遗漏，已在 Deviations 报告），无范围外的实质缺陷需要登记为 Issue。

**Remaining Issues**

- 无未解决的 Critical / Important 问题；遗留 Minor 问题三条见 Self-Review。
- 本 Cycle 未跑全量回归——按 change 预算，整个 change 一次，作为收尾验证由 Iteration 001 的 task 2.5 执行（M19 分批串行）。
- 停止条件：当前 Cycle 的 task 1.1/1.2/1.3 全部完成且 Gate 3 / Gate 4 逐项通过，`Act Response` 状态置 `reported`。本 Cycle 未创建下一 Cycle 或下一 Iteration，未归档 change，未同步全局文档（tasks / SNAPSHOT / 项目记忆 / `openspec/specs/` 均未改动）。
- 下一步由用户审计，并调用 `openspec-plan` 对本 Cycle 做 Plan Review。

## Plan Review

- Review Result: rework-required

**Findings**

- **F1（阻塞 Acceptance）—— 回收把「并发执行中仍活着的隐式事务」误判为「被放弃的事务」，未取消的语句被中止。偏差分类：Plan 遗漏（调查遗漏了同一 `Database` 句柄的并发执行模型）。**

  实现把「库属隐式事务登记表」填在 `begin()` 之后（`src/pipeline.rs:573`、`src/pipeline.rs:1006`、`src/executor/alter_table.rs:99`），回收时按 `active_transactions().contains(&tx_id)` 判定「这条事务还活着 → 它是被放弃的那一条」（`src/database.rs:565-575`）。这两个判据对**被放弃的事务**与**仍在并发执行的事务**完全同形：两者的 tx id 都在活跃集里。登记表因此无法区分「调用方已放弃这条语句」与「另一条语句还在跑这条语句」，回收把后者一并 `abort_by_id`。

  可达性（不是理论面）：`Database` 是 `Clone` 且内部为 `Arc`（`src/database.rs:109-149`），`src/network/server.rs:46-58` 把**每个连接** spawn 成独立任务、共享同一个 `Arc<Database>`，`src/network/handler.rs:41` 在其上直接 `execute_sql` → 同一句柄上的并发语句是本项目的服务器常态（多客户端），不是边缘用法。

  实测（Plan Review 期间补跑，Act 未覆盖）：同一句柄上 A = `INSERT INTO t2 SELECT id FROM t`（2000 行，spawn 在独立任务）与 B = `INSERT INTO t3 VALUES (1)` 并发。
  - 变更前基线 `0a08b14`（`git worktree` 独立检出，同一探针）：`B result = Ok(AffectedRows(1))`、`active after B = [2001]`、`A result = Ok(AffectedRows(2000))` → **通过**。
  - 当前工作区代码（同一探针）：`B result = Ok(AffectedRows(1))`、`active after B = []`（A 的活事务 2001 被回收移出活跃集）、`A result = Err(SqlError { message: "Commit failed: transaction error: Transaction 2001 already committed" })` → **失败**。
  结论：A 是**未被取消**的语句，因 B 触发回收而被中止，提交失败、写入被回滚。这是本次改动引入的回归，不是既有行为。

  违反的既有约束：规格 `statement-cancellation-outcome` 的 requirement「取消触发集合是语句 future 被放弃」明文「库入口 SHALL NOT 因此改变未被取消的语句的行为」；本 Cycle `Plan Context` 的 Invariant「未发生取消的语句行为不变」；design D2 的前提（登记表登记的是「被放弃」的事务）。

  归属：Act 忠实实现了 Task 1.2 的 Required behavior，缺口出在 Plan——`Plan Context` 的 Current-State Evidence 只读了 `src/network/connection.rs:18-35`（单连接内「先 await 语句执行完再写响应」），从未确认「多连接共享同一 `Arc<Database>` 且并发执行」这一事实；该事实是 1.2 的 Required behavior 与 Invariant 在并发下不可同时成立的根因。按 `references/iteration-planning.md`，修复需要新的 Current-State Evidence、repair item 与执行依据 → `rework-required`。

- **F2（Act 报告的第三条实质偏差，已裁定：接受，无需回退或收窄；分类为 Plan 遗漏，回写 change 层设计）。** `src/executor/alter_table.rs:95` 的 ALTER TABLE DROP COLUMN 臂是第三处库属隐式事务（库自己 `begin`/`commit`/`abort`），Task 1.2 的 Targets 只枚举了 DML 臂与 CTAS 两处。裁定为 **Plan 遗漏、Act 实施正确**：requirement「提交前被取消的语句不提交」对「库属隐式事务 / 隐式写语句」是通用表述，ALTER TABLE DROP COLUMN 同型同域；不纳入反而留下一个真实的悬空事务泄漏口；Act 的实现与另两处模式完全一致，未新增公共 API、未改接口、未触碰 Forbidden 项。
  枚举完整性已独立核对：全仓 `transaction_manager.begin()` 只有三处库内隐式事务点 —— `src/pipeline.rs:569`（DML 臂）、`src/pipeline.rs:1002`（CTAS）、`src/executor/alter_table.rs:95`（ALTER TABLE DROP COLUMN）；`src/database.rs:496`（`Database::begin()`）与 `src/transaction/session.rs:52` 是**调用方持有**的显式事务，正确地不登记。Act 的三处枚举完整，无遗漏站点。
  回写：`design.md` D2 补记第三处站点；`tasks.md` 任务 1.2 的 Targets 补记第三处站点。Cycle 的 `Plan Context` 不可改写，故不回写该文件，改由 rework Cycle 的 Plan Context 承载完整站点清单。
  随之产生的验收缺口见 F4。

- **F3（非阻塞 Minor）—— 回收的「已不在活跃集 → 跳过」守卫依赖「每条提交路径都摘登记项」。** 将来新增提交路径而漏摘时，回收会静默跳过该事务（不中止、不写 WAL）：行为安全但回收不彻底。Act 已自检记录；不阻塞。

- **F4（阻塞 Acceptance）—— 第三处站点（ALTER TABLE DROP COLUMN）无任何用例覆盖。** Act 为该站点新增了登记/摘除/回收代码，属未经先行测试的行为变更；且 F2 裁定该站点在本 Iteration 的 Acceptance 范围内。Iteration 000 的稳定结果是「被放弃的隐式语句不提交并被回收」，该结论对 ALTER TABLE 站点当前无任何证据支撑 → 作为 rework 的第二个 repair item 关闭。

- **F5（非阻塞 Minor）—— `TransactionManager::abort_by_id` / `discard_tx_version_tables` 为 `pub`。** 二者是公开类型 `TransactionManager` 上的新公共方法，形态照既有 `commit_by_id`（同为 `pub`）。`Database` 入口面未新增公共 API（登记表与回收入口为 `pub(crate)`），requirement 的「不新增取消/公共 API」意图未受影响；收紧可见性属表面问题，不作为阻塞理由。

- **F6（非阻塞 Minor）—— 回收失败的错误面在三处触发点不一致。** DML 臂与 CTAS 路径返回 `SqlError { message: "Reclaim failed: {}" }`；ALTER TABLE 臂受执行器错误面约束返回 `StorageError::ExecutionError("Reclaim failed: ...")`，由 pipeline 渲染为 `Execution error: Reclaim failed: ...`。Act 已在 Deviations 第 4 条记录；不阻塞。

- **F7（非阻塞 Minor）—— 夹具成本。** 2000 行单行插入使每个用例准备约 6 秒（两个用例合计约 12–15 秒）。Act 已记录；不阻塞。

- **F8（通过项）—— 无身份证明机制、无项目测试框架外的判定层。** `tests/statement_cancellation_test.rs` 的观察点只有 `TransactionManager::active_transactions()`、`get_tx_versions()` 与库入口 `execute_sql` 的可观察结果；锚点轮询用 `tokio::task::yield_now()` 让出执行权，`tokio::time::timeout(10s)` 只作夹具失控兜底（触发即判失败，不是阈值判定）；无 Hash/指纹/run-id/位点 pin/manifest/时间顺序证明，无自引用验证，无判定脚本。

- **F9（通过项）—— 1.1 与 1.3 互为有效判别。** 1.1 的 RED 失败原因为 `Duplicate key`（计划允许的两条断言之一）；1.3 的负控（临时只在 `checkpoint()` 触发点停用回收）失败原因为「rows written by the abandoned statement must not survive the checkpoint, got [[1],[2]]」——两者失败点不同，负控可判别力成立。Act 记录负控已回退且 `git diff` 核对无残留，本次 Review 复核工作区 diff 中无负控残留代码（未见任何临时开关）。

- **F10（通过项）—— `abort` → `abort_by_id` 的委托语义不变。** `src/transaction/manager.rs:142-184` 的 diff 显示 `abort` 的原函数体逐字迁入 `abort_by_id`，仅把 `tx.id()` 换成入参 `tx_id`；WAL `AbortTxn` → `abort_cleanup_versions` → 移出活跃集 → 清 `tx_versions` 的顺序与返回值不变。

- **F11（通过项）—— 变更前基线结论仍可沿用。** Act 沿用的 `explicit_tx_test` 21 / `mvcc_abort_test` 3 / `mvcc_commit_test` 4 / `dml_tx_id_test` 6 属变更前基线；本次改动**覆盖**这四个目标（`src/transaction/manager.rs` 的 `abort` 路径与 `src/database.rs` 的 `commit`/回收都在其执行面上），因此基线本身不足以支撑结论——但 Act 已在改动后**重跑**同一命令并得到同样的 21/3/4/6 且退出码 0（Act Response Verification Evidence 表第 3 行），覆盖后的结论有效，本 Review 直接沿用，不重复运行。

- **F12（通过项）—— 事务类目标与并发目标未回归。** Act 未覆盖的两个并发相关目标本次补跑：`cargo test --test concurrent_test --test prepared_statement_test -- --test-threads=1` 退出码 0（`concurrent_test` 10 passed、`prepared_statement_test` 14 passed，均 0 failed）。这两个目标不触发并发写同一句柄的场景，故不覆盖 F1。

**Acceptance Gaps**

- **GAP-1（阻塞）**：Plan Context 的 Invariant「未发生取消的语句行为不变」与 requirement「库入口 SHALL NOT 因此改变未被取消的语句的行为」在「同一 `Database` 句柄并发执行」这一真实可达场景下不成立——并发中的非取消语句被另一条语句的回收中止并提交失败（证据见 F1 的两次实跑对比）。Iteration 000 的 Stable result「被放弃的隐式事务在同一句柄的下一次写语句和下一次 checkpoint 之前已回滚」在修复前无法与「未取消语句的并发行为不变」同时成立。
- **GAP-2（阻塞）**：Iteration 000 的 Stable result 覆盖三处库属隐式事务站点，但 ALTER TABLE DROP COLUMN 站点（`src/executor/alter_table.rs:95`）既无先行测试也无回归用例——该站点的回收行为零证据（F2 裁定站点在范围内 + F4）。
- 非 GAP 记录：1.1 的 RED→GREEN、1.2 的四目标基线、1.3 的 GREEN 与负控判别力、F8–F12 全部满足，不构成缺口。

**Evidence**

- 独立阅读的完整 diff：`git diff src/database.rs src/transaction/manager.rs src/pipeline.rs src/executor/alter_table.rs`（+112 / +39 / +62 / +26 行）＋ 未跟踪新文件 `tests/statement_cancellation_test.rs`（177 行）逐行读过。Act 声明的改动位置与语义与代码一致（登记/摘除/回收三处触发点、`abort_by_id` 委托、`discard_tx_version_tables`、`commit` 后摘除）。
- F1 的决定性证据（Plan Review 补跑，Act 未覆盖）：临时探针 `tests/zz_plan_review_concurrency_probe.rs`（**已删除**，`git status --short` 复核工作区已复原为审查前状态，无残留）：
  - 变更后工作区：`cargo test --test zz_plan_review_concurrency_probe -- --test-threads=1` → `1 failed`，输出 `PROBE: active after B = [] (A tx = 2001)`、`PROBE: A result = Err(SqlError { message: "Commit failed: transaction error: Transaction 2001 already committed", sqlstate: None })`。
  - 变更前基线：`git worktree add /tmp/rtsql-baseline 0a08b14`（独立检出，不触碰本工作区）+ 同一探针 → `1 passed`，输出 `PROBE: active after B = [2001] (A tx = 2001)`、`PROBE: A result = Ok(AffectedRows(2000))`。
  - 探针文件与 worktree 均已删除（`git worktree list` 仅余主工作区）。
- F1 的静态证据：`src/network/server.rs:46-58`（每连接一个 task、共享 `Arc<Database>`）、`src/database.rs:109`（`Database: Clone`）、`src/database.rs:149`（登记表为 `Arc<Mutex<HashSet<u64>>>`，克隆间共享）、`src/database.rs:565-575`（回收只按「是否仍在活跃集」判定）、`src/pipeline.rs:560`（DML 臂回收触发点）、`src/database.rs:621`（`maybe_auto_checkpoint` 回收触发点）、`src/database.rs:819`（`checkpoint` 回收触发点）。
- 沿用未失效结论（来源：Act Response 的 Verification Evidence 表，本 Review 未重复运行）：`cargo test --test statement_cancellation_test -- --test-threads=1` → 2 passed / 0 failed；`cargo test --test explicit_tx_test --test mvcc_abort_test --test mvcc_commit_test --test dml_tx_id_test -- --test-threads=1` → 21/3/4/6 全绿、退出码 0；9 目标受影响面 → 101 passed / 0 failed；1.3 负控判别力实跑。
- 补跑（Act 未覆盖）：`cargo test --test concurrent_test --test prepared_statement_test -- --test-threads=1` → 退出码 0，10 + 14 passed / 0 failed。
- Persisted Evidence 模式为 `none`，本 Cycle 未创建 `evidence/` 目录——按 Review 规则不因此提出问题；本 Review 的探针结论以本节记录为准（结论均为低成本可重跑的定向命令）。
- 本次 Review 未修改任何产品代码；未回滚既有改动；未提交。

**Follow-up Decision**

`rework-required`（非 `replan-required`，非当前 Cycle 修复）。

- 不走 `replan-required`：目标、范围、依赖、requirement、验收边界与验证方式**均不改变**——requirement 文本不变，Iteration Map 不变，验收仍由库入口行为观察，仍不做全量回归（按 change 预算在 Iteration 001 的 2.5 一次）。改变的只是 1.2 内部机制的一个判据（登记表登记「已被放弃的事务」而非「库自己开的事务」），属 repair item 范围内的实现机制修正，不是设计目标或验收边界的改变。
- 不留在当前 Cycle 修复：修复需要新的 Current-State Evidence（同一 `Database` 句柄的并发执行模型，本 Cycle 未建立）、新的 repair item 拆分、新的先行测试与新的执行依据（登记表语义的替换），原 Task Definition 1.2 的 Required behavior 无法继续约束 Act → 按 `references/iteration-planning.md`「当前 Cycle 修复与 rework Cycle 的边界」创建 rework Cycle。
- 不接受当前实现：`rework-required` 不修改 Iteration Map，Iteration 000 在 rework Cycle 获得 `accepted` 前保持未完成；**不展开 Iteration 001**（Iteration 001 的 2.1–2.5 依赖 Iteration 000 的稳定结果——场景 3/4/5 都要先制造一次放弃，回收机制的现场确定性是它们的前提；并发修复未落地前不展开）。
- Act 自评「Gate 3 Code quality PASS、状态与资源生命周期正确」不成立：并发维度未进入 Act 的自检范围，该缺口在 Plan 侧（分类：Plan 遗漏）。

**Iteration Plan Update**

None——Iteration 000 的 Iteration Plan 保持不变（Tasks 1.1/1.2/1.3、Stable result、Verification boundary、Diagnostic boundary、Non-goals 全部不改）。修复以 rework repair item 承载，不修改 Iteration Map。

**Next Cycle**

`iterations/000-abandoned-implicit-transaction-reclaim/001-rework.md`（同 Iteration 目录内的后继 Cycle，repair items：`T1-R1`、`T1-R2`）。

**Next Iteration**

None——当前 Iteration 未被接受，Iteration 001 不展开。
