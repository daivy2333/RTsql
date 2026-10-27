# Iteration 000 / Cycle 000: 索引树叶条目长度纳入页级判据

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 既有域 `index-read-bounded-read`（IRBR：索引树读 / 写路径的有界读与「明确错误」语义）、既有域 `release-path-bounded-read`（RPBR：释放路径的有界读与「计数 + 报出 + 跳过」语义——**处置方向与读 / 写路径相反**）、既有域 `index-page-ownership` / `index-write-side-atomicity`（索引页归属与写侧原子性）、既有域 `free-page-lifecycle`（释放集合语义）、既有域 `ddl-rewrite-consistency`（改写窗口内的读拒绝）、`design.md` D6（ISS26 的两条相反处置）与 D10（验证预算）、D11（禁止事项）、`.agents/runbooks/m19-batched-full-regression.md`（M19 分批串行纪律）、ISS23 的规模约束（本 change 内不使用 ≥ 300 行的表规模构造用例）。
- Excluded scope: 其余页型的无界读点（数据页 / 目录页 / 恢复面 / 缓冲池，同形态约 21 处——Iteration 001 / 003 / 004）；DDL 提交点统一模型（Iteration 005）；写盘失败状态（Iteration 006）；dump / restore 合同面（Iteration 007）；语料面（Iteration 008）；磁盘格式（保 MS36 的 G1 / G2 / G3 兼容面）；`free_page` / `complete_free_release` / `promote_free_releases` 的集合语义；索引名注入 `IndexManager`（17 处构造点）；损坏页的**修复**（本 Cycle 只判别，不修）；完整性扫描的诊断分级。
  - **（2026-10-25 二次结构修正见 `design.md` D15：写路径组（T18 / T19）插入为 Iteration 003，其后依次顺延 ⇒ 上列页型组 001 / 003 / 004 现为 **001 / 004 / 005**；DDL 提交点统一模型 → **006**；写盘失败状态 → **007**；dump / restore 合同面 → **008**；语料面 → **009**。）**

**Objective**

索引树**叶页**的**条目长度**纳入页级判据 ⇒ 一条槽负载长度短于条目格式（`MAX_KEY_LEN + RowId::SIZE` = 38 字节）但偏移仍落在页内的叶条目，在**读 / 写路径**上不再被当作「没有这个条目」而静默跳过，而是让语句以**点名损坏的页与该索引树**的明确错误结束；在**释放路径**上则按既有语义**计数 + 报出 + 跳过**（该页被计入损坏页计数并被报出，其子树不被释放，语句不 abort）。未损坏路径的读 / 写与释放行为**零变化**。

**Background**

本缺陷由前一轮 change（`2026-10-24-open-defect-batch-storage-ddl-legacy-evidence`）的 Iteration 006 的 Plan Review（2026-10-25）登记为 Issue 候选 **IC-1**：「叶页槽负载短于条目格式（38 字节）时不被判为损坏 ⇒ 该条目被静默跳过」，并明确「判据扩展须同时复核 `check_tree_page` 的既有释放面用法（Iteration 002 的成果）不受破坏」。该候选已由 Recorder 登记为台账条目 **ISS26**（`.agents/issues/ISS26-index-leaf-entry-length-unchecked.md`，状态 `open`）。

同一评审登记的 **IC-2**（其余页型的无界读点）已由 Recorder 登记为 **ISS27**（Iteration 001 / 003 / 004 承载）。

**Investigation Facts**

- Starting point:
  - 工作区 HEAD `e9cb9d3`（「save: 缺陷台账生命周期清理——六条已关闭 Issue…」）。
  - 工作区当前**唯一的既有未提交修改**是 `.agents/docs/tasks.md`（MS37–MS40 路线批次）——属**既有现场**：本 Cycle 在其上继续，**不回滚、不清理、不提交**。本 change 的产物目录 `openspec/changes/2026-10-25-ms37-defect-ledger-closure/` 为本 Cycle 新建。
  - 前序全量终态 **2231 passed / 0 failed / 2 ignored**（143 目标 / 14 批，来源 `.agents/runbooks/m19-batched-full-regression.md`）在本 change 改动前**已失效** ⇒ **不得**作为依据；按 change 预算，本 change 的收尾全量归 **Iteration 008 / T16**（M19 分批串行纪律）⇒ **本 Cycle 不跑全量**。
  - 前序定向结论的**有效性裁定**：本 Cycle 改 `src/storage/integrity.rs` ⇒ 覆盖到该文件的既有结论失效（须重跑）；覆盖到 `src/storage/btree/*` 的结论**不受本 Cycle 改动影响**（本 Cycle 不改这些文件），但为确认「未损坏路径零变化」，相关回归仍在本 Cycle 重跑（成本低、结论直接可用）。
  - 已知限制（不在本 Cycle）：其余页型的无界读点（ISS27）；DDL 提交点统一模型（ISS25 / ISS28 / ISS29）；写盘失败状态（ISS30 / ISS31）；dump / restore 合同面（ISS32 / ISS33）。
