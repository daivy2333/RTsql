# Iteration 001 / Cycle 000: 数据页组的有界读（读 / 写路径点名错误）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 既有域 `release-path-bounded-read`（RPBR：释放路径的有界读与「计数 + 报出 + 跳过」语义——**处置方向与读 / 写路径相反**）、既有域 `index-read-bounded-read`（IRBR：索引树读 / 写路径的有界读与「明确错误」语义——本 Cycle 是它在**数据页**上的同型扩展）、既有域 `ddl-rewrite-consistency`（改写窗口内的读拒绝）、既有域 `free-page-lifecycle`（释放集合语义）、既有域 `data-integrity-verification`（`rtsql check` 的损坏检出面）、既有域 `wal-recovery-replay-integrity`（K05：不可忠实重放的已提交记录 SHALL 报错，SHALL NOT 被静默丢弃）、`design.md` D7（ISS27 的按页型分组判定）、D10（验证预算）、D11（禁止事项）、`.agents/runbooks/m19-batched-full-regression.md`（M19 分批串行纪律）、`.agents/issues/ISS27-*.md`（台账条目）、Iteration 000 的 accepted Review（索引树叶条目长度已纳入页级判据）。
- Excluded scope: **目录页组**（T4）与**恢复面组**（T5）——后继 Iteration（003 / 004），本 Cycle 不碰；**索引树**的任何改动（`src/storage/btree/*`、`check_tree_page`——Iteration 000 已覆盖叶页条目长度，索引树访问器本就有界）；**目录页**（`src/storage/catalog.rs`）与**恢复面**（`src/wal/recovery.rs`）的读点；数据页的**页级判据**（`check_data_page`）与其释放 / 重建 / `rtsql check` 消费面（本 Cycle 只改读 / 写路径的取行点）；损坏页的**修复**；完整性扫描的诊断分级；dump / restore 合同面；磁盘格式；全量回归（整个 change 的收尾全量归 Iteration 008 / T16）。

**Objective**

数据页组（数据页 + 缓冲池 + executor / 事务调用点）的**越界槽**读取全部经由**有边界检查**的路径取得：越界槽的偏移或长度 SHALL NOT 造成越界切片、SHALL NOT 导致进程 abort。在**读 / 写路径**上，越界槽 SHALL 让语句以**点名该页与该槽**的明确错误结束（SHALL NOT 被当作「没有这一行」而静默跳过）；缓冲池的**布尔型页辅助** SHALL 降级为安全值（非「全可见」⇒ 走慢路径），错误由实际取行处抛出。未损坏路径的行为**零变化**。

**Background**

- 缺陷台账条目 **ISS27**（`.agents/issues/ISS27-*.md`，状态 `open`）：「存储层对页内容的读取存在**无范围检查**的切片（`SlottedPageRef::get_slot_data`）⇒ 损坏页上的读取造成越界切片 / 进程 abort」。本 change 的 `design.md` D7 已把 ISS27 拆成按页型分组的判定方向。
- Iteration 000 已完成 ISS26（索引树叶条目长度判据），其 Plan Review 为 `accepted`（见 `iterations/000-index-leaf-entry-criterion/000-initial.md`）。**Iteration 000 的产物是本 Cycle 的既有现场**：不回滚、不清理、不提交。
- **拆分信号裁决（Iteration 级，三组逐一裁定）**：change `tasks.md` 的 Iteration Plan 写明「若 001 的某一页型组需要**新的产品决策**（如目录页损坏是否允许打开库）⇒ 该组独立成 Iteration 并回 Plan 请求裁定」。本 goal 授权 Plan 按全局最优解直接裁定并记录理由。三组结论如下：
  - **T3 数据页组 ⇒ 不触发拆分信号**。D7 的方向（「读路径明确错误，点名页与槽」；缓冲池布尔辅助降级为安全值）与**已被验收的** `index-read-bounded-read` 语义同型，只是换到数据页上；数据页的**释放 / 重建 / 扫描面**早已经由既有的 `check_data_page` 页级判据有界（见 Current-State Evidence），无须新决策；逐点「越界 vs 过短」的既有语义差异是**实现细节**，由本 Cycle 在 Plan Context 中定稿，不构成方向性决策。
  - **T4 目录页组 ⇒ 不触发拆分信号（需要的是既有方向的确认 + 一处措辞细化，不是新方向）**。D7 已定「打开 / 查询路径明确错误、释放 / 重建路径沿用既有尽力而为与重建放弃语义」。本次 Plan 依 goal 授权按全局最优解**裁定**：「目录页损坏 ⇒ 该库 SHALL NOT 打开」——目录是「哪些页仍然存活」的根依据，没有它既无法给出可信的页清单（`rtsql check`）也无法确定恢复范围（`rtsql restore`）；跳过该目录行会让该表**静默消失**（其页成为孤儿，后续释放可能回收活页）⇒ 比打不开坏得多；打不开同时严格优于现状（今天是 panic / abort）。⇒ 该裁定与 D7 同向，**不新增方向**，故不拆分。**该裁定的可见后果已写入 delta spec**：目录损坏时该库的整个使用面（打开 / 完整性扫描 / 导出 / 模式查看）报出打开错误，而不是给出基于残缺目录的结论。
  - **T5 恢复面组 ⇒ 不触发拆分信号（需要的是把 D7 的单一方向细分为两类恢复读取，不改方向）**。本次 Plan 依 goal 授权裁定并记录：① **已提交记录重放类**（更新重放所需的旧行推导）⇒ 越界槽 SHALL **NOT** 跳过该记录（跳过会让不可忠实重放的已提交记录被静默丢弃，与既有 `wal-recovery-replay-integrity` K05 直接冲突）⇒ SHALL 以**明确的错误**结束恢复与打开；② **扫描类**恢复读取（重放前候选扫描 / 重放后索引重建 / 未提交事务标记）⇒ 跳过该槽、**计入恢复告警并报出**、其余记录继续重放（与这三处**既有的**「过短即跳过」容错同向，只是把该容错从「过短」扩到「越界」并要求报出）。⇒ 方向未变，仅把 D7 的单一方向按恢复类别写细，**不拆分**。
  - ⇒ **三组均留在 Iteration 001**，按 Iteration Plan 预排的三个 Cycle 推进：本 Cycle 000 = T3；T4 = 本 Iteration 的后继 Cycle；T5 = 再后继 Cycle。**（2026-10-25 结构修正见 `design.md` D14：承载面由「同一 Iteration 内的本地 Cycle 序列」改为「每组各自一个 Iteration」——本 Cycle 000 = T3 留在 Iteration 001；T17 落 Iteration 002；T4 落 Iteration 003；T5 落 Iteration 004。三组的**方向裁定**（上列两条 ⇒）不变。**2026-10-25 二次结构修正见 `design.md` D15：写路径组（T18 / T19）插入为 Iteration 003 ⇒ T4 落 **Iteration 004**、T5 落 **Iteration 005**。）**
  - ⇒ 上列裁定已同步进本 change 的 delta spec `specs/page-read-bounded-read/spec.md`（逐组表达），T4 / T5 的细化在其各自 Cycle 的 Plan Context 中再展开。

**Investigation Facts**

- Starting point:
  - 工作区 HEAD `e9cb9d3`（「save: 缺陷台账生命周期清理——六条已关闭 Issue…」）。
  - 工作区既有未提交修改：`.agents/docs/tasks.md`（MS37–MS40 路线批次）属**既有现场**，**不回滚、不清理、不提交**；本 change 的 Iteration 000 产物（`src/storage/integrity.rs`、`tests/index_read_bounded_read_test.rs`、`tests/release_path_bounded_read_test.rs`、change 产物目录）是**既有现场**的一部分，本 Cycle 在其上继续。
  - **前序全量终态已失效**（本 change 改动前即已失效）⇒ 不得作为依据；按 change 预算，收尾全量归 **Iteration 008 / T16**（M19 分批串行纪律）⇒ **本 Iteration 不跑全量**（本 Cycle 与 T4 / T5 均不跑）。
  - **前序定向结论的有效性裁定**：本 Cycle 将改动 `src/storage/page_format/slotted_page.rs`、`src/storage/error.rs`、`src/storage/data_page.rs`、`src/storage/data/table_manager.rs`、`src/storage/buffer_pool.rs`、`src/executor/data_scan.rs`、`src/executor/create_index.rs`、`src/transaction/manager.rs`、`src/pipeline.rs` ⇒ 覆盖到这些文件的既有结论**失效**（须重跑）；覆盖到 `src/storage/integrity.rs` / `src/storage/btree/*` / `src/storage/catalog.rs` / `src/wal/*` 的结论**不受影响**（本 Cycle 不改这些文件）。
  - Iteration 000 的验证结论（`index_read_bounded_read_test` 7/0、`release_path_bounded_read_test` 9/0、`index_manager_test` 3/0、`secondary_index_test` 27/0、`index_page_ownership_test` 5/0、`index_page_ownership_stack_overflow_test` 1/0、`cargo test --lib` 420/0、旁证面 `free_page_lifecycle_test` 12/0 / `check_command_test` 11/0 / `drop_table_free_test` 6/0）**沿用**（覆盖范围未变化），来源为 Iteration 000 的 Act Response；其中 `cargo test --lib` 的覆盖面因本 Cycle 改动 `page_format` / `error` / `table_manager` 而**部分失效** ⇒ 本 Cycle 重跑 `cargo test --lib`。
