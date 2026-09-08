# wal-recovery-replay-integrity Specification

## Purpose
TBD - created by archiving change 2026-09-06-ms10-t02-file-lock-graceful-shutdown. Update Purpose after archive.
## Requirements
### Requirement: 重放位置寻址且幂等

崩溃恢复的 redo SHALL 按记录携带的原始位置（`row_id`）重放数据变更，而非从 tail 盲目追加：目标槽位已存在时 SHALL 跳过，不存在时 SHALL 精确落在记录位置（页间切换按记录序列重建页链 next 指针与内存 tail）。在任何驱逐/刷盘状态下，重放结果 SHALL 与原运行提交状态一致；重放重复执行（crash-during-recovery 后重开）SHALL 收敛到同一状态。重放无法落位时 SHALL 显式报错（K05 语义不变）。

#### Scenario: 驱逐规模大 WAL 恢复数据精确

- **GIVEN** 数据规模超过 BufferPool 容量（>100 页）且 WAL 未 checkpoint（如 1 万行、50 行/显式事务、drop 不 close）
- **WHEN** `Database::open` 执行崩溃恢复
- **THEN** `COUNT(*)` 精确等于已提交行数（无重复、无丢失；此前：行数虚增、头部区域重复、原链尾部丢失且损坏经 checkpoint 持久化）

#### Scenario: 恢复后索引与数据一致

- **GIVEN** 同上场景恢复完成
- **WHEN** 对已存在 PK 重复 INSERT、对新 PK INSERT
- **THEN** 已存在 PK 被索引拒绝（DuplicateKey）、新 PK 成功——PK 索引内容与数据页一致（此前：重放行不在索引中）

#### Scenario: 恢复重跑幂等

- **GIVEN** 恢复完成后无新写入、未 checkpoint，进程再次崩溃后重开
- **WHEN** 恢复再次执行
- **THEN** 行数与内容与首次恢复完全一致

### Requirement: 重放保持 DML 语义

Update 与 Delete 的重放 SHALL 复现运行期全部副作用：Update 新版本 SHALL 写入记录携带的位置并重建 `next_version → old_row_id` 版本链（old_row_id 由 `old_tuple` 提取 PK 经索引推导，与运行期同源）；Delete SHALL 同时重放数据页墓碑（`mark_deleted`）与索引清理。恢复后查询语义（更新可见新值、已删行不可见、行数为已提交终态）SHALL 与原运行一致。

#### Scenario: Update/Delete 混合负载恢复语义正确

- **GIVEN** 含 checkpoint 中位点与驱逐规模的 INSERT/UPDATE/DELETE 混合负载 WAL，崩溃（drop 不 close）后重开
- **WHEN** 查询被更新行、被删除行与总行数
- **THEN** 被更新行返回新值、被删除行不可见、总行数精确等于已提交终态（此前：追加式重放产生重复新版本且丢失墓碑与版本链）

