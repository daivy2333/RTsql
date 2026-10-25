# Iteration 001 / Cycle 000: 列改写的数据链一致性与放弃可见性

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4, T5, T6, T7
- Acceptance gaps: None（initial）
- Repair items: None
- Inherited scope: DRC R1（列改写不丢失已提交行）、DRC R2（被放弃的列改写不隐藏已改写的行）、既有域 `mvcc-tombstone-visibility` R1（墓碑自描述且不影响前驱版本）、既有域 `ddl-catalog-transactional` R1/R2/R3（目录列的窄化在提交点生效、放弃 DDL 后重开库目录与数据面一致）；Iteration 000 已验收的 IPO R1/R2/R3（页归属唯一性、插入下降必然终止）保持成立；MS35 已验收的 `free-page-lifecycle` 判定时点语义不变。
- Excluded scope: Iteration 002（释放路径有界读）、Iteration 003（改写窗口读一致性 / ISS25）、Iteration 004（释放可分配判定的耐久性前提）、Iteration 005（旧格式兼容证据补齐）；B-3（已被已验收规格显式界定为范围外既有行为）；`VersionHeader::mark_aborted` 的「保留链指针」语义；磁盘格式（`VersionHeader` 布局、目录行布局）；`FileStorage` 的释放集合语义（`free_page` / `complete_free_release` / `promote_free_releases` / `discard_free_releases` / `pending_frees` / `free_pages`）；`SlottedPageRef::get_slot_data` 的签名。

**Objective**

`ALTER TABLE DROP COLUMN` 的改写**只使用一条尾游标**、数据页链**不分叉**，跨页容量边界与重开库两个维度上**已提交行不丢失**（ISS22 闭合）；被放弃（丢弃或超时）的改写在被回收后**不隐藏**它已开始改写的行，且**已回滚的墓碑不抑制其前驱版本**——既有域 `mvcc-tombstone-visibility` R1 的实现面闭合（ISS24 闭合）。ISS24 的缺失行究竟在哪一层消失，按 `design.md` D5 的协议**先判别再修**；M2 无论是否为本次实测症状**都必须修**。

**Background**

ISS22 与 ISS24 是 MS34 修复批登记的两条 open 缺陷（`.agents/issues/ISS22-drop-column-data-page-full-loses-rows.md`、`.agents/issues/ISS24-abandoned-ddl-hides-rewritten-row.md`，均 open）。ISS24 的登记现场明确「根因：Inferred」，因此本 Cycle 的第一步是**判别**，不是直接按某个候选机制改代码。`design.md` D4 给出 ISS22 的修复形态（单一尾游标），D5 给出 ISS24 的判别协议与修复方向。

**Investigation Facts**

- Starting point: HEAD `0a08b14`（ahead of origin 9）。工作区有 **既有未提交改动**（MS34 取消/超时批、MS34 issue-fix 批〔ISS20/ISS21〕、MS35 空闲页批、MS36 兼容批、Iteration 000 的 `src/storage/btree/btree.rs` 改动与 2 个新测试文件），属**既有现场**：不得回滚、不得清理、不得提交；本 Cycle 的改动叠加在其上。
  - Iteration 000（`iterations/000-index-page-ownership/000-initial.md`）的 Review Result 为 **`accepted`**：IPO R1/R2/R3 达成，ISS23 的归属结论为「B-2 的下游后果」。其定向验证结论（`index_page_ownership_test` 5/0、`index_page_ownership_stack_overflow_test` 1/0、6 个既有树相关文件 55/0、`cargo test --lib` 420/0）在本 Cycle **不受影响**（本 Cycle 不改 `btree.rs` 也不改这两个测试文件）⇒ 可沿用。
  - MS35 / MS36 收尾的全量终态 **2192 passed / 0 failed / 2 ignored**（来源 `.agents/runbooks/m19-batched-full-regression.md` R48）**覆盖范围已变**（Iteration 000 之后 `btree.rs` 与 2 个新测试文件已变）⇒ **不得**再作为「当前工作区全量通过」的依据；本 change 的收尾全量在 Iteration 005。
  - 本 Cycle **不跑全量回归**（收尾全量在 Iteration 005，全 change 一次预算）；但**必须**重跑 `cargo test --lib`（本 Cycle 改 `src/storage/data/table_manager.rs` 等 ⇒ Iteration 000 的 420 条 lib 结论对本次改动失效）。
- Current-State Evidence（Plan 已逐行确认，行号为**当前工作区**）:
  - **改写的两条尾游标**：`src/storage/data/table_manager.rs:853-865` 构造窄化副本 `new_meta`（`:854` `name` 与 `old` **同名**，`:863` `data_page_head` 与 `old` 相同），但 `:864` `data_page_tail: Mutex::new(*old.data_page_tail.lock().unwrap())` ⇒ **一个独立的尾游标 `Mutex`**，与 `old.data_page_tail` 互不知情。
  - **DELETE 腿**（`table_manager.rs:945-950`）：墓碑 `VersionHeader::new(tx_id, None).with_next_version(*rid).mark_deleted()`；写入走 `write_tuple_to_data_page(&self.buffer_pool, &old, &tombstone, &[])` ⇒ 用 **`old.data_page_tail`**、**不经 `heal_stale_tail`**、**不回写目录尾指针**。随后 `:956-964` 删除旧行的 PK/唯一/二级索引条目，`:966-968` `record_version(tx_id, &old.name, tombstone_rid)`，`:969-976` 写 WAL `Delete`。
  - **INSERT 腿**（`table_manager.rs:978-1012`）：窄化后的新行 `VersionHeader::new(tx_id, None)`（**无 `next_version`**）；写入走 `self.write_tuple(&new_meta, &vh, &buf)`（`:992`）⇒ `TableManager::write_tuple`（`:2010`）→ `heal_stale_tail`（`:2019` / `:2040`）→ `write_tuple_to_data_page`（`:2021`）→ `catalog.update_table_tail`（`:2026-2029`）⇒ 用 **`new_meta.data_page_tail`**、**自愈陈旧尾**、**回写目录尾指针**。随后 `:996-1012` 重建索引条目，`:1014-1016` `record_version(tx_id, &new_meta.name, new_rid)`，`:1017-1024` 写 WAL `Insert`。
  - **分叉机制（ISS22 的根因）**：`src/storage/data_page.rs:18` `write_tuple_to_data_page` 在 `:39` 读 `tail_id = *table_meta.data_page_tail.lock()`；`:43` 的 `add_slot` 失败 ⇒ `:56` `allocate_page()` ⇒ `:65` **把尾页的 `next` 指针改写为新页**（`page.data[5..9]`）⇒ `:87` `table_meta.data_page_tail = new_page_id`。⇒ 两条腿各自持有尾游标、却写进**同一条物理链**：谁先在某个尾页写满并分配新页并改写 `next`，另一条腿（游标仍停在旧尾页）在旧尾页再次写满时会**再分配一个页并覆盖同一个 `next`** ⇒ **先分配的页成为链上不可达的孤儿**。登记实测：40 行改写后剩 25 行。
  - **只有 DDL 改写存在两条游标**：`heal_stale_tail` 的**唯一**调用点是 `TableManager::write_tuple`（`:2019`）；`src/executor/insert.rs:253`、`update.rs:311`、`delete.rs:169`、`upsert.rs:284/477/646` 等既有写路径虽然也直接调底层入口，但都用**同一张活表 meta**（`self.table_meta`）⇒ 共享一条游标，不分叉。
  - **`pointed` 抑制集合的四处构造**（都把**任何** `next_version` 目标当作「被指向的版本」，**不区分指向者是否已回滚**）：`table_manager.rs` `alter_drop_column`（`:877` 建集、`:897-899` 收集、`:914` `live_rows.retain(|rid| !pointed.contains(rid))`）；`table_manager.rs` `table_has_live_rows`（`:1074` 建集、`:1095-1097` 收集、`:1113` 判定）；`src/executor/create_index.rs`（`:123` 建集、`:143-145` 收集、`:171` `!pointed.contains(&rid) && is_live`）；`src/wal/recovery.rs`（`:1166` 建集、`:1186-1188` 收集、`:1237` `pointed.contains(&tail_rid)` 跳过）。
  - **既有正确判据**：`src/executor/data_scan.rs:339` `DataScanExecutor::superseder_suppresses` 对 `vh.create_tx_id() == 0`（aborted 标记）**返回 false** ⇒ aborted 的指向者**不抑制**其前驱版本。这与已验收的 `mvcc-tombstone-visibility` R1「未提交/已回滚墓碑 SHALL NOT 抑制、评估回溯前驱」同源。
  - **aborted 的表达方式**：`src/transaction/version_chain.rs:81` `VersionHeader::mark_aborted` ⇒ `create_tx_id = 0`、`commit_tx_id = DELETED_TX_ID`、**保留 `next_version`** ⇒ 中性化只能靠 `create_tx_id == 0` 识别（本 Cycle **不改**该语义）。
  - **【Plan 独立复核 · 影响判别设计】扫描路径的版本求值已经正确处理 aborted 指向者**：`data_scan.rs:233` `build_superseded_map` 建立「目标 → 指向者」映射，`:293` `slot_is_superseded` 沿指向者链**逐级**调用 `superseder_suppresses` ⇒ 一个 aborted 的墓碑**不会**让它的前驱版本被判为被压制；`:479-481` 的 `is_deleted()` 跳过只作用于**墓碑槽自身**（墓碑有独立的 RowId，与旧版本不是同一 slot）。⇒ **按当前代码，被放弃的改写丢失的那一行不应出现在「全表扫描的版本求值」这一层**。缺失行更可能落在下面四层之一，RED 必须先钉住是哪一层：
    1. **链分叉（M1）**：该行落在链上不可达的孤儿页 ⇒ 全表扫描沿链走不到它（与 ISS22 同一根因；登记的 40→25 即此形态）。
    2. **索引条目层**：旧版本的 PK / 唯一条目未按预期还原，或指向了不可达的 RowId ⇒ 点查与索引扫描丢行。
    3. **计划期的活行判定层**：`table_has_live_rows`（`:1072`）用不识别 aborted 的 `pointed` 集合判定「表是否还有活行」⇒ 计划期把「其实还有活行」的表判成空表。
    4. **中性化层**：墓碑未被成功中性化（`create_tx_id != 0` 且仍带删除哨兵）⇒ 墓碑按 `superseder_suppresses` 的「已提交删除」分支**压制**其前驱版本 ⇒ 旧版本整行不可见。
  - **放弃语句的既有夹具先例**：`tests/ddl_catalog_transactional_test.rs:63-105` 已有可直接复用的放弃夹具——`abandon_statement(db, sql)`：`Box::pin(db.execute_sql(sql))` + 以「活跃事务集合中出现**已记录版本数 ≥ 2** 的事务 id」为锚点（`anchor_tx`）+ `tokio::task::yield_now()` 轮询 + `tokio::select!` + **`drop(statement)` 丢弃语句 future**；锚点超时只作夹具失控兜底（`ANCHOR_TIMEOUT`），不作行为判据。放弃后由 `src/database.rs:585` `reclaim_abandoned_implicit_transactions` 在**下一条写语句**触发回收（`src/database.rs:613` `reclaim_implicit_tx` → `transaction_manager.abort` → `abort_cleanup_versions`，`src/transaction/manager.rs:321`）。DDL 执行侧见 `src/executor/alter_table.rs:100-146`（`implicit_tx_guard` + `Ok` 分支提交 + `apply_column_drop` 在提交点之后生效）。
  - **既有测试入口**：`tests/alter_table_test.rs`、`tests/ddl_catalog_transactional_test.rs`、`tests/mvcc_tombstone_visibility_test.rs`、`tests/mvcc_abort_test.rs`、`tests/recovery_test.rs`、`tests/recovery_e2e_test.rs`、`tests/recovery_tail_persistence_test.rs`、`tests/index_manager_test.rs`、`tests/secondary_index_test.rs`。**本 Cycle 新增** `tests/ddl_rewrite_consistency_test.rs`。
