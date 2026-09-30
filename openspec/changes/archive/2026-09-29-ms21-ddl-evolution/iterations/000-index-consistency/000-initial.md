# Iteration 000 / Cycle 000-initial: 索引条目维护一致性收口

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1-①②③（`tasks.md` T1 全部）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `update-index-maintenance` 域 ADDED Requirement「回滚与冲突删除路径索引条目一致」全部场景；design D1/D2
- Excluded scope: T2–T5（二级索引、ALTER、TRUNCATE、VIEW）；`update.rs`/`upsert.rs` 写侧 rekey 逻辑（Forbidden）

**Objective**

rekey UPDATE（键列或唯一列）回滚后，PK 与唯一索引条目与存活版本一致（旧键点查可达、新键无残留、双键 INSERT 不被误拒、干净重开与崩溃恢复两态一致）；UPSERT 冲突行删除在 slot 不可读态与 DELETE 执行器对齐（按命中键清理条目）；既有回滚还原矩阵零回归。

**Investigation Facts**

- Current Baseline: revision 0fe6971（工作区干净）；全量 1458 tests / 0 failures / 2 ignored（2026-09-29 MS20 收尾 Plan Review 全量串行新鲜结论，覆盖面未变化，直接采信）。`explicit_tx_test` 现无任何 rekey 回滚用例（grep `rekey` 零命中）。
- Current-State Evidence:
  - `TransactionManager::abort`（`src/transaction/manager.rs:142-169`）先写 `AbortTxn` WAL（:151-153）再调 `abort_cleanup_versions`（:156）；`Database::rollback` 组装 tables 映射（`src/database.rs:200-220`）。
  - `abort_cleanup_versions`（manager.rs:282-370）：分桶趟（:300-316）按 `find_key_by_row_id(row_id).is_some()` 分入 A 趟（indexed）/B 趟（tombstones），分桶在任何索引写入前完成；A 趟（:318-355）PK 臂 `:321 find_key_by_row_id` 反查 → `:323-328 next_version()` Some → `update(&key, prev_row_id)`（:325）/ None → `delete(&key)`（:327）；唯一索引臂 :334-342 逐字同构；:353 `mark_aborted` 中性化。**rekey 缺陷根因**：反查键来自内存 `row_to_key` 反向映射（`src/storage/btree/index_manager.rs:352-354`），rekey UPDATE 运行期 `delete(old)` + `insert(new)`（`src/executor/update.rs:314-320`）使反查键=新键，而 prev 版本 tuple 携带旧键 → `update(新键, prev)` 错置。
  - B 趟 `restore_tombstone_index_entries`（manager.rs:385-442）：单次页读 `read_tuple_from_data_page` 同取 header+tuple（:410-414）、`extract_index_keys`（`src/wal/recovery.rs:225-254`，`pub(crate)`，manager.rs:427-428 已在消费）从 tuple 派生 PK+唯一列真实键、回溯 `next_version()` 上限 64（:13, :404）。「按目标版本 tuple 内容派生真实键」模式在 B 趟已是现成先例；佐证：rekey 置 NULL 的 UPDATE 回滚现落 B 趟且行为正确。
  - 唯一索引 A 趟臂与 PK 同缺陷（manager.rs:334-342 反查 `uindex.find_key_by_row_id` → `update`）。同根因附加形态：SET 唯一列从 NULL 改值的回滚（:337 会把新值键 update 指向 prev 而 prev 该列为 NULL）。
  - `upsert.rs::delete_conflict_row`（:550-618）：`:567 SlotNotFound` 容忍臂返回 `(None, Vec::new())` → PK 条目删除（:590-594）与唯一条目删除（:596-598）被跳过；墓碑写入自备容忍（:586）；WAL Delete 照写（:608-615）。调用点仅 REPLACE 臂（:644-656，仲裁 `search` 命中键在 :151 可得）。对照 `src/executor/delete.rs:135`：PK 条目删除无条件按 WHERE 搜索键执行，其 SlotNotFound 臂只跳过墓碑（:126-130）与唯一键派生（:49-75, :93-96）。
  - 夹具可达性：`TableMeta.index_manager` 经 `table_manager.get_table()` 可达（先例 `tests/drop_table_free_test.rs:102-134` 用 `meta.index_manager.collect_all_pages()`）；`IndexManager::insert` 可注入任意 (key, rid)（直构先例 `tests/index_manager_test.rs:15`、`tests/mvcc_abort_test.rs:79-167`）。`tests/` 全目录 grep `SlotNotFound` 零命中（无现存夹具）；MS24 归档 Review 为纯代码读发现、无复现配方。
  - 恢复语义：aborted 事务的 WAL 记录永不被重放（`src/wal/recovery.rs:474-503` 分类、`:527-533` redo 只重放 committed），恢复终态=运行期 abort 落盘状态；`redo_count > 0` 时 rebuild 从数据页按 tuple 派生键重建（天然收敛），`redo_count == 0` 沿用运行期索引。两态断言模式先例 `tests/update_index_maintenance_test.rs::recovery_matches_runtime_after_rekey`（:437：checkpoint 固化 → `wal_buffer.shutdown()` + drop 不 close → 重开）。
