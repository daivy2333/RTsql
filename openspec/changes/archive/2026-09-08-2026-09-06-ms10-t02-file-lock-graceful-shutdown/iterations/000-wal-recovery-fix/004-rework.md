# Iteration 000 / Cycle 004-rework: 恢复期索引去信任与重放后重建

## Plan Context

- Status: ready（Gate 2 修复面扩大经用户批准 2026-09-08，原话「更改gate，开始实施」；仅状态字段流转，Plan Context 正文不可改写）
- Iteration: 000-wal-recovery-fix（WAL 恢复逐帧无歧义与重放正确性）
- Cycle: 004-rework
- Cycle Type: rework
- Parent cycle: 003-rework（Act Response `blocked`；Plan Review `rework-required`，裁定 design D10）

> **Rework 注记**：父 Cycle 完成 R-T0b-R6（G5 扫描去重，独立验证 GREEN）与 R-T0b-R5 实现（catalog root 同步，见证 GREEN），但 G4 完整闭合被**撕裂树**阻塞（Plan 独立复现：裸读磁盘树 `scan_all` 184/10000 + `InvalidPageType` 洞页；catalog root 本身已正确）。本 Cycle 按 design D10 执行：`redo_count > 0` 时恢复路径完全不消费磁盘索引树——redo 经磁盘版本多映射派生、重放后从数据页重建 PK 索引。以 2 个新 repair item + R-T0b-R1 收口 + R-Gate 完成既有 Acceptance，不修改 Iteration Map。**修复面扩大（recovery 重放臂 + index_manager 收集 + table_manager 换入 API）需用户 Gate 2 批准**。

**Cycle Scope**

- Trigger: rework（父 Cycle Plan Review 裁定 D10 方向 a′）
- Acceptance gaps: R2-S1「混合负载恢复计数/语义精确」未满足——恢复重放消费撕裂磁盘树 → `update redo: old key not in index` 打开失败（R-T0b-R1 精确见证 RED 即阻塞项本体）
- Repair items: R-T0b-R7（redo 去索引化）、R-T0b-R8（重放后 PK 索引重建）、R-T0b-R1 收口、R-Gate——不作为新的全局 change task，不修改 Iteration Map
- Inherited scope: 父 Cycle 链（000-initial/001-replan/002-rework/003-rework）全部已落地成果——reader 修复（T0）、位置寻址重放（T0b）、B-Tree 三修复（G1-G3）、catalog root 同步（R5）、扫描替代集合去重（R6）、`btree_scale_test` 5 用例、`wal_recovery_large_test` 6 绿 + 1 阻塞 RED——全部保持，零回退
- Excluded scope: 锁/信号（Iteration 001）；WAL/页格式；`IndexScan` 路径；非 PK 索引；驱逐策略改造（撕裂树的运行期根修，improvement 候选）；`mark_tx_aborted` 空实现的补全（既有，见 Risks 4）

**Objective**

`redo_count > 0` 的打开（不洁关闭后恢复）不再依赖磁盘 B-Tree 的结构一致性：redo 阶段经数据页自建的版本多映射完成 `old_row_id` 派生，重放完成后从最终数据页重建各表 PK 索引并换入——混合负载（含中位点 checkpoint + 驱逐规模）崩溃恢复的行数/更新/删除/唯一性语义全部精确，且对重复 PK 损坏保持显式报错（K05）。

**Current Baseline**

- 工作区（未提交，位于 `590fdc6` 之上）：reader.rs（T0）、recovery.rs（T0b 位置寻址）、btree.rs + key.rs（G1-G3）、catalog.rs + index_manager.rs + table_manager.rs + database.rs（R5）、data_scan.rs（R6）、两个见证测试文件。
- 测试基线（Plan Review 3 独立复跑，2026-09-08）：`btree_scale_test` 5/5；`wal_recovery_large_test` 6 绿 + `mixed_dml_recovery_semantics` RED（`WalError("WAL redo failed: update redo: table 't' old key not in index")`）；pushdown 15 / projection 6 / executor 39 / prefetch 3 全绿；clippy 0 / fmt 0 / validate PASS。
- 阻塞实证（Plan 独立探针，2026-09-08）：mixed_dml 夹具崩溃后裸读（`FileStorage::open` + `Catalog::open` + `IndexManager::from_root`）——`catalog_root=155`（正确）、`scan_all`=184（期望 10000）、`collect_all_pages` 报 `InvalidPageType { expected: 0x1, actual: 0x0 }`。