- Code and Critical Path:
  - 调用链（改写）：`AlterTableExecutor`（`executor/alter_table.rs`）→ `TableManager::alter_drop_column`（`table_manager.rs:734`）→ 构造 `new_meta`（`:853`）→ 扫描链收集 `live_rows`（`:876-914`，含 `pointed`）→ **逐行**改写循环（`:916-1024`）：DELETE 腿（`:945-976`）→ INSERT 腿（`:978-1012`）→ `record_version` + WAL → 提交点由 `apply_column_drop`（`:1050`）窄化目录。
  - 数据流与状态变化：`old.data_page_tail`（活表尾游标）与 `new_meta.data_page_tail`（副本尾游标）**两份**；物理数据页链由页头 `next_page_id` 串起（`SlottedPageRef::header().next_page_id`）；目录尾指针是数据链的**既有**目录字段（`catalog.update_table_tail`）。
  - 错误路径：`write_tuple_to_data_page` 在新页放不下时返回 `StorageError::PageFull`（并释放本次分配的溢出链）；`heal_stale_tail` 在页越界 / 链成环时返回 `StorageError::InvalidPageId` / `StorageError::Internal`。
  - 外部影响：链分叉 ⇒ 改写副本落在不可达页 ⇒ 已提交行丢失（ISS22）；被放弃的改写 ⇒ 已改写行不可见（ISS24）；四处 `pointed` 集合影响改写选行、活行判定、索引回填与恢复重建。

**Implementation Guidance**

- 建议顺序：**T4（RED）→ T5（修 ISS22）→ T6（RED + 判别）→ T7（修 M2 / 闭合 ISS24）**。T5 先落地，可顺带让 T6 的判别少一个候选层。
- **T5 的形态（`design.md` D4）**：DELETE 腿与 INSERT 腿必须经由**同一条会自愈并回写目录尾指针的写入口**（`TableManager::write_tuple` 的既有语义）。两种等价形态由 Act 选：
  - (a) DELETE 腿改为 `self.write_tuple(&old, &tombstone_header, &[])`——与 INSERT 腿走同一入口；`heal_stale_tail` 让两腿各自看到物理链的真实尾 ⇒ 不会各自分配并覆盖同一个 `next`。
  - (b) DELETE 腿与 INSERT 腿都用 `new_meta` 作写入口的 meta ⇒ 两条腿共用**同一个 `Mutex`**（连自愈都不需要）。
  - 两种形态都必须满足：① 同一尾游标；② 自愈陈旧尾；③ 回写目录尾指针。**不得**用「两条游标 + 显式同步尾指针」（D4 已否决：仍保留「页写满时各自分配」的窗口）。
  - 实现时必须核对：`catalog.update_table_tail`（`:2026-2029`）**只**写尾指针字段、**不**触及目录**列**定义（列的窄化仍在提交点由 `apply_column_drop` 生效，`ddl-catalog-transactional` R1/R2 不变）。
- **T6 的判别协议（`design.md` D5，硬性步骤，不是可选项）**：RED 用例必须**同时**给出三个判别读数，缺一不可：
  1. **缺失行的链可达性（M1 读数）**：缺失行的改写副本所在页是否在数据页链上（链可达 ⇒ M1 不成立；不可达 ⇒ M1 成立）。
  2. **缺失行版本链上的 aborted 指向者（M2 读数）**：缺失行的版本链上是否存在 `create_tx_id == 0` 的指向者（存在 ⇒ M2 成立）。
  3. **缺失行究竟在哪一层消失（本 Plan 新增的判别读数，见 Current-State Evidence 的四点）**：全表扫描（数据面版本求值）/ 点查与索引扫描（索引条目）/ 计划期活行判定（`table_has_live_rows`）/ 中止清理是否成功中性化了墓碑（`create_tx_id` 是否已为 0）。**这一读数不可省略**——按当前代码，扫描路径的版本求值已经正确处理 aborted 指向者，因此若不先定位层次就动手修，很可能修在错误的面上而症状「看起来通过」但缺陷仍在。
- **T7 的修复形态（D5 决策 3）**：抑制集合的构造只承认「指向者**未被回滚**」的链接——以指向者的 `create_tx_id == 0` 为准，与 `superseder_suppresses` 的既有判据同源。修复面限于四处 `pointed` 构造点（`table_manager.rs` 的 `alter_drop_column` / `table_has_live_rows`、`executor/create_index.rs` 的回填、`wal/recovery.rs` 的 `rebuild_pk_indexes`）——**M2 无论是否为本次实测症状都要修**（它是对已验收 `mvcc-tombstone-visibility` R1 的违例）。**不改** `VersionHeader::mark_aborted` 的「保留链指针」语义。
- **规模约束（`design.md` D3，change 级）**：本 change 内不使用 **≥ 300 行的表规模**构造用例。ISS22 用 **N ≈ 40**（登记现场 40 → 25 的容量边界），ISS24 用 **N = 30**（登记现场 30 → 29）。两者都在约束内。
- 观察口径：全部落在既有可观察量上——行数（`SELECT COUNT(*)` / `SELECT id FROM t`）、数据页链可达性（沿 `SlottedPageRef::header().next_page_id` 走链）、页归属（`page_count()` 与链上页集合的比对）、版本头（`VersionHeader::from_bytes` 的 `create_tx_id` / `next_version`）、目录（`catalog().scan_columns`）。**不新增指标**；判定只在 `cargo test` 集合内；**不用 timer 阈值作行为判据**（放弃夹具的 `ANCHOR_TIMEOUT` 只作夹具失控兜底，与既有先例同）。

**Task Definitions**

### T4: 跨页容量边界的删列改写不丢行、数据页链不分叉

- Requirement/Scenario: DRC R1（列改写不丢失已提交行）·场景「跨页容量边界的删列改写后行数不减」「改写后的数据页链不分叉（无不可达页）」。
- Depends on: None
- Targets: `tests/ddl_rewrite_consistency_test.rs`（新增）；`src/storage/data/table_manager.rs::TableManager::alter_drop_column`（作为被测对象，不在本任务修改）。
- Current behavior: 改写的 DELETE 腿（`:949-950`）用 `old.data_page_tail`、INSERT 腿（`:992`）用 `new_meta.data_page_tail`（`:864` 拷贝出的独立 `Mutex`）；两条腿写进同一条物理链，页写满时各自分配新页并**覆盖同一个 `next` 指针** ⇒ 先分配的页成为链上不可达的孤儿 ⇒ 写在其中的已提交行丢失（登记实测 40 行剩 25）。
- Required behavior: 改写后**已提交行数不减**（改写前行数 == 改写后行数）；改写后的数据页链**不分叉**——链上可达的页集合包含改写期间分配的全部新页，且不存在「被 `next` 覆盖而掉出链」的页；目录**列**的窄化仍在提交点生效。
- Preserve: `ddl-catalog-transactional` R1/R2（目录列定义的窄化时机与列数语义）；既有 `ALTER TABLE DROP COLUMN` 的成功/失败返回语义；数据页的槽位布局与 `VersionHeader` 布局（**不改磁盘格式**）；`FileStorage` 的释放集合语义；IPO R1/R2/R3（页归属唯一性、插入下降必然终止）。
- Forbidden: 不得改磁盘格式（`VersionHeader` 布局、目录行布局——会破坏 MS36 已验收的 G1/G2/G3 兼容面）；不得改 `FileStorage::free_page` / `complete_free_release` / `promote_free_releases` / `discard_free_releases` 的集合语义；不得把「两条游标 + 显式同步尾指针」当作修复（D4 已否决）；不得在测试框架外建判定脚本或封装命令；不得引入身份证明机制（run-id / 指纹 / revision pin）；不得用 timer 阈值作行为判据；不得用 ≥ 300 行的表规模构造用例。
- First test: `tests/ddl_rewrite_consistency_test.rs`（新增，RED）——用例名 `drop_column_across_page_boundary_keeps_all_rows`（计划名；Act 可改名并记录实际名）。位置：真实文件库 + 真实 `Database::open`；动作：建表 → 写入 **N ≈ 40** 行（跨过数据页容量边界）→ `ALTER TABLE t DROP COLUMN v`；观察：改写后的行数与改写前相等，且数据页链上可达的页集合覆盖改写期间新分配的页（无孤儿页）。命令：`cargo test --test ddl_rewrite_consistency_test -- --test-threads=1`。预期（修复前）：行数断言**失败**（登记形态 40 → 25）或链可达性断言失败。
- GREEN condition: 上述两条断言在修复后全部通过；既有 `alter_table_test` / `ddl_catalog_transactional_test` 通过。
- Verification: `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` 退出码 0、failed 计数 0；`cargo test --test alter_table_test -- --test-threads=1`、`cargo test --test ddl_catalog_transactional_test -- --test-threads=1` 退出码 0、failed 计数 0（既有断言不得被弱化）。失败含义：T4 的 RED 未复现 ⇒ 触发条件不足以跨过容量边界，须调整用例规模/形态（**不**放宽断言）；既有用例失败 ⇒ 修复越界。
- Stop when: 跨页容量边界在 **< 300 行**的表规模下无法构造（页容量边界与行宽强相关）⇒ 回到 Plan 复核触发条件与规模口径；或修复需要改动磁盘格式 / `FileStorage` 释放集合语义 ⇒ 回到 Plan。