- Code and Critical Path: 变更面仅 `src/transaction/manager.rs::abort_cleanup_versions`（A 趟两臂）与 `src/executor/upsert.rs::delete_conflict_row` + REPLACE 调用点；对照面 `src/executor/delete.rs`；见证面 `tests/explicit_tx_test.rs`（lib 直连 `Database::open/begin/execute_in_tx/rollback`，helper :14-42）、`tests/update_index_maintenance_test.rs`、`tests/upsert_test.rs`、`tests/mvcc_abort_test.rs`（直构形态参照）。

**Implementation Guidance**

按 design D1/D2 实施。T1-① 建议顺序：先在 A 趟 `next_version().is_some()` 分支内引入「prev tuple 派生键」（`read_tuple_from_data_page` 同页读 header+tuple——:319 已读 header，可合并为单次读，B 趟 :410-414 同型）；派生键与反查键相等时保持既有 `update(&key, prev)`（非 rekey 快路径逐字节不变），`Some` 且不等 → `delete(反查键)` + `insert(派生键, prev)`，`None` → 仅 `delete(反查键)`；PK 臂与唯一臂同型。次序约束：delete 先于 insert（`row_to_key` 反向映射经 delete 清理，update.rs:296-298 注释同源）。T1-② 把 REPLACE 臂仲裁命中的 PK 键值作为参数传入 `delete_conflict_row`，容忍臂内补 `index_manager.delete(命中键)`。测试先 RED：新增用例跑出缺陷断言失败，再实施转 GREEN。

**Task Contracts**

### T1-①: rekey UPDATE 回滚后 PK 与唯一索引条目一致

- Requirement/Scenario: `update-index-maintenance` ADDED「回滚与冲突删除路径索引条目一致」场景 1–4
- Depends on: None
- Targets: `src/transaction/manager.rs::abort_cleanup_versions`（A 趟 PK 臂 :321-328、唯一索引臂 :334-342）；可引用 `restore_tombstone_index_entries`（:385-442）与 `wal::recovery::extract_index_keys` 的既有形态
- Current behavior: rekey UPDATE 回滚后新键条目指向旧键版本、旧键条目缺失、旧键等值点查漏行、新键 INSERT 可能异常；唯一列同型；SET 唯一列从 NULL 改值的回滚错置新键条目
- Required behavior: 回滚后旧键条目恢复指向 prev 版本（点查可达）、新键条目不存在（点查空集、INSERT 可用）、唯一列同型、干净重开与崩溃恢复两态一致；非 rekey UPDATE 回滚行为与现状逐字节一致（走 `update` 快路径）
- Preserve: 分桶趟（:300-316）判定不变；B 趟不变；`mark_aborted` 中性化与 AbortTxn WAL 时序不变；`update.rs`/`upsert.rs` 零触碰
- Forbidden: 修改写侧 rekey 逻辑；修改 WAL 记录类型；新增恢复路径逻辑（恢复面现状已正确）
- Test witness: `tests/explicit_tx_test.rs` 新增 ≥4 用例（PK rekey 回滚旧键可达 / 新键无残留+双键 INSERT / 唯一列 rekey 回滚 / 干净重开与崩溃恢复两态〔`recovery_matches_runtime_after_rekey` :437 模式〕），先观察 RED（缺陷断言失败）
- GREEN condition: 新用例全绿 + `explicit_tx`/`update_index_maintenance`/`mvcc_tombstone_visibility` 既有用例零修改通过
- Verification: `cargo test --test explicit_tx_test --test update_index_maintenance_test --test mvcc_tombstone_visibility_test --test upsert_test -- --test-threads=1`（退出码 0、0 failed）+ 全量 `cargo test -- --test-threads=1` 零回归
- Stop when: prev tuple 在 A 趟上下文不可读（页 I/O 结构性障碍）或派生键方案与分桶语义冲突——返回 Plan