**Current-State Evidence**（Plan Review 3 独立取证，2026-09-08）

- **redo 三臂的索引依赖点**（`src/wal/recovery.rs`）：Insert 臂 `:321` 判重搜索 + 插入；Update 臂 `:377-397` `extract_pk_key(old_tuple)` → `index_manager.search(&old_key)` → `old_row_id`（`.ok_or_else("old key not in index")` = 阻塞报错点）→ `:418` `index_manager.update(new_key, row_id)`；Delete 臂 `find_key_by_row_id` → `index_manager.delete`。三臂在撕裂基座上的任何读都不健全。
- **`RecoveryResult.redo_count` 已存在**（`recovery.rs:259-276`）：Step 2 重放循环计数，`records.is_empty()` 提前返回 default——触发信号现成，无需新机制。
- **`mark_uncommitted_aborted` 实为空转**（`buffer_pool.rs:369-371` `mark_tx_aborted` 为 no-op）：未提交/中止行的 header 保持 `commit_tx_id=None`——重建谓词据此排除即可，与 T0b「只重放已提交」的索引状态一致。
- **版本链方向 new→old**：新版本 header 携带 `with_next_version(old_row_id)`（`update.rs:98`）；「链尾」= 不被任何 slot 的 `next_version` 指向的 slot；同键版本链 rid 序 == LSN 序（同行并发写者被行锁串行化、提交序即执行序，WAL 按提交序写）。
- **重建谓词所需信息齐备**：header（`commit_tx_id`/`is_deleted`）+ WAL 分类集合（`committed_tx_ids`，`full_recover` Step 1 产出）——墓碑创作者是否已提交以 WAL 集合消解（见 R8 契约）。
- **既有先例**：`IndexManager::new` 经 `spawn_blocking` 构造（`table_manager.rs:241-244` create_table）；页释放 `buffer_pool.free_page` + 收集失败 warn 放弃（`drop_table:353-364`）；`collect_all_pages` 严格 DFS（`index_manager.rs:335`，遇洞 `InvalidPageType` 显式错）。
- **恢复后可见性机制**：恢复行 header 的提交编码由 redo 写入路径决定（Act 实现时核实 `redo_tuple_at_row_id` 的 header 构造与 `commit_tx_id` 语义，保证谓词与实际编码一致）。

**Relevant Code**

| 文件/符号 | 职责与本 Cycle 关系 |
|---|---|
| `src/wal/recovery.rs` | R7：redo 三臂去索引化 + 磁盘版本多映射；R8：重放后重建编排（`redo_count > 0` 门控） |
| `src/storage/btree/index_manager.rs` | R8：洞容忍页收集变体（父指针枚举洞 id）；换入实例的 catalog context 由既有 attach 覆盖 |
| `src/storage/data/table_manager.rs` | R8：重建/换入 API（tables map 写路径替换 `index_manager` Arc） |
| `src/storage/catalog.rs` | R8 复用 `update_table_root`（零修改） |
| `tests/wal_recovery_large_test.rs` | R1 收口（RED → GREEN）；既有 6 用例零修改 |
| `src/storage/page_format/*`、`src/wal/{writer,record,buffer,checkpoint}.rs` | **禁止修改**（格式冻结面） |

**Critical Path**

R7（多映射 + update 派生改道，解除对磁盘树的读依赖）→ R8（重放后重建 + 换入 + catalog 写回 + 旧树释放，依赖 R7 的最终数据页状态）→ R-T0b-R1 收口（依赖 R7+R8）→ R-Gate。

**Implementation Guidance**

