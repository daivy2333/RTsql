# Iteration 005 / Cycle 000: 恢复面组的有界读（WAL 重放面上的损坏槽按恢复类别分别处置）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T5
- Acceptance gaps: None
- Repair items: None
- Inherited scope:
  - delta spec `page-read-bounded-read` 的 Requirement 首段与 Scenario「恢复面上的损坏槽按恢复类别分别处置」—— **已获批**，本 Cycle 执行它；
  - delta spec `page-read-bounded-read` 的 Scenario「页头声称的槽数超出槽目录容量时读路径不 abort」—— 本 Cycle 执行其**恢复面部分**（该 Scenario 的 GIVEN 的 WHEN 明确含「按逻辑 id 查找某个槽」；其恢复面处置条款已于本 Cycle 展开前补全为逐面粒度，见下方「delta spec 措辞补全」）；
  - `design.md` D7 的恢复面行（尽力而为 + 明确报出）与 `tasks.md` Iteration 005 的**方向裁定**（2026-10-25，Plan）：① **已提交记录重放类** ⇒ 越界槽 SHALL NOT 跳过该记录 ⇒ SHALL 以明确错误结束恢复与打开；② **扫描类** ⇒ 跳过该槽、计入恢复告警并报出、其余记录继续重放；
  - T3 已交付并**冻结**的产品原语：`SlottedPageRef::get_slot_data_checked`（`src/storage/page_format/slotted_page.rs:253-258`）与 `StorageError::CorruptedDataPage { page_id: u64, detail: String }`（`src/storage/error.rs:67-75`，`Display` = `corrupted data page {page_id}: {detail}`）—— 本 Cycle **复用**，**不得重复实现**、**不得新增错误变体**；
  - Iteration 002（T17 / ISS35）**冻结**的页级语义：页格式层提供**唯一的**槽目录容量界限 `MAX_PAGE_SLOTS`（`src/storage/page_format/slotted_page.rs:89`）与**一个页级判据** `slot_count_exceeds_capacity`（`:172-174` / 只读变体 `:204-206`）；本 Cycle 按该**语义**调用（API 名称以 Iteration 002 的实际实现为准）；
  - Iteration 003（T18 / T19）**已收口**的恢复面**写点**：`src/wal/recovery.rs` 的 `add_slot_with_logical_id` 落位（`:205-208` → `src/storage/page_format/slotted_page.rs` 的 `add_slot_inner`，页级判据前置在 `:373-377`）与 `update_version_header_in_data_page` 版本头写（`src/wal/recovery.rs:1519` → `src/storage/data_page.rs:292` 的 `update_version_header_in_data_page`，其页级判据前置在 `:306`）—— 本 Cycle **不重复实现**，只核对并记录其现状。
- Excluded scope:
  - 目录页组（Iteration 004 / Cycle 000 已交付并被接受）与数据页组（Iteration 001 / Cycle 000 已交付并被接受）；
  - 页格式层（`src/storage/page_format/**`）、索引树页（`src/storage/btree/**`）、数据页的页级判据 `check_data_page` / `check_tree_page` / `validate_slots`、`src/storage/integrity.rs`、`src/storage/data_page.rs`、`src/storage/buffer_pool.rs`、`src/executor/**`、`src/transaction/**`、`src/pipeline.rs`；
  - 恢复的**写路径语义**（重放的落位策略、幂等跳据、索引重建策略、checkpoint / WAL 截断策略）、`wal-recovery-replay-integrity` 的 K05 语义（不可重放的已提交记录 SHALL 报错）；
  - 损坏恢复 / 抢救、磁盘格式、dump / restore 合同面；
  - 全量回归（整个 change 的全量预算在 Iteration 009 / T16）。

**Objective**

WAL 重放面（`src/wal/recovery.rs`）上的槽读取不再造成越界切片 panic 与进程 abort：恢复面上的**全部**槽负载读取经有界路径取得、**全部**取槽点在取槽之前问页级槽数判据；损坏槽按**恢复类别**分别处置——**已提交记录重放类**以**明确的错误**结束恢复与打开（不静默丢弃已提交记录），**扫描类**跳过该槽（页级形态 ⇒ 跳过该页）、**计入恢复告警并报出**，其余可恢复的记录继续重放。未损坏路径的行为**零变化**。

**Background**

- 缺陷来源：ISS27 台账（`.agents/issues/ISS27-unbounded-slot-reads-other-page-types.md`）的**恢复面组**（`design.md` D7 已把 ISS27 拆成按页型分组的判定方向），以及 Iteration 002 的页级判据（ISS35 台账）。
- 本 Cycle 的方向**已定**（`tasks.md` Iteration 005 的方向裁定）：D7 的单一方向按恢复类别写细（**不是改方向**）⇒ 无需新的产品决策 ⇒ 不触发拆分信号。
- 本 Cycle 为何排在 Iteration 004 之后：Iteration 004 交付并冻结了目录页组的有界读与「目录损坏 ⇒ 该库 SHALL NOT 打开」的裁定；恢复面的打开路径会先经目录面（`Catalog::scan_tables`），两者的错误面形态（`CorruptedDataPage` vs `CorruptedCatalogPage`）须在同一 change 内保持一致 ⇒ 排在后面不返工。
- **本 Cycle 是本 change 中第一个解禁 `src/wal/**` 的 Iteration**：Iteration 002 / 003 / 004 都把 `src/wal/**` 列为 Forbidden 且保持零 diff（其恢复面**写点**由 Iteration 003 在**数据页侧**收口，见下方 Current-State Evidence）。

**Investigation Facts**

- Starting point:
  - 工作区 HEAD `e9cb9d3`；前置状态 = Iteration 004 / Cycle 000（T4）**已被 Plan Review 接受**（`iterations/004-catalog-bounded-read/000-initial.md` 的 `Plan Review` 段，`Review Result: accepted`）。
  - 变更前的验证结果（沿用，未失效）：Iteration 004 / Cycle 000 的 Act Response 记录 GREEN `cargo test --test catalog_bounded_read_test -- --test-threads=1` 5 passed / 0 failed（来源：该 Act Response 的 Verification Evidence）。本 Cycle 改动 `src/wal/recovery.rs` ⇒ **不覆盖**上述结论涉及的被测文件（`src/storage/catalog.rs` / `src/storage/error.rs` 本 Cycle 零改动）⇒ 该结论**继续有效**；`cargo test --lib` 与定向回归的既有基线**覆盖到 `src/wal/recovery.rs` 的结论失效**（须重跑，见 Verification）。
- Current-State Evidence（本 Cycle 已核对的事实，供 Act 直接使用；行号为**当前工作区**）：
  - **恢复面的槽读取点共 5 处**（`grep -rn "get_slot_data\b" src/wal/recovery.rs` + `grep -n "get_slot_by_logical_id" src/wal/recovery.rs` 实测；`grep -rn "get_slot_data\b" src/` 全项目 21 处命中 = 20 处生产调用（数据页组 11 + 目录页组 5 + **恢复面组 4**）+ 1 处 `#[cfg(test)]` 单元测试辅助 ⇒ 与 `tasks.md` 的分组计数一致）：

    | # | 位置 | 所在函数 / 类别 | 取槽方式 | 暴露的损坏形态 |
    |---|---|---|---|---|
    | W1 | `src/wal/recovery.rs:374` | `build_pk_version_maps`（`:338`）—— **重放前的候选扫描** ⇒ **扫描类** | `for index in 0..slotted.slot_count()` + `slotted.get_slot(index)` + `slotted.get_slot_data(&slot)`（**无范围检查**） | 负载越界（ISS27）+ 页头槽数越界（ISS35，取槽定位下溢） |
    | W2 | `src/wal/recovery.rs:462`（负载）与 `:459`（取槽） | `derive_old_row_id`（`:434`）—— **更新重放所需的旧行推导** ⇒ **已提交记录重放类** | `slotted.get_slot_by_logical_id(old_row_id.slot_id)`（`:459`）+ `slotted.get_slot_data(&slot)`（`:462`，**无范围检查**） | 同上 |
    | W3 | `src/wal/recovery.rs:1177` | `RecoveryManager::rebuild_pk_indexes`（`:1050`）—— **重放后的索引重建** ⇒ **扫描类** | `for index in 0..slotted.slot_count()` + `get_slot` + `get_slot_data`（**无范围检查**） | 同上 |
    | W4 | `src/wal/recovery.rs:1490` | `RecoveryManager::mark_uncommitted_aborted`（`:1445`）—— **未提交事务标记** ⇒ **扫描类** | 同 W3 | 同上 |
    | W5 | `src/wal/recovery.rs:166` | `redo_tuple_at_row_id`（`:144`）的**幂等探测**（`already_present`）—— 发生在**已提交记录的重放**中 ⇒ **已提交记录重放类** | `slotted.get_slot_by_logical_id(row_id.slot_id).is_some()`（**可写变体**，**无页级判据前置**） | **仅页头槽数越界**（该探测只读槽目录，不读负载） |

    ⇒ **W5 是本 Cycle 新识别的一处恢复面 abort 点**：`src/storage/page_format/slotted_page.rs` 的两个 `get_slot_by_logical_id`（只读 `:229-237` 形态 / 可写 `:323-331` 形态）**均无页级判据前置**，其内部 `for i in 0..self.slot_count()` + `get_slot(i)` ⇒ 页头槽数越界时 `PAGE_SIZE - (i+1) * Slot::SIZE` 整数下溢 ⇒ 越界切片 panic ⇒ 进程 abort。Iteration 002（T17）的 12 个取槽点**全部在数据页组**（`data_page.rs` / `table_manager.rs` / `executor/` / `transaction/` / `buffer_pool.rs`），**不含 `src/wal/**`** ⇒ 该形态在恢复面**未被前序 Iteration 覆盖**；delta spec 的页级形态 Scenario 的 WHEN 明确含「**按逻辑 id 查找某个槽**」⇒ W5 在本 Cycle 的承载面内。
  - **恢复面已有的写侧防线（Iteration 003 已收口，本 Cycle 不重复实现）**：
    - `src/wal/recovery.rs:205-208` → `SlottedPage::add_slot_with_logical_id` → `add_slot_inner`（`slotted_page.rs`）：**页级判据前置**在 `:373-377`（`if self.header.slot_count_exceeds_capacity()`）⇒ 页头槽数越界 ⇒ `PageWriteError::SlotCountExceedsCapacity`，**不 panic**；
    - `src/wal/recovery.rs:1519` → `update_version_header_in_data_page`（`src/storage/data_page.rs`）：页级判据前置在 `:232` 与 `:306`，槽级判界与长度判据在 `:313-334` ⇒ 越界写已被拦截；
    - ⇒ 本 Cycle 只读这两个点做**现状核对**（确认防线在位），**不改** `src/storage/page_format/**` 与 `src/storage/data_page.rs`。
  - **恢复的执行顺序（决定处置的可达性）**：`RecoveryManager::full_recover`（`src/wal/recovery.rs:538`）：
    1. Step 1 分类事务（`:585-608`）；
    2. Step 2 重放（`:610-651`）：`will_redo` 为真时**先** `build_pk_version_maps`（**W1**，`full_recover:632`）建立磁盘版本多映射，**再**逐条 `redo_record`（`:640-648`）⇒ **W1 总是先于 W2 / W5**；
    3. Step 2b `persist_replayed_tails`（`:653-655`）；
    4. Step 3 `mark_uncommitted_aborted`（**W4**，`:657`）；
    5. Step 4 `rebuild_pk_indexes`（**W3**，`:660-662`，仅 `redo_count > 0` 时进入）。
    ⇒ 三个**扫描类**点的先后为 W1 → W4 → W3；**重放类**点（W2、W5）在 W1 之后。
  - **W2 自身损坏槽分支的可达性（本 Cycle 的关键判定）**：`derive_old_row_id` 的候选集**只**来自 W1 建立的版本映射（`src/wal/recovery.rs:862-877`：`maps.keyed` / `maps.keyless`）⇒ **W1 跳过损坏槽后，该槽不在候选集内 ⇒ W2 不会读到它** ⇒ **W2 自身的越界槽分支在当前恢复流下不可达**；W2 的**可观察后果**由既有语义给出：已提交 UPDATE 记录**不被静默丢弃**，恢复以明确错误结束（既有 `WalError::RedoFailed("update redo: table '…' old key has no version before row … on disk")`（`:445-448`）；若候选回退到更旧版本则为 `"… derived slot … tuple mismatch with WAL old_tuple"`（`:489-493`））⇒ 打开失败。⇒ **W2 的实现仍必须做**（delta spec 的 Requirement 要求恢复面的**全部**槽负载读取经有界路径；且 `:459` 的 `get_slot_by_logical_id` 需要页级判据前置），但其损坏槽分支**不作为测试的断言对象**。
  - **W5 的可达性（可达，且可测）**：重放是**位置寻址**的（`redo_tuple_at_row_id` 按 WAL 记录的 `row_id` 落位）⇒ W1 跳过损坏页**不影响**重放去探测该页 ⇒ **W5 在损坏页上可达** ⇒ 页头槽数越界时 W5 ⇒ abort ⇒ GREEN 后 ⇒ 明确的错误结束恢复与打开。
  - **既有「过短负载」语义（保持不变，不是越界）**：W1 / W3 / W4 的 `slot_data.len() < VersionHeader::SIZE` ⇒ `continue`（跳过）；W2 的同条件 ⇒ `StorageError::Internal("derived slot … shorter than version header")` ⇒ `WalError::RedoFailed` ⇒ 恢复以错误结束。⇒ delta spec 的判定口径明确「负载短于该页型既定的最小负载**不属于**越界槽，其既有逐点语义 SHALL 保持」⇒ 本 Cycle **不改**这四处「过短」分支。
  - **「报出」的既有承载面**：`src/wal/recovery.rs` 的既有告警形态是 `eprintln!`（例：`:1394-1420` 的旧索引树释放告警「free old index page …」）⇒ 恢复告警沿用同一形态（stderr），**不新增公共 API 字段**、不新增命令参数 / 配置项 / 缓存 / 闸门机制。
  - **恢复告警的可观测性**：恢复只在**读写模式**打开时运行（`src/database.rs:407-411`：`if mode == OpenMode::ReadWrite { RecoveryManager::full_recover(...) }`）⇒ 只读打开（`rtsql check` 等）**不**触发恢复 ⇒ 断言「报出」的用例必须用**读写模式**的**真二进制子进程**（`CARGO_BIN_EXE_rtsql`，默认读写）⇒ 沿用 `tests/catalog_bounded_read_test.rs:228-277` 的 `spawn_cli` / `wait_cli` 形态捕获 stderr。
  - **崩溃模拟（触发重放）的既有形态**：`db.wal_buffer.as_ref().unwrap().shutdown().await;` + `drop(db);` ⇒ WAL 落盘但**不 checkpoint / 不截断** ⇒ 下次打开触发全量重放（沿用 `tests/recovery_replay_convergence_test.rs:171-176` 的形态）。
  - **数据页号的取得**：`TableMeta::data_page_head`（`src/storage/data/table_manager.rs:204`，新表在 `:655` 由 `allocate_page` 分配）⇒ 夹具可经 `db.table_manager.get_table("t").await?.data_page_head` 取得（沿用 `tests/data_page_bounded_read_test.rs` 的既有做法）；在「页 0 = `__tables`、页 1 = `__columns`」的布局下首个数据页通常为**页 2** ⇒ 夹具**以实测为准**（Act Response 记录实测页号）。
  - **损坏注入的字节算术（Act 已验证可用的形态）**：页 N 的文件偏移 = `HEADER_SIZE(64) + N * PAGE_SIZE(4096)`（`src/storage/file_header.rs:20`、`src/storage/page.rs:11`）；槽目录项 i 的页内起点 = `PAGE_SIZE - (i+1) * Slot::SIZE(6)`，其 **offset 字段在项起点 +2** ⇒ 槽 0 ⇒ 页内 `4092` ⇒ 文件偏移 = `64 + N*4096 + 4092`；**页头 `slot_count` 字段**位于页内偏移 **1..3**（`SlottedPageHeader::deserialize` 读 `buf[1..3]`）⇒ 文件偏移 = `64 + N*4096 + 1`，写入 u16 小端。
  - **既有测试入口**：`tests/recovery_test.rs`、`tests/recovery_e2e_test.rs`、`tests/recovery_replay_convergence_test.rs`、`tests/recovery_error_surface_test.rs`、`tests/recovery_wal_convergence_test.rs`、`tests/recovery_tail_persistence_test.rs`、`tests/wal_recovery_large_test.rs`、`tests/wal_integration_test.rs`、`tests/checkpoint_redo_reduction_test.rs`、`tests/ddl_crash_recovery_test.rs`。
