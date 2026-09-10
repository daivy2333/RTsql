# wal-recovery-replay-integrity Specification

## MODIFIED Requirements

### Requirement: 重放保持 DML 语义

Update 与 Delete 的重放 SHALL 复现运行期全部副作用：Update 新版本 SHALL 写入记录携带的位置并重建 `next_version → old_row_id` 版本链。old_row_id SHALL 与运行期同源推导：`old_tuple` 键位值可键控（Int）时由 PK 推导（重放形态下经磁盘版本多映射，沿用 max rid < 记录 row_id + old_tuple 逐字节校验语义）；键位值不可键控（NULL / 非 Int）时 SHALL 经无键行回退推导——恢复预扫描与重放按 tuple 原始字节为无键行建立候选集并沿重放追加分派，old_row_id 从该候选集以同一 max-rid + 逐字节校验语义派生，SHALL NOT 因键位值不可键控而 RedoFailed。Delete SHALL 同时重放数据页墓碑（`mark_deleted`）与索引清理。恢复后查询语义（更新可见新值、已删行不可见、行数为已提交终态）SHALL 与原运行一致。

#### Scenario: Update/Delete 混合负载恢复语义正确

- **GIVEN** 含 checkpoint 中位点与驱逐规模的 INSERT/UPDATE/DELETE 混合负载 WAL，崩溃（drop 不 close）后重开
- **WHEN** 查询被更新行、被删除行与总行数
- **THEN** 被更新行返回新值、被删除行不可见、总行数精确等于已提交终态（此前：追加式重放产生重复新版本且丢失墓碑与版本链）

#### Scenario: 无键行 Update 崩溃恢复语义正确

- **GIVEN** 表含键位（PK 列，未声明时为第一列）值为 NULL 或非 Int 的行，行经 INSERT 与 UPDATE 后进程崩溃（drop 不 close）
- **WHEN** `Database::open` 执行崩溃恢复
- **THEN** 恢复成功（无 RedoFailed），被更新无键行返回新值，总行数精确等于已提交终态，可键控行恢复语义不受影响
