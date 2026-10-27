# Iteration 004 / Cycle 000: 目录页组的有界读（目录页损坏 ⇒ 该库 SHALL NOT 打开）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4
- Acceptance gaps: None
- Repair items: None
- Inherited scope:
  - delta spec `page-read-bounded-read` 的 Requirement 首段与 Scenario「目录页上的损坏槽给出明确的错误」—— **已获批**，本 Cycle 执行它；
  - `design.md` D7 的目录页行（打开 / 查询路径明确错误；释放 / 重建路径沿用既有尽力而为与「重建放弃」语义）；
  - Iteration 001 / Cycle 000 的裁定（**目录页损坏 ⇒ 该库 SHALL NOT 打开**，理由：目录是「哪些页仍然存活」的根依据；跳过该目录行会让该表静默消失，比打不开更糟；打不开同时严格优于现状（今天是 panic / abort））与其可见后果（该库的整个使用面——打开 / 完整性扫描 / 导出 / 模式查看——报出打开错误，而不是给出基于残缺目录的结论）；
  - T3 已交付并**冻结**的产品原语：`SlottedPageRef::get_slot_data_checked`（`src/storage/page_format/slotted_page.rs:137-144`）与 `StorageError::CorruptedDataPage` —— 本 Cycle **复用**，**不得重复实现**；
  - Iteration 002（T17 / ISS35）**冻结**的页级语义：页格式层提供**唯一的**槽目录容量界限与**一个页级判据**（页头声称的槽数是否超出该页的槽目录容量），只读变体 `SlottedPageRef` 可直接问出；本 Cycle 按该**语义**调用（API 名称以 Iteration 002 的实际实现为准）。
- Excluded scope:
  - 恢复面组（`src/wal/recovery.rs` 4 处）—— 归 **T5**（Iteration 005）；
  - 数据页组（Iteration 001 / Cycle 000 已交付并被接受）与页头槽数越界形态（Iteration 002 承载）；
  - 索引树页（`src/storage/btree/**`）、数据页的页级判据 `check_data_page` / `check_tree_page` / `validate_slots`、物理释放 / 重建路径（它们消费的是页级判据，不是本 Cycle 的 5 个目录读点）；
  - 损坏目录的**修复 / 抢救**、`rtsql check` 的诊断分级、dump / restore 合同面、磁盘格式；
  - 全量回归（整个 change 的全量预算在 Iteration 009 / T16）。

**Objective**

目录页（`__tables` / `__columns` 链）上的越界槽在打开 / 读取路径上不再造成越界切片 panic 与进程 abort：读取经由有边界检查的路径取得；越界槽与页头槽数越界都让读取以**点名该页与该槽**的明确错误结束，**不得**被当作「没有这张表」而静默跳过；因目录损坏而打不开该库时，该后果在使用者面（完整性扫描 / 导出 / 模式查看）**可见**。未损坏路径的行为**零变化**。

**Background**

- 缺陷来源：ISS27 台账（`.agents/issues/ISS27-*.md`）的**目录页组**（`design.md` D7 已把 ISS27 拆成按页型分组的判定方向），以及 Iteration 002 的页级判据（ISS35 台账）。
- 本 Cycle 的方向**已定**（Iteration 001 / Cycle 000 的 Plan Context「拆分信号裁决」中的 T4 条）：不触发拆分信号；需要的是既有方向的确认 + 一处措辞细化，不是新方向 ⇒ 原记为「留在 Iteration 001」，承载面已修正为独立 **Iteration 004**（见 `design.md` D14 与 D15；D15 因写路径组插入而再顺延一位）；**方向裁定不变**。
- 本 Cycle 为何排在 Iteration 002 与 Iteration 003 之后：Iteration 002 冻结「页级槽数判据」的形状；Iteration 003（写路径组，T18 / T19）与本 Cycle **同改 `src/storage/catalog.rs` 的写入点**（`:707` 目录页链追加、`:924` 目录行原地改写）⇒ 写路径组必须先行落地，避免两个 Iteration 改同一批目录写入点；本 Cycle 的目录读点同时消费**槽负载有界读**（T3 冻结）与**页级槽数判据**（Iteration 002 冻结）⇒ 两者都写在最终形状上，不返工。

**Investigation Facts**

- Starting point:
  - 工作区 HEAD `e9cb9d3`；前置状态 = Iteration 001 / Cycle 000（T3）**已被 Plan Review 接受**；本 Cycle 的**直接前置** = Iteration 003 / Cycle 000（T18 / T19，写路径组）—— 它与本 Cycle **同改 `src/storage/catalog.rs` 的写入点**（`:707` / `:924`）⇒ 写路径组未落地前本 Cycle 不得执行；**间接前置** = Iteration 002 / Cycle 000（T17 / ISS35）—— 其交付的页级槽数判据是本 Cycle 的前置（**Iteration 002 未被接受前本 Cycle 不得执行**）。
  - 变更前的验证结果（沿用，未失效）：Iteration 001 / Cycle 000 的 Act Response 记录 GREEN `cargo test --test data_page_bounded_read_test` 6 passed / 0 failed、`cargo test --lib` 420 passed / 0 failed、定向回归 20 目标 2 批串行 354 passed / 0 failed（来源：Iteration 001 / Cycle 000 的 Act Response「Verification Evidence」）。本 Cycle 改动 `src/storage/catalog.rs` / `src/storage/error.rs` ⇒ **覆盖到这两个文件的既有结论失效**（须重跑）；覆盖到 `src/storage/page_format/slotted_page.rs` / `src/storage/btree/**` 的结论（Iteration 002 的产物）由 Iteration 002 自己负责，本 Cycle 不重复。
  - 已知限制：目录页组 5 处读点仍用**无范围检查**的 `SlottedPageRef::get_slot_data`（`src/storage/page_format/slotted_page.rs:125-129`）⇒ 槽偏移 / 长度越界 ⇒ `&self.data[start..end]` 越界 panic ⇒ 进程 abort。
- Current-State Evidence（本 Cycle 已核对的事实，供 Act 直接使用；行号为**当前工作区**）：
  - **目录页的页号**：`TABLES_PAGE_ID = 0`、`COLUMNS_PAGE_ID = 1`（`src/storage/catalog.rs:38/40`）⇒ `__tables` 目录页 = 文件的第 **0** 页，`__columns` 目录页 = 第 **1** 页；两者都是**槽式页**（slotted page）。
  - **文件布局**：页 N 的文件偏移 = `HEADER_SIZE(64) + N * PAGE_SIZE(4096)`（`src/storage/file_header.rs:20`、`src/storage/page.rs:11`）；槽目录项 i 的页内起点 = `PAGE_SIZE - (i+1) * Slot::SIZE(6)`，其中 **offset 字段在项起点 +2** ⇒ 槽 0 的 offset 字段在页内偏移 `4096 - 6 + 2 = 4092` ⇒ **文件偏移 = 64 + page_id * 4096 + 4092**（页 0 ⇒ 4156；页 1 ⇒ 8252）。
  - **5 个目录读点（`grep -n "get_slot_data" src/storage/catalog.rs` 实测）**：

    | # | 位置 | 所在函数 / 路径类 | 既有取槽形态 | 既有「短负载」语义 |
    |---|---|---|---|---|
    | 1 | `src/storage/catalog.rs:764-765` | `scan_chain` —— **打开路径**（`scan_tables` ← `TableManager::open_or_init`）与**查询路径**（`scan_columns` ← `open_or_init` / `get_columns`） | `if let Some(slot) = slotted.get_slot(i)` + `get_slot_data(&slot).to_vec()` | `deserialize_catalog_row` 返回 `Err(StorageError::Internal("catalog row too short …"))` ⇒ 打开失败（**既有语义，保持**） |
    | 2 | `src/storage/catalog.rs:808-809` | `delete_from_chain` —— DROP TABLE / ALTER TABLE DROP COLUMN 的**目录行删除**（DDL 写路径） | `if let Some(slot) = slotted.get_slot(i)` + `get_slot_data(&slot)` | 同上（`deserialize_catalog_row` 的既有错误） |
    | 3 | `src/storage/catalog.rs:890-893` | `update_field_in_chain` —— DDL 窄化发布的**目录行改写**（写路径） | `let Some(slot) = slotted.get_slot(i) else { continue; }` + `get_slot_data(&slot)` | 同上 |
    | 4 | `src/storage/catalog.rs:965-974` | `delete_named_copies_except` —— 上一处的**同链清理**（写路径） | `let Some(slot) = slotted.get_slot(i) else { continue; }` + `matches_catalog_row_name(slotted.get_slot_data(&slot), target_name)` | 同上 |
    | 5 | `src/storage/catalog.rs:1649-1652` | `rewrite_column_rows_drop` —— ALTER TABLE DROP COLUMN 的**列目录改写**（写路径） | `let Some(slot) = slotted.get_slot(i) else { continue; }` + `get_slot_data(&slot)` | 同上 |

  - **5 处的调用者全部以 `?` 传播错误**（无「尽力而为 / 重建放弃」的容差点）：`TableManager::open_or_init`（`src/storage/data/table_manager.rs:373-374`：`self.catalog.scan_tables().await?`）、`Catalog::scan_columns`（`src/storage/catalog.rs:314`，由 `open_or_init:395` 与 `get_columns` 调用）、`Catalog::delete_table` / `update_table_column_count_ddl` / `rewrite_column_rows_drop` 的调用者 `TableManager::apply_column_drop`（`src/storage/data/table_manager.rs:1219-1225`，逐个 `?`）⇒ ⇒ **D7 的「释放 / 重建路径沿用既有尽力而为」条款在本组不触发**（物理释放 / 重建路径消费的是 `check_data_page` / `check_tree_page` 的页级判据，不消费这 5 个读点）⇒ **5 处统一为明确错误**。
  - **打开路径的传播链（决定「该库 SHALL NOT 打开」）**：`Database::open`（`src/database.rs:199`）→ `TableManager::open_or_init`（`src/storage/data/table_manager.rs:373`）→ `Catalog::scan_tables`（`src/storage/catalog.rs:304`）→ `scan_chain`（`:747-782`）⇒ 任一目录读点的错误以 `?` 一路上抛 ⇒ **打开失败**。
  - **使用者面的可见性**：`src/cli/mod.rs:668-672` 的打开失败分支调用 `open_error_status(db_path, e)`（`src/cli/mod.rs:586-601`）⇒ `ExitStatus::General(format!("failed to open database {}: {}", db_path.display(), other))` ⇒ **非零退出码 + 错误文案**（`rtsql check` / `dump` / `.schema` 等命令都先开库 ⇒ 必然报出该打开错误）。⇒ 该可见性**结构上已具备**，本 Cycle 只须让目录读点产生**命名**错误。
  - **既有错误变体**：`src/storage/error.rs` 的 `StorageError::CorruptedIndexPage`（迭代器侧）与 T3 新增的 `CorruptedDataPage { page_id: u64, detail: String }`（`#[error("corrupted data page {page_id}: {detail}")]`）⇒ 目录页是**槽式页但不是数据页**（页号 0 / 1 是 `__tables` / `__columns`），复用 `CorruptedDataPage` 会让使用者看到「corrupted data page 0」这种与实际页型不符的文案 ⇒ 本 Cycle **新增一个变体** `CorruptedCatalogPage { page_id: u64, detail: String }`（`#[error("corrupted catalog page {page_id}: {detail}")]`），与 `CorruptedDataPage` 同形。
  - **判据来源**：槽负载越界判据用 T3 冻结的 `SlottedPageRef::get_slot_data_checked`（其内部判界与索引树既有 `slot_payload_in_bounds` 同形）；页级槽数判据用 Iteration 002 冻结的页级判据（语义：页头声称的槽数是否超出该页的槽目录容量）。
  - **既有测试入口**：`tests/catalog_durability_test.rs`（目录持久化）、`tests/ddl_catalog_transactional_test.rs`、`tests/ddl_rewrite_consistency_test.rs`、`tests/check_command_test.rs`（`rtsql check` 的真二进制用例，用 `CARGO_BIN_EXE_rtsql` 启动真进程，`tests/cli_test.rs` 同形）、`tests/cli_test.rs`。
