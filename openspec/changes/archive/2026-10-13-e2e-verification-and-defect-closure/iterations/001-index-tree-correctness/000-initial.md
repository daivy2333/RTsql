# Iteration 001 / Cycle 000: 索引与树结构正确性

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R2（`sql-secondary-indexes` delta ×2 MODIFIED）；ISS05 台账 + 调查修正（B1/B2 同面并入，design D2/D3）；用户决策 D-3
- Excluded scope: DDL 持久化与原子性（Iteration 002 / T3）；planner 读面（Iteration 003）；文案批（Iteration 004）；同步 `BTree::search_all` 死代码（btree.rs:1070-1131，无生产调用者，不动）

**Objective**

非唯一二级索引等值查询结果集与 DataScan 双向严格相等（任意重复键规模）；树遍历原语（`IndexManager::scan_all`）对任意层树返回全部条目；叶子链在任何叶内重排后保真；失败语句对索引零副作用；大表无 WHERE 全表 UPDATE/DELETE 触达全部行。e2e E4 正确性面解除 ignore 转 GREEN。

**Background**

ISS05（外部评测 P0-1）+ Plan 调查修正（proposal Why / design D2/D3）：回填无辜；三缺陷在 btree 层——机制 A（等值分隔键双侧收集重复发射）、机制 B（同值键跨「小于 K 的分隔键」边界散布 + 遇等值分隔键跳过常规路由 → 边界子树漏访；`delete_exact` 单径下探 + 叶链回退〔叶链恰被 B1 破坏〕同源）、B1（`shift_slots_right` 重建清零叶链指针）、B2（异步 `scan_all` 内部页当叶读 + leftmost 单链走 + page-0 哨兵 → 多层树返回空/碎片）。

**Investigation Facts**

- Current Baseline: Iteration 000 accepted（2026-10-13）——e2e 套件入库（12 passed / 10 ignored），产品代码零变化；全量基线 1929 tests / 0 failures / 2 ignored 仍有效；`e4_index_correctness_surface`（15000 行）与 `e3b` 等ignore 项即为后续验收入口。
- Current-State Evidence（全部经本会话源码读证与探针实证）：
  - **等值下探现状**：`src/storage/btree/index_manager.rs:192-257` `search_all_from_page_async`——内部节点对每个 `sep == key` 收集左右两子页（相邻等值分隔键共享中间子页 → 重复收集成倍放大）；`child_page_ids` 非空即**跳过常规路由**（:240-243）——同值键散布在「分隔键 < K」子树时该子树永不被访问。探针实证（15000/30000 行，n∈1..100 确定性分布）：重复发射 23420、漏发 476、全文件页枚举证明漏发条目**物理在树中**。
  - **结构不变量**（修复算法依据）：分裂以 `sorted[mid]` 为 middle_key、右半含一切 `>= middle` 的键 → `child(j)` 键域 ⊆ `[sep_j, sep_{j+1}]`（**闭区间**，两端可取等）；叶序全局有序、同值键在叶序连续。探针实证形态：分隔键 `(2,252)|(3,31)`，叶 252 尾部持 v=3 条目。
  - **正确子树集合**：内部节点上记 `lo` = 首个 `sep >= K` 下标（二分下界）、`hi` = 末个 `sep <= K` 下标。需访问子树 = `{ child(j) : j ∈ [max(0, lo-1), hi] } ∪ (lo == 0 ? { leftmost } : ∅)`。验证：K 严格介于分隔键间 → 单子树（≡常规路由）；单等值分隔键 → 其左右两子树（≡现意图，无重复）；多等值分隔键连串 → 覆盖全部相邻子树（修复漏访）。每子树恰访一次。
  - **delete_exact 现状**：`src/storage/btree/btree.rs:1204-1288`——内部节点对**首个**等值分隔键尝试左后右（KeyNotFound 回退，:1260-1280），无等值分隔键则单径二分路由（:1282-1285）；叶级命中键但无目标 rid 时沿 `next_leaf_page_id` 链继续（:1241-1242）——该链恰被 B1 破坏 → 回退失效 → KeyNotFound（探针 A 实证：对在树条目 `delete_exact` Err KeyNotFound，eval 的 `UPDATE … Key not found` 同源）。
  - **B1 现状**：`src/storage/btree/node.rs:164-197` `LeafNode::shift_slots_right`——读全部条目 → `Page::new` + `LeafNode::init` 重建 → `copy_from_slice` 回拷，**不恢复 `next_leaf_page_id`、不 `reload_header`**。对照 internal 版 `shift_slots_right_internal`（:769-799）已有保真先例（重建前读 `header().next_page_id`、重建后写 `data[5..9]` + `reload_header()`）。`LeafNode::split`（:329-380）已 `reload_header` ✓。全页普查实证：~90% 叶子 next=0。
  - **B2 现状**：`src/storage/btree/index_manager.rs:339-373` `scan_all`/`scan_all_async_from_root`——`while page_id.0 != 0` 哨兵、把内部页当叶读（`get_row_id` 长度校验 36B<38B 全 None → 静默丢弃）、只沿 header.next（内部页 = leftmost）单链走。多层树返回 0/碎片（探针：190/30000）。消费面四处：`src/executor/update.rs:108`、`src/executor/delete.rs:119`、`src/executor/scan.rs:59`、`src/storage/data/table_manager.rs:128`（全表 DML 双执行器 + Scan 执行器 + table_manager 内部通道）。
  - **失败语句索引副作用**（ISS05 事件记录）：对缺失条目行执行失败 UPDATE 后 abort 重插改变索引计数（探针实证 COUNT +2/DISTINCT +1）；全表 UPDATE 在 30000 行表整语句失败 `Abort failed: Key not found`（abort 路径亦触 `delete_exact`）。下探修复后两类路径自然可达；零副作用语义以测试锁定。
  - **不动面**：`IndexManager::search`（唯一索引点查，`search_from_page_async` :161-188 二分定位）与 `insert` 路由（`find_child_page_id_binary`，node.rs:1053-1081，等值路由右侧——插入落路由子树保持叶内有序与全局叶序，无正确性问题）；恢复重建通道（`rebuild_pk_indexes` 自数据页版本链重建，不走 `scan_all`/`search_all`）；`row_to_key` 反查映射（delete_exact 包装 :318-336 现状正确）。
