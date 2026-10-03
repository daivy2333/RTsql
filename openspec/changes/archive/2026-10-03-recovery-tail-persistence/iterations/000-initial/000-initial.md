# Iteration 000 / Cycle 000: 恢复 tail 持久一致与自愈闭环

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1、T2、T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R1–R3（delta spec `wal-recovery-replay-integrity` ADDED 三 Requirement）；`update_table_tail` 等长原地更新通道、`seed_pending_wal` 夹具纪律（checkpoint 固化 → 写 WAL → 停 flush loop → drop 不 close）、`data_chain_status` 预检先例、integrity.rs tail 方向分级全部继续有效
- Excluded scope: 存量库 open 时主动 heal、check info 分级移除、dump 预检语义变更、checkpoint 位点格式演进、性能优化

**Objective**

恢复 → close → 重开后 catalog 持久 tail 与数据链 terminal 一致（`rtsql check` exit 0 无 info、`rtsql dump` 无链告警）；存量陈旧 tail 库首次写入即自愈（内存与持久 tail 修正至 terminal、已恢复页行全部可达、不覆写链中 next）；`redo_count == 0` 打开路径与健康写路径零变化；既有恢复/观测面零回归。完成后 Issue「WAL 恢复重放不持久化 data_page_tail」的根因、实证后果与推演风险全部关闭。

**Investigation Facts**

- Current Baseline: master 工作区（含 MS27 收尾未提交实施面：15 修改 + 5 新增 + 归档 carrier + 状态文档）。全量 1797 tests / 0 failed / 2 ignored；fmt/clippy 干净（2026-10-03 MS27 002 Act Response M19 全量 12 批，Plan Review 002 accepted 采信）。
- Current-State Evidence（本 Cycle 展开审读，file:line 已核实）：
  - 运行期通道：`TableManager::write_tuple`（`src/storage/data/table_manager.rs:1636-1657`）——读 `old_tail` → `write_tuple_to_data_page` → `new_page_id != old_tail` 时 `catalog.update_table_tail(&name, new_page)`；这是恢复面缺失的持久化调用。
  - `update_table_tail(&self, name: &str, new_tail: u32) -> Result<()>`（`src/storage/catalog.rs:368`）：等长字段原地更新（catalog-update-persistence R1 通道），已有 200 次循环零泄漏 / 窒息页持久化单测；错误为 `StorageError`。
  - 恢复结构：`full_recover`（`src/wal/recovery.rs:532-627`）——位点消费 → 分类 → redo 循环（`will_redo` 门控 + `redo_count` 计数，:610-627 区域）→ `mark_uncommitted_aborted` → `redo_count > 0` 时 `rebuild_pk_indexes`。`RedoContext { table_last_page: HashMap<String, PageId>, pk_versions }`（:57-68）。
  - tail 推进点：`redo_tuple_at_row_id` 步骤 3/4（:218-235）——`ctx.table_last_page.insert(table_name, page_id)` 无条件执行（三臂全经此函数），`*table_meta.data_page_tail.lock() = page_id` 只写内存。**`table_last_page` 的 key 集合 = 发生过 ≥1 次重放数据写入的表**。
  - 共享原语警告：Delete 墓碑重放直接调 `write_tuple_to_data_page(buffer_pool, &table_meta, &tombstone, &[])`（:932）——该原语被恢复面复用，重放期页 `next ≠ 0` 是链重建中间态；**自愈逻辑必须挂在运行期独有的 `write_tuple`，不得进入 `write_tuple_to_data_page`**。
  - `write_tuple_to_data_page`（`src/storage/data_page.rs`，MS27 加溢出分支）：tail 页 `add_slot`，页满分配新页、旧 tail `data[5..9]` 挂链、更新内存 tail；自身不触 catalog。
  - 观测面：`data_chain_status`（`table_manager.rs:999-1005`，`DataChainStatus { terminal, cyclic, tail }` + `is_complete()`——terminal 对照 catalog tail）；`check` tail 方向分级（`src/storage/integrity.rs:308-339`，terminal > 持久 tail → info、≤ 但不等 → error）；dump 链预检（`src/cli/lifecycle.rs` dump 每表预检，stderr 单行告警 rc 0）。
  - 校准锚点：`tests/check_command_test.rs::test_check_pending_wal_exit_1_then_recovers`（:260-292）末段断言恢复后 check 输出含 info 行（`contains("info") && contains("benign after crash recovery")`）——本修复后该形态反转，断言校准为干净单行 exit 0（白名单）；同文件 `seed_pending_wal`（:51-60）为夹具纪律先例。
  - 错误面：恢复错误统一 `WalError`；redo 物理失败先例 `WalError::RedoFailed(说明)`；K05 = 恢复任何失败显式 `Err`。
  - 单元测试面：`src/storage/data/table_manager.rs` 尾部已有 `#[cfg(test)]` 模块（`FileStorage` + `BufferPool` + `TableManager::new` 构造先例），可触达 `TableMeta` 字段（`data_page_tail: Mutex<PageId>`）与 `pub(crate)` 项——自愈测试在此构造陈旧形态。