1. **R-T0b-R7**：`full_recover` 在 `redo_count` 将 > 0 的前提下（判定：存在 `lsn >= redo_from` 的已提交 data 记录），于 Step 2 前对每表扫描数据页链构建 `HashMap<Vec<u8>, Vec<RowId>>`（键 = `extract_pk_key`；值 = rid 升序；页链遍历复用 buffer pool 读路径，slot 遍历同 `build_superseded_map` 形态）。Update 臂：`old_row_id` = 映射[key] 中 `< record.row_id` 的最大 rid（无候选或派生槽 tuple != old_tuple → `RedoFailed`）；写成功后向映射追加 `record.row_id`。Insert 臂：位置寻址写入后追加 rid（判重/索引插入删除）。Delete 臂：墓碑重放保留，索引清理删除。重放上下文（映射）随 `RedoContext` 或独立结构传递——形态留给 Act。
2. **R-T0b-R8**：Step 3 之后、返回前（门控 `redo_count > 0`）：每表 `IndexManager::new`（spawn_blocking）→ 扫描最终数据页链，按 PK 分组，自链尾（不被指向的 slot）沿 `next_version`（new→old）回溯取第一个「已提交 ∧ 非墓碑」版本；墓碑 slot 若其创作者 ∉ `committed_tx_ids` → 视为未提交删除，继续回溯（既有未提交删除崩溃语义）；否则止步（整行不建条目）→ `insert(key, rid)`，遇同 key 已存在 → `Err`（K05）。完成后：TableMeta 换入新实例（新公共或 `pub(crate)` 方法，形态留给 Act）→ `catalog.update_table_root(name, new_root)` → 旧树洞容忍收集 + `free_page`（收集错误 warn + 放弃）。`database.rs` 的 attach 时序不变（full_recover 返回后执行，自然附加到换入实例）。
3. 关键取舍（已定，D10）：redo 期零 B-Tree 依赖（任何部分消费都在撕裂基座上）；重建仅在 `redo_count > 0`（clean 打开零变化）；判重职责移至重建（K05 保持）；旧树释放容忍洞但不阻塞打开（泄漏显式记录，先例 drop_table）。非实质留给 Act：映射与上下文的具体类型、换入 API 签名、谓词编码核实方式、日志文案。

**Behavioral Change**

| 输入 | 现行为 | 目标行为 |
|---|---|---|
| `redo_count > 0` 打开（撕裂树形态） | `update redo: old key not in index` 打开失败 | 恢复成功；索引 = 最终数据页的已提交链尾精确重建 |
| `redo_count > 0` 打开（完好树形态） | 依赖磁盘树（当前恰好可用） | 同上（不再消费磁盘树，结果一致） |
| `redo_count == 0` 打开 | 消费磁盘树 | **零变化**（不进入 R7/R8 路径） |
| WAL 含重复 PK 损坏 | redo 判重显式报错 | 重建判重显式报错（K05 保持） |
| 运行期（非恢复）全部路径 | — | 零变化（R5/R6 既有面不动） |

**Change Surface**

| Repair item | Requirement/Scenario | File/Symbol | Planned Change |
|---|---|---|---|
| R-T0b-R7 | R2-S1 可用性 | recovery.rs（Insert/Update/Delete 臂 + full_recover 前置扫描） | redo 去索引化 + 多映射派生 |
| R-T0b-R8 | R2-S1 可用性 + 一致性 | recovery.rs（重建编排）+ index_manager.rs（洞容忍收集）+ table_manager.rs（换入 API） | 重放后 PK 索引重建 |
| R-T0b-R1 收口 | R2-S1 完整见证 | tests/wal_recovery_large_test.rs | 既有精确见证 RED → GREEN（零修改） |
| R-Gate | 全部 | 无新代码 | 四命令复跑 |

**Task Contracts**

### R-T0b-R7: redo 去索引化（D10）

