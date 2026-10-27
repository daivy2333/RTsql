# Iteration 002 / Cycle 000: 页头槽数越界的读路径 abort（ISS35 承载）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: **T17**（本 Cycle 新增到 change `tasks.md` 的任务；本 Cycle 是 **Iteration 002 的首个 Cycle** ⇒ 本 Cycle 的 Cycle Type 为 `initial`；原记录为「Iteration 001 的计划因纳入 T17 而修订 ⇒ 本 Cycle 的 Cycle Type 为 `replan`」，承载形态的修正见 `design.md` D14）
- Acceptance gaps: None（本 Cycle 执行的是**新增**的 T17，不是既有 Acceptance 的返工）
- Repair items: None
- Inherited scope:
  - delta spec `page-read-bounded-read` 的 Requirement 首段（槽负载越界不产生越界切片）与「数据页上的损坏槽在读 / 写路径给出明确的错误」场景 —— **已由 Iteration 001 / Cycle 000（T3）交付并被 Review 接受**，本 Cycle **不改动**其语义；
  - T3 已交付并**冻结**的产品原语：`SlottedPageRef::get_slot_data_checked`（`src/storage/page_format/slotted_page.rs:137-144`）与 `StorageError::CorruptedDataPage { page_id, detail }`（`src/storage/error.rs`）—— 本 Cycle **复用**它们，**不得重复实现**；
  - T3 已交付并冻结的逐点「过短负载」语义（#2–#8 / #11）—— 本 Cycle **不得**改动；
  - ISS35 台账：`.agents/issues/ISS35-slot-count-underflow-aborts-read-path.md`（**需求来源**，只读；台账的关闭由 Recorder 按用户指令执行，本 Cycle 不改台账文件）。
- Excluded scope:
  - 目录页组（`src/storage/catalog.rs` 5 处）—— 归 **T4**（Iteration 003）；
  - 恢复面组（`src/wal/recovery.rs` 4 处）—— 归 **T5**（Iteration 004）；
  - 可写变体的**负载偏移**无界写（`src/storage/data_page.rs:237` 的 `page.data[slot_offset..slot_offset + VersionHeader::SIZE]`）—— 属 T3 的**槽负载**形态在写路径上的相邻面，**不在 ISS35 台账的承载面内**（台账明确「本条只主张读路径」）⇒ 本 Cycle **不实施**，作为 Issue 候选上报；
  - 索引树页的既有页级闸门 `ensure_index_page_intact`、索引树访问器、数据页的页级判据 `check_data_page` / `check_tree_page` / `validate_slots`、磁盘格式、dump / restore 合同面；
  - 全量回归（整个 change 的全量预算在 Iteration 008 / T16）。
  - **（2026-10-25 二次结构修正见 `design.md` D15：写路径组（T18 / T19）插入为 Iteration 003，其后依次顺延 ⇒ 上列 T4 现为 **Iteration 004**、T5 现为 **Iteration 005**；全量预算现为 **Iteration 009 / T16**。）**

**Objective**

数据页组在**页头声称的槽数超出槽目录容量**时不再因槽目录定位下溢而 abort：读 / 写路径在**取槽之前**识别该页头损坏，返回点名该页与页头声称槽数的明确错误；槽目录容量界限在全项目**只有一处定义**（页格式层），索引树侧的既有判据与取值**零变化**。

**Background**

- **缺陷来源**：ISS35 台账（`.agents/issues/ISS35-slot-count-underflow-aborts-read-path.md`）。根因 `Confirmed（读码确认）`：`SlottedPageRef::get_slot`（`src/storage/page_format/slotted_page.rs:96-110`）与可写变体 `SlottedPage::get_slot`（`:183-201`）用 `let slot_start = Page::PAGE_SIZE - (index + 1) * Slot::SIZE` 直接算槽目录项位置，**无下溢保护**；读路径的迭代上界取自页头的 `slot_count` ⇒ 页头 `slot_count ≥ 683` 时该减法下溢 ⇒ 切片越界 panic ⇒ 进程 abort。
- **本 Cycle 为何排在 T4 / T5 之前（执行正确性）**：T4 / T5 的读点同样经由 `get_slot` 取得 `&Slot`。若 ISS35 的修复改变 `get_slot` 的形状（返回 `Option` / `Result` 或新增界限），T4 / T5 必须写在最终形状上，否则返工。本 Cycle 先把「页级槽数判据 + 统一容量界限」的形状**冻结**，T4 / T5 在该形状上实施各自的槽负载有界读。
- **Plan 对 ISS35 台账三个待判问题的裁定**（记录在 `design.md` D12）：
  1. **修复位置** —— 页格式层提供**唯一的**槽目录容量界限（`MAX_PAGE_SLOTS` 上移到 `src/storage/page_format/slotted_page.rs`，`src/storage/btree/node.rs` 改为引用，**值与判据不变** ⇒ 索引树路径零回归）+ **页级判据**（页头声称的槽数是否超出该界限），由**读点在取槽之前**消费。**不改 `get_slot` 的 `None` 语义**：原语层把越界索引折叠成 `None` 会与「槽不存在」不可区分 ⇒ 在尚未纳入判据的读点上会把 abort 变成**静默跳过**（违反「跳过 = 静默错读」）。
  2. **损坏页在该页型的正确行为** —— 数据页组 ⇒ 明确的 `StorageError::CorruptedDataPage`（点名该页与页头声称的槽数）；目录页组 ⇒ 明确错误 ⇒ 该库 SHALL NOT 打开（**沿用 T4 已定的裁定**，本 Cycle 不改）；恢复面组 ⇒ 按 T5 已定的两类语义（**T5 尚未展开**）。
  3. **是否把可写变体与 `add_slot_inner` 的同形空间检查纳入同一 Iteration** —— **不纳入**：ISS35 台账明确「本条只主张读路径」；写路径的同形无界访问需要独立方向 ⇒ 作为 Issue 候选上报，不登记。

**Investigation Facts**

- Starting point:
  - 工作区 HEAD `e9cb9d3`；本 Cycle 的前置状态 = Iteration 001 / Cycle 000（T3）**已被 Plan Review 接受**（`000-initial.md` 的 `Review Result: accepted`）。
  - T3 已交付并在 Cycle 000 的 Plan Review 中被独立核对通过的内容：`get_slot_data_checked`（槽负载越界判界）、`StorageError::CorruptedDataPage`、数据页组 11 个读点的逐点替换、`src/pipeline.rs` 的页损坏错误上浮。
  - **变更前的验证结果（沿用，未失效）**：Cycle 000 的 Act Response 记录 RED 5 例逐个单独运行 SIGABRT（`src/storage/page_format/slotted_page.rs:128:19: range start index 65535 out of range for slice of length 4096`）、GREEN `cargo test --test data_page_bounded_read_test` 6 passed / 0 failed、`cargo test --lib` 420 passed / 0 failed、定向回归 20 目标 2 批串行 354 passed / 0 failed（来源：Cycle 000 的 Act Response「Verification Evidence」第 1–8 项）。⇒ **本 Cycle 的 RED 只针对**页头槽数越界这一**新**形态（T3 的负载越界形态已修复，不在本 Cycle 的 RED 范围）。
  - 已知限制：`get_slot` / `SlottedPage::get_slot` 的槽目录定位**仍无下溢保护**；页级判据 `integrity::validate_slots` 只在**释放 / 重建 / `rtsql check`** 面被消费，读路径**不**消费它。
- Current-State Evidence（本 Cycle 已核对的事实，供 Act 直接使用）：
  - 常量与容量：`Page::PAGE_SIZE = 4096`（`src/storage/page.rs:11`）、`SlottedPageHeader::SIZE = 16`（`src/storage/page_format/slotted_page.rs:27`）、`Slot::SIZE = 6`（`:12`）⇒ `MAX_PAGE_SLOTS = (4096 - 16) / 6 = 680`。
  - **既有界限只有一处，且只在索引树侧**：`src/storage/btree/node.rs:11-14` 的 `const MAX_PAGE_SLOTS: usize = (Page::PAGE_SIZE - SlottedPageHeader::SIZE) / Slot::SIZE;`（注释已记载「页头声称的槽数超过该上限时，槽目录定位会下溢成越界切片」），在 `bounded_slot_data_ref`（`:32`）/ `bounded_slot_data_mut`（`:44`）中于 `get_slot` **之前**判断。
  - **既有页级判据与同一阈值**：`src/storage/integrity.rs` 的 `validate_slots` 在 `slot_area_start < SlottedPageHeader::SIZE`（即 `slot_count ≥ 681`）时报 violation `slot count {n} exceeds the page capacity`；`validate_slots` 返回 `None` 后**不再**进入 `page.get_slot(i)` 循环（`:558-564`）⇒ **完整性扫描面不受**本形态影响。
  - **索引树路径不受影响**：`src/storage/btree/node.rs:843` 的 `SlottedPage::get_slot`（可写变体）在索引树**页级闸门** `ensure_index_page_intact` 之后；闸门经 `integrity::check_tree_page` ⇒ `validate_slots` 先行 ⇒ 损坏页在闸门处即被拒 ⇒ 索引树读 / 写路径**零暴露**。
  - **数据页组 12 个槽获取点**（`grep -n "\.get_slot\(|\.get_slot_by_logical_id\(" src/` 的实际行号；前 11 个 = T3 的 11 个读点，第 12 个 = 可写变体的写路径）：

    | # | 位置 | 取槽方式 | 所在闭包 / 函数的返回类型 | T3 的处置 |
    |---|---|---|---|---|
    | 1 | `src/storage/data_page.rs:165`（`read_tuple_from_data_page`） | `get_slot_by_logical_id` | `storage::Result` | 越界槽 ⇒ `CorruptedDataPage` |
    | 2 | `src/storage/data_page.rs:232`（`update_version_header_in_data_page`） | `get_slot_by_logical_id`（**可写** `SlottedPage`） | `std::result::Result<(), String>` | **不在 T3 范围**（写路径） |
    | 3 | `src/storage/data_page.rs:263`（`delete_tuple_from_data_page` 探测） | `get_slot_by_logical_id` | `storage::Result` | 越界槽 ⇒ `CorruptedDataPage` |
    | 4 | `src/storage/data/table_manager.rs:1001` | `get_slot(index)` 循环 | `storage::Result` | 越界槽 ⇒ `CorruptedDataPage` |
    | 5 | `src/storage/data/table_manager.rs:1258` | `get_slot(index)` 循环 | `storage::Result` | 越界槽 ⇒ `CorruptedDataPage` |
    | 6 | `src/executor/data_scan.rs:254` | `get_slot(index)` 循环 | `storage::Result` | 越界槽 ⇒ `CorruptedDataPage` |
    | 7 | `src/executor/data_scan.rs:476` | `get_slot(slot_index)` + `.expect("slot_index < slot_count validated above")` | `storage::Result` | 越界槽 ⇒ `CorruptedDataPage` |
    | 8 | `src/executor/create_index.rs:132` | `get_slot(index)` 循环 | `storage::Result` | 越界槽 ⇒ `CorruptedDataPage` |
    | 9 | `src/transaction/manager.rs:567` | `get_slot(index)` 循环 | `storage::Result` | 越界槽 ⇒ `CorruptedDataPage` |
    | 10 | `src/storage/buffer_pool.rs:425` | `get_slot_by_logical_id` | `storage::Result` | 越界槽 ⇒ `CorruptedDataPage` |
    | 11 | `src/storage/buffer_pool.rs:480` | `get_slot_by_logical_id` | `storage::Result` | 越界槽 ⇒ `CorruptedDataPage` |
    | 12 | `src/storage/buffer_pool.rs:633`（`check_page_all_visible`） | `get_slot(i)` 循环 | `storage::Result`（布尔辅助） | 越界槽 ⇒ **降级为 `false`**（安全值） |

  - **abort 的实际可达性（读码结论）**：`get_slot_by_logical_id`（`slotted_page.rs:113-122`）在**找到目标槽时提前返回** ⇒ 只有「目标行不在该页」时才会遍历到 `slot_count` 的末端 ⇒ 页头槽数越界时，**查一个不存在的行**就会走到 `get_slot(index ≥ 682)` ⇒ abort。而按 `slot_count` **全量遍历**的读点（#4 / #5 / #6 / #8 / #9 / #12）在页头槽数越界时**必然**遍历到越界索引 ⇒ 必然 abort。
  - `src/executor/data_scan.rs:476-477` 的 `.expect("slot_index < slot_count validated above")` —— 页级判据生效后 `slot_count ≤ 680`，`slot_index < slot_count` 恒成立 ⇒ 该 `expect` **不再**可能被页头损坏触发（当前它是被 `get_slot` 内部的 abort 先一步打断）。
  - **测试入口**：`tests/data_page_bounded_read_test.rs`（T3 新增，6 个用例，真实文件库 + 真实 `Database::open` + 真实 SQL 入口 + 真实文件字节改写夹具）。损坏注入的字节算术（Act 已验证可用）：文件偏移 = `HEADER_SIZE(64) + page_id * PAGE_SIZE(4096) + (4096 - (slot_index+1) * Slot::SIZE(6) + 2)`（该项的 offset 字段）；**页头 `slot_count` 字段**位于页内偏移 **1..3**（`SlottedPageHeader::deserialize` 读 `buf[1..3]`）⇒ 文件偏移 = `HEADER_SIZE(64) + page_id * PAGE_SIZE(4096) + 1`，写入 u16 小端。
  - **既有回归目标集合**（与 Cycle 000 的 Verification 清单**完全一致**，20 个）：`storage_test`、`full_table_dml_test`、`table_manager_test`、`alter_table_test`、`executor_test`、`secondary_index_test`、`visibility_test`、`mvcc_abort_test`、`mvcc_commit_test`、`explicit_tx_test`、`overflow_page_test`、`ddl_rewrite_consistency_test`、`drop_table_free_test`、`free_page_lifecycle_test`、`planner_test`、`plan_exec_test`、`ddl_catalog_transactional_test`、`index_manager_test`、`version_chain_test`、`e2e_test`。