- Current-State Evidence（Plan 已逐行确认，行号为**当前工作区**；Act 不需回读即可执行）:
  - **缺陷核心（无边界切片）**：`src/storage/page_format/slotted_page.rs:125` 的 `pub fn get_slot_data(&self, slot: &Slot) -> &'a [u8]`，函数体为
    ```rust
    let start = slot.offset as usize;
    let end = start + slot.length as usize;
    &self.data[start..end]
    ```
    ⇒ 槽偏移 / 长度越界时**直接 panic**（`range start index 65535 out of range for slice of length 4096`）⇒ 进程 abort。`src/storage/page_format/slotted_page.rs:195` 的 `SlottedPage`（可写变体）同名方法同形。
  - **槽与页的字节常量**：`Page::PAGE_SIZE = 4096`（`src/storage/page.rs:11`）；`SlottedPageHeader::SIZE = 16`（`src/storage/page_format/slotted_page.rs:16`）；`Slot::SIZE = 6` = `[logical_id:u16][offset:u16][length:u16]`（`:5-13`）；**槽目录从页尾向上生长**：槽目录项 i 的页内起点 = `PAGE_SIZE - (i+1) * Slot::SIZE`（`get_slot` 的实现，`src/storage/page_format/slotted_page.rs:95-110`），其中 **offset 字段在项起点 +2、length 字段在项起点 +4**；**数据区上界** `slot_area_start = PAGE_SIZE - slot_count * Slot::SIZE`。
  - **文件布局**：页 N 的文件偏移 = `HEADER_SIZE + N * PAGE_SIZE`，`HEADER_SIZE = 64`（`src/storage/file_header.rs:20`）；页类型字节在页首（`page_type`：叶 0x01 / 内 0x02 / 数据 0x03，见 `src/storage/btree/node.rs:8-9` 与 `src/storage/data_page.rs:46`）。
  - **索引树已有的有界读（本 Cycle 不改，作为判界形状的既有先例）**：`src/storage/btree/node.rs:21` 的 `slot_payload_in_bounds(header, slot)`——`start >= SlottedPageHeader::SIZE && end <= slot_area_start`（`end` 用 `checked_add`）；`src/storage/btree/node.rs:31` / `:43` 的 `bounded_slot_data_ref` / `bounded_slot_data_mut` 先判界再取数 ⇒ 索引树访问器**本就有界**，ISS27 的缺口不在索引树。
  - **数据页的页级判据（已存在，本 Cycle 不改）**：`src/storage/integrity.rs` 的 `check_data_page(data) -> DataPageCheck`，内部复用与索引树同源的 `validate_slots` ⇒ 越界槽被记为 violation。其三个消费者：
    - `src/storage/data/table_manager.rs:1736`（`collect_overflow_chain_pages`）——**释放路径**：计数 + 报出（`[overflow-release] data page N is damaged: …`）+ 跳过 ⇒ RPBR 既有语义，本 Cycle 不改；
    - `src/storage/data/table_manager.rs:1928`（`collect_data_chain_strict`）——**重建路径**：严格报错（`internal error: data page N of the persisted directory is damaged: …`），本 Cycle 不改；
    - `src/storage/integrity.rs:271`（`check_integrity`）——**`rtsql check` 扫描面**，本 Cycle 不改。
    ⇒ **数据页的释放 / 重建 / 扫描面已是有界读**；ISS27 在数据页上的缺口只落在**读 / 写路径的取行点**。
  - **21 处 `get_slot_data` 调用点（`src/` 内，按文件分组；Plan 已逐点核对）**：
    - 数据页组（T3）：`src/storage/data_page.rs:137`、`:229`；`src/storage/data/table_manager.rs:1003`、`:1257`、`:2762`；`src/executor/data_scan.rs:254`、`:472`；`src/executor/create_index.rs:135`；`src/transaction/manager.rs:570`；`src/storage/buffer_pool.rs:426`、`:476`、`:619`。
    - 目录页组（T4）：`src/storage/catalog.rs:765`、`:809`、`:893`、`:974`、`:1652`。
    - 恢复面组（T5）：`src/wal/recovery.rs:374`、`:462`、`:1177`、`:1490`。
    - 已在界内、**本 Cycle 不改**：`src/storage/btree/node.rs:39`、`:51`（先判界后取数）。
    - `src/storage/data/table_manager.rs:2762` 位于 `#[cfg(test)]` 模块内的 `count_live_rows` 辅助 ⇒ **不是生产读点**。⇒ **生产读点 = 11（数据页组）+ 5（目录页组）+ 4（恢复面组）= 20**；`src/` 内调用点合计 21（含 1 个测试辅助）。ISS27 台账条目写的「约 17 处」是**低估值**，以本清单为准（`design.md` D7 的「约 21 处」与本清单一致）。
  - **逐点既有语义（本 Cycle 必须保留的「过短」行为，与「越界」严格区分）**：槽区间落在页内但负载短于 `VersionHeader::SIZE`（行版本头）时，各点今天的处置互不相同——`src/storage/data_page.rs:138` 的 `&slot_data[..VersionHeader::SIZE]` **无长度守卫 ⇒ panic**；`src/storage/buffer_pool.rs:476-478` 同样**无守卫 ⇒ panic**；`src/storage/buffer_pool.rs:426-428` 的快路径返回 `FastPath::Done(None)`（= 查不到该行）；`src/storage/data_page.rs:230` 返回 `Ok(None)`（非溢出定位）；`src/storage/data/table_manager.rs:1003`、`:1257`、`src/executor/data_scan.rs:255`、`src/executor/create_index.rs:136`、`src/transaction/manager.rs:571` 与 `src/executor/data_scan.rs:475-478`（`PageAction::SkipSlot`）为**跳过**。
  - **`src/transaction/manager.rs:545-547` 的既有文档注释**：「损坏页 / 越界页按既有页读错误面**显式传播，不静默**。链成环时按既有 `data_chain_status` / `heal_stale_tail` 的形态显式报错，不无限走查。」⇒ 该点的方向**已经是**显式传播，与本 Cycle 一致。
  - **`src/storage/error.rs` 的现有变体**：`CorruptedIndexPage { page_id, index_root_page_id, detail }`，`#[error("corrupted index page {page_id} (index root page {index_root_page_id}): {detail}")]`（`:60`）——既有「读 / 写路径点名损坏页」的专用变体与文案形态；`InvalidPageType { expected, actual }`；`SlotNotFound(RowId)`；`Io(std::io::Error)`；`Internal(String)`。⇒ **没有数据页损坏变体**，本 Cycle 需新增一个与 `CorruptedIndexPage` 同形的变体。
  - **调用方吞错点（必要后果）**：`src/pipeline.rs:335-340` 的
    ```rust
    let has_rows = database
        .table_manager
        .table_has_live_rows(&table_name)
        .await
        .unwrap_or(false);
    plan_builder.set_table_has_live_rows(&table_name, has_rows);
    ```
    该 `unwrap_or(false)` 今天是给「表不存在」准备的兜底 ⇒ 若 `table_has_live_rows` 因**页损坏**返回错误，会被吞成「该表为空」⇒ ALTER TABLE 的计划期判定（`src/parser/planner/ddl_dml.rs:1532-1542`：`ADD COLUMN NOT NULL` 无 DEFAULT 时按「表非空」拒绝）会**失真放行** ⇒ 损坏错误**必须**到达使用者。
  - **`table_has_live_rows` 的用途**：`src/storage/data/table_manager.rs:1242` 的 `pub async fn table_has_live_rows(&self, table: &str) -> Result<bool>`，沿数据页链统计存活行；唯一调用方是 `src/pipeline.rs:337`（**仅 ALTER TABLE 分支**）⇒ 爆炸半径限于 ALTER TABLE 的计划期。
  - **测试入口与夹具事实**：`Database::table_manager` 是 `pub` 字段（`src/database.rs:112`）；`TableManager::get_table(name)` 是 `pub async`（`src/storage/data/table_manager.rs:740`）；`TableMeta::data_page_head: PageId` 是 `pub`（`src/storage/data/table_manager.rs:203`）⇒ 测试夹具可**先取到数据页号 → 关闭库 → 按文件偏移改坏该页 → 重开**。既有同类夹具（**技术参照，非必须回读**）：`tests/index_read_bounded_read_test.rs` 的 `page_file_offset(page, in_page)` / `slot_offset_field_offset(slot_index)` / `corrupt_slot_offset(path, page, slot_index)`（把 offset 字段写成 `0xFFFF`）。
  - **既有回归目标**（本 Cycle 的定向回归范围，142 个集成测试目标中的相关子集）：`storage_test`、`full_table_dml_test`、`table_manager_test`、`alter_table_test`、`executor_test`、`secondary_index_test`、`visibility_test`、`mvcc_abort_test`、`mvcc_commit_test`、`explicit_tx_test`、`overflow_page_test`、`ddl_rewrite_consistency_test`、`drop_table_free_test`、`free_page_lifecycle_test`、`planner_test`、`plan_exec_test`、`ddl_catalog_transactional_test`、`index_manager_test`、`version_chain_test`、`e2e_test`。
- Code and Critical Path:
  - **读路径（取行）**：`Database::execute_sql` → 计划 / 执行器 → `BufferPool::read_tuple`（`src/storage/buffer_pool.rs` 的快路径 `:426` 与主循环 `:476`）与 `src/storage/data_page.rs:137` 的 `read_tuple_from_data_page` ⇒ 槽负载切片 ⇒ **当前在此 panic**。
  - **读路径（扫描）**：`src/executor/data_scan.rs` 的 `build_superseded_map`（`:254`）与主扫描循环（`:472`）⇒ 逐槽取负载 ⇒ 当前越界即 panic。
  - **写路径**：`src/executor/create_index.rs:135`（CREATE INDEX 回填）、`src/storage/data/table_manager.rs:1003`（ALTER TABLE DROP COLUMN 改写前的存活行收集）、`src/transaction/manager.rs:570`（`versions_created_by`：回滚 / 提交按事务版本收集行）、`src/storage/data_page.rs:229`（DELETE 前的溢出定位探测）。
  - **计划期读**：`src/storage/data/table_manager.rs:1257` → `src/pipeline.rs:337` → `src/parser/planner/ddl_dml.rs:1532`。
  - **布尔辅助**：`src/storage/buffer_pool.rs:619` 的 `check_page_all_visible`（供扫描器惰性置位 `all_visible`）。
  - ⇒ **唯一新增的产品原语 = 一个有边界检查的槽负载读取**；其余全部是调用点替换 + 一处调用方吞错的必要后果。

**Implementation Guidance**

- **建议顺序**：
  1. 先落**有边界检查的槽负载读取原语**（`src/storage/page_format/slotted_page.rs`），并落**数据页损坏错误变体**（`src/storage/error.rs`）——两者是所有调用点的共同依赖。
  2. 再按**路径类**逐组替换调用点（取行 → 扫描 → 写 → 计划期读 → 布尔辅助），每组一次 RED→GREEN。
  3. 最后处理 `src/pipeline.rs:337` 的吞错必要后果，并跑定向回归。
- **有边界检查的槽负载读取原语（形状已定，Act 不需再定接口）**：
  - 在 `SlottedPageRef` 上新增 `pub fn get_slot_data_checked(&self, slot: &Slot) -> Option<&'a [u8]>`：**判界与索引树既有有界读同形**——`start >= SlottedPageHeader::SIZE` 且 `end <= slot_area_start`（`end` 用 `checked_add`，`slot_area_start = Page::PAGE_SIZE - slot_count * Slot::SIZE`）；越界返回 `None`，**不做任何越界切片**。与 `src/storage/btree/node.rs:21` 的 `slot_payload_in_bounds` 同源但落在页格式层，供数据页 / 目录页 / 恢复面共用。
  - **既有 `get_slot_data` 保留不动**（`pub` API 不破坏；`src/storage/btree/node.rs:39/:51` 与 `src/storage/page_format/slotted_page.rs` 自带的单元测试仍在用它）。**生产读路径一律不再调用它**——这是本 Cycle 的保证边界；保留它只为不破坏既有公开 API 与既有单元测试。
  - `SlottedPage`（可写变体，`src/storage/page_format/slotted_page.rs:195`）本 Cycle **不加**检查版方法：它在 `src/` 内**没有**未受保护的调用方（唯一调用方 `node.rs:51` 先判界），且本 Cycle 的 11 个生产读点全部是只读引用。
- **数据页损坏错误变体（形状已定）**：在 `src/storage/error.rs` 新增与 `CorruptedIndexPage` 同形的变体
  ```rust
  #[error("corrupted data page {page_id}: {detail}")]
  CorruptedDataPage { page_id: u64, detail: String },
  ```
  `detail` 由调用点组装，**必须点名该槽**（形如 `slot 0 range [65535, 65566) lies outside the page data area`）。`page_id` 取该页的 `PageId.0`（各调用点均在其 `with_page_data(pid, …)` 闭包内或已知页号处，页号可得）。
