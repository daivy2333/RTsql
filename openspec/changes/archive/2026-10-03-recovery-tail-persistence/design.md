# Design — WAL 恢复重放 data_page_tail 持久化与写路径陈旧 tail 自愈

## Current-State Evidence（2026-10-03 Plan 调查，file:line 已核实）

- **运行期 tail 持久化通道**：`TableManager::write_tuple`（`src/storage/data/table_manager.rs:1636-1657`）读 `old_tail` → `write_tuple_to_data_page` → 若写落页 `new_page_id != old_tail` 则 `catalog.update_table_tail(&name, new_page)`。`update_table_tail(&self, name: &str, new_tail: u32) -> Result<()>`（`src/storage/catalog.rs:368`，等长原地更新通道，已有 200 次循环零泄漏/窒息页持久化单测）。
- **恢复面 tail 推进点**：`redo_tuple_at_row_id` 步骤 3/4（`src/wal/recovery.rs:218-235`）——跨页时 `prev.next` 置后继、无条件 `ctx.table_last_page.insert(table_name, page_id)`、`*table_meta.data_page_tail.lock() = page_id` 只写内存。`full_recover`（`:532-627`）结构：位点消费 → 记录分类 → redo 循环（`will_redo` 门控，`redo_count` 计数）→ `mark_uncommitted_aborted` → `redo_count > 0` 时 `rebuild_pk_indexes`。`RedoContext { table_last_page: HashMap<String, PageId>, pk_versions }`（`:57-68`）——`table_last_page` 的 key 集合 = 发生过 ≥1 次重放数据写入的表（Insert/Update/Delete 墓碑三臂全经 `redo_tuple_at_row_id`）。
- **恢复面复用运行期写原语**：Delete 墓碑重放直接调 `write_tuple_to_data_page(buffer_pool, &table_meta, &tombstone, &[])`（`recovery.rs:932`）——该原语被恢复面共享，**自愈逻辑不得放入其中**（重放期链 next 指针正在按记录序重建，页 next ≠ 0 是重放中间态而非陈旧 tail，误自愈会破坏位置寻址不变量）。
- **写原语现状**：`write_tuple_to_data_page`（`src/storage/data_page.rs`，MS27 加溢出分支）——tail 页 `add_slot`，页满分配新页、旧 tail 挂链、更新内存 tail；其自身不触 catalog。
- **观测面**：`data_chain_status`（`table_manager.rs:999-1005`，`DataChainStatus { terminal, cyclic, tail }` + `is_complete()`）；`check` 的 tail 方向分级（`src/storage/integrity.rs:308-339`，terminal > 持久 tail → info「benign after crash recovery」，terminal ≤ 但不等 → error）；dump 链预检（`src/cli/lifecycle.rs` dump 每表 `data_chain_status`，stderr 单行告警 rc 0）。
- **既有测试锚点**：`tests/check_command_test.rs::test_check_pending_wal_exit_1_then_recovers`（:260-292）末段**当前断言恢复后 check 输出含 info 行**——该断言锁定的是本缺陷的形态，本 change 修复后需校准为干净单行（校准白名单）；`seed_pending_wal` 夹具纪律（checkpoint 固化 → 写 WAL → 停 flush loop → drop 不 close）见同文件 ：51-60；恢复套件 `tests/wal_recovery_large_test.rs`（7）、`tests/recovery_replay_convergence_test.rs`（3）、`tests/overflow_page_test.rs` 恢复用例（崩溃两态）、`tests/mvcc_tombstone_visibility_test.rs`（11）。
- **错误面**：恢复路径错误统一 `WalError`；K05 = 恢复任何失败显式 `Err` 拒绝打开。redo 相关物理失败先例用 `WalError::RedoFailed(说明)`。
- **基线**：master 工作区（含 MS27 收尾未提交实施面）；全量 1797 tests / 0 failed / 2 ignored（2026-10-03 MS27 002 Act M19 全量，Plan Review 002 accepted 采信）；fmt/clippy 干净。

## 目标行为

R1–R3 见 proposal。核心：①恢复重放收尾把每个发生重放写入的表的内存 tail 经 `update_table_tail` 持久化——恢复 → close → 重开后持久 tail == 链 terminal；②运行期 `write_tuple` 写入前探测 tail 页 `next ≠ 0`（陈旧形态）→ 沿链前进至 terminal（自愈）+ 持久化修正值 + 在 terminal 落笔——存量库「页满覆写 next → 已恢复页行丢失」风险闭环；③观测面校准（check 恢复库转干净、dump 不再误告警）与既有面零回归。

