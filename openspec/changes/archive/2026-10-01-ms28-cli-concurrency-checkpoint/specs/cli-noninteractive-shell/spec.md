# cli-noninteractive-shell Delta — busy-timeout 通道与 checkpoint 子命令

## ADDED Requirements

### Requirement: 锁等待 busy-timeout 通道

CLI SHALL 提供全局 `--busy-timeout <MS>` flag（对齐 `--key` 的 global 先例，作用于全部开库命令）与 env 通道 `RTSQL_BUSY_TIMEOUT`（显式 flag 优先），以毫秒为单位配置 `database-file-lock` 域定义的锁等待预算。值 SHALL 为非负十进制整数：非数字、负数或空值 SHALL 在任何开库动作前以 exit 2 拒绝（对齐 `RTSQL_WAL_CHECKPOINT_BYTES` 校验先例）。未配置时使用默认预算 5000ms。显式配置 0 表示不额外等待（仅保留 `database-file-lock` 域的内部可见性窗口下限）。`:memory:` 主命令组合 SHALL 接受该 flag/env 并忽略（内存库无文件锁；env 全局导出 SHALL NOT 破坏 `:memory:` 使用）。`delete` 子命令的锁探测路径 SHALL NOT 受等待预算影响（立即探测、锁占用即拒绝，行为不变）。

#### Scenario: flag 等待后成功

- **GIVEN** 另一进程持有一个已建库的写锁并将在 1 秒内释放
- **WHEN** `rtsql --busy-timeout 5000 x.db "INSERT ..."`
- **THEN** 第二方等待至锁释放，语句执行成功 exit 0

#### Scenario: env 生效且 flag 优先

- **GIVEN** `RTSQL_BUSY_TIMEOUT=100`，另一进程持有锁并将在 1 秒内释放
- **WHEN** `rtsql x.db "SELECT 1"`（仅 env）
- **THEN** 约 100ms 后 exit 4（`DatabaseLocked`）；同场景加 `--busy-timeout 5000` 则等待至释放后成功（flag 优先）

#### Scenario: 非法值 open 前拒绝

- **GIVEN** 任意库状态（含不存在的库）
- **WHEN** `rtsql --busy-timeout abc x.db "SELECT 1"` / `--busy-timeout -1 ...` / `RTSQL_BUSY_TIMEOUT=`（空）
- **THEN** exit 2，错误点名非法值与通道；未创建文件、未执行语句

#### Scenario: 内存库组合接受并忽略

- **GIVEN** 无前置条件
- **WHEN** `RTSQL_BUSY_TIMEOUT=3000 rtsql :memory: "SELECT 1"` 及 `rtsql --busy-timeout 3000 :memory: "SELECT 1"`
- **THEN** 均 exit 0 正常执行（忽略等待配置，无拒绝）

#### Scenario: 超时错误面不变

- **GIVEN** 另一进程长期持有 `x.db` 写锁
- **WHEN** `rtsql --busy-timeout 200 x.db "SELECT 1"`
- **THEN** 约 200ms 后 exit 4，错误信息维持既有 `DatabaseLocked` 文案（含库名）

### Requirement: 生命周期子命令 checkpoint

`rtsql checkpoint <db>` SHALL：经 `resolve_existing_db` 解析目标库（缺失以既有 General 语义 exit 1 拒绝）→ 以读写模式独占打开（锁冲突经 `--busy-timeout`/env 等待预算后 `DatabaseLocked` exit 4）→ 执行 `Database::checkpoint()` → 向 stdout 输出一行确认并 exit 0；WAL 无未决工作（无追加且无脏页）时 SHALL 输出明示零工作早退的确认并 exit 0（`Database::checkpoint()` 零工作跳过语义不变）。`--readonly` 组合与 `:memory:` 目标 SHALL 以 exit 2 拒绝（对齐既有子命令拒绝先例）。加密库 SHALL 经既有全局 `--key`/`RTSQL_KEY` 打开，密钥错误沿用 exit 5。checkpoint 的执行 SHALL 与 close/水位自动 checkpoint 的全量九步路径同源，落盘后外部备份可见完整已提交数据。

#### Scenario: 写入后显式 checkpoint

- **GIVEN** `x.db` 存在且有已提交写入（WAL 含未 checkpoint 记录）
- **WHEN** `rtsql checkpoint x.db`
- **THEN** exit 0，stdout 输出确认行；随后 `rtsql dump x.db` 可见全部已提交数据，WAL 文件被截断/缩减

#### Scenario: 零工作早退明示

- **GIVEN** `x.db` 干净（无 WAL 未决记录且无脏页）
- **WHEN** `rtsql checkpoint x.db`
- **THEN** exit 0，stdout 确认行明示零工作（无 WAL 截断等副作用）

#### Scenario: 缺失库拒绝

- **GIVEN** `RTSQL_HOME` 下不存在 `nope.db`
- **WHEN** `rtsql checkpoint nope`
- **THEN** exit 1，错误含库名（`resolve_existing_db` 既有文案），未创建文件

#### Scenario: 锁冲突经等待后拒绝

- **GIVEN** 另一进程长期持有 `x.db` 写锁
- **WHEN** `rtsql --busy-timeout 300 checkpoint x.db`
- **THEN** 约 300ms 等待耗尽后 exit 4（`DatabaseLocked` 错误面），未产生 checkpoint 副作用

#### Scenario: 只读与内存库组合拒绝

- **GIVEN** `x.db` 存在
- **WHEN** `rtsql --readonly checkpoint x.db`；`rtsql checkpoint :memory:`
- **THEN** 均 exit 2，错误点名组合不支持

#### Scenario: 加密库 checkpoint

- **GIVEN** 以 `--key secret` 创建的加密库 `enc.db` 有未决 WAL
- **WHEN** `rtsql --key secret checkpoint enc.db`；随后 `rtsql --key wrong checkpoint enc.db`
- **THEN** 正确密钥 exit 0 完成落盘；错误密钥 exit 5（既有解密失败错误面）
