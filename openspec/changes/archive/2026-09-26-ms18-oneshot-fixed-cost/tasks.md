# MS18 one-shot 路径性能收口 — Tasks

> 来源：MS18（`.agents/docs/tasks.md`）T01/T02 并入本 change（用户裁定 2026-09-26 单 change 双 Iteration）；README 刷新为收尾任务 T3。范围与决策记录见 proposal。
> 设计依据：`design.md` D1–D6。

## Task List

| Task | 内容 | Requirement | Iteration |
|---|---|---|---|
| T1 | Checkpoint 零工作跳过（D1/D2/D3/D4：`write_count==0 && 无脏页` 双条件守卫于 `CheckpointManager::checkpoint` 顶部、`BufferPool::has_dirty_pages()` 只读探测、跳过返回 `Ok(0)`、close doc 更新）+ 跳过矩阵测试 | R1（`ms07-rest-tx-checkpoint-pushdown` ADDED，6 场景） | 000 |
| T2 | CLI runtime flavor 实测定形（D5：两 flavor × 三组实测〔one-shot 时延 50 循环 / RSS one-shot+负载 / 负载 Elapsed〕×3 次取中位；判据 RSS≥15% 降且回归 ≤5% → 切 `current_thread` 单行变更，否则保持并留档） | R2（proposal 级，无行为 spec） | 001 |
| T3 | README 双语对比板块数字刷新（D6：R26 重测受影响数字，双语一致，引擎级数字不动） | R3（proposal 级，文档交付物） | 001 |

## Iteration Plan

### Iteration 000: Checkpoint 零工作跳过

- Tasks: T1
- Depends on: None
- Stable baseline: 零工作（WAL 无追加 + 池无脏页）close/显式 checkpoint 为整体无操作（位点文件字节不变或不存在、WAL 不变）；DML/DDL-only/crash 脏库/显式有工作路径全量行为逐字节不变；新库零工作 close 不产生位点文件且重开干净；全量测试零回归（既有 checkpoint/schema 套件零校准通过）
- Verification boundary: `tests/checkpoint_skip_test.rs` 矩阵 + `src/wal/checkpoint.rs` 守卫单测 + 既有 `checkpoint_test`/`checkpoint_redo_reduction_test`/`schema_persistence_test` 零校准 + 全量串行零回归 + one-shot 时延前后对比（R26 50 循环，effect 方向见证）
- Diagnostic boundary: `src/wal/checkpoint.rs`、`src/wal/writer.rs`（既有 `get_write_count` 复用）、`src/storage/buffer_pool.rs`（`has_dirty_pages`）、`src/database.rs`（close doc）、`tests/checkpoint_skip_test.rs`
- Non-goals: runtime flavor（T2）、README 刷新（T3）、WAL 格式/九步流程语义变更、阈值跳过、`open` 恢复后路径新接线

### Iteration 001: runtime flavor 实测定形与 README 刷新

- Tasks: T2, T3
- Depends on: Iteration 000（T2 实测基线须基于 T1 落地后的 close 路径——零工作 close 不再含 checkpoint 开销，flavor 差异不被淹没）
- Stable baseline: 两 flavor 三组实测数据齐备并留档 design.md D5；flavor 决策按批准判据执行（切换 = `main.rs` 单行变更且全量零回归，保持 = 无代码变更）；README 双语对比板块受影响数字刷新且双语一致
- Verification boundary: R26 口径三组实测执行记录（命令与输出入 Act Response）+ 判据套用结论 + 全量串行零回归 + README diff 双语一致检查
- Diagnostic boundary: `src/main.rs`、`README.md`、`README.zh-CN.md`、R26 runbook 测量流程
- Non-goals: Server 面 runtime、引擎级 criterion 重测（数字不受影响不刷新）、分配器/流式化/fsync 合并等 MS22 域、WAL fsync 语义变更

### 平衡审计

- Iteration 000 单任务形成完整可验收结果（性能守卫 + 行为等价矩阵），工作流为标准 TDD（RED→GREEN→回归），非过碎；诊断边界集中单一故障域（checkpoint 入口守卫）。
- Iteration 001 两任务同属「one-shot 固定开销收口的测量与定形」主题：T2 的三组实测中 one-shot 时延/RSS 正是 T3 README 要刷新的数字，测量一次两用；拆开会重复同一测量轮次。T2 可能无代码变更（判据不满足即保持），与 T3 合并为「测量→决策→文档落数」单一闭环，聚合不过重。
- 两 Iteration 故障域不同（存储引擎 checkpoint 行为 vs CLI runtime 与文档测量面），依赖边为顺序耦合（实测基线），无环。

## Requirements Traceability Matrix

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| R1 Checkpoint 零工作跳过（6 场景） | T1 | `tests/checkpoint_skip_test.rs`：zero_work_close_skips_site_and_wal（场景 1：位点/WAL 字节不变 + 重开等价）/ fresh_db_zero_work_close_creates_no_site（场景 2：RED→GREEN）/ dml_session_close_keeps_full_checkpoint（场景 3：位点字节变化 + WAL 截断 + 数据重开在）/ ddl_only_close_keeps_full_checkpoint_and_schema（场景 4：位点写入 + 重开建表在）/ crash_dirty_db_close_keeps_full_checkpoint（场景 5：drop 不 close → 重开 → close → 位点推进 + WAL 截断）/ explicit_checkpoint_zero_work_is_noop（场景 6）；`src/wal/checkpoint.rs` 单测 zero_work_checkpoint_returns_zero_without_site；既有套件零校准（checkpoint_test 3 / checkpoint_redo_reduction_test 9 / schema_persistence_test 8） | Covered |
| R2 CLI runtime flavor 实测定形 | T2 | R26 三组实测执行记录（两 flavor × 3 次 × 3 组，命令与决定性输出入 Act Response）+ design.md D5 数据回填 + 判据套用结论；若切换：全量串行零回归 + 既有 cli/pg/tx e2e 零校准 | Covered |
| R3 README 双语对比板块刷新 | T3 | README diff（受影响行数值更新 + 测量日期与口径注记，双语逐项一致）；测量命令与输出入 Act Response | Covered |

Scenario→Task→Test 链路完整，无 Simplified、无 Missing。