- **逐点处置表（本 Cycle 的行为定稿；Act 按此实施，不需再判断错误语义）**：
  | # | 位置 | 路径类 | **越界槽**（本 Cycle 的变更点） | **过短负载**（页内但短于版本头） |
  |---|---|---|---|---|
  | 1 | `src/storage/data_page.rs:137` `read_tuple_from_data_page` | 读（取行） | 明确错误（`CorruptedDataPage`） | **今天 panic ⇒ 补长度守卫 ⇒ 明确错误**（同一读路径不得一边报错一边静默） |
  | 2 | `src/storage/data_page.rs:229` `delete_tuple_from_data_page` 探测 | 写（DELETE） | 明确错误 | 保留既有 `Ok(None)` |
  | 3 | `src/storage/data/table_manager.rs:1003` ALTER DROP COLUMN 改写前存活行收集 | 写（DDL 改写） | 明确错误 | 保留既有跳过 |
  | 4 | `src/storage/data/table_manager.rs:1257` `table_has_live_rows` | 读（计划期输入） | 明确错误（且**必须**到达使用者，见下） | 保留既有跳过 |
  | 5 | `src/executor/data_scan.rs:254` `build_superseded_map` | 读（扫描） | 明确错误 | 保留既有跳过 |
  | 6 | `src/executor/data_scan.rs:472` 主扫描循环 | 读（扫描） | 明确错误 | 保留既有 `PageAction::SkipSlot` |
  | 7 | `src/executor/create_index.rs:135` CREATE INDEX 回填 | 写（DDL） | 明确错误 | 保留既有跳过 |
  | 8 | `src/transaction/manager.rs:570` `versions_created_by` | 写（事务） | 明确错误 | 保留既有跳过 |
  | 9 | `src/storage/buffer_pool.rs:426` 全可见快路径取行 | 读（取行） | 明确错误 | **与 #10 同一读路径 ⇒ 一并改为明确错误**（不得快路径「查不到」而慢路径报错） |
  | 10 | `src/storage/buffer_pool.rs:476` 主取行循环 | 读（取行） | 明确错误 | **今天 panic ⇒ 补长度守卫 ⇒ 明确错误** |
  | 11 | `src/storage/buffer_pool.rs:619` `check_page_all_visible` | 布尔辅助 | **降级为安全值** `false`（非「全可见」）⇒ 走慢路径 ⇒ 错误由 #9 / #10 抛出 | 保留既有 `Ok(false)` |
  | 12 | `src/storage/data/table_manager.rs:2762` `count_live_rows` | `#[cfg(test)]` 辅助 | 不在生产面 | 本 Cycle 不改 |
- **调用方吞错的必要后果（`src/pipeline.rs:335-340`）**：越界槽在 `table_has_live_rows` 上产生的 `CorruptedDataPage` **必须**到达使用者，**SHALL NOT** 被 `unwrap_or(false)` 吞成「该表为空」⇒ 损坏时 ALTER TABLE 的计划期判定不得失真放行。实现上：把该处改为「**页损坏错误向上传播**、其余错误保留既有 `false` 兜底」；**不改变**「表不存在 ⇒ `false`」的既有行为。
- **不得引入的东西**：Hash / 指纹 / revision pin / run-id / manifest / 日志 Hash 链 / 时间证明等身份证明机制；判定层 / 裁决脚本 / 为工具而写的工具；新的命令参数；新的配置项。
- **边界保持**：未损坏路径上**逐点**行为不变（含「过短负载」的既有跳过语义、「查不到该行」的既有返回、`all_visible` 快路径的既有命中）；数据页的**页级判据**（`check_data_page`）与其释放 / 重建 / `rtsql check` 消费面**不动**；`src/storage/integrity.rs`、`src/storage/btree/*`、`src/storage/catalog.rs`、`src/wal/*` **不动**。

**Task Definitions**

### T3: 数据页组（数据页 + 缓冲池 + executor / 事务调用点）的越界槽读取经由有界路径；越界槽在读 / 写路径给出点名该页与该槽的明确错误

- Requirement/Scenario: `specs/page-read-bounded-read/spec.md` 的「其余页型的页内容读取经由有边界检查的路径」Requirement；Scenario「数据页上的损坏槽在读 / 写路径给出明确的错误」、「缓冲池的布尔型页辅助在损坏槽上降级为安全值」、「未损坏路径的行为零变化」。
- Depends on: None（Iteration 000 已 accepted，不依赖后继 Cycle）
- Targets:
  - `src/storage/page_format/slotted_page.rs::SlottedPageRef::get_slot_data_checked`（新增）
  - `src/storage/error.rs::StorageError::CorruptedDataPage`（新增变体）
  - `src/storage/data_page.rs::read_tuple_from_data_page`、`::delete_tuple_from_data_page`
  - `src/storage/data/table_manager.rs::table_has_live_rows`、`::alter_drop_column`（改写前的存活行收集点，`:1003`）
  - `src/storage/buffer_pool.rs::read_tuple`（快路径 `:426`、主循环 `:476`）、`::check_page_all_visible`
  - `src/executor/data_scan.rs::build_superseded_map`、主扫描循环的取槽点
  - `src/executor/create_index.rs` 的 CREATE INDEX 回填取槽点
  - `src/transaction/manager.rs::versions_created_by`
  - `src/pipeline.rs` 的 ALTER TABLE 计划期 `table_has_live_rows` 调用点（`:335-340`，吞错修正）
  - `tests/data_page_bounded_read_test.rs`（新增）
- Current behavior: 上述 11 个生产读点用 `SlottedPageRef::get_slot_data` 做**无范围检查**的切片 ⇒ 槽偏移 / 长度越界时 panic（`range start index N out of range for slice of length 4096`）⇒ 进程 abort；其中 `src/storage/data_page.rs:138` 与 `src/storage/buffer_pool.rs:476` 对**过短**负载同样 panic；`src/pipeline.rs:337` 把 `table_has_live_rows` 的错误吞成 `false`。
- Required behavior:
  - 11 个生产读点一律经由 `get_slot_data_checked` 取得槽负载，越界槽**不产生任何越界切片**、**不 panic**；
  - 读 / 写路径（#1–#10）遇越界槽 ⇒ 返回 `StorageError::CorruptedDataPage { page_id, detail }`，`detail` 点名该槽（**不得**被当作「没有这一行」而静默跳过）；
  - #1 / #9 / #10 的**过短**负载由 panic 改为明确错误（同一读路径的快慢两支语义一致）；
  - #11 `check_page_all_visible` 遇越界槽 ⇒ 返回 `false`（安全值）⇒ 走慢路径 ⇒ 错误由 #9 / #10 抛出；
  - `src/pipeline.rs:337` **不吞**页损坏错误；「表不存在 ⇒ `false`」的既有行为不变；
  - 未损坏路径行为零变化。
- Preserve: 数据页的页级判据 `check_data_page` 及其释放 / 重建 / `rtsql check` 消费面；`src/storage/integrity.rs` / `src/storage/btree/*` / `src/storage/catalog.rs` / `src/wal/*` 的行为；`SlottedPageRef::get_slot_data` 的既有签名与既有单元测试；「过短负载」的既有逐点语义（#2 / #3 / #4 / #5 / #6 / #7 / #8 / #11）；`all_visible` 快路径的既有命中；ALTER TABLE 计划期「表不存在 ⇒ `false`」。
- Forbidden: 改动 `src/storage/integrity.rs`、`src/storage/btree/**`、`src/storage/catalog.rs`、`src/wal/**`；改动 `check_data_page` / `check_tree_page` / `validate_slots`；改动 `SlottedPageRef::get_slot_data` 的既有签名或删除它；新增错误语义之外的**新行为**（如把「过短」在 #2–#8 / #11 一并改成报错）；目录页组与恢复面组的任何改动；全量回归；身份证明机制与判定层。
- First test: 位置 `tests/data_page_bounded_read_test.rs`（新增）。**RED**（变更前）：夹具建库 → 建表 `t` → 插入 ≥ 2 行 → 取 `db.table_manager.get_table("t").await?.data_page_head` 拿到数据页号 → 关闭库 → 按 `HEADER_SIZE(64) + page_id * PAGE_SIZE(4096)` 定位该页文件偏移，把**槽目录项 0**（页内起点 `4096 - 1*6 = 4090`）的 **offset 字段（项起点 +2 = 页内 4092）** 写成 `0xFFFF` → 重开库 → 执行 `SELECT … FROM t`（点查与全表扫描各一） ⇒ **预期 RED：进程 panic**（`range start index 65535 out of range for slice of length 4096`），即语句没有返回命名错误。同一夹具另需一条**未损坏对照**：同样步骤但不改字节 ⇒ 变更前即 GREEN（结果集与写入结果与既有行为一致）。
- GREEN condition: 上述 RED 用例变为「语句返回 `CorruptedDataPage` 形态的错误，文案含 `corrupted data page {page_id}` 且点名该槽，**不 panic**」；未损坏对照结果不变；`cargo test --lib` 与本 Cycle 的定向回归清单全绿。
- Verification:
  - 行为类（真实入口）：`cargo test --test data_page_bounded_read_test` —— 覆盖「点查」「全表扫描」「写入」「布尔辅助降级」「未损坏零变化」五类；损坏注入必须是**真实文件字节改写**（不得用替身 / 桩），观察对象是语句返回的错误与进程是否存活。
  - 回归：`cargo test --lib`；`cargo test --test storage_test --test full_table_dml_test --test table_manager_test --test alter_table_test --test executor_test --test secondary_index_test --test visibility_test --test mvcc_abort_test --test mvcc_commit_test --test explicit_tx_test --test overflow_page_test --test ddl_rewrite_consistency_test --test drop_table_free_test --test free_page_lifecycle_test --test planner_test --test plan_exec_test --test ddl_catalog_transactional_test --test index_manager_test --test version_chain_test --test e2e_test` —— 全部 0 failed。
  - **本 Cycle 不跑全量回归**（整个 change 的收尾全量归 Iteration 008 / T16，按 M19 分批串行纪律）。
  - 通过条件：上述命令全部 0 failed，且损坏夹具的语句返回点名错误而非 panic。失败含义：任何一条失败 ⇒ 对应 Acceptance 未满足 ⇒ 停在当前 task。
- Stop when: 需要改动 `src/storage/integrity.rs` / `src/storage/btree/**` / `src/storage/catalog.rs` / `src/wal/**`；或需要新增本 Task Definition 未定义的行为 / 错误语义 / 接口；或发现「越界槽」的正确行为应按另一种页型语义处理（本 Cycle 的页型 = 数据页）；或需要全量回归才能判定（应回到 Plan 调整验证边界）。

**Invariants**

- 未损坏路径上，行集合、写入结果、可见性判定、ALTER TABLE 的计划期判定、事务版本收集结果与既有行为**逐点一致**。
- 「过短负载」的既有逐点语义（跳过 / `Ok(None)` / `SkipSlot` / `check_page_all_visible` 返回 `false`）不变，唯独今天会 panic 的 #1 / #9 / #10 改为明确错误。
- 数据页的页级判据与释放 / 重建 / `rtsql check` 面不变；索引树、目录页、恢复面不变。
- `SlottedPageRef::get_slot_data` 的公开签名与既有单元测试不变。
- 不引入身份证明机制、判定层、裁决脚本、新命令参数或新配置项。

**Non-goals**