- Code and Critical Path: T1 = `src/wal/recovery.rs::full_recover`（redo 循环后插入按表写回段）+ `tests/recovery_tail_persistence_test.rs`；T2 = `src/storage/data/table_manager.rs::write_tuple`（写入前探测 + 自愈段）+ 同文件 `#[cfg(test)]` 新用例；T3 = `tests/check_command_test.rs` 断言校准 + `recovery_tail_persistence_test` CLI 见证 + 回归收口。

**Implementation Guidance**

- T1 写回段置于 redo 循环结束之后、`mark_uncommitted_aborted` 之前；`if redo_count > 0` 门控（复用既有 `will_redo` 语义，不重算）；遍历 `ctx.table_last_page.keys()` 克隆收集后逐表处理（`ctx` 随后仍被 `rebuild_pk_indexes` 使用前需注意借用——先收集 `Vec<String>` 再循环，避免借用冲突）；每表 `get_table` → 读内存 tail → `update_table_tail`；`get_table`/`update_table_tail` 失败统一 `.map_err(|e| WalError::RedoFailed(format!("persist data_page_tail of table '{name}' after redo: {e}")))`。
- T2 自愈段置于 `write_tuple` 读 `old_tail` 之后、调 `write_tuple_to_data_page` 之前：`buffer_pool.with_page_data(old_tail, |d| Ok(SlottedPageRef::new(d).header().next_page_id))`；`== 0` 直接前进；`≠ 0` 循环沿 next 前进（visited 防环、页界校验，异常按既有 `StorageError` 面传播）至 terminal，更新内存 tail、`update_table_tail(name, terminal)`，以 terminal 为追加目标继续既有路径；落页后既有 `new_page_id != old_tail` 持久化条件天然覆盖两形态终态（是否单独持久化 heal 值由 Act 按最小改动定，非实质）。
- T3 校准仅动 `test_check_pending_wal_exit_1_then_recovers` 末段断言（前半段 pending WAL exit 1 不变）；`recovery_tail_persistence_test` 的 CLI 见证用 `CARGO_BIN_EXE_rtsql` 真二进制 + `RTSQL_HOME` 隔离（同 check_command_test 纪律，env 移除 `RTSQL_KEY`/`RTSQL_BUSY_TIMEOUT`）。
- RED 见证基线：修复前 `data_chain_status.is_complete()` 对恢复后库为 false（terminal 领先持久 tail）、check 输出含 info 行——新测试先以此观察 RED 再实现。

**Task Contracts**

### T1: 恢复重放收尾按表持久化 tail

- Requirement/Scenario: R1（恢复后 tail 两态一致、重复恢复幂等、无重放零变化）
- Depends on: None
- Targets: `src/wal/recovery.rs::full_recover`（redo 循环后新增写回段）；新 `tests/recovery_tail_persistence_test.rs`
- Current behavior: 恢复重放只推进内存 tail（`recovery.rs:234-235`），catalog 持久 tail 滞留 checkpoint 时的旧值；恢复 → close → 重开后 `data_chain_status.is_complete()` 为 false、check 有 info 行、dump 误告警。
- Required behavior: `redo_count > 0` 时恢复收尾逐表 `update_table_tail(name, 内存 tail)`；恢复 → close → 重开后持久 tail == 链 terminal（`is_complete()` true）、check exit 0 无 info、dump 无链告警；写回失败显式 `WalError::RedoFailed` 点名表；重复恢复幂等；`redo_count == 0`/无 WAL 打开路径零变化。
- Preserve: redo 位置寻址与幂等语义、K05 显式报错、`mark_uncommitted_aborted` 与 `rebuild_pk_indexes` 行为与顺序、`redo_count == 0` 路径逐字节不变、`update_table_tail` 通道自身实现不动。
- Forbidden: 不改 `redo_tuple_at_row_id` 的内存 tail 语义；不动 integrity.rs 分级；不做 open 时主动 heal；不新增错误变体。
- Test witness: `tests/recovery_tail_persistence_test.rs` 的 `recovery_persists_tail_two_state_consistent`（checkpoint → 跨页写入 ≥3 数据页 → drop 不 close → 重开恢复 → close → 重开 → `data_chain_status("t").is_complete()` + 行数一致 + CLI check 单行 exit 0 + dump stderr 无告警）与 `repeated_recovery_converges`（首次恢复后不 close 再 drop → 重开再恢复 → 行集一致无错误）与 `clean_open_untouched`（无 WAL 库写入往返不变）。RED：实现前 `is_complete()` false / check 含 info。命令 `cargo test --test recovery_tail_persistence_test -- --test-threads=1`。
- GREEN condition: 新测试全绿 + 既有恢复套件零回归。
- Verification: `cargo test --test wal_recovery_large_test --test recovery_replay_convergence_test --test overflow_page_test -- --test-threads=1` 0 failed。
- Stop when: 写回需触碰 redo 中间态或 `table_last_page` 语义不成立（如出现不经 `redo_tuple_at_row_id` 的重放写入臂），或持久化通道对恢复期调用有未预见约束（返回 Plan）。

