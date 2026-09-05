# proposal: MS07 剩余工作（显式事务 / Checkpoint / 谓词与 LIMIT 下推）

## Why

MS07-T01（schema 持久化）、T02（drop_table 物理释放）、T03（planner 模块化）已完成并归档（`archive/2026-08-30-*`）。MS07 剩余四个子项中，T04/T05/T06 是要真正交付的基础能力，T07 是依 T04/T05 结论的并发协调手段（见 Out of Scope）。

- **T04 显式事务缺失**：`Database` 没有 `begin/commit/rollback` 公开 API。当前每个 DML 在 `execute_stage`（`src/pipeline.rs:116-183`）内被隐式包裹为独立事务（begin→execute→commit/abort），用户无法把多语句放进同一个事务，也就无法保证"要么全部成功要么全部回滚"的原子性。
- **T05 Checkpoint 名存实亡**：`CheckpointManager::checkpoint()`（`src/wal/checkpoint.rs:83-110`）只做"取 LSN → 刷脏页 → 写位点文件 → 写 Checkpoint WAL 记录 → reset_write_count"。但恢复端 `RecoveryManager::full_recover`（`src/wal/recovery.rs:60-131`）**从 WAL 头全量重放**，从不调用 `read_checkpoint_site`。即 checkpoint 位点写而不用，崩溃恢复的 redo 量与 checkpoint 无关，WAL 无界增长。恢复路径还残留 MS06 前定案的静默吞错（K05：recovery.rs:116/148/165/179/193）。
- **T06 谓词/LIMIT 无下推**：planner 对非 PK WHERE 产生 `Filter(DataScan/Scan)`（`src/parser/planner/query.rs:311-327`），LIMIT 是顶层 `Limit` 节点（query.rs:490-498）。扫描先物化全部匹配行、过滤/截断在上层执行，无法提前终止或行内过滤。MS07-T03 已把 planner 拆为 `query.rs`/`expression.rs`，为下推提供了清晰落点。

## What Changes

把 MS07 剩余可交付能力合并为一个 OpenSpec change，按功能拆为 3 个逻辑 Iteration（T04 → T05 → T06），各自可独立验证：

- **Iteration 000（T04 显式事务）**：公开 `Database` 层事务 API（`begin`/`commit`/`rollback`）+ 事务内执行路径（复用事务 id、不做隐式提交）+ 显式多表事务回滚的版本表标记。隐式 `execute_sql` 在无显式事务时可执行路径保持现有自动包裹，向后兼容。隔离级别/快照语义本轮不变（见 Out of Scope，对齐 MS09）。
- **Iteration 001（T05 Checkpoint）**：checkpoint 恢复端消费位点（跳过已落盘前缀以缩减 redo），把 `CheckpointManager` 接入数据库关闭/写入循环，并把恢复路径静默吞错（K05）改为显式错误。引入 checkpoint 截断 WAL 的决策。
- **Iteration 002（T06 谓词/LIMIT 下推）**：把 WHERE 谓词并入 DataScan/IndexScan 执行器（行内过滤，避免物化后再过滤），把无 Sort/OrderBy 的 LIMIT 下推进扫描以提前终止。

## Capabilities

### New Capabilities

- `ms07-rest-tx-checkpoint-pushdown`：MS07 剩余三项基础能力的合并能力自审计于 3 个 Iteration。改前：无显式事务 API、checkpoint 位点不被恢复消费、谓词/LIMIT 不进入扫描。改后：DDL/DML 显式事务可单测；重启后 redo 从 checkpoint 位点裁剪、静默吞错显式化；谓词/LIMIT 在扫描层提前过滤/终止。
  - 关联 M/K：`M10`（事务版本链）、`M11`（WAL 崩溃恢复）、`M19`（DataScan 路径）、`M21`（页面级 MVCC 摘要）、K05（WAL 静默吞错遗留）。

### Out of Scope（本 change 不做）

- **MS07-T07 消息传递重构**：不纳入本 change。T04 采用"属有事务句柄 + 显式 in-tx 执行"，不引入共享"当前事务"可变状态；T05 checkpoint 若在 Iteration 001 实现中暴露与 WAL flush actor 的并发协调需求，**单独**评估新 change，不在此扩大范围。
- **隔离级别 / 快照隔离正确性**：显式事务本轮只提供生命周期与原子性，不改变读可见性语义（扫描当前用 `snapshot: None`，全见）。Read Committed 属 MS09-T01。
- **新 SQL 方言 / 新执行器类型 / 网络协议变更**。
- **伤及 schema 持久化、drop_table、planner 模块化的回归**。

## Impact

- **影响模块**：
  - T04：`src/database.rs`（新增 `begin`/`commit`/`rollback`/`execute_in_tx`）、`src/pipeline.rs`（显式事务内执行路径 + 隐式兼容分支）、`src/transaction/manager.rs`（`record_version`/`tx_versions` 表标记以支持多表回滚）、`src/executor/{insert,update}.rs`（`record_version` 调用点表名）。
  - T05：`src/wal/{checkpoint,recovery,reader,writer}.rs`、`src/database.rs`（接线 `CheckpointManager`）。
  - T06：`src/parser/planner/{query,expression}.rs`、`src/executor/{data_scan,index_scan,index_scan_all,scan,filter,limit}.rs`。
- **影响接口**：T04 新增公开 API（`Database::begin/commit/rollback/execute_in_tx`）；`execute_sql` 行为不变（无显式事务时）。T05/T06 不改变公共 SQL/网络接口。
- **影响行为**：T04 显式事务提供跨语句原子性；T05 重启 redo 从 checkpoint 位点裁剪、静默错误转为显式错误；T06 查询结果不变，仅物理扫描方式优化。
- **兼容性**：隐式 `execute_sql` 与既有集成/单元测试必须全绿；网络协议与 SQL 方言不变。
- **风险**：
  - **T04 多表回滚**（中）：当前 `abort_cleanup_versions` 单 `TableMeta`，需把版本按表标记才能回滚多表事务——改为 `tx_versions` 按 `(table, row_id)` 或等价结构。
  - **T05 恢复语义**（中）：checkpoint 位点消费必须保证"已落盘前缀 + 后续 WAL"的重放正确性（幂等、不丢/不多）；静默吞错显式化可能使原本"假装成功"的损坏表启动报错。
  - **T06 下推正确性**（中）：谓词下推必须严格保持 Filter 语义（含 NULL、类型、错误条件），LIMIT+Sort/OrderBy 组合不得被错误提前终止。
- **回退方案**：逐 Iteration 独立提交，任意 Iteration `git revert` 不影响其他。

## 关联

- 关联里程碑：**MS07**（基础能力建设）；完成 Iteration 000/001/002 后 MS07-T04/T05/T06 达标。
- 后续决策（不在本 change）：MS07-T07（消息传递）、MS09 隔离级别。