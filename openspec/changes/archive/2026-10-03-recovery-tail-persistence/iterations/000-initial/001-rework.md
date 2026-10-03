# Iteration 000 / Cycle 001: 恢复 tail 持久一致与自愈闭环（rework）

## Plan Context

- Status: ready
- Cycle Type: rework

**Cycle Scope**

- Change tasks: T1（经 repair items 修订契约）、T2、T3（契约原样继承父 Cycle）
- Acceptance gaps: R1 对「仅 DELETE 重放 + 墓碑新页分配」类表欠覆盖（父 Cycle Blocker Handoff 确立、Plan Review 确认）
- Repair items: **T1-R1**（写回集合机制修订：`ctx.table_last_page` keys → catalog 表清单 + 差异比较）、**T1-R2**（补 DELETE-only 重放见证）
- Inherited scope: R1–R3 全部 requirement 与 delta spec 不变；T2 契约（`write_tuple` 自愈 + `stale_tail_heals_on_write`）、T3 契约（check_command_test 白名单校准 + 定向回归 + M19 全量）原样继承；`seed_pending_wal` 夹具纪律、`update_table_tail` 通道、`data_chain_status` 预检先例继续有效
- Excluded scope: 与父 Cycle 相同（不做 open 时主动 heal、不动 check info 分级/dump 预检、不动 checkpoint 位点格式）

**Objective**

关闭父 Cycle 的 Acceptance gap：恢复收尾的 tail 写回在结构上覆盖**任意**重放写入臂（含不经 `redo_tuple_at_row_id` 的 Delete 墓碑臂），恢复 → close → 重开后持久 tail == 链 terminal 对全部重放形态成立；以 INSERT 驱动与 DELETE-only 驱动两类见证锁定。完成后与父 Cycle Objective 相同（R1–R3 全部可验证）。

**Background**

父 Cycle `000-initial.md` Act 阶段于 T1 契约核读时触发 Stop-when（Gate 5 阻塞，Blocker Handoff 返 Plan，Plan Review `rework-required` 确认）：Plan 错误地断言「三臂全经 `redo_tuple_at_row_id`」，实际 Delete 墓碑臂绕行（直调 `write_tuple_to_data_page`），原写回集合机制对该类表欠覆盖。本 Cycle 以修订后的 T1 契约与补口见证重新执行，T2/T3 不受影响。

**Investigation Facts**