- Current-State Evidence（Plan 已逐行确认，行号为**当前工作区**；Act 不需回读即可执行）:
  - **页级判据的唯一实现点**：`src/storage/integrity.rs:593` 的 `pub(crate) fn check_tree_page(data: &[u8]) -> TreePageCheck`。它按页类型分两支，返回 `children` / `invalid_type` / `violations`。
  - **叶页分支缺条目长度判据**：`src/storage/integrity.rs:606-613` 的 `LEAF_NODE =>` 分支体为
    ```rust
    let mut violations = Vec::new();
    // Slot bounds only — the child is the sibling chain pointer.
    let _ = validate_slots(data, &header, &mut violations);
    out.violations = violations;
    ```
    ⇒ `validate_slots` 的返回值（逐槽的 `(序号, 槽, 数据起点, 数据终点)` 列表）被**丢弃**，叶页**只做槽边界校验**。
  - **内部页分支已有条目长度判据**：`src/storage/integrity.rs:614-641` 的 `INTERNAL_NODE =>` 分支在 `if let Some(slots) = validate_slots(...)` 之后逐槽判
    `if end - start < MAX_KEY_LEN + 4 { violations.push(format!("slot {i} is too short for an internal-node entry ({} bytes, need {})", end - start, MAX_KEY_LEN + 4)); continue; }`
    ⇒ **叶 / 内两支的判据不对称**，正是 ISS26 的缺口形态。
  - **叶条目的格式**：`[Key][RowId]` ⇒ `MAX_KEY_LEN`（`src/storage/page_format/key.rs:4` = **32**）+ `RowId::SIZE`（`src/storage/page_format/row_id.rs:34` = **6**）= **38 字节**。`src/storage/integrity.rs:32` 已 `use crate::storage::page_format::MAX_KEY_LEN;`（`RowId` 未导入）。
  - **`validate_slots` 的语义**（`src/storage/integrity.rs:538-577`）：逐槽做范围检查（`start < SlottedPageHeader::SIZE` 或 `end > slot_area_start` ⇒ 记 violation 并**跳过该槽**）；返回 `None` 仅在「槽数本身使迭代不安全」时（记 violation 并早退）；否则返回**页内合法槽**的 `(i, slot, start, end)` 列表。
  - **读 / 写路径的闸门**：`src/storage/btree/node.rs:63` 的 `pub(crate) fn ensure_index_page_intact(data, page_id, index_root_page_id)`——`check_tree_page` 的 `invalid_type` ⇒ `StorageError::InvalidPageType { expected: LEAF_NODE, actual }`；`violations` 非空 ⇒ `StorageError::CorruptedIndexPage { page_id, index_root_page_id, detail }`，`detail = integrity::violation_summary(&check.violations)`（`src/storage/integrity.rs:479-483` ⇒ 形如 `1 violation: <逐条明细>`）。该闸门由既有 Iteration 006 的成果铺在**每个**「据索引树页字节决定下降 / 取条目 / 取子指针」的访问点上 ⇒ **判据扩展自动覆盖全部读 / 写路径，无需改访问器**。
  - **叶条目被静默跳过的机制**：索引树访问器已是有界读（`src/storage/btree/node.rs` 的 `bounded_slot_data_ref` / `bounded_slot_data_mut`）⇒ 短负载**不越界**、返回 `Some(&[u8])`（长度 = 实际负载长）⇒ `get_key` / `get_row_id` 因长度不足返回 `None` ⇒ 该条目被当作「没有这个条目」跳过 ⇒ **点查返回「查不到该行」** ⇒ 静默错读（比 abort 更隐蔽）。
  - **释放路径的处置点**：`src/storage/btree/index_manager.rs:644` 的 `collect_all_pages`（及 `:700` 的 `collect_all_pages_tolerant`）用 `check_tree_page` 读树页 ⇒ `violations` 非空 ⇒ `eprintln!("[index-release] index tree page {} is damaged: {}", page_id.0, violation_summary(...))` 并 `continue`（**跳过该子树**，已收集的页保留）⇒ 释放路径的「计数 + 报出 + 跳过」由**同一处判据**驱动。
  - **既有夹具形态**（真实改坏页字节，非替身）：`tests/index_read_bounded_read_test.rs` 的 `slot_offset_field_offset(slot_index)`（槽目录项 i 落在页尾 `PAGE_SIZE - (i+1)*Slot::SIZE`，**offset 字段**再 +2）、`page_file_offset(page, in_page)`（`HEADER_SIZE + page*PAGE_SIZE + in_page`）、`corrupt_slot_offset(path, page, slot_index)`（把 offset 字段写成 `0xFFFF`）、`seed_pk_tree(path)`（100 行 ⇒ PK 树 = 内部根 + 两个叶；返回 `(pk_root, leftmost_leaf)`，并断言根页类型为 `INTERNAL_NODE`）、`assert_damaged_index_error(resp, page_id, index_root_page_id)`（断言 `Response::Error` 且文案含 `corrupted index page {page_id}` 与 `(index root page {index_root_page_id})`）、`row_count` / `affected_rows`。
  - **槽目录项的字节布局**：`[logical_id:2][offset:2][length:2]`（`Slot::SIZE = 6`）⇒ **length 字段**位于槽目录项起点 **+4**（既有 helper `slot_offset_field_offset` 返回的是 **offset 字段**的位置 = 槽目录项起点 +2）⇒ 新的「短叶条目」夹具可由既有 helper 位置 +2 定位 length 字段。
  - **释放路径的既有断言形态**：`tests/release_path_bounded_read_test.rs` 的 `run_cli_ddl(&dir, &path, sql) -> (退出码, stderr)`（走**真实 CLI 入口**），既有断言如 `stderr.contains(&format!("index tree page {sec_root} is damaged: 1 violation"))` ⇒ 释放面的「报出」是**可经真实入口观测**的。
- Code and Critical Path:
  - **读 / 写路径**：`Database::execute_sql` → 执行器（`src/executor/*`）→ `IndexManager` / `BTree` 的下降与条目提取 → `src/storage/btree/node.rs` 的访问器族 → **闸门 `ensure_index_page_intact`** → `integrity::check_tree_page`。⇒ **判据扩展点 = `check_tree_page` 的叶页分支**（唯一落点）。
  - **释放路径**：`DROP INDEX` / `DROP TABLE` / `TRUNCATE` → `IndexManager::collect_all_pages` → **`integrity::check_tree_page`** ⇒ 同一个落点，处置方向相反。
  - ⇒ **一处判据扩展同时达成两条相反的处置方向**（这是 ISS26 处置方向「看似相反、实则同源」的关键）。

**Implementation Guidance**

- **唯一产品代码落点**：`src/storage/integrity.rs` 的 `check_tree_page` 的 **叶页分支**：把 `let _ = validate_slots(data, &header, &mut violations);` 改为**消费返回值**（`if let Some(slots) = validate_slots(...)`），并对每个页内合法槽补与内部页分支**同型**的长度判据：
  `if end - start < MAX_KEY_LEN + RowId::SIZE { violations.push(format!("slot {i} is too short for a leaf-node entry ({} bytes, need {})", end - start, MAX_KEY_LEN + RowId::SIZE)); continue; }`
  （叶分支**不需要**提取子指针——叶页的孩子是兄弟链指针 `header.next_page_id`，既有逻辑已处理。）
- `RowId::SIZE` 需在 `src/storage/integrity.rs` 顶部补导入：`RowId` 已由 `src/storage/page_format/mod.rs:8` 再导出（`pub use row_id::RowId;`）⇒ 可并入既有的 `use crate::storage::page_format::{Slot, SlottedPageHeader, SlottedPageRef};`（`:33`）为 `use crate::storage::page_format::{RowId, Slot, SlottedPageHeader, SlottedPageRef};`；`MAX_KEY_LEN` 已在 `:32` 导入，无需重复。
- **不得**改动：`src/storage/btree/node.rs`（访问器与闸门）、`src/storage/btree/btree.rs`、`src/storage/btree/index_manager.rs`、`src/storage/error.rs`（`CorruptedIndexPage` 变体已存在且文案形态已被既有用例锁定）、`src/storage/page_format/*`、目录与页格式、`free_page` 集合语义。
- **判据对称性**：叶分支的长度判据与内部页分支的判据**同型**（同一 `violations` 通道、同一条 `violation_summary` 汇总通道）⇒ 读 / 写路径的 `CorruptedIndexPage` 与释放路径的 `[index-release] … is damaged: …` 两条既有报出形态**均自动成立**，无需新增错误变体或报出通道。
- **健康页不误报**：页内合法槽的叶负载长度恒为 38（叶条目写入路径只写 `[Key][RowId]`）⇒ 健康叶页的 `violations` 仍为空 ⇒ 读 / 写路径的闸门仍是空操作。

**Task Definitions**

### T1: 短叶条目在读 / 写路径给出明确错误、在释放路径被计数 + 报出 + 跳过——先行 RED 用例

- Requirement/Scenario: `specs/index-read-bounded-read/spec.md`（本 change 的增量域）IRBR R1 的「叶页条目长度过短的索引树页给出明确的错误」Scenario 与「未损坏路径的读 / 写行为零变化」Scenario；`specs/release-path-bounded-read/spec.md`（本 change 的增量域）RPBR R1 的「叶页条目长度过短的索引树页被计数、报出并跳过」Scenario；`design.md` D6。
- Depends on: None
- Targets: `tests/index_read_bounded_read_test.rs`（新增用例与新的损坏夹具 helper）；`tests/release_path_bounded_read_test.rs`（新增用例与新的损坏夹具 helper）。作为被测对象：`src/storage/integrity.rs` 的 `check_tree_page`（本任务**不**修改它）。
- Current behavior: 索引树叶页某个槽的**负载长度**短于条目格式（38 字节）而**偏移仍落在页内**时，`check_tree_page` 的叶页分支只做槽边界校验 ⇒ `violations` 为空 ⇒ 读 / 写路径的闸门放行 ⇒ `get_key` / `get_row_id` 因长度不足返回 `None` ⇒ 该条目被当作「没有这个条目」**静默跳过**（点查返回「查不到该行」⇒ 静默错读）；释放路径同样不报出、不跳过该页。
- Required behavior:
  1. 读 / 写路径：真实入口的查询在损坏叶页上 SHALL 返回**明确的错误**（点名损坏的页与该索引树），SHALL NOT panic、SHALL NOT abort、SHALL NOT 静默跳过该条目。
  2. 释放路径：真实 CLI 入口的 `DROP INDEX`（或 `DROP TABLE` / `TRUNCATE` 释放主键索引树）SHALL 成功（退出码 0）SHALL 把该页**计入损坏页计数并报出**（stderr 点名该页），SHALL **跳过**该页的不可读部分（其子树不被释放）。
  3. 未损坏库上的同样语句行为**零变化**（行数、命中数、退出码、stderr 无告警）。