## 关键决策

### D1 恢复收尾按表写回（挂点与错误语义；rework Cycle 001 修订写回集合机制）

`full_recover` redo 循环结束、`mark_uncommitted_aborted` 之前：若 `redo_count > 0`，**遍历 catalog `scan_tables()` 的全部 `kind=Table` 行，对每表比较内存 `data_page_tail` 与行持久值，不一致时调 `catalog.update_table_tail(name, mem_tail)`**；`get_table`/`scan_tables`/`update_table_tail` 失败映射 `WalError::RedoFailed(format!("persist data_page_tail of table '{name}' after redo: {e}"))` 显式拒绝打开（K05 一致，不新增错误变体，文案点名表与操作）。

- **修订原因（2026-10-03 rework Cycle 001，父 Cycle Blocker Handoff 确认）**：原机制遍历 `ctx.table_last_page` keys，依赖「三臂全经 `redo_tuple_at_row_id`」断言——该断言失实：Delete 墓碑臂（`recovery.rs:894-952`）直调 `write_tuple_to_data_page`（:932）绕行，仅 post-checkpoint DELETE 且墓碑触发新页分配的表内存 tail 前进而不在集合中，恰欠覆盖本 change 要关闭的缺陷形态。catalog 清单 + 差异比较在结构上覆盖任意现行与未来写入臂：未分配新页的表内存 tail == 持久 tail 自然跳过（保留零多余 IO 取舍），分配过的表必然不一致 → 写回。不采用「向 `table_last_page` 登记 Delete 臂」：属恢复面行为变化，且与链重建分支（:218-231 `prev.next` 覆写）存在交互风险（Act 阻塞分析成立）。
- 幂等：crash-during-recovery 后重开重放从头再执行（既有位置寻址幂等），收尾再比较写回，收敛一致。
- `redo_count == 0` / 无 WAL：不进入该分支，打开路径与 IO 面零变化（复用既有 `will_redo` 门控）。
- 与 `rebuild_pk_indexes` 的顺序：tail 写回在 redo 循环后立即执行，先于 abort 标记与索引重建——tail 是纯数据页状态，与两者无耦合；中途崩溃则下次恢复重做并再写回，无半态风险（tail 字段单值、非链）。

### D2 自愈挂点 = `TableManager::write_tuple` 运行期入口（不在共享原语内）

`write_tuple` 在调 `write_tuple_to_data_page` 之前：`with_page_data(内存 tail 页)` 读 header `next_page_id`；`next == 0`（健康）→ 直接落既有路径（探测命中 BufferPool 缓存、该页随即被写入访问，健康库无额外磁盘读、不触发行走查）；`next != 0`（陈旧）→ 沿 `next` 链前进至 terminal（visited 防环、越界即按既有错误面报错），更新内存 tail = terminal、`update_table_tail` 持久化修正值，然后照常追加（terminal 有空间则写入 terminal，无空间则既有分配分支从 terminal 挂新页——不再覆写任何链中 next）。

- **替代方案否决——自愈放进 `write_tuple_to_data_page`**：恢复面 Delete 墓碑重放复用该原语（`recovery.rs:932`），重放期页 next ≠ 0 是链重建中间态，误自愈会跳写错误页、破坏位置寻址（R1 域既有语义）。挂 `write_tuple` 运行期入口天然隔离（恢复面不经过 `write_tuple`）。
- **替代方案否决——open 时逐表 heal**（集中决策选项 C）：干净打开常态每表 +1 页读，成本回归；存量库经首次写入自然修正，`check` info 行为其间指示器。
- **替代方案否决——tail 改由 checkpoint 位点承载**：格式演进，超范围。
- 自愈持久化与追加持久化的合并：追加落页 `page_id != old_tail`（heal 前读的旧值）时既有条件已持久化新页；heal 值（terminal）在「terminal 有空间直接写入」形态下由同一条件覆盖（terminal ≠ old_tail → 持久化 terminal，随后追加同页）；「terminal 满分配新页」形态持久化新页。实现上是否单独持久化 heal 值由 Act 按最小改动定（两形态终态 tail 均被持久化，非实质）。

### D3 观测面校准（白名单）