### T5: 改写路径收敛到单一尾游标

- Requirement/Scenario: DRC R1 · 场景「跨页容量边界的删列改写后行数不减」「改写后的数据页链不分叉」。
- Depends on: T4
- Targets: `src/storage/data/table_manager.rs::TableManager::alter_drop_column`（`:945-950` 的 DELETE 腿写入口；`:853-865` 的 `new_meta` 尾游标）。
- Current behavior: DELETE 腿直接调用底层写入口 `write_tuple_to_data_page(&self.buffer_pool, &old, &tombstone, &[])`，该入口不读 `heal_stale_tail`、不回写目录尾指针；两条腿各持一条尾游标（`old.data_page_tail` 与 `new_meta.data_page_tail`）⇒ 页写满时各自分配新页并覆盖同一 `next` ⇒ 孤儿页 ⇒ 丢行。
- Required behavior: DELETE 腿与 INSERT 腿经由**同一条会自愈并回写目录尾指针的写入口**（`TableManager::write_tuple`），使两腿看到同一条链的同一个尾；改写后行数不减、数据页链不分叉；目录**列**的窄化仍在提交点生效（`apply_column_drop`），目录**尾指针**的回写时机调整为「每条改写写入都回写」（D4 已批准的数据链既有字段变化）。
- Preserve: `ddl-catalog-transactional` R1/R2（目录列定义面不变）；`heal_stale_tail` 的既有语义（沿物理链自愈、页越界/链成环时显式报错）；`TableManager::write_tuple` 对既有写路径（insert/update/upsert/delete）的行为不变；墓碑的槽位内容与 `next_version` 指向不变；WAL 记录序列不变（`Delete` / `Insert` 的 tx_id、table_name、row_id、tuple_data）。
- Forbidden: 不得把修复点放在 `FileStorage` 或 `data_page.rs` 的集合/游标语义上；不得改 `VersionHeader::mark_aborted`；不得改磁盘格式；不得顺手重构改写循环的其它逻辑（如索引条目重建、WAL 写入、列值序列化）；不得把目录**列**的窄化时机提前到改写开始。
- First test: 复用 T4 的用例（变更前已 RED）。命令：`cargo test --test ddl_rewrite_consistency_test -- --test-threads=1`。预期（修复前）：T4 的行数 / 链可达性断言失败。
- GREEN condition: T4 全部用例通过；`cargo test --lib` 通过（改写路径在 `table_manager.rs` 内有单元测试）；既有 `alter_table_test` / `ddl_catalog_transactional_test` / `recovery_tail_persistence_test` 通过。
- Verification: `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` + `cargo test --test alter_table_test -- --test-threads=1` + `cargo test --test ddl_catalog_transactional_test -- --test-threads=1` + `cargo test --test recovery_tail_persistence_test -- --test-threads=1` + `cargo test --lib -- --test-threads=1`，各退出码 0、failed 计数 0。失败含义：改写路径的行数、目录面或恢复期的尾指针语义被改变。
- Stop when: 单一尾游标导致某条既有路径（恢复期链重建、既有 DML 写路径）行为改变且无法在本任务范围内修复 ⇒ 回到 Plan；或修复需要改动磁盘格式 ⇒ 回到 Plan。

### T6: 被放弃的删列改写不隐藏已改写的行——RED 用例与 M1/M2 判别

- Requirement/Scenario: DRC R2（被放弃的列改写不隐藏已改写的行）·场景「放弃的删列改写在回收后，已改写的行仍可见」「放弃后重开库，目录与数据面一致」；MTV R1（墓碑自描述且不影响前驱版本）·「已回滚墓碑不抑制其前驱版本」；DCT R3（放弃 DDL 之后重开库目录与数据面一致）。
- Depends on: T5
- Targets: `tests/ddl_rewrite_consistency_test.rs`（新增用例）；作为被测对象：`src/storage/data/table_manager.rs::TableManager::alter_drop_column`、`src/transaction/manager.rs::TransactionManager::abort_cleanup_versions`、`src/executor/data_scan.rs` 的扫描求值面、索引条目面、`src/storage/data/table_manager.rs::TableManager::table_has_live_rows`（本任务**不**修改它们）。
- Current behavior: 被放弃（语句 future 被丢弃 / 超时）的 `ALTER TABLE DROP COLUMN` 在被回收后，让它**已开始改写的那一行**从扫描结果中消失（登记实测 30 → 29、2000 → 1999，缺失的是 `id=2`）。根因在登记时为 **Inferred**：`design.md` D5 给出两个候选——M1（链分叉，与 ISS22 同一根因：该行落在链上不可达的页）与 M2（已回滚的墓碑仍抑制其前驱版本：四处 `pointed` 集合不识别 aborted 标记）。本 Plan 的独立复核追加了第三类读数：按当前代码，**扫描路径的版本求值已经正确处理 aborted 指向者**（`data_scan.rs:293` `slot_is_superseded` → `:339` `superseder_suppresses` → `create_tx_id == 0` 不抑制），因此缺失行究竟落在哪一层（链可达性 / 索引条目 / 计划期活行判定 / 墓碑中性化）**必须由 RED 钉住**。
- Required behavior: 被放弃的改写在被回收后**不隐藏**它已开始改写的行（该行在数据面仍可见、点查仍命中）；**判别步骤完成**：RED 用例给出 M1 读数（缺失行的改写副本是否在链上可达）、M2 读数（其版本链上是否存在 `create_tx_id == 0` 的指向者）与层次读数（缺失行在全表扫描 / 点查与索引扫描 / 计划期活行判定 / 墓碑中性化这四层中的哪一层消失），并在 Act Response 写明判别结论与依据。
- Preserve: `mvcc-tombstone-visibility` R1 的既有语义（已提交墓碑抑制整条链、未提交/已回滚墓碑不抑制）；`ddl-catalog-transactional` R1/R2/R3（目录列窄化在提交点、放弃后目录面不变）；MS34 已验收的库属隐式事务回收语义（`database.rs:585` 回收路径、`transaction/manager.rs:321` 中止清理）；放弃夹具的既有形态（`tests/ddl_catalog_transactional_test.rs:63-105`）。
- Forbidden: 不得把「判别步骤」降级为可选项；不得只改断言不放宽——「行仍可见」的断言强度不得低于登记现场（30 → 29 的规模下逐行可数）；不得用 ≥ 300 行的表规模构造用例；不得引入身份证明机制、判定层或 timer 阈值判据（放弃夹具的 `ANCHOR_TIMEOUT` 只作夹具失控兜底）；不得在判别结论确定前修改 `pointed` 集合或扫描求值面。
- First test: `tests/ddl_rewrite_consistency_test.rs`（新增，RED）——用例名 `abandoned_drop_column_keeps_rewritten_row_visible`（计划名；Act 可改名并记录实际名）与 `abandoned_drop_column_reopen_keeps_all_rows`（重开库维度）。位置：真实文件库 + 真实 `Database::open`；动作：建表 → 写入 **N = 30** 行 → 用既有放弃夹具形态丢弃 `ALTER TABLE t DROP COLUMN v` 的语句 future（锚点 = 活跃事务已记录版本数 ≥ 2）→ 触发回收（下一条写语句）→ 观察：改写过的行仍可见、行数不减；重开库后目录与数据面一致。命令：`cargo test --test ddl_rewrite_consistency_test -- --test-threads=1`。预期（修复前）：「行仍可见 / 行数不减」断言**失败**（登记形态 30 → 29）；同时 RED 必须输出上面三条判别读数。
- GREEN condition: 「行仍可见 / 行数不减 / 重开库一致」三条断言在修复后通过；Act Response 写明 M1 / M2 / 层次读数的判别结论（若判别为 M1，则由 T5 一并关闭；若判别为 M2 或其它层，由 T7 关闭）。
- Verification: `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` 退出码 0、failed 计数 0；`cargo test --test ddl_catalog_transactional_test -- --test-threads=1`、`cargo test --test mvcc_tombstone_visibility_test -- --test-threads=1`、`cargo test --test mvcc_abort_test -- --test-threads=1` 退出码 0、failed 计数 0。失败含义：放弃语义、墓碑可见性语义或目录面被改变。
- Stop when: 判别结论需要当前 Task Definition 之外的执行依据（例如必须新增故障注入设施才能稳定复现放弃窗口）⇒ 停止并退回 Plan（`replan-required`），在 Act Response 写明 Blocker Handoff；或缺失行在 < 300 行的规模下无法稳定复现 ⇒ 回到 Plan 复核夹具与规模口径。

### T7: 闭合抑制集合的活性规则（已回滚墓碑不抑制前驱版本）