- Preserve: 健康页的 `violations` 为空（不误报）；既有 `CorruptedIndexPage` 的错误文案形态（`corrupted index page {page_id}` + `(index root page {index_root_page_id})`）；释放路径的 `[index-release] index tree page {id} is damaged: {summary}` 报出形态与「跳过子树、保留已收集页」的语义；`free_page` 集合语义；既有域 `index-page-ownership` / `index-write-side-atomicity` / `free-page-lifecycle` / `ddl-rewrite-consistency`；磁盘格式（MS36 的 G1 / G2 / G3 兼容面）。
- Forbidden: 不得用「跳过读」之类的替身代替真实损坏（夹具必须真实改坏页字节的 length 字段）；不得改动 `SlottedPageRef::get_slot_data` / 访问器族 / 闸门 / 错误变体；不得把损坏页降级为告警 + 跳过（读 / 写路径 SHALL NOT 跳过条目）；不得改 `check_tree_page` 的**既有**判据（槽边界、页类型、兄弟链指针）；不得引入身份证明机制（hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明）、判定层或 timer 阈值行为判据；不得在测试框架外建判定脚本或封装命令；本任务**不改产品代码**。
- First test:
  - 在 `tests/index_read_bounded_read_test.rs` 新增夹具 helper `corrupt_slot_length(path: &Path, page: u32, slot_index: usize, new_len: u16)`：按 `page_file_offset(page, slot_offset_field_offset(slot_index) + 2)` 定位**槽目录项的 length 字段**（槽目录项布局 `[logical_id:2][offset:2][length:2]`，既有 helper 返回 offset 字段位置 = 起点 +2），写入 `new_len`。
  - 新增用例 `short_leaf_entry_makes_select_fail_with_a_named_error`：沿用既有 `seed_pk_tree` 夹具（100 行 ⇒ PK 树 = 内部根 + 两个叶），把**最左叶页**的**槽 0** 的 length 字段写成 `10`（偏移不动 ⇒ 页内合法），重开后执行 `SELECT * FROM t WHERE id = 1` ⇒ 期望 `assert_damaged_index_error(&resp, leftmost_leaf, pk_root)`（错误点名损坏的叶页与 PK 树根页）。
  - 新增用例 `short_leaf_entry_does_not_change_undamaged_reads`：同一夹具但不损坏 ⇒ `SELECT` 行数仍为 100、`INSERT` 仍成功。
  - 在 `tests/release_path_bounded_read_test.rs` 新增用例 `drop_index_cli_reports_the_short_leaf_entry`：构造一个**小表**的二级索引（其索引树**根页即叶页**——须在夹具中断言该页的页类型字节为 `LEAF_NODE`，与既有 `seed_pk_tree` 断言 `INTERNAL_NODE` 同法），把该叶页槽 0 的 length 字段写成 `10`，再走**真实 CLI 入口**执行 `DROP INDEX idx_v` ⇒ 期望退出码 0 且 stderr 含 `index tree page {sec_root} is damaged: 1 violation`。
  - 变更前运行 `cargo test --test index_read_bounded_read_test -- --test-threads=1` 与 `cargo test --test release_path_bounded_read_test -- --test-threads=1`，预期 **RED**：读 / 写路径用例因「查询返回 0 行而非点名错误」失败（**非** panic）；释放路径用例因「stderr 未报出该损坏页」失败；两条未损坏对照用例在 RED 阶段即通过。
- GREEN condition: T2 修复后同两条命令全部 passed / 0 failed；读 / 写路径用例的判定**不是**「不 panic」而已——须断言错误文案同时点名损坏页与索引树根页；释放路径用例须断言退出码 0 **且** stderr 含该页的报出。
- Verification: `cargo test --test index_read_bounded_read_test -- --test-threads=1`；`cargo test --test release_path_bounded_read_test -- --test-threads=1`（T2 完成后跑，RED 阶段跑 T1 的两条命令）。
- Stop when: 夹具无法在真实文件库上稳定命中（短叶条目未被实际扫描到）⇒ **更换损坏点**（落在该语句真正读取的叶页上），而不是放宽断言；若二级索引的根页在该夹具下**不是**叶页 ⇒ 换用更小的表规模使索引树为单叶，仍不得放宽断言；若修复需要改动访问器族 / 闸门 / 错误变体 / `get_slot_data` 签名 ⇒ 回到 Plan。

### T2: `check_tree_page` 叶页分支补条目长度判据

- Requirement/Scenario: 同 T1（实现 T1 的 RED）。
- Depends on: T1
- Targets: `src/storage/integrity.rs`（`check_tree_page` 的 `LEAF_NODE` 分支 + 顶部 `RowId` 导入）。
- Current behavior: 叶页分支丢弃 `validate_slots` 的返回值 ⇒ 只做槽边界校验 ⇒ 短叶条目（长度 < 38、偏移页内）不判为损坏 ⇒ 读 / 写路径静默跳过该条目、释放路径不报出也不跳过该页。
- Required behavior: ①叶页分支消费 `validate_slots` 的返回值并对每个页内合法槽补条目长度判据（`< MAX_KEY_LEN + RowId::SIZE` ⇒ 记 violation，文案与内部页分支同型）；②读 / 写路径经既有闸门给出点名损坏页与索引树根页的明确错误；③释放路径经既有 `collect_all_pages` 计数 + 报出 + 跳过；④健康叶页的 `violations` 仍为空 ⇒ 未损坏路径行为逐条不变。
- Preserve: 内部页分支的既有判据与文案；`check_tree_page` 的返回结构（`children` / `invalid_type` / `violations`）与调用方（`node.rs` 的闸门、`index_manager.rs` 的释放遍历、`table_manager.rs` 的重建面）；`violation_summary` 的汇总形态；`TreePageCheck` 的可见性（`pub(crate)`）；`StorageError::CorruptedIndexPage` / `InvalidPageType` 变体；磁盘格式；`free_page` 集合语义。
- Forbidden: 不得改 `validate_slots` 的既有判据（槽边界、槽数越界、自由空间偏移）；不得改叶页的孩子语义（兄弟链指针 `header.next_page_id`）；不得新增错误变体或新增报出通道；不得改访问器族 / 闸门 / 错误文案形态；不得改磁盘格式；不得引入身份证明机制、判定层或 timer 阈值行为判据；不得夹带 ISS27 / ISS25 / ISS28 / ISS29 / ISS30 / ISS31 / ISS32 / ISS33 的任何改动。
- First test: 沿用 T1 的用例（先 RED 后 GREEN）。
- GREEN condition: T1 的两条命令全部 passed / 0 failed；既有索引与释放用例保持通过；健康库上的读 / 写 / 释放行为与修复前逐条一致（stderr 无新增告警）。
- Verification: `cargo test --test index_read_bounded_read_test -- --test-threads=1`；`cargo test --test release_path_bounded_read_test -- --test-threads=1`；`cargo test --test index_manager_test -- --test-threads=1`；`cargo test --test secondary_index_test -- --test-threads=1`；`cargo test --test index_page_ownership_test -- --test-threads=1`；`cargo test --test index_page_ownership_stack_overflow_test -- --test-threads=1`；`cargo test --lib -- --test-threads=1`（本 Cycle 改 `src/storage/integrity.rs` ⇒ 覆盖该文件的既有 lib 结论失效，必须重跑）。全部以 `--test-threads=1` 串行执行；任一命令 failed > 0 ⇒ 本 Cycle 未完成。
- Stop when: 判据扩展导致**健康页误报**（`violations` 在健康库上非空）⇒ 回到 Plan；修复需要改磁盘格式 / 改访问器族 / 改错误变体 ⇒ 回到 Plan；发现 `check_tree_page` 的调用方对叶页 `violations` 有**超出「报出 + 跳过」的处置依赖** ⇒ 回到 Plan。