### T2: 写路径陈旧 tail 自愈

- Requirement/Scenario: R2（陈旧 tail 页满时已恢复页行不丢失、健康库零额外常态 IO）
- Depends on: None（与 T1 并行可达；验证边界共享）
- Targets: `src/storage/data/table_manager.rs::write_tuple`（写入前探测 + 自愈段）；同文件 `#[cfg(test)]` 新用例
- Current behavior: `write_tuple` 直接以内存 tail 页为写入目标；tail 陈旧时行写入链中页（可达但 tail 持续陈旧），页满分配新页并覆写陈旧 tail 页的 next 指针——其后链页行不可达（数据丢失风险）。
- Required behavior: 写入前探测内存 tail 页 header `next_page_id`：`== 0` 走既有路径（无链行走查）；`≠ 0` 沿链前进至 terminal（visited 防环、页界校验）→ 更新内存 tail = terminal + `update_table_tail` 持久化修正值 → 照常追加（terminal 有空间写入 terminal；满则既有分配分支从 terminal 挂新页，不覆写链中既有 next）；修正后新旧行全可达。
- Preserve: `write_tuple_to_data_page` 原语不动（恢复面复用，:932）；健康路径写序与错误面逐字节不变；单写者模型假设；`update_table_tail` 通道不动。
- Forbidden: 不把自愈放进 `write_tuple_to_data_page`；不改变非 tail 页的写入路由；不做 open 时 heal；不动 recovery 对该原语的调用。
- Test witness: `src/storage/data/table_manager.rs` `#[cfg(test)]` 的 `stale_tail_heals_on_write`——write_tuple 跨页写入（tail 前进至 T2）后人为回拨内存与 catalog tail 至旧页（模拟存量形态）→ 单次 write_tuple → 断言内存 tail == terminal、`scan_tables` 读回 catalog tail == terminal、全表扫描行数 = 旧 + 新。RED：实现前行落陈旧页、tail 不修正。命令 `cargo test --lib table_manager -- --test-threads=1`。
- GREEN condition: 新单测绿 + lib 零回归。
- Verification: `cargo test --lib -- --test-threads=1` 0 failed。
- Stop when: 探测需为健康路径引入额外磁盘 IO 或破坏写序原子性，或陈旧形态构造在单测面不可达（返回 Plan）。

### T3: 观测面校准与回归收口

- Requirement/Scenario: R3（存量库观测面保持、新建恢复库干净、既有恢复面零回归）
- Depends on: T1、T2
- Targets: `tests/check_command_test.rs::test_check_pending_wal_exit_1_then_recovers`（末段断言校准）；`tests/recovery_tail_persistence_test.rs`（CLI check/dump 见证，随 T1 用例承载）
- Current behavior: 恢复 + close 后 check 输出含 info 行（`benign after crash recovery`）——锁定的是缺陷形态。
- Required behavior: 校准后断言恢复 + close 后 check 输出干净单行 exit 0（无 info）；`recovery_tail_persistence_test` 的 check/dump CLI 见证随 T1 GREEN；定向回归与全量收口 0 failed。
- Preserve: check info 分级实现（integrity.rs）不动；dump 预检代码不动；该测试前半段（pending WAL exit 1 + stderr 点名）不变。
- Forbidden: 不校准白名单之外的既有断言；发现其他测试锚定旧形态时不自行修改（返回 Plan）。
- Test witness: `cargo test --test check_command_test -- --test-threads=1` 0 failed（校准后）；定向回归批 0 failed；fmt/clippy 干净；M19 全量分批 0 failed。
- GREEN condition: 校准后矩阵绿、全量收尾 0 failed。
- Verification: 定向 `cargo test --test wal_recovery_large_test --test recovery_replay_convergence_test --test overflow_page_test --test mvcc_tombstone_visibility_test --test check_command_test --test catalog_durability_test -- --test-threads=1` 0 failed；M19 全量分批（`--lib`/`--bins` + 集成目标按名字典序 8–12 个/批串行）。
- Stop when: 校准白名单外出现旧行为锚定（返回 Plan）。

