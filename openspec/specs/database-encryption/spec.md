# database-encryption Specification

## Purpose
定义主数据库可选整库加密的磁盘格式、Argon2id 密钥派生与打开拒绝面、CLI 密钥通道及明文兼容边界。来源：MS17-T01 change `2026-09-23-ms17-initial-release`。

## Requirements

### Requirement: 加密库磁盘格式

数据库文件 SHALL 以格式头 flags bit0（`FLAG_ENCRYPTED`）区分明文库与加密库。加密库的页存储 SHALL 采用扩展步长记录：页 N 的文件偏移为 `64 + N × 4124`，每条记录为 `12B nonce | 4096B 密文 | 16B GCM tag`（AES-256-GCM，AAD = page_id u64 LE，nonce 每次写入经 OS 随机源新生成）。明文库偏移公式 `64 + N × 4096` 与记录形态 SHALL 保持逐字节不变。加密 transform SHALL 完全封装在 `FileStorage` 页读写路径：`AsyncStorage` trait、BufferPool、页格式层与执行器 SHALL NOT 感知加密。加密库文件长度扣除头后 SHALL 为 4124 整除，违反复用 `PageSizeMismatch`。`free_page` 的零页写入 SHALL 经同一加密写路径（无明文旁路）。

#### Scenario: 加密库页记录落盘形态

- **GIVEN** `--key pw` 创建的加密库（含 catalog 页与至少一页数据）
- **WHEN** 以 4124 步长裸读任一页记录
- **THEN** 记录长 4124B，内容不与任何明文页镜像匹配（nonce/tag 随机性），页数据以 `64 + N × 4124` 定位可解密读回

#### Scenario: 页加解密往返与 AAD 绑定

- **GIVEN** 任一 4096B 页镜像与派生密钥
- **WHEN** `encrypt_page(page_id, page)` 后以同 page_id `decrypt_page`
- **THEN** 得到逐字节相同页镜像；将密文记录放置到另一 page_id 位置解密 SHALL 认证失败（AAD 换页防护）

#### Scenario: 加密库页数与步长校验

- **GIVEN** 一个加密头 + 非 4124 整除数据区长度的文件
- **WHEN** 带正确密钥打开
- **THEN** 返回 `PageSizeMismatch`（expected 携带 4124）；4124 整除的加密库 `page_count()` 语义与明文库一致

### Requirement: 密钥派生与打开面

加密库 SHALL 以 Argon2id 派生 AES-256 密钥：输入为密码（CLI `--key`/`RTSQL_KEY` 值或 lib API `key`）+ 头 32B 随机盐 + 头保留区参数三元组（m_kib u32 LE + t u32 LE + p u32 LE，建库默认 m=19456/t=2/p=1）。打开加密库 SHALL 在头校验之后、页解析与 WAL 触碰之前完成密钥检查：加密库无密钥 SHALL 返回 `StorageError::InvalidKey`；明文库带密钥 SHALL 返回 `InvalidKey`（明密互斥）；错误密钥或密文损坏导致的 GCM 认证失败 SHALL 返回 `InvalidKey`。锁冲突 SHALL 仍优先于密钥错误（既有顺序守卫，`DatabaseLocked` exit 4 语义不变）。KDF 参数 SHALL 自头读取（参数持久化），未来默认参数变更 SHALL NOT 破坏既有加密库。

#### Scenario: 错误密钥显式拒绝

- **GIVEN** 密钥 `pw` 创建的加密库含已提交数据，已 `close`
- **WHEN** 以错误密钥（`wrong`）重开并触达任意页读取
- **THEN** 返回 `InvalidKey`（GCM 认证失败语义），CLI 退出码 5，stderr 消息含 `decryption failed`

#### Scenario: 明密互斥双向拒绝

- **GIVEN** 一个明文库与一个加密库
- **WHEN** 对明文库提供密钥打开；对加密库不提供密钥打开
- **THEN** 两者均返回 `InvalidKey`；加密库错误消息点名 `--key` / `RTSQL_KEY`；两者均在页解析与 WAL 触碰之前拒绝

#### Scenario: 锁冲突优先于密钥错误

- **GIVEN** 进程 A 持有某加密库文件锁
- **WHEN** 进程 B 带正确密钥（或不带密钥）打开
- **THEN** B 得到 `DatabaseLocked`（exit 4），而非密钥错误（exit 5）

#### Scenario: 损坏密文检测