**Invariants**

- **读 / 写路径 SHALL NOT 跳过条目**：未通过页级判据的索引树页 ⇒ 明确的错误 ⇒ 语句整体失败；**不**是「查不到该行」、**不**是「当作没有这个条目继续」。
- **任何路径 SHALL NOT 因短叶条目 panic / abort**：短负载在页内 ⇒ 不越界；有界读是最后一道保障。
- **两条处置方向相反且各自成立**：读 / 写路径 = **明确错误**；释放路径 = **计数 + 报出 + 跳过**（检测到损坏 ≠ 忽略损坏）。
- **健康页零误报**：叶页的页内合法槽负载长度恒为 38 ⇒ 健康页 `violations` 为空 ⇒ 未损坏路径行为**零变化**。
- **判据对称**：叶页与内部页的条目长度判据**同型**（同一条 violation 通道 + 同一条汇总通道）⇒ 不新增错误形态、不新增报出通道。
- `free_page` / `complete_free_release` / `promote_free_releases` 的集合语义不变；磁盘格式不变（MS36 的 G1 / G2 / G3 兼容面不变）；既有域 `index-page-ownership` / `index-write-side-atomicity` / `free-page-lifecycle` / `ddl-rewrite-consistency` 不变。
- 不得引入身份证明机制（hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明）、判定层或 timer 阈值行为判据；判定一律在 `cargo test` 内。

**Non-goals**

- 其余页型的无界读点（ISS27；数据页 / 目录页 / 恢复面 / 缓冲池 / executor 与事务调用点）——Iteration 001 / 003 / 004。
- DDL 提交点统一模型（ISS25 / ISS28 / ISS29）、写盘失败状态（ISS30 / ISS31）、dump / restore 合同面（ISS32 / ISS33）、语料面（ISS34 / I045 / I064）。
- IRBR 的 Requirement 与 Scenario **标题措辞**（ISS34）——本 Cycle 只扩展判据覆盖面与新增 Scenario；标题统一在 Iteration 008 以**交接项**（交 Maintainer 与增量合并同批应用）完成。
- 损坏页的**修复**、完整性扫描的诊断分级、`SlottedPageRef::get_slot_data` 的签名改造、索引名注入 `IndexManager`、磁盘格式、`free_page` 集合语义。
- ISS23 的规模面（本 change 内不使用 ≥ 300 行的表规模）。

**Acceptance**

1. **短叶条目在读 / 写路径给出明确的错误**：真实文件库 + 真实 `Database::open` + 真实 `SELECT`（点查 `WHERE id = 1`）；损坏点 = **最左叶页**的槽 0 的**负载长度**（10 字节，偏移页内）；语句 SHALL 返回 `Response::Error`，文案同时含 `corrupted index page {leftmost_leaf}` 与 `(index root page {pk_root})`；SHALL NOT panic、SHALL NOT abort、SHALL NOT 返回「查不到该行」。映射：IRBR R1 Scenario「叶页条目长度过短的索引树页给出明确的错误」、T1、T2。
2. **短叶条目在释放路径被计数 + 报出 + 跳过**：真实文件库 + **真实 CLI 入口** `DROP INDEX`；索引树根页即叶页（夹具中断言页类型）；退出码 0；stderr 含 `index tree page {sec_root} is damaged: 1 violation`；该页的子树不被释放（既有语义）。映射：RPBR R1 Scenario「叶页条目长度过短的索引树页被计数、报出并跳过」、T1、T2。
3. **未损坏路径零变化**：健康库上 `SELECT` / `INSERT` / `DELETE` / `UPDATE` 的命中数、行集合与写入结果不变；释放路径的 stderr **无**新增告警；既有索引与释放用例全部通过。映射：IRBR R1 Scenario「未损坏路径的读 / 写行为零变化」、T2。
4. **判别力与零判定层**：读 / 写路径用例的 RED 形态是「查询返回 0 行而非点名错误」（**不是** panic，也不是任何其它失败形态）；释放路径用例的 RED 形态是「stderr 未报出该损坏页」；未损坏对照用例在 RED 阶段即通过；损坏夹具**真实改坏页字节**（length 字段），无替身；判定全部落在 `cargo test` 内，无身份证明机制、无框架外判定层、无 timer 阈值行为判据。映射：T1 的 First test、T2。
5. **判据对称与既有形态保持**：叶页与内部页的条目长度判据同型；`CorruptedIndexPage` 与 `[index-release] … is damaged: …` 两条既有报出形态逐字不变；`check_tree_page` 的返回结构与调用方不变。映射：T2。

**Verification**

- 行为类 scenario（1–4）走**真实入口**：真实文件库 + 真实 `Database::open` + 真实 SQL；释放路径走**真实 CLI 入口**（`rtsql` 二进制）；损坏通过**按文件偏移改写页字节**注入（真实损坏，不是替身）。
- 判定命令：`cargo test --test index_read_bounded_read_test -- --test-threads=1` ⇒ 0 failed（scenario 1、3、4）；`cargo test --test release_path_bounded_read_test -- --test-threads=1` ⇒ 0 failed（scenario 2、3）。
- 回归命令：`cargo test --test index_manager_test -- --test-threads=1`、`cargo test --test secondary_index_test -- --test-threads=1`、`cargo test --test index_page_ownership_test -- --test-threads=1`、`cargo test --test index_page_ownership_stack_overflow_test -- --test-threads=1`、`cargo test --lib -- --test-threads=1` ⇒ 各自 0 failed（scenario 3、5）。
- 全部命令以 `--test-threads=1` 串行执行；判定只看 cargo 自己的退出码与原生输出（允许用一步通用管道读计数，管道只做计数不做判定）。
- **本 Iteration 不跑全量回归**（本 change 一次预算，归 Iteration 008 / T16，M19 分批串行纪律）。
- 无人工验证项；无身份证明机制；无框架外判定层。

**Gate 2 Readiness**

- PASS——需求与场景：`specs/index-read-bounded-read/spec.md` 与 `specs/release-path-bounded-read/spec.md`（本 change 的增量域）的 R1 与新增 Scenario + 「未损坏路径零变化」Scenario 已逐条映射到 T1 / T2 与 Acceptance 1–5（BLOCKED: 无）。
- PASS——调查：叶 / 内两支的判据不对称、`validate_slots` 的返回与语义、叶条目格式（38 字节）、闸门与错误变体、访问器的有界读与「短负载 ⇒ None ⇒ 静默跳过」机制、释放路径的报出通道、既有夹具与槽目录项字节布局，均已在 Investigation Facts 中落到**当前工作区的行号**（BLOCKED: 无）。
- PASS——设计闭合：唯一落点（`check_tree_page` 的叶页分支）、判据的同型文案、两条相反处置由同一判据达成、健康页不误报的推导，均已定死，Act 不需再决定接口或错误语义（BLOCKED: 无）。
- PASS——任务分轮：T1（RED）→ T2（实现），两轮可独立验证与诊断（BLOCKED: 无）。
- PASS——追踪：Cycle RTM（见下）与 change `tasks.md` 的 RTM 行一致（T1 / T2 ↔ 增量域 `index-read-bounded-read` + `release-path-bounded-read`）（BLOCKED: 无）。
- PASS——当前轮验证：T1 的 RED 命令与其预期失败形态已写明；T2 的 GREEN 判据与回归清单已写明；Persisted Evidence = `none`（全部结论可低成本重跑）（BLOCKED: 无）。
- 无 BLOCKED / SKIPPED 项。⇒ **Gate 2 通过**，Plan Context 状态置 `ready`。

