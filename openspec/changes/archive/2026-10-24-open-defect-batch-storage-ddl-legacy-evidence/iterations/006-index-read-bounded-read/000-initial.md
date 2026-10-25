# Iteration 006 / Cycle 000: 索引树读路径的有界读与显式拒绝

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T22, T23
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 既有域 `data-integrity-verification`（完整性扫描的判据与诊断分级）、`design.md` D13（索引树读路径的损坏页语义：显式错误、不跳过）、`design.md` D6 与既有域 `release-path-bounded-read`（RPBR R1：释放路径的有界读与尽力而为语义——**处置方向与本 Iteration 相反**）、Iteration 002 已验收的释放路径有界读成果（`src/storage/integrity.rs` 的 `check_tree_page` / `validate_slots` 读法）、Iteration 002 的损坏注入夹具形态（`tests/release_path_bounded_read_test.rs` 的按文件偏移改写槽目录项）、`design.md` D12 与 `.agents/runbooks/m19-batched-full-regression.md`（R48）——收尾全量的预算与 M19 分批串行纪律、ISS23 的规模约束（本 change 内不使用 ≥ 300 行的表规模构造用例）、`design.md` D11（禁止事项）。
- Excluded scope: `SlottedPageRef::get_slot_data` 的**签名**改造（波及全部调用点，含数据页 / 目录页 / 恢复面 / 缓冲池同形态约 17 处，属范围外候选）；存储层其余无界读点（数据页 / 目录页 / 恢复面 / 缓冲池）各自的「损坏页正确行为」（需各自调查与设计决定，另立专项 change 承载）；释放路径的处置语义（已由 Iteration 002 闭合：告警 + 跳过 + 返回，**不**改成整体失败）；完整性扫描的诊断分级；`free_page` / `complete_free_release` / `promote_free_releases` 的集合语义；磁盘格式（保 MS36 的 G1/G2/G3 兼容面）；Iteration 001 / 003 / 004 / 005 的成果面；本 change 的 Iteration 007 与收尾全量（T21）。

**Objective**

损坏的索引树页在**读 / 写热路径**（`SELECT` / `INSERT` / `DELETE` / `UPDATE` 的索引下降与条目提取）上**不再越界切片 panic、不再导致进程 abort**；损坏的索引页给出**明确的错误**（点名损坏的页与它所属的索引树），SHALL NOT 被跳过成「查不到该行」（跳过一个子树 ⇒ 下降无法确定子结点 ⇒ 若当作「没有这个子树」继续 ⇒ 点查返回「查不到该行」⇒ **静默错读**，比 abort 更隐蔽）；未损坏路径的读 / 写行为**零变化**。

**Background**

本 Iteration 的缺陷由 **Iteration 002 的 Plan Review（2026-10-25）** 登记承载：Iteration 002 闭合的是释放路径（`DROP TABLE` / `TRUNCATE` / `DROP INDEX`）的读面，而 Act 实测发现**同一根因层**（`src/storage/btree/node.rs` 的索引树访问器族用未检查的 `get_slot_data` 无界读）在**普通 `SELECT` / `INSERT` / `DELETE` / `UPDATE` 的索引下降与条目提取**上仍然存在 ⇒ 损坏的索引页在读 / 写热路径上仍会越界切片 panic（进程 abort）。`design.md` D13 已把该条的决定写死：损坏的索引页在读 / 写路径上给出**明确的错误**（点名损坏的页与索引），SHALL NOT abort；未损坏路径零变化；**不复用**释放路径的「告警 + 跳过」（读路径跳过 ⇒ 静默错读）。

**Investigation Facts**

- Starting point:
  - 工作区 HEAD `0a08b14`（ahead of origin 9）；MS34 + issue-fix + MS35 + MS36 + 本 change Iteration 000–005 的改动**全部未提交**（`src/`、`tests/`、`tests/fixtures/legacy/`、`README.md` / `README.zh-CN.md`、`.agents/` 与 `openspec/specs/` 下的文档改动）。属**既有现场**：本 Cycle 在其上继续，**不回滚、不清理、不提交**。
  - Iteration 000–005 的 `Review Result` 均为 **`accepted`**（005 由 Plan Review 2026-10-25 判定：Acceptance 1–6 全部满足）⇒ 本 Cycle 不依赖任何未完成的 Iteration。
  - 前序验证结论的**有效性裁定**：Iteration 005 的 Act Response（2026-10-25）记录 `cargo test --lib -- --test-threads=1` → 420 passed / 0 failed、`cargo build --tests` → 0 warning、`index_page_ownership_test` 5/0、`index_page_ownership_stack_overflow_test` 1/0、`index_manager_test` 3/0、`secondary_index_test` 27/0。**本 Cycle 改 `src/storage/btree/node.rs` / `src/storage/btree/btree.rs` / `src/storage/btree/index_manager.rs`** ⇒ 上述覆盖到这些文件的结论**对改动面失效**，必须在本 Cycle 重跑（不是沿用）。MS35 / MS36 的全量终态 2192 passed / 0 failed / 2 ignored（来源 `.agents/runbooks/m19-batched-full-regression.md` R48）对当前工作区**早已失效** ⇒ 按 change 预算，本 change 的收尾全量归 **Iteration 007 / T21**（M19 分批串行纪律），**本 Cycle 不跑全量**。
  - 已知限制（不在本 Cycle）：存储层其余无界读点（同形态约 17 处，各自带不同的「损坏页正确行为」）；释放路径的有界读（Iteration 002 已闭合）；ISS25 保持 `open`。
- Current-State Evidence（Plan 已逐行确认，行号为**当前工作区**；Act 不需回读即可执行）:
  - **无边界检查的槽数据读取器**：`src/storage/page_format/slotted_page.rs:125-129`（只读 `SlottedPageRef::get_slot_data`）与 `:195-199`（可写 `SlottedPage::get_slot_data`）：`let start = slot.offset as usize; let end = start + slot.length as usize; &self.data[start..end]`——`offset` / `length` 直接来自页内槽目录（槽目录项 6 字节 = `logical_id(2) | offset(2) | length(2)`，`Slot::SIZE = 6`，槽目录项 i 落在页尾 `PAGE_SIZE - (i+1)*Slot::SIZE`），**没有任何范围检查** ⇒ 槽偏移或长度越界 ⇒ 越界切片 panic（panic 发生在异步执行路径 ⇒ 进程 abort）。
  - **索引树访问器族的 8 处无界读点**（`src/storage/btree/node.rs`）：`LeafNode::get_key`（`:86`）、`LeafNode::get_row_id`（`:98`）、`LeafNodeRef::get_key`（`:555`）、`LeafNodeRef::get_row_id`（`:565`）、`InternalNode::get_key`（`:673`）、`InternalNode::get_child_page_id`（`:685`）、`InternalNodeRef::get_key`（`:1041`）、`InternalNodeRef::get_child_page_id`（`:1051`）——四类访问器（可变 `LeafNode` / `InternalNode` 与只读 `LeafNodeRef` / `InternalNodeRef`）各 2 处，形态一致：`let data = self.slotted.get_slot_data(&slot); if data.len() < MAX_KEY_LEN + RowId::SIZE { return None; } …`。
  - **由访问器派生的查找/定位辅助**（同文件，消费上述 8 处，因而继承其越界行为）：`LeafNode::find_key_position`（`:107`）、`LeafNodeRef::find_all_matches`（`:573`）、`LeafNodeRef::find_key_position`（`:588`）、`LeafNodeRef::find_key_position_binary`（`:606`）、`InternalNode::find_child_page_id`（`:703`）、`InternalNodeRef::get_all_separators`（`:1004`）、`InternalNodeRef::find_child_page_id`（`:1064`）、`InternalNodeRef::find_child_page_id_binary`（`:1097`）、`InternalNodeRef::equality_candidate_children`（`:1141`）。
  - **读 / 写热路径上的调用点**：
    - `src/storage/btree/index_manager.rs::search_from_page_async`（`:289-316`，异步下降：`LeafNodeRef::find_key_position_binary` + `get_row_id`，内部结点走 `find_child_page_id_binary`）——`SELECT` 点查与 DML 路径的索引下降；
    - `src/storage/btree/index_manager.rs::search_all_from_page_async`（`:320` 起，等值候选子树下降 `equality_candidate_children` + `get_row_id`）；
    - `src/storage/btree/index_manager.rs` 的**条目提取全扫**（`:520-560`：叶页 `get_key` / `get_row_id`、内部页 `get_child_page_id` + `leftmost_child`）；
    - `src/storage/btree/index_manager.rs:905-906`（条目导出路径对 `get_key` / `get_row_id` 直接 `.unwrap()` ⇒ 损坏槽除越界切片外还有**空值 panic** 面）；
    - `src/storage/btree/btree.rs`：`search_from_page`（`:148-172`，同步下降）、`search_from_page_async`（`:176-205`，异步下降）、`find_child_position`（`:43-73`，**写路径**的子结点选择，`get_key` / `get_child_page_id`）、`sibling_ids`（`:75-99`，写路径的兄弟结点选择）、`find_separator_slot`（`:740-745`）、`get_parent_separator_key`（`:927-930`）、`read_leaf_pair`（`:949-968`）、`read_internal_seps`（`:990-998`）、`scan_all`（`:1018-1042`）、`search_all_from_page`（`:1112-1155`）、`delete_all_from_page`（`:1182-1228`）、`delete_exact_from_page`（`:1252-1262`）。以上 BTree 侧函数**全部**已返回 `Result<…>` ⇒ 错误传播不需要改签名。
  - **同仓库已有的有界读（同一来源，修复形态的模板）**：`src/storage/integrity.rs:593-646` `check_tree_page(data) -> TreePageCheck { children, invalid_type, violations }`——对叶页与内部页做**槽边界校验**（内部页另校验条目长度 `MAX_KEY_LEN + 4`），越界槽与过短槽都记为 violation；其底层 `validate_slots`（`:538` 起）逐槽做范围检查，**不做越界切片**。该读法**已在生产路径**使用：`src/storage/btree/index_manager.rs:625-670` `collect_all_pages`（Iteration 002 的成果——释放路径用 `check_tree_page` 读树页，损坏页**计数 + 报出 + 跳过其子树**）与 `src/storage/data/table_manager.rs:1966` 的 `collect_tree_pages`（重建面）。
  - **两条路径的处置方向必须相反**（D13）：释放路径跳过损坏页是**既有契约**（物理释放尽力而为，`collect_all_pages` 的注释即写明损坏结点的子树「leaked by design, warned」）；读路径**不能**照搬——跳过损坏的内部结点 ⇒ 下降无法确定子结点 ⇒ 当作「没有这个子树」继续 ⇒ 点查返回「查不到该行」⇒ 静默错读；任选一个子结点继续 ⇒ 读到**别的行**。⇒ 读 / 写路径只有两种安全形态：**明确错误**或**进程 abort**；本 change 取前者。
  - **打开期对损坏页的容忍**：`src/database.rs:450-451` 把 `rebuild_free_page_set` 的错误吞掉并告警（`[free-page-rebuild] rebuild abandoned`）⇒ 损坏的索引页**不会**让打开失败 ⇒ 损坏夹具可以「写库 → 关闭 → 改坏页字节 → 重开」后直接执行 DML / 查询。
  - **损坏注入的既有先例（可直接复用）**：`tests/release_path_bounded_read_test.rs`——`slot_offset_field_offset(slot_index)`（页尾 `PAGE_SIZE - (index+1)*Slot::SIZE + 2`，即槽目录项的 offset 字段）、`page_file_offset(page, in_page)`（`HEADER_SIZE + page*PAGE_SIZE + in_page`）、`corrupt_slot_offset(path, page, slot_index)`（把 offset 字段写成 `0xFFFF` = 越界值）、`seed_index_table(path)`（建表 + 建二级索引 + 写 150 行 + checkpoint，返回 `(二级索引根页号, 数据页头页号, PK 树根页号)`）。同仓另有 `tests/check_command_test.rs` 的同形态 `poke`。
  - **索引的身份标识**：`IndexManager`（`src/storage/btree/index_manager.rs:143-166`）**没有**索引名字段（`root_page_id: Arc<AtomicU64>` / `sync_loader` / `async_loader` / `row_to_key` / `catalog_ctx` / `write_gate` / `write_pause`）；`IndexManager::new` 与 `IndexManager::from_root` 共 17 处调用点（含 `src/wal/recovery.rs`、`src/executor/create_index.rs`、`src/storage/data/table_manager.rs`、`tests/index_manager_test.rs`、`tests/structural_property_test.rs`）⇒ **把索引名注入 `IndexManager` 属范围外的接口改造**；索引树的身份可由**其根页号**唯一确定（每个索引树有独立的根页；根分裂时 `root_page_id` 随新根更新）。
  - **既有错误类型**：`src/storage/error.rs:8` `pub enum StorageError`（含 `InvalidPageType { expected, actual }`、`SlotNotFound(RowId)`、`Internal(String)` 等；枚举在 `:130` 结束）⇒ 损坏索引页需要**一个专属变体**（可被匹配、文案自带页号与索引身份），**不复用** `Internal(String)`（后者无结构、无法点名页与索引）。
  - **既有测试入口**：`tests/index_manager_test.rs`（索引管理器基础面）、`tests/index_page_ownership_test.rs` 与 `tests/index_page_ownership_stack_overflow_test.rs`（索引页归属与下降终止）、`tests/secondary_index_test.rs`（二级索引面）、`tests/release_path_bounded_read_test.rs`（损坏注入与释放面）。**本 Cycle 新增** `tests/index_read_bounded_read_test.rs`。