- Current Baseline: 与父 Cycle 相同（master 工作区含 MS27 收尾未提交实施面；1797 tests / 0 failed / 2 ignored 采信链不变）。本 change 零代码产出（父 Cycle Act 未实施，`git status` 对照核实）。
- Current-State Evidence（gap 涉及维度，本 Cycle 核读补充；file:line 已核实）：
  - **Delete 重放臂绕行（gap 根因）**：`redo_record` 的 `WalRecord::Delete` 臂（`src/wal/recovery.rs:894-952`）——索引清理（:910-921）后，`read_version_header` 探测被删 slot 存在（:929）→ 构造墓碑 `VersionHeader::new(tx_id, None).with_next_version(row_id).mark_deleted()` → **直调 `write_tuple_to_data_page(buffer_pool, &table_meta, &tombstone, &[])`（:932）**，全程不触碰 `ctx.table_last_page`（全文件触点仅 :60/:71/:218/:232/:614，均在 `redo_tuple_at_row_id` 内、仅 Insert/Update 臂可达）。`SlotNotFound` 时跳过墓碑只做索引清理（:938-940，运行期 delete.rs 容错对齐）。
  - **`write_tuple_to_data_page` 的 tail 副作用**（`src/storage/data_page.rs` :30-92 区域）：读内存 tail（:39）→ `add_slot`；失败 → `allocate_page` + 旧 tail `data[5..9]` 挂链（:56-66）+ `*table_meta.data_page_tail.lock() = new_page_id`（:87 区域）。即墓碑写入页满时推进内存 tail 而 catalog tail 不变。
  - **墓碑 footprint**：slot_data = `VersionHeader`（22B）+ 0 字节 tuple = 22B，加 slot 目录 6B = **28B**——tail 页空闲 < 28B 时触发新页分配。
  - **catalog 表清单与持久 tail 读数**：`table_manager.catalog().scan_tables() -> Vec<CatalogRow>`（`data_chain_status` 与 rebuild 先例同通道）；`CatalogRow.data_page_tail: u32` 字段（`table_manager.rs:276` 消费先例 `PageId(row.data_page_tail as u64)`）；`CatalogKind::Table` 过滤先例（integrity.rs :126）。`get_table(name) -> Arc<TableMeta>` 内存 tail 读法：`*meta.data_page_tail.lock().unwrap()`（:1005 先例）。
  - **机制健全性推论（修订依据）**：未发生新页分配的表（无论哪条臂），内存 tail == 打开时装载的持久 tail → 差异比较自然跳过、零多余 IO；发生新页分配的表（任何臂），内存 tail ≠ 持久 tail → 写回。比较语义结构性覆盖任意现行与未来写入臂，不依赖「臂经某函数」这一脆弱断言。
  - **测试夹具几何**：数据页数据区 4080B（4096 − 16B header）。单行夹具：tuple payload ≈ 4040B → slot_data ≈ 4062B → 页内空闲 ≈ 18B < 28B（墓碑 footprint）→ checkpoint 后的 DELETE 墓碑写入必然触发新页分配。恢复期重放同型：装载 tail（旧页，18B 空闲）→ 墓碑追加 → 分配新页 → 内存 tail 前进。预期孤儿：运行期分配的墓碑页被恢复期重新分配的页在链上取代（`data[5..9]` 覆写），成为 unreferenced 页——MS27 D5/D6 已知边界，`check` 报 info 级 exit 0，**本夹具的 CLI 见证断言 exit 0 而非「无 info」**（干净单行断言保留在 INSERT 驱动夹具）。
- Code and Critical Path: T1-R1 = `src/wal/recovery.rs::full_recover`（写回段改为 catalog 清单驱动）；T1-R2 = `tests/recovery_tail_persistence_test.rs` 增 `delete_only_replay_allocating_page_persists_tail`；T2/T3 沿父 Cycle 契约（`src/storage/data/table_manager.rs::write_tuple`、`tests/check_command_test.rs`）。

**Implementation Guidance**

- T1-R1 写回段：`if redo_count > 0` 门控内，`scan_tables()` 后对每个 `kind == CatalogKind::Table` 行：`get_table(&row.table_name)` → `let mem_tail = *meta.data_page_tail.lock().unwrap();` → `if mem_tail.0 != row.data_page_tail { catalog().update_table_tail(&row.table_name, mem_tail.0 as u32).await }`；`get_table`/`scan_tables`/`update_table_tail` 失败统一 `WalError::RedoFailed(format!("persist data_page_tail of table '{}' after redo: {e}"))`。位置仍在 redo 循环后、`mark_uncommitted_aborted` 前；先收集所需数据再异步循环，注意 `scan_tables` 返回行的所有权与 `get_table` 的 await 边界（无借用冲突即可，组织方式非实质）。
- T1-R2 夹具按 Investigation Facts 的几何构造（payload 尺寸可微调，但必须保证 checkpoint 时 tail 页空闲 < 28B，并以一行注释说明算术）；崩溃恢复断言：`data_chain_status("t").is_complete()` + 行数 = 删后行数；不断言 info 行数（孤儿页预期存在）。
- T2/T3 按 000-initial.md 契约原样执行；T3 的校准断言（INSERT 驱动夹具恢复 + close 后干净单行）不受孤儿页影响（该夹具无崩溃窗口孤儿）。
- RED 见证基线：T1-R1 实现前（含按旧机制实现）DELETE-only 夹具 `is_complete()` 为 false；T2 的 RED 同父 Cycle。

**Repair Item Contracts**

### T1-R1: 恢复收尾 tail 写回改为 catalog 清单驱动（覆盖任意重放臂）