- Code and Critical Path:
  - 入口：`Database::open` → `TableManager::open_or_init` → `Catalog::scan_tables` / `scan_columns` → `scan_chain`（目录链遍历，`next_page_id` 串联）；DDL 路径：`TableManager::apply_column_drop` → `Catalog::rewrite_column_rows_drop` → `delete_from_chain` / `update_field_in_chain` → `delete_named_copies_except`。
  - 页格式层：`SlottedPageRef::new(data)` → `get_slot(i)` / `get_slot_data(&slot)`；本 Cycle 只把**取负载**换成 `get_slot_data_checked`，并在**取槽之前**加页级槽数判据。
  - 错误面：`src/storage/error.rs` 的 `StorageError`；`src/storage/mod.rs` 的重导出。
  - 状态与并发：目录读写都在 `Catalog::lock`（`self.lock.lock().await`）之下 ⇒ 本 Cycle 不改变锁语义；判据是纯读（读页头的一个 u16），判定与取槽之间无 await ⇒ 无竞态窗口。

**Implementation Guidance**

- 建议顺序：① `src/storage/error.rs` 新增 `CorruptedCatalogPage` 变体 → ② `src/storage/catalog.rs` 加错误构造器（点名页 + 槽 + 区间；**不臆造表名**）→ ③ 5 处读点：取槽**之前**加页级槽数判据（超容量 ⇒ 明确错误），取负载换成 `get_slot_data_checked`（越界 ⇒ 明确错误）→ ④ 测试：先行用例（RED → GREEN）。
- 关键取舍：
  - **5 处统一为明确错误**：D7 的「释放 / 重建路径沿用既有尽力而为」在本组**不触发**（这 5 个读点的调用者全部以 `?` 传播，且物理释放 / 重建路径消费的是页级判据而非这些读点）⇒ 与 Iteration 001 / Cycle 000 的裁定（目录是活页事实的根；跳过该目录行等于静默删表）一致 ⇒ **不得**在任何一处把越界槽改成「跳过该目录行」。
  - **错误文案不臆造表名**：越界槽的负载**读不出来** ⇒ 只能点名**该页与该槽**（与 delta spec 的措辞一致）⇒ 不得尝试解析或猜测表名。
  - **`get_slot` 的 `None` 语义不改**：5 处读点里的 `else { continue; }` / `if let Some(slot)` 分支在「按 `slot_count` 遍历」下**不可达**（`i < slot_count` ⇒ `get_slot(i)` 恒为 `Some`）⇒ **保持原样**，不改为错误，避免改变既有控制流。
  - **页级判据的接入点**：判据必须在**取槽之前**问（取槽即可能 abort）⇒ 放在「拿到页缓冲、构造 `SlottedPageRef` 之后、第一次取槽之前」。
  - **「短负载」不动**：delta spec 的判定口径明确「负载短于该页型既定的最小负载」**不属于**越界槽，其既有逐点语义 SHALL 保持 ⇒ 目录侧保持 `deserialize_catalog_row` 的既有 `StorageError::Internal("catalog row too short …")`（打开失败）不变。

**Task Definitions**

### T4: 目录页上的越界槽给出明确的错误，目录损坏 ⇒ 该库 SHALL NOT 打开

- Requirement/Scenario:
  - `page-read-bounded-read` Requirement 首段（有界读、不产生越界切片、不 abort）；
  - `page-read-bounded-read` Scenario「目录页上的损坏槽给出明确的错误」；
  - `page-read-bounded-read` Scenario「未损坏路径的行为零变化」；
  - `page-read-bounded-read` Scenario「页头声称的槽数超出槽目录容量时读路径不 abort」（Iteration 002 新增，目录页部分）。
- Depends on: Iteration 002 / Cycle 000（T17 / ISS35）**与** Iteration 003 / Cycle 000（T18 / T19，写路径组）—— 页级槽数判据由 Iteration 002 交付并冻结；`src/storage/catalog.rs` 的写入点由 Iteration 003 先行落地（**这两个前置未被接受前本 Cycle 不得执行**）。
- Targets:
  - `src/storage/error.rs` —— 新增 `StorageError::CorruptedCatalogPage { page_id: u64, detail: String }`。
  - `src/storage/catalog.rs` —— 错误构造器（点名页 + 槽 + 区间）；5 个读点：`scan_chain`（`:764-765`）、`delete_from_chain`（`:808-809`）、`update_field_in_chain`（`:890-893`）、`delete_named_copies_except`（`:965-974`）、`rewrite_column_rows_drop`（`:1649-1652`）。
  - `tests/catalog_bounded_read_test.rs` —— 新增测试文件（先行用例 + 对照 + CLI 面用例）。
- Current behavior: 目录页上的越界槽在 5 个读点处经由**无范围检查**的 `SlottedPageRef::get_slot_data` 取得负载 ⇒ `&self.data[start..end]` 越界 panic ⇒ **进程 abort**；页头槽数越界时，取槽的槽目录定位下溢 ⇒ 同样 abort。⇒ 目录损坏 today 既不能打开也不能给出任何诊断。
- Required behavior:
  - 5 个读点的**槽负载**一律经由 `get_slot_data_checked` 取得；越界槽**不产生任何越界切片**、**不 panic**；
  - 5 个读点在**取槽之前**问页级槽数判据；页头声称的槽数超出槽目录容量 ⇒ 明确错误；
  - 两类损坏都让读取以 `StorageError::CorruptedCatalogPage { page_id, detail }` 结束，`detail` 点名**该页与该槽**（越界时**不得**臆造表名）；
  - 该错误**不得**被改写成「该表不存在 / 目录为空」一类语义（跳过该目录行会让该表静默消失）；
  - 打开路径（`scan_tables` / `scan_columns`）上的该错误 ⇒ **打开以该错误结束** ⇒ **该库 SHALL NOT 打开**；
  - 该后果在使用者面可见：`rtsql check` / `dump` / `.schema` 等命令 SHALL 报出该打开错误（而非基于残缺目录的结论）；
  - 未损坏路径行为**零变化**（含目录行「过短负载」的既有 `StorageError::Internal` 语义）。
- Preserve:
  - `SlottedPageRef::get_slot_data_checked` / `slot_payload_in_bounds` 的判据与语义（T3 已接受）；
  - `SlottedPageRef::get_slot` / `get_slot_by_logical_id` 的签名与 `None` 语义（**不改**）；
  - `SlottedPageRef::get_slot_data` 的签名与既有单元测试；
  - 目录行「过短负载」的既有语义（`deserialize_catalog_row` 的 `StorageError::Internal`）与目录链遍历顺序、`next_page_id` 链式结构；
  - `Catalog::lock` 的锁语义、DDL 窄化发布的既有提交点顺序（`TableManager::apply_column_drop` 的「列行先、计数后」顺序）；
  - `rtsql check` 的既有诊断分级与输出形态、dump / restore 合同面；
  - 物理释放 / 重建路径的既有语义（它们消费页级判据，不在本 Cycle）。
- Forbidden:
  - 改动 `src/wal/**`（恢复面组 → T5）、`src/storage/page_format/slotted_page.rs`（Iteration 002 / 003 承载）、`src/storage/btree/**`、`src/storage/integrity.rs`、`src/storage/data_page.rs`、`src/storage/buffer_pool.rs`、`src/executor/**`、`src/transaction/**`、`src/pipeline.rs`；
  - 把越界槽改成「跳过该目录行」或任何形式的静默容错；
  - 在错误文案里臆造 / 猜测表名；
  - 改动目录行「过短负载」的既有语义；
  - 新增命令参数 / 配置项 / 缓存 / 闸门机制 / 判定层 / 身份证明机制；改动磁盘格式、dump / restore 合同面、`rtsql check` 的诊断分级；
  - 全量回归。
- First test:
  - 位置：`tests/catalog_bounded_read_test.rs`（**新增测试文件**；沿用 `tests/data_page_bounded_read_test.rs` 的夹具形态：真实文件库 + 真实 `Database::open` + 真实 SQL / CLI 入口 + **真实文件字节改写**，无替身 / 桩）。
  - **RED（变更前）**：夹具建库 → 建表 `t`（含 ≥ 1 列）→ 插入 ≥ 1 行 → 关闭库 → 按文件偏移 `HEADER_SIZE(64) + page_id * PAGE_SIZE(4096) + 4092`（页 0 ⇒ 4156；页 1 ⇒ 8252）把**目录页槽 0 的 offset 字段**写成 `0xFFFF`（u16 小端）⇒ 重开库 ⇒ 执行 `Database::open`（以及一次 `SELECT … FROM t`）。
    ⇒ **预期 RED：进程 panic / abort**（`range start index 65535 out of range for slice of length 4096`，位置 `src/storage/page_format/slotted_page.rs:128`）—— 即打开既没有返回命名错误，也没有给出任何诊断。
  - 夹具的**未损坏对照**：同样步骤但不改字节 ⇒ 变更前即 GREEN（库能打开、表与列可见、DML 结果与既有行为一致）。
  - **CLI 面用例**（同一夹具）：`rtsql check <db>`（真二进制，`CARGO_BIN_EXE_rtsql`）⇒ **预期 RED：进程 abort（非零退出但为信号终止）**；GREEN 后 ⇒ 退出码非零且输出含 `failed to open database` 与 `corrupted catalog page 0`。