- Requirement/Scenario: MTV R1（墓碑自描述且不影响前驱版本）·场景「已回滚墓碑不抑制其前驱版本」；DRC R2 ·「放弃的删列改写在回收后，已改写的行仍可见」；DCT R3 ·「放弃 DDL 之后重开库目录与数据面一致」。
- Depends on: T6
- Targets: `src/storage/data/table_manager.rs` 的 `TableManager::alter_drop_column`（`:877` / `:897-899` / `:914`）与 `TableManager::table_has_live_rows`（`:1074` / `:1095-1097` / `:1113`）；`src/executor/create_index.rs` 的索引回填（`:123` / `:143-145` / `:171`）；`src/wal/recovery.rs` 的 `rebuild_pk_indexes`（`:1166` / `:1186-1188` / `:1237`）。
- Current behavior: 四处以 `next_version` 链接构造抑制集合的活性规则**不识别 aborted 标记**：任何 `next_version` 目标都被当作「被指向的版本」，不区分指向者是否已回滚（`create_tx_id == 0`）⇒ 与已验收的 `mvcc-tombstone-visibility` R1（「未提交/已回滚墓碑 SHALL NOT 抑制、评估回溯前驱」）冲突，也与同源判据 `DataScanExecutor::superseder_suppresses`（`data_scan.rs:339`）不一致。
- Required behavior: 抑制集合只承认「指向者**未被回滚**」的链接——以指向者的 `create_tx_id == 0`（aborted 标记）为准，与 `superseder_suppresses` 同源；四处构造点行为一致；已回滚的墓碑**不**抑制其前驱版本；改写选行、活行判定、索引回填、恢复重建四条路径的语义随之闭合。**M2 无论是否为本次实测症状都要修**。
- Preserve: `VersionHeader::mark_aborted` 的「保留链指针」语义（`version_chain.rs:81`）不变；已提交墓碑**仍然**抑制整条版本链（`mvcc-tombstone-visibility` R1 的既有行为）；`DataScanExecutor::superseder_suppresses` 本身不变（本 Cycle 只是让另外四处与它同源）；恢复期的索引重建语义（`redo_count > 0` 索引不信任重建通道）不变；磁盘格式不变。
- Forbidden: 不得改 `VersionHeader::mark_aborted` 的链指针语义；不得改 `DataScanExecutor::superseder_suppresses` 的既有判据；不得把这四处构造点重构成新的公共抽象（超出本 Cycle 的最小面）；不得顺手改 `SlottedPageRef::get_slot_data` 的签名（Iteration 002 的范围外候选）；不得引入身份证明机制、判定层或 timer 阈值判据。
- First test: 复用 T6 的用例（变更前已 RED；若 T6 判别为 M1，则本任务的判别读数是 M2 的**独立**证据——M2 仍须修，其 RED 用例须另行构造：以 `create_tx_id == 0` 的指向者构造抑制集合被错误承认的场景，断言前驱版本不被抑制）。命令：`cargo test --test ddl_rewrite_consistency_test -- --test-threads=1`。预期（修复前）：抑制集合相关断言失败。
- GREEN condition: T6 用例通过；抑制集合的活性规则与 `superseder_suppresses` 同源的断言通过；`cargo test --lib` 通过；既有 `mvcc_tombstone_visibility_test` / `mvcc_abort_test` / `index_manager_test` / `secondary_index_test` / `recovery_test` / `recovery_e2e_test` 通过。
- Verification: `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` + `cargo test --test mvcc_tombstone_visibility_test -- --test-threads=1` + `cargo test --test mvcc_abort_test -- --test-threads=1` + `cargo test --test index_manager_test -- --test-threads=1` + `cargo test --test secondary_index_test -- --test-threads=1` + `cargo test --test recovery_test -- --test-threads=1` + `cargo test --test recovery_e2e_test -- --test-threads=1` + `cargo test --lib -- --test-threads=1`，各退出码 0、failed 计数 0。失败含义：墓碑可见性语义、索引回填或恢复重建的语义被改变。
- Stop when: 修复需要改动 `VersionHeader` 的磁盘格式或 `mark_aborted` 的链指针语义 ⇒ 回到 Plan；或判别结论表明缺失行的消失层既非 M1 也非 M2（且不是上述四层中的任何一层）⇒ 停止并退回 Plan（`replan-required`），在 Act Response 写明 Blocker Handoff 与已排除的层。

**Cycle RTM**

| Requirement / Scenario | Task | First Test（计划名） | Status |
|---|---|---|---|
| DRC R1 列改写不丢失已提交行 · 跨页容量边界后行数不减 | T4, T5 | `tests/ddl_rewrite_consistency_test.rs::drop_column_across_page_boundary_keeps_all_rows` | Covered |
| DRC R1 · 改写后的数据页链不分叉（无不可达页） | T4, T5 | 同上（同一用例的链可达性断言） | Covered |
| DRC R2 被放弃的列改写不隐藏已改写的行 | T6, T7 | `tests/ddl_rewrite_consistency_test.rs::abandoned_drop_column_keeps_rewritten_row_visible` | Covered |
| MTV R1（既有域）已回滚墓碑不抑制其前驱版本 | T6, T7 | 同上（M2 判别断言） | Covered |
| DCT R3（既有域）放弃 DDL 之后重开库目录与数据面一致 | T6, T7 | `tests/ddl_rewrite_consistency_test.rs::abandoned_drop_column_reopen_keeps_all_rows` | Covered |
| （Cycle 内新增）判别步骤本身（M1 / M2 / 层次三组读数） | T6 | 同上（RED 必须输出三组读数） | Covered |

**Invariants**

- **不改磁盘格式**：`VersionHeader` 布局、目录行布局、槽位布局一律不变（保护 MS36 已验收的 `legacy-format-compat` G1/G2/G3 支持面）。
- `FileStorage` 的释放集合语义不变（`free_page` / `complete_free_release` / `promote_free_releases` / `discard_free_releases` / `pending_frees` / `free_pages`）——本 Cycle 的两腿收口在 B-Tree/表元数据层完成。
- 目录**列**定义的窄化仍在提交点生效（`ddl-catalog-transactional` R1/R2）；目录**尾指针**是数据链的既有字段，其回写时机按 D4 调整为「每条改写写入都回写」（数据面，不触及列定义面）。
- `VersionHeader::mark_aborted` 的「保留链指针」语义不变；已提交墓碑仍然抑制整条版本链。
- Iteration 000 已验收的 IPO R1/R2/R3 保持成立（页归属唯一性、插入下降必然终止）；本 Cycle 不得使 `src/storage/btree/btree.rs` 的既有断言回退。
- 既有 DML 写路径（insert / update / delete / upsert）共用活表 meta 的单游标形态不变。
- 不引入身份证明机制；不建测试框架外的判定层；不用 timer 阈值作行为判据；任何需要串行语义的用例按 `--test-threads=1` 执行。
- 工作区既有未提交改动不回滚、不清理、不提交。

**Non-goals**

- 释放路径的有界读（Iteration 002：B-4）。
- 改写窗口的在途窄化状态与读路径显式拒绝（Iteration 003：ISS25）——本 Cycle 的读路径语义保持现状。
- 释放可分配判定的耐久性前提（Iteration 004：B-1）。
- 旧格式兼容的证据补齐（Iteration 005：C-1 / C-2 / C-3）。
- B-3（已被已验收规格显式界定为范围外既有行为）。
- `VersionHeader::mark_aborted` 的链指针语义；`DataScanExecutor::superseder_suppresses` 的既有判据；`SlottedPageRef::get_slot_data` 的签名；磁盘格式；`FileStorage` 的释放集合语义；四处 `pointed` 构造点的**结构性重构**（只改活性规则，不重构）。
- ISS22 / ISS24 之外的新缺陷面。

**Acceptance**

1. **DRC R1**：跨页容量边界的 `ALTER TABLE DROP COLUMN` 后**已提交行数不减**（改写前行数 == 改写后行数），且改写后的数据页链**不分叉**（链上可达的页集合覆盖改写期间新分配的页，无被 `next` 覆盖而掉出链的页）。证据：T4 的 RED→GREEN 用例 + T5 的实现 + `design.md` D4。
2. **DRC R2**：被放弃（丢弃或超时）的删列改写在被回收后**不隐藏**它已开始改写的行——该行在全表扫描与点查中均可见、行数不减。证据：T6 的 RED→GREEN 用例 + T7 的实现。
3. **MTV R1（既有域的实现面）**：以 `next_version` 构造抑制集合的活性规则**承认 aborted 标记**（指向者 `create_tx_id == 0` 时不承认该链接），四处构造点与 `DataScanExecutor::superseder_suppresses` 同源；已提交墓碑仍抑制整条版本链。证据：T7 的用例 + T6 的判别读数。
4. **DCT R3（既有域）**：放弃 DDL 之后重开库，目录（列定义与列数）与数据面（行数与行宽）一致。证据：T6 的 `abandoned_drop_column_reopen_keeps_all_rows`。
5. **判别步骤已执行**：T6 的 RED 输出 M1 / M2 / 层次三组读数，Act Response 写明判别结论与依据；**M2 已修**（无论其是否为实测症状）。
6. **既有断言零回退**：`alter_table_test` / `ddl_catalog_transactional_test` / `mvcc_tombstone_visibility_test` / `mvcc_abort_test` / `index_manager_test` / `secondary_index_test` / `recovery_test` / `recovery_e2e_test` / `recovery_tail_persistence_test` / `cargo test --lib` 全部退出码 0、failed 计数 0。

**Verification**

| Scenario | 判定方式 | 命令 / 步骤 | 通过依据 |
|---|---|---|---|
| 跨页容量边界后行数不减、数据页链不分叉 | 真实入口（真实文件库 + 真实 `Database::open` + 真实 `ALTER TABLE DROP COLUMN`，N ≈ 40） | `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` | 退出码 0、failed 0 |
| 放弃的改写不隐藏已改写的行 + 三组判别读数 | 真实入口（真实库 + 既有放弃夹具形态，N = 30） | 同上（同一文件内另一用例） | 退出码 0、failed 0；RED 阶段输出 M1 / M2 / 层次读数 |
| 放弃后重开库目录与数据面一致 | 真实入口（真实库 + 真实重开） | 同上 | 退出码 0、failed 0 |
| 已回滚墓碑不抑制其前驱版本 | 真实入口 + 版本头观察（`VersionHeader::from_bytes`） | 同上 | 退出码 0、failed 0 |
| 既有行为零回退 | 真实入口（既有测试文件） | `cargo test --test alter_table_test -- --test-threads=1` 等 9 个目标 + `cargo test --lib -- --test-threads=1` | 各退出码 0、failed 0 |

**Gate 2 Readiness**

PASS——需求与 BDD 覆盖：ISS22（完成路径丢行）与 ISS24（放弃路径行不可见）各自的触发面已定位，BDD 场景已扫（Happy / Sad / Edge / 组合：跨页边界 × 放弃窗口 × 重开库），场景草图见 T4 / T6 的 First test；实现调查已完成（入口、目标符号、调用者/被调用者、数据流、状态变化、错误与并发边界、既有测试入口均已记入 Investigation Facts）；设计闭合（`design.md` D4 / D5 给出修复形态、判别协议与被否决的替代方案）；本 Cycle 的 tasks / specs / design 一致（DRC R1/R2、MTV R1、DCT R3 均在 RTM 中有映射）；验证方式已定（真实入口 + `cargo test` 集合内判定，无身份证明机制、无判定层、无 timer 阈值判据）；`Persisted Evidence` 模式已定；**用户批准计划**（goal 指令原文，见 `proposal.md` 的「授权与批准依据」）。