- **GIVEN** 加密库某页记录密文或 tag 被篡改 1 字节
- **WHEN** 带正确密钥重开并读取该页
- **THEN** 认证失败，返回 `InvalidKey`（错误密钥与损坏共用认证失败面，消息含 wrong key or corrupted page 语义）

#### Scenario: 建库与重启往返

- **GIVEN** 0 字节路径 + 密钥（`new --key` 或 lib `open_with_key`）
- **WHEN** 创建加密库、写入数据、`close` 后以正确密钥重开
- **THEN** 全部数据完整可读；头 flags=bit0、盐非全零、参数为建库默认值

### Requirement: 密钥通道与 CLI 映射

CLI SHALL 提供全局 `--key <KEY>` 参数并读取环境变量 `RTSQL_KEY`（`--key` SHALL 覆盖 env），对主命令与 `new`/`schema`/`dump`/`restore`/`import`/`stats`/`sample`/`profile` 全部开库命令生效；`new` 与 `restore` 带密钥 SHALL 创建加密库。空密钥（`--key ""` 或 `RTSQL_KEY=""`）SHALL 以用法错误退出码 2 拒绝（开库前）。密钥错误 SHALL 映射退出码 5（`ExitStatus::InvalidKey` 既有枚举获得产生路径）。`list` 不开库、不受密钥影响。

#### Scenario: 全命令面密钥可用

- **GIVEN** `new --key pw` 创建的加密库含表与数据
- **WHEN** 分别以 `--key pw` 执行主命令 SELECT、`schema`、`dump`、`import`、`stats`、`sample`、`profile`
- **THEN** 全部命令正常完成，行为与明文库同名命令一致

#### Scenario: RTSQL_KEY 与 --key 等效及优先级

- **GIVEN** 加密库（密钥 `pw`）
- **WHEN** 仅设 `RTSQL_KEY=pw` 执行 SELECT；再同时设 `RTSQL_KEY=wrong` 与 `--key pw` 执行 SELECT
- **THEN** 前者成功（env 等效）；后者成功（`--key` 覆盖 env）

#### Scenario: 退出码 5 产生路径

- **GIVEN** CLI 退出码枚举（0/1/2/3/4/5）
- **WHEN** 以错误密钥打开加密库
- **THEN** 退出码 5，stderr 输出 `invalid key` 语义消息；用法错误（空密钥）仍为退出码 2

#### Scenario: dump→restore 静态加密迁移

- **GIVEN** 明文库含数据
- **WHEN** `dump` 明文库 → `restore --key pw` 到新路径
- **THEN** 目标库为加密库（头 flags=bit0），行集与源库一致；反向（加密源 → 明文 restore）同理可用

### Requirement: 明文库零回归与已知限制

除加密引入的变化外，既有打开/创建/读写语义 SHALL 保持逐字节不变：无密钥打开 = 既有路径（`FileStorage::open` 委托 `open_with_key(None)`）；明文库全链路（CRUD/恢复/checkpoint/锁/信号停机）零回归；WAL 帧格式与 checkpoint 位点格式零变化且保持明文。加密范围 SHALL 仅覆盖主库文件：`.wal` 与 `.checkpoint` 伴生文件保持明文（已知限制，README 记载）。打开延迟 SHALL 实测记录（Argon2id 标准参数），不设阈值断言、不建 bench 设施。

#### Scenario: 明文库路径零回归

- **GIVEN** 既有全量测试套件（基线 1065）与明文库全部行为面
- **WHEN** 全量测试运行
- **THEN** 除预授权校准面（`file_header_test` 加密位拒绝用例按语义演进而改写、`cli/mod.rs` 结构测试直构点签名机械适配）外零修改通过；`rtsql <db> <sql>` 无密钥路径行为与本 change 前逐字节一致

#### Scenario: 伴生文件保持明文（已知限制）

- **GIVEN** 加密库的完整 open/close 周期
- **WHEN** 检查同目录 `.wal` 与 `.checkpoint` 文件
- **THEN** 内容为既有明文格式（WAL 帧、24B 位点），可被既有 reader 解析——加密不覆盖伴生文件（README 已知限制）

#### Scenario: 打开延迟实测记录

- **GIVEN** 同规模明文库与加密库
- **WHEN** 各自 `Database::open` 计时（≥3 次取样）
- **THEN** 实测数字记入 Act Response（加密库含 Argon2id 派生开销，预期数十毫秒级）；无阈值断言