- Requirement/Scenario: R1（恢复后 tail 两态一致——全形态；重复恢复幂等；无重放零变化）
- Depends on: None
- Targets: `src/wal/recovery.rs::full_recover`（写回段，替换父 Cycle D1 的 `table_last_page` keys 机制）
- Current behavior: 无任何写回（父 Cycle 未实施）；若按父 Cycle 原机制实施，DELETE-only + 新页分配类表欠覆盖。
- Required behavior: `redo_count > 0` 时遍历 catalog 全部 `kind=Table` 行，内存 tail ≠ 持久 tail 的表逐表 `update_table_tail` 写回；`== 0` 零变化；失败 `WalError::RedoFailed` 点名表。INSERT 驱动与 DELETE-only 驱动两形态恢复后 `is_complete()` 均为 true。
- Preserve: 与父 Cycle T1 相同（redo 位置寻址与幂等、K05、abort/rebuild 顺序、`redo_count == 0` 逐字节不变、通道实现不动）；另加——不改 Delete 臂与 `write_tuple_to_data_page` 的恢复面行为（不向 `table_last_page` 登记）。
- Forbidden: 与父 Cycle T1 相同；不以「登记 `table_last_page`」方式实现（恢复面行为变化 + 链重建交互风险，Act 阻塞分析成立）。
- Test witness: `tests/recovery_tail_persistence_test.rs` 全部四用例 GREEN（含 T1-R2 新用例）；父 Cycle 三用例契约原样（两态一致/幂等/零变化）。RED：DELETE-only 夹具在实现前 `is_complete()` false。命令 `cargo test --test recovery_tail_persistence_test -- --test-threads=1`。
- GREEN condition: 四用例全绿 + 恢复定向回归 0 failed。
- Verification: `cargo test --test wal_recovery_large_test --test recovery_replay_convergence_test --test overflow_page_test -- --test-threads=1` 0 failed。
- Stop when: catalog 清单驱动的写回在恢复期不可实施（如 scan_tables 在恢复上下文行为异常），或出现两类夹具之外的第三种 tail 推进臂（返回 Plan）。

### T1-R2: 补 DELETE-only 重放见证

- Requirement/Scenario: R1（S1 两态一致对 DELETE 驱动形态成立）
- Depends on: T1-R1（同文件同轮交付）
- Targets: `tests/recovery_tail_persistence_test.rs` 新用例 `delete_only_replay_allocating_page_persists_tail`
- Current behavior: 计划内见证仅 INSERT 驱动，对该 gap 不可见。
- Required behavior: 夹具——建表（INT PK + STRING 列）→ 按「每页恰一行且页内空闲 < 28B」几何写入若干行 → `checkpoint()` 固化 → `DELETE` 一行（提交）→ drop 不 close → 重开（恢复，墓碑写入触发新页分配）→ close → 重开 → 断言 `data_chain_status.is_complete()` + 行数 = 删后值；`rtsql check` exit 0（容忍 info 行——孤儿页为 MS27 D5/D6 已知边界，不断言「无 info」）。
- Preserve: 夹具纪律对齐 `seed_pending_wal`（checkpoint 固化 → 写 WAL → 停 flush loop → drop 不 close）；其余与父 Cycle 相同。
- Forbidden: 不为凑断言改变孤儿页语义；不建身份型证据。
- Test witness: 本用例 RED（实现前 `is_complete()` false）→ GREEN；命令同 T1-R1。
- GREEN condition: 用例绿且与 T1-R1 联合成立。
- Verification: 同 T1-R1 命令。
- Stop when: 几何构造无法稳定触发墓碑新页分配（返回 Plan）。

**Invariants**

继承父 Cycle 全部 Invariants（恢复面零行为变化、`redo_count == 0` 逐字节不变、K05、健康库无链行走查、integrity/dump/通道实现不动），另加：

- tail 写回集合来源为 catalog 表清单 + 内存/持久差异比较，SHALL NOT 依赖「重放臂经过某特定函数」的结构断言。

**Non-goals**

与父 Cycle 相同。

**Acceptance**

与父 Cycle 相同（R1–R3 映射不变），R1 的 Test Witness 扩展为四用例（增 DELETE-only 驱动）；repair item 映射：T1-R1/T1-R2 → R1 gap（来源：父 Cycle Blocker Handoff + Plan Review rework-required）。

**Verification**

- `cargo test --test recovery_tail_persistence_test -- --test-threads=1` 0 failed（四用例）。
- `cargo test --lib -- --test-threads=1` 0 failed（T2）。
- `cargo test --test check_command_test -- --test-threads=1` 0 failed（T3 校准后）。
- 定向回归与 fmt/clippy/M19 全量分批：同父 Cycle Verification 节。

**Gate 2 Readiness**

PASS——2026-10-03 用户批准 rework 计划（选项「批准 rework 计划（推荐）」）。受影响维度（设计闭合 D1 机制修订、repair item 契约 T1-R1/T1-R2、验证充分性 DELETE-only 见证补口）已补齐；未变化维度引用父 Cycle Gate 2 PASS 结论（调查基线、T2/T3 契约、Persisted Evidence、追踪链）；`openspec validate` valid；无 BLOCKED/WAIVED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- DELETE-only 夹具中运行期墓碑页成为孤儿（恢复期链上被取代）——已知边界（MS27 D5/D6），`check` info 级指示，不影响 is_complete 与行数断言。
- 页几何为 4080B 数据区 + 22B VersionHeader + 6B slot 的常量推算；Act 实现时若夹具尺寸需微调，以「checkpoint 时 tail 页空闲 < 28B」为准绳并注释算术（非实质）。
- 其余继承父 Cycle Risks and Notes（`table_last_page` 结构断言的脆弱性已由本修订结构性消除）。

## Act Response

- Status: reported
- Cycle: Iteration 000 / Cycle 001（rework）
- 进入时状态：Plan Context `ready`（2026-10-03 用户批准 rework 计划）；无后继 Cycle；本文件无既有 Act Response（pending 语义成立）。

### Repair Item / Task 状态

- **T1-R1 完成**：写回集合机制按修订契约实施——`src/wal/recovery.rs` 新增 `persist_replayed_tails`（`full_recover` 关联函数），在 redo 循环后、`mark_uncommitted_aborted` 前由 `if redo_count > 0` 门控调用：`scan_tables()` 遍历 catalog 全部 `kind == CatalogKind::Table` 行，`get_table` 读内存 `data_page_tail`，与行内持久值差异比较，不一致时 `update_table_tail` 写回；`scan_tables`/`get_table`/`update_table_tail` 失败统一映射 `WalError::RedoFailed`（文案点名表与操作）。未触碰 `redo_tuple_at_row_id`、Delete 墓碑臂、`write_tuple_to_data_page` 与 `table_last_page` 语义（Invariants 保持）。
- **T1-R2 完成**：`tests/recovery_tail_persistence_test.rs` 新用例 `delete_only_replay_allocating_page_persists_tail`——夹具按 28B 墓碑 footprint 几何构造（4030 字符行 → 单行页空闲 10B < 28B，算术注释在文件头），DELETE 前后 `data_chain_status` 断言墓碑必然分配新页（夹具自检防几何失准的空转通过），恢复 → close → 重开后 `is_complete()` + 删后行数 2 + `rtsql check` exit 0（容忍 info）。
- **T2 完成**（契约原样继承）：`src/storage/data/table_manager.rs` 新增私有 `heal_stale_tail`——`write_tuple` 读内存 tail 后调用：tail 页 `next != 0` 沿链前进至 terminal（`page_count` 页界校验 + visited 防环，异常按既有 `StorageError` 面传播：越界 `InvalidPageId`、环 `Internal` 点名表），更新内存 tail 并 `update_table_tail` 持久化修正值，返回 terminal 作为追加目标；`next == 0` 健康路径单次 header 探测即返回（无链行走查）。`write_tuple_to_data_page` 原语零改动。新单元测试 `stale_tail_heals_on_write` + 链行计数 helper `count_live_rows`（同文件 `#[cfg(test)]`）。
- **T3 完成**（契约原样继承）：`tests/check_command_test.rs::test_check_pending_wal_exit_1_then_recovers` 末段断言校准（design D3 白名单内）——原「info + benign after crash recovery」断言替换为恢复 + close 后 stdout 恰一行 `check ok` exit 0；该文件其余 7 用例零改动。`recovery_tail_persistence_test` 的 CLI check/dump 干净见证随 T1 用例承载（INSERT 驱动夹具断言单行 exit 0 + dump stderr 无告警）。