- Code and Critical Path:
  - 入口：`src/executor/` 的 DML / 查询执行器 → `TableMeta` 的 `index_manager`（PK）、`unique_indexes` / `secondary_indexes`（唯一 / 二级）→ `IndexManager` 的异步读方法 → `AsyncPageLoader` → `BufferPool` 页帧 → `btree/node.rs` 的访问器族 → `SlottedPageRef::get_slot_data`（越界切片）。
  - 调用链（读路径）：`SELECT` 点查 / `INSERT` / `DELETE` / `UPDATE` → `IndexManager::search` / `search_async` → `search_from_page_async`（`index_manager.rs:289`）→ `LeafNodeRef::find_key_position_binary` → `get_row_id` ⇒ `get_slot_data` ⇒ **越界切片 panic**。
  - 调用链（写路径）：`IndexManager::insert` / `delete` / `find_key_by_row_id` → `spawn_blocking` 内的 `BTree`（同步下降 `btree.rs:148` / `find_child_position` `:43`）→ `InternalNode` / `LeafNode`（可变形态）→ `get_key` / `get_child_page_id` ⇒ **越界切片 panic**。
  - 数据流：页字节 → `SlottedPageRef::get_slot(index)`（有界：槽目录在页内固定位置）→ `get_slot_data(&slot)`（**无界**）→ 槽负载（叶：分隔键 / 键 + RowId；内部：分隔键 + 子指针）→ 下降决策（选哪个子结点 / 命中哪个条目）。
  - 状态变化：本 Iteration **不改变**任何持久状态（只读面与错误面）；损坏页上的写入**整体失败**（不留半写状态：越界切片发生在读取阶段，写入尚未发生）。
  - 外部影响：当前形态下损坏索引页上的 `SELECT` / `INSERT` / `DELETE` / `UPDATE` **进程 abort**（越界切片 panic）；修复后语句返回**明确的错误**（点名损坏的页与索引），进程不 abort。

**Implementation Guidance**

- **两个互补的机制，缺一不可**（这是本 Cycle 的实现骨架，Act 不需另选方案）：
  1. **访问器族改用有界读（保证「不 panic」）**：在 `src/storage/btree/node.rs` 内为 8 处读点提供**有边界检查**的槽数据读取（越界 ⇒ 不切片、不 panic）。可复用 `src/storage/integrity.rs` 的 `validate_slots` 口径，或在 `node.rs` 内加一个同口径的私有有界读辅助（**不得**改 `SlottedPageRef::get_slot_data` 的签名——波及全部调用点，属范围外候选）。
  2. **页级校验 + 显式错误（保证「不静默错读」）**：在**每个以索引树页字节为输入、并据此决定下降 / 取条目 / 取子指针**的访问点上，先经 `src/storage/integrity.rs` 的 `check_tree_page` 做页级校验；`violations` 非空 ⇒ **返回显式错误**（不进入该页的任何子结点、不返回「查不到」）。两条机制互补：机制 1 保证**任何**路径都不会 panic；机制 2 保证损坏页**不会**被当作「空子树 / 无此条目」静默跳过。
- **错误语义（本 Cycle 定死，Act 不需再定）**：新增 `StorageError` 专属变体，携带**损坏页号**与**该索引树的根页号**（索引身份），文案同时点名两者（形态：`corrupted index page {page_id} (index root page {index_root})`）。页号由访问点给出（访问点知道它加载的页）；索引根页号由 `IndexManager::root_page_id` / `BTree::root_page_id` 给出。**不**把索引名注入 `IndexManager`（17 处构造点，范围外）。
- **错误不得被降级**：损坏页上的语句 SHALL 整体失败并把该错误返回给调用方；SHALL NOT 把它改写成「查不到该行」、SHALL NOT 退化成告警后继续下降、SHALL NOT 静默跳过该子树。
- **写路径的处置**：损坏页上的 `INSERT` / `DELETE` / `UPDATE` 在**读取阶段**失败（越界切片发生在下降 / 条目提取阶段，写入尚未发生）⇒ 不产生半写状态。
- **未损坏路径零变化**：页级校验只对**已损坏**的页生效（`violations` 非空）；健康页的 `violations` 为空 ⇒ 下降与取条目的行为与修复前**逐条相同**（同样的键序、同样的子结点选择、同样的命中数）。
- **夹具形态**：沿用 Iteration 002 的注入手法（真实文件库 + 真实 `Database::open` → 关闭 → 按文件偏移把**索引树页**的槽目录项 offset 字段改写为 `0xFFFF` → 重开 → 执行真实 `SELECT` / `INSERT` / `DELETE` / `UPDATE`）。损坏点选在**被语句实际扫描的页**上（与 Iteration 002 的读集澄清同法：损坏点必须落在该语句真正读取的索引树页，否则该语句不会命中损坏）。
- **规模约束**：ISS23 ⇒ 本 change 内不使用 ≥ 300 行的表规模；本缺陷与规模无关（单页 + 数行即可命中越界切片）⇒ 用例表规模 **≪ 300**。

**Task Definitions**

### T22: 损坏的索引页上普通 `SELECT` / `INSERT` / `DELETE` / `UPDATE` 不 panic、不 abort、不静默错读——先行 RED 用例

- Requirement/Scenario: `specs/index-read-bounded-read/spec.md`（本 Cycle 新增增量域）IRBR R1 的四个损坏页 Scenario（`SELECT` / `INSERT` / `DELETE` / `UPDATE`）+「未损坏路径的读 / 写行为零变化」Scenario；`design.md` D13。
- Depends on: None
- Targets: `tests/index_read_bounded_read_test.rs`（新增用例）；作为被测对象：`src/storage/btree/node.rs` 的索引树访问器族、`src/storage/btree/btree.rs` 与 `src/storage/btree/index_manager.rs` 的下降与条目提取路径（本任务**不**修改它们）。
- Current behavior: 页内容损坏（索引树页的槽目录项 offset 越界）时，普通 `SELECT` / `INSERT` / `DELETE` / `UPDATE` 在索引下降 / 条目提取阶段无边界地切页内字节 ⇒ 越界切片 panic ⇒ 进程 abort（同一根因层已在释放路径由 Iteration 002 闭合；读 / 写路径未闭合）。
- Required behavior: 四类语句在损坏的索引页上**不 panic、不 abort**，且**不得**被静默当作「查不到该行」——语句 SHALL 返回**明确的错误**（点名损坏的页与该索引），进程保持存活；未损坏库上的读 / 写行为与修复前一致（点查命中数、行集合、写入结果不变）。
- Preserve: 未损坏路径的读 / 写行为（键序、命中数、子结点选择）；既有域 `index-write-side-atomicity`（同一索引的结构变更串行）与 `sql-secondary-indexes` / `sql-constraint-enforcement` 的索引语义；`free_page` / `complete_free_release` 的集合语义；磁盘格式（MS36 的 G1/G2/G3 兼容面）；既有释放路径的尽力而为语义（Iteration 002 成果）。
- Forbidden: 不得用「跳过读」之类的替身代替真实损坏（夹具必须真实改坏页字节）；不得改动 `SlottedPageRef::get_slot_data` 的签名；不得把损坏页降级为告警 + 跳过（读路径 SHALL NOT 跳过子树）；不得引入身份证明机制（hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明）、判定层或 timer 阈值行为判据；不得在测试框架外建判定脚本或封装命令；本任务**不改产品代码**。
- First test: 在 `tests/index_read_bounded_read_test.rs` 中新增**五条**用例：四条损坏页用例（`SELECT` / `INSERT` / `DELETE` / `UPDATE` 各一）+ 一条未损坏对照用例；损坏夹具 = 真实文件库 + 真实 `Database::open` → 建表 + 建二级索引 + 写少量行 → 关闭 → 按文件偏移把**索引树页**（PK 树根页或二级索引根页，取决于该语句实际扫描的页）的槽目录项 offset 字段改写为 `0xFFFF` → 重开 → 执行真实语句。变更前运行 `cargo test --test index_read_bounded_read_test -- --test-threads=1`，预期 **RED**：损坏页用例以 panic 失败（进程 abort 由测试进程捕获为失败），未损坏对照用例通过。
- GREEN condition: 修复后同一条命令 5 个用例全部 passed / 0 failed；四条损坏页用例的判定**不是**「不 panic」而已——须断言语句返回**明确的错误**且错误文案点名损坏的页与该索引（错误变体的字段或文案均可）。
- Verification: `cargo test --test index_read_bounded_read_test -- --test-threads=1` ⇒ 5 passed / 0 failed；失败即表示损坏索引页的读 / 写路径仍会 panic，或仍被静默跳过（点查返回「查不到」而全表扫描命中）。
- Stop when: 损坏夹具无法在真实文件库上稳定命中越界切片，或某条语句实际不扫描被损坏的页 ⇒ **更换该用例的损坏点**（落在该语句真正读取的页上），而不是放宽断言；若某条语句在该损坏形态下**本来就不读索引树页**（因而不可能命中损坏）⇒ 记入 Act Response 并把该语句的损坏用例改为「经由真实入口验证其既有行为未变」，不得为此改判据；修复需要改 `SlottedPageRef::get_slot_data` 的签名或波及全部调用点 ⇒ 回到 Plan。