BLOCKED / SKIPPED：**None**。ISS24 的**根因**未确定，但按 D5 的协议，判别是 T6 的**执行步骤**（有明确的执行依据与停止条件），**不是**阻塞执行的实质未知项。

**Persisted Evidence**

- Mode: none

理由：本 Cycle 的验收全部落在 `cargo test` 集合内的定向用例上（命令、实际结果、修改文件与符号写入 Act Response 即足够）；判别读数是 RED 用例的输出，随用例固化在测试代码中，不需要归档；不存在「Act Response 装不下」「无法低成本重跑」「缺了就无法作决定」的情形。**Plan 不创建 `evidence/` 目录。**

**Risks and Notes**

- **ISS24 的根因仍未确定**（非实质未知项，已在 T6 的 Task Definition 中给出判别协议、三个判别读数与停止条件）：Act 必须**先判别再修**；M2 无论如何都要修（它是对已验收 `mvcc-tombstone-visibility` R1 的违例）。若判别结论落在四层之外，T7 的 Stop when 要求退回 Plan。
- **Plan 的独立复核结论（已写入执行依据）**：按当前代码，扫描路径的版本求值**已经**正确处理 aborted 指向者（`data_scan.rs:293` → `:339`），因此缺失行**不应**在扫描求值层消失。T6 的层次读数（链可达性 / 索引条目 / 计划期活行判定 / 墓碑中性化）是据此追加的——避免修复打在错误的面上而症状「看起来通过」。
- **T5 的目录副作用**：DELETE 腿改走 `TableManager::write_tuple` 后，改写期间的目录尾指针回写频率上升（D4 已批准：数据链既有字段，非列定义面）。Act 必须在实现时核对 `catalog.update_table_tail` 只写尾指针字段；若发现它会触及列定义面，停止并退回 Plan。
- **规模约束**：change 级约束（< 300 行的表规模）继续有效；ISS22 用 N ≈ 40、ISS24 用 N = 30，均在约束内且与登记现场同量级。
- **验证预算**：本 Iteration **不跑全量回归**（收尾全量在 Iteration 005，全 change 一次预算）。MS35 / MS36 的全量终态 2192/0/2 的覆盖范围已被 Iteration 000 的改动改变，**不得**再作为「当前工作区全量通过」的依据；本 Iteration 只需重跑 `cargo test --lib`（改了 `table_manager.rs` 等 ⇒ Iteration 000 的 420 条 lib 结论对本次改动失效）。
- **放弃夹具的既有先例**：`tests/ddl_catalog_transactional_test.rs:63-105` 的 `abandon_statement` / `anchor_tx` 可直接复用（锚点 = 活跃事务已记录版本数 ≥ 2；`yield_now` 轮询；`ANCHOR_TIMEOUT` 只作夹具失控兜底）。T6 不得引入第二套放弃机制。
- 若 Act 发现必须提前跑全量才能判定，须在 Act Response 记录理由并回报 Plan 追加预算。

## Act Response

- Status: reported

### 任务完成清单

| Task | 状态 | 涉及文件 / 符号 |
|---|---|---|
| T4 | done | `tests/ddl_rewrite_consistency_test.rs`（新增）：`drop_column_across_page_boundary_keeps_every_row_on_one_chain`（N=40） |
| T5 | done | `src/storage/data/table_manager.rs::TableManager::alter_drop_column`：DELETE 腿与 INSERT 腿统一经 `TableManager::write_tuple(&old, …)`；改写结束后同步 `new_meta.data_page_tail` |
| T6 | done | `tests/ddl_rewrite_consistency_test.rs`（新增）：`abandoned_drop_column_keeps_rewritten_row_visible`、`abandoned_drop_column_reopen_keeps_all_rows`、`abandoned_drop_column_before_version_registration_keeps_row_visible`、`abandoned_drop_column_before_narrow_copy_registration_keeps_row_count`（含三组判别读数输出） |
| T7 | done | `src/storage/data/table_manager.rs`（`alter_drop_column` / `table_has_live_rows` 的 `pointed`）、`src/executor/create_index.rs`（`backfill_tree` 的 `pointed`）、`src/wal/recovery.rs`（`rebuild_pk_indexes` 的 `pointed` + 链尾起点规则）、`src/transaction/manager.rs`（`abort_cleanup_versions` + 新增 `versions_created_by`） |

### 实际改动（含文件与符号）

1. **`src/storage/data/table_manager.rs::TableManager::alter_drop_column`**
   - **ISS22（链分叉）**：DELETE 腿由 `write_tuple_to_data_page(&self.buffer_pool, &old, …)` 改为 `self.write_tuple(&old, …)`，INSERT 腿由 `self.write_tuple(&new_meta, …)` 改为 `self.write_tuple(&old, …)` ⇒ 两腿共用**一条尾游标**、同一条自愈（`heal_stale_tail`）与目录尾指针回写路径。改写结束后把 `*new_meta.data_page_tail` 同步为 `*old.data_page_tail`（`apply_column_drop` 在提交点把 `new_meta` 装成活表 meta，尾游标必须指向真实链尾）。
   - **ISS24（放弃窗口）**：DELETE 腿的墓碑改为「**先写（不带链指针）→ 登记版本 → 补链指针 → 再清理索引条目**」。墓碑槽的**最终内容**（create_tx / 删除哨兵 / `next_version`）与 DeleteExecutor 形态逐字节同形。
   - **M2**：`pointed` 抑制集合只承认「指向者未被回滚」（`create_tx_id != 0`）的链接。
2. **`src/storage/data/table_manager.rs::TableManager::table_has_live_rows`**：`pointed` 同一活性规则（计划期活行判定）。
3. **`src/executor/create_index.rs::CreateIndexExecutor::backfill_tree`**：`pointed` 同一活性规则（索引回填）。
4. **`src/wal/recovery.rs::RecoveryManager::rebuild_pk_indexes`**：`pointed` 同一活性规则（恢复重建）；并新增「链尾回溯**不以已回滚版本为起点**」的伴随规则（见「自检发现」第 1 条）。
5. **`src/transaction/manager.rs::TransactionManager::abort_cleanup_versions` + 新增自由函数 `versions_created_by`**：中止清理在登记版本之外，按**创建者事务 id**沿数据页链收回本事务创建但**未被登记**的版本（链成环显式报错，不无限走查）。
6. **`tests/ddl_rewrite_consistency_test.rs`（新增，6 个用例）**：共用夹具（真实文件库 + 真实 `Database::open` + 真实 `ALTER TABLE DROP COLUMN`；放弃夹具沿用 `tests/ddl_catalog_transactional_test.rs:63-105` 的形态：锚点 + `yield_now` 轮询 + `drop(statement)`；`ANCHOR_TIMEOUT` 只作夹具失控兜底）。

### ISS24 的判别结论（D5 协议：先判别再修）

判别由 T6 的 RED 用例输出（三组读数随用例固化在测试代码中）：

- **M1 读数（链可达性）**：缺失行（被放弃的改写已触及的行）的槽位所在页**在数据页链上可达**（`touched_pages_on_chain = true`；N=30 的放弃窗口下链未分叉）⇒ **M1 不成立**。
- **M2 读数（缺失行版本链上的 aborted 指向者）**：缺失行（`id=2`）的版本链上唯一的指向者 `create_tx_id = 31`（≠ 0）⇒ 缺失行上**不存在**已回滚指向者 ⇒ **M2 不是缺失行的成因**。（M2 在**另一行**——被完整改写、其墓碑已被中和的行——上成立，见 T7 用例。）
- **层次读数**：缺失行消失于**第 4 层（墓碑中性化）**——该墓碑的 `create_tx_id` 仍是被放弃事务的 id（**未被中和**）；回收把该事务移出活跃集后，扫描的 `superseder_suppresses` 走「已提交删除」分支 ⇒ 压制其前驱版本 ⇒ 该行不可见。**同源损伤**：该行的主键条目已被 DELETE 腿删除且未被还原（第 2 层）。
- **实测根因**：放弃窗口落在「墓碑写入 + 主键条目删除」与「向事务管理器登记该版本」之间——墓碑已在链上（按删除者事务状态压制前驱版本），而事务管理器看不到它 ⇒ 回收无法中和它。
- **M1 / M2 分量**：本次实测症状 = 第 4 层（**M1 0%、M2 0%**（作为缺失行成因））；**M2 作为独立缺陷 100% 成立**（已回滚墓碑仍抑制前驱版本，影响改写选行 / 计划期活行判定 / 索引回填 / 恢复重建四条路径）——按 D5 决策 3 无论是否为实测症状都修 ✓。
- **超出 Plan 的 M1/M2 二分的补充判别**（T7 的 Stop when 不触发：缺失行落在四层之内）：另外两个更窄的放弃窗口被实测确认并一并关闭——(a) 放弃落在「写出版本槽」与「登记版本」之间 ⇒ 该墓碑**未被登记**，回收完全看不到该表 ⇒ 缺失行 + 主键条目丢失（RED：`missing ids: [1]`）；(b) 放弃落在窄化副本写入与登记之间 ⇒ 窄化副本既不被中和也不被提交 ⇒ 无快照扫描把它当作额外的可见行（RED：30 → 31，可见 `id=1` 重复）。

### AR1 / AR2 的用例与判据

- **AR1（改写不丢行 / 数据页链不分叉）** — 用例 `drop_column_across_page_boundary_keeps_every_row_on_one_chain`（N=40，真实文件库 + 真实 `ALTER TABLE t DROP COLUMN v`）：
  1. `SELECT * FROM t` 的行数 == 改写前（40 == 40）；
  2. `SELECT id FROM t` 的 id 集合逐行相等；
  3. 主键索引指向的行所在页**全部**在数据页链上（无行落在孤儿页）；
  4. 库内首字节 `0x03` 的数据页（排除目录面两条页链）**全部**在链上——无「被 `next` 覆盖而掉出链」的页。
- **AR2（被放弃的 DDL 不隐藏已改写的行）** — 用例 `abandoned_drop_column_keeps_rewritten_row_visible`（N=30）：(a) `SELECT id FROM t` **逐行可数**：30 行且 id 集合完整（缺失即失败）；(b) `SELECT * FROM t WHERE id = ?` 对 1..=30 每行**命中 1 行**；(c) 被放弃的改写留下的每一个墓碑**已被中和**（`create_tx_id == 0`）。用例 `abandoned_drop_column_reopen_keeps_all_rows`（DCT R3）：重开库后目录列定义 `["id","v","w"]`、列数 `3`、行数 `30`、每行行宽 `3`。用例 `abandoned_drop_column_before_version_registration_keeps_row_visible` / `…_before_narrow_copy_registration_keeps_row_count`：两个更窄的放弃窗口下同样逐行可数且点查命中。