**Cycle RTM**

| Requirement / Scenario | Task | First Test（计划名） | Status |
|---|---|---|---|
| IRBR R1「未通过页级判据的索引树页在读 / 写路径上给出明确的错误」Scenario「叶页条目长度过短的索引树页给出明确的错误」 | T1, T2 | `tests/index_read_bounded_read_test.rs::short_leaf_entry_makes_select_fail_with_a_named_error`（计划名；Act 可改名并记录实际名） | Covered |
| RPBR R1「释放路径读取页内容时有边界检查」Scenario「叶页条目长度过短的索引树页被计数、报出并跳过」 | T1, T2 | `tests/release_path_bounded_read_test.rs::drop_index_cli_reports_the_short_leaf_entry`（计划名） | Covered |
| IRBR R1 Scenario「未损坏路径的读 / 写行为零变化」 | T2 | `tests/index_read_bounded_read_test.rs::short_leaf_entry_does_not_change_undamaged_reads`（计划名） | Covered |

**Persisted Evidence**

- Mode: none

全部结论可低成本重跑（两个测试目标 + 五个既有测试目标 + `cargo test --lib`），命令与结果写入 Act Response 即可承载；不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ 不创建 `evidence/` 目录。

**Risks and Notes**

- **健康页误报是本 Cycle 的头号风险**：判据一旦过宽（例如误用 `MAX_KEY_LEN` 而非 `MAX_KEY_LEN + RowId::SIZE`），健康叶页会被判为损坏 ⇒ 全部索引读 / 写路径报错。⇒ T2 的 Stop when 明确把「健康页误报」列为回 Plan 条件；验收以「未损坏对照用例 + 既有索引用例全通过」为准。
- **叶条目的真实长度是 38 而不是 36**：`MAX_KEY_LEN`（32）+ `RowId::SIZE`（6）。内部页条目是 `MAX_KEY_LEN + 4`（子页号 4 字节）⇒ 两者**不可互换**；误用 36 会让合法的叶条目（38 ≥ 36）仍然通过，但**短条目（10 < 36）也会被抓住** ⇒ 两种取值都能抓住本夹具的损坏，但 38 才是与叶条目格式一致的判据 ⇒ 验收以 38 为准（Act 须在 Response 中记录实际取值与理由）。
- **损坏点必须落在该语句真正读取的叶页上**：点查 `WHERE id = 1` 下降到**最左叶**（`seed_pk_tree` 返回它）⇒ 损坏点选在它的槽 0。既有夹具已用「根页类型字节」断言前置条件 ⇒ 新夹具沿用同法。
- **释放路径的夹具须保证索引树根页就是叶页**：小表 ⇒ 二级索引为单叶；夹具须以页类型字节断言该前置（否则长度损坏会落在内部页分支上，**RED 形态不成立**——内部页分支已有长度判据 ⇒ 用例会直接通过 ⇒ 失去判别力）。
- **`violation_summary` 的计数语义**：报出文案为 `1 violation: slot 0 is too short for a leaf-node entry (10 bytes, need 38)` ⇒ 既有断言形态 `index tree page {id} is damaged: 1 violation` 仍成立（`contains` 匹配）。
- **重建面**：`src/storage/data/table_manager.rs` 的 `collect_tree_pages` 同样消费 `check_tree_page` ⇒ 判据扩展会改变**重建面**对短叶条目的处置（由「当作健康页」变为「按损坏页处置」）⇒ 这是**期望行为**（重建面不得把损坏页当健康页照常使用），但 Act 须在 Response 中记录该连带面并确认 `free_page_lifecycle_test` 仍通过。
- **规模口径**：用例表规模 **≪ 300**（ISS23 约束）。
- **不改磁盘格式**（保 MS36 的 G1 / G2 / G3 兼容面）；本 Cycle 不改 `free_page` 集合语义。

**后续任务边界**

- **授权边界**：无。本 Cycle 的两个任务（T1 / T2）均在已批准范围内；Gate 1 / Gate 2 已由 goal 授权覆盖。
- **能力边界**：无。
- **停止边界**：本 Cycle 完成且 Plan Review 判定 `accepted` ⇒ 展开 **Iteration 001**（其余页型的有界读，ISS27 的数据页组：T3），其 Plan Context 在该 Iteration 展开时另行撰写。**（2026-10-25 结构修正见 `design.md` D14：T3 落 Iteration 001、T4 落 Iteration 003、T5 落 Iteration 004，各自独立成 Iteration。**2026-10-25 二次结构修正见 `design.md` D15：写路径组（T18 / T19）插入为 Iteration 003 ⇒ T4 落 **Iteration 004**、T5 落 **Iteration 005**。**）**
- **不在本 Cycle、需在后续 Iteration 处理的事项**：ISS27（Iteration 001 / 003 / 004）、ISS25 / ISS28 / ISS29（Iteration 005）、ISS30 / ISS31（Iteration 006）、ISS32 / ISS33（Iteration 007）、ISS34 / I045 / I064 与收尾全量（Iteration 008）。
- **（2026-10-25 二次结构修正见 `design.md` D15：上列迭代号依次顺延一位 —— ISS27 → **001 / 004 / 005**、ISS25 / ISS28 / ISS29 → **006**、ISS30 / ISS31 → **007**、ISS32 / ISS33 → **008**、ISS34 / I045 / I064 与收尾全量 → **009**；写路径组（T18 / T19）另落在 **Iteration 003**。）**
- **交 Maintainer / Recorder 的事项**：ISS26 在本 change 验收并关闭（由 Recorder 按用户指令执行）；本 Cycle 不登记 Issue、不改全局状态、不合并规格。

## Act Response

- Status: reported
- Cycle: `000-initial`（initial）｜Iteration: `000-index-leaf-entry-criterion`（ISS26）
- 实施日期：2026-10-25｜工作区 HEAD `e9cb9d3`（**未 commit**）

### 1. 实际改动

#### 1.1 产品代码（唯一落点：`src/storage/integrity.rs`）

| 符号 | 改动 |
|---|---|
| `check_tree_page` 的 `LEAF_NODE` 分支 | 消费 `validate_slots` 的返回值（`let _ = validate_slots(...)` → `if let Some(slots) = validate_slots(data, &header, &mut violations)`），对每个**页内合法槽**补条目长度判据：`< MAX_KEY_LEN + RowId::SIZE` ⇒ `violations.push(format!("slot {i} is too short for a leaf-node entry ({} bytes, need {})", end - start, MAX_KEY_LEN + RowId::SIZE))` |
| 顶部导入（`:33`） | `use crate::storage::page_format::{Slot, SlottedPageHeader, SlottedPageRef};` → `{RowId, Slot, SlottedPageHeader, SlottedPageRef}`（`MAX_KEY_LEN` 已在 `:32`，不重复导入） |
| `check_tree_page` 文档注释 | 叶页分支由「只做槽边界校验」改为「槽边界 + 条目长度校验」 |