### T23: 索引树访问器族改用有界读 + 损坏索引页的显式错误语义

- Requirement/Scenario: 同 T22（实现 T22 的 RED）。
- Depends on: T22
- Targets: `src/storage/btree/node.rs`（8 处 `get_slot_data` 无界读点：`:86` / `:98` / `:555` / `:565` / `:673` / `:685` / `:1041` / `:1051`，及其派生辅助 `:107` / `:573` / `:588` / `:606` / `:703` / `:1004` / `:1064` / `:1097` / `:1141`）、`src/storage/btree/btree.rs`（下降与条目提取：`find_child_position` `:43`、`sibling_ids` `:75`、`search_from_page` `:148`、`search_from_page_async` `:176`、`find_separator_slot` `:740`、`get_parent_separator_key` `:927`、`read_leaf_pair` `:949`、`read_internal_seps` `:990`、`scan_all` `:1018`、`search_all_from_page` `:1112`、`delete_all_from_page` `:1182`、`delete_exact_from_page` `:1252`）、`src/storage/btree/index_manager.rs`（`search_from_page_async` `:289`、`search_all_from_page_async` `:320`、条目提取全扫 `:520-560`、条目导出 `:905-906`）、`src/storage/error.rs`（新增 `StorageError` 专属变体）。
- Current behavior: 索引树访问器族用 `SlottedPageRef::get_slot_data` 无边界地切页内字节 ⇒ 损坏页 ⇒ 越界切片 panic ⇒ 进程 abort；若只把切片改成「有界但返回 `None`」，则损坏页会被当作「无此条目 / 空子树」⇒ 点查返回「查不到该行」⇒ 静默错读（比 abort 更隐蔽）。
- Required behavior: ①**8 处读点全部改用有边界检查的槽数据读取**（越界 ⇒ 不切片、不 panic）；②**每个以索引树页字节为输入、并据此决定下降 / 取条目 / 取子指针的访问点**在读槽之前做页级校验（`src/storage/integrity.rs` 的 `check_tree_page` 同一读法），`violations` 非空 ⇒ **返回显式错误**（`StorageError` 的新专属变体，携带损坏页号与该索引树的根页号，文案同时点名两者）⇒ 该页**不被跳过**、**不被当作空子树**、语句整体失败；③未损坏路径的读 / 写行为与修复前**逐条相同**。
- Preserve: 未损坏路径的键序 / 命中数 / 子结点选择；`IndexManager` 与 `BTree` 的对外签名（`search` / `search_async` / `insert` / `delete` / `scan_all` 等）与 `Result` 语义；`IndexManager::new` / `from_root` 的签名与 17 处调用点；`row_to_key` / `catalog_ctx` / `write_gate` / `write_pause` 的既有语义；`free_page` 集合语义；磁盘格式；释放路径的尽力而为语义（Iteration 002 成果不改）。
- Forbidden: 不得改 `SlottedPageRef::get_slot_data` 的签名或把它 `Option` 化（波及全部调用点，属范围外候选）；不得把损坏页降级为「告警 + 跳过」或「当作查不到」（读路径 SHALL NOT 跳过子树）；不得改 `src/storage/integrity.rs` 的判据本身（`check_tree_page` / `validate_slots` 的 violations 定义）与既有释放路径对它的用法；不得把索引名注入 `IndexManager`（17 处构造点，范围外）；不得改磁盘格式；不得改 `free_page` 集合语义；不得引入身份证明机制、判定层或 timer 阈值行为判据。
- First test: 沿用 T22 的用例（先 RED 后 GREEN）。若某条损坏页用例在某条语句上未能命中（该语句不扫描被损坏的页），须**更换该用例的损坏点**，不得放宽断言。
- GREEN condition: `cargo test --test index_read_bounded_read_test -- --test-threads=1` 全部通过（5/0），且既有索引相关用例保持通过；损坏页上的四条语句返回**明确错误**且进程不 abort。
- Verification: `cargo test --test index_read_bounded_read_test -- --test-threads=1`（5/0）；`cargo test --test index_manager_test -- --test-threads=1`；`cargo test --test secondary_index_test -- --test-threads=1`；`cargo test --test index_page_ownership_test -- --test-threads=1`；`cargo test --test index_page_ownership_stack_overflow_test -- --test-threads=1`；`cargo test --test release_path_bounded_read_test -- --test-threads=1`；`cargo test --lib -- --test-threads=1`（本 Cycle 改 `src/storage/btree/*` ⇒ Iteration 005 的 420 条 lib 结论失效，必须重跑）。任一命令 failed > 0 ⇒ 本 Cycle 未完成。
- Stop when: 修复需要改 `SlottedPageRef::get_slot_data` 的签名、改 `integrity` 模块的判据、或把读路径改成「遇损坏页整体跳过 / 当作查不到」 ⇒ 回到 Plan；或未损坏路径的既有索引用例出现行为差异 ⇒ 回到 Plan；或需要改磁盘格式 ⇒ 回到 Plan。

**Invariants**

- **读路径 SHALL NOT 跳过子树**：损坏的索引页 ⇒ 明确的错误 ⇒ 语句整体失败；**不**是「查不到该行」、**不**是「任选一个子结点继续」（那会读到别的行）。
- **任何路径 SHALL NOT 因损坏的索引页 panic / abort**：越界切片在**所有**索引树访问点被消除（访问器族的有界读是最后一道保障）。
- 未损坏路径的读 / 写行为**零变化**：键序、命中数、子结点选择、写入结果与修复前逐条相同。
- 释放路径的处置**不变**（尽力而为：告警 + 跳过 + 返回已收集的部分）——读 / 写路径与释放路径的处置方向**相反**，不得互相照搬。
- `free_page` / `complete_free_release` / `promote_free_releases` 的集合语义不变；磁盘格式不变（MS36 的 G1/G2/G3 兼容面不变）。
- 既有域 `index-write-side-atomicity`（同一索引的结构变更串行）与索引的既有语义不变。
- 不得引入身份证明机制（hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明）、判定层或 timer 阈值行为判据；判定一律在 `cargo test` 内。

**Non-goals**

- `SlottedPageRef::get_slot_data` 的签名改造（含数据页 / 目录页 / 恢复面 / 缓冲池的同形态调用点）。
- 存储层其余无界读点（数据页 / 目录页 / 恢复面 / 缓冲池，同形态约 17 处）——各自的「损坏页正确行为」需单独判定，另立 change 承载。
- 释放路径的读面与处置语义（Iteration 002 已闭合）。
- 改写窗口的读一致性（Iteration 003）、释放可分配判定的耐久性前提（Iteration 004）、DML 墓碑的放弃窗口（Iteration 005）。
- 完整性扫描的诊断分级与损坏页的**修复**（本 Iteration 只拒绝，不修复）。
- 索引名注入 `IndexManager`（17 处构造点）。
- 磁盘格式；`free_page` 集合语义。
- ISS23 的规模面（本 change 内不使用 ≥ 300 行的表规模）。

**Acceptance**

1. **损坏的索引页上的 `SELECT` 不 panic、不 abort、不静默错读**：真实文件库 + 真实 `Database::open` + 真实 `SELECT`（点查与全表扫描各一）；损坏点 = 该语句实际扫描的**索引树页**的槽目录项 offset 字段越界；语句 SHALL NOT panic、SHALL NOT 导致进程 abort，且 SHALL 返回**明确的错误**（点名损坏的页与该索引）——SHALL NOT 返回「查不到该行」而全表扫描命中。映射：IRBR R1 Scenario「损坏的索引页上的查询不 panic、不静默错读」、T22、T23。
2. **损坏的索引页上的 `INSERT` 不 panic、不 abort**：同上形态（损坏点落在该语句实际扫描的索引树页）；语句返回明确的错误，进程不 abort；**不产生半写状态**（读取阶段失败）。映射：IRBR R1 Scenario「损坏的索引页上的插入不 panic」、T22、T23。
3. **损坏的索引页上的 `DELETE` 不 panic、不 abort**：同上形态。映射：IRBR R1 Scenario「损坏的索引页上的删除不 panic」、T22、T23。
4. **损坏的索引页上的 `UPDATE` 不 panic、不 abort**：同上形态。映射：IRBR R1 Scenario「损坏的索引页上的更新不 panic」、T22、T23。
5. **未损坏路径的读 / 写行为零变化**：内容完好的库执行 `SELECT` / `INSERT` / `DELETE` / `UPDATE` ⇒ 点查命中数、行集合、写入结果与既有行为一致；既有索引用例（`index_manager_test` / `secondary_index_test` / `index_page_ownership_test` / `index_page_ownership_stack_overflow_test` / `release_path_bounded_read_test`）保持通过。映射：IRBR R1 Scenario「未损坏路径的读 / 写行为零变化」、T23。
6. **判别力与零判定层**：四条损坏页用例的 RED 必须是**越界切片 panic**（而非任何其它失败形态），未损坏对照用例在 RED 阶段即通过；损坏夹具必须真实改坏页字节（不得用替身）；判定全部落在 `cargo test` 内，无身份证明机制、无框架外判定层、无 timer 阈值行为判据。映射：T22 的 First test、T23。

**Verification**