- Requirement/Scenario: `wal-recovery-replay-integrity` R2-S1（恢复可用性）
- Depends on: None
- Targets: `src/wal/recovery.rs`（三臂 + `full_recover` 前置扫描与门控判定）
- Current behavior: Update 臂 `search(old_key)` 消费磁盘树 → 撕裂形态 `old key not in index`；Insert/Delete 臂在撕裂基座上判重/清理
- Required behavior: `redo_count > 0` 时三臂零 B-Tree 读写；`old_row_id` = 多映射 max{rid < record.row_id} + old_tuple 校验；重放追加保持映射新鲜
- Required changes: Implementation Guidance 1（约 80-120 行；`redo_record` 签名可加映射参数，非实质）
- Preserve: 位置寻址语义（T0b）、`RedoContext` 页链/tail 重建、M21 可见性维护、site/位点与事务分类语义、K05 显式报错、WAL/页格式
- Forbidden: 磁盘树消费（redo_count > 0 时）；`redo_count == 0` 路径行为变化；WAL/页格式变更
- Test witness（RED 先行）: 既有 `mixed_dml_recovery_semantics` 即 RED（阻塞形态实测在案）——本 item 落地后该用例**仍 RED**（重建未建，索引空）属预期中间态，Act 记录实际形态（预期：重开成功但唯一性探针失败或计数偏差——索引缺位）；独立 RED 见证 = `full_recover` 单测或既有用例中间态断言，形态留 Act 记录
- GREEN condition: 与 R8 合并验收（R1 见证全绿）
- Verification: `cargo test --test wal_recovery_large_test` 中间态输出记 Act Response
- Stop when: 同键版本链出现 rid 序与 LSN 序反例（max-rid 派生不可判定）→ 返回 Plan（需 WAL 携带 old_row_id 的格式决策）；或派生槽 tuple != old_tuple 在合法序列上出现 → 返回 Plan

### R-T0b-R8: 重放后 PK 索引重建（D10）

- Requirement/Scenario: R2-S1（恢复可用性 + 索引/数据一致）
- Depends on: R-T0b-R7
- Targets: `src/wal/recovery.rs`（重建编排）、`src/storage/btree/index_manager.rs`（洞容忍收集）、`src/storage/data/table_manager.rs`（换入 API）
- Current behavior: `redo_count > 0` 打开后索引 = 撕裂磁盘树（或缺失）；无重建路径
- Required behavior: 重放完成后每表重建（链尾回溯谓词 + 判重显式报错）→ 换入 → `update_table_root` → 旧树洞容忍释放
- Required changes: Implementation Guidance 2（约 100-150 行）
- Preserve: `redo_count == 0` 零变化；`attach_index_catalog_contexts` 时序（database.rs 不改）；K05；页格式/WAL 格式；drop_table 释放语义
- Forbidden: 恢复期 BTree context 附加（root 同步不走 redo）；同步阻塞刷盘；非 PK 索引
- Test witness（RED 先行）: R7 中间态（索引缺位）即本 item 的 RED——`mixed_dml_recovery_semantics` 在 R7 后仍 RED、R8 后 GREEN；唯一性探针（重复 INSERT 已存在 PK → DuplicateKey）与最小键可达由该用例既有断言覆盖
- GREEN condition: `mixed_dml_recovery_semantics` 全绿 + `recovery_rerun_is_idempotent` 保持绿（重建二次幂等）+ 既有 6 用例零回归
- Verification: `cargo test --test wal_recovery_large_test` 输出+退出码记 Act Response
- Stop when: 重建谓词在 Acceptance 工作负载上无法从 header + WAL committed 集合无歧义判定（编码冲突）→ 返回 Plan；或换入与既有 tables map 所有权冲突无法局部消解 → 返回 Plan

### R-T0b-R1 收口: R2-S1 完整见证转绿

- Requirement/Scenario: R2-S1 完整见证
- Depends on: R-T0b-R7, R-T0b-R8
- Targets: 无代码修改（见证已在位且为精确版、`#[ignore]` 已移除）
- Current behavior: RED（`old key not in index`）
- Required behavior: 全断言 GREEN——COUNT 精确 9950、被更新行 v=9999、被删行 0 行、重复 INSERT 最小键与中位点前键 DuplicateKey
- Preserve: 见证文件零修改（003 已按契约精确化）
- Forbidden: 任何容差；跳过失败语句
- Test witness: `cargo test --test wal_recovery_large_test mixed_dml` GREEN 输出
- GREEN condition: 见证绿 + 全量门
- Verification: 记 Act Response
- Stop when: R7/R8 落地后见证仍红且归因于本 Cycle 契约内无法消解的状态 → 返回 Plan

### R-Gate: 全量验证门

