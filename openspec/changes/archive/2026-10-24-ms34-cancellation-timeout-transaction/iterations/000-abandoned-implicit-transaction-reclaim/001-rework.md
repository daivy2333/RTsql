# Iteration 000 / Cycle 001: 并发安全的隐式事务回收与第三处站点覆盖

## Plan Context

- Status: ready
- Cycle Type: rework

**Cycle Scope**

- Change tasks: 1.2（repair items `T1-R1`、`T1-R2`；任务 1.1/1.3 的既有成果继承，不重做）
- Acceptance gaps: `GAP-1`（并发中的非取消语句被回收误中止）、`GAP-2`（第三处库属隐式事务站点 ALTER TABLE DROP COLUMN 零覆盖）
- Repair items: `T1-R1`、`T1-R2`
- Inherited scope: 父 Cycle `000-initial.md` 已交付且结论未失效的部分——`src/database.rs` 的隐式事务登记表与回收入口（`register_implicit_tx` / `unregister_implicit_tx` / `reclaim_abandoned_implicit_transactions` / `reclaim_implicit_tx`）、`src/transaction/manager.rs` 的 `abort_by_id` 与 `discard_tx_version_tables`、三处回收触发点、`tests/statement_cancellation_test.rs` 的 `abandoned_implicit_statement_is_not_committed`（1.1）与 `abandoned_statement_does_not_survive_reopen`（1.3）两个用例及其夹具；规格 `statement-cancellation-outcome` 的全部 requirement；design D1-D6；任务 1.2 的既有实现（登记/摘除/回收挂载点）作为修复起点保留，不回退重做。
- Excluded scope: 场景 3（提交已完成但未送达）、场景 4/5（显式事务继续或回滚、超时不代替回滚）、后台 I/O 的会话级核对、公共取消或 deadline API、CLI 信号停机行为、WAL 格式与 checkpoint 九步、`Executor` 接口、协作式取消上下文；**被放弃的 ALTER TABLE DROP COLUMN 的「目录已窄化、数据面混合宽度」语义**（见 Risks and Notes，本 Iteration 只定义回收契约，不定义该数据面语义）；全量回归（按 change 预算在 Iteration 001 的任务 2.5 一次）。

**Objective**

同一 `Database` 句柄上并发执行的两条语句互不干扰：仍在执行的语句的隐式事务不会被另一条语句的回收中止，该语句照常提交成功；同时，第三处库属隐式事务站点（ALTER TABLE DROP COLUMN）的回收行为有回归用例守护。

**Background**

父 Cycle `000-initial.md`（`rework-required`）的 Plan Review 发现：回收把「并发执行中仍活着的隐式事务」误判为「被放弃的事务」——登记表填在 `begin()` 之后，回收时按「是否仍在活跃集」判定，两条判据对「被放弃」与「仍在并发执行」完全同形。实测（变更后工作区）：同一句柄上 A = `INSERT INTO t2 SELECT id FROM t`（2000 行，spawn 在独立任务）与 B = `INSERT INTO t3 VALUES (1)` 并发时，B 触发的回收把 A 的活事务移出活跃集，A 以 `Commit failed: transaction error: Transaction 2001 already committed` 失败；同一探针在变更前基线 `0a08b14`（`git worktree` 独立检出）下 A 成功（`Ok(AffectedRows(2000))`、A 的事务在 B 之后仍活跃）。这是本次改动引入的回归，违反 requirement「取消触发集合是语句 future 被放弃」的「SHALL NOT 因此改变未被取消的语句的行为」与父 Cycle 的 Invariant「未发生取消的语句行为不变」。偏差分类：**Plan 遗漏**（父 Cycle 的 Current-State Evidence 未确认「多连接共享同一 `Arc<Database>` 且并发执行」这一事实）。

**Investigation Facts**

- Starting point: 修订 `0a08b14`（MS33 收尾入库），`master` 比 `origin/master` 超前 9。工作区状态：`src/database.rs`、`src/transaction/manager.rs`、`src/pipeline.rs`、`src/executor/alter_table.rs` 四个文件是父 Cycle 的未提交改动（保留，不回退），`tests/statement_cancellation_test.rs` 是父 Cycle 新建的未跟踪文件（保留并在其上追加用例），`openspec/changes/2026-10-24-ms34-cancellation-timeout-transaction/` 是本 change 目录。`.agents/docs/tasks.md` 的未提交修改不属于本 Cycle。
  变更前基线（父 Cycle 已实跑、覆盖范围未失效，直接沿用）：`cargo test --test explicit_tx_test --test mvcc_abort_test --test mvcc_commit_test --test dml_tx_id_test -- --test-threads=1` 退出码 0（21 / 3 / 4 / 6，0 failed）；MS33 收尾的全量终态 2155/0/2（`.agents/docs/SNAPSHOT.md`）。父 Cycle 改动后的同一命令重跑结果同样是 21/3/4/6（父 Cycle Act Response 的 Verification Evidence 表第 3 行）。