- 行为类 scenario（1–5）走**真实入口**：真实文件库 + 真实 `Database::open` + 真实 `SELECT` / `INSERT` / `DELETE` / `UPDATE`；损坏通过**按文件偏移改写页字节**注入（真实损坏，不是替身）。
- 判定命令：`cargo test --test index_read_bounded_read_test -- --test-threads=1` ⇒ 5 passed / 0 failed（scenario 1–6）。
- 回归命令：`cargo test --test index_manager_test -- --test-threads=1`、`cargo test --test secondary_index_test -- --test-threads=1`、`cargo test --test index_page_ownership_test -- --test-threads=1`、`cargo test --test index_page_ownership_stack_overflow_test -- --test-threads=1`、`cargo test --test release_path_bounded_read_test -- --test-threads=1`、`cargo test --lib -- --test-threads=1` ⇒ 各自 0 failed（scenario 5、6）。
- 全部命令以 `--test-threads=1` 串行执行；判定只看 cargo 自己的退出码与原生输出（允许用一步通用管道读计数，管道只做计数不做判定）。
- **本 Iteration 不跑全量回归**（本 change 一次预算，归 Iteration 007 / T21，M19 分批串行纪律）。
- 无人工验证项；无身份证明机制；无框架外判定层。

**Gate 2 Readiness**

- PASS——需求与场景：`specs/index-read-bounded-read/spec.md`（本 Cycle 新增增量域）的 R1 与四个损坏页 Scenario + 「未损坏路径零变化」Scenario 已逐条映射到 T22 / T23 与 Acceptance 1–5（BLOCKED: 无）。
- PASS——调查：8 处无界读点、派生辅助、读 / 写热路径的调用点、同源有界读（`check_tree_page`）、释放路径与读路径的处置差异、索引身份的可用标识、错误类型的既有形态、打开期容忍与损坏注入先例，均已在 Investigation Facts 中落到**当前工作区的行号**（BLOCKED: 无）。
- PASS——设计闭合：错误语义（专属变体 + 点名损坏页与索引树根页号 + 不跳过 + 不降级 + 写路径读取阶段失败）、两机制互补的实现骨架、未损坏路径零变化的判据，均已定死，Act 不需再决定接口或错误语义（BLOCKED: 无）。
- PASS——任务分轮：T22（RED）→ T23（有界读 + 显式错误），两轮可独立验证与诊断（BLOCKED: 无）。
- PASS——追踪：Cycle RTM（见下）与 change `tasks.md` 的 RTM 行一致（T22 / T23 ↔ 新增域 `index-read-bounded-read`）（BLOCKED: 无）。
- PASS——当前轮验证：T22 的 RED 命令与其预期失败形态已写明；T23 的 GREEN 判据与回归清单已写明；Persisted Evidence = `none`（全部结论可低成本重跑）（BLOCKED: 无）。
- 无 BLOCKED / SKIPPED 项。⇒ **Gate 2 通过**，Plan Context 状态置 `ready`。

**Cycle RTM**

| Requirement / Scenario | Task | First Test（计划名） | Status |
|---|---|---|---|
| IRBR R1「损坏的索引页在读 / 写路径上给出明确的错误」（新增域，本 Cycle 产出增量） | T22, T23 | `tests/index_read_bounded_read_test.rs::damaged_index_page_makes_select_fail_with_a_named_error`（计划名；Act 可改名并记录实际名） | Covered |
| IRBR R1 Scenario「损坏的索引页上的插入 / 删除 / 更新不 panic、不 abort」 | T22, T23 | 同文件的三条对应用例（计划名 `damaged_index_page_makes_insert_…_fail_with_a_named_error` / `…_delete_…` / `…_update_…`） | Covered |
| IRBR R1 Scenario「未损坏路径的读 / 写行为零变化」 | T23 | 同文件的未损坏对照用例（计划名 `undamaged_index_reads_and_writes_are_unchanged`） | Covered |

**Persisted Evidence**

- Mode: none

全部结论可低成本重跑（一个新增测试文件 + 5 个既有测试文件 + `cargo test --lib`），命令与结果写入 Act Response 即可承载；不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ 不创建 `evidence/` 目录。

**Risks and Notes**

- **页级校验的覆盖面**：页级校验必须落在**每个**以索引树页字节为输入、并据此决定下降 / 取条目 / 取子指针的访问点（枚举见 Investigation Facts）。漏掉一处 ⇒ 该处仍可能把损坏页当作「无此条目」⇒ 静默错读。⇒ Act 须在 Act Response 中**逐点列出**已加校验的访问点，并说明未加校验的点为何不受影响（例如只读页头、不取槽负载的路径）。**访问器族的有界读是兜底**：即使漏检也不会 panic。
- **`index_manager.rs:905-906` 的 `.unwrap()`**：该处对 `get_key` / `get_row_id` 直接 `.unwrap()` ⇒ 损坏槽除越界切片外还有**空值 panic**面；T23 的有界读 + 页级校验必须同时覆盖它（否则该路径仍可 panic）。
- **损坏点必须落在该语句实际扫描的页上**：与 Iteration 002 的读集澄清同法（`DROP INDEX` 只扫描索引树页）。若某条语句在该损坏形态下本来就不读索引树页 ⇒ 按 T22 的 Stop when 处理（记入 Act Response 并改该用例的判据为「既有行为未变」），**不得**改判据迁就实现。
- **`check_tree_page` 的既有语义边界**：它对**叶页**只做槽边界校验、对**内部页**另校验条目长度（`MAX_KEY_LEN + 4`）⇒ 健康页的 `violations` 为空 ⇒ 不影响未损坏路径；损坏页的 `invalid_type`（页类型不是叶 / 内部）另有 `StorageError::InvalidPageType` 既有形态，**不**在本 Cycle 改变。
- **索引身份的命名方式**：本 Cycle 用**索引树的根页号**点名索引（`IndexManager::root_page_id` / `BTree::root_page_id`）⇒ 不改 `IndexManager::new` / `from_root` 的签名。若 Act 认为该形态不足以「点名索引」，**不得**自行扩到 `IndexManager` 的接口改造（17 处构造点，范围外）⇒ 记入 Act Response 由 Plan Review 裁定。
- **规模口径**：用例表规模 **≪ 300**（ISS23 约束）；不得为规避夹具问题放宽到更大规模。
- **不改磁盘格式**（保 MS36 的 G1/G2/G3 兼容面）；本 Cycle 不改 `free_page` 集合语义。

## Plan Review

- Review Result: accepted

**Findings**

Plan Review 2026-10-25（`openspec-plan`，Review 模式）。独立阅读了完整 diff（`git diff HEAD -- src/storage/btree/node.rs`、`src/storage/btree/btree.rs`、`src/storage/btree/index_manager.rs`、`src/storage/error.rs` + 新增 `tests/index_read_bounded_read_test.rs` 全文）与实际代码、change 的 `tasks.md` / `design.md` D13 / `proposal.md` / `specs/index-read-bounded-read/spec.md` 全文；**未**以 Act Self-Review 代替独立检查；**未**重复运行 Act 已通过且覆盖范围未变化的验证（来源：Act Response 的 GREEN 证据表；本 Review 未改动任何产品代码，覆盖范围自 Act 运行后未变）。

- **F1（通过·覆盖完整性：索引树页访问点与条目提取点逐点枚举）** 独立枚举 `src/storage/btree/` 下的**全部**索引树页访问点并逐点核对「有界槽读 + 页级校验 ⇒ 显式错误」两条同时成立：
  - **条目提取面**：`src/storage/btree/node.rs` 的 8 个访问器（`LeafNode::get_key` / `::get_row_id`、`LeafNodeRef::get_key` / `::get_row_id`、`InternalNode::get_key` / `::get_child_page_id`、`InternalNodeRef::get_key` / `::get_child_page_id`）全部改用 `bounded_slot_data_mut` / `bounded_slot_data_ref`；**独立 grep 确认** `node.rs` 内剩余的 `get_slot_data` 调用只有 `bounded_slot_data_ref`（`node.rs:39`）与 `bounded_slot_data_mut`（`node.rs:51`）两处 ⇒ 访问器族内**无**遗留的无界切片点；派生辅助（`find_key_position` / `find_all_matches` / `find_key_position_binary` / `find_child_page_id` / `find_child_page_id_binary` / `equality_candidate_children`）全部经由这 8 个访问器 ⇒ 继承有界读。
  - **页级校验面**：`btree.rs` 19 处 + `index_manager.rs` 3 处 = **22 处**，逐处核对其**据页字节决定什么**并与 Act 的清单一致（`btree.rs` 的 `sibling_ids:85` / `search_from_page:161` / `search_from_page_async:199` / `insert_into_page_step:296` / `delete_from_page:480` / `leaf_key_count:583` / `internal_key_count:590` / `delete_from_internal:691` / `find_separator_slot:765` / `get_parent_separator_key:953` / `update_parent_separator:969` / `read_leaf_pair:992`+`:993` / `read_internal_seps:1027` / `scan_all:1060` / `update_in_page:1120` / `search_all_from_page:1155` / `delete_all_from_page:1227` / `delete_exact_from_page:1299`；`index_manager.rs` 的 `search_from_page_async:301` / `search_all_from_page_async:338` / `scan_all_async_from_root:544`）。
  - **未加校验但不受影响的点（独立复核结论）**：`find_child_position`（`btree.rs:46`）的**唯一**调用点（`:692`）紧邻 `:691` 的校验点 ⇒ 其所读页已校验；`shrink_root`（`:819`）/ `handle_child_merge`（`:706`）/ `rebuild_leaf` / `rebuild_internal` 只读**页头字段**（`key_count` / `leftmost_child` / `next_leaf_page_id`，固定 16 字节头内，读取本身有界），且其页在进入前已被上表中的点校验或由 `read_leaf_pair` / `read_internal_seps` 的已校验条目整页重写；`merge_leaves`（`:634`）/ `merge_internal_nodes`（`:905`）所改写的页分别由 `read_leaf_pair`（`:992`+`:993`）/ `read_internal_seps`（`:1027`）/ `get_parent_separator_key`（`:953`）/ `sibling_ids`（`:85`）在同一写单元内先行校验。⇒ **无遗漏点**。
  - **释放面有意不改**：`IndexManager::collect_all_pages`（`index_manager.rs:625`）/ `collect_all_pages_tolerant`（`:700`）的「计数 + 报出 + 跳过其子树」与 `table_manager.rs::collect_tree_pages`（`:1966`，损坏页 ⇒ **报错**）保持原样 ⇒ 读 / 写路径与释放路径的处置方向相反，D13 成立。