- Code and Critical Path:
  - 入口：`Database::open`（读写模式）→ `RecoveryManager::full_recover`（`src/wal/recovery.rs:538`）→ `build_pk_version_maps`（W1）→ `redo_record`（W2 经 `derive_old_row_id`、W5 经 `redo_tuple_at_row_id`）→ `mark_uncommitted_aborted`（W4）→ `rebuild_pk_indexes`（W3）。
  - 页格式层：`SlottedPageRef::new(data)`（只读）/ `SlottedPage::new(&mut Page)`（可写）→ `slot_count()` / `get_slot(i)` / `get_slot_by_logical_id(id)` / `get_slot_data(&slot)`（**无范围检查**）/ `get_slot_data_checked(&slot)`（**有界**，T3 冻结）。本 Cycle 只把**取负载**换成 `get_slot_data_checked`，并在**取槽之前**加页级槽数判据。
  - 错误面：`src/storage/error.rs` 的 `StorageError::CorruptedDataPage { page_id, detail }`（T3 冻结）—— 恢复面读的是**数据页** ⇒ **复用**该变体，**不新增变体**、**不改** `src/storage/error.rs`。恢复面已有的 `WalError::RedoFailed(msg)`（`src/wal/record.rs:507`，`Display` = `WAL redo failed: {msg}`）承载「明确的错误结束恢复与打开」的既有语义。
  - 状态与并发：恢复在打开期单线程串行执行；页级判据是纯读（页头的一个 `u16`），判定与取槽之间无 await ⇒ 无竞态窗口；本 Cycle 不改变任何锁语义、状态所有权或磁盘格式。

**Implementation Guidance**

- 建议顺序：① 先写**先行测试**（RED）→ ② W1（扫描类 · 重放前候选扫描）→ ③ W3 / W4（扫描类 · 索引重建 / 未提交标记）→ ④ W2 / W5（已提交记录重放类）→ ⑤ GREEN 与定向回归。**本 Cycle 的实现必须遵守「行为变更前先有真实 RED」**（见 First test：损坏注入必须产生**可判定的行为失败**，不是编译失败 ⇒ 测试断言**行为**（打开结果 / 进程是否存活 / stderr 文本），**不断言**尚不存在的错误变体类型）。
- 关键取舍：
  - **两个面分别处置，不得笼统**：**扫描类**（W1 / W3 / W4）⇒ 跳过该槽（页级形态 ⇒ 跳过该**页**）+ **计入恢复告警并报出** + 其余可恢复记录继续重放；**已提交记录重放类**（W2 / W5）⇒ **不跳过该记录** ⇒ 以**明确的错误**结束恢复与打开。
  - **页级形态的粒度是页**：页头声称的槽数越界 ⇒ 该页的槽目录整体不可定位（`get_slot(i)` 对 `i ≥ 681` 必然下溢）⇒ 扫描类**跳过该页**（不是跳某一个槽），并计入告警报出。
  - **「报出」的内容**：告警 SHALL 点名**该页**（页级形态）与**该页的槽序号**（槽级形态）⇒ 不得静默（delta spec：「跳过必须被报出，不得静默」）。计数用**局部计数**（每个扫描点各自维护），**不新增** `RecoveryResult` 的公共字段、不新增命令参数 / 配置项。
  - **「报出」的面**：沿用 `src/wal/recovery.rs` 既有的 `eprintln!` 告警形态（例 `:1394-1420`）⇒ 走 stderr。
  - **复用既有错误变体**：恢复面读的是**数据页** ⇒ 用 `StorageError::CorruptedDataPage { page_id, detail }`（T3 冻结）⇒ **不新增变体**、不改 `src/storage/error.rs`。
  - **既有「过短负载」分支不动**：W1 / W3 / W4 的 `len < VersionHeader::SIZE ⇒ continue`、W2 的同条件 ⇒ `StorageError::Internal` ⇒ 既有逐点语义保持（delta spec 的判定口径）。
  - **W5 是页级判据点，不是负载点**：它只读槽目录 ⇒ 只需**页级判据前置**（无需负载判界）；命中 ⇒ 按**已提交记录重放类**以明确错误结束。
  - **写侧不重复实现**：`add_slot_with_logical_id` 与 `update_version_header_in_data_page` 的防线已由 Iteration 003 交付 ⇒ 本 Cycle 只核对、不改。

**Task Definitions**

### T5: 恢复面上的损坏槽按恢复类别分别处置（扫描类跳过 + 计入告警并报出；已提交记录重放类以明确错误结束）

- Requirement/Scenario:
  - `page-read-bounded-read` Requirement 首段（恢复面的页内容读取经由有边界检查的路径；不产生越界切片、不 abort）；
  - `page-read-bounded-read` Scenario「恢复面上的损坏槽按恢复类别分别处置」；
  - `page-read-bounded-read` Scenario「页头声称的槽数超出槽目录容量时读路径不 abort」（**恢复面部分**）；
  - `page-read-bounded-read` Scenario「未损坏路径的行为零变化」；
  - `wal-recovery-replay-integrity` 的既有 K05 语义（不可重放的已提交记录 SHALL 报错）——**保持**，不修订。
- Depends on: Iteration 004 / Cycle 000（T4）—— 目录页组的有界读与「目录损坏 ⇒ 该库 SHALL NOT 打开」的裁定已冻结，两者的错误面形态在同一 change 内保持一致。**本 Cycle 不依赖任何未落地的 Iteration**（Iteration 002 的页级判据与 Iteration 003 的写侧收口均已 `accepted`）。
- Targets:
  - `src/wal/recovery.rs` —— W1（`:374`）、W2（`:459` 取槽 + `:462` 负载）、W3（`:1177`）、W4（`:1490`）、W5（`:166`）共 5 处；配套的错误构造与恢复告警报出（沿用本文件既有的 `eprintln!` 形态）。
  - `tests/recovery_bounded_read_test.rs` —— **新增测试文件**（先行用例 + 未损坏对照 + CLI 子进程用例）。
- Current behavior: 恢复面上的槽负载经**无范围检查**的 `SlottedPageRef::get_slot_data` 取得 ⇒ `&self.data[start..end]` 越界 panic ⇒ **进程 abort**；页头槽数越界时，`get_slot(i)` 的槽目录定位整数下溢 ⇒ 同样 abort（W1 / W2 / W3 / W4 / W5 均暴露）⇒ 恢复在损坏数据页上 today 既不能完成也不能给出任何诊断。
- Required behavior:
  - W1 / W2 / W3 / W4 的**槽负载**一律经 `get_slot_data_checked` 取得；越界槽**不产生任何越界切片**、**不 panic**；
  - W1 / W2 / W3 / W4 / W5 在**取槽之前**问页级槽数判据；页头声称的槽数超出槽目录容量 ⇒ 明确错误（**不 abort**）；
  - **扫描类**（W1 / W3 / W4）遇到越界槽 ⇒ 跳过该槽（页级形态 ⇒ 跳过该**页**）、**计入恢复告警并报出**（报出内容点名该页；槽级形态另点名槽序号）、**其余可恢复的记录继续被重放**；
  - **已提交记录重放类**（W2 / W5）遇到越界槽 ⇒ **不跳过该记录** ⇒ 以**明确的错误**结束恢复与打开（既有 `WalError::RedoFailed` 语义；W2 的可观察形态为既有的「old key has no version before row …」/「tuple mismatch with WAL old_tuple」）；
  - 恢复告警**不得**被静默（跳过必须被报出）；
  - 未损坏路径行为**零变化**（含四处既有的「过短负载」逐点语义）。
- Preserve:
  - `SlottedPageRef::get_slot_data_checked` / `slot_payload_in_bounds` 的判据与语义（T3 已接受）；
  - `StorageError::CorruptedDataPage` 的变体形状与文案前缀 `corrupted data page {page_id}: `（T3 已接受）；**不新增错误变体**；
  - `SlottedPageRef::get_slot` / `get_slot_by_logical_id` / `SlottedPage::get_slot` / `get_slot_by_logical_id` 的**签名与 `None` 语义**（本 Cycle 只在**调用点**加判据，不改页格式层）；
  - `SlottedPageRef::get_slot_data` 的签名与既有单元测试；
  - 四处既有的「过短负载」逐点语义（W1 / W3 / W4 的跳过；W2 的 `StorageError::Internal`）；
  - `wal-recovery-replay-integrity` 的 K05 语义与 `WalError::RedoFailed` 的既有文案形态；
  - 恢复的**写路径**语义（位置寻址落位、幂等跳据、`add_slot_with_logical_id` 的强制落位、`update_version_header_in_data_page` 的版本头写）与 Iteration 003 已交付的防线；
  - 恢复的执行顺序（W1 → 重放 → W4 → W3）、`will_redo` 门控、`redo_count == 0` 的 clean 打开零变化路径；
  - 磁盘格式、checkpoint / WAL 截断策略、`release-durability-precondition` 与 `wal-writer-handle-reuse` 的既有语义。
- Forbidden:
  - 改动 `src/storage/page_format/**`、`src/storage/btree/**`、`src/storage/integrity.rs`、`src/storage/data_page.rs`、`src/storage/buffer_pool.rs`、`src/executor/**`、`src/transaction/**`、`src/pipeline.rs`；
  - 改动 `src/storage/error.rs`（**不新增错误变体**，复用 `CorruptedDataPage`）；
  - 改动 `src/wal/**` 中本 Cycle 5 个读点与告警报出**之外**的代码（尤其：重放的落位策略、索引重建策略、`wal-writer` / `checkpoint` 语义、错误文案既有形态）；
  - 把「过短负载」一并改成报错；
  - 在扫描类上把越界槽改成「静默跳过」（必须报出）；在已提交记录重放类上把越界槽改成「跳过该记录」；
  - 新增命令参数 / 配置项 / 缓存 / 闸门机制 / 判定层 / 身份证明机制（Hash / 指纹 / revision pin / run-id / manifest / 时间证明）；新增 `RecoveryResult` 的公共字段；
  - 改动磁盘格式、dump / restore 合同面；
  - 全量回归。