### RED → GREEN 证据（实际命令与输出要点）

命令：`cargo test --test ddl_rewrite_consistency_test -- --test-threads=1`

| 用例 | RED（修复前） | GREEN（修复后） |
|---|---|---|
| `drop_column_across_page_boundary_keeps_every_row_on_one_chain` | `completed DROP COLUMN must preserve every row; got 25 of 40`（left: 25, right: 40）——与 ISS22 登记的 40 → 25 同形；页级普查另见孤儿页 `{4}`（链 `{2,5}`，全部数据页 `{2,4,5}`） | `test result: ok. 6 passed; 0 failed` |
| `abandoned_drop_column_keeps_rewritten_row_visible` | `the abandoned rewrite must not hide the row it started rewriting; missing ids: [2] (visible: [1, 3, …, 30])`——与 ISS24 登记的 30 → 29、缺失 `id=2` 同形 | 同上 |
| `abandoned_drop_column_reopen_keeps_all_rows` | `the abandoned DDL must not change the visible row count, got Shape { columns: ["id","v","w"], column_count: 3, row_count: 29, widths: {3} }` | 同上 |
| `abandoned_drop_column_before_version_registration_keeps_row_visible` | `missing ids: [1] (visible: [2, …, 30])` | 同上 |
| `abandoned_drop_column_before_narrow_copy_registration_keeps_row_count` | `an abandoned rewrite must not leave a visible narrow copy behind; visible: [1, …, 30, 1]`（left: 31, right: 30） | 同上 |
| `rolled_back_rewrite_does_not_suppress_predecessor_for_later_rewrite` | `a rolled-back rewrite must not suppress its predecessor version for later rewrites; rows keeping the stale width: [(1, 10), (2, 20)]`（M2 的独立证据：层 4 修复后仍在） | 同上 |

RED 的采集方式：临时把对应修复（单一尾游标 / 墓碑登记次序 / 链扫描）还原为修复前形态后运行同一批用例，随后恢复；恢复后 6 个用例全绿。

### 验证命令与实际结果

- `cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` → `test result: ok. 6 passed; 0 failed; 0 ignored`
- `cargo test --lib -- --test-threads=1` → `test result: ok. 420 passed; 0 failed; 0 ignored`（**420 基线仍成立**；本 Cycle 改了 `table_manager.rs` / `transaction/manager.rs` / `create_index.rs` / `recovery.rs` ⇒ Iteration 000 的 420 条结论已失效并重跑）
- 定向批次 1（6 目标）：`alter_table_test` 16/0、`ddl_catalog_transactional_test` 3/0、`ddl_rewrite_consistency_test` 6/0、`mvcc_abort_test` 3/0、`mvcc_tombstone_visibility_test` 11/0、`recovery_tail_persistence_test` 4/0
- 定向批次 2（10 目标）：`index_manager_test` 3/0、`secondary_index_test` 27/0、`secondary_index_equality_test` 3/0、`recovery_test` 3/0、`recovery_e2e_test` 6/0、`recovery_wal_convergence_test` 5/0、`recovery_replay_convergence_test` 3/0、`ddl_crash_recovery_test` 6/0、`table_manager_test` 6/0、`update_index_maintenance_test` 11/0
- 定向批次 3（12 目标）：`statement_cancellation_test` 4/0、`explicit_tx_test` 6/0、`tx_statement_test` 6/0、`isolation_level_test` 21/0、`dml_tx_id_test` 12/0、`mvcc_commit_test` 9/0、`mvcc_record_test` 4/0、`version_chain_test` 5/0、`truncate_test` 8/0、`drop_table_free_test` 7/0、`free_page_lifecycle_test` 21/0、`catalog_durability_test` 3/0
- 定向批次 4（13 目标，收尾复跑）：`alter_table_test` 16/0、`ddl_catalog_transactional_test` 3/0、`ddl_rewrite_consistency_test` 6/0、`mvcc_abort_test` 3/0、`mvcc_tombstone_visibility_test` 11/0、`recovery_tail_persistence_test` 4/0、`index_manager_test` 3/0、`secondary_index_test` 27/0、`recovery_test` 3/0、`recovery_e2e_test` 6/0、`ddl_crash_recovery_test` 6/0、`statement_cancellation_test` 4/0、`table_manager_test` 6/0
- **本 Iteration 不跑全量回归**（收尾全量归 Iteration 005，全 change 一次预算）。MS35 / MS36 的 2192/0/2 终态覆盖范围已失效，不作为当前工作区的依据。

### Gate 3 / Gate 4 结论

- **Gate 3（Task Review）**：T4 / T5 / T6 / T7 逐项 review 通过——需求与场景完整实现；`ddl-catalog-transactional` R1/R2/R3（目录列窄化仍在提交点）、`mvcc-tombstone-visibility` R1（已提交墓碑仍抑制整条链）、`VersionHeader::mark_aborted` 的「保留链指针」语义、磁盘格式、`FileStorage` 释放集合语义均未破坏；先行测试均为行为断言（无宽松断言、无判定层、无身份证明机制、无 timer 阈值判据）；diff 无计划外修改（`git diff` 中 `create_index.rs` / `recovery.rs` / `manager.rs` 的 `free-page-lifecycle` 与 `abort_by_id` 改动属**既有现场**，非本 Cycle 改动）。
- **Gate 4（Verified Completion）**：T4–T7 的 Verification 全部满足（各 Task Definition 列出的命令退出码 0、failed 计数 0），完成声明基于上述真实运行。T4–T7 标记完成。

### 自检发现、已修复内容与遗留 Minor 问题

1. **Important（已修）**：M2 的规则落到 `wal/recovery.rs` 后，恢复期的索引重建会对**同一存活版本**建两次条目（`index rebuild: table 't' duplicate PK across chains`）——捕获于 `mvcc_tombstone_visibility_test::uncommitted_delete_not_applied_after_restart`（重开库失败）。原因：M2 下「已回滚的指向者」不再把其前驱版本标为非链尾 ⇒ 前驱版本自身也是链尾，从已回滚版本起回溯会与前驱那条链重复命中同一存活版本。修法：链尾回溯**不以已回滚版本为起点**（已回滚版本不是存活版本——扫描跳过它、它也不抑制其前驱版本 ⇒ 其前驱版本自身即链尾）。
2. **Important（已修）**：`versions_created_by` 的链走查缺环保护（损坏库上可能无限走查）⇒ 加 `visited` 集合，链成环显式报错（与 `data_chain_status` / `heal_stale_tail` 同形态）。
3. **Minor（已修）**：测试文件的两个未读字段告警（`SlotView.commit_tx` / `Readings.touched`）⇒ 删除前者、后者并入断言消息。
4. **遗留 Minor（不阻塞，记录待 Plan Review 终审）**：(a) `abort_cleanup_versions` 现在对每个被中止事务的每张表做一次数据页链走查（O(链长)）——中止路径本就 O(版本数)，量级同源，但比原实现多一次全链扫描；(b) 恢复面的「已回滚版本不作链尾起点」与 `pointed` 的构造规则分列两处（未抽公共抽象——T7 禁止结构性重构）。

### Deviations from Plan

1. **T7 的修复面增加了 `src/transaction/manager.rs::abort_cleanup_versions`（新增 `versions_created_by`）**——依据：T6 的层次读数把缺失行钉在**第 4 层（墓碑中性化）**，该层不在 Plan 的 M1 / M2 候选内；D5 决策 1 要求「先判别再修」，T7 的 Stop when 只在「四层之外」才退回 Plan（第 4 层在四层之内 ⇒ 继续修）。不修该层，DRC R2 的「该行在全表扫描与点查中均可见、行数不减」无法达成。
2. **DELETE 腿的墓碑改为「先写（不带链指针）→ 登记 → 补链指针 → 再清理索引条目」**——T7 的 Targets 未列此项；依据：实测确认的更窄放弃窗口（放弃落在「写出版本槽」与「登记版本」之间）下，未登记的墓碑若已带链指针会压制其前驱版本 ⇒ 缺失行 + 主键条目丢失。墓碑槽的**最终内容与链指针不变**（与 DeleteExecutor 形态逐字节同形），仅次序调整；T5 的「墓碑的槽位内容与 `next_version` 指向不变」在终态成立。
3. **`wal/recovery.rs` 增加「链尾回溯不以已回滚版本为起点」的伴随规则**——由 M2 的修复直接引出（否则恢复期同一存活版本被建两次条目），属 T7「四处构造点行为一致」的最小后果。
4. **用例名**：T6 的两个用例沿用计划名（`abandoned_drop_column_keeps_rewritten_row_visible`、`abandoned_drop_column_reopen_keeps_all_rows`）；T4 的用例名由计划的 `drop_column_across_page_boundary_keeps_all_rows` 改为 `drop_column_across_page_boundary_keeps_every_row_on_one_chain`（含链可达性断言，故名更准——Task Definition 允许改名并记录）。另新增两个窄窗口用例（`…_before_version_registration_…`、`…_before_narrow_copy_registration_…`）。
5. **夹具的源表取值改为 `(id, id*10, id*100)`**（计划未指定具体取值）——使「行宽停留在旧宽度」可被读出（M2 用例的判据需要三列取值互不相同）；行宽与行数不变，页容量边界与规模约束不受影响。

### Persisted Evidence

None required——Plan 的 `Persisted Evidence` 模式为 `none`；判别读数已固化在测试代码中（随用例重跑），RED / GREEN 的实际输出与命令已写入本节，不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形；未创建 `evidence/` 目录。

### Experience Candidates

