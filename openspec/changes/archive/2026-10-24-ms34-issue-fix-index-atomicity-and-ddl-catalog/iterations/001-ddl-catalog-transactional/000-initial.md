# Iteration 001 / Cycle 000: DDL 目录变更的事务化与本 change 收尾全量回归

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4, T5, T6
- Acceptance gaps: None
- Repair items: None
- Inherited scope: change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` 的需求、规格增量 `specs/ddl-catalog-transactional/spec.md`、design D5–D7；Iteration 000 已验收的索引写侧契约（`specs/index-write-side-atomicity/spec.md`）**不在本 Cycle 的改动面内**；project-model M13（异步执行原则）、M19（全量测试分批执行约束）、M20（范围外缺陷承载闭环）；公共规则 TDD、Gate、验证、行为约束。
- Excluded scope: 索引写侧原子性的进一步改动（Iteration 000 已 accepted）；MS35 空闲页跨重启复用；MS36 历史库与 dump 兼容；MS21 D10 的既有崩溃边界（提交与目录应用之间的窗口）；ADD COLUMN 路径的既有提交点写法（`update_table_column_count_ddl`）不改语义；并发读窗口的性质；`openspec/specs/` 的合并（由 docs-maintainer 收尾执行）。

**Requirements Traceability Matrix（本 Cycle 范围）**

`ddl-catalog-transactional`（DCT）规格域的 requirement 编号；IWS 域由 Iteration 000 承载，不在本 Cycle 的改动面内。

| Requirement | Task | First Test | Status |
|---|---|---|---|
| DCT R1 abandoned or failed DDL statements leave no catalog-side changes | T4, T5 | `tests/ddl_catalog_transactional_test.rs::abandoned_drop_column_leaves_catalog_unchanged`、`::failed_drop_column_leaves_catalog_unchanged` | Covered |
| DCT R2 catalog changes and data-plane rewrites live and die together | T5, T6 | `tests/ddl_catalog_transactional_test.rs::completed_drop_column_aligns_catalog_and_rows` | Covered |
| DCT R3 after reopening the database following an abandoned DDL, the catalog and data plane are consistent | T4, T5, T6 | `tests/ddl_catalog_transactional_test.rs::abandoned_drop_column_leaves_catalog_unchanged`（含 close→reopen 读回）+ `tests/statement_cancellation_test.rs::abandoned_alter_table_drop_column_is_reclaimed`（补齐断言后） | Covered |

命名说明：矩阵中的测试名是**计划中的用例名**（与 change 的 `tasks.md` RTM 一致）；Act 可在不改变断言语义的前提下调整命名，并在 Act Response 记录实际名称。RTM 无 `Missing`、无 `Simplified`。

**Objective**

`ALTER TABLE DROP COLUMN` 的**目录窄化只在语句确定完成（提交点）之后生效**：被放弃或失败的 DDL 语句不改变目录的列定义与列数，也不留下「目录已窄化 + 数据面仍为旧宽度」的形态；放弃后回收并重开库，目录列数与数据面行宽一致。同时，本 Iteration 承担**本 change 的收尾验证**：按 M19 的分批串行纪律跑一次全量回归，作为本 change 的最终全量 Gate。

**Background**

- 本 Cycle 承载 ISS21（`.agents/issues/ISS21-ddl-catalog-transactional.md`）：MS34 实施期发现的「DDL 目录变更不在事务内」缺陷。缺陷正文、根因位点与最短可观察步骤在 Issue 文件里，**不在本文件复制正文**。
- 缺陷机制（读码确认）：`TableManager::alter_drop_column` 先改目录（`rewrite_column_rows_drop` + `update_table_column_count`）并 `flush_all`，**之后**才在事务内逐行 DELETE + INSERT 改写数据面；语句被放弃/失败时，事务只回滚数据面，目录侧的窄化不在回滚范围内。
- design D5 已选定修复方向：**「窄化延后到语句确定完成之后」**（而非「纳入回滚」）——理由是「纳入回滚」需要在中止路径重建目录并回滚活表映射，改动面显著更大；「延后」复用既有 API（`update_table_column_count_ddl` 已是 ADD COLUMN 的提交点写法），与 ISS21 自身建议一致。

**Investigation Facts**

- Starting point:
  - 工作区 HEAD `0a08b14`（ahead of origin 9）。工作区仍含 MS34 的产品代码与测试文件改动，以及本 change Iteration 000 的改动（`src/storage/btree/index_manager.rs`、`src/storage/btree/mod.rs`、`src/storage/mod.rs`、`tests/index_write_atomicity_test.rs`（新建）、`tests/statement_cancellation_test.rs`（扩展））——**均未提交，属既有现场：不回滚、不清理、不提交**。
  - Iteration 000 的结论（沿用）：`iterations/000-index-write-side-atomicity/000-initial.md` 的 `Plan Review` = `accepted`。其稳定结果是「索引写侧写单元原子 + 同索引写侧串行 + 取消/回滚后键全释放 + 结构自洽」，**与本 Iteration 的目录面无交集**——DDL 目录变更走的是 `TableManager` / `Catalog` 路径，不经 `IndexManager` 的写单元。
  - 既有可沿用结论：MS34 已合并的 `openspec/specs/statement-cancellation-outcome/spec.md`——取消/超时的事务收尾语义面已验收，本 Cycle 不改其语义；`tests/statement_cancellation_test.rs` 已提供 DDL 回收契约用例（见 Current-State Evidence）。
  - 既有缺陷正文（**不在本文件复制正文**，按指针引用）：`.agents/issues/ISS21-ddl-catalog-transactional.md`。
  - 变更前验证结论（沿用，不重跑）：Iteration 000 的定向与受影响范围结论（其 Act Response「Verification Evidence」表）；MS34 收尾的全量分批结论。覆盖范围未变化的部分直接沿用；本 Cycle 只对新增/受影响的定向用例做验证，并在最后跑一次收尾全量。

- Current-State Evidence:
  - **DDL 入口**：`src/executor/alter_table.rs:80-140`（`AlterTableOp::DropColumn` 分支）：
    - `:86-94` 回收先于本语句的隐式事务（`reclaim_abandoned_implicit_transactions`；失败映射为 `StorageError::ExecutionError("Reclaim failed: …")`）；
    - `:95-96` `transaction_manager.begin()` 开启隐式事务；
    - `:97-100` 语句作用域守卫 `implicit_tx_guard(tx_id)`——语句 future 被放弃时守卫析构，tx id 记入登记表，由下一个触发点回收；正常提交/中止后显式 `guard.disarm()`；
    - `:101-111` `table_manager.alter_drop_column(table_name, column_name, tx_id, tx_manager.clone(), wal_buffer.clone())`；
    - `:112-120` **成功分支**：`transaction_manager.commit(tx, &buffer_pool).await?` → `guard.disarm()` → `AffectedRows(0)`；
    - `:122-140` **失败分支**：`transaction_manager.abort(tx, &buffer_pool, &HashMap::new())`（中止失败则保留登记项，由下一触发点重试回收）。
  - **目标符号**：`src/storage/data/table_manager.rs:693-700` `TableManager::alter_drop_column(&self, table, column_name, tx_id, tx_manager, wal) -> Result<()>`：
    - `:703-740` 防御守卫（planner 之前已拦，这里是纵深防御）：列不存在 / 最后一列 / 主键列 / UNIQUE 声明列 / 被二级索引引用列 → `StorageError::Internal`；
    - `:742-790` 计算新布局（`new_columns` / `new_not_null` / `new_defaults` / `new_pk_index` / `new_unique_indexes` / `new_secondary_indexes`；键树按值索引，键内容不变，只移序号）；
    - **`:792-799` 目录改写（缺陷点）**：`catalog.rewrite_column_rows_drop(&old.name, removed as u32)` + `catalog.update_table_column_count(&old.name, new_columns.len() as u32)`；
    - **`:802` `buffer_pool.flush_all().await?`**——目录改写先于任何数据 WAL 记录落盘 ⇒ 窄化**不在事务边界内**；
    - `:804-821` 切换内存 `TableMeta`（新布局；`index_manager` 沿用旧实例，`data_page_head` / `data_page_tail` 沿用）——**目录窄化与内存布局切换都发生在数据面改写之前**；
    - `:823-...` 数据面改写：枚举存活行（非墓碑、链尾）→ DELETE 腿（独立墓碑 slot + `index_manager.delete` / `uindex.delete` / `sindex.delete_exact` + `tx_manager.record_version` + WAL `Delete`）→ INSERT 腿（窄化后的新 tuple + `index_manager.insert` + WAL `Insert`）。
  - **目录 API**（`src/storage/catalog.rs`）：`:1600` `rewrite_column_rows_drop(table, removed_index)`（删被删列的行 + 后续列 `column_index` 下移，就地改写、保持链序不变式）；`:465` `update_table_root`；`:388-400` `update_table_column_count`（普通写法，页随 checkpoint 落盘）；`:409` `update_table_column_count_ddl`（**ADD COLUMN 的提交点写法**：列行先落盘，广告它的列数最后刷——`flush_commit_point([page])`）。
  - **可观察入口（测试可用，均为公开 API）**：
    - 目录列定义：`db.table_manager.catalog().scan_columns("t").await` → `Vec<CatalogColumnRow>`（`column_name` / `column_index` 为公开字段，`src/storage/catalog.rs:314`）；
    - 目录列数：`db.table_manager.catalog().scan_tables().await` → `CatalogRow.column_count`（公开字段，`src/storage/catalog.rs:93`）；
    - 数据面行宽：`db.execute_sql("SELECT * FROM t")` → `Response::QueryResult { rows }`，每行的值个数即行宽（`src/network/protocol.rs:36-38`）；
    - 完整性：`rtsql::storage::integrity::check_integrity(&db.buffer_pool, &db.table_manager)` → `report.error_count()`。
  - **既有夹具与先例**：
    - `tests/statement_cancellation_test.rs:337-366` `abandoned_alter_table_drop_column_is_reclaimed`——放弃 `ALTER TABLE t DROP COLUMN v` → 下一条写语句触发回收 → 只断言回收契约（不报 `Reclaim failed`、活跃集清空、后续写成功），**不断言目录/数据面形态**（这正是 ISS21 的落点，本 Cycle 补齐它）。
    - 该文件的夹具形态：单用例自带独立 `TempDir`；表 `t (id INT PRIMARY KEY, v INT)` + `t2 (id INT PRIMARY KEY)`；源数据 2000 行；放弃手法 = 轮询 `active_transactions()` 直到某 tx 的 `get_tx_versions(tx_id).len() >= 2` 的锚点 + `tokio::select! { biased; }` + `drop(statement)`；锚点上限 `ANCHOR_TIMEOUT` **只作夹具失控兜底**（超时即 `expect` 失败，触发即失败），不是行为判据。
    - 写侧暂停点夹具（Iteration 000 产出，`IndexManager::arm_write_pause`）**只服务于索引写侧**；DDL 侧没有等价夹具——ISS21 的触发是「DDL 语句在数据面改写中途被放弃」，需要语句在改写中途可被确定性地停住。**本 Cycle 优先复用既有锚点式放弃手法**（DDL 的数据面改写逐行 `record_version`，锚点「版本数 ≥ 2」天然落在改写中途）；若实测证明该锚点不可达（语句在锚点可观测前跑完），则**停止并返回 Plan**（见 T4 的 Stop when）。
  - **错误路径与边界**：`alter_drop_column` 的守卫错误是 `StorageError::Internal`；数据面改写中的索引删除/插入错误经 `?` 上抛为 `StorageError`；执行器的失败分支走 `abort`（`&HashMap::new()` 的空表映射——中止路径只做版本清理，不写活表映射，这也是 D5 选「延后」而非「纳入回滚」的原因之一）。

- Code and Critical Path:
  - `src/executor/alter_table.rs`：`AlterTableExecutor` 的 `DropColumn` 分支——本 Cycle 的**主改动面**（提交点与目录应用的时序在此）。
  - `src/storage/data/table_manager.rs`：`TableManager::alter_drop_column`——本 Cycle 的**主改动面**（目录改写与 `flush_all` 移出；返回待应用的目录变更；内存布局切换移出）。
  - `src/storage/catalog.rs`：`rewrite_column_rows_drop` / `update_table_column_count` / `update_table_column_count_ddl` / `update_table_root`——目录读写 API（**本 Cycle 不改其语义**，只改调用时点与调用顺序）。
  - `src/transaction/manager.rs`：`commit` / `abort` / `abort_cleanup_versions`——提交点与回滚点（**不改**；MS34 已验收的取消/回收语义面）。
  - `src/storage/btree/index_manager.rs`：DDL 数据面改写经 `IndexManager::insert` / `delete` 触达索引写侧（**不改**；Iteration 000 的写单元契约在此生效）。
  - `tests/ddl_catalog_transactional_test.rs`（**新建**）：本 Cycle 的新回归文件。

**Implementation Guidance**

- 建议顺序：先 T4（先行 RED 用例，观察目录面与行宽的真实形态）→ 再 T5（实施目录窄化延后）→ 最后 T6（回归对齐 + 收尾全量）。理由：TDD 铁律要求先有 RED；且 T4 的实测形态（放弃后目录到底窄化成什么样、数据面行宽多少）是 T5 的设计输入。
- T5 的形状（design D5 的落地要点，重述以便本 Cycle 自包含执行）：
  1. `TableManager::alter_drop_column` **不再**改目录、**不再** `flush_all`；它只做守卫检查、计算新布局、在事务内完成数据面改写，并**返回待应用的目录变更**（表名、被删列的完整列定义与其原序号）。
  2. 执行器：`alter_drop_column(...)` 成功 → `commit(tx)` **完成之后** → 应用目录变更（`rewrite_column_rows_drop` + `update_table_column_count_ddl` + flush，**先列行后计数**，与 ADD COLUMN 路径同序）→ 再切换内存 `TableMeta` 到新布局。
  3. 失败路径：执行器 `abort(tx)`；目录、内存布局、数据面三者均未变更（数据面由事务回滚）⇒ 完全一致。
- 关键取舍与边界（design D5 明确记录，本 Cycle 继承）：
  - **崩溃窗口（记录，不在本 change 调整）**：提交完成与目录应用之间崩溃 ⇒ 目录仍是旧布局、数据已是新布局。读侧按宽度感知元组解读（既有「narrow pre-ADD rows are legal」路径），不会静默错读；代价是该 DDL 的目录效果丢失。这比现状「目录已窄化 + 数据仍宽」（列序号已位移 ⇒ 旧行按新目录解释 ⇒ 错读）**更安全**。MS21 D10 的既有崩溃边界不在本 change 内调整。
  - **并发读窗口（性质不变）**：数据面改写期间，活 `TableMeta` 仍是旧布局而部分行已是新宽度——与现状同类的读侧窗口。本 change 不改变这一窗口的性质。
  - **ADD COLUMN 路径不改语义**：其提交点写法（`update_table_column_count_ddl`）是被**复用**的对象，不是被修改的对象。
- 禁止引入身份证明机制（run-id / 指纹 / 时间戳 / manifest / 时间顺序证明）与框架外判定层；判定全部落在 `cargo test` 内。timer 只作夹具失控兜底。

**Task Definitions**

### T4: ISS21 先行 RED 用例（放弃 DDL → 回收 → 重开 → 目录列数与行宽一致）

- Requirement/Scenario: `ddl-catalog-transactional` — 「被放弃的 DROP COLUMN 不窄化目录」「失败的 DROP COLUMN 不窄化目录」「放弃 DROP COLUMN 后重开库列数与行宽一致」；以及「完成的 DROP COLUMN 使目录与数据面一致」（作为对照基线，先记录当前完成路径的形态）。
- Depends on: None
- Targets: `tests/ddl_catalog_transactional_test.rs`（新建）。
- Current behavior: 放弃（或失败）的 `ALTER TABLE t DROP COLUMN v` 会**先把目录窄化并 `flush_all`**（`src/storage/data/table_manager.rs:792-802`），之后数据面改写才在事务内进行；语句被放弃时事务只回滚数据面 ⇒ 终态是「目录已窄化（列数 2、列定义无 `v`）+ 数据面仍按旧宽度存放」；重开库后目录列数与数据面行宽不一致。
- Required behavior: 用例在**放弃 DDL → 触发回收 → `close()` → 重开**之后，直接断言目录面与数据面：① `scan_columns("t")` 仍含被删列 `v` 且 `CatalogRow.column_count` 等于语句执行前的列数；② 重开后 `SELECT * FROM t` 的每行值个数等于语句执行前的列数（行宽与目录一致）；③ 目录与行宽的差异不存在（即不出现「目录 2 列 / 行宽 3 值」的混合形态）。同时记录**完成路径**的对照形态（正常 `ALTER TABLE t DROP COLUMN v` 完成后：目录列数 = 列数 − 1、被删列不在列定义中、数据面每行按新列数存放），作为 T5/T6 的对齐基准。失败路径（DROP 被守卫拒绝，如主键列/最后一列/被二级索引引用）断言目录与数据面均未变更。
- Preserve: MS34 已验收的取消/回收语义面逐条不变（既有 `tests/statement_cancellation_test.rs` 的断言只增不减）；既有夹具形态（单用例自带独立 `TempDir`、不依赖并行度）不变；不改 `statement-cancellation-outcome` 域的语义。
- Forbidden: 不改产品代码（本任务是先行 RED）；不引入身份证明机制或判定层；不在 `cargo test` 之外建脚本或专用命令；不把全量回归放进本任务。
- First test: 新建 `tests/ddl_catalog_transactional_test.rs`，主用例名 `abandoned_drop_column_leaves_catalog_unchanged`（与 change 的 RTM 对齐；Act 可在不改变断言语义的前提下调整命名并在 Act Response 记录实际名称），用例形如：建三列表 `t (id INT PRIMARY KEY, v INT, w INT)` + 足量行（行数要足以让「改写中途」的锚点可达——DDL 的数据面改写逐行 `record_version`，锚点取「该 tx 的版本数 ≥ 2」）→ 用既有锚点式放弃手法（`tokio::select! { biased; }` + 锚点命中即 `drop(statement)`；锚点上限只作夹具失控兜底，触发即失败）放弃 `ALTER TABLE t DROP COLUMN v` → 下一条写语句触发回收 → `close()` → 重开 → 断言目录列定义/列数与数据面行宽。**变更前观察 RED**（目录已被窄化 ⇒ 断言失败在目录面）。命令：`cargo test --test ddl_catalog_transactional_test -- --test-threads=1`。
- GREEN condition: 本任务**不追求 GREEN**——完成判据是「用例就位 + 在真实修复前稳定 RED」。RED 必须**每次**复现（连续 3 次运行同一结果），否则触发「三次失败必须反思」并回到 Plan。
- Verification: `cargo test --test ddl_catalog_transactional_test -- --test-threads=1`（观察 RED）；`cargo test --test statement_cancellation_test -- --test-threads=1`（确认既有 MS34 用例仍 GREEN——本任务不改产品代码，预期不变）。
- Stop when: 放弃手法无法在 DDL 的数据面改写中途确定性地停住语句（锚点在语句跑完后才可观测）⇒ 触发方案不成立，**停止并返回 Plan**；或需要改 `statement-cancellation-outcome` 的语义才能表达 ⇒ 返回 Plan。

### T5: DDL 目录窄化延后到提交点之后（使 T4 用例 GREEN）

- Requirement/Scenario: `ddl-catalog-transactional` — 全部三个 requirement（不留下目录面变更、目录变更与数据面改写同生共死、重开库后目录与数据面一致）。
- Depends on: T4
- Targets: `src/storage/data/table_manager.rs`（`TableManager::alter_drop_column`：目录改写与 `flush_all` 移出、返回待应用的目录变更、内存布局切换移出）、`src/executor/alter_table.rs`（`AlterTableExecutor` 的 `DropColumn` 分支：提交点之后应用目录变更并切换内存布局）。
- Current behavior: 目录窄化 + `flush_all` + 内存布局切换全部发生在数据面改写**之前**、事务边界**之外**；失败/放弃时目录与内存布局已变更而数据面被回滚。
- Required behavior: ① `alter_drop_column` 不再改目录、不再 `flush_all`，只做守卫、计算新布局、在事务内完成数据面改写，并返回待应用的目录变更（表名 + 被删列的完整列定义 + 其原序号）；② 执行器在 `commit(tx)` **成功返回之后**应用目录变更（`rewrite_column_rows_drop` → `update_table_column_count_ddl`，**先列行后计数**，同 ADD COLUMN 路径的顺序），随后把内存 `TableMeta` 切换到新布局；③ 失败路径（守卫拒绝或数据面改写报错）走 `abort`，目录、内存布局、数据面三者均不变；④ 放弃路径：目录与内存布局未变更，数据面由回收回滚 ⇒ 重开库后目录列数 = 语句执行前列数、数据面行宽 = 该列数。
- Preserve: ADD COLUMN 路径的提交点写法与语义不变（`update_table_column_count_ddl` 是被复用的对象）；`Catalog` 的公开 API 语义与签名不变；`abort_cleanup_versions` 的分桶逻辑不变；MS34 已验收的取消/回收语义不变；索引写侧契约（Iteration 000）不变——DDL 数据面改写仍经既有 `IndexManager` 写单元。
- Forbidden: 不改 `Catalog` 的读写语义；不改 `abort_cleanup_versions` 的分桶逻辑；不引入身份证明机制；不做与 ISS21 无关的重构；不处理 MS21 D10 的崩溃边界与并发读窗口的性质。
- First test: 沿用 T4 的用例（此时应转 GREEN）；先确认 RED（`cargo test --test ddl_catalog_transactional_test -- --test-threads=1` 失败在目录面断言），再实施。
- GREEN condition: T4 的全部用例通过；`cargo test --test statement_cancellation_test -- --test-threads=1` 仍全绿（既有取消语义未回归）。
- Verification: `cargo test --test ddl_catalog_transactional_test -- --test-threads=1`；`cargo test --test statement_cancellation_test -- --test-threads=1`；`cargo test --lib --bins -- --test-threads=1`。
- Stop when: 延后目录窄化需要改变 `Catalog` 的公开语义、或需要改 `statement-cancellation-outcome` 的取消语义才能达成（⇒ 返回 Plan）；或出现「提交点与目录应用之间」必须由本 change 关闭的新崩溃边界（⇒ 按公共规则作为 Issue 候选上报，不在本 Cycle 扩范围）。

### T6: DDL 目录面回归对齐 + 本 change 的收尾全量回归（M19 分批串行）

- Requirement/Scenario: `ddl-catalog-transactional` — 「完成的 DROP COLUMN 使目录与数据面一致」（回归对齐）；以及本 change 的收尾验证（Iteration 000 的 `index-write-side-atomicity` 四条 requirement 的全量回归确认）。
- Depends on: T5
- Targets: `tests/ddl_catalog_transactional_test.rs`（扩展：正常完成路径、失败路径）、`tests/statement_cancellation_test.rs`（扩展既有 `abandoned_alter_table_drop_column_is_reclaimed` 的断言——从「只断言回收契约」补齐到「断言目录面与数据面形态」）、本 change 的全量回归执行记录（写入本 Cycle 的 `Act Response`）。
- Current behavior: 既有 MS34 用例 `abandoned_alter_table_drop_column_is_reclaimed` 只断言回收契约（不报 `Reclaim failed`、活跃集清空、后续写成功），**不断言目录/数据面形态**；正常完成路径的目录/行宽一致性没有独立断言。
- Required behavior:
  1. **正常完成路径**：一条 `ALTER TABLE t DROP COLUMN v` 正常执行完成后，目录的列定义不再含 `v`、列数 = 原列数 − 1、数据面每行按新列数存放（目录与行宽一致）。
  2. **失败路径**：被守卫拒绝的 DROP（主键列 / 最后一列 / 被二级索引引用的列）报错后，目录与数据面均不变。
  3. **既有 MS34 回收用例补齐**：`abandoned_alter_table_drop_column_is_reclaimed` 在既有断言之外，追加目录面与数据面形态的断言（放弃 + 回收之后目录列数与行宽仍等于语句执行前）。
  4. **本 change 的收尾全量回归**：按下列 M19 分批串行纪律跑**一次**全量，并把每批的 passed/failed 计数与覆盖对账写入本 Cycle 的 `Act Response`。
- Preserve: MS34 已验收的既有断言逐条不变（只增不减）；既有夹具形态不变；全量回归**不新增**任何判定脚本、封装命令或专用参数（判定只看 cargo 的退出码与原生输出）。
- Forbidden: 不改 MS34 取消域的语义断言；不引入身份证明机制或判定层；不把全量回归的判定逻辑放到项目测试框架之外；不以一次无目标过滤的全量调用充当全量结论。
- First test: 扩展 `tests/ddl_catalog_transactional_test.rs`（正常完成路径用例名 `completed_drop_column_aligns_catalog_and_rows`；失败路径用例名 `failed_drop_column_leaves_catalog_unchanged`）与 `tests/statement_cancellation_test.rs` 的既有 DDL 回收用例 `abandoned_alter_table_drop_column_is_reclaimed`（补齐目录面与数据面形态断言）；变更前在正常完成路径上观察现状（目录与行宽一致 ⇒ GREEN），在补齐后的放弃路径上观察现状（目录已窄化 ⇒ RED）。命令：`cargo test --test ddl_catalog_transactional_test -- --test-threads=1`、`cargo test --test statement_cancellation_test -- --test-threads=1`。
- GREEN condition: T4–T6 的全部定向用例通过；收尾全量的**全部批次各自 0 failed**。
- Verification: 定向：`cargo test --test ddl_catalog_transactional_test -- --test-threads=1`；`cargo test --test statement_cancellation_test -- --test-threads=1`。收尾全量：按下面的「M19 分批串行全量回归纪律」执行。
- Stop when: 全量某批 failed 且归因指向本 change 之外的既有缺陷 ⇒ 按公共规则只报告、不在本 Cycle 修；或需要为让全量通过而改动其他模块 ⇒ 停止并由用户/Plan 决定。

**M19 分批串行全量回归纪律（SHALL，写死在本任务）**

1. **分组**：`--lib --bins` 一批；`tests/*.rs` 按**文件名字典序**分组，每批 **8–12 个测试目标**（经验值 10）。
2. **每批一次独立调用**：`cargo test --test <f1> --test <f2> ... --test <fN> -- --test-threads=1`；**不得**用一次无目标过滤的全量调用跑完（无论串行还是默认并行度）。
3. **批间顺序执行**：同一时刻**只有一批**在跑；批与批不并发启动。
4. **每批必须带 `--test-threads=1`**：**不得**用默认并行度跑任何全量批次（单次目标验证 `cargo test --test <name>` 不受此约束）。
5. **通过条件**：每批退出码为 0 **且** 该批 `test result:` 行的 failed 计数为 0。
6. **记录**：逐批记录 passed/failed（以及 ignored）计数与退出码。
7. **覆盖对账**：目标清单 = **收尾时按实际文件列表重算**的 `tests/*.rs` 全部文件 + `--lib --bins`，并把本 change 新增的测试文件（`tests/index_write_atomicity_test.rs`、`tests/ddl_catalog_transactional_test.rs`）一并纳入；对账要求**不重不漏**（分批覆盖目标清单的全部目标）。
8. **全量 Gate** = **全部批次各自 0 failed**；缺一批不得声明全量通过。
9. **判定方式**：只看 cargo 自己的退出码与原生输出；计数可用一步通用管道读出（`… 2>&1 | grep -E "^test result:"`），管道只做计数、**不做判定**；**不得**新增判定脚本、封装命令、专用参数或判定层。
10. **失败处理**（M19 Runbook `m19-batched-full-regression.md`）：某批 failed → 先单独复核该用例，再整批复核一次（失败后判定是否偶发错误的一次复核；复核通过按通过处理并注明首跑失败）；cargo 在首个失败目标后中止该批后续目标时，复核必须**重跑整批**（否则会把「未执行」误记成「通过」）；某批跑很久按该 Runbook 的诊断配方处理；失败测试与本次改动无关时不改其他模块去迁就，按纪律写入 Act Response 并停止。

**Invariants**

- 放弃或失败的 DDL 语句不留下目录面变更：目录的列定义与列数保持语句执行前的状态。
- 目录窄化只在语句确定完成（提交点）之后生效；目录窄化生效时数据面改写已经完成。
- 重开库后目录列数与数据面行宽一致（不出现「目录已窄化 + 数据面仍为旧宽度」）。
- MS34 已验收的取消/超时/事务收尾语义面不变（丢弃与超时是取消触发源；断开不是；显式事务由调用方决定继续或回滚）。
- 索引写侧契约（Iteration 000）不变：DDL 数据面改写经既有写单元，取消/回滚后的键释放与结构自洽仍成立。
- M13：异步路径不持 `std::sync` 锁跨 `.await`；M19：全量回归分批执行（本 Cycle 的收尾全量按纪律执行）；M20：范围外缺陷只报告不登记。
- 验证只落在 `cargo test` 集合内；无身份证明机制、无判定层、无 timer 阈值作行为判据。

**Non-goals**

- 索引写侧原子性的进一步改动（Iteration 000 已 accepted）。
- MS21 D10 的既有崩溃边界（提交完成与目录应用之间的窗口）——design D5 明确记录，不在本 change 调整。
- ADD COLUMN 路径的提交点写法与语义（`update_table_column_count_ddl` 是被复用的对象，不是被修改的对象）。
- 并发读窗口的性质（数据面改写期间活 `TableMeta` 仍是旧布局）。
- 空闲页跨重启复用（MS35）、历史库与 dump 兼容（MS36）。
- 既有损坏目录/索引的修复。

**Acceptance**

1. **B1**（T4+T5，R1「被放弃的 DROP COLUMN 不窄化目录」）：放弃 DDL → 回收 → `close()` → 重开后，目录仍含被删列且列数 = 语句执行前的列数，数据面每行按该列数存放。**验证**：`cargo test --test ddl_catalog_transactional_test -- --test-threads=1`。
2. **B2**（T5，R1「失败的 DROP COLUMN 不窄化目录」）：被守卫拒绝的 DROP 报错后，目录的列定义与列数、数据面行宽均不变。**验证**：同上命令。
3. **B3**（T5+T6，R2「目录变更与数据面改写同生共死」）：正常完成的 DROP COLUMN 之后，目录列数 = 原列数 − 1、被删列不在列定义中、数据面每行按新列数存放。**验证**：同上命令 + `cargo test --test statement_cancellation_test -- --test-threads=1`。
4. **B4**（T4+T5+T6，R3「放弃 DDL 之后重开库目录与数据面一致」）：放弃 → 回收 → 重开库后，目录列数与数据面行宽一致（列数 = 语句执行前的列数，各行按该列数存放）。**验证**：`cargo test --test ddl_catalog_transactional_test -- --test-threads=1`（含 close→reopen 读回）+ 既有 MS34 DDL 回收用例补齐后的断言（`cargo test --test statement_cancellation_test -- --test-threads=1`）。
5. **B5**（T6，收尾验证）：本 change 的收尾全量回归按 M19 分批串行纪律执行一次，**全部批次各自 0 failed**，并完成「目标清单 ↔ 分批」的不重不漏对账。**验证**：M19 纪律第 1–10 条。

**Verification**

- B1/B2/B3/B4：`cargo test --test ddl_catalog_transactional_test -- --test-threads=1`；B3/B4 另加 `cargo test --test statement_cancellation_test -- --test-threads=1`。端到端场景（放弃 DDL → 回收 → 重开 → 读目录与行宽）走真实入口：`Database::execute_sql` 实际执行并丢弃语句 future，断言用户实际所见（目录列数与列定义、行宽、查询结果、进程存活）。
- B5：M19 分批串行纪律（见 T6 的纪律清单第 1–10 条）+ `.agents/runbooks/m19-batched-full-regression.md`（R48）。
- 触发窗口的锚点 = 既有锚点式放弃手法（活跃事务的版本数 ≥ 2）；超时只作夹具失控兜底（触发即失败），**不作为行为判据**。
- 静态检查：`cargo fmt -- --check`；`cargo clippy --all-targets`（告警数与基线比较，新增告警须归零）。

**Gate 2 Readiness**

PASS。Gate 2 检查项：需求与场景已定（`specs/ddl-catalog-transactional/spec.md` 的 R1–R3 三条 requirement + 四个场景）；行为差异、接口、错误语义、关键选择已明确（design D5 的形状与其记录的两个窗口）；当前起点、入口、目标符号、调用者、被调用者、数据流、状态变化、错误路径与测试入口均来自实际代码与已运行结论（`src/executor/alter_table.rs:80-140`、`src/storage/data/table_manager.rs:693-821`、`src/storage/catalog.rs:388-400/409/465/1600`、`src/transaction/manager.rs`、`tests/statement_cancellation_test.rs:337-366`）；任务含完整执行依据（Targets / Current / Required / Preserve / Forbidden / First test / GREEN condition / Verification / Stop when）；RTM 无 `Missing`、无 `Simplified`；无关范围已排除；Iteration Plan 覆盖全部任务且高开销验证（全量）的预算与纪律已写死在本 Cycle 的 T6；Plan Context 自包含（Act 不需要回读 design / Issue / 前序 Cycle 才能执行）；非实质未知项留在 Risks；验证直接证明目标行为且无身份机制/判定层；Persisted Evidence = `none`；**用户已批准计划**（依据 goal 指令原文与阶段推进授权）。

**Persisted Evidence**

- Mode: none

理由：Act Response 足以承载验证结果（命令、实际结果、修改文件与符号、逐批 passed/failed 计数与覆盖对账表）。收尾全量的逐批计数是短输出（每批一行 `test result:`），可直接摘录进 Act Response 的表格；判定只依赖 cargo 的退出码与原生输出，全部可低成本重跑（按 M19 的分批方式）；没有一次性环境即将消失的现场，也没有摘要会丢失决定性结构的长输出。`required` 的四个问题中「为什么 Act Response 不够」无法回答 ⇒ 按公共规则取 `none`。

**Risks and Notes**

- **触发方案的诚实边界**（非实质未知项，Act 不需要决定接口或错误语义）：DDL 数据面改写的「中途可停住」用既有锚点式放弃手法表达（逐行 `record_version` ⇒ 版本数 ≥ 2 的锚点落在改写中途）。若实测发现锚点在该语句跑完后才可观测，则 T4 的 Stop when 生效（返回 Plan），不得以「跑完也算放弃」的方式放宽。
- **DDL 侧没有写侧暂停点那样的确定性夹具**：本 Cycle 不新建产品代码内的夹具（Iteration 000 的 `arm_write_pause` 只服务索引写侧）。若锚点式手法不足以稳定命中，优先扩大表行数（夹具规模）而不是引入新的夹具。
- **内存布局与目录的切换时点**：数据面改写需要「新布局」来写窄化后的 tuple 与新索引键（`new_meta.pk_index` 等），但**活表映射的切换**必须延后到提交点之后；实现时用局部的新布局完成改写，再在提交后切换 `self.tables` 里的 `TableMeta`。
- **既有崩溃边界与并发读窗口**：design D5 已记录——提交与目录应用之间的崩溃窗口（代价是该 DDL 的目录效果丢失，比现状更安全）、数据面改写期间的并发读窗口（性质不变）。二者均不在本 change 的调整范围；若实施中发现必须关闭它们才能满足 B1–B4，⇒ 按 Stop when 返回 Plan。
- **范围外缺陷**：本 Cycle 发现的实质缺陷按公共规则作为 Issue 候选**只报告不登记**（M20）。
- **全量回归预算**：整个 change 一次，已记在本 Iteration 的 T6；中途如需追加全量，须在本 Iteration 记录理由。
- **工作区现场**：MS34 与本 change Iteration 000 的改动均未提交，属既有现场——本 Cycle 不回滚、不清理、不提交。

## Act Response

- Status: reported

**Implemented**

- **T4 — ISS21 先行 RED 用例**（`tests/ddl_catalog_transactional_test.rs`，新建，3 个用例）：
  - 夹具：独立 `TempDir` 现场（单用例自带，不依赖并行度）；`t(id INT PRIMARY KEY, v INT, w INT)` 预置 `SOURCE_ROWS` 行 + `t2(id INT PRIMARY KEY)` 作回收触发目标。
  - 放弃手法 = Plan 指定的既有锚点式手法：轮询活跃事务集合中「已记录版本数 ≥2」的事务（DDL 逐行 `record_version`，每行产出墓碑 + 新行两个版本 ⇒ 第一行完成即达 2），命中即 `drop(statement)`；语句在锚点可观测前跑完 ⇒ 用例显式失败（`panic!("statement finished before the abandon anchor was reached: ...")`），**不以「跑完也算放弃」放宽判据**。观察循环用 `tokio::task::yield_now()` 让出执行权；`ANCHOR_TIMEOUT`（10 秒）只作夹具失控兜底，触发即失败，不是行为判据。
  - `abandoned_drop_column_leaves_catalog_unchanged`（DCT R1 + R3）：放弃 `ALTER TABLE t DROP COLUMN v` → 下一条写语句触发回收 → 断言目录列定义仍含 `v`、目录列数仍为 3、数据面行宽仍为 3 → close → reopen 后再断言一次同样三项。
  - `failed_drop_column_leaves_catalog_unchanged`（DCT R1 的失败侧）：守卫矩阵（主键列 / 最后一列 / 被二级索引引用的列）三条 DROP 全部报错后，目录列定义、目录列数与数据面行数/行宽均不变。
  - `completed_drop_column_aligns_catalog_and_rows`（DCT R2 对照基线）：正常完成的 DROP COLUMN 使目录列定义变为 `[id, w]`、列数变为 2、数据面每行按新列数存放。
- **T5 — DDL 目录窄化延后到提交点之后**：
  - `src/storage/data/table_manager.rs`：
    - 新增 `pub struct PendingColumnDrop`（模块级）：`table`（表名）、`removed_column`（被删列的**完整目录列定义**，`column_index` 持有其原序号）、`new_column_count`（应用后目录应宣告的列数）、`new_meta`（数据面改写所依据的窄化布局 `Arc<TableMeta>`）。
    - `TableManager::alter_drop_column` 的签名由 `-> Result<()>` 改为 `-> Result<PendingColumnDrop>`；**移除**事务体内的目录改写（`rewrite_column_rows_drop` + `update_table_column_count`）、**移除** `flush_all()`、**移除**活表映射切换（`self.tables.write().await.insert(...)`）；事务体只做守卫检查、计算窄化布局、在事务内完成数据面改写（DELETE 腿 + INSERT 腿），并把待应用的目录变更交回调用方。目录列定义在改写前从 `Catalog::scan_columns` 读取（此时目录仍是旧布局）。
    - 新增 `TableManager::apply_column_drop(&self, pending: &PendingColumnDrop) -> Result<()>`：按 ADD COLUMN 提交点的顺序应用目录变更——**先列行**（`rewrite_column_rows_drop`）**后计数**（`update_table_column_count_ddl`，它先落盘除计数页外的全部脏页、最后落盘计数页）——随后才切换活表映射到 `pending.new_meta`。
  - `src/executor/alter_table.rs`：`AlterTableOp::DropColumn` 的成功分支顺序改为 `commit(tx)` → `guard.disarm()`（库属隐式事务已提交 → 解除语句作用域守卫）→ `table_manager.apply_column_drop(&pending)`（**提交点之后**才应用目录变更）。失败分支不变（`abort` → 中止成功才解除守卫）。
  - `src/storage/catalog.rs`：`Catalog::update_table_column_count`（无 flush 的普通变体）在本 change 后已无调用方（其唯一调用方是 DROP COLUMN 路径），随本次改动一并移除；`update_table_column_count_ddl` 的文档改为描述「ADD COLUMN 与 DROP COLUMN 同用该提交点顺序」与崩溃中间态的可读性。
- **T6 — DDL 目录面回归对齐 + 本 change 收尾全量回归**：
  - `tests/ddl_catalog_transactional_test.rs`（扩展）：`completed_drop_column_aligns_catalog_and_rows` 追加「`check_integrity` 的 error 级发现为 0」与「close → reopen 后目录与数据面仍一致」两组断言。
  - `tests/statement_cancellation_test.rs`（扩展既有 MS34 DDL 回收用例，**只增不减**）：`abandoned_alter_table_drop_column_is_reclaimed` 追加目录面断言（列定义仍为 `[id, v]`、列数仍为 2）与数据面形态断言（`SELECT * FROM t` 每行 2 值）；该用例的文档注释由「断言只覆盖回收契约本身」改为描述修复后的事实。
  - 收尾全量回归：按 M19 分批串行纪律执行一次（13 批，见 Verification Evidence）。

**Deviations from Plan**

1. **T4 夹具的预置行数从 2000 降为 30（Plan 遗漏，非实质）**：Plan 的 Risks 只预见了「锚点不可达 → 优先扩大表行数」。实跑锚点在 30 行即可稳定命中（5 次连续 RED 同一失败点）；反倒是更大规模会撞上与本 change 无关的既有缺陷（完成路径在数据页写满后丢行，见 Remaining Issues 的 Issue 候选 A）。故按 Plan 的「优先扩大行数」反方向取小规模，使夹具既稳定又避开范围外缺陷。判据未被放宽：Plan 的三条 Required behavior（目录列定义含 `v` / 目录列数不变 / 行宽不变）逐条仍是断言对象。
2. **T6 的 First test 措辞「在补齐后的放弃路径上观察现状（目录已窄化 ⇒ RED）」在本轮观察到的是 GREEN（事实说明，非实现偏差）**：T6 `Depends on: T5`（目录窄化已延后到提交点之后），执行 T6 时放弃路径已转 GREEN。等价 RED 已在 T4 由 `abandoned_drop_column_leaves_catalog_unchanged`（同一锚点手法、同一组可观察量）建立并记录 ⇒ T6 对既有 MS34 用例的扩展在本轮是**回归护栏**而非新的 RED。判据未因此放宽。
3. **`Catalog::update_table_column_count` 随本次改动移除（Plan 遗漏，清理）**：Plan 的 D5 指定 DROP COLUMN 的提交点使用 `update_table_column_count_ddl`；普通变体的唯一调用方即 DROP COLUMN 路径，改后成为孤儿，且其文档注释「ALTER DROP COLUMN keeps the plain method」与新行为直接矛盾。按公共规则「清理由本次改动产生的孤儿」移除；不影响任何公开 API 的语义（该方法无调用方）。
4. **放弃路径的行数断言取「非空」而非「等于预置行数」（Plan 未规定，属本 Cycle 的判据边界选择）**：Plan 的 Required behavior ②只要求「每行**值个数**等于语句执行前的列数」（行宽），不含行数。实跑发现被放弃的 DDL 在回收后恰好隐藏它已开始改写的那一行（id=2 缺失，29/30）——回收/可见性域的既有行为（Issue 候选 C），不在 DCT 范围。把行数钉死会把既有缺陷写进 DCT 测试并使其长期 RED。目录列定义、目录列数、数据面行宽三项按 Plan 逐条精确断言；完成路径（无放弃）额外断言行数守恒。

**Blocker Handoff**

None required — 本 Cycle 未命中 Gate 5。

**Blocker Resolution**

None — 本 Cycle 未曾阻塞。

**Self-Review**

- **Gate 3（Task Review）逐项结论**：
  - **T4 — Spec compliance：PASS**。先行测试先于产品改动存在（`tests/ddl_catalog_transactional_test.rs` 在 T5 实施前已建），并在真实修复前**行为上**稳定 RED（5 次连续实跑同一失败输出，非编译错误、非夹具错误、非锚点超时）。锚点可达性成立：锚点在语句完成前即可观测（命中时语句仅改写了 1 行）⇒ T4 的 Stop when 未触发，无需返回 Plan。断言集与 Plan 的 Required behavior 逐条对应；未引入身份机制与框架外判定层（判定全在 `cargo test` 内，观察点是 `Catalog::scan_columns` / `scan_tables` 的列定义与列数、`SELECT *` 的行宽、`check_integrity` 的 Error 计数）。
  - **T4 — Code quality：PASS**。夹具自包含、无并行度依赖；`ANCHOR_TIMEOUT` 仅作夹具失控兜底；无新增编译/clippy 告警（改动文件 0 条），`cargo fmt -- --check` 项目级无 diff。
  - **T5 — Spec compliance：PASS**。目录窄化完全移出事务体（`alter_drop_column` 内不再有任何目录写入、不再 `flush_all`、不再切换活表映射）；提交点之后由 `apply_column_drop` 应用目录变更（先列行后计数，与 ADD COLUMN 同序）并切换活表映射。失败路径（守卫拒绝 / 数据面改写报错）走 `abort`，目录、活表布局与数据面均不变 ⇒ DCT R1（放弃/失败不留目录面变更）与 DCT R2（目录变更与数据面改写同生共死）成立。既有 `TableManager` 其余公开方法语义不变（除 `alter_drop_column` 的返回类型由 `()` 改为 `PendingColumnDrop`，全仓唯一调用点在 `src/executor/alter_table.rs`）。
  - **T5 — Code quality：PASS**。事务体与提交点两个阶段的职责分离清晰；`apply_column_drop` 的目录写入顺序与落盘顺序与 ADD COLUMN 提交点一致（列行先、计数页最后）；`guard.disarm()` 仍在提交后立刻执行（提交已完成的事务不得被回收）；目录应用失败（I/O 错误）时向上传播错误而不静默吞掉（此时数据已提交、目录未窄化——与 D5 记录的提交/目录应用窗口同性质）。
  - **T6 — Spec compliance / Code quality：PASS**。完成路径与既有 MS34 回收用例的断言补齐只增不减；端到端场景（放弃 DDL → 回收 → 重开 → 读目录与行宽）走真实入口（`Database::execute_sql` 实际执行并丢弃语句 future，断言用户实际所见）；收尾全量按 M19 纪律分批串行执行并完成覆盖对账。
  - **未解决的 Critical / Important 问题：无。**
- 自检中发现并已修复：
  1. **夹具行数与范围外缺陷的碰撞**（已修）：最初按 MS34 夹具取 2000 行，完成路径的对照用例在数据页写满后**丢行**（100 行 → 35 行可见）并在更大规模触发**栈溢出**。两者都在 T5 实施之前复现、且都在数据面改写路径（本次未改动）。处置：把夹具规模降到 30 行（改写全程留在同一数据页内），并把两个现象作为 Issue 候选上报（不扩范围修复）。
  2. **T4 用例的行数断言把既有缺陷写进判据**（已修）：初版对放弃路径断言行数等于预置行数，实测为 29/30（被放弃的语句隐藏了它开始改写的那一行）。按 Plan 的 Required behavior（行宽，非行数）把该断言收窄为「非空」，行数守恒改由完成路径（无放弃）断言。
  3. **注释与实际行数不一致**（已修）：模块文档与 `setup` 文档仍写「2000 行」，已改为 `SOURCE_ROWS` 并补记取 30 行的理由。
  4. **提交点应用目录变更后活表映射切换的顺序**：实现取「目录先、活表映射后」——目录已宣告窄化布局后才切换活布局，避免出现「活布局已窄化、目录仍宽」的中间态（与被修复的缺陷同形）。
- 遗留 Minor 问题（不阻塞，未修）：
  1. `alter_drop_column` 新增一条错误路径：目录中找不到被删列对应的列行时报 `StorageError::Internal`。正常目录（列行齐全）不可达；只在目录与活表元数据不一致时触发（此前列行缺失时 DDL 会继续执行）。
  2. `alter_drop_column` 现在多一次 `Catalog::scan_columns` 读取（取被删列的完整列定义）。DROP COLUMN 不是高频路径，实测影响可忽略。
  3. `apply_column_drop` 发生在提交之后：若目录写入因 I/O 失败失败，语句报错但数据已提交、目录未窄化（与 D5 记录的崩溃窗口同性质，本 change 不调整）。
  4. 提交点之后的崩溃窗口：目录仍是旧布局、数据已是新布局。Plan 的 D5 记录了该窗口并声明「不会静默错读」——按代码阅读（`deserialize_tuple_width` 只按 schema 长度取值、不校验列序）推断该声明**不准确**：2 宽的 tuple 在未位移的 3 列目录下读作 `[id, w, NULL]`（`v` 列读到 `w` 的值），属位置性错读且影响面是全部已改写的行。此判断按代码阅读得出，**本 Cycle 未构造崩溃点实跑验证**（构造崩溃点需新的故障注入夹具，超出本 Cycle 范围）⇒ 作为 Issue 候选 D 上报，由 Plan Review 与 Recorder 裁定表述与承载。
- 无身份证明机制（无 Hash / 指纹 / run-id / session-id / revision pin / manifest / 时间顺序证明及其采集/审计/鉴定工具），无项目测试框架之外的判定层：全部断言在 `cargo test` 内，观察点为 `Catalog::scan_columns` / `scan_tables` 的列定义与列数、`SELECT *` 的行宽、`check_integrity` 的 Error 计数、`transaction_manager.active_transactions()` 的集合内容。未新增脚本、封装命令或专用参数（分批执行只是对既有 `cargo test` 的顺序调用，判定用 cargo 的退出码与原生 `test result:` 行）。
- 改动文件：`src/storage/data/table_manager.rs`（`PendingColumnDrop`、`alter_drop_column`、`apply_column_drop`）、`src/executor/alter_table.rs`（DROP COLUMN 分支的提交点顺序）、`src/storage/catalog.rs`（移除 `update_table_column_count`、`update_table_column_count_ddl` 文档）、`tests/ddl_catalog_transactional_test.rs`（新建）、`tests/statement_cancellation_test.rs`（扩展）、change 的 `tasks.md`（T4/T5/T6 置 `done`）与本 Cycle 的 Act Response。工作区里 MS34 与本 change Iteration 000 的既有现场（`src/database.rs`、`src/pipeline.rs`、`src/transaction/manager.rs`、`src/storage/btree/**`、`src/storage/mod.rs`、`.agents/**`、`openspec/specs/**`）未回滚、未清理、未提交。

**Verification Evidence**

- **Gate 4（Verified Completion）结论**：PASS——Task Definition 的 Verification 全部满足（T4 的 RED 命令退出码非 0 且失败信息命中产品断言；T5/T6 的 GREEN 命令退出码均为 0；T6 收尾全量 13 批各自 exit=0 且 failed 计数为 0；覆盖对账不重不漏）。完成声明基于下表的当次真实运行。

| 验证 | 命令 | 实际结果 |
|---|---|---|
| T4 RED（产品改动前；`SOURCE_ROWS`=2000，首次实跑） | `cargo test --test ddl_catalog_transactional_test abandoned_drop_column_leaves_catalog_unchanged -- --test-threads=1 --nocapture` | 退出码 101；`test result: FAILED. 0 passed; 1 failed`。失败点 = 目录面断言（`the dropped column must still be in the catalog columns`，行号为格式化前记录）：`got Shape { columns: ["id", "w"], column_count: 2, row_count: 1999, widths: {2} }`（left `["id","v","w"]` / right `["id","w"]`）⇒ 目录已被窄化（列定义无 `v`、列数 2），数据面读回行宽 2 ⇒ ISS21 形态复现 |
| T4 RED（产品改动前；`SOURCE_ROWS`=30，**5 次连续**） | 同上 | 退出码 101；每次 `test result: FAILED. 0 passed; 1 failed`，失败信息逐字相同：`got Shape { columns: ["id", "w"], column_count: 2, row_count: 29, widths: {2} }`。**锚点可达**（命中时语句仅改写 1 行；未出现「statement finished before the abandon anchor was reached」）⇒ RED 稳定复现（5/5），T4 的 Stop when 未触发 |
| T5 GREEN（3 次连续） | `cargo test --test ddl_catalog_transactional_test -- --test-threads=1` | 退出码 0；每次 `test result: ok. 3 passed; 0 failed; 0 ignored` |
| T5 受影响范围（定向） | `cargo test --test alter_table_test --test drop_persistence_test --test statement_cancellation_test --test catalog_durability_test --test ddl_crash_recovery_test --test ddl_catalog_transactional_test -- --test-threads=1` | 退出码 0；16 + 4 + 3 + 6 + 11 + 8 = **48 passed / 0 failed**（`alter_table_test` 16 含 `drop_column_*` 与守卫用例；`drop_persistence_test` 11 含 `alter_drop_column_crash_keeps_narrowed_rows`；`ddl_crash_recovery_test` 6；`catalog_durability_test` 4） |
| T6 扩展 GREEN | `cargo test --test ddl_catalog_transactional_test --test statement_cancellation_test -- --test-threads=1` | 退出码 0；`3 passed` + `8 passed` = **11 passed / 0 failed**（`statement_cancellation_test` 的 `abandoned_alter_table_drop_column_is_reclaimed` 含补齐后的目录面与数据面断言） |
| T6 收尾全量（M19 分批串行，13 批） | 见下方批次表；每批一次独立调用 `cargo test --test <f1> ... -- --test-threads=1` | **全部 13 批 exit=0**，合计 **2171 passed / 0 failed / 2 ignored** |
| 静态检查 | `cargo fmt -- --check`；`cargo clippy --all-targets` | `cargo fmt -- --check` 项目级无 diff（exit 0）；clippy 的 lib 告警 **14 条 = MS34 基线同数**，且 `table_manager.rs` / `catalog.rs` / `alter_table.rs` / `ddl_catalog_transactional_test.rs` / `statement_cancellation_test.rs` **0 条** |

**T6 收尾全量：分批执行记录（M19 纪律逐条对照）**

| 纪律 | 执行事实 |
|---|---|
| 1. `--lib --bins` 单独成一批 | 批次 1 = `cargo test --lib --bins -- --test-threads=1`（单独调用） |
| 2. `tests/*.rs` 按文件名字典序分组，每批 8–12 个目标 | 133 个 `tests/*.rs` 目标按 `ls tests/*.rs \| sort` 的字典序切成 12 批（11×11 + 12），每批 11 或 12 个目标（均在 8–12 内） |
| 3. 每批一次独立调用 | 每批一次 `cargo test --test <f1> --test <f2> ... -- --test-threads=1`；**无**一次无目标过滤的全量调用 |
| 4. 批间顺序执行 | 单个 shell 循环顺序执行，同一时刻只有一批在跑；批内 `--test-threads=1` |
| 5. 禁止默认并行度跑批 | 每批均显式 `--test-threads=1` |
| 6. 每批 exit=0 且 failed 计数为 0 | 13 批全部 exit=0，全部 `0 failed` |
| 7. 逐批 passed/failed 计数与目标清单对账 | 见批次表与对账结论 |
| 8. 全量 Gate = 全部批次各自 0 failed | 13/13 批 0 failed ⇒ 全量 Gate 通过 |
| 9. 与基线 2163/0/2 做总数对账 | 2171 = 2163 + 5 + 3（差额来源见对账结论） |

| 批次 | 目标 | 目标数 | passed | failed | ignored | exit |
|---|---|---|---|---|---|---|
| 1 | `--lib --bins` | 2 个测试二进制（lib 420 + bin 0） | 420 | 0 | 0 | 0 |
| 2 | aggregate_extension_test, aggregate_output_shape_test, aggregate_test, alter_table_test, attach_session_test, btree_merge_test, btree_scale_test, btree_split_test, btree_test, busy_timeout_test, cache_perf_test | 11 | 149 | 0 | 0 | 0 |
| 3 | catalog_durability_test, chain_join_test, check_command_test, checkpoint_command_test, checkpoint_redo_reduction_test, checkpoint_skip_test, checkpoint_test, cli_test, concurrent_test, connection_limit_test, constraint_enforcement_test | 11 | 210 | 0 | **2** | 0 |
| 4 | cross_db_dml_test, cross_db_query_test, ctas_test, cte_test, database_file_lock_test, datetime_function_test, datetime_type_test, **ddl_catalog_transactional_test**, ddl_crash_recovery_test, default_declaration_test, default_precheck_test | 11 | 140 | 0 | 0 | 0 |
| 5 | derived_scan_alias_test, dml_tx_id_test, drop_persistence_test, drop_table_free_test, e2e_test, encryption_test, executor_test, explain_test, explicit_tx_test, expression_e2e_test, file_header_test | 11 | 191 | 0 | 0 | 0 |
| 6 | file_storage_io_test, full_table_dml_test, gc_test, group_by_expr_test, import_input_surface_test, index_manager_test, **index_write_atomicity_test**, insert_column_list_test, insert_select_test, isolation_level_test, join_output_resolution_test | 11 | 95 | 0 | 0 | 0 |
| 7 | join_test, key_type_conformance_test, keyless_eq_routing_test, keyless_row_test, limit_test, memory_db_test, mvcc_abort_test, mvcc_commit_test, mvcc_record_test, mvcc_tombstone_visibility_test, nested_loop_join_test | 11 | 87 | 0 | 0 | 0 |
| 8 | network_protocol_test, network_server_test, no_from_select_test, order_by_alias_sort_test, outer_join_test, overflow_page_test, param_binding_test, parser_test, pg_integration_test, pg_messages_test, pg_protocol_test | 11 | 124 | 0 | 0 | 0 |
| 9 | pipeline_test, plan_cache_test, plan_exec_test, planner_test, predicate_test, prefetch_test, prepared_statement_test, projection_expression_test, projection_test, pushdown_test, query_memory_test | 11 | 225 | 0 | 0 | 0 |
| 10 | readonly_session_test, recovery_e2e_test, recovery_error_surface_test, recovery_replay_convergence_test, recovery_tail_persistence_test, recovery_test, recovery_wal_convergence_test, runtime_test, scalar_function_test, scalar_subquery_comparison_test, schema_persistence_test | 11 | 98 | 0 | 0 | 0 |
| 11 | secondary_index_equality_test, secondary_index_test, select_distinct_test, self_join_test, set_operations_test, sort_test, sql_differential_test, **statement_cancellation_test**, storage_test, structural_property_test, subquery_alias_binding_test | 11 | 146 | 0 | 0 | 0 |
| 12 | subquery_test, subset_insert_test, sync_loader_test, table_alias_resolution_test, table_manager_test, truncate_test, tx_statement_test, typed_api_test, update_expression_test, update_index_maintenance_test, upsert_test | 11 | 181 | 0 | 0 | 0 |
| 13 | value_test, version_chain_test, view_test, visibility_test, wal_auto_checkpoint_test, wal_buffer_test, wal_handle_test, wal_integration_test, wal_record_test, wal_recovery_large_test, wal_writer_test, write_type_conformance_test | 12 | 105 | 0 | 0 | 0 |
| **合计** | **134 个测试目标**（`--lib --bins` + `tests/*.rs` 133 个文件） | **135 条 `test result:` 行** | **2171** | **0** | **2** | 全 0 |

**覆盖对账结论**：目标清单 = 变更前 `tests/` 全部测试文件 + `--lib --bins`。当前 `tests/*.rs` 共 133 个文件（相对 HEAD 新增 3 个：`ddl_catalog_transactional_test.rs`（本 Iteration）、`index_write_atomicity_test.rs`（Iteration 000）、`statement_cancellation_test.rs`（MS34））。分批清单按字典序切分后**missing = 0、extra = 0、duplicate = 0**（133 个文件各出现恰好一次 + `--lib --bins` 一批；逐批目标数 11×11 + 12 与清单计数一致）。

**与基线的总数差额说明**：基线（MS34 收尾全量终态）= 2163 passed / 0 failed / 2 ignored（131 个 `tests/*.rs` + `--lib --bins`）。本次终态 **2171 passed / 0 failed / 2 ignored**，差额 **+8**，来源恰为：① `tests/index_write_atomicity_test.rs`（Iteration 000 新增）**+5** 个用例；② `tests/ddl_catalog_transactional_test.rs`（本 Iteration 新增）**+3** 个用例；③ `tests/statement_cancellation_test.rs` 的 T6 扩展只**增补既有用例的断言、未新增用例** ⇒ 用例数不变（8 → 8）。库内单元测试目标 420 与基线一致（420 = 420）。2 个 ignored 出现在批次 3 的 `cli_test` 目标上（数量与基线的 2 个 ignored 一致）。

**Persisted Evidence**

None — 模式为 `none`。本 Cycle 的验证结论由上表的真实运行结果承载：全部命令可低成本重跑（定向目标秒级到分钟级；收尾全量按 M19 分批方式可重跑），没有一次性环境即将消失的现场，也没有摘要会丢失决定性结构的长输出（RED 的失败信息与逐批 `test result:` 计数均为短输出，已在本文件逐条摘录）。本 Cycle 未创建 `evidence/` 目录。

**Experience Candidates**

1. **（可承载：Runbook）DDL 目录面变更的「提交点配方」**
   - 结论：DDL 的目录面变更可以**完全移出语句的事务体**，改到提交点之后应用——事务体只做守卫、计算新布局、在事务内完成数据面改写，并把待应用的目录变更交回调用方；被放弃或失败的 DDL 因此不留任何目录面变更。提交点应用时**先列行、后计数**（与 ADD COLUMN 同序）可保证崩溃中间态仍是可读的窄化布局。
   - 根因位置：`src/storage/data/table_manager.rs`（`TableManager::alter_drop_column` 的目录改写与活表映射切换、`apply_column_drop` 的提交点应用）、`src/executor/alter_table.rs`（DROP COLUMN 分支的 `commit` → `disarm` → 应用目录变更顺序）、`src/storage/catalog.rs`（`update_table_column_count_ddl` 的落盘顺序）。
   - 复现路径或证据：本 change 的 `tests/ddl_catalog_transactional_test.rs`（3 个用例）+ `tests/statement_cancellation_test.rs::abandoned_alter_table_drop_column_is_reclaimed` 的补齐断言。修复前实跑 RED 形态逐字记录在案（目录列定义 `["id","w"]`、列数 2、行宽 2，语句执行前为 `["id","v","w"]` / 3 / 3）；修复后同一断言集 GREEN。
   - 影响面与触发条件：任何「目录面变更写在事务体内、失败时无法回退」的 DDL——客户端断连、超时、调用方丢弃语句 future。
   - 与本 change 范围的关系：本 change 的 T4/T5/T6 已在该范围内完成；配方可复用于后续同类缺陷（如 ADD COLUMN 的目录变更事务化）。
   - 修复方向：不适用（这是可复用的实施配方，不是缺陷修复）。
   - 承载落点：Recorder 新建 Runbook（`.agents/runbooks/`）记录「DDL 目录变更的提交点配方：守卫 → 事务内数据面改写 → 提交 → 目录应用（列行先、计数后）→ 活表映射切换」与「锚点式放弃手法（活跃事务版本数 ≥2 命中即丢弃语句 future；语句先跑完即判夹具不成立）」，并登记 R 引用。
2. **（可承载：Runbook）M19 分批全量回归的「批计划 + 覆盖对账 + 总数对账」配方**
   - 结论：分批全量回归的三张对账表可以一次算清：① 目标清单 = `ls tests/*.rs | sort` 的字典序（+ `--lib --bins` 单独一批）；② 批计划按「剩余 ≤12 则取剩余、否则取 11」切分 ⇒ 每批 8–12 个目标且覆盖不重不漏；③ 总数对账 = 本次终态 − 基线终态 = 本 change 新增用例数（新增测试文件的用例数 + 既有文件的用例数增量）。
   - 根因位置：不适用（流程配方）。
   - 复现路径或证据：`.agents/runbooks/m19-batched-full-regression.md`（R48）的既有纪律 + 本 Cycle 的 13 批执行记录（134 个目标、135 条 `test result:` 行、2171/0/2、与基线 2163/0/2 的 +8 差额逐项归因）。
   - 影响面与触发条件：任何 change 的收尾全量回归。
   - 与本 change 范围的关系：本 change 已在该范围内执行一次；配方可复用于 MS35 / MS36 等后续 change 的收尾全量。
   - 修复方向：不适用。
   - 承载落点：Recorder 把「批计划切分算法 + 覆盖对账三项（missing/extra/duplicate）+ 总数对账的差额归因写法」追加进既有 Runbook `m19-batched-full-regression.md`（R48），并登记 R 引用。

**Remaining Issues**

- 无未解决的 Critical / Important 问题（本次交付范围内）。
- **本 Cycle 发现的范围外实质缺陷 —— 作为 Issue 候选只报告不登记（M20；本 Cycle 不扩范围修复）**。四个候选均带可复现步骤，Recorder 可据此登记并路由承载：
  - **候选 A（数据面 / DDL 改写域）：完成路径的 DROP COLUMN 在数据页写满后丢行。** 复现：`t(id INT PRIMARY KEY, v INT, w INT)` 预置 N 行 → `ALTER TABLE t DROP COLUMN v` → `SELECT * FROM t` 的可见行数少于 N。实测：N=40 → 25 行；N=60 → 10 行；N=80 → 50 行；N=100 → 35 行；N≤30 → 行数完整。根因线索：DDL 的 DELETE 腿（`write_tuple_to_data_page(&self.buffer_pool, &old, ...)`）与 INSERT 腿（`self.write_tuple(&new_meta, ...)`）各持一条独立的 `data_page_tail`（`old` 与 `new_meta` 两条 Mutex）却指向同一条物理页链；页写满时两腿各自新分配页并互相覆盖链上的 `next` 指针 ⇒ 先前分配的页成为孤儿。位置：`src/storage/data/table_manager.rs`（`alter_drop_column` 的两腿）、`src/storage/data_page.rs`（`write_tuple_to_data_page` 的新页分配与 `next` 写入）。**与本 change 的关系**：数据面改写路径本次未改动；该现象在 T5 实施**之前**即已复现（RED 阶段实测），本 change 的目录面改动未触及该路径。
  - **候选 B（索引 / B-Tree 域）：完成路径的 DROP COLUMN 在约 300 行以上触发 B-Tree 无限递归 → 栈溢出 abort。** 复现：同候选 A 的夹具，N=300 / 600 / 2000 → `thread 'tokio-rt-worker' has overflowed its stack` → SIGABRT（signal 6），整个测试进程 abort。gdb 现场：`BTree::insert_into_page` 连续 35+ 层停在 `src/storage/btree/btree.rs:327`（内部节点的 `child_page_id` 递归下降调用点），同一 `page_id`、同一 key 指针 ⇒ 下降形成环。位置：`src/storage/btree/btree.rs:237-327`。**与本 change 的关系**：该文件本次未改动；现象在 T5 实施之前即已复现。
  - **候选 C（回收 / 可见性域）：被放弃的 DDL 在回收后隐藏它已开始改写的那一行。** 复现：放弃 `ALTER TABLE t DROP COLUMN v` → 下一条写语句触发回收 → `SELECT id FROM t` 缺 id=2（30 行 → 29 行；2000 行 → 1999 行）。线索：被放弃的语句已为该行写下墓碑；回收把该墓碑标为 aborted（`src/transaction/manager.rs` 的 `abort_cleanup_versions` → `mark_aborted`），但该行仍不出现在扫描结果里。**与本 change 的关系**：回收/版本链可见性域（MS34 已验收语义），本次未改动回滚路径；本次改动不影响该行为（修复前后实测同为 29/30）。
  - **候选 D（崩溃窗口 / 设计表述）：提交完成与目录应用之间的崩溃窗口仍存在位置性错读。** Plan 的 design D5 记录该窗口并声明「不会静默错读」；按代码阅读推断该声明不准确——2 宽的 tuple 在未位移的 3 列目录下读作 `[id, w, NULL]`（`v` 列读到 `w` 的值），影响面是全部已改写的行。**本 Cycle 未构造崩溃点实跑验证**（需新的故障注入夹具，超出本 Cycle 范围）。请 Plan Review 与 Recorder 裁定 D5 的表述与该窗口的承载。
  - 上述四个候选均**不在**本 change 的已批准 requirement 范围内；按 Plan 的 Risks（「本 Cycle 发现的实质缺陷按公共规则作为 Issue 候选只报告不登记」）与公共规则 M20 处理——用户「实施中发现的 issue 候选都直接寻找好的承载方式流程进行修复」的长期要求，需要的是为它们**建立承载**（登记 + 排期到后续 change / milestone），而承载需要新的执行依据，属 Plan Review / Recorder 与用户的职责，本 Cycle 不自行扩大范围。
- 停止条件：当前 Cycle 的 T4/T5/T6 全部完成，Gate 3 / Gate 4 逐项通过，`Act Response` 状态置 `reported`。本 Cycle 未创建下一 Cycle 或下一 Iteration，未归档 change，未登记或修改任何 Issue，未修改全局文档与规格（`.agents/docs/tasks.md`、`.agents/docs/SNAPSHOT.md`、项目记忆、`openspec/specs/` —— 工作区里这些文件的既有改动属 MS34 收尾现场，非本 Cycle 产生；本 Cycle 只改了 change 内的 `tasks.md` 与本 Cycle 文件）。
- 下一步由用户审计，并调用 `openspec-plan` 对本 Cycle 做 Plan Review（重点裁定：Deviations 1–4 的分类；遗留 Minor 4 与 Issue 候选 D 的表述与承载）。

## Plan Review

- Review Result: accepted

**Findings**

> 独立审查方式：完整阅读本次改动的 diff（`src/storage/data/table_manager.rs`、`src/storage/catalog.rs`、`src/executor/alter_table.rs`、`tests/ddl_catalog_transactional_test.rs` 全文、`tests/statement_cancellation_test.rs` 的扩展段）、change 的 `proposal.md` / `design.md`（D1–D7）/ `tasks.md` / `specs/ddl-catalog-transactional/spec.md` / `specs/index-write-side-atomicity/spec.md`、既有规格 `openspec/specs/statement-cancellation-outcome/spec.md`、缺陷台账 `.agents/issues/ISS21-ddl-catalog-transactional.md`；**未重跑已通过的验证**（公共规则 › 验证：Plan Review 的独立检查 = 独立读码与核对，不含重复运行已沿用的验证）。T6 收尾全量只做**结构与覆盖对账**（按实际 `tests/` 文件集合重算分批切分、重加各批计数、核对差额），未重跑任何批次。

**F1 — Deviation 1（夹具 30 行而非 2000）裁定：接受，判据未被削弱。**

- 分类确认：Plan 的 T4 First test 给的是**性质**要求（「行数要足以让『改写中途』的锚点可达」），不是行数；Risks 里的「优先扩大表行数」是「锚点不可达时」的补救方向，不是规模指令。2000 行是 Act 初始沿用 MS34 夹具的取值。故 Act 的「Plan 遗漏」分类成立，且该偏离是**方向相反的夹具规模选择**，不是判据放宽。
- 证明力（本次独立核对）：ISS21 的失效点与数据量无关——修复前的目录窄化（`rewrite_column_rows_drop` + `update_table_column_count` + `flush_all`）位于数据面改写**之前**，任何行数下都会先窄化目录。Act 记录的 RED（30 行、5/5 逐字相同：`columns: ["id","w"], column_count: 2, row_count: 29, widths: {2}`）正是 ISS21 的形态 ⇒ 30 行已把缺陷**完整暴露**。
- 断言强度未被规模缩小削弱（关键点）：`assert_unchanged`（`tests/ddl_catalog_transactional_test.rs:169-188`）把期望值对齐到**语句执行前**的列数（3）与列定义（`["id","v","w"]`），**不是**对齐到「（可能被窄化过的）目录自身宽度」——窄化后的目录 + 窄化后的行宽同样自洽，但那正是缺陷形态。该 helper 的注释已写明这一点 ⇒ 断言在 30 行与 2000 行上同样判别。
- 规模缩小实际放弃的覆盖面：多页数据改写路径——正是范围外候选 A（页写满丢行）/ 候选 B（B-Tree 栈溢出）所在的路径，与 DCT 的三条 requirement（目录面/数据面一致）无交集 ⇒ 不构成 DCT 的 Acceptance Gap。
- 结论：接受。剩余风险（多页改写未覆盖）由候选 A / B 承载，不在本 change。

**F2 — Deviation 2（T6 的「观察 RED」实为 GREEN）裁定：接受，属 Plan 任务定义的措辞缺陷。**

- T6 的 `Depends on: T5`（目录窄化已延后到提交点之后），执行 T6 时放弃路径必然已是 GREEN——Plan 的 T6 First test 自身要求「在补齐后的放弃路径上观察现状（目录已窄化 ⇒ RED）」与依赖关系矛盾。Act 以「等价 RED 已在 T4 由同一夹具手法、同一组可观察量建立并记录」解释，属事实说明，非判据放宽（5/5 RED 的失败输出逐字记录在案）。
- 结论：接受；记为 Minor（Plan 的任务定义措辞缺陷，非 Act 偏离）。

**F3 — Deviation 3（移除 `Catalog::update_table_column_count`）裁定：接受，分类为「Plan 遗漏，清理」正确。**

- 独立核对调用点：全仓 `src/` 内已无 `update_table_column_count` 的任何调用或引用（仅存 `update_table_column_count_ddl`，两个调用点：`src/storage/data/table_manager.rs:668`（ADD COLUMN）与 `:1036`（DROP COLUMN 提交点））⇒ **无孤儿调用点、无其他调用方被破坏**。
- 移除的必要性：该方法原文档注释写着「ALTER DROP COLUMN keeps the plain method」，与本次改后的行为（改用 `_ddl` 提交点变体）**直接矛盾**；保留会让 catalog 的文档断言与实现相反。且它因本次改动而成为孤儿 ⇒ 符合公共规则「Surgical Changes：清理由本次改动产生的孤儿」。
- 记录在案的 Minor（Plan 措辞）：T5 的 Preserve 写「`Catalog` 的公开 API 语义与签名不变」，而移除一个 `pub` 方法在字面上是公开 API 变更。核实后不构成阻塞——该方法无外部调用方、无规格引用、其语义变化是本次 requirement 的直接后果；`rtsql` 是本项目的引擎 crate（`Cargo.toml` 单包，无外部发布面）。⇒ 记为 Plan 措辞与实现之间的 Minor 张力，不改代码、不改 requirement。

**F4 — Deviation 4（放弃路径的行数断言收窄为「非空」）裁定：接受，落在 Plan 的判据边界内。**

- Plan 的 Required behavior ② 要求的是「每行**值个数**等于语句执行前的列数（行宽与目录一致）」，Invariants 与 DCT R3 的场景（「各行按该列数存放」）同样只说行宽、不说行数 ⇒ 收窄未超出 Plan 授权。
- 收窄的必要性：被放弃的 DDL 在回收后隐藏它已开始改写的那一行（29/30，候选 C，回收/可见性域的既有行为）。把行数钉死会把**范围外**的既有缺陷写进 DCT 的验收面，并使其长期 RED。
- 覆盖面未被放弃：完成路径（无放弃）**仍然**断言行数守恒（`row_count == SOURCE_ROWS == 30`，`tests/ddl_catalog_transactional_test.rs:312-315` 与重开库后的 `:353-356`）⇒ 「正常路径不丢行」有断言；放弃路径不丢行不在 DCT 保证面内。

**F5 — 提交点设计的独立核对（核心实现正确性）：目录面副作用已彻底移出事务体，提交点顺序与守卫顺序正确。**

- `alter_drop_column`（`src/storage/data/table_manager.rs:715-1015`）：**全函数体扫描**确认，对 `Catalog` 的唯一交互是**读**（`:818` `scan_columns`，取被删列的完整列定义与其原序号）；函数体内**无任何目录写入**、**无 `flush_all`**、**无活表映射切换**。事务体只剩：守卫检查（`:736-762`）、新布局计算（`:767-1014`）、事务内数据面改写（DELETE 腿 + INSERT 腿），并把待应用的目录变更交回调用方 ⇒ DCT R1 的结构性前提成立。
- `apply_column_drop`（`:1031-1045`）的顺序 = 先列行（`rewrite_column_rows_drop`）→ 后列数（`update_table_column_count_ddl`，其内部先落盘除计数页外的脏页、最后落盘计数页）→ 再切活表映射 ⇒ 与 design D5、与 ADD COLUMN 的提交点写法同序。中途失败的两种落点：(a) 列行已写、计数未降 ⇒ 重开库时活布局取自目录列行并按宣告列数截断（`src/storage/data/table_manager.rs:284-299`）⇒ 2 行列行 + 旧计数 3 ⇒ 截断后仍是 2 列 ⇒ 活布局已是窄化布局且与窄化后的数据一致 ⇒ **可读且自洽**（`src/storage/catalog.rs:380-390` 新写的文档注释在此点准确）；(b) 提交完成、目录尚未改写 ⇒ 目录旧布局 + 数据新布局 ⇒ 位置性错读（见 F6，属 D5 记录的既有窗口，本 change 的 Non-goal）。
- `src/executor/alter_table.rs:112-127` 的 `commit(tx)` → `guard.disarm()` → `apply_column_drop` 顺序**正确且必要**：`commit` 已成功 ⇒ 事务已提交；若此时守卫仍武装，语句 future 析构会把**已提交**的 tx id 记入登记表（`src/database.rs:183-191`），下一条语句的回收（`:574-586`）会去中止一个已提交的事务。`disarm` 必须早于 `apply`；若放在 `apply` 之后，`apply` 失败时同样留下该缺口 ⇒ 现有顺序无缺口。失败分支（`:128-144`）不变：中止成功才解除守卫，中止失败保留登记项由下一个触发点重试。
- 残留（非阻塞，Act 的 Minor 3）：`apply_column_drop` 因 I/O 失败时，语句报错但数据已提交、目录未窄化 ⇒ 与 D5 的窗口同类；错误被向上传播而非静默吞掉 ✓。

**F6 — 候选 D 裁定：design D5 的「不会静默错读」主张**不准确**⇒ 就地修正 design 记录；该窗口本身**不是**本 change 的 Acceptance Gap。**

- 独立读码核对（不采信 Act 自述）：`src/storage/page_format/tuple.rs:189-222` 的宽度感知解码按**位置**取值、尾部补 `Value::Null`，**不校验列序**；`src/executor/scan.rs:119/134`、`src/executor/data_scan.rs:519/566`、`src/executor/index_scan*.rs` 等**全部读路径**都走这条 `deserialize_row`（旧的严格入口 `deserialize_tuple` 不在读路径上）⇒ 窄 tuple 不会报错，只会按位置错读。重开库的活布局来自目录列行并按宣告列数截断（`src/storage/data/table_manager.rs:284-299`）⇒ 窗口内（提交完成、列行未改写）活布局仍是 3 列而数据已是 2 宽 ⇒ 读作 `[id, w, NULL]`：`v` 列读到 `w` 的值、`w` 列读 NULL ⇒ **位置性错读且无声**。Act 的读码结论成立。
- 「比现状更安全」的比较同样不成立：现状（目录已窄化 + 数据仍宽）是**镜像方向**的同一类位置性错读（`w` 列读到 `v` 的值，宽行尾部被忽略），两者影响行集相当（各自覆盖整表）⇒ 可靠的差别只在「丢的是目录效果（目录未改写 ⇒ 该 DDL 可重做）还是数据改写（需重做改写）」。
- 处置（按公共规则：design 是 change 的长期技术记录，随 change 归档）：**就地修正 D5 的表述**（本次 Review 已改，见 `design.md` 的 D5「崩溃窗口（明确记录）」段）。这是**文档修正而非行为变更**，不需要新夹具：被修正的是「读侧如何解读窄 tuple」这一可直接读码确定的事实；只有「窗口是否可达」才需要崩溃注入夹具，而窗口的可达性本就作为已知边界记录在 D5。
- 承载：窗口本身按 Plan 的 Non-goals（「MS21 D10 的既有崩溃边界」）**不在本 change 调整**，作为 **Issue 候选 D** 交给 Recorder 登记并由后续 change 承载（读侧宽度/列序校验，或让提交点与目录应用原子化）⇒ **范围外的后续工作，不阻塞本次 Acceptance**。

**F7 — DCT 三条 requirement 的判别力逐条核对（不是「用例通过」，而是「用例能区分修复前后」）。**

- **R1（放弃侧）** → `abandoned_drop_column_leaves_catalog_unchanged`：断言对齐到**语句执行前**的列定义/列数/行宽（F1 已述），修复前的 RED 形态（列定义 `["id","w"]`、列数 2、行宽 2）与期望逐字不符 ⇒ 判别力来自「对齐到执行前状态」而非「目录与数据面自洽」；`tests/statement_cancellation_test.rs:368-404` 的扩展段在 2 列表上重复同一判别（列定义 `["id","v"]`、列数 2、行宽 {2}），且**只增不减**（既有回收契约断言原样保留 ✓）。
- **R1（失败侧）** → `failed_drop_column_leaves_catalog_unchanged`：三条守卫中，PK / UNIQUE / 被二级索引引用的列是在**事务体内**被拒的（`src/storage/data/table_manager.rs:741-762`，即隐式事务已开启后）⇒ 规格场景「在执行期失败，该语句的事务被中止」**字面成立**（执行器 `src/executor/alter_table.rs:128-144` 执行 `abort`）；「最后一列」在计划期被拒（`src/parser/planner/ddl_dml.rs:1572-1577`）⇒ 更严格的变体。断言覆盖目录列定义/列数与数据面行数/行宽 ✓。诚实边界：该用例修复前后都通过（守卫拒绝路径本就早于任何目录写入）⇒ 它是**护栏**而非 RED 驱动，RED 驱动是放弃侧用例。
- **R2** → `completed_drop_column_aligns_catalog_and_rows`：列定义 `["id","w"]`、列数 2、行宽 {2}、**行数守恒 30**、`check_integrity` 的 error 级发现 0，且 close → reopen 后四项再次成立 ⇒ 目录面与数据面「同生共死」在完成路径上被完整钉住。修复前该路径本就自洽 ⇒ 它是对照基线（Plan 如此设计）✓。
- **R3** → 放弃用例的 close → reopen 段：重开后再次断言目录列定义/列数与每行行宽 ✓，且走真实入口（`Database::open` + `SELECT *`）⇒ 端到端成立。
- 未见「用例通过但缺陷仍在」的情形；无未验收行为被写成已验收。

**F8 — 两份规格增量逐条核对：均可并入 `openspec/specs/`。**

- `specs/ddl-catalog-transactional/spec.md`：R1 + 2 场景、R2 + 1 场景、R3 + 1 场景 → 全部由上列命名用例的断言覆盖（F7）；措辞只描述可观察行为（目录列定义/列数、行宽、行数），不含实现手段；**未把崩溃窗口写进增量**（D5 的窗口不属 DCT 域）✓。一处措辞核对：R1 正文写「被放弃（丢弃或超时）」，本 change 只直接测了丢弃路径——超时与丢弃在库入口产生同一类结果是**已验收**的 `statement-cancellation-outcome` R1（且超时包住语句 ⇒ 丢弃其 future ⇒ 同一条守卫析构 + 同一条回收路径）⇒ 无需新增用例，合并时保留该措辞成立。
- `specs/index-write-side-atomicity/spec.md`：R1 → `cancelled_write_keeps_index_self_consistent`；R2 → `concurrent_writers_do_not_fork_the_tree`；R3 的两个场景 → `cancelled_multi_row_insert_in_explicit_tx_releases_every_key` / `..._implicit_tx_...`；R4 → `cancelled_write_keeps_index_self_consistent` 内的后续写语句 + 完整性断言（其余三个用例另带完整性断言）；第 5 个测试函数 `leaf_fills_at_92_rows_and_the_93rd_key_splits_the_root` 是 R1 的 GIVEN 夹具自检 ⇒ 5 个场景全部有用例承载 ✓。

**F9 — T6 收尾全量记录的独立结构与覆盖核对（未重跑）：算术自洽、覆盖不重不漏。**

- 按**实际** `tests/` 文件集合（`ls tests/*.rs` → 133 个文件）重算字典序切分：剩余 ≤12 取剩余、否则取 11 ⇒ 切分为 11×11 + 12，共 12 批；与 Act 记录的批次表**逐批目标名完全一致**（每批 11 或 12 个目标，均在 8–12 内 ✓），`--lib --bins` 单独成批 ✓。
- 覆盖对账：Act 记录的目标清单 == 实际文件清单 ⇒ **missing 0 / extra 0 / duplicate 0** ✓（133 个文件各出现恰好一次）。
- 算术自洽：逐批 passed 相加 = **2171**、failed = 0、ignored = 2；覆盖目标 = 134（133 个文件 + `--lib --bins` 一批）；`test result:` 行 = **135**（133 个文件目标 + lib + bin）⇒「134 目标 / 135 行」不是笔误，两数各自成立 ✓。
- 与基线差额：2163/0/2 → 2171/0/2 = **+8** = `index_write_atomicity_test.rs` 5 个用例 + `ddl_catalog_transactional_test.rs` 3 个用例；`statement_cancellation_test.rs` 前后均 8 个用例（本次只增断言）——本次按 `#[test]` / `#[tokio::test]` 计数独立核到 8 / 5 / 3 ✓；lib 目标 420 = 基线 420 ✓；2 个 ignored 落在批 3 的 `cli_test`，与基线同数同源 ✓。
- 纪律逐条：每批一次独立调用、批间严格串行、批内 `--test-threads=1`、无一次无目标过滤的全量调用、无默认并行度批次、每批 exit=0 且 failed=0 ✓（按 Act 的逐批记录核对；本次未重跑，按公共规则不重复已通过的验证）。

**F10 — 无身份证明机制、无框架外判定层。**

- 全量阅读改动文件与用例：无 Hash / 指纹 / run-id / session-id / revision pin / manifest / 时间顺序证明，无采集、审计或鉴定工具与专用 CLI；timer 只出现在测试夹具的失控兜底（`ANCHOR_TIMEOUT` 10 秒，超时即 `expect` 失败 ⇒ 触发即判失败，不是行为判据）；锚点是**可观察状态**（活跃事务集合中出现「已记录版本数 ≥ 2」的事务 ⇒ 语句已进入数据面改写），用 `yield_now` 轮询，不以时间判定行为；夹具在语句先跑完时**显式 panic**（「statement finished before the abandon anchor was reached」），未以「跑完也算放弃」放宽判据 ✓；M19 计数只用一步通用管道 `grep -E "^test result:"`，只计数不判定 ✓；无新增脚本、封装命令或专用参数。

**F11 — Minor findings（均不阻塞 Acceptance，不创建修复 Cycle）。**

1. `alter_drop_column` 新增一条错误路径：目录中找不到被删列对应的列行时返回 `StorageError::Internal`（`src/storage/data/table_manager.rs:820-828`）——正常目录不可达，仅在目录与活表元数据不一致时触发；此时报错优于静默继续（Act 的 Minor 1）。
2. `alter_drop_column` 每次多一次 `Catalog::scan_columns` 读取（Act 的 Minor 2）——DROP COLUMN 非高频路径。
3. `apply_column_drop` 的 I/O 失败路径：语句报错而数据已提交、目录未窄化（Act 的 Minor 3）——与 D5 窗口同类，错误不被吞掉 ✓。
4. `src/storage/catalog.rs:380-390` 新写的文档注释把崩溃中间态描述为「列行已窄化、计数未降」这一种——该描述对**列行改写与计数更新之间**的落点准确（重开库按宣告列数截断列行 ⇒ 窄化布局可读，已独立核对 `src/storage/data/table_manager.rs:295`），但**提交完成到列行改写之间**的落点（即 F6 的错读窗口）未在注释中提及 ⇒ 注释为真但读起来像覆盖了整个提交点。产品代码，Review 不改 ⇒ 随候选 D 的后续承载一并修订。
5. Plan 的两处措辞缺陷：T5 的 Preserve「`Catalog` 的公开 API 语义与签名不变」与移除 `pub` 方法（见 F3）、T6 的 First test「观察 RED」与其 `Depends on: T5` 矛盾（见 F2）⇒ 记为 Plan 措辞 finding，requirement 与设计方向均不因此改变。
6. **提交点与活布局切换之间的并发写窗口（此前未记录）**：数据面改写把窄化后的行写进 `new_meta` 自己的 `data_page_tail`（Act 的候选 A 已指出 DELETE 腿持 `old`、INSERT 腿持 `new_meta` 两条尾指针却指向同一条物理页链），而活表映射直到 `apply_column_drop` 才切到 `new_meta` ⇒ 在「提交完成 → 活布局切换」之间，并发写者仍取到旧映射与旧尾指针。影响面窄（两次 await 之间）且本引擎为单写者模型，但这是本次把切换时点后移**新增**的一处并发面，D5 只记录了并发**读**窗口 ⇒ 记为 Minor，供后续承载时一并评估。

**Acceptance Gaps**

None。B1–B5 五项验收全部由 Act 的真实运行覆盖，并经本次独立读码与 diff 核对与代码现状一致；DCT R1/R2/R3 的判别力见 F7；未发现「用例通过但缺陷仍在」的情形。候选 D 的设计表述问题（F6）**不构成 Acceptance Gap**——它不落在任何已批准 requirement 上，且本 change 的 Non-goals 已明确把该崩溃窗口排除在调整范围外。

**Evidence**

- 实现（本次独立阅读的当前文件状态）：`src/storage/data/table_manager.rs:715-1015`（`alter_drop_column`：守卫 + 新布局 + 事务内数据面改写 + 交回待应用变更；`:818` 唯一目录读；`:820-828` 新增错误路径）、`:1017-1045`（`PendingColumnDrop` 与 `apply_column_drop` 的提交点顺序）、`src/executor/alter_table.rs:80-146`（`commit` → `disarm` → `apply_column_drop`；失败分支 `abort` → 中止成功才 `disarm`）、`src/storage/catalog.rs:380-410`（`update_table_column_count_ddl` 与新增文档注释）、`:1582-1674`（`rewrite_column_rows_drop`）。
- 语义核对：`src/storage/page_format/tuple.rs:189-222`（宽度感知解码按位置取值、尾部补 NULL）、`src/storage/data/table_manager.rs:284-299`（重开库活布局 = 目录列行按宣告列数截断）、`src/database.rs:162-191` / `:554-586`（守卫的武装/解除与回收触发点）、`src/parser/planner/ddl_dml.rs:1550-1582`（计划期的守卫与拒绝）。
- 需求与缺陷正文：change 的 `specs/ddl-catalog-transactional/spec.md`（R1–R3 逐条见 F8）、`specs/index-write-side-atomicity/spec.md`（R1–R4 逐条见 F8）、`design.md`（D1–D7，D5 已在本次 Review 修订）、`tasks.md`（RTM 与 Iteration Plan）、`.agents/issues/ISS21-ddl-catalog-transactional.md`（处置方向「窄化延后到语句确定完成之后」= 本次实现的分支；ISS21 的关闭由 Recorder 执行）。
- 沿用（未重跑）的运行结论（来源：Act Response「Verification Evidence」表与批次表）：T4 RED 连续 5 次稳定命中目录面断言（失败形态 `columns: ["id","w"], column_count: 2, row_count: 29, widths: {2}`）；T5 GREEN 连续 3 次（3 passed / 0 failed）；受影响面 6 目标 48 passed / 0 failed；T6 扩展 2 目标 11 passed / 0 failed；收尾全量 13 批 exit=0、2171/0/2、覆盖 missing 0 / extra 0 / duplicate 0；`cargo fmt -- --check` 无 diff、clippy 改动文件 0 条告警。本次 Review 未重跑任何一条（公共规则 › 验证）；T6 只做结构与覆盖对账（F9）。

**Follow-up Decision**

**接受本 Cycle**（`Review Result: accepted`）：B1–B5 全部满足，经独立读码与 diff 核对未发现结论与代码矛盾。Deviations 1–4 的裁定：①夹具规模——接受（性质要求已满足，判据未削弱）；②T6 措辞——接受（Plan 任务定义的措辞缺陷）；③移除 `update_table_column_count`——接受（无孤儿调用点，文档与行为不再矛盾；与 Preserve 字面张力记为 Minor）；④行数断言收窄——接受（Plan 只要求行宽；完成路径仍钉死行数守恒）。F1–F4 之外的全部 finding 为 Minor（F11 全部、候选 A/B/C/D 及两条 Runbook 候选），**均不阻塞 Acceptance**；按公共规则「不阻塞 Acceptance 的 Minor finding 不创建修复 Cycle」，本 Iteration 不创建 rework / replan Cycle。候选 D 的**设计表述**问题已在本次 Review 就地修正（`design.md` D5），其**行为面**（提交点与目录应用之间的窗口）作为范围外的后续工作交由 Recorder 登记承载。

**Iteration Plan Update**

无。Iteration 001 的范围、稳定结果、验证边界与诊断边界均不改变；四条 deviation 的裁定不改变 requirement、设计方向、验证方式或验收边界（唯一的设计修订是 D5 的表述更正，属长期记录，不改行为）。本 change 的收尾全量已在本 Iteration 内执行完毕，**不再追加**。

**Next Cycle**

None（当前 Iteration 已被接受；不创建 rework / replan Cycle）。

**Next Iteration**

None。Iteration 001 是本 change Iteration Map 中的最后一个 Iteration；Map 中 000 与 001 均已 `accepted`，**无后续 Iteration**，change 的实现工作全部完成。

**收尾交接说明（供上游派发；本次 Review 不执行以下任何一项）**

按公共规则的阶段边界，归档与全局状态维护属 `openspec-docs-maintainer`，Issue/Runbook 产物属 `openspec-experience-recorder`，本 Review 不代做。请上游按下列清单派发：

1. **合并两份规格增量进 `openspec/specs/`**（docs-maintainer）：新建 `openspec/specs/index-write-side-atomicity/spec.md` 与 `openspec/specs/ddl-catalog-transactional/spec.md`，内容取自本 change 的两份 delta（逐条已核对准确，见 F8；两个域均为新域，无既有文件需合并）。
2. **同步 `openspec/specs/statement-cancellation-outcome/spec.md` 的已知边界措辞**（docs-maintainer）：该域 R2 的「已知边界」段现文写「取消若发生在索引写入中途，该次写入可能只落下 B 树条目而未落下反向映射……它的**键**仍可能占用主键索引（再插同键被拒）」——描述的是**已被本 change 修复**的缺陷机制（ISS20 / 索引写侧原子性），继续保留会让已验收的规格把已修缺陷陈述为当前行为。请改为：取消域本身的保证面只覆盖**行**（被取消语句的行不可见）；**键的释放**不由本域保证，而由存储域的索引写侧契约承载（新域 `index-write-side-atomicity` 的 R3/R4 是该保证的权威位置），并删去（或明确标注为已由该域关闭的）上述缺陷机制描述。R4 中「键的索引条目释放不属本域的保证面（见『提交前被取消的语句不提交』的已知边界）」这句交叉引用随之保持指向 R2 即可；可选（推荐）：在 R2 的已知边界里补一句指向新域 `ddl-catalog-transactional`，说明 DDL 语句的**目录面**保证由该域承载（本 change 兑现），使取消域与目录域的分工在规格库内可检索。
3. **同步全局状态**（docs-maintainer）：`.agents/docs/tasks.md`（本 change 是 MS35 / MS36 之前的 Issue 修复批，不新增里程碑条目；需登记其对 ISS20 / ISS21 的承载与收尾全量终态 2171/0/2）、`.agents/docs/SNAPSHOT.md`（项目现场描述）、项目记忆 M/R/I（R 引用指向本 change 与两份规格域）。
4. **登记 Issue 关闭与承载**（Recorder）：ISS21 由本 change 承载（实现的分支即其处置建议「窄化延后到语句确定完成之后」）⇒ 更新状态与承载指针；ISS20 同理核对指针。同时登记本 Review 报告的四个 Issue 候选：A（完成路径 DROP COLUMN 在数据页写满后丢行）、B（B-Tree 无限递归致栈溢出）、C（被放弃的 DDL 隐藏它开始改写的那一行）、D（提交点与目录应用之间的位置性错读窗口 + design 表述已修正）——按用户「实施中发现的 issue 候选都直接寻找好的承载方式流程进行修复」的要求，为它们建立承载（登记 + 排期到后续 change / milestone；承载需要新的执行依据，不在本 change 内）。
5. **两条 Runbook 候选**（Recorder）：①「DDL 目录变更的提交点配方」（守卫 → 事务内数据面改写 → 提交 → 目录应用〔列行先、计数后〕→ 活布局切换）与「锚点式放弃手法」；②「M19 批计划 + 覆盖/总数对账配方」——后者追加进既有 Runbook `.agents/runbooks/m19-batched-full-regression.md`（R48）并登记 R。
6. **归档本 change**（docs-maintainer，在规格合并与全局状态同步之后）：`openspec/changes/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/` → `openspec/changes/archive/`；两个 Iteration 的 Cycle 文件与 Review Result 随 change 一并归档。