- Current-State Evidence:
  - **并发可达性（父 Cycle 遗漏的事实）**：`Database` 是 `Clone` 且内部字段全是 `Arc`（`src/database.rs:109-149`），克隆之间共享同一个 `TransactionManager` 与同一张登记表。`src/network/server.rs:46-58` 把**每个连接** `tokio::spawn` 成独立任务、共享同一个 `Arc<Database>`；`src/network/handler.rs:41` 在其上直接 `execute_sql` → 同一句柄上的并发语句是本项目服务器端的常态（多客户端同时发语句），不是边缘用法。`src/transaction/manager.rs` 的 `exec_gate` 不在此列——它是**单条预编译语句**的并发闸门（`src/database.rs:865`、`src/database.rs:888`），不串行化 `Database::execute_sql`；`tests/prepared_statement_test.rs` 的 `tokio::join!` 并发写之所以不触发本缺陷，正是因为那三条 INSERT 共用同一条预编译语句、被 `exec_gate` 串行化。
  - **登记表与回收的当前实现**：`src/database.rs:149` 登记表为 `Arc<Mutex<HashSet<u64>>>`；`register_implicit_tx`（`src/database.rs:519`）/ `unregister_implicit_tx`（`src/database.rs:524`）是登记与摘除；`reclaim_abandoned_implicit_transactions`（`src/database.rs:537`）先 `drain()` 整个登记表再逐个 `reclaim_implicit_tx`，失败者回填；`reclaim_implicit_tx`（`src/database.rs:565`）的判定是「`active_transactions()` 不含该 tx id → 跳过，否则 `abort_by_id`」——**被放弃的事务与仍在执行的事务在这里同形**（两者的 tx id 都在活跃集里），这是缺陷的直接成因。
  - **三处登记点与三处回收触发点**：登记在 `src/pipeline.rs:573`（DML 臂）、`src/pipeline.rs:1006`（CTAS 路径）、`src/executor/alter_table.rs:99`（ALTER TABLE DROP COLUMN 臂）；摘除在 `src/pipeline.rs:603`、`src/pipeline.rs:636`、`src/pipeline.rs:1017`、`src/pipeline.rs:1058`、`src/pipeline.rs:1085`、`src/executor/alter_table.rs:118`、`src/executor/alter_table.rs:134`、`src/database.rs:514`。回收触发点在 `src/pipeline.rs:560`（DML 臂 `begin()` 之前）、`src/pipeline.rs:881`（CTAS 入口）、`src/executor/alter_table.rs:87`（ALTER TABLE 臂）、`src/database.rs:621`（`maybe_auto_checkpoint` 的活跃集判定之前）、`src/database.rs:819`（`checkpoint` 之前，覆盖 `close()`）。
  - **库属隐式事务的完整清单**（本 Review 独立核对全仓 `transaction_manager.begin()`）：库内隐式事务只有三处——`src/pipeline.rs:569`（DML 臂）、`src/pipeline.rs:1002`（CTAS）、`src/executor/alter_table.rs:95`（ALTER TABLE DROP COLUMN）。`src/database.rs:496`（`Database::begin()`）与 `src/transaction/session.rs:52`（`TransactionSession::begin`）是**调用方持有**的显式事务，正确地不登记、不回收（spec requirement「显式事务继续还是回滚由调用方决定」）。`src/pipeline.rs:503-517` 的 `PhysicalPlan::InsertSelect` 先物化源、再递归进 DML 臂，因此 INSERT SELECT 只有**一个**隐式事务（内层 DML 臂那个），外层不开事务、不重复登记。
  - **`abort_by_id` 与 `discard_tx_version_tables`**（`src/transaction/manager.rs:158-186`、`src/transaction/manager.rs:187-206`）：`abort` 已改为委托 `abort_by_id`，函数体逐字不变（父 Cycle Review F10 已核对）。`discard_tx_version_tables` 用于「表已被 DROP」的路径：先摘掉该事务在这些表上的版本条目再中止，让中止能进行到底。这两个入口在返工后仍需保留（回收仍然走既有中止路径）。
  - **回收与提交的顺序约束**：`Database::commit`（`src/database.rs:504-515`）先 `transaction_manager.commit`（提交成功后事务已移出活跃集），再摘登记项，最后 `maybe_auto_checkpoint()` → 回收触发点。因此「已提交但登记项还在」的事务会被 `reclaim_implicit_tx` 的「已不在活跃集 → 跳过」守卫跳过，**不写多余的 `AbortTxn`**——这个守卫同时覆盖 Iteration 001 场景 3（提交已完成但调用方未收到结果），返工不得削弱它。
  - **ALTER TABLE DROP COLUMN 的事务边界（只影响 `T1-R2` 的断言选择）**：`src/storage/data/table_manager.rs:693-980` 的 `alter_drop_column` 先改目录并 `flush_all`（新 schema 持久化，MS33 的崩溃窗口设计），再在事务内做逐行的 DELETE + INSERT 改写。目录窄化**不在**事务内，事务只覆盖数据面改写。因此被放弃的 ALTER TABLE 在回收后仍是「目录已窄化、数据面混合宽度」的状态——本 Cycle 不定义该数据面语义（见 Non-goals 与 Risks and Notes）。
  - **既有测试夹具**（`tests/statement_cancellation_test.rs`，父 Cycle 产出，可直接复用）：`setup(&dir)` 建 `t(id INT PRIMARY KEY, v INT)` 并预置 2000 行、建 `t2(id INT PRIMARY KEY)`；`anchor_tx(&db)` 返回「活跃且 `get_tx_versions(tx_id).len() >= 2`」的事务 id；`abandon_statement(&db, sql)` 用 `Box::pin` + `tokio::select!`（`biased`，锚点支在前）在锚点命中时 `drop(fut)` 放弃语句 future，观察循环用 `tokio::task::yield_now()` 让出执行权、`tokio::time::timeout(10s)` 只作夹具失控兜底；`expect_affected(&db, sql)`、`expect_rows(&db, sql)` 是库入口结果的断言助手。
- Code and Critical Path: 库入口 `Database::execute_sql`（`src/database.rs:461-463`）→ `pipeline::execute` → `execute_stage_routed`（DML 臂 `src/pipeline.rs:536-640`）→ `execute_executor(executor).await`（放弃点落在此处与其后 `commit` 之间的任一 await 点）→ `target_db.commit(tx)` → `Database::commit` → `maybe_auto_checkpoint` → 回收触发点。CTAS 路径 `execute_ctas_routed`（`src/pipeline.rs:875-1090`）；ALTER TABLE 臂 `src/executor/alter_table.rs:80-138` → `TableManager::alter_drop_column`（`src/storage/data/table_manager.rs:693`）。回收要用的中止路径是 `TransactionManager::abort_by_id`（`src/transaction/manager.rs:158`）：写 `AbortTxn` → `abort_cleanup_versions`（墓碑化未提交版本、还原索引条目）→ 移出活跃集 → 清 `tx_versions`。

**Implementation Guidance**

- 把登记表的语义从「库自己开的事务」改成「调用方已放弃其语句的事务」——这是修复 T1-R1 的必要条件，不是风格选择：在 `begin()` 时登记，回收点就无法把「被放弃」与「仍在执行」区分开。
- 直接形态：每个库属隐式语句在 `begin()` 之后持有一个**语句作用域的守卫**，守卫在 `Drop` 时把 tx id 记入登记表（同步插入，无 I/O，不 spawn 任务）；语句正常提交或中止成功后显式解除守卫，使 `Drop` 不记入。语句被放弃时守卫随 future 析构 → tx id 入表 → 下一个回收触发点按既有中止路径回收它。
- 该形态不违反 design D2 的既有取舍：D2 拒绝的是「在 `Drop` 里 `spawn` 回收任务」（回收与下一条语句之间无顺序保证），回收仍然只发生在三个库自有同步点；守卫只把「被放弃」这个事实记下来，回收的时序保证不变。design D2 已按父 Cycle Review 的裁定补记第三处站点与该语义修正。
- 解除必须在提交/中止成功之后**立即**完成，否则 `Database::commit` 内部的 `maybe_auto_checkpoint` 回收触发点会看到自己的事务。`reclaim_implicit_tx` 的「已不在活跃集 → 跳过」守卫保留不动（它同时守住 Iteration 001 场景 3 的提交窗口）。
- 「`begin()` 后登记 / `commit`、`abort` 后摘除」的调用点（`src/pipeline.rs:573`、`src/pipeline.rs:1006`、`src/executor/alter_table.rs:99`）与 `Database::commit` 里的摘除（`src/database.rs:514`）随该替换一并调整；由本次改动产生的孤儿调用点与死代码一并清除（AGENTS.md › Surgical Changes）。
- 实现形态（守卫的具体类型、字段名、辅助函数拆分）由 Act 决定，不写入 Task Definition；Task Definition 只约束可观察行为。

**Task Definitions**

### T1-R1: 回收只作用于已被放弃的事务，并发语句互不干扰

- Requirement/Scenario: 取消触发集合是语句 future 被放弃 / 丢弃、超时与断开是同一类事件（该 requirement 明文「库入口 SHALL NOT 因此改变未被取消的语句的行为」）；提交前被取消的语句不提交 / 隐式语句在执行中被放弃后不提交；取消不留下悬空事务 / 取消不留下悬空事务
- Depends on: None（父 Cycle 的 1.1/1.2/1.3 成果保留在起点上，本 repair item 只修并发缺陷）
- Targets: `src/database.rs`（登记表语义与回收入口）、`src/pipeline.rs`（DML 臂与 CTAS 路径的登记/摘除点）、`src/executor/alter_table.rs`（ALTER TABLE 臂的登记/摘除点）、`src/transaction/manager.rs`（`abort_by_id`、`discard_tx_version_tables` 保持可用）、`tests/statement_cancellation_test.rs`（新增并发用例）
- Current behavior: 登记表在 `begin()` 之后填入 tx id，回收按「是否仍在活跃集」判定。被放弃的事务与仍在并发执行的事务在该判据下同形 → 同一句柄上另一条语句的回收会中止前者。实测：A 与 B 并发时 A 以 `Commit failed: transaction error: Transaction 2001 already committed` 失败（变更前基线下 A 成功）。
- Required behavior:
  1. 回收只中止「其语句 future 已被放弃」的事务。一个**仍在执行中**的语句的隐式事务，在同一 `Database` 句柄上另一条语句执行期间保持活跃，且该语句照常提交成功（返回正常的受影响行数，不返回错误）。
  2. 既有取消语义不变：被放弃的语句仍然不提交；它的隐式事务仍然在「下一条隐式写语句开始之前」与「下一次 checkpoint 之前」完成回收（活跃集清空、索引条目还原、同一个键可以再次插入成功）。
  3. 回收的失败语义不变：中止本身失败时回收返回点名错误并保留登记项，由下一个触发点重试。
  4. 库入口不新增公共 API；`Executor` 接口不变；WAL 记录格式、位点布局与 checkpoint 九步顺序不变。
