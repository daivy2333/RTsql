# 页头 `slot_count` 越界时槽目录定位整数下溢，读路径在 `SlottedPageRef::get_slot` 内 panic 并终止进程

- Status: closed（2026-10-26 fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`）
- Filed: 2026-10-25
- Source: Plan Context 的 Issue 候选（「页头 `slot_count` 越界导致的读路径 abort 不在本 change 承载的缺陷形态内」），change `2026-10-25-ms37-defect-ledger-closure` Iteration 001 Cycle `iterations/001-bounded-read-other-page-types/000-initial.md` 的 `Risks and Notes` 段
- Environment: Linux x86_64 / WSL2 6.18.40.1、rustc 1.99.1 / cargo 1.99.1、RTsql 仓库 HEAD `e9cb9d3`；登记期间工作区正被 MS37 Iteration 001 的 T3 实施改动（`src/storage/page_format/slotted_page.rs` 等）⇒ 本条的行号相对**登记时的工作区状态**；本条为**读码确认**，未构造复现

## 缺陷描述

- 预期：页内容读取 SHALL 经由有边界检查的路径；页头损坏（如 `slot_count` 超过页容量）时读取 SHALL 给出明确错误或按该页型的既定语义处理，SHALL NOT 造成整数下溢 / 越界切片 / 进程 abort。
- 实际：`SlottedPageRef::get_slot` 用页头的 `slot_count` 参与槽目录位置计算，**无下溢保护**：

  ```rust
  // src/storage/page_format/slotted_page.rs:96-110
  pub fn get_slot(&self, index: usize) -> Option<Slot> {
      if index >= self.slot_count() { return None; }
      let slot_start = Page::PAGE_SIZE - (index + 1) * Slot::SIZE;   // ← 无下溢保护
      let slot_buf = &self.data[slot_start..slot_start + Slot::SIZE];
      ...
  }
  ```

  读路径按页头的 `slot_count` 迭代槽 ⇒ `slot_count ≥ 683` 时，`index = 682` 处 `4096 - 683 * 6 = 4096 - 4098` **整数下溢** ⇒ 紧接着的切片下标越界 ⇒ panic（发生在异步执行路径 ⇒ 进程 abort）。
- 位置：
  - 根因实现点：`src/storage/page_format/slotted_page.rs::SlottedPageRef::get_slot`（`:96-110`）；可写变体 `SlottedPage::get_slot`（`:183-201`）同形。
  - 读路径入口（迭代上界即页头值）：`src/storage/page_format/slotted_page.rs::SlottedPageRef::get_slot_by_logical_id`（`:113-122`，`for i in 0..self.slot_count()`）；生产调用点见「影响」段清单。
  - 常量：`Page::PAGE_SIZE = 4096`（`src/storage/page.rs:11`）；`Slot::SIZE = 6`（`src/storage/page_format/slotted_page.rs:12`）；`SlottedPageHeader::SIZE = 16`（`:27`）。
- 根因：Confirmed（读码确认——`get_slot` 的减法无下溢保护，迭代上界取自页头 `slot_count`；下溢阈值 683 由常量算术推出）。**未构造复现**：未注入损坏页、未运行 ⇒ 不标「已复现」。
- 触发条件：页头 `slot_count ≥ 683`（`slot_count` 为 `u16`，损坏后取值范围宽）⇒ 任何按该页头迭代槽的读路径在 `get_slot` 内 panic。

### 阈值与既有判据的关系（读码确认）

| 判据 | 阈值 | 位置 |
|---|---|---|
| 实际下溢 | `slot_count ≥ 683`（`index = 682` 起 `4096 - (index+1)*6 < 0`） | `slotted_page.rs:100` |
| 页级判据 `validate_slots` | `slot_count ≥ 681` 即记 violation「slot count N exceeds the page capacity」并**提前返回 `None`**（不进入槽循环） | `src/storage/integrity.rs:538-550` |
| 索引树访问器的既有界限 | `index ≥ MAX_PAGE_SLOTS`（`= (4096-16)/6 = 680`）即返回 `None`（**读槽之前**判界） | `src/storage/btree/node.rs:14`、`:31-40`、`:43-52` |

⇒ **页级判据已能发现该页损坏**（`check_data_page` `integrity.rs:502` / `check_tree_page` `:594` 的消费者：释放路径 `src/storage/data/table_manager.rs:1736`、重建路径 `:1928`、`rtsql check` 扫描面 `src/storage/integrity.rs:271`），**但读路径不消费该判据** ⇒ 只有读路径暴露该 abort；完整性扫描能报出该损坏，但**不阻止**读路径 abort。

### 与仓库内既有实现的对照

`src/storage/btree/node.rs:11-13` 的注释**已明确记载该缺陷形态**：「页头声称的槽数超过该上限时，槽目录定位会下溢成越界切片，故访问器在读槽**之前**先做此界限判断。」⇒ 该形态**不是未知风险**，而是**已知但只在索引树一侧有防护**的缺陷：索引树的 `bounded_slot_data_ref` / `bounded_slot_data_mut` 有 `MAX_PAGE_SLOTS` 前置界限，而 `SlottedPageRef::get_slot` 本身与其余页型的读路径**没有**该界限。

## 影响

- 当前：**进程 abort**（panic 发生在异步执行路径）——数据页 / 目录页 / 恢复面 / 缓冲池 / executor 与事务的读路径都可被单个损坏页头终止。
- 与 ISS27 的关系（**同域不同机制，不合并**）：ISS27 的实现点是 `get_slot_data` 的槽 `offset` / `length` 越界 ⇒ 越界切片；本条是**页头 `slot_count` 越界** ⇒ 槽目录位置计算下溢，发生在**读槽目录项本身**这一步，**早于**槽负载读取 ⇒ ISS27 的修复载体**不覆盖**本条 ⇒ 须独立承载、独立关闭。
  - 已在实施中的 ISS27 载体**确实不覆盖本条**（本次登记时读码核对）：`SlottedPageRef::get_slot_data_checked`（`slotted_page.rs:137-144`，MS37 Iteration 001 的 T3 正在新增）接收**已经取到的** `&Slot`、只判**槽负载**是否越过数据区（`slot_payload_in_bounds` `:147-150`）⇒ 槽目录位置的计算发生在它之前的 `get_slot` 里 ⇒ **该载体不消除本条的 abort**。
- 读路径暴露面（生产读点，按页头 `slot_count` 迭代并调用 `get_slot` / `get_slot_by_logical_id`；行号为本次逐点核对所得，**相对登记时的工作区状态**——`src/storage/data_page.rs`、`src/storage/buffer_pool.rs`、`src/storage/data/table_manager.rs`、`src/executor/data_scan.rs`、`src/executor/create_index.rs` 在登记期间被在途 T3 实施改动，后续仍可能位移）：
  - `src/storage/data_page.rs:167`（`read_tuple_from_data_page`）、`:234`、`:265`
  - `src/storage/buffer_pool.rs:425`（快路径）、`:486`（主取行循环）、`:641`（`check_page_all_visible`）
  - `src/storage/catalog.rs:764`、`:808`、`:890`、`:965`、`:1649`
  - `src/wal/recovery.rs:166`、`:371`、`:459`、`:1174`、`:1487`
  - `src/executor/data_scan.rs:254`、`:476`
  - `src/executor/create_index.rs:134`
  - `src/transaction/manager.rs:567`
  - `src/storage/data/table_manager.rs:1001`、`:1258`
  - 索引树**不受影响**（`MAX_PAGE_SLOTS` 前置界限，`node.rs:31-52`）。
  - `src/storage/data/table_manager.rs:2768`（`count_live_rows`）位于 `#[cfg(test)]` 模块内 ⇒ 不是生产读点。