- **F2（通过·「读路径绝不跳过」不变量逐分支核对）** 22 处校验点全部以 `?` 直接返回 `Err`（无告警降级、无「当作空子树」）；`sibling_ids` 改为返回 `Result` 后，**两个**调用点（`btree.rs:532` 的 `handle_leaf_underflow`、`:784` 的 `handle_internal_underflow`）均以 `?` 传播，**无**把 `Err` 静默转成「无兄弟节点」；读 / 写路径中唯一的 `Err(...) => continue` 是 `delete_exact_from_page`（`:1334-1339`）的 `Err(StorageError::KeyNotFound) => continue`——那是「该子树确实没有此条目」的正常信号，`CorruptedIndexPage` **不是** `KeyNotFound` ⇒ 被逐层传播。**裁定：不变量成立。**
- **F3（通过·健康页不误报，独立推导而非采信自述）** 独立验证页级判据在合法页上恒为空：
  - `SlottedPage::add_slot_inner`（`src/storage/page_format/slotted_page.rs:231-263`）的容量检查用 `free_space_end = PAGE_SIZE - slot_count*Slot::SIZE` 并要求 `free_space_start + Slot::SIZE + data_len <= free_space_end`；加入后 `slot_count+1` ⇒ 新的 `slot_area_start = free_space_end - Slot::SIZE` ⇒ **`fso <= slot_area_start` 恒成立**（新增槽后仍成立）。
  - `SlottedPage::delete_slot`（`:277`）**不动** `free_space_offset`，只减 `slot_count` ⇒ `slot_area_start` **变大** ⇒ **槽被回收（free_space_offset 推进）后该不等式仍成立** ⇒ 页级守卫不会把合法页判为损坏。
  - `shift_slots_right`（`node.rs:239`）/ `rebuild_leaf` / `rebuild_internal` 整页重写 ⇒ 不等式由 `add_slot` 重新建立。
  - **半满合法叶页**（N=46 条、条目 38 字节）：`fso = 16+38*46 = 1764 <= slot_area_start = 4096-6*46 = 3820` ✓；**满叶页**（N=92 = `btree.rs` 注释给出的叶页上限 `16+44n ≤ 4096`）：`fso = 16+38*92 = 3512 <= slot_area_start = 4096-6*92 = 3544` ✓ ⇒ 均不误报。
  - `node.rs::slot_payload_in_bounds` 的逐槽判据（`start >= SlottedPageHeader::SIZE && end <= slot_area_start`）与 `integrity::validate_slots`（`src/storage/integrity.rs:538-575`）**逐条同口径** ⇒ 两套机制不会互相打架；`validate_slots` 的另两条判据（槽数超页容量、`fso` 越界）由 `ensure_index_page_intact` 统一执行。
- **F4（通过·RED 可信度与工作区现状）** 独立核对 Act 的「临时禁用两套机制」是否等价于修复前行为：修复前访问器**逐字**是 `get_slot(index)?` + **未检查**的 `get_slot_data(&slot)`（见 `git diff HEAD -- src/storage/btree/node.rs` 中 8 个访问器的 `-` 侧）⇒ 把 `bounded_slot_data_ref` / `bounded_slot_data_mut` 还原为未检查的 `get_slot_data`、并把 `ensure_index_page_intact` 还原为空实现，**逐条**等价于修复前（唯一新增的 `index < MAX_PAGE_SLOTS` 前置判据对夹具不可达——夹具的损坏页 `slot_count` 是正常值）。实测 panic 位点 `src/storage/page_format/slotted_page.rs:128` = `SlottedPageRef::get_slot_data` 里的 `&self.data[start..end]` **无界切片**，与 `offset = 0xFFFF` ⇒ `range start index 65535 out of range for slice of length 4096` **完全对应** ⇒ RED 的失败形态确为越界切片 panic（Acceptance 6 的形态要求满足），且伴随 `PoisonError` ⇒ `signal: 6, SIGABRT`。**工作区现状核对**：`node.rs` 的两个有界辅助处于**启用**状态、22 处闸门在位 ⇒ 当前工作区确为 GREEN 版本；Plan Context 的 `Plan Context` 状态仍为 `ready`、Act Response 状态 `reported`，与交接描述一致。
- **F5（通过·`CorruptedIndexPage` 的信息充分性与根页号可得性）** 变体携带 `page_id` + `index_root_page_id` + `detail`，文案 `corrupted index page {page_id} (index root page {index_root_page_id}): {detail}` ⇒ 同时点名**损坏页**与**索引树**。22 处校验点的索引根页号**均可正确取得**：`btree.rs` 取 `self.root_page_id.0`（`BTree` 实例即一棵树，其 `root_page_id` 在根分裂时随新根更新 ⇒ 始终是该树的当前根）、`index_manager.rs` 取 `self.root_page_id.load(Ordering::Acquire)`。**损坏页即根页时**两个数字相同（如 `corrupted index page 6 (index root page 6)`）——该页号本身即树根 ⇒ 仍能定位到**哪棵树**（每个索引树有独立根页）；`SELECT` 用例的损坏页是**非根**叶页 ⇒ 实测 `corrupted index page 3 (index root page 6)`，两个数字**不同** ⇒ 证明两者确实分别携带而非同一字段的复写。
- **F6（非阻塞 Minor·叶页条目长度不在页级判据内 ⇒ 短条目被静默跳过）** `integrity::check_tree_page` 的**内部页**分支另校验条目长度（`end - start >= MAX_KEY_LEN + 4`），**叶页**分支**只**校验槽边界 ⇒ 一个**落在页内但短于条目格式**（38 字节 = Key 32 + RowId 6）的叶槽**不被判为损坏** ⇒ 访问器返回 `None`（`node.rs` 的 `get_key` / `get_row_id` 判 `data.len() < MAX_KEY_LEN + RowId::SIZE`）⇒ 该条目被**静默跳过**（`read_leaf_pair` 的 `filter_map`、`scan_all_async_from_root` 的 `if let (Some, Some)`、`find_key_position_binary` 的 `else { hi = mid }` ⇒ 表现为「查不到该行」）。**判定**：①**非回归**（修复前同一条短切片同样使 `get_key` 返回 `None` ⇒ 行为一致）；②**明确在 Plan 范围外**（T23 Forbidden：「不得改 `src/storage/integrity.rs` 的判据本身」；Plan Context Risks 亦写明「它对**叶页**只做槽边界校验」）；③**不阻塞**本 Iteration 的 Acceptance（Acceptance 1–6 的损坏形态明确为「槽目录项 offset 字段越界」）。⇒ 登记为**范围外实质缺陷的 Issue 候选**（承载方式见下）；并**建议**在 T20 的「规格增量与验收边界对齐」中把 IRBR Requirement 的「损坏的索引页」收敛为「**未通过页级判据**的索引树页」（行为级措辞，不引入实现术语），使并入 `openspec/specs/` 的规范只陈述**已验收**的行为。
- **F7（非阻塞 Minor·页级校验的分配开销）** 沿用 Act 的诚实记录：每次索引页访问都经 `check_tree_page`，其 `validate_slots` 为每页分配一个 `Vec`（容量 = 槽数）⇒ 索引下降热路径上多出每页一次分配。Acceptance 无性能判据、未做测量 ⇒ **不阻塞**；记为风险，供后续专项。
- **F8（非阻塞 Minor·`InvalidPageType { expected: LEAF_NODE }` 的措辞）** 沿用 Act 的记录：页类型既非叶也非内部时沿用释放路径的同款既有形态（`expected: LEAF_NODE`），在本 Cycle 的调用点上「期望类型」可能原本是内部结点 ⇒ 该分支**不点名索引根页**（该变体未携带索引身份）。变体本身按 Plan Forbidden 未改；与释放路径对称 ⇒ **不阻塞**。
- **F9（非阻塞 Minor·被点名的根页在「根分裂尚未发布」窗口内可能不是目录登记的根）** `index_root_page_id` 取自 `BTree` 实例的**内存当前根**；MS34 的写侧暂停窗口内（结构变更已生效、根页 id 未发布）它可能是**新根**而目录仍登记旧根 ⇒ 错误文案点名的根页号在该窗口内与目录登记值不同。仍指向**同一棵树**的真实页 ⇒ 仍可定位；触发前提 = 未提交的根分裂与对损坏页的读并发。**不阻塞**。
- **F10（通过·无身份证明机制与无框架外判定层）** 独立通读新测试文件全文与 4 个源文件 diff：无 hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明；断言全部落在**可观察行为**上（`Response::Error` 的**穷尽匹配** + 错误文案含两个页号 + 行集合 / 行数）；损坏夹具**真实改坏页字节**（按文件偏移把槽目录项 offset 字段写成 `0xFFFF`），无替身；无框架外脚本、封装命令或判定层；无 timer 阈值行为判据。**通过。**
- **F11（通过·Stop when 的适用与「不 panic」不是数据页通道的行为）** Act 依 T22 的 Stop when 处理「无 `WHERE` 的 `SELECT` 走**数据页通道**（M19 路由 ⇒ `PhysicalPlan::DataScan`）⇒ **根本不读索引树页** ⇒ 该形态不可能命中索引页损坏」：判据改为「经由真实入口验证其既有行为未变」，且断言**确实存在**（损坏库上 `SELECT * FROM t` 仍返回全部 100 行）⇒ **未放宽任何判据**（改的是该形态的**前提**，不是判据的强度）。真正遍历整棵索引树的形态（复合 PK 谓词 ⇒ `Filter(Scan)` ⇒ `index_manager.scan_all()`）**命中损坏页并以明确错误结束** ⇒ Scenario 的两种查询形态均被覆盖。
- **F12（通过·Deviation 1 / 2 的裁定：Plan 遗漏的补齐，非超范围改动）** 独立判断 Act 追加的 7 处闸门（4 个写路径下降点 + 3 个再平衡 / 父页点）：
  - `insert_into_page_step` / `delete_from_page` / `update_in_page` / `delete_from_internal`：写路径决定「下探哪个子结点」「更新 / 删除哪个槽」⇒ 无闸门时，损坏页上的有界访问返回 `None` ⇒ 写路径把损坏页当作「**无此条目**」（插入追加到末尾 / 更新报 `KeyNotFound` / 删除报 `KeyNotFound`）⇒ 正是 Requirement 禁止的「当作『没有这个条目』继续」，且与 Acceptance 2–4 冲突 ⇒ **必要**。
  - `leaf_key_count` / `internal_key_count`：按兄弟页的**槽数**决定再平衡（损坏的槽数会误导合并决策）⇒ **必要**。
  - `update_parent_separator`：→ `replace_separator` → `SlottedPage::update_slot_in_place`（`slotted_page.rs:321-339`）**按槽的 offset / length 无检查地切页字节** ⇒ 损坏的父页在此 panic ⇒ 该闸门是「不 panic」的必要条件 ⇒ **必要**。
  - 全部 7 处均服务于 T23 的 Required behavior ②，且在**健康页上不改变任何行为**（`violations` 为空 ⇒ 闸门是空操作）⇒ 属**计划遗漏的补齐**，接受。
