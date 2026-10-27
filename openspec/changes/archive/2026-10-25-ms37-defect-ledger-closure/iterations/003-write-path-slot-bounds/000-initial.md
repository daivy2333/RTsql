# Iteration 003 / Cycle 000: 写路径的槽界限（ISS37 + ISS38）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T18（ISS37）、T19（ISS38）
- Acceptance gaps: None
- Repair items: None
- Inherited scope:
  - Iteration 001（T3）**冻结**的读侧原语：`SlottedPageRef::get_slot_data_checked`（`src/storage/page_format/slotted_page.rs:137-144`）与其内部判界 `slot_payload_in_bounds`（`:180-183`，**当前为私有**）—— 本 Cycle **复用**其判界口径，**不得重复实现**；
  - Iteration 002（T17 / ISS35）**冻结**的页级语义：`MAX_PAGE_SLOTS = 680`（`src/storage/page_format/slotted_page.rs:21`，**全项目唯一定义**）、页级判据 `SlottedPageHeader::slot_count_exceeds_capacity`（`:89-91`，`slot_count > MAX_PAGE_SLOTS` ⇒ `slot_count ≥ 681` 判损坏；只读 / 可写委托 `:121-123` / `:217-219`）、`StorageError::CorruptedDataPage { page_id, detail }` 的**变体形状与文案前缀** `corrupted data page {page_id}: `；
  - `src/storage/data_page.rs` 的三个错误构造器：**逐槽越界** `corrupted_data_page_slot_error`（`:15`）、**页头槽数** `corrupted_data_page_slot_count_error`（`:48`）、**过短负载** `short_data_page_slot_error`（`:28`）—— 本 Cycle **复用**，文案逐字不变；
  - `design.md` D12 的读路径冻结方向：① 判定实现唯一、页级判据前置消费；② 损坏页在该页型的正确行为；③ **`get_slot` 的 `None` 语义不改**（改它会把 abort 变成静默跳过）⇒ 本 Cycle 的写侧**沿用同一形态**，不触碰 `get_slot`。
- Excluded scope:
  - **读路径**（T1 / T3 / T17）—— 已交付并被 Plan Review 接受；本 Cycle **不改**读侧的任何逐点语义；
  - 目录页组的**读**点（`src/storage/catalog.rs` 的 5 处读点）与目录损坏 ⇒ 该库 SHALL NOT 打开的语义 —— 归 **T4**（Iteration 004）；
  - 恢复面组（`src/wal/recovery.rs` 的读点、按恢复类别分别处置的语义）—— 归 **T5**（Iteration 005）；
  - 索引树页（`src/storage/btree/**`）的取值与判据 —— 既有页级闸门 `ensure_index_page_intact` 已保护，本 Cycle **零 diff**；
  - 写盘失败状态面（`write-failure-state` 域、ISS30 / ISS31）—— 归 Iteration 007；本 Cycle 只保证**不与之冲突**；
  - 损坏页的修复 / 抢救、`rtsql check` 的诊断分级、dump / restore 合同面、磁盘格式；
  - 全量回归（整个 change 的全量预算在 Iteration 009 / T16）。

**Objective**

写路径对**页头槽数越界**与**槽负载越界 / 载荷不足**两类页损坏都给出**点名该页**的明确错误：不整数下溢、不越界切片、不越界写、不 panic、不 abort。写入失败 ⇒ **语句失败**（会话事务回滚），**不得**被当作「该页已满 ⇒ 换新页继续」而静默继续，**不得**被折叠成「行 / 槽不存在」，**不得**静默改写相邻字节。判据与错误形态与读侧**同源**；未损坏路径的行为**零变化**。

**Background**

- 缺陷来源（需求来源）：**ISS37** 台账（`.agents/issues/ISS37-write-path-slot-count-underflow.md`）—— 写路径页头槽数下溢（`add_slot_inner` / `free_space` 的 `PAGE_SIZE - slot_count * Slot::SIZE` 无下溢保护）；**ISS38** 台账（`.agents/issues/ISS38-write-path-slot-payload-offset-unbounded.md`）—— 写路径槽负载偏移无界写（`data_page.rs` 的版本头原地写无边界检查）。两份台账由 Recorder 建档，**处置指针字段归 Recorder**，本 Cycle **不改台账**。
- 登记来源：Iteration 002 / Cycle 000 的 Act Response「Experience Candidates → Issue 候选 1 / 2」（Act 只报告不登记；Recorder 已据此建档 ISS37 / ISS38）⇒ 本 Cycle 是该候选在本 change 内的**承载单元**。
- 方向已定（goal 指令授权 + D12 的形态一致性原则）：写侧**复用**读侧同一判据与同一错误形态；写路径的**失败语义**由本 Plan Context 判定（见「Implementation Guidance」第 3 条与 Acceptance A13）—— 读路径是「查询失败 ⇒ 错误返回」，写路径是「**写入失败 ⇒ 事务失败 / 回滚**」。
- 本 Cycle 为何排在 Iteration 002 之后：Iteration 002 冻结了页级判据的**唯一实现**与阈值（`slot_count > MAX_PAGE_SLOTS`）⇒ 写侧直接问它，不另立阈值 ⇒ 两侧判据不会漂移。
- 本 Cycle 为何排在 Iteration 004（目录页组）**之前**：两者**同改 `src/storage/catalog.rs` 的写入点**（`:707` 目录页链追加、`:924` 目录行原地改写）⇒ 写路径先行落地，目录组随后在**写路径已冻结**的形状上改读点，避免两个 Iteration 改同一批目录写入点。
- 本 Cycle 为何排在 Iteration 005（恢复面组）**之前**：恢复重放经 `src/wal/recovery.rs:1519` 到达 `data_page.rs:272` 的无界写；写路径未收口时，T5 的「恢复面有界」保证是**空**的 ⇒ 写路径必须先行。

**Investigation Facts**

- Starting point:
  - 工作区 HEAD `e9cb9d3`；前置状态 = Iteration 001 / Cycle 000（T3）**已被 Plan Review 接受**；Iteration 002 / Cycle 000（T17 / ISS35）**已被 Plan Review 接受**（本 change 的 Plan Review，2026-10-25）；本 Cycle 是 Iteration Map 中下一待执行 Iteration。
  - 变更前的验证结果（沿用，未失效）：Iteration 002 / Cycle 000 的 Act Response 记录 GREEN `cargo test --test data_page_bounded_read_test` 8 passed / 0 failed、`cargo test --lib` 420 passed / 0 failed、定向回归 20 目标 2 批串行 354 passed / 0 failed（来源：Iteration 002 / Cycle 000 的 Act Response「Verification Evidence」第 2–5 项）。本 Cycle 改动 `src/storage/page_format/slotted_page.rs` / `src/storage/data_page.rs` / `src/storage/catalog.rs` ⇒ **覆盖到这三个文件的既有结论失效**（须重跑）；覆盖到 `src/storage/btree/**` / `src/wal/**` 的结论不受影响（本 Cycle 对这两个目录**零 diff**）。
  - 已知限制（本次调查确认的现状，见 Current-State Evidence）：写路径的槽计数与槽负载**无任何有界检查**；「页满 ⇒ 换新页继续」的既有分支把**任何**落位失败都当作页满。
- Current-State Evidence（本 Cycle 已逐行核对的事实，供 Act 直接使用；行号为**当前工作区**）：
  - **页格式层的写入原语**（`src/storage/page_format/slotted_page.rs`）：

    | # | 符号 / 位置 | 现状（实测） | 危险形态 |
    |---|---|---|---|
    | 1 | `MAX_PAGE_SLOTS`（`:21`） | `= (Page::PAGE_SIZE(4096) - SlottedPageHeader::SIZE(16)) / Slot::SIZE(6)` = **680**，**全项目唯一定义** | — |
    | 2 | `SlottedPageHeader::slot_count_exceeds_capacity`（`:89-91`） | `self.slot_count as usize > MAX_PAGE_SLOTS` ⇒ `slot_count ≥ 681` 判页级损坏；**唯一判定实现**（只读 / 可写委托 `:121-123` / `:217-219`） | — |
    | 3 | `add_slot_inner`（`:288`） | `:295` `let free_space_end = Page::PAGE_SIZE - self.slot_count() * Slot::SIZE;` ⇒ `slot_count ≥ 683` 时 `4096 - 683*6` **整数下溢**（debug 构建 panic；release 构建回绕成巨大值）⇒ 随后的 `:302`（负载写）、`:306`（`slot_start` 同样下溢）、`:315-316`（槽目录写）越界 | 写入时 panic / abort，或越界写 |
    | 4 | `free_space`（`:405`） | `:407` 同形减法下溢；`:408` `free_space_end - free_space_start` 在 `free_space_offset > free_space_end` 时**再**下溢 | 索引树侧读该值时 panic |
    | 5 | `delete_slot`（`:340`） | `:349` `PAGE_SIZE - (i+2)*6`、`:350` `PAGE_SIZE - (i+1)*6`、`:357` `PAGE_SIZE - count*6` 三处同形减法 ⇒ `slot_count ≥ 683` 时下溢 | 删槽时 panic / abort |
    | 6 | `delete_slot_by_logical_id`（`:370`） | 先调 `get_slot_by_logical_id` → `get_slot`（`:225`；`:231` 下溢、`:232` 越界切片）⇒ 损坏页上**先在取槽处** abort | 取槽时 panic / abort |
    | 7 | `update_slot_in_place`（`:384`） | `:385-387` 取槽（同 6）；`:389` 等长校验；`:397-399` `self.page.data[start..end].copy_from_slice(data)` ⇒ **无边界检查**（`start = slot.offset`，`end = start + slot.length`） | 越界写 / 覆盖相邻字节 |
    | 8 | `slot_payload_in_bounds`（`SlottedPageRef` 私有，`:180-183`） | `start >= SlottedPageHeader::SIZE && end <= slot_area_start`，`slot_area_start = PAGE_SIZE.saturating_sub(slot_count * Slot::SIZE)` ⇒ **读侧**已有的判界原语，但**私有**、写侧无法复用 | — |

  - **写路径的槽负载偏移无界写**（`src/storage/data_page.rs`）：
    - `update_version_header_in_data_page`（`:249-274`）：`:259` 已有 T17 的**页级判据**（取槽之前）⇒ 页头损坏已被拦；`:267` 取槽；`:269` `let slot_offset = slot.offset as usize;`（**无校验**）；`:272` `page.data[slot_offset..slot_offset + VersionHeader::SIZE].copy_from_slice(&header_bytes)` ⇒ **无边界检查**的写，`VersionHeader::SIZE = 22`（`src/transaction/version_chain.rs:119`）；**未校验 `slot.length >= 22`** ⇒ 负载短于 22 时这 22 字节**越过该行自身负载**、覆盖相邻字节（静默错写）。
  - **生产触点**（`grep` 实测；括号内为行号）：

    | 原语 / 写点 | 位置 | 触达路径 |
    |---|---|---|
    | `add_slot` | `src/storage/data_page.rs:99` | INSERT 落**尾页**（`insert_tuple_to_data_page`） |
    | `add_slot` | `src/storage/data_page.rs:119` | INSERT：尾页已满 ⇒ **新页落位** |
    | `add_slot` | `src/storage/catalog.rs:707` | 目录页链追加（`append_to_chain`） |
    | `add_slot` | `src/storage/catalog.rs:732` | 目录页链追加 ⇒ 新页落位 |
    | `add_slot_with_logical_id` | `src/wal/recovery.rs:207` | 恢复面按 id 强制落位 |
    | `delete_slot_by_logical_id` | `src/storage/data_page.rs:327` | DELETE 删槽（`delete_tuple_from_data_page`） |
    | `delete_slot_by_logical_id` | `src/storage/catalog.rs:824` / `:987` | 目录行删除 |
    | `update_slot_in_place` | `src/storage/catalog.rs:924` | 目录行**等长原地改写**（`update_field_in_chain`） |
    | 版本头无界写 | `src/storage/data_page.rs:272` | commit（`src/storage/buffer_pool.rs:366`）、abort（`src/transaction/manager.rs:514` / `:525`）、DELETE（`src/executor/delete.rs:216`）、UPSERT（`src/executor/upsert.rs:688`）、恢复面（`src/wal/recovery.rs:1519`）、`src/storage/data/table_manager.rs:1116` |

  - **「页满 ⇒ 换页继续」的既有语义（写路径失败语义的关键事实）**：
    - `src/storage/data_page.rs:92-140`：`:102` `match add_result { … Err(_) => { allocate_page(); … 新页落位 … } }` ⇒ **任何** `add_slot` 失败都被当作「尾页已满」；`:122-134` 新页落位再失败 ⇒ `StorageError::PageFull`（`:132`）。
    - `src/storage/catalog.rs:703-742`：`:713` `Err(_) => { allocate_page(); … }`（同形）；`:734` `map_err(|_| StorageError::PageFull)`。
    - ⇒ **若损坏页的 `add_slot` 失败被折叠进 `Err(_)`**，写入会**换新页继续**（损坏页被跳过、数据继续落盘）⇒ **静默继续**。这是写路径必须区分「页满」与「页损坏」的根因，也解释了 ISS37 台账「生产触点」的可达性。
  - **既有错误面**：`StorageError::PageFull`（`src/storage/error.rs:108-109`，已存在）；`StorageError::CorruptedDataPage { page_id, detail }`（T17 冻结，Display = `corrupted data page {page_id}: {detail}`）⇒ 本 Cycle **不新增** `StorageError` 变体。
  - **索引树侧（零 diff 目标）**：页级闸门 `ensure_index_page_intact` 在索引树取槽 / 写入前拦截损坏页 ⇒ 索引树写路径**不暴露**；`src/storage/btree/node.rs` 的落位 / 删槽错误映射**统一**为 `.map_err(|e| StorageError::Io(std::io::Error::other(e)))`（`:225` / `:298` / `:322` / `:332` / `:825` / `:931`）⇒ 只要新的写入失败类型实现 `Display` + `Debug` + `Send` + `Sync` + `'static`（即 `std::error::Error`），这些**既有代码逐字不变**即可编译 ⇒ 索引树**零 diff**、既有错误文本不变。
  - **恢复面（零 diff 目标）**：`src/wal/recovery.rs:207` 的错误映射是 `.map_err(|e| format!("add_slot failed: {}", e))`（`String` 通道）⇒ 只要新错误类型实现 `Display`，该处**逐字不变**即可编译 ⇒ `src/wal/**` **零 diff**；`:1519` 走 `update_version_header_in_data_page` ⇒ 被本 Cycle 的版本头有界写**自动覆盖**（这正是「写路径必须排在 T5 之前」的机械原因）。
  - **`free_space()` 的调用点**（`grep` 实测）：`src/storage/btree/node.rs:212` / `:286` / `:402` / `:814` / `:921`（**全部**在索引树页级闸门之后）+ `src/storage/catalog.rs:1934`（`#[cfg(test)]`）+ `src/storage/page_format/slotted_page.rs:482` / `:486` / `:489` / `:613` / `:623`（单元测试）⇒ **无未受保护的生产调用点** ⇒ 其失败通道的选择**不影响生产可达性**。
  - **写盘失败状态面**（`write-failure-state` 域、ISS30 / ISS31）：该域记录的是**刷盘 / 落盘**失败（页内容未能持久化），是**页级、可重试**的暂态；本 Cycle 的页损坏是**写入之前**发现的**确定性、永久**条件 ⇒ 两者**不共用状态、不共用语义**（见 Invariants）。
  - **既有写路径的两条错误文案（须逐字保持）**：`add_slot` 的 `"No enough space in page"`（`slotted_page.rs:298`）；`update_slot_in_place` 的 `"in-place update requires equal length: slot {n} bytes, got {m} bytes"`（`:390-394`）；另有 `"Slot index out of range"`（`:342`）、`"logical_id {id} not found"`（`:373`、`:387`）。
  - **测试入口**：`tests/data_page_bounded_read_test.rs`（T17 已交付：真实文件库 + 真实 `Database::open` + 真实 SQL 入口 + **真实文件字节改写**夹具 + 页头 `slot_count` 的文件偏移算法 `HEADER_SIZE(64) + page_id * PAGE_SIZE(4096) + 1`）；槽 `offset` 字段的页内偏移 = `PAGE_SIZE - Slot::SIZE + 2 = 4092`、槽 `length` 字段的页内偏移 = `4096 - 6 + 4 = 4094`（槽目录自页尾向上生长，槽 0 的项起点 = `4096 - 6`）。
