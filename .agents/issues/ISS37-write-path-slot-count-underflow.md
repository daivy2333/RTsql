# 写路径的槽目录容量下溢：页头 `slot_count` 越界时 `add_slot_inner` / `free_space` 的 `PAGE_SIZE - slot_count * Slot::SIZE` 整数下溢 ⇒ 写路径 panic

- Status: closed（2026-10-26 fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`）
- Filed: 2026-10-25
- Source: Act Response 的 Issue 候选（`Experience Candidates` 段第 2 项的第 1 条「写路径的同形无界访问」），change `2026-10-25-ms37-defect-ledger-closure` Iteration 002 Cycle `iterations/002-slot-count-underflow/000-initial.md`；同 Cycle 的 `Plan Context` 的 `Risks and Notes` 段（登记前的候选表述）
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 / cargo 1.99.1、RTsql 仓库 HEAD `e9cb9d3`（产品改动未提交）；登记期间工作区含 MS37 change Iteration 000 / 001 / 002 的在途实施改动 ⇒ 本条的行号相对**登记时的工作区状态**；本条为**读码确认**，未构造复现

## 缺陷描述

- 预期：写路径 SHALL NOT 因页头损坏产生整数下溢 / 越界切片 / 进程 abort；页头声称的槽数超出槽目录容量时，写路径 SHALL 给出明确错误（或按该页型的既定语义处理），SHALL NOT 继续写入。
- 实际：页格式层的**写入原语**直接用页头的 `slot_count` 计算槽目录边界与剩余空间，**无下溢保护**：

  ```rust
  // src/storage/page_format/slotted_page.rs:294-299（SlottedPage::add_slot_inner）
  let free_space_start = self.header.free_space_offset as usize;
  let free_space_end = Page::PAGE_SIZE - self.slot_count() * Slot::SIZE;   // ← 无下溢保护
  if free_space_start + needed_space > free_space_end {
      return Err("No enough space in page".to_string());
  }
  ```

  页头 `slot_count` 损坏到 `≥ 683` 时，`4096 - 683 * 6 = 4096 - 4098` **整数下溢**。

  同形点：`SlottedPage::free_space`（`:405-409`）的同一表达式（`:407`）与随后的 `free_space_end - free_space_start`（`:408`）。
- 位置：
  - 根因实现点：`src/storage/page_format/slotted_page.rs::SlottedPage::add_slot_inner`（`:295` 空间边界、`:306` 槽目录位置）、`::SlottedPage::free_space`（`:407`、`:408`）。
  - 同文件同形兄弟点（登记为**相邻面**，见「影响」段）：`::SlottedPage::delete_slot`（`:349`、`:350`、`:357` 的 `PAGE_SIZE - (i + 2) * Slot::SIZE` 与 `PAGE_SIZE - count * Slot::SIZE`）。
  - 常量：`Page::PAGE_SIZE = 4096`（`src/storage/page.rs:11`）、`Slot::SIZE = 6`（`slotted_page.rs:12`）、`SlottedPageHeader::SIZE = 16`（`:35`）、`MAX_PAGE_SLOTS = 680`（`:21`，MS37 T17 迁入的**全项目唯一定义**）。
  - 阈值：下溢阈值 `slot_count ≥ 683`（`4096 / 6 = 682.67`）；页级判据阈值 `slot_count ≥ 681`（`MAX_PAGE_SLOTS + 1`）。
- 根因：Confirmed（读码确认——两处表达式直接消费页头 `slot_count` 且无下溢保护；阈值 683 由常量算术推出，与页级判据阈值 681 同源）。**未构造复现**：未注入损坏页、未运行 ⇒ 不标「已复现」。
- 触发条件：页头 `slot_count ≥ 683`（`slot_count` 为 `u16`，损坏后取值范围宽）⇒ 在该页上调用 `add_slot` / `add_slot_with_logical_id` 的写路径在 `add_slot_inner` 内 panic；调用 `free_space()` 时同样下溢。

### 生产触达路径（本次逐点核对）

| 触点 | 位置 | 写操作 | 暴露 |
|---|---|---|---|
| 数据页尾页追加 | `src/storage/data_page.rs:99`（`insert_tuple_to_data_page` 落尾页）、`:119`（尾页已满时的新页） | `slotted.add_slot(&slot_data)` | **暴露**——尾页是所有后续 INSERT 的落点 ⇒ 尾页页头损坏 ⇒ **每次**后续 INSERT 都在写路径 panic |
| 目录页链追加 | `src/storage/catalog.rs:707`（`append_to_chain`） | `slotted.add_slot(payload)` | **暴露**（目录页写路径） |
| 恢复面强制落位 | `src/wal/recovery.rs:207` | `slotted.add_slot_with_logical_id(&slot_data, row_id.slot_id)` | **暴露**（恢复面；恢复面自身的**读**侧无界访问由 MS37 change 的 T5 承载） |
| 索引树写入 | `src/storage/btree/node.rs:225`、`:298`、`:824`、`:931` | `add_slot` | **不暴露**——页级闸门 `ensure_index_page_intact`（`node.rs:61`）经 `integrity::check_tree_page` ⇒ `validate_slots` 先行 ⇒ `slot_count ≥ 681` 的页在闸门处即被拒 |

`free_space()` 的调用点：`src/storage/btree/node.rs:212`、`:286`、`:402`、`:814`、`:921`（同上，受闸门保护）与 `src/storage/catalog.rs:1934`（位于 `#[cfg(test)] mod tests`（`catalog.rs:1739`）内的 `suffocate_page` ⇒ **非生产路径**）⇒ **`free_space()` 当前没有无保护的生产调用点**；该点按**同形原语缺保护**登记，不按「当前可达的崩溃」登记。

