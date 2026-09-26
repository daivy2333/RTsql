# Iteration 001 / Cycle 001: UPSERT 与 REPLACE INTO — replan（PK 预检补齐 + 回滚索引还原）

## Plan Context

- Status: ready
- Iteration: 001-upsert-replace
- Cycle: 001-replan
- Cycle Type: replan
- Parent cycle: ./000-initial.md

**Iteration Scope**

- Change tasks: 2.7, 2.8, 2.9, 2.10（2.1-2.6 已由父 Cycle 完成，见下 Cycle Scope）
- Depends on: Iteration 000（accepted 2026-09-25）
- Stable baseline: 显式冲突目标只仲裁该约束、违反仲裁外约束（PK 或唯一）以既有 DuplicateKey 拒绝；删除者事务回滚后被删行的 PK 与唯一索引条目完整还原（索引状态与「删除未发生」一致）；既有 INSERT 路径与三执行器文件逐字节不变；全量零回归
- Verification boundary: upsert_test（补 2 例）+ explicit_tx_test 回滚还原矩阵 + constraint_enforcement_test / mvcc_tombstone_visibility_test 零回归 + cli_test 零回归 + `cargo test` 全绿
- Diagnostic boundary: `src/executor/upsert.rs::insert_row`、`src/transaction/manager.rs::abort_cleanup_versions`、`src/wal/{mod.rs,recovery.rs}` 取键 helper 可见性、`README.md` / `README.zh-CN.md` 写面段与本 Cycle
- Deferred tasks: None（001 为 change 最后一个 Iteration）

**Cycle Scope**

- Trigger: replan-required
- Acceptance gaps: 父 Cycle Plan Review 的两项阻塞发现（R3 显式目标与仲裁外 PK 约束；R5 文档与实现一致）＋ 扩围项「回滚后墓碑行索引条目不还原」（唯一值释放导致 `UNIQUE` 静默失效、PK 等值点查漏行）
- Repair items: None（replan Cycle 使用更新后的全局任务 2.7-2.10，不建本地 repair item）
- Inherited scope: change 全部 R3/R4/R5 requirement 与 design D4/D5/D6；本 Cycle 修订部分为 D9/D10 与 delta `mvcc-tombstone-visibility`（ADDED）、`sql-constraint-enforcement`（MODIFIED R3）；父 Cycle 产物——`UpsertExecutor` 三动作、冲突仲裁、DO UPDATE 写形状、REPLACE 删除语义、恢复两态、既有门优先级与文本锚点（2.1-2.6 全绿，1231 tests）
- Excluded scope: DO UPDATE WHERE；`INSERT OR ...` 方言迁移；算术/函数赋值表达式；组合唯一约束（MS21 域）；提交路径与恢复重放通道的索引策略变更；删除时延迟移除索引条目等运行期索引语义改写；DDL 约束面（MS24-T04 仍为独立任务行）

**Objective**

`UpsertExecutor::insert_row` 在无冲突插入路径上补齐 PK 重复预检，使显式唯一列冲突目标下的 PK 冲突以既有 `DuplicateKey` 零副作用拒绝；`TransactionManager::abort_cleanup_versions` 为墓碑行还原删除前最新存活版本的 PK 与唯一索引条目，使删除者事务回滚后的索引状态与「该删除未发生」一致（含同事务 update→delete、REPLACE 与失败语句三形态，处理顺序与版本集合迭代顺序无关）；README 写面段的零副作用表述按修复后实测复核。

**Background**

父 Cycle（`000-initial.md`，Act Response `reported`、Review Result 原为 `pending`）交付 2.1-2.6 全部任务，Plan Review 独立审查判定两项阻塞发现：① `insert_row` 只镜像了 `InsertExecutor` 的 UNIQUE 预检与其后序列，PK 重复预检留在 `arbitrate` 内仅对 `All` 与 `Column(pk)` 生效 → 显式唯一列目标下 PK 冲突静默写入重复主键行（违反 delta spec R3「违反仲裁外约束 SHALL 以既有 DuplicateKey 拒绝」）；② README 双语「被拒绝的写入零副作用」被失败 REPLACE 证伪（先删后校验，删除已发生后语句级回滚，而回滚不还原索引条目）。

发现 ② 的根因是既有缺陷：`abort_cleanup_versions`（`src/transaction/manager.rs:271-335`）以 `index_manager.find_key_by_row_id(rid)` 定位本事务写入的索引条目，而 DELETE 与 REPLACE 删除段记录的是墓碑 slot（`delete.rs:147-151`、`upsert.rs:581-587`），墓碑从不入索引 → 恒为 `None` → 整个条目还原被跳过（`manager.rs:302-305` 注释已自认）。Plan Review 给出的三个路线中，用户 2026-09-26 裁定**并入当前 change**（`replan-required`）：该扩围改变验收边界与诊断边界，不作为当前 Cycle 修复或 rework 处理。

**Investigation Facts**