- Code and Critical Path:
  - 入口：`Database::open` → `TableManager::open_or_init`（`src/storage/data/table_manager.rs:373-374`）→ `Catalog::scan_tables` → `scan_chain`（目录页，**本 Cycle 不改**）；数据页的读取入口 = `BufferPool::read_tuple` / `read_version_header` → `read_tuple_from_data_page`；扫描入口 = `src/executor/data_scan.rs` 的扫描循环。
  - 页格式层：`src/storage/page_format/slotted_page.rs` —— `SlottedPageRef`（只读，`new(data)`）与 `SlottedPage`（可写，`new(&mut Page)`）；两者各有一份 `slot_count()` / `get_slot()` / `get_slot_by_logical_id()`；`page_format/mod.rs:9` 统一重导出。
  - 错误面：`src/storage/error.rs` 的 `StorageError`（T3 新增的 `CorruptedDataPage { page_id: u64, detail: String }`）；`src/storage/mod.rs:31-34` 重导出 `data_page` 的错误构造器。
  - 状态与并发：页级判据是**纯读**判据（读页头的一个 u16），不改变任何状态所有权、不引入锁或新依赖；每个读点在**同一页缓冲**内先判据再取槽，判定与取槽之间无 await ⇒ 无竞态窗口。

**Implementation Guidance**

- 建议顺序：① 页格式层加 `MAX_PAGE_SLOTS`（从 `btree/node.rs` 迁移，值不变）与页级判据 → ② `btree/node.rs` 改为引用该常量（**判据代码逐字保留**）→ ③ `data_page.rs` 加错误构造器并经 `storage/mod.rs` 重导出 → ④ 12 个取槽点逐点插入「取槽前」的页级判据（#12 降级为 `false`）→ ⑤ 测试：先行用例（RED → GREEN）。
- 关键取舍：
  - **判据的位置**：判据必须**在取槽之前**问，不能放在取槽之后（取槽即 abort）。因此判据放在「拿到页缓冲、构造 `SlottedPageRef` / `SlottedPage` 之后，第一次取槽之前」。
  - **判据的唯一实现**：容量界限与「是否超容量」的判定**只写一次**（页格式层）；`btree/node.rs` 改为引用同一常量，**不复制**第二份判定。
  - **阈值口径**：判据用 `slot_count > MAX_PAGE_SLOTS`（= `slot_count ≥ 681`）⇒ 与 `integrity::validate_slots` 的 `slot count exceeds the page capacity` violation **同一阈值**（该阈值**严于** abort 阈值 683 ⇒ 同时覆盖「槽目录压到页头」与「定位下溢」两种形态）。
  - **#2（可写变体）的错误类型**：该点当前闭包返回 `Result<(), String>`，外层 `result.map_err(|_| StorageError::SlotNotFound(row_id))` 会把**任何**错误折叠成 `SlotNotFound` ⇒ 要让页损坏错误到达使用者，闭包返回类型须改为 `storage::Result<()>`（`PageGuard::modify_page` 对返回类型泛型，`src/storage/page_frame.rs:89-96`），并把「槽不存在」这一既有语义**原样**映射为 `StorageError::SlotNotFound(row_id)`。
  - **不得**在本次改动中把 `get_slot` 的 `None` 语义改成「越界 ⇒ `None`」：那会让**尚未**纳入判据的目录页组 / 恢复面组把 abort 变成静默跳过（违反「跳过 = 静默错读」）。`get_slot` 本 Cycle **只允许**被读取，不得被改写。

**Task Definitions**

### T17: 页头槽数越界的数据页读 / 写路径给出明确的错误，槽目录容量界限全项目唯一

- Requirement/Scenario:
  - `page-read-bounded-read` Requirement 首段（有界读、不产生越界切片、不 abort）；
  - `page-read-bounded-read` Scenario「数据页上的损坏槽在读 / 写路径给出明确的错误」（**扩展**：本任务覆盖的是页头槽数越界这一**页级**损坏形态，与该场景的槽负载越界形态**并列**，不替换）；
  - ISS35 台账 `.agents/issues/ISS35-slot-count-underflow-aborts-read-path.md`（需求来源）。
- Depends on: Iteration 001 / Cycle 000（T3）—— 其交付的 `get_slot_data_checked` 与 `CorruptedDataPage` 是本任务复用的冻结原语（Cycle 000 的 `Review Result: accepted`）。
- Targets:
  - `src/storage/page_format/slotted_page.rs` —— 新增 `pub const MAX_PAGE_SLOTS`（**从 `src/storage/btree/node.rs` 迁移**，值不变）+ 页级判据（页头声称的槽数是否超出该界限；**只读变体 `SlottedPageRef` 与可写变体 `SlottedPage` 都要能直接问出**，判定实现**只有一处**）。
  - `src/storage/page_format/mod.rs` —— 重导出 `MAX_PAGE_SLOTS`（`pub use slotted_page::{...}` 一行）。
  - `src/storage/btree/node.rs` —— 删除本地 `const MAX_PAGE_SLOTS`（`:11-14`），改为从 `crate::storage::page_format` 导入；`bounded_slot_data_ref` / `bounded_slot_data_mut` 的判据**逐字保留**。
  - `src/storage/data_page.rs` —— 新增错误构造器（点名该页与页头声称的槽数与容量上限）；`src/storage/mod.rs` 重导出。
  - 数据页组 12 个取槽点（见 Current-State Evidence 的表）：`src/storage/data_page.rs:165`、`:232`、`:263`、`src/storage/data/table_manager.rs:1001`、`:1258`、`src/executor/data_scan.rs:254`、`:476`、`src/executor/create_index.rs:132`、`src/transaction/manager.rs:567`、`src/storage/buffer_pool.rs:425`、`:480`、`:633`。
  - `tests/data_page_bounded_read_test.rs` —— 新增先行用例（页头槽数越界）。
- Current behavior: 数据页组在页头 `slot_count ≥ 683` 时，取槽的槽目录定位 `Page::PAGE_SIZE - (index + 1) * Slot::SIZE` **整数下溢** ⇒ 切片越界 panic ⇒ **进程 abort**（debug profile 文案 `attempt to subtract with overflow`；release profile 文案 `range start index N out of range for slice of length 4096`）；按 `slot_count` 全量遍历的读点（#4 / #5 / #6 / #8 / #9 / #12）**必然**触发，查不存在行的读点（#1 / #2 / #3 / #10 / #11）在「找不到目标槽」时触发。索引树侧**已有** `MAX_PAGE_SLOTS` 前置界限 ⇒ 同一形态在索引树路径上**已被拦截**（页级闸门 + 访问器界限），数据页组**没有**。
- Required behavior:
  - 数据页组的 12 个取槽点在**取槽之前**问页级判据：页头声称的槽数超出槽目录容量 ⇒ 该页头损坏 ⇒ **明确错误** `StorageError::CorruptedDataPage { page_id, detail }`，`detail` 点名该页与页头声称的槽数与容量上限（如 `slot count 700 exceeds the page slot directory capacity of 680 slots`）；
  - 该形态**不得**产生任何 panic / abort / 越界切片；**不得**被当作「没有这一行」而静默跳过；
  - #12 `check_page_all_visible`（布尔辅助）遇该形态 ⇒ **降级为安全值 `false`**（与 T3 的 A2 同形）⇒ 错误由随后的实际取行处（#10 / #11）抛出；
  - 槽目录容量界限**全项目只有一处定义**（页格式层）；索引树侧的取值与判据**零变化**；
  - 未损坏路径行为**零变化**（含 T3 已冻结的「过短负载」逐点语义）。
- Preserve:
  - `get_slot_data_checked` / `slot_payload_in_bounds` 的判据与语义（T3 已接受）；
  - `StorageError::CorruptedDataPage` 的变体形状（`page_id` + `detail`）与既有文案前缀 `corrupted data page {page_id}: `；
  - `SlottedPageRef::get_slot` / `SlottedPage::get_slot` / `get_slot_by_logical_id` 的**签名与 `None` 语义**（本 Cycle 只读不改）；
  - `SlottedPageRef::get_slot_data` 的签名与既有单元测试；
  - 索引树页级闸门 `ensure_index_page_intact`、`integrity::validate_slots` / `check_data_page` / `check_tree_page`；
  - T3 的逐点「过短负载」语义（#2–#8 / #11）与 `src/pipeline.rs` 的「表不存在 ⇒ `false`」；
  - `check_page_all_visible` 在健康页上仍返回「全可见」。
