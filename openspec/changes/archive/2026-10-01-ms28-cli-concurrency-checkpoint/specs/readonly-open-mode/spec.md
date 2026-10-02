# readonly-open-mode Delta — 共享锁冲突按等待预算执行

## MODIFIED Requirements

### Requirement: 只读打开入口与共享锁

系统 SHALL 提供只读打开模式：lib 经 `Database::open_readonly(path)` 与 `Database::open_readonly_with_key(path, key)`；CLI 主命令经 `--readonly` flag。只读打开 SHALL 对主数据库文件获取 advisory 共享锁（`try_lock_shared`），共享锁之间 SHALL 互不排斥（多个只读会话可同时打开同一数据库）。共享锁被其他持有者的独占锁排除（`WouldBlock`）时 SHALL 按 `database-file-lock` 域定义的锁等待预算周期性重试（默认 5000ms，可经 `--busy-timeout`/`RTSQL_BUSY_TIMEOUT`/lib 通道配置；内部 ~8ms 可见性窗口下限恒存），预算内锁释放 SHALL 打开成功，预算耗尽 SHALL 返回既有 `StorageError::DatabaseLocked`（CLI exit 4）。只读打开对不存在的数据库文件 SHALL 报错拒绝，SHALL NOT 创建文件。加密库只读打开 SHALL 按提供的密钥正常解密读取（错误密钥沿既有 `InvalidKey` 拒绝）。共享锁 SHALL 与文件描述符同生命周期（drop/进程退出自动释放），与既有独占锁的生命周期语义同型。

#### Scenario: 两只读会话并发打开与查询

- **GIVEN** 一个 checkpoint-clean 的数据库文件 `x.db` 含表 `t`
- **WHEN** 两个独立的只读连接先后打开 `x.db` 并各自执行 `SELECT * FROM t`
- **THEN** 两个连接均打开成功，查询结果一致且完整；任一连接关闭后另一连接不受影响

#### Scenario: 写者持独占锁时只读者被拒

- **GIVEN** 某持有者已以读写模式打开 `x.db`（独占锁在持，且持有跨越等待预算）
- **WHEN** 只读模式打开 `x.db`
- **THEN** 打开失败返回 `StorageError::DatabaseLocked`，主文件与伴生文件未被触碰（拒绝发生在等待预算耗尽之后；预算内释放则打开成功）

#### Scenario: 写者在预算内释放时只读者等待后可达

- **GIVEN** 某持有者以读写模式打开 `x.db` 并将在等待预算内 close
- **WHEN** 只读模式打开 `x.db`
- **THEN** 共享锁等待至独占锁释放后打开成功，查询行为与无争用打开一致

#### Scenario: 只读者持共享锁时写者被拒

- **GIVEN** 某持有者已以只读模式打开 `x.db`
- **WHEN** 读写模式打开 `x.db`
- **THEN** 打开失败返回 `StorageError::DatabaseLocked`（写写互斥语义同样排除写者，拒绝发生在写侧等待预算耗尽之后）

#### Scenario: 只读打开缺失文件不创建

- **GIVEN** 路径 `missing.db` 不存在
- **WHEN** 只读模式打开（lib 或 CLI `--readonly`）
- **THEN** 打开失败并报错点名路径，目录中 SHALL NOT 出现任何新文件

#### Scenario: 加密库只读打开

- **GIVEN** MS17 加密库
- **WHEN** `open_readonly_with_key(path, 正确密钥)` 后执行 SELECT
- **THEN** 查询结果与明文语义一致；错误密钥沿既有 `InvalidKey` 拒绝

#### Scenario: --readonly 与生命周期子命令组合拒绝

- **GIVEN** CLI
- **WHEN** `rtsql --readonly schema <db>`（`new`/`dump`/`restore`/`import`/`delete` 同型）
- **THEN** Usage 点名拒绝（exit 2），不静默忽略 flag——生命周期子命令维持读写打开（proposal 假设 10）
