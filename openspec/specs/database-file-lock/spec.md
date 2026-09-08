# database-file-lock Specification

## Purpose
TBD - created by archiving change 2026-09-06-ms10-t02-file-lock-graceful-shutdown. Update Purpose after archive.
## Requirements
### Requirement: 打开即独占文件锁

`FileStorage::open` SHALL 在打开主数据库文件时对其文件描述符获取 advisory 独占锁（`std::fs::File::try_lock`，非阻塞）。锁被其他持有者占用（`WouldBlock`）SHALL 返回 `StorageError::DatabaseLocked`（携带路径信息）；锁系统调用本身失败（`WouldBlock` 以外的 IO 错误，如文件系统不支持 flock）SHALL 按普通 IO 错误传播，不得降级为无锁运行。锁 SHALL 在打开 WAL 与执行崩溃恢复之前完成。

#### Scenario: 单进程独占打开成功

- **GIVEN** 一个未被其他持有者锁定的数据库文件
- **WHEN** `Database::open` 打开该文件
- **THEN** 打开成功，锁保持到 Database 生命周期结束，执行与查询行为与加锁前完全一致

#### Scenario: 第二持有者打开被拒

- **GIVEN** 某持有者已对 `x.db` 持有 flock（另一进程或测试进程）
- **WHEN** 第二方打开 `x.db`
- **THEN** 打开失败并返回 `StorageError::DatabaseLocked`，主文件数据与 WAL 未被第二方触碰

#### Scenario: 同进程第二实例被拒

- **GIVEN** 同一进程内 `Database::open(path)` 已返回且未 drop
- **WHEN** 再次 `Database::open(path)`（同一文件）
- **THEN** 第二次 open 返回 `StorageError::DatabaseLocked`（flock 按 open-file-description 互斥，不因同进程而豁免）

#### Scenario: 锁系统调用失败严格报错

- **GIVEN** 底层文件系统不支持 flock（`try_lock` 返回 `ENOLCK` 等非 `WouldBlock` 错误）
- **WHEN** `Database::open`
- **THEN** 按普通 IO 错误失败（CLI 退出码 1），不降级为无锁打开

### Requirement: 锁生命周期与自动释放

锁 SHALL 与主文件文件描述符同生命周期：Database/FileStorage drop 或进程退出（含 kill）时由内核自动释放，无需显式解锁协议。锁释放后 SHALL 无需任何清理即可重新打开。`Database` Clone 共享同一文件描述符（`Arc<FileStorage>`），重复加锁幂等，不产生自冲突。

#### Scenario: 正常关闭后可重开

- **GIVEN** 持有者打开 `x.db` 后正常退出（close + drop，或进程结束）
- **WHEN** 下一方打开 `x.db`
- **THEN** 打开成功，无锁残留

#### Scenario: 强杀后无死锁

- **GIVEN** 持有者持有 `x.db` 锁期间被 SIGKILL
- **WHEN** 下一方打开 `x.db`
- **THEN** 打开成功（内核对 advisory 锁的进程退出自动释放），WAL 恢复照常执行

