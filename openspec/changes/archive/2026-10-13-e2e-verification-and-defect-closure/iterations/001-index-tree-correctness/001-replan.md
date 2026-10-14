# Iteration 001 / Cycle 001: 索引与树结构正确性（分隔键落位）

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R2；Cycle 000 已落地且保持的叶链保真、`scan_all` 树遍历（页号 0 是合法子页）、`equality_candidate_children` 闭区间下探、`delete_exact_from_page` 走同一子树集合且叶级不沿叶链回退
- Excluded scope: DDL 持久化（Iteration 002 / T3）；planner 读面（Iteration 003）；文案批（Iteration 004）；同步 `BTree::search_all`（btree.rs:1070-1131）；`BTree::delete_by_key` / `delete_all_from_page`（无生产调用者）；`find_child_page_id_binary` 的等值向右路由

**Objective**

重复键索引上，等值查询与 `delete_exact` 覆盖全部在树条目。分隔键插在被分裂子页的紧右边，子树最大键不超过下一分隔键。Cycle 000 的叶链、`scan_all` 和闭区间下探保持。e2e E4 解除 ignore 后转绿。

**Background**

父 Cycle `000-initial.md` Act Response 为 `blocked`。`search_all` 在 5 键 × 400 rid 的乱序树上对 `dup01` 只返回 395 条，`scan_all` 返回 400 条，随后 `delete_exact` 对在树条目返回 `Key not found`。停止条件已命中：闭区间下探仍漏访。

审计复现（探针写入 lib 测试，跑完已从 `index_manager.rs` 删除）：树高 2，根为内部页 2，37 个分隔键。`dup01` 少的 5 条在 child[6]（页 32）：该叶 42 条 `dup00` + 5 条 `dup01`，左右分隔键都是 `dup00`，下一分隔键 sep[7] 才是 `dup01`。`equality_candidate_children` 的候选集不含该页。同一根上还有 child[29]：分隔键都是 `dup03`，叶内含 11 条 `dup04`。没有子树的最小键小于前一分隔键。

原因在 `InternalNode::insert_separator`（node.rs:733-764）和 `shift_slots_right_internal`（node.rs:779-808）。新分隔键按键查找插入位；重建时只按键排序。等值分隔键里，后插入的一条排到整段等值键末尾，不挨着刚分裂的子页。边界叶（右半含有比上推键更大的键）因此会落到更晚的等值分隔键左边，上界分隔键小于叶内最大键。闭区间公式只覆盖「前一分隔键 ≤ K ≤ 后一分隔键」的子树，进不了这片叶。放宽公式不恢复上界。

**Investigation Facts**

- Current Baseline: 父 Cycle 工作区未提交。叶链单测与多层唯一键 `scan_all` 单测已绿（Act：`cargo test --lib index_manager::tests:: -- --test-threads=1 --skip delete_exact_reaches_every_duplicate_entry`，5 passed）。`delete_exact_reaches_every_duplicate_entry` 仍红。集成测试、e2e ignore 解除、全量未跑。
- Current-State Evidence:
  - 叶分裂（node.rs:339-390，btree.rs:269）：`middle_key = sorted[mid]`，左半是 `sorted[..mid]`（键 `<= middle`），右半是 `sorted[mid..]`（键 `>= middle`，可以含有严格更大的键）。右页是新页，左页保留原页号。
  - 分隔键插入不看被分裂的页。`find_insert_position`（node.rs:767）返回首个 `>=` 键的下标。`shift_slots_right_internal` 收集全部槽后 `sort_by` 键；`sort` 对等值键稳定，新槽在物理末尾，于是新的等值分隔键排到等值串末尾。
  - 内部节点分裂后再插入（btree.rs:351-382）用 `child_split_result.middle_key >= internal_split.middle_key` 选左右半。两键相等时进右半。被分裂子页可能仍在左半，分隔键与子页拆开。
  - 根分裂（btree.rs:213-221）只有一个分隔键，左子是旧根。无等值串问题。
  - `update_parent_separator`（btree.rs:890-904）先 `remove_separator(slot)` 再 `insert_separator`。重分配要改的是这一对子页之间的键，按键重插会在等值串里挪位。
  - 下探消费侧已按闭区间实现：`equality_candidate_children`（node.rs:1107-1144），`search_all_from_page_async` 与 `delete_exact_from_page` 已调用它。本 Cycle 不改这个函数的区间。
  - `find_child_page_id_binary`（node.rs:1063-1090）等值时进该分隔键的右子。插入路由继续用它。