- GREEN condition: 上述 RED 用例变为「打开（或命令）返回 / 报出**点名该页与该槽**的明确错误（文案含 `corrupted catalog page {page_id}` 且点名槽序号与区间），**进程不 abort**」；`Database::open` 返回 `Err`（⇒ 该库打不开）；CLI 用例退出码为**普通非零**（非信号终止）且输出含该错误；未损坏对照结果不变；`cargo test --test catalog_bounded_read_test`、`cargo test --lib` 与本 Cycle 的定向回归清单全绿；`cargo fmt --check` / `cargo clippy --all-targets` 在**本 Cycle 触碰的文件**上零新增 finding。
- Verification:
  - 行为类（真实入口）：`cargo test --test catalog_bounded_read_test -- --test-threads=1` —— 覆盖「`__tables` 页损坏 ⇒ 打开失败」「`__columns` 页损坏 ⇒ 打开失败」「CLI 面报出打开错误」「未损坏零变化」；损坏注入必须是**真实文件字节改写**，观察对象是打开 / 命令返回的错误与进程是否存活。
  - 单元与库面：`cargo test --lib` —— 覆盖 `page_format` 既有单元测试与目录 / 事务 / 执行器的库内单测。
  - 定向回归（18 个目标，**M19 分批串行纪律**：批内 `--test-threads=1`、批间串行、批 1 全绿后才启动批 2）：
    - 批 1（9 个）：`cargo test --test storage_test --test table_manager_test --test alter_table_test --test ddl_catalog_transactional_test --test ddl_rewrite_consistency_test --test catalog_durability_test --test schema_persistence_test --test drop_table_free_test --test free_page_lifecycle_test -- --test-threads=1`
    - 批 2（9 个）：`cargo test --test check_command_test --test cli_test --test full_table_dml_test --test executor_test --test e2e_test --test view_test --test truncate_test --test plan_exec_test --test version_chain_test -- --test-threads=1`
    - 通过条件：两批全部 0 failed。
  - **本 Cycle 不跑全量回归**（整个 change 的收尾全量归 Iteration 009 / T16；M19：目标名字典序分组、每批 8–12 个目标、批内 `--test-threads=1`、批间串行、全量 Gate = 全部批次各自 0 failed）。
  - 失败含义：任何一条失败 ⇒ 对应 Acceptance 未满足 ⇒ 停在当前 task（不进入下游 task）。
- Stop when: 需要改动 `src/wal/**` / `src/storage/page_format/**` / `src/storage/btree/**` / `src/storage/integrity.rs` / `src/storage/data_page.rs` / `src/storage/buffer_pool.rs`；需要把越界槽改成跳过；需要改动目录行「过短负载」的既有语义；需要新增错误变体之外的错误语义；或需要全量回归才能判定（应回到 Plan 调整验证边界）。

**Invariants**

- 未损坏路径上，库的打开、表 / 列目录内容、DML 结果、DDL 窄化发布的提交点顺序与既有行为**逐点一致**。
- 目录链的遍历顺序与链式结构不变；目录行「过短负载」的既有 `StorageError::Internal` 语义不变。
- 物理释放 / 重建路径与 `rtsql check` 的诊断分级不变。
- 不引入身份证明机制、判定层、裁决脚本、新命令参数、新配置项、缓存或闸门机制。

**Non-goals**

- 恢复面组（T5，Iteration 005）；页头槽数越界形态的**实现**（Iteration 002）；写路径的槽界限（Iteration 003，T18 / T19）；数据页组（Iteration 001 / Cycle 000）。
- 目录损坏的**修复 / 抢救**、损坏目录的诊断报告（`rtsql check` 的分级）、dump / restore 合同面、磁盘格式。
- 目录页的页级判据扩展（例如把「过短负载」也纳入页级判据）。
- 全量回归（Iteration 009 / T16）。

**Acceptance**

| # | 验收条件 | 需求 / 场景 | 设计 | 任务 | 代码位置 | 测试 |
|---|---|---|---|---|---|---|
| A9 | 目录页上的越界槽在打开 / 读取路径上不 panic、不 abort，返回点名该页与该槽的明确错误；不被当作「没有这张表」静默跳过 | `page-read-bounded-read` / 「目录页上的损坏槽给出明确的错误」 | D7 目录页行 + Iteration 001 / Cycle 000 裁定 | T4 | `src/storage/catalog.rs`（5 处读点）、`src/storage/error.rs` | `tests/catalog_bounded_read_test.rs`（`__tables` / `__columns` 页损坏用例） |
| A10 | 目录损坏 ⇒ **该库 SHALL NOT 打开**（打开以该错误结束）；该后果在使用者面可见（`check` / `dump` / `.schema` 报出该打开错误，而非基于残缺目录的结论） | `page-read-bounded-read` / 「目录页上的损坏槽给出明确的错误」（末两条 AND） | Iteration 001 / Cycle 000 裁定 + D7 | T4 | `src/storage/catalog.rs::scan_chain`、`src/storage/data/table_manager.rs::open_or_init`（传播链，不改）、`src/cli/mod.rs::open_error_status`（既有，不改） | `tests/catalog_bounded_read_test.rs`（CLI 面用例）+ 库级打开失败用例 |
| A11 | 未损坏路径行为零变化（含目录行「过短负载」的既有 `StorageError::Internal` 语义） | `page-read-bounded-read` / 「未损坏路径的行为零变化」 | D7 | T4 | 5 处读点 | `tests/catalog_bounded_read_test.rs`（未损坏对照）+ 定向回归清单 |

**Verification**

- 逐 scenario 的最简直接判定（行为类 scenario 走**真实入口**：真实建库 → 真实写库 → 真实改坏磁盘字节 → 真实重开 → 真实执行语句 / 真实启动 CLI 二进制；观察返回的错误与进程是否存活）：
  1. 「目录页上的损坏槽给出明确的错误」：`cargo test --test catalog_bounded_read_test` 的库级用例 ⇒ 通过依据 = 打开返回 `Err`，错误文案含 `corrupted catalog page {page_id}` 且点名槽序号与区间，**测试进程未 panic / 未 abort**。
  2. 「该库 SHALL NOT 打开 + 后果可见」：同文件的 CLI 用例 ⇒ 通过依据 = `rtsql check` 退出码为普通非零（非信号终止），输出含 `failed to open database` 与 `corrupted catalog page`。
  3. 「未损坏路径的行为零变化」：同文件的未损坏对照 + 定向回归清单 ⇒ 通过依据 = 全部 0 failed。
- **不建判定层**：判定只落在 `cargo test` 的断言与退出码；不新增脚本 / 封装命令 / 专用参数；不使用 Hash / 指纹 / revision pin / run-id / manifest / 时间证明。
- **验证边界**：本 Cycle 跑上列定向回归，**不跑全量**（整个 change 的全量预算在 Iteration 009 / T16）。

**Gate 2 Readiness**

- Step 5 逐项复核：TBD / TODO = 无；需求裁剪 = 无（delta spec 已按三组裁定写细，`Simplified` = 0）；tasks / specs / design 一致 = 一致（T4 = 目录页组，与 D7 的目录页行、delta 的目录页场景、Iteration 001 / Cycle 000 的裁定逐条对应）；Current-State Evidence 全部来自当前工作区的逐行核对（5 处读点的实测行号、目录页页号常量、文件字节算术、打开路径传播链、CLI 的 `open_error_status` 形态、既有测试入口）；每个 task 的执行依据完整；调用者、影响范围、测试入口已定位（5 个读点 + 5 条传播链 + 18 个回归目标 + 1 个新测试文件）；未修改无关范围（Forbidden 已列明 `wal/**` / `page_format/**` / `btree/**` / `integrity.rs` / 数据页组文件）；Iteration Plan 覆盖全部任务且高开销验证预算明确（全量归 Iteration 009 / T16）；**Act 只需读本 Cycle 文件即可建立第一个先行测试并执行**（字节算术、页号、预期 RED 形态、错误文案形态、CLI 启动方式均已写明，无需回读前序 Cycle / 其它规划资料 / 重新调查）；非实质未知项已留在 Risks and Notes；验证直接证明目标行为，行为类 scenario 走真实入口，无身份证明机制与判定层。⇒ **全部 PASS**。
- RTM：`page-read-bounded-read` Requirement + 「目录页上的损坏槽给出明确的错误」+ 「未损坏路径的行为零变化」+ 「页头声称的槽数超出槽目录容量时读路径不 abort」（目录页部分）⇒ T4 ⇒ Covered（无 `Missing`、无 `Simplified`）。
- 设计闭合：行为差异、接口（`get_slot_data_checked` 复用 + `CorruptedCatalogPage` 新增）、错误语义（点名该页与该槽；不臆造表名）、5 处的处置均已定稿 ⇒ 无 TBD。
- 一致性：本 Cycle 与 change 的 `tasks.md`（T4 归属 Iteration 004）、`specs/page-read-bounded-read/spec.md`、`design.md` D7 / D14 / D15 一致。
- Persisted Evidence：`none`（见下）。
- 用户批准：本 goal 指令（「按照流程把最新规划的这一批ms做完……开始吧」）构成 Gate 1 / Gate 2 的批准授权；**批准不降低判据**，上述各项仍逐条检查并记录。

**Cycle RTM**

| Requirement / Scenario | Task | First Test（计划名） | Status |
|---|---|---|---|
| PRBR Scenario「目录页上的损坏槽给出明确的错误」 | T4 | `tests/catalog_bounded_read_test.rs::…`（计划名：`__tables` 页损坏 ⇒ 打开失败；`__columns` 页损坏 ⇒ 打开失败；Act 可改名并记录实际名） | Covered |
| PRBR Scenario「目录页上的损坏槽给出明确的错误」（末两条 AND：不得跳过该目录行；该库 SHALL NOT 打开 + 后果可见） | T4 | 同上 + CLI 面用例 | Covered |
| PRBR Scenario「页头声称的槽数超出槽目录容量时读路径不 abort」（目录页部分） | T4 | 同上（页级形态） | Covered |
| PRBR Scenario「未损坏路径的行为零变化」 | T4 | `tests/catalog_bounded_read_test.rs::…`（未损坏对照）+ 定向回归清单 | Covered |
| PRBR Scenario「恢复面上的损坏槽按恢复类别分别处置」 | T5（Iteration 005） | 计划名随 T5 的 Cycle 定稿 | Covered（归属 T5） |

**Persisted Evidence**

- Mode: none

全部结论可低成本重跑（1 个测试目标 + `cargo test --lib` + 18 个既有回归目标），命令与实际结果写入 Act Response 即可承载；不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ 不创建 `evidence/` 目录，不预建占位目录。

**Risks and Notes**

