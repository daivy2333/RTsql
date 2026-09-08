# Iteration 000 / Cycle 003-rework: catalog root 同步与扫描版本去重

## Plan Context

- Status: ready
- Iteration: 000-wal-recovery-fix（WAL 恢复逐帧无歧义与重放正确性）
- Cycle: 003-rework
- Cycle Type: rework
- Parent cycle: 002-rework（Act Response `reported`；Plan Review `rework-required`）

> **Rework 注记**：父 Cycle 完成 R-T0b-R2/R3/R4（B-Tree 三缺口修复，Plan 独立验证 GREEN）并发现两个更深层既有缺陷：G4（catalog `index_root_page_id` 不随根分裂同步 → 含 checkpoint 的混合负载恢复失败，Plan 独立复现确认）与 G5（DataScan 对被更新行的新旧版本双计——**运行期即复现**，Plan 独立探针：100 行 + 10 UPDATE → COUNT=110）。本 Cycle 以 2 个新 repair item + R-T0b-R1 完成化收口 Iteration 000 的既有 Acceptance，不修改 Iteration Map。

**Cycle Scope**

- Trigger: rework（父 Cycle Plan Review）
- Acceptance gaps: R2-S1「混合负载恢复计数/语义精确」未满足——G4 使中位点 checkpoint 形态打开失败（`update redo: old key not in index`）；G5 使任何形态的 UPDATE 后计数虚高（恢复与运行期同源）。R-T0b-R1 精确断言契约未落地（父 Cycle 以 loose 版 + `#[ignore]` 处置）
- Repair items: R-T0b-R5（G4 catalog root 同步）、R-T0b-R6（G5 DataScan 被替代版本去重）、R-T0b-R1 完成化、R-Gate——不作为新的全局 change task，不修改 Iteration Map
- Inherited scope: 父 Cycle（001-replan/002-rework）全部已落地成果（reader 修复、位置寻址重放、B-Tree 三修复、btree_scale_test 4 用例）与不变量
- Excluded scope: 锁/信号（Iteration 001）；WAL/页格式；`IndexScan` 路径（经索引 newest 版本解析，无双计）；非 PK 索引；混合长度键语义（父 Cycle Minor，无触发路径，维持记录）

**Objective**

catalog root 与 B-Tree 实际根页保持可恢复一致（site 前条目在恢复后可达）；DataScan 对 MVCC 版本链只产出每行的最新可见版本（运行期与恢复后一致）；见证 ② 以精确断言落地并移除 `#[ignore]`——混合负载（含中位点 checkpoint）崩溃恢复的行数/更新/删除语义全部精确。

**Current Baseline**

- 工作区（未提交，位于 `590fdc6` 之上）：reader.rs（T0）、recovery.rs（T0b）、btree.rs + key.rs（G1-G3）、btree_scale_test.rs（4 用例）、wal_recovery_large_test.rs（5 用例，1 个 `#[ignore]`）。
- 测试基线（Plan 独立复跑）：`btree_scale_test` 4/4；`wal_recovery_large_test` 4 绿 + 1 ignored；clippy 0 / fmt 0 / validate PASS。

**Current-State Evidence**（Plan Review 独立探针与代码审查，2026-09-07；临时探针已删除，recipe 在内）

- **G4 stale catalog root**：`BTree::insert` 根分裂返回 `Ok(Some(new_root_page_id))`（`src/storage/btree/btree.rs:205-215`，注释「caller should update root」）；IndexManager 仅更新内存 root（AtomicU64），catalog 行 `index_root_page_id` 只在 `create_table` 写入一次（`src/storage/data/table_manager.rs:223-224`）且无任何更新路径。恢复 `open_or_init` 经 `IndexManager::from_root(stale_root)` 加载 → site 前的索引条目（位于真根下的页，已随 checkpoint 落盘）不可达。**独立复现**：中位点 checkpoint 混合 WAL（create→checkpoint→5k→checkpoint→5k→100 UPDATE（运行期全部成功）→50 DELETE→drop 不 close）重开 → `WalError("WAL redo failed: update redo: table 't' old key not in index")`。对照组：仅 create 时 checkpoint 的同构负载重开成功（全部记录重放、在 stale root 下重建完整树）→ 证明缺陷依赖「site 前已有索引条目落盘 + 根已分裂」。
- **G4 可恢复性论证（修复设计依据）**：catalog 行在根变化时同步更新（内存池内），持久性由既有刷盘路径保证（checkpoint 全量刷盘 / 页驱逐）。恢复正确性不变量：**落盘 root 版本必须 ≥ site 时点版本**——checkpoint 时点 S 全量刷盘含 catalog 页 → 落盘 root = root(S) = site 时点版本 ✓；S 与崩溃之间 root 再变化只使落盘版本更新（若 catalog 页被驱逐刷盘）或保持 root(S)（未刷盘）——两者都 ⊇ site 前条目 ✓。二次崩溃安全：恢复重放自 site 起，加载点不变，幂等 ✓。对照先例：`update_table_tail`（`catalog.rs:229`）同为 catalog 行字段同步，机制可复用。
- **G5 扫描版本双计**：`DataScanExecutor`（`src/executor/data_scan.rs`）逐 slot 产出——`is_deleted` 跳过墓碑（`:295-298`）、`find_visible_in_chain`（`:185-213`）仅在**当前 slot 不可见**时沿 `next_version` 向旧版本回溯；**没有反向判定**：被某新版本的 `next_version` 指向的旧 slot 自身 header 可见即被照常产出。版本链方向 new→old（`update.rs:98` `with_next_version(old_row_id)`），索引点查从最新版本进入故正确（恢复后 v0=9999 ✓），全表扫描无从得知「本 slot 已被替代」。**运行期独立复现（无恢复介入）**：100 行 + 10 UPDATE → `SELECT COUNT(*)` = 110；恢复侧同构：契约 R1 夹具（仅 create checkpoint、100 UPDATE 运行期成功、50 DELETE）重开成功但 COUNT=10050 = 9950 + 100。`IndexScan`/点查路径不受影响（索引始终指向最新版本）。
- **G5 修复设计依据**：候选 ①「更新时在旧 slot header 打替代标记」被拒——旧版本必须对旧快照保持可见（`Snapshot::is_visible` 语义），全局标记破坏快照隔离；候选 ② **扫描级替代集合**（选定）：扫描前置阶段收集全表所有 `next_version` 目标（target_rid → 最新替代者 rid 映射），逐 slot 产出时——slot 在映射中且其替代者对当前快照可见 → 跳过（替代者自身会被产出）；否则按既有可见性规则产出。内存 O(被更新行数)，快照语义保持，一次实现同时修正运行期与恢复后路径（同一 executor）。
- **代码面**：`src/storage/data/table_manager.rs`（IndexManager 构造点 ×2）、`src/storage/btree/index_manager.rs`（变更入口 insert/update/delete + root 访问器）、`src/storage/catalog.rs`（新增 `update_table_root`，镜像 `update_table_tail:229`）、`src/executor/data_scan.rs`（扫描产出逻辑）。**父 Cycle 冻结面不动**：`src/wal/*`、页格式、`redo_record` 语义（G5/G4 均不需触碰 recovery）。