- Code and Critical Path: `src/storage/btree/node.rs` 的 `insert_separator` / `shift_slots_right_internal`；`src/storage/btree/btree.rs` 的 `insert_into_page`、根分裂、内部节点分裂后的再插入、`update_parent_separator`。测试在 `src/storage/btree/index_manager.rs` 的 `delete_exact_reaches_every_duplicate_entry`，以及 node.rs 里直接调用 `insert_separator` 的单测（签名要跟着改，唯一递增键的断言不变）。

**Implementation Guidance**

新分隔键的左兄弟是被分裂的那一页。`leftmost_child` 等于该页时，新分隔键放在下标 0，左子仍是该页，右子是新页。某个分隔键的右子等于该页时，新分隔键插在它的下一个下标，原分隔键的右子不变。找不到该页时返回错误，不要退回按键排序。

内部节点本页放不下而分裂时，先按现有分裂把分隔键切成两半，再看被分裂子页落在哪一半，只在那一半里按上面的规则拼接。不要用上推键与待插入键的大小比较选边。

`update_parent_separator` 在原槽位改写键和右子页。左右相邻分隔键的顺序不动。

`shift_slots_right_internal` 若仍负责拼接，必须保留 leftmost 的页头字节，并在把字节拷回后 `reload_header`。它不再把新的等值分隔键排到等值串末尾。

**Task Contracts**

### T2: 分隔键落在被分裂子页右侧（R2）

- Requirement/Scenario: R2 / S1–S6 + E4 正确性面
- Depends on: T1（已 accepted）；父 Cycle 已完成的叶链、`scan_all`、闭区间下探接线
- Targets: `src/storage/btree/node.rs::InternalNode::insert_separator`、`src/storage/btree/node.rs::shift_slots_right_internal`、`src/storage/btree/btree.rs::insert_into_page`、`src/storage/btree/btree.rs` 根分裂处的 `insert_separator`、内部节点分裂后的再插入、`src/storage/btree/btree.rs::update_parent_separator`
- Current behavior: 5 键 × 400 rid 乱序插入后，`scan_all` 能数到每个键 400 条，`search_all(dup01)` 为 395，`delete_exact` 对漏掉的条目返回 `Key not found`。漏掉的条目所在叶，上界分隔键严格小于叶内最大键。
- Required behavior: 分裂产生的分隔键位于被分裂子页与新页之间。任一内部节点上，左子树的键都 `<=` 第一个分隔键；第 i 个右子（分隔键 i 的右页）的键都 `>=` 分隔键 i，并且在存在下一分隔键时都 `<=` 下一分隔键。`search_all` 与 `delete_exact` 因此能到达全部重复键条目。`equality_candidate_children` 的区间不改。
- Preserve: `find_child_page_id_binary`；`search_from_page_async`；叶链恢复；`scan_all` 的栈遍历及页号 0；`delete_exact` 的候选子树与叶级不沿叶链；同步 `search_all` 与 `delete_by_key`；公开的 `IndexManager` 方法签名；唯一键、范围谓词、复合谓词、主键点查。
- Forbidden: 为了盖住漏访而放宽 `equality_candidate_children`；改页格式；改 Iteration 002–004 的文件；清理死代码。
- First test: 已有 `delete_exact_reaches_every_duplicate_entry`（`src/storage/btree/index_manager.rs`）。变更前运行 `cargo test --lib delete_exact_reaches_every_duplicate_entry -- --test-threads=1`，预期失败：`search_all` 或 `delete_exact` 到不了全部 `dup01`。不要改这个夹具的 5×400 形状。另加一个 lib 测试：8 个键 × 800 个 rid、同一洗牌序插入；插入后从根向下必须能走到内部子页（树高至少 3）；每个键 `search_all` 的条数等于 800；随后逐条 `delete_exact` 成功，再 `search_all` 为空。变更前这个测试同样失败。已绿的 `shift_slots_right_preserves_leaf_chain_on_mid_inserts` 与 `scan_all_returns_full_set_on_multi_level_tree` 保持绿，先跑一次作为基线。
- GREEN condition: 上述三个 lib 测试通过。`tests/secondary_index_equality_test.rs` 覆盖 S1–S3（15000 或 30000 行、`i % 100 + 1`、建索引前后 `COUNT(*)` / `COUNT(DISTINCT id)` 逐键相等、边界行 UPDATE 后旧键减一新键加一、失败语句前后计数不变）。`full_table_dml_test` 增加乱序主键至少 10000 行、树高至少 3 的无 WHERE UPDATE/DELETE，`affected_rows` 等于行数。e2e `e4_index_correctness_surface` 去掉 `#[ignore]` 后通过。
- Verification: `cargo test --lib index_manager::tests:: -- --test-threads=1`；`cargo test --test secondary_index_equality_test --test full_table_dml_test --test e2e_test -- --test-threads=1`；定向回归 `secondary_index_test`、`btree_scale_test`、`btree_split_test`、`btree_merge_test`、`update_index_maintenance_test`、`checkpoint_redo_reduction_test`。全量按 M19 分批一次，各批 0 failed。node.rs 里 `insert_separator` 调用点按新签名编译通过，原断言不改语义。
- Stop when: 分隔键已贴着被分裂子页，子树最大键仍大于下一分隔键（还有第二条生产路径在搬分隔键）；树高至少 3 的夹具仍漏访；`scan_all` 或叶链测试由绿变红。出现这些情况时停，把复现带回 Plan，不要改下探区间。