1. **Issue 候选（当前 change 范围外的实质缺陷，已实测，只报告不登记）**：`src/executor/delete.rs` 的 DELETE 执行器有与 ISS24 **同形**的放弃窗口——墓碑「先写（带链指针）→ 删除索引条目 → 登记版本」，放弃落在「写出版本槽」与「登记版本」之间时，该墓碑未被登记 ⇒ 回收看不到该表 ⇒ 该墓碑按删除者事务状态压制其前驱版本 ⇒ **被放弃的 DELETE 让该行不可见**。实测（N=30，真实 `DELETE FROM t WHERE id = 1` + 锚点式放弃 + 下一条写语句触发回收）：可见 id 集合为 `[2..30]`（`id=1` 缺失）。候选修法（与本 Cycle 对 DDL 改写路径的处理同形）：墓碑「先写（不带链指针）→ 登记 → 补链指针 → 再清理索引条目」；或让中止清理覆盖该表。承载面：Iteration 001 的范围是 DDL 改写路径，DML 墓碑路径属 `ISS24` 登记时的「潜在」面（`.agents/issues/ISS24-abandoned-ddl-hides-rewritten-row.md` 已注明「同一『墓碑 + aborted 版本叠加』形态对任何『先写墓碑再被放弃』的语句都可能成立」），需由 Recorder 登记 Issue 并另择承载。
2. **Runbook 候选**：无（本 Cycle 的验证全部是既有 `cargo test` 命令的组合，未产生新的可复用命令面；收尾全量的 M19 分批纪律已有既有 Runbook `.agents/runbooks/m19-batched-full-regression.md`）。

### 未解决问题

- **无阻塞项**（Gate 5 未命中）。
- **范围外的实质缺陷**：见「Experience Candidates」第 1 条（DML 墓碑路径的同形放弃窗口，已实测）——由 Recorder 登记与承载，不在本 Cycle 的修复面内。
- **本 Cycle 的 Review Result 仍为 `pending`**——由 Plan Review 填写；Act 不填 `Plan Review`，不创建下一 Cycle / 下一 Iteration，不归档 change。



## Plan Review

- Review Result: accepted

**Findings**

1. **ISS24 的四层判别——独立复核成立**（偏差分类：新证据 ⇒ Act 的结论正确，无偏离）。

   - **M1（链分叉）排除成立，且比 Act 的读数更强**：Act 的读数是「被触及的行落在链上」（N=30 时 `touched_pages_on_chain = true`）。独立复核：ISS22 的登记现场本身就记录「N ≤ 30 ⇒ 行数完整」、分叉出现在 N=40；实测 N=30 时 30 行 + 3 个版本槽**全部落在同一个数据页（page 2）** ⇒ 该规模下链分叉在结构上**不可能**发生。⇒ M1 不是 ISS24 症状的成因，且 ISS22 与 ISS24 **不耦合**（两者各有独立的 RED 症状与规模）。
   - **M2 不是本症状的成因，但确为独立缺陷**：缺失行的唯一指向者 `create_tx_id = 31 ≠ 0` ⇒ 不是「已回滚指向者」形态。独立读四处构造点确认 M2 规则（只承认 `create_tx_id ≠ 0` 的指向者）已一致落地：`table_manager.rs::alter_drop_column` / `table_has_live_rows`、`executor/create_index.rs::backfill_tree`、`wal/recovery.rs::rebuild_pk_indexes`。
   - **第 4 层（墓碑中和）+ 第 2 层（主键条目被删未还原）**：与代码一致。`data_scan.rs::superseder_suppresses` 的墓碑分支是「删除者事务不在活动集合 ⇒ 压制」；被放弃的事务在回收后已不在活动集合 ⇒ 未中性化的墓碑**压制其前驱版本**（第 4 层）；同时 DELETE 腿先删主键条目、后登记版本 ⇒ 放弃落在该窗口时条目也不可还原（第 2 层）。两者叠加即「一行在全表扫描与点查中均消失」。

2. **墓碑的新写序不引入新窗口**（偏差分类：新证据 ⇒ 关键风险排除；残留窗口为既有面）。

   - 新次序「写**不带链指针**的墓碑 → 登记 → 补链指针 → 再清理索引条目」使两个中间态都**是惰性的**：无链指针的墓碑不进入 `build_superseded_map` 的抑制映射、不参与 `pointed`、扫描按 `is_deleted` 跳过、恢复期索引重建也不建条目 ⇒ 「写出版本槽」与「登记版本」之间崩溃，重启后**不会出现有可见性影响的悬空墓碑**；「登记」与「补链指针」之间崩溃时登记只在内存（事务管理器的注册表），崩溃即消失 ⇒ 与上一态同形。
   - **残留窗口**：「补链指针 → 索引清理 → WAL Delete」之间崩溃时，墓碑带链指针且创建事务未提交 ⇒ 重开后它按「删除者不在活动集合」压制其前驱版本，而主键条目仍在 ⇒ 扫描隐藏 / 点查可见的不一致。这是**既有窗口**（修复前的次序覆盖「写墓碑 → WAL Delete」的更宽区间），新次序**收窄**而非新增它。实测（放弃 → 崩溃（未回收）→ 重开）：30 行全部可见、完整性 error 级发现 0 ⇒ 观察到的放弃态崩溃恢复是干净的。⇒ 登记为 Issue 候选（见下），不阻塞本 Cycle。

3. **`abort_cleanup_versions` + `versions_created_by` 的正确性——独立复核通过**。

   - **不会误取其它事务的版本**：判据是槽头 `create_tx_id == tx_id`，其它事务创建的槽 `create_tx_id` 不同。
   - **重复登记不产生重复索引条目**：调用方的 `row_ids` 是 `HashSet<RowId>`（`tx_version_tables` 返回按表的 `HashSet`），`extend` 合并进集合 ⇒ 集合语义去重。实测：放弃态回收后主键条目 **30 条、无重复键**；随后真实 `DELETE` 一行后 **29 条、该键无残留**。（Review 期间曾怀疑会产生重复主键条目，实测否定。）
   - **环守卫完备**：`versions_created_by` 带 `visited` 集合，链成环显式报错（与 `data_chain_status` / `heal_stale_tail` 同形态）。
   - **与既有语义一致**：`mark_aborted` 保留链指针 ⇒ 被回滚的墓碑不再压制前驱版本（`superseder_suppresses` 的 `create_tx_id == 0` 分支），但仍留在链上供历史追溯；与已验收的 `mvcc-tombstone-visibility`「未提交与已回滚墓碑不抑制、评估回溯前驱」不冲突。

4. **恢复面的伴随改动不会使已提交行丢失——独立确认**。

   - `pointed` 排除 `create_tx_id == 0` 的指向者 ⇒ 被回滚的墓碑不再把其前驱版本标为非链尾 ⇒ 前驱版本重新成为链尾并被建条目。
   - 「链尾回溯不以已回滚版本为起点」消除了同一存活版本被建两次条目（Act 自检 Important #1 已修）。
   - 已提交行丢失的唯一条件是「该行被某个 `create_tx_id ≠ 0` 的版本指向，且该指向者自身不可达」；在可达链上链尾回溯会走到该行并建条目。不可达的分叉（悬空页）不属本 change 的恢复面。

5. **六个用例的判别力——逐条锚定真实可观察行为**。

   - 断言全部锚定可观察量（行数 / 可见 id 集合 / 目录列定义与列数 / 行宽 / 点查命中数 / 链可达性）；无宽松断言、无框架外判定层、无身份证明机制（无 hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明）、无 timer 阈值行为判据（10s 锚点超时只是夹具失控兜底，panic 文案点名「夹具未达锚点」，与既有 `tests/ddl_catalog_transactional_test.rs:63-105` 同形态）。
   - **RED 与登记症状一致**（不是自造的失败）：ISS22 登记「N=40 → 25 行」↔ 用例 RED「got 25 of 40」；ISS24 登记「30 → 29，缺 id=2」↔ 用例 RED「missing ids: [2]」；M2 的 RED「行宽停留在旧宽度 [(1,10),(2,20)]」↔ 判据是第二次改写后仍读到旧宽度，与 `pointed` 排除已回滚指向者的因果链一致。
   - **因果对应**：用例 1→T5 单一尾游标；用例 2/3→T7 墓碑登记次序 + 回收覆盖面；用例 4→四处 `pointed` 的 M2 规则；用例 5→登记先于补链指针；用例 6→`versions_created_by` 覆盖未登记的窄化副本。
   - **规模**：全部用例 ≤ 40 行（< 300，ISS23 约束）✓。

6. **规格增量 ↔ 验收边界：一项非阻塞覆盖缺口**（偏差分类：Plan 遗漏）。

   - `specs/ddl-rewrite-consistency/spec.md` 的场景「改写后重开库行数不减」把三要素写在同一条：跨页容量边界规模 + close/reopen + 完整性扫描 error 级发现为 0。现有覆盖是**两个用例各覆盖一部分**：跨边界规模由 `…_keeps_every_row_on_one_chain`（40 行）覆盖但**不重开库**；重开库 + 完整性扫描由既有 `tests/ddl_catalog_transactional_test.rs::completed_drop_column_aligns_catalog_and_rows`（小规模）覆盖。**没有一个用例同时满足三要素**。
   - 独立实测该行为本身成立：40 行跨边界 → 完成改写 → close → reopen → **40 行全部可见、完整性 error 级发现 0** ⇒ 缺的是**覆盖**，不是实现。
   - 判定：非阻塞（Cycle 的 Acceptance 六项全部已验收）。承载：**T20**（规格增量与验收边界对齐，Iteration 006）必须闭合该覆盖缺口——补一条跨边界规模的重开 + 完整性断言，或在收敛时把该场景文本收窄到已验收的组合。

7. **Act 的 Deviations 复核**（偏差分类：Plan 遗漏，Act 在既有授权内补齐 ⇒ 不需返工）。

   - **Deviation 1**（T7 修复面扩到 `abort_cleanup_versions` + `versions_created_by`）：Plan 的 T6 四层协议把缺失行钉在第 4 层（不在 M1/M2 候选内）；D5 决策 1「先判别再修」与 T7 的 Stop-when（仅「四层之外」才退回 Plan）授权继续修第 4 层 ⇒ 扩面在授权内。Plan 的 Task Definition 未把第 4 层的修复面写全（Plan 遗漏），Act 补齐正确。
   - **Deviation 2**（DELETE 腿墓碑改为「不带链指针写入 → 登记 → 补链指针 → 再清理索引条目」）：终态的墓碑槽内容与链指针不变，只调整次序 ⇒ T5 的「槽位内容与 `next_version` 指向不变」在终态成立；新次序是第 4 层修复的必要组成（见 Findings 2）。
   - **Deviation 3**（恢复面「链尾回溯不以已回滚版本为起点」）：M2 修复的直接最小后果（否则同一存活版本被建两次条目）⇒ 属 T7「四处构造点行为一致」的最小后果。
   - **Deviation 4 / 5**（用例改名、夹具取值 `(id, id*10, id*100)`）：记录在案，无语义变化；`tasks.md` 的 RTM 行已同步为实际用例名。