**Invariants**

- 恢复面（`redo_tuple_at_row_id`、Delete 墓碑的 `write_tuple_to_data_page` 调用）零行为变化；自愈只存在于运行期 `write_tuple` 入口。
- `redo_count == 0` 与无 WAL 打开路径逐字节不变。
- K05：tail 写回与自愈持久化失败均显式 `Err`，不静默。
- 健康库：无链行走查、无额外磁盘读；存量库：首次写入自愈后不再覆写链中 next。
- integrity.rs info 分级、dump 预检、`update_table_tail` 通道实现均不动。

**Non-goals**

存量库 open 时主动 heal；check info 分级移除；dump 预检语义变更；checkpoint 位点格式演进；VACUUM/GC；性能优化。

**Acceptance**

R1——`recovery_tail_persistence_test` 两态一致/幂等/零变化三用例 + 恢复定向回归；R2——`stale_tail_heals_on_write` 单元测试（含 catalog 持久化断言与全行可达）；R3——check_command_test 校准后矩阵 + dump 干净见证 + 定向回归 + M19 全量。映射：T1→R1、T2→R2、T3→R3。

**Verification**

- `cargo test --test recovery_tail_persistence_test -- --test-threads=1` 0 failed（T1）。
- `cargo test --lib -- --test-threads=1` 0 failed（T2 单测 + lib 零回归）。
- `cargo test --test check_command_test -- --test-threads=1` 0 failed（T3 校准后）。
- 定向回归：`cargo test --test wal_recovery_large_test --test recovery_replay_convergence_test --test overflow_page_test --test mvcc_tombstone_visibility_test --test catalog_durability_test -- --test-threads=1` 0 failed。
- `cargo fmt --check` 与 `cargo clippy --tests` 无新告警。
- M19 全量分批收尾（`--lib`/`--bins` + 集成目标按名字典序 8–12 个/批，串行 `--test-threads=1`），每批 0 failed 方可宣称全量。

**Gate 2 Readiness**

PASS——2026-10-03 用户批准完整计划（选项「批准完整计划（推荐）」）；调查、设计闭合、任务契约、RTM（R1–R3 全 Covered）、验证与 Persisted Evidence 各维度无 BLOCKED/WAIVED；`openspec validate 2026-10-03-recovery-tail-persistence` valid。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- `ctx.table_last_page` 的 key 集合依赖「三臂全经 `redo_tuple_at_row_id`」这一现行结构；若未来出现绕行臂，写回集合需同步（本 change 范围内已核实成立）。
- 自愈在「terminal 满分配新页」形态下的持久化值是终态新页（heal 中间值 terminal 被立即取代）；两形态终态均被既有/新增持久化覆盖，Act 按最小改动实现。
- 存量库首次写入前 check info 行与 dump 告警保持——指示器语义，非缺陷（R3）。
- 非实质选择（写回段的收集方式、自愈探测的具体访问器、测试夹具组织）由 Act 自定。

## Act Response

- Status: blocked
- Cycle: Iteration 000 / Cycle 000（initial）
- 进入时状态：Plan Context `ready`；无后继 Cycle；`Review Result` 未写入（pending 语义成立）。

### 已完成工作

- 按 Task Contract 完成目标代码核读（T1/T2/T3 全部 Targets 及 Plan Context 引用的 file:line 逐点核实）：`src/wal/recovery.rs`（`full_recover` :532-645、`redo_record` 三臂 :661-955、`redo_tuple_at_row_id` 链/tail 推进 :217-242）、`src/storage/data/table_manager.rs`（`write_tuple` :1638-1656、`data_chain_status` :1003-1033、`#[cfg(test)]` 模块 :1659 起）、`src/storage/data_page.rs::write_tuple_to_data_page`（:18-92）、`tests/check_command_test.rs` 夹具与锚点（:51-60、:260-292）。
- 未修改任何产品代码、测试或体系文档；未建立测试见证（发现阻塞早于 TDD RED 步骤）。