### T1-②: UPSERT 冲突删除 slot 不可读态按命中键清理条目

- Requirement/Scenario: `update-index-maintenance` ADDED 同 Requirement 场景 5
- Depends on: None（与 T1-① 无代码耦合，可并行实施）
- Targets: `src/executor/upsert.rs::delete_conflict_row`（:550-618，容忍臂 :567 与 PK 条目删除 :590-594）+ REPLACE 调用点（:644-656）
- Current behavior: 冲突条目指向不可读 slot 时跳过 PK 条目删除，悬空条目残留
- Required behavior: 容忍臂下仍以仲裁命中的 PK 键执行 `index_manager.delete`（对齐 `delete.rs:135`）；墓碑写入跳过与唯一条目无键跳过保持；正常路径（slot 可读）行为逐字节不变
- Preserve: `arbitrate` 仲裁逻辑不变；DO UPDATE 臂（`apply_do_update`）不经本函数、零触碰；WAL Delete 记录形态不变
- Forbidden: 修改 `delete.rs`；改变正常路径删除语义
- Test witness: `tests/upsert_test.rs` 新增夹具用例：建表插行 → 经 `get_table().index_manager.insert(悬空键, 悬空 rid)` 注入条目 → `REPLACE INTO` 命中悬空键 → RED 观察条目残留 → GREEN 断言条目被清理（`search`/`find_key_by_row_id`）且语句行为正确
- GREEN condition: 夹具用例绿 + `upsert_test` 既有 31 用例零修改通过
- Verification: `cargo test --test upsert_test -- --test-threads=1`（0 failed）+ 全量串行零回归
- Stop when: 悬空条目经 pub API 不可注入（可达性假设失效）——返回 Plan

**Invariants**

- aborted 事务 WAL 记录不被重放、恢复终态=运行期落盘状态（既有事实，修复不得引入恢复路径改动）。
- 分桶趟先于任何索引写入完成、顺序无关性保持。
- 非 rekey 回滚与正常路径 UPSERT 的行为逐字节不变。
- `update.rs`/`upsert.rs` 写侧 rekey、`delete.rs`、WAL 记录类型、恢复逻辑为零触碰面。

**Non-goals**

- 二级索引（Iteration 001）；A 趟反查键双查询的消除（replan Deviation 4 既定保留）；`delete_exact` 类新原语（001 的 T2-b）。

**Acceptance**

1. `update-index-maintenance` ADDED Requirement 6 场景全部有可观察见证（场景 1–4 → T1-①；场景 5 → T1-②；场景 6 → 既有矩阵零回归）。
2. RTM（本 change 全量）：

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| update-index-maintenance › 回滚与冲突删除路径索引条目一致（PK rekey 回滚 4 场景） | T1-① | explicit_tx 新增 rekey 回滚矩阵 | Covered |
| 同上（UPSERT 夹具态对齐） | T1-② | upsert_test 悬空条目夹具 | Covered |
| 同上（既有回滚还原零回归） | T1-③ | explicit_tx/upsert/mvcc 既有矩阵 | Covered |
| sql-secondary-indexes（R1–R5） | T2-a…g | 001 Cycle 定稿 | Covered（未展开） |
| alter-table-column-evolution（R1–R3） | T3-a…d | 002 Cycle 定稿 | Covered（未展开） |
| truncate-table（R1–R2） | T4-a…b | 003 Cycle 定稿 | Covered（未展开） |
| sql-views（R1–R3） | T5-a…d | 004 Cycle 定稿 | Covered（未展开） |

**Verification**

- 场景 1–4（rekey 回滚矩阵）：`cargo test --test explicit_tx_test -- --test-threads=1` 新增用例 GREEN + 既有 0 failed。
- 场景 5（UPSERT 对齐）：`cargo test --test upsert_test -- --test-threads=1` 新增夹具 GREEN + 既有 0 failed。
- 场景 6（零回归）：全量 `cargo test -- --test-threads=1`（判定：0 failed；基线 1458 + 本轮新增）。
- 静态门：`cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate` 全部通过。

**Gate 2 Readiness**