- Preserve: 未发生取消的语句行为不变（含并发场景）；既有 `commit`/`abort` 路径语义不变；`active_transactions()`、`tx_versions()`、`get_tx_versions()` 的语义不变；`Database::rollback` 的显式回滚语义不变；显式事务的生死仍由调用方决定（绝不登记、绝不回收）。
- Forbidden: 为让并发用例通过而串行化语句执行（例如在 `Database` 上加全局执行闸门）或改变语句的并发语义；新增取消或 deadline 公共 API；修改 `Executor` 接口；用固定 sleep 阈值、哈希、指纹、run-id、时间顺序之类判定代替行为观察；在项目测试框架之外建判定层；改 WAL 帧、位点布局或 checkpoint 步骤顺序；动 CLI 信号路径。
- First test: 在 `tests/statement_cancellation_test.rs` 新增用例 `concurrent_statement_does_not_abort_other_statement_tx`。夹具：复用现有 `setup(&dir)`（`t` 2000 行、`t2`），另建 `t3(id INT PRIMARY KEY)`；A = `INSERT INTO t2 SELECT id FROM t`（2000 行）以 `tokio::spawn` 在**克隆出的同一 `Database` 句柄**上执行（`db.clone()` 与服务器「多连接共享一个 `Arc<Database>`」同形）；锚点用现有 `anchor_tx(&db)` 命中 A 的事务（活跃且版本数 ≥2）——命中即证明 A 在执行期；随后在同一句柄上 `await` 执行 B = `INSERT INTO t3 VALUES (1)` 到完成。命令：`cargo test --test statement_cancellation_test concurrent_statement_does_not_abort_other_statement_tx -- --test-threads=1`。预期（在 T1-R1 修复之前）：失败，且失败原因是「A 的事务在 B 之后不再活跃」或「A 的结果不是成功」——不是编译错误、夹具错误或锚点超时。
- GREEN condition: 该用例通过；`cargo test --test statement_cancellation_test -- --test-threads=1` 退出码 0（1.1、1.3 与本用例全部通过，failed 计数为 0）；`cargo test --test explicit_tx_test --test mvcc_abort_test --test mvcc_commit_test --test dml_tx_id_test -- --test-threads=1` 退出码 0（21 / 3 / 4 / 6，与变更前基线一致）。
- Verification: 上列三条命令的退出码都是 0；并发用例的失败原因与 1.1（`Duplicate key`）、1.3（未提交行随 checkpoint 落盘）三者互不相同。
- Stop when: 要让「回收不误伤并发中的语句」成立就必须改变语句的并发语义（例如串行化同一句柄上的语句执行）或必须新增公共 API——那是范围扩大，返回 Plan；或既有事务类用例在改动后失败且修复要动 `TransactionManager::abort` / `abort_by_id` 的既有语义。

### T1-R2: 第三处库属隐式事务站点（ALTER TABLE DROP COLUMN）的回收回归用例

- Requirement/Scenario: 提交前被取消的语句不提交 / 隐式语句在执行中被放弃后不提交（该 requirement 对「库属隐式事务」是通用表述，涵盖 DDL 隐式事务）；取消不留下悬空事务 / 取消不留下悬空事务
- Depends on: T1-R1（共用修复后的回收语义与登记表）
- Targets: `tests/statement_cancellation_test.rs`（新增 ALTER TABLE 站点用例）
- Current behavior: `src/executor/alter_table.rs:95` 的 ALTER TABLE DROP COLUMN 臂是库属隐式事务，父 Cycle 已给它加上登记/摘除/回收，但**没有任何用例覆盖**——该站点的回收行为零证据。
- Required behavior: 新增用例 `abandoned_alter_table_drop_column_is_reclaimed`：制造一次放弃（`ALTER TABLE t DROP COLUMN v`，锚点命中即 `drop(fut)`）→ 执行下一条写语句触发回收 → 该回收**成功返回**（不报 `Reclaim failed`）→ 回收之后 `active_transactions()` 为空（没有悬空事务）→ 下一条写语句照常成功。断言只覆盖回收契约本身（事务被回收、无悬空、回收不报错），**不断言被放弃的 ALTER TABLE 之后表的数据面形态**（目录窄化与数据面混合宽度的语义见 Non-goals 与 Risks and Notes）。
- Preserve: `ALTER TABLE DROP COLUMN` 的既有成功路径行为不变（成功时目录窄化 + 数据改写 + 提交，语义与改动前逐字节一致）；`alter_drop_column` 的既有崩溃窗口设计不变；`src/storage/data/table_manager.rs` 不改。
- Forbidden: 改 `alter_drop_column` 的目录/数据面事务边界；为了让用例通过而改变 ALTER TABLE 的成功语义；把「被放弃的 ALTER TABLE 之后表应长什么样」写成本用例的断言（该语义本 Iteration 不定义）；用文件字节比较代替库入口的可观察结果。
- First test: 在 `tests/statement_cancellation_test.rs` 新增用例 `abandoned_alter_table_drop_column_is_reclaimed`。夹具：`setup(&dir)` 的 `t` 有 2000 行且含 `v` 列 → 放弃 `ALTER TABLE t DROP COLUMN v`（锚点用现有 `anchor_tx(&db)`）→ 执行下一条写语句 → 断言。命令：`cargo test --test statement_cancellation_test abandoned_alter_table_drop_column_is_reclaimed -- --test-threads=1`。
- GREEN condition: 该用例通过；`cargo test --test statement_cancellation_test -- --test-threads=1` 退出码 0（四个用例全部通过）。
- Verification: 同一条命令；本用例的失败原因与 1.1、1.3、T1-R1 用例互不相同（它看的是回收是否报错与活跃集是否清空，不是同键再插与落盘）。
- Stop when: 该用例的通过需要改动 `alter_drop_column` 的目录/数据面事务边界（把目录窄化纳入事务）——那是设计变更，返回 Plan；或回收该事务必然报错而根因在中止路径的既有语义——返回 Plan。

**Invariants**

- 未发生取消的语句行为不变（**含并发场景**：同一句柄上的并发语句互不中止）。
- 库入口不新增公共 API，`Executor` 接口不变。
- 显式事务的生死仍由调用方决定：任何取消路径都不提交、不回滚调用方的事务；显式事务不进登记表、也不被回收。
- WAL 记录格式、位点布局、checkpoint 九步顺序不变。
- 被放弃的事务永远不会以「已提交」的面貌出现在恢复期：没有提交帧的事务在恢复期仍被标记为中止。
- 「已提交但调用方未收到结果」的事务不被回收误伤（`reclaim_implicit_tx` 的「已不在活跃集 → 跳过」守卫保留）。

**Non-goals**

- 场景 3（提交已完成但调用方未送达）、场景 4/5（显式事务继续或回滚、超时不代替回滚）、后台 I/O 的会话级核对、公共取消或 deadline API、协作式取消上下文、CLI 信号停机行为、真实断电与 `kill -9`、WAL 格式与 checkpoint 九步。
- 被放弃的 ALTER TABLE DROP COLUMN 的**数据面语义**（目录已窄化、数据面混合宽度的后果）——本 Iteration 只定义回收契约，不定义该语义，也不改 `alter_drop_column` 的事务边界。
- 语句并发模型的改造（不引入全局执行闸门，不改变同一句柄上语句的并发语义）。
- 全量回归（按 change 预算，整个 change 一次，作为收尾验证在 Iteration 001 的任务 2.5）。