### 实际改动（文件与符号）

- `src/wal/recovery.rs`：`full_recover` 内新增门控调用；新关联函数 `persist_replayed_tails`。（T1-R1）
- `src/storage/data/table_manager.rs`：`write_tuple` 接线自愈；新私有函数 `heal_stale_tail`；测试模块新增 `count_live_rows`、`stale_tail_heals_on_write`。（T2）
- `tests/check_command_test.rs`：`test_check_pending_wal_exit_1_then_recovers` 末段断言校准。（T3）
- `tests/recovery_tail_persistence_test.rs`：新文件，4 用例 + CLI harness（spawn_cli/wait_cli/run_cli，沿 check_command_test 纪律：`RTSQL_HOME` 隔离、`RTSQL_KEY`/`RTSQL_BUSY_TIMEOUT` env 移除）。（T1 两态/幂等/零变化 + T1-R2 DELETE-only）

工作区既有 MS27 收尾未提交批次原样未动；本 change 无其余文件修改。

### 与计划的偏差

1. **夹具锁纪律（非实质，契约内）**：两态测试引入 close → 重开序列——`Database::close()` 只做 checkpoint 不释放 advisory flock（锁随 `Database` drop 释放，MS07-T01 既有语义），close 后立即重开会等满锁预算后 `DatabaseLocked`。测试在每处 close 后显式 `drop` 再重开/起 CLI。Plan Risks 已列「测试夹具组织非实质」；首跑 RED 阶段发现并修正，不影响见证语义。
2. **测试文件机械项**：`DataChainStatus` 未实现 `Debug`，断言消息改手工格式化字段；`with_page_data` 闭包返回 `Result` 包裹。均为编译适配。
3. **M19 批次执行过程注记**：首轮批处理命令构造错误（cargo 多目标需逐个 `--test` 旗标，批量名被当多余参数，各批次实际未执行且管道 grep 掩盖了失败退出码），修正后重跑全部批次；第二轮批次转写遗漏 `write_type_conformance_test`（字典序在 wal_writer_test 之后），经 112 目标清单核对发现后单独补跑。两轮均为执行 harness 问题，非测试或产品行为问题；最终覆盖 112 集成目标 + lib/bins 齐全。

### 自检发现与修复

- Gate 3（逐 task）：T1-R1 修订单文件单函数、错误面与门控位置符合契约；T2 自愈仅存在于 `write_tuple` 入口、原语未动、健康路径零链行走查；T3 校准仅动白名单断言。发现一处本次新增注释多余空格（recovery.rs T1-R1 行内注释），已修正；无计划外修改、无新增告警（clippy --tests 干净）、无死代码。无遗留 Critical/Important；遗留 Minor：无。
- 见证有效性：T1 与 T2 均在实现前取得 RED（T1：`terminal=PageId(7) / tail=PageId(2)`〔两例〕与 DELETE-only `tail=PageId(5)`；T2：2/4 行可达）；`clean_open_untouched` 全程绿（`redo_count == 0` 零变化）。

### 验证

- T1 目标（实现后 GREEN）：`cargo test --test recovery_tail_persistence_test -- --test-threads=1` → `4 passed; 0 failed`，exit 0。
- T1 定向回归：`cargo test --test wal_recovery_large_test --test recovery_replay_convergence_test --test overflow_page_test -- --test-threads=1` → `7 + 3 + 27 passed; 0 failed`，exit 0。
- T2 目标：`cargo test --lib table_manager::tests::stale_tail_heals_on_write -- --test-threads=1` → `1 passed`，exit 0（RED 先行：`left: 2, right: 4`）。
- T2 回归：`cargo test --lib -- --test-threads=1` → `394 passed; 0 failed`，exit 0。
- T3 校准：`cargo test --test check_command_test -- --test-threads=1` → `8 passed; 0 failed`，exit 0（`test_check_unreferenced_overflow_is_info_exit_0` 同批绿——info 分级保留见证）。
- T3 定向回归（5 套件）：catalog_durability 4 + mvcc_tombstone_visibility 11 + overflow_page 27 + recovery_replay_convergence 3 + wal_recovery_large 7 = `52 passed; 0 failed`。
- fmt/clippy：`cargo fmt --check` 干净（两处本次新增代码格式差异已应用 fmt 后复查通过）；`cargo clippy --tests` 无告警。
- **M19 全量分批**（`--lib`/`--bins` + 112 集成目标按名字典序 10 批，串行 `--test-threads=1`）：每批 `0 failed`，合计 **1802 passed / 0 failed / 2 ignored**（MS27 基线 1797 + 本 change 净增 5：recovery_tail_persistence_test 4 + stale_tail_heals_on_write 1，数目闭合）。
- `openspec validate 2026-10-03-recovery-tail-persistence` → `valid`。