- First test:
  - 位置：`tests/recovery_bounded_read_test.rs`（**新增测试文件**；沿用 `tests/catalog_bounded_read_test.rs` 的夹具形态：真实文件库 + 真实 `Database::open` / 真实 `rtsql` 二进制 + **真实文件字节改写**，无替身 / 桩）。
  - **夹具（三个用例共用）**：
    1. `Database::open(path)` → `CREATE TABLE t (id INT PRIMARY KEY, v TEXT)` → 插入 ≥ 2 行 → 对其中一行执行一次 `UPDATE`（⇒ WAL 产生已提交的 Update 记录）⇒ 提交；
    2. **崩溃模拟**：`db.wal_buffer.as_ref().unwrap().shutdown().await;` + `drop(db);` ⇒ WAL 落盘但**不 checkpoint / 不截断** ⇒ 下次打开触发全量重放；
    3. 取数据页号：`db.table_manager.get_table("t").await?.data_page_head`（或重开后从目录面取得）；
    4. **真实文件字节改写**（两种形态分别注入）：
       - **槽级形态**（负载越界）：把该数据页**某个槽**的 offset 字段写成 `0xFFFF`（u16 小端）⇒ 文件偏移 = `64 + page_id*4096 + (4096 - (slot_index+1)*6 + 2)`；
       - **页级形态**（页头槽数越界）：把该数据页**页头 `slot_count`** 写成 u16 小端 `700` ⇒ 文件偏移 = `64 + page_id*4096 + 1`。
  - **RED（变更前，实测形态）**：
    - **用例 A（扫描类 · 槽级形态）**：损坏**未被 UPDATE 触及**的那一行的槽 ⇒ 重开 ⇒ 恢复的 W1 读到该槽 ⇒ **预期 RED：进程 panic / abort**（`range start index 65535 out of range for slice of length 4096`，位置 `src/storage/page_format/slotted_page.rs` 的 `get_slot_data`）—— 不是「打开失败」也不是「跳过」。
    - **用例 B（已提交记录重放类 · 页级形态）**：损坏**页头 `slot_count`** ⇒ 重开 ⇒ 恢复的 W1 / W5 触达该页 ⇒ **预期 RED：进程 panic / abort**（槽目录定位下溢 ⇒ `range start index …` 或 debug profile 的 `attempt to subtract with overflow`）。
    - **用例 C（已提交记录重放类 · 槽级形态）**：损坏 **UPDATE 旧行所在**的槽 ⇒ 重开 ⇒ 恢复在重放该 UPDATE 时以既有明确错误结束 ⇒ **预期 RED（当前即 RED，但形态不同）**：**进程 abort**（W1 先命中该槽）⇒ GREEN 后为「打开以明确错误结束」。
    - 三个用例的 RED 形态都是**行为失败**（进程 abort / 非预期结果），**不是编译失败** ⇒ 测试断言只落在**行为**上（打开结果、进程是否存活、stderr 文本）。
  - **夹具的未损坏对照**：同样步骤但**不改字节** ⇒ 变更前即 GREEN（库能打开、重放完成、行集合与既有行为一致）。
  - **CLI 面用例**（用于断言「报出」）：真二进制子进程（`CARGO_BIN_EXE_rtsql`，**读写模式** ⇒ 触发恢复；`RTSQL_HOME` 指向临时目录、`env_remove("RTSQL_KEY")` / `env_remove("RTSQL_BUSY_TIMEOUT")`、`stdin(Stdio::null())`，沿用 `tests/catalog_bounded_read_test.rs:228-277`）⇒ **用例 A 的 GREEN 期望**：CLI 退出码为**普通非零或零**（不 abort；用例 A 的恢复成功 ⇒ 打开成功 ⇒ 退出码 0），**stderr 含恢复告警**（点名被跳过的页号；槽级形态另含槽序号）⇒ 「跳过被报出」可判定。⇒ **恢复告警的报出面必须在 stderr**（既有 `eprintln!` 形态）才可被 CLI 子进程判定。
- GREEN condition:
  - 用例 A ⇒ 打开**成功**（恢复未 abort）、**进程存活**、**stderr 含点名该页（与该槽）的恢复告警**、其余可恢复记录继续重放（行集合与未损坏对照一致，除被跳过的损坏行外）；
  - 用例 B / C ⇒ 打开**以明确错误结束**（`Err`，非 panic、非 abort）；用例 B 的错误来自 W5 的页级判据（可点名该页与页头声称的槽数）；用例 C 的错误为既有 `WalError::RedoFailed` 形态（文案含 `update redo`）；
  - 未损坏对照结果不变；`cargo test --test recovery_bounded_read_test -- --test-threads=1`、`cargo test --lib` 与本 Cycle 的定向回归清单全绿；`cargo fmt --check` / `cargo clippy --all-targets` 在**本 Cycle 触碰的文件**上零新增 finding。
- Verification:
  - 行为类（真实入口）：`cargo test --test recovery_bounded_read_test -- --test-threads=1` —— 覆盖「扫描类：越界槽被跳过且被报出、恢复继续」「已提交记录重放类：越界槽以明确错误结束恢复与打开、进程不 abort」「未损坏零变化」；损坏注入必须是**真实文件字节改写**，观察对象是打开结果、进程是否存活与 stderr 文本。
  - 单元与库面：`cargo test --lib` —— 覆盖 `page_format` 既有单元测试与 WAL / 目录 / 事务 / 执行器的库内单测。
  - 定向回归（20 个目标，**M19 分批串行纪律**：目标名字典序分组、每批 8–12 个目标、批内 `--test-threads=1`、批间串行、批 1 全绿后才启动批 2）：
    - 批 1（10 个）：`cargo test --test checkpoint_redo_reduction_test --test ddl_crash_recovery_test --test recovery_e2e_test --test recovery_error_surface_test --test recovery_replay_convergence_test --test recovery_tail_persistence_test --test recovery_test --test recovery_wal_convergence_test --test wal_integration_test --test wal_recovery_large_test -- --test-threads=1`
    - 批 2（10 个）：`cargo test --test explicit_tx_test --test mvcc_abort_test --test mvcc_commit_test --test overflow_page_test --test version_chain_test --test wal_auto_checkpoint_test --test wal_buffer_test --test wal_handle_test --test wal_record_test --test wal_writer_test -- --test-threads=1`
    - 通过条件：两批全部 0 failed。
  - **本 Cycle 不跑全量回归**（整个 change 的收尾全量归 Iteration 009 / T16；M19：目标名字典序分组、每批 8–12 个目标、批内 `--test-threads=1`、批间串行、全量 Gate = 全部批次各自 0 failed）。
  - 失败含义：任何一条失败 ⇒ 对应 Acceptance 未满足 ⇒ 停在当前 task（不进入下游 task）。
- Stop when: 需要改动 `src/storage/page_format/**` / `src/storage/btree/**` / `src/storage/integrity.rs` / `src/storage/data_page.rs` / `src/storage/buffer_pool.rs` / `src/executor/**` / `src/transaction/**` / `src/pipeline.rs` / `src/storage/error.rs`；需要改动恢复的**写路径**语义（落位 / 幂等 / 索引重建 / checkpoint）；需要把「过短负载」一并改成报错；需要把扫描类的跳过改成静默；需要把已提交记录重放类改成跳过该记录；需要新增公共 API 字段 / 命令参数 / 配置项 / 判定层 / 身份证明机制；或需要全量回归才能判定（应回到 Plan 调整验证边界）。

**Invariants**

- 未损坏路径上，恢复的执行顺序、重放结果、行集合、索引重建结果与既有行为**逐点一致**；`redo_count == 0` 的 clean 打开零变化。
- 恢复告警的既有承载面（stderr）不变；只读打开仍**不**触发恢复。
- 不引入身份证明机制、判定层、裁决脚本、新命令参数、新配置项、新缓存、新闸门机制、新公共 API 字段。

**Non-goals**

- 目录页组（T4，Iteration 004）；数据页组（T3，Iteration 001）；页级槽数判据的**实现**（Iteration 002）；写路径的槽界限（Iteration 003，T18 / T19）；页格式层的任何改动。
- 恢复的**写路径**语义与 WAL / checkpoint 策略；损坏数据的修复 / 抢救；磁盘格式；dump / restore 合同面。
- 全量回归（Iteration 009 / T16）。

**Acceptance**

| # | 验收条件 | 需求 / 场景 | 设计 | 任务 | 代码位置 | 测试 |
|---|---|---|---|---|---|---|
| A16 | 恢复面的**全部**槽负载读取经有界路径取得（不越界切片、不 abort）；**全部**取槽点在取槽之前问页级槽数判据 | PRBR Requirement 首段 + PRBR「页头声称的槽数超出槽目录容量时读路径不 abort」（恢复面部分） | D7 恢复面行 + Iteration 002 冻结的页级判据 | T5 | `src/wal/recovery.rs`（W1 `:374`、W2 `:459`/`:462`、W3 `:1177`、W4 `:1490`、W5 `:166`） | `tests/recovery_bounded_read_test.rs`（用例 A / B） |
| A17 | **扫描类**恢复读取（重放前候选扫描 / 重放后索引重建 / 未提交事务标记）遇到越界槽 ⇒ 跳过该槽（页级形态 ⇒ 跳过该页）、**计入恢复告警并报出**、其余可恢复记录继续重放 | PRBR「恢复面上的损坏槽按恢复类别分别处置」（扫描类条款） | D7 + tasks.md Iteration 005 方向裁定 ② | T5 | `src/wal/recovery.rs`（W1 / W3 / W4）+ 恢复告警报出 | `tests/recovery_bounded_read_test.rs`（用例 A + CLI 子进程用例：stderr 含点名该页的告警） |
| A18 | **已提交记录重放类**恢复读取（更新重放所需的旧行推导 + 重放的幂等探测）遇到越界槽 ⇒ **不跳过该记录** ⇒ 以**明确的错误**结束恢复与打开 | PRBR「恢复面上的损坏槽按恢复类别分别处置」（重放类条款）+ PRBR 页级形态 Scenario 的恢复面条款 | D7 + tasks.md Iteration 005 方向裁定 ① + `wal-recovery-replay-integrity` K05 | T5 | `src/wal/recovery.rs`（W2 / W5） | `tests/recovery_bounded_read_test.rs`（用例 B / C） |
| A19 | 未损坏路径行为零变化（含四处既有的「过短负载」逐点语义；恢复的执行顺序与 `will_redo` 门控不变） | PRBR「未损坏路径的行为零变化」 | D7 | T5 | 5 处读点 | `tests/recovery_bounded_read_test.rs`（未损坏对照）+ 定向回归清单 |

**Verification**

- 逐 scenario 的最简直接判定（行为类 scenario 走**真实入口**：真实建库 → 真实写入 → 真实崩溃模拟（WAL 落盘 + drop）→ 真实改坏磁盘字节 → 真实重开触发恢复；观察打开结果、进程是否存活与 stderr 文本）：
  1. 「扫描类跳过 + 报出 + 继续重放」：用例 A ⇒ 通过依据 = 打开成功、进程未 panic / 未 abort、stderr 含点名该页（与该槽）的恢复告警、行集合与未损坏对照一致（除被跳过的损坏行）。
  2. 「已提交记录重放类以明确错误结束」：用例 B / C ⇒ 通过依据 = 打开返回 `Err`（**进程未 abort**），用例 B 的错误点名该页与页头声称的槽数，用例 C 的错误文案含 `update redo`。
  3. 「未损坏路径的行为零变化」：同文件的未损坏对照 + 定向回归清单 ⇒ 通过依据 = 全部 0 failed。
- **不建判定层**：判定只落在 `cargo test` 的断言与退出码；不新增脚本 / 封装命令 / 专用参数；不使用 Hash / 指纹 / revision pin / run-id / manifest / 时间证明。
- **验证边界**：本 Cycle 跑上列定向回归，**不跑全量**（整个 change 的全量预算在 Iteration 009 / T16）。

**Gate 2 Readiness**

- Step 5 逐项复核：TBD / TODO = 无；需求裁剪 = 无（delta spec 的恢复面条款已按两类写细，页级形态的恢复面处置已补全，**Simplified** = 0）；tasks / specs / design 一致 = 一致（T5 = 恢复面组，与 D7 的恢复面行、delta 的恢复面 Scenario、tasks.md Iteration 005 的方向裁定逐条对应）；Current-State Evidence 全部来自当前工作区的逐行核对（5 处读点的实测行号与所属函数、恢复的执行顺序与 `will_redo` 门控、W2 候选集来源、W5 的可达性、既有「过短负载」四处分支、写侧防线现状、字节算术、告警承载面、只读不恢复、崩溃模拟形态、页号取得方式、20 个回归目标全部实测存在）；每个 task 的执行依据完整；调用者、影响范围、测试入口已定位（5 处读点 + 恢复的 4 个阶段 + 20 个回归目标 + 1 个新测试文件）；未修改无关范围（Forbidden 已列明）；Iteration Plan 覆盖全部任务且高开销验证预算明确（全量归 Iteration 009 / T16）；**Act 只需读本 Cycle 文件即可建立第一个先行测试并执行**（字节算术、页号取得方式、预期 RED 形态、错误文案形态、CLI 启动方式、两个面的处置语义均已写明，无需回读前序 Cycle / 其它规划资料 / 重新调查）；非实质未知项已留在 Risks and Notes；验证直接证明目标行为，行为类 scenario 走真实入口，无身份证明机制与判定层。⇒ **全部 PASS**。
- RTM：`page-read-bounded-read` Requirement + 「恢复面上的损坏槽按恢复类别分别处置」+ 「页头声称的槽数超出槽目录容量时读路径不 abort」（恢复面部分）+ 「未损坏路径的行为零变化」⇒ T5 ⇒ Covered（无 `Missing`、无 `Simplified`）。
- 设计闭合：两个面的处置语义（扫描类跳过 + 报出 + 继续；重放类明确错误结束）、两种损坏形态的粒度（槽 / 页）、错误变体的复用、告警承载面与计数方式、5 个读点的归属均已定稿 ⇒ 无 TBD。
- 一致性：本 Cycle 与 change 的 `tasks.md`（T5 归属 Iteration 005）、`specs/page-read-bounded-read/spec.md`、`design.md` D7 一致。
- Persisted Evidence：`none`（见下）。
- 用户批准：本 goal 指令（「继续实施，但是缩小目标，只把当前 ms 做完就好」；原始 goal：「按照流程把最新规划的这一批 ms 做完……开始吧」）构成 Gate 1 / Gate 2 的批准授权；**批准不降低判据**，上述各项仍逐条检查并记录。