**Acceptance**

- GAP-1 关闭：并发用例 `concurrent_statement_does_not_abort_other_statement_tx` 通过——同一句柄上 A 与 B 并发时，A 的隐式事务在 B 执行期间保持活跃，A 正常提交成功（不是「未被取消的语句被改变行为」）。
- GAP-1 关闭的旁证：既有取消语义未退化——`abandoned_implicit_statement_is_not_committed` 与 `abandoned_statement_does_not_survive_reopen` 仍通过；`explicit_tx_test` / `mvcc_abort_test` / `mvcc_commit_test` / `dml_tx_id_test` 仍全绿（21 / 3 / 4 / 6）。
- GAP-2 关闭：ALTER TABLE DROP COLUMN 站点的回收用例 `abandoned_alter_table_drop_column_is_reclaimed` 通过——回收成功返回、活跃集清空、无悬空事务。
- 四个用例的失败原因互不相同（各自可判别），无身份证明机制、无项目测试框架外的判定层。
- 变更前基线（21 / 3 / 4 / 6）仍成立，父 Cycle 的 1.1 / 1.3 结论仍成立。

**Verification**

全部从库入口观察（`Database::open` / `execute_sql` / `close` / 重开），并发用例的观察点是 `TransactionManager::active_transactions()`、`get_tx_versions()` 与库入口的语句结果；不使用哈希、指纹、run-id、位点 pin、manifest、时间顺序等身份证明机制，不在本项目测试框架之外建判定层。锚点轮询用 `yield_now` 让出执行权，`tokio::time::timeout` 只作夹具失控兜底（触发即判失败，不是阈值判定）。**本 Cycle 不跑全量回归**——高开销验证按 change 预算，整个 change 一次，作为收尾验证在 Iteration 001 的任务 2.5（M19 分批串行）。

**Gate 2 Readiness**

PASS。范围、requirement、依赖、验收边界与验证方式均不变，返工只关闭父 Cycle Review 的 `GAP-1` 与 `GAP-2`；执行依据由本 Plan Context 重新建立（并发可达性事实、三处站点清单、登记表语义修正、两个 repair item 的完整 Task Definition），不要求 Act 回读父 Cycle。用户批准依据沿用父 Cycle Gate 2 的同一条：2026-10-24 用户原话「/openspec-autopilot 把已经规划的三个ms逐步完成，要求遵守各种规范，尤其是全量测试时候的多批次串行」与补充「因为这个项目有点大，三个ms也是串行实施就好」——goal 指令构成目标范围内阶段推进的显式授权，本次返工在该授权范围内且不扩大 change 范围。无 SKIPPED、无 BLOCKED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 「被放弃的 ALTER TABLE DROP COLUMN 之后表的数据面形态」是本 change 范围外的实质问题：`alter_drop_column` 先把目录窄化并 `flush_all`（`src/storage/data/table_manager.rs:793-802`），窄化不在事务内；回收只回滚数据面。结果是「目录已窄化 + 数据面混合宽度」。该形态不由本 Cycle 决定，Plan Review 把它作为 Issue 候选报告（只报告不登记）。T1-R2 因此只断言回收契约，不断言数据面。
- 回收失败时的错误面在三处触发点不一致（DML 臂与 CTAS 路径是 `SqlError { message: "Reclaim failed: {}" }`，ALTER TABLE 臂是 `StorageError::ExecutionError("Reclaim failed: ...")` 被渲染为 `Execution error: Reclaim failed: ...`）——父 Cycle 已记录为 Minor finding，本次返工不必统一（属表面问题）。
- `TransactionManager::abort_by_id` / `discard_tx_version_tables` 为 `pub`（形态照既有 `commit_by_id`）——父 Cycle 已记录为 Minor finding，本次返工不必收紧（`Database` 入口面未新增公共 API）。
- 「回收的『已不在活跃集 → 跳过』守卫依赖每条提交路径都摘登记项」——父 Cycle 已记录为 Minor finding；本 Cycle 的语义修正会改变这条依赖的形态，返工时注意该守卫仍需覆盖「提交已完成但登记项仍在」的窗口。
- 夹具成本：现有 `setup()` 的 2000 行单行插入约 6 秒/用例；本 Cycle 新增两个用例，合计准备成本约 24 秒量级。可接受，不要求优化。
- Iteration 001 的场景 3（提交已完成但调用方未送达）依赖「提交完成之后、结果送达之前」这个窗口稳定出现；本 Cycle 的语义修正（登记表只在语句被放弃时才有条目）不改变该窗口的行为，但返工时不得让「提交完成但 future 被放弃」的事务进入可回收状态——`reclaim_implicit_tx` 的「已不在活跃集 → 跳过」守卫是它的保护，返工不得削弱。
- 若 T1-R1 的修复被实现成「同一句柄上串行化语句执行」，T1-R1 会通过但违反 Forbidden——Review 会以「并发语义被改变」为由打回。

## Act Response

- Status: reported

**Implemented**

- **T1-R1 — 登记表语义从「库属隐式事务」改为「已被放弃语句的事务」，并发语句互不干扰**：
  - `src/database.rs`：新增 `pub(crate) struct ImplicitTxGuard`（字段 `registry: Arc<Mutex<HashSet<u64>>>`、`tx_id: u64`、`armed: bool`）——语句作用域守卫。`Drop` 在 `armed` 时把 tx id 同步插入登记表（无 I/O、不 spawn 任务）；`ImplicitTxGuard::disarm()` 解除武装，`Drop` 不记入。新增 `Database::implicit_tx_guard(tx_id) -> ImplicitTxGuard`（`pub(crate)`，无新公共 API）。删除 `register_implicit_tx` / `unregister_implicit_tx`（孤儿调用点一并清除，见 Deviations 第 2 条）；`Database::commit` 随之不再摘登记项。`reclaim_abandoned_implicit_transactions` / `reclaim_implicit_tx`（含「已不在活跃集 → 跳过」守卫）逐字未动。
  - `src/pipeline.rs`：DML 臂（`src/pipeline.rs:557-566`）在 `begin()` 之后建守卫；提交成功（`:652`）与中止成功（`:638`、`:600`）后 `guard.disarm()`；提交失败、中止失败、执行器构造失败且无表可中止时保持武装 → 守卫析构记入登记项，由下一个触发点重试。CTAS 路径（`:1007-1009` 建守卫，`:1016`、`:1057` 中止成功后解除，`:1079` 提交成功后解除）同型。
  - `src/executor/alter_table.rs`：ALTER TABLE DROP COLUMN 臂（`:99-101` 建守卫，`:118` 提交成功后解除，`:131` 中止成功后解除）同型。
  - 语义效果：登记项的唯一来源是「语句 future 被放弃」（守卫析构）；仍在并发执行中的语句不在登记表内，同一句柄上另一条语句的回收不会碰它。**未引入任何执行闸门**——语句并发语义未变（并发用例中 B 在 A 仍在执行时即完成，见 Verification Evidence）。
  - `src/transaction/manager.rs` 未改：`abort_by_id`、`discard_tx_version_tables` 保持可用（回收仍走既有中止路径）。
- **T1-R2 — 第三处库属隐式事务站点（ALTER TABLE DROP COLUMN）的回收回归用例**：`tests/statement_cancellation_test.rs` 新增 `abandoned_alter_table_drop_column_is_reclaimed`——放弃 `ALTER TABLE t DROP COLUMN v`（锚点命中即 `drop(fut)`）→ 下一条写语句触发回收 → 回收成功返回（不报 `Reclaim failed`）→ 回收之后 `active_transactions()` 为空 → 下一条写语句照常成功。断言只覆盖回收契约（回收是否报错、活跃集是否清空），**不断言数据面形态**。

