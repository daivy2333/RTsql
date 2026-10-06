## ADDED Requirements

### Requirement: 恢复会话 WAL 收敛（含零 redo 形态）

读写打开完成恢复后，若打开时 WAL 含任一非 Checkpoint 记录（无论 redo 是否实际发生），该会话的 `close()` 与显式 `checkpoint()` SHALL 执行完整 checkpoint 并截断 WAL——「打开时 WAL 曾有事务记录」SHALL 视同未决工作参与零工作判定。收敛后 `rtsql check` 与 `--readonly` 打开 SHALL 正常（文档承诺「open read-write once to recover, then check」对纯读恢复会话成立）；`rtsql checkpoint` 在该状态 SHALL 报告实际执行（`wal truncated; checkpoint site advanced`）。WAL 从未含事务记录的形态（纯读会话、干净库）SHALL 保持 MS18 零工作跳过语义（位点与 WAL 字节不变）。已提交数据崩溃恢复（redo > 0）的既有收敛路径 SHALL 零变化；恢复重放的数据正确性语义（未提交丢弃、已提交完好）SHALL 零变化。

#### Scenario: 未提交事务崩溃后纯读会话收敛

- **GIVEN** 已提交数据完好的库 + 显式事务写入中途 SIGKILL（WAL 含未提交记录、数据页无该行）
- **WHEN** 纯 SELECT 的读写打开（触发恢复，数据正确）→ close → `rtsql check`
- **THEN** check exit 0 单行干净（失败边界——修复前：永久 exit 1 "pending WAL"，与 checkpoint 命令 "no pending work" 构成死循环）

#### Scenario: checkpoint 命令报告实际执行

- **GIVEN** 同上恢复后的库（会话未再写入）
- **WHEN** `rtsql checkpoint <db>`
- **THEN** 输出 `wal truncated; checkpoint site advanced`（失败边界——修复前：`no pending work; wal untouched`）

#### Scenario: 零工作跳过保持

- **GIVEN** WAL 从未含事务记录的干净库
- **WHEN** 纯读会话 close；显式 checkpoint
- **THEN** 位点文件与 WAL 字节不变（MS18 语义保持，checkpoint_skip_test S1/S2/S6 语义）

#### Scenario: marker-only WAL 收敛

- **GIVEN** WAL 仅含空事务标记记录（BEGIN/COMMIT 无数据写入）的崩溃残留
- **WHEN** 纯读读写打开 → check
- **THEN** 收敛 exit 0（修复前：同样楔死于 pending WAL 拒绝）

#### Scenario: redo 收敛路径零变化

- **GIVEN** 含已提交写入的崩溃库
- **WHEN** 读写打开（恢复重放）→ close → check
- **THEN** 行为与修复前一致（checkpoint_skip_test S5、check_command_test 既有用例零回归）