- Code and Critical Path: `src/storage/btree/index_manager.rs`（search_all/scan_all）、`src/storage/btree/btree.rs`（delete_exact_from_page）、`src/storage/btree/node.rs`（shift_slots_right）；消费面零改动继承；测试面 `tests/secondary_index_equality_test.rs`（新）+ btree 单测 + `full_table_dml_test` 扩展 + e2e 解除 ignore。

**Implementation Guidance**

建议序：③叶链保真（最小独立面，先行）→ ①search_all 范围下探 → ②delete_exact 对齐（同一子树集合；叶级候选叶穷尽后仍无目标才 KeyNotFound，不依赖叶链）→ ④scan_all 树遍历重写 → ⑤零副作用与大表用例。范围下探的 `lo`/`hi` 可在 `InternalNodeRef` 上以两次二分实现（`find_key_position_binary` 返回 (found, pos) 可复用：found 即 lo==hi==该位；未命中 pos 即 lo，hi = pos-1 或按末分隔键比较）。scan_all 用显式栈 + visited 去重（探针验证过的算法形态：内部页收集 leftmost + 全部分隔键子页，叶页收集条目；M13 约束下迭代实现避免 async 递归 boxing）。既有单测 `index_manager_test.rs`/`btree_*` 如有对旧形状的直构断言随实现机械校准（语义零变化）；公开 API 签名不变。

**Task Contracts**

### T2: 等值下探 + 树遍历原语 + 叶链保真（R2）