**Invariants**

- 异步路径不在持有 std Mutex 期间 await。
- 先有失败测试再改插入路径。断言直接看 `search_all` / `delete_exact` 的条数、错误和 `affected_rows`。

**Non-goals**

见 Excluded scope。不迁移已经写坏的存量索引文件。不改 `delete_by_key`。

**Acceptance**

- R2：S1–S3 由 `secondary_index_equality_test` 覆盖；S4/S5 由 `full_table_dml_test` 与 `scan_all` 单测覆盖；S6 由叶链单测覆盖；E4 解除 ignore 后通过。
- 5×400 与 8×800 两则 lib 测试里，每个重复键的 `search_all` 条数等于插入条数，`delete_exact` 全部成功。
- RTM：R2 → T2 → 上述测试（Covered）。

**Verification**

SQL 行为用 lib API 或 CLI 断言行数和退出状态。B-tree 行为用 `IndexManager::search_all`、`scan_all`、`delete_exact`、`insert` 断言集合大小。全量按 M19 分批。

**Gate 2 Readiness**

PASS——用户 2026-10-13 以「批准」接受本 Cycle。无 TBD；需求无裁剪；tasks 与 design D2 审计修正一致；插入路径、失败夹具和停止条件已写明。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- node.rs 现有单测以 `(key, page_id)` 调用 `insert_separator`。签名增加左兄弟页之后，这些调用要改成「第一个分隔键的左兄弟是 leftmost，其后每个分隔键的左兄弟是上一个右子页」。断言仍是键的升序和子页号，不改期望值。
- 8×800 若树高不到 3，夹具不够，按 Stop when 停。不要把规模改小来换取通过。

## Act Response

- Status: reported

**Changes**

- `InternalNode::insert_separator` 增加左兄弟页。`separator_index_after_child`：leftmost 命中则下标 0，否则插在右子页等于该页的分隔键的下一个下标；找不到该页返回错误，不按键排序。`splice_separator` 只搬动槽位，保留 leftmost 页头并 `reload_header`。
- 内部节点放不下时先按原 `split()` 切半，待插入分隔键只进入仍持有被分裂子页的那一半。根分裂的左子是旧根。
- `update_parent_separator` 改为 `replace_separator`，原槽改写键和右子页。
- node.rs 里原有 `insert_separator(key, page)` 调用改为传入左兄弟（第一个是 leftmost，其后是上一个右子页）。断言未改。
- 新增 lib 测试 `delete_exact_reaches_every_entry_on_height_at_least_3`（8 键 × 800 rid，同一洗牌序，树高至少 3）。5×400 夹具未改。
- `tests/e2e_test.rs` 的 `e4_index_correctness_surface` 去掉 ignore。
- `BTree::merge_leaves` 改为整页重写左叶再接上右叶的后继指针。碎片叶上逐条 `insert` 会在键数仍低于容量时返回 `PageFull`；合并后的条数满足 `2*MIN_KEYS-1`，重写能装下。

**Deviations from Plan**

- 全表 UPDATE 已能碰到 20000 行。随后的全表 DELETE 在 `merge_leaves` 失败：左叶碎片化，逐条插入右叶条目时 `LeafNode::insert` 返回 `PageFull`（调用栈是 `DeleteExecutor::delete_row` → `IndexManager::delete` → `handle_leaf_underflow`）。契约的插入路径没有这条合并。不改它，S4 的 `affected_rows = 20000` 过不了。合并语义仍是把两片叶子收成一片并摘掉父分隔键，只是写入方式从逐条插入换成 `rebuild_leaf`。`equality_candidate_children`、`find_child_page_id_binary`、叶链和 `scan_all` 未改。

**Self-Review**