- Requirement/Scenario: 全部
- Depends on: R-T0b-R1
- Targets: 无新代码；`cargo test --all`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown`
- Current behavior: 白名单（3 信号用例）外除 `mixed_dml_recovery_semantics` 外全绿；0 ignored
- Required behavior: 白名单外全绿（含 mixed_dml 转绿）；clippy 0 / fmt 0 / validate PASS
- Test witness: 各命令决定性输出（≤20 行）与退出码
- Stop when: 白名单外回归失败且无法归因于 R7/R8 → BASELINE-CHANGED 返回 Plan

**Invariants**

- WAL 格式、页格式、catalog 结构、site/位点与事务分类语义零变化
- `redo_count == 0` 打开路径逐字节零变化；运行期（非恢复）路径零变化（R5/R6 既有面不动）
- K05：重建判重、派生校验、落位校验全部显式报错
- 既有测试断言语义零修改（见证文件本轮零修改）；4 文件夹具锁适配例外不变
- 不引入哈希/校验和/内容指纹；不新建 Evidence 占位目录

**Non-goals**

锁/信号（Iteration 001）、IndexScan 路径、非 PK 索引、WAL/页格式、do_flush 并发、驱逐策略改造（撕裂树运行期根修——improvement 候选）、`mark_tx_aborted` 空实现补全、多行 INSERT Page full（观察项）。

**Acceptance**

1. R2-S1 可用性：撕裂形态（中位点 checkpoint + 驱逐 + 混合 DML）崩溃恢复成功且 site 前条目可达——`mixed_dml_recovery_semantics` 全断言。
2. R2-S1 一致性：恢复后索引 = 最终数据页已提交链尾精确重建——同一见证的唯一性探针 + 计数断言。
3. 幂等：二次恢复收敛同一状态——`recovery_rerun_is_idempotent` 保持绿。
4. 回归门：白名单外全绿 + clippy 0 / fmt 0 / validate PASS——R-Gate。

**Verification**

- `cargo test --test wal_recovery_large_test`（7 用例全绿无 ignore）
- `cargo test --all`（白名单口径）+ clippy/fmt/validate
- 输出（每项 ≤20 行）+ 退出码记 Act Response

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | redo 三臂依赖点/触发信号/谓词信息源/先例全部代码级定位（Current-State Evidence）；撕裂树 Plan 独立复现 |
| Design | PASS | D10 已裁定（a′），拒绝备选有据（b 违 K05 / c 超范围）；无 TBD |
| Iteration Plan | PASS | Map 不变；repair item 形式；故障域（recovery + index 重建）集中 |
| Cycle Scope | PASS | 既有 Acceptance（R2-S1）必要条件；**修复面扩大（recovery 重放臂 + index_manager + table_manager），需用户 Gate 2 批准** |
| Task Contracts | PASS | 4 项均含 Targets/行为/Preserve/Forbidden/RED-GREEN/Verification/Stop |
| Traceability | PASS | R2-S1 可用性→R7+R8；一致性→R8+R1；幂等→既有 rerun 用例；回归门→R-Gate |
| Verification | PASS | 验证直接证明目标行为；Persisted Evidence = none |

**Persisted Evidence**

- Mode: none

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

1. 同键版本链 rid 序 == LSN 序依赖行锁串行化论证——若 Act 构造出反例（派生歧义），按 Stop 返回 Plan（升级为 WAL 格式携带 old_row_id 的 replan 决策）。
2. 重建谓词的提交编码（`redo_tuple_at_row_id` 的 header 构造 × `mark_deleted`/`commit_tx_id` × WAL committed 集合）需 Act 核实；Acceptance 工作负载全为已提交操作，编码冲突仅影响边缘形态 → 异常即 Stop。
3. 多映射内存 O(磁盘版本数)、重建 O(N·logN)——仅不洁打开承担（10k 行级可忽略）；冷路径不对齐 MS08 bench 门，复杂度记录即可。
4. **新发现（Plan Review 3，非本 Cycle 范围）**：`BufferPool::mark_tx_aborted` 为 no-op——`mark_uncommitted_aborted` 实际空转，未提交行靠 `commit_tx_id=None` 不可见性兜底。既有行为，不影响本 Acceptance；建议列为 improvement 候选（未提交事务恢复清理）。
5. 旧树释放的洞容忍收集若与 `collect_all_pages` 严格语义冲突，新增独立变体方法（不动既有 pub 语义）；其他收集错误 warn + 泄漏（先例 drop_table），泄漏量 = 单次不洁关闭的旧索引树，显式记录。

## Act Response

- Status: reported（2026-09-08）

**Gate 2 授权记录**：修复面扩大（recovery 重放臂 + index_manager 洞容忍收集 + table_manager 换入 API）经用户 2026-09-08 明确批准（原话「更改gate，开始实施」）；Plan Context 状态字段由 Act 按授权流转 draft → ready，正文未改写。

### 实际改动

| Repair item | 文件 / 符号 | 改动 |
|---|---|---|
| R-T0b-R7 | `src/wal/recovery.rs` | `RedoContext` 增加 `pk_versions: Option<PkVersionMaps>`（table → PK key → rid 升序多映射）；`full_recover` 新增 `will_redo` 门控（存在 `lsn >= redo_from` 的已提交 data 记录 ⟺ `redo_count > 0`）+ 重放前 `build_pk_version_maps` 预扫描（页链/slot 遍历同 `build_superseded_map` 形态）；三臂 deindexed 分支：Insert=位置写入后追加映射、Update=`derive_old_row_id`（max rid < record.row_id + 派生槽 tuple 与 old_tuple 逐字节校验，失败显式 `RedoFailed`）、Delete=跳过索引清理（墓碑重放不变）；`redo_count == 0` 既有磁盘树路径逐字节保留 |
| R-T0b-R8 | `src/wal/recovery.rs::rebuild_pk_indexes` + `src/storage/btree/index_manager.rs::collect_all_pages_tolerant` + `src/storage/data/table_manager.rs::replace_index_manager` | `redo_count > 0` 门控重建：每表新建 `IndexManager`（spawn_blocking，先于旧树释放分配）→ 一遍扫描最终数据页（slots + `pointed` 集）→ 链尾回溯取首个「已提交 ∧ 非墓碑」版本（运行期已提交编码 `commit_tx_id=Some`；崩溃前被驱逐的未提交副本以 `create_tx ∈ committed_tx_ids` 判定；墓碑创作者 ∈ committed → 整行不建条目，否则视为未提交删除继续回溯）→ 同键第二条目显式报错（K05）→ 批量 insert → `replace_index_manager` 换入（返回旧实例）→ `catalog.update_table_root` 持久化新根 → 旧树洞容忍收集 + `free_page`（失败 warn + 泄漏，先例 drop_table）。洞容忍收集 = `collect_all_pages` 变体：洞页 id 仍入释放集、不递归、不返回 Err；`collect_all_pages` pub 语义未动 |
| R-T0b-R1 收口 | 无代码 / 无测试修改 | `mixed_dml_recovery_semantics` 全断言转绿（COUNT 9950、v=9999 点查+全量口径 100 行、被删行 0、存活区间 100、重复 PK 双探针 DuplicateKey） |
| R-Gate | 无新代码 | 见下方验证表 |

### Deviations from Plan

1. **redo 写入 header 提交编码修正**（`recovery.rs` Insert/Update 两臂构造点 `VersionHeader::new(*tx_id, None)` → `new(*tx_id, Some(*tx_id))`；Update 另带 `with_next_version`）：R7 落地后中间态实测 RED 形态为 COUNT=10050（非契约预测的「唯一性探针失败」形态）——根因是 R6 扫描去重的 `superseder_suppresses`（`data_scan.rs:310-316`）要求新版本 `commit_tx_id = Some` 才压制旧版本，而 T0b 时代的 redo 编码 `None` 永不压制 → 100 个被更新行双计（9950+100）。判定为 R1 见证 COUNT=9950 的必要条件（运行期路径修改属 Forbidden，只能改恢复侧编码），且 Plan Context Current-State Evidence 已预见（「Act 实现时核实 redo_tuple_at_row_id 的 header 构造与 commit_tx_id 语义，保证谓词与实际编码一致」）。redo 只重放已提交事务，`Some(tx)` 与运行期 commit 传播语义（create == commit）一致；WAL/页格式、位置寻址、M21 镜像（create_tx 不变）、`redo_count == 0` 路径均无变化。
2. 其余按契约执行；`update_table_root` 在换入之后调用（Plan 顺序）、`IndexManager::new` 先于旧树释放分配（防复用本表旧树页）。

### Self-Review

- **Spec compliance**：逐 item 对照通过——R7 三臂 deindexed 零 B-Tree 读写、派生+校验语义、映射追加新鲜性；R8 重建五步顺序与 Plan guidance 2 一致、`collect_all_pages` / `drop_table` / `attach_index_catalog_contexts`（database.rs 零修改）/ `redo_count == 0` 路径全部保持；K05 全链显式报错；WAL/页格式零变化；见证文件本轮零修改。
- **Code quality**：全量 diff 走查无计划外修改、无新增警告（clippy 0）、无死代码；错误/边界（空候选集、slot 短读、链目标不在 slots 集、free 失败）均有显式处理。
- **已修复发现**：clippy `type_complexity` ×2（引入 `PreScanPage`/`RebuildScanPage` 类型别名）+ `needless_borrow` ×1；fmt 重排后按 Gate 5 重跑全部验证（见下）。
- **遗留 Minor（不阻塞）**：① `rebuild_pk_indexes` 链尾遍历用 `keys().copied().collect::<Vec<_>>()` 多一次表级分配（直接迭代 `slots.keys()` 即可，行为等价）；② deindexed Insert 臂对不可索引 PK（`to_key` = None，如 String）静默跳过映射追加而 legacy 臂报错——与运行期「此类 PK 不建索引」语义一致，属有意不对称，建议 Plan Review 时知悉。

### 验证（最终代码状态，Gate 5）

| 验证项 | 命令 | 输出摘录 | 退出码 | 结论 |
|---|---|---|---|---|
| 见证套件 | `cargo test --test wal_recovery_large_test` | `test result: ok. 7 passed; 0 failed; 0 ignored`（含 mixed_dml 转绿、rerun 幂等、驱逐规模） | 0 | PASS |
| 全量回归 | `cargo test --all --no-fail-fast` | `TOTAL passed=632 failed=3 ignored=2`；失败恰为白名单 `test_sigint_during_open_130` / `test_sigint_during_run_graceful_130` / `test_sigterm_during_run_143`（T4-RED 见证）；2 ignored 为 Plan T4② 设计的 `#[ignore]` 标定用例；`test_sigkill_leaves_recoverable_db` 在 14 个 cli 通过之列 | 101（白名单所致） | PASS（白名单口径） |
| Lint | `cargo clippy --all-targets -- -D warnings` | error 计数 0 | 0 | PASS |
| Format | `cargo fmt --check` | Diff 计数 0 | 0 | PASS |
| OpenSpec | `openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown` | `Change ... is valid` | 0 | PASS |