### Blocker Handoff

- **发现位置**：T1 / Plan Context 调查事实核验（Phase 3 step 2「按 Task Contract 读取目标代码」）；Gate 2 ready 基础上的实施前核验。触发 Gate 5「实际代码与契约存在实质冲突」。
- **Plan 预期**：Plan Context Investigation Facts 与 design D1 均断言「Insert/Update/Delete 墓碑三臂全经 `redo_tuple_at_row_id`」，故 `ctx.table_last_page` 的 key 集合 = 发生过 ≥1 次重放数据写入的表；T1 写回段遍历 `ctx.table_last_page.keys()` 即覆盖 R1 的「每个发生重放写入的表」。Task Contract T1 的 Stop-when 为此预设了返 Plan 条件：「如出现不经 `redo_tuple_at_row_id` 的重放写入臂」。
- **实际情况**：该预设条件已命中。Delete 墓碑臂**不经** `redo_tuple_at_row_id`（`src/wal/recovery.rs:893-952`）：索引清理后直接调 `write_tuple_to_data_page(buffer_pool, &table_meta, &tombstone, &[])`（:932），全程不触碰 `ctx.table_last_page`。而 `write_tuple_to_data_page` 以内存 tail 为写入目标、页满时分配新页并推进内存 tail（`src/storage/data_page.rs:39` 读 tail、:56-87 分配 + 旧 tail 挂链 + `*data_page_tail.lock() = new_page_id`）。`ctx.table_last_page.insert` 仅存在于 `redo_tuple_at_row_id` :232（Insert/Update 两臂经此函数）。Plan 自身引用的 :932 共享原语警告与「三臂全经此函数」陈述互相矛盾，前者正确、后者失实。
- **影响（缺口可达性）**：仅 Post-checkpoint 已提交 DELETE 记录的表，若恢复重放时墓碑写入因 tail 页空间不足（checkpoint 固化时空闲 < 墓碑 slot ≈38B）分配新页，则该表内存 tail 前进而**不在** `table_last_page` key 集合中。按 D1/T1 机制执行：该表 tail 不写回 → 恢复 → close → 重开后持久 tail 落后链 terminal——正是本 change 要对新建恢复库关闭的缺陷形态（R1 SHALL「每个发生重放写入的表」对该类欠覆盖、S1 两态一致不成立、check 仍报 info/dump 仍告警）。规划中的 RTM 见证（INSERT 驱动跨页增长）对该缺口不可见，即按契约实施也能通过全部计划内测试——测试策略同样需要 Plan 补口。
- **为何不在契约内处理**：闭合该缺口的两条路径均超出 Act 授权——①改写回集合来源（如按 catalog 全表比较内存 tail 与持久 tail）与 D1 明文机制及「不比较新旧值」取舍冲突，属设计层修订；②让 Delete 臂登记 `table_last_page` 属恢复面行为变化，被 Task Contract Invariants「恢复面零行为变化」与 Preserve「不动 recovery 对该原语的调用」禁止，且该登记会与后续 Insert 重放的链重建分支（:217-231 `prev.next` 覆写）产生交互风险。
- **任务状态**：T1/T2/T3 均 not started（无部分完成）；已完成项仅上述代码核读。工作区无本 change 产生的文件修改（MS27 既有未提交批次原样未动）。
- **Evidence**：None required（偏差可由上述 file:line 直接复现核验，无需保存运行现场）。

### 恢复条件

用户二选一后恢复：

1. 调用 `openspec-plan` 修订 D1/T1 写回集合机制（例如：写回段改为按 catalog 表清单逐表比较内存 tail 与持久 tail、差异时 `update_table_tail`——该机制同时结构性覆盖未来任何绕行臂），并在 D4/RTM 补一类「仅 DELETE 重放 + 墓碑分配新页」见证；Plan 完成后重新授权 Act。
2. 显式接受收窄范围（承认 Delete-only 重放表为已知边界、放宽 R1 SHALL 措辞）——属需求变更，同样由 Plan 落字。

### 验证

- 无（未进入实施；无代码改动即无验证对象）。

### Persisted Evidence

- None required（Mode: none 维持不变）。

### Experience Candidates

