## ADDED Requirements

### Requirement: 默认组合行为等价

启用 `default` features（`cli`、`network`、`crypto` 全开）的构建 SHALL 与引入 feature 表之前的行为完全一致：`rtsql` 二进制、库公共 API、测试全集、错误语义、退出码与磁盘格式均不变。既有全部规格域在默认组合下 SHALL 继续成立。项目质量门（`cargo fmt --check`、`cargo clippy --offline --all-targets -- -D warnings`）在默认组合下 SHALL 退出码 0。

#### Scenario: 默认组合全量回归

- **GIVEN** default features 构建的工作区
- **WHEN** 按 M19 分批串行执行全量测试
- **THEN** 每一批退出码 0 且 failed 为 0；批次覆盖对账 missing / extra / duplicate 均为 0

#### Scenario: 死依赖移除不可观察

- **GIVEN** 运行时依赖表不再包含 `lru`、`anyhow`，`futures` 仅存在于 dev-dependencies
- **WHEN** 在默认组合构建并运行既有测试
- **THEN** 行为与移除前一致（三者全仓零引用，无可达代码路径变化）

### Requirement: network 可选组合

`network` feature 关闭时，库 SHALL 构建通过且不包含 network 模块（`rtsql::network` 路径不存在），`tokio-util` SHALL NOT 进入依赖图。依赖 network 的测试目标 SHALL 经条件编译剥除，其余测试目标 SHALL 照常编译运行。`network` 开启时，PG/JSON 协议面 SHALL 与既有行为一致并由既有协议测试锚定。

#### Scenario: 关闭 network 的构建面

- **GIVEN** `--no-default-features --features cli,crypto`
- **WHEN** 构建库与测试目标并检查依赖图
- **THEN** 构建退出码 0；`rtsql::network` 不可引用；`tokio-util` 不在依赖图

#### Scenario: 关闭 network 的核心行为

- **GIVEN** 同上组合
- **WHEN** 运行组合内单元测试与既定核心行为测试目标（存储、解析、执行域目标）
- **THEN** 全部通过，结果与默认组合同名目标一致

#### Scenario: 默认组合协议面不变

- **GIVEN** default features 构建
- **WHEN** 运行既有协议测试目标（pg_protocol、pg_integration、pg_messages、network_server、network_protocol、connection_limit）
- **THEN** 全部通过，行为与既有结果一致

### Requirement: crypto 可选组合

`crypto` feature 关闭时，库 SHALL 构建通过，`argon2`、`aes-gcm` SHALL NOT 进入依赖图，明文库全部功能 SHALL 可用。打开格式头 `FLAG_ENCRYPTED` 位置位的库 SHALL 以既有 unknown feature flags 拒绝面拒绝（`IncompatibleHeader`，消息含 `unknown feature flags`）。携带密钥的打开（明文库既有路径或 0 字节新建路径）SHALL 在文件锁获取之后、任何格式头写入之前返回 `StorageError::InvalidKey`，错误消息 SHALL 点名本构建未包含加密能力；CLI 侧 SHALL 映射既有退出码 5。`crypto` 开启时，`database-encryption` 域全部既有合同 SHALL 不变。

#### Scenario: 加密头文件被拒绝

- **GIVEN** crypto 关闭组合，且一个 64 字节文件：magic、version=1、flags bit0 置位、page_size=4096
- **WHEN** 以任何密钥参数打开
- **THEN** 返回 `IncompatibleHeader`，消息含 `unknown feature flags`，与既有未知 flag 位拒绝面一致

#### Scenario: 携带密钥打开被拒绝

- **GIVEN** crypto 关闭组合，一个可用明文库与一个 0 字节路径
- **WHEN** 分别携带密钥打开（lib `open_with_key`/`open_readonly_with_key` 传 `Some(key)`，或 CLI `--key`/`RTSQL_KEY`）
- **THEN** 两者均返回 `InvalidKey`，消息点名本构建未包含加密能力；CLI 退出码 5；新建路径不写入格式头；无其他进程持锁时锁序守卫不可观察差异，有锁冲突时 `DatabaseLocked` 仍优先

#### Scenario: 明文库全功能

- **GIVEN** crypto 关闭组合
- **WHEN** 对明文库执行建表、写入、查询、重开往返
- **THEN** 行为与默认组合一致

### Requirement: cli 可选组合

`cli` feature 关闭时，`rtsql` 二进制目标 SHALL NOT 构建（`required-features`），`clap`、`clap_complete`、`csv` SHALL NOT 进入依赖图，库 SHALL 构建通过，bin 锚定测试目标 SHALL 经条件编译剥除。`cli` 开启时，CLI 全部子命令、退出码与输出 SHALL 与既有行为一致。

#### Scenario: 关闭 cli 的构建面

- **GIVEN** `--no-default-features --features network,crypto`
- **WHEN** 构建库与测试目标
- **THEN** `rtsql` bin 不产出；lib 与非 bin 锚定测试目标编译通过

#### Scenario: 默认组合 CLI 面不变

- **GIVEN** default features 构建的 `rtsql` 二进制
- **WHEN** 运行既有 CLI 测试目标（`cli_test`）
- **THEN** 全部通过，退出码与输出与既有结果一致

### Requirement: 组合共存与验证命令面

三个 feature 的任意组合（含全关）SHALL 使库与组合内测试目标集构建通过。非默认组合的行为验证 SHALL 按预先选定的目标集执行；全量回归 SHALL 只在默认组合执行并遵守 M19 分批串行约束。验证 SHALL 直接使用 cargo 既有命令面，不新增判定脚本或封装层。

#### Scenario: 全关组合构建与单元测试

- **GIVEN** `--no-default-features`
- **WHEN** 构建库与测试目标并运行组合内单元测试
- **THEN** 构建退出码 0，单元测试通过

#### Scenario: 组合矩阵收口

- **GIVEN** 五种所选组合（默认、三单关、全关）
- **WHEN** 逐一执行组合的构建门与既定验证集
- **THEN** 每种组合的构建门退出码 0，既定验证集全部通过；任一组合失败即收尾不通过