- Code and Critical Path:
  - 入口：`INSERT`（`src/storage/data_page.rs::insert_tuple_to_data_page`）→ `add_slot`（尾页 / 新页）；`UPDATE` / `DELETE` / commit / abort / UPSERT / 恢复重放 → `update_version_header_in_data_page`（版本头原地写）；`DELETE` → `delete_tuple_from_data_page` → `delete_slot_by_logical_id`；目录 DDL → `append_to_chain`（`add_slot`）与 `update_field_in_chain`（`update_slot_in_place`）；恢复重放 → `add_slot_with_logical_id` 与 `update_version_header_in_data_page`。
  - 状态变化与错误传播：页格式层写入原语失败 ⇒ 调用点（`data_page.rs` / `catalog.rs`）⇒ `StorageError` ⇒ 语句错误 ⇒ 会话事务由既有 `rollback_session`（`src/cli/mod.rs:1061-1069`）显式回滚 ⇒ **写入失败 ⇒ 事务失败 / 回滚**。
  - 对调用者 / 被调用者的影响面：页格式层（`slotted_page.rs` 的 5 个写入原语 + 判界原语）、`data_page.rs`（落位 / 版本头写 / 删槽）、`catalog.rs`（目录链追加 / 目录行原地改写）；**不触及** `src/storage/btree/**`、`src/wal/**`、`src/storage/integrity.rs`、读侧逐点语义。

**Implementation Guidance**

1. **判据复用（不得另造一套）**：
   - **页级判据**：直接问 Iteration 002 冻结的 `SlottedPageHeader::slot_count_exceeds_capacity`（唯一实现，阈值 `slot_count > MAX_PAGE_SLOTS`）。写路径**不得**新增第二个容量界限、不得改阈值。
   - **槽级判界**：读侧 `slot_payload_in_bounds`（`slotted_page.rs:180-183`，**私有**）与写侧需要的判界**同源** ⇒ **上提为页格式层的一个共享判界原语**（读侧与写侧共用同一实现），读侧**语义逐字不变**（`start >= SlottedPageHeader::SIZE && end <= slot_area_start`）；写侧在此之上追加**写入长度不得超过该槽自身负载**这一条（`slot.length >= write_len`）⇒ 两条判据（页级 + 槽级）在全项目各只有一处实现。
2. **写入原语的结构化失败（区分「页满」与「页损坏」）**：页格式层的写入原语当前以 `String` 表达失败 ⇒ 调用点**无法**区分「页满（可换页）」与「页损坏（不可重试）」⇒ 引入页格式层的**结构化写入失败类型**（至少可区分：页满 / 页损坏 / 等长不符），使写侧能够：
   - 「页满」⇒ **维持**既有「换新页继续」；
   - 「页损坏」⇒ 以命名错误结束**写入**，**不得**换页继续；
   - 「等长不符」⇒ **维持**既有语义（`update_slot_in_place` 的等长校验）。
   - 既有两条错误文案的**逐字**保持（见 Current-State Evidence 末项）⇒ 索引树侧 `StorageError::Io(std::io::Error::other(e))` 的文本不变 ⇒ 索引树零回归。
   - 新失败类型须实现 `Display` + `Debug` + `Send` + `Sync` + `'static`（即 `std::error::Error`；项目已依赖 `thiserror = "1.0"`）⇒ `src/storage/btree/**` 与 `src/wal/**` 的既有映射**逐字继续编译**。
3. **写路径的失败语义（本次判定）**：
   - 写入命中页损坏 ⇒ **语句失败**，返回**点名该页**的明确错误（`StorageError::CorruptedDataPage { page_id, detail }`；detail 复用既有三个构造器的文案：页头形态 → `corrupted_data_page_slot_count_error`；负载越界 → `corrupted_data_page_slot_error`；负载不足 → `short_data_page_slot_error`）；
   - **不得**把页损坏折叠成「页满」（那会换页继续）；**不得**折叠成「行 / 槽不存在」（那会让 DELETE 报告「行已删除」）；**不得**静默跳过；
   - 判据**前置**于任何字节写入 ⇒ 失败的写入**不产生部分修改**（页内容不被部分改写）；
   - 事务面：语句错误经既有 `?` 传播链上浮 ⇒ 会话事务由既有的 `rollback_session` 显式回滚 ⇒ **写入失败 ⇒ 事务失败 / 回滚**（这是写路径与读路径「查询失败 ⇒ 错误返回」的**实质差别**）；
   - **与 `write-failure-state` 域不冲突**：页损坏错误发生在**写入前、刷盘前** ⇒ **不进入**刷盘路径、**不设置 / 复位**该域的失败状态、也**不得**被该域的失败状态遮蔽；反之刷盘失败**不得**被表述为页损坏。两者是不同事实、不同面、不同可重试性。
4. **共享判界的唯一实现**：读侧 `get_slot_data_checked` 与写侧的判界必须是**同一处实现**（否则两侧阈值漂移）；读侧**行为零变化**（Preserve）。
5. **零 diff 目标**：`src/storage/btree/**` 与 `src/wal/**` 在本 Cycle **零 diff**（靠新失败类型实现 `Display` + `Error`，使既有 `.map_err(|e| StorageError::Io(std::io::Error::other(e)))` 与 `format!("add_slot failed: {}", e)` 逐字继续编译）。若 Act 发现做不到 ⇒ 属**实质偏差**，须回 Plan（**不得**就地扩大 `src/wal/**` / `src/storage/btree/**` 的改动面）。
6. **任务顺序**：**T18 先于 T19**（同 Cycle 内串行）—— T19 的版本头有界写依赖 T18 落定的共享判界原语与页级判据前置。
7. **纳入判断（`delete_slot` 同形减法 / `update_slot_in_place` 同族兄弟点 ⇒ 纳入同一 Iteration 的理由）**：
   - `delete_slot`（`:340`）：与 ISS37 的根因**同形**（页头 `slot_count` 被无保护地参与减法）、同文件、同原语族；且它**有生产触点**（`data_page.rs:327`、`catalog.rs:824` / `:987`）⇒ 修复是**同一个页级判据的前置调用**（一行级成本），排除它会留下一个同规则的已知 abort 原语、并需要在后续 Iteration 再动同一文件 ⇒ **纳入**。
   - `update_slot_in_place`（`:384`）：与 ISS38 **同族**（槽负载写入无边界检查）、是目录行**唯一**的生产原地改写路径（`catalog.rs:924`）；其写入区间 = 该槽自身负载（等长已校验）⇒ 判界**精确**且零歧义；排除它会使「写路径的槽界限」**不完整**（目录写路径仍可越界写）⇒ **纳入**。
   - `free_space`（`:405`）：ISS37 的根因点之一；无错误通道 ⇒ 采取**安全降级**（页头损坏 ⇒ 返回 0 ⇒ 调用方判定「无可用空间」而不会写入）；其**全部**生产调用点在索引树页级闸门之后 ⇒ 该降级在生产中**不可达**，不引入「页满 / 页损坏」混淆 ⇒ **纳入**。

**Task Definitions**

### T18: 写路径的页头槽数越界以点名该页的错误结束，不下溢、不换页继续

- Requirement/Scenario: `page-write-bounded-write` / 「页头声称的槽数超出槽目录容量时写路径不 abort 且不换页继续」
- Depends on: Iteration 002 / Cycle 000（T17 / ISS35）—— 页级判据的唯一实现与阈值已冻结
- Targets:
  - `src/storage/page_format/slotted_page.rs` —— `add_slot_inner`（`:288`）、`free_space`（`:405`）、`delete_slot`（`:340`）、`delete_slot_by_logical_id`（`:370`）、`update_slot_in_place`（`:384`）的**页级判据前置**与**结构化失败**；页格式层的写入失败类型；与读侧共用的槽级判界原语（上提 `slot_payload_in_bounds`，`:180-183`）。
  - `src/storage/page_format/mod.rs` —— 新增类型的重导出（若需要）。
  - `src/storage/data_page.rs` —— `insert_tuple_to_data_page`（`:92-140`）的**「页满 / 页损坏」分流**；`delete_tuple_from_data_page`（`:325-330`）的失败映射（页损坏**不得**被折叠成 `StorageError::SlotNotFound`）。
  - `src/storage/catalog.rs` —— `append_to_chain`（`:703-742`）的**「页满 / 页损坏」分流**。
  - `tests/write_path_slot_bounds_test.rs` —— **新增测试文件**（先行用例 + 未损坏对照）。
- Current behavior: 写路径的 `add_slot_inner:295` / `free_space:407` / `delete_slot:349` / `:350` / `:357` 在页头槽数越界时**整数下溢** ⇒ debug 构建 panic（进程 abort）、release 构建回绕 ⇒ 越界写；`data_page.rs:104` 与 `catalog.rs:713` 的 `Err(_)` 分支把**任何**落位失败当作「页满」⇒ **换新页继续写入**（损坏页被跳过、数据继续落盘）；`data_page.rs:330` 把删槽的任何失败折叠成 `StorageError::SlotNotFound`（⇒ DELETE 报告「行不存在」）。
- Required behavior:
  - 上述 5 个写入原语在**做任何算术 / 写入之前**先问页级判据；页头声称的槽数超出容量 ⇒ 返回**结构化的页损坏失败**（点名该页与页头声称的槽数），**不产生任何下溢 / 越界写 / panic / abort**；
  - `free_space()`（无错误通道）在页头损坏时**降级为安全值 0**（⇒ 调用方判定「无可用空间」而不会写入）；
  - `add_slot` 的失败可被调用方**区分为**「页满」与「页损坏」；
  - `data_page.rs` 与 `catalog.rs` 的既有「页满 ⇒ 换新页继续」路径**仅**在「页满」时触发；「页损坏」⇒ 以 `StorageError::CorruptedDataPage { page_id, detail }` 结束写入并上浮（detail 点名该页与页头声称的槽数）；
  - `delete_slot_by_logical_id` 在损坏页上返回页损坏失败，**不得**被映射成「槽不存在 / 行不存在」；
  - 既有错误文案**逐字**保持：`"No enough space in page"`、`"Slot index out of range"`、`"logical_id {id} not found"`。