- Current Baseline: 父 Cycle Act Response（2026-09-25）——全量 `cargo test -- --test-threads=1` 1231 passed / 0 failed / 2 ignored exit 0、`cargo clippy --all-targets` 0 warning、改动面 fmt 零漂移、`openspec validate --strict` valid。Plan Review 只读基线检查确认覆盖范围表面自该运行零变化（源文件 mtime 全部早于构建产物），结论未失效、直接采信；本次 replan 涉及表面（`transaction/manager.rs`、`wal/{mod,recovery}.rs`、两 README、测试）自该结论零变化。
- Current-State Evidence（本会话独立追读，位点在案）：
  - `abort_cleanup_versions`（`manager.rs:271-335`）：逐 rid `read_version_header` → `find_key_by_row_id` 命中则按 `next_version()` `update`、否则 `delete`（:292-300）；唯一索引同型循环（:306-314）；随后 `update_version_header_in_data_page(..., header.mark_aborted(), &[])` 中性化（:329-330，`mark_aborted` = `create_tx_id = 0` + `commit_tx_id = DELETED_TX_ID`，`version_chain.rs:81-90`）。记录集合来自 `record_version`（:176-184，按表聚合为 `HashSet<RowId>`，**迭代顺序不确定**）。
  - 墓碑记录点：`delete.rs:105-132` 写墓碑（`with_next_version(rid).mark_deleted()`）→ :147-151 `record_version(tx, table, tombstone_rid.unwrap_or(rid))` → :158-165 WAL `Delete{row_id: 原 rid}`；REPLACE 镜像段 `upsert.rs:531-599`（:581-587 记录墓碑，:589-596 WAL Delete）。墓碑 slot 不含元组（`write_tuple_to_data_page(..., &[])`）。
  - 键不可从索引反查：`IndexManager::delete`（`btree/index_manager.rs:272-293`）删除 B-Tree 条目的同时 `row_to_key.write().await.remove(&row_id)`；`find_key_by_row_id`（:352-354）只读该反向映射 → 删除后前驱版本的键无任何索引侧来源。
  - 取键先例：`wal/recovery.rs:227-257` `extract_index_keys(table_meta, unique_cols, tuple_data) -> (Option<Vec<u8>>, Vec<Option<Vec<u8>>>)`——一次 `deserialize_tuple` 兼提 PK 键（`values[pk_index].to_key()`）与各唯一列键，`Err` 时返回全 None。当前为模块私有；`wal/mod.rs:9` 为 `mod recovery;`（私有模块）。
  - 链与页读原语：`VersionHeader::{create_tx_id, next_version, is_deleted}`（`version_chain.rs:31-46`/`:88-90`）；`read_tuple_from_data_page`（`storage/data_page.rs`）单次页读同时返回 `VersionHeader` 与元组字节，slot 缺失返回 `StorageError::SlotNotFound(row_id)`。
  - 语句级回滚接线：`pipeline.rs:117-186` DML 臂在 `Response::Error` 时 `transaction_manager.abort(tx, &buffer_pool, abort_tables)`（:174-186），`abort_tables` 自 `get_table(table_name)` 解析（:132-137）→ 失败 REPLACE 的删除段确由该 abort 清理。
  - `abort` 写 WAL `AbortTxn`（`manager.rs:145-148`）→ 崩溃恢复按已回滚事务处理（`mark_uncommitted_aborted`，MS09 既有通道），恢复期 `redo_count > 0` 走 `rebuild_pk_indexes` / `extract_index_keys` 重建 → 修复不新增 WAL 类型、不触碰恢复通道语义。
  - 目标缺陷的两类可观察错误结果（父 Cycle Review 独立复现，exit 码与原生输出判定；`/tmp` 临时库已清理）：
    ```text
    $ rtsql /tmp/audit25/r.db "CREATE TABLE t (id INT PRIMARY KEY, code INT UNIQUE)"
    $ rtsql /tmp/audit25/r.db "INSERT INTO t VALUES (1, 100)"
    $ rtsql /tmp/audit25/r.db "BEGIN; DELETE FROM t WHERE id = 1; ROLLBACK"
    $ rtsql /tmp/audit25/r.db "SELECT * FROM t"                 # [[1,100]] 行复现
    $ rtsql /tmp/audit25/r.db "SELECT * FROM t WHERE id = 1"    # [] PK 点查漏行
    $ rtsql /tmp/audit25/r.db "INSERT INTO t VALUES (2, 100)"   # affected_rows 1 —— UNIQUE 静默失效
    $ rtsql /tmp/audit25/r.db "SELECT * FROM t"                 # [[1,100],[2,100]] 两条存活行共享 code=100
    ```
  - 失败 REPLACE 路径（父 Cycle Review 复现）：`REPLACE INTO t VALUES (1, 'abc')` → exit 3 `column 'n' expects INT, got String`（`insert_row` 类型门），删除已发生 → 扫描可见原行但 `WHERE id = 1` 漏行。
  - `insert_row` 缺口定位：`upsert.rs:218-234` 仅 UNIQUE 预检（:219-234）+ 类型门（:236-254），其后 serialize → 落位 → visibility → WAL → `record_version` → PK 条目（:288-293）→ 唯一条目（:295-299）；`InsertExecutor` 对应序列在 `insert.rs:177-187`（PK 预检）/`:195-210`（UNIQUE）/`:212-236`（类型门），PK 预检在 upsert 中仅由 `arbitrate`（:150-154）承担。
  - 仲裁形态对修复的约束：`All`（:148-172）PK 先、唯一列按序；`Column(pk_index)`（:173-186）只查 PK；`Column(unique)`（:187-211）只查该唯一列。REPLACE 恒 `All`（`ddl_dml.rs:build_upsert_action` 的 `(None, true)` 臂）→ 冲突行必先删除，故新增 PK 预检对 REPLACE 自然放行。
  - 受影响既有测试面：`tests/explicit_tx_test.rs`（MS07-T04 显式事务 8 例，回滚断言主阵面）、`tests/constraint_enforcement_test.rs`（唯一索引 + 恢复两态）、`tests/mvcc_tombstone_visibility_test.rs`（墓碑回滚扫描中性化，R3 主阵面）、`tests/upsert_test.rs::explicit_tx_replace_rollback_restores_row`（当前断言扫描复现并以注释记录 PK 点查丢失，修复后需更新断言与注释）、`tests/cli_test.rs`（回滚/会话面）。
- Code and Critical Path: 计划期无新增（2.7 只改执行期；2.9 只改回滚清理）。执行期 `UpsertExecutor::next` → `prepare_row` → `arbitrate` → 分派；无冲突路径 `insert_row` 新增 PK 预检位（`upsert.rs` :218 起，UNIQUE 预检之前）。回滚期 `TransactionManager::abort` → `abort_cleanup_versions` 新增两趟划分与 B 趟还原（`manager.rs` :289-331），B 趟经 `read_tuple_from_data_page` 回溯链并调 `wal::recovery::extract_index_keys` 取键后 `index_manager.insert` / `uindex.insert`。

**Implementation Guidance**

顺序：2.7（单点补齐，先 RED 后 GREEN，测试见证最短）→ 2.9（先建回滚矩阵观察 RED，再实现；实现顺序为 A/B 分趟 → 链回溯 → 取键还原 → 可见性提升 → 复用 `extract_index_keys`）→ 2.8（依赖 2.9 的实测结果）→ 2.10（收口）。

技术细节：
- 2.7 的预检必须复用 `row_values[self.pk_index].to_key()` 的 `Option` 语义（`None` = 无键行，跳过），位置在 `insert_row` 的 UNIQUE 预检循环之前、`compute_tuple_size` 之前，错误用既有 `StorageError::DuplicateKey`（不新增变体、不新增文案）。
- 2.9 的两趟划分必须在任何索引写入前一次性完成（`row_ids` 先按 `find_key_by_row_id(rid).await.is_some()` 分区），避免 A 趟的移除抹除 B 趟的还原；A 趟逻辑与顺序保持现状。
- B 趟还原目标 = 从墓碑 `next_version()` 出发、跳过 `create_tx_id() == tx_id` 的版本后的首个版本；该版本 `is_deleted()` 时无还原（防御）；链上不存在则无还原（插入后同事务删除的新行形态）。回溯需有迭代上限与 `SlotNotFound` 终止。
- 键派生复用 `extract_index_keys`（提为 `pub(crate)` + `wal/mod.rs` 暴露模块，行为零变化）；还原用 `insert`（条目已在删除时移除）而非 `update`；`to_key()` 为 `None`（无键行 / NULL 唯一值）自然跳过。
- 中性化（`mark_aborted`）保持在每 rid 处理的末尾，既有次序不变。
- 2.8 只在实测证伪时才改 README 措辞；不写实现未交付的承诺。