Gate 2 全部 PASS（2026-09-29 用户批准计划，含 design D13 事务边界修订与 proposal 假设 11 同步修订；无 BLOCKED、无 WAIVED 项）。调查完整（两缺陷根因、修复面、见证可达性均有代码证据）、设计闭合（D1/D2 无 TBD）、任务契约完备、验证最简直接（原生 cargo test 退出码判定）、Persisted Evidence 无必要。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- A 趟内 `read_tuple_from_data_page` 为异步页读（B 趟同型已在同函数使用，无新增并发面）。
- 派生键快路径相等判定使用字节比较（`Vec<u8>` 相等），与 `Key` 32B 全量比较语义一致（非 rekey 时反查键即 tuple 派生键，逐字节相等成立）。
- T1-② 夹具注入的悬空 rid 指向不存在页；REPLACE 完成后若后续用例崩溃重开会遇到含悬空 rid 的 WAL Delete——夹具库为一次性 tempdir、不做恢复断言，无影响。
- 非实质选择留给 Act：派生键与页读的具体函数组织、用例命名与断言组织。

## Act Response

- Status: reported
- Date: 2026-09-29

### 实际改动

**T1-①**（`src/transaction/manager.rs`）：A 趟 PK 臂与唯一索引臂按 design D1 重构——`next_version()` 为 `Some(prev)` 时新增 `read_prev_index_keys`（单次页读 prev tuple + 按 schema 反序列化 + `wal::recovery::extract_index_keys` 派生真实键）；四分支匹配：派生键==反查键 → 既有 `update` 快路径（逐字节不变）；派生键≠反查键 → `delete(反查键)` + `insert(派生键, prev)`（delete 先于 insert）；派生键位 NULL → 仅 `delete`；派生不可得 → 既有 `update` 形态回退。无前驱分支保持 `delete`。新增自由函数 `read_prev_index_keys`（SlotNotFound 与反序列化失败 → `Ok(None)` 回退；其余错误传播）。分桶趟、B 趟、`mark_aborted` 中性化、AbortTxn WAL 时序零改动；`update.rs`/`upsert.rs` 写侧、`delete.rs`、恢复路径零触碰。

**T1-②**（`src/executor/upsert.rs`）：`delete_conflict_row` 签名加 `hit_pk_key: Option<Vec<u8>>`（REPLACE 臂传入 `prepared[pk_index].to_key()`）；SlotNotFound 容忍臂按命中键清理 PK 条目（带在位守卫 `search(key) == Some(rid)`）；容忍臂不再把悬空 rid 记入 `record_version`。正常路径（slot 可读）行为逐字节不变；`arbitrate`、`apply_do_update`、WAL Delete 记录形态零改动。

**测试见证**：`tests/explicit_tx_test.rs` +5 用例（PK rekey 旧键可达 / 新键无残留+INSERT 可用 / 唯一列 rekey 占用与释放 / 崩溃重开两态 / 唯一列 NULL→值回滚释放新值）；`tests/upsert_test.rs` +1 夹具用例（悬空条目经 `index_manager.insert` 注入已存在页缺失 slot，REPLACE 命中后断言条目清理与语句正确）。既有测试零修改（全量 1464 = 基线 1458 + 6）。

### 与计划的偏差

1. **容忍臂删除带在位守卫**（契约未写守卫）：实施调查实证 `IndexManager::delete` 对缺失键会误删「首个 ≥」邻近条目（CLI 实测：3 行表 `DELETE WHERE id = 0` 报 affected 1 且 key 1 条目被误删、点查漏行——见 Issue 候选 1）。不带守卫的盲删会把该歧义引入 UPSERT 路径（经唯一索引命中的冲突行其 PK 键与仲裁键无关，必为缺失键）。
2. **容忍臂不记录悬空 rid**（Plan 遗漏，Acceptance 必要后果）：RED 阶段实测契约夹具报 `Abort failed: slot not found`——悬空 rid 入 `tx_versions` 后 `commit_mark_versions`/abort 对不可读 slot 传播错误，事务无法终结，delta spec 场景 5「条目被清理且语句行为正确」不可达成。改为墓碑写入被跳过且无元组可读时不记录；WAL Delete 形态不变（恢复面按位置寻址，夹具态不做恢复断言，Plan Risks 已预声明）。
3. **helper 增加反序列化失败回退分支**（回归修复）：首次实现把「prev tuple 反序列化失败」与「键位 NULL」合并为 delete-only，分批全量批 5 暴露既有直构用例 `mvcc_abort_test::test_abort_update_reverts_to_previous_version` 回归（夹具条目键为手写 `b"test_key"`、tuple 首字节 0x0A 非 Int tag，反序列化必失败）——变更前该角落从不读 tuple，行为必须逐字节保持。修复：反序列化失败 → `Ok(None)` → 既有 `update` 回退；反序列化成功但键位 NULL → 仅删除（真实键位 NULL 形态保留）。
4. **M19 约束修订（Cycle 外，用户指令）**：用户以「分批次运行测试……豁免进行直接更改文档」指示将 project-model M19 由「全量单次串行」改为「确定性分批、批内串行、批间顺序」，Act 直接修订 `.agents/memory/project-model.md`（豁免原话留存本 Response）。

