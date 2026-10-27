# database-file-lock Specification

## Purpose
读写打开对主数据库文件取得非阻塞 advisory 独占锁，锁被占用时按等待预算重试，预算耗尽后返回 `DatabaseLocked`；只读打开对共享锁使用同一等待预算。来源：`2026-09-06-ms10-t02-file-lock-graceful-shutdown`
## Requirements
### Requirement: 打开即独占文件锁

读写模式的 `FileStorage` 打开（`FileStorage::open`/`open_with_key` 及其上游 `Database::open`/`open_with_isolation`/`open_with_key`）SHALL 在打开主数据库文件时对其文件描述符获取 advisory 独占锁（`std::fs::File::try_lock`，非阻塞）。锁被其他持有者占用（`WouldBlock`）时 SHALL 按**锁等待预算**周期性重试：预算 SHALL 可经 lib 打开面与 CLI 通道（`cli-noninteractive-shell` 域）配置，默认 **5000ms**；预算内锁被释放 SHALL 立即获得并正常打开，预算耗尽仍被占用方 SHALL 返回 `StorageError::DatabaseLocked`（携带路径信息），错误面与退出码不变。内部 ~8ms 量级的有界退避 SHALL 始终生效并构成等待下限（吸收平台 flock 释放可见性窗口——同进程 close→立即重开场景；用户预算小于该下限时实际等待取下限），该下限不属于用户等待策略。锁系统调用本身失败（`WouldBlock` 以外的 IO 错误，如文件系统不支持 flock）SHALL 按普通 IO 错误传播，不得降级为无锁运行。锁 SHALL 在打开 WAL 与执行崩溃恢复之前完成。只读模式打开（`open_readonly`/`open_readonly_with_key`）SHALL 对共享锁（`try_lock_shared`）采用同款等待预算语义；只读模式的共享锁语义（多读单写矩阵）由 `readonly-open-mode` 域承载，本 Requirement 的独占语义 SHALL NOT 适用于只读打开。

#### Scenario: 单进程独占打开成功

- **GIVEN** 一个未被其他持有者锁定的数据库文件
- **WHEN** 读写模式 `Database::open` 打开该文件
- **THEN** 打开成功，锁保持到 Database 生命周期结束，执行与查询行为与加锁前完全一致

#### Scenario: 等待预算内获得锁

- **GIVEN** 某持有者已对 `x.db` 持有独占 flock，且将在等待预算内释放（如 one-shot 命令毫秒级完成）
- **WHEN** 第二方以读写模式打开 `x.db`
- **THEN** 第二方阻塞等待，持有者释放后打开成功，执行与查询行为与无争用打开完全一致

#### Scenario: 第二持有者打开被拒

- **GIVEN** 某持有者已对 `x.db` 持有 flock（另一进程或测试进程），且持有跨越等待预算
- **WHEN** 第二方以读写模式打开 `x.db`
- **THEN** 打开失败并返回 `StorageError::DatabaseLocked`，主文件数据与 WAL 未被第二方触碰；拒绝发生在等待预算耗尽之后，CLI 错误面维持 exit 4 不变

#### Scenario: 零预算回到立即拒绝

- **GIVEN** 某持有者已对 `x.db` 持有 flock 且跨越内部下限窗口
- **WHEN** 第二方以等待预算 0（flag/env/lib 显式配置）打开 `x.db`
- **THEN** 仅内部 ~8ms 量级下限窗口的重试后返回 `StorageError::DatabaseLocked`（当前行为的显式表达，不额外等待）

#### Scenario: 只读共享锁同款等待

- **GIVEN** 一个写持有者以读写模式打开 `x.db` 并在等待预算内 close
- **WHEN** 第二方以只读模式（`open_readonly`）打开 `x.db`
- **THEN** 共享锁按同款等待预算重试，写持有者释放后打开成功；写持有者不释放时预算耗尽返回 `DatabaseLocked`

#### Scenario: 同进程第二实例被拒

- **GIVEN** 同一进程内读写模式 `Database::open(path)` 已返回且未 drop
- **WHEN** 再次以读写模式 `Database::open(path)`（同一文件），等待预算为默认值
- **THEN** 第二次 open 在等待预算耗尽后返回 `StorageError::DatabaseLocked`（flock 按 open-file-description 互斥，不因同进程而豁免；等待 SHALL NOT 使活跃持有被绕过）

#### Scenario: close 后立即重开可达

- **GIVEN** 同一进程内读写会话对 `x.db` 打开后正常 close/drop（锁随 fd 释放）
- **WHEN** 立即再次以读写模式打开 `x.db`，如此循环 ≥ 300 轮
- **THEN** 每轮打开成功、数据可读写（平台的锁释放可见性窗口由内部下限吸收，不产生假 `DatabaseLocked`；该回归不依赖用户预算配置）

#### Scenario: 锁系统调用失败严格报错

- **GIVEN** 底层文件系统不支持 flock（`try_lock` 返回 `ENOLCK` 等非 `WouldBlock` 错误）
- **WHEN** 读写模式 `Database::open`
- **THEN** 按普通 IO 错误失败（CLI 退出码 1），不降级为无锁打开，且等待路径 SHALL NOT 掩盖非 `WouldBlock` 错误

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