**Behavioral Change**

- 当前：`ON CONFLICT (唯一列)` 目标下 PK 冲突静默写入重复主键行并覆盖 PK 条目（原行点查漏行）；删除者事务回滚后被删行不复原 PK 与唯一索引条目（PK 点查漏行；唯一值被释放后可再次插入，`UNIQUE` 静默失效）；失败 REPLACE 留下同类索引缺口。
- 目标：显式目标仅仲裁该约束、仲裁外约束（PK 或唯一）一律以既有 `DuplicateKey` 零副作用拒绝；回滚后索引状态与「删除未发生」一致（PK 点查可达、唯一值仍被复现行占用、同事务 update→delete 指向更新前版本、处理顺序与集合顺序无关）；恢复两态与提交路径不变。
- 接口/错误语义：不改任何公开签名；`abort_cleanup_versions` 签名不变；`wal::recovery` 与 `wal::recovery::extract_index_keys` 仅可见性提升（`pub(crate)`）；新增错误面为零（复用 `DuplicateKey`）；slot 读取失败为跳过而非报错。

**Task Contracts**

### 2.7: `insert_row` 补 PK 重复预检

- Requirement/Scenario: R3（显式目标与仲裁外约束）、R4（显式目标仅仲裁该约束）
- Depends on: None
- Targets: `src/executor/upsert.rs::UpsertExecutor::insert_row`；`tests/upsert_test.rs`
- Current behavior: 无冲突插入路径不校验 PK 重复；`ON CONFLICT (code) DO NOTHING` 遇 PK 冲突时 exit 0 并写入重复主键行、覆盖 PK 条目
- Required behavior: `insert_row` 在 UNIQUE 预检之前执行 PK 重复预检——`row_values[pk_index].to_key()` 为 `Some(key)` 且 `table_meta.index_manager.search(key.as_bytes())` 命中即返回 `Err(StorageError::DuplicateKey)`；`None`（无键行）跳过
- Required changes: 上述预检块（镜像 `src/executor/insert.rs:177-187` 的位置与错误）
- Preserve: `All` 与 `Column(pk_index)` 仲裁命中路径（不入 `insert_row`）；REPLACE 臂（冲突行已删、条目已清，预检自然放行）；`insert.rs` / `update.rs` / `delete.rs` 零改动；既有错误面优先级与文本（PK DuplicateKey 先于 UNIQUE DuplicateKey，与 `insert.rs` 同序）
- Forbidden: 不改 `arbitrate` / `apply_do_update` / `delete_conflict_row` / 三动作分派；不新增 WAL 类型；不改 `InsertExecutor`；不把预检移出执行期
- Test witness: `tests/upsert_test.rs` 两例——`explicit_unique_target_with_pk_conflict_rejected`（DO NOTHING 臂）与 `explicit_unique_target_with_pk_conflict_do_update_rejected`（DO UPDATE 臂），均断言 `Duplicate key`、表行数不变、原行经 `WHERE id = 1` 点查仍产出原值；RED 先行（当前实现返回 `affected_rows` 1 且行数增加）
- GREEN condition: 两例通过且 upsert_test 既有 29 例全绿
- Verification: `cargo test --test upsert_test`，退出码 0
- Stop when: 预检位置与既有门优先级冲突，或需要改动 `insert.rs` / 仲裁结构

### 2.9: 回滚后墓碑行索引条目还原

- Requirement/Scenario: R6 S1-S7（`mvcc-tombstone-visibility` ADDED）、R3' 8 场景（`sql-constraint-enforcement` MODIFIED R3）
- Depends on: None（与 2.7 互不依赖）
- Targets: `src/transaction/manager.rs::TransactionManager::abort_cleanup_versions`；`src/wal/recovery.rs::extract_index_keys` 与 `src/wal/mod.rs`（可见性）；`tests/explicit_tx_test.rs`；`tests/upsert_test.rs::explicit_tx_replace_rollback_restores_row`
- Current behavior: 墓碑 slot 无索引条目 → 整段还原跳过；回滚后 PK 点查漏行、唯一值可被再次插入
- Required behavior: 记录集合先按「当前是否持有索引条目」一次性划分为 A 趟（有条目，现状回退/移除）与 B 趟（墓碑，还原）；B 趟从墓碑 `next_version()` 回溯跳过 `create_tx_id() == tx_id` 的版本，取首个非本事务版本为还原目标（`is_deleted()` 或不存在则无还原），经 `read_tuple_from_data_page` 读其元组并用 `extract_index_keys` 取 PK 键与各唯一列键，逐项 `insert(key, 目标 rid)` 还原；反序列化失败或 slot 缺失跳过该行不报错；A 趟逻辑、中性化次序与 `mark_aborted` 语义不变
- Required changes: `abort_cleanup_versions` 的两趟结构与 B 趟还原；`extract_index_keys` 与 `wal::recovery` 模块的 `pub(crate)` 可见性
- Preserve: 既有 INSERT 回滚（同值可重插）、UPDATE 回滚（原值恢复、条目回退前驱）、墓碑回滚扫描中性化；提交路径、恢复重放与 `redo_count > 0` 重建通道；`record_version` 聚合结构与 `tx_versions` 数据形状；`VersionHeader` 布局；DELETE / INSERT / UPDATE 执行器文件零改动
- Forbidden: 不改删除时的索引移除时机（运行期索引语义）；不改 `delete.rs` / `insert.rs` / `update.rs` / `upsert.rs`；不新增 WAL 记录类型；不改 `rebuild_pk_indexes` / `extract_index_keys` 的键派生规则（仅可见性）；不以报错替代 slot 缺失的跳过
- Test witness: `tests/explicit_tx_test.rs` 六例（DELETE 回滚后 PK 点查可达 / DELETE 回滚后唯一值仍被占用 / REPLACE 回滚后原行完整复现 / 同事务 update→delete 回滚指向更新前版本 / 失败 REPLACE 语句回滚无残留 / 回滚后干净重开两态一致）+ `tests/upsert_test.rs::explicit_tx_replace_rollback_restores_row` 更新为点查断言；RED 先行（当前实现 PK 点查为空、唯一值可重用）
- GREEN condition: 六例通过；`constraint_enforcement_test`、`mvcc_tombstone_visibility_test`、`explicit_tx_test` 既有断言零校准
- Verification: `cargo test --test explicit_tx_test --test upsert_test --test constraint_enforcement_test --test mvcc_tombstone_visibility_test`，退出码 0
- Stop when: 还原需要改动删除时序或运行期索引语义，或两趟划分仍无法消除同键形态的顺序依赖

### 2.8: README 写面段复核