- **判据取值 = 38**：`MAX_KEY_LEN`(32) + `RowId::SIZE`(6)，与叶条目格式 `[Key][RowId]` 一致；内部页条目是 `MAX_KEY_LEN + 4`(=36)，两者**不可互换**。
- **判据同型**：与内部页分支共用同一条 `violations` 通道、同一条 `violation_summary` 汇总通道 ⇒ 读 / 写路径的 `CorruptedIndexPage` 与释放路径的 `[index-release] … is damaged: …` 两条既有报出形态**逐字未变**，未新增错误变体或报出通道。
- **未触碰**：`src/storage/btree/node.rs`（访问器族 + 闸门）、`btree.rs`、`index_manager.rs`、`src/storage/error.rs`、`src/storage/page_format/*`、`src/storage/data/table_manager.rs`、目录与页格式、`free_page` 集合语义、磁盘格式。

#### 1.2 测试（先行用例 + 损坏夹具 helper）

`tests/index_read_bounded_read_test.rs`

- `slot_length_field_offset(slot_index)`（新）：槽目录项 **length 字段**位置 = 槽目录项起点 + 4（既有 `slot_offset_field_offset` 返回 offset 字段 = 起点 +2；布局 `[logical_id:2][offset:2][length:2]`）。
- `corrupt_slot_length(path, page, slot_index, new_len)`（新）：按文件偏移改写 length 字段；**偏移不动** ⇒ 负载仍落在页内 ⇒ 损坏形态是「负载短于条目格式」而非「越界槽」（真实改坏页字节，非替身）。
- `short_leaf_entry_makes_select_fail_with_a_named_error`（新用例）：`seed_pk_tree`（100 行 ⇒ 内部根 + 两个叶）⇒ 把**最左叶页槽 0** 的 length 写成 10 ⇒ `SELECT * FROM t WHERE id = 1` 必须是点名损坏页与索引树根页的错误。
- `short_leaf_entry_does_not_change_undamaged_reads`（新用例）：同一夹具不损坏 ⇒ `SELECT` 100 行 / 点查 1 行 / `INSERT` 影响 1 行。
- 模块文档补「短叶条目」形态一段。

`tests/release_path_bounded_read_test.rs`

- `slot_length_field_offset` + `corrupt_slot_length`（新，同法）。
- `seed_small_index_table(path)`（新夹具）：**5 行**小表 ⇒ 二级索引树为**单叶**（根页即叶页）；用例内以页类型字节断言该前置（`LEAF_NODE`）——否则长度损坏会落在**已有长度判据**的内部页分支上，用例会直接通过而失去判别力。
- `drop_index_cli_reports_the_short_leaf_entry`（新用例）：真实 CLI 入口 `DROP INDEX idx_v` ⇒ 退出码 0 + stderr 点名该损坏页。
- `drop_index_cli_on_an_undamaged_leaf_tree_reports_nothing`（新对照用例）：健康叶根索引树 ⇒ 退出码 0 且 stderr 不含 `is damaged`（Acceptance 3 的「释放路径 stderr 无新增告警」此前**无直接见证**）。
- `use rtsql::storage::{…, LEAF_NODE}` 导入 + 模块文档补一段。

### 2. 与计划的偏差

1. **新增 1 个对照用例**（计划只列了 3 个）：Acceptance 3 明确要求「释放路径的 stderr **无**新增告警」，既有用例只能**间接**见证（健康叶页一旦误报 ⇒ 子树被跳过 ⇒ 释放页集合不完整），没有直接断言 stderr ⇒ 补 `drop_index_cli_on_an_undamaged_leaf_tree_reports_nothing`。该用例在 RED 与 GREEN 两态下都通过（健康叶页两态均无 violations）⇒ 属**对照**而非判别用例，不影响判别力结论。
2. **两个测试文件的模块文档各补一段**（计划未列）：模块文档原本逐条列举见证矩阵，新增形态不同步会让文档与文件内容不一致 ⇒ 补形态说明（纯注释，无行为影响）。
3. **`seed_small_index_table` 行数 = 5**（计划只要求「小表 ⇒ 单叶」）：叶容量 92 ⇒ 5 行不分裂 ⇒ 根页即叶页（用例内页类型断言通过）；ISS23 的「不用 ≥ 300 行规模」约束满足。
4. **未偏差项**：损坏长度取 **10 字节**（计划钉死值）；判据阈值取 **38**（与叶条目格式一致，**非**内部页的 36）——按计划执行。

### 3. 自检发现（Gate 3 / Gate 4）

- **Spec compliance**：IRBR R1 的「叶页条目长度过短的索引树页给出明确的错误」与「未损坏路径的读 / 写行为零变化」两个 Scenario、RPBR R1 的「叶页条目长度过短的索引树页被计数、报出并跳过」Scenario 均由新用例**逐字断言**（错误文案同时点名损坏页与索引树根页；stderr 点名该页；退出码 0）；计划列出的文件、符号、调用者与错误路径均已覆盖；既有 `CorruptedIndexPage` / `[index-release]` 两条文案形态、`check_tree_page` 的返回结构与三个调用方（`node.rs` 闸门 / `index_manager.rs` 释放遍历 / `table_manager.rs` 重建面）均未变 ⇒ 通过。
- **Code quality**：diff 仅 3 个文件且均为计划列出的文件与符号，无计划外修改；健康页 `violations` 恒空 ⇒ 不误报；损坏夹具**真实改坏页字节**（length 字段），无替身；判定全部落在 `cargo test` 内 ⇒ 无身份证明机制、无框架外判定层、无 timer 阈值行为判据；无新增编译告警，clippy 在本人触碰的三个文件上 0 告警（见 §5）。
- **健康页零误报（计划标注的头号风险）**：叶条目写入路径（`LeafNode::insert` / `insert_simple` / `shift_slots_right`）写出的负载恒为 38 字节 ⇒ 健康叶页 `violations` 恒空；实测：未损坏对照用例通过 + 4 个索引回归目标 + `cargo test --lib`（420）全通过 + 释放面「stderr 无告警」对照用例通过。
- **连带面（计划 Risks 已提示，此处确认）**：`src/storage/data/table_manager.rs::collect_tree_pages`（重建面）与 `src/storage/integrity.rs` 的 `check_integrity`（`rtsql check` 面）同样消费 `check_tree_page` ⇒ 短叶条目在重建面由「当作健康页」变为「按损坏页处置」（重建面不再把损坏页当健康页照常使用，**期望行为**），在 `check` 面由「无发现」变为一条 error 发现（完整性扫描的诊断分级仍在本 change 范围外）。已实测 `free_page_lifecycle_test`(12) / `check_command_test`(11) / `drop_table_free_test`(6) 全部通过。
- **释放面语义确认**：`collect_all_pages` 先把页压入释放集合、再判损坏 ⇒ 被判损坏的叶根页**本身仍被释放**、其子树不被释放（既有语义，未变）。
- **未发现计划外的实质缺陷**；无未解决的 Critical / Important 问题；无遗留 Minor 问题（§5 的工具链基线是环境事实，非本 Cycle 的代码缺陷）。

### 4. 验证命令与实际结果

**T1（RED，变更前）**

| 命令 | 实际结果 |
|---|---|
| `cargo test --test index_read_bounded_read_test -- --test-threads=1` | 6 passed / **1 failed** —— `short_leaf_entry_makes_select_fail_with_a_named_error` 失败于 `a damaged index page must fail the statement with a named error, got QueryResult { rows: [] }` ⇒ **RED 形态 = 返回 0 行而非点名错误**（**不是** panic）；同批 `short_leaf_entry_does_not_change_undamaged_reads` **已通过** |
| `cargo test --test release_path_bounded_read_test -- --test-threads=1` | 7 passed / **1 failed** —— `drop_index_cli_reports_the_short_leaf_entry` 失败于 `the damaged leaf page must be counted and reported: stderr=`（stderr 为空）⇒ **RED 形态 = stderr 未报出该损坏页**；其前置的退出码断言（= 0）先于它通过 ⇒ 语句未 abort |