### 自检发现与遗留

- 已修复：偏差 3 的回归（批 5 截断暴露，修复后 batch_ae 68/0）。
- 遗留 Minor：多语句同事务对同一行连续 UPDATE 后回滚的 A 趟处理序依赖 `HashSet` 迭代序（条目最终指向可能与语句逆序假设不符）——pre-existing，单语句 rekey charter 外，未改动。
- 遗留观察（charter 外 adjacent gap）：SET 唯一列值→NULL 的回滚后唯一条目缺失（被删旧键条目不还原，恢复面 rebuild 会收敛、运行期不还原）——delta spec 场景仅覆盖值→值 rekey 与 NULL→值，本 Cycle 未扩展。

### 验证

- RED：5 个 explicit_tx 新用例全部在缺陷断言失败（旧键点查空集 / 唯一值静默失效 AffectedRows 1 / 新键 DuplicateKey）；upsert 夹具 `Abort failed: slot not found: RowId { page_id: 2, slot_id: 9999 }`。
- GREEN：`cargo test --test explicit_tx_test --test update_index_maintenance_test --test mvcc_tombstone_visibility_test --test upsert_test -- --test-threads=1` → **71 passed / 0 failed**（19/9/11/32，fmt 后复跑）。
- 全量（M19 分批形态，`--test-threads=1` 批内串行、批间顺序）：批 0 lib+bins 327/0、批 1 98/0、批 2 210/0/2i、批 3 129/0、批 4 97/0、批 5 68/0（含 mvcc_abort 回归修复）、批 6 88/0、批 7 170/0、批 8 139/0、批 9 113/0、批 10 25/0——**合计 1464 passed / 0 failed / 2 ignored**（= 基线 1458 + 新增 6）。批 2–4 在 helper 回归修复前跑绿；该修复仅改变「prev tuple 反序列化失败」分支（真实 tuple 行为面不变，直构夹具面批 5 与批 0 复跑已覆盖），按覆盖未变化采信。批 0 于修复后复跑 327/0。
- 静态门：`cargo clippy --all-targets -- -D warnings` 0 errors；`cargo fmt --check` 通过（fmt 对两文件排版微调后复跑契约矩阵 71/0）；`openspec validate 2026-09-29-ms21-ddl-evolution` → valid。

### Persisted Evidence

- None required（Plan 设 none；所有验证可由上述命令低成本重放）。

### Experience Candidates

- **Issue 候选 1（change 范围外实质缺陷）**：`DELETE`/`IndexManager::delete` 歧义删除原语对缺失键误删「首个 ≥」邻近条目——实证：3 行表（id 1/2/3）`DELETE FROM t WHERE id = 0` 返回 affected_rows 1、三行扫描仍在，但 `WHERE id = 1` 点查空集、`INSERT (1,…)` 不再 DuplicateKey；`DELETE WHERE id = 999`（大于全部键）则 `KeyNotFound` 报错。根因 `LeafNode::delete`（node.rs:222-235）以插入位删除 + `DeleteExecutor`（delete.rs:135）对 search 未命中仍无条件按键删除。证据：本 Response 验证段 CLI 复现序列。本 Cycle 零触碰面，未修复。
- **Issue 候选 2（边界观察）**：SET 唯一列值→NULL 回滚后唯一条目缺失（运行期不还原、恢复面收敛），见「遗留观察」。
- Runbook 候选：无（分批全量为 M19 即时约束，一次性分组）。

### 未解决问题

- 多语句同行 UPDATE 回滚迭代序依赖（Minor，见自检段）；候选 1/2 待 Recorder 按用户指令落账。

## Plan Review

- Review Result: accepted
- Date: 2026-09-29

**Findings**

非阻塞 Minor findings（3）：