**Relevant Code**

| 文件/符号 | 职责与本 Cycle 关系 |
|---|---|
| `src/storage/catalog.rs` | 新增 `update_table_root(name, root_page_id)`（镜像 `update_table_tail`） |
| `src/storage/btree/index_manager.rs` | 变更后 root 同步：持 catalog 句柄 + 表名，根变化时写 catalog |
| `src/storage/data/table_manager.rs` | 两处 IndexManager 构造点注入 catalog 句柄（`new`/`from_root` 路径） |
| `src/executor/data_scan.rs` | G5：替代集合预收集 + 逐 slot 跳过判定 |
| `tests/btree_scale_test.rs` / `tests/wal_recovery_large_test.rs` | R5/R6 见证扩充 + R1 完成化（移除 `#[ignore]`） |
| `src/wal/*`、`src/storage/page_format/*` | **禁止修改**（冻结面） |

**Critical Path**

G4（catalog root 同步：构造点注入 → 变更入口检查 → catalog 新方法）与 G6（G5 扫描去重：预收集 → 逐 slot 判定）相互独立 → R-T0b-R1 完成化（依赖两者）→ R-Gate。

**Implementation Guidance**

1. **R-T0b-R5（G4）**：`IndexManager` 增加可选 catalog 上下文（`catalog: Option<(Arc<Catalog>, String)>` 或等价回调——非实质，留给 Act；推荐持有 `Arc<Catalog>` + 表名字段，`new` 与 `from_root` 增参或 builder）。在 `insert`/`update`/`delete` 变更完成后比较 `root_page_id()` 与持久镜像（初始 = 构造时 root），变化即调 `catalog.update_table_root(name, root)` 并刷新镜像。`catalog.rs` 新增 `update_table_root`（结构照抄 `update_table_tail`：定位 `__tables` 行 → 改字段 → 写回）。`create_table` 路径的 `IndexManager::new` 与 `open_or_init` 的 `from_root` 都注入上下文；无 catalog 的测试构造路径传 None（行为不变）。
2. **R-T0b-R6（G5）**：扫描生命周期前置阶段（首次 `next()` 前或惰性首轮）遍历表数据页链，收集 `map: HashMap<RowId /*target*/, RowId /*superseder 链头*/>`——遍历各 slot 的 `vh.next_version()`；链上多次指向以最新为准（后写覆盖或比较 tx 序，非实质）。逐 slot 产出判定改为：`is_deleted` → 跳过（不变）；slot 在 map 中且**链头对当前快照可见** → 跳过；否则既有可见性/回溯逻辑不变。预收集的驱逐页访问复用 `buffer_pool` 既有读路径；MS08-T02 预取开关不受影响（默认关）。
3. 关键取舍（已定）：G4 在 IndexManager 层同步而非 BTree 层（BTree 无 catalog 依赖，层次不倒置）；G5 选扫描级集合而非 header 标记（快照语义）；两者都不触碰 recovery/writer/页格式。非实质留给 Act：上下文注入的具体形态、map 构建时机、`update_table_root` 的行定位细节、测试夹具微调。

**Behavioral Change**

| 输入 | 现行为 | 目标行为 |
|---|---|---|
| 根分裂后崩溃 + 含 checkpoint 的 WAL 恢复 | `old key not in index` 打开失败 | 恢复成功，site 前条目可达 |
| 运行期 UPDATE 后 `SELECT COUNT(*)` | 双计新旧版本（110/100+10） | 每行计一次（100） |
| 恢复后 UPDATE 行的 COUNT | 同上（10050） | 精确 = 已提交终态（9950） |
| 点查/索引路径 | 正确（最新版本） | 不变 |
| 旧快照对被更新行的可见性 | 经 `find_visible_in_chain` 回溯 | 不变（替代集合按快照可见性条件跳过） |

**Change Surface**