### Persisted Evidence

- None required（Mode: none 维持不变；全部验证可低成本重跑，决定性输出已录本 Response）。

### Experience Candidates

- None（夹具锁纪律为 MS07-T01 既有文档化语义的测试面应用；M19 批次命令构造为一次性执行失误，均不构成 Runbook/Issue 准入）。

### 未解决问题

- 无。R1–R3 全部可验证成立；父 Cycle 确立的 Acceptance gap（DELETE-only 欠覆盖）经 T1-R1 机制修订与 T1-R2 见证闭合。

## Plan Review

- Review Result: accepted

### Findings

非阻塞 Minor finding 一项；偏差三项全部核实为非实质；无 Acceptance gap。

**独立代码与 diff 核读（对照 Act 声明，全部一致）**

1. **T1-R1**：`persist_replayed_tails`（`src/wal/recovery.rs:661-696`）——`scan_tables()` → `kind == CatalogKind::Table` 过滤 → `get_table` → 内存 tail 与 `row.data_page_tail` 差异比较 → `update_table_tail`；三处失败（scan/get/update）统一 `WalError::RedoFailed` 且文案点名表与操作；门控 `if redo_count > 0` 位于 redo 循环后、`mark_uncommitted_aborted` 前（:630-633）。未触碰三臂、`write_tuple_to_data_page` 与 `table_last_page`（Invariants 保持）——`table_last_page` 触点仍仅限 `redo_tuple_at_row_id` 内。修订机制在结构上覆盖任意重放臂，父 Cycle gap（DELETE-only 绕行臂欠覆盖）按 design D1（rework 修订版）闭合。
2. **T2**：`write_tuple`（`table_manager.rs:1638-1660`）经 `heal_stale_tail`（:1668-1716）——`page_count` 页界校验（越界 `InvalidPageId`）、visited 防环（环 `Internal` 点名表）、逐跳单次 header 探测、`terminal == tail` 早退（健康路径单次探测无链行走查）、heal 命中时内存 + `update_table_tail` 双写回并以 terminal 为追加目标；`old_tail` 取 heal 后值使既有持久化条件免去冗余写（design D2 授权的最小改动取舍）。`write_tuple_to_data_page` 零改动（`data_page.rs` diff 统计与 MS27 审计时一致：133 行）——恢复面隔离保持。
3. **T1-R2 见证有效性独立确认**：DELETE-only 夹具对「无修复」与「父 Cycle 旧机制（`table_last_page` keys）」两个状态均为 RED（两状态下 `t` 均不在写回集合、恢复 + close 后 `is_complete()` false）——正是 gap 的直接见证；几何算术（单行页空闲 10B < 28B 墓碑 footprint）以文件头注释 + 夹具自检（DELETE 后链必须增长）双保险，空转通过被结构性排除。
4. **T3 校准**：仅 `test_check_pending_wal_exit_1_then_recovers` 末段断言替换（info 行 → 干净单行 `check ok`），注释援引 design D3 白名单；前半段 pending WAL exit 1 不变；其余 7 用例（含 `test_check_unreferenced_overflow_is_info_exit_0`——info 分级保留的存量指示器见证）零改动。
5. **幂等测试断言时点核实**：`repeated_recovery_converges` 的 `is_complete()` 仅在 close → 重开后断言——正确规避「恢复期 catalog tail 写为脏页、崩溃 drop 即失、重放重写收敛」窗口；与通道语义（catalog 更新不写 WAL、靠 checkpoint 刷盘）自洽。