- `tests/check_command_test.rs::test_check_pending_wal_exit_1_then_recovers` 末段断言从「恢复后 check 输出含 info 行」校准为「恢复 + close 后 check 干净单行 exit 0」（该断言原是缺陷形态的见证；修复后形态反转）。前半段（pending WAL exit 1 + stderr 点名）不变。
- `integrity.rs` 的 info 分级保留不动（存量库指示器，R3）；`dump` 预检代码不动（tail 一致后自然不再告警）。
- 既有测试预计零校准之外无其他断言变化；若 Act 发现其他测试锚定旧形态（恢复后 tail 陈旧），属计划遗漏，返回 Plan。

### D4 测试策略（rework Cycle 001 补 DELETE-only 见证类）

新 `tests/recovery_tail_persistence_test.rs`：
- `recovery_persists_tail_two_state_consistent`（S1）：checkpoint 固化 → 跨页写入（行值撑过页容量，产生 ≥2 数据页 + 终页非 tail 起点）→ drop 不 close → 重开（恢复）→ close → 重开 → `db.table_manager.data_chain_status("t")` 断言 `is_complete()`（terminal == catalog tail）+ 行数一致；CLI 面：`check` exit 0 单行、`dump` stderr 无链告警。RED：修复前 catalog tail 陈旧 → `is_complete()` false / check 有 info 行。
- `delete_only_replay_allocating_page_persists_tail`（S1 DELETE 驱动形态，rework 补口）：建表 → 按「每页恰一行且 checkpoint 时 tail 页空闲 < 墓碑 footprint 28B（22B header + 6B slot）」几何写入 → checkpoint → DELETE 一行（提交）→ 崩溃 → 恢复（墓碑写入触发新页分配、内存 tail 前进）→ close → 重开 → 断言 `is_complete()` + 删后行数；check exit 0（容忍 info——运行期墓碑页被恢复期新页在链上取代成为孤儿，MS27 D5/D6 已知边界）。RED：无写回（或按原 `table_last_page` 机制）时 `is_complete()` false——该用例对原机制失败，是 gap 的直接见证。
- `repeated_recovery_converges`（S4）：首次恢复后不 close 再 drop → 重开再恢复 → 行集一致、无错误、tail 一致。
- `clean_open_untouched`（S3）：无 WAL 库打开写入往返行为不变（守卫 redo_count==0 分支不触碰 tail 通道；与既有套件共同承载）。
自愈单元测试（`src/storage/data/table_manager.rs` `#[cfg(test)]`，可触达 pub(crate) 与字段）：
- `stale_tail_heals_on_write`（S2）：write_tuple 跨页写入后，人为回拨内存 tail 与 catalog tail 至旧页（模拟存量形态）→ 再 write_tuple（一次即触发 heal）→ 断言内存 tail == terminal、catalog tail == terminal（scan_tables 读回）、新旧行全可达（扫描计数 = 旧 + 新）。RED：修复前写落陈旧页、tail 不修正。
- `healthy_write_no_walk`（S2 零常态面）：健康库连续 write_tuple，断言行为不变（隐式承载；行走查触发以陈旧形态用例的反面覆盖）。
定向回归：`wal_recovery_large_test` / `recovery_replay_convergence_test` / `overflow_page_test` / `mvcc_tombstone_visibility_test` / `check_command_test` / `catalog_durability_test` / `checkpoint_*` → fmt/clippy → M19 全量分批。

## 实现顺序与依赖

单一 Iteration：T1 恢复收尾持久化（R1）→ T2 写路径自愈（R2，与 T1 无代码耦合但共享夹具纪律与验证边界）→ T3 观测面校准与回归收口（R3，依赖 T1 落地后形态反转）。

## 风险与已知边界

- 恢复收尾写回对「表在 WAL 期间被 drop」形态：redo 对缺失表本就 RedoFailed（既有），persist 循环内 `get_table` 失败同样映射 RedoFailed——与 K05 一致，无新语义。
- 自愈行走查 O(gap)（gap = 陈旧 tail 与 terminal 间的页数）仅在存量异常形态首次写入时发生，健康库零行走查；单写者模型（独占锁）下无并发探测竞争。
- 存量库在首次写入前，check info 行与 dump 告警保持（R3 明确保留）——这是指示器而非缺陷。
- `data_page_tail` 字段为 u32 等长原地更新（catalog-update-persistence R1 既有通道），无变长链追加失败面。