**Cycle RTM**

| Requirement / Scenario | Task | First Test（计划名） | Status |
|---|---|---|---|
| PRBR Scenario「恢复面上的损坏槽按恢复类别分别处置」（扫描类条款） | T5 | `tests/recovery_bounded_read_test.rs::…`（计划名：扫描类损坏槽被跳过并被报出、恢复继续；Act 可改名并记录实际名） | Covered |
| PRBR Scenario「恢复面上的损坏槽按恢复类别分别处置」（已提交记录重放类条款） | T5 | 同上（计划名：已提交记录重放类以明确错误结束恢复与打开） | Covered |
| PRBR Scenario「页头声称的槽数超出槽目录容量时读路径不 abort」（恢复面部分，含「按逻辑 id 查找某个槽」） | T5 | 同上（页级形态用例） | Covered |
| PRBR Scenario「未损坏路径的行为零变化」 | T5 | 同上（未损坏对照）+ 定向回归清单 | Covered |

**Persisted Evidence**

- Mode: none

全部结论可低成本重跑（1 个测试目标 + `cargo test --lib` + 20 个既有回归目标），命令与实际结果写入 Act Response 即可承载；不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ 不创建 `evidence/` 目录，不预建占位目录。

**Risks and Notes**

- **既有基线条件（非本 Cycle 引入，不纳入本 change）**：项目无 `rust-toolchain.toml`；HEAD（`e9cb9d3`）上 `cargo fmt --check` 报 **50 处 diff 分布在 9 个既有文件**，`cargo clippy --all-targets -- -D warnings` 报 **17 条 lib 错误**（deny 模式下 lib 即中止）、非 deny 模式 **32 条 warning** ⇒ 基线在 HEAD 即红。本 Cycle **只核对本 Cycle 触碰的文件**零新增 finding，**不重排**无关文件 ⇒ 该排除已记录在 `design.md` D13（ISS36）。
- **W5 是本 Cycle 新识别的恢复面 abort 点**：页级形态（ISS35）在 `src/wal/**` **未被前序 Iteration 覆盖**（Iteration 002 的 12 个取槽点全在数据页组）⇒ 由 delta spec 的页级形态 Scenario（WHEN 含「按逻辑 id 查找某个槽」）纳入本 Cycle ⇒ 不构成范围扩张。
- **W2 自身损坏槽分支不可达**：W1 先建立版本映射且只收录未被跳过的槽 ⇒ W2 的候选集不含损坏槽 ⇒ W2 的可观察后果由既有错误（「old key has no version before row …」/「tuple mismatch with WAL old_tuple」）给出。⇒ **W2 的实现仍必须做**（Requirement 覆盖恢复面的全部负载读取；`:459` 的取槽需要页级判据前置），但**测试不断言** W2 的损坏槽分支的错误文案只含页号 ⇒ 测试断言的是「打开以明确错误结束、进程不 abort」。
- **「报出」的可判定性**：恢复告警走 stderr ⇒ 只有**读写模式**的进程能观察到（只读打开不触发恢复）⇒ 断言「报出」的用例必须走**真二进制子进程**；**不得**为此新增公共 API 字段或判定层。
- **用例 A 的恢复结果依赖被损坏行是否被 UPDATE 触及**：用例 A 损坏**未被 UPDATE 触及**的行 ⇒ 其 INSERT 记录的重放是**位置寻址**且幂等探测只读槽目录 ⇒ 落位被跳过 ⇒ 恢复成功 ⇒ 打开成功 ⇒ 断言「其余可恢复记录继续重放」。若 Act 发现该形态导致重放以其它明确错误结束（属**既有**语义，非本 Cycle 的判定对象），**记录实际形态**并按「打开以明确错误结束、进程不 abort」判定 ⇒ **不得**为了让用例通过而改变既有重放语义（那是 Stop when 的触发条件）。
- **API 名称的耦合**：Iteration 002 的页级判据的**方法名**由 Act 决定；本 Cycle 只依赖其**语义**（问「页头声称的槽数是否超出该页的槽目录容量」）⇒ 若名称不同，本 Cycle 按实际名称调用即可（**非返工**）。
- **既有合同的逐条核对结论**：`wal-recovery-frame-parsing`（WAL 帧解析）—— 本 Cycle 不改帧解析，恢复读的是**页内容**而非 WAL 帧 ⇒ 不受影响；`wal-writer-handle-reuse`（写句柄复用）—— 本 Cycle 不改写句柄生命周期 ⇒ 不受影响；`release-durability-precondition`（释放前的持久性前置）—— 本 Cycle 不改 close / checkpoint 路径 ⇒ 不受影响；`mvcc-tombstone-visibility`（墓碑可见性）—— 本 Cycle 改的是**恢复读取**的有界性，墓碑的可见性判定（`VersionHeader::is_deleted` 等）未改 ⇒ 不受影响；`drop-persistence-boundary`（释放与持久化的边界）—— 本 Cycle 不改释放路径 ⇒ 不受影响；`page-read-bounded-read`（本 change 的新增域）—— 本 Cycle 执行其恢复面部分。
- **跳过项**：本 Cycle 不跑全量回归（change 预算归 Iteration 009 / T16，非跳过而是预算安排）；不做全仓 fmt / clippy 修复（既有基线条件）；不改恢复的写路径语义；不改四处「过短负载」的既有语义。

**后续任务边界**