- 目录页组（T4）与恢复面组（T5）——后继 Iteration（003 / 004）。
- 索引树叶条目长度（Iteration 000 已覆盖）；索引树访问器（已有界）。
- 数据页的**页级**判据扩展（例如把「过短」也纳入 `check_data_page`）——本 Cycle 只改读 / 写路径的取行点。
- 损坏页的修复 / 重建 / 数据抢救；完整性扫描的诊断分级；`rtsql check` 的输出形态。
- 全量回归（Iteration 008 / T16）；磁盘格式；dump / restore 合同面。

**Acceptance**

| # | 验收条件 | 需求 / 场景 | 设计 | 任务 | 代码位置 | 测试 |
|---|---|---|---|---|---|---|
| A1 | 数据页上的越界槽在读 / 写路径上不 panic、不 abort，语句返回点名该页与该槽的明确错误，不被当作「没有这一行」静默跳过 | `page-read-bounded-read` / 「数据页上的损坏槽在读 / 写路径给出明确的错误」 | D7 数据页组 | T3 | `src/storage/data_page.rs`、`src/storage/data/table_manager.rs`、`src/storage/buffer_pool.rs`、`src/executor/data_scan.rs`、`src/executor/create_index.rs`、`src/transaction/manager.rs` | `tests/data_page_bounded_read_test.rs`（点查 / 扫描 / 写入用例） |
| A2 | 缓冲池的布尔型页辅助在越界槽上返回安全值（非「全可见」）并转入慢路径，错误由实际取行处抛出 | `page-read-bounded-read` / 「缓冲池的布尔型页辅助在损坏槽上降级为安全值」 | D7 数据页组 | T3 | `src/storage/buffer_pool.rs::check_page_all_visible` | `tests/data_page_bounded_read_test.rs`（布尔辅助用例） |
| A3 | 页损坏错误不被调用方吞成「表为空 / 行不存在」；ALTER TABLE 的计划期判定不因损坏而失真放行 | `page-read-bounded-read` / 「数据页上的损坏槽在读 / 写路径给出明确的错误」（末条 AND） | D7 数据页组 | T3 | `src/pipeline.rs`（ALTER TABLE 计划期调用点） | `tests/data_page_bounded_read_test.rs`（ALTER 计划期用例） |
| A4 | 未损坏路径行为零变化（含「过短负载」的既有逐点语义） | `page-read-bounded-read` / 「未损坏路径的行为零变化」 | D7 | T3 | 全部 11 个生产读点 | `tests/data_page_bounded_read_test.rs`（未损坏对照）+ 定向回归清单 |
| A5 | 越界槽不再产生任何越界切片（`get_slot_data_checked` 的判界与索引树既有有界读同形） | `page-read-bounded-read` Requirement 首段 | D7 | T3 | `src/storage/page_format/slotted_page.rs::get_slot_data_checked` | `tests/data_page_bounded_read_test.rs` + `cargo test --lib`（含 `page_format` 单元测试） |

**Verification**

- 逐 scenario 的最简直接判定（行为类 scenario 走**真实入口**：真实建库 → 真实写库 → 真实改坏磁盘字节 → 真实重开 → 真实执行语句；观察语句返回的错误与进程是否存活）：
  1. 「数据页上的损坏槽在读 / 写路径给出明确的错误」：`cargo test --test data_page_bounded_read_test` 中的点查 / 全表扫描 / 写入三例 ⇒ 通过依据 = 语句返回的错误文案含 `corrupted data page {page_id}` 且点名该槽，测试进程未 panic。
  2. 「缓冲池的布尔型页辅助在损坏槽上降级为安全值」：同文件中的布尔辅助用例 ⇒ 通过依据 = 辅助返回「非全可见」，随后的实际取行以 A1 的明确错误结束。
  3. 「未损坏路径的行为零变化」：同文件中的未损坏对照 + 定向回归清单 ⇒ 通过依据 = 全部 0 failed。
- **不建判定层**：判定只落在 `cargo test` 的断言与退出码；不新增脚本 / 封装命令 / 专用参数；不使用 Hash / 指纹 / revision pin / run-id / manifest / 时间证明。
- **验证边界**：本 Cycle 只跑上述定向回归，**不跑全量**（整个 change 的全量预算在 Iteration 008 / T16）。

**Gate 2 Readiness**

- Step 5 逐项复核：TBD / TODO = 无；需求裁剪 = 无（delta spec 已按三组裁定写细，`Simplified` = 0）；tasks / specs / design 一致 = 一致（T3 = 数据页组，与 D7 的数据页行、delta 的数据页场景逐条对应）；Current-State Evidence 全部来自当前工作区代码的逐行核对与本次实际运行（`cargo fmt --check` / `cargo clippy --all-targets` / 既有夹具与常量核对）；每个 task 的执行依据完整（Targets / Current / Required / Preserve / Forbidden / First test / GREEN / Verification / Stop when 齐备）；调用者、影响范围、测试入口已定位（11 个生产读点 + 1 个调用方 + 1 个测试辅助 + 21 个回归目标）；未修改无关范围（Forbidden 已列明 `integrity.rs` / `btree` / `catalog.rs` / `wal`）；Iteration Plan 覆盖全部任务且高开销验证预算明确（全量归 Iteration 008 / T16）；**Act 只需读本 Cycle 文件即可建立第一个先行测试并执行**（夹具字节布局、页号取得方式、损坏写法、预期 RED 形态、错误文案形态均已写明，无需回读前序 Cycle / 其它规划资料 / 重新调查）；非实质未知项已留在 Risks and Notes（不迫使 Act 决定接口或错误语义）；验证直接证明目标行为，行为类 scenario 走真实入口，无身份证明机制与判定层。⇒ **全部 PASS**。
- RTM：`page-read-bounded-read` Requirement + 3 个相关 Scenario ⇒ T3 ⇒ Covered（无 `Missing`、无 `Simplified`）。
- 设计闭合：行为差异、接口（`get_slot_data_checked` / `CorruptedDataPage`）、错误语义（点名该页与该槽）、逐点处置表均已定稿 ⇒ 无 TBD。
- 一致性：本 Cycle 与 change 的 `tasks.md`（T3 归属 Iteration 001）、`specs/page-read-bounded-read/spec.md`、`design.md` D7 一致。
- Persisted Evidence：`none`（见下）。
- 用户批准：本 goal 指令（「按照流程把最新规划的这一批ms做完……开始吧」）构成 Gate 2 计划批准的授权；**批准不降低判据**，上述各项仍逐条检查并记录。

**Cycle RTM**

| Requirement / Scenario | Task | First Test（计划名） | Status |
|---|---|---|---|
| PRBR「其余页型的页内容读取经由有边界检查的路径」Scenario「数据页上的损坏槽在读 / 写路径给出明确的错误」 | T3 | `tests/data_page_bounded_read_test.rs::…`（计划名：越界槽上的点查 / 全表扫描 / 写入各一例；Act 可改名并记录实际名） | Covered |
| PRBR Scenario「缓冲池的布尔型页辅助在损坏槽上降级为安全值」 | T3 | `tests/data_page_bounded_read_test.rs::…`（计划名：布尔辅助一例） | Covered |
| PRBR Scenario「未损坏路径的行为零变化」 | T3 | `tests/data_page_bounded_read_test.rs::…`（计划名：未损坏对照一例）+ 定向回归清单 | Covered |
| PRBR Requirement 首段（越界槽不产生越界切片、不 abort） | T3 | 同上 + `cargo test --lib`（含 `page_format` 单元测试） | Covered |
| PRBR Scenario「目录页上的损坏槽给出明确的错误」 | T4（Iteration 003） | 计划名随 T4 的 Cycle 定稿 | Covered（归属 T4） |
| PRBR Scenario「恢复面上的损坏槽按恢复类别分别处置」 | T5（Iteration 004） | 计划名随 T5 的 Cycle 定稿 | Covered（归属 T5） |

**（2026-10-25 二次结构修正见 `design.md` D15：上表 T4 现为 Iteration **004**、T5 现为 Iteration **005** —— 写路径组插入为 Iteration 003，其后依次顺延。）**

**Persisted Evidence**

- Mode: none

全部结论可低成本重跑（一个新增测试目标 + 21 个既有回归目标 + `cargo test --lib`），命令与实际结果写入 Act Response 即可承载；不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ 不创建 `evidence/` 目录，不预建占位目录。

**Risks and Notes**