- Forbidden:
  - 改动 `src/storage/catalog.rs`（目录页组 → T4）、`src/wal/**`（恢复面组 → T5）、`src/storage/integrity.rs`、`src/storage/btree/**` 的**判据逻辑**（只允许常量迁移）；
  - 改动 `get_slot` / `get_slot_by_logical_id` 的签名或把越界折叠成 `None`；
  - 把「过短负载」在 #2–#8 / #11 一并改成报错；
  - 新增错误变体（复用 `CorruptedDataPage`）、新增命令参数 / 配置项 / 缓存 / 闸门机制 / 判定层 / 身份证明机制；
  - 修复 `src/storage/data_page.rs:237` 的**负载偏移**无界写（可写变体的相邻面 ⇒ Issue 候选）；
  - 全量回归。
- First test:
  - 位置：`tests/data_page_bounded_read_test.rs`（**新增 1 个用例**，沿用该文件既有的真实文件库 + 真实 `Database::open` + 真实 SQL 入口 + 真实字节改写夹具）。
  - **RED（变更前）**：夹具建库 → 建表 `t` → 插入 ≥ 2 行 → 取 `db.table_manager.get_table("t").await?.data_page_head` 得到数据页号 → 关闭库 → 按文件偏移 `HEADER_SIZE(64) + page_id * PAGE_SIZE(4096) + 1` 把**页头 `slot_count` 字段**（页内偏移 1..3）写成 u16 小端 `700` ⇒ 重开库 ⇒ 执行 `SELECT … FROM t`（**全表扫描**与**点查**各一）。
    ⇒ **预期 RED：进程 panic / abort**（debug profile：`attempt to subtract with overflow`，位置 `src/storage/page_format/slotted_page.rs:100` 或 `:189`；release profile：`range start index N out of range for slice of length 4096`，位置 `:101` 或 `:190`）—— 即语句**没有**返回命名错误。
    ⇒ 该夹具的**未损坏对照**沿用该文件既有的 `undamaged_data_page_reads_and_writes_are_unchanged`（同步骤但不改字节 ⇒ 变更前即 GREEN）。
  - **可写变体取槽点（#2）的覆盖**：该点的 abort 需要「指向**不存在**行的陈旧 `row_id`」才会触发（`get_slot_by_logical_id` 找到目标即提前返回）⇒ 无法用真实 SQL 入口稳定触发 ⇒ 允许在**项目测试框架内**以存储层用例覆盖（真实 `BufferPool` + 真实文件，调用 `update_version_header_in_data_page` 传入该页上不存在的 `slot_id`）⇒ 断言**不 abort** 且错误**点名该页**；不得为此新增判定层、专用命令或独立于项目测试框架的脚本。
- GREEN condition: 上述 RED 用例变为「语句返回 `CorruptedDataPage` 形态的错误，文案含 `corrupted data page {page_id}` 且点名页头声称的槽数与容量上限，**进程不 abort**」；未损坏对照结果不变；`cargo test --test data_page_bounded_read_test`、`cargo test --lib` 与本 Cycle 的定向回归清单全绿；`cargo fmt --check` / `cargo clippy --all-targets` 在**本 Cycle 触碰的文件**上零新增 finding。
- Verification:
  - 行为类（真实入口）：`cargo test --test data_page_bounded_read_test -- --test-threads=1` —— 覆盖「页头槽数越界 ⇒ 命名错误」「可写变体取槽点 ⇒ 不 abort」「未损坏零变化」；损坏注入必须是**真实文件字节改写**，观察对象是语句返回的错误与进程是否存活。
  - 单元与库面：`cargo test --lib` —— 覆盖 `page_format` 既有单元测试与索引树路径（常量迁移的零回归证据）。
  - 定向回归（20 个目标，**M19 分批串行纪律**：批内 `--test-threads=1`、批间串行、批 1 全绿后才启动批 2；**目标集合与 Cycle 000 的 Verification 清单完全一致**）：
    - 批 1（10 个）：`cargo test --test storage_test --test full_table_dml_test --test table_manager_test --test alter_table_test --test executor_test --test secondary_index_test --test visibility_test --test mvcc_abort_test --test mvcc_commit_test --test explicit_tx_test -- --test-threads=1`
    - 批 2（10 个）：`cargo test --test overflow_page_test --test ddl_rewrite_consistency_test --test drop_table_free_test --test free_page_lifecycle_test --test planner_test --test plan_exec_test --test ddl_catalog_transactional_test --test index_manager_test --test version_chain_test --test e2e_test -- --test-threads=1`
    - 通过条件：两批全部 0 failed（合计 354 passed / 0 failed 与 Cycle 000 的基线一致）。
  - **本 Cycle 不跑全量回归**（整个 change 的收尾全量归 Iteration 008 / T16；M19：目标名字典序分组、每批 8–12 个目标、批内 `--test-threads=1`、批间串行、全量 Gate = 全部批次各自 0 failed）。
  - 失败含义：任何一条失败 ⇒ 对应 Acceptance 未满足 ⇒ 停在当前 task（不进入下游 task）。
- Stop when: 需要改动 `src/storage/catalog.rs` / `src/wal/**` / `src/storage/integrity.rs`；需要改写 `get_slot` / `get_slot_by_logical_id` 的签名或 `None` 语义；需要新增错误变体或新行为；需要修复 `src/storage/data_page.rs:237` 的负载偏移无界写；或需要全量回归才能判定（应回到 Plan 调整验证边界）。

**Invariants**

- 未损坏路径上，行集合、写入结果、可见性判定、事务版本收集、提交 / 回滚的版本头改写与既有行为**逐点一致**。
- 索引树读 / 写路径的行为**逐字节不变**（常量迁移只改定义位置，取值 680 与判据表达式不变）。
- 目录页组与恢复面组**本 Cycle 不动**（其行为与 Cycle 000 结束时一致）。
- 不引入身份证明机制、判定层、裁决脚本、新命令参数、新配置项、缓存或闸门机制。

**Non-goals**

- 目录页组（T4）与恢复面组（T5）。
- 可写变体的负载偏移无界写（`src/storage/data_page.rs:237`）—— Issue 候选，不在本 Cycle。
- 数据页的**页级**判据扩展（例如把「过短负载」纳入 `check_data_page`）、损坏页的修复 / 重建 / 数据抢救、`rtsql check` 的输出形态与诊断分级。
- 磁盘格式、dump / restore 合同面、索引树访问器与页级闸门。
- 全量回归（Iteration 008 / T16）。

**Acceptance**

| # | 验收条件 | 需求 / 场景 | 设计 | 任务 | 代码位置 | 测试 |
|---|---|---|---|---|---|---|
| A6 | 数据页上的页头槽数越界在读 / 写路径上不 panic、不 abort，返回点名该页与页头声称槽数的明确错误，不被当作「没有这一行」静默跳过 | `page-read-bounded-read` / 「数据页上的损坏槽在读 / 写路径给出明确的错误」（页级形态）+ Requirement 首段 | D12 | T17 | `src/storage/page_format/slotted_page.rs`、`src/storage/data_page.rs`、`src/storage/data/table_manager.rs`、`src/executor/data_scan.rs`、`src/executor/create_index.rs`、`src/transaction/manager.rs`、`src/storage/buffer_pool.rs` | `tests/data_page_bounded_read_test.rs`（页头槽数越界用例 + 可写变体取槽点用例） |
| A7 | 槽目录容量界限全项目**只有一处定义**（页格式层），索引树侧的取值与判据零变化 | ISS35 台账「已知但只在一侧有防护」 | D12 | T17 | `src/storage/page_format/slotted_page.rs`、`src/storage/btree/node.rs` | `cargo test --lib`（含 `page_format` 与索引树单元测试）+ 定向回归中的 `index_manager_test` / `secondary_index_test` |
| A8 | 未损坏路径行为零变化（含 T3 已冻结的「过短负载」逐点语义与 `check_page_all_visible` 的健康页「全可见」） | `page-read-bounded-read` / 「未损坏路径的行为零变化」 | D7 + D12 | T17 | 12 个取槽点 | `tests/data_page_bounded_read_test.rs`（未损坏对照）+ 定向回归清单 |

**Verification**

- 逐 scenario 的最简直接判定（行为类 scenario 走**真实入口**：真实建库 → 真实写库 → 真实改坏磁盘字节 → 真实重开 → 真实执行语句；观察语句返回的错误与进程是否存活）：
  1. 「页头槽数越界 ⇒ 明确的错误」：`cargo test --test data_page_bounded_read_test` 的页头槽数越界用例 ⇒ 通过依据 = 语句返回的错误文案含 `corrupted data page {page_id}` 且点名页头声称的槽数与容量上限，**测试进程未 panic / 未 abort**。
  2. 「可写变体取槽点 ⇒ 不 abort」：同文件（或同项目测试框架内）的存储层用例 ⇒ 通过依据 = 调用返回错误而非 panic，且错误点名该页。
  3. 「未损坏路径的行为零变化」：同文件的未损坏对照 + 定向回归清单 ⇒ 通过依据 = 全部 0 failed。
  4. 「容量界限唯一且索引树零回归」：`cargo test --lib` + 定向回归中的索引树目标 ⇒ 通过依据 = 全部 0 failed，且 `src/storage/btree/node.rs` 中不再存在第二个 `MAX_PAGE_SLOTS` 定义（读码核对）。
- **不建判定层**：判定只落在 `cargo test` 的断言与退出码；不新增脚本 / 封装命令 / 专用参数；不使用 Hash / 指纹 / revision pin / run-id / manifest / 时间证明。
- **验证边界**：本 Cycle 跑上列定向回归，**不跑全量**（整个 change 的全量预算在 Iteration 008 / T16）。

**Gate 2 Readiness**