- None（本发现属当前 change 范围内的计划缺陷，随 Blocker Handoff 返 Plan，不构成范围外 Issue 候选）。

## Plan Review

- Review Result: rework-required

### Findings

**Blocker Handoff 处理结果：成立，分类为 Plan 错误（调查断言失实）；Act 偏差处理正确（Gate 5 阻塞合规，未越权实施）。**

1. **阻塞事实独立核实成立**：`redo_record` 的 Delete 臂（`src/wal/recovery.rs:894-952`）只做索引清理 + 直调 `write_tuple_to_data_page(...)`（:932），全程不触碰 `ctx.table_last_page`（该 map 全文件仅 :218/:232 两处消费/写入，均在 `redo_tuple_at_row_id` 内、仅 Insert/Update 臂可达）；`write_tuple_to_data_page` 以内存 tail 为写入目标、页满分配新页并覆写旧 tail `data[5..9]` + 推进内存 tail（`src/storage/data_page.rs` :39/:56-87 区域）。Plan Context 的「Insert/Update/Delete 墓碑三臂全经 `redo_tuple_at_row_id`……本 change 范围内已核实成立」断言失实，且与 Plan 自身在 D2 引用的 :932 共享原语事实直接矛盾——属 Plan 调查错误，非 Act 偏离。
2. **缺口可达性核实成立**：仅 post-checkpoint 已提交 DELETE 且墓碑写入触发新页分配的表，内存 tail 前进而 tail 值不在 `ctx.table_last_page` key 集合中——按原 D1 机制该表不写回，恢复 → close → 重开后持久 tail 落后链 terminal，恰为本 change 要关闭的缺陷形态；且计划内见证（INSERT 驱动跨页）对该缺口不可见，测试策略需同步补口。Task Contract T1 的 Stop-when（「出现不经 `redo_tuple_at_row_id` 的重放写入臂」）按设计命中，Act 返 Plan 是契约要求的行为。
3. **工作区状态核实**：`git status`/`diff --stat` 对照 MS27 收尾基线——无本 change 产生的产品代码或测试修改，无 `recovery_tail_persistence_test.rs`；Act「未进入实施、无部分完成」声明属实。已完成且未失效的结论：仅代码核读（不构成验证结论），无采信负担。
4. **恢复路径 1（修订机制）为正确路径**：Act 建议的「按 catalog 表清单逐表比较内存 tail 与持久 tail、差异时写回」在结构上覆盖任意现行与未来写入臂（未分配新页的表内存 tail == 持久 tail 自然跳过，保留 D1 的零多余 IO 取舍），且不触碰恢复面行为（Invariants 保持）。路径 2（收窄 R1）是需求变更，与本 change 立项目标（完整关闭该缺陷）相悖，不采纳。
5. Non-blocking Minor：无。

### Acceptance Gaps

R1「恢复收尾 SHALL 把每个发生重放写入的表的 `data_page_tail` 持久化」对「仅 DELETE 重放 + 墓碑新页分配」类表不成立（原机制欠覆盖）——既有 Acceptance 未满足，需修订 T1 执行契约（写回集合来源）并补齐 D4 见证类。R2/R3 契约不受影响。

### Evidence

- 独立阅读（本 Review）：`src/wal/recovery.rs` :885-955（Delete 臂全文）、`table_last_page` 全部触点（:60/:71/:218/:232/:614）、`src/storage/data_page.rs` tail 处理段、`git status --porcelain` 与 `git diff --stat`（17 files = MS27 的 15 + 两新语料 spec；无 `recovery_tail_persistence_test.rs`）。
- 采信 Act Response：代码核读所引 file:line（与本次独立阅读逐点一致）；其余无验证结论可采信（未进入实施）。

### Follow-up Decision

`rework-required`：Acceptance gap 的闭合需要修订 T1 的执行契约（写回集合机制属于 design D1 层修订）并新增 D4 见证类，需要新的执行上下文——按 iteration-planning › Rework Cycle 创建同 Iteration 后继 Cycle `001-rework.md`（repair items T1-R1 / T1-R2，T2/T3 契约原样继承）；Iteration Map 不变，R1–R3 requirement 与 delta spec 不变。design.md D1/D4 由本 Review 同步修订（标注修订来源）。Gate 2 在 rework Cycle 重新通过（引用父 Cycle 未变化维度，补审查受影响维度）后交 Act。

### Next Cycle

`iterations/000-initial/001-rework.md`

### Next Iteration

None（Iteration Map 不变；本 Iteration 在 rework Cycle accepted 前保持未完成）