**Deviations from Plan**

1. **T1-R2 的先行测试在继承实现上首次运行即通过（GREEN）**：该站点的登记/摘除/回收实现由父 Cycle `000-initial` 交付并在返工起点上保留，本 repair item 的交付物是 F4 指出的**零覆盖缺口**（回归用例）。因此 T1-R2 不存在「修复前 RED」这一形态——这是覆盖缺口分支，不是新功能分支。修复前基线已实跑留证（见 Verification Evidence 第 2 行）：用例在父实现上即通过，且修复后仍通过 → 该用例守护的是本次语义修正后的 ALTER TABLE 站点（若该站点漏建守卫，活跃集不会清空，用例失败）。
2. **`register_implicit_tx` / `unregister_implicit_tx` 被删除**（Plan Implementation Guidance 已指示「随该替换一并调整；由本次改动产生的孤儿调用点与死代码一并清除」）。`Database::commit` 的摘除点一并移除：新语义下登记项只由守卫析构产生，提交路径不再有摘除职责（保留它会在语义上与守卫的解除重复，且让「被放弃事务」的登记项可能被误删）。
3. **守卫形态由 Act 选定**（Plan 明示「实现形态……由 Act 决定，不写入 Task Definition」）：`ImplicitTxGuard` + `Database::implicit_tx_guard`，`pub(crate)`，库入口面零新增公共 API。

**Blocker Handoff**

None required — 本 Cycle 未命中 Gate 5。

**Blocker Resolution**

None — 本 Cycle 未曾阻塞。

**Self-Review**

- **Gate 3（Task Review）逐项结论**：
  - Spec compliance：PASS——requirement「取消触发集合是语句 future 被放弃」由登记表语义修正直接承载（登记项唯一来源＝守卫析构＝语句被放弃）；「SHALL NOT 因此改变未被取消的语句的行为」由并发用例实证（同一句柄上 A 与 B 并发，A 的事务在 B 期间保持活跃且 A 提交成功）；「提交前被取消的语句不提交」与「取消不留下悬空事务」由 1.1/1.3 用例保持（仍通过）。计划列出的文件、符号、调用者与错误路径均已处理；不变量未破：库入口零新增公共 API（`implicit_tx_guard` 与 `ImplicitTxGuard` 均 `pub(crate)`）、`Executor` 接口未动、显式事务不进登记表（守卫只在三处库属隐式事务点创建）、WAL 帧与位点布局与 checkpoint 九步未动、「已提交但调用方未收到结果」的事务仍被 `reclaim_implicit_tx` 的「已不在活跃集 → 跳过」守卫跳过。Forbidden 逐条未触：未加全局执行闸门（并发语义未变，见下）、未新增取消/deadline 公共 API、未改 `Executor` 接口、无 sleep 阈值/哈希/指纹/run-id/时间顺序判定、无框架外判定层、未动 CLI 信号路径。先行测试符合任务类型：T1-R1 先 RED 后 GREEN；T1-R2 为覆盖缺口分支（修复前基线已留证）。
  - Code quality：PASS——diff 无计划外修改（只动 `src/database.rs`、`src/pipeline.rs`、`src/executor/alter_table.rs`、`tests/statement_cancellation_test.rs`；`src/transaction/manager.rs` 本 Cycle 未动）；错误、边界、状态与资源生命周期正确（提交/中止成功后解除守卫、失败时保持武装以保留重试语义；守卫 `Drop` 只做同步插入，不做 I/O、不 spawn 任务，回收时序仍由三个库自有触发点承担）；无新增编译告警与 clippy 告警；测试不会因错误原因通过（四个用例的失败原因互不相同，见下）。
- **未解决的 Critical / Important 问题：无。**
- 自检中发现并已处理：
  1. `Database::commit` 的摘除点在语义修正后成为孤儿调用点 → 随替换移除（Deviations 第 2 条）。
  2. 「中止失败保留登记项」的旧语义在新机制下会丢失 → 改为「守卫保持武装」，三处 abort 路径与 DML 臂的执行器构造失败路径均按「中止成功才解除」处理，失败时守卫析构仍记入登记项，下一个触发点重试（旧语义完整保留）。
  3. 残留登记项是否会误伤新事务 → `TransactionId::allocate` 单调递增、恢复后 `advance_past` 保证 id 不复用，故残留登记项只可能指向那个被放弃的事务。
  4. 守卫 `Drop` 持登记 mutex 插入是否可能与持锁路径重入 → 三个触发点持锁期间不做 await（`drain`/`extend` 均为同步短段），无重入路径。
  5. 格式化：`cargo fmt --check` 只在测试文件报一处行宽差异 → 已格式化；格式化只改测试文件，库代码字节未变（`cargo fmt --check` 对三个库文件此前即无 diff）。
- 遗留 Minor 问题（不阻塞，未修）：
  1. 回收入口的错误面在三处触发点仍不一致（DML 臂与 CTAS 路径是 `Reclaim failed: {}`，ALTER TABLE 臂是 `Execution error: Reclaim failed: ...`）——父 Cycle 已记录，Plan 明示本次不必统一。
  2. `TransactionManager::abort_by_id` / `discard_tx_version_tables` 仍为 `pub`——父 Cycle 已记录的 Minor，本次未收紧。
  3. 「future 在 `transaction_manager.commit` 中途被放弃」的窗口仍可能让后续回收写出 `AbortTxn`：与父实现同形（父实现的登记窗口更宽——从 `begin()` 起就有登记项；新实现只在 future 被放弃后才有登记项），属 Iteration 001 场景 3 的范围，本 Iteration 不定义。
- 四个用例的失败原因互不相同：1.1＝回收未发生（活跃集非空 / 同键再插 `Duplicate key`）；1.3＝未提交行随 checkpoint 落盘；T1-R1＝并发语句的事务被另一条语句的回收中止；T1-R2＝ALTER TABLE 站点的回收报错（`Reclaim failed`）或该站点事务未被回收。无身份证明机制、无项目测试框架外的判定层：全部断言在 `cargo test` 内，观察点为 `active_transactions()`、`get_tx_versions()` 与库入口的语句结果；锚点轮询用 `yield_now`，`tokio::time::timeout` 只作夹具失控兜底。
- 改动文件：`src/database.rs`、`src/pipeline.rs`、`src/executor/alter_table.rs`、`tests/statement_cancellation_test.rs`、本 Cycle 的 Act Response。change 的 `tasks.md` 未改（rework Cycle 只记录本地 repair item 状态，不新增全局 task；任务 1.2 的状态由 Plan Review 收尾时统一同步）。

**Verification Evidence**

- **Gate 4（Verified Completion）结论**：PASS——T1-R1 与 T1-R2 的 Task Definition 的 Verification 全部满足；完成声明基于下表的当次真实运行。全量回归本 Cycle 未跑（按 change 预算，整个 change 一次，作为收尾验证在 Iteration 001 的任务 2.5，M19 分批串行）。