| Repair item | Requirement/Scenario | File/Symbol | Planned Change |
|---|---|---|---|
| R-T0b-R5 | R2-S1 可用性 | catalog.rs + index_manager.rs + table_manager.rs | 根变化同步 catalog |
| R-T0b-R6 | R2-S1 计数精确 | data_scan.rs | 替代集合去重 |
| R-T0b-R1 完成化 | R2-S1 完整见证 | wal_recovery_large_test.rs | 移除 `#[ignore]` + 精确断言重写 |
| R-Gate | 全部 | 无新代码 | 四命令复跑 |

**Task Contracts**

### R-T0b-R5: catalog root 同步（G4）

- Requirement/Scenario: `wal-recovery-replay-integrity` R2-S1（恢复可用性——site 前条目可达）
- Depends on: None
- Targets: `catalog.rs`（+`update_table_root`）、`index_manager.rs`（root 变更同步）、`table_manager.rs`（构造点注入）
- Current behavior: 根分裂只更新内存；恢复从 stale root 加载 → site 前条目不可达 → Update 重放 `old key not in index`
- Required behavior: 任何 root 变化（分裂上探/收缩）在变更后写 catalog 行；恢复加载的落盘 root ≥ site 时点版本（Current-State Evidence 不变量）
- Required changes: Implementation Guidance 1（约 60-100 行）
- Preserve: `update_table_tail` 既有机制；`Database::open`/`from_root` 签名兼容（无 catalog 测试路径传 None 行为不变）；K05；恢复/重放语义零修改
- Forbidden: 恢复层重建索引；页格式/WAL 格式变更；同步阻塞刷盘（写 catalog 行即可，持久性由既有刷盘路径保证）
- Test witness（RED 先行）: `tests/btree_scale_test.rs` 新增 `root_sync_survives_midpoint_checkpoint`——create→checkpoint→5k→checkpoint（中位点）→5k→drop 不 close → 重开 → 重复 INSERT 中位点前键（如 id=42）必须 DuplicateKey（RED：`old key not in index` 类失败或键不可达——以重开失败/探针失败为准，Act 记录实际 RED 形态）；GREEN 后 `min_key_searchable_after_recovery` 等既有 4 用例零回归
- GREEN condition: 新用例绿 + 既有套件零回归
- Verification: `cargo test --test btree_scale_test` 记 Act Response
- Stop when: root 收缩路径（delete 合并）无法统一观测（需改 BTree 公共签名）→ 返回 Plan

### R-T0b-R6: DataScan 被替代版本去重（G5）

- Requirement/Scenario: R2-S1（计数精确）+ 运行期正确性
- Depends on: None（与 R5 独立）
- Targets: `src/executor/data_scan.rs`
- Current behavior: 每个 header 可见的 slot 都产出 → 被更新行新旧版本双计（运行期 110/100+10 实测）
- Required behavior: 每行只产出对当前快照的最新可见版本（Implementation Guidance 2 的替代集合方案）
- Required changes: 预收集 + 逐 slot 判定（约 60-120 行）
- Preserve: `find_visible_in_chain` 回溯语义（不可见当前版本 → 旧版本）；墓碑跳过；投影/谓词/MVCC 裁剪顺序（MS10-T01 真投影）；MS08-T02 预取默认关
- Forbidden: header 替代标记（破坏快照语义）；页格式变更；IndexScan 路径变更
- Test witness（RED 先行）: `tests/btree_scale_test.rs` 或 `wal_recovery_large_test.rs` 新增运行期用例 `count_after_update_exact`——100 行 + 10 UPDATE → `COUNT(*)` == 100 且被更新行点查新值（RED：110，Plan 已实测）；跨页更新场景（更新行分属 ≥3 数据页）至少一例
- GREEN condition: 用例绿 + 既有套件零回归（重点：pushdown_test 15 / projection_test 6 / executor_test 39）
- Verification: `cargo test --test btree_scale_test --test pushdown_test --test projection_test` 记 Act Response
- Stop when: 替代集合方案与谓词下推/MVCC 裁剪顺序产生语义冲突且无法局部消解 → 返回 Plan

### R-T0b-R1 完成化: 见证 ② 精确断言（依赖 R5/R6）

- Requirement/Scenario: R2-S1 完整见证
- Depends on: R-T0b-R5, R-T0b-R6
- Targets: `tests/wal_recovery_large_test.rs::mixed_dml_recovery_semantics`（移除 `#[ignore]` + 重写为精确版）
- Current behavior: loose 版（±5 容差、`let _` 吞 UPDATE）+ `#[ignore]`
- Required behavior: 夹具 = create→checkpoint→5k→checkpoint（中位点，spec R2-S1 前提）→5k→UPDATE 100 行（含最小键/中部/尾部，全部须成功——R4 后可达成）→DELETE 50 行→drop 不 close→重开断言全部精确：`COUNT(*)` == 9950、被更新行 v=9999（点查 + 全量口径抽查）、被删行 0 行、重复 INSERT 最小键与中位点前键均 DuplicateKey
- Required changes: 重写该用例；其余 4 用例 + btree_scale 既有用例零修改
- Preserve: 既有断言语义（除本用例按契约精确化外）
- Forbidden: 任何容差；跳过失败语句不计入期望值
- Test witness: 先在 R5/R6 落地前移除 `#[ignore]` 观察 RED（G4 打开失败），落地后 GREEN
- GREEN condition: 用例绿 + 全量门
- Verification: `cargo test --test wal_recovery_large_test` 记 Act Response
- Stop when: R5/R6 落地后夹具仍无法精确构造 → 返回 Plan

### R-Gate: 全量验证门