- **既有基线条件（非本 Cycle 引入，不纳入本 change）**：项目无 `rust-toolchain.toml`，当前工具链（rustfmt 1.10.0 / cargo-clippy 1.10.0）下 `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 在 **HEAD 上即已失败**（fmt 标记 9 个既有文件；clippy 32 条 warning，lib 侧 17 条全在未触碰文件）。本 Cycle **不要求**全仓 fmt / clippy 通过，**不重排**这些既有文件；只核对**本 Cycle 触碰的文件自身**没有新增 finding。⇒ Act 在本 Cycle 结束时只对本 Cycle 触碰的文件核对这一点（既有文件不动 ⇒ 新增 finding 的可能面 = 本 Cycle 的新增行）。
- **「过短」与「越界」的边界**：本 Cycle 的变更面是**越界**槽；「过短」负载的既有逐点语义保留，唯独今天会 panic 的 #1 / #9 / #10 改为明确错误。若 Act 发现某点的「越界」正确行为应落到**另一种**页型语义（目录页 / 恢复面 / 索引树），⇒ 停下回 Plan（那是页型判定的错误，超出本 Task Definition）。
- **`table_has_live_rows` 的既有兜底**：本 Cycle 只让**页损坏错误**向上传播；「表不存在 ⇒ `false`」必须保留 ⇒ Act 改动 `src/pipeline.rs:337` 时**不得**把兜底整体删掉（否则 ALTER TABLE 在表不存在时的报错形态会变 ⇒ 超出本 Task Definition）。
- **`check_page_all_visible` 的降级值**：安全值 = `false`（非「全可见」）⇒ 扫描器转入逐行可见性判定 ⇒ 随后的实际取行以 A1 的明确错误结束。**不得**把它改成返回 `true` 或直接报错（那会让调用方拿到一个**假的**「全可见」结论）。
- **性能**：逐点判界的成本是常数级（每次取槽一次比较），**不引入**按页全槽扫描的开销 ⇒ 不需要新增缓存或闸门机制。（本 Cycle **不**引入数据页的页级闸门——与索引树的 `ensure_index_page_intact` 不同，数据页的越界判定是**逐槽**的：一个槽越界不影响同页其它槽的读出，整页拒绝会误伤健康行。）
- **测试隔离**：新增测试文件 `tests/data_page_bounded_read_test.rs` 独立承载本 Cycle 的先行测试与对照；损坏夹具用**真实文件字节改写**，不得引入替身 / 桩 / 假存储后端。
- **ISS27 台账条目**在 T4 / T5 完成后才具备关闭条件；本 Cycle **不**改台账状态（关闭由 Recorder 依用户指令执行）。
- **Issue 候选（只报告不登记）：页头 `slot_count` 越界导致的读路径 abort 不在本 change 承载的缺陷形态内**。Plan 读码确认：`SlottedPageRef::get_slot(index)`（`src/storage/page_format/slotted_page.rs:95-110`）以 `let slot_start = Page::PAGE_SIZE - (index + 1) * Slot::SIZE` 直接算槽目录项位置，**无下溢保护**；读路径的迭代上界取自页头的 `slot_count` ⇒ 当页头 `slot_count ≥ 683` 时该减法下溢（release 下回绕成巨大的 `usize`，debug 下直接 panic）⇒ **在 `get_slot` 内 abort，早于本 Cycle 的取槽路径** ⇒ 本 Cycle 的 `get_slot_data_checked` **不覆盖**该形态。页级判据已覆盖它（`validate_slots` 的 `slot_count exceeds the page capacity` violation ⇒ 释放 / 重建 / `rtsql check` 面安全），缺口只在**读路径**。**该形态不在 ISS27 的承载面内**（ISS27 的实现点与触发条件是 `get_slot_data` 的槽 `offset` / `length` 越界），本 Cycle **不擅自扩大**范围 ⇒ 作为 Issue 候选报告，登记与关闭由 Recorder 按用户指令执行。⇒ **Act 不得把「本 Cycle 后任何损坏页都不会 abort」当作已完成结论。**
- **跳过项**：本 Cycle 不跑全量回归（依 change 预算归 Iteration 008 / T16，非跳过而是预算安排）；本 Cycle 不处理目录页组 / 恢复面组（依 Iteration Plan 分 Iteration 执行）；本 Cycle 不做全仓 fmt / clippy 修复（既有基线条件，见上）；本 Cycle 不处理页头 `slot_count` 越界形态（见上一条 Issue 候选）。

**后续任务边界**

- **授权边界**：无。T3 在已批准的范围内（goal 指令构成 Gate 1 / Gate 2 的批准）；Gate 2 各项已逐条检查并记录（无 BLOCKED / SKIPPED）。
- **能力边界**：无。
- **停止边界**：本 Cycle 完成且 Plan Review 判定 `accepted` ⇒ 展开 **Iteration 003**（T4 目录页组）；T4 被接受后再展开 Iteration 004（T5 恢复面组）。Act 不得自行创建下一 Cycle / 下一 Iteration。
- **（2026-10-25 二次结构修正见 `design.md` D15：上句的 **Iteration 003** / **Iteration 004** 已失效 —— 目录页组（T4）现为 **Iteration 004**（`iterations/004-catalog-bounded-read/000-initial.md`），恢复面组（T5）现为 **Iteration 005**；本 Cycle 的实际后继顺序为 **Iteration 003（写路径的槽界限，T18 / T19）→ Iteration 004（T4 目录页组）→ Iteration 005（T5 恢复面组）**。）**
- **不在本 Cycle、需在后续处理的事项**：T4（目录页组，Iteration 003）、T5（恢复面组，Iteration 004）；ISS25 / ISS28 / ISS29（Iteration 005）；ISS30 / ISS31（Iteration 006）；ISS32 / ISS33（Iteration 007）；ISS34 / I045 / I064 与收尾全量（Iteration 008）。
- **（2026-10-25 二次结构修正见 `design.md` D15：上列迭代号依次顺延一位 —— T4 → **004**、T5 → **005**、ISS25 / ISS28 / ISS29 → **006**、ISS30 / ISS31 → **007**、ISS32 / ISS33 → **008**、ISS34 / I045 / I064 与收尾全量 → **009**；写路径组（T18 / T19）另落在 **Iteration 003**。）**
- **交 Maintainer / Recorder 的事项**：ISS27 台账条目的关闭须等 T3 / T4 / T5 全部完成（由 Recorder 按用户指令执行）；本 Cycle 不登记 Issue、不改全局状态（`.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆均不动）、不合并规格。

## Act Response

- Status: reported

### Implemented（实际改动）

**新增产品原语与错误变体**

- `src/storage/page_format/slotted_page.rs::SlottedPageRef::get_slot_data_checked`（新增，`:137-144`）+ 私有判界 `slot_payload_in_bounds`（`:147-150`）：判界与 `src/storage/btree/node.rs` 的既有有界读**同形**——`start >= SlottedPageHeader::SIZE` 且 `end <= slot_area_start`（`end` 用 `checked_add`；`slot_area_start = Page::PAGE_SIZE.saturating_sub(slot_count * Slot::SIZE)`）；越界返回 `None`，**不做任何越界切片**。既有 `get_slot_data` 的签名与自带单元测试**未动**。
- `src/storage/error.rs::StorageError::CorruptedDataPage { page_id: u64, detail: String }`（新增变体，`:67-76`），`#[error("corrupted data page {page_id}: {detail}")]`，与既有 `CorruptedIndexPage` 同形。
- `src/storage/data_page.rs::corrupted_data_page_slot_error`（`:13-25`）与 `::short_data_page_slot_error`（`:27-38`）：集中组装 `detail`（点名槽序号与区间）——`slot {index} range [{start}, {end}) lies outside the page data area` / `slot {index} payload of {len} bytes is shorter than the {22}-byte version header`。经 `src/storage/mod.rs:31-34` 重导出。

**11 个生产读点逐点替换**（与本 Cycle 的逐点处置表逐行一致）

| # | 位置 | 越界槽处置 | 过短负载处置 |
|---|---|---|---|
| 1 | `src/storage/data_page.rs:169` `read_tuple_from_data_page` | `CorruptedDataPage` | **panic ⇒ 明确错误**（`short_data_page_slot_error`） |
| 2 | `src/storage/data_page.rs:269` `delete_tuple_from_data_page` 探测 | `CorruptedDataPage` | 保留既有 `Ok(None)`（非溢出定位） |
| 3 | `src/storage/data/table_manager.rs:1006` ALTER DROP COLUMN 存活行收集 | `CorruptedDataPage` | 保留既有跳过 |
| 4 | `src/storage/data/table_manager.rs:1263` `table_has_live_rows` | `CorruptedDataPage` | 保留既有跳过 |
| 5 | `src/executor/data_scan.rs:259` `build_superseded_map` | `CorruptedDataPage` | 保留既有跳过 |
| 6 | `src/executor/data_scan.rs:479` 主扫描循环 | `CorruptedDataPage` | 保留既有 `PageAction::SkipSlot` |
| 7 | `src/executor/create_index.rs:137` CREATE INDEX 回填 | `CorruptedDataPage` | 保留既有跳过 |
| 8 | `src/transaction/manager.rs:572` `versions_created_by` | `CorruptedDataPage` | 保留既有跳过 |
| 9 | `src/storage/buffer_pool.rs:431` 全可见快路径 | `CorruptedDataPage` | **查不到 ⇒ 明确错误**（与 #10 同语义） |
| 10 | `src/storage/buffer_pool.rs:485` 主取行循环 | `CorruptedDataPage` | **panic ⇒ 明确错误** |
| 11 | `src/storage/buffer_pool.rs:639` `check_page_all_visible` | **降级为安全值 `false`**（非「全可见」）⇒ 走慢路径 | 保留既有 `Ok(false)` |

**调用方吞错的必要后果（A3）**

- `src/pipeline.rs:333-353`（ALTER TABLE 计划期分支）：`table_has_live_rows` 的错误改为**页损坏错误向上传播**（`PlanFailure`，message = `corrupted data page {page_id}: {detail}`，sqlstate `58000`），**其余错误保留既有 `false` 兜底** ⇒「表不存在 ⇒ `false`」的既有行为不变。

**新增测试**

- `tests/data_page_bounded_read_test.rs`（新增，6 个用例）：真实文件库 + 真实 `Database::open` + 真实 SQL 入口；损坏注入 = **真实文件字节改写**（把槽目录项 0 的 offset 字段写成 `0xFFFF`），无替身 / 桩。
  - `damaged_data_page_makes_point_query_fail_with_a_named_error`（点查）
  - `damaged_data_page_makes_table_scan_fail_with_a_named_error`（全表扫描）
  - `damaged_data_page_makes_writes_fail_with_a_named_error`（写入：UPDATE / DELETE / CREATE INDEX 回填 / ALTER TABLE DROP COLUMN）
  - `damaged_data_page_degrades_the_boolean_page_helper_to_the_safe_value`（布尔辅助降级 + 随后的实际取行以点名错误结束）
  - `damaged_data_page_is_not_swallowed_by_the_alter_table_plan_stage`（A3 计划期不失真放行）
  - `undamaged_data_page_reads_and_writes_are_unchanged`（未损坏对照：点查 / 扫描 / UPDATE / INSERT / DELETE / ALTER 计划期拒绝 + 布尔辅助在健康页上仍为「全可见」）

### Deviations from Plan（与计划的偏差及原因）

1. **验证命令的等价调整**：Cycle 的 Verification 给出单条 20 目标命令；本 Cycle 按 M19 的多批次串行纪律把它拆成**两批各 10 个目标**，批内 `--test-threads=1`、批间串行（批 1 全绿后才跑批 2）。覆盖的目标集合与 Cycle **完全一致**，Gate 判据仍是「全部 0 failed」。命令形式为 `cargo test --test X … -- --test-threads=1`（本机 cargo 1.99.1 拒绝 `cargo test --test-threads=1 --test X`：`error: unexpected argument '--test-threads' found`）。
2. **新增两个具名错误构造器**（`corrupted_data_page_slot_error` / `short_data_page_slot_error`，置于 `src/storage/data_page.rs`）：Cycle 钉定的是错误**变体**的形状与 `detail` 的内容要求，**未钉定 detail 的组装位置**；把组装集中到数据页模块（而非在 11 个调用点各写一份 `format!`）是执行依据内的等价局部实现选择。**不新增**任何错误语义、接口或行为。
3. **`get_slot_data_checked` 末步用 `self.data.get(start..end)` 收口**：判界条件与 Cycle 钉定的**完全相同**；末步改用 `get` 而非 `&self.data[start..end]`，只为在页缓冲短于 `PAGE_SIZE` 时也不越界（当前所有调用点的页缓冲恒为 4096 字节 ⇒ 行为等价）。Clippy 在首版实现上就此报 `enclosing Some and ? operator are unneeded`（`slotted_page.rs:143`）⇒ 已按其建议改为 `self.data.get(start..end)`。
4. **`src/pipeline.rs` 的 sqlstate 取 `"58000"`**：Cycle 未钉定 sqlstate；取 `PlanError::sqlstate()` 对未分类错误的既有兜底值，与既有计划期错误面一致。

### Blocker Handoff

None —— 未命中 Gate 5（无缺失依赖、无验证失败、无 Task Definition 冲突、无实质偏离）。

### Self-Review（任务级 Review 与自检）

**Gate 3 — Spec compliance**

- **A1**（越界槽在读 / 写路径不 panic / 不 abort，点名页与槽，不被当作「没有这一行」跳过）：逐点处置表 11 行全部落地（见上表）；越界槽一律返回 `CorruptedDataPage { page_id, detail }`，`detail` 点名槽序号与区间 ⇒ **满足**。
- **A2**（布尔型页辅助降级为安全值并转入慢路径，错误由实际取行处抛出）：`check_page_all_visible` 遇越界槽返回 `false`（`:639`），布尔辅助**自身不报页损坏**；随后的实际取行以 A1 的点名错误结束 ⇒ **满足**。
- **A3**（页损坏错误不被调用方吞成「表为空 / 行不存在」；ALTER 计划期不失真放行）：`src/pipeline.rs:345` 显式上抛 `CorruptedDataPage` ⇒ **满足**。
- **A4**（未损坏路径零变化，含「过短负载」的既有逐点语义）：#2–#8 / #11 的「过短」逐点语义**逐条保留**（`Ok(None)` / 跳过 / `SkipSlot` / `Ok(false)`）；唯独今天会 panic 的 #1 / #9 / #10 改为明确错误 —— 与 Cycle 的 Required behavior 一致 ⇒ **满足**。
- **A5**（越界槽不再产生任何越界切片，判界与索引树既有有界读同形）：`get_slot_data_checked` 与 `slot_payload_in_bounds` 同形且**不做任何越界切片** ⇒ **满足**。
- **Preserve / Forbidden 核对**：数据页页级判据 `check_data_page` 与释放 / 重建 / `rtsql check` 消费面**未动**；`src/storage/integrity.rs` / `src/storage/btree/**` / `src/storage/catalog.rs` / `src/wal/**` **未动**；`check_data_page` / `check_tree_page` / `validate_slots` **未动**；`SlottedPageRef::get_slot_data` 的签名与既有单元测试**未动**；`all_visible` 快路径的既有命中**未动**；目录页组 / 恢复面组**未动**；未引入任何新命令参数 / 配置项 / 判定层 / 身份证明机制。