- **授权边界**：无（T5 在 goal 指令授权范围内）。
- **能力边界**：无。
- **停止边界**：本 Cycle 完成且 Plan Review 判定 `accepted` ⇒ 展开 Iteration 006（T6 / T7 / T8，DDL 提交点统一模型）；Act 不得自行创建下一 Cycle / 下一 Iteration。
- **不在本 Cycle、需在后续处理的事项**：T6 / T7 / T8（Iteration 006）；T9 / T10（Iteration 007）；T11 / T12（Iteration 008）；T13 / T14 / T15 / T16（Iteration 009）。
- **交 Maintainer / Recorder 的事项**：ISS27 台账条目的关闭须等 T4 / T5 全部完成（由 Recorder 按用户指令执行）；本 Cycle 不登记 Issue、不改全局状态（`.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆均不动）、不合并规格。

## Act Response

**状态**: `reported`
**Cycle**: Iteration 005 / Cycle 000（`initial`）　**Task**: T5（恢复面组：WAL 重放面上的有界读）
**Act 执行日期**: 2026-10-26

### 接手时的实际状态

- 工作区 HEAD `e9cb9d3`。未提交修改：`.agents/docs/tasks.md`（MS37–MS40 路线批次，既有现场 —— **未回滚、未清理、未提交**）+ Iteration 000–004 的产品代码与测试改动（`src/storage/page_format/slotted_page.rs`、`src/storage/page_format/mod.rs`、`src/storage/data_page.rs`、`src/storage/data/table_manager.rs`、`src/storage/buffer_pool.rs`、`src/storage/catalog.rs`、`src/storage/integrity.rs`、`src/storage/error.rs`、`src/storage/mod.rs`、`src/storage/btree/node.rs`、`src/executor/*`、`src/transaction/manager.rs`、`src/pipeline.rs`、`tests/data_page_bounded_read_test.rs`、`tests/write_path_slot_bounds_test.rs`、`tests/index_read_bounded_read_test.rs`、`tests/release_path_bounded_read_test.rs`、`tests/catalog_bounded_read_test.rs`）+ ISS35–ISS38 台账条目 + 本 change 产物目录。
- **本 Cycle 无既有产物**：`src/wal/**` 在开工时**零 diff**（`git status --short` 未列 `src/wal/recovery.rs`）；`tests/recovery_bounded_read_test.rs` **不存在**。⇒ **不是断点续做，是从零实施**；无重复实现、无他人改动被回退。
- 前置条件核对：`Plan Context: ready`、`Review Result: pending`、无后继 Cycle；Iteration 002（页级槽数判据冻结）、Iteration 003（恢复面**写点**收口）、Iteration 004（目录页组）均已 `accepted` ⇒ **可执行**。
- **写侧防线的现状核对（Cycle 要求只核对、不重复实现）**：`src/storage/page_format/slotted_page.rs:373-377` 的 `add_slot_inner` 页级判据前置（`PageWriteError::SlotCountExceedsCapacity`）与 `src/storage/data_page.rs:306` 的 `update_version_header_in_data_page` 页级判据前置**均在位** ⇒ 本 Cycle 对这两个文件**零 diff**。

### Implemented（实际改动）

**`src/wal/recovery.rs`** —— 新增 1 个私有枚举 + 1 个私有计数结构 + 1 个私有报出函数，改造 **5 处**读点：

| 符号 | 实际行号 | 作用 |
|---|---|---|
| `enum RecoverySkip` | 134 | 恢复面扫描类的**跳过记录**：页级形态（`Page { page_id, slot_count }`）/ 槽级形态（`Slot { page_id, slot_index, offset, length }`） |
| `struct RecoverySkipCount` | 148 | 扫描点的**局部**跳过计数（`pages` / `slots`）——不进 `RecoveryResult` 公共面 |
| `RecoverySkipCount::of` | 157 | 由跳过记录汇总出局部计数 |
| `fn report_recovery_skips` | 173 | **报出**（沿用既有 `eprintln!` 形态 ⇒ stderr）：一行局部计数汇总 + 逐条细节（页级形态点名该页与页头声称的槽数；槽级形态点名该页与该槽序号与越界区间） |

| 读点 | 类别 | 实际行号 | 读点内改动 |
|---|---|---|---|
| **W1** `build_pk_version_maps` | 扫描类 | 485（页级判据）/ 498（有界负载）/ 529（报出） | 页级判据前置 ⇒ **跳过整页**并报出；槽负载改 `get_slot_data_checked` ⇒ 越界**跳过该槽**并报出；其余槽继续入映射 |
| **W2** `derive_old_row_id` | 已提交记录重放类 | 603（页级判据）/ 615（有界负载） | 页级判据前置 ⇒ `CorruptedDataPage`（页级）；有界负载 ⇒ `CorruptedDataPage`（槽级）⇒ **不跳过该记录**，以明确错误结束恢复与打开 |
| **W3** `rebuild_pk_indexes` | 扫描类 | 1339（页级判据）/ 1352（有界负载）/ 1399（报出） | 同 W1：跳过整页 / 跳过该槽 + 报出，其余槽继续重建索引 |
| **W4** `mark_uncommitted_aborted` | 扫描类 | 1683（页级判据）/ 1696（有界负载）/ 1732（报出） | 同 W1：跳过整页 / 跳过该槽 + 报出，其余槽继续标记 |
| **W5** `redo_tuple_at_row_id`（幂等探测） | 已提交记录重放类 | 262（页级判据） | 闭包改为返回 `Result<bool>`；页级判据前置 ⇒ `CorruptedDataPage`（页级）⇒ **不跳过该记录**，以明确错误结束恢复与打开 |

- **复用的冻结判据**（未另造阈值、未第二处实现）：页级槽数判据 = `SlottedPageRef::slot_count_exceeds_capacity()` / `SlottedPage::slot_count_exceeds_capacity()`（Iteration 002 冻结的唯一实现）；槽级判界 = `SlottedPageRef::get_slot_data_checked`（Iteration 001 / 003 冻结）。
- **复用的错误变体**：`StorageError::CorruptedDataPage`（T3 冻结）经既有构造器 `corrupted_data_page_slot_count_error` / `corrupted_data_page_slot_error`（`src/storage/data_page.rs`，**零 diff**）产出 ⇒ **未新增错误变体**、`src/storage/error.rs` **零 diff**。
- **既有的「过短负载」四处逐点语义零改动**：W1 / W3 / W4 的 `len < VersionHeader::SIZE ⇒ continue`、W2 的 `StorageError::Internal("derived slot … shorter than version header")`。
- **写路径语义零改动**：位置寻址落位、幂等跳据、`add_slot_with_logical_id` 的强制落位、`update_version_header_in_data_page` 的版本头写均未动。

**`tests/recovery_bounded_read_test.rs`**（**新建**）—— 6 个用例，真实文件库 + 真实 `Database::open` + 真实 CLI 二进制（`CARGO_BIN_EXE_rtsql`）+ **真实文件字节改写**（无替身 / 桩）：

- `scanning_class_skips_the_damaged_slot_and_the_replay_continues`（用例 A · 库面：槽级形态，损坏未被 UPDATE 触及的行）
- `scanning_class_reports_the_skipped_slot_on_the_read_write_cli_stderr`（用例 A · CLI 面：读写模式真二进制断言退出码 + stderr）
- `replay_class_fails_the_open_with_a_named_error_on_a_damaged_page_header`（用例 B · 库面：页级形态）
- `replay_class_fails_the_open_with_the_existing_redo_error_on_a_damaged_old_row`（用例 C · 库面：槽级形态，损坏 UPDATE 旧行所在槽）
- `scanning_class_reports_the_skipped_slot_while_marking_uncommitted_rows`（用例 D · CLI 面：**未提交事务标记点** W4）
- `undamaged_recovery_is_unchanged`（未损坏对照）

**夹具形态**（实测）：`Database::open` → `CREATE TABLE t (id INT PRIMARY KEY, v TEXT)` → 插入 2 行 → 对第 2 行一次 `UPDATE` → `wal_buffer.shutdown()`（WAL 落盘，**不** checkpoint / 不截断）→ `buffer_pool.flush_all()`（脏页落盘 ⇒ 磁盘上的数据页真实承载 3 个槽）→ `drop(db)` ⇒ 下次打开触发全量重放。实测数据页号 = **2**（页 0 = `__tables`，页 1 = `__columns`），页头 `slot_count` = **3**。

### W1–W5 逐点处置落地情况

| 读点 | 类别 | 处置落地（实际形态） | 可达性证据（实测） |
|---|---|---|---|
| **W1** `build_pk_version_maps` | 扫描类 | 页级形态 ⇒ **跳过整页**（`entries` 为空）+ 报出；槽级形态 ⇒ **跳过该槽** + 报出；其余槽继续入版本映射 | 用例 A / B / D 的 stderr 均出现 `[recovery] pre-scan of table 't': skipped …` ⇒ **可达且已报出** |
| **W2** `derive_old_row_id` | 已提交记录重放类 | 页级 / 槽级越界 ⇒ `CorruptedDataPage` ⇒ **不跳过该记录**，恢复与打开以明确错误结束 | **用例 C 实测命中 W2 自身的越界槽分支**（见下「与 Cycle 预期的偏差」）；W2 的页级分支在用例 B 中**未被触达**（W1 先跳页 ⇒ W5 先失败）⇒ W2 的页级分支**在本恢复流下不可达**（与 Cycle 判定一致），但**已按 Requirement 实现** |
| **W3** `rebuild_pk_indexes` | 扫描类 | 同 W1 | 用例 A / D 的 stderr 均出现 `[recovery] index rebuild of table 't': skipped …` ⇒ **可达且已报出** |
| **W4** `mark_uncommitted_aborted` | 扫描类 | 同 W1 | 用例 D 的 stderr 出现 `[recovery] mark uncommitted aborted of table 't': skipped …` ⇒ **可达且已报出**（该点有既有早退：`uncommitted_tx_ids` 为空即零工作 ⇒ 用例 D 的夹具额外开一个**未提交**事务（WAL 留 `BeginTxn` 无 `CommitTxn`）才进入该点） |
| **W5** `redo_tuple_at_row_id`（幂等探测） | 已提交记录重放类 | 页级判据前置 ⇒ `CorruptedDataPage`（点名该页与页头声称的槽数）⇒ **不跳过该记录**，恢复与打开以明确错误结束 | **用例 B 实测命中**：错误原文 `redo_tuple_at_row_id for row_id RowId { page_id: 2, slot_id: 0 } idempotency probe failed: corrupted data page 2: slot count 700 exceeds the page slot directory capacity of 680 slots` ⇒ **W5 是可达的**（重放按位置寻址，不受 W1 跳页影响） |

**W5 可达性的独立佐证**：W5 的页级判据是本 Cycle **首次**在 `src/wal/**` 生效 ⇒ 全项目 `get_slot_by_logical_id` 的其余 5 个调用点（`src/storage/data_page.rs:239` / `:314` / `:369`、`src/storage/buffer_pool.rs:433` / `:496`）均已由 Iteration 002（T17）加页级判据 ⇒ **恢复面是最后一个未加判据的调用面**，本 Cycle 关闭后全项目该原语无裸调用。

**W2 自身损坏槽分支的可达性 —— 与 Cycle 预期的偏差（重要）**：Cycle 的「关键判定」为「`derive_old_row_id` 的候选集**只**来自 W1 建立的版本映射 ⇒ W1 跳过损坏槽后该槽不在候选集内 ⇒ W2 的越界槽分支不可达」。**实测该判定不成立**：`redo_record` 的 Insert 臂在 deindexed 模式下会把 WAL 记录的 `row_id` **再次** `insert_row_id_sorted` 进**同一份** `ctx.pk_versions` 映射（`src/wal/recovery.rs` Insert 臂，deindexed 分支）⇒ 被 W1 跳过的槽的 rid 会经**重放本身**重新进入候选集 ⇒ **W2 的越界槽分支可达**，且**正是用例 C 的实际错误路径**。⇒ Cycle 关于「W2 分支不是测试断言对象」的指示作废：**W2 的实现被用例 C 直接覆盖**（更严格的覆盖，非缺陷）。

### RED → GREEN 实况

**RED（变更前，实测，逐用例单独运行以取得可判定证据）**

| 用例 | 观察到的实际形态 |
|---|---|
| A（扫描类 · 槽级形态） | `thread '…' panicked at src/storage/page_format/slotted_page.rs:244:19:` → `range start index 65535 out of range for slice of length 4096`；unwinding 中在 `src/storage/page_frame.rs:101:27` 二次 panic ⇒ **进程 abort**（`signal: 6, SIGABRT`） |
| A（CLI 面） | 子进程被**信号终止**（退出码 `None`）⇒ 断言「必须以普通退出码结束」失败 |
| B（重放类 · 页级形态） | `panicked at src/storage/page_format/slotted_page.rs:244:19:` → `range start index 25088 out of range for slice of length 4096` ⇒ **进程 abort**（SIGABRT） |
| C（重放类 · 槽级形态） | `range start index 65535 out of range for slice of length 4096` ⇒ **进程 abort**（SIGABRT） |
| D（扫描类 · 未提交事务标记点 W4） | 临时回退 W4 单点改动后：CLI 子进程被**信号终止**（SIGABRT）⇒ **W4 自身的 RED 成立**（回退只限 W4 一处，随后原样恢复并复跑 GREEN） |
| 未损坏对照 | 变更前即 **GREEN**（`1 passed; 0 failed`） |

⇒ RED 形态与 Cycle 钉死一致：三个损坏用例都是**行为失败**（进程 abort），**不是编译失败**；测试断言只落在**行为**上（打开结果 / 进程是否存活 / stderr 文本），**未**对尚不存在的错误变体做类型级断言。

**GREEN（变更后，实测）**

| 用例 | 观察到的实际形态 |
|---|---|
| A（库面） | `Database::open` → **Ok**（恢复完成，不 panic / 不 abort）；点查 `WHERE id = 2` → `QueryResult { rows: [[Number(2), String("c")]] }`（**未被损坏的行继续被重放并进重建后的主键索引**，值 `c` 为 UPDATE 后的版本）；点查 `WHERE id = 1` → `QueryResult { rows: [] }`（**被损坏的行在恢复中被跳过**） |
| A（CLI 面） | 退出码 **0**（**普通退出码，非信号终止**）；stdout = `checkpoint complete (wal truncated; checkpoint site advanced)`；**stderr** = `[recovery] pre-scan of table 't': skipped 1 damaged slot(s)` / `[recovery] pre-scan of table 't': skipped slot 0 of page 2 (its payload range [65535, 65570) lies outside the page data area)` / `[recovery] index rebuild of table 't': skipped 1 damaged slot(s)` / `[recovery] index rebuild of table 't': skipped slot 0 of page 2 (its payload range [65535, 65570) lies outside the page data area)` ⇒ **点名该页与该槽序号，跳过被报出** |
| B | 打开以 `Err` 结束：`WAL error: WAL redo failed: redo_tuple_at_row_id for row_id RowId { page_id: 2, slot_id: 0 } idempotency probe failed: corrupted data page 2: slot count 700 exceeds the page slot directory capacity of 680 slots` ⇒ **不 panic、不 abort**，点名该页与页头声称的槽数 |
| B（CLI 面） | 退出码 **1**（普通非零）；stderr = 预扫描的整页跳过告警 + `{"error":"failed to open database /tmp/…/b.db: WAL error: WAL redo failed: … corrupted data page 2: slot count 700 exceeds the page slot directory capacity of 680 slots"}` |
| C | 打开以 `Err` 结束：`WAL error: WAL redo failed: update redo: table 't' derived slot RowId { page_id: 2, slot_id: 1 } read failed: corrupted data page 2: slot 1 range [65535, 65570) lies outside the page data area` ⇒ 文案含 `update redo`，**不 panic、不 abort** |
| D（CLI 面） | 退出码 **0**；stderr 含 `[recovery] mark uncommitted aborted of table 't': skipped 1 damaged slot(s)` 与 `skipped slot 0 of page 2 (…)` ⇒ **W4 的跳过被报出**，其余记录继续被标记（恢复完成 ⇒ 打开成功） |
| 未损坏对照 | 全表扫描 → `QueryResult { rows: [[Number(1), String("a")], [Number(2), String("c")]] }` ⇒ 两行都在，与既有行为一致 |

**可观察的组合形态（非缺陷，记录）**：用例 A 下恢复**成功**（跳过 + 报出），但随后的**全表扫描**以 `Execution error: corrupted data page 2: slot 0 range [65535, 65570) lies outside the page data area` 结束 —— 这是 **Iteration 001（T3）既有**的数据页读路径语义（数据页损坏 ⇒ 点名错误，**不静默跳过该行**），与本 Cycle 的「恢复面按恢复类别处置」是**两个面各自的既定裁定**（恢复面扫描类跳过并报出；读路径面严格报错）。⇒ **非本 Cycle 的回归**，也不是本 Cycle 可改的范围（读路径面归 Iteration 001）。

### Deviations from Plan（与计划的偏差及原因）

1. **用例 C 的实际错误来源与 Cycle 预测不同（W2 自身的越界槽分支，而非「old key has no version before row …」）** —— 原因见上节「W2 自身损坏槽分支的可达性」：Cycle 的候选集来源判定漏了 `redo_record` Insert 臂对**同一份**映射的再次写入 ⇒ W2 的越界槽分支**可达**且是实际路径。**影响**：Cycle 关于「W2 分支不是测试断言目标」的指示作废 ⇒ 该分支被用例 C **直接覆盖**（覆盖面**增加**，非减少）。Cycle 的 GREEN 条件（「打开以明确错误结束、进程不 abort」、文案含 `update redo`）**逐字满足** ⇒ **非实质偏差**。既有重放语义**未改**（未为让用例通过而改既有语义）。
2. **本 Cycle 新增一个用例（用例 D），承载 Cycle 未点名的 W4** —— Cycle 的 First test 只列了 A / B / C + 未损坏对照 + CLI 面；实测 A / B / C **均不进入 W4**（`mark_uncommitted_aborted` 的既有早退：`uncommitted_tx_ids` 为空即零工作返回）⇒ **W4 在 Cycle 的用例集下零覆盖**。为满足 A16「**全部**取槽点在取槽之前问页级槽数判据」的验收证据，Act 在本 Cycle 的目标文件内新增用例 D（夹具额外开一个**未提交**事务）。**顺序合规**：先写用例 D ⇒ 临时回退 W4 单点改动 ⇒ 观察**真实 RED**（CLI 子进程 SIGABRT）⇒ 原样恢复 W4 ⇒ GREEN ⇒ **行为变更前存在真实 RED**。**未新增**产品代码、未越出 Cycle 的 Target 文件、未改既有语义。
3. **「报出」的实现形态比 Cycle 描述多一行**：Cycle 要求「计数用**局部计数**（每个扫描点各自维护）」且「报出内容点名该页与该槽序号」。Act 的实现 = **一行局部计数汇总**（`skipped N damaged slot(s)` / `skipped N damaged page(s)`）+ **逐条细节行**（点名页与槽序号）。计数因此**可观察**（而不只是内部变量）；两行都走 stderr，**未新增公共 API 字段 / 命令参数 / 配置项 / 缓存 / 闸门机制**。
4. **Cycle 的 Target 行号按符号名重新定位**（Cycle 写于本 Cycle 实施之前）—— 实际行号见 Implemented 表；W1 `:374`→`:485`/`:498`、W2 `:459`/`:462`→`:603`/`:615`、W3 `:1177`→`:1339`/`:1352`、W4 `:1490`→`:1683`/`:1696`、W5 `:166`→`:262`。行号差异**不是失败**。
5. **页级形态的 RED panic 文本与 Cycle 预测不同**：Cycle 预测「槽目录定位下溢 ⇒ `range start index …` 或 debug profile 的 `attempt to subtract with overflow`」；实测为 `range start index 25088 out of range for slice of length 4096` —— 页头 `slot_count` = 700 时，**垃圾槽目录项**（索引 3..681 读到数据区残留字节）的 `offset` 字段先触发**负载**越界，早于槽目录定位下溢 ⇒ 仍为**进程 abort**，**非失败**。
6. **夹具用 `buffer_pool.flush_all()` 而非 `checkpoint()` 落盘**：Cycle 的夹具未指定落盘方式；`checkpoint()` 会**截断 WAL** ⇒ 下次打开不触发重放 ⇒ 与「崩溃模拟 ⇒ 触发全量重放」的前提冲突。`flush_all()` 只落脏页、**不截断 WAL** ⇒ 磁盘上的数据页真实承载 3 个槽（前置断言 `page_slot_count == 3` 已固化在夹具内），WAL 仍保有全部记录 ⇒ 重放走「幂等命中」路径。属**夹具形态的等价调整**，不改产品语义。
7. **CLI 面用例选用 `rtsql checkpoint <db>`（读写模式）而非 `rtsql check <db>`**：`check` 走**只读**打开（`src/cli/lifecycle.rs:281` 的 `readonly = true`）⇒ **不触发恢复** ⇒ 无法观察恢复告警；`checkpoint` 走读写打开（`execute_command_inner` ⇒ `readonly = false`）⇒ 触发恢复。属 Cycle「断言『报出』的用例必须走读写模式的真二进制子进程」的执行方式落地，**未新增命令参数**。
8. **定向回归命令与 Cycle 清单逐字一致**：批 1（10 个目标）与批 2（10 个目标）均按 Cycle 给出的命令执行，批内 `--test-threads=1`、批间串行、**批 1 全绿后才启动批 2**。

### Self-Review（任务级 Review 与自检）

**Gate 3 · Spec compliance**

- **A16**（恢复面的**全部**槽负载读取经有界路径取得；**全部**取槽点在取槽之前问页级槽数判据）：**满足**。W1 / W2 / W3 / W4 的槽负载**全部**经 `get_slot_data_checked`（`get_slot_data` 在 `src/wal/recovery.rs` 中**已无任何调用点** —— 逐点检索确认）；W1 / W2 / W3 / W4 / W5 **五处**全部在取槽之前消费 `slot_count_exceeds_capacity()`；**未产生任何越界切片、未 abort**（4 个损坏用例的 GREEN 形态均为「打开成功」或「以明确错误结束」）。
- **A17**（扫描类：跳过该槽（页级形态 ⇒ 跳过该**页**）、**计入恢复告警并报出**、其余可恢复记录继续重放）：**满足**。W1 / W3 / W4 三点均实现「页级 ⇒ 整页跳过；槽级 ⇒ 该槽跳过」+ **局部计数 + stderr 报出**（点名该页；槽级形态另点名该槽序号与越界区间）；其余记录继续被重放（用例 A：未被损坏的行点查命中且值为 UPDATE 后版本 `c`；用例 D：恢复完成 ⇒ 打开成功）。
- **A18**（已提交记录重放类：**不跳过该记录** ⇒ 以**明确的错误**结束恢复与打开）：**满足**。W2 / W5 两点均**不跳过**记录，改为 `StorageError::CorruptedDataPage` ⇒ `WalError::RedoFailed` ⇒ 恢复与打开以明确错误结束（用例 B / C 实测：进程存活、退出码普通非零、错误文案点名该页与该槽 / 声称的槽数）。
- **A19**（未损坏路径行为零变化，含四处既有「过短负载」逐点语义；恢复的执行顺序与 `will_redo` 门控不变）：**满足**。四处「过短负载」分支**逐字未改**；恢复的执行顺序（W1 → 重放 → W4 → W3）、`will_redo` 门控、`redo_count == 0` 的 clean 打开路径、`RedoContext` 结构**均未改**；未损坏对照 + `cargo test --lib` 420/0 + 20 个定向回归目标全绿。
- **RTM 四行**（扫描类条款 / 重放类条款 / 页级形态恢复面部分 / 未损坏零变化）⇒ **Covered**，无 `Missing`、无 `Simplified`。
- **Preserve 清单逐项核对**：`get_slot_data_checked` / `slot_payload_in_bounds` / `slot_count_exceeds_capacity` 的判据与语义**未改**（`src/storage/page_format/**` 本 Cycle **零 diff**）；`get_slot` / `get_slot_by_logical_id` / `get_slot_data` 的签名与 `None` 语义**未改**（页格式层零 diff）；`StorageError::CorruptedDataPage` 的变体形状与文案前缀**未改**（`src/storage/error.rs` 本 Cycle **零 diff**）；`WalError::RedoFailed` 的既有文案形态**未改**；恢复的**写路径**语义与 Iteration 003 已交付的防线**未改**；磁盘格式、checkpoint / WAL 截断策略**未改**；`release-durability-precondition` / `wal-writer-handle-reuse` 的既有语义**未改**（本 Cycle 不改写句柄生命周期与 close / checkpoint 路径）。
- **Forbidden 逐项核对**：`src/storage/page_format/**`、`src/storage/btree/**`、`src/storage/integrity.rs`、`src/storage/data_page.rs`、`src/storage/buffer_pool.rs`、`src/storage/executor/**`、`src/transaction/**`、`src/pipeline.rs`、`src/storage/error.rs` ⇒ **本 Cycle 零 diff**（工作区中这些文件的既有改动是 Iteration 000–004 的现场，非本 Cycle）。未把「过短负载」改成报错；未把扫描类的跳过改成静默（**必报出**）；未把重放类改成跳过该记录；**未新增**公共 API 字段 / 命令参数 / 配置项 / 缓存 / 闸门机制 / 判定层 / 身份证明机制；未改磁盘格式与 dump / restore 合同面；**未跑全量回归**（按预算归 Iteration 009 / T16）。
- **两个面分别处置（不得笼统）**：已逐点核对 —— W1 / W3 / W4 的代码路径**只**做「跳过 + 报出」（无 `Err` 传播）；W2 / W5 的代码路径**只**做「明确错误」（无跳过分支）。

**Gate 3 · Code quality**

- **diff 无计划外修改**：改动仅落在 Cycle 的两个 Target 上（`src/wal/recovery.rs` + 新测试文件）。全项目符号检索确认本 Cycle 新增的符号（`RecoverySkip` / `RecoverySkipCount` / `report_recovery_skips`）**只**出现在 `src/wal/recovery.rs`。
- **错误 / 边界 / 状态 / 资源生命周期正确**：页级判据与槽级判界各**只有一处**实现（复用冻结判据，未另造阈值）；新代码**无** `unwrap` / `expect` / panic 路径；`report_recovery_skips` 在**页锁闭包之外**调用（跳过记录在闭包内收集、闭包外打印）⇒ 不在页锁内做 IO。
- **无新增警告**：`cargo clippy --all-targets`（非 deny）在本 Cycle 触碰的 2 个文件上**零 finding**；全项目 warning 数 **32 条**，与**变更前实测**（把本 Cycle 的 2 个文件临时还原后同命令复跑）**逐项一致**（per-target 汇总行 `diff` 完全相同）⇒ **零新增**。
- **格式**：`rustfmt --edition 2021 --check` 在本 Cycle 触碰的 2 个文件上 **0 处 diff**（`src/wal/recovery.rs` 在 HEAD 上亦为 0 处 ⇒ 本 Cycle 自身引入的 3 处已做**精准修正**，**未**对全仓跑 `cargo fmt`，**未**重排 9 个既有红文件）。
- **测试不会因错误原因通过**：损坏注入是**真实文件字节改写**（非替身 / 桩），观察对象是**打开结果 / 进程是否存活 / stderr 文本**；CLI 用例走真二进制 `CARGO_BIN_EXE_rtsql` 并断言**退出码**与**输出文案**；断言不含对尚不存在的错误变体的类型级匹配（只断言**错误文案**含 `corrupted data page 2` / `slot count 700` / `update redo`）。
- **无身份证明机制、无判定层**：判定只落在 `cargo test` 的断言与退出码；未新增脚本 / 封装命令 / 专用参数 / Hash / 指纹 / revision pin / run-id / manifest / 时间证明。

**自检发现与已修复内容**（本 Cycle 实施过程中发现并在本 Cycle 内修复）：

1. **W4 零覆盖（Cycle 用例集不进入该点）** —— 用例 A / B / C 均不触发 `mark_uncommitted_aborted`（既有早退：`uncommitted_tx_ids` 为空即返回）⇒ W4 在 Cycle 计划下无任何断言目标。**已修复**：新增用例 D（夹具开一个未提交事务 ⇒ WAL 留 `BeginTxn` 无 `CommitTxn`）⇒ W4 获得直接覆盖（并先行取得真实 RED）。
2. **本 Cycle 自身引入的 3 处 rustfmt 差异**（`report_recovery_skips` 的两个 match 臂换行、`derive_old_row_id` 的 `let Some((slot, slot_index)) = …` 换行）。**已修复**：只对**本 Cycle 触碰的 2 个文件**跑 `rustfmt`（未对全仓跑 `cargo fmt`，未重排 9 个既有红文件）⇒ 本 Cycle 文件 0 处 diff。
3. **Case B 的 GREEN 错误文案需实测确认而非照抄 Cycle 预测** —— Cycle 预测「用例 B 的错误来自 W5 的页级判据（点名该页与页头声称的槽数）」，实测**完全一致**（`redo_tuple_at_row_id … idempotency probe failed: corrupted data page 2: slot count 700 exceeds the page slot directory capacity of 680 slots`）⇒ 该预测成立，无须修正。
4. **夹具落盘方式与「触发重放」前提冲突（潜在）** —— `checkpoint()` 会截断 WAL ⇒ 下次打开不重放。**已修复**：改用 `buffer_pool.flush_all()`（只落脏页），并把「页头 `slot_count` == 3」写成夹具内的**前置断言**。

**遗留 Minor 问题**（非阻塞，记录不改）：

1. **恢复成功（扫描类跳过）与读路径严格报错在**同一库**上并存** —— 用例 A 下库能打开、点查可用，但**全表扫描**以 `corrupted data page 2: slot 0 range […]` 结束（Iteration 001 的既定语义）。这是两个面各自的既定裁定的组合，**非本 Cycle 的缺陷**，也**不在本 Cycle 的可改范围**（读路径面归 Iteration 001）⇒ 记录不改。
2. **`RecoverySkipCount` 的两个计数在单次页扫描内实际上恒为「同类」**（页级形态下 `slots` 必为 0，槽级形态下 `pages` 必为 0）⇒ 两个字段目前不会同时非零；保留两字段是为了让「页级形态跳过整页」与「槽级形态跳过该槽」在报出文案上可区分。**非缺陷**，记录不改。
3. **Cycle 的「W2 越界槽分支不可达」判定不成立** —— 已在本 Response 的 W1–W5 表与 Deviations 第 1 项完整记录（实际该分支**可达**且被用例 C 覆盖）⇒ **不构成产品缺陷**，但**属于 Plan 调查事实的偏差**，供 Plan Review 参考。

### Verification Evidence（验证命令与实际结果）

| # | 命令 | 实际结果 |
|---|---|---|
| 1 | `cargo test --test recovery_bounded_read_test -- --test-threads=1`（**RED**，变更前；逐用例 `--exact` 单独运行） | A：`panicked at src/storage/page_format/slotted_page.rs:244:19: range start index 65535 out of range for slice of length 4096` ⇒ **SIGABRT**；A（CLI）：子进程被信号终止（退出码 `None`）；B：`range start index 25088 out of range for slice of length 4096` ⇒ **SIGABRT**；C：`range start index 65535 …` ⇒ **SIGABRT**；D（临时回退 W4 单点后）：CLI 子进程 **SIGABRT**；未损坏对照：**ok** ⇒ **RED 成立（行为失败，非编译失败）** |
| 2 | `cargo test --test recovery_bounded_read_test -- --test-threads=1`（**GREEN**，变更后） | `test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s` |
| 3 | `cargo test --lib` | `test result: ok. 420 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.76s` |
| 4 | 定向回归 **批 1**（10 个目标，Cycle 清单逐字一致，批内 `--test-threads=1`） | 全部 `ok`，合计 **48 passed / 0 failed**：`checkpoint_redo_reduction_test` 9、`ddl_crash_recovery_test` 6、`recovery_e2e_test` 6、`recovery_error_surface_test` 3、`recovery_replay_convergence_test` 3、`recovery_tail_persistence_test` 4、`recovery_test` 3、`recovery_wal_convergence_test` 5、`wal_integration_test` 2、`wal_recovery_large_test` 7 |
| 5 | 定向回归 **批 2**（10 个目标，**批 1 全绿后才启动**，串行） | 全部 `ok`，合计 **95 passed / 0 failed**：`explicit_tx_test` 21、`mvcc_abort_test` 3、`mvcc_commit_test` 4、`overflow_page_test` 27、`version_chain_test` 3、`wal_auto_checkpoint_test` 11、`wal_buffer_test` 4、`wal_handle_test` 4、`wal_record_test` 14、`wal_writer_test` 4 |
| 6 | `rustfmt --edition 2021 --check src/wal/recovery.rs tests/recovery_bounded_read_test.rs` | **0 处 diff**（HEAD 上 `src/wal/recovery.rs` 亦为 0 处 ⇒ 本 Cycle 自身引入的 3 处已修正；**未**全仓 `cargo fmt`，**未**重排既有红文件） |
| 7 | `cargo clippy --all-targets`（非 deny） | 本 Cycle 触碰的 2 个文件 ⇒ **0 finding**；全项目 **32 条 warning** —— 与把本 Cycle 的 2 个文件临时还原后同命令复跑的结果**逐项一致**（per-target 汇总 `diff` 无差异）⇒ **零新增** |
| 8 | CLI 实跑（真二进制 `CARGO_BIN_EXE_rtsql`，读写模式） | 用例 A：`rtsql checkpoint <db>` ⇒ 退出码 **0**，stderr 含 `[recovery] pre-scan of table 't': skipped 1 damaged slot(s)` + `[recovery] pre-scan of table 't': skipped slot 0 of page 2 (its payload range [65535, 65570) lies outside the page data area)` + 对称的 `index rebuild` 两行；用例 B：退出码 **1**，stderr 含整页跳过告警 + `{"error":"failed to open database …: WAL error: WAL redo failed: redo_tuple_at_row_id … corrupted data page 2: slot count 700 exceeds the page slot directory capacity of 680 slots"}`；用例 D：退出码 **0**，stderr 含 `mark uncommitted aborted of table 't'` 的跳过告警 |

**既有结论失效的处理**：Iteration 004 的 `cargo test --lib`（420/0）与定向回归数字因本 Cycle 改动 `src/wal/recovery.rs` **已失效** ⇒ 本次**全部重跑**（上表 #3 / #4 / #5），不复用旧结论。Iteration 004 的 `cargo test --test catalog_bounded_read_test`（5/0）因本 Cycle **未触碰** `src/storage/catalog.rs` / `src/storage/error.rs`（本 Cycle 二者零 diff）⇒ 该结论**覆盖范围未变化，继续有效**。

**结论时效性**：上表 #2–#5 的运行发生在本 Cycle 自身引入的 3 处 rustfmt 修正**之后**（修正仅改 `src/wal/recovery.rs` 与测试文件的排版，不改语义；修正后已重跑 #2 / #3 / #4 / #5）⇒ 上表全部结论均为**当次真实运行**的当前结论。

### Persisted Evidence

**None required.**（Plan 的 `Persisted Evidence: Mode: none`；本次 RED / GREEN 均为可低成本复现的观察（1 个测试目标 + `cargo test --lib` + 20 个既有回归目标），不满足公共规则 › 验证 的保存白名单；**未创建** `evidence/` 目录、未预建占位目录。）

### Experience Candidates

1. **Runbook 候选：「制造一次会被重放的崩溃」的夹具流程** —— 已端到端验证成功且可重复：**`wal_buffer.shutdown()`（WAL 落盘，**不** checkpoint / 不截断）→ `buffer_pool.flush_all()`（脏页落盘，使磁盘页真实承载槽）→ `drop(db)`** ⇒ 下次读写打开触发全量重放。**关键点**：用 `checkpoint()` 落盘会**截断 WAL** ⇒ 下次打开**不**重放 ⇒ 该流程不可用；本 Cycle 的 4 个损坏用例与 2 个对照用例全部依赖此形态。证据：本 Response 的 RED → GREEN 实况与 Verification Evidence #1 / #2（可低成本复跑）。
2. **Issue 候选：无** —— 本 change 范围外**未发现**有证据的实质缺陷。核对记录：全项目 `get_slot_by_logical_id` 的其余 5 个调用点（`src/storage/data_page.rs:239` / `:314` / `:369`、`src/storage/buffer_pool.rs:433` / `:496`）**均已**由 Iteration 002（T17）加页级判据 ⇒ 本 Cycle 关闭后该原语**无裸调用**；Cycle 的「W2 不可达」判定偏差属**计划调查事实**的偏差、**非产品缺陷**（且实际覆盖更严）；遗留 Minor 1（恢复面跳过与读路径报错并存）是两个面各自既定裁定的组合，**非缺陷**。

### 未解决问题

1. **无阻塞项**。Gate 3 / Gate 4 全部通过，Act Response 状态 = `reported`。
2. **待 Plan Review 裁定**：本 Cycle 的实现与验收结论；特别是 **Deviations 第 1 项**（W2 越界槽分支**实际可达**、与 Cycle 的「不可达」判定相反，且用例 C 覆盖的是该分支）与 **Deviations 第 2 项**（新增用例 D 承载 W4）是否被接受。
3. **交 Maintainer / Recorder 的事项**（本 Cycle **不执行**）：ISS27 台账条目的关闭须等 T4 / T5 全部完成（由 Recorder 按用户指令回填「处置指针」）；本 Cycle **未登记 Issue、未改台账**、**未同步全局文档**（`.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆 / references 均未动）、**未合并规格**、**未归档 change**、**未填写 `Plan Review`**、**未创建下一 Cycle 或下一 Iteration**、**未执行 `git commit`**。

### 状态同步

- change `tasks.md`：T5 状态 → `Act Response reported（Iteration 005 / Cycle 000），待 Plan Review`；Iteration Map 的 **005** 行状态 → `Cycle 000 = T5 已报告，待 Plan Review`（T6 / 006–009 与 **Next Iteration** 指针不变 —— 展开 Iteration 006 属 Plan 的权限，须待 Iteration 005 被接受）。
- 本 Cycle 的 **Plan Context** 未改写；**Plan Review** 未填写。

---

## Plan Review

- Review Result: accepted

**核对方法（独立于 Act 的验证）**

- 独立阅读当前工作区代码与 `git diff`（HEAD `e9cb9d3`）：`src/wal/recovery.rs`（全部 10 个 hunk）、`src/storage/page_format/slotted_page.rs`、`src/storage/data_page.rs`、`src/storage/buffer_pool.rs`、`src/storage/catalog.rs`、`src/storage/error.rs`、`src/wal/record.rs`、`src/transaction/manager.rs`、`src/executor/alter_table.rs`、`src/executor/ddl_commit_window.rs`、`src/storage/data/table_manager.rs`，以及 `tests/recovery_bounded_read_test.rs`（全文 441 行）。
- Act 报告的 RED / GREEN 结论**直接沿用**（来源：Act Response 的 Verification Evidence #1–#8）。只读 git 检查确认 Act 报告后 `src/wal/recovery.rs` 与 `tests/recovery_bounded_read_test.rs` **未再变化** ⇒ 覆盖范围未失效 ⇒ 本 Review **未重跑**任何 Act 已沿用的验证。
- 本 Review 新增的判定**只有代码事实类**的静态核对（符号、行号、控制流、错误语义、调用点计数、可见性），不引入新的运行结论；不需要新的运行证据的理由 = 本 Review 的每个问题都能由最终代码形态唯一确定（无时序依赖、无并发依赖、无不稳定输出）。
- 逐条核对对象：Acceptance A16 / A17 / A18 / A19、Required behavior、Preserve / Forbidden 清单、Act 的 8 项 Deviations（其中 5 项由本 Plan 指令点名裁定，另 3 项一并核对），以及 Cycle RTM 四行。

**Findings**

**F1（PASS，A16）5 处读点全部在取槽之前问页级判据；页级判据消费 Iteration 002 冻结的唯一实现；槽负载一律经 `get_slot_data_checked`；`src/wal/recovery.rs` 内 `get_slot_data` 零调用点。**

- 逐点检索 `src/wal/recovery.rs`（`get_slot_data\b|get_slot_data_checked|slot_count_exceeds_capacity|get_slot_by_logical_id`）⇒ 命中**只有** `get_slot_data_checked`（`:498` / `:615` / `:1352` / `:1696`）与 `slot_count_exceeds_capacity`（`:262` / `:485` / `:603` / `:1339` / `:1683`）；**`get_slot_data`（裸名）在该文件内零命中** ⇒ Act 的「已无任何调用点」声明**属实**。
- 页级判据的**唯一语义实现**在 `src/storage/page_format/slotted_page.rs:172`（`SlottedPageHeader::slot_count_exceeds_capacity`）；`:204`（`SlottedPageRef`）与 `:294`（`SlottedPage`）是**薄委托**（`:295` 直接 `self.header.slot_count_exceeds_capacity()`）⇒ 未另造阈值、未第二实现。`MAX_PAGE_SLOTS` 全项目**单点定义**（`slotted_page.rs:89`）。
- 5 处「取槽前问判据」的先后关系逐点确认：W5 `:262` → `:268`（`get_slot_by_logical_id`）；W1 `:485` → `:498`；W2 `:603` → `:609`（`get_slot_by_logical_id`）/ `:615`；W3 `:1339` → `:1352`；W4 `:1683` → `:1696` ⇒ **全部先判据、后取槽**。
- Act 的收口声明**经独立核对成立**：全项目 `get_slot_by_logical_id` 的其余调用点 `src/storage/data_page.rs:239` / `:314` / `:369`、`src/storage/buffer_pool.rs:433` / `:496`、`src/storage/page_format/slotted_page.rs:478` / `:508` **均在调用之前**消费了页级判据 ⇒ 本 Cycle 之后该原语**无裸调用点**（Act 只列举了 5 个，另有页格式层自身 2 个写侧调用点同样已加判据 ⇒ 结论方向一致、覆盖更全）。

**F2（PASS，A17）扫描类三点各自处置：页级 ⇒ 跳过整页 / 槽级 ⇒ 跳过该槽 + 局部计数 + stderr 报出；其余记录继续重放。**

- W1 `:485` / `:498` / `:529`、W3 `:1339` / `:1352` / `:1399`、W4 `:1683` / `:1696` / `:1732` 三点**各自**持有本扫描点的 `RecoverySkip` 收集与 `RecoverySkipCount::of` ⇒ **局部计数**按扫描点分立（不进 `RecoveryResult` 公共面）。
- 报出走 `eprintln!`（stderr）⇒ 沿用既有承载面；页级形态点名该页与页头声称的槽数，槽级形态点名该页 + **该槽序号** + 越界区间 ⇒ 满足 Cycle「报出内容点名该页与该槽序号」。
- 新增的 3 个符号 `enum RecoverySkip`（`:134`）、`struct RecoverySkipCount`（`:148`）、`fn report_recovery_skips`（`:173`）**均无 `pub`**；`git diff -- src/wal/recovery.rs` 中**无任何新增 `pub` 项** ⇒ **未新增**公共 API 字段 / 命令参数 / 配置项 / 缓存 / 闸门机制 ✓。
- `report_recovery_skips` 在**页锁闭包之外**调用（跳过记录在闭包内收集、闭包外打印）⇒ 锁内无 IO ✓（代码事实核对）。

**F3（PASS，A18）重放类两点不跳过该记录 ⇒ 以明确错误结束恢复与打开；两类处置分立。**

- W2 `:603` / `:615` ⇒ `StorageError::CorruptedDataPage`（页级 / 槽级两形态）⇒ `WalError::RedoFailed` ⇒ 恢复与打开以**明确错误**结束（不 panic、不 abort）；W5 `:262` ⇒ 同一变体，点名该页与页头声称的槽数 ⇒ **不跳过该记录** ✓。
- **两类处置在代码上分立**：W1 / W3 / W4 的损坏槽路径**只**做「跳过 + 报出」（无 `Err` 传播）；W2 / W5 的损坏槽路径**只**做「明确错误」（无跳过分支）⇒ 不存在「既跳过又报错」或「既报错又跳过」的混合形态 ✓。
- 既有重放语义（重放顺序、幂等探针语义、`RedoContext` 结构、`will_redo` 门控）**零改动**（diff 的 10 个 hunk 全部落在 5 处读点 + 3 个新增私有符号 + import）✓。

**F4（PASS，A19）未损坏路径行为零变化；四处既有的「过短负载」逐点语义零改动；Forbidden 文件本 Cycle 零 diff。**

- 四处「过短负载」分支现状：`src/wal/recovery.rs:507`（W1 `continue`）、`:622`（W2 `StorageError::Internal("derived slot … shorter than version header")`）、`:1361`（W3 `continue`）、`:1705`（W4 `continue`）⇒ **逐字未改**（未被改成报错）✓。
- `src/storage/page_format/**`、`src/storage/btree/**`、`src/storage/integrity.rs`、`src/storage/data_page.rs`、`src/storage/buffer_pool.rs`、`src/executor/**`、`src/transaction/**`、`src/pipeline.rs`、`src/storage/error.rs` ⇒ 本 Cycle **零 diff**（逐文件检索其工作区 diff 中的 `MS37 T5` 标记 ⇒ 全部 0 命中；这些文件的既有改动属 Iteration 000–004 的现场，非本 Cycle 引入）✓。
- 磁盘格式、dump / restore 合同面、`rtsql check` 的诊断分级**未改**；`write-failure-state` 域（ISS30 / ISS31）语义**未改**（本 Cycle 不碰写路径与 close / checkpoint 路径）✓。

**F5（Plan 错误 —— 非阻塞）Cycle 的「W2 越界槽分支不可达」判定被代码事实推翻 ⇒ 归类为 **Plan 错误**（Cycle 的可达性分析漏了 Insert 臂对同一份候选集的再次写入），**不是 Act 偏离**。**

- 代码事实：`redo_record` 的 **Insert 臂在 deindexed 模式下**把 WAL 记录的 `row_id` **再次** `insert_row_id_sorted` 进**同一份** `ctx.pk_versions` 映射 ⇒ 被 W1 跳过的槽的 rid 经**重放本身**重新进入候选集 ⇒ W2 的越界槽分支**可达**。
- 该分支**正是用例 C 的实际错误路径**（Act 实测文案 `update redo: table 't' derived slot RowId { page_id: 2, slot_id: 1 } read failed: corrupted data page 2: slot 1 range [65535, 65570) lies outside the page data area` ⇒ 该错误由 W2 自身产生，而非 Cycle 预测的「old key has no version before row …」）。
- ⇒ Cycle 中「W2 的分支不是测试断言目标」「测试断言的是『打开以明确错误结束、进程不 abort』」这一指示**作废**；该分支被用例 C **直接覆盖** ⇒ **覆盖面增加**，且 Cycle 的 GREEN 条件（打开以明确错误结束、进程不 abort、文案含 `update redo`）**逐字满足** ⇒ 非实质偏差，不需 Act 返工。
- 偏差归类表：**Plan 遗漏**（Cycle 的可达性分析不完整）—— 不是 Act 偏离、不是 Plan 错误的设计方向、不是现状变化、不是新证据。归为「Plan 调查事实的偏差」而非「Plan 错误」中的**方向性**错误。
- ⇒ 以**追加标注**方式更正该失效指示（见本文件末尾的「追加标注」节）；**冻结的 Plan Context 正文未改写**。

**F6（非阻塞 —— 测试顺序事实）Act 的 Deviations 第 2 项（新增用例 D 承载 W4）的裁定：**

- **Cycle 层面**：**满足「没有先行测试不变更」**。Cycle 的行为变更（恢复面按恢复类别处置损坏槽）在变更前已有先行测试，且观察到**真实 RED**：用例 A / B / C 逐例单独运行 ⇒ 均为**行为失败**（进程 SIGABRT，非编译失败）；未损坏对照变更前即 GREEN。依据条款：公共规则 › TDD「先有测试，再有变更（NO CHANGE WITHOUT A PRIOR TEST）」与「Bug：测试复现问题，观察 RED，再修复 GREEN」；公共规则 › 验证「修改前的先行测试按同一规则沿用既有结论」——本例中既有结论即变更前的合法观察。
- **W4 单点层面**：**不满足「先行测试」的字面顺序**。W4 的产品改动先于任何进入该点的测试落地；Act 实际做的是「写用例 D ⇒ **临时回退 W4 单点改动** ⇒ 观察真实 RED（CLI 子进程 SIGABRT）⇒ 原样恢复 W4 ⇒ GREEN」。这是**鉴别证明**（mutation 式回退验证），**不是**先行测试。
- **是否构成阻塞**：**不构成**。理由：(a) W4 的改动与 W1 / W3 的改动**形态相同**（页级判据前置 + `get_slot_data_checked` + 跳过该槽 + 局部计数 + stderr 报出），而该形态已被**变更前**的用例 A 覆盖并观察到真实 RED ⇒ W4 实现的**行为合同**在变更前已有先行测试；(b) 回退验证给出了该测试**确实能判别**该点的证据（无该改动 ⇒ 真实 abort）；(c) 该点无「无先行测试且无 RED」的空洞状态。
- **裁定**：**非阻塞偏差**（单点层面的测试书写顺序偏差；非「缺少测试」、非「缺少 RED」）。
- **是否需要在 change 产物中补记该点的测试顺序事实**：**需要**，且已补记 —— 本 F6 即为该事实的权威记录点（Cycle 的 Plan Context 与 Act Response 均已冻结，不改写）。台账「处置指针」归 Recorder 所有，本 Review **不登记 Issue、不改台账**。

**F7（PASS —— Deviation 3）「报出」的实现比 Cycle 描述多一行 ⇒ 满足 Cycle 的 Required behavior。**

- Cycle 要求两条：(i)「计数用**局部计数**（每个扫描点各自维护）」；(ii)「报出内容点名**该页**（页级形态）与**该页的槽序号**（槽级形态）」。
- Act 的实现 = 一行局部计数汇总（`skipped N damaged slot(s)` / `skipped N damaged page(s)`）+ 逐条细节行 ⇒ (i) 满足（计数由 `RecoverySkipCount::of` 在**各扫描点内**汇总，F2 已逐点确认）；(ii) 满足。
- ⇒ 计数因此**可观察**（而不只是内部变量）—— 这是实现选择的**后果**，**不是**对 Required behavior 的违反；两行都走 stderr，**未新增**公共 API 字段 / 命令参数 / 配置项 / 缓存 / 闸门机制 ⇒ **PASS**。

**F8（非阻塞 —— Deviation 4）Cycle 的 Target 行号按符号名重新定位。**

- W1 `:374` → `:485` / `:498`；W2 `:459` / `:462` → `:603` / `:615`；W3 `:1177` → `:1339` / `:1352`；W4 `:1490` → `:1683` / `:1696`；W5 `:166` → `:262` ⇒ 符号在 `src/wal/recovery.rs` 内**均唯一**，定位无歧义 ⇒ 行号漂移**不是失败**（本 change 已有多个 Cycle 因前序改动而偏移的先例）⇒ **PASS**。

**F9（非阻塞 —— Deviation 5）页级形态的 RED panic 文本与 Cycle 预测不同。**

- Cycle 预测「槽目录定位下溢 ⇒ `range start index …` 或 `attempt to subtract with overflow`」；实测 `range start index 25088 out of range for slice of length 4096` ⇒ 页头 `slot_count` = 700 时，**垃圾槽目录项**（索引 3..681 读到数据区残留字节）的 `offset` 字段**先**触发**负载**越界，早于槽目录定位下溢。
- ⇒ 变更前仍为**进程 abort**（非「打开失败」、非「跳过」）⇒ RED 形态成立；Cycle 的 GREEN 判据是「打开以明确错误结束 + 点名该页与页头声称的槽数」，**不依赖 panic 文本** ⇒ **非失败**。⇒ Cycle 的预测表述不精确（不是缺陷）。

**F10（PASS —— Deviations 6 / 7 / 8）三项执行形态调整与 Cycle 自身前提一致。**

- **Deviation 6**（夹具用 `buffer_pool.flush_all()` 而非 `checkpoint()`）：`checkpoint()` 会**截断 WAL** ⇒ 下次打开不触发重放 ⇒ 与 Cycle 夹具的「崩溃模拟 ⇒ 触发全量重放」前提**直接冲突**；`flush_all()` 只落脏页、不截断 WAL ⇒ 与前提一致。`flush_all`（`src/storage/buffer_pool.rs:217`）与 `wal_buffer` / `buffer_pool` 公共字段均为**既有** API（本 Cycle 未新增公共面，见 F2）⇒ **PASS**。
- **Deviation 7**（CLI 面用例选用 `rtsql checkpoint <db>` 读写模式而非 `rtsql check <db>`）：Cycle 的 Required behavior 明确要求「断言『报出』的用例必须走**读写模式**的真二进制子进程」（只读打开不触发恢复）⇒ 该选择正是 Cycle 要求的落地方式，**未新增命令参数** ⇒ **PASS**。
- **Deviation 8**（定向回归命令与 Cycle 清单逐字一致；批内 `--test-threads=1`、批间串行、批 1 全绿后才启动批 2）⇒ 与 M19 分批串行纪律一致 ⇒ **PASS**。

**Acceptance Gaps**

**无。** A16 / A17 / A18 / A19 四条验收条件逐条满足（见 F1 / F2 / F3 / F4）；8 项 Deviations 全部为非实质偏差且不改变 Acceptance（F5 归类为 Plan 调查事实的偏差、覆盖面增加；F6 为非阻塞的测试顺序偏差；F7–F10 为形态与表述层面）；无需要 Act 返工的缺口。

**收敛判断**

- 本 Cycle 的 Acceptance 已全部满足；实现与 Plan Context 的方向、Required behavior、Preserve / Forbidden、验证边界**逐条一致**；无未解决的实质未知项。
- 未引入新方向（「恢复面按恢复类别处置」的方向未变；F5 是对既有方向的**可达性事实**更正，覆盖面更严）⇒ 不触发拆分信号（tasks.md 平衡审计的拆分信号条件不成立）。
- 当前 Iteration 的 Review Result = `accepted` ⇒ Iteration 005 **完成**，可展开 Iteration Map 的下一 Iteration（006 / DDL 提交点统一模型）。
- 本 Cycle **未**产生需要 Act 返工的问题 ⇒ **不创建 rework Cycle**；同一 Acceptance gap 不存在连续两次未缩小的情形 ⇒ 不触发 replan。

**Persisted Evidence**

**None required** —— Plan 的 `Persisted Evidence` 为 `none`；全部结论（RED 形态、GREEN 形态、未损坏对照、定向回归）由低成本可重跑的 `cargo test --test recovery_bounded_read_test -- --test-threads=1` + `cargo test --lib` + 20 个定向回归目标承载，Act Response 的 Verification Evidence 段已完整承载命令与实际结果 ⇒ 未创建 `evidence/` 目录，不预建占位目录。

**Issue 候选**

**无。** 逐项核对：

(a) 本 change 范围内 —— 无未解决的实质缺陷。Act 遗留的 2 项 Minor 均非缺陷：`RecoverySkipCount` 的两个计数字段在单次页扫描内恒为同类（形态区分用途，**非缺陷**）；「恢复成功（扫描类跳过）与读路径严格报错在同一库上并存」是**两个面各自既定裁定**的组合（恢复面按 D7 跳过并报出 / 读路径面按 Iteration 001 点名报错），**非缺陷**、且不在本 Cycle 可改范围。
(b) 本 change 范围外 —— 本次 Review 未发现新的范围外实质缺陷。Act 记录的「W2 的页级分支在本恢复流下不可达（W1 先跳页、W5 先失败）」属**当前不可达**的分支（已按 Requirement 实现），归类为**非阻塞 Minor finding**，按公共规则 › Scope Control「候选工作无法关联到可达代码和数据时，不实施」⇒ **不实施**、**不登记为 Issue**、不新立 Iteration。
(c) F6 的测试顺序偏差属**过程**偏差，非产品缺陷 ⇒ 不登记为 Issue。

⇒ **只报告、不登记**；ISS27 台账的「处置指针」归 Recorder 回填，本 Review **不改台账**。

**未解决问题**

- **无阻塞项。**
- F6 记录的单点测试顺序事实（用例 D 写在 W4 改动之后，RED 由回退取得）已在本 Review 留痕；本 Review **未改写**冻结的 Plan Context 与 Act Response。
- 本 Review **未修改产品代码、未同步全局文档（`.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆 / references 均未动）、未改 ISS27 台账、未执行 `git commit`、未归档 change**；只更新了本 change 内的 `tasks.md` 与 `specs/ddl-layout-publication/spec.md`（delta 增量），并按公共规则展开 Iteration 006。

**下一 Iteration**

- `iterations/006-ddl-commit-point-unified-model/000-initial.md`（T6 / T7 / T8；DDL 提交点统一模型）。

---

## 追加标注（来自 Iteration 005 / Cycle 000 的 Plan Review，2026-10-26）

> 本节是**追加标注**，不改动上方已冻结的 `Plan Context` 正文（Plan Context 不可改写），也不改写 `Act Response`。它更正 Cycle 中**一条已失效的指示**，并指向 F6 的测试顺序事实记录。

**来源**：Iteration 005 / Cycle 000 的 `Plan Review` F5 / F6（2026-10-26，Review Result = `accepted`）。

**失效的指示（Cycle 原文，位于 Plan Context 的「Risks and Notes」）**：

> 「**W2 自身损坏槽分支不可达**：W1 先建立版本映射且只收录未被跳过的槽 ⇒ W2 的候选集不含损坏槽 ⇒ W2 的越界槽分支不可达。……但**测试不断言** W2 的损坏槽分支的错误文案只含页号 ⇒ 测试断言的是『打开以明确错误结束、进程不 abort』。」

**更正（追加标注，不改写原文）**：

- 该可达性判定**不成立** ⇒ 上述指示中「W2 的越界槽分支不可达」与「测试不断言 W2 的损坏槽分支」这两点**作废**。
- **代码事实**：`redo_record` 的 **Insert 臂在 deindexed 模式下**把 WAL 记录的 `row_id` **再次** `insert_row_id_sorted` 进**同一份** `ctx.pk_versions` 映射 ⇒ 被 W1 跳过的槽的 rid 经**重放本身**重新进入候选集 ⇒ W2 的越界槽分支**可达**。
- **实际覆盖**：`tests/recovery_bounded_read_test.rs` 的**用例 C（重放类 · 槽级形态）**直接命中该分支（实测错误文案含 `update redo: … corrupted data page 2: slot 1 range [65535, 65570) lies outside the page data area`）⇒ 该分支**已被直接覆盖**，覆盖面**增加**。
- **未被作废的部分**：Cycle 的 **GREEN 条件**（打开以明确错误结束、进程不 abort、用例 C 文案含 `update redo`）与「测试断言落在行为上（打开结果 / 进程存活 / stderr 文案）」的要求**继续有效**，且**逐字满足**。
- **归类**：**Plan 调查事实的偏差**（Cycle 的可达性分析漏了 Insert 臂的再次写入）⇒ **不是** Act 偏离；不触发 rework / replan。
- **与 F6 的关系**：本标注只更正可达性指示；**用例 D / W4 的测试顺序事实**记录在 `Plan Review` 的 **F6**（不重写 Act Response）。

**后续处置**：本 Cycle 已 `accepted`；上述作废指示**仅影响本 Cycle 的历史解释**，不产生新的 work item ⇒ 本 change 的 T5 范围内**无遗留项**。ISS27 台账条目的关闭与「处置指针」回填归 Recorder 按用户指令执行。