- **F13（通过·Deviation 3 的独立核实）** `index_manager.rs:905-906` 落在 `#[cfg(test)] mod tests`（起于 `index_manager.rs:769`）的测试辅助 `walk_leaf_chain` 内 ⇒ **非生产路径**，Plan 把它列为生产目标属 Plan 的行号陈旧；全仓 `get_key(…)` / `get_row_id(…)` / `get_child_page_id(…)` 的 `.unwrap()` **全部**位于 `#[cfg(test)]` 模块内（`node.rs:1256` / `index_manager.rs:769` 之后）⇒ 生产侧条目提取面就是已校验的 `scan_all_async_from_root` ⇒ **无遗留 panic 面**。
- **F14（通过·Deviation 4 的裁定：索引身份 = 索引树根页号）** 与 Plan 的 Implementation Guidance / Risk 5 一致：每个索引树有独立根页、根分裂时 `root_page_id` 随新根更新 ⇒ 根页号**唯一标识**该索引树，满足「错误须点名损坏的页与该索引树」；把索引名注入 `IndexManager`（17 处构造点）属 Plan Forbidden 的范围外改造 ⇒ **接受**。Act 记录的「不含人类可读的索引名（如 `idx_v`）」为已知取舍，不是 Acceptance 缺口。
- **F15（通过·未损坏路径零变化与既有语义）** 页级判据对健康页返回空 `violations` ⇒ 闸门为空操作 ⇒ 键序 / 命中数 / 子结点选择 / 写入结果不变；Act 的真实运行（`index_read_bounded_read_test` 5/0、`index_manager_test` 3/0、`secondary_index_test` 27/0、`index_page_ownership_test` 5/0、`index_page_ownership_stack_overflow_test` 1/0、`release_path_bounded_read_test` 7/0、`cargo test --lib` 420/0，另重跑 `ddl_rewrite_consistency_test` 9/0、`release_durability_precondition_test` 2/0、`dml_tombstone_abandon_window_test` 3/0、`mvcc_tombstone_visibility_test` 11/0、`free_page_lifecycle_test` 12/0）全部 0 failed，覆盖范围自 Act 运行后未变 ⇒ **直接沿用**。释放路径的尽力而为语义、`free_page` 集合语义、磁盘格式、`IndexManager::new` / `from_root` 的签名与 17 处调用点均**未改动**（diff 核对）。
- **F16（非阻塞 Minor·本次 Review 不跑全量）** 本 change 的全量回归预算在 Iteration 007 / T21（M19 分批串行），本 Iteration 按 Plan **不跑**全量 ⇒ 不构成 Gate 4 缺口；但意味着「索引树读路径改动 ⇒ 全仓无回归」尚未成立 ⇒ 由 T21 承担。

**Acceptance Gaps**

无。Acceptance 1–6 逐条满足：

1. **损坏索引页上的 `SELECT` 不 panic / 不 abort / 不静默错读** ✔ 真实文件库 + 真实 `Database::open` + 真实 `SELECT`（点查 `WHERE id = 1` 与整棵索引树扫描 `WHERE id = 1 AND v = 1` 各一）；损坏点 = 该语句实际扫描的**最左叶页**（非根）；断言 = `Response::Error` 的**穷尽匹配** + 文案含 `corrupted index page 3 (index root page 6)`（F5）。
2. **`INSERT` 不 panic / 不 abort** ✔ 损坏点 = 索引树根页；返回明确错误；**零副作用**（断言走不读索引树的通道观测「行未被写」）。
3. **`DELETE` 不 panic / 不 abort** ✔ 同上形态；零副作用（行未被删）。
4. **`UPDATE` 不 panic / 不 abort** ✔ 同上形态；零副作用（旧值仍在、新值不存在）。
5. **未损坏路径零变化** ✔ 对照用例 + 既有索引用例全通过（F15）。
6. **判别力与零判定层** ✔ RED 形态 = 越界切片 panic（`slotted_page.rs:128`，`65535 out of range for slice of length 4096`），未损坏对照在 RED 阶段即通过，夹具真实改坏页字节，判定全在 `cargo test` 内（F4、F10）。

**Evidence**

- 代码与 diff（本 Cycle 的工作区改动）：`src/storage/btree/node.rs`（`MAX_PAGE_SLOTS` / `slot_payload_in_bounds` / `bounded_slot_data_ref` / `bounded_slot_data_mut` / `ensure_index_page_intact` + 8 个访问器）、`src/storage/btree/btree.rs`（19 处闸门 + `sibling_ids` 改 `Result`）、`src/storage/btree/index_manager.rs`（3 处闸门）、`src/storage/error.rs`（`CorruptedIndexPage` 变体）、`tests/index_read_bounded_read_test.rs`（新增，5 用例）。
- 机制坐实的读码位点：`src/storage/integrity.rs:538-575`（`validate_slots` 判据）、`:582-646`（`check_tree_page` 的叶 / 内部分支差异）、`src/storage/page_format/slotted_page.rs:125-133`（`get_slot_data` 无界切片 = RED panic 位点）、`:231-263`（`add_slot_inner` 的容量判据 ⇒ `fso <= slot_area_start` 不变式）、`:277`（`delete_slot` 不动 `free_space_offset`）、`:321-339`（`update_slot_in_place` 无检查切片 ⇒ 父页闸门的必要性）、`src/storage/btree/node.rs:63-85`（`ensure_index_page_intact` 的两个错误分支）、`src/storage/btree/btree.rs:46`（`find_child_position`，唯一调用点 `:692` 紧邻 `:691` 闸门）、`:532` / `:784`（`sibling_ids` 的两个 `?` 传播点）、`:1334-1339`（`delete_exact_from_page` 只容忍 `KeyNotFound`）、`src/storage/page_frame.rs:38-96`（`PageDataGuard` 持 `MutexGuard` ⇒ 闸门在取锁视图上做、`modify_page` 前已释放 ⇒ 无自锁）、`src/storage/btree/index_manager.rs:769`（`#[cfg(test)]` 起点 ⇒ Deviation 3 的核实）、`:625` / `:700`（释放路径的既有语义未动）、`src/storage/data/table_manager.rs:1966-2003`（重建面仍是「损坏页 ⇒ 报错」）。
- 验证结论（**沿用 Act Response 的真实运行，覆盖范围未变化**，未重复运行）：`cargo test --test index_read_bounded_read_test -- --test-threads=1` → 5/0；`index_manager_test` 3/0；`secondary_index_test` 27/0；`index_page_ownership_test` 5/0；`index_page_ownership_stack_overflow_test` 1/0；`release_path_bounded_read_test` 7/0；`cargo test --lib` → 420/0；另重跑 `ddl_rewrite_consistency_test` 9/0、`release_durability_precondition_test` 2/0、`dml_tombstone_abandon_window_test` 3/0、`mvcc_tombstone_visibility_test` 11/0、`free_page_lifecycle_test` 12/0；`cargo build --lib` 0 warning。RED 证据：`src/storage/page_format/slotted_page.rs:128` 的越界切片 panic（`range start index 65535 out of range for slice of length 4096`）⇒ `PoisonError` ⇒ `signal: 6, SIGABRT`，四条损坏页用例逐条单独运行均落在同一断言点。Persisted Evidence = `none` ⇒ 无 `evidence/` 目录，不据此提出问题。
- 本 Review 未修改任何产品代码、未新增 / 删除任何文件 ⇒ 上列结论的覆盖范围自 Act 运行后**未变化**。

**Issue 候选（范围外实质缺陷，只报告不登记）**

- **IC-1：叶页槽负载短于条目格式（38 字节）时不被判为损坏 ⇒ 该条目被静默跳过。** 证据：`integrity::check_tree_page` 的叶页分支只校验槽边界（内部页分支另有长度校验）⇒ 短条目落在页内时 `violations` 为空 ⇒ `node.rs` 的 `get_key` / `get_row_id` 返回 `None` ⇒ `read_leaf_pair` 的 `filter_map` / `scan_all_async_from_root` / `find_key_position_binary` 把它当作「无此条目」。**承载方式**：另立 change 承载「叶页条目长度的页级判据」，按**读 / 写路径 = 明确错误**、**释放路径 = 计数 + 报出 + 跳过**两条相反处置方向分别设计（与本 Iteration 的 D13 同构）；判据扩展须同时复核 `check_tree_page` 的既有释放面用法（Iteration 002 的成果）不受破坏。本 change **不承载**（T23 Forbidden + Iteration 006 的 Non-goals）。
- **IC-2：存储层其余页型的无界读点（数据页 / 目录页 / 恢复面 / 缓冲池，同形态约 17 处）。** 证据：全仓 `get_slot_data` 的未检查调用点分布在 `src/storage/data_page.rs` / `src/storage/catalog.rs` / `src/storage/buffer_pool.rs` / `src/storage/data/table_manager.rs` / `src/wal/recovery.rs` / `src/executor/*`。**承载方式**：按**页型**分组另立 change，每组各自判定「损坏页在该页型的正确行为」（数据页与目录页的损坏语义与索引树不同：索引树是「下降决策」，数据页是「行内容」），避免一次 change 混合不同处置方向；每个页型的夹具沿用本 Cycle 已验证的字节级损坏注入手法（`tests/release_path_bounded_read_test.rs` / `tests/index_read_bounded_read_test.rs` 共用的 `slot_offset_field_offset` / `page_file_offset` / `corrupt_slot_offset`）。本 change **不承载**（Iteration 006 的 Non-goals + 验证预算：每组页型需各自的损坏形态与恢复语义判定）。

**Follow-up Decision**

接受当前 Cycle。判据：既有 Acceptance 1–6 **全部满足**且无阻塞项（F1–F5、F10–F15 为通过项；F6–F9、F16 为**非阻塞 Minor finding**；F12–F14 对 Act 的 4 条 Deviation 逐条裁定为合规）。F6 的叶页短条目缺口**非回归**、**明确在 Plan 范围外**（T23 Forbidden）、**不属本 Iteration 的 Acceptance 形态**⇒ 登记为 Issue 候选 IC-1，不要求当前 Cycle 修复。**不要求当前 Cycle 修复**，不创建 rework / replan Cycle。Iteration 006 的 Iteration Plan **保持不变**（T22 / T23、依赖 Iteration 002、验收边界、验证边界均无需修改）。当前 Iteration 完成 ⇒ 按 Iteration Map 展开**下一个** Iteration（007）。

**Iteration Plan Update**

None（Iteration 006 的 Iteration Plan 不变）。

**Next Cycle**

None。

**Next Iteration**

`openspec/changes/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/007-legacy-compat-evidence/000-initial.md`（Iteration 007 / Cycle 000，承载 T16–T21：旧格式兼容的证据补齐 + 本 change 收尾全量回归 T21）。

## Act Response

- Status: reported
- Cycle: `iterations/006-index-read-bounded-read/000-initial.md`（initial Cycle）
- Tasks: T22 `done`、T23 `done`（本 Cycle 的全部 task 正常完成，无 repair item）

### 实际改动