- 相邻面（**本条不主张**，供 Plan 划定范围时判断）：可写变体 `SlottedPage::get_slot`（`slotted_page.rs:183-201`）与 `add_slot_inner` 的空间检查（`:253` 的 `Page::PAGE_SIZE - self.slot_count() * Slot::SIZE`）、`free_space()`（`:365` 的同形减法）同形 ⇒ 写路径在同样的损坏页头上亦可下溢；本条只主张读路径。
- 未爆发：本次执行未在真实库上观测到该 abort（未构造损坏页）⇒ 可达性由读码确认，未经实测。

## 事件记录

- 2026-10-25（MS37 change Iteration 001 的 Plan 阶段）：Plan 在为 ISS27 的其余页型组定稿「有边界检查的槽负载读取原语」时读码发现该形态不在 `get_slot_data_checked` 的覆盖范围内 ⇒ 作为 Issue 候选报告，登记与关闭由 Recorder 按用户指令执行。该 Cycle 明确记录：**Act 不得把「本 Cycle 后任何损坏页都不会 abort」当作已完成结论**。
- 2026-10-25（本次登记）：Recorder 按用户指令独立核实机制（读码），确认下溢点、阈值 683、既有判据阈值 681、索引树前置界限 680 三者的关系，并确认「页级判据可发现、读路径不消费」⇒ 机制与候选描述一致；**未构造复现** ⇒ 根因标 Confirmed（读码确认）。
- 2026-10-25（MS37 change Iteration 002 / T17 的 **Act** 阶段——实施与验证结论；来源 `openspec/changes/2026-10-25-ms37-defect-ledger-closure/iterations/002-slot-count-underflow/000-initial.md` 的 `Act Response` 段，Act Response 状态 `reported`）：
  - **RED（真实复现进程 abort）**——可写变体取槽点（#2）：

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
  - **RED 的形态偏差（Act 记录，与 Plan 的读码预期不同）**：真实 SQL 入口（全表扫描 / 点查）在变更前**不是** abort，而是先命中 T3 已交付的逐槽越界判据（`slot_area_start = PAGE_SIZE.saturating_sub(slot_count * Slot::SIZE)` 在 `slot_count ≥ 683` 时饱和为 0 ⇒ 任何槽的负载都判越界）⇒ 返回**逐槽形态**的 `CorruptedDataPage`（`corrupted data page 2: slot 0 range [16, 56) lies outside the page data area`）⇒ 该用例的 RED 是「错误形态不符（断言失败）」；**两种 RED 都是「目标行为未达成」的失败，断言未放宽**。
  - **GREEN**：`cargo test --test data_page_bounded_read_test -- --test-threads=1` ⇒ `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s`。
  - **单元与库面**：`cargo test --lib` ⇒ `test result: ok. 420 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.81s`。
  - **定向回归**（20 个目标，M19 分批串行纪律：批内 `--test-threads=1`、批间串行、批 1 全绿后才启动批 2；目标集合与 Cycle 清单完全一致）⇒ 批 1（10 目标）**153 passed / 0 failed**；批 2（10 目标）**201 passed / 0 failed**；合计 **354 passed / 0 failed**。
  - **结构性判据（读码核对）**：`MAX_PAGE_SLOTS = 680` 的**唯一定义**在 `src/storage/page_format/slotted_page.rs:21`（`src/storage/btree/node.rs` 只导入，判据代码逐字保留 ⇒ 索引树路径零变化）；`slot_count_exceeds_capacity` 的**判定实现唯一一处**（`SlottedPageHeader::slot_count_exceeds_capacity`，`:89-91`，阈值 `slot_count > MAX_PAGE_SLOTS` = `slot_count ≥ 681`），**消费点恰好 12 处**（data_scan ×2、create_index ×1、transaction/manager ×1、table_manager ×2、data_page ×3、buffer_pool ×3）；`get_slot` / `get_slot_by_logical_id` 的签名与 `None` 语义**逐字未动**。
  - **承载范围与不关闭理由**：数据页组（12 个取槽点）已闭合；**目录页组（T4）与恢复面组（T5）在页头槽数越界时仍会 abort**（与 Cycle 000 结束时一致，**不是**本 Cycle 引入的回归）⇒ 本条**保持 `Status: open`**，关闭待该 change 的最终 Review Result 为 `accepted`。
  - **交叉引用**：写路径的同形无界访问已另立台账 **ISS37**（页头槽数下溢：`add_slot_inner` / `free_space`）与 **ISS38**（槽负载偏移无界写：`data_page.rs:272`）⇒ 本条的 T17 载体**不覆盖**这两条；恢复面（T5）的承载范围不变。

