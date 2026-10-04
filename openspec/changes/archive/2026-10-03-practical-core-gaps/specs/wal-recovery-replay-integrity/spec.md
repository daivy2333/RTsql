## ADDED Requirements

### Requirement: 恢复重放目标表缺失的错误面诚实

恢复重放（`redo_record` 的 Insert/Update/Delete 三臂）遇「WAL 已提交记录引用的表在 catalog 中不存在」时，错误信息 SHALL 点名该表名、陈述「WAL 记录引用的表在 catalog 中缺失」的事实，并陈述最可能原因——数据库上次关闭未经 checkpoint（如 lib `Database` 句柄未调用 `close()` 即 drop，未持久化的 DDL 随 catalog 脏页丢失）。打开 SHALL 维持失败（既有 K05 显式报错语义不变）：恢复 SHALL NOT 跳过、静默丢弃或自动清理不可重放的已提交记录。

其他 redo 失败类别（页 I/O、索引操作、PK 派生失败等）的既有错误文案 SHALL 零变化；打开失败的退出路径与错误传播方式 SHALL 零变化（本 Requirement 只改错误面指因，不改恢复行为与打开结果）。

#### Scenario: 句柄 drop 不 close 后重开的错误指因

- **GIVEN** lib 打开数据库，`CREATE TABLE` + 提交 `INSERT` 后句柄直接 drop（未 `close()`）——catalog 脏页丢失、WAL 留有已提交记录
- **WHEN** 重新 open
- **THEN** open 失败，错误信息含表名、catalog 缺失事实与「上次关闭未经 checkpoint（如句柄未 close 即 drop）」的可能原因（失败边界——修复前：`WAL redo failed: table 't' lookup failed during redo: table not found: t`，将 DDL 未持久化误诊为表缺失）

#### Scenario: 正常关闭不触发定向错误面

- **GIVEN** 同上序列但句柄经 `close()` 正常落盘
- **WHEN** 重新 open 并查询该表
- **THEN** open 成功、行可见（定向错误面只覆盖无 checkpoint 关闭形态，正常恢复路径不误伤）
