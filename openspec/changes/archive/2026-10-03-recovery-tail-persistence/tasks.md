# 任务与逻辑迭代

需求于 2026-10-03 获批（Gate 1，集中决策 A 双层修复）；实现调查与设计见 design.md。复选框表示实施状态。2026-10-03 父 Cycle `000-initial` Act 阶段 T1 契约核读触发 Stop-when（Blocker Handoff：Plan「三臂全经 `redo_tuple_at_row_id`」断言失实，Delete 墓碑臂绕行致写回集合欠覆盖），Plan Review `rework-required`——rework Cycle `001-rework`（repair items T1-R1 写回集合机制修订 + T1-R2 DELETE-only 见证补口）承接实施，T2/T3 契约原样继承，Iteration Map 与 R1–R3 不变。

## 1. 恢复收尾 tail 持久化与写路径自愈（关联 Issue：恢复重放不持久化 data_page_tail）

- [x] 1.1 T1：恢复重放收尾按表持久化 tail——`full_recover` redo 循环后、`mark_uncommitted_aborted` 前，`redo_count > 0` 时遍历 `ctx.table_last_page` keys，逐表取内存 `data_page_tail` 经 `catalog.update_table_tail` 写回；失败映射 `WalError::RedoFailed`（文案点名表与操作）；`redo_count == 0` 零变化（design D1）（R1）；依赖 None；以 `tests/recovery_tail_persistence_test.rs` 两态一致/重复恢复幂等用例 RED→GREEN 验收。（2026-10-03 Act rework：写回集合经 T1-R1 修订为 catalog 表清单 + 内存/持久差异比较〔覆盖 Delete 墓碑绕行臂〕；recovery_tail_persistence_test 4 用例 GREEN——RED 见证 terminal=7/tail=2 与 DELETE-only tail=5；定向恢复回归 wal_recovery_large 7 / recovery_replay_convergence 3 / overflow_page 27 零回归）
- [x] 1.2 T2：写路径陈旧 tail 自愈——`TableManager::write_tuple` 写入前探测内存 tail 页 header `next_page_id`：`≠ 0` 沿链前进至 terminal（visited 防环）+ 更新内存 tail + `update_table_tail` 持久化修正值，再照常追加；`== 0` 直接落既有路径（design D2；自愈不得进入恢复面复用的 `write_tuple_to_data_page`）；失败映射同 T1 错误面（R2）；依赖 None（与 T1 并行可达，验证边界共享）；以 `stale_tail_heals_on_write` 单元测试 RED→GREEN 验收。（2026-10-03 Act rework：`heal_stale_tail` GREEN——RED 见证 2/4 行可达〔陈旧 tail 覆写 head.next 丢行形态〕；lib 394/0）
- [x] 1.3 T3：观测面校准与回归收口——`check_command_test::test_check_pending_wal_exit_1_then_recovers` 末段断言校准为恢复 + close 后干净单行 exit 0（design D3 白名单）；`recovery_tail_persistence_test` 补 CLI check/dump 干净见证；定向回归（recovery/overflow/tombstone/check/catalog/checkpoint 套件）+ fmt/clippy + M19 全量分批（R3）；依赖 T1/T2；以定向回归 0 failed + 全量收尾验收。（2026-10-03 Act rework：校准后 check_command_test 8/8〔info 分级用例不受影响〕；定向回归 5 套件 52/0；fmt/clippy 干净；M19 全量 112 集成目标 + lib/bins 分批串行 1802 passed / 0 failed / 2 ignored）

## Iteration Plan

### Iteration 000：恢复 tail 持久一致与自愈闭环

- Tasks：T1、T2、T3。
- Depends on：None。
- 阶段结果：恢复 → close → 重开后持久 tail == 链 terminal（check exit 0 无 info、dump 无告警）；存量陈旧 tail 库首次写入即自愈且已恢复页行全部可达；`redo_count == 0` 与健康写路径零变化；既有恢复/观测面零回归。
- Stable baseline：Issue 的根因、实证后果与推演风险全部关闭，可进入全量收尾。
- Verification boundary：新 `tests/recovery_tail_persistence_test.rs` + `stale_tail_heals_on_write` 单元测试 + check_command_test 校准 + 定向回归 + fmt/clippy + M19 全量分批。
- Diagnostic boundary：`src/wal/recovery.rs::full_recover`、`src/storage/data/table_manager.rs::{write_tuple, data_chain_status}`、`tests/check_command_test.rs`、catalog `update_table_tail` 通道消费。
- Non-goals：存量库 open 时主动 heal（决策 C 否决）；check info 分级移除；dump 预检语义变更；checkpoint 位点格式演进。
- 平衡审计：T1（根因）与 T2（风险闭环）同属「tail 跨重启收敛」单一故障域，共享夹具纪律（seed_pending_wal 同型）与验证边界（恢复套件 + check/dump 观测）；T3 是两者共同的验收收口且工作量薄（一处断言校准 + 见证补齐），单独成轮会形成过薄 Iteration。合为一轮形成「恢复库两态一致 + 存量库无丢失风险」的单一可验收结果。

## Requirements Traceability Matrix

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| R1 恢复重放收敛 tail 持久一致 | T1（rework：T1-R1/T1-R2） | `recovery_persists_tail_two_state_consistent` + `delete_only_replay_allocating_page_persists_tail` + `repeated_recovery_converges` + `clean_open_untouched` | Covered |
| R2 写路径陈旧 tail 自愈 | T2 | `stale_tail_heals_on_write` 单元测试 | Covered |
| R3 观测面与既有恢复语义零回归 | T3 | check_command_test 校准后矩阵 + dump 干净见证 + 恢复/溢出/墓碑定向回归 + M19 全量 | Covered |

R1–R3 全部 Covered，无 Simplified/Missing。

## Phase / Gate 与验证状态

- CLARIFY completed；Gate 1 PASS（2026-10-03 用户批准，集中决策 A 双层修复见 proposal）。
- INVESTIGATE completed；Current-State Evidence 见 design（运行期通道/恢复面结构/共享原语复用/观测面/测试锚点 file:line 核实）。
- DESIGN / TASKS / RTM completed；实质未知项 None。
- PLAN REVIEW completed（计划交付自检）；Gate 2 PASS——2026-10-03 用户批准完整计划（选项「批准完整计划（推荐）」）；当前 Cycle Status=ready，无 BLOCKED/WAIVED；`openspec validate` valid。
- Persisted Evidence：none；验证结果可低成本重跑，Act Response 足以承载决定性输出。
- SKIPPED：产品代码修改（Act 职责）、后续 Iteration 目录创建（单一 Iteration）。

最终回归按 M19：`--lib`/`--bins` 与全部集成测试目标按名字典序分批，每批约 8–12 个目标，串行独立 `cargo test <明确目标> -- --test-threads=1`；每批 0 failed 方可宣称全量。不使用无目标过滤的全量调用，不创建辅助判定脚本、身份字段或证据协议。