### Persisted Evidence

None required（Plan 设定 `Mode: none`；全部验证可低成本复跑，未创建 Evidence 目录）。

### Experience Candidates

None（撕裂树诊断叙事已由 Plan Review 2-4 记录于 Cycle 文档；本轮无新增可复用操作路径或故障现场）。

### 未解决问题

1. 运行期「更新后删除」行在扫描中因墓碑不压制而可复活旧版本——既有语义（R6 注释明示 delete semantics stay exactly as before），不在本 Cycle 范围，建议 improvement 候选。
2. `mark_tx_aborted` no-op（既有，Plan 已记 improvement 候选）。
3. 理论边缘（Acceptance 工作负载不触发）：崩溃前被驱逐的未提交版本副本若恰为某更新链的中间版本且从未重刷，其 `commit_tx_id=None` 不压制前驱——与既有 eviction 落盘语义同源，若未来复现按 BASELINE-CHANGED 处理。

## Plan Review

- Review Result: accepted

（Plan 独立审查，2026-09-08。方法：R7/R8 新增代码逐段审读（`build_pk_version_maps` / `derive_old_row_id` / `redo_record` 三臂 deindexed 分支 / `will_redo` 门控 / `rebuild_pk_indexes` / `insert_row_id_sorted` / `collect_all_pages_tolerant` / `replace_index_manager`）+ 独立复跑全部验证。Act Self-Review 仅作输入。）