RED 阶段既有 6 / 7 个用例全部保持通过 ⇒ 判据扩展未破坏既有行为。

**T2（GREEN + 定向回归，全部串行 `--test-threads=1`）**

| 命令 | 实际结果 |
|---|---|
| `cargo test --test index_read_bounded_read_test -- --test-threads=1` | **ok. 7 passed; 0 failed**（含 2 个新用例；末次运行覆盖模块文档改动后的最终文件态） |
| `cargo test --test release_path_bounded_read_test -- --test-threads=1` | **ok. 9 passed; 0 failed**（含 3 个新用例；末次运行含新增对照用例） |
| `cargo test --test index_manager_test -- --test-threads=1` | ok. 3 passed; 0 failed |
| `cargo test --test secondary_index_test -- --test-threads=1` | ok. 27 passed; 0 failed |
| `cargo test --test index_page_ownership_test -- --test-threads=1` | ok. 5 passed; 0 failed |
| `cargo test --test index_page_ownership_stack_overflow_test -- --test-threads=1` | ok. 1 passed; 0 failed |
| `cargo test --lib -- --test-threads=1` | **ok. 420 passed; 0 failed**（本 Cycle 改了 `src/storage/integrity.rs` ⇒ 覆盖该文件的既有 lib 结论失效，已重跑） |
| `cargo test --test free_page_lifecycle_test -- --test-threads=1` | ok. 12 passed; 0 failed（连带面：重建面） |
| `cargo test --test check_command_test -- --test-threads=1` | ok. 11 passed; 0 failed（连带面：`rtsql check`） |
| `cargo test --test drop_table_free_test -- --test-threads=1` | ok. 6 passed; 0 failed |

**错误 / 报出文案**（由代码格式串与 `violation_summary` 决定，断言命中其前缀 `… is damaged: 1 violation` / `corrupted index page {id}`）：

- 读 / 写路径：`corrupted index page {leftmost_leaf} (index root page {pk_root}): 1 violation: slot 0 is too short for a leaf-node entry (10 bytes, need 38)`
- 释放路径：`[index-release] index tree page {sec_root} is damaged: 1 violation: slot 0 is too short for a leaf-node entry (10 bytes, need 38)`

**未跑全量**：本 Iteration 按 M19 分批纪律**不跑全量**——本 change 的唯一一次全量在 Iteration 008 / T16（143 目标 / 14 批）。

### 5. 工具链基线事实（环境记录，非本 Cycle 引入）

`cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 在 **HEAD 即失败**（既有基线漂移，与本 Cycle 的改动无关）：

- fmt 漂移分布在 ~15 个本 Cycle **未触碰**的文件（`src/storage/btree/btree.rs`、`src/storage/data/table_manager.rs`、`src/storage/data/mod.rs`、`tests/ddl_rewrite_consistency_test.rs`、`tests/free_page_lifecycle_test.rs`、`tests/index_page_ownership_test.rs` 等）；本 Cycle 触碰的三个文件中，`src/storage/integrity.rs` 唯一被标记的行（`:478` 的 `violation_summary`）是**既有代码**，新判据所在区域**无**标记。
- `tests/release_path_bounded_read_test.rs` 中新增代码唯一被 fmt 标记的是一处断言行，且与**相邻既有同型断言**（同文件第 394 行）逐字同形 ⇒ 不引入新的书写形态；只改这一行反而会让同一文件出现两种形态（全仓统一重排属范围外工作，本 Cycle 不做）。
- clippy 的 17 条 lib 错误全在 `src/transaction/manager.rs`、`src/storage/async_storage.rs`、`src/storage/buffer_pool.rs`、`src/storage/file_storage.rs`、`src/network/protocol.rs`、`src/executor/executor_trait.rs`、`src/cli/lifecycle.rs`、`src/pipeline.rs`、`src/parser/planner/ddl_dml.rs`；以不 deny 的 `cargo clippy --all-targets` 复查：本人触碰的三个文件 **0 条告警**。

### 6. Persisted Evidence

`none required`（Plan Context 的 `Persisted Evidence: Mode: none`）：全部结论可低成本重跑（2 个测试目标 + 7 个回归目标 + `cargo test --lib`），Act Response 足以承载；不创建 `evidence/` 目录。

### 7. Experience Candidates

- **Issue 候选（当前 change 范围外，只报告不登记）**：本仓在**当前工具链**（rustfmt 1.10.0 / clippy 1.99）下 `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 于 HEAD 即失败 ⇒ README「构建与测试」段列出的这两个门禁目前**不可满足**（疑为工具链版本漂移引入：归档 change 记录中的「clippy/fmt/validate 全 0/PASS」与当前实测不符）。影响面：本 change 收尾（Iteration 008 / T16 与 Maintainer 收尾的「clippy/fmt 干净」口径）。证据：本节 §5 的实测输出。处置建议：由用户在收尾时裁定（固定门禁口径 or 另开一次范围明确的重排 change）；本 Cycle **不做**（范围外，且与 MS37 的缺陷清账目标无关）。
- Runbook：无（无新的、已端到端验证且可重复或高风险的操作路径）。

### 8. 未解决问题

无。（ISS26 的台账关闭由 Recorder 按用户指令执行；本 Cycle 不登记 Issue、不改全局状态、不合并规格。）

### 9. 本 Cycle 边界