| 验证 | 命令 | 实际结果 | 本次重跑 / 沿用 |
|---|---|---|---|
| T1-R1 RED（实现改动前） | `cargo test --test statement_cancellation_test concurrent_statement_does_not_abort_other_statement_tx -- --test-threads=1` | 退出码 101；`test result: FAILED. 0 passed; 1 failed`；失败点 `the concurrently running statement's transaction must stay active while the other statement runs, active transactions are []`——命中计划允许的「A 的事务在 B 之后不再活跃」，不是编译错误、夹具错误或锚点超时 | 本次重跑 |
| T1-R2 修复前基线 | `cargo test --test statement_cancellation_test abandoned_alter_table_drop_column_is_reclaimed -- --test-threads=1` | 退出码 0；`test result: ok. 1 passed; 0 failed`——该站点实现由父 Cycle 交付，本 repair item 补覆盖（Deviations 第 1 条） | 本次重跑 |
| T1-R1 GREEN | `cargo test --test statement_cancellation_test concurrent_statement_does_not_abort_other_statement_tx -- --test-threads=1` | 退出码 0；`test result: ok. 1 passed; 0 failed` | 本次重跑 |
| 取消面整文件（4 个用例） | `cargo test --test statement_cancellation_test -- --test-threads=1` | 退出码 0；`test result: ok. 4 passed; 0 failed`（`abandoned_alter_table_drop_column_is_reclaimed`、`abandoned_implicit_statement_is_not_committed`、`abandoned_statement_does_not_survive_reopen`、`concurrent_statement_does_not_abort_other_statement_tx` 全通过） | 本次重跑（格式化后复跑一次，结论一致） |
| 事务类基线 | `cargo test --test explicit_tx_test --test mvcc_abort_test --test mvcc_commit_test --test dml_tx_id_test -- --test-threads=1` | 退出码 0；21 / 3 / 4 / 6，0 failed——与变更前基线一致 | 本次重跑（父 Cycle 结论因实现变更失效，故重跑确认） |
| Review 点名的两个目标 | `cargo test --test concurrent_test --test prepared_statement_test -- --test-threads=1` | 退出码 0；`concurrent_test` 10 passed / 0 failed、`prepared_statement_test` 14 passed / 0 failed | 本次重跑（父 Cycle 未覆盖这两个目标） |
| 受影响面（9 目标，定向非全量） | `cargo test --test alter_table_test --test ctas_test --test insert_select_test --test checkpoint_test --test checkpoint_command_test --test wal_auto_checkpoint_test --test tx_statement_test --test drop_persistence_test --test truncate_test -- --test-threads=1` | 退出码 0；9 个目标合计 101 passed / 0 failed（与父 Cycle 的 101 一致） | 本次重跑（父 Cycle 结论因实现变更失效，故重跑确认） |
| 静态检查 | `cargo build --all-targets`、`cargo clippy --all-targets`、`cargo fmt --check`（4 个改动文件） | 构建退出码 0、无编译告警；clippy 退出码 0，告警全部落在既有代码（`storage/async_storage.rs`、`network/protocol.rs`、`executor/executor_trait.rs`、`transaction/manager.rs:420,469`、`cli/lifecycle.rs`、`pipeline.rs:2838`、`parser/planner/ddl_dml.rs` 与两个既有测试文件），与本次改动无关；`cargo fmt --check` 无 diff | 本次重跑 |
| 变更前基线 21 / 3 / 4 / 6 | 父 Cycle `000-initial.md` Act Response 的 Verification Evidence 表 | 与本次重跑结果一致 | 沿用（来源：父 Cycle；本次已复跑确认） |
| MS33 收尾的全量终态 2155 / 0 / 2 | `.agents/docs/SNAPSHOT.md`（父 Cycle 引用） | 本 Cycle 未跑全量，不作为本 Cycle 的完成声明 | 沿用（来源：父 Cycle Act Response） |

补充说明：格式化后重跑了「取消面整文件」一条命令（格式化只改测试文件的行宽，库代码字节未变 → 其余目标的结论继续有效，未重复运行）。

**Persisted Evidence**

None — 模式为 `none`。本 Cycle 的验证结论由上表的真实运行结果承载：全部命令为定向目标、可低成本重跑（秒级到分钟级），没有一次性环境即将消失的现场，也没有摘要会丢失决定性结构的长输出。本 Cycle 未创建 `evidence/` 目录。

**Experience Candidates**

- **Issue 候选（只报告，不登记）**：「被放弃的 ALTER TABLE DROP COLUMN 之后表的数据面形态」——`alter_drop_column` 先把目录窄化并 `flush_all`（窄化不在事务内），回收只回滚数据面，结果是「目录已窄化 + 数据面混合宽度」。该形态是本 change 范围外的实质问题（父 Cycle Plan Review 已裁定只报告不登记）；本 Cycle 的 T1-R2 只断言回收契约，未触碰该语义。证据：本 Cycle 的 Act Response 与 rework Cycle `001-rework.md` 的 Risks and Notes。

**Remaining Issues**

- 无未解决的 Critical / Important 问题；遗留 Minor 问题三条见 Self-Review。
- Issue 候选一条（被放弃的 ALTER TABLE 的数据面语义）——按规则只报告不登记，由 `openspec-experience-recorder` 按用户指令处置。
- 本 Cycle 未跑全量回归——按 change 预算，整个 change 一次，作为收尾验证在 Iteration 001 的任务 2.5 执行（M19 分批串行）。
- 停止条件：repair item `T1-R1`、`T1-R2` 全部完成，Gate 3 / Gate 4 逐项通过，`Act Response` 状态置 `reported`。本 Cycle 未创建下一 Cycle 或下一 Iteration，未归档 change，未同步全局文档（`.agents/docs/tasks.md` 的未提交修改不是本 Cycle 的改动，予以保留；change 的 `tasks.md`、SNAPSHOT、项目记忆、`openspec/specs/` 均未改动），未填写 `Plan Review`。

## Plan Review

- Review Result: accepted

**Findings**

- **F1（父 Review 的阻塞 GAP-1）已关闭——独立核实，不是采信自述。** 登记表语义确已从「库自己开的事务」改成「持有它的语句 future 已被放弃」：`ImplicitTxGuard`（`src/database.rs:162-190`）的 `Drop` 是登记项的**唯一**来源，`disarm()` 之后 `Drop` 不记入。全仓 `grep implicit_tx_guard` 只有三个创建点（`src/pipeline.rs:575`、`src/pipeline.rs:1009`、`src/executor/alter_table.rs:100`），与父 Review 独立核对的「三处库属隐式事务」清单一一对应；`register_implicit_tx` / `unregister_implicit_tx` 已彻底删除（`git diff` 中既无新增也无残留调用点），无孤儿死代码。
  - **独立复现（用户显式要求独立复现，故重跑；非重复已通过验证以增强信心）**：临时探针 `tests/zz_review_probe.rs`（**已删除**，`git status --short` 复核工作区已复原为审查前状态：`git diff --stat` 仍是 5 files / 278 insertions / 20 deletions，无 worktree 残留）。探针复刻父 Review 的**同一探针形态**：A = `INSERT INTO t2 SELECT id FROM t`（2000 行）在 `tokio::spawn` 的**克隆句柄**上执行，B = `INSERT INTO t3 VALUES (1)` 在同一句柄上执行。实际结果：`cargo test --test zz_review_probe -- --test-threads=1` → 退出码 0，`2 passed; 0 failed`；探针 A 的三条断言全部成立——B 成功返回 `AffectedRows(1)`；**B 完成后 A 的事务仍在活跃集内**（`active.contains(&a_tx)`）→ 证明 B 确实在 A 仍在执行时完成，**并发语义未被串行化**；A 照常提交 `AffectedRows(2000)`；A 结束后活跃集为空（无泄漏）。
  - Act 报告的 T1-R1 RED 证据与父 Review 的允许失败原因吻合（`active transactions are []`），不是编译错误、夹具错误或锚点超时；RED 命中的是「A 的事务在 B 之后不再活跃」这一条，与 Plan Context 的 First test 预期一致。
- **F2（本次修复的核心机制：Drop 守卫）逐点追踪结论——六点全部成立。**
  1. **正常完成路径不会被登记。** 三处站点在 commit/abort 成功返回后**紧接** `guard.disarm()`，两者之间**没有 await 点**（`src/pipeline.rs:645-655` 的 `target_db.commit(tx).await` → `guard.disarm()`；`src/pipeline.rs:1076-1083`；`src/executor/alter_table.rs:114-119`）。因此语句正常完成时守卫不可能在「已武装」状态下析构——**不存在「正常提交的事务被登记进回收表」的路径**。中止成功路径同理（`src/pipeline.rs:632-637`、`:1051-1057`、`:129-136`）。
  2. **放弃路径成立。** 语句 future 在 await 中被 drop → async 状态机的局部变量析构 → 守卫 `Drop` → armed → tx id 同步插入登记表 → 下一个触发点回收。`Drop` 只做一次 `HashSet::insert`（无 I/O、不 spawn 任务），回收的时序保证仍由三个库自有同步点承担（design D2 的取舍未被破坏）。
  3. **「提交已完成但结果未送达」窗口的守卫核实——成立。** `reclaim_implicit_tx`（`src/database.rs:588-596`）的「不在活跃集 → 跳过」是**单点 per-tx 漏斗**：三个触发点（DML 臂 `src/pipeline.rs:560`、CTAS 入口 `src/pipeline.rs:881`、ALTER TABLE 臂 `src/executor/alter_table.rs:86`、`Database::checkpoint()` `src/database.rs:853`、`Database::maybe_auto_checkpoint()` `src/database.rs:655`）全部经 `reclaim_abandoned_implicit_transactions` → `reclaim_implicit_tx`，**三处都受该守卫保护**。前提「已提交事务确实不在活跃集」也已独立核实：`TransactionManager::commit`（`src/transaction/manager.rs:125-129`）在返回 `Ok` 前 `active.remove(&tx_id)`，因此提交完成后该 tx id 已不在 `active_transactions()`。结论：提交已完成但结果未送达的事务会被**跳过**（不写 `AbortTxn`、不碰版本），Iteration 001 场景 3 的保护机制成立。
  4. **`Database::commit` 不再摘登记项未引入遗漏。** 新语义下登记项只由守卫析构产生，提交路径本就没有摘除职责；显式事务路径**从未**登记过（`Database::begin()` `src/database.rs:537`、`TransactionSession::begin` `src/transaction/session.rs:52` 均不创建守卫），故移除该摘除点不会遗留任何登记项。
  5. **显式事务绝不会被登记。** 已独立核对：守卫只在三处库属隐式事务点创建（见 F1 的 grep），显式事务的语句走 `execute_stage_in_tx_routed`（`src/pipeline.rs:1256+`），该路径**不** `begin()`、**不** `abort`、**不** `commit`、**不**触发回收、**不**建守卫——失败语句返回错误并把事务留给调用方收尾（既有语义，逐字未改）。
  6. **CTAS 路径与 ALTER TABLE DROP COLUMN 臂的守卫生命周期与 DML 臂同形。** 三处均为「`begin()` 后建守卫 → commit/abort 成功后 `disarm()` → 失败保持武装由下一个触发点重试」。补充核实：`src/executor/alter_table.rs:128-131` 的**就地 abort 传入空 table map**（既有形态，非本次改动），而 `abort_cleanup_versions`（`src/transaction/manager.rs:331-336`）对每个被触及的表要求可解析的 `TableMeta` → 该 arm 的就地 abort 在事务触及过表时**必然失败** → 守卫保持武装 → 清理实际由回收路径完成（回收按 `tx_version_tables` 解析出真实 table meta）。这正是 T1-R2 用例所覆盖的路径，也说明「中止失败保留登记项」的新语义在该站点是**承重**的而非装饰性的。
- **F3（无身份证明机制、无项目测试框架外的判定层）——通过。** 夹具的锚点是可观察状态（`TransactionManager::active_transactions()` 出现 tx id 且 `get_tx_versions(tx_id).len() >= 2`），轮询用 `tokio::task::yield_now()` 让出执行权；`tokio::time::timeout(10s)` 只作夹具失控兜底（触发即 `expect` 失败，不是阈值判定）。全文件无 Hash / 指纹 / run-id / manifest / 位点 pin / 时间顺序证明；无脚本化判定层；全部断言在 `cargo test` 内，观察点为 `active_transactions()`、`get_tx_versions()` 与库入口 `execute_sql` 的结果。`anchor_tx` 夹具与新增并发用例的锚点均满足「可观察状态 + `yield_now` 轮询」。
- **F4（T1-R2 的断言边界）——符合要求，父 Review 的 F4 覆盖缺口已闭合。** `abandoned_alter_table_drop_column_is_reclaimed`（`tests/statement_cancellation_test.rs:250-271`）只断言回收契约：回收成功返回（不报 `Reclaim failed`）、回收后 `active_transactions()` 为空、下一条写语句照常成功。**未**断言「目录已窄化 + 数据面混合宽度」这一范围外形态，也未用文件字节比较代替库入口结果。该站点的回收行为从此有回归用例守护，F4 关闭。
- **F5（新增用例的有效性与判别力）——通过。** 四个用例的失败原因互不相同：1.1＝回收未发生（活跃集非空 / 同键再插 `Duplicate key`）；1.3＝未提交行随 checkpoint 落盘（`rows written by the abandoned statement must not survive the checkpoint`）；T1-R1＝并发语句的事务被另一条语句的回收中止（`the concurrently running statement's transaction must stay active while the other statement runs, active transactions are []`）；T1-R2＝ALTER TABLE 站点的回收报错或该站点事务未被回收。判别力逐条成立：T1-R1 在**串行化实现**下同样会失败——串行化后 B 必须等 A 提交完成才执行，A 的事务此时已不在活跃集，`active.contains(&a_tx)` 断言必然失败（该断言同时就是「B 必须在 A 仍在执行时完成」的编码）；T1-R2 在该站点漏建守卫时必然失败（漏登记 → 回收找不到该事务 → 活跃集不会清空）。T1-R1 的 RED 由 Act 实跑见证，失败信息命中计划允许的两条断言之一。
- **F6（偏差裁定，无阻塞项）。** Act 的三条偏差均不构成阻塞：偏差 #1（T1-R2 在继承实现上首次运行即通过）是 **Plan 遗漏**——Plan Context 的 T1-R2 交付物本就是父 Review F4 指出的**零覆盖缺口**（回归用例），而非行为修复，故不存在「修复前 RED」形态；Act 已在修复前基线上留证并记录，符合 TDD 的「重构/表征用例先观察 GREEN」分支，不要求 Act 造 RED。偏差 #2（删除 `register_implicit_tx` / `unregister_implicit_tx`、`Database::commit` 不再摘登记项）是 Plan Implementation Guidance 明确指示的「随该替换一并调整；由本次改动产生的孤儿调用点与死代码一并清除」，非擅自扩范围。偏差 #3（守卫形态由 Act 选定）是 Plan 明示「实现形态……由 Act 决定」，不构成偏离。
- **F7（非阻塞 Minor finding，不返工）。**
  1. 三处触发点的回收失败错误面仍不一致（DML 臂与 CTAS 路径是 `Reclaim failed: {}`，ALTER TABLE 臂被渲染为 `Execution error: Reclaim failed: ...`）——父 Review F6 遗留，Plan 明示本次不必统一。
  2. `TransactionManager::abort_by_id` / `discard_tx_version_tables` 仍为 `pub`——父 Review F5 遗留，属表面问题，不作为阻塞理由。
  3. `ImplicitTxGuard::drop` 用 `Mutex::lock().unwrap()`：登记 mutex 只在三处**同步短段**（`drain` / `extend`）内持有，无 await、无重入路径，因此不存在死锁；但一旦 panic 造成 mutex 中毒，后续 `unwrap()` 会 panic。属可低成本收紧的表面问题（可改 `lock().unwrap_or_else(|e| e.into_inner())`），不阻塞 Acceptance。
  4. `checkpoint()` / `maybe_auto_checkpoint()` 只回收**主库**登记表；ATTACH 附加库的登记表由该库自己的下一条隐式写语句回收。既有形态，不在本 Iteration 范围。
- **F8（须带入 Iteration 001 的既有事实，非本 Cycle 缺口）。** `TransactionManager::commit` 的内部顺序是「写 WAL `CommitTxn` + fsync → `commit_mark_versions` → `active.remove` → `tx_versions.remove`」（`src/transaction/manager.rs:109-133`）。因此存在一个**极窄**子窗口：`commit_mark_versions` 完成后、`active.remove` 之前（此处有一个 await 点）future 被放弃 → 守卫登记该 tx id → 后续回收会发现它**仍在活跃集**而中止它。**该窗口在父实现下更宽**（父实现自 `begin()` 起即登记，覆盖整条语句生命周期），本次返工是**收窄**而非引入；且该窗口属 Iteration 001 场景 3（提交已完成但结果未送达）的范围，Plan Context 的 Invariant 与 Non-goals 已明确本 Iteration 不定义。已作为 Iteration 001 的必查事实写入其 Plan Context，不作为本 Cycle 的 Acceptance gap。

**Acceptance Gaps**

无。父 Review 的两个阻塞缺口均已关闭：

- **GAP-1（关闭）**：并发中的非取消语句不再被另一条语句的回收中止——独立探针复刻父 Review 的同一探针形态并 PASS，且断言证明并发语义未被串行化（B 在 A 仍在执行时完成）。见 F1。
- **GAP-2（关闭）**：第三处库属隐式事务站点 ALTER TABLE DROP COLUMN 有回归用例守护，断言只覆盖回收契约。见 F4、F5。

非 GAP 记录：父 Review 的 F1–F12 中，F3（提交路径摘登记项依赖）、F5（可见性）、F6（错误面）、F7（夹具成本）四条 Minor 维持不阻塞；F8（无身份证明机制）、F9（1.1/1.3 判别力）、F10（`abort` → `abort_by_id` 委托语义不变）、F11（变更前基线）、F12（并发目标未回归）五条通过项在本轮重新核实后仍然成立（`abort_by_id` 的函数体确为 `abort` 原函数体逐字迁入、仅 `tx.id()` 换成入参 `tx_id`；`abort` 现委托 `abort_by_id`，`abort_by_id` 为 `pub`；`src/transaction/manager.rs` 本轮未改）。

**Evidence**

- **独立阅读的完整 diff**：`git diff src/database.rs src/transaction/manager.rs src/pipeline.rs src/executor/alter_table.rs` 全部读过（净 +149 / +39 / +65 / +27 行），加未跟踪新文件 `tests/statement_cancellation_test.rs`（271 行）全文读过。Act 声明的改动位置与语义与代码一致：守卫三处创建点、`disarm()` 的六处调用点、`register`/`unregister` 的彻底删除、`Database::commit` 不再摘登记项、`reclaim_implicit_tx` 与 `reclaim_abandoned_implicit_transactions` 逐字未动（「不在活跃集 → 跳过」守卫完好）。
- **独立补跑（Act 未覆盖 / 用户显式要求独立复现）**：
  - `cargo test --test zz_review_probe -- --test-threads=1` → 退出码 0，`2 passed; 0 failed`（探针 A：F1 复现形态；探针 B：正常提交的事务不被后续回收回滚——提交 200 行 → 下一条写语句触发回收 → 下一条写语句成功且已提交行仍在（`SELECT id FROM t2` 仍 200 行）→ 活跃集为空）。**探针文件已删除**。
  - `cargo test --test statement_cancellation_test -- --test-threads=1` → 退出码 0，`4 passed; 0 failed`（`abandoned_alter_table_drop_column_is_reclaimed`、`abandoned_implicit_statement_is_not_committed`、`abandoned_statement_does_not_survive_reopen`、`concurrent_statement_does_not_abort_other_statement_tx`）。来源说明：本条与 Act 报告的同一条命令重叠，此处重跑是因为用户明确要求独立复现 F1 的消除、并要求核对新增用例的判别力；结论与 Act 一致。
- **沿用未失效的结论（来源：Act Response 的 Verification Evidence 表，本 Review 未重复运行）**：`cargo test --test explicit_tx_test --test mvcc_abort_test --test mvcc_commit_test --test dml_tx_id_test -- --test-threads=1` → 退出码 0，21 / 3 / 4 / 6，0 failed；`cargo test --test concurrent_test --test prepared_statement_test -- --test-threads=1` → 退出码 0，10 + 14 passed；受影响面 9 目标 → 101 passed / 0 failed；`cargo build --all-targets`、`cargo clippy --all-targets`（无新增告警）、`cargo fmt --check`（无 diff）。这些目标被本次实现改动覆盖，Act 已在改动后重跑确认，覆盖有效。
- **静态核实（未运行即可判定的事实）**：全仓 `transaction_manager.begin()` 调用点只有三处库属隐式事务（`src/pipeline.rs:569`、`src/pipeline.rs:1005`、`src/executor/alter_table.rs:95`）＋两处显式事务（`src/database.rs:537`、`src/transaction/session.rs:52`）＋两处既有测试内调用（`src/pipeline.rs:4020`、`src/transaction/session.rs:192`）——与父 Review 的枚举一致；`active_tx_ids` 的移出点只在 `commit`（`src/transaction/manager.rs:125`）、`abort_by_id`（`:176`）与 `commit_by_id`（`:282`）。
- Persisted Evidence 模式为 `none`，本 Cycle 未创建 `evidence/` 目录——按 Review 规则不因此提出问题。
- 本次 Review 未修改任何产品代码；未回滚既有改动；未提交；未调用其他 Skill。

**Follow-up Decision**

`accepted`（非 `rework-required`，非 `replan-required`，非当前 Cycle 修复）。

- 父 Review 的两个阻塞缺口（GAP-1 并发误伤、GAP-2 第三站点零覆盖）均已闭合，且闭合方式经独立复核（代码逐点追踪 + 独立探针 + 独立重跑），不是采信 Act 自述。
- 不走 `rework-required`：既有 Acceptance 全部满足，剩余发现（F7 四条）全是不阻塞 Acceptance 的 Minor finding——按 `references/iteration-planning.md`「只有不阻塞 Acceptance 的 Minor finding → `accepted`」，不为表面问题创建返工 Cycle。
- 不走 `replan-required`：目标、范围、依赖、requirement、验收边界与验证方式均未改变。
- 不留在当前 Cycle 修复：当前 Cycle 无待修项；F8 的窄窗口与 F7 的 Minor 均已在 Iteration 001 的 Plan Context 中承接为必查事实或非阻塞记录，不构成本 Cycle 的执行依据缺口。
- 不接受 Act 自评代替独立检查：本次 Review 独立阅读了完整 diff、独立复现了父 Review 的探针形态、逐点追了 Drop 守卫的六个分支，独立结论与 Act 声明一致。

**Iteration Plan Update**

None——Iteration 000 的 Iteration Plan 保持不变（Tasks 1.1/1.2/1.3、Stable result、Verification boundary、Diagnostic boundary、Non-goals 全部不改）。`rework` 修复以本地 repair item `T1-R1` / `T1-R2` 承载，未修改 Iteration Map。本 Cycle 获 `accepted` 后，Iteration 000 的稳定结果正式成立，Iteration 001 可以展开。

**Next Cycle**

None——当前 Cycle 已 `accepted`，不再创建后继 Cycle；`Plan Context` 保持不可改写，本 Cycle 冻结。

**Next Iteration**

`iterations/001-caller-visible-outcomes-and-final-regression/000-initial.md`——Iteration 001「调用方可见的三种结果与收尾回归」的初始 Cycle，按 change 的既有 Iteration Map 展开（Tasks 2.1–2.5），含任务 2.5 的 M19 分批串行收尾全量回归纪律。