**Gate 3 — Code quality**

- diff 无计划外修改：改动仅落在 Task Definition 的 Targets 列出的文件与符号 ⇒ **通过**。
- 无新增警告：本 Cycle 触碰的文件在 `cargo clippy --all-targets` 下**零 finding** ⇒ **通过**。
- 测试不会因错误原因通过：损坏夹具是真实文件字节改写，断言同时检查**错误文案**（含 `corrupted data page {page_id}` 且点名 `slot 0`）与**进程存活**（未损坏对照用例在变更前即 GREEN，说明断言不是恒真）⇒ **通过**。

**自检发现（已在本 Cycle 内修复并复验）**

1. **[Important]** `get_slot_data_checked` 首版写成 `Some(self.data.get(start..end)?)` ⇒ clippy `enclosing Some and ? operator are unneeded`（`slotted_page.rs:143`）⇒ 已改为 `self.data.get(start..end)`（语义等价）；lib 侧 clippy warning 由 18 条回到既有基线 17 条。
2. **[Important]** 首版**误读**逐点处置表，把 #2（`delete_tuple_from_data_page` 探测）的「过短负载」也改成了明确错误。Cycle 的处置表明确 #2 **保留既有 `Ok(None)`** ⇒ 已回退为 `Ok(None)`（并留注释说明既有语义）。**未放宽任何断言**，且该回退发生在任何针对 #2 的验证之前。
3. **[Minor]** `rustfmt` 在本 Cycle 触碰的 5 个文件里产生格式差异（`create_index.rs` / `data_scan.rs` / `buffer_pool.rs` / `data_page.rs` / 新测试文件）⇒ 已对这 5 个文件**单独**运行 `rustfmt --edition 2021`。**刻意未**对 `src/storage/data/table_manager.rs` 运行 rustfmt：该文件有 6 处 fmt 差异（`:720`、`:1817`、`:1864`、`:1891`、`:1921`、`:1955`）**全在我未触碰的既有代码区**（既有基线条件），重排会违反 Surgical Changes ⇒ 保留；我的改动区（`:1006`、`:1263`）fmt 干净。

**遗留 Minor 问题**

None —— 上述 3 项均已修复并复验；无未解决的 Critical / Important 问题。

### Verification Evidence（验证命令与实际结果）

**1) 先行测试的 RED（变更前）** —— `cargo test --test data_page_bounded_read_test`

5 个损坏用例**逐个单独运行**，全部 RED，进程 abort（SIGABRT）：

```
thread 'damaged_data_page_makes_point_query_fail_with_a_named_error' panicked at
  src/storage/page_format/slotted_page.rs:128:19:
range start index 65535 out of range for slice of length 4096
process didn't exit successfully: ... (signal: 6, SIGABRT: process abort signal)
```

其余 4 个损坏用例（扫描 / 写入 / 布尔辅助 / ALTER 计划期）的 RED 形态**逐字相同**（同一 panic 位置、同一文案）。未损坏对照用例在**变更前即 GREEN**（`test undamaged_data_page_reads_and_writes_are_unchanged ... ok`）。⇒ RED 形态与 Task Definition 钉定的形态**逐字一致**，**未放宽断言**。

**2) GREEN（变更后）** —— `cargo test --test data_page_bounded_read_test -- --test-threads=1`

```
running 6 tests
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.39s
```

**3) `cargo test --lib`**（Iteration 000 的 420/0 结论对本 Cycle 已失效 ⇒ 重跑）

```
test result: ok. 420 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.53s
```

**4) 定向回归 批次 1/2**（10 个目标，批内 `--test-threads=1`）—— 16 + 39 + 21 + 11 + 3 + 4 + 27 + 21 + 6 + 5 = **153 passed / 0 failed**。

**5) 定向回归 批次 2/2**（10 个目标，批内 `--test-threads=1`；批 1 全绿后才启动）—— 3 + 9 + 6 + 27 + 12 + 3 + 27 + 4 + 107 + 3 = **201 passed / 0 failed**。

定向回归合计 **354 passed / 0 failed**（目标集合与 Cycle 的 Verification 清单**完全一致**）。

**6) 格式化 / 静态分析（与既有基线对比，只核对本 Cycle 触碰的文件）**

- `cargo fmt --check` ⇒ 报告 **9 个既有文件**：`src/storage/btree/btree.rs`、`src/storage/data/mod.rs`、`src/storage/data/table_manager.rs`、`src/storage/integrity.rs`、`tests/ddl_rewrite_consistency_test.rs`、`tests/free_page_lifecycle_test.rs`、`tests/index_page_ownership_test.rs`、`tests/index_read_bounded_read_test.rs`、`tests/release_path_bounded_read_test.rs` —— 与本 Cycle 的 Risks and Notes 记录的既有基线（**fmt 标记 9 个既有文件**）**逐项一致**。本 Cycle 触碰的 11 个文件中**只有 `table_manager.rs` 在列**，且其 6 处差异全在我未触碰的既有代码区（见 Self-Review 第 3 项）⇒ **本 Cycle 对 fmt 基线的贡献为 0**。
- `cargo clippy --all-targets` ⇒ lib 侧 **17 条 warning**，与既有基线记录（**lib 侧 17 条全在未触碰文件**）**一致**；本 Cycle 触碰的文件**零 finding**。

**7) 保证边界核对（生产读路径不再调用无检查的 `get_slot_data`）**

`grep get_slot_data src/` ⇒ `src/` 内剩余调用点只有：`src/storage/btree/node.rs:39`/`:51`（索引树，先判界后取数）、`src/storage/catalog.rs:765/809/893/974/1652`（**T4 目录页组**）、`src/wal/recovery.rs:374/462/1177/1490`（**T5 恢复面组**）、`src/storage/data/table_manager.rs:2769`（`#[cfg(test)]` 的 `count_live_rows` 辅助 —— 处置表 #12 明列「本 Cycle 不改」）、`src/storage/page_format/slotted_page.rs` 的定义与自带单元测试。⇒ **数据页组的 11 个生产读点已全部脱离无检查路径**；目录页组 / 恢复面组按计划留给 T4 / T5。

**8) 改动后的复验**：`rustfmt` 重排（5 个文件）与 clippy 修复（`slotted_page.rs:143`）发生在首次 GREEN **之后** ⇒ 被覆盖的代码已变化 ⇒ **第 2–5 项的全部命令在最终代码状态上完整重跑了一遍**（本次记录即复验结果，全部 0 failed）。

### Persisted Evidence

**None required** —— 全部结论可低成本重跑（1 个新增测试目标 + `cargo test --lib` + 20 个既有回归目标），命令与实际结果已在本 Response 承载；不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ 不创建 `evidence/` 目录，不预建占位目录（与 Plan 的 `Persisted Evidence: none` 一致）。

### Experience Candidates

1. **Runbook 候选**：槽损坏注入夹具（真实文件字节改写）的字节布局与步骤 —— 文件偏移 = `HEADER_SIZE(64) + page_id * PAGE_SIZE(4096) + (4096 - (slot_index+1) * Slot::SIZE(6) + 2)`，把 offset 字段写成 `0xFFFF`。本 Cycle **端到端验证**（RED 的 panic 文案逐字命中钉定形态，GREEN 后同一夹具产出点名错误）；仓库内已有 3 个测试文件使用同形夹具（`tests/index_read_bounded_read_test.rs`、`tests/release_path_bounded_read_test.rs`、本 Cycle 新增的 `tests/data_page_bounded_read_test.rs`）⇒ **可重复**且字节算术易错，值得固化为 Runbook。
2. **Issue 候选（只报告不登记）**：页头 `slot_count ≥ 683` 时 `SlottedPageRef::get_slot` 的槽目录定位整数下溢 ⇒ 读路径在 `get_slot` 内 abort，**早于**本 Cycle 的取槽路径 ⇒ `get_slot_data_checked` **不覆盖**该形态。本 Cycle 的实施路径（`get_slot_by_logical_id` / `get_slot(index)`）确实经过该点，已用读码确认。**并行 Recorder 已在本会话建档为 `.agents/issues/ISS35-slot-count-underflow-aborts-read-path.md`**（Act 未修改该文件）⇒ 本 Cycle **不**声明「任何损坏页都不会 abort」。

### Remaining Issues（未解决问题）

- 目录页组（T4）与恢复面组（T5）仍用无检查的 `get_slot_data` ⇒ 后继 Iteration（003 / 004）承载。
- 页头 `slot_count` 越界形态（ISS35）**独立承载**，本 Cycle 未实施、未扩范围。
- 数据页的**页级**判据（`check_data_page`）与其释放 / 重建 / `rtsql check` 消费面未动（本 Cycle 非目标）。
- 本 Cycle **不跑全量回归**（change 预算归 Iteration 008 / T16）⇒「本 Cycle 后全库无回归」**不是**本 Cycle 的结论。
- 未发现当前 change 范围内需要额外工作的实质缺陷。

### 跳过项

- **全量回归** —— 依 change 预算归 Iteration 008 / T16（M19 分批串行纪律，本 change 唯一一次）—— 是**预算安排**，不是跳过。
- **目录页组 / 恢复面组** —— 依 Iteration Plan 分 Iteration 执行（T4 / T5 为后继 Iteration：003 / 004）。
- **全仓 fmt / clippy 修复** —— 既有基线条件（9 个既有文件 / lib 侧 17 条 warning 全在未触碰文件），重排违反 Surgical Changes ⇒ 只核对本 Cycle 触碰的文件自身。
- **页头 `slot_count` 越界形态** —— 独立缺陷、需独立承载（ISS35），见 Remaining Issues。

### 任务状态记录

- 本环境的任务追踪工具不可用（`tool_search` 查询 `todo task tracking list create` 无命中）⇒ task / Gate / 跳过项状态记录于本 Response 与 change `tasks.md`（T3 状态已同步为「Act Response `reported`，待 Plan Review」；Iteration 001 状态同步为「Cycle 000 = T3 已报告，待 Plan Review；T4 / T5 待后续 Iteration 展开」）。

## Plan Review

- Review Result: accepted

**Findings**

**F1（PASS，A1 / A5）11 个生产读点全部改走有边界检查的访问器；判据与索引树侧既有有界读逐项同形。**
独立核对（读码，不重跑 Act 的验证）：`grep -n "get_slot_data_checked" src/` 命中 **11 个生产调用点**，与 Act Response 的逐点处置表**逐行对应**：