- Gate 3：分隔键贴被分裂子页；内部节点分裂按子页所在半边拼接；`update_parent_separator` 不重排邻居。闭区间下探、点查二分、叶链、`scan_all`（页号 0）保持。诊断打印已从 node.rs、btree.rs、index_manager.rs、pipeline.rs、catalog.rs、data_page.rs 去掉。
- 内部节点合并 `merge_internal_nodes` 仍按键排序分隔键，本轮未改。唯一键全表删除和 `btree_merge_test` 已通过。重复键的 `delete_exact` 不重平衡，没有走到这条路径。

**Verification**

- `cargo test --lib index_manager::tests:: -- --test-threads=1`：7 passed / 0 failed（11.10s）。含 5×400、8×800、叶链、多层 `scan_all`。
- `cargo test --test secondary_index_equality_test --test full_table_dml_test --test e2e_test -- --test-threads=1`：e2e 13 passed / 0 failed / 9 ignored（68.20s，`e4_index_correctness_surface` 在其中）；full_table 11 passed（33.57s，含 `s27`）；equality 3 passed（56.58s）。
- 定向：`secondary_index_test` 27、`btree_scale_test` 5、`btree_split_test` 12、`btree_merge_test` 10、`update_index_maintenance_test` 11、`checkpoint_redo_reduction_test` 9，各 0 failed。
- M19：`tests/*.rs` 名字典序 120 个集成目标，每批 10 个，`cargo test --test … -- --test-threads=1`，12 批全部 0 failed；`--lib` 398 passed / 0 failed；`--bins` 0 passed / 0 failed。合计 1950 passed / 0 failed / 11 ignored。

**Persisted Evidence**

None required。上述命令可复跑。

**Experience Candidates**

None

## Plan Review

- Review Result: accepted

**Findings**

独立读了 `insert_separator`、`separator_index_after_child`、`splice_separator`、`replace_separator`、根分裂与内部节点分裂后的再插入、`update_parent_separator`、`equality_candidate_children`，以及 `merge_leaves`。分隔键按被分裂子页拼接，找不到该页就返回错误，不再按键排序。内部节点分裂用子页是否落在右半来决定待插入分隔键进哪一半，不再用两个键的大小比较。`update_parent_separator` 走 `replace_separator`，原槽改写。闭区间下探函数的区间与 Cycle 000 相同，没有放宽。根分裂把旧根当作左兄弟。`splice_separator` 保留 leftmost 并 `reload_header`。

`merge_leaves` 改成整页重写，是契约目标列表之外的改动。全表 DELETE 在碎片叶上逐条插入会返回 `PageFull`，不改的话 S4 的 `affected_rows = 20000` 过不了。重写之后仍是两片叶子收成一片、摘掉父分隔键、左叶的后继指针改成右叶原来的 next。`s27_full_table_dml_multi_level_pk_tree` 已通过，采信 Act。这是为了满足既有验收的实现偏差，不构成缺口。

非阻塞：

- `merge_internal_nodes` 仍按键排序分隔键。`delete_exact` 不重平衡，重复键测试没有走到这里。唯一键全表删除和 `btree_merge_test` 已通过。等值分隔键若将来走上内部节点合并，排序仍可能把子页挪开；当前验收路径到不了。
- 5×400 的 lib 测试没有在删除前断言 `search_all` 条数。逐条 `delete_exact` 成功已经要求每条都能下探到。8×800 那则在删除前断言了条数，并断言树高至少 3。
- `s27` 没有单独断言树高。夹具是 20000 个乱序唯一键，`affected_rows` 等于 20000。

**Acceptance Gaps**

无。R2 的 S1–S3 由 `secondary_index_equality_test` 覆盖，S4/S5 由 `full_table_dml_test` 的 `s27` 和 `scan_all` 单测覆盖，S6 由叶链单测覆盖，E4 已去掉 ignore。5×400 与 8×800 的 `delete_exact` 全部成功。

**Evidence**

`node.rs` 736–839、`btree.rs` 213–225 与 353–390、`btree.rs` 583–625 与 891–904、`index_manager.rs` 774–858、`tests/e2e_test.rs` 的 `e4_index_correctness_surface` 无 ignore、`tests/full_table_dml_test.rs` 的 `s27`、`tests/secondary_index_equality_test.rs`。Act 的验证采信：lib `index_manager::tests` 7 passed；e2e 13 passed / 9 ignored；`full_table_dml_test` 11 passed；`secondary_index_equality_test` 3 passed；定向六项目 0 failed；M19 全量 1950 passed / 0 failed / 11 ignored。Persisted Evidence 为 none，不另建目录。

**Follow-up Decision**

验收满足。`merge_leaves` 的偏差留在记录里，不返工。Iteration 001 完成。按原 Map 展开 Iteration 002。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

`iterations/002-ddl-persistence/000-initial.md`（已展开，Plan Context ready）