### 独立验证结果（全部与 Act Response 一致）

| 验证项 | Plan 独立手段 | 结果 |
|---|---|---|
| R1 见证（阻塞项本体转绿） | 复跑 `cargo test --test wal_recovery_large_test` | **7 passed / 0 failed / 0 ignored**（mixed_dml 全断言绿）✓ |
| 既有套件零回归 | 复跑 `cargo test --test btree_scale_test` | 5/5 ✓ |
| 全量白名单口径 | 复跑 `cargo test --all --no-fail-fast` | cli_test **14 passed / 3 failed / 2 ignored**——失败恰为 T4-RED 白名单三用例（sigint_open_130 / sigint_run_130 / sigterm_143），`test_sigkill_leaves_recoverable_db` 在通过之列；其余套件全绿 ✓ |
| 门禁 | 复跑 clippy/fmt/validate | 0 warning / 0 diff / valid ✓ |
| 见证文件零修改 | mtime 核对 | 两测试文件 14:29（本轮 Act 工作于 17:38-17:55）✓ |
| 偏差 1 语义依据 | 代码核实 | R6 `superseder_suppresses` 要求 commit `Some`；运行期已提交行终态为 `Some`（runtime `count_after_update_exact` 绿为证）；redo 只写已提交事务 → `Some(create==commit)` 与运行期传播终态一致 ✓ |