| # | 位置（实际行号） | 调用形态 |
|---|---|---|
| 1 | `src/storage/data_page.rs:169` | `.get_slot_data_checked(&slot).ok_or_else(|| corrupted_data_page_slot_error(page_id.0, slot_index, &slot))?` |
| 2 | `src/storage/data_page.rs:269` | 同形 |
| 3 | `src/storage/data/table_manager.rs:1006` | 同形（`pid.0, index`） |
| 4 | `src/storage/data/table_manager.rs:1263` | 同形（`pid.0, index`） |
| 5 | `src/executor/data_scan.rs:259` | 同形（`page_id.0, slot_index`） |
| 6 | `src/executor/data_scan.rs:479` | 同形（`page_id.0, slot_index`） |
| 7 | `src/executor/create_index.rs:137` | 同形（`pid.0, index`） |
| 8 | `src/transaction/manager.rs:572` | 同形（`pid.0, index`） |
| 9 | `src/storage/buffer_pool.rs:431` | 同形（`page_id.0, slot_index`） |
| 10 | `src/storage/buffer_pool.rs:485` | 同形（`current_page_id.0, slot_index`） |
| 11 | `src/storage/buffer_pool.rs:639` | `match … { Some(d) => d, None => /* 降级为 false */ }`（布尔辅助的**安全值**路径） |

判据同形性逐项核对 `src/storage/page_format/slotted_page.rs:137-150` 与 `src/storage/btree/node.rs:21-28`：
`slot_area_start = Page::PAGE_SIZE.saturating_sub(slot_count * Slot::SIZE)`（新侧 `self.slot_count()` ≡ 旧侧 `header.slot_count as usize`，同值）、`end` 两侧均用 `checked_add`（新侧 `?` ⇒ `None`，旧侧 `false` ⇒ `None`，等价）、判据式 `start >= SlottedPageHeader::SIZE && end <= slot_area_start` **逐字符一致** ⇒ **A5 成立**。
`get_slot_data_checked` 末步为 `self.data.get(start..end)` ⇒ **不产生任何越界切片** ✓。

**F2（PASS，A1 / A4）短负载语义逐点保持；只有原先会 panic 的 #1 / #9 / #10 改为显式错误。**
逐点核对：#1 `data_page.rs:169` 前置 `slot_data.len() < VersionHeader::SIZE` ⇒ `short_data_page_slot_error`（**panic ⇒ 明确错误**）✓；#2 `data_page.rs:269` 的短负载仍走既有 `Ok(None)`（非溢出定位），**未被改成错误** ✓ —— Act 自检第 2 项记录的「一度误改 #2、已在任何针对 #2 的验证前回退」在当前代码中**看不到残留**，回退干净；#3 / #4 / #7 / #8 的 `else { continue; }` 跳过语义保留 ✓；#5 / #6 的 `SkipSlot` 保留 ✓；#11 `check_page_all_visible` 保留 `Ok(false)` ✓。⇒ **A4 成立**。

**F3（PASS，A3）`src/pipeline.rs` 只让页损坏错误上浮，「表不存在 ⇒ false」零变化。**
核对 `src/pipeline.rs:339-352`：`Err(StorageError::CorruptedDataPage { page_id, detail }) => return Err(PlanFailure { message: format!("corrupted data page {page_id}: {detail}"), sqlstate: "58000" })`，其余 `Err(_) => false` **原样保留** ⇒ 「表不存在」等既有错误仍被兜底成 `false`，只有页损坏错误上浮 ⇒ **A3 成立**。

**F4（PASS，声明一致性）Act 未声称「任何损坏页都不会 abort」，且代码与该声明一致。**
`src/storage/page_format/slotted_page.rs:96-110` 的 `get_slot` **逐行未改**：`let slot_start = Page::PAGE_SIZE - (index + 1) * Slot::SIZE;` **仍无下溢保护** ⇒ 页头 `slot_count ≥ 683` 时该减法下溢 ⇒ `&self.data[slot_start..slot_start + Slot::SIZE]` 越界 panic ⇒ 进程 abort。Act Response 的 Risks / Experience Candidates / Remaining Issues 三处均显式写明该形态仍 abort 并已登记为 ISS35，**没有**出现「任何损坏页都不会 abort」这类过强结论 ⇒ **声明与代码一致**。
补充核对：`get_slot_data_checked` 的入参是**已取得的 `&Slot`**（`&self, slot: &Slot`）⇒ 它在结构上**不可能**覆盖槽目录定位的下溢 ⇒ 与 Act 的说明自洽。

**F5（PASS，边界纪律）目录页组与恢复面组未被触碰；`integrity.rs` / `btree/**` / `wal/**` 未被本 Cycle 改动。**
`grep -n "get_slot_data\b" src/` 的生产调用点剩余：索引树 `src/storage/btree/node.rs`（先判界后取数）、目录页组 `src/storage/catalog.rs:765/809/893/974/1652`、恢复面组 `src/wal/recovery.rs:374/462/1177/1490`、`src/storage/data/table_manager.rs:2769`（`#[cfg(test)]` 辅助，处置表 #12 明列不改）——**与 Act 的边界核对逐项一致** ⇒ 无越界改动。

**F6（PASS，非阻塞 Minor）`src/pipeline.rs:347` 手工拼接错误文案而未复用 `StorageError` 的 Display。**
`format!("corrupted data page {page_id}: {detail}")` 与 `StorageError::CorruptedDataPage` 的 `#[error("corrupted data page {page_id}: {detail}")]` 文本**一致**，故使用者所见相同 ⇒ 纯等价、**不阻塞 Acceptance**。属局部写法偏好，按公共规则不为风格偏好创建修复，**仅记录**。

**F7（PASS，非阻塞 Minor）Act 的 fmt 基线数字与调度方独立核实的数字口径一致。**
Act 记录 `cargo fmt --check` 报 **9 个既有文件**、clippy lib 侧 **17 条 warning**、其触碰文件内 **0 条**；调度方在 HEAD 上独立核实为 fmt **50 处 diff 分布在 9 个既有文件**、deny-clippy **17 条 lib 错误**、非 deny **32 条 warning** ⇒ 两者**同口径一致**（Act 的 9 文件是「文件数」，调度方的 50 处是「差异处数」）。Act 自检第 3 项对 `table_manager.rs` 的 6 处 fmt 差异全部位于未触碰的既有代码区、刻意未重排，符合 Surgical Changes ⇒ **不构成 finding**。基线在 HEAD 即红，本 change **不纳入**无关文件重排（见 `design.md` D13 对 ISS36 的排除记录）。

**F8（PASS，证据充分性）Act 的验证结论覆盖 Acceptance，且可低成本重跑；Persisted Evidence `none` 成立。**
RED 形态（`panicked at src/storage/page_format/slotted_page.rs:128:19: range start index 65535 out of range for slice of length 4096` → SIGABRT）与 Task Definition 钉定的形态**逐字一致**，且 RED 指向的 `:128` 正是 `get_slot_data` 的无界切片行 ⇒ 修复点正确；GREEN `cargo test --test data_page_bounded_read_test` 6 passed / 0 failed；`cargo test --lib` 420 passed / 0 failed；定向回归 20 目标 **2 批各 10 个**、批内 `--test-threads=1`、批间串行 ⇒ 354 passed / 0 failed（与 Cycle 的 Verification 清单目标集合完全一致）；`cargo fmt --check` 与 `cargo clippy --all-targets` 只核对本 Cycle 触碰文件 ⇒ 与基线零差异。全部结论**可低成本重跑**且「Act Response 装得下」⇒ `Persisted Evidence: none` 成立，无须建 `evidence/` 目录。

**F9（新证据，不阻塞）ISS35 的**读码事实**在本 Review 中被独立复核，并成为后继 Cycle 的需求来源。**
独立复核 `get_slot` 的下溢算术：`slot_start = 4096 - (index + 1) * 6`；`(index+1)*6 > 4096` ⇒ `index ≥ 682` ⇒ 页头 `slot_count ≥ 683` 时下溢。复核页级判据的阈值一致性：`integrity::validate_slots` 在 `slot_area_start < SlottedPageHeader::SIZE`（即 `slot_count ≥ 681`）时报 violation，而 `src/storage/btree/node.rs:14` 的 `MAX_PAGE_SLOTS = (4096 - 16) / 6 = 680` 与该阈值**同一容量界限** ⇒ 索引树侧**已有**同形界限，数据页 / 目录页 / 恢复面侧**没有** ⇒ ISS35 的「已知但只在一侧有防护」成立。
同时确认：`src/storage/integrity.rs:558-564` 在 `validate_slots` 返回 `None` 后**不再**进入 `page.get_slot(i)` 循环 ⇒ 完整性扫描面**不受**该形态影响；`src/storage/btree/node.rs:843` 的 `SlottedPage::get_slot`（**可写**变体）在索引树**页级闸门** `ensure_index_page_intact` 之后 ⇒ 索引树写路径**不受**影响。⇒ ISS35 的暴露面**只在数据页组 / 目录页组 / 恢复面组的只读读路径**。

**F10（Minor finding，非阻塞）`src/storage/data_page.rs:237` 的无界写仍可 panic（可写变体域，ISS35 台账问题 ③ 的相邻面）。**
`update_version_header_in_data_page` 用**可写**变体 `SlottedPage::new(page)` 取槽（`:231-233`），随后执行**无范围检查**的写 `page.data[slot_offset..slot_offset + VersionHeader::SIZE].copy_from_slice(...)`（`:237`）⇒ 若该槽的 `offset` 越界，此处 panic ⇒ 进程 abort。该点**不在** T3 的 11 个读点内（T3 的 Target 是「读点」，此处是**写**路径且用可写变体）⇒ **不构成 A1–A5 的缺口**：delta spec 的场景触发形态是「**读**到一个损坏页」，而数据页组内到达该写的路径（UPDATE / DELETE / rollback / commit 的版本头改写）**均先经过已被 T3 改造的受检读取**（`read_tuple_from_data_page` / `versions_created_by`）⇒ 损坏槽在到达该写之前已以点名错误结束；T3 的写入用例（UPDATE / DELETE / CREATE INDEX 回填 / ALTER TABLE DROP COLUMN）实测为点名错误而非 panic，与此一致。
**残留**：① 该写路径对「读取之后页被改坏」的形态无保护；② **`src/wal/recovery.rs:1519`** 从损坏页的槽（`:1487`）取 `logical_id` 后调用同一函数 ⇒ **恢复面**可达该 panic（既非槽负载越界，也非槽目录下溢，而是**可写变体 + 负载偏移**的同形）⇒ 属 T5（恢复面组）的范围，不在本 Cycle。
⇒ **记录为 Issue 候选**（只报告不登记）：可写变体（`SlottedPage`）的同形无界访问——`update_version_header_in_data_page` 的无界写（`src/storage/data_page.rs:237`）与 `SlottedPage::get_slot`（`:183-201`）的槽目录下溢——建议独立登记；ISS35 台账明确「本条只主张读路径」，故**不并入 ISS35**。

**偏差分类**：无 Plan 遗漏、无 Plan 错误、无 Act 偏离、无现状变化；F6 / F7 / F10 为非阻塞 Minor finding（等价写法 / 数字口径 / 可写变体域的相邻面），F9 为新证据（不阻塞本 Cycle 的 Acceptance，转为后继 Cycle 的需求来源）。