- Requirement/Scenario: R5（文档与实现一致）
- Depends on: 2.9
- Targets: `README.md:197`、`README.zh-CN.md:197`（写面段）
- Current behavior: 表述称「所有写入都在触行之前完成校验，被拒绝的写入零副作用」；修复前被失败 REPLACE 证伪
- Required behavior: 2.9 落地后按实测复核该表述在 INSERT / UPDATE / DO NOTHING / DO UPDATE / REPLACE 五个面上是否成立；成立则保持原文，不成立则按实测收口措辞并同步双语
- Required changes: 仅在复核证伪时修改写面段首句（不新增未来承诺、不改其他章节）
- Preserve: 写面段其余条目、约束执行段、加密段与其余文档
- Forbidden: 不承诺未交付能力；不改其他 README 章节；不以措辞替代行为修复
- Test witness: 2.9 的「失败 REPLACE 语句回滚无残留」用例即行为见证（点查可达 + 扫描单行）；文档改动本身由 2.10 的 `git diff -- README.md README.zh-CN.md` 复核
- GREEN condition: 表述与实测行为一致（要么保持，要么双语同步收口）
- Verification: `cargo test --test explicit_tx_test` 退出码 0 + 文档 diff 复核；无文档改动时以 2.9 行为用例为唯一依据
- Stop when: 复核发现除 D9 之外仍有未覆盖面导致表述无法成立

### 2.10: replan 收口验证

- Requirement/Scenario: R3 / R5 / R6 / R3' 全部
- Depends on: 2.7, 2.8, 2.9
- Targets: 全量验证与 change 产物自检（`openspec/changes/2026-09-25-ms24-write-surface-completion/`）
- Current behavior: 2.7-2.9 完成后无收口判定
- Required behavior: 目标测试 → 受影响既有边界（约束面 / 墓碑可见性 / 显式事务 / CLI）→ 全量 `cargo test` 0 failures（计数 = 1231 + 净增用例，ignored 不增）；`cargo clippy --all-targets` 0 warning；改动面 `cargo fmt --check` 零漂移；`openspec validate <change> --strict` 通过；change 结构自检（tasks 勾选与实现一致、specs/design 与实现一致、两个 Cycle 文件齐全且状态自洽）
- Required changes: tasks 2.7-2.10 勾选与本 Cycle Act Response 写入
- Preserve: MS23 未提交区的既有 fmt 漂移（按 Surgical Changes 保持原样，单独记录）
- Forbidden: 不为通过而弱化断言；不删防回归意图；不新增身份型证据工程或判定层
- Test witness: 各命令原生输出与退出码
- GREEN condition: 上述全部通过
- Verification: `cargo test --test upsert_test --test explicit_tx_test --test constraint_enforcement_test --test mvcc_tombstone_visibility_test --test cli_test` + `cargo test` + `cargo clippy --all-targets` + `cargo fmt --check` + `openspec validate 2026-09-25-ms24-write-surface-completion --strict`，退出码均为 0
- Stop when: 出现无法归因本 change 的回归（BASELINE-CHANGED 返回 Plan）

**Invariants**

- `src/executor/{insert,update,delete}.rs` 零改动；UpsertExecutor 的仲裁、三动作分派与写形状不变（2.7 只在 `insert_row` 增一个前置校验块）。
- 既有错误面优先级与文本逐字节不变（`KeyTypeMismatch` / `NullConstraintViolation` / F1 守卫 / `DuplicateKey` / `ColumnTypeMismatch` / 既有计划期拒绝）。
- 无新 WAL 记录类型；`abort` 的 `AbortTxn` 时序、提交路径与恢复重放通道不变。
- 记录集合的迭代顺序不得影响回滚终态（两趟划分在任何索引写入前完成）。
- 存活版本键值不可读时跳过还原而非报错（与既有 SlotNotFound 容忍同型）。
- DML 不进 plan cache；`PlanBuilder` 公共 API 既有签名不变。
- 不建身份型证据工程；验证用原生 `cargo` / `openspec` 命令的退出码与输出。

**Non-goals**

- DO UPDATE WHERE；`INSERT OR ...` 方言迁移；算术/函数赋值表达式；组合唯一约束；多列 UPDATE SET 语句。
- 提交路径与恢复期索引重建策略的变更；删除时延迟移除索引条目等运行期索引语义改写。
- 既有 DELETE 语句成功路径的任何行为变化（仅回滚后的索引终态修正）。
- MS24-T04（DEFAULT 声明期类型校验）——仍为 tasks.md 独立任务行，不并入本 Cycle。

**Acceptance**

R3：显式唯一列目标 + PK 冲突以既有 `DuplicateKey` 零副作用拒绝（2.7 两例见证；DO NOTHING 与 DO UPDATE 两臂）。R5：既有写面语义零回归（三执行器文件不动 + 全量零校准 + README 表述与实测一致）。R6：`mvcc-tombstone-visibility` ADDED 的 S1-S7 经 `explicit_tx_test` 六例 + `upsert_test` 更新用例 + `constraint_enforcement_test` / `mvcc_tombstone_visibility_test` 零回归见证。R3'：`sql-constraint-enforcement` MODIFIED R3 的唯一性强制与既有 8 场景零校准，新增「DELETE / REPLACE 回滚后同值被复现行占用」场景经 R6 S2 见证。映射见 change tasks.md RTM（R3/R5/R6/R3' 行）。

**Verification**

- 2.7 / 2.9 目标矩阵：`cargo test --test upsert_test --test explicit_tx_test` 退出码 0。
- 扩围零回归：`cargo test --test constraint_enforcement_test --test mvcc_tombstone_visibility_test --test explicit_tx_test` 退出码 0。
- CLI 与既有边界：`cargo test --test cli_test` 退出码 0。
- R5：`cargo test` 全量 0 failures（计数 = 1231 + 净增用例，ignored 不增）。
- 静态与结构：`cargo clippy --all-targets` 0 warning；改动面 `cargo fmt --check` 零漂移；`openspec validate 2026-09-25-ms24-write-surface-completion --strict` 通过。
- 全部为原生命令输出与退出码直接判定，无封装判定层、无身份型证据工程。

**Gate 2 Readiness**