- Requirement/Scenario: 全部
- Depends on: R-T0b-R1
- Targets: 无新代码；`cargo test --all`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown`
- Current behavior: 白名单（3 信号用例）外全绿；1 个 `#[ignore]`（本 Cycle 移除后归零）
- Required behavior: 白名单外全绿（含 `mixed_dml_recovery_semantics` 转正）；clippy 0 / fmt 0 / validate PASS
- Test witness: 各命令决定性输出（≤20 行）与退出码
- Stop when: 白名单外回归失败且无法归因于 R5/R6/R1 → BASELINE-CHANGED 返回 Plan

**Invariants**

- 页格式、WAL 格式、reader/recovery/checkpoint 语义零变化
- K05；快照隔离语义保持（G5 不引入全局替代标记）
- 既有测试断言语义零修改（`mixed_dml_recovery_semantics` 按契约精确化为既定例外）；4 文件夹具锁适配例外不变
- 不引入哈希/校验和/内容指纹；不新建 Evidence 占位目录

**Non-goals**

锁/信号（Iteration 001）、IndexScan 路径、非 PK 索引、混合长度键语义、WAL/页格式、do_flush 并发、多行 INSERT Page full（观察项）。

**Acceptance**

1. R2-S1 可用性：中位点 checkpoint 混合 WAL 恢复成功且 site 前条目可达——`root_sync_survives_midpoint_checkpoint`。
2. R2-S1 计数精确：运行期与恢复后 COUNT 均精确——`count_after_update_exact` + 精确版 `mixed_dml_recovery_semantics`。
3. 更新/删除/唯一性语义：更新可见新值、删除不可见、最小键判重——精确版见证 ② 全断言。
4. 回归门：白名单外全绿 + clippy 0 / fmt 0 / validate PASS——R-Gate。

**Verification**

- `cargo test --test btree_scale_test`（5 用例）、`--test wal_recovery_large_test`（5 用例全绿无 ignore）
- `cargo test --all`（白名单口径）+ clippy/fmt/validate
- 输出（每项 ≤20 行）+ 退出码记 Act Response

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | G4/G5 均有 Plan 独立复现（运行期与恢复侧）+ 代码级定位（btree.rs:205-215 / table_manager.rs:223-224 / data_scan.rs:185-213）；G4 可恢复性不变量已论证 |
| Design | PASS | 两修复方案已定且拒绝备选有据（G4 层次不倒置 / G5 快照语义优先）；无 TBD |
| Iteration Plan | PASS | Map 不变；repair item 形式；故障域（storage catalog/executor scan）集中 |
| Cycle Scope | PASS | 既有 Acceptance 必要条件；**G5 触及 executor 扫描语义（新故障域），需用户 Gate 2 批准扩大** |
| Task Contracts | PASS | 4 项均含 Targets/行为/Preserve/Forbidden/RED-GREEN/Verification/Stop |
| Traceability | PASS | R2-S1 可用性→R5；计数精确→R6+R1；回归门→R-Gate |
| Verification | PASS | 验证直接证明目标行为；Persisted Evidence = none |

**Persisted Evidence**

- Mode: none

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

1. G5 的替代集合预收集为每次扫描增加一次全链遍历（O(数据页数) 页读取）——对大表扫描有性能影响；MS08 纪律：若 Act 实测引入显著回退（对照 data_scan bench），在 Act Response 记录数据并可提出「仅存在版本链时启用」的条件化优化（非实质扩展，不改变语义）。
2. G4 的 catalog 行更新频率 = 根变化频率（每 ~90 插入一次），量级可忽略；但根收缩（delete 合并）路径的观测点需 Act 确认（BTree::delete 的 merge_info 处理），若收缩不改 root_page_id 则天然无需同步。
3. G5 与谓词下推/投影裁剪（MS07-T06/MS10-T01）的交互：替代集合判定发生在可见性裁剪点，Act 需保证 pushdown_test 15 用例零回归；若顺序冲突按 Stop-when 返回 Plan。
4. R1 精确断言依赖 R4（UPDATE 运行期成功）已在本 Cycle 前达成；期望值 9950 = 10000 − 50，UPDATE 不改变行数（R6 后由计数预言机保证）。
5. 父 Cycle Minor（Key 混合长度）维持记录不处理。

## Act Response

- Status: blocked

**Implemented**

- **R-T0b-R6 完成（Gate 3-5 通过）**：`DataScanExecutor` 替代集合去重落地——惰性首轮全链遍历构建 `superseded_map`（target rid → 直接替代者 rid + create_tx，链上多指向以新 create_tx 为准）；逐 slot 产出前查 map 并沿替代者链上行判定抑制（深度上限 64）；抑制谓词 = 已提交且非墓碑的替代者才抑制（快照存在时经 `Snapshot::is_visible`）。见证 2 用例 RED→GREEN（`count_after_update_exact` 110→100、`count_after_cross_page_update_exact` 303→300）；边界回归全绿（pushdown 15 / projection 6 / executor 39 / prefetch 3 / recovery_e2e 6 / schema_persistence 8 / storage 21）。
- **R-T0b-R5 实现完成、见证 GREEN，但其修复目标（G4 完整闭合）被更深层的既有机制阻塞**（见 Blocker Handoff）：`catalog.rs` +`update_table_root`（镜像 `update_table_tail`）；`index_manager.rs` catalog 上下文（`Mutex<Option<(Arc<Catalog>, String)>>` + builder/后附加 setter）+ `sync_root_to_catalog`（insert/delete 的 root 变更点严格传播）；`table_manager.rs` create_table 附加 + `attach_index_catalog_contexts`；`database.rs` 恢复完成后附加。见证 `root_sync_survives_midpoint_checkpoint` RED→GREEN（5/5）。
- **R-T0b-R1 见证精确化完成（重写 + 移除 `#[ignore]`，RED 形态 = 契约预期的 G4 打开失败）**；GREEN 依赖 R5 闭合 → **阻塞**。
- **R-Gate 部分完成**：clippy 0 / fmt 0 / validate PASS；`cargo test --all --no-fail-fast` 除 T4-RED 白名单 3 用例（计划内）外唯一失败 = `mixed_dml_recovery_semantics`（阻塞项本体）。0 ignored。