1. 场景 3 的「WHERE u = 9 空集」以「重插 u = 9 不被误拒」见证——唯一列无索引路由，扫描空集由数据本身（回滚后行仍为 (1,5)）平凡保证，占用/释放语义才是可观察正确面；场景 5 用例已含 u=9 扫描空集直接断言。见证等价，不要求补齐。
2. 偏差 1（容忍臂在位守卫）分类：Act 偏离，必要后果成立——`IndexManager::delete` 歧义删除原语（→ 本 change 扩围 T6）使盲删会把「首个 ≥」邻近条目误删引入 UPSERT 路径；守卫与 delete.rs 语义对齐且经夹具见证。偏差 2（容忍臂不记录悬空 rid）分类：Plan 遗漏，Acceptance 必要后果（否则场景 5「语句行为正确」不可达，事务无法终结）。偏差 3（反序列化失败回退）分类：回归修复——变更前该角落从不读 prev tuple，既有直构夹具行为逐字节保持，处理正确。偏差 4（M19 分批修订）经用户指令，修订文本与指令一致（豁免原话留存 Act Response）。
3. `hit_pk_key` 逐 rid clone 的微小分配——非实质，不要求处理。

**Acceptance Gaps**

None——6 场景全部有可观察见证（场景 1–4 → T1-① 5 用例；场景 5 → T1-② 夹具；场景 6 → 既有矩阵零修改通过），RTM Covered 链路与代码一致。

**Evidence**

- 独立 diff 审读（Plan Review 双检查之二）：`src/transaction/manager.rs`（A 趟四分支 + `read_prev_index_keys`：SlotNotFound→Ok(None)、反序列化失败→Ok(None)、键位 NULL→Some((None,…))）与 `src/executor/upsert.rs`（`delete_conflict_row` 签名/在位守卫 `search(key)==Some(rid)`/记录条件 `tombstone_rid.or_else(old_values.is_some().then_some(rid))`/REPLACE 臂 `prepared[pk_index].to_key()` 穿参）与 Act Response 声明逐项一致；分桶趟、B 趟、`mark_aborted`、AbortTxn WAL 时序、`update.rs`/`delete.rs`/恢复路径零改动（git diff 全量核对，仅 5 文件 + change 目录）。
- 键派生同源性：`extract_index_keys`（`src/wal/recovery.rs:225-254`）输出 `to_key().as_bytes()` 与反查键（`IndexManager::insert` 存储入参字节）同源，非 rekey 快路径字节相等判定成立。
- 测试见证：explicit_tx +5（场景 1/2/3/4/NULL→值附加形态）、upsert +1（悬空条目夹具：注入已存在页缺失 slot、命中键清理、`scan_all` 恰余 1 条目）与场景映射核对一致；崩溃模拟形态（`flush_all` + drop 不 close）与 `update_index_maintenance_test:437` 先例同型。
- 验证采信：GREEN 71/0、全量分批 1464/0/2i、clippy/fmt/`openspec validate` 均为本会话 Act 新鲜运行，其后工作区未变化（只读 git 检查），按覆盖未变化采信（Plan Review 双检查之一由 Act GREEN 承担）。
- 身份型证据检查：无新增身份型证据工程或判定层。

**Follow-up Decision**

接受：既有 Acceptance 全部满足，4 项偏差均为非阻塞且处置正确。Act 报告的两个 Issue 候选（`IndexManager::delete` 缺失键误删邻近条目；SET 唯一列值→NULL 回滚条目缺失）为范围外实质缺陷，经用户 2026-09-29 指令「把这两个 issue 计划解决」并入本 change 计划（扩围 T6/T7，插入为新 Iteration 001，design D16/D17），不作为本 Cycle 返工——本 Cycle 验收不依赖其修复。缺陷台账（`.agents/issues/`）登记由用户另行指令 openspec-experience-recorder。

**Iteration Plan Update**

2026-09-29 用户扩围指令已落实（change tasks.md / proposal / design / delta spec 同步）：任务清单新增 T6/T7 节；Iteration Map 插入「001 删除原语精确化与回滚 NULL-rekey 还原」（T6/T7，Depends on 000），原 001–004 顺延为 002–005 并更新依赖引用；delta spec 增补「DELETE 未命中键零副作用」Requirement（3 场景）与 NULL-rekey 场景。

**Next Cycle**

None

**Next Iteration**

`iterations/001-delete-exactness-restore/000-initial.md`（扩围后 Map 首个后继；Status: draft——Gate 2 除「用户批准计划」外全部 PASS，批准后转 ready 交 Act）