- Step 5 逐项复核：TBD / TODO = 无；需求裁剪 = 无（ISS35 台账的三个待判问题已由 Plan 裁定并记录在 `design.md` D12，`Simplified` = 0）；tasks / specs / design 一致 = 一致（T17 已写入 change `tasks.md` 并归属 Iteration 002；delta spec 新增页级形态场景；design D12 记录裁定与排除面）；Current-State Evidence 全部来自当前工作区的逐行核对（`grep` 实测行号、常量取值、`validate_slots` 阈值、索引树闸门位置、`PageGuard::modify_page` 泛型签名、`slotted_page.rs` 的 `get_slot` / `get_slot_by_logical_id` 实现）；每个 task 的执行依据完整；调用者、影响范围、测试入口已定位（12 个取槽点 + 20 个回归目标 + 1 个测试文件）；未修改无关范围；Iteration Plan 覆盖全部任务且高开销验证预算明确（全量归 Iteration 008 / T16）；**Act 只需读本 Cycle 文件即可建立第一个先行测试并执行**（字节算术、页号取得方式、预期 RED 形态（两种 profile 文案）、错误文案形态均已写明，无需回读前序 Cycle / 其它规划资料 / 重新调查）；非实质未知项已留在 Risks and Notes；验证直接证明目标行为，行为类 scenario 走真实入口，无身份证明机制与判定层。⇒ **全部 PASS**。
- RTM：`page-read-bounded-read` Requirement + 「数据页上的损坏槽在读 / 写路径给出明确的错误」场景（页级形态）+ ISS35 台账 ⇒ T17 ⇒ Covered（无 `Missing`、无 `Simplified`）。
- 设计闭合：行为差异、接口（`MAX_PAGE_SLOTS` + 页级判据 + `CorruptedDataPage`）、错误语义（点名该页与页头声称的槽数）、12 个取槽点的处置均已定稿 ⇒ 无 TBD。
- 一致性：本 Cycle 与 change 的 `tasks.md`（T17 归属 Iteration 002）、`specs/page-read-bounded-read/spec.md`、`design.md` D12 一致。
- Persisted Evidence：`none`（见下）。
- 用户批准：本 goal 指令（「按照流程把最新规划的这一批ms做完……中途发现并登记的issue直接在那个change分配iter进行解决……开始吧」）构成 Gate 1 / Gate 2 的批准授权；**批准不降低判据**，上述各项仍逐条检查并记录。

**Cycle RTM**

| Requirement / Scenario | Task | First Test（计划名） | Status |
|---|---|---|---|
| PRBR Requirement 首段（有界读、不产生越界切片、不 abort）+ ISS35 台账（页头槽数越界形态） | T17 | `tests/data_page_bounded_read_test.rs::…`（计划名：页头槽数越界 ⇒ 命名错误；Act 可改名并记录实际名） | Covered |
| PRBR Scenario「数据页上的损坏槽在读 / 写路径给出明确的错误」（页级形态扩展） | T17 | 同上 + 可写变体取槽点用例 | Covered |
| ISS35 台账「容量界限只在一侧有防护」⇒ 统一为页格式层唯一 | T17 | `cargo test --lib` + 定向回归（`index_manager_test` / `secondary_index_test`） | Covered |
| PRBR Scenario「未损坏路径的行为零变化」 | T17 | `tests/data_page_bounded_read_test.rs::…`（未损坏对照）+ 定向回归清单 | Covered |
| PRBR Scenario「目录页上的损坏槽给出明确的错误」 | T4（Iteration 003） | 计划名随 T4 的 Cycle 定稿 | Covered（归属 T4） |
| PRBR Scenario「恢复面上的损坏槽按恢复类别分别处置」 | T5（Iteration 004） | 计划名随 T5 的 Cycle 定稿 | Covered（归属 T5） |

**（2026-10-25 二次结构修正见 `design.md` D15：上表 T4 现为 Iteration **004**、T5 现为 Iteration **005** —— 写路径组插入为 Iteration 003，其后依次顺延。）**

**Persisted Evidence**

- Mode: none

全部结论可低成本重跑（1 个测试目标 + `cargo test --lib` + 20 个既有回归目标），命令与实际结果写入 Act Response 即可承载；不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ 不创建 `evidence/` 目录，不预建占位目录。

**Risks and Notes**

- **既有基线条件（非本 Cycle 引入，不纳入本 change）**：项目无 `rust-toolchain.toml`；HEAD（`e9cb9d3`）上 `cargo fmt --check` 报 **50 处 diff 分布在 9 个既有文件**，`cargo clippy --all-targets -- -D warnings` 报 **17 条 lib 错误**（deny 模式下 lib 即中止，test 目标未被 lint）、非 deny 模式 **32 条 warning** ⇒ 基线在 HEAD 即红。本 Cycle **只核对本 Cycle 触碰的文件**零新增 finding，**不重排**无关文件（重排违反 Surgical Changes）⇒ 该排除已记录在 `design.md` D13（ISS36）。
- **`get_slot` 仍是原语级隐患（已裁定不本 Cycle 改）**：本 Cycle 不改 `get_slot` 的 `None` 语义 ⇒ **任何新增的**按页头 `slot_count` 迭代槽的读点，**必须**先问页级判据（`slot_count > MAX_PAGE_SLOTS`），否则会重蹈 abort。该规则写入页格式层的文档注释。
- **目录页组 / 恢复面组的暴露仍在**：本 Cycle 只覆盖数据页组 ⇒ 目录页组与恢复面组在页头槽数越界时**仍然 abort**（**与 Cycle 000 结束时一致，不是本 Cycle 引入的回归**）⇒ 由 T4 / T5 用**本 Cycle 冻结的同一判据**关闭。
- **`src/executor/data_scan.rs:476-477` 的 `.expect`**：页级判据生效后 `slot_count ≤ 680`，该 `expect` 不再可能被页头损坏触发 ⇒ 本 Cycle 不改它（只改其前置的判据）。
- **`src/storage/data_page.rs:232` 的错误类型改动**：闭包返回类型由 `Result<(), String>` 改为 `storage::Result<()>` ⇒ 「槽不存在」的既有语义**必须**仍映射为 `StorageError::SlotNotFound(row_id)`（不得被折叠成别的错误）。
- **Issue 候选（只报告不登记，不在本 Cycle 实施）**：可写变体的同形无界访问 —— `src/storage/data_page.rs:237` 的 `page.data[slot_offset..slot_offset + VersionHeader::SIZE]`（负载偏移越界 ⇒ panic）与 `SlottedPage::get_slot`（`:183-201`）的槽目录下溢；`src/wal/recovery.rs:1519` 从损坏页的槽（`:1487`）取 `logical_id` 后调用 `update_version_header_in_data_page` ⇒ **恢复面可达**该 panic（属 T5 的范围）。
- **跳过项**：本 Cycle 不跑全量回归（change 预算归 Iteration 008 / T16，非跳过而是预算安排）；不做全仓 fmt / clippy 修复（既有基线条件）；不改目录页组 / 恢复面组；不修复负载偏移无界写。

**后续任务边界**