- **既有基线条件（非本 Cycle 引入，不纳入本 change）**：项目无 `rust-toolchain.toml`；HEAD（`e9cb9d3`）上 `cargo fmt --check` 报 **50 处 diff 分布在 9 个既有文件**，`cargo clippy --all-targets -- -D warnings` 报 **17 条 lib 错误**（deny 模式下 lib 即中止）、非 deny 模式 **32 条 warning** ⇒ 基线在 HEAD 即红。本 Cycle **只核对本 Cycle 触碰的文件**零新增 finding，**不重排**无关文件 ⇒ 该排除已记录在 `design.md` D13（ISS36）。
- **执行顺序依赖**：本 Cycle **必须**在 Iteration 002（`iterations/002-slot-count-underflow/000-initial.md`）**与** Iteration 003（`iterations/003-write-path-slot-bounds/000-initial.md`）被接受后执行 —— 页级槽数判据由 Iteration 002 交付并冻结；`src/storage/catalog.rs` 的**写入点**（`:707` 目录页链追加、`:924` 目录行原地改写）由 Iteration 003 交付并冻结（避免两个 Iteration 改同一批目录写入点）。这两个前置未落地前，本 Cycle 的「页级形态」与目录写入点部分无稳定形状可用。
- **API 名称的耦合**：Iteration 002 的页级判据的**方法名**由 Act 决定；本 Cycle 只依赖其**语义**（问「页头声称的槽数是否超出槽目录容量」）⇒ 若名称不同，本 Cycle 按实际名称调用即可（**非返工**）。
- **`__tables` 与 `__columns` 的链式结构**：目录链可有多页（`next_page_id`）；本 Cycle 的夹具只用单页目录（1 张表 ⇒ 1 行）⇒ 多页链上的同形行为由同一处代码路径覆盖（`scan_chain` 逐页同构）。
- **「短负载」与「越界」的分界**：目录行的「过短」既有语义是 `StorageError::Internal("catalog row too short …")` ⇒ 打开失败 ⇒ **保持不变**；本 Cycle 的改动只覆盖**越界**（偏移 / 长度越界）与**页头槽数越界**两种形态。
- **目录损坏后 DDL 路径的可达性**：因目录损坏时库打不开，5 处中的 2–5（DDL 写路径）只在「库打开后目录页被改坏」的情形下可达 ⇒ 它们的错误以命名错误结束（`?` 传播，不静默跳过）⇒ 与裁定同向。
- **跳过项**：本 Cycle 不跑全量回归（change 预算归 Iteration 009 / T16，非跳过而是预算安排）；不做全仓 fmt / clippy 修复（既有基线条件）；不改恢复面组；不改目录行「过短负载」的既有语义。

**后续任务边界**

