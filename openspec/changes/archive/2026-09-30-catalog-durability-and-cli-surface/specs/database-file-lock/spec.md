# database-file-lock Delta — 锁冲突有界退避重试

## MODIFIED Requirements

### Requirement: 打开即独占文件锁

读写模式的 `FileStorage` 打开（`FileStorage::open`/`open_with_key` 及其上游 `Database::open`/`open_with_isolation`/`open_with_key`）SHALL 在打开主数据库文件时对其文件描述符获取 advisory 独占锁（`std::fs::File::try_lock`，非阻塞）。锁被其他持有者占用（`WouldBlock`）SHALL 先做**同描述符有界退避重试**（短间隔、总窗口 ~8ms 量级、有限次数），窗口耗尽仍被占用方 SHALL 返回 `StorageError::DatabaseLocked`（携带路径信息）。重试仅吸收平台 flock 释放可见性窗口（同进程 close→立即重开时，内核锁释放在负载下可能晚于下一次加锁尝试到达——design D10 取证：WSL2 6.18.33.2 纯 std 探针 20000 轮 × 8 路负载 2 次 EAGAIN；活跃持有者不受影响，窗口耗尽后照旧拒绝）。锁系统调用本身失败（`WouldBlock` 以外的 IO 错误，如文件系统不支持 flock）SHALL 按普通 IO 错误传播，不得降级为无锁运行。锁 SHALL 在打开 WAL 与执行崩溃恢复之前完成。只读模式打开（`open_readonly`/`open_readonly_with_key`）SHALL 对共享锁（`try_lock_shared`）采用同款有界重试，窗口耗尽后返回 `DatabaseLocked`；只读模式的共享锁语义（多读单写矩阵）由 `readonly-open-mode` 域承载，本 Requirement 的独占语义 SHALL NOT 适用于只读打开。

#### Scenario: 单进程独占打开成功

- **GIVEN** 一个未被其他持有者锁定的数据库文件
- **WHEN** 读写模式 `Database::open` 打开该文件
- **THEN** 打开成功，锁保持到 Database 生命周期结束，执行与查询行为与加锁前完全一致

#### Scenario: 第二持有者打开被拒

- **GIVEN** 某持有者已对 `x.db` 持有 flock（另一进程或测试进程），且持有跨越重试窗口
- **WHEN** 第二方以读写模式打开 `x.db`
- **THEN** 打开失败并返回 `StorageError::DatabaseLocked`，主文件数据与 WAL 未被第二方触碰；拒绝发生在重试窗口耗尽之后（时延增加窗口量级，语义不变）

#### Scenario: 同进程第二实例被拒

- **GIVEN** 同一进程内读写模式 `Database::open(path)` 已返回且未 drop
- **WHEN** 再次以读写模式 `Database::open(path)`（同一文件）
- **THEN** 第二次 open 在重试窗口耗尽后返回 `StorageError::DatabaseLocked`（flock 按 open-file-description 互斥，不因同进程而豁免，有界重试 SHALL NOT 使活跃持有被绕过）

#### Scenario: close 后立即重开可达

- **GIVEN** 同一进程内读写会话对 `x.db` 打开后正常 close/drop（锁随 fd 释放）
- **WHEN** 立即再次以读写模式打开 `x.db`，如此循环 ≥ 300 轮
- **THEN** 每轮打开成功、数据可读写（平台的锁释放可见性窗口由有界重试吸收，不产生假 `DatabaseLocked`）

#### Scenario: 锁系统调用失败严格报错

- **GIVEN** 底层文件系统不支持 flock（`try_lock` 返回 `ENOLCK` 等非 `WouldBlock` 错误）
- **WHEN** 读写模式 `Database::open`
- **THEN** 按普通 IO 错误失败（CLI 退出码 1），不降级为无锁打开，且重试路径 SHALL NOT 掩盖非 `WouldBlock` 错误