**Acceptance Gaps**

None —— A1 / A2 / A3 / A4 / A5 全部满足（逐条证据见 F1–F5、F8）；本 Cycle 的 Verification 清单与 Act 实际执行的命令**目标集合一致**且全绿；无被跳过的 Acceptance（跳过项仅为「全量回归」与「目录页组 / 恢复面组 / ISS35 / 全仓 fmt-clippy」四类**范围性**排除，均已在本 Cycle 的 Risks / 跳过项中记录且与 change 的预算安排一致）。

**Evidence**

- 代码与 diff：`git diff`（11 个产品文件 + 新测试 `tests/data_page_bounded_read_test.rs`）；逐行核读 `src/storage/page_format/slotted_page.rs:92-150`、`src/storage/btree/node.rs:1-60`、`src/storage/error.rs`（`CorruptedDataPage`）、`src/storage/mod.rs:31-34`（重导出）、`src/storage/data_page.rs:160-175/260-275`、`src/storage/data/table_manager.rs:1000-1010/1258-1268`、`src/storage/buffer_pool.rs:425-490/633-645`、`src/executor/data_scan.rs:250-265/473-485`、`src/executor/create_index.rs:130-142`、`src/transaction/manager.rs:566-578`、`src/pipeline.rs:333-353`。
- 边界核对：`grep -n "get_slot_data_checked" src/`（11 处生产调用）、`grep -n "get_slot_data\b" src/`（剩余目录页组 / 恢复面组 / 索引树 / `#[cfg(test)]` 辅助）、`grep -n "\.get_slot\(|\.get_slot_by_logical_id\(" src/`（全量 `get_slot` 调用点清单：数据页组 11、目录页组 5、恢复面组 4、索引树 2、`integrity.rs` 2（`validate_slots` 前置保护）、`slotted_page.rs` 内部实现）。
- 夹具与常量：`SlottedPageHeader::SIZE = 16`（`slotted_page.rs:27`）、`Slot::SIZE = 6`（`:12`）、`Page::PAGE_SIZE = 4096`（`src/storage/page.rs:11`）⇒ `MAX_PAGE_SLOTS = 680`；`TABLES_PAGE_ID = 0` / `COLUMNS_PAGE_ID = 1`（`src/storage/catalog.rs:38/40`）。
- 沿用 Act 已报告且覆盖范围未失效的验证结论（来源：Act Response「Verification Evidence」第 1–8 项）：RED 5 例逐个单独运行 SIGABRT（panic 文案逐字命中 `slotted_page.rs:128:19`）、GREEN `cargo test --test data_page_bounded_read_test` 6/0、`cargo test --lib` 420/0、定向回归 2 批 354/0、fmt 9 文件 / clippy lib 17 条且触碰文件 0 条。**Plan 未重跑上述验证**（公共规则 › 验证：覆盖范围未变化 ⇒ 直接沿用），只做了代码与 diff 的独立阅读。
- 调度方独立核实的基线事实（HEAD `e9cb9d3`）：项目**无** `rust-toolchain.toml`；`cargo fmt --check` 在 HEAD 报 50 处 diff 分布在 **9 个既有文件**；`cargo clippy --all-targets -- -D warnings` 在 HEAD 即失败（**17 条 lib 错误**，deny 模式下 lib 即中止、test 目标未被 lint），非 deny 模式 **32 条 warning**；仓库内另装 `nightly` 与 `nightly-2026-02-25` 两个本地工具链。

**Follow-up Decision**

**接受本 Cycle（`accepted`）。** 依据：既有 Acceptance（A1–A5）全部满足且有独立代码核对支撑；无阻塞 finding；F6 / F7 为非阻塞 Minor finding（按公共规则不为等价写法 / 数字口径创建修复）；F9 是**新证据**，不改变本 Cycle 的结论，而是把 ISS35 从「Act 报告的 Issue 候选」推进为**本 change 内已分配的承载任务**。
**执行正确性要求的顺序**（本次 Review 的核心结论）：ISS35 必须**先于** T4 / T5 落地 —— T4 / T5 的读点同样经由 `SlottedPageRef::get_slot` 取得 `&Slot`；若 ISS35 的修复改变 `get_slot` 的形状（返回 `Option` / `Result` 或新增界限），T4 / T5 必须写在最终形状上，否则返工。故 ISS35 承载为 **Iteration 001 内的本地后继 Cycle**（`001-slot-count-underflow.md`，Cycle Type = `replan`——Iteration 001 的计划因纳入 T17 而修订），**排在 T4 之前**；T4 顺延为 `002-catalog-bounded-read.md`。**（2026-10-25 结构修正，见 `design.md` D14：上文 `001-slot-count-underflow.md` 与 `002-catalog-bounded-read.md` 为**当时的文件名，已失效**；该承载形态已改为独立 Iteration —— T17 落在 `iterations/002-slot-count-underflow/000-initial.md`（Cycle Type = `initial`），T4 落在 `iterations/003-catalog-bounded-read/000-initial.md`，T5 落在 Iteration 004。本 Review 的实质结论「ISS35 必须先于 T4 / T5 落地，否则 T4 / T5 返工」不变。**2026-10-25 二次结构修正见 `design.md` D15：上句的 `iterations/003-catalog-bounded-read/000-initial.md` 已再次顺延为 `iterations/004-catalog-bounded-read/000-initial.md`（Iteration **004**），T5 落在 Iteration **005**，写路径组（T18 / T19）插入为 Iteration **003**；本 Review 的实质结论不变。）**
**ISS35 的形态裁定**（记录在 `design.md` D12 与后继 Cycle 的 Plan Context）：① **修复位置**——页格式层提供**统一的容量界限**（`MAX_PAGE_SLOTS` 上移到 `src/storage/page_format/slotted_page.rs`，`src/storage/btree/node.rs` 改为引用，值与判据不变 ⇒ 索引树路径零回归）+ **页级判据**（页头声称的槽数超出槽目录容量 ⇒ 页损坏），由**读点在取槽之前**消费；**不改 `get_slot` 的 `None` 语义**（理由：与「槽不存在」不可区分 ⇒ 会把 abort 变成静默跳过，违反「跳过 = 静默错读」）；② **损坏页在该页型的正确行为**——数据页组 ⇒ 明确的 `CorruptedDataPage`；目录页组 ⇒ 明确错误 ⇒ 该库 SHALL NOT 打开（沿用 T4 的既有裁定）；恢复面组 ⇒ 按 T5 已定的两类语义；③ **可写变体与 `add_slot_inner` 的同形空间检查**——**不纳入**（ISS35 台账明确「本条只主张读路径」；写路径需独立方向）⇒ 作为 Issue 候选报告，不登记。
**本 Cycle 不再需要修复**：无待修项；F6 / F7 仅记录，不建 rework Cycle。
**状态同步**：change `tasks.md` 已同步 —— T3 = `done（Cycle 000 Plan Review 2026-10-25 accepted）`；新增 **T17**（ISS35 承载，归属 Iteration 002）；T4 = `planned（Iteration 003 / Cycle 000）`；Iteration Map 的 001 行状态 = `in progress`（Cycle 000 已 `accepted`），T17 / T4 / T5 分落 Iteration 002 / 003 / 004（002 / 003 `ready`，004 待展开）。**Act Response 中「T3 状态已同步为『Act Response reported，待 Plan Review』」一句是 Act 当时的记录，现已被本 Review 的结论取代。** **（2026-10-25 二次结构修正见 `design.md` D15：上句的「T4 = `planned（Iteration 003 / Cycle 000）`」与「T17 / T4 / T5 分落 Iteration 002 / 003 / 004」为当时的编号 —— 写路径组（T18 / T19）插入为 **Iteration 003**（Cycle `iterations/003-write-path-slot-bounds/000-initial.md`，Plan Context `ready`），T4 现为 **Iteration 004**、T5 现为 **Iteration 005**；Iteration 002 已于 2026-10-25 被 Plan Review 判定 `accepted`。）**

**Iteration Plan Update**

本 Cycle 自身的目标、范围、依赖、验证方式与验收边界**未变**。Iteration 001 的**后续**计划变更（纳入 T17 / ISS35 承载、调整后继 Cycle 顺序为 `001-slot-count-underflow` → `002-catalog-bounded-read` → `003-recovery-bounded-read`）记录在 change `tasks.md` 的 Iteration 001 计划与后继 Cycle `001-slot-count-underflow.md`（Cycle Type: `replan`）中。**（2026-10-25 结构修正，见 `design.md` D14：上文 `001-slot-count-underflow` / `002-catalog-bounded-read` / `003-recovery-bounded-read` 为**当时的 Cycle 顺序，已失效**；后继承载单元由「同一 Iteration 内的本地 Cycle 序列」改为「每组各自一个 Iteration」——Iteration 002 = T17 / ISS35、Iteration 003 = T4、Iteration 004 = T5；本 Cycle 的目标、范围、依赖、验证方式与验收边界未变。**2026-10-25 二次结构修正见 `design.md` D15：写路径组（T18 / T19）插入为 **Iteration 003** ⇒ **Iteration 003 = 写路径组、Iteration 004 = T4、Iteration 005 = T5**；本 Cycle 的目标、范围、依赖、验证方式与验收边界仍未变。）**

**Next Cycle**

`openspec/changes/2026-10-25-ms37-defect-ledger-closure/iterations/002-slot-count-underflow/000-initial.md`（T17 / ISS35 承载，Cycle Type = `initial`）与 `.../003-catalog-bounded-read/000-initial.md`（T4 目录页组）。二者是 **Iteration 002 / 003** 各自的唯一 Cycle（非本 Cycle 的 rework / replan）；**执行顺序**：先 `002-slot-count-underflow`（Iteration 002），再 `003-catalog-bounded-read`（Iteration 003）。**（2026-10-25 结构修正，见 `design.md` D14；原记录为同目录的 `001-slot-count-underflow.md` 与 `002-catalog-bounded-read.md`。）** **（2026-10-25 二次结构修正见 `design.md` D15：上句的 `.../003-catalog-bounded-read/000-initial.md` 已失效 —— 目录页组（T4）现为 **Iteration 004**（`.../004-catalog-bounded-read/000-initial.md`）；实际后继为 **Iteration 003（写路径的槽界限，T18 / T19，`.../003-write-path-slot-bounds/000-initial.md`）→ Iteration 004（T4 目录页组）→ Iteration 005（T5 恢复面组，尚未创建）**。）**

**Next Iteration**

None —— 本 Cycle 的 Plan Review 判定 `accepted` ⇒ Iteration 001（T3）已完成；Map 中的下一 Iteration = **002**（T17 / ISS35，已展开，Act Response `reported`，待 Plan Review），其后为 003（T4）、004（T5）。**（2026-10-25 结构修正，见 `design.md` D14；原文为「Iteration 001 尚未被接受（T4 / T5 与 ISS35 承载未完成），不得展开 Map 中的下一 Iteration（002）」——该表述基于修正前「Iteration 001 同时承载 T3 / T17 / T4 / T5」的形态。）** **（2026-10-25 二次结构修正见 `design.md` D15：Iteration 002 已于 2026-10-25 被 Plan Review 判定 `accepted`；其后为 **003（写路径组，T18 / T19，已展开，Plan Context `ready`）→ 004（T4）→ 005（T5）**。）**