**Changed Files and Symbols**

| 文件 | 变更 |
|---|---|
| `src/executor/data_scan.rs` | R6：`PageAction::NeedVersionChain(Option<RowId>)` → `SkipSlot`；+`build_superseded_map`/`slot_is_superseded`/`superseder_suppresses`（+`VersionLink` 别名、模块级 `MAX_CHAIN_DEPTH`）；`next()` 惰性建 map + 前置跳过判定；删除 `find_visible_in_chain`；`data_page_head`/`superseded_map` 字段 |
| `src/storage/catalog.rs` | R5：+`update_table_root`（镜像 `update_table_tail`/`update_field_in_chain`） |
| `src/storage/btree/index_manager.rs` | R5：+`catalog_ctx: Mutex<Option<(Arc<Catalog>, String)>>`、`with_catalog_context`/`set_catalog_context`/`sync_root_to_catalog`；insert/delete root 变更点同步 |
| `src/storage/data/table_manager.rs` | R5：create_table 附加 context；+`attach_index_catalog_contexts`（open_or_init 不附加） |
| `src/database.rs` | R5：`Database::open` 在 `full_recover` 后调用 `attach_index_catalog_contexts` |
| `tests/btree_scale_test.rs` | +R5 见证 `root_sync_survives_midpoint_checkpoint`；既有 4 用例零修改 |
| `tests/wal_recovery_large_test.rs` | R1：`mixed_dml_recovery_semantics` 重写为精确版（零容差 + UPDATE/DELETE 逐条断言 + 唯一性探针），移除 `#[ignore]`；+R6 见证 `count_after_update_exact`/`count_after_cross_page_update_exact`；既有 4 用例零修改 |
| `iterations/000-wal-recovery-fix/003-rework.md` | Plan Context 状态 draft→ready（用户批准 Gate 2）+ 本 Response |

**Deviations from Plan**

1. **G5 机制修正（契约内消解，实质记录）**：计划的「slot 在 map 中且其**直接替代者**可见 → 跳过 + 保留 `find_visible_in_chain` 回溯」在旧快照 + ≥2 次更新的链（T→A→B，快照见 A 不见 B）下双计 A、见 T 不见 A 时三计——替代集合与链回溯双机制重叠且互相冲突（推导：链回溯产出的版本必然也被自身 slot 产出，除非该版本被 map 抑制；而 map 抑制条件与回溯可见性判定不一致）。修正为单一机制：**「产出 X ⟺ X 可见 ∧ 无任何更新版本对其抑制」+ 不可见 slot 直接跳过（无回溯）**——可证每行恰产出最新可见版本。`find_visible_in_chain` 对 pipeline 唯一构造路径（`pipeline.rs:447` 传 `None` 快照）本就是死代码，保留将触发 dead-code 使 clippy -D warnings 失败，故删除。抑制谓词 = 已提交且非墓碑（墓碑/未提交替代者不抑制——零改动 abort 回滚、update→delete 旧值重现、显式事务内未提交读的既有行为）。
2. **R5 附加点重构（Gate 6 尝试 ② 的产物，依据 Plan 自身不变量保留）**：Plan 要求「`create_table` 路径 `IndexManager::new` 与 `open_or_init` 的 `from_root` 都注入上下文」；实际 open_or_init **不**附加，改为 `Database::open` 在 `full_recover` 之后经 `attach_index_catalog_contexts` 附加。依据 = Plan G4 可恢复性论证原文「二次崩溃安全：恢复重放自 site 起，**加载点不变**，幂等」——恢复期 redo 的根分裂不得持久化（尝试 ① 全构造点附加时，恢复重建树的 root 经 catalog 池页被后续驱逐刷盘持久化，下次恢复消费「部分持久化重建树」→ BTree 同步递归入环 → `recovery_rerun_is_idempotent` 栈溢出）。此重构使 recovery_rerun 转绿且与 Plan 不变量一致。
3. `catalog_ctx` 用 `Mutex<Option<..>>` 支持后附加（构造时 builder 与运行时 setter 并存）；`sync_root_to_catalog` 锁内 clone 后锁外 await（不跨 await 持 std 锁）。非实质。
4. 直接 `new_root` 捕获替代计划的「持久镜像比较」：root 仅能经 `BTree::insert/delete` 的 `Option<PageId>` 返回值变化（`btree.rs:207-226`/`:403-410`；`update` 原地改 row_id 无结构变化、无 root 返回），直接在两处返回点同步 = 等效且更简。非实质。

**Blocker Handoff**