**Acceptance Gaps**

- **无阻塞项**：Cycle 的 Acceptance 六项逐项满足（判别步骤 ⇒ DRC R1 ⇒ DRC R2 ⇒ MTV R1 ⇒ DCT R3 ⇒ 既有断言零回归）。
- **非阻塞 1**（覆盖缺口）：规格增量场景「改写后重开库行数不减」缺一条同时满足「跨边界规模 + 重开库 + 完整性扫描」三要素的用例（见 Findings 6）⇒ 承载 T20（Iteration 006）。
- **非阻塞 2**（Act 自检遗留）：`abort_cleanup_versions` 现在对每个被中止事务的每张表做一次数据页链走查（O(链长)）——中止路径本就 O(版本数)，量级同源。
- **非阻塞 3**（Act 自检遗留）：恢复面的「已回滚版本不作链尾起点」与 `pointed` 的构造规则分列两处（未抽公共抽象；T7 禁止结构性重构）。

**Evidence**

- 本次 Review 独立阅读的代码与产物：`src/storage/data/table_manager.rs`、`src/executor/create_index.rs`、`src/wal/recovery.rs`、`src/transaction/manager.rs`、`tests/ddl_rewrite_consistency_test.rs` 的本次改动面（`git diff` 全文），以及 change 的 `tasks.md` / `design.md`（D1 / D4 / D5 / D6 / D12）/ `proposal.md` / 四份规格增量 / 既有域 `mvcc-tombstone-visibility` 与 `.agents/issues/ISS22-*`、`ISS24-*` 的登记现场。
- 本次 Review 实际运行的验证（**临时探针，已删除**，工作区已恢复到进入 Review 时的状态；`git status` 与进入时一致）：
  1. 放弃态回收后的索引形态：主键条目 **30 条、无重复键**；随后真实 `DELETE` 一行后 **29 条、该键无残留**（否定「重复登记 ⇒ 重复主键条目」的怀疑）。
  2. 跨边界完成改写后 close → reopen：**40 行全部可见、完整性 error 级发现 0**（规格增量场景的行为成立，缺的是覆盖）。
  3. 放弃态崩溃（未回收）后重开：**30 行全部可见、完整性 error 级发现 0**（观察到的放弃态崩溃恢复干净）。
  4. `src/executor/delete.rs` 的同形窗口：真实 `DELETE FROM t WHERE id = 1`（N=30）+ 锚点式放弃 + 下一条写语句触发回收 ⇒ 可见 id 集合 **[2..30]**（`id=1` 缺失）⇒ 独立复现 Act 的实测。
  5. N=30 的链形态：30 行 + 3 个版本槽全在**同一个**数据页（page 2）⇒ 该规模下无链分叉可能（M1 排除的结构性依据）。
- **沿用** Act 的验证结论（覆盖范围未变化，按公共规则不重复运行）：`cargo test --test ddl_rewrite_consistency_test -- --test-threads=1` → **6 passed / 0 failed**；`cargo test --lib -- --test-threads=1` → **420 passed / 0 failed**；31 个定向目标分 4 个串行批次全部 0 failed。
- **未做**：全量回归（收尾全量归 Iteration 006，本 change 一次预算）；MS35 / MS36 的 2192/0/2 终态覆盖范围已失效，不作为当前工作区的依据。

**Issue Candidates**（只报告不登记；登记与承载由 Recorder / 后续 Plan 执行）

1. **可承载**：`src/executor/delete.rs` 的 DML 墓碑放弃窗口（ISS24 的同形面）。
   - **结论**：已实测的实质缺陷（静默丢行级）。**归属本 change 的新增 Iteration 005**（DML 墓碑的放弃窗口收敛，承载 T14–T15），已写入 `tasks.md` 的任务表、RTM、Iteration Plan 与 Iteration Map；**Issue 的登记由 Recorder 执行**（Plan 只报告不登记）。
   - **根因位置**：`src/executor/delete.rs` 的单行删除路径——墓碑「**先写（带链指针）** → 删除 PK/唯一/二级索引条目 → 登记版本」；在「写出版本槽」与「登记版本」之间放弃时，该墓碑未被登记 ⇒ 回收按事务版本集合看不到该表（`abort_cleanup_versions` 只遍历**已登记**版本的表，`versions_created_by` 只在已登记的表上被调用）⇒ 墓碑保持 `create_tx_id ≠ 0` ⇒ 按 `data_scan.rs::superseder_suppresses` 的「删除者不在活动集合」分支**压制其前驱版本** ⇒ 该行不可见。`src/executor/upsert.rs` 的删除段（REPLACE）同形。
   - **复现路径与实测证据**（本次 Review 独立复现，与 Act 的结论一致）：真实文件库 + 真实 `Database::open` + 真实 `DELETE FROM t WHERE id = 1`（N=30）+ 锚点式放弃（活跃事务已在链上留下版本槽）+ 下一条写语句触发回收 ⇒ 可见 id 集合 **[2..30]**（`id=1` 缺失）。
   - **影响面与触发条件**：`DELETE`（及 REPLACE 的删除段）在「写出版本槽」与「登记版本」之间被放弃（语句 future 被丢弃 / 超时）时触发；表现为**行静默消失**；不限于本 change 的 DDL 改写面。
   - **与范围的关系**：不在 Iteration 001 的 Included scope（DDL 改写路径 + 四处 `pointed` 构造点）内，也不在 Iteration 002 的范围内（释放路径的**读面**）；既有域 `mvcc-tombstone-visibility` 的「未提交与已回滚墓碑不抑制、评估回溯前驱」在 DML 路径上尚未闭合 ⇒ 是既有已验收规格的实现缺口，不需要新的规格增量。
   - **修复方向**：与 Iteration 001 对 DDL 改写路径的处理**同形**——墓碑「先写（不带链指针）→ 登记 → 补链指针 → 再清理索引条目」，使放弃窗口内的墓碑不压制任何前驱版本；修复面限于 `src/executor/delete.rs`（及 `upsert.rs` 同形路径）的**写序**，**不改** `VersionHeader::mark_aborted` 的链指针语义、**不改**既有 MTV 判据。
   - **承载落点**：`tasks.md` 的 **T14（RED 用例）+ T15（写序收敛）**，Iteration **005**（依赖 Iteration 001，排在收尾全量之前）；验证入口 `tests/dml_tombstone_abandon_window_test.rs`。
2. **可承载**：崩溃后未中性化墓碑的可见性（既有窗口，本次变更收窄但未闭合）。
   - **结论**：既有缺陷面，**不承载**在本 change（需要改动崩溃模型与恢复面的中性化范围，超出本 change 的边界）⇒ 作为 Issue 候选登记，由 Recorder 择承载（另立 change 或后续里程碑）。
   - **根因位置**：`src/storage/data/table_manager.rs::alter_drop_column` 的 DELETE 腿（墓碑带链指针的时刻早于其创建事务的持久化点）+ `src/executor/data_scan.rs::superseder_suppresses` 的墓碑分支（删除者不在活动集合即视为已提交删除）。
   - **复现路径与实测证据**：代码级推演（本次 Review）：崩溃落在「补链指针 → 索引清理」之间时，重开后该墓碑压制其前驱版本而主键条目仍在 ⇒ 扫描隐藏 / 点查可见。观察到的放弃态崩溃（`放弃 → 崩溃（未回收）→ 重开`）实测为 30 行全部可见、完整性 0 ⇒ 观察到的窗口未命中该中间态。
   - **影响面与触发条件**：崩溃恰好落在改写的「墓碑已带链指针、其创建事务未提交」区间时；表现为改写过的行在重开后于扫描面消失。
   - **与范围的关系**：本 change 的 D5 决策与本 Cycle 的 Acceptance 都限定在「放弃 → 回收」路径（运行期）；崩溃后的未提交墓碑窗口属既有崩溃模型（既有域 `drop-persistence-boundary` 与恢复面）。
   - **修复方向**：恢复期对「创建事务未提交且带链指针」的墓碑显式中性化（与运行期 `abort_cleanup_versions` 同形），或在墓碑带链指针前先完成其创建事务的持久化标记。
   - **承载落点**：登记为 Issue 候选，由 Recorder 择承载；本次变更已把该窗口从「写墓碑 → WAL Delete」**收窄**为「补链指针 → WAL Delete」，**不构成回归**。
3. **无其它 Issue 候选**：除上述两条，本次 Review 未发现新的实质缺陷面。

**Follow-up Decision**

- **当前 Cycle 无需修复**：无阻塞 Acceptance 缺口 ⇒ **不**创建 rework / replan Cycle；Iteration 001 的 Plan Context 与 Act Response 保持原样（Plan Context 不可改写）。
- **后续承载**：Issue 候选 1 ⇒ Iteration 005（T14–T15，已写入 `tasks.md`）；Issue 候选 2 ⇒ Issue 登记（Recorder）；覆盖缺口 ⇒ T20（Iteration 006）。

**Iteration Plan Update**

- Iteration 001 的计划**不变**：Acceptance 六项全部满足；Act 的 5 条 Deviations 均在既有授权内（见 Findings 7），无需返工或重规划；稳定结果与诊断边界按 `tasks.md` 的 Iteration 001 段落保留。
- **change 层面的计划调整**（因 Issue 候选 1 的承载需要）：`tasks.md` 新增 **T14–T15** 与 **Iteration 005**（DML 墓碑的放弃窗口收敛），原 Iteration 005（旧格式兼容证据补齐）顺延为 **006** 并继续承担收尾全量（T14–T19 → T16–T21）；`design.md` 的 D1（Iteration 数 6 → 7）与 D12（收尾全量指针 005 → 006）同步更新。
  - 副作用（明示）：已冻结的 Iteration 000 / 001 的 `Plan Context` 中「收尾全量在 Iteration 005」的指针随之**过时**（Plan Context 不可改写 ⇒ 未修改）；以 `tasks.md` 的 Iteration Map 为准。

**Next Cycle**

None

**Next Iteration**

- Iteration 002 已展开：`iterations/002-release-path-bounded-read/000-initial.md`（Cycle 000，`Plan Context` 状态 `ready`，承载 T8–T9：B-4 的释放路径有界读）。
- 下一个待展开 Iteration 为 **003**（改写窗口的读一致性）。**Iteration 005**（DML 墓碑的放弃窗口收敛）待展开。