| 文件 | 符号 | 改动 |
|---|---|---|
| `src/storage/btree/node.rs` | `MAX_PAGE_SLOTS` / `slot_payload_in_bounds` / `bounded_slot_data_ref` / `bounded_slot_data_mut` | **新增**：与 `integrity::validate_slots` **同口径**的有界槽读——槽负载必须完整落在页数据区内（不与页头重叠、不越过槽目录区）；越界 ⇒ `None`，不切片。读槽**之前**先判 `index < MAX_PAGE_SLOTS`（页头声称的槽数超限时，`get_slot` 的槽目录定位 `PAGE_SIZE - (i+1)*Slot::SIZE` 会下溢成越界切片） |
| `src/storage/btree/node.rs` | `ensure_index_page_intact` | **新增**：读 / 写路径的**页级校验闸门**——经 `integrity::check_tree_page` 判页；`invalid_type` ⇒ 既有 `StorageError::InvalidPageType`（形态未改）；`violations` 非空 ⇒ 新变体 `StorageError::CorruptedIndexPage`（点名损坏页 + 索引树根页） |
| `src/storage/btree/node.rs` | 8 个索引树访问器 | `LeafNode::get_key` / `LeafNode::get_row_id` / `LeafNodeRef::get_key` / `LeafNodeRef::get_row_id` / `InternalNode::get_key` / `InternalNode::get_child_page_id` / `InternalNodeRef::get_key` / `InternalNodeRef::get_child_page_id` **全部**改用有界槽读（原：未检查的 `get_slot_data`） |
| `src/storage/error.rs` | `StorageError::CorruptedIndexPage { page_id, index_root_page_id, detail }` | **新增变体**；文案 `corrupted index page {page_id} (index root page {index_root_page_id}): {detail}` |
| `src/storage/btree/btree.rs` | 19 处访问点 | 逐点加页级校验（清单见「已加校验的访问点」） |
| `src/storage/btree/btree.rs` | `sibling_ids` | 签名 `(Option<PageId>, Option<PageId>)` ⇒ `Result<(Option<PageId>, Option<PageId>)>`——把父页号与索引根页号带进校验点（唯一被改的既有函数签名；私有自由函数，2 处调用点） |
| `src/storage/btree/index_manager.rs` | 3 处访问点 | `search_from_page_async` / `search_all_from_page_async` / `scan_all_async_from_root` 加页级校验 |
| `tests/index_read_bounded_read_test.rs` | 新增文件 | 5 条用例：4 条损坏页（`SELECT` / `INSERT` / `DELETE` / `UPDATE`）+ 1 条未损坏对照 |

**未改动**：`src/storage/page_format/slotted_page.rs`（`get_slot_data` 的签名与语义均未动）、`src/storage/integrity.rs`（`check_tree_page` / `validate_slots` 的判据本身与释放路径对它的用法均未动）、`IndexManager::new` / `from_root` 的签名与 17 处调用点、磁盘格式、`free_page` 集合语义、释放路径的尽力而为语义。

### 已加校验的访问点（Plan Risk 要求的逐点清单）

`btree.rs`（19 处，索引根页号取 `self.root_page_id`，与该 `BTree` 的树根一致）：

| # | 位置 | 函数 | 该点据页字节决定什么 |
|---|---|---|---|
| 1 | `btree.rs:85` | `sibling_ids` | 父页的兄弟子指针（写路径的子结点选择） |
| 2 | `btree.rs:161` | `search_from_page` | 同步下降（叶命中 / 内部选子） |
| 3 | `btree.rs:199` | `search_from_page_async` | 异步下降 |
| 4 | `btree.rs:296` | `insert_into_page_step` | **写路径**下降 / 叶插入位置 |
| 5 | `btree.rs:480` | `delete_from_page` | **写路径**下降（叶 / 内部分派） |
| 6 | `btree.rs:583` | `leaf_key_count` | 兄弟叶的键数（再平衡决策） |
| 7 | `btree.rs:590` | `internal_key_count` | 兄弟内部结点的键数（再平衡决策） |
| 8 | `btree.rs:691` | `delete_from_internal` | **写路径**的子结点选择（`find_child_position` 的页） |
| 9 | `btree.rs:765` | `find_separator_slot` | 父页分隔键槽定位 |
| 10 | `btree.rs:953` | `get_parent_separator_key` | 父页分隔键取值 |
| 11 | `btree.rs:969` | `update_parent_separator` | 父页分隔键原槽改写（`replace_separator` → `update_slot_in_place` 会按槽的 offset / length 切页字节） |
| 12–13 | `btree.rs:992` / `:993` | `read_leaf_pair` | 两个叶页的条目提取 |
| 14 | `btree.rs:1027` | `read_internal_seps` | 内部页的分隔键提取 |
| 15 | `btree.rs:1060` | `scan_all` | 同步全扫的下降 |
| 16 | `btree.rs:1120` | `update_in_page` | **写路径**下降 / 叶更新位置 |
| 17 | `btree.rs:1155` | `search_all_from_page` | 等值候选子树下降 + 条目提取 |
| 18 | `btree.rs:1227` | `delete_all_from_page` | **写路径**下降 + 条目提取 |
| 19 | `btree.rs:1299` | `delete_exact_from_page` | **写路径**下降 + 条目提取 |

`index_manager.rs`（3 处，索引根页号取 `self.root_page_id.load(Acquire)`）：

| # | 位置 | 函数 | 该点据页字节决定什么 |
|---|---|---|---|
| 20 | `index_manager.rs:301` | `search_from_page_async` | 异步下降（`SELECT` 点查与 DML 的索引下降） |
| 21 | `index_manager.rs:338` | `search_all_from_page_async` | 等值候选子树下降 + 条目提取 |
| 22 | `index_manager.rs:544` | `scan_all_async_from_root` | 条目提取全扫的下降 |

**未加校验但不受影响的点**：

- `IndexManager::collect_all_pages`（`index_manager.rs:625` 起）——**释放路径**，Iteration 002 的成果（计数 + 报出 + 跳过其子树），处置方向与读路径**相反**，本 Cycle **有意不改**。
- `table_manager.rs` 的 `collect_tree_pages`（重建面）——非读 / 写热路径，且同样沿用 `check_tree_page` 的既有用法。
- 只读页头的路径：`shrink_root`（根的 `key_count` / `leftmost_child`）、`handle_child_merge`（`iref.key_count()`）、`rebuild_leaf` / `rebuild_internal` 的 `next_leaf_page_id` —— 只取页头字段，**不取槽负载**，页头字段的读取本身是有界的（固定 16 字节头）；这些函数所处的页在进入前已被上表中的点校验过（`rebuild_*` 的条目来自已校验的 `read_leaf_pair` / `read_internal_seps`）。
- **Plan Risk 提到的 `index_manager.rs:905-906` 的 `.unwrap()`**：在当前工作区该行号落在 `#[cfg(test)] mod tests`（起于 `:750`）的测试辅助 `walk_leaf_chain` 内，**不是生产路径**；全仓 `get_key(...)` / `get_row_id(...)` / `get_child_page_id(...)` 的 `.unwrap()` 全部位于 `#[cfg(test)]` 模块内（`grep` 全量确认）。生产侧的条目提取面就是上表的 `scan_all_async_from_root`（已校验）；且有界读之后访问器只可能返回 `None`、不会越界切片 ⇒ 该面不再有「越界切片」形态。

### 两套机制各自的用例与判据

**机制 1（有界槽读 ⇒ 不 panic）**：承载在 8 个访问器上，是**最后一道保障**——即使页级校验漏掉一处，越界槽也只会得到 `None` 而不会切越界区间。判据体现在 RED→GREEN：RED 的四处失败形态是 `src/storage/page_format/slotted_page.rs:128` 的**越界切片 panic**；GREEN 后同一批语句不再出现该 panic。

**机制 2（页级校验 ⇒ 显式报错，不静默错读）**：判据是**错误文案点名损坏页与索引树根页**，且语句整体失败。实测文案（`SELECT` 用例；损坏页 = **最左叶 3**，索引树根页 = **6**——两个不同的页号）：

```
Execution error: corrupted index page 3 (index root page 6): 1 violation: slot 0 data range [65535, 65573) overlaps the page header or the slot area
```

（根页损坏的 `INSERT` / `DELETE` / `UPDATE` 用例的实测文案同形：`corrupted index page 6 (index root page 6): 1 violation: slot 0 data range [65535, 65571) overlaps the page header or the slot area`。）

**不得退化为「行不存在」**的判据：`assert_damaged_index_error` 对响应做**穷尽匹配**——只要不是 `Response::Error` 就 panic；即「空结果 / 别的行」不可能通过。四条损坏页用例同时断言文案含 `corrupted index page {损坏页号}` 与 `(index root page {索引树根页号})`；`SELECT` 用例的损坏页是**非根**的叶页 ⇒ 两个页号**不同**，证明错误同时点名了损坏页与索引身份。

### 读路径与释放路径的语义区别（如何在实现中保持）

- 读 / 写路径：`ensure_index_page_intact` 在**每次**据页字节决定下降 / 取条目 / 取子指针之前返回 `Err` ⇒ 语句整体失败 ⇒ **绝不**进入该页的任何子结点、**绝不**把该子树当作「不存在」。这与「跳过子树 ⇒ 点查返回『查不到该行』⇒ 静默错读」正相反。
- 释放路径：`IndexManager::collect_all_pages` 保持 Iteration 002 的既有语义（`check_tree_page` → 计数 + 报出 + 跳过其子树 + 返回已收集的部分），**一行未改**；两条路径的处置方向**相反**且互不照搬。
- 夹具层面同时可观察到两者并存：损坏库被打开时仍出现 `[free-page-rebuild] rebuild abandoned: internal error: index tree page 6 is damaged: …`（释放 / 重建面的尽力而为告警，语句照常可执行），而同一页上的读 / 写语句**以明确错误结束**。

### 验证命令与实际结果（全部当次运行）

**先行 RED**（在**临时关闭两处机制**的构建上取得：`bounded_slot_data_*` 退回未检查的 `get_slot_data`、`ensure_index_page_intact` 退回空实现 ⇒ 索引树读路径的可观察行为与修复前逐条相同；夹具、语句与断言与 GREEN 相同）：

```
$ cargo test --test index_read_bounded_read_test
running 5 tests
test undamaged_index_reads_and_writes_are_unchanged ... ok
thread 'insert_on_damaged_index_page_makes_insert_fail_with_a_named_error' panicked at src/storage/page_format/slotted_page.rs:128:19:
range start index 65535 out of range for slice of length 4096
thread '…' panicked at src/storage/page_frame.rs:101:27:
called `Result::unwrap()` on an `Err` value: PoisonError { .. }
  process didn't exit successfully: … (signal: 6, SIGABRT: process abort signal)
```

逐用例（`cargo test --test index_read_bounded_read_test -- --exact <name>`，四条损坏页用例各自单独运行）**全部**落在同一个断言点上：