- **发现位置**：R-T0b-R1 见证 GREEN 验证 / `mixed_dml_recovery_semantics` / Gate 5；三次实现尝试 + 三轮探针诊断（临时测试 `_diag_tmp.rs` 已建已删，recipe 见下）。
- **Plan 预期 vs 实际**：Plan 预期 R5（catalog root 同步）使「中位点 checkpoint + 驱逐规模混合负载」崩溃重开可达（G4 闭合）；实际 R5 落地后 mixed_dml 仍 `WalError("WAL redo failed: update redo: table 't' old key not in index")`。
- **根因（Plan 独立验证数据，非采信推测）**：
  1. **root 分裂频率与 Plan 假设不符**：root 分裂仅在 ~90 键时发生一次（叶→内部）；内部节点容量 ~220 分隔符，10k 键（221 叶）仍运行于单一内部根下（实测：10k 插入后 LIVE root=145 与 5k 时相同，`scan_all`=10000、222 页可达、0 插入失败）。Plan Risk 2 的「每 ~90 插入一次根变化」模型不成立。
  2. **撕裂树（TORN TREE）——本阻塞的本体**：中位点 checkpoint 全量刷盘正确（场景 A 实测：checkpoint 后立即裸读落盘树 = 5000 条 / 111 页可达 / 无洞）；其后运行期修改经**页驱逐**碎片化落盘——被驱逐的父页（含指向新叶的分隔符）与仍在池中的新叶页组合，磁盘树同时含新内容与洞（场景 B 实测：5k→5k 后裸读 = `scan_all` 6808 条（应为 10000）+ `collect_all_pages` 命中 `InvalidPageType { expected: 1, actual: 0 }` 洞页）。**撕裂在树页本身，与 root 行无关**——驱逐 LRU 不感知树拓扑，任何 root 同步策略（运行期/恢复期/checkpoint 期）都无法使磁盘多页树保持结构一致。
  3. **恢复消费撕裂树**：R5 同步使 catalog root 指向真实根（145），恢复重放首次消费多页磁盘树 → search/insert 命中洞/垃圾 → 部分老键不可达 → update redo `old key not in index` → 打开失败。**修复前基线（无同步）靠 stale 自含叶根「意外自愈」**——恢复在 stale 叶下全量重建索引，从不消费多页磁盘树（这正是 002-rework 对照组「仅 create 时 checkpoint 的同构负载重开成功」的机制）。002-rework 对 G4 的 stale-root 归因是表层机制；深层是撕裂树。
  4. **Plan G4 可恢复性论证的缺口**：「落盘 root 版本 ≥ site 时点版本 ⊇ site 前条目 ✓」在**逻辑条目层**成立，但在**物理页层**不成立——论证假设「持久性由既有刷盘路径保证（checkpoint 全量刷盘 / 页驱逐）」，而页驱逐恰恰是撕裂源。R5 的见证（无驱逐场景）与 Plan 论证在该场景内自洽，mixed_dml 形态（checkpoint 后继续 DML 触发驱逐）暴露缺口。
- **三次尝试记录**：① 全构造点附加 context（恢复期同步不抑制）→ `recovery_rerun_is_idempotent` 栈溢出（BTree 同步递归入环：恢复重建树 root 被持久化 + 部分刷盘成为下次恢复基座）；② context 后置附加（恢复期抑制，依据「加载点不变」）→ recovery_rerun 转绿，mixed_dml 仍失败；③ 根因解剖（裸读 catalog + `scan_all`/`collect_all_pages` + 双场景对照）→ 撕裂树机制确认、契约内无消解路径 → 停止，不开始第四次同类尝试。
- **影响面**：① R-T0b-R1 的 Acceptance（R2-S1 完整见证）在 R5 契约内不可达——候选修复方向（**Plan 裁定，非 Act 选型**）：(a) 恢复层索引重建（需解除 R5 Forbidden「恢复层重建索引」并重新设计 R2-S1 见证路径）；(b) B-Tree 页洞容忍（与 K05 显式失败哲学冲突，需论证）；(c) 结构感知刷盘 / checkpoint 树快照（重设计面最大）。② R5 代码面：见证 GREEN 的机制已核实（首次根分裂在任何 checkpoint 之前发生并被同步；witness 场景无驱逐、恢复仅消费完好树），是否保留/如何与上述方向组合属 Plan 裁定；当前形态下 R5 不引入新失败（全量除阻塞项外零回归）。③ R6 完成且独立成立（运行期与恢复后同源去重），其恢复侧端到端价值在 R1 转绿前无法见证。
- **任务状态**：R6 完成；R5 实现完成 + 见证 GREEN（G4 完整闭合被阻塞）；R1 见证精确化完成、GREEN 阻塞；R-Gate 部分完成（四项中 clippy/fmt/validate PASS，`cargo test --all` 白名单口径除阻塞项外全绿）。
- **工作区状态**：全部未提交（位于 `590fdc6` 之上），无未完成中间态；`_diag_tmp.rs` 已删除。
- **已通过 Gate**：Gate 3（R6 两见证、R5 见证 RED→GREEN；R1 见证 RED 按契约预期形态）；Gate 4（R5/R6 分任务 spec+code review 完成；完整 diff Self-Review 见下）；Gate 5（可完成项均有新鲜证据）；Gate 6（本阻塞）。
- **诊断 recipe（~30s/轮，debug）**：复刻 mixed_dml 夹具（10k/50 每事务 + 中位点 checkpoint + 100 UPDATE + 50 DELETE）→ `wal_buffer.shutdown()` + drop → 裸 `FileStorage::open` + `Catalog::open` → `scan_tables` 打印 `index_root_page_id` → `IndexManager::from_root(root)` → `scan_all`（条目数 < 期望即撕裂）+ `collect_all_pages`（`InvalidPageType actual: 0` 即洞页）。对照场景 A（checkpoint 后立即裸读）树完好。
- **恢复条件**：用户/Plan 审定撕裂树修复设计并创建后继 Cycle（扩展执行契约或修订 Acceptance 路径）后，Act 从恢复点继续（R1 GREEN + R-Gate 收尾）。R6 成果与 R5 现有代码面建议纳入后继 Cycle 基线复核。

**Self-Review**

- Plan compliance: R6/R5 契约逐项核验满足（Targets/Preserve/Forbidden/RED-GREEN）；R1 按契约精确化重写。偏差 1-4 已记录（偏差 1 为契约内消解的机制修正，偏差 2 依据 Plan 自身不变量）。
- Full diff reviewed: 已审查完整工作区 diff（7 文件 + 状态行）；跨任务交互核验：R6 不触碰 recovery/WAL/页格式；R5 复用 `update_table_tail` 既有机制、恢复语义零修改（重放路径无 context）。
- Critical findings unresolved: 1（阻塞本体——撕裂树）。
- Important findings unresolved: 0。
- Minor findings unresolved: 2——(a) G5 替代集合为每次扫描增加一次全链页遍历（Plan Risk 1 已预案条件化优化；本轮未跑 bench 对照）；(b) update→delete 行的旧版本在无快照扫描路径仍重现（既有行为，墓碑不抑制原则精确保留，未扩大）。

**Verification Evidence**

| 验证项 | 命令 | 输出摘录 | 结论 |
|---|---|---|---|
| R6 见证 RED | `cargo test --test wal_recovery_large_test`（修复前） | `count_after_update_exact`: left 110 / right 100；cross-page: 303/300 | RED ✓（与 Plan 探针同值） |
| R6 见证 GREEN | 同上（修复后） | `7 tests: 6 passed（阻塞项外全绿）` | GREEN ✓ |
| R6 边界回归 | `pushdown/projection/prefetch/recovery_e2e/schema_persistence/storage` 六目标 | 15+6+3+6+8+21 全 `ok` | 零回归 ✓ |
| R5 见证 RED | `cargo test --test btree_scale_test`（修复前） | `id=300 应被拒，实际 AffectedRows { count: 1 }` | RED ✓（契约「探针失败」形态） |
| R5 见证 GREEN | 同上（修复后） | `5 passed; 0 failed` | GREEN ✓ |
| R1 见证 RED | `cargo test --test wal_recovery_large_test mixed_dml` | `WalError("WAL redo failed: update redo: table 't' old key not in index")` | RED ✓（契约预期形态）；GREEN 被阻塞 |
| recovery_rerun 栈溢出（尝试 ①） | `cargo test --test wal_recovery_large_test recovery_rerun` | `thread has overflowed its stack`；context 后置附加后 `ok` | 已修复 ✓ |
| 全量门 | `cargo test --all --no-fail-fast` | 除 cli_test 3 个 T4-RED 白名单（`left: None` 形态）+ mixed_dml 外全部 `ok`（lib 176 等 ~40 目标） | 白名单口径达成（阻塞项除外） |
| clippy / fmt / validate | `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` / `openspec validate ...` | Finished 0 warning / 0 diff / `is valid` | PASS ✓ |
| 撕裂树诊断（场景 A/B） | 裸读 catalog + `scan_all`/`collect_all_pages`（recipe 见 Handoff） | A：5000 条/111 页/无洞；B：6808 条/`InvalidPageType actual: 0` | 阻塞证据 ✓ |

**Persisted Evidence**

`None required`——全部决定性数字（≤20 行）已载入本 Response；阻塞场景可由 Handoff recipe 以临时测试低成本复现，无一次性环境或结构化丢失。

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Incident | 驱逐规模 + 中位点 checkpoint 混合负载崩溃恢复打开失败：B-Tree 磁盘树被页驱逐撕裂（被驱逐父页指向未刷盘子页），恢复重放消费撕裂树；修复前由 stale 自含叶根的全量重建「意外自愈」掩盖。系统性诊断路径（LIVE root 观测 → 裸读 catalog → scan_all/collect_all_pages → 双场景对照）完整 | 本 Act Response「Blocker Handoff」+ Verification Evidence | 显著影响（数据可用性）、需异常恢复、难以归因（三层机制叠加）、诊断信息系统性强；等 Plan 审定后可由 Recorder 落 Incident |

**Remaining Issues**

- Blocker Handoff 的撕裂树修复设计（Plan 裁定方向 a/b/c 或其他）。
- Plan Risk 1：G5 替代集合每次扫描一次全链遍历的性能影响（未测 bench；条件化优化预案在案）。
- Minor (b)：update→delete 行旧版本在无快照扫描路径重现（既有行为，未扩大）。

**Commit or Diff Reference**

工作区未提交（`590fdc6` 之上）：`src/{database.rs,executor/data_scan.rs,storage/catalog.rs,storage/btree/index_manager.rs,storage/data/table_manager.rs}`、`tests/{btree_scale_test,wal_recovery_large_test}.rs`、本文件。

## Plan Review

- Review Result: rework-required

（Plan 独立审查，2026-09-08。方法：实际代码 diff 审读 + 独立复跑全部关键见证 + 按 Act recipe 临时探针独立复现阻塞根因（探针已删除）。Act Self-Review 仅作输入，未替代独立检查。）

### 独立验证结果（全部与 Act Response 一致）

