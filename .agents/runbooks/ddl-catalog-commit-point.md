# DDL 目录变更的提交点配方

- Status: active
- Last validated: 2026-10-25
- Environment: Linux x86_64（WSL2 6.18.40.1）、cargo 1.99.1 / rustc 1.99.1、RTsql `tests/ddl_catalog_transactional_test.rs` 与 `tests/dml_tombstone_abandon_window_test.rs` 集成测试、tokio 多线程 test runtime、单用例自带 `tempfile::tempdir()` 现场
- Source: change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` Iteration 001 / Cycle 000 Act Response 与 Plan Review（`openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/iterations/001-ddl-catalog-transactional/000-initial.md`，2026-10-24）；change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 001 / Cycle 000 Act Response 与 Plan Review（`.../iterations/001-ddl-rewrite-consistency/000-initial.md`，2026-10-25，**墓碑写入次序**与 `abort_cleanup_versions` 扩面）

## 适用范围

- 实现或修复「DDL 的**目录面**变更不在事务 / 回滚边界内」类缺陷时使用——被放弃或失败的 DDL 不应留下任何目录面变更。
- 验证「目录窄化延后到提交点」后的三类可观察结果：放弃路径目录面不变、完成路径目录面与数据面同生共死、放弃后重开库目录与数据面一致。
- 需要「让一条 DDL 在改写中途可停住」的夹具手法时使用（锚点式放弃）。
- 不适用：纯数据面 DML 的回滚语义；索引写侧原子性（用 `.agents/runbooks/index-write-side-atomicity-repro.md`）；进程内崩溃语义（用 `.agents/runbooks/in-process-crash-simulation.md`）；ADD COLUMN 的目录变更（本配方只覆盖已验证的 DROP COLUMN 形态，但提交点写法与 ADD COLUMN 同序，可推广）。

## 前置条件

- 目录面的读与写分处两侧：事务体**只读**目录，提交点才**写**目录。事务体内出现任何目录写入、`flush_all` 或活表映射切换 ⇒ 配方被破坏。
- 载体类型：`PendingColumnDrop`（`src/storage/data/table_manager.rs`）承载「被删列定义 + 原序号 + 新列数 + 窄化后的活布局」，由事务体交回调用方。
- 放弃守卫：库入口的隐式事务守卫在语句作用域内武装（`implicit_tx_guard`）；提交点必须先 `disarm()`，否则语句 future 析构会把**已提交**的 tx id 记入登记表，下一条语句的回收会去中止一个已提交的事务。

## 操作步骤

1. **守卫（事务体内）**：执行期守卫拒绝不安全的删列——被删列是 PK / UNIQUE / 被二级索引引用时在**事务体内**拒绝（此时隐式事务已开启，语句失败 ⇒ 事务被中止）；「最后一列」在**计划期**拒绝。
2. **新布局计算（事务体内）**：只**读**目录取出被删列的完整列定义与其原序号（`Catalog::scan_columns`），算出窄化后的列数与活布局。事务体内不写目录。
3. **事务内数据面改写**：逐行 DELETE（旧行墓碑，`write_tuple_to_data_page` 写墓碑槽）+ INSERT（窄化后的新行），全部在语句的隐式事务内；每行 `record_version` 登记版本。
4. **交回待应用的目录变更**：把窄化结果装进 `PendingColumnDrop` 返回给调用方。此时数据面已改写、目录仍是旧布局——**提交点之前，目录面没有任何变更**。
5. **提交**：`commit(tx)` 成功后立刻 `guard.disarm()`（顺序不可换：先解除守卫再应用目录）。
6. **提交点应用目录**：`apply_column_drop(pending)`，内部顺序固定为
   - **先列行**：`rewrite_column_rows_drop`；
   - **后列数**：`update_table_column_count_ddl`（它先把除计数页外的脏页落盘、**最后**落盘计数页 ⇒ 崩溃中间态不会出现「持久化的计数没有列行支撑」）；
   - **再切 live meta**：活表映射切换到窄化布局（只在目录已宣告窄化之后切）。
7. **失败分支**：`alter_drop_column` 返回错误 → `abort(tx)` → **中止成功才** `disarm()`；中止失败保留登记项，由下一个回收触发点重试。

### 锚点式放弃手法（让 DDL 在改写中途可停住）

- **锚点定义**：活跃事务集合中出现某个 tx id，且该事务的**已记录版本数 ≥ 2** ⇒ 语句已进入数据面改写（逐行 `record_version` 的直接可观察后果）。
- **观测方式**：`db.transaction_manager.active_transactions()` + `get_tx_versions(tx_id).len() >= 2`，用 `tokio::task::yield_now()` 轮询——**按可观察状态判定，不按时间判定**。
- **可达规模**：**30 行规模即可命中锚点**。行数是性质要求（「足以让改写中途的锚点可达」），不是规模指令——30 行已把「目录已窄化」的缺陷形态完整暴露。
- **放弃动作**：锚点命中即丢弃语句 future（取消）；语句在锚点命中前跑完 ⇒ **夹具不成立**，夹具显式 panic（「statement finished before the abandon anchor was reached」），不得把「跑完」当成被放弃。
- **失控兜底**：`ANCHOR_TIMEOUT` 10 秒 timer，触发即 `expect` 失败——是兜底不是行为判据。

### 墓碑写入次序（2026-10-25 补充）

- 适用范围：DELETE 腿在数据面改写中写下的墓碑。墓碑的**终态**与 DeleteExecutor 形态逐字节同形；本条只约束**写入次序**，不改终态。
- 固定次序：**先写（不带链指针）→ 登记版本 → 补链指针 → 再清理索引条目**。
- 为什么是这个次序：墓碑一旦在链上（带链指针、且删除者事务仍活跃），扫描时的 `superseder_suppresses` 会走「已提交删除」分支而**压制其前驱版本** ⇒ 该行立即不可见。若墓碑**未被中和**（其 `create_tx` 仍是被放弃事务的 id）而事务又已被移出活跃集，回收就无法中和它 ⇒ **被放弃的 DDL 隐藏了它开始改写的那一行**。
  - 因此「不带链指针」先行：墓碑在**登记之前**对扫描不可见（不是链上的有效版本）⇒ 登记与链指针补写之间的窗口才是唯一的暴露窗口；
  - 登记（`record_version`）在链指针补写之前 ⇒ 回收**看得到**这个版本，才能中和它；
  - 索引条目清理放在最后 ⇒ 清理失败不会留下「已登记但未在链上」的中间态。
- 与中止清理的配合：`abort_cleanup_versions` 在登记版本之外，还要按**创建者事务 id** 取回该事务创建但**从未登记**的版本（`versions_created_by`，沿数据页链收回，链成环显式报错）⇒ 覆盖「放弃落在写出版本槽与登记版本之间」的更窄窗口（该窗口下墓碑未被登记、回收完全看不到该表）。
- 复现与验证配方：`.agents/runbooks/dml-tombstone-abandon-window.md`（锚点式放弃 + 逐 id 点查）。
- 边界：同表并发 DDL 与在途状态布尔量的交互（第二个 DDL 清除先行 DDL 的读拒绝、豁免读按旧 schema 解码已窄化行）**不属**本面，已单独登记为 `.agents/issues/ISS29-same-table-concurrent-ddl-inflight-flag.md`。

## 验证

- 成功判据：`cargo test --test ddl_catalog_transactional_test -- --test-threads=1` 退出码 0，输出 `test result: ok. 3 passed; 0 failed`（2026-10-24 实测，连续 3 次 GREEN）。
- **修复前 RED 形态**（2026-10-24 实测，5/5 连续稳定命中目录面断言）：放弃后目录列定义 `["id","w"]`、列数 2、行宽 2；而**语句执行前**为 `["id","v","w"]` / 3 / 3。
- **断言强度的判别点**：期望值必须对齐到**语句执行前**的列数与列定义，不能对齐到「（可能被窄化过的）目录自身宽度」——窄化后的目录 + 窄化后的行宽同样自洽，但那正是缺陷形态。
- 完成路径对照基线：目录列定义 `["id","w"]`、列数 2、行宽 {2}、**行数守恒 30**、`check_integrity` 的 error 级发现 0；close → reopen 后四项再次成立（真实入口 `Database::open` + `SELECT *`）。
- 判据全部落在可观察状态上（目录列定义 / 列数、每行行宽、行数、完整性报告、`active_transactions`）；判定层在 `cargo test` 内。

## 失败处理

- **锚点不可达**：行数不足 → 加大行数（性质要求，不是任意数值）；夹具显式 panic 时不要放宽判据。
- **重开后目录列数与行宽不一致**：按落点区分——「列行已写、计数未降」这一落点，重开库的活布局取自目录列行并按宣告列数截断 ⇒ 已是窄化布局且与窄化后的数据一致（**可读且自洽**）；「提交完成、列行未改写」这一落点会产生**位置性错读**（窄 tuple 在未移位目录下按位置取值）——该窗口已单独登记为 `.agents/issues/ISS25-ddl-commit-point-positional-misread.md`，不属本配方面。
- **`apply_column_drop` 的 I/O 失败**：语句报错但数据已提交、目录未窄化 ⇒ 与崩溃窗口同类；错误必须向上传播，不得静默吞掉。
- **提交后守卫未解除**：下一条语句的回收会去中止一个已提交的事务 ⇒ `disarm()` 必须**早于** `apply_column_drop`；放在其后会在 `apply` 失败时留下同一缺口。
- **完成路径行数不守恒**：不是本配方的目录面问题，是数据面多页改写丢行——已单独登记为 ISS22（台账已归档，指针见 references.md R52），不属本配方面。

## 回滚

不适用（实施配方，不产生需要回滚的运行时状态）。配方被采纳后的代码回退随承载 change 的 diff 承担；配方本身不修改仓库文件。

## 证据

- `openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/iterations/001-ddl-catalog-transactional/000-initial.md` —— Act Response 的 T4 / T5 / T6 实现说明与 Experience Candidates 第 1 条（提交点配方与锚点式放弃手法）；Plan Review 的 F5（提交点设计的独立核对：事务体无目录写入、`apply_column_drop` 的先列行后列数再切活表映射、`commit` → `disarm` → `apply` 顺序正确且必要）、F7（三条 requirement 的判别力核对）。
- 见证代码：`tests/ddl_catalog_transactional_test.rs`（3 用例）、`tests/statement_cancellation_test.rs::abandoned_alter_table_drop_column_is_reclaimed`（既有 MS34 用例的断言补齐，只增不减）、`tests/dml_tombstone_abandon_window_test.rs`（3 用例，墓碑写入次序的 RED+GREEN 见证）。
- 实现位点：`src/storage/data/table_manager.rs`（`alter_drop_column` / `PendingColumnDrop` / `apply_column_drop`）、`src/executor/alter_table.rs`（`commit` → `disarm` → `apply_column_drop`，失败分支 `abort` → 中止成功才 `disarm`）、`src/storage/catalog.rs`（`rewrite_column_rows_drop` / `update_table_column_count_ddl`）、`src/executor/delete.rs::DeleteExecutor::delete_row` 与 `src/executor/upsert.rs::UpsertExecutor::delete_conflict_row`（墓碑写序）、`src/transaction/manager.rs`（`abort_cleanup_versions` + `versions_created_by`）。
- 行为规格：`openspec/specs/ddl-catalog-transactional/spec.md`（3 Requirement / 4 Scenario）。
- 相关配方：`.agents/runbooks/index-write-side-atomicity-repro.md`（索引写侧确定性复现）、`.agents/runbooks/in-process-crash-simulation.md`（进程内崩溃模拟）、`.agents/runbooks/m19-batched-full-regression.md`（分批全量）。