```
select  → range start index 65535 out of range for slice of length 4096
insert  → range start index 65535 out of range for slice of length 4096
delete  → range start index 65535 out of range for slice of length 4096
update  → range start index 65535 out of range for slice of length 4096
```

⇒ RED 的失败形态是**越界切片 panic**（不是任何其它失败形态），且**伴随进程 abort**（panic 发生在持页锁的异步执行路径 ⇒ 页帧析构时二次 panic ⇒ SIGABRT）——正是本 Iteration 承载的缺陷。未损坏对照用例在 RED 阶段即通过（`ok`）。

**GREEN 与回归**（修复后，全部当次运行）：

| 命令 | 结果 |
|---|---|
| `cargo test --test index_read_bounded_read_test -- --test-threads=1` | `test result: ok. 5 passed; 0 failed` |
| `cargo test --test index_manager_test` | `ok. 3 passed; 0 failed` |
| `cargo test --test secondary_index_test` | `ok. 27 passed; 0 failed` |
| `cargo test --test index_page_ownership_test -- --test-threads=1` | `ok. 5 passed; 0 failed` |
| `cargo test --test index_page_ownership_stack_overflow_test -- --test-threads=1` | `ok. 1 passed; 0 failed` |
| `cargo test --test release_path_bounded_read_test` | `ok. 7 passed; 0 failed` |
| `cargo test --lib` | `test result: ok. 420 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |

`cargo test --lib` 的 **420 基线仍成立**（本 Cycle 改了 `src/storage/btree/{node,btree,index_manager}.rs` 与 `src/storage/error.rs` ⇒ Iteration 005 的 420 条结论失效，已按 Plan 要求重跑）。`cargo build --lib` **0 warning**。

因本 Cycle 触及索引读 / 写面而**重跑**（而非沿用）的其余既有目标：`ddl_rewrite_consistency_test` 9/0、`release_durability_precondition_test` 2/0、`dml_tombstone_abandon_window_test` 3/0、`mvcc_tombstone_visibility_test` 11/0、`free_page_lifecycle_test` 12/0。

**本 Iteration 不跑全量回归**（change 预算，归 Iteration 007 / T21 的 M19 分批串行）。本次全部为**定向单目标**验证，未使用全量调用、默认并行度跑批或任何框架外判定层。

### Plan Review 中「Stop when」的一处适用（记入，不改判据）

**无 `WHERE` 的 `SELECT * FROM t` 走数据页通道（M19 路由 ⇒ `PhysicalPlan::DataScan`），本就不读索引树页** ⇒ 该形态**不可能**命中索引页损坏。按 T22 的 Stop when：把该形态的判据改为「经由真实入口验证其既有行为未变」——损坏库上 `SELECT * FROM t` 仍返回全部 100 行（断言在 `damaged_index_page_makes_select_fail_with_a_named_error` 中）。**未**为此放宽任何判据。

真正遍历整棵索引树的 `SELECT` 形态是**复合 PK 谓词**（`WHERE id = 1 AND v = 1` ⇒ `Filter(Scan)` ⇒ `index_manager.scan_all()` ⇒ 遍历全部叶）——它命中损坏页并以明确错误结束，两条形态都已覆盖。

### Gate 3 / Gate 4 逐项结论

**Gate 3（Task Review）— PASS**

- Spec compliance：IRBR R1 的四个损坏页 Scenario + 「未损坏路径零变化」Scenario 全部实现并被用例覆盖；Plan 列出的文件与符号全部处理（`node.rs` 8 个访问器 + 派生辅助经由访问器继承；`btree.rs` 12 个列出点全部加校验；`index_manager.rs` 3 个列出点全部加校验；`error.rs` 新变体）；不变量（不跳过子树 / 不 panic / 未损坏路径零变化 / 释放路径处置不变 / 磁盘格式与 `free_page` 集合不变）逐条成立；先行测试符合任务类型（RED 为越界切片 panic，GREEN 为明确错误）。
- Code quality：diff 无计划外修改（`git diff` 逐文件复核：4 个源文件 + 1 个新测试文件）；错误 / 边界 / 状态 / 资源生命周期正确（页级校验在取页锁的视图上做，`modify_page` 前已释放 `PageDataGuard`，无自锁）；`cargo build --lib` 0 warning；测试不会因错误原因通过（损坏夹具真实改坏页字节，RED 的失败形态被逐用例确认为越界切片 panic）；无身份证明机制、无判定层、无 timer 阈值行为判据。
- Critical / Important 问题：**0 项未解决**（见下节）。

**Gate 4（Verified Completion）— PASS**

Task Definition 的 Verification 六条命令全部当次运行、各自 0 failed（含 `cargo test --lib` 的 420 基线）；完成声明基于真实运行的输出；无人工验证项；无 `required` Evidence。

### 自检发现、已修复内容与遗留 Minor 问题

- **已修复（实施过程中的自检发现）**：三条损坏页用例最初的「零副作用」断言用了会读索引的查询（`WHERE id = … AND v = …`）⇒ 它们会撞上同一个损坏页而测不到副作用。改为走**不读索引树的通道**（无 `WHERE` 的 `SELECT` 走数据页通道；非 PK 列谓词 `WHERE v = …` 同样走数据页通道）后，断言才真正观测「拒绝发生在写入之前 ⇒ 行未写 / 未删 / 未改」。
- **遗留 Minor 问题**：
  1. **页级校验的开销**：每次索引页访问都经 `check_tree_page`（其 `validate_slots` 为每页分配一个 `Vec`（容量 = 槽数，最多约 97 项））⇒ 索引下降的热路径上多出每页一次分配。本 Cycle 的 Acceptance 不含性能判据，未做测量；作为**风险**记录，交 Plan Review 裁定是否需要后续专项。
  2. `ensure_index_page_intact` 对 `invalid_type` 采用既有 `StorageError::InvalidPageType { expected: LEAF_NODE, actual }`——`expected` 沿用释放路径 `collect_all_pages` 的同款取值（该处同样是「不是任何树」的场景），但对本 Cycle 而言调用点的期望类型可能原本是内部结点 ⇒ 文案里的 `expected` 在该形态下不完全贴合。变体本身与既有形态**未改**（Plan Forbidden）。
  3. `sibling_ids` 的签名由返回元组改为 `Result<元组>`——私有自由函数、2 处调用点，代价可忽略。

### Deviations from Plan

1. **写路径的下降点比 Plan 列表多加了 4 处**：`insert_into_page_step`（`btree.rs:296`）、`update_in_page`（`:1120`）、`delete_from_page`（`:480`）、`delete_from_internal`（`:691`）。Plan 的 `btree.rs` 目标列表只列了 12 个下降 / 条目提取函数，但 `INSERT` / `UPDATE` / `DELETE` 的**写路径下降**分别落在这 4 处（Plan 的列表里的 `find_child_position` / `sibling_ids` 是它们的下游）。不加这 4 处 ⇒ 写路径仍会把损坏页当作「无此条目 / 空子树」⇒ 静默错读（比 abort 更隐蔽）⇒ 与 Acceptance 2–4 冲突。属于**计划遗漏**的补齐，非设计变更。
2. **另加 3 处**：`leaf_key_count`（`:583`）/ `internal_key_count`（`:590`）/ `update_parent_separator`（`:969`）——前者按兄弟页的槽数决定再平衡（损坏页的槽数会误导合并决策），后者是父页分隔键的原槽改写（`update_slot_in_place` 会按槽的 offset / length 切页字节）。
3. **未加校验项已逐条说明**（见「未加校验但不受影响的点」），其中 Plan Risk 提到的 `index_manager.rs:905-906` 的 `.unwrap()` 经全量确认位于 `#[cfg(test)]` 模块内（非生产路径）。
4. **索引身份 = 索引树根页号**（按 Plan 的 Implementation Guidance 与 Risk 第 5 条执行）：错误文案点名 `index root page {根页号}`，**未**把索引名注入 `IndexManager`（17 处构造点，范围外）。Plan Risk 已要求「若认为该形态不足以点名索引，不得自行扩到接口改造 ⇒ 记入 Act Response 由 Plan Review 裁定」——此项**提请 Plan Review 裁定**：根页号是索引树的唯一标识（根分裂时 `root_page_id` 随新根更新），但**不含**索引名（如 `idx_v`）这一人类可读标识。
5. **用例命名与 Cycle RTM 的计划名一致**（`damaged_index_page_makes_select_fail_with_a_named_error` / `…_insert_…` / `…_delete_…` / `…_update_…` / `undamaged_index_reads_and_writes_are_unchanged`）⇒ 未发生改名，RTM 的「计划名」列无需更新。

### Persisted Evidence

`None required`——Plan 的 `Persisted Evidence` 模式为 `none`，且全部结论可低成本重跑（1 个新增测试文件 + 5 个既有索引 / 释放目标 + `cargo test --lib` + 5 个可沿用目标），不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ 未创建 `evidence/` 目录。

### Experience Candidates

1. **Issue 候选（范围外，同根因层）**：存储层其余无界读点（数据页 / 目录页 / 恢复面 / 缓冲池，同形态约 17 处）仍用未检查的 `get_slot_data` 切页字节 ⇒ 损坏页在各自路径上的正确行为尚未判定。证据：本 Cycle 的 RED 证据（`src/storage/page_format/slotted_page.rs:128` 的越界切片 panic ⇒ SIGABRT）与 Plan Context 的 Non-goals 登记。各页型的「损坏页正确行为」需各自调查与设计决定（与本 Iteration 的读 / 写路径语义不同者尤其需要单独裁定）。
2. **Runbook 候选**：**以字节改写注入损坏页**的夹具流程——真实文件库 ⇒ 写数据 ⇒ `checkpoint()` + `close()`（确保字节落盘）⇒ 按文件偏移把索引树页的槽目录项 offset 字段改写为 `0xFFFF`（槽目录项 i 在页尾 `PAGE_SIZE - (i+1)*Slot::SIZE`，offset 字段再 +2；页在文件中的偏移 `HEADER_SIZE + page*PAGE_SIZE + in_page`）⇒ 重开 ⇒ 真实语句。已端到端验证成功且可重复，现被两个测试文件复用（`tests/release_path_bounded_read_test.rs`、`tests/index_read_bounded_read_test.rs`）；对后续承载其余页型的专项 change 可直接复用。证据：两个测试文件的夹具函数与本 Cycle 的 RED / GREEN 实跑结果。

### 未解决问题与停止条件

- 未解决项：**无**（本 Cycle 的 T22 / T23 全部完成，Gate 3 / Gate 4 均通过）。
- 停止条件：不适用——本 Cycle 的全部 task 正常完成，等待用户审计与 Plan Review 填写 `Review Result`（本文件不得由 Act 填写）。