## 处置

- 处置指针：**已指向 MS37 change `2026-10-25-ms37-defect-ledger-closure` 的 Iteration 002（任务 T17）——已实施**（Act Response 状态 `reported`，见 `iterations/002-slot-count-underflow/000-initial.md`）。本条**保持 `Status: open`**：关闭须等该 change 的最终 Review Result 为 `accepted`（其中包含 T4 目录页组与 T5 恢复面组——Act Response 的 `Remaining Issues` 段记载的关闭前置即「T4 / T5 完成」）；**关闭由收尾时的 Recorder 按用户指令执行，本次不关闭**。
  （登记时的指针为「当前为空——上层已在同一 change 内分配 Iteration 承载」，见下方事件记录。）
- 承载落点建议（供 Plan 判断，本条不裁定）：落在**槽目录定位的有界化**——把「读槽之前先判槽目录容量」这一**仓库内已有的判界形态**（`src/storage/btree/node.rs:14` 的 `MAX_PAGE_SLOTS` 与 `:11-13` 的注释）提升到 `SlottedPageRef::get_slot` / `get_slot_by_logical_id` 或其调用点，使页头 `slot_count` 越界在读路径给出明确错误而非下溢 abort。
- 需要 Plan 判定的点：① 修复位置放在 `get_slot` 内部（覆盖所有调用点，含写路径与 `get_slot_by_logical_id`）还是放在各调用点（保持 `get_slot` 现有签名）；② 损坏页在该页型的正确行为（明确错误 vs 计数 + 报出 + 跳过）——须与 ISS27 的按页型分组判定（`design.md` D7）一致，且与 `page-read-bounded-read` 既有场景不冲突；③ 是否把可写变体与 `add_slot_inner` 的同形空间检查纳入同一 Iteration。
- 三个待判点的裁定结果（由 change `design.md` D12 记录，2026-10-25）：① **页格式层提供唯一的容量界限 + 读点在取槽之前消费页级判据**，**不改 `get_slot` 的 `None` 语义**（把越界折叠成 `None` 会与「槽不存在」不可区分 ⇒ 在尚未纳入判据的读点上把 abort 变成静默跳过）；② 数据页组 = 明确的 `StorageError::CorruptedDataPage { page_id, detail }`（点名该页与页头声称的槽数与容量上限），目录页组 = T4 的裁定，恢复面组 = T5 的两类语义；③ **不纳入**——ISS35 只主张读路径 ⇒ 写路径的同形无界访问另立台账（见下方交叉引用）。
- 与写路径同族台账的交叉引用（2026-10-25 登记）：`.agents/issues/ISS37-write-path-slot-count-underflow.md`（**写路径**的页头槽数下溢：`add_slot_inner` / `free_space` 的 `PAGE_SIZE - slot_count * Slot::SIZE`，生产触点含 INSERT 落尾页、目录页链追加、恢复面强制落位）与 `.agents/issues/ISS38-write-path-slot-payload-offset-unbounded.md`（**写路径**的槽负载偏移无界写：`data_page.rs:272` 的版本头就地写未校验槽偏移）⇒ 两条与本条**同族不同机制**，独立承载、独立关闭；本条的 T17 载体**不覆盖**这两条。
- 验收时注意：修复只覆盖**槽目录定位**这一形态 ⇒ **不得**据此声明「任何损坏页都不会 abort」（ISS27 的槽负载越界是另一形态，由 `get_slot_data_checked` 承载）。
- 与 ISS27 的交叉引用：`.agents/issues/ISS27-unbounded-slot-reads-other-page-types.md`（实现点 `get_slot_data`、触发条件为槽 `offset` / `length` 越界）⇒ 本条与它**同域不同机制**，独立承载、独立关闭。
- 2026-10-26 关闭。原因：本条自己写的关闭前置已经满足。数据页组由 Iteration 002 任务 T17 处置；目录页组与恢复面组由 Iteration 004（T4）和 Iteration 005（T5）处置。这三个 Cycle 以及该变更最后一轮 Iteration 009 的 Plan Review 均为 `accepted`，变更已归档。上文「保持 open、本次不关闭」保留为当时记录。处置指针：fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`。

## 证据

- 2026-10-26 关闭依据：`openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure/iterations/002-slot-count-underflow/000-initial.md`、`iterations/004-catalog-bounded-read/000-initial.md`、`iterations/005-recovery-bounded-read/000-initial.md` 的 Plan Review 均为 `accepted`（T17 / T4 / T5）。最后一轮 `iterations/009-corpus-and-full-regression/000-initial.md` 的 Plan Review 亦为 `accepted`。
- Issue 候选原文（不在本 Issue 复制）：`openspec/changes/2026-10-25-ms37-defect-ledger-closure/iterations/001-bounded-read-other-page-types/000-initial.md` 的 `Risks and Notes` 段（Issue 候选「页头 `slot_count` 越界导致的读路径 abort 不在本 change 承载的缺陷形态内」）。
- 机制核实（本次 Recorder 读码核对；revision `e9cb9d3` + MS37 Iteration 000 未提交改动；**行号相对登记时的工作区状态**，`src/storage/page_format/slotted_page.rs` 在登记期间正被 MS37 Iteration 001 的 T3 实施改动（新增 `get_slot_data_checked`），下述行号已按改动后的现状核对）：
  - `src/storage/page_format/slotted_page.rs:96-110`（`get_slot` 无下溢保护）、`:113-122`（`get_slot_by_logical_id` 按 `slot_count` 迭代）、`:183-201`（可写变体 `SlottedPage::get_slot` 同形）、`:12`（`Slot::SIZE = 6`）、`:27`（`SlottedPageHeader::SIZE = 16`）、`:253`（`add_slot_inner` 的同形空间减法）、`:365`（`free_space()` 的同形减法）。
  - `src/storage/page_format/slotted_page.rs:137-144`（T3 新增的 `get_slot_data_checked`）、`:147-150`（`slot_payload_in_bounds`）——证明在实施的 ISS27 载体**只判槽负载**、**不覆盖**槽目录定位。
  - `src/storage/page.rs:11`（`Page::PAGE_SIZE = 4096`）。
  - `src/storage/integrity.rs:538-550`（`validate_slots` 的容量判据与「slot count N exceeds the page capacity」violation，**提前返回 `None`**）、`:502`（`check_data_page`）、`:594`（`check_tree_page`）。
  - `src/storage/btree/node.rs:11-14`（`MAX_PAGE_SLOTS` 与「下溢成越界切片」的既有记载）、`:31-40`、`:43-52`（读槽**之前**的前置界限）。
  - 读路径暴露面：上列生产读点的行号均为本次逐点核对所得。
  - `Cargo.toml:79-84`：`[profile.release]` 未设 `overflow-checks` ⇒ release 下减法回绕（默认 `overflow-checks = false`）。
- 既有判据的消费者：释放路径 `src/storage/data/table_manager.rs:1736`、重建路径 `:1928`、`rtsql check` 扫描面 `src/storage/integrity.rs:271`（本次读码核对）。
- **未确认项**：① 未构造损坏页、未运行 ⇒ panic 的具体文案与 profile 差异未实测（读码推断：debug 在减法处 panic；release 下减法回绕为 `usize` 大值，随后 `&self.data[slot_start..slot_start + Slot::SIZE]` 的下标同样越界 ⇒ **两种 profile 都会 panic**——该推断**未经运行验证**，且与 Plan Context「release 下回绕成巨大的 usize，debug 下直接 panic」的措辞相比，此处更严：release 并非不 panic）；② `slot_count ≥ 683` 之外是否还有其它触发形态未穷举。
