# wal-writer-handle-reuse

## Purpose

定义 WalWriter 在 MS06-T03 之后的文件句柄生命周期契约：单一句柄全程复用，fd 数量有可验证静态上界，错误语义与 LSN 语义对调用方零变化。

## ADDED Requirements

### Requirement: WalWriter 持久句柄复用

WalWriter SHALL 持有一个持久文件句柄，其全部 IO 方法（write_record / write_batch / fsync / truncate_to / get_current_lsn）SHALL 通过该句柄完成操作，不得逐次打开关闭文件。

#### Scenario: 顺序写入复用句柄

- **WHEN** 数据库打开后顺序执行多条写事务（每条产生 WAL write_record 或 write_batch）
- **THEN** 进程 fd 计数不随事务数量增长
- **AND** 全部记录成功落盘且 LSN 单调递增

#### Scenario: 并发写入复用句柄且无损坏

- **WHEN** 多个并发任务同时通过共享的 `Arc<WalWriter>` 写入
- **THEN** 无 fd 增长、无 panic、无交错写坏记录
- **AND** WAL 文件可通过 RecoveryManager 完整读取

#### Scenario: truncate 后同句柄继续追加

- **WHEN** checkpoint 调用 `truncate_to(lsn)` 截断文件
- **AND** 之后继续 `write_record` 追加
- **THEN** 新记录写在截断后的文件末尾（O_APPEND 语义）
- **AND** 后续 `get_current_lsn` 返回截断后真实长度

### Requirement: 句柄数上界可验证

在 10K 事务压测下，进程持有的与 WAL 相关的文件描述符 SHALL 少于 10 个；该约束 SHALL 可通过进程内 `/proc/self/fd` 计数断言在 cargo test 中自动验证。

#### Scenario: 10K tx 压测 fd 上界断言

- **WHEN** 集成测试在单进程中执行累计 10K 条写事务（含批量路径）
- **THEN** 压测期间 `/proc/self/fd` 条目计数始终低于 10 的净增量阈值（相对测试前基线）
- **AND** 测试结束后数据库正常 close，无句柄泄漏告警

### Requirement: 错误语义保持

WAL 写入失败 SHALL 保持现有错误传播行为：返回 `WalError::IoError` 上抛给调用方，不自动重试、不自动重新打开文件。

#### Scenario: 写失败直接上抛

- **WHEN** 底层写入发生 IO 错误
- **THEN** 调用方收到与现状一致的 `WalError::IoError`
- **AND** 不发生自动重开或静默重试
- **AND** 已持有句柄不因失败路径泄漏

### Requirement: LSN 文件位置语义保持

`write_record` SHALL 继续以写入前的文件末尾字节偏移作为返回 LSN；`write_batch` SHALL 继续使用调用方传入的 LSN 序列。LSN 权威来源保持为文件位置，不引入内存权威计数器。

#### Scenario: write_record LSN 等于文件偏移

- **WHEN** WAL 文件当前长度为 N 时调用 `write_record`
- **THEN** 返回的 LSN 等于 N
- **AND** 写入后文件长度为 N + record 序列化长度

#### Scenario: write_batch 使用调用方传入 LSN

- **WHEN** WALBuffer 以 `(lsn, record)` 序列调用 `write_batch`
- **THEN** 各记录按传入 LSN 序列化落盘，与现状一致