- 无 Missing requirement：RTM 七行全 Covered（tasks.md），新增 R6 / R3' 两行已建立 requirement ↔ design ↔ task ↔ code ↔ test 链路。PASS
- 无未批准 Simplified：用户 2026-09-26 显式裁定并入当前 change（proposal 范围裁定段与 tasks.md 头部在案）；无其他 Simplified。PASS
- 调查完整：Current-State Evidence 全部来自本会话对当前工作树与父 Cycle 产物的独立追读（`abort_cleanup_versions` / 墓碑记录点 / `IndexManager::delete` 反向映射清除 / `extract_index_keys` 先例 / `read_tuple_from_data_page` 原语 / pipeline abort 接线 / `insert_row` 缺口位点）；基线 1231 tests 采信父 Cycle Act Response（表面零变化经只读基线检查核对）。缺陷的两类可观察错误结果由父 Cycle Review 独立复现（原生 CLI 输出在案）。PASS
- 设计闭合：D9 行为/接口/错误/兼容语义完整（两趟划分、链回溯、取键还原、slot 缺失跳过、可见性提升范围），四个替代方案与否决理由在案；D10 测试布点明确；无契约语义 TBD。PASS
- 任务可执行：2.7-2.10 每个 Task Contract 有 Targets/Current/Required/Preserve/Forbidden/witness/GREEN/Verification/Stop。PASS
- 分轮合理：Iteration 001 计划已按扩围修订（tasks.md Iteration Plan 含修订后的 Stable/Verification/Diagnostic 边界、Non-goals 与平衡审计，拆分候选与用户裁定在案）；replan Cycle 使用更新后的全局任务。PASS
- 追踪完整：RTM 七行 R↔S↔D↔Task↔Code↔Test 链路闭合。PASS
- 验证充分：全部 scenario（含 sad path：唯一值释放、顺序依赖、失败语句回滚、链上无存活版本；edge：无键行、slot 缺失、update→delete 同事务）映射到测试命令；最简直接判定；无身份型证据工程/判定层。PASS
- Act 自包含：本 Cycle 含全部实现事实、复现证据、设计决策与契约，Act 无需回读父 Cycle / Assistant / Analysis。PASS
- 非实质未知项入 Risks：链回溯的迭代上限取值、slot 缺失在损坏库中的实际占比——均为防御性参数选择，不迫使 Act 决定契约语义。PASS
- Persisted Evidence：none（见下）。PASS
- 计划获批：change 级裁定与本次扩围批准在案（用户 2026-09-26 明确选择 replan 并入；Gate 1 范围扩围已批准，Gate 2 逐项 PASS）。PASS

**Persisted Evidence**

- Mode: none

`none` 依据：全部验证为 `cargo test` / `cargo clippy` / `cargo fmt --check` / `openspec validate` 原生命令，输出与退出码由 Act Response 承载；复现证据为原生 CLI 的 exit 码与输出（已记录在本 Plan Context，无不可低成本重跑的现场）。

- Budget: 按 Evidence 预算执行（公共规则 › 验证）。

**Risks and Notes**

- 链回溯的迭代上限：取常量上限（实现自定），仅防御损坏库的无环性；正常链长受版本链深度限制。
- `is_deleted()` 的存活目标防御：正常路径不可达（已删行的条目已不存在，删除走不到），保留为防御分支。
- `find_key_by_row_id` 判定分桶发生在任何索引写入之前，是顺序无关性的唯一保证点；若实现改为边遍历边判定，B 趟还原会被 A 趟移除抹除（Gate 4 必查）。
- `extract_index_keys` 提为 `pub(crate)` 属跨模块可见性扩大：行为零变化，仅键派生单一来源；`wal/mod.rs` 的 `pub use recovery::RecoveryManager` 等既有导出不变。
- MS23 未提交区既有 fmt 漂移（`delete.rs` 等 5 文件）按 Surgical Changes 保持原样，由 2.10 单独记录。

## Act Response

- Status: reported

### 交付任务

2.7、2.9、2.8、2.10 全部完成（replan Cycle 的全部 task）。父 Cycle 的两项阻塞发现分别由 2.7 与 2.8 关闭，根因缺陷由 2.9 修复。

### 实际改动与文件符号

| 任务 | 文件 | 符号 / 位置 | 改动 |
|---|---|---|---|
| 2.7 | `src/executor/upsert.rs` | `UpsertExecutor::insert_row`（:227-238 新增块，函数头 :215-226 文档同步） | UNIQUE 预检之前补 PK 重复预检：`row_values[pk_index].to_key()` 为 `Some(key)` 且 `index_manager.search(key)` 命中即 `Err(StorageError::DuplicateKey)`；`None` 跳过。镜像 `insert.rs:177-187` 的位置与错误，无新增变体与文案 |
| 2.9 | `src/transaction/manager.rs` | `MAX_TOMBSTONE_CHAIN_WALK`（:14-16）、`TransactionManager::abort_cleanup_versions`（:282-371）、`restore_tombstone_index_entries`（:395-457 新增模块级私有函数） | 记录集合先按 `index_manager.find_key_by_row_id(rid).is_some()` 一次性划为 `indexed`（A 趟，逻辑与次序逐字保持现状）与 `tombstones`（B 趟）；B 趟从墓碑 `next_version()` 回溯（单次页读经 `read_tuple_from_data_page` 同取版本头与元组），跳过 `create_tx_id() == tx_id` 的版本，取首个非本事务且非墓碑的版本为还原目标，经 `extract_index_keys` 派生 PK 键与各唯一列键后逐项 `insert`；随后对该墓碑执行同型 `mark_aborted` 中性化 |
| 2.9 | `src/wal/recovery.rs` | `extract_index_keys`（:232） | `fn` → `pub(crate) fn` + 注释说明复用性质；键派生规则零变化 |
| 2.9 | `src/wal/mod.rs` | 模块声明（:12-14） | `mod recovery;` → `pub(crate) mod recovery;`；既有 `pub use` 导出不变 |
| 2.8 | `README.md` / `README.zh-CN.md` | 写面段首句（各 :197） | 收口措辞，见下「Deviations」第 3 条 |
| 2.7/2.9 | `tests/upsert_test.rs` | 新增 `explicit_unique_target_with_pk_conflict_rejected`、`explicit_unique_target_with_pk_conflict_do_update_rejected`；更新 `explicit_tx_replace_rollback_restores_row` | 2.7 两例按契约 RED 先行；`explicit_tx_replace_rollback_restores_row` 按契约更新为点查断言，并把末尾「同唯一值可复用」改为「同唯一值报 DuplicateKey」 |
| 2.9 | `tests/explicit_tx_test.rs` | 新增 6 例（:404-673） | R6 S1-S6 矩阵，见「测试见证」 |

未改动（契约 Forbidden 核对）：`src/executor/{insert,update,delete}.rs`、`src/parser/planner/*`、`src/pipeline.rs`、WAL 记录类型、`rebuild_pk_indexes` 与 `extract_index_keys` 的键派生规则、`VersionHeader` 布局、`record_version` 聚合结构与 `tx_versions` 数据形状。

### Deviation from Plan

