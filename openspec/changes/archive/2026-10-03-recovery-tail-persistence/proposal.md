# WAL 恢复重放 data_page_tail 持久化与写路径陈旧 tail 自愈

## Why

MS27 change `2026-10-02-ms27-overflow-integrity` Iteration 002 Plan Review 确认的 Issue 候选（域 `wal-recovery-replay-integrity`，用户 2026-10-03 裁定直接立 change 解决，原话「这个issue我们直接创建一个change进行解决，开始吧」）：

- **根因**：崩溃恢复重放只推进内存 `data_page_tail`（`redo_tuple_at_row_id` 步骤 4，`src/wal/recovery.rs:234-235`），无 catalog 写回；运行期 DML 经 `TableManager::write_tuple` 在新页分配时持久化 tail（`src/storage/data/table_manager.rs` `update_table_tail` 通道），恢复路径无对应调用。实证后果：恢复后健康库磁盘 tail 落后链 terminal（MS27 002 Cycle 测试实证 tail=2 / terminal=11）。
- **实证后果 ①**：`dump` 既有链完整性预检对恢复后库误告警「备份可能不完整，库可能已损坏」（rc 0，但噪音误导）。
- **实证后果 ②**：`rtsql check` 以 info 报告「terminal 领先持久化 tail」（MS27 引入的容忍分级）。
- **推演风险**：恢复后的库再次写入时以陈旧 tail 页为写目标；该页已满时分配新页并覆写其 `next` 指针，陈旧 tail 与 terminal 之间的已恢复页行存在链不可达（数据丢失）风险（未构造复现，代码推演）。**遗留窗口**：本修复前已恢复的存量库 tail 已陈旧且不会再被恢复路径触碰（重开时 WAL 已清、redo_count==0），该风险对其持续存在。

## What Changes

2026-10-03 用户批准集中决策（见「已批准的集中决策」）。本文件为需求基线，实现调查完成前 design/tasks/Gate 2 均为草稿。

- R1：崩溃恢复存在重放（redo_count > 0）时，恢复收尾 SHALL 把每个发生重放写入的表的 `data_page_tail` 经既有 `update_table_tail` 通道持久化到 catalog，使「恢复 → close → 重开」后持久 tail 与数据链 terminal 一致；写回失败 SHALL 显式报错（K05 语义不变）；重放重复执行（crash-during-recovery）SHALL 幂等；redo_count == 0 打开路径 SHALL 零变化。
- R2：运行期写路径以内存 tail 页为写入目标前 SHALL 检测陈旧形态（该页 `next` 指针非零 = tail 落后链 terminal）：检出时 SHALL 沿链前进到 terminal（自愈）、同步持久化修正后的 tail，再执行写入；SHALL NOT 覆写链中既有 `next` 指针导致已恢复页行不可达；健康库（tail 页 `next == 0`）SHALL 零额外常态 IO。
- R3：既有面零回归——恢复套件与 K05 错误面、页释放、索引重建行为不变；`rtsql check` 对「持久 tail 落后 terminal」的 info 分级保留（存量库指示器，不移除）；dump 链预检语义不变（新建恢复库因其 tail 一致而自然不再误告警，存量库保持既有告警行为）。

## 场景草图

| 场景 | 前置状态 | 动作 | 可观察结果及失败边界 |
|---|---|---|---|
| S1 恢复后 tail 两态一致 | checkpoint 后写入含跨页增长（页满分配新页），崩溃（drop 不 close） | `Database::open` 恢复 → close → 重开 | catalog 持久 tail == 链 terminal；`rtsql check` exit 0 且无 info 行；dump 无链告警。失败边界：tail 写回失败 → 恢复显式 Err（K05），不静默 |
| S2 恢复后写入全链可达 | 恢复完成的库（或存量陈旧 tail 库） | 持续写入至 tail 页满分配新页 | 新页挂链 terminal、陈旧 tail 与 terminal 间已恢复页行全部可达、tail 字段同步修正持久化；重开一致。失败边界（修复前）：next 被覆写、已恢复页行丢失 |
| S3 无重放零变化 | 干净库（无 WAL / redo_count == 0） | 打开与读写 | 打开路径不触碰 tail 持久化，行为与 IO 面不变 |
| S4 重复恢复幂等 | 恢复后未 checkpoint 再次崩溃重开 | 恢复重执行 | tail 写回幂等，行集与首次恢复一致 |
| S5 存量库观测面保持 | 存量陈旧 tail 库（WAL 已清） | check / dump | check 仍以 info 报告、exit 0（分级不移除）；dump 保持既有告警语义；首次写入触发自愈后转干净 |
| S6 既有恢复零回归 | 既有恢复测试面（驱逐规模 / 无键行 / 中止重插收敛 / 溢出行恢复） | 全量恢复套件 + 定向回归 | 全部零回归；K05 错误面不变 |

## 已批准的集中决策

1. 2026-10-03 用户裁定修复范围为 **A 双层修复**（选项「A 双层修复（推荐）」）：①恢复重放收尾按表持久化 tail（根因，新建恢复库两态一致）＋②写路径 tail 页陈旧自愈（追加前检测 tail 页 `next ≠ 0` 则沿链行查到 terminal 再写，零常态 IO）——同时关死存量库「陈旧 tail 页满覆写 next → 恢复页行丢失」的推演风险；`check` info 分支保留作为存量库指示器。备选否决：B 仅修恢复路径（存量风险保留为已知边界）、C 根因＋open 时逐表 heal（每表常态 +1 页读，干净打开成本回归）。

## Non-goals

存量库 open 时主动 heal（选项 C 已否决——干净打开不增加常态 IO，存量 tail 字段经首次写入自然修正）；页校验和与 `check` 扫描语义扩展；`check` info 分级移除或降级调整；dump 预检告警语义变更；VACUUM/GC 生产激活；`checkpoint` 位点格式演进。

## 工作状态与边界

- Phase 1 CLARIFY：completed；场景扫描、集中决策完成。
- Gate 1：PASS；2026-10-03 用户批准（选项「批准（推荐）」）。
- Phase 2 INVESTIGATE AND PLAN：completed；Current-State Evidence 与关键决策 D1–D4 见 design（D1 写回集合机制与 D4 见证类经 2026-10-03 rework Cycle 001 修订——父 Cycle Act 阻塞返 Plan：Delete 墓碑臂绕行 `redo_tuple_at_row_id` 致原机制欠覆盖）。
- Gate 2：PASS；2026-10-03 用户批准完整计划与 rework 计划（rework Cycle `001-rework` Status=ready，repair items T1-R1/T1-R2；T2/T3 契约继承）；尚无 Act 实施授权（父 Cycle 未进入实施，零代码产出）。
- Persisted Evidence：none；验证结果可低成本重跑。
- SKIPPED：产品代码修改、全局状态同步、实施、归档，均不属于本次 Plan 授权。

## Impact 与回退

关联域：`wal-recovery-replay-integrity`（MODIFIED 域 +ADDED Requirement）。影响面：`src/wal/recovery.rs`（恢复收尾写回）、`src/storage/data_page.rs` 或 `src/storage/data/table_manager.rs::write_tuple`（写路径自愈挂点，随调查定稿）、`tests/recovery_*` 与 `tests/check_command_test.rs`/`overflow_page_test.rs` 定向回归。

回退：代码回退后恢复行为回到现状（tail 不持久化）；本修复期间持久化过的 tail 字段值正确，对旧二进制无副作用（字段既有）。