### 两种 profile 的表现（读码推断，未经实测）

- **debug**（`overflow-checks` 默认开启）：减法处直接 panic，文案 `attempt to subtract with overflow`（与 T17 实测的 `slotted_page.rs:232` 同类形态）。
- **release**（`Cargo.toml` 的 `[profile.release]` 未设 `overflow-checks` ⇒ 默认关闭）：减法**回绕**成巨大的 `usize` ⇒ 第 297 行的空间检查 `free_space_start + needed_space > free_space_end` **被击穿**（不再返回 `Err("No enough space in page")`）⇒ 继续执行第 302 行的数据写与第 306 / 315 行的槽写；第 306 行 `Page::PAGE_SIZE - (slot_index + 1) * Slot::SIZE`（`slot_index = self.slot_count()`）同样下溢 ⇒ 第 315 行 `self.page.data[slot_start..slot_start + Slot::SIZE].copy_from_slice(...)` 越界切片 panic。
  ⇒ **两种 profile 都会 panic**；差别在于 release 下「空间不足」的既有错误语义被绕过。

## 影响

- 当前：写路径 panic ⇒ **进程 abort**（panic 发生在异步执行路径）——INSERT / 目录页追加 / 恢复面重放可被单个损坏页头终止；尾页页头损坏时**每一次**后续 INSERT 都失败。
- 与 ISS35 的关系（**同族不同面，不合并**）：ISS35 主张**读路径**的槽目录定位下溢（`PAGE_SIZE - (index + 1) * Slot::SIZE`，实现点 `SlottedPageRef::get_slot` / `SlottedPage::get_slot`），本条主张**写路径**的空间边界计算下溢（`PAGE_SIZE - slot_count * Slot::SIZE`，实现点 `add_slot_inner` / `free_space`）⇒ 触发条件相同（页头 `slot_count` 越界）、根因相同（写路径不消费页级判据），但**实现点、表达式、后果与承载面不同**；ISS35 台账正文明确「本条只主张读路径」并把 `add_slot_inner` / `free_space` 列为「相邻面（本条不主张）」⇒ 本条独立承载、独立关闭。
  - **ISS35 的载体（T17）不覆盖本条**：T17 在数据页组的 **12 个取槽点**之前消费页级判据（`slot_count_exceeds_capacity`），而 `add_slot_inner` / `free_space` 是**页格式层的写入原语**，不经那 12 个取槽点 ⇒ T17 未在这两处加判据。