1. **2.9 场景 5 的测试输入与 Plan Context 记录的复现命令不同（Act 修正观察，非契约偏离）**。Plan Context 与父 Cycle Review 记录的复现为 `REPLACE INTO t VALUES (1, 'abc')`（唯一列收 String）。实测该输入在 `arbitrate` 的 F1 守卫处即被 `KeyTypeMismatch` 拒绝——**早于 `delete_conflict_row`，删除尚未发生**，因此不覆盖「先删后校验」路径（首次观察：该用例首跑即 GREEN）。改用非键 VARCHAR 列收 Int（`REPLACE INTO t (id, code, note) VALUES (1, 10, 123)`），使失败落在 `insert_row` 一般类型门（删除已发生后）。契约要求的行为见证（点查可达 + 扫描单行）不变，另增「原唯一值仍被占用」断言。RED 基线在改写后逐例确认（点查 `[]`、同值插入 `AffectedRows 1`）。CLI 独立复核（checkpoint 后跨进程）：扫描 `[[1,10,"keep"]]`、点查 `[[1,10,"keep"]]`、同唯一值插入 exit 3 `Duplicate key`。
2. **`restore_tombstone_index_entries` 实现为模块级私有函数而非 `TransactionManager` 私有方法**。Plan Context 表述为「私有方法」；初版写成 `&self` 方法但不使用 `self`（clippy 默认 lint 不报），Gate 4 code quality review 记为 Minor 并改为模块级函数消除未使用的接收者。位置仍在契约 Targets（`src/transaction/manager.rs`）内，行为等价。
3. **2.8 判定为「部分不成立」→ 按实测收口首句**。契约判定式为「该表述在五个面上是否成立」。实测：「被拒绝的写入零副作用」在 INSERT / UPDATE / DO NOTHING / DO UPDATE / REPLACE 五面全部成立（2.9 落地后）；但前半机制描述「所有写入都在触行之前完成校验」对 REPLACE 不成立——REPLACE 按 design D5 步骤 6 先删冲突行再校验插入序列，删除确已触行，终态一致由语句回滚 + 索引条目还原保证。故按契约「不成立则按实测收口措辞」改写首句，逐动作说明校验时机，不新增未交付能力的承诺，约束段与其余章节未动。
4. **A 趟的 `find_key_by_row_id` 每 rid 调用两次**（分桶一次、A 趟内一次）。分桶必须发生在任何索引写入之前（顺序无关性的唯一保证点），A 趟逻辑按契约「保持现状」逐字未改，故保留第二次查询。行为无差异，第二次为纯查表。

### Gate 4 Self-Review

**Spec compliance（逐契约）**

- 2.7 Required behavior 全部满足：预检位于 UNIQUE 预检与 `compute_tuple_size` 之前；复用 `to_key()` 的 `Option` 语义；错误为既有 `StorageError::DuplicateKey`（无新变体、无新文案）。Preserve 成立：`All` 与 `Column(pk_index)` 仲裁命中路径不入 `insert_row`；REPLACE 恒 `All` 仲裁、冲突行已删条目已清，实测 REPLACE 六例全绿；`insert.rs` / `update.rs` / `delete.rs` 零改动；错误面优先级 PK DuplicateKey 先于 UNIQUE DuplicateKey，与 `insert.rs` 同序。Forbidden 全部满足。
- 2.9 Required behavior 全部满足：两趟划分在任何索引写入前完成；B 趟回溯跳过本 tx 创建版本、目标 `is_deleted()` 或链上不存在时无还原；`read_tuple_from_data_page` + `extract_index_keys` 取键后 `insert` 还原；反序列化失败与 slot 缺失跳过不报错。A 趟逻辑、中性化次序与 `mark_aborted` 语义未变。Preserve 由零回归见证（INSERT 回滚同值可重插、UPDATE 回滚原值恢复、墓碑回滚扫描中性化、提交路径与恢复重放、`VersionHeader` 布局、`record_version` 结构）。Forbidden 全部满足：未改删除时索引移除时机、未改四个执行器文件、未新增 WAL 记录类型、未改键派生规则、未以报错替代跳过。
- 2.8 判定与改动见 Deviation 3；`git diff -- README.md README.zh-CN.md` 复核为写面段首句各一处，双语同步。
- Invariants 逐条成立：三执行器文件零改动（`git status` 确认无 unstaged 改动）；既有错误面优先级与文本未触碰（新增错误面为零）；无新 WAL 类型，`AbortTxn` 时序不变；顺序无关性由分桶位置保证；键不可读时跳过不报错；DML 不进 plan cache、`PlanBuilder` 签名不变；无身份型证据工程与判定层。
- Non-goals 未被侵入：DO UPDATE WHERE、`INSERT OR ...` 方言、算术/函数赋值、组合唯一约束、提交路径与恢复期索引重建策略、删除时索引移除时机、DELETE 成功路径行为（仅回滚后终态修正）、MS24-T04 均未触及。

**Code quality**

无新增警告（`cargo clippy --all-targets` 0 warning）、无死代码、无重复实现、无依据复杂度。命名与局部结构符合项目惯例（镜像 `insert.rs` / `delete.rs` 的既有写法与注释风格）。测试不会因错误原因通过：每例均含原值反向断言或行数不变断言，且 6 例与 2.7 两例全部经历 RED → GREEN。

**已修复发现（Gate 4 内）**

1. `restore_tombstone_index_entries` 的未使用 `&self` 接收者 → 改为模块级私有函数（Deviation 2）。
2. 改动面 5 处 rustfmt 漂移（`manager.rs` 1 处、`explicit_tx_test.rs` 4 处）→ 逐处修正，未触碰 MS23 既有漂移文件。

**遗留 Minor（不阻塞，不伪装为已解决）**

1. A 趟 `find_key_by_row_id` 的重复查询（Deviation 4）——契约为保持 A 趟现状而刻意保留，行为无差异。
2. 链回溯上限常量 64 为防御性取值，无测试覆盖损坏库的无环性形态（正常路径不可达，Plan Context 已列为 Risk）。

### 测试见证

| 任务 | 用例 | RED 观察 | GREEN |
|---|---|---|---|
| 2.7 | `explicit_unique_target_with_pk_conflict_rejected` | `AffectedRows { count: 1 }`（静默写入重复主键行） | ok |
| 2.7 | `explicit_unique_target_with_pk_conflict_do_update_rejected` | `AffectedRows { count: 1 }` | ok |
| 2.9 | `rollback_of_delete_restores_pk_point_lookup` | 点查 `[]` | ok |
| 2.9 | `rollback_of_delete_keeps_unique_value_occupied` | 同值插入 `AffectedRows { count: 1 }`（UNIQUE 静默失效） | ok |
| 2.9 | `rollback_of_replace_restores_row_fully` | 点查 `[]` | ok |
| 2.9 | `rollback_of_update_then_delete_restores_pre_update_version` | 点查 `[]` | ok |
| 2.9 | `failed_replace_statement_leaves_no_index_residue` | 点查 `[]`（唯一值占用断言随输入修正后同时 RED） | ok |
| 2.9 | `rollback_index_restore_survives_clean_reopen` | 重开后点查 `[]` | ok |

### Verification