- **授权边界**：无（T4 在 goal 指令授权范围内）。
- **能力边界**：无。
- **停止边界**：本 Cycle 完成且 Plan Review 判定 `accepted` ⇒ 展开 Iteration 005（T5，恢复面组；`iterations/005-recovery-bounded-read/000-initial.md` —— **尚未创建**，按正常流程在 T4 被接受后由 Plan 展开）；Act 不得自行创建下一 Cycle / 下一 Iteration。
- **不在本 Cycle、需在后续处理的事项**：T5（恢复面组，Iteration 005，含 `src/wal/recovery.rs:1519` 的写路径暴露）；ISS25 / ISS28 / ISS29（Iteration 006）；ISS30 / ISS31（Iteration 007）；ISS32 / ISS33（Iteration 008）；ISS34 / I045 / I064 与收尾全量（Iteration 009）。
- **交 Maintainer / Recorder 的事项**：ISS27 台账条目的关闭须等 T4 / T5 全部完成（由 Recorder 按用户指令执行）；本 Cycle 不登记 Issue、不改全局状态（`.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆均不动）、不合并规格。

---

## 追加标注（来自 Iteration 003 / Cycle 000 的 Plan Review，2026-10-26）

> 本节是**追加标注**，不改动上方已冻结的 `Plan Context` 正文（Plan Context 不可改写）。它把 Iteration 003 收尾时登记的一条 Minor finding 转交给本 Iteration，并给出条件式处置。

**来源**：Iteration 003 / Cycle 000 的 `Plan Review` F9（Minor finding，非阻塞；来源 = Act Response 的「遗留 Minor 问题」）。

**finding 内容**（Act 上报，Plan 独立核读确认成立）：

| 位置 | 现象 |
|---|---|
| `src/storage/catalog.rs:831`（`delete_from_chain`） | `let _ = slotted.delete_slot_by_logical_id(*lid);` ⇒ 删除错误被**丢弃** |
| `src/storage/catalog.rs:1005-1014`（`delete_named_copies_except`） | 删除失败被折叠成 `StorageError::Internal("catalog update of table '…' failed to remove stale copy (…)")` ⇒ **不点名该页**（`PageWriteError` 的 Display 不含页号） |

**归类更正**：这两点是目录**删除（写）路径**的语义，**不是**读点；Act 的「位于目录读点之后 ⇒ 属目录页组」在归类上不精确（那是**调用位置**关系，不是**性质**关系）。

**归属裁定：并入本 Iteration（T4），不新立 Iteration、不登记 Issue。** 依据是**可达性**而非位置：

1. 这两个删除各自的**前置读点本身就是本 Iteration 的 Target** —— `delete_from_chain`（`catalog.rs:808-809`）与 `delete_named_copies_except`（`catalog.rs:965-974`）；本 Iteration 的 Required behavior 要求「**取槽之前**问页级槽数判据」并把读点改为具名错误 ⇒ 落地后，损坏目录页上这两个删除**不可达**（页级槽数越界与槽负载越界两种形态都在读点被拦截）⇒ 错误丢弃 / 折叠**不可观察**。
2. 本 Iteration 的方向「目录页损坏 ⇒ 该库 SHALL NOT 打开」使整条目录删除路径在损坏库上不可达。
3. 按公共规则 › Scope Control「候选工作无法关联到可达代码和数据时，不实施」⇒ 无需新立 Iteration，Issue 台账亦无新增条目。

**条件式处置（执行时必须核对）**：若本 Cycle 实施时**未**在上述读点先行拦截（保留了 `let _ =` / 折叠分支仍可达的形态），则本标注**转为 T4 的一个 work item** —— 届时应把这两处删除错误改为**点名该页**并上浮（与 delta spec `page-write-bounded-write` 的「写入失败 SHALL NOT 被折叠为……泛化的无细节错误」一致）。若读点已按计划拦截并返回具名错误，则本标注**关闭**，无需改动删除路径。

**本次标注不改变**本 Cycle 的 Acceptance、验证边界、Target 清单与 `Plan Context` 状态（`ready`）。

---

## Act Response

**状态**: `reported`
**Cycle**: Iteration 004 / Cycle 000（`initial`）　**Task**: T4（目录页组：`__tables` / `__columns` 链的有界读）
**Act 执行日期**: 2026-10-26

### 接手时的实际状态

- 工作区 HEAD `e9cb9d3`。未提交修改：`.agents/docs/tasks.md`（MS37–MS40 路线批次，既有现场 —— **未回滚、未清理、未提交**）+ Iteration 000–003 的产品代码与测试改动（`src/storage/catalog.rs`、`src/storage/error.rs`、`src/storage/page_format/slotted_page.rs`、`src/storage/data_page.rs`、`src/storage/buffer_pool.rs`、`src/storage/integrity.rs`、`src/storage/btree/node.rs`、`src/storage/data/table_manager.rs`、`src/storage/mod.rs`、`src/storage/page_format/mod.rs`、`src/executor/*`、`src/transaction/manager.rs`、`src/pipeline.rs`、`tests/data_page_bounded_read_test.rs`、`tests/write_path_slot_bounds_test.rs`、`tests/index_read_bounded_read_test.rs`、`tests/release_path_bounded_read_test.rs` 等）+ 本 change 产物目录 + ISS35–ISS38 台账条目。
- **本 Cycle 无既有产物**：`tests/catalog_bounded_read_test.rs` **不存在**；`src/storage/catalog.rs` 中无本 Cycle 的改动（Iteration 003 的目录**写入点**改动在位，本 Cycle 只动**读点**）。⇒ **不是断点续跑，是从零实施**；无重复实现、无他人改动被回退。
- Cycle 的 Target 行号写于 Iteration 003 实施**之前**（`catalog.rs` +42 行）⇒ 全程**按符号名定位**（实际行号见 Implemented 表）。
- 前置条件核对：`Plan Context: ready`、`Review Result: pending`、无后继 Cycle；Iteration 002（页级槽数判据冻结）与 Iteration 003（目录写入点先行落地）均已 `accepted` ⇒ **可执行**。

### Implemented（实际改动）

**`src/storage/error.rs`** —— 新增错误变体（Cycle 明确授权）：

- `StorageError::CorruptedCatalogPage { page_id: u64, detail: String }`，`Display` = `corrupted catalog page {page_id}: {detail}`。

**`src/storage/catalog.rs`** —— 新增 4 个内部辅助函数 + 改造 5 处读点：

| 符号 | 实际行号 | 作用 |
|---|---|---|
| `corrupted_catalog_page_slot_count_error` | 797 | **页级**错误构造器：点名该页 + 页头声称的槽数 + 槽目录容量上限（`MAX_PAGE_SLOTS`） |
| `corrupted_catalog_page_slot_error` | 811 | **槽级**错误构造器：点名该页 + 该槽序号 + 越界区间 `[start, end)`；**不臆造表名** |
| `check_catalog_page_slot_count` | 825 | **取槽之前**问页级判据 `SlottedPageRef::slot_count_exceeds_capacity()` —— Iteration 002 冻结的**唯一**实现，未另造阈值、未第二处实现 |
| `catalog_slot_payload` | 839 | 槽负载一律经 `get_slot_data_checked` 取得（Iteration 001 / 003 冻结的有界判界）；越界 ⇒ 具名错误，**不跳过该目录行** |

| 读点 | 实际行号 | 读点内改动 |
|---|---|---|
| `scan_chain` | 754 | 页级判据（769）+ 有界槽负载（773） |
| `delete_from_chain` | 854 | 页级判据（873）+ 有界槽负载（877） |
| `update_field_in_chain` | 935 | 页级判据（956）+ 有界槽负载（962） |
| `delete_named_copies_except` | 1028 | 页级判据（1043）+ 有界槽负载（1055） |
| `rewrite_column_rows_drop`（Pass 2） | 1697 | 页级判据（1729）+ 有界槽负载（1735） |

**`tests/catalog_bounded_read_test.rs`**（**新建**）—— 5 个用例，真实文件库 + 真实 `Database::open` + 真实 CLI 二进制 + **真实文件字节改写**（无替身 / 桩）：

- `damaged_tables_catalog_page_makes_the_database_fail_to_open`（`__tables` 页槽 0 的 offset 字段写成 `0xFFFF`，文件偏移 4156）
- `damaged_columns_catalog_page_makes_the_database_fail_to_open`（`__columns` 页槽 0，文件偏移 8252）
- `damaged_catalog_page_slot_count_makes_the_database_fail_to_open`（**页级形态**：页头 `slot_count` 写成 700）
- `damaged_catalog_page_makes_the_cli_report_the_open_error`（`CARGO_BIN_EXE_rtsql` 真二进制跑 `rtsql check <db>`）
- `undamaged_catalog_reads_are_unchanged`（**未损坏对照**：库能打开、表与列目录可见、SELECT / INSERT / DELETE 结果与既有行为一致）

### 追加标注的条件式判定（Iteration 003 Plan Review F9）

**判定：标注「关闭」—— 落在「读点先行拦截」一侧 ⇒ 本 Cycle 未改动任何删除路径，未把删除错误改为点名该页。**

依据（可达性，三条）：

1. **`delete_from_chain`（`catalog.rs:854`）**：**读循环（870–885）在删除循环（887–895）之前**。读循环现在先消费页级判据（873）再逐槽取有界负载（877）⇒ 页级槽数越界与槽负载越界**两种形态**都在读循环内以具名错误 `?` 传播 ⇒ **删除循环在损坏目录页上不可达**。
2. **`delete_named_copies_except`（`catalog.rs:1028`）**：同构 —— 读循环（1040–1062：页级判据 1043 + 有界负载 1055）在删除循环（1064–1080，折叠错误 1076–1078）之前 ⇒ 折叠分支在目录页损坏下不可达。
3. **删除循环自身唯一可能的失败源**是 `delete_slot_by_logical_id` ⇒ `PageWriteError::SlotCountExceedsCapacity`（页头槽数越界 —— 已被**同一函数内**的读点点级判据先行拒绝）或 `LogicalIdNotFound`（非损坏形态）。⇒ 损坏目录页上「错误被丢弃 / 被折叠」**不可观察**。

**附带观察（不构成 work item，未改动）**：`rewrite_column_rows_drop`（`catalog.rs:1697`）内另有**第三处**同形态的 `let _ = slotted.delete_slot_by_logical_id(...)`（1764），不在追加标注的两处范围内，但由同一可达性论证覆盖（Pass 1 经 `delete_from_chain` 的有界读点；Pass 2 的读循环 1725–1752 在 `modify_page` 1756–1773 之前）⇒ 同样不可达。标注未授权该处，本 Cycle **未改动**。

### RED → GREEN 实况

**RED（变更前，实测）**

| 夹具 | 观察到的实际形态 |
|---|---|
| 库面 · 槽负载越界（`__tables` 页） | `thread '…' panicked at src/storage/page_format/slotted_page.rs:244:19:` → `range start index 65535 out of range for slice of length 4096`；随后 unwinding 中在 `src/storage/page_frame.rs:101:27` 二次 panic（页帧互斥量中毒）⇒ **进程 abort**（`signal: 6, SIGABRT`） |
| 库面 · 槽负载越界（`__columns` 页） | 同上（同一代码路径） |
| 库面 · 页级形态（`slot_count` = 700） | `panicked at src/storage/page_format/slotted_page.rs:244:19:` → `range end index 27392 out of range for slice of length 4096` ⇒ **进程 abort** |
| CLI 面（`rtsql check <db>`，真二进制） | **进程 abort（SIGABRT）**，shell 观察退出码 **134**；stderr = panic 文本 + backtrace（`check_db` → `open_readonly_with_lock_wait` → `open_internal` → `open_or_init` → `scan_tables` → `scan_chain` → `with_page_data` → `get_slot_data`）；**不含** `failed to open database`、**不含** `corrupted catalog page 0` |
| 未损坏对照 | 变更前即 GREEN |

⇒ RED 形态与 Cycle 钉死一致：**打开既不返回命名错误，也不给出任何诊断**。

**GREEN（变更后，实测）**

| 夹具 | 观察到的实际形态 |
|---|---|
| 库面 · `__tables` 页 | 打开以 `StorageError::CorruptedCatalogPage { page_id: 0, detail: "slot 0 range [65535, 65571) lies outside the page data area" }` 结束 —— **无 panic、无 abort**；该库**打不开** |
| 库面 · `__columns` 页 | 同形，`page_id: 1` |
| 库面 · 页级形态 | `CorruptedCatalogPage { page_id: 0, detail: "slot count 700 exceeds the page slot directory capacity of 680 slots" }` —— 点名该页、页头声称的槽数与容量上限 |
| CLI 面 | 退出码 **1**（**普通非零、非信号终止**）；输出 `{"error":"failed to open database /tmp/…/cat.db: corrupted catalog page 0: slot 0 range [65535, 65571) lies outside the page data area"}` ⇒ 含 `failed to open database` 与 `corrupted catalog page 0` |
| CLI 面 · `rtsql dump` | 同样以**打开错误**结束（退出码 1，同一文案）⇒ **不基于残缺目录给结论** |
| 未损坏对照 | 5/5 用例全绿（表与列目录可见、SELECT / INSERT / DELETE 结果与既有行为一致） |

### Deviations from Plan（与计划的偏差及原因）

1. **错误变体的声明先于先行测试落地**（顺序偏差，**非实质**）。Cycle 的 Implementation Guidance 顺序为 ① error.rs 新增变体 → ② catalog.rs 错误构造器 → ③ 5 处读点 → ④ 测试（先行用例 RED → GREEN）。本 Cycle 依该顺序先落地 ①（**纯声明性新增**：无任何既有代码路径能产生该变体，行为零变化），再写测试并观察**真实 RED**，再实施 ②③。
   - 原因：测试以变体名做**类型级**断言（`StorageError::CorruptedCatalogPage { page_id, detail }`），变体不存在时测试**编译不过** ⇒ 无法产生可判定的 RED。若退化为纯字符串断言则可编译，但会失去「错误必须是该具名变体、不得被改写成『该表不存在 / 目录为空』一类语义」的类型级保证。
   - **TDD 铁律未被破坏**：行为变更（③ 读点改造）之前存在真实 RED 观察（库面 panic + CLI 面 abort）。
2. **库面 RED 的实际形态比 Cycle 预测更重**：Cycle 预测「进程 panic / abort」。Rust 默认 unwind 下实测为「panic → unwinding 中因页帧互斥量中毒二次 panic ⇒ **abort（SIGABRT）**」，而非单纯的 panic 退出。panic **文本**与钉死值逐字一致（`range start index 65535 out of range for slice of length 4096`）；panic **位置**为 `slotted_page.rs:244:19`（Cycle 写的 `:128` 是 Iteration 003 之前的行号 —— 行号差异不是失败）。
3. **页级形态的 panic 文本与钉死值不同**：钉死文本对应**槽负载越界**形态；页级形态（页头槽数越界）的 panic 文本为 `range end index 27392 out of range for slice of length 4096`（槽目录定位下溢落到别的索引）⇒ **非失败**（两种形态的判据路径不同；页级形态的 GREEN 断言是「点名页 + 声称的槽数 + 容量上限」，不是 panic 文本）。
4. **CLI 面 RED 与 Cycle 预测一致**：Cycle 预测「进程 abort（非零退出但为信号终止）」⇒ 实测**确为信号终止**（SIGABRT，退出码 134）；GREEN 后为**普通非零退出码 1** + `failed to open database` + `corrupted catalog page 0`。
5. **Cycle 的 Target 行号全部按符号名重新定位**（见 Implemented 表）。附带发现：`catalog.rs` 当前共有 **3 处**同形态的 `let _ = slotted.delete_slot_by_logical_id(...)`（892 / 1764），追加标注只列了其中 1 处（892）。
6. **定向回归命令与 Cycle 清单逐字一致**：批 1（9 个目标）与批 2（9 个目标）均按 Cycle 给出的命令执行，批内 `--test-threads=1`、批间串行、批 1 全绿后才启动批 2。

### Self-Review（任务级 Review 与自检）

**Gate 3 · Spec compliance**

- A9（越界槽不 panic / 不 abort，返回点名该页与该槽的明确错误，不被当作「没有这张表」静默跳过）：**满足**。5 处读点全部经 `check_catalog_page_slot_count` + `catalog_slot_payload`；错误为 `CorruptedCatalogPage` 具名变体，`detail` 点名该页 + 该槽序号 + 越界区间；**无任何「跳过该目录行」路径**（损坏页一律 `?` 传播）；**不臆造表名**（越界时负载读不出 ⇒ 只点名页与槽）。
- A10（目录损坏 ⇒ 该库 SHALL NOT 打开；后果在使用者面可见）：**满足**。`scan_tables` / `scan_columns` 均经 `scan_chain` ⇒ 打开以该错误结束；`rtsql check` 与 `rtsql dump` 均报出该**打开错误**（退出码 1），不基于残缺目录给结论。
- A11（未损坏路径零变化，含目录行「过短负载」的既有 `StorageError::Internal` 语义）：**满足**。`deserialize_catalog_row` 未改动 ⇒ 「过短负载」语义不变；未损坏对照用例 + 18 个定向回归目标全绿。
- RTM 四行（损坏槽明确错误 / 末两条 AND / 页级形态 / 未损坏零变化）⇒ **Covered**，无 `Missing`、无 `Simplified`。
- Preserve 清单逐项核对：`get_slot_data_checked` / `slot_payload_in_bounds` 的判据与语义**未改**（`src/storage/page_format/slotted_page.rs` 本 Cycle 零 diff）；`get_slot` / `get_slot_by_logical_id` 的签名与 `None` 语义**未改**；`get_slot_data` 的签名与既有单元测试**未改**；目录行「过短负载」的既有 `StorageError::Internal` 语义**未改**；目录链遍历顺序与 `next_page_id` 链式结构**未改**；`Catalog::lock` 的锁语义**未改**；DDL 窄化发布的提交点顺序**未改**；`rtsql check` 的既有诊断分级与输出形态**未改**；dump / restore 合同面**未改**；物理释放 / 重建路径的既有语义**未改**。
- Forbidden 逐项核对：Forbidden 清单中的文件（`src/wal/**`、`src/storage/page_format/**`、`src/storage/btree/**`、`src/storage/integrity.rs`、`src/storage/data_page.rs`、`src/storage/buffer_pool.rs`、`src/executor/**`、`src/transaction/**`、`src/pipeline.rs`）**本 Cycle 零 diff** —— 全项目符号检索确认本 Cycle 新增的符号只出现在 `src/storage/catalog.rs`、`src/storage/error.rs`、`tests/catalog_bounded_read_test.rs`（工作区中这些文件的其余改动是 Iteration 000–003 的既有现场）。未跳过越界槽、未臆造表名、未改「过短负载」语义、未新增命令参数 / 配置项 / 缓存 / 闸门机制 / 判定层 / 身份证明机制、未改磁盘格式 / dump-restore 合同面 / `check` 诊断分级、未跑全量回归。

**Gate 3 · Code quality**

- diff 无计划外修改：改动仅落在 Cycle 的三个 Target 上（`error.rs` / `catalog.rs` / 新测试文件）。
- 错误 / 边界 / 状态 / 资源生命周期正确：页级判据与槽级判界各**只有一处**实现（复用冻结判据）；错误构造器集中且对称；新代码无 `unwrap` / `expect` / panic 路径。
- 无新增警告：`cargo clippy --all-targets`（非 deny）在本 Cycle 触碰的 3 个文件上**零 finding**（全项目 32 条 warning 与 HEAD 基线一致；deny 模式的 19 条 error 位置与基线一致，均不在本 Cycle 文件）。
- 测试不会因错误原因通过：损坏注入是**真实文件字节改写**（非替身 / 桩），观察对象是打开 / 命令返回的**错误与进程是否存活**；CLI 用例走真二进制 `CARGO_BIN_EXE_rtsql` 并断言**退出码**与**输出文案**。
- 无身份证明机制、无判定层：判定只落在 `cargo test` 的断言与退出码；未新增脚本 / 封装命令 / 专用参数 / Hash / 指纹 / revision pin / run-id / manifest / 时间证明。

**自检发现与已修复内容**（本 Cycle 实施过程中发现并在本 Cycle 内修复）：

1. **测试辅助函数的管道读取所有权错误**（`wait_cli` 把 `Vec` 移进读取线程后再在主线程使用）⇒ 编译不过。**已修复**：改用 `std::sync::mpsc::channel` 把每个管道的字节回传给等待方。
2. **CLI 面用例最初写成同步 `#[test]` + `futures::executor::block_on`** —— 该执行器无 tokio reactor，`Database::open` 的后台任务会 panic。**已修复**：改为 `#[tokio::test]`，CLI 等待放入 `tokio::task::spawn_blocking`。
3. **本 Cycle 自身引入的 4 处 rustfmt 差异**（`catalog.rs` 的 import 换行与 `catalog_slot_payload` 的 `Err(...)` 换行；测试文件的 `f.seek(...)` 换行与 `panic!` 换行）。**已修复**：按 rustfmt 的建议做**精准编辑**（未对全仓跑 `cargo fmt`，未重排 11 个既有红文件）⇒ `cargo fmt --check` 回到 HEAD 基线的 **50 处 diff / 11 个文件**，本 Cycle 触碰的文件 **0 处**。

**遗留 Minor 问题**（非阻塞，记录不改）：

1. **`rewrite_column_rows_drop`（`catalog.rs:1764`）的第三处 `let _ = slotted.delete_slot_by_logical_id(...)`** —— 不在追加标注的两处范围内，且由同一可达性论证覆盖（不可达）⇒ 本 Cycle 不改。若后续把「库打开后目录页被改坏」变成可稳定复现的场景，该处需要与另两处一并处理。
2. **`catalog.rs` 的 `let _ =` / 折叠与追加标注的行号已随本 Cycle 改动漂移** —— 标注引用的 `catalog.rs:831` / `:1005-1014` 现在对应 `:892` / `:1069-1078`。⇒ 该标注**关闭**，无需回填；台账「处置指针」字段归 Recorder 所有，本 Cycle **不改台账**。
3. **页级形态的 RED panic 文本与 Cycle 钉死文本不同**（见 Deviations 第 3 项）—— 已在上面记录，不影响验收。

### Verification Evidence（验证命令与实际结果）

| # | 命令 | 实际结果 |
|---|---|---|
| 1 | `cargo test --test catalog_bounded_read_test -- --test-threads=1`（**RED**，变更前） | **失败**：库面 panic `range start index 65535 out of range for slice of length 4096`（`slotted_page.rs:244:19`）→ 二次 panic（`page_frame.rs:101:27`）⇒ 进程 abort（`signal: 6, SIGABRT`）；CLI 面用例失败（子进程被信号终止 ⇒ 退出码 `None`）。**RED 成立** |
| 2 | `cargo test --test catalog_bounded_read_test -- --test-threads=1`（**GREEN**，变更后） | `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s` |
| 3 | `cargo test --lib` | `test result: ok. 420 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.11s` |
| 4 | 定向回归 **批 1**（9 个目标，Cycle 清单逐字一致，批内 `--test-threads=1`） | 全部 `ok`，合计 **85 passed / 0 failed**：`storage_test` 21、`table_manager_test` 6、`alter_table_test` 16、`ddl_catalog_transactional_test` 3、`ddl_rewrite_consistency_test` 9、`catalog_durability_test` 4、`schema_persistence_test` 8、`drop_table_free_test` 6、`free_page_lifecycle_test` 12 |
| 5 | 定向回归 **批 2**（9 个目标，批 1 全绿后才启动，串行） | 全部 `ok`，合计 **245 passed / 0 failed**（`cli_test` 2 ignored 为既有）：`check_command_test` 11、`cli_test` 124、`full_table_dml_test` 11、`executor_test` 39、`e2e_test` 27、`view_test` 19、`truncate_test` 7、`plan_exec_test` 4、`version_chain_test` 3 |
| 6 | `cargo fmt --check` | **50 处 diff / 11 个文件**（与 HEAD 基线**完全一致**）；本 Cycle 触碰的 `src/storage/catalog.rs`、`src/storage/error.rs`、`tests/catalog_bounded_read_test.rs` ⇒ **0 处** |
| 7 | `cargo clippy --all-targets`（非 deny） | **32 条 warning**（与基线一致）；本 Cycle 触碰的 3 个文件 ⇒ **0 finding** |
| 8 | `cargo clippy --all-targets -- -D warnings`（deny） | 仍在 lib 即中止，**19 条 error**，位置与基线一致（`src/cli/lifecycle.rs`、`src/executor/executor_trait.rs`、`src/network/protocol.rs`、`src/parser/planner/ddl_dml.rs`、`src/pipeline.rs`、`src/storage/async_storage.rs`、`src/storage/buffer_pool.rs`、`src/storage/file_storage.rs`、`src/transaction/manager.rs`）⇒ 本 Cycle 触碰的文件**零新增 finding**；test 目标在 deny 模式下未被 lint（与基线同） |
| 9 | CLI 实跑（真二进制，损坏 `__tables` 页） | `rtsql check <db>` ⇒ 退出码 **1**，输出 `{"error":"failed to open database /tmp/…/cat.db: corrupted catalog page 0: slot 0 range [65535, 65571) lies outside the page data area"}`；`rtsql dump <db>` ⇒ 同文案、退出码 1 |

**既有结论失效的处理**：Iteration 003 的 `cargo test --lib`（420/0）与定向回归（354/0）因本 Cycle 改动**已失效** ⇒ 本次**重跑**（上表 #3 / #4 / #5），不复用旧结论。

**结论时效性（本 Cycle 的 4 处格式修正之后重跑）**：上表 #1–#5 的**首次**运行发生在本 Cycle 自身引入的 4 处 rustfmt 修正**之前**（`catalog.rs` 的 import 换行 / `Err(...)` 换行，测试文件的 `f.seek(...)` / `panic!` 换行）。按公共规则 › 验证「被覆盖的代码在结论产生后发生变化 ⇒ 定向审计」，已在修正后**重跑全部受影响结论**：`catalog_bounded_read_test` **5 passed / 0 failed**（0.25s）、`cargo test --lib` **420 passed / 0 failed**（9.35s）、定向回归批 1 **85 passed / 0 failed**、批 2 **245 passed / 0 failed** —— 与上表数字**逐项一致**。`cargo fmt --check`（50 处基线 diff / 本 Cycle 文件 0 处）与 `cargo clippy --all-targets`（32 条 warning / 本 Cycle 文件 0 finding）本就在修正**之后**运行，未失效。⇒ 上表全部结论均为**当次真实运行**的当前结论。

### Persisted Evidence

**None required.**（Plan 的 `Persisted Evidence: Mode: none`；本次 RED / GREEN 均为可低成本复现的观察，不满足公共规则 › 验证 的保存白名单；未创建 `evidence/` 目录。）

### Experience Candidates

**None.** —— 本 Cycle 未产生满足记录边界的候选：无已端到端验证且可重复 / 高风险的操作流程可入 Runbook；本 change 范围外**无**有证据的实质缺陷（追加标注经判定**关闭**；第三处 `let _ =` 不可达，已在遗留 Minor 中记录，不构成缺陷）。

### 未解决问题

1. **无阻塞项**。Gate 3 / Gate 4 全部通过，Act Response 状态 = `reported`。
2. **待 Plan Review 裁定**：本 Cycle 的实现与验收结论；以及追加标注（F9）的**关闭**判定是否被接受。
3. **交 Maintainer / Recorder 的事项**（本 Cycle **不执行**）：ISS27 台账条目的关闭须等 T4 / T5 全部完成（由 Recorder 按用户指令回填「处置指针」）；本 Cycle **未登记 Issue、未改台账**、**未同步全局文档**（`.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆 / references 均未动）、**未合并规格**、**未归档 change**、**未填写 `Plan Review`**、**未创建下一 Cycle 或下一 Iteration**、**未执行 `git commit`**。

### 状态同步

- change `tasks.md`：T4 状态 → `Act Response reported（Iteration 004 / Cycle 000），待 Plan Review`；Iteration Map 的 **004** 行状态 → `Cycle 000 = T4 已报告，待 Plan Review`（T5 / 005–009 与 **Next Iteration** 指针不变 —— 展开 Iteration 005 属 Plan 的权限，须待 Iteration 004 被接受）。
- 本 Cycle 的 **Plan Context** 未改写；**Plan Review** 未填写。

## Plan Review

- Review Result: accepted

**核对方法（独立于 Act 的验证）**

- 独立阅读当前工作区代码与 `git diff`（HEAD `e9cb9d3`）：`src/storage/catalog.rs`、`src/storage/error.rs`、`src/storage/page_format/slotted_page.rs`、`src/storage/data/table_manager.rs`、`src/storage/data_page.rs`、`src/cli/mod.rs`、`src/cli/lifecycle.rs`。
- Act 报告的 RED / GREEN 结论**直接沿用**（来源：Act Response 的 Verification Evidence；只读 git 检查确认 Act 报告后 `src/storage/catalog.rs` / `src/storage/error.rs` 未再变化 ⇒ 覆盖范围未失效）⇒ 本 Review **未重跑**任何 Act 已沿用的验证。
- 本 Review 新增的判定**只有代码事实类**的静态核对（符号、行号、控制流、错误语义、错误变体的产生路径），不引入新的运行结论；不需要新的运行证据的理由 = 本 Review 的每个问题都能由最终代码形态唯一确定（无时序依赖、无并发依赖、无不稳定输出）。
- 逐条核对对象：Acceptance A9 / A10 / A11、Implementation Guidance 的关键取舍、Preserve 清单、Forbidden 清单，以及来自 Iteration 003 / Cycle 000 的 `Plan Review` F9 条件式 work item（追加标注节）。

**Findings**

**F1（PASS，A9）5 处目录读点全部在取槽之前问页级判据；页级判据消费 Iteration 002 冻结的唯一实现；槽负载一律经 `get_slot_data_checked`；越界槽以点名该页与该槽的明确错误结束，不被跳过。**

独立核读 `src/storage/catalog.rs` 的 5 处读点（行号为当前工作区）：

| 读点 | 页级判据（取槽之前） | 槽负载（有界） | 错误传播 |
|---|---|---|---|
| `scan_chain` | `:769` `check_catalog_page_slot_count(&slotted, page_id.0)?` | `:773` `catalog_slot_payload(&slotted, page_id.0, i, &slot)?` | `:779` `with_page_data(...).await?` ⇒ 打开路径上抛 |
| `delete_from_chain` | `:873` | `:877` | `:885` `?` |
| `update_field_in_chain` | `:956` | `:962` | `:972` `?` |
| `delete_named_copies_except` | `:1043` | `:1055` | `:1062` `?` |
| `rewrite_column_rows_drop` | `:1729` | `:1735` | `:1752` `?` |

- **页级判据是唯一实现、无另造阈值**：`check_catalog_page_slot_count`（`:825-833`）的函数体**只**调 `slotted.slot_count_exceeds_capacity()`（`src/storage/page_format/slotted_page.rs:204-206`）⇒ 委派 `SlottedPageHeader::slot_count_exceeds_capacity`（`:172-174`，判据 `self.slot_count as usize > MAX_PAGE_SLOTS`）⇒ `MAX_PAGE_SLOTS`（`:89`）是全项目**唯一定义**（`grep` 确认 `src/` 内无第二处容量阈值常量）。`src/storage/catalog.rs` 内无任何自造阈值 ⇒ 满足 delta spec「槽目录容量的界限 SHALL 在全项目只有一处定义」。
- **槽负载一律经有界访问器**：`grep -n "get_slot_data" src/storage/catalog.rs` 的命中项只有 `get_slot_data_checked`（`:845`，在 `catalog_slot_payload` 内）与注释 ⇒ **无一处**直调无范围检查的 `SlottedPageRef::get_slot_data`；`get_slot_data_checked`（`slotted_page.rs:253-258`）内部以 `slot_payload_in_bounds` 判界（读侧与写侧共用同一判界实现）。
- **错误变体与 detail 形态**：`src/storage/error.rs:78-86` 新增 `CorruptedCatalogPage { page_id: u64, detail: String }`，`Display` = `corrupted catalog page {page_id}: {detail}`。
  - 槽级形态：`corrupted_catalog_page_slot_error`（`catalog.rs:811-818`）⇒ detail = `slot {slot_index} range [{start}, {end}) lies outside the page data area` ⇒ **点名该页（外层 `page_id`）+ 该槽序号 + 越界区间**；**不含表名** ⇒ 不臆造表名 ✓。
  - 页级形态：`corrupted_catalog_page_slot_count_error`（`catalog.rs:797-804`）⇒ detail = `slot count {slot_count} exceeds the page slot directory capacity of {MAX_PAGE_SLOTS} slots` ⇒ 点名该页 + 页头声称的槽数 + 容量上限。
- **不得改写成「该表不存在 / 目录为空」**：5 处全部以 `?` 传播（无 `unwrap` / `expect` / `ok()` / `Err(_) =>` 吞错分支）；`scan_chain` 的解析仍在 `deserialize_catalog_row`（`:781` `parse(&row_bytes)?`）⇒ 既有 `StorageError::Internal("catalog row too short …")` 语义**未改**（本 Cycle 的 diff 未触碰该函数）⇒ 目录行「过短负载」仍按既有语义打开失败。
- **`get_slot` 的 `None` 语义未改**：5 处读点的 `if let Some(slot)` / `else { continue; }` 分支保持原样（Cycle 的 Implementation Guidance 明确要求不改）⇒ 未把「页损坏」折叠成「槽不存在」。

**F2（PASS，A10）打开路径以该错误结束 ⇒ 该库 SHALL NOT 打开；使用者面报出打开错误而非基于残缺目录的结论。**

- 传播链（独立核读）：`Database::open`（`src/database.rs:199`）→ `TableManager::open_or_init`（`src/storage/data/table_manager.rs:373-374` `self.catalog.scan_tables().await?`）→ `Catalog::scan_tables` / `scan_columns` → `scan_chain`（`catalog.rs:754`）⇒ 目录读点的 `Err` 以 `?` 一路上抛 ⇒ **打开失败**。该链路上**没有**任何「跳过损坏目录行 / 当作目录为空」的分支。
- 使用者面：`src/cli/mod.rs:671` 的打开失败分支 → `open_error_status`（`:586-602`）⇒ `CorruptedCatalogPage` 落入 `other` 臂 ⇒ `ExitStatus::General("failed to open database {path}: corrupted catalog page {page_id}: {detail}")` ⇒ **普通非零退出码 + 明确文案**（非信号终止 ⇒ 不 abort）。
- `rtsql check` / `dump` / `.schema` **都先开库**（`src/cli/lifecycle.rs` 的 `check_db:269` / `dump_db` / `schema_db` 均经 `execute_command_inner*` ⇒ `src/cli/mod.rs:658-675` 的开库段）⇒ 打开失败即在命令体执行**之前**终止 ⇒ **不会**给出基于残缺目录的结论 ✓。
- 与 Act 报告一致（Act：CLI 面退出码 1、输出含 `failed to open database` 与 `corrupted catalog page 0`）；本 Review 未重跑该验证（沿用）。

**F3（PASS，A11）未损坏路径零变化；Preserve 清单逐条成立。**

| Preserve 项 | 核对结果 | 证据 |
|---|---|---|
| `get_slot_data_checked` / `slot_payload_in_bounds` 的判据与语义 | 成立 | 本 Cycle 未改 `src/storage/page_format/**`（见 F6）⇒ 判据原样复用 |
| `get_slot` / `get_slot_by_logical_id` 的签名与 `None` 语义 | 成立 | `slotted_page.rs` 零 diff（本 Cycle）⇒ 签名未动 |
| `get_slot_data` 的签名与既有单元测试 | 成立 | 同上 |
| 目录行「过短负载」的既有 `StorageError::Internal` 语义 | 成立 | `deserialize_catalog_row` 未被本 Cycle 触碰；`scan_chain:781` 仍以 `parse(...)?` 传播 |
| 目录链遍历顺序与 `next_page_id` 链式结构 | 成立 | 5 处读点仍是 `while let Some(page_id) = current` + `visited` 去环 + `slotted.header().next_page_id` 串联；本 Cycle 的 diff **只**插入判据与有界负载两行调用，未改控制流 |
| `Catalog::lock` 的锁语义 | 成立 | `catalog.rs` 的 diff 未触碰任何 `self.lock.lock().await`；判据是纯读（页头的一个 `u16`），判定与取槽之间**无 await** ⇒ 无竞态窗口 |
| DDL 窄化发布的既有提交点顺序 | 成立 | `TableManager::apply_column_drop`（`src/storage/data/table_manager.rs:1229-1236`）仍是「列行先（`rewrite_column_rows_drop`）、计数后（`update_table_column_count_ddl`）」，该文件的 diff 属 Iteration 001，**本 Cycle 零改动** |
| `rtsql check` 的既有诊断分级与输出形态 | 成立 | 本 Cycle 未改 `src/cli/**`、未改 `src/storage/integrity.rs` ⇒ `check` 的分级与输出逐字未动 |
| dump / restore 合同面 | 成立 | 本 Cycle 未改 dump / restore 实现 |
| 物理释放 / 重建路径的既有语义 | 成立 | 本 Cycle 未改 `src/storage/btree/**`、`src/storage/integrity.rs`、物理释放路径 |

**F4（PASS）Forbidden 与落点纪律成立。**

- 本 Cycle 的**实际落点 = 2 个文件**：`src/storage/error.rs`（+21 行，仅新增一个变体）与 `src/storage/catalog.rs`（+123/-20 行：4 个内部辅助函数 + 5 处读点的两行调用改造）。`git diff --stat` 对这两个文件的增删行数与 Act 的 Implemented 表逐项吻合。
- **Forbidden 文件本 Cycle 零 diff**：工作区是**累积**状态（Iteration 000–003 的改动同在未提交区），因此本项按「各前置 Cycle 自身记录的落点」逐文件归属核对：Iteration 000 → `src/storage/integrity.rs`；Iteration 001 → `src/storage/page_format/slotted_page.rs`、`src/storage/data_page.rs`、`src/storage/mod.rs`、`src/storage/buffer_pool.rs`、`src/executor/data_scan.rs`、`src/executor/create_index.rs`、`src/transaction/manager.rs`、`src/pipeline.rs`、`src/storage/data/table_manager.rs`；Iteration 002 → `src/storage/page_format/slotted_page.rs`、`src/storage/page_format/mod.rs`、`src/storage/btree/node.rs`、`src/storage/data_page.rs`、`src/storage/mod.rs`；Iteration 003 → `src/storage/page_format/slotted_page.rs`、`src/storage/data_page.rs`、`src/storage/catalog.rs`（**目录写入点**）、`src/storage/page_format/mod.rs`。⇒ 本 Cycle 的 2 个落点（`error.rs` + `catalog.rs` 的**读点**）与 Forbidden 清单**零交集**；`src/wal/**` 全程零 diff（Iterations 000–003 的落点表中不含 `src/wal/**`）。

**F5（PASS，偏差裁定）3 项 Deviation 的分类与裁定。**

| # | Act 上报 | 分类 | 裁定 |
|---|---|---|---|
| ① | 错误变体的**声明**先于先行测试落地 | **非实质的执行顺序偏差**（Plan 预期偏差 / 现状变化），**不构成 TDD 铁律的实质违规** | 见下方逐条论证 |
| ② | 库面 RED 的 panic 位置为 `slotted_page.rs:244:19`（Cycle 写的是 `:128`） | **Plan 预期偏差**（行号过时），非实质 | panic **文本**逐字一致（`range start index 65535 out of range for slice of length 4096`）⇒ 断言语义未变；`:128` 是 Iteration 003 之前的行号，Cycle 的 First test 写于那次改动之前 ⇒ 属**行号过时**而非行为偏差 |
| ③ | 页级形态的 panic 文本与钉死值不同（`range end index 27392 …`） | **Plan 预期偏差**，非实质 | 两种形态走**不同的判据路径**（页级形态在 `get_slot` 的槽目录定位处下溢 ⇒ `range end`；槽级形态在负载切片处 ⇒ `range start`）⇒ 文本本就不应相同；GREEN 断言是「点名页 + 声称的槽数 + 容量上限」而非 panic 文本 ⇒ 验收口径未被削弱 |

**Deviation ① 的裁定（依据条款）**

- **裁定 = 非实质的执行顺序偏差，不是「实质违规」。** 理由（逐条对应公共规则）：
  1. **被保护的对象是「行为变更」**。AGENTS.md › TDD 的铁律表述为「新功能：测试定义期望，**观察 RED**，再实现 GREEN」；「没有先行测试不变更」约束的是**行为语义**的变更。本 Cycle 的**行为变更**是 5 处读点的改造（判据 + 有界负载），Act 的顺序是：**类型声明 → 先行测试 → 观察真实 RED → 实现错误构造器与读点改造 → GREEN** ⇒ **每一个行为变更之前都存在真实、可判定的 RED 观察**（库面 `__tables` / `__columns` 页槽负载越界 ⇒ `slotted_page.rs:244:19` panic ⇒ unwinding 中 `page_frame.rs:101:27` 二次 panic ⇒ **SIGABRT**；CLI 面真二进制 `rtsql check` ⇒ **SIGABRT、shell 退出码 134**，stderr 为 panic 文本 + backtrace，**不含** `failed to open database`、**不含** `corrupted catalog page 0`）⇒ TDD 铁律的**实质要求已满足**。
  2. **先行落地的只是「类型声明」，行为零变化**。独立核实：`CorruptedCatalogPage` 在 `src/` 内的全部出现点 = `src/storage/error.rs:86`（变体声明）+ `src/storage/catalog.rs:798` / `:814`（**新增**的两个错误构造器）⇒ 在读点改造落地之前，**没有任何既有路径能产生该变体**（变体未被 uninhabited 之外的任何构造点使用）⇒ 该步骤是**纯声明性新增**，不改变任何可观察行为，符合公共规则 › Scope Control 的「不改变系统可观察行为（功能语义）的修改不建 change / 默认直接执行」的精神，也符合 AGENTS.md › 轻量工作。
  3. **先写测试在该断言形态下无法产生可判定的 RED**。Act 的测试以**类型级**断言（`StorageError::CorruptedCatalogPage { page_id, detail }`）⇒ 变体不存在时测试**编译不过**；而 AGENTS.md › 验证 要求「完成声明基于真实运行」且公共规则要求 RED 必须是**可观察的行为失败**（编译错误不是行为失败）⇒ 该断言形态下「先测试」在客观上无法产生合规的 RED。
  4. **该偏差本可避免，但不是本 Cycle 的返工理由**。若先行测试改为断言**可观察行为**（例如断言打开失败的错误文案含 `corrupted catalog page {page_id}`，不断言变体类型），则变体可作为 GREEN 实现的一部分落地，从而完全不触碰顺序问题。⇒ 记录为**可避免的执行顺序偏差**，**不构成 Acceptance gap**，**不触发 rework**（改写范围会超出「本 Cycle 的既有 Acceptance 的有限修复」边界）。
  5. **归入 Plan 预期偏差**：Plan 的 Implementation Guidance 给的是「**建议顺序**」，公共规则 › Gate 2 允许「非实质选择留给 Act」；Act 的偏离既未改变冻结的方向裁定（目录页损坏 ⇒ 该库 SHALL NOT 打开），也未改变 Acceptance、Forbidden、验证边界。
- 依据条款：AGENTS.md › TDD（铁律与三形态）、› 执行约束 7、› 轻量工作、› Gate 2（非实质选择留给 Act）、公共规则 › Scope Control、› 验证（真实运行与可判定的 RED）。

**F6（PASS）新增变体是「纯声明性新增」这一前提成立（Deviation ① 的成立前提已据代码核实）。**

- `grep -rn "CorruptedCatalogPage" src/` ⇒ 仅 `src/storage/error.rs:86`（定义）与 `src/storage/catalog.rs:798` / `:814`（两个**新增**构造器）；`src/storage/mod.rs` 的重导出未新增该变体（变体随 `StorageError` 整体导出）⇒ 变体在读点改造前**不可被产生**、**不可被观察** ⇒ 纯声明性新增成立。
- 其余既有变体的语义**未被改写**：`CorruptedIndexPage`（迭代器侧）、`CorruptedDataPage`（Iteration 001，数据页侧）均未改动 ⇒ 目录页**不复用** `CorruptedDataPage`（页型不符 ⇒ 会让使用者看到「corrupted data page 0」），与 Plan 的取舍一致 ✓。

**F7（PASS，F9 条件式 work item 的裁定）追加标注的条件成立 ⇒ 标注关闭；三处删除路径的可达性逐条论证如下。**

**标注的条件**（来自本 Cycle 文件的追加标注节）：「若本 Cycle 实施时**未**在上述读点先行拦截 ⇒ 标注转为 T4 的一个 work item；若读点已按计划拦截并返回具名错误 ⇒ 标注**关闭**，无需改动删除路径。」

**条件成立的直接证据**：F1 已逐点核实，5 处读点**全部**在取槽之前问页级判据、且槽负载全部经 `get_slot_data_checked` ⇒ **两种损坏形态都在读点被拦截**（页级形态：`slot_count > MAX_PAGE_SLOTS` ⇒ `:769/:873/:956/:1043/:1729` 返回具名错误；槽级形态：负载越界 ⇒ `:773/:877/:962/:1055/:1735` 返回具名错误）⇒ 条件的第一分支不成立 ⇒ 标注**关闭**。

**三处删除路径的逐条可达性论证**（记法：页级形态 = 页头槽数越界；槽级形态 = 槽负载偏移 / 长度越界）

1. **`delete_from_chain`（`catalog.rs:854-913`；Act 报告的 `let _ =` 在 `:892`）**
   - 读段 `:870-885`：`:873` 拦页级形态（`?` ⇒ `Err` 直接从闭包返回）；`:875-882` 的遍历对**每个**槽调 `:877` `catalog_slot_payload` ⇒ 任一槽的负载越界即 `Err`（槽级形态）⇒ `:885` `?` 把错误上抛。
   - 删除段 `:887-895`（`let _ =` 在 `:892`）**只在读段返回 `Ok` 后**才可达（`:887` 的 `if !to_delete.is_empty()`）。
   - ⇒ 损坏目录页上删除段**不可达** ⇒ 被丢弃的删除错误**不可观察**。
2. **`delete_named_copies_except`（`catalog.rs:1028-1089`；Act 报告的折叠错误在 `:1069-1078`）**
   - 读段 `:1040-1062`：`:1043` 拦页级形态；`:1055` 拦槽级形态；`:1062` `?` 上抛。
   - 删除段 `:1064-1080`（折叠为 `StorageError::Internal("catalog update of table '…' failed to remove stale copy (…)")`）**只在读段返回 `Ok` 后**才可达。
   - ⇒ 损坏目录页上删除段**不可达** ⇒ 折叠掉页号的错误**不可观察**。
3. **`rewrite_column_rows_drop`（`catalog.rs:1697-1790`；Act **额外发现**的第三处 `let _ =` 在 `:1764`）**
   - Pass 1 `:1701-1710` 调 `delete_from_chain`（其读点即上文第 1 条）⇒ 损坏的 `__columns` 页在 Pass 1 即被拦截 ⇒ `:1710` `?` ⇒ **Pass 2 不运行**。
   - Pass 2 `:1718-1788`：读循环 `:1727-1752` 内 `:1729` 拦页级形态、`:1735` 拦槽级形态、`:1752` `?` 上抛；`modify_page` 段 `:1756-1773`（含 `:1764` 的 `let _ =`）**只在读段返回 `Ok` 后**才可达。
   - ⇒ 第三处与前两处**同形同论证**，损坏目录页上**不可达** ⇒ 错误丢弃**不可观察**。
   - 补充：第三处与前两处的**唯一结构差别**是 `:1759` 的 `add_slot` 在同一 `modify_page` 闭包内先于 `:1764` 的删除执行；`add_slot` 的失败（`PageFull`）经 `:1768-1772` 以既有 `StorageError::Internal` 上抛，属**既有语义**，与本标注无关。

**与公共规则 › Scope Control 的一致性**：两处标注位置 + Act 额外发现的第三处，在本 Cycle 落地后于**损坏目录页**上均不可达 ⇒ 其错误丢弃 / 折叠**无法关联到可达代码与数据** ⇒ 按 Scope Control「候选工作无法关联到可达代码和数据时，不实施」，**不实施**该改动 ⇒ 标注**关闭**（三处一并关闭），**不新立 Iteration、不登记 Issue**。

**边界声明（既不因「读侧已拦」断言该层无价值，也不因「有 defense-in-depth」放宽验收）**：
- 该层**不是**被认定为无价值：它只是在本 change 的两种损坏形态（页级槽数越界 / 槽负载越界）下**不可观察**。
- 遗留的**唯一可失败源**：`delete_slot_by_logical_id` 在通过读点的页上仍可能返回 `PageWriteError::LogicalIdNotFound`（`slotted_page.rs:479`）——但该 logical_id 刚刚从**同一页的同一批字节**读出（`delete_from_chain` 的 `:879` / `delete_named_copies_except` 的 `:1057`）⇒ 只有在**同一页出现重复 logical_id** 时才可能发生；重复 logical_id 属于**本 change 判定口径之外**的损坏形态（delta spec 的判定口径只覆盖「负载越界」与「页头槽数越界」），且**当前写路径不产生重复 logical_id**（运行期 logical_id 单调分配且页内不复用；恢复面 `add_slot_with_logical_id`（`src/wal/recovery.rs:207` 附近）以 `row_id.slot_id` 强制落位，且 `redo_tuple_at_row_id` 先判 `already_present` ⇒ 不产生重复）⇒ 该残留**当前不可达**，**不构成本 change 范围内的实质缺陷**，登记为**非阻塞 Minor finding**（见下），**不作为 Issue 候选上报**。

**Acceptance Gaps**

**无。** A9 / A10 / A11 三条验收条件逐条满足（见 F1 / F2 / F3）；3 项 Deviation 全部为非实质偏差且不改变 Acceptance；追加标注的条件成立 ⇒ 无需要 Act 返工的缺口。

**收敛判断**

- 本 Cycle 的 Acceptance 已全部满足；实现与 Plan Context 的方向、Target、Preserve、Forbidden、验证边界**逐条一致**；无未解决的实质未知项。
- 「既有方向 + 一处措辞细化」的定位成立（目录页损坏 ⇒ 该库 SHALL NOT 打开），**未引入新方向** ⇒ 不触发拆分信号（tasks.md 平衡审计的拆分信号条件不成立）。
- 当前 Iteration 的 Review Result = `accepted` ⇒ Iteration 004 **完成**，可展开 Iteration Map 的下一 Iteration（005 / 恢复面组）。
- 本 Cycle **未**产生需要 Act 返工的问题 ⇒ **不创建 rework Cycle**。

**Persisted Evidence**

**None required** —— Plan 的 `Persisted Evidence` 为 `none`；全部结论（RED 形态、GREEN、未损坏对照）由低成本可重跑的 `cargo test --test catalog_bounded_read_test -- --test-threads=1` + `cargo test --lib` + 18 个定向回归目标承载，Act Response 的 Verification Evidence 段可完整承载命令与实际结果 ⇒ 未创建 `evidence/` 目录，不预建占位目录。

**Issue 候选**

**无。** 逐项核对：(a) 本 change 范围内 —— 无未解决的实质缺陷（F7 的遗留项为**当前不可达**的重复 logical_id 形态，不可达 ⇒ 非实质缺陷）；(b) 本 change 范围外 —— 本次 Review 未发现新的范围外实质缺陷（`src/wal/**` 的 4 处恢复面读点属 **T5 / Iteration 005** 的既有范围，非范围外缺陷）。⇒ **只报告、不登记**；ISS27 台账的「处置指针」归 Recorder 回填。

**未解决问题**

- **无阻塞项。**
- 恢复面组（`src/wal/recovery.rs` 的 4 处读点：`build_pk_version_maps` / `derive_old_row_id` / `rebuild_pk_indexes` / `mark_uncommitted_aborted`）仍是无界读 ⇒ 属 **T5 / Iteration 005** 的范围，本 Cycle Forbidden 明列不改；已在下一 Iteration 展开。
- 本 Review **未修改产品代码、未同步全局文档（`.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆 / references 均未动）、未改 ISS27 台账、未执行 `git commit`、未归档 change**；只更新了本 change 内的 `tasks.md` 与 `specs/page-read-bounded-read/spec.md`（delta 增量），并按公共规则展开 Iteration 005。