- 与 ISS27 的关系：**不重叠**——ISS27 是**读**侧的无界切片（`get_slot_data`），本条是**写**侧的空间计算下溢。
- 与本台账另一条目（ISS38，负载偏移无界写）的关系：**不合并**——ISS38 的触发条件是槽目录项的 `offset` / `length` 越界（**槽负载**形态），本条的触发条件是页头的 `slot_count` 越界（**页头**形态）；两者的根因位置、检测判据与承载面不同。
- 相邻面（**本条不主张**，供 Plan 划定范围时判断）：
  - `SlottedPage::delete_slot`（`:349`、`:350`、`:357`）的同形减法——在当前调用链上被可写变体 `get_slot`（`:225-243`）的同形下溢先行打断 ⇒ 属**同族原语**，是否纳入同一 Iteration 由 Plan 判定。
  - `SlottedPage::update_slot_in_place`（`:397-399`）的**负载偏移**无界写——属**槽负载**形态，由本台账 ISS38 承载。
- 未爆发：本次执行未在真实库上观测到该 abort（未构造损坏页）⇒ 可达性由读码确认，未经实测。

## 事件记录

- 2026-10-25（MS37 change Iteration 002 / T17 的 Act 阶段）：Act 在实施 T17（页头槽数越界的数据页读 / 写路径判据）后，把「写路径的同形无界访问」作为 Issue 候选上报（`Experience Candidates` 段第 2 项的第 1 条），并按 Plan 的 D12 裁定 ③ 明确**不在本 Cycle 实施**（ISS35 只主张读路径）。同 Cycle 的 `Plan Context` 的 `Risks and Notes` 段已把该形态列为范围外候选。
- 2026-10-25（本次登记）：Recorder 按用户指令独立核实（读码），确认：两处表达式与 Act 候选一致（`add_slot_inner` `:295`、`free_space` `:407`，行号相对登记时工作区）；下溢阈值 683；页级判据阈值 681；T17 的 12 个取槽点**不覆盖**这两处写入原语；生产触达路径为 `data_page.rs:99` / `:119`、`catalog.rs:707`、`recovery.rs:207`（索引树侧受闸门保护）；`free_space()` 当前无无保护的生产调用点（`catalog.rs:1934` 位于 `#[cfg(test)]`）⇒ 机制与候选描述一致；**未构造复现** ⇒ 根因标 Confirmed（读码确认）。

## 处置

- 处置指针：**当前为空**——上层已在同一 change `2026-10-25-ms37-defect-ledger-closure` 内分配 Iteration 承载（由 Plan 写入具体 Iteration / Task）。
- 承载落点建议（**只记录，不裁定**）：落在**页格式层写入原语的有界化**——`add_slot_inner` / `free_space` 在使用页头 `slot_count` 之前先问 T17 已冻结的页级判据 `slot_count_exceeds_capacity()`（`slotted_page.rs:89-91`，判定实现全项目唯一），页头损坏时给出**明确错误**而非下溢 panic。
- 需要 Plan 判定的点：
  ① 判据放在**原语内部**（`add_slot_inner` / `free_space`，覆盖数据页 / 目录页 / 恢复面全部调用点）还是放在**各调用点**（`data_page.rs` / `catalog.rs` / `recovery.rs`）；
  ② `free_space()` 的返回类型是 `usize`（**无错误通道**）⇒ 损坏页在该 API 上如何表达（返回 0 / 饱和值 / 改签名）——这是与 ISS35 不同的接口约束；
  ③ `add_slot_inner` 的错误通道是 `Result<_, String>`（`add_slot` / `add_slot_with_logical_id` 均为 `String`）⇒ 是否改为结构化错误（`page_id` 在 `SlottedPage::page_id()`（`:423`）可得）；
  ④ 是否把 `delete_slot` 的同形减法一并纳入（T17 冻结的「判定实现只有一处」不覆盖它）；
  ⑤ 与 T5（恢复面组，`src/wal/recovery.rs`）的范围划分，避免同一处被两个 Iteration 重复改动。