- Requirement/Scenario: R2 / S1–S6（proposal 场景草图）+ E4 正确性面
- Depends on: T1（已 accepted；解除 `e4_index_correctness_surface` ignore）
- Targets: `src/storage/btree/index_manager.rs::search_all_from_page_async`、`src/storage/btree/index_manager.rs::scan_all[_async_from_root]`、`src/storage/btree/btree.rs::delete_exact_from_page`、`src/storage/btree/node.rs::shift_slots_right`
- Current behavior: 等值扫描重复发射 + 漏发（15000 行实测虚发/漏发）；`delete_exact` 对跨边界条目 KeyNotFound；`shift_slots_right` 清零叶链指针（~90% 断链）；`scan_all` 多层树返回空/碎片（190/30000）→ 全表 UPDATE/DELETE 与 Scan 执行器大表失真。
- Required behavior: ① `search_all_from_page_async` 按正确子树集合下探（Investigation Facts 公式），每子树恰访一次、无「有等值即跳过路由」分支；② `delete_exact_from_page` 内部级同集合下探、叶级候选叶穷尽后 KeyNotFound（不依赖叶链）；③ `shift_slots_right` 重建后恢复 `next_leaf_page_id` 并 `reload_header`（internal 版同型）；④ `scan_all` 树遍历式（显式栈、内部页感知、visited 去重、不依赖叶链、无 page-0 哨兵依赖——终止条件为栈空）；⑤ 失败语句（含 abort 路径）索引零副作用、大表全表 DML affected=行数（随①—④自然达成，以测试锁定）。
- Preserve: `search`/`insert` 路由与 `search_from_page_async` 不动；同步 `BTree::search_all`（btree.rs:1070-1131）不动；恢复重建通道不动；`row_to_key` 语义不动；公开 API 签名不变；唯一索引点查/范围/复合谓词回落/PK 点查既有面零回归（M18 恢复去信任不变量不涉及）。
- Forbidden: DDL/持久化面（Iter 002）；planner（Iter 003）；文案（Iter 004）；页格式/序列化变更；`delete`/`update` 歧义原语语义变更（继续仅限唯一索引，见其 doc 注记）。
- First test: 先行测试 RED 三层——i) 新 `tests/secondary_index_equality_test.rs`：15000 行确定性分布（`i%100+1`）建索引前后逐 k `COUNT(*)`/`COUNT(DISTINCT id)` 对照（基线经建索引前 DataScan 取得）+ 边界行 UPDATE 探针（改 n 后旧键计数递减、新键递增）+ 失败语句前后索引计数不变；ii) btree 单测：乱序键插入后逐叶 `next_leaf_page_id` 非零链完整（shift 保真）、多层树 `scan_all` == 插入集合、随机键全条目 `delete_exact` 逐条可达；iii) `full_table_dml_test` 扩展：≥3 层 PK 树（乱序键 ≥10000 行）无 WHERE UPDATE/DELETE `affected_rows` = 行数且逐行触达；iv) `cargo test --test e2e_test e4_index_correctness_surface --ignored -- --nocapture` 观察 RED。当前全部 RED（形态见 Investigation Facts 探针数字）。
- GREEN condition: 上述测试全绿；e2e `e4_index_correctness_surface` 解除 `#[ignore]` 后全绿；既有 secondary_index/btree_*/index_manager/update_index_maintenance/full_table_dml/scan 相关套件零回归（机械校准点逐条记录）。
- Verification: 定向：新测试文件 + `cargo test --test e2e_test -- --test-threads=1` + `secondary_index_test`/`btree_scale_test`/`btree_split_test`/`btree_merge_test`/`index_manager_test`/`update_index_maintenance_test`/`full_table_dml_test`/`checkpoint_redo_reduction_test`（恢复重建面）各单目标。全量：M19 分批一次（本 Iteration 预算），各批 0 failed。
- Stop when: 范围下探在某树形态下仍漏访（说明结构不变量假设不成立——带复现返 Plan）；发现恢复/回滚通道依赖旧 `scan_all` 形状的隐藏耦合；性能面出现量级退化（>10×，需评估算法选择——返 Plan）。

**Invariants**

- M13 异步纪律（async 路径不持 std Mutex 跨 await；同步写经 spawn_blocking 不变）；M17 flush/淘汰约束不涉及；M18 恢复去信任不变量不涉及。
- AGENTS TDD（先 RED 后变更）与验证纪律（直接断言目标行为，无判定层）。

**Non-goals**

见 Excluded scope；另：存量已断链索引文件的迁移修复（预防式——修复后新写入正确；存量文件经恢复重建通道重开自愈的场景由既有恢复语义承载，不新增迁移工具）。

**Acceptance**