- Preserve:
  - `get_slot` / `get_slot_by_logical_id` 的**签名与 `None` 语义**（D12 冻结，**不改**）；
  - `MAX_PAGE_SLOTS` 的**唯一定义**与值 **680**；页级判据的**唯一**实现（`slot_count > MAX_PAGE_SLOTS`）；
  - `StorageError::CorruptedDataPage` 的**变体形状**与文案前缀 `corrupted data page {page_id}: `；**不新增** `StorageError` 变体；既有 `StorageError::PageFull` 语义不变；
  - `data_page.rs` / `catalog.rs` 的「页满 ⇒ 换新页继续」既有行为；
  - 索引树侧（`src/storage/btree/**`）的取值与判据**零变化**、**零 diff**；`src/wal/**` **零 diff**；
  - T3 冻结的**读侧**逐点语义（过短负载逐点行为）**零变化**；读侧判界原语的**语义**逐字不变。
- Forbidden:
  - 改 `get_slot` 的 `None` 语义；新增页级阈值 / 第二个容量界限 / 第二个判据实现；
  - 在 `src/storage/btree/**` 或 `src/wal/**` 改动**任何**代码；
  - 把页损坏折叠成「页满」「行 / 槽不存在」「跳过该行继续」或任何无细节的泛化错误；
  - 改动刷盘失败状态面（`write-failure-state` 域、ISS30 / ISS31）的任何语义；
  - 改动目录页组的**读**点（Iteration 004 的范围）；改动恢复面（Iteration 005 的范围）；
  - 新增命令参数 / 配置项 / 缓存 / 闸门机制 / 判定层 / 身份证明机制（Hash / 指纹 / revision pin / run-id / manifest / 时间证明）；改动磁盘格式；
  - 全量回归。
- First test:
  - 位置：`tests/write_path_slot_bounds_test.rs`（**新增测试文件**；沿用 `tests/data_page_bounded_read_test.rs` 的夹具形态：真实文件库 + 真实 `Database::open` + 真实 SQL 入口 + **真实文件字节改写**，无替身 / 桩）。
  - **夹具 A（页头槽数越界）**：建库 → 建表 `t`（≥ 1 列）→ 插入 ≥ 1 行 ⇒ 读出该表**尾页**的页号（既有目录 / 表元数据入口）⇒ 关库 ⇒ 按文件偏移 `HEADER_SIZE(64) + page_id * PAGE_SIZE(4096) + 1` 把该页页头的 `slot_count` 字段写成 u16 小端的 **700**（> 680）⇒ 重开库 ⇒ 执行 `INSERT INTO t VALUES (…)`。
    ⇒ **预期 RED（变更前）**：**进程 panic**（`attempt to subtract with overflow` @ `src/storage/page_format/slotted_page.rs:295` 的 `Page::PAGE_SIZE - self.slot_count() * Slot::SIZE`）⇒ SIGABRT；**不得**是「插入成功」或「换页继续后插入成功」。
  - **夹具 A'（删槽面）**：同一损坏页 ⇒ 执行 `DELETE FROM t WHERE …`。
    ⇒ **预期 RED（变更前）**：**进程 panic**（取槽处 `slotted_page.rs:231-232` 下溢 / 越界切片）⇒ SIGABRT。
  - **夹具 A''（布尔 / 非静默对照）**：同一损坏页 ⇒ 执行 `SELECT … FROM t`（读侧已在 Iteration 002 被拦，本夹具用于确认写侧修复**不改变**读侧既有错误形态）。
    ⇒ 变更前即 GREEN（点名该页的 `CorruptedDataPage`）⇒ **不作为 RED**，作为回归对照。
  - **夹具的未损坏对照**：同样步骤但不改字节 ⇒ 变更前即 GREEN（INSERT / DELETE 的结果与既有行为一致）。
- GREEN condition:
  - 夹具 A / A' 的进程**不 panic / 不 abort**（测试进程存活），写入以**点名该页**的明确错误结束（错误文案含 `corrupted data page {page_id}` 与页头声称的槽数 `700`）；
  - **不得**出现「INSERT / DELETE 成功」；**不得**出现「换新页后写入成功」（损坏页被跳过 ⇒ 静默继续）；表内行数**不增**；
  - DELETE 面**不得**报告「行不存在 / 槽不存在」；
  - 未损坏对照：INSERT / DELETE 的结果与既有行为一致（零变化）；读侧对照的既有错误形态不变；
  - `cargo test --test write_path_slot_bounds_test` 全部 0 failed。
- Verification:
  - `cargo test --test write_path_slot_bounds_test -- --test-threads=1` ⇒ 全部 0 failed；
  - `cargo test --lib` ⇒ 全部 0 failed（覆盖 `page_format` 与索引树单元测试 ⇒ 索引树零回归）；
  - 定向回归 **20 个既有目标**（与 Iteration 002 的清单一致），按 M19 **多批次串行**纪律拆成 **两批各 10 个目标**（批内 `--test-threads=1`、**批 1 全绿后才启动批 2**）；
  - `cargo fmt --check` 与 `cargo clippy --all-targets` **只核对本 Cycle 触碰的文件**（既有基线：50 处 fmt diff / 9 个既有文件、lib 侧 17 条 clippy error —— 见 `design.md` D13 / ISS36）⇒ 本 Cycle **零新增** finding。
- Stop when: 需要改 `src/storage/btree/**` 或 `src/wal/**`；需要新增 `StorageError` 变体或第二个容量界限；需要改读侧逐点语义；页级判据的形状与 Iteration 002 冻结的不一致；需要改 `VersionHeader::SIZE`；发现存在「负载短于 22 字节的合法槽」（⇒ 回 Plan 判定写侧最小负载）。

### T19: 写路径的槽负载写入有界（版本头原地写与目录行原地改写）

- Requirement/Scenario: `page-write-bounded-write` / 「槽负载越界或过短时写路径不越界写且不静默改写相邻字节」
- Depends on: T18（共享判界原语与页级判据前置已落地）
- Targets:
  - `src/storage/page_format/slotted_page.rs` —— `update_slot_in_place`（`:384`）的**负载有界写**；与读侧共用的槽级判界原语（上提 `slot_payload_in_bounds`，`:180-183`）。
  - `src/storage/data_page.rs` —— `update_version_header_in_data_page`（`:249-274`）的**版本头有界写**（含「写入长度不得超过该槽自身负载」的判定）。
  - `src/storage/catalog.rs` —— `update_field_in_chain`（`:920-931`）的失败映射（页损坏**不得**被折叠成无细节的泛化错误 / 不得被吞掉）。
  - `tests/write_path_slot_bounds_test.rs` —— **同一测试文件**（新增夹具 B / B2 用例，不新增夹具语义）。
- Current behavior: `data_page.rs:272` 的 `page.data[slot_offset..slot_offset + 22]` **无边界检查** ⇒ 槽 `offset` 越界 ⇒ 越界切片 panic；`update_slot_in_place`（`slotted_page.rs:397-399`）的 `page.data[start..end]` **无边界检查** ⇒ 同样的越界写；**未校验 `slot.length >= 22`** ⇒ 负载短于 22 时版本头写**越过该行自身负载**、覆盖相邻字节（**静默错写**，不 panic）。
- Required behavior:
  - 版本头写入的目标区间 `[slot.offset, slot.offset + 22)` SHALL 落在该页的数据区内（不触及页头与槽目录），且 **SHALL NOT** 超出该槽自身的负载（⇒ `slot.length >= 22`）；不满足 ⇒ 以**点名该页与该槽**（含区间 / 长度）的明确错误结束，**不产生任何字节写入**；
  - `update_slot_in_place` 的写入区间 = 该槽自身负载（等长已校验）⇒ SHALL 落在页数据区内；不满足 ⇒ 同上；
  - 写侧对「负载不足」的形态 = **点名该页与该槽**的明确错误（复用 `short_data_page_slot_error` 的文案形态）⇒ 与读侧 `data_page.rs:315-318` 的「过短负载 = 非溢出定位」**并存且不冲突**（读侧逐点语义零变化；写侧独立判定）；
  - 目录行原地改写（`catalog.rs:924`）在槽损坏时以**点名该页**的错误结束，**不得**被折叠成「表不存在」或无细节的泛化错误。
- Preserve:
  - T3 冻结的**读侧**逐点语义（含 `data_page.rs:315-318` 的「过短负载 = 非溢出定位」）；
  - `VersionHeader::SIZE = 22`（`src/transaction/version_chain.rs:119`）**不改**；`VersionHeader` 的字节布局**不改**；
  - `StorageError::CorruptedDataPage` 的变体形状与文案前缀；**不新增** `StorageError` 变体；
  - 目录行「过短负载」的既有 `StorageError::Internal` 语义（**读**侧，Iteration 004 处置）**不改**；
  - `src/storage/btree/**` 与 `src/wal/**` **零 diff**。
- Forbidden:
  - 改读侧的过短负载语义；改 `VersionHeader::SIZE` 或版本头字节布局；改目录页组的**读**点（Iteration 004 的范围）；改 `src/wal/**` / `src/storage/btree/**`；新增错误变体 / 判定层 / 身份证明机制；全量回归。
- First test:
  - 位置：`tests/write_path_slot_bounds_test.rs`（同一文件，新增用例）。
  - **夹具 B（槽 offset 越界）**：建库 → 建表 `t` → 插入 ≥ 1 行 ⇒ 读出该表数据页页号 ⇒ 关库 ⇒ 按文件偏移 `HEADER_SIZE(64) + page_id * PAGE_SIZE(4096) + 4092`（槽 0 的 `offset` 字段：页内起点 `4096 - 6`，字段偏移 `+2`）把该槽的 `offset` 字段写成 u16 小端的 **65535** ⇒ 重开库 ⇒ 执行 `UPDATE t SET …`（触发版本头原地改写）。
    ⇒ **预期 RED（变更前）**：**进程 panic**（`range start index 65535 out of range for slice of length 4096` @ `src/storage/data_page.rs:272`）⇒ SIGABRT。
  - **夹具 B2（负载不足 / 静默改写相邻字节）**：同一页 ⇒ 按文件偏移 `HEADER_SIZE(64) + page_id * PAGE_SIZE(4096) + 4094`（槽 0 的 `length` 字段：页内起点 `4096 - 6`，字段偏移 `+4`）把该槽的 `length` 字段写成 u16 小端的 **4**（< 22）⇒ 重开库 ⇒ 执行 `UPDATE t SET …`。
    ⇒ **预期 RED（变更前）**：进程**存活**但**静默越界写**（22 字节版本头越过该行 4 字节的负载、覆盖相邻字节）⇒ RED 表现为**断言失败**（断言「以明确错误结束」与「相邻字节逐字未变」两条**均**不成立），**断言不得放宽**。
  - **未损坏对照**：同样步骤但不改字节 ⇒ 变更前即 GREEN（UPDATE 的结果与既有行为一致）。
- GREEN condition:
  - 夹具 B：进程**不 panic / 不 abort**，写入以**点名该页与该槽**的明确错误结束（文案含 `corrupted data page {page_id}` 与该槽的区间）；
  - 夹具 B2：进程存活，写入以**点名该页与该槽**的**负载不足**错误结束；且**被覆盖范围内的字节逐字未变**（不得静默改写相邻字节）；
  - 未损坏对照：UPDATE 的结果与既有行为一致（零变化）；读侧既有语义未变；
  - `cargo test --test write_path_slot_bounds_test` 全部 0 failed。
- Verification: 同 T18（同一测试目标 + `cargo test --lib` + 20 目标两批串行 + fmt / clippy 只核对触碰文件）。
- Stop when: 需要改读侧语义 / `VersionHeader::SIZE` / 版本头布局 / 目录读点 / `src/wal/**` / `src/storage/btree/**`；需要新增 `StorageError` 变体。

**Invariants**

- **判据唯一性**：页级判据全项目只有一处实现（`slot_count > MAX_PAGE_SLOTS` ⇒ 681）；槽级判界全项目只有一处实现（读侧与写侧共用）；槽目录容量界限全项目只有一处定义（`MAX_PAGE_SLOTS = 680`）。
- **错误形态统一**：写路径的页损坏错误与读路径**同名同形**（`StorageError::CorruptedDataPage { page_id, detail }`，Display 前缀 `corrupted data page {page_id}: `）；detail 点名该页，并在负载形态时同时点名该槽与区间。
- **写路径的失败语义**：写入失败 ⇒ **语句失败**（会话事务由既有 `rollback_session` 回滚）；页损坏**不得**换页继续、**不得**静默跳过、**不得**被表述为页满 / 行不存在；失败的写入**不产生部分修改**（判据前置于任何字节写）。
- **与 `write-failure-state` 域不冲突**：页损坏（写入前、确定性、永久）≠ 刷盘失败（刷盘面、可重试、页级暂态）；两者不共用状态、不互相遮蔽、不互相表述。
- **索引树与恢复面零 diff**：`src/storage/btree/**` 与 `src/wal/**` 在本 Cycle 零改动；索引树既有判据与取值零变化；既有错误文本不变。
- **读侧零变化**：T3 / T17 冻结的读侧逐点语义（含过短负载逐点行为、布尔辅助的安全值降级）零变化。
- **M19 多批次串行纪律**：全量回归不在本 Cycle（归 Iteration 009 / T16）；定向回归按「两批各 10、批内 `--test-threads=1`、批间串行」执行。