| 验证项 | 命令 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| 2.7 目标 | `cargo test --test upsert_test` | `31 passed; 0 failed`（原 29 + 新增 2） | `upsert.rs::insert_row` PK 预检 + upsert 既有全矩阵 | PASS |
| 2.9 目标 | `cargo test --test explicit_tx_test` | `14 passed; 0 failed`（原 8 + 新增 6） | `manager.rs::abort_cleanup_versions` 两趟 + 回滚还原矩阵 | PASS |
| 扩围零回归 | `cargo test --test constraint_enforcement_test --test mvcc_tombstone_visibility_test` | `24 passed; 0 failed` / `11 passed; 0 failed` | 唯一索引强制与恢复两态、墓碑可见性 R1-R6 | PASS |
| 既有矩阵重复执行 | `cargo test --test upsert_test --test constraint_enforcement_test --test mvcc_tombstone_visibility_test --test explicit_tx_test -- --test-threads=1` | `24 / 14 / 11 / 31 passed; 0 failed` | 同上（Minor 修复后重跑） | PASS |
| R5 全量 | `cargo test -- --test-threads=1` | 82 个测试二进制，`passed=1239 failed=0 ignored=2`（基线 1231 + 净增 8，ignored 不增） | 全仓 | PASS |
| 静态 | `cargo clippy --all-targets` | 0 warning，exit 0 | 改动面 + 全 targets | PASS |
| 格式 | `cargo fmt --check` | 改动面零漂移；残留漂移仅 `delete.rs`(1) / `table_manager.rs`(18) / `recovery.rs:92`(1) / `cli_test.rs`(10) / `constraint_enforcement_test.rs`(6)，**全部为 MS23 未提交区既有漂移**，按 Surgical Changes 保持原样 | 改动面 | PASS |
| 结构 | `openspec validate 2026-09-25-ms24-write-surface-completion --strict` | `Change '...' is valid`，exit 0 | change 产物 | PASS |
| 行为复核 | 项目原生 CLI（`/tmp` 临时库，checkpoint 后跨进程） | 失败 REPLACE → 扫描 `[[1,10,"keep"]]`、点查 `[[1,10,"keep"]]`、同唯一值插入 exit 3 `Duplicate key` | README 首句措辞依据 | PASS |

全量命令说明：用户要求全量测试串行执行（并行会拖垮 WSL），本轮全量与目标矩阵均以 `--test-threads=1` 运行；临时库与日志已清理。

change 结构自检：tasks 2.1-2.10 与实现一致（本轮勾选 2.7-2.10）；`specs/sql-write-surface` R3/R4、`specs/mvcc-tombstone-visibility` ADDED R6、`specs/sql-constraint-enforcement` MODIFIED R3 与实现一致；两个 Iteration 目录与三个 Cycle 文件齐全；`Review Result` 与流程状态一致（001-replan 的 Review 待 Plan 填写，Act 不代填）。

### Persisted Evidence

None required —— 与 Plan Context 的 `Persisted Evidence: none` 一致。全部验证为原生命令的退出码与输出，已由上表承载；无不可低成本重跑的现场（缺陷复现可由确定性用例与 CLI 序列重跑）。

### Experience Candidates

1. **Issue 候选（范围外，既有缺陷）**：`upsert.rs::delete_conflict_row` 的 `SlotNotFound` 容忍臂（:567）在无元组可读时跳过 PK 条目删除，而同形态的 `delete.rs:134` 仍按搜索键执行 `index_manager.delete`。仅在索引条目指向不存在 slot 的夹具/损坏态可达，正常路径 rid 必有 slot。证据：父 Cycle Review 独立检查记录（`iterations/001-upsert-replace/000-initial.md` Plan Review Minor 4）。本 change 不修（越出 Diagnostic boundary）。
2. **Issue 候选（范围外，行为差异）**：仲裁内 F1 守卫（`upsert.rs:160-166` / `:203-209`）使「PK 冲突 + 唯一列非法类型值」的行报 `KeyTypeMismatch` 而非被 `DO NOTHING` 跳过——同输入的纯 INSERT 报 `Duplicate key`。方向安全（无静默写坏），但 R3「DO NOTHING 跳过冲突行」在该角落不成立。证据：父 Cycle Review Minor 5（含独立复现记录）。建议 change 收尾合并规格时作为已知边界记录；是否写入 spec 由用户裁定。

Act 不创建持久化产物；两项候选的落账由用户在授权后交 `openspec-experience-recorder`。

### 未解决问题

None。契约 R3 / R5 / R6 / R3' 的全部 scenario 均有测试见证，无遗留 Critical 或 Important 问题。

## Plan Review

- Review Result: accepted

**Findings**

独立审查（2026-09-26，Plan Review）。只读基线检查：全部 src/tests 源文件最新 mtime（`manager.rs` 15:41:09）早于最新构建产物（`upsert_test` / `explicit_tx_test` / CLI 二进制 15:41:19），`find -newer` 零命中——覆盖范围表面自 Act 验证运行后零变化，按公共规则 › 验证 采信 Act Response 的验证结论（全量 `cargo test -- --test-threads=1` 1239 passed / 0 failed / 2 ignored、`cargo clippy --all-targets` 0 warning、改动面 fmt 零漂移），不重复运行。独立重读 `upsert.rs`（全文）、`manager.rs::abort_cleanup_versions` 与 `restore_tombstone_index_entries`（全文）、`wal/mod.rs`、`wal/recovery.rs::extract_index_keys`、镜像源 `insert.rs` PK 预检区、`README.md` / `README.zh-CN.md` 写面段，以及 2.7 两例 + 1 更新例、2.9 六例测试正文。

阻塞 Acceptance 的发现：无。父 Cycle 两项阻塞发现均已关闭并经独立核实：

1. 阻塞发现 1（显式唯一列目标缺 PK 预检）→ 2.7 关闭：`insert_row`（`upsert.rs:226-236`）在 UNIQUE 预检与 `compute_tuple_size` 之前补 PK 重复预检——`to_key()` Option 语义（无键行跳过）、既有 `StorageError::DuplicateKey`、位置与错误镜像 `insert.rs:170-190`（本次独立对照）；`arbitrate` / `apply_do_update` / `delete_conflict_row` / 三动作分派零改动；两例见证（DO NOTHING 与 DO UPDATE 臂）均含行数不变与原行点查反向断言，RED 先行在案。
2. 阻塞发现 2（README 零副作用表述被证伪）→ 2.8 + 2.9 关闭：根因缺陷由 2.9 修复（见 Evidence），README 双语首句按实测收口为逐动作校验时机表述（REPLACE 明示「删除冲突行之后才校验失败时经语句回滚 + 条目还原」），与实现一致且由 `failed_replace_statement_leaves_no_index_residue` 与 CLI 跨进程复核承载；无未来承诺，其余章节未动。

非阻塞 Minor findings：