**偏差分类**

1. 夹具锁纪律（close 只 checkpoint 不释 flock、显式 drop 后再重开）→ Plan 预留的非实质夹具组织自由度（Risks and Notes），MS07-T01 既有语义的测试面应用；非实质。
2. 编译机械项（`DataChainStatus` 无 `Debug`、`with_page_data` Result 包裹）→ 非实质。
3. M19 批次执行失误（首轮多目标命令构造错误——cargo 多目标需逐个 `--test` 旗标、批量名成多余参数、管道 grep 掩盖失败退出码致各批实际未执行；次轮转写遗漏 `write_type_conformance_test`，经 112 目标清单核对发现后单独补跑）→ Act 执行层失误且**自行发现并修正**，最终覆盖闭合（112 集成目标 + lib/bins）；非测试或产品行为问题。分类：Act 偏离（执行过程）·已自纠，不阻塞。**过程注记（Minor，记录不返工）**：M19 分批验证中「管道 grep 掩盖 cargo 退出码」会吞掉整批未执行的事实——后续 M19 批次宜让 cargo 退出码直接可见（如先跑后过滤，或 `set -o pipefail` 习惯），避免同类静默空跑。

**范围外 Issue 候选**：None。

### Acceptance Gaps

None。

- R1：`recovery_tail_persistence_test` 4 用例（INSERT 驱动两态〔含 CLI check 单行 exit 0 + dump 无告警〕/ 幂等 / `redo_count == 0` 零变化 / DELETE-only）+ 恢复定向回归（wal_recovery_large 7 / recovery_replay_convergence 3 / overflow_page 27）——代码与测试独立读毕。
- R2：`stale_tail_heals_on_write`（内存与 catalog tail 双修正断言 + `count_live_rows` 全链 4 行可达）+ 健康路径单次探测（代码核读）+ lib 394/0。
- R3：check_command_test 8/8（白名单校准 + info 保留用例）+ dump 干净见证 + 定向 5 套件 52/0 + M19 全量 **1802 / 0 failed / 2 ignored**（1797 基线 + 净增 5，数目闭合）+ fmt/clippy 干净 + `openspec validate` valid。

### Evidence

- 独立阅读（本 Review）：`src/wal/recovery.rs` :626-705（门控与 `persist_replayed_tails` 全文）、`src/storage/data/table_manager.rs` :1638-1716（`write_tuple` + `heal_stale_tail` 全文）与 :2180-2276（`count_live_rows` + 单元测试全文）、`tests/recovery_tail_persistence_test.rs` 全文（289 行）、`tests/check_command_test.rs` 校准段；`git diff --stat` 范围核对（增量仅 recovery.rs +48 / table_manager.rs +143 / 新测试文件，`data_page.rs` 等其余文件与 MS27 审计时统计一致——无范围外修改）。
- 采信 Act Response 未失效结论（2026-10-03 新鲜执行，其后工作区未变化）：各目标命令输出与 M19 全量 12 批 1802/0/2、fmt/clippy、`openspec validate` valid、RED 见证数值（terminal=7/tail=2、DELETE tail=5、2/4——均与本 Review 的独立行为推演吻合）。Plan Review 未触发公共规则 › 验证 的重跑情形，不重复运行。

### Follow-up Decision

接受（`accepted`）：repair items T1-R1/T1-R2 与继承的 T2/T3 全部按契约交付，三项偏差均为非实质且已记录；父 Cycle 确立的 Acceptance gap 经机制修订与直接见证闭合，R1–R3 全部可验证成立。无当前 Cycle 修复项、无 rework/replan 后继。Iteration 000（经 001-rework）完成；本 change 为单 Iteration change，实施面全部收口——后续收尾（delta spec 合并、SNAPSHOT/tasks 刷新、归档）属 `openspec-docs-maintainer` 职责，等待用户指令。

### Next Cycle

None

### Next Iteration

None（Iteration Map 无剩余 Iteration；change 实施完成）
