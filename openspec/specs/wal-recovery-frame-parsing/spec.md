# wal-recovery-frame-parsing Specification

## Purpose
TBD - created by archiving change 2026-09-06-ms10-t02-file-lock-graceful-shutdown. Update Purpose after archive.
## Requirements
### Requirement: 恢复解析逐帧无歧义

`WalReader` 对 WAL 流的逐帧解析 SHALL 无歧义且自验证：新格式帧（`[lsn:8][type:1][len:4][body][crc:4]`）以 CRC 验证为接受判据；在歧义偏移上新格式与旧格式（`[type:1][len:4][data]`，如 Checkpoint 记录）SHALL 都被尝试，恰好一种按其自身验证规则成功。解析 SHALL 不得依赖「帧首字节是否为合法 type 值」的启发式判别作为唯一依据。两条解析路径皆失败时 SHALL 显式报错（K05 语义不变）。

#### Scenario: 大 WAL 恢复成功

- **GIVEN** 一个 WAL 含 ≥19 条新格式记录（文件 >1KB，存在文件偏移低字节 ∈ 0x01-0x09 的记录）且帧完好
- **WHEN** `Database::open` 执行崩溃恢复
- **THEN** 恢复成功，全部已提交数据完整可查（此前：`Incomplete WAL record` 导致打开失败）

#### Scenario: 混合格式流恢复成功

- **GIVEN** WAL 同时含新格式记录与旧格式 Checkpoint 记录（`write_record` 路径写入）
- **WHEN** 恢复解析逐帧行走
- **THEN** 两种格式的记录均被正确解析，互不误判

#### Scenario: 损坏帧显式报错

- **GIVEN** WAL 帧真实损坏（截断/位翻转）
- **WHEN** 恢复解析
- **THEN** 显式报错（不静默跳过），`Database::open` 失败（K05 语义保持）