**Non-goals**

- 读路径（已交付并接受）；目录页组的**读**点与「目录损坏 ⇒ 该库 SHALL NOT 打开」（Iteration 004）；恢复面组的读点与按恢复类别分别处置的语义（Iteration 005）；写盘失败状态面（Iteration 007）；索引树侧的取值与判据；损坏页的修复 / 抢救；`rtsql check` 的诊断分级；dump / restore 合同面；磁盘格式；全量回归。

**Acceptance**

| # | 验收条件 | 需求 / 场景 | 设计 | 任务 | 代码位置 | 测试 |
|---|---|---|---|---|---|---|
| A12 | 写路径在页头槽数越界时**不 panic / 不 abort**、不下溢、不越界写；以**点名该页与页头声称的槽数**的明确错误结束 | `page-write-bounded-write` / 「页头声称的槽数超出槽目录容量时写路径不 abort 且不换页继续」 | D15 | T18 | `src/storage/page_format/slotted_page.rs`（`add_slot_inner` / `free_space` / `delete_slot` / `delete_slot_by_logical_id` / `update_slot_in_place`）、`src/storage/data_page.rs`、`src/storage/catalog.rs` | `tests/write_path_slot_bounds_test.rs`（夹具 A / A'） |
| A13 | 写入失败 ⇒ **语句失败 / 事务回滚**；页损坏**不换页继续**、**不折叠成页满 / 行不存在**、**不静默跳过** | 同上场景的失败语义 + 「写入失败使语句失败并回滚事务」 | D15 | T18 | `src/storage/data_page.rs:92-140` / `:325-330`、`src/storage/catalog.rs:703-742`、`src/cli/mod.rs:1061-1069`（既有回滚路径，不改） | 同上 + 会话事务面用例 |
| A14 | 槽负载越界 / 过短时写路径**不越界写**、**不静默改写相邻字节**；以**点名该页与该槽**的明确错误结束 | `page-write-bounded-write` / 「槽负载越界或过短时写路径不越界写且不静默改写相邻字节」 | D15 | T19 | `src/storage/data_page.rs:249-274`、`src/storage/page_format/slotted_page.rs:384`、`src/storage/catalog.rs:920-931` | 同上（夹具 B / B2） |
| A15 | 未损坏路径行为**零变化**；判据**唯一**；索引树侧与 `src/wal/**` **零 diff**；**不新增**错误变体 | `page-write-bounded-write` / 「未损坏路径的行为零变化」+ 本文件 Invariants | D12 / D15 | T18 / T19 | `src/storage/page_format/slotted_page.rs`、`src/storage/data_page.rs`、`src/storage/catalog.rs`、`src/storage/error.rs`（只读核对） | 未损坏对照 + 定向回归 20 目标 + `cargo test --lib` |

**Verification**

- 逐 scenario 的最简直接判定（行为类 scenario 走**真实入口**：真实建库 → 真实写库 → 真实改坏磁盘字节 → 真实重开 → 真实执行语句）：
  1. 「页头槽数越界 ⇒ 写路径命名错误且不换页继续」：`cargo test --test write_path_slot_bounds_test -- --test-threads=1` 的夹具 A / A' ⇒ 通过依据 = 测试进程存活；错误文案含 `corrupted data page {page_id}` 与 `slot count 700`；该表**行数不增**（未换页继续写入）。
  2. 「槽负载越界 / 过短 ⇒ 不越界写」：夹具 B / B2 ⇒ 通过依据 = 测试进程存活；点名该页与该槽的错误；夹具 B2 的相邻字节**逐字未变**。
  3. 「写入失败 ⇒ 语句失败 / 回滚」：会话事务面用例（事务开启 ⇒ 写入命中损坏页 ⇒ 语句失败 ⇒ 事务回滚）⇒ 通过依据 = 语句以非零退出码失败，且该事务内的先前写入随事务回滚（未生效）。
  4. 「未损坏路径零变化」：未损坏对照 + `cargo test --lib` + 定向回归 20 目标（两批各 10、批内 `--test-threads=1`、批间串行）⇒ 全部 0 failed。
- **不建判定层**：判定只落在 `cargo test` 的断言与退出码；不新增脚本 / 封装命令 / 专用参数；不使用 Hash / 指纹 / revision pin / run-id / manifest / 时间证明。
- **验证边界**：本 Cycle **不跑全量回归**（整个 change 的收尾全量归 Iteration 009 / T16）。

**Gate 2 Readiness**

- Step 5 逐项复核：TBD / TODO = 无；需求裁剪 = 无（delta spec `page-write-bounded-write` 已写细 1 个 Requirement + 4 个 Scenario，`Simplified` = 0）；tasks / specs / design 一致 = 一致（T18 ↔ ISS37 台账、T19 ↔ ISS38 台账、delta 场景 ↔ Acceptance A12–A15、D15 ↔ 承载形态与依赖顺序）；Current-State Evidence 全部来自本 Cycle 的逐行核对（5 个写入原语的下溢行号与算术、6 类生产触点、「页满 ⇒ 换页继续」的两处既有分支、索引树与恢复面的零 diff 依据、既有错误文案逐字、夹具的文件偏移算术）；每个 task 的执行依据完整（Targets / Current / Required / Preserve / Forbidden / First test / GREEN / Verification / Stop when 齐备）；调用者、影响范围、测试入口已定位（页格式层 5 个原语 + 8 个生产触点 + 2 个既有分流分支 + 1 个新测试文件）；未修改无关范围（Forbidden 已列明 `btree/**` / `wal/**` / 读侧逐点语义 / 目录读点 / 写盘失败面）；Iteration Plan 覆盖全部任务且高开销验证预算明确（全量归 Iteration 009 / T16）；**Act 只需读本 Cycle 文件即可建立第一个先行测试并执行**（字节算术、页号取法、预期 RED 形态、错误文案形态、命令与批次均已写明，无需回读前序 Cycle / 其它规划资料 / 重新调查）；非实质未知项已留在 Risks and Notes；验证直接证明目标行为，行为类 scenario 走真实入口，无身份证明机制与判定层。⇒ **全部 PASS**。
- RTM：`page-write-bounded-write` Requirement + 「页头声称的槽数超出槽目录容量时写路径不 abort 且不换页继续」+ 「槽负载越界或过短时写路径不越界写且不静默改写相邻字节」+ 「写入失败使语句失败并回滚事务」+ 「未损坏路径的行为零变化」⇒ T18 / T19 ⇒ Covered（无 `Missing`、无 `Simplified`）。
- 一致性：本 Cycle 与 change 的 `tasks.md`（T18 / T19 归属 Iteration 003）、`specs/page-write-bounded-write/spec.md`、`design.md` D15、ISS37 / ISS38 台账的事实一致。
- 设计闭合：写入原语的失败形态（页满 / 页损坏 / 等长不符）、判据的共用方式、写路径的失败语义、与 `write-failure-state` 域的边界、`delete_slot` / `update_slot_in_place` / `free_space` 的纳入理由均已定稿 ⇒ 无 TBD。
- Persisted Evidence：`none`（见下）。
- 用户批准：本 goal 指令（「按照流程把最新规划的这一批ms做完……中途发现并登记的issue直接在那个change分配iter进行解决……开始吧」）构成 Gate 1 / Gate 2 的批准授权；**批准不降低判据**，上述各项仍逐条检查并记录。

**Cycle RTM**

| Requirement / Scenario | Task | First Test（计划名） | Status |
|---|---|---|---|
| PWBW Scenario「页头声称的槽数超出槽目录容量时写路径不 abort 且不换页继续」 | T18 | `tests/write_path_slot_bounds_test.rs::…`（计划名：`corrupted_page_slot_count_makes_writes_fail_with_a_named_error`；`…_makes_deletes_fail_…`；Act 可改名并记录实际名） | Covered |
| PWBW Scenario「写入失败使语句失败并回滚事务」 | T18 | 同上 + 会话事务面用例 | Covered |
| PWBW Scenario「槽负载越界或过短时写路径不越界写且不静默改写相邻字节」 | T19 | 同上（计划名：`corrupted_slot_offset_makes_updates_fail_with_a_named_error`；`short_slot_payload_makes_updates_fail_without_touching_neighbours`） | Covered |
| PWBW Scenario「未损坏路径的行为零变化」 | T18 / T19 | 未损坏对照 + 定向回归 20 目标 + `cargo test --lib` | Covered |

**Persisted Evidence**

- Mode: none

全部结论可低成本重跑（1 个新增测试目标 + `cargo test --lib` + 20 个既有回归目标），命令与实际结果写入 Act Response 即可承载；逐条回答四个必要性问题：① 支持哪个 Acceptance —— A12–A15 全部由上述命令直接判定；② 为什么 Act Response 不够 —— Act Response 的「Verification Evidence」段可完整承载命令与实际结果（与 Iteration 001 / 002 的既有做法一致）；③ 为什么无法低成本重跑 —— **不成立**（全部为 `cargo test` 目标，可低成本重跑）；④ 缺少它会阻止哪个决定 —— 无（Gate 4 的完成判定与 Plan Review 的验收判定均不依赖不可重跑的证据）⇒ **不创建** `evidence/` 目录，不预建占位目录。

**Risks and Notes**

- **既有基线条件（非本 Cycle 引入，不纳入本 change）**：项目无 `rust-toolchain.toml`；HEAD（`e9cb9d3`）上 `cargo fmt --check` 报 **50 处 diff 分布在 9 个既有文件**，`cargo clippy --all-targets -- -D warnings` 报 **17 条 lib 错误**（deny 模式下 lib 即中止）、非 deny 模式 **32 条 warning** ⇒ 基线在 HEAD 即红。本 Cycle **只核对本 Cycle 触碰的文件**零新增 finding，**不重排**无关文件 ⇒ 该排除已记录在 `design.md` D13（ISS36）。
- **写入失败类型的形态**：Act 可自行决定类型名与变体划分，但**必须**满足：① 能区分「页满 / 页损坏 / 等长不符」；② 既有两条错误文案逐字保持（`"No enough space in page"`、`"in-place update requires equal length: slot {n} bytes, got {m} bytes"`）；③ 实现 `Display` + `Debug` + `Send` + `Sync` + `'static`（项目已依赖 `thiserror = "1.0"`）⇒ `src/storage/btree/**` 与 `src/wal/**` 的既有映射逐字继续编译；④ **不新增** `StorageError` 变体（页损坏错误复用 `CorruptedDataPage`）。
- **`free_space()` 的安全降级**：页头损坏 ⇒ 返回 0。全部生产调用点在索引树页级闸门之后 ⇒ 生产中不可达；若 Act 发现该降级与索引树既有语义冲突 ⇒ **回 Plan**（实质问题）。`src/storage/catalog.rs:1934` 的 `#[cfg(test)]` 用法与 `slotted_page.rs` 的单元测试期望可能需局部适配（属非实质局部差异，记录即可）。
- **写侧最小负载判定（`slot.length >= 22`）**：正常数据页槽的负载恒 ≥ 22（行首是版本头）⇒ 无误报风险；若 Act 发现存在负载 < 22 的**合法**槽 ⇒ **回 Plan**（该情形的写侧语义需要新的裁定）。
- **跨 Iteration 协调（非返工触发）**：Iteration 004（T4）将为目录**读**路径引入 `StorageError::CorruptedCatalogPage`；本 Cycle 的**写**路径统一使用 `CorruptedDataPage`（页格式层的写入原语不感知页类型）。T4 若要统一目录**写**路径的错误变体，**必须**在目录调用点做映射并保留 detail，**不得**改写路径原语 ⇒ 不构成本 Cycle 的返工触发；反之本 Cycle 不得预先引入 `CorruptedCatalogPage`（那是 T4 的设计决定）。
- **`src/wal/recovery.rs:1519` 的覆盖说明**：该点调用 `update_version_header_in_data_page` ⇒ 本 Cycle 的版本头有界写**自动**覆盖它（这是「写路径必须排在 T5 之前」的机械原因，也是 T5 的「恢复面有界」保证得以成立的前提）。本 Cycle 对 `src/wal/**` **零 diff**；T5 不得重复实现同一有界写。
- **ISS37 / ISS38 台账的处置指针**：该字段归 Recorder 所有，本 Cycle **不改台账**；承载分配记录在 change 的 `tasks.md` 与 `design.md` D15，由上层转交 Recorder 回填。
- **跳过项**：本 Cycle 不跑全量回归（change 预算归 Iteration 009 / T16，非跳过而是预算安排）；不做全仓 fmt / clippy 修复（既有基线条件）；不改目录页组的读点（Iteration 004）；不改恢复面（Iteration 005）；不改索引树判据；不改写盘失败状态面（Iteration 007）。

**后续任务边界**