1. delta spec `mvcc-tombstone-visibility` R6 S3 场景文本：GIVEN 行值 `(1, 'Alice', 100)` 与其表定义 `t(id INT PRIMARY KEY, code INT UNIQUE)`（2 列）不一致（3 值对 2 列）——收尾合并主 spec 时校正；同场景 THEN 第三子句「按 `code = 300` 的插入不被该回滚行阻断」无直接断言见证（S1-S3 主断言与 upsert_test 变体的全表扫描单行已覆盖还原与无残留面，该子句由 A 趟移除语义与本次代码核实蕴含）——记录，收尾合并时可随场景文本一并校正或补断言，不阻塞。
2. 范围外既有缺陷（Issue 候选，本次代码读审查发现，非本 change 引入）：A 趟对 rekey UPDATE 回滚的条目回退不按 rekey 语义处理——`abort_cleanup_versions` A 趟以 `find_key_by_row_id` 所得**新键**执行 `update(key, prev)`（`manager.rs:318-329`），而 prev 版本 tuple 携带**旧键**；回滚一个改键 UPDATE 后新键条目指向旧键版本、旧键条目缺失，旧键等值点查漏行。与 R6 墓碑缺陷同面但不同形态；A 趟按契约「保持现状」逐字未改，`explicit_tx_test` 无 rekey 回滚用例（grep 核实）。落账由用户指令交 `openspec-experience-recorder`。
3. Act 自报遗留 Minor 维持：`delete_conflict_row` SlotNotFound 容忍臂不删 PK 条目（父 Cycle Minor 4）、仲裁内 F1 守卫角落（父 Cycle Minor 5，建议收尾合并规格时作已知边界）、A 趟重复查询（偏差 4）、链回溯上限 64 无损坏库测试（防御性参数）。

**Deviation Classification**

1. 2.9 场景 5 测试输入与 Plan Context 记录复现命令不同 — `PLAN-OMISSION`（非阻塞）。独立核实：唯一列收 String 的 REPLACE 在 `arbitrate` F1 守卫（`upsert.rs:160-166`）先于 `delete_conflict_row` 被拒，删除尚未发生，Plan Context 记录的复现命令不覆盖「先删后校验」路径；Act 改用非键 VARCHAR 列收 Int 使失败落在 `insert_row` 一般类型门，行为见证契约不变且新增「原唯一值仍被占用」断言，RED 基线逐例确认。修正正确。
2. `restore_tombstone_index_entries` 为模块级私有函数而非私有方法 — 非实质：等价控制流，位于契约 Targets 内，消除未使用接收者。
3. 2.8 判定「部分不成立」→ 按实测收口首句 — 非偏差：契约判定式本为「成立则保持、不成立则按实测收口」，Act 按契约执行；措辞与实现一致（本次逐句对照）。
4. A 趟 `find_key_by_row_id` 每 rid 调用两次 — 非实质：分桶在任何索引写入之前是顺序无关性的唯一保证点（`manager.rs:300-316`，本次核实），A 趟按契约逐字保持；第二次为纯查表。

**Acceptance Gaps**

None——父 Cycle 的两项 gap（R3 显式目标仲裁外约束、R5 文档一致性）均已关闭；R6 S1-S6 经 `explicit_tx_test` 六例逐场景见证（S7 既有回滚零回归经 constraint / mvcc / explicit_tx 套件承载）、R3' 新增回滚占用场景经 R6 S2/S3 见证且既有 8 场景零校准、R3/R4/R5 既有见证经零回归重复执行维持。RTM 七行与实现一致（本次逐行对账）。

**Convergence**

父 Cycle gap → replan Cycle 关闭：reduced → closed。无新增 gap，无 rework 迹象。

**Evidence**

- 代码独立重读：(2.7) `upsert.rs:215-236`（PK 预检块 + 文档同步）、`:146-213`（arbitrate 未动）、`:238-273`（UNIQUE 预检与类型门位置在其后）；(2.9) `manager.rs:11-16`（上限常量）、`:282-370`（两趟划分先于任何索引写入 + A 趟现状逐字 + B 趟还原与中性化）、`:385-442`（回溯跳过本 tx 版本、`is_deleted()` / 读失败 / 无前驱静默终止、`extract_index_keys` 取键后 `insert` 还原）、`wal/mod.rs:9-11`（`pub(crate) mod recovery`，既有导出不变）、`wal/recovery.rs:228-261`（仅可见性 + 注释；键派生体逐行核实：单次反序列化、PK/唯一键 `to_key` 派生、Err 全 None）；镜像源对照 `insert.rs:170-190`（PK 预检位置与错误一致）。
- 测试独立重读：upsert_test `:269-325`（2.7 两例，含行数不变 + 原行点查反向断言）、`:1090-1141`（REPLACE 回滚更新例：扫描单行 + 点查 + 唯一值占用）；explicit_tx_test `:417-676`（R6 S1-S6 六例，全部含反向断言；S5 用例注释准确记录 F1 守卫时序）。
- README 双语写面段逐句对照（首句收口措辞 + 其余条目未动）。
- 验证采信：Act Response Verification 表（2026-09-26）；采信依据为上述只读基线检查。禁改面核实：`insert.rs` / `update.rs` / `delete.rs` mtime 2026-09-25（Iteration 000 / MS23 后未动）、planner / pipeline mtime 14:03-14:45（replan Act 前）、`version_chain.rs` / `data_page.rs` 未动；`record_version` 聚合结构与 `VersionHeader` 布局未变（本次重读确认）。
- Persisted Evidence：none 模式核对——无 `evidence/` 目录属预期，不作为问题。
- change 结构自检：tasks 2.1-2.10 勾选与实现一致；四个 delta specs 与实现一致（R6 S3 场景文本笔误除外，Minor 1）；两个 Iteration 目录与三个 Cycle 文件齐全；本 Review 写入后 `Review Result: accepted` 与流程状态一致（001 为 change 最后 Iteration）。

**Follow-up Decision**

接受（`accepted`），无当前 Cycle 修复项。Iteration 001 完成，change `2026-09-25-ms24-write-surface-completion` 全部 Iteration 完成、具备收尾条件。跟进项（均不入本 Cycle）：

1. Issue 候选落账（Recorder，按用户指令）：Act 上报两项（`delete_conflict_row` SlotNotFound 容忍臂、仲裁 F1 守卫角落）+ 本次 Review 新增一项（A 趟 rekey UPDATE 回滚条目回退不按 rekey 语义，Minor 2）。
2. 收尾合并规格时（maintainer）：校正 R6 S3 场景 GIVEN 行值笔误；裁定「仲裁 F1 守卫角落」是否写入 spec 已知边界段。
3. MS24-T04（声明期 DEFAULT 类型校验）仍为 tasks.md 独立 planned 任务行，随后续 change 收口。
4. 工作树 fmt 漂移（MS23 未提交区 5 文件）建议独立批次；commit 时机由用户指令。

**Iteration Plan Update**

None（Iteration 001 计划已于 replan 时修订完毕，本次无进一步变化）

**Next Cycle**

None

**Next Iteration**

None（001 为 change 最后一个 Iteration；收尾由用户调用 openspec-docs-maintainer）