### Findings 与偏差分类

1. **[ACT-DEVIATION，契约内消解，非阻塞] 偏差 1（redo header 提交编码 `None`→`Some(tx)`）**：实质记录并独立确认正确——(a) Plan Context Current-State Evidence 明确将 header 编码核实留给 Act；(b) 修复对象是恢复侧行编码（运行期路径属 Forbidden 未触碰）；(c) 只影响重放行（已提交事务），`redo_count == 0` 路径、WAL/页格式、M21 镜像零变化；(d) 它是 R1 见证 COUNT=9950 的必要条件（T0b 时代 `None` 编码使 R6 压制永不生效 → 10050）。Plan 预测的中间态 RED 形态（唯一性探针失败）不准——实际 COUNT=10050，属 **PLAN-OMISSION（非实质，见证中间态预测偏差，不影响契约语义）**。
2. **[ACT-DEVIATION，非实质] 偏差 2（新索引先于旧树释放分配）**：防页复用自吞，正确且与 guidance 顺序一致。
3. **[MINOR，不阻塞] Act 遗留 ①②**：`keys().copied().collect()` 多一次分配（等价改写空间）；deindexed Insert 臂对不可索引 PK 静默跳过 vs legacy 臂报错——与运行期「此类 PK 不建索引」语义一致，接受为有意不对称。
4. **[MINOR，记录] `collect_all_pages_tolerant` 额外跟随叶兄弟链**（`next_leaf_page_id`）：visited 集合防环，冗余但无害。
5. 未解决问题 1-3（update→delete 旧值重现 / `mark_tx_aborted` no-op / 未提交驱逐副本边缘）均为既有语义或理论边缘，Acceptance 工作负载不触发——维持 improvement 候选，不阻塞。

### Acceptance 判定

R2-S1 四项 Acceptance 全部满足（撕裂形态恢复可用 + 索引精确重建 + 二次恢复幂等 + 白名单口径回归门），Cycle 004 无阻塞 finding → **Iteration 000 完成**。

### 后继产物

- **Next Cycle**: None（Iteration 000 终局）
- **Iteration Plan Update**: Iteration 000 标记 completed（Map 不变，本 Iteration 无剩余 task）
- **Next Iteration**: `iterations/001-lock-shutdown/000-initial.md`（已展开；T2/T3/T4/T5 契约以 change tasks.md 为权威执行依据，Plan Context 载明当前基线与存量收编要求）

### 用户待决事项

1. 本 Review 审计通过后可调 `openspec-act` 执行 Iteration 001（T2/T3 存量复核收编 + T4 生产接线 + T5 门）。
2. improvement 候选两项（撕裂树运行期根修/驱逐改造；`mark_tx_aborted` 空转补全）与撕裂树 Incident 落档是否授权，随 Iteration 001 收尾由 `openspec-docs-maintainer` / `openspec-experience-recorder` 处理。