- 验收时注意：修复只覆盖**写路径的空间 / 槽目录计算** ⇒ **不得**据此声明「任何损坏页都不会 abort」（ISS27 的槽负载越界、ISS38 的负载偏移无界写是另外的形态）。
- 与恢复面（T5）的关系：恢复面**读**侧的无界访问由 MS37 change 的 **T5** 承载；本条只主张恢复面**写**侧（`recovery.rs:207` → `add_slot_with_logical_id`）的同形下溢 ⇒ **不另开条目**；T5 的承载归属由 Plan 在 change 产物中记录。
- 与 ISS35 的交叉引用：`.agents/issues/ISS35-slot-count-underflow-aborts-read-path.md`（读路径的页头槽数下溢，已由 MS37 change Iteration 002 / T17 实施）⇒ 本条与它**同族不同面**，独立承载、独立关闭。
- 2026-10-26 关闭。原因：写路径的页头槽数下溢已由归档变更 Iteration 003 任务 T18 处置（落位 / 删槽 / 原地改写在页级判据之前拒绝，页损坏不换页继续）。该 Cycle `iterations/003-write-path-slot-bounds/000-initial.md` 的 Plan Review 为 `accepted`。处置指针：fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`。

## 证据

- 2026-10-26 关闭依据：`openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure/iterations/003-write-path-slot-bounds/000-initial.md` 的 Plan Review 为 `accepted`（T18）。
- Issue 候选原文（不在本 Issue 复制）：`openspec/changes/2026-10-25-ms37-defect-ledger-closure/iterations/002-slot-count-underflow/000-initial.md` —— Act Response 的 `Experience Candidates` 段第 2 项的第 1 条；同文件 `Plan Context` 的 `Risks and Notes` 段。
- 机制核实（本次 Recorder 读码核对；revision `e9cb9d3` + MS37 在途改动；**行号相对登记时的工作区状态**）：
  - `src/storage/page_format/slotted_page.rs:288-299`（`add_slot_inner` 的空间边界与检查）、`:302`（数据写）、`:305-315`（槽位置与槽写）、`:405-409`（`free_space`）、`:340-367`（`delete_slot` 的同形减法）、`:12`（`Slot::SIZE = 6`）、`:21`（`MAX_PAGE_SLOTS = 680`）、`:35`（`SlottedPageHeader::SIZE = 16`）、`:89-91`（`slot_count_exceeds_capacity`，判定实现唯一）、`:423`（`page_id()`）。
  - `src/storage/page.rs:11`（`Page::PAGE_SIZE = 4096`）。
  - 生产触点：`src/storage/data_page.rs:99`、`:119`（INSERT 落尾页 / 新页）、`src/storage/catalog.rs:707`（目录页链追加）、`src/wal/recovery.rs:207`（恢复面强制落位）。
  - `free_space()` 调用点：`src/storage/btree/node.rs:212`、`:286`、`:402`、`:814`、`:921`；`src/storage/catalog.rs:1934`（`#[cfg(test)] mod tests`，`catalog.rs:1739`）。
  - 索引树闸门：`src/storage/btree/node.rs:61`（`ensure_index_page_intact`）⇒ `integrity::check_tree_page` ⇒ `validate_slots`（`src/storage/integrity.rs:538-550`，`slot_count ≥ 681` 即记 violation 并提前返回 `None`）。
  - `Cargo.toml` 的 `[profile.release]` 未设 `overflow-checks` ⇒ release 下减法回绕（默认 `overflow-checks = false`）。
- **未确认项**：① 未构造损坏页、未运行 ⇒ debug / release 的具体 panic 文案与「两种 profile 都会 panic」为**读码推断**（T17 实测的同类 abort 形态为 debug `attempt to subtract with overflow` @ `slotted_page.rs:232`，可作形态旁证，但不等于本条两处的实测）；② `slot_count ≥ 683` 之外是否还有其它触发形态（如 `free_space_offset` 越界）未穷举；③ `free_space()` 是否存在非索引树、非测试的生产调用方（本次全仓检索未见，但调用点随代码演进可能变化）。