- **授权边界**：无（T17 在 goal 指令授权范围内）。
- **能力边界**：无。
- **停止边界**：本 Cycle 完成且 Plan Review 判定 `accepted` ⇒ 展开 Iteration 003（T4，`iterations/003-catalog-bounded-read/000-initial.md`）；Act 不得自行创建下一 Cycle / 下一 Iteration。
- **（2026-10-25 二次结构修正见 `design.md` D15：上句的 **Iteration 003** 与路径 `iterations/003-catalog-bounded-read/000-initial.md` **已失效** —— 目录页组（T4）现为 **Iteration 004**，其 Cycle 路径为 `iterations/004-catalog-bounded-read/000-initial.md`；本 Cycle 的实际后继顺序为 **Iteration 003（写路径的槽界限，T18 / T19，`iterations/003-write-path-slot-bounds/000-initial.md`）→ Iteration 004（目录页组，T4）**。）**
- **不在本 Cycle、需在后续处理的事项**：T4（目录页组，Iteration 003）、T5（恢复面组，Iteration 004，含 `src/wal/recovery.rs:1519` 的写路径暴露）；ISS25 / ISS28 / ISS29（Iteration 005）；ISS30 / ISS31（Iteration 006）；ISS32 / ISS33（Iteration 007）；ISS34 / I045 / I064 与收尾全量（Iteration 008）。
- **（2026-10-25 二次结构修正见 `design.md` D15：上列迭代号依次顺延一位 —— T4 → **004**、T5 → **005**、ISS25 / ISS28 / ISS29 → **006**、ISS30 / ISS31 → **007**、ISS32 / ISS33 → **008**、ISS34 / I045 / I064 与收尾全量 → **009**；写路径组（T18 / T19）另落在 **Iteration 003**。）**
- **交 Maintainer / Recorder 的事项**：ISS35 台账的**关闭**须等 T4 / T5 完成后数据页组外的两组合齐（由 Recorder 按用户指令执行）；本 Cycle 不登记 Issue、不改全局状态（`.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆均不动）、不合并规格。

## Act Response

- Status: reported

### Implemented（实际改动）

**页格式层：全项目唯一的槽目录容量界限 + 页级判据**

- `src/storage/page_format/slotted_page.rs::MAX_PAGE_SLOTS`（`:15-21`，**从 `src/storage/btree/node.rs` 迁入，值不变 = 680**）：文档注释随之迁移，并补记「全项目唯一定义（页格式层）」。
- `src/storage/page_format/slotted_page.rs::SlottedPageHeader::slot_count_exceeds_capacity`（`:89-91`）：**判定实现只有一处** —— `slot_count > MAX_PAGE_SLOTS`（= `slot_count ≥ 681`），与 `integrity::validate_slots` 的 `slot count exceeds the page capacity` violation **同阈值**。文档注释写入规则：「任何按页头 `slot_count` 迭代槽的读点都必须先问这个判据」。
- `SlottedPageRef::slot_count_exceeds_capacity`（`:121-123`）与 `SlottedPage::slot_count_exceeds_capacity`（`:217-219`）：只读变体与可写变体都能直接问出，**委托**给页头方法 ⇒ 判定仍只有一处。
- `src/storage/page_format/mod.rs:9`：重导出 `MAX_PAGE_SLOTS`。
- `src/storage/btree/node.rs`：删除本地 `const MAX_PAGE_SLOTS`（原 `:11-14`），改为从 `crate::storage::page_format` 导入；`bounded_slot_data_ref` / `bounded_slot_data_mut` 的判据**逐字保留**（`if index >= MAX_PAGE_SLOTS { return None; }` 未动）⇒ **索引树路径取值与判据零变化**。
- `SlottedPageRef::get_slot` / `SlottedPage::get_slot`：**签名与 `None` 语义未动**（只补文档注释，指向页级判据）。

**错误构造器（未新增错误变体）**

- `src/storage/data_page.rs::corrupted_data_page_slot_count_error(page_id, slot_count)`（`:40-55`）：复用 T3 已冻结的 `StorageError::CorruptedDataPage { page_id, detail }`，`detail` = `slot count {n} exceeds the page slot directory capacity of {MAX_PAGE_SLOTS} slots`（点名该页、页头声称的槽数与容量上限）。经 `src/storage/mod.rs:31-35` 重导出。

**12 个数据页组取槽点：取槽之前的页级判据（逐点落地）**

| # | 位置（改动后行号） | 处置 |
|---|---|---|
| 1 | `src/storage/data_page.rs:185` `read_tuple_from_data_page` | `CorruptedDataPage`（页头槽数形态） |
| 2 | `src/storage/data_page.rs:259` `update_version_header_in_data_page`（可写变体） | `CorruptedDataPage`（页头槽数形态） |
| 3 | `src/storage/data_page.rs:301` `delete_tuple_from_data_page` 探测 | `CorruptedDataPage`（页头槽数形态） |
| 4 | `src/storage/data/table_manager.rs:1002` ALTER DROP COLUMN 存活行收集 | `CorruptedDataPage` |
| 5 | `src/storage/data/table_manager.rs:1267` `table_has_live_rows` | `CorruptedDataPage` |
| 6 | `src/executor/data_scan.rs:253` `build_superseded_map` | `CorruptedDataPage` |
| 7 | `src/executor/data_scan.rs:461` 主扫描循环 | `CorruptedDataPage` |
| 8 | `src/executor/create_index.rs:135` CREATE INDEX 回填 | `CorruptedDataPage` |
| 9 | `src/transaction/manager.rs:568` `versions_created_by` | `CorruptedDataPage` |
| 10 | `src/storage/buffer_pool.rs:426` 全可见快路径 | `CorruptedDataPage` |
| 11 | `src/storage/buffer_pool.rs:490` 主取行循环 | `CorruptedDataPage` |
| 12 | `src/storage/buffer_pool.rs:650` `check_page_all_visible`（布尔辅助） | **降级为安全值 `false`**（非「全可见」）⇒ 走慢路径，错误由随后的实际取行处（#10 / #11）抛出 |

**#2 的错误类型改动（Plan 已钉定）**

- `update_version_header_in_data_page` 的闭包返回类型由 `Result<(), String>` 改为 `Result<()>`（`PageGuard::modify_page` 对返回类型泛型）；「槽不存在」**原样**映射为 `StorageError::SlotNotFound(row_id)`（原 `result.map_err(|_| SlotNotFound(row_id))` 的唯一错误来源），**页损坏错误原样上浮**（不再被折叠成 `SlotNotFound`）。

**新增测试**（`tests/data_page_bounded_read_test.rs`，沿用该文件既有的真实文件库 + 真实 `Database::open` + 真实 SQL 入口 + 真实字节改写夹具）

- 夹具辅助 `corrupt_page_slot_count(path, page, slot_count)`：页头 `slot_count` 字段（页内偏移 1..3）⇒ 文件偏移 `HEADER_SIZE + page * PAGE_SIZE + 1`，写 u16 小端。
- 断言辅助 `assert_slot_count_corruption_error(resp, page_id, slot_count)`：错误文案必须含 `corrupted data page {page_id}`、`slot count {n}`、`680 slots`。
- `corrupted_page_slot_count_makes_reads_fail_with_a_named_error`：**真实 SQL 入口** —— 布尔辅助降级为安全值 + 全表扫描 + 点查，各以点名错误结束。
- `corrupted_page_slot_count_makes_the_writable_slot_lookup_fail_with_a_named_error`：**可写变体取槽点（#2）** —— 真实 `BufferPool` + 真实文件 + 指向该页**不存在**的 `slot_id`（`RowId::new(head, 999)`）⇒ 断言返回点名该页的错误（不 abort）。

### Deviations from Plan（与计划的偏差及原因）

1. **RED 的真实形态与 Plan 的读码预期不同（重要，不影响 Acceptance）**：Plan 预期「全表扫描与点查」在变更前是**进程 panic / abort**。实测：**可写变体取槽点（#2）确实是进程 abort**（SIGABRT，见 Verification 第 1 项）；而**真实 SQL 入口**（全表扫描 / 点查）在变更前**先**命中 T3 已交付的逐槽越界判据 —— `get_slot_data_checked` 的 `slot_area_start = PAGE_SIZE.saturating_sub(slot_count * Slot::SIZE)` 在 `slot_count ≥ 683` 时饱和为 `0` ⇒ **任何**槽的负载都判越界 ⇒ 第一个垃圾槽即报错，**早于**遍历到越界索引 ⇒ 返回的是**逐槽形态**的 `CorruptedDataPage`（`corrupted data page 2: slot 0 range [16, 56) lies outside the page data area`），既不是页头槽数形态的错误，也**没有** abort。
   ⇒ 后果：SQL 入口用例的 RED 是「错误形态不符（断言失败）」，可写变体用例的 RED 是「进程 abort」。两者都是「目标行为未达成」的失败，**断言未放宽**。Plan 的「按 `slot_count` 全量遍历的读点必然 abort」在 T3 的逐槽判据已生效后不再成立（这是 Plan 读码推断与实测的差异，**不是**实现差异）；Plan 冻结的处置方向（不改 `get_slot` 的 `None` 语义、页级判据在取槽之前消费、12 个取槽点全覆盖）**未被改动**。
2. **RED 的复现手段（Plan 未钉定）**：变更前形态在本 Cycle 的工作区里已不可直接取得（页级判据是本次实现的一部分，且 Cycle 000 的产物已在工作区）⇒ 用「把唯一的判定实现 `SlottedPageHeader::slot_count_exceeds_capacity` 临时短路为恒 `false`」复现**等价**的变更前行为（= 页级判据不存在 ⇒ 12 个取槽点直接取槽），观察后**立即逐字恢复**（已用 grep 核对工作区无残留标记）。**未改写任何断言**，未新增判定层 / 脚本 / 专用命令 / 身份证明机制。
3. **验证命令的等价调整**：Cycle 的 Verification 给出单条 20 目标命令；按 M19 多批次串行纪律拆成**两批各 10 个目标**（批内 `--test-threads=1`、批间串行，批 1 全绿后才启动批 2）。目标集合与 Cycle 清单**完全一致**，Gate 判据仍是「全部 0 failed」。命令形式与 Cycle 000 一致（`cargo test --test X … -- --test-threads=1`）。
4. **在计划内的用例内追加 1 条断言**：Plan 的 Required behavior 明确 #12 遇该形态须**降级为安全值 `false`**，但 First test / GREEN 条件只列了「命名错误 / 不 abort / 未损坏零变化」三项 ⇒ 在**既定的 SQL 入口用例内**追加 1 条断言（`check_page_all_visible` 在页头槽数越界时为 `false`），覆盖该 Required behavior；**不新增用例**、不新增夹具语义。

### Blocker Handoff

None —— 未命中 Gate 5（无缺失依赖、无验证失败、无 Task Definition 冲突、无实质偏离）。

### Self-Review（任务级 Review 与自检）

**Gate 3 — Spec compliance**

- **A6**（页头槽数越界在读 / 写路径不 panic、不 abort，返回点名该页与页头声称槽数的明确错误，不被当作「没有这一行」静默跳过）：上表 12 行逐点落地，判据一律在**取槽之前**消费；错误统一为 `CorruptedDataPage { page_id, detail }`，`detail` 点名页头声称的槽数与容量上限（`slot count 700 exceeds the page slot directory capacity of 680 slots`）⇒ **满足**。#12 降级为 `false` 且错误由随后的实际取行处（#10 / #11）抛出 ⇒ **满足**。
- **A7**（槽目录容量界限全项目只有一处定义，索引树侧取值与判据零变化）：读码核对 —— `MAX_PAGE_SLOTS` 的**唯一定义**在 `src/storage/page_format/slotted_page.rs:21`；`src/storage/btree/node.rs` 只导入、不再定义，判据代码逐字保留 ⇒ **满足**。索引树零回归证据：`cargo test --lib` 420/0（含 `page_format` 与索引树单元测试）+ 定向回归中的 `index_manager_test`（27/0）与 `secondary_index_test`（4/0）全绿 ⇒ **满足**。
- **A8**（未损坏路径零变化，含 T3 冻结的「过短负载」逐点语义与健康页「全可见」）：`undamaged_data_page_reads_and_writes_are_unchanged` GREEN + `cargo test --lib` 420/0 + 定向回归 354/0 ⇒ **满足**。
- **Preserve 核对**：`get_slot_data_checked` / `slot_payload_in_bounds` **未动**；`StorageError::CorruptedDataPage` 的变体形状与文案前缀 `corrupted data page {page_id}: ` **未动**（未新增变体）；`get_slot` / `get_slot_by_logical_id` 的签名与 `None` 语义**未动**；`get_slot_data` 的签名与既有单元测试**未动**；`ensure_index_page_intact` / `validate_slots` / `check_data_page` / `check_tree_page` **未动**；T3 的逐点「过短负载」语义（#2–#8 / #11）**未动**；`src/pipeline.rs` 的「表不存在 ⇒ `false`」**未动**（本 Cycle 未触碰该文件）；健康页上 `check_page_all_visible` 仍返回「全可见」（未损坏对照用例断言）。
- **Forbidden 核对**：`src/storage/catalog.rs` **零 diff**；`src/wal/**` **零 diff**；`src/storage/integrity.rs` 本 Cycle **零改动**（工作区中的 diff 属 Iteration 000 / Cycle 000 的既有产物）；`src/storage/btree/**` 的判据逻辑**零改动**（只常量迁移）；未把「过短负载」在 #2–#8 / #11 一并改成报错；未新增错误变体 / 命令参数 / 配置项 / 缓存 / 闸门机制 / 判定层 / 身份证明机制；`src/storage/data_page.rs:272` 的**负载偏移**无界写**未修**（范围外）；未跑全量回归。

**Gate 3 — Code quality**

- diff 无计划外修改：改动仅落在 Task Definition 的 Targets 列出的文件与符号 ⇒ **通过**。
- 无新增警告：`cargo build --lib` 与 `cargo clippy --all-targets` 在本 Cycle 触碰的文件上**零新增 finding**（见 Verification 第 6 项）⇒ **通过**。
- 测试不会因错误原因通过：损坏夹具是**真实文件字节改写**；断言同时检查**错误文案**（点名该页 + 页头声称的槽数 + 容量上限）与**进程存活**（panic ⇒ 用例失败）；未损坏对照用例在变更前即 GREEN ⇒ 断言非恒真 ⇒ **通过**。

**自检发现（已在本 Cycle 内修复并复验）**

1. **[Minor]** `src/storage/data/table_manager.rs` 的导入被拆成 3 条独立的 `use crate::storage::data_page::…`（新增构造器时形成）⇒ 合并为 1 条 ⇒ 复验：`cargo test --lib` 与两批定向回归在该状态下全绿。
2. **[Minor]** RED 探针（临时短路页级判据）必须**逐字恢复** ⇒ 已用 `grep -n "TEMP-RED-PROBE" src/ tests/` 核对为**零命中**，并在其后重跑 GREEN。
3. **[Minor · 既有基线]** `cargo fmt --check` 在 `src/storage/data/table_manager.rs` 报 6 处差异（`:721`、`:1834`、`:1881`、`:1908`、`:1938`、`:1972`），**全在我未触碰的既有代码区** ⇒ 刻意不重排（Surgical Changes），与 Cycle 000 的同项处理一致。

**遗留 Minor 问题**

- None —— 上述 3 项均已处理并复验；无未解决的 Critical / Important 问题。（`src/executor/data_scan.rs` 的 `.expect("slot_index < slot_count validated above")` 按 Plan **刻意不改**：页级判据生效后 `slot_count ≤ 680` ⇒ `slot_index < slot_count` 恒成立 ⇒ 该 `expect` 不再可能被页头损坏触发。）

### Verification Evidence（验证命令与实际结果）

**1) 先行测试的 RED（变更前形态）** —— 复现方式见 Deviations 第 2 项（临时短路页级判据 ⇒ 等价于「无页级判据」的变更前行为）

- **可写变体取槽点（#2）—— 真实进程 abort**：

```
thread 'corrupted_page_slot_count_makes_the_writable_slot_lookup_fail_with_a_named_error' panicked at
  src/storage/page_format/slotted_page.rs:232:26:
attempt to subtract with overflow
thread '...' panicked at src/storage/page_frame.rs:101:27:
called `Result::unwrap()` on an `Err` value: PoisonError { .. }
panic in a destructor during cleanup
thread caused non-unwinding panic. aborting.
process didn't exit successfully: ... (signal: 6, SIGABRT: process abort signal)
```

  （`slotted_page.rs:232` = 可写变体 `SlottedPage::get_slot` 的槽目录定位 `Page::PAGE_SIZE - (index + 1) * Slot::SIZE`；`4096 - 683 * 6` 下溢 ⇒ 与 Plan 钉定的 abort 形态**逐字一致**。）

- **真实 SQL 入口（全表扫描 / 点查）—— 错误的形态（逐槽形态而非页头槽数形态），未 abort**：

```
test corrupted_page_slot_count_makes_reads_fail_with_a_named_error ... FAILED
panicked at tests/data_page_bounded_read_test.rs:270:13:
the error must name the slot count the page header claims:
  Execution error: corrupted data page 2: slot 0 range [16, 56) lies outside the page data area
test result: FAILED. 0 passed; 1 failed;
```

  ⇒ 两种 RED 形态都真实、都不放宽断言（详见 Deviations 第 1 项的原因分析）。夹具另外打印 `[free-page-rebuild] rebuild abandoned: internal error: data page 2 of the persisted directory is damaged: slot count 700 exceeds the page capacity` —— 既有 `integrity::validate_slots` 在开库时已独立识别该页头损坏（与本 Cycle 的判据同阈值），可佐证损坏注入真实生效。

**2) GREEN（变更后）** —— `cargo test --test data_page_bounded_read_test -- --test-threads=1`

```
running 8 tests
test corrupted_page_slot_count_makes_reads_fail_with_a_named_error ... ok
test corrupted_page_slot_count_makes_the_writable_slot_lookup_fail_with_a_named_error ... ok
test damaged_data_page_degrades_the_boolean_page_helper_to_the_safe_value ... ok
test damaged_data_page_is_not_swallowed_by_the_alter_table_plan_stage ... ok
test damaged_data_page_makes_point_query_fail_with_a_named_error ... ok
test damaged_data_page_makes_table_scan_fail_with_a_named_error ... ok
test damaged_data_page_makes_writes_fail_with_a_named_error ... ok
test undamaged_data_page_reads_and_writes_are_unchanged ... ok
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s
```

**3) `cargo test --lib`**（Cycle 000 的 420/0 结论对本 Cycle 已失效 ⇒ 重跑）

```
test result: ok. 420 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.81s
```

**4) 定向回归 批次 1/2**（10 个目标，批内 `--test-threads=1`）—— 16 + 39 + 21 + 11 + 3 + 4 + 27 + 21 + 6 + 5 = **153 passed / 0 failed**。

**5) 定向回归 批次 2/2**（10 个目标，批内 `--test-threads=1`；批 1 全绿后才启动）—— 3 + 9 + 6 + 27 + 12 + 3 + 27 + 4 + 107 + 3 = **201 passed / 0 failed**。

定向回归合计 **354 passed / 0 failed**（目标集合与 Cycle 的 Verification 清单**完全一致**，与 Cycle 000 的基线数字一致）。

**6) 格式化 / 静态分析（与既有基线对比，只核对本 Cycle 触碰的文件）**

- `cargo fmt --check` ⇒ **50 处 diff / 9 个文件**（`src/storage/btree/btree.rs`、`src/storage/data/mod.rs`、`src/storage/data/table_manager.rs`、`src/storage/integrity.rs`、`tests/ddl_rewrite_consistency_test.rs`、`tests/free_page_lifecycle_test.rs`、`tests/index_page_ownership_test.rs`、`tests/index_read_bounded_read_test.rs`、`tests/release_path_bounded_read_test.rs`）—— 与既有基线（**50 处 / 9 个文件**）**逐项一致**，本 Cycle **对 fmt 基线的贡献为 0**。本 Cycle 触碰的 11 个文件中只有 `table_manager.rs` 在列，且其 6 处差异全在我未触碰的既有代码区（`:721`、`:1834`、`:1881`、`:1908`、`:1938`、`:1972`）；`src/storage/data/table_manager.rs` 的我的改动区（`:13-16`、`:1002`、`:1267`）fmt 干净。
- `cargo clippy --all-targets`（非 deny）⇒ lib 侧 **17 条 warning**，与既有基线（**17 条**）一致；deny 模式（`-- -D warnings`）⇒ lib 侧 **17 条 error**（与基线同数），**无一条**指向本 Cycle 触碰的行。本 Cycle 触碰文件内的 finding 全部位于我未触碰的行（`src/storage/buffer_pool.rs:238`、`:322`；`src/transaction/manager.rs:432`、`:481`）⇒ **零新增 finding**。新建测试文件 `tests/data_page_bounded_read_test.rs` **零 finding**。

**7) 读码核对（Acceptance A7 的结构性判据）**

```
grep -rn "MAX_PAGE_SLOTS" src/
  ⇒ 唯一定义：src/storage/page_format/slotted_page.rs:21
    src/storage/btree/node.rs 只含导入（:5）与两处使用（:30、:42）
grep -rn "slot_count_exceeds_capacity" src/
  ⇒ 消费点恰好 12 处（data_scan ×2、create_index ×1、transaction/manager ×1、
    table_manager ×2、data_page ×3、buffer_pool ×3）
    判定实现唯一一处：SlottedPageHeader::slot_count_exceeds_capacity（:89-91）
```

`get_slot` / `get_slot_by_logical_id` 的签名（`-> Option<Slot>` / `-> Option<(Slot, usize)>`）与 `None` 语义**逐字未动**。

**8) 改动后的复验**：自检第 1 项的导入合并发生在 `cargo test --lib` 与两批定向回归**之前**；自检第 2 项的 RED 探针在 GREEN 之后**逐字恢复**（探针期间与之后均未改变被覆盖代码的最终形态）⇒ 第 2–6 项的命令与结果即最终代码状态上的结果，**未重复运行已通过的验证**。

### Persisted Evidence

**None required** —— 全部结论可低成本重跑（1 个测试目标 + `cargo test --lib` + 20 个既有回归目标），命令与实际结果已在本 Response 承载；不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ 不创建 `evidence/` 目录，不预建占位目录（与 Plan 的 `Persisted Evidence: none` 一致）。

### Experience Candidates

1. **Runbook 候选**：页头 `slot_count` 的字节布局与损坏注入步骤 —— 页头 `slot_count` 字段位于页内偏移 **1..3**（`SlottedPageHeader::deserialize` 读 `buf[1..3]`）⇒ 文件偏移 = `HEADER_SIZE(64) + page_id * PAGE_SIZE(4096) + 1`，写 u16 小端；夹具校验用「开库时既有 `integrity::validate_slots` 会打印 `data page N of the persisted directory is damaged: slot count 700 exceeds the page capacity`」即可确认注入生效。本 Cycle **端到端验证**（RED 的 panic 文案逐字命中钉定形态，GREEN 后同一夹具产出点名错误）⇒ 可重复。
2. **Issue 候选（只报告不登记，均在本 change 范围外或被 Plan 显式排除）**：
   1. **写路径的同形无界访问** —— `src/storage/page_format/slotted_page.rs:295`（`add_slot_inner`）与 `:407`（`free_space`）的 `Page::PAGE_SIZE - self.slot_count() * Slot::SIZE`：页头槽数越界时同样**下溢** ⇒ `add_slot` 在写路径上 panic（`INSERT` 落尾页可达）。Plan 的 D12 裁定 ③ 明确「不纳入本 Cycle」（ISS35 只主张读路径）⇒ 只报告。
   2. **可写变体的负载偏移无界写** —— `src/storage/data_page.rs:272` 的 `page.data[slot_offset..slot_offset + VersionHeader::SIZE]`（负载偏移越界 ⇒ panic）⇒ Plan 的 F10 / Issue 候选，本 Cycle 未修。
   3. **恢复面可达** —— `src/wal/recovery.rs:1519` 从损坏页的槽（`:1487`）取 `logical_id` 后调用 `update_version_header_in_data_page` ⇒ 恢复面仍可达上述形态 ⇒ 属 T5 范围。

### Remaining Issues（未解决问题）

- 目录页组（T4）与恢复面组（T5）在页头槽数越界时**仍会 abort**（与 Cycle 000 结束时一致，**不是**本 Cycle 引入的回归）⇒ 由后继 Iteration（T4 / T5）用本 Cycle 冻结的同一判据关闭。
- 写路径的同形无界访问（`add_slot_inner` / `free_space` 的下溢、`data_page.rs:272` 的负载偏移无界写）**未修** ⇒ Issue 候选（见上）。
- ISS35 台账**未关闭**（关闭归 Recorder，须等 T4 / T5 完成）⇒ 本 Cycle 未修改 `.agents/issues/ISS35-slot-count-underflow-aborts-read-path.md`。
- 本 Cycle **不跑全量回归**（change 预算归 Iteration 008 / T16）⇒「本 Cycle 后全库无回归」**不是**本 Cycle 的结论。
- 未发现当前 change 范围内需要额外工作的实质缺陷。

### 跳过项

- **全量回归** —— 依 change 预算归 Iteration 008 / T16（M19 分批串行纪律，本 change 唯一一次）—— 是**预算安排**，不是跳过。
- **目录页组 / 恢复面组** —— 依 Iteration Plan 分 Iteration 执行（T4 / T5 为后继 Iteration：003 / 004）。
- **全仓 fmt / clippy 修复** —— 既有基线条件（50 处 fmt diff / 9 个文件；lib 侧 17 条 clippy），重排违反 Surgical Changes ⇒ 只核对本 Cycle 触碰的文件自身。
- **`src/storage/data_page.rs:272` 的负载偏移无界写 / `add_slot_inner` 的同形空间检查** —— Plan 的 D12 裁定 ③ 与 F10 显式排除 ⇒ 作为 Issue 候选上报，不实施。
- **`src/wal/**`（恢复面组）** —— 归 T5；本 Cycle 对该目录**零 diff**。

### 任务状态记录

- 本环境的任务追踪工具不可用（`tool_search` 查询 `todo task tracking list create` 返回 0 命中）⇒ task / Gate / 跳过项状态记录于本 Response 与 change `tasks.md`（T17 状态同步为「Act Response `reported`，待 Plan Review」；Iteration 002 状态同步为「Cycle 000 = T17 已报告，待 Plan Review」）。

## Plan Review

- Review Result: accepted

**Findings**

**F1（PASS，A6）页级判据的消费点恰好 12 处，每一处都在「取槽之前」判，且统一产出点名该页与页头声称槽数的 `CorruptedDataPage`。**
独立核对（读码 + grep，不重跑 Act 的验证）：`grep -rn "slot_count_exceeds_capacity" src/` 的**判定实现唯一一处** = `src/storage/page_format/slotted_page.rs:89-91`（`self.slot_count as usize > MAX_PAGE_SLOTS`）；另两处为只读 / 可写的**委托**（`:121-123` / `:217-219`）；**消费点恰好 12 处**，逐点核对「判据 ⇒ 取槽」的先后顺序：

| # | 位置 | 判据行 | 取槽 / 迭代行 | 结论 |
|---|---|---|---|---|
| 1 | `src/storage/data_page.rs::read_tuple_from_data_page` | `:185` | `:196` `get_slot_data_checked` | 判在前 ✓ |
| 2 | `src/storage/data_page.rs::update_version_header_in_data_page` | `:259` | `:267` `get_slot_by_logical_id` | 判在前 ✓ |
| 3 | `src/storage/data_page.rs::delete_tuple_from_data_page`（探测） | `:301` | `:313` `get_slot_data_checked` | 判在前 ✓ |
| 4 | `src/storage/data/table_manager.rs`（ALTER DROP COLUMN 存活行收集） | `:1002` | `:1008` 起逐槽 | 判在前 ✓ |
| 5 | `src/storage/data/table_manager.rs::table_has_live_rows` | `:1267` | `:1273` 起逐槽 | 判在前 ✓ |
| 6 | `src/executor/data_scan.rs::build_superseded_map` | `:253` | `:259` 起逐槽 | 判在前 ✓ |
| 7 | `src/executor/data_scan.rs`（主扫描循环） | `:461` | `:467` 起逐槽 | 判在前 ✓ |
| 8 | `src/executor/create_index.rs`（CREATE INDEX 回填） | `:135` | `:142` 起逐槽 | 判在前 ✓ |
| 9 | `src/transaction/manager.rs::versions_created_by` | `:568` | `:574` 起逐槽 | 判在前 ✓ |
| 10 | `src/storage/buffer_pool.rs`（全可见快路径） | `:426` | `:433` `get_slot_by_logical_id` | 判在前 ✓ |
| 11 | `src/storage/buffer_pool.rs`（主取行循环） | `:490` | `:496` `get_slot_by_logical_id` | 判在前 ✓ |
| 12 | `src/storage/buffer_pool.rs::check_page_all_visible`（布尔辅助） | `:650` | `:653` 起逐槽 | 判在前，且降级为安全值 ✓ |

12 处**全部**产出 `corrupted_data_page_slot_count_error(page_id, slot_count)`（`src/storage/data_page.rs:48-55`）⇒ 错误统一为 `StorageError::CorruptedDataPage { page_id, detail }`，`detail` = `slot count {n} exceeds the page slot directory capacity of 680 slots` ⇒ **点名该页、页头声称的槽数与容量上限** ⇒ **A6 成立**。

**F2（PASS，A6 / #12 的降级语义）布尔辅助降级为安全值 `false`，且该降级**不会**把损坏页变成「查不到」。**
`src/storage/buffer_pool.rs:645-652`：`check_page_all_visible` 在页头槽数越界时 `return Ok(false)`（**非**「全可见」）⇒ 调用方 `src/executor/data_scan.rs:580-587` 不再 `set_all_visible` ⇒ 该页走慢路径 ⇒ 错误由随后的实际取行处（#7 `data_scan.rs:461`）以点名错误抛出 ⇒ **降级 ⇒ 仍报错，不是静默跳过** ⇒ 与 A6 的「不得被当作『没有这一行』静默跳过」一致。

**F3（PASS，A7）槽目录容量界限全项目只有一处定义，值不变；索引树侧的取值与判据**逐字**未变（diff 只有常量迁移 + 导入）。**
`grep -rn "MAX_PAGE_SLOTS" src/` ⇒ 定义**唯一**在 `src/storage/page_format/slotted_page.rs:21` = `(Page::PAGE_SIZE(4096) - SlottedPageHeader::SIZE(16)) / Slot::SIZE(6)` = **680**，与原 `btree/node.rs` 的值**一致**；`src/storage/btree/node.rs` 只剩导入（`:5`）与两处使用（`:30`、`:42`）。
`git diff -U1 -- src/storage/btree/node.rs` 的**全部**改动 = ① 导入块加入 `MAX_PAGE_SLOTS`（rustfmt 换行）；② 删除本地 `const MAX_PAGE_SLOTS` 及其文档注释。**无任何逻辑行改动** ⇒ `bounded_slot_data_ref`（`:29-38`）与 `bounded_slot_data_mut`（`:41-50`）的判据 `if index >= MAX_PAGE_SLOTS { return None; }` **逐字保留** ⇒ **索引树路径零回归** ⇒ **A7 成立**。

**F4（PASS，A8 / Preserve）`get_slot` / `get_slot_by_logical_id` 的签名与 `None` 语义未动；读侧判界与错误变体未动；新增错误变体为零。**
`git diff` 显示 `SlottedPageRef::get_slot`（`:129`）与 `SlottedPage::get_slot`（`:225`）**只增加文档注释**（指向页级判据），函数体**逐字未动**（`:133-134` / `:231-232` 的下溢算术仍在——这是 D12 冻结的「不改 `None` 语义」）；`get_slot_by_logical_id` 同样只增注释。`slot_payload_in_bounds`（`:180-183`）与 `get_slot_data_checked`（`:137-144`）**逐字未动** ⇒ T3 冻结的逐点「过短负载」语义（#2–#8 / #11）与布尔辅助的安全值语义**零变化**。`StorageError::CorruptedDataPage` 的**变体形状**（`{ page_id, detail }`）与文案前缀 `corrupted data page {page_id}: ` **未动**，且**未新增任何错误变体**（`git diff -- src/storage/error.rs` 只涉及既有变体的文档与重导出）⇒ **A8 成立**。

**F5（PASS，边界纪律）本 Cycle 未越界：目录页组 / 恢复面组 / 完整性扫描 / 索引树判据均未被改动。**
`git diff --stat` 的产品面改动落在 Task Definition 列出的文件内；`src/storage/catalog.rs` **零 diff**；`src/wal/**` **零 diff**；`src/storage/integrity.rs` 的工作区 diff 属 Iteration 000 / 001 的既有产物（本 Cycle 未触碰其行）；`src/storage/btree/**` 只有常量迁移（见 F3）；`src/pipeline.rs` 本 Cycle 未触碰 ⇒ 「表不存在 ⇒ `false`」的既有语义**未动** ⇒ 与 Act 的 Forbidden 核对**逐项一致**。

**F6（PASS，RED 复现无残留、断言未放宽）判定实现是**单一**实现且**非恒 `false`**；两处新断言严于「不 panic」。**
独立核对 `src/storage/page_format/slotted_page.rs:89-91` 的判定实现 = `self.slot_count as usize > MAX_PAGE_SLOTS` ⇒ **不是**恒 `false`，也没有第二处判定实现 ⇒ Act 的「临时短路判定实现 ⇒ 复现变更前行为 ⇒ 逐字恢复」**无残留**。测试侧 `tests/data_page_bounded_read_test.rs`：`assert_slot_count_corruption_error`（`:263-278`）同时断言错误文案含 `corrupted data page {page_id}`、**`slot count {n}`**（`:269-273`）、**`680 slots`**（`:274-277`）⇒ 断言**未放宽**；`:300-305` 的「布尔辅助降级为安全值」断言是 Act 在**既定的 SQL 入口用例内**追加的 1 条（Deviations 第 4 项），**未新增用例**、**未新增夹具语义** ⇒ 符合 Task Definition。

**F7（PASS，偏差分类正确）Deviations 第 1 项属「现状变化 / Plan 预期偏差」，**不是** Act 偏离；冻结的处置方向未被改动。**
Plan 预期「真实 SQL 入口在变更前会 abort」；Act 实测「**不 abort**，而是先命中 T3 已交付的逐槽判据」。独立复核该实测的机理：`src/storage/page_format/slotted_page.rs:181` 的 `slot_area_start = Page::PAGE_SIZE.saturating_sub(self.slot_count() * Slot::SIZE)` 在 `slot_count ≥ 683` 时**饱和为 0** ⇒ 任何槽的负载都判越界 ⇒ **第一个垃圾槽**即报错 ⇒ **早于**遍历到会下溢的索引 ⇒ 返回**逐槽形态**的 `CorruptedDataPage`（`corrupted data page 2: slot 0 range [16, 56) lies outside the page data area`）。⇒ Plan 的推断建立在「T3 的逐槽判据不存在」的前提上，而该前提在 Cycle 000 之后已不成立 ⇒ 属**现状变化**导致的 Plan 预期偏差，**非实现偏差**。
Act 同时实测「可写变体取槽点（#2）确实 abort」：独立复核 `src/storage/page_format/slotted_page.rs:231-232` 的可写变体 `get_slot` **无下溢保护**（与只读变体同形）⇒ 该 abort 形态与 Act 的记录**一致**。
⇒ Deviations 第 1 项**如实记录**且**分类正确**；Plan 冻结的方向（不改 `get_slot` 的 `None` 语义；页级判据在取槽之前消费；12 个取槽点全覆盖）**未被改动** ⇒ **不构成 rework 触发**。
Deviations 第 2 项（RED 复现手段）、第 3 项（验证命令按 M19 拆两批）、第 4 项（用例内追加 1 条断言）均为**非实质**执行细节，且与 Task Definition / Cycle 的 Verification 清单**一致**（两批的目标集合与清单**完全一致**）。

**F8（PASS，证据充分性）Act 的验证结论覆盖 Acceptance 且可低成本重跑；`Persisted Evidence: none` 成立。**
沿用 Act Response「Verification Evidence」（覆盖范围未变化，本 Plan **未重跑**）：RED 形态逐字核对 —— 可写变体 abort 的 panic 文案 `attempt to subtract with overflow` @ `slotted_page.rs:232` 与**当前代码**的 `:232`（可写变体 `get_slot` 的越界切片行）**逐字对应** ⇒ RED 指向的正是修复点；真实 SQL 入口的 RED 文案 `corrupted data page 2: slot 0 range [16, 56) lies outside the page data area` 与**当前代码**的逐槽判据形态**逐字对应**（F7 已复核机理）⇒ 两种 RED 都是真实的「目标行为未达成」，**断言未放宽**。GREEN `cargo test --test data_page_bounded_read_test` 8/0；`cargo test --lib` 420/0；定向回归 2 批各 10 = 153 + 201 = **354/0**；`cargo fmt --check` **50 处 / 9 文件**与基线**逐项一致**、clippy lib **17 条**与基线同数且**无一条**指向本 Cycle 触碰的行。全部结论可低成本重跑（1 个测试目标 + `cargo test --lib` + 20 个既有回归目标）⇒ Act Response 装得下 ⇒ `Persisted Evidence: none` 成立，无须建 `evidence/` 目录。

**F9（Minor finding，非阻塞）`src/storage/page_format/slotted_page.rs` 的写入侧仍是「同形缺保护」，本 Cycle 未修且**不应**由本 Cycle 修。**
`add_slot_inner:295`、`free_space:407`、`delete_slot:349/:350/:357` 的 `Page::PAGE_SIZE - slot_count * Slot::SIZE` 仍无下溢保护；`src/storage/data_page.rs:272` 的版本头写仍无边界检查。这**不是**本 Cycle 的 Acceptance 缺口：Plan 的 D12 裁定 ③ 明确「写路径需独立方向」，ISS35 台账只主张读路径 ⇒ Act 未修是**遵守执行依据**，不是偏离。⇒ 已由 Recorder 建档为 **ISS37 / ISS38** 并在本 change 内**分配承载**（见 F10）⇒ **不构成 finding**，仅记录以闭环 Act 报告的 Issue 候选。

**F10（新证据，已转为本 change 内的承载任务）Act 报告的两条 Issue 候选已被 Recorder 登记为 ISS37 / ISS38，本 Review 确认承载分配成立且顺序正确。**
- ISS37（写路径页头槽数下溢）：生产触点 `data_page.rs:99` / `:119`（INSERT 落尾页 / 新页）、`catalog.rs:707`（目录页链追加）、`recovery.rs:207`（恢复面强制落位）；索引树侧受 `ensure_index_page_intact` 闸门保护不暴露；`free_space()` 当前**无未受保护的生产调用点**（本 Review 独立复核：全部调用点 = `btree/node.rs:212/286/402/814/921`（均在闸门之后）+ `catalog.rs:1934`（`#[cfg(test)]`）+ 单元测试）⇒ 按「同形原语缺保护」登记成立。
- ISS38（写路径槽负载偏移无界写）：无界写在 `data_page.rs:272`（`VersionHeader::SIZE = 22`，本 Review 复核 `src/transaction/version_chain.rs:119`）；触达路径 = commit（`buffer_pool.rs:366`）、abort（`transaction/manager.rs:514/:525`）、DELETE（`executor/delete.rs:216`）、UPSERT（`executor/upsert.rs:688`）、恢复面（`wal/recovery.rs:1519`）、`table_manager.rs:1116`；同族兄弟点 = `slotted_page.rs::update_slot_in_place`（`:397-399`，生产调用点 `catalog.rs:924`）⇒ 登记成立。
- 本 Review 独立确认写路径的**关键语义事实**（决定承载顺序）：`data_page.rs:102-140` 与 `catalog.rs:711-742` 的既有 `Err(_)` 分支把**任何**落位失败当作「页满 ⇒ 换新页继续」⇒ 若写路径不区分「页满 / 页损坏」，损坏页会被跳过而数据继续落盘 ⇒ 写路径必须先行收口；又 `wal/recovery.rs:1519` 经 `data_page.rs:272` 到达无界写 ⇒ **写路径必须排在 T5（恢复面组）之前**；且写路径与 T4 同改 `catalog.rs:707` / `:924` ⇒ **写路径必须排在 T4 之前**。
⇒ 承载分配 = **Iteration 003「写路径的槽界限（ISS37 + ISS38）」**（任务 T18 / T19），排在 Iteration 002 之后、Iteration 004（T4 目录页组）之前 ⇒ 顺序正确（详见 `design.md` D15）。

**偏差分类**：无 Plan 遗漏、无 Plan 错误、无 Act 偏离、无现状变化导致的**阻塞**偏差；Deviations 第 1 项属**现状变化 / Plan 预期偏差**（非 Act 偏离），已被如实记录且不改变 Acceptance；F9 为非阻塞 Minor finding（范围外，已登记并分配承载）；F10 为新证据（已转为本 change 内的承载任务）。

**Acceptance Gaps**

None —— A6 / A7 / A8 全部满足（逐条证据见 F1–F8）；本 Cycle 的 Verification 清单与 Act 实际执行的命令**目标集合一致**且全绿；无被跳过的 Acceptance（跳过项仅为「全量回归」与「目录页组 / 恢复面组 / ISS35 / 全仓 fmt-clippy」四类**范围性**排除，均已在本 Cycle 的 Risks / 跳过项中记录，且与 change 的预算安排一致）。

**Evidence**

- 代码与 diff：`git diff -U1 -- src/storage/btree/node.rs`（**全部**改动 = 导入 + 常量删除，零逻辑行）；`git diff -- src/storage/page_format/slotted_page.rs src/storage/page_format/mod.rs`、`git diff -- src/storage/data_page.rs src/storage/error.rs src/storage/mod.rs`；逐行核读 `src/storage/page_format/slotted_page.rs:21/89-91/121-123/129-144/180-183/217-219/225-232/288-337/340-367/370-375/384-402/405-409`、`src/storage/btree/node.rs:1-50`、`src/storage/buffer_pool.rs:418-436/483-500/645-663`、`src/storage/data_page.rs:48-55/92-140/185-197/259-273/301-320/325-330`、`src/storage/data/table_manager.rs:998-1008/1263-1273`、`src/executor/data_scan.rs:248-262/457-468/576-588`、`src/executor/create_index.rs:130-143`、`src/transaction/manager.rs:564-574`、`src/storage/catalog.rs:690-743/915-931`、`src/wal/recovery.rs:200-215/1510-1525`、`src/storage/page_frame.rs:87-97`、`src/storage/error.rs`（`PageFull` / `CorruptedDataPage`）、`src/transaction/version_chain.rs:119`。
- 结构核对：`grep -rn "MAX_PAGE_SLOTS" src/`（唯一定义 + 导入 + 两处使用）；`grep -rn "slot_count_exceeds_capacity" src/`（判定实现唯一一处 + 2 个委托 + **12 个消费点**）；`grep -rn "free_space()" src/`（5 个索引树调用点 + 1 个 `#[cfg(test)]` + 单元测试）；`grep -rn "add_slot\(|add_slot_with_logical_id|delete_slot|update_slot_in_place|update_version_header_in_data_page" src/`（写路径的完整触点清单）。
- 测试与断言：`tests/data_page_bounded_read_test.rs:78-84`（页头 `slot_count` 注入夹具）、`:263-278`（点名断言）、`:288-315`（真实 SQL 入口用例 + 安全值断言）、`:320-360`（可写变体用例）、`:363-369`（未损坏对照）。
- 沿用 Act 已报告且覆盖范围未失效的验证结论（来源：Act Response「Verification Evidence」第 1–8 项）：RED 两种形态（可写变体 abort @ `slotted_page.rs:232`；真实 SQL 入口逐槽形态）、GREEN `cargo test --test data_page_bounded_read_test` 8/0、`cargo test --lib` 420/0、定向回归 2 批 354/0、fmt 50 处 / 9 文件与基线逐项一致、clippy lib 17 条且触碰文件 0 条、读码核对（唯一定义 / 12 个消费点）。**本 Plan 未重跑上述验证**（公共规则 › 验证：覆盖范围未变化 ⇒ 直接沿用），只做了代码与 diff 的**独立阅读**。

**Follow-up Decision**

**接受本 Cycle（`accepted`）。** 依据：既有 Acceptance（A6 / A7 / A8）全部满足且有独立代码核对支撑（F1–F8）；无阻塞 finding；Deviations 第 1 项为**现状变化**导致的 Plan 预期偏差，方向未被改动（F7）；F9 为范围外的 Minor finding（已登记并分配承载）；F10 是**新证据**，把 Act 报告的 Issue 候选推进为**本 change 内已分配的承载任务**。
**本 Cycle 不再需要修复**：无待修项；F9 仅记录，不建 rework Cycle。
**写路径承载的形态裁定**（记录在 `design.md` D15 与 Iteration 003 的 Plan Context）：① **判据复用**——写侧**复用** Iteration 002 冻结的页级判据（唯一实现、阈值 `slot_count > MAX_PAGE_SLOTS`）与 T3 冻结的槽负载判界（**上提为读侧 / 写侧共用的唯一实现**），不另造阈值；② **错误形态**——写侧与读侧**同名同形**（`StorageError::CorruptedDataPage { page_id, detail }`），detail 点名该页（负载形态时同时点名该槽与区间）；③ **失败语义（写侧独有）**——写入失败 ⇒ **语句失败**（会话事务由既有 `rollback_session` 回滚）；页损坏**不得**换页继续（既有 `Err(_)` 分支必须区分「页满 / 页损坏」）、**不得**折叠成「行 / 槽不存在」、**不得**静默跳过；④ **与 `write-failure-state` 域（ISS30 / ISS31）不冲突**——页损坏是写入前发现的确定性永久条件，刷盘失败是刷盘面的可重试暂态，两者不共用状态、不互相遮蔽、不互相表述。
**状态同步**：change `tasks.md` 已同步 —— T17 = `done（Cycle 000 Plan Review 2026-10-25 accepted）`；**新增 T18（ISS37）/ T19（ISS38）**，归属 **Iteration 003**；T4 = `planned（Iteration 004 / Cycle 000）`；Iteration Map 的 002 行状态 = `accepted`，T18 / T19 分落 **Iteration 003**（`ready`），其后为 004（T4）、005（T5）。
**Act Response 中「T17 状态已同步为『Act Response reported，待 Plan Review』」一句是 Act 当时的记录，现已被本 Review 的结论取代。**

**Iteration Plan Update**

本 Cycle 自身的目标、范围、依赖、验证方式与验收边界**未变**。Iteration 002 的**后续**计划变更记录在 change `tasks.md` 的 Iteration 002 计划与 `design.md` D15：写路径组（ISS37 + ISS38）作为**独立 Iteration 003** 插入在本 Iteration 之后、目录页组（原 Iteration 003）之前 —— 原 Iteration 003（目录页组）**顺延为 Iteration 004**，其目录 `iterations/003-catalog-bounded-read/` 已重命名为 `iterations/004-catalog-bounded-read/`（内容除「所属 Iteration 标识」与「路径 / 顺序引用」外逐字保留）；其后 005（恢复面组）/ 006（DDL）/ 007（写盘失败）/ 008（dump-restore）/ 009（语料 + 收尾全量）顺延。**依赖理由**：写路径必须排在 T5 之前（恢复重放经 `wal/recovery.rs:1519` 到达 `data_page.rs:272` 的无界写；写路径未收口时 T5 的「恢复面有界」保证是空的），且写路径必须排在 T4 之前（同改 `catalog.rs:707` / `:924`，避免两个 Iteration 改同一批目录写入点）。

**Next Cycle**

None —— 本 Cycle 是 Iteration 002 的唯一 Cycle，无 rework / replan 后继。

**Next Iteration**

`openspec/changes/2026-10-25-ms37-defect-ledger-closure/iterations/003-write-path-slot-bounds/000-initial.md`（Iteration 003 / Cycle 000，任务 T18（ISS37）+ T19（ISS38），Cycle Type = `initial`，Plan Context 状态 `ready`）—— **执行顺序**：先 `002-slot-count-underflow`（本 Cycle，已 `accepted`），再 `003-write-path-slot-bounds`，再 `004-catalog-bounded-read`（T4）、`005-recovery-bounded-read`（T5，尚未创建）。