- R2 Covered：S1（等值严格相等）→ `secondary_index_equality_test` 基线对照；S2（边界 UPDATE 探针）→ 同文件；S3（失败零副作用）→ 同文件；S4/S5（多层树全表 DML/全量枚举）→ `full_table_dml_test` 扩展 + btree 单测；S6（叶链保真）→ btree 单测；E4 正确性面解除 ignore GREEN。
- RTM：R2 → T2 → 上述测试（Covered）。

**Verification**

逐 scenario 最简直接判定：SQL 行为经 lib API 或 CLI 真实入口断言行集/计数；btree 原语经公开 API（`IndexManager::{search_all,scan_all,delete_exact,insert}`）断言集合等值；全量按 M19 分批。

**Gate 2 Readiness**

PASS——契约自包含（现状代码位置与形态、正确算法公式、不动面、RED 定义、停止条件齐备）；无 TBD；Persisted Evidence = none。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 范围下探的复杂度上界：等值分隔键连串长度 m → 子树集合 O(m)——重复键极端偏斜（同值占比 85%+）下访问子树数与现有「等值双侧收集」同阶，无新增量级风险（探针 t2 形态 8000 行 85% 同值已属该类）。
- 既有直构测试若有对节点形状的断言，机械校准允许（语义零变化），逐条记录。
- e2e E4 绿面（30000 行）与本 Iteration 新测试（15000 行）共用播种形态，运行时长已在套件预算内。

## Act Response

- Status: blocked

**Changes**

- `src/storage/btree/node.rs::LeafNode::shift_slots_right`：本轮开始前已恢复 `next_leaf_page_id` 并 `reload_header`。单测 `shift_slots_right_preserves_leaf_chain_on_mid_inserts` 绿。
- `src/storage/btree/node.rs::InternalNodeRef::equality_candidate_children` 与 `IndexManager::search_all_from_page_async`：本轮开始前已按计划公式下探。
- `src/storage/btree/index_manager.rs::scan_all_async_from_root`：改为显式栈树遍历（内部页收集 leftmost 与全部分隔键子页，叶页收集条目，visited 去重，栈空终止）。页号 0 是合法子页（根分裂后旧根），不再当作哨兵跳过。
- `src/storage/btree/btree.rs::delete_exact_from_page`：内部节点改走 `equality_candidate_children`；叶级只删本页命中项，不再沿 `next_leaf_page_id` 回退。
- 删除 `index_manager` 测试模块里三个只打印、不断言的临时探针（`probe_leftmost_zero_bisect`、`probe_invariant_audit`、`probe_deficit_shape`）。

**Deviations from Plan**

- `scan_all` 不能把子页号 0 当成「无页」。计划里的「无 page-0 哨兵」指终止条件是栈空，不是丢弃页号 0。丢掉之后 3000 条唯一键只扫到 2943；保留页 0 后 `scan_all_returns_full_set_on_multi_level_tree` 绿。

**Self-Review**

- Gate 3 未通过。`search_all` / `delete_exact` 使用的候选子树公式在重复键树上漏访，S1/S3 与 delete 单测达不到。
- 已删除临时探针，避免它们进入 lib 测试。
- 未改 `search` / `insert` 路由，未改同步 `BTree::search_all`。

**Verification**

- `cargo test --lib index_manager::tests:: -- --test-threads=1 --skip delete_exact_reaches_every_duplicate_entry`：5 passed / 0 failed（叶链、多层树 `scan_all`、既有 `delete_exact` 小例、`collect_all_pages` 两例）。
- `delete_exact_reaches_every_duplicate_entry` RED：5 键 × 400 rid 乱序插入后、任何删除之前，`scan_all` 中 `dup01` 为 400，`search_all` 为 395。随后 `delete_exact(dup01, RowId { page_id: 1540, slot_id: 140 })` 返回 `Key not found`。
- 集成测试、e2e ignore 解除、全量分批未跑。

**Persisted Evidence**

None required。漏访可由上述 lib 单测复现。

**Experience Candidates**

None

**Unresolved**

见 Blocker Handoff。

**Blocker Handoff**