- **授权边界**：无（T18 / T19 在 goal 指令授权范围内）。
- **能力边界**：无。
- **停止边界**：本 Cycle 完成且 Plan Review 判定 `accepted` ⇒ 展开 Iteration 004（T4 目录页组，`iterations/004-catalog-bounded-read/000-initial.md` —— **已存在**，Plan Context `ready`）；Act 不得自行创建下一 Cycle / 下一 Iteration。
- **不在本 Cycle、需在后续处理的事项**：T4（目录页组，Iteration 004）；T5（恢复面组，Iteration 005）；ISS25 / ISS28 / ISS29（Iteration 006）；ISS30 / ISS31（Iteration 007）；ISS32 / ISS33（Iteration 008）；ISS34 / I045 / I064 与收尾全量（Iteration 009）。
- **交 Maintainer / Recorder 的事项**：ISS37 / ISS38 台账的「处置指针」由 Recorder 回填（本 Cycle 只报告承载分配，不改台账）；本 Cycle 不登记 Issue、不改全局状态（`.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆 / references 均不动）、不合并规格。

## Act Response

- Status: reported

### 接手时的实际状态

- 工作区核对（`git status --short` + `git diff --stat`）：`src/storage/catalog.rs` **零 diff**；`tests/write_path_slot_bounds_test.rs` **不存在**；`src/storage/page_format/slotted_page.rs` 与 `src/storage/data_page.rs` 的既有 diff **全部来自已接受的 Iteration 001 / 002**（页级判据、逐槽判据、`CorruptedDataPage` 变体），**不含本 Cycle 的任何产物**。
- ⇒ 本 Cycle **从零开始**，无半成品、无他人未完成改动需要接手；未回退任何既有改动。
- 上一轮同名 Act 在**调查阶段**被会话级中断取消（无产品代码改动留存）。

### Implemented（实际改动）

**页格式层：结构化写入失败 + 读侧 / 写侧共用的槽级判界原语**

- `src/storage/page_format/slotted_page.rs::PageWriteError`（`:28-81`，**新增**）：页格式层的**结构化**写入失败——`PageFull`（可换页继续）/ `SlotCountExceedsCapacity`（页级损坏）/ `SlotPayloadOutOfBounds`（负载越界）/ `LengthMismatch`（等长不符）/ `SlotIndexOutOfRange` / `LogicalIdNotFound`。`Display` + `std::error::Error`。
  - 既有错误文案**逐字**保持：`"No enough space in page"`、`"Slot index out of range"`、`"logical_id {id} not found"`、`"in-place update requires equal length: slot {n} bytes, got {m} bytes"` ⇒ 索引树侧（`StorageError::Io(std::io::Error::other(e))`）与恢复面（`format!("add_slot failed: {}", e)`）的既有映射**逐字继续编译、零 diff**。
  - 引入理由：既有原语以 `String` 表达失败 ⇒ 调用点**无法**区分「页满」与「页损坏」⇒ 损坏页上的落位失败被当作「页满」⇒ 换新页继续写入。
- `src/storage/page_format/slotted_page.rs::slot_payload_in_bounds(slot_count, start, end)`（`:101-104`，**新增**）：槽负载界线的**唯一**实现——由读侧私有方法 `SlottedPageRef::slot_payload_in_bounds`（原 `:180-183`）**上提**为自由函数，语义**逐字不变**（`start >= SlottedPageHeader::SIZE && end <= PAGE_SIZE.saturating_sub(slot_count * Slot::SIZE)`）。
  - 读侧：`SlottedPageRef::get_slot_data_checked`（`:253-261`）改为调用它（行为逐字不变）。
  - 写侧：`SlottedPage::update_slot_in_place`（`:494-533`）与 `data_page.rs::update_version_header_in_data_page`（`:292-349`）共用它 ⇒ 两侧界限口径不会漂移。
- `src/storage/page_format/mod.rs:9-12`：重导出 `PageWriteError` 与 `slot_payload_in_bounds`。
- **5 个写入原语在做任何算术 / 写入之前先问页级判据**（`slot_count > MAX_PAGE_SLOTS`）：
  - `SlottedPage::add_slot_inner`（`:365-392`，页级判据在 `:374-379`）、`delete_slot`（`:430-441`）、`delete_slot_by_logical_id`（`:469-480`）、`update_slot_in_place`（`:494-503`）；`free_space`（`:535-544`，无错误通道 ⇒ 页头损坏时**安全降级为 0**，且 `free_space_end - free_space_start` 改为 `saturating_sub`）。
- 返回类型由 `Result<_, String>` 改为 `Result<_, PageWriteError>`：`add_slot`（`:342`）、`add_slot_with_logical_id`（`:353`）、`delete_slot`（`:430`）、`delete_slot_by_logical_id`（`:469`）、`update_slot_in_place`（`:494`）。

**存储层：页满 / 页损坏的分流 + 唯一的写入失败映射点**

- `src/storage/data_page.rs::page_write_error_to_storage_error(page_id, err)`（`:68-98`，**新增**）：页格式层写入失败 → 存储层错误的**唯一**映射点。页满 ⇒ `StorageError::PageFull`；页损坏 ⇒ `StorageError::CorruptedDataPage { page_id, detail }`（detail **复用既有构造器**：`corrupted_data_page_slot_count_error` / `corrupted_data_page_slot_error`）；其余 ⇒ `StorageError::Internal`（既有泛化文案形态逐字保留）。经 `src/storage/mod.rs:33` 重导出。
- `src/storage/data_page.rs::write_tuple_to_data_page`（`:103-206`）：**「页满 ⇒ 换新页继续」分支仅在页满时触发**——原 `Err(_)` 改为 `Err(PageWriteError::PageFull)` 走换页分支，`Err(e)`（页损坏）走**新分支**：以点名该页的错误结束写入，**不换页继续**、**不分配新页**、**不改写损坏页**（判据前置于任何字节写 ⇒ 该页未被部分改写），并**释放本次调用已分配的溢出链**（best-effort，与既有页满失败路径同处理）。
- `src/storage/data_page.rs::update_version_header_in_data_page`（`:292-349`）：**版本头原地写的有界写**——判据前置于任何字节写入：① 槽负载区间必须完整落在页的数据区内（共用 `slot_payload_in_bounds`）⇒ 否则 `corrupted_data_page_slot_error`（点名该页与该槽的区间）；② `slot.length >= VersionHeader::SIZE`（22）⇒ 否则 `short_data_page_slot_error`（点名该页与该槽的负载不足）；两者皆不产生任何字节写入。
- `src/storage/data_page.rs::delete_tuple_from_data_page`（`:351-...`）：删槽失败**不得**被折叠成「行不存在」——「槽不存在 / 下标越界」**原样**映射为 `StorageError::SlotNotFound(row_id)`（既有语义不变），页损坏错误**原样上浮**为点名该页的 `CorruptedDataPage`。
- `src/storage/catalog.rs::append_to_chain`（`:695-...`）：与 `write_tuple_to_data_page` 同构的分流——`Err(PageFull)` 走既有「分配新页 + 链接 + 写入」分支；`Err(e)`（页损坏）**不换页继续**，以点名该页的错误结束。
- `src/storage/catalog.rs::update_field_in_chain`（`:874-...`，目录行原地改写）：槽损坏 ⇒ 以**点名该页**的错误结束（`CorruptedDataPage`），**不再**被折叠成无细节的泛化错误；其余失败沿用既有文案 `catalog update of table '{target_name}' failed: {e}`。

**新增测试**（`tests/write_path_slot_bounds_test.rs`，**新建**，11 个用例；沿用项目既有夹具形态：真实文件库 + 真实 `Database::open` + 真实 SQL 入口 + **真实文件字节改写**）

- 夹具辅助：页头 `slot_count` 改写（页内偏移 1..3）、槽 `offset` 改写（页内偏移 `4096-6+2`）、槽 `length` 改写（页内偏移 `4096-6+4`）；页字节解析辅助 `slot0_fields`；错误断言辅助 `assert_named_corruption_error`（点名该页 + 含指定串 + **不含** `not found`）。
- T18：夹具 A（INSERT）、夹具 A'（DELETE）、夹具 A''（SELECT 读侧回归对照）、删槽面（`delete_tuple_from_data_page` 直接入口）、未损坏对照（INSERT / DELETE）。
- T19：夹具 B（UPDATE）、夹具 B2（UPDATE + 相邻字节逐字未变）、版本头原地写直接入口（offset 越界 / 负载过短 + 相邻字节逐字未变）、未损坏对照（UPDATE）。
- A13 会话事务面：`write_failure_rolls_back_the_session_transaction` —— 真实 `rtsql` 二进制跑 `BEGIN; INSERT INTO other VALUES (1, 1); INSERT INTO t VALUES (3, 3);` ⇒ 非零退出码 + 点名损坏页 + 「rolled back with the transaction」+ 重开库后 `other` **0 行**。

### Deviations from Plan（与计划的偏差及原因）

1. **夹具 A' 与夹具 B 的 RED 形态与 Plan 的读码预期不同（重要；断言未放宽）**：Plan 预期「同一损坏页执行 DELETE」在变更前是**进程 panic**（`slotted_page.rs:231-232` 取槽处下溢）、「槽 offset 越界」的 UPDATE 在变更前是**进程 panic**（`data_page.rs:272` 越界切片）。**实测**：这两条 SQL 入口在变更前**先**被 Iteration 001 / 002 已交付的**读侧**判据拦下 ⇒
   - 夹具 A'（DELETE）：`delete_row` → `read_version_header` → `read_tuple_from_data_page` 的**页级判据**先命中 ⇒ 返回**页头槽数形态**的 `CorruptedDataPage`（`corrupted data page {page_id}: slot count 700 exceeds the page slot directory capacity of 680 slots`），**无 panic** ⇒ 变更前即 GREEN。
   - 夹具 B（UPDATE）：`update_row` → `read_tuple_from_data_page` 的**逐槽判据**先命中（槽 0 的 `offset=65535` ⇒ 负载越界）⇒ 返回**逐槽形态**的 `CorruptedDataPage`，**无 panic** ⇒ 变更前即 GREEN。
   - 夹具 B2（UPDATE）：读侧的**过短负载**判据先命中（`slot.length=4 < 22`）⇒ 变更前即 GREEN。
   ⇒ 后果：这三个用例是**回归对照**（守住读侧语义不被写侧修复改动），**不是** RED。**Plan 钉定的 RED 形态由写侧原语的直接入口如实复现**（见 Verification 第 1 项）：版本头原地写直接入口在变更前确实是 `src/storage/data_page.rs:272:18` 的 `range start index 65535 out of range for slice of length 4096`（SIGABRT，**与 Plan 钉定的文案与行号逐字对应**），负载过短直接入口在变更前确实是**静默越界写**（进程存活、断言失败）。Plan 冻结的处置方向（页级判据前置、槽级判界共用、页满 / 页损坏分流、既有文案逐字）**未被改动**。
2. **新增 2 个写侧原语的直接入口用例（Plan 未列出）**：因上述读侧先行拦截，写侧原语自身才是写侧缺陷的**可达入口** ⇒ 在**既定的测试目标内**追加 `corrupted_slot_offset_makes_the_version_header_write_fail_with_a_named_error` 与 `short_slot_payload_makes_the_version_header_write_fail_without_touching_neighbours` 两个用例（沿用项目既有形态：Iteration 002 的 `data_page_bounded_read_test.rs` 已用直接调用 `update_version_header_in_data_page` 覆盖可写变体取槽点）⇒ **不新增测试目标、不新增夹具语义、不新增判定层**。
3. **验证命令的等价调整**：Cycle 的 Verification 给出单条 20 目标命令；按 M19 多批次串行纪律拆成**两批各 10 个目标**（批内 `--test-threads=1`、批间串行、**批 1 全绿后才启动批 2**）。目标集合与 Cycle / Iteration 002 清单**完全一致**，Gate 判据仍是「全部 0 failed」。
4. **fmt 只核对本 Cycle 触碰的文件**：基线在 HEAD 即红（50 处 / 9 个既有文件）。本 Cycle 用 `rustfmt` 只格式化**本 Cycle 触碰的 4 个文件**（`src/storage/data_page.rs`、`src/storage/catalog.rs`、`src/storage/page_format/mod.rs`、`tests/write_path_slot_bounds_test.rs`）⇒ 格式化后 `cargo fmt --check` 回到**基线的 50 处 / 9 个既有文件**，**本 Cycle 触碰的文件 0 处**（`slotted_page.rs` 与 `src/storage/mod.rs` 自始 0 处）。未触碰任何无关文件 ⇒ 未违反 Surgical Changes。

### Blocker Handoff

None —— 未命中 Gate 5：无缺失依赖、无验证失败、无 Task Definition 冲突、无实质偏离、无需新增 `StorageError` 变体 / 第二个容量界限、无需改读侧逐点语义、无需改 `VersionHeader::SIZE`、未发现「负载短于 22 字节的合法槽」。

### Self-Review（任务级 Review 与自检）

**Gate 3 — Spec compliance**

- **A12**（页头槽数越界 ⇒ 写路径以点名该页的错误结束，不下溢、不 abort、**不换页继续**）：`add_slot_inner` / `delete_slot` / `delete_slot_by_logical_id` / `update_slot_in_place` 的页级判据**在任何算术之前**消费；`write_tuple_to_data_page` 与 `catalog::append_to_chain` 的「换新页继续」分支**仅**在 `PageFull` 时触发，页损坏走独立分支（点名该页 + 不分配新页 + 不改写损坏页 + 释放溢出链）⇒ **满足**。夹具 A GREEN（错误含 `corrupted data page {page_id}` 与 `slot count 700`；**尾页指针仍指向损坏页** ⇒ 未换页继续）。
- **A13**（槽负载写入有界 + 写入失败 ⇒ 语句失败 / 事务回滚）：`update_version_header_in_data_page` 的两条判据（区间在数据区内 + `slot.length >= 22`）**前置于任何字节写入**；`update_slot_in_place` 的等长 + 区间判界同源 ⇒ **满足**。会话事务面：真实 CLI 的语句以**非零退出码**失败、错误点名损坏页、收尾显式回滚，重开库后同事务内的先前写入**未生效**（`other` 0 行）⇒ **满足**。
- **A14**（页满与页损坏分流，既有文案逐字）：分流由 `PageWriteError` 的变体承载；既有 4 条文案**逐字**保留（见 Implemented）⇒ **满足**。
- **A15**（未损坏路径零变化、判据唯一、索引树与 `src/wal/**` 零 diff、无新增错误变体）：未损坏对照 3 个用例 GREEN + `cargo test --lib` 420/0 + 定向回归 354/0 ⇒ 未损坏路径零变化；`MAX_PAGE_SLOTS` **全项目唯一定义**（`slotted_page.rs:89`）、页级判据**唯一实现**（`SlottedPageHeader::slot_count_exceeds_capacity`，两个访问器变体只委托）、数据页路径的槽级判界**唯一实现**（`slot_payload_in_bounds`，读侧与写侧共用）⇒ 判据唯一；`git diff --stat src/storage/btree/ src/wal/` ⇒ `btree/node.rs` 保持 Iteration 001 / 002 的既有 diff（4 增 6 删，本 Cycle **零改动**），`src/wal/**` **零 diff**；`src/storage/error.rs` **本 Cycle 零改动**（工作区中的 `CorruptedDataPage` 变体属 Iteration 001 / T3 的既有产物）⇒ 无新增错误变体。
- **Preserve 核对**：`get_slot_data_checked` 的语义**逐字不变**（仅把私有判据换成共用自由函数）；`get_slot` / `get_slot_by_logical_id` 的签名与 `None` 语义**未动**；`VersionHeader::SIZE = 22` 与版本头字节布局**未动**；`write-failure-state` 域（ISS30 / ISS31）**未触碰**；目录页组的**读**点**未动**（Iteration 004 范围）；`src/wal/recovery.rs:1519` 的版本头写**自动**被本 Cycle 的有界写覆盖（`src/wal/**` 零 diff）。
- **Forbidden 核对**：未新增页级阈值 / 第二个容量界限 / 第二个判据实现；未新增命令参数 / 配置项 / 缓存 / 闸门机制 / 判定层 / 身份证明机制（Hash / 指纹 / revision pin / run-id / manifest / 时间证明）；未改磁盘格式；未新增 `StorageError` 变体；未跑全量回归（归 Iteration 009 / T16）。

**Gate 3 — Code quality**

- diff 无计划外修改：`git diff --stat` = `catalog.rs` 46、`data_page.rs` 218、`storage/mod.rs` 5、`page_format/mod.rs` 5、`page_format/slotted_page.rs` 231（行）+ 新测试文件；逐 hunk 复核确认**全部落在 Task Definition 列出的符号内**（`catalog.rs` 的 hunk 全部落在 import / `append_to_chain` / `update_field_in_chain`；`data_page.rs` 全部落在 import / `write_tuple_to_data_page` / `update_version_header_in_data_page` / `delete_tuple_from_data_page`）。
- 错误 / 边界 / 状态 / 资源生命周期：
  - 判据一律**前置于任何字节写** ⇒ 失败路径不产生部分修改（夹具 B2 的「相邻字节逐字未变」断言直接证明）。
  - 溢出链生命周期：页损坏分支与既有页满失败分支**同样释放**本次调用分配的链页（best-effort）⇒ 不产生孤儿链页。
  - 损坏页**不被改写**：`modify_page` 虽置脏位，但闭包内未写任何字节 ⇒ 回写内容与原字节逐字相同。
  - `free_space` 的降级路径：`saturating_sub` + 页级判据 ⇒ 任何页头损坏形态都返回 0（调用方判定「无可用空间」⇒ 不写入），不再回绕成巨大值。
  - 测试不会因错误原因通过：每个断言都绑定**具体可观察**（错误文案含页号与槽号 / 尾页指针未变 / 字节区间逐字相同 / CLI 退出码 / 重开库后的行数）；未损坏对照与夹具共用同一夹具辅助，仅改字节 ⇒ 差异由被测行为解释。
- **无新增 clippy finding**：lib 侧 17 条（与基线同数，位置全部不在本 Cycle 触碰的文件）；本 Cycle 触碰的 6 个文件（含新测试文件）**0 条**；`cargo fmt --check` 本 Cycle 触碰的文件**0 处**。
- 既有错误文案的**调用点兼容性**：`btree/node.rs` 的 `StorageError::Io(std::io::Error::other(e))` 与 `wal/recovery.rs` 的 `format!("add_slot failed: {}", e")` **逐字继续编译**（`PageWriteError: Error + Send + Sync + 'static`）⇒ 两个目录**零 diff**。

**遗留 Minor 问题（不阻塞，不蔓延）**

1. `src/storage/catalog.rs::delete_from_chain`（`:824` 附近）沿用既有形状 `let _ = slotted.delete_slot_by_logical_id(*lid);`（**丢弃**错误）：页头槽数越界的目录页上，本 Cycle 的页级判据使**删槽**不再 panic，但该页的**读**路径（`SlottedPageRef::get_slot` 遍历 `0..slot_count()`）会**先**在槽目录定位下溢处 panic —— 属**目录读点**（Iteration 004 / T4 的范围，本 Cycle Forbidden 明列不改）。⇒ 本 Cycle 未改动该调用点，其行为未变（读点先 panic）。报告供 T4 参考。
2. `src/storage/catalog.rs::delete_named_copies_except` 的删槽失败仍以 `logical_id {lid}: {e}` 归入泛化的 `StorageError::Internal`（既有形状，未点名页）：该点不在 T18 / T19 的 Task Definition 目标内，且其**前置读点**（目录读面）先命中页级判据 ⇒ 本 Cycle 不扩大范围。

### Verification Evidence（验证命令与实际结果）

**1) 先行测试的 RED 实况（变更前，逐个夹具单独运行以避免一次 panic 掩盖其它夹具）**

| 夹具 / 用例 | 变更前实测形态 |
|---|---|
| 夹具 A（INSERT，页头 `slot_count`=700） | **RED — 进程 abort**：`panicked at src/storage/page_format/slotted_page.rs:295:30: attempt to subtract with overflow` ⇒ `signal: 6, SIGABRT`（**与 Cycle 钉定的 RED 文案与行号逐字对应**） |
| 夹具 A'（DELETE，同一损坏页） | 变更前即 **GREEN**：读侧页级判据先命中 ⇒ `corrupted data page {page_id}: slot count 700 exceeds the page slot directory capacity of 680 slots`（无 panic） |
| 夹具 A''（SELECT，同一损坏页） | 变更前即 **GREEN**（读侧回归对照） |
| 夹具 B（UPDATE，槽 0 `offset`=65535） | 变更前即 **GREEN**：读侧逐槽判据先命中（无 panic） |
| 夹具 B2（UPDATE，槽 0 `length`=4） | 变更前即 **GREEN**：读侧过短负载判据先命中（无 panic） |
| 版本头原地写直接入口（夹具 B 同型） | **RED — 进程 abort**：`panicked at src/storage/data_page.rs:272:18: range start index 65535 out of range for slice of length 4096` ⇒ SIGABRT（**与 Cycle 钉定的 RED 文案与行号逐字对应**） |
| 版本头原地写直接入口（夹具 B2 同型） | **RED — 断言失败**（进程存活）：`expect_err` 收到 `Ok(())` ⇒ 变更前版本头**静默越界写**（22 字节越过该行 4 字节负载、覆盖相邻字节） |
| 会话事务面（真实 CLI） | **RED**：CLI 进程在 `slotted_page.rs:295:30` panic（`attempt to subtract with overflow`）⇒ 未产生「点名损坏页 + 回滚」的失败面 |
| 未损坏对照（3 个用例） | 变更前即 **GREEN**（零变化基线） |

**2) GREEN（变更后，全部在最终代码状态上运行）**

- `cargo test --test write_path_slot_bounds_test -- --test-threads=1` ⇒ **11 passed / 0 failed**。
- `cargo test --lib` ⇒ **420 passed / 0 failed**（覆盖 `page_format` 与索引树单元测试 ⇒ 索引树零回归）。
- 定向回归 **批 1/2**（10 个目标，批内 `--test-threads=1`）：`storage_test` 16 + `full_table_dml_test` 39 + `table_manager_test` 21 + `alter_table_test` 11 + `executor_test` 3 + `secondary_index_test` 4 + `visibility_test` 27 + `mvcc_abort_test` 21 + `mvcc_commit_test` 6 + `explicit_tx_test` 5 = **153 passed / 0 failed**。
- 定向回归 **批 2/2**（10 个目标，批内 `--test-threads=1`；**批 1 全绿后才启动**）：`overflow_page_test` 3 + `ddl_rewrite_consistency_test` 9 + `drop_table_free_test` 6 + `free_page_lifecycle_test` 27 + `planner_test` 12 + `plan_exec_test` 3 + `ddl_catalog_transactional_test` 27 + `index_manager_test` 4 + `version_chain_test` 107 + `e2e_test` 3 = **201 passed / 0 failed**。
- 定向回归合计 = **354 passed / 0 failed**（与 Iteration 002 的 354/0 逐目标同数 ⇒ 未损坏路径零变化）。
- `cargo fmt --check` ⇒ **50 处 / 9 个既有文件**（与基线同数同分布）；**本 Cycle 触碰的文件 0 处**。
- `cargo clippy --all-targets -- -D warnings` ⇒ lib 侧 **17 条**（与基线同数），**无一条**指向本 Cycle 触碰的文件；非 deny 模式下本 Cycle 触碰的文件同样 **0 条**。
- `git diff --stat src/storage/btree/ src/wal/` ⇒ `src/storage/btree/node.rs` 4 增 6 删（Iteration 001 / 002 的既有 diff，本 Cycle 零改动）、`src/wal/**` **零 diff**。

**3) 未损坏对照的零变化证据**

- `undamaged_page_inserts_and_deletes_are_unchanged`：INSERT → 1 行、SELECT → 3 行、DELETE → 1 行、SELECT → 2 行 ⇒ 与既有行为一致。
- `undamaged_page_updates_are_unchanged`：UPDATE → 1 行、SELECT → 1 行。
- `cargo test --lib` 420/0 + 定向回归 354/0 ⇒ 未损坏路径（页格式层、数据页写路径、目录写路径、索引树、恢复面调用点）零回归。

**4) M19 纪律**：本 Cycle **未跑全量回归**（change 预算归 Iteration 009 / T16）；定向回归按「两批各 10、批内 `--test-threads=1`、批间串行」执行；上述命令均**实际执行**且结果如上，**未重复运行已通过的验证**。

### Persisted Evidence

**None required** —— Plan 的 `Persisted Evidence` 为 `none`；全部结论（RED 形态、GREEN、未损坏对照、fmt / clippy 基线对比）均由低成本可重跑的 `cargo test` 命令承载，Act Response 的 Verification Evidence 段可完整承载命令与实际结果 ⇒ 未创建 `evidence/` 目录。

### Experience Candidates

**None** —— 本 Cycle 无「已端到端验证成功且可重复 / 高风险」的可复用操作路径（夹具字节算术已写入测试文件本身，随代码归档）；范围外的实质缺陷：无（见遗留 Minor 问题 1–2：均为**目录读点**范畴，属 Iteration 004 / T4 的既有范围，不是本 change 范围外的实质缺陷）⇒ 不构成 Issue 候选。

### 未解决问题

- **无阻塞项。**
- 目录读点（`SlottedPageRef::get_slot` 遍历在页头槽数越界时的下溢）仍会在目录页上 panic —— 属 Iteration 004 / T4 的范围，本 Cycle Forbidden 明列不改；已在遗留 Minor 问题 1 中报告并给出定位。
- ISS37 / ISS38 台账的「处置指针」字段归 Recorder 所有 —— 本 Cycle **未改台账**，承载分配（T18 / T19 落在此 Iteration）记录于本 Response 与 change 的 `tasks.md` / `design.md` D15，由上层转交 Recorder 回填。
- 本 Cycle **未归档 change、未同步全局文档（`.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆 / references 均未动）、未创建下一 Cycle 或下一 Iteration、未填写 `Plan Review`、未执行 `git commit`**。

## Plan Review

- Review Result: accepted

**Findings**

**F1（PASS，A12）写路径在**任何算术 / 字节写之前**问页级判据；页级损坏一律以点名该页与页头声称槽数的错误结束。**
独立核读（不重跑 Act 的验证）：`src/storage/page_format/slotted_page.rs` 的六个写侧入口逐点核对「判据 ⇒ 算术 / 写入」的先后顺序：

| 写侧入口 | 页级判据 | 被保护的算术 / 写入 | 结论 |
|---|---|---|---|
| `add_slot` → `add_slot_inner` | `:373` `if self.header.slot_count_exceeds_capacity()` | `:385` `Page::PAGE_SIZE - self.slot_count() * Slot::SIZE`、`:392` 负载写、`:396` 槽目录写 | 判在前 ✓ |
| `delete_slot`（`:430` 起） | `:434` | `:448/:449` 槽目录搬移、`:452` 槽目录写、`:456` `PAGE_SIZE - count * Slot::SIZE` | 判在前 ✓ |
| `delete_slot_by_logical_id`（`:469` 起） | 判据先于 `get_slot_by_logical_id` | 取槽定位 | 判在前 ✓ |
| `update_slot_in_place`（`:494` 起） | `:501` | `:507` 取槽、`:522` 槽级判界 → `:525` `self.page.data[start..end].copy_from_slice(data)` | 判在前 ✓ |
| `free_space`（`:535` 起） | `:536` | `:540` `PAGE_SIZE - slot_count * Slot::SIZE` + `:542` `saturating_sub` | 判在前 ✓ |
| `data_page.rs::update_version_header_in_data_page` | `:306` | `:313` 取槽、`:326` 槽级判界、`:329` 长度判据、`:334` 版本头写 | 判在前 ✓ |

页级判据**只有一处实现**（`src/storage/page_format/slotted_page.rs:172-174` 的 `slot_count_exceeds_capacity` = `self.slot_count as usize > MAX_PAGE_SLOTS`），写侧调用的是**同一个**实现，未另造阈值 ⇒ 满足 delta spec「页级判据与槽级判界在全项目 SHALL 各只有一处实现，读侧与写侧 SHALL 共用」。
错误形态：**单一映射点** `src/storage/data_page.rs:68-91` 的 `page_write_error_to_storage_error` ⇒ 页级损坏 → `corrupted_data_page_slot_count_error(page_id, slot_count)`、槽级损坏 → `corrupted_data_page_slot_error(page_id, slot_index, &slot)`，二者都是**既有**读侧构造函数（`:49` / `:16`）⇒ detail 与读侧**同名同形**（`slot count 700 exceeds the page slot directory capacity of 680 slots` / `slot 0 range [...] lies outside the page data area`）⇒ 统一落在 `StorageError::CorruptedDataPage { page_id, detail }`，**未新增任何 `StorageError` 变体**（`git diff -- src/storage/error.rs` 只含 Iteration 001 交付的 `CorruptedDataPage`，本 Cycle 未改该文件），Display 前缀 `corrupted data page {page_id}: ` 未动 ⇒ **A12 成立**。

**F2（PASS，A15 / delta spec「未损坏路径的行为零变化」/ Preserve）既有错误文案逐字保留。**
`git show HEAD:src/storage/page_format/slotted_page.rs` 的四处原文 = `"No enough space in page"`（`:235`）、`"Slot index out of range"`（`:279`）、`"logical_id {} not found"`（`:310`、`:324`）、`"in-place update requires equal length: slot {} bytes, got {} bytes"`（`:328`）。当前 `PageWriteError` 的 `Display` 实现（`slotted_page.rs:28-99`）与之**逐字一致**（`LengthMismatch` 的 `slot_length` / `write_length` 分别取 `slot.length` / `data.len()`，渲染结果相同）；既有调用点（索引树 `StorageError::Io(std::io::Error::other(e))`、恢复面 `format!("add_slot failed: {}", e)`）的渲染文本不变 ⇒ 既有文案零变化。

**F3（PASS，A12 / A13）「页满 ⇒ 换新页继续」**仅**在页满时触发；页损坏以点名该页的错误结束，不换页、不折叠、不静默跳过。**
- `src/storage/data_page.rs::write_tuple_to_data_page`（`:103-187`）：`add_slot` 的错误按 `PageFull` 与**其余**两类分流 —— `PageFull` ⇒ 走既有「换新页继续」（`:141-176`）；**页损坏** ⇒ `:181-186` 以 `page_write_error_to_storage_error` 上浮，**不**新建页、**不**改写该页 ⇒ 损坏页被跳过而数据继续落盘的情形已消除。
- `src/storage/catalog.rs::append_to_chain`（`:695-742`）：同一分流 —— `PageFull` ⇒ 既有「追加新页」；其余 ⇒ 点名错误上浮。
- `src/storage/data_page.rs::delete_tuple_from_data_page`（`:351-430`）：`LogicalIdNotFound` / `SlotIndexOutOfRange` ⇒ 既有 `SlotNotFound(row_id)`；**页损坏** ⇒ `page_write_error_to_storage_error` ⇒ **不再**把损坏折叠成「行不存在」。
- `src/storage/catalog.rs::update_field_in_chain`（`:874-941`）：目录行等长原地改写失败 ⇒ 点名错误上浮，**不**折叠成「表不存在」。

**F4（PASS，A14）槽负载判界前置于任何字节写；短负载不静默改写相邻字节。**
- `slotted_page.rs::update_slot_in_place`：`slot_payload_in_bounds(self.slot_count(), start, end)` 先于 `copy_from_slice` ⇒ 越界槽不产生任何字节写入。
- `data_page.rs::update_version_header_in_data_page`：两条判据**都在**写之前 —— `:326` 槽区间须完整落在页数据区（复用读侧**同一实现** `slot_payload_in_bounds`）、`:329` `slot.length >= VersionHeader::SIZE`（否则 22 字节版本头会越过该槽自身负载、覆盖相邻字节）⇒ `:334` 的写只在两条判据都满足时发生。
- `slot_payload_in_bounds` 由读侧私有方法**上提**为读写共用的自由函数（`slotted_page.rs:101`），实现与上提前**逐字同形**（`start >= SlottedPageHeader::SIZE && end <= slot_area_start`，`slot_area_start = PAGE_SIZE.saturating_sub(slot_count * Slot::SIZE)`）⇒ 读侧语义零变化；`get_slot_data_checked` 与 `get_slot` 的函数体**逐字未动**（`get_slot` 的签名与 `None` 语义未动）⇒ 读侧冻结语义（逐点「过短负载」判定、布尔辅助安全值降级）零变化 ⇒ **A14 成立**。

**F5（PASS，偏差分类）Act 的 Deviations 第 1 项属「现状变化 / Plan 预期偏差」，**不是** Act 偏离；断言未放宽。**
Plan 预期「夹具 A'（DELETE）与 B / B2（UPDATE）在真实 SQL 入口上变更前会 RED（panic / 静默覆盖）」；Act 实测「变更前**已 GREEN** —— Iteration 001 / 002 交付的读侧判据先拦截，SQL 入口到不了写路径」。
独立复核该实测的机理：`src/storage/data_page.rs::read_tuple_from_data_page`（`:205-290`）的判定顺序是 **页级判据 → 槽级判界 → 短负载判据**，三者在任何无界读之前 ⇒ UPDATE / DELETE 必须先读出行 ⇒ 损坏页上先命中读侧判据并返回点名错误 ⇒ **写路径确实到不了**。该事实与 Plan 的推断前提（「读侧逐点判据不存在」）在 Cycle 000 之后已不成立 ⇒ **现状变化**导致的 Plan 预期偏差，冻结的**处置方向未被改动**（不改读侧语义；写侧仍须有界）。
断言未被放宽：`tests/write_path_slot_bounds_test.rs:175-198` 的 `assert_named_corruption_error` 要求错误**点名该页** + 指定 needle，且**显式禁止**出现 `"not found"`；`:411-440` / `:446-490` 的直接入口用例要求错误点名该页与该槽（越界形态还要求出现 `"65535"`，即区间），并**逐字比对**覆盖范围内字节前后一致 ⇒ 断言严于「不 panic」。

**F6（PASS，新证据）写原语层**不是**「读侧已拦 ⇒ 无价值」的防御性冗余：非读取型入口确实直达写原语。**
逐条据代码核对：

| 入口 | 到达写原语前是否读该页 | 结论 |
|---|---|---|
| `src/executor/insert.rs:253` → `data_page.rs:103` `write_tuple_to_data_page` → `add_slot` | **否**（尾页直接落位，不先读槽） | 写原语是该路径的**第一道**闸门 ⇒ 夹具 A 的 SQL 入口 RED（`slotted_page.rs:295:30 attempt to subtract with overflow` → SIGABRT）是**真实**的 SQL 入口 RED |
| `src/storage/catalog.rs:695` `append_to_chain` → `add_slot` | **否**（`chain_head` 只读 `next_page_id` 头字段，不取槽） | 目录追加路径的第一道闸门 |
| `src/wal/recovery.rs:207` `add_slot_with_logical_id` → `add_slot` | **否**（直接 `modify_page`） | 恢复面强制落位的第一道闸门 |
| `src/transaction/manager.rs:514/:525`（abort 墓碑）→ `data_page.rs:292` `update_version_header_in_data_page` | **否**（`header` 取自既有版本链，不重读槽负载） | 版本头原地写的第一道闸门 |

⇒ 写侧页级判据与槽级判界是**可达代码**上的必要防线（ISS37 / ISS38 的原始主张成立）；读侧先拦截只**遮蔽**了「必须先读出行才能写」的那部分 SQL 语句形态，**不**使写原语层成为冗余；本 Cycle 未以「读侧已拦」为由放宽任何判据。

**F7（PASS，A15）容量界限唯一定义且值不变；索引树侧与 `src/wal/**` 零 diff；读侧冻结语义零变化。**
`grep -rn "MAX_PAGE_SLOTS" src/` ⇒ 定义**唯一**在 `src/storage/page_format/slotted_page.rs:89` = `(4096 - 16) / 6` = **680**，取值与原 `btree/node.rs` 的定义**一致**；`src/storage/btree/**` 的工作区 diff **只有** Iteration 002 的常量迁移（导入 + 删除本地 `const`），**零逻辑行** ⇒ 索引树侧既有取值与判据逐字未变；`git diff --stat src/wal/` ⇒ **零 diff**；`src/storage/error.rs` 本 Cycle 未改。⇒ **A15 成立**。
（记录一项**非阻塞观察**：槽级判界在数据面为**一处**（`page_format::slot_payload_in_bounds`，读写共用），索引树侧另有**同形**私有实现 `src/storage/btree/node.rs` —— 该并存是 delta spec「索引树侧既有的取值与判据 SHALL NOT 改变」的**必然结果**，且属 Iteration 001 / 002 已验收的既有状态，非本 Cycle 引入，不构成 finding。）

**F8（PASS，RED 复现指向修复点、证据充分性）Act 报告的 RED 文本逐字复现了 Plan 钉死的形态；本 Plan 未重跑 Act 已沿用的验证。**
独立复核两个 RED 的指向：
- `slotted_page.rs:295:30 attempt to subtract with overflow`：该 panic 只可能来自 `Page::PAGE_SIZE - self.slot_count() * Slot::SIZE`（`slot_count = 700` ⇒ `4096 - 4200` 下溢）；该表达式在当前代码里位于 `slotted_page.rs:385`，**前置**页级判据在 `:373`；列号可复核 —— 8 空格缩进 + `let free_space_end = `（21 字符）⇒ `Page::PAGE_SIZE` 起始于**第 30 列**，与 RED 的 `:30` 一致 ⇒ RED 指向的正是现在被守卫的那条表达式。
- `data_page.rs:272:18 range start index 65535 out of range for slice of length 4096`：`65535` = 注入的槽偏移 `0xFFFF`，`4096` = 页长；`update_version_header_in_data_page` 内**唯一**的负载切片就是版本头写 `page.data[slot_offset..slot_offset + VersionHeader::SIZE]`（该函数不调用 `get_slot_data`）⇒ RED 必定来自那条写；当前代码在 `:326`（区间判界）与 `:329`（长度判据）之后才写 ⇒ 指向的正是现在被守卫的那次写。
- 短负载直接入口的「静默覆盖」RED（断言失败）：当前代码 `:329` 的 `slot.length < VersionHeader::SIZE` 判据正是该断言的对应面。
⇒ RED 是真实的「目标行为未达成」，指向现被守卫的位置，**断言未放宽**。
沿用 Act Response「Verification Evidence」的结论（覆盖范围未变化 ⇒ 直接沿用，本 Plan **未重跑**）：GREEN `cargo test --test write_path_slot_bounds_test` **11/0**；`cargo test --lib` **420/0**；定向回归 2 批 **153/0 + 201/0 = 354/0**（**批 1 全绿后才启动批 2**，符合 M19 分批串行纪律）；`cargo fmt --check` 回到基线 **50 处 / 9 既有文件**且**触碰文件 0 处**；clippy lib **17 条**（= 基线）且**触碰文件 0 条**。⇒ 结论可低成本重跑（1 个测试目标 + `cargo test --lib` + 既有回归目标）⇒ `Persisted Evidence: none` 成立，无须建 `evidence/` 目录。

**F9（Minor finding，非阻塞）目录**删除**路径的两处错误丢弃 / 折叠 —— 归属裁定：**并入 Iteration 004**，不新立 Iteration、不登记 Issue。**
Act 上报的两点经独立核读确认成立：
- `src/storage/catalog.rs:831`（`delete_from_chain`）：`let _ = slotted.delete_slot_by_logical_id(*lid);` ⇒ 删除错误被**丢弃**。
- `src/storage/catalog.rs:1005-1014`（`delete_named_copies_except`）：删除失败被折叠成 `StorageError::Internal("catalog update of table '…' failed to remove stale copy (…)")` ⇒ **不点名该页**（`PageWriteError` 的 Display 不含页号）。
**Act 的归类不精确**：这两点是目录**删除（写）路径**的语义，不是读点；「位于目录读点之后」只是它们的**调用位置**关系，不等于「属读点范围」。但**归属仍落在 Iteration 004**，理由是可核的**可达性**判定：这两个删除各自的前置读点**本身就是 T4 的 Target** —— `delete_from_chain`（`catalog.rs:808-809`）与 `delete_named_copies_except`（`catalog.rs:965-974`）；T4 的 Required behavior 要求「**取槽之前**问页级判据」并把读点改为具名错误 ⇒ T4 落地后，损坏目录页上这两个删除**不可达**（页级槽数越界与槽负载越界两种形态都在读点被拦截）⇒ 错误丢弃 / 折叠**不可观察**；且 T4 的方向「目录页损坏 ⇒ 该库 SHALL NOT 打开」使整条目录删除路径在损坏库上不可达。⇒ 按公共规则 › Scope Control「候选工作无法关联到可达代码和数据时，不实施」⇒ **不需要新立 Iteration**，也**不需要**登记 Issue。
**条件（写入 Iteration 004 的追加标注）**：若 T4 实施时未在读点先行拦截（保留 `let _ =` / 折叠分支仍可达），则该标注转为 T4 的一个 work item —— 届时需把删除错误改为点名该页并上浮。

**F10（PASS，规格层次核对）delta spec 的 Scenario 层次与实际可观察行为**一致**：Scenario 1 / 2 写在**写操作**层，Scenario 3 写在**语句**层 —— 无需按「误写为 SQL 入口形态」修正。**
逐条核对 `openspec/changes/2026-10-25-ms37-defect-ledger-closure/specs/page-write-bounded-write/spec.md`：Scenario 1 的 WHEN = 「**对该页执行写入**（新槽落位 / 槽删除 / 槽负载原地改写 / 行版本头原地改写）」⇒ **写操作层**，枚举的恰是本 Cycle 收口的四个写原语族；Scenario 2 的 WHEN = 「**对该槽执行原地改写**（行版本头原地改写 / 目录行等长原地改写）」⇒ 同样写操作层；Scenario 3 的 WHEN = 「该会话事务内的**一条写入语句**命中该损坏页」⇒ **语句层**，且该形态由真实 CLI 会话用例实证（`tests/write_path_slot_bounds_test.rs:318-357`：非零退出码 + 错误点名该页 + 报告 `rolled back with the transaction` + 重开库后同事务内的先前写入为 0 行）。
⇒ **spec 没有把写原语层误写成 SQL 入口形态**，本次不需要按该理由修正 spec；夹具 A' / B / B2 的 SQL 入口用例保留为**回归对照**（它们断言「语句以点名错误失败、相邻字节未变」），与写操作层的 RED **互补**而非替代。

**F11（Minor finding，已在本 Review 内修正规格措辞）「区间」措辞只由**越界**形态满足，**载荷不足**形态复用既有 `short_data_page_slot_error` 文案（点名该页 + 该槽 + 负载**长度**，不含字节区间）。**
delta spec 的 Requirement 正文写「（负载形态时同时点名该槽与**该区间**）」、Scenario 2 写「（含该槽的**字节区间**）」；而本 Cycle 的 T19 Task Definition 写的是「点名该页与该槽（**含区间 / 长度**）」⇒ 两者**互相矛盾**。实现按 Task Definition 复用 Iteration 001 冻结的 `short_data_page_slot_error`（`corrupted data page {id}: slot {i} payload of {len} bytes is shorter than the {22}-byte version header`）⇒ 点名该页、该槽与负载长度，**不**含区间。**A14 本身只要求「点名该页与该槽」⇒ 满足** ⇒ 非 Acceptance 缺口；但收尾合并时该措辞会造成假缺口。
**处置**：本 Review 在 change 产物内把 delta spec 的两处措辞改为**析取**形式（与已获批的 Task Definition 一致，**不放松任何 Acceptance**）：
- Requirement 正文：「（负载形态时同时点名该槽；槽负载越界形态额外点名该槽的字节区间）」；
- Scenario 2：「（槽负载越界形态含该槽的字节区间；载荷不足形态点名该槽的负载长度）」。
同步对齐 `design.md` D15 的同源措辞（`:341` 错误形态段、`:372` 行为条款 2），使 design / delta spec / 实现三者口径一致。
（spec 与 design 均属 change 产物，在 Plan 权限内修正；`openspec/specs/` 未动。）

**F12（PASS，记录）Act Response 的「任务状态记录」自检段对 A13 / A14 的**编号**与 Plan Context 的 Acceptance 表**不一致**（内容一致，仅标签置换）——以 Plan Context 的表为准。**
Plan Context 的 Acceptance 表（`:230-233`）= A12（页级判据 / 点名槽数）、A13（写入失败 ⇒ 语句失败 + 回滚 + 不换页继续 + 不折叠 + 不静默跳过）、A14（槽负载越界 / 过短 ⇒ 不越界写 + 不改写相邻字节 + 点名该页与该槽）、A15（零变化 / 判据唯一 / 索引树与 `src/wal/**` 零 diff / 不新增错误变体）。Act Response 自检段（`:348-351`）把「页满与页损坏分流 + 既有文案逐字」并入 A14、把「槽负载有界 + 会话事务面」并入 A13 ⇒ **同一批行为、标签置换**，无遗漏、无放宽。本 Review 的 A12–A15 判定**一律按 Plan Context 的表**（权威）执行 ⇒ 四条 Acceptance 的**实质内容**全部满足，标签差异不构成 Acceptance Gap，也不需要 Act 补正（Act Response 已冻结，不改写）。

**偏差分类**：无 Plan 遗漏、无 Plan 错误、无 Act 偏离、无现状变化导致的**阻塞**偏差。Act 的 Deviations 第 1 项属**现状变化 / Plan 预期偏差**（读侧先拦截已在 Cycle 000 之后成立），已被如实记录且**不改变** Acceptance 与冻结方向（F5）；Deviations 第 2–4 项为非实质执行细节（RED 复现手段改用写原语直接入口、用例保留为回归对照、断言未放宽）。F9 / F11 为**非阻塞 Minor finding**：F9 归入 Iteration 004（可达性判定，见上）；F11 已在 change 产物内修正规格措辞。

**Acceptance Gaps**

None —— A12 / A13 / A14 / A15 全部满足（逐条证据见 F1–F8、F10）；本 Cycle 的 Verification 清单与 Act 实际执行的命令**目标集合一致**且全绿；无被跳过的 Acceptance（跳过项为「全量回归」与「目录页组 / 恢复面组 / 索引树侧 / 全仓 fmt-clippy」等**范围性**排除，已记录于 Act Response 的「跳过项」，与 change 的预算安排一致）。

**Evidence**

- 代码与 diff（逐行核读，未改动产品代码）：`git diff -- src/storage/page_format/slotted_page.rs src/storage/page_format/mod.rs src/storage/data_page.rs src/storage/catalog.rs src/storage/error.rs src/storage/mod.rs`；`git diff --stat src/wal/`（**零 diff**）、`git diff src/storage/btree/`（**只有** Iteration 002 的常量迁移）；逐行核读 `src/storage/page_format/slotted_page.rs:28-99/101-104/160-180/365-400/430-467/469-492/494-528/535-545`、`src/storage/data_page.rs:16-91/103-187/205-290/292-341/351-430`、`src/storage/catalog.rs:695-742/794-840/874-941/966-1020`。
- 结构核对：`grep -rn "MAX_PAGE_SLOTS" src/`（唯一定义 = `slotted_page.rs:89`，值 680）；`grep -rn "slot_count_exceeds_capacity|slot_payload_in_bounds" src/`（页级判定唯一实现一处 + 委托 + 消费点；槽级判界数据面一处 + 索引树同形私有实现一处，见 F7 观察）。
- 既有文案核对：`git show HEAD:src/storage/page_format/slotted_page.rs`（`:235`/`:279`/`:310`/`:324`/`:328`）与当前 `PageWriteError` 的四个 `Display` 逐字比对（F2）。
- 测试与断言：`tests/write_path_slot_bounds_test.rs:175-198`（点名断言 + 禁止 `"not found"`）、`:215-237`（夹具 A）、`:239-253`（夹具 A' 回归对照）、`:254-268`（读侧形态不变）、`:269-289`（删除原语直接入口）、`:293-310`（未损坏对照）、`:318-357`（真实 CLI 会话回滚）、`:366-376`（夹具 B 回归对照）、`:381-406`（夹具 B2 回归对照）、`:411-440` / `:446-490`（版本头写直接入口）、`:494-505`（未损坏对照）—— 共 **11** 例。
- 沿用 Act 已报告且覆盖范围未失效的验证结论（来源：Act Response「Verification Evidence」）：RED 逐夹具隔离进程复现（夹具 A → `slotted_page.rs:295:30 attempt to subtract with overflow` → SIGABRT；版本头写直接入口 → `data_page.rs:272:18 range start index 65535 out of range for slice of length 4096` → SIGABRT；短负载直接入口 → 断言失败；真实 CLI 会话 → CLI panic）；GREEN `cargo test --test write_path_slot_bounds_test` **11/0**、`cargo test --lib` **420/0**、定向回归 2 批 **354/0**（批 1 全绿后才启动批 2）、fmt **50 处 / 9 文件**与基线逐项一致且触碰文件 0 处、clippy lib **17 条**且触碰文件 0 条。**本 Plan 未重跑上述验证**（公共规则 › 验证：覆盖范围未变化 ⇒ 直接沿用），只做了代码与 diff 的**独立阅读**与上述结构核对。

**Follow-up Decision**

**接受本 Cycle（`accepted`）。** 依据：既有 Acceptance（A12 / A13 / A14 / A15）全部满足且有独立代码核对支撑（F1–F8、F10）；无阻塞 finding；Act 的 Deviations 第 1 项为**现状变化**导致的 Plan 预期偏差，冻结的处置方向未被改动（F5、F6）；F9 为非阻塞 Minor finding，归属裁定为**并入 Iteration 004**（追加标注）；F11 为非阻塞 Minor finding，已在 change 产物内修正 delta spec 的两处措辞（不放松任何 Acceptance）。
**本 Cycle 不再需要修复**：无待修项；不建 rework / replan Cycle。
**第 5 节三问的逐条判定**（记录在案）：
1. **Deviations 第 1 项是否阻塞 A12–A15？** **否。** A12 / A14 的主语是「**写路径**」，其 Scenario 的 WHEN 是「对该页 / 该槽**执行写入 / 原地改写**」（写操作层），不是「执行一条 SQL 语句」；Act 以**写原语直接入口**复现的 RED（文本与行号逐字一致）+ SQL 入口用例作为回归对照，**足以**支撑 A12 / A14；A13 的语句层形态由真实 CLI 会话用例**独立实证**（RED：CLI panic；GREEN：非零退出 + 点名该页 + 事务回滚）。
2. **读侧先拦截是否使写原语层在 SQL 入口不可达？该层是否仍有价值？** 读侧先拦截**只**发生在「必须先读出行才能写」的语句形态（UPDATE / DELETE 的夹具 A' / B / B2）；**非读取型入口仍直达写原语**（INSERT 落尾页、`catalog.rs:695` 目录链追加、`wal/recovery.rs:207` 恢复面强制落位、abort 墓碑的版本头写）⇒ 写原语层是**可达代码**上的必要防线，**不是**冗余防御；本 Review 亦**未**以「有 defense-in-depth」为由放宽任何判据（F6）。
3. **delta spec 的 Scenario 文本是否与实际可观察行为一致？** **一致**，且**不存在**「把写原语层误写成 SQL 入口形态」的问题（F10）；仅发现「区间」措辞与「载荷不足」形态不匹配，已按析取形式修正（F11）。
**状态同步**：change `tasks.md` 已同步 —— T18 = `done（Iteration 003 / Cycle 000，Plan Review 2026-10-26 accepted）`、T19 同；Iteration Map 的 **003** 行状态 = `accepted`；**Next Iteration** = **004**（T4，`ready`）；T4 仍为 `planned（Iteration 004 / Cycle 000，Plan Context ready）`；005–009 顺延不变。
**Act Response 中「T18 / T19 状态已同步为『Act Response reported，待 Plan Review』」是 Act 当时的记录，现已被本 Review 的结论取代。**

**Iteration Plan Update**

本 Cycle 自身的目标、范围、依赖、验证方式与验收边界**未变**；Iteration 003 的两个 task（T18 / T19）全部关闭，Iteration 003 **完成**。F9 的处置**不改变** Iteration Plan（并入已存在的 Iteration 004，不新立 Iteration、不顺延 005–009）；F11 只改 change 产物的措辞，不改任何 Iteration Plan。Iteration 004（T4）的 Plan Context **已 `ready`**，本 Review **不展开**它（仅按 F9 追加标注）。

**Next Cycle**

None —— 本 Cycle 是 Iteration 003 的唯一 Cycle，无 rework / replan 后继。

**Next Iteration**

`openspec/changes/2026-10-25-ms37-defect-ledger-closure/iterations/004-catalog-bounded-read/000-initial.md`（Iteration 004 / Cycle 000，任务 T4（ISS27 目录页组），Cycle Type = `initial`，Plan Context 状态 `ready`）—— **执行顺序**：`002-slot-count-underflow`（已 `accepted`）→ `003-write-path-slot-bounds`（本 Cycle，已 `accepted`）→ `004-catalog-bounded-read`（T4，**已 `ready`**，含 F9 的追加标注）→ `005-recovery-bounded-read`（T5，尚未创建）→ 006 / 007 / 008 / 009。