未归档 change；未同步全局文档（`.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆均未动）；未创建下一 Cycle 或下一 Iteration；未 `git commit`（既有未提交修改 `.agents/docs/tasks.md` 保持原样，本 Cycle 只在 change 产物目录内新增文件）。

## Plan Review

- Review Result: accepted

**Findings**

（独立检查方法：独立阅读 `src/storage/integrity.rs` 与两个测试文件的完整 diff、逐条核对 Act Response 的声明与代码一致、核对 Acceptance 与本 change 的 delta spec 对应；沿用 Act 已报告且覆盖范围未变化的验证结论，未重复运行。）

- F1 实施与 Plan Context 一致（PASS）：`src/storage/integrity.rs` 的叶页分支与 Plan Context「Implementation Guidance」逐条对应——`validate_slots` 的返回值由 `let _ =` 改为 `if let Some(slots)` 并在 `Some` 分支内追加条目长度判据；判据为 `end - start < MAX_KEY_LEN + RowId::SIZE`，阈值 38 与 `src/storage/page_format/key.rs:4`（`MAX_KEY_LEN = 32`）+ `src/storage/page_format/row_id.rs:34`（`RowId::SIZE = 6`）一致；违规文本与内部节点分支同形（`slot {i} is too short for a leaf-node entry ({n} bytes, need 38)`）；`RowId` 以既有 `use crate::storage::page_format::{…}` 单行追加导入；函数文档注释同步更新。变更只落在 Plan Context 指定的唯一产品落点，无额外改动、无范围蔓延。
- F2 判据无假阳性（PASS）：叶页条目的写入侧恒为 38 字节（`src/storage/btree/node.rs` 的 `LeafNode::insert` / `insert_simple` / `shift_slots_right` 均以 `MAX_KEY_LEN + RowId::SIZE` 为条目长度），健康叶页 ⇒ `violations` 为空；`validate_slots`（`src/storage/integrity.rs`）以 `end = start + slot.length` 计算 ⇒ `end - start` 恒等于槽长度，无下溢（`length` 为 `u16`，`end ≥ start`）。
- F3 验收 1（读 / 写路径给出点名错误）成立：测试辅助 `assert_damaged_index_error` 同时断言**损坏页号**与**索引树根页号**（Act Response 的「0 行、不 panic」RED 形态与之相符——修复前是静默 0 行而非崩溃）；损坏页取左端叶页，命中的是「据索引树页字节决定取条目」的路径。释放路径 `IndexManager::collect_all_pages`（`src/storage/btree/index_manager.rs`）确实先 `result.push(page_id)` 再判损坏 ⇒ 损坏叶根本身仍被释放、其后代不被释放，与既有洞页策略一致（本次未改）。
- F4 验收 2（释放面计数 + 报出 + 跳过）成立：`tests/release_path_bounded_read_test.rs` 走真实 CLI 入口，断言退出码 0 且 stderr 含 `index tree page {sec_root} is damaged: 1 violation`，与 `collect_all_pages` 的实际打印形态（`integrity::violation_summary`）逐字同形。
- F5 验收 3（未损坏路径零变化）成立：新增对照用例 `drop_index_cli_on_an_undamaged_leaf_tree_reports_nothing` 直接见证「无新增 stderr 告警」；`short_leaf_entry_does_not_change_undamaged_reads` 见证读路径不变。该对照用例是 Act 相对 Plan Context 的**追加**（Plan 只要求「零变化」断言），属非实质偏差，方向更严，不构成偏离。
- F6 验收 4（判别力、无裁决层）成立：损坏通过**真实字节改写**（按页内偏移覆写槽目录项的长度字段）注入，无替身、无桩；全链未引入 Hash / 指纹 / revision pin / run-id / manifest / 日志 Hash 链 / 时间证明；判定完全落在 `cargo test` 内。
- F7 验收 5（判据与报告面同构）成立：叶页分支与内部节点分支共用同一条 `violations` 通道与同一条 `violation_summary` 汇总通道；既有两种报告形态（`CorruptedIndexPage` 读路径 / `[index-release] …` 释放路径）与 `TreePageCheck` 结构及其三个调用点均未改。
- F8 delta spec 对应（PASS）：本 change 的 `specs/index-read-bounded-read/spec.md` 新增场景「叶页条目长度过短的索引树页给出明确的错误」与 `specs/release-path-bounded-read/spec.md` 新增场景「叶页条目长度过短的索引树页被计数、报出并跳过」与实现逐字对应；既有 Requirement / 场景标题逐字保留（符合 ISS34 的交接约束）。
- F9 连带面与 Plan Context 风险预判一致：`check_tree_page` 的另外两个消费者——重建面 `table_manager.rs::collect_tree_pages` 与完整性扫描面 `integrity.rs::check_integrity`——现在同样把「叶页条目过短」判为损坏页，这是预期行为；Act 报告的旁证面（`free_page_lifecycle_test` 12/0、`check_command_test` 11/0、`drop_table_free_test` 6/0）与之相符。
- F10（Minor，非阻塞，保留不改）`tests/release_path_bounded_read_test.rs:425` 是本次新增行且被当前 `cargo fmt --check` 标记（rustfmt 1.10.0 要求把该 `assert!` 折成多行）。该行与同文件既有的第 397 行断言**逐字同形**，而那一行在 HEAD 上已被标记 ⇒ 属工具链版本漂移下的既有书写惯例，非本次引入的代码质量缺陷；该文件在 HEAD 上本就未过 `fmt --check`，本次改动不改变其门状态。重排该文件违反 Surgical Changes（全仓漂移涉及既有文件，与本 change 无关），故记录而不修。本 Cycle 触碰的另外两个文件（`src/storage/integrity.rs`、`tests/index_read_bounded_read_test.rs`）的新增行**均未**出现在 `fmt --check` 名单中。
- F11（Minor，非阻塞，保留不改）delta `specs/release-path-bounded-read/spec.md` 中「其子树不被释放」的括注，对**有子页的内部节点页**精确，对**叶根页**略松：既有洞页策略下被判损坏的页自身已先入释放集合（`index_manager.rs:654` 的 `result.push(page_id)` 先于 `:667` 的损坏判定），被跳过的是其后代页。该措辞与既有代码注释（`index_manager.rs:641-642`）同源，非本次引入，不影响行为与任何验收项；建议 Maintainer 收尾合并该 spec 时顺带对齐措辞。

**Acceptance Gaps**

无。验收 1–5 逐条成立（见 F3–F7），无阻塞项。

**Evidence**

- Act Response 的验证结论**沿用**（覆盖范围未变化，本 Cycle 只改动 `src/storage/integrity.rs`、`tests/index_read_bounded_read_test.rs`、`tests/release_path_bounded_read_test.rs` 三个文件）：RED 形态为读路径返回 `QueryResult { rows: [] }`（0 行、不 panic）、释放路径 stderr 为空、损坏对照失败、未损坏对照通过；GREEN / 回归 `index_read_bounded_read_test` 7/0、`release_path_bounded_read_test` 9/0、`index_manager_test` 3/0、`secondary_index_test` 27/0、`index_page_ownership_test` 5/0、`index_page_ownership_stack_overflow_test` 1/0、`cargo test --lib` 420/0；旁证面 `free_page_lifecycle_test` 12/0、`check_command_test` 11/0、`drop_table_free_test` 6/0；`openspec validate --strict` valid。
- 本次 Review **新增**的独立核对（只针对本 Cycle 触碰的文件，不重复 Act 已通过的定向回归）：
  - `cargo fmt --check`（全仓，rustfmt 1.10.0）：9 个文件被标记；本 Cycle 的三个触碰文件中，`src/storage/integrity.rs` 仅第 478 行（既有 `violation_summary`）、`tests/index_read_bounded_read_test.rs` 仅第 147/191/298/308/324 行（均为既有行，新增内容自第 330 行起）、`tests/release_path_bounded_read_test.rs` 第 23/98/219/268/357/373/394 行为既有行，第 425 行为本次新增行（见 F10）。
  - `cargo clippy --all-targets`：全仓 32 条 warning，本 Cycle 触碰的三个文件 **0 条**；lib 侧 17 条全部落在未触碰文件（`async_storage.rs` / `buffer_pool.rs` / `file_storage.rs` / `transaction/manager.rs`），test 侧落在 `dml_tombstone_abandon_window_test.rs` / `ddl_rewrite_consistency_test.rs` / `sql_differential_test.rs` / `secondary_index_equality_test.rs`。
  - 代码阅读核对：叶页条目写入长度恒为 38（`src/storage/btree/node.rs`）、`MAX_KEY_LEN = 32`、`RowId::SIZE = 6`、`validate_slots` 的 `end` 计算方式、`collect_all_pages` 的「先入释放集合再判损坏」顺序、`assert_damaged_index_error` 的双重断言内容。

**Follow-up Decision**

无需修复：F10 / F11 均为非阻塞 Minor finding，记录后不引入本 Cycle 的返工，也不改变验收结论。本 Cycle 不创建后继 Cycle。

**Iteration Plan Update**

Iteration 000 的 Iteration Plan 保持不变：任务 T1 / T2 状态 `done` 与实际完成一致，验收全部成立，无需调整。ISS27 的剩余部分（其余页型的有界读）按原计划在 Iteration 001（数据页组）/ 003（目录页组）/ 004（恢复面组）展开。**（2026-10-25 二次结构修正见 `design.md` D15：写路径组（T18 / T19）插入为 Iteration 003 ⇒ 上句的 003（目录页组）/ 004（恢复面组）现为 **004 / 005**。）**

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-10-25-ms37-defect-ledger-closure/iterations/001-data-page-bounded-read/000-initial.md`（Iteration 001 已展开，承载 ISS27 的数据页组：T3）
