# 写路径的槽负载偏移无界写：版本头就地写未校验槽偏移 ⇒ 越界切片 panic 或页头 / 槽目录静默改写

- Status: closed（2026-10-26 fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`）
- Filed: 2026-10-25
- Source: Act Response 的 Issue 候选（`Experience Candidates` 段第 2 项的第 2 条「可写变体的负载偏移无界写」），change `2026-10-25-ms37-defect-ledger-closure` Iteration 002 Cycle `iterations/002-slot-count-underflow/000-initial.md`；同 Cycle 的 `Plan Context` 的 `Risks and Notes` 段（登记前的候选表述，行号 `:237`）
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 / cargo 1.99.1、RTsql 仓库 HEAD `e9cb9d3`（产品改动未提交）；登记期间工作区含 MS37 change Iteration 000 / 001 / 002 的在途实施改动 ⇒ 本条的行号相对**登记时的工作区状态**；本条为**读码确认**，未构造复现

## 缺陷描述

- 预期：写路径 SHALL 只在**槽负载落在页的数据区内**时写入该槽的字节；槽目录项的 `offset` / `length` 损坏时 SHALL 给出明确错误，SHALL NOT 产生越界切片 panic，也 SHALL NOT 改写页头或槽目录区。
- 实际：`update_version_header_in_data_page` 取到槽后，直接把槽的 `offset` 当作页内写起点，**无边界检查**：

  ```rust
  // src/storage/data_page.rs:266-273（update_version_header_in_data_page）
  let (slot, _) = slotted
      .get_slot_by_logical_id(row_id.slot_id)
      .ok_or(StorageError::SlotNotFound(row_id))?;
  let slot_offset = slot.offset as usize;

  let header_bytes = new_header.to_bytes();
  page.data[slot_offset..slot_offset + VersionHeader::SIZE].copy_from_slice(&header_bytes);
  ```

  `slot.offset` 是**页内字节**里的槽目录项字段（`Slot { logical_id, offset, length }`，`src/storage/page_format/slotted_page.rs:5-9`）⇒ 损坏后可以是 `0..=65535` 的任意值。

  ⇒ `slot_offset + 22 > 4096` 时 ⇒ **越界切片 panic**（发生在异步执行路径 ⇒ 进程 abort）；
  ⇒ `slot_offset` 落在页头（`< 16`）或槽目录区内时 ⇒ **静默改写**页头 / 槽目录项（不 panic）。

  写入长度固定为 `VersionHeader::SIZE = 22`（`src/transaction/version_chain.rs:119`），**不检查该槽自身的 `length`** ⇒ 槽负载短于 22 字节时，22 字节的写会越过该行自身的负载、覆盖相邻字节。
- 位置：
  - 根因实现点（主）：`src/storage/data_page.rs::update_version_header_in_data_page`（`:272`）。
  - 同族兄弟点：`src/storage/page_format/slotted_page.rs::SlottedPage::update_slot_in_place`（`:397-399`）——`self.page.data[start..end].copy_from_slice(data)`，`start = slot.offset as usize`、`end = start + slot.length as usize`，同样无边界检查；生产调用点 `src/storage/catalog.rs:924`（目录页定长字段改写）。
  - 常量：`Page::PAGE_SIZE = 4096`（`src/storage/page.rs:11`）、`VersionHeader::SIZE = 22`（`src/transaction/version_chain.rs:119`）、`SlottedPageHeader::SIZE = 16`（`slotted_page.rs:35`）、`Slot::SIZE = 6`（`:12`）。
- 根因：Confirmed（读码确认——写路径直接消费槽目录项的 `offset` 而不做范围判定；T3 已交付的载荷判界只服务**读**路径）。**未构造复现**：未注入损坏槽、未运行 ⇒ 不标「已复现」。
- 触发条件：槽目录项的 `offset` 字段损坏到越界（`offset + 22 > 4096`）或落到页头 / 槽目录区（`offset < 16`，或落在自页尾向上生长的槽目录区内）⇒ 该槽上的写操作 panic 或静默改写。

### 生产触达路径（本次逐点核对）

`update_version_header_in_data_page` 的调用点：

| 调用点 | 场景 | 暴露 |
|---|---|---|
| `src/storage/buffer_pool.rs:366` | 提交路径改写版本头（commit） | **暴露** |
| `src/transaction/manager.rs:514`、`:525` | 放弃 / 中止路径把版本头标记为 aborted | **暴露** |
| `src/executor/delete.rs:216` | DELETE 的墓碑补链指针 | **暴露** |
| `src/storage/data/table_manager.rs:1116` | 墓碑补链指针（表管理层） | **暴露** |
| `src/executor/upsert.rs:688` | UPSERT 的墓碑补链指针 | **暴露** |
| `src/wal/recovery.rs:1519` | 恢复面「把未提交事务标记为 aborted」 | **暴露**（恢复面；其**读**侧无界访问由 MS37 change 的 T5 承载） |

⇒ 该形态**不是**只在恢复面可达：**提交 / 放弃 / DELETE / UPSERT** 四条普通 DML 路径都会走到这一写。

## 影响

- 当前：写路径 panic ⇒ **进程 abort**；或（偏移落在页头 / 槽目录区时）**静默改写**页头 / 槽目录 ⇒ 后续读路径可能读到被改写的槽目录项（错读）。
- 与 ISS27 的关系（**同域不同面，不合并**）：ISS27 的实现点是 `get_slot_data` 的**读**侧无界切片（`SlottedPageRef::get_slot_data`，`slotted_page.rs:158-162`）⇒ 本条是**写**侧的同形无界访问（`page.data[slot_offset..slot_offset + 22]` 与 `update_slot_in_place` 的 `page.data[start..end]`）⇒ 触发字段相同（槽目录项的 `offset` / `length`），但**方向相反**（读 vs 写）、后果不同（abort vs 静默改写页头 / 槽目录）⇒ ISS27 的载体（`get_slot_data_checked`，**只判读**）**不覆盖**本条 ⇒ 本条独立承载、独立关闭。
  - T3 已交付的 `get_slot_data_checked`（`slotted_page.rs:170-177`）与 `slot_payload_in_bounds`（`:180-183`）**只服务读路径**；写路径没有任何等价判界。
- 与 ISS35 / ISS37 的关系：**不同机制**——ISS35（读路径）与 ISS37（写路径的页头槽数形态）的触发条件是页头的 `slot_count` 越界（**页头**形态），本条的触发条件是槽目录项的 `offset` / `length` 越界（**槽负载**形态）⇒ 不合并。
- 与恢复面（T5）的关系：恢复面**读**侧的无界访问（`recovery.rs:1490` 的 `get_slot_data` 与 `:1487` 的 `get_slot`）由 MS37 change 的 **T5** 承载；本条只主张恢复面**写**侧（`recovery.rs:1519` → `data_page.rs:272`）的同形无界写 ⇒ **不另开条目**；T5 的承载归属由 Plan 在 change 产物中记录。
- 相邻面（**本条不主张**，供 Plan 划定范围时判断）：可写变体 `SlottedPage::get_slot_data`（`:258-262`）是**读**侧同形无界读（生产调用点 `src/storage/catalog.rs:765`、`:809`、`:893`、`:974`、`:1652`）⇒ 属 ISS27 的目录页组（T4）承载面。
- 未爆发：本次执行未在真实库上观测到该 panic（未构造损坏槽）⇒ 可达性由读码确认，未经实测。

## 事件记录

- 2026-10-25（MS37 change Iteration 002 / T17 的 Act 阶段）：Act 把「可写变体的负载偏移无界写」作为 Issue 候选上报（`Experience Candidates` 段第 2 项的第 2 条），并把「恢复面可达」记为第 3 条候选（`recovery.rs:1519` 从损坏页的槽 `:1487` 取 `logical_id` 后调用 `update_version_header_in_data_page`）⇒ 本 Cycle 未修（Plan 的 F10 与 D12 裁定 ③ 显式排除）。
- 2026-10-25（本次登记）：Recorder 按用户指令独立核实（读码），确认：Act 候选所指 `src/storage/data_page.rs:272` 的 `page.data[slot_offset..slot_offset + VersionHeader::SIZE]` **无边界检查**（行号相对登记时工作区，与 Act 记录的 `:272` 一致）；`VersionHeader::SIZE = 22`；写路径**不检查**该槽的 `length`；`get_slot_data_checked` 的载荷判界只服务读路径；生产调用点覆盖 commit / abort / DELETE / UPSERT / 恢复面五条路径；同族兄弟点 `update_slot_in_place`（`slotted_page.rs:397-399`，生产调用点 `catalog.rs:924`）同样无边界检查 ⇒ 机制与候选描述一致；**未构造复现** ⇒ 根因标 Confirmed（读码确认）。

## 处置

- 处置指针：**当前为空**——上层已在同一 change `2026-10-25-ms37-defect-ledger-closure` 内分配 Iteration 承载（由 Plan 写入具体 Iteration / Task）。
- 承载落点建议（**只记录，不裁定**）：落在**写路径的槽负载有界写**——写入前用与读路径同源的判界（页数据区起点 / 槽目录区起点）确认 `[offset, offset + len)` 落在页的数据区内，否则给出**明确错误**；`StorageError::CorruptedDataPage { page_id, detail }` 是 T3 / T17 已冻结的错误面，可复用，**不新增错误变体**。
- 需要 Plan 判定的点：
  ① 判据放在**写入原语内部**（`data_page.rs:272` 与 `slotted_page.rs::update_slot_in_place` 两处分别加）还是抽到页格式层的一个共享判界原语（读路径的 `slot_payload_in_bounds` 当前是私有 `fn`，`slotted_page.rs:180`）；
  ② **写**路径是否也须校验槽负载长度 `slot.length ≥ VersionHeader::SIZE`（否则 22 字节的写会越过该行自身负载、覆盖相邻字节；读路径对「过短负载」已有既定语义，T17 的 Preserve 条款要求不改动它 ⇒ 写路径的新语义需独立判定）；
  ③ 与目录页组（T4）、恢复面组（T5）的范围划分；
  ④ 是否同时覆盖 `delete_tuple_from_data_page` 的可写变体路径（`data_page.rs:325-328`）上的同类写入面。
- 验收时注意：修复只覆盖**写路径的槽负载偏移** ⇒ **不得**据此声明「任何损坏页都不会 abort」（ISS27 的读侧无界读、ISS35 / ISS37 的页头槽数形态是另外的形态）。
- 与 ISS27 的交叉引用：`.agents/issues/ISS27-unbounded-slot-reads-other-page-types.md`（**读**侧无界读）⇒ 本条与它**同域不同面**，独立承载、独立关闭。
- 2026-10-26 关闭。原因：写路径的槽负载偏移无界写已由归档变更 Iteration 003 任务 T19 处置（版本头原地写与目录行原地改写改为有界写）。该 Cycle `iterations/003-write-path-slot-bounds/000-initial.md` 的 Plan Review 为 `accepted`。处置指针：fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`。