- Task: T2。Step: ① `search_all_from_page_async` 范围下探（公式已接入）之后的先行测试。Gate: 5（Stop when：范围下探仍漏访）。
- 计划预期：`equality_candidate_children` 的闭区间公式覆盖全部持有键 K 的子树，每子树恰访一次。
- 实际：同一棵树上 `scan_all` 能枚举全部 400 条 `dup01`，公式下探只有 395 条。条目在树里，但不在公式给出的子树集合中。结构不变量「child(j) 键域 ⊆ [sep_{j-1}, sep_j]」在这棵重复键树上不成立。继续放宽公式会离开当前契约。
- 已完成：叶链保真（③）；`scan_all` 树遍历（④），含页号 0 为合法子页。
- 部分完成：① 公式已接线但漏访；② `delete_exact` 已改同集合下探且不再依赖叶链，因①漏访仍 KeyNotFound。
- 未开始：⑤ 失败语句零副作用与大表全表 DML 的 GREEN；e2e `e4_index_correctness_surface` 解除 ignore；定向回归与全量分批。
- 工作区：上述源码与单测改动未提交。Gate 3/4 未通过。
- 恢复条件：`openspec-plan` 按这次复现修订下探不变量或候选子树公式，并更新本 Cycle 的 Task Contract（或后继 Cycle）。修订前不要再改公式碰运气。

## Plan Review

- Review Result: replan-required

**Findings**

独立读了 `equality_candidate_children`（node.rs:1107）、`search_all_from_page_async`、`delete_exact_from_page`、`shift_slots_right`、`scan_all_async_from_root`，以及叶分裂与 `insert_separator`。Act 声明的四处改动都在工作区里。叶链恢复和 `scan_all` 的栈遍历与契约一致；页号 0 不再被丢掉，和契约里「终止条件是栈空」一致，不是漏做。Act 对 `scan_all` 的偏离记录成立，不构成缺口。

漏访属 Plan 错误，不是下探函数少写一个下标。用与 `delete_exact_reaches_every_duplicate_entry` 相同的 5 键 × 400 rid 夹具读树（临时探针，跑完已从 `index_manager.rs` 删除）：`scan_all` 中 `dup01` 为 400，`search_all` 为 395。少的 5 条在根的 child[6]，左右分隔键都是 `dup00`，叶内还有 42 条 `dup00`；sep[7] 才是 `dup01`。候选集不含这页。另一处同类破坏是 child[29]（`dup03` 分隔键之间含有 `dup04`）。没有最小键小于前一分隔键的子树。

`insert_separator` 按键排序，新的等值分隔键排到等值串末尾，所以后一次分裂会插到已经含有更大键的边界叶右边。design D2 把这种树当成闭区间去搜。闭区间消费公式只能覆盖分隔键真正封顶的子树。父 Cycle 的 Stop when 因此命中，Act 停手符合契约。继续放宽公式离开原契约，也盖不住「更大的键在更小分隔键左边」。

**Acceptance Gaps**

R2 的 S1/S2/S3 与 E4 未满足。S6（叶链）和 S5 的 `scan_all` 多层唯一键枚举已在 lib 单测里通过，采信 Act 的 `cargo test --lib index_manager::tests::`（跳过仍红的 `delete_exact_reaches_every_duplicate_entry`，5 passed）。S4 全表 DML、e2e、定向回归和全量未跑。

**Evidence**

`000-initial.md` 的 Act Response；node.rs / index_manager.rs / btree.rs 上述符号；临时探针对 5×400 树的 stderr（`scan dup01=400 search dup01=395`，child[6] hist `dup00: 42, dup01: 5`）。探针已删除，不留在树上。

**Follow-up Decision**

设计改为：闭区间下探保持不动，分隔键改贴被分裂子页插入。Iteration 001 的验收结果不变，诊断面加上插入路径。后继 Cycle 见下。当前 Cycle 不再修。

**Iteration Plan Update**

`tasks.md` 的 T2 与 Iteration 001 诊断边界已改为包含分隔键落位。阶段结果、任务划分和后继 Iteration 不变。

**Next Cycle**

`iterations/001-index-tree-correctness/001-replan.md`（Plan Context 为 `draft`，等用户批准后才能交给实施）

**Next Iteration**

None