| 验证项 | Plan 独立手段 | 结果 |
|---|---|---|
| R6 见证 GREEN | 复跑 `cargo test --test wal_recovery_large_test` | `count_after_update_exact` / `count_after_cross_page_update_exact` 绿 ✓ |
| R6 回归面 | 复跑 pushdown/projection/executor/prefetch | 15+6+39+3 全绿 ✓ |
| R5 见证 GREEN | 复跑 `cargo test --test btree_scale_test` | 5/5（含 `root_sync_survives_midpoint_checkpoint`）✓ |
| R1 阻塞形态 | 复跑同套件 | `mixed_dml_recovery_semantics` FAILED 于重开 unwrap，错误 = `WalError("WAL redo failed: update redo: table 't' old key not in index")`，与 Handoff 逐字一致 ✓ |
| 门禁 | 复跑 clippy/fmt/validate | 0 warning / 0 diff / valid ✓ |
| 偏差 1 死代码声明 | grep + `pipeline.rs:257/447` 核对 | `find_visible_in_chain` 零引用；生产唯一构造点恒 `snapshot: None` ✓ |
| 偏差 2 attach 时序 | diff 审读 `database.rs`/`table_manager.rs` | `attach_index_catalog_contexts` 在 `full_recover` 后调用、`open_or_init` 不附加，与「加载点不变」论证一致 ✓ |
| **撕裂树根因** | 临时探针（mixed_dml 夹具崩溃后裸 `FileStorage`+`Catalog`+`from_root`） | `catalog_root=155`（正确）；`scan_all`=**184/10000**；`collect_all_pages` 报 **`InvalidPageType { expected: 0x1, actual: 0x0 }`** 洞页 ✓ |

### Findings 与偏差分类

1. **[NEW-EVIDENCE，阻塞本体] 撕裂树**：Blocker Handoff 成立且经独立复现升级为实证——G4 的「stale catalog root」归因是表层机制（root 已正确同步为 155），深层是中位点 checkpoint 后页驱逐按 LRU 而非树拓扑刷盘造成的磁盘树洞/孤儿。Handoff 的三次尝试记录、对照组论证与诊断 recipe 完整可复现，Gate 6 停止正确。
2. **[ACT-DEVIATION，非阻塞] G5 机制修正（偏差 1）**：单一抑制机制替代「替代集合 + 链回溯」双机制。独立审查确认：修正后的谓词（已提交 ∧ 非墓碑 ∧ 快照可见）在无快照/旧快照/未提交/中止各形态下均精确产出「最新可见版本」；被删除的 `find_visible_in_chain` 在生产唯一构造路径上为死代码（保留将违反 clippy -D warnings）。Acceptance（每行恰产出最新可见版本）达成，Preserve 项的前提（Plan 假设回溯必要）不成立，修正有效。
3. **[ACT-DEVIATION，非阻塞] R5 附加点重构（偏差 2）**：依据 Plan 自身「加载点不变」不变量，恢复期不附加 context。尝试 ① 栈溢出 → ② 转绿的记录完整，机制与不变量一致。
4. **[MINOR] 扫描替代集合每次全链预收集**（Act Minor a）：维持 Plan Risk 1 记录与条件化优化预案；本轮未跑 bench，不阻塞。
5. **[MINOR] update→delete 行旧版本在无快照扫描重现**（Act Minor b）：独立核实为**既有行为**（墓碑不抑制），且 R1 见证 UPDATE 域 [0..50, 4970..5000, 9980..10000] 与 DELETE 域 [200..250] 显式不相交——不阻塞本 Acceptance。维持记录。
6. **[MINOR，新发现] `BufferPool::mark_tx_aborted` 为 no-op**（`buffer_pool.rs:369-371`）——`mark_uncommitted_aborted` 实际空转，未提交行仅靠 `commit_tx_id=None` 不可见性兜底。既有行为、不影响本 Acceptance；建议列为 improvement 候选（未提交事务恢复清理），不在本 change 处理。

### Acceptance Gaps 与收敛判断

- **Gap**：R2-S1「混合负载恢复计数/语义精确」——`mixed_dml_recovery_semantics` RED（打开失败）。自 Cycle 002 起 gap 在测试口径上未缩小，但诊断实质收敛：002 归因 stale root（表层）→ 003 独立实证撕裂树（根因）并确认 root 策略类修复无效。同一根因的定向修复尝试次数 = 0（Act 依 Gate 6 停止），三次失败规则未触发，004 是对根因的首次修复。
- **契约边界判定**：撕裂树修复需解除 003 契约 Forbidden「恢复层重建索引」并改写 redo 三臂的索引依赖——超出当前执行契约，需新 Task Contract 与 Gate 2 → 按分类表为 `rework-required`（Acceptance 与 Map 均不变，非 replan）。
- **修复方向裁定（design D10）**：方向 a′（恢复期索引去信任 + 重放后重建）。(b) 页洞容忍拒——静默丢条目违反 K05 且孤儿页同样丢失；(c) 结构感知刷盘/驱逐改造拒——BufferPool 重设计超出既有 Acceptance 必要面，与 `do_flush` 并发、`mark_tx_aborted` 一并列为 improvement 候选。

### 后继产物

- **Next Cycle**: `iterations/000-wal-recovery-fix/004-rework.md`（repair items R-T0b-R7 redo 去索引化 / R-T0b-R8 重放后 PK 索引重建 / R-T0b-R1 收口 / R-Gate；Plan Context status = **draft**，待用户 Gate 2 批准后转 ready 交 openspec-act）
- **Iteration Plan Update**: None（Map 不变）
- **Next Iteration**: None（Iteration 000 未完成）

### 用户待决事项

1. **Gate 2 批准**：004 修复面扩大（`src/wal/recovery.rs` 重放臂 + `src/storage/btree/index_manager.rs` 洞容忍收集 + `src/storage/data/table_manager.rs` 换入 API）。
2. Incident 候选（撕裂树——驱逐规模 + 中位点 checkpoint 混合负载恢复失败，Act Response 具备完整诊断链）是否授权 `openspec-experience-recorder` 落 Incident。
3. improvement 候选两项（撕裂树运行期根修/驱逐改造；`mark_tx_aborted` 空转）是否登记。