## 证据

- 2026-10-26 关闭依据：`openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure/iterations/003-write-path-slot-bounds/000-initial.md` 的 Plan Review 为 `accepted`（T19）。
- Issue 候选原文（不在本 Issue 复制）：`openspec/changes/2026-10-25-ms37-defect-ledger-closure/iterations/002-slot-count-underflow/000-initial.md` —— Act Response 的 `Experience Candidates` 段第 2 项的第 2 条与第 3 条（恢复面可达）；同文件 `Plan Context` 的 `Risks and Notes` 段（登记前的候选表述，行号 `:237`）。
- 机制核实（本次 Recorder 读码核对；revision `e9cb9d3` + MS37 在途改动；**行号相对登记时的工作区状态**）：
  - `src/storage/data_page.rs:255-274`（`update_version_header_in_data_page` 的闭包：取槽 → 取 `slot.offset` → 无界写）、`:266-268`（`get_slot_by_logical_id` 与 `SlotNotFound` 语义）、`:325-328`（同函数内可写变体的删槽路径）。
  - `src/storage/page_format/slotted_page.rs:5-9`（`Slot { logical_id, offset, length }`）、`:158-162`（只读变体 `get_slot_data`，无界读）、`:170-177`（T3 的 `get_slot_data_checked`，**只判读**）、`:180-183`（私有 `slot_payload_in_bounds`）、`:258-262`（可写变体 `get_slot_data`，无界读）、`:384-402`（`update_slot_in_place`；`:389` 的长度相等检查；`:397-399` 的无界写）、`:12`（`Slot::SIZE = 6`）、`:35`（`SlottedPageHeader::SIZE = 16`）。
  - `src/transaction/version_chain.rs:119`（`VersionHeader::SIZE = 22`）。
  - `src/storage/page.rs:11`（`Page::PAGE_SIZE = 4096`）。
  - 生产调用点：`src/storage/buffer_pool.rs:366`、`src/transaction/manager.rs:514`、`:525`、`src/executor/delete.rs:216`、`src/storage/data/table_manager.rs:1116`、`src/executor/upsert.rs:688`、`src/wal/recovery.rs:1519`。
  - 同族兄弟点的生产调用点：`src/storage/catalog.rs:924`（`update_slot_in_place`）；可写变体 `get_slot_data` 的调用点：`src/storage/catalog.rs:765`、`:809`、`:893`、`:974`、`:1652`。
- **未确认项**：① 未构造损坏槽、未运行 ⇒ panic / 静默改写的具体表现与文案为**读码推断**；② 「22 字节的写越过短负载、覆盖相邻字节」的实际后果依赖同页相邻行的布局，未实测；③ 目录页组的 `update_slot_in_place` 路径与 T4 的承载面重叠程度由 Plan 判定。
