# Iteration 000 / Cycle 000: encryption-core 初始执行

## Plan Context

- Status: ready
- Iteration: 000-encryption-core
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3, T4, T5, T6
- Depends on: None
- Stable baseline: 可选整库加密全链路可用（建/开/读写/重启两态一致，错误密钥·明密互斥·损坏检测三拒绝面 exit 5，明文库零回归，`--key`/`RTSQL_KEY` 全命令面生效）；全量门稳定（基线 1065 + 新增）
- Verification boundary: crypto/file_header/file_storage 单测 + `tests/encryption_test.rs` + cli_test 新用例全绿 + 既有套件零修改（预授权校准面除外）+ T6 全量收口与打开延迟实测记录
- Diagnostic boundary: `src/storage/{crypto,file_header,file_storage,error,mod}.rs`、`src/database.rs`、`src/cli/mod.rs`（+ `src/cli/lifecycle.rs` 传参机械面）、`tests/{file_header_test,encryption_test,cli_test}.rs`
- Deferred tasks: T7-T9（Iteration 001）、T10-T12（Iteration 002）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 全部承诺中本 Iteration 的 T1-T6（最小加密 + 密钥通道 + 收口）；design D1-D6、D9；用户决策 2（明密互斥显式拒绝）/ 3（全命令面密钥通道）/ 4（扩展步长 4124B）
- Excluded scope: completions/install.sh（T7/T8）；README/SKILL.md（T10/T11）；WAL/checkpoint 加密；bench 设施；密钥便利层（I054-I056）；ISS/Runbook 落账

**Objective**

数据库文件可选整库加密按 Task Contract 落地并有 RED→GREEN 测试见证：Argon2id KDF + 页级 AES-256-GCM（4124 步长布局）封装在 FileStorage，格式头加密位/盐/参数区激活，`InvalidKey` 三拒绝面 + 退出码 5 产生路径，`--key`/`RTSQL_KEY` 全命令面穿线；明文库行为逐字节零回归；打开延迟实测记录。

**Background**

MS17 初版分发收口第一棒（proposal Why 节）：T01 最小加密是初版唯一重活。格式头自 MS10-T03 起预留加密位（bit0）、32B 盐槽与 12B 保留区（`KNOWN_FLAGS_MASK=0` 拒绝加密位至 MS12）；MS17-T02 已消除断言门假失败源（I041）并收口 RC 水位（24B 位点）。本 Iteration 将预留面激活为可用能力，后续 Iteration 的安装面与文档均以本 Iteration 冻结的 CLI 面为记载基准。

**Investigation Facts**

- Current Baseline: master `7364bc9` + MS13 实施 + MS17-T02 实施 + 两批 docs sync（均未提交，工作区待用户统一 commit）；**1065 tests pass / 0 failed / 2 ignored**（MS17-T02 收口 maintainer 记录，SNAPSHOT 同步状态 current，本会话未修改覆盖范围内表面，采信）；clippy/fmt 0、validate 37 specs PASS。T02 change 已归档（`archive/2026-09-23-ms17-t02-defect-closeout/`）。
- Current-State Evidence（全部本会话新鲜读码）：
  - **格式头预留面**：`file_header.rs:23` `FLAG_ENCRYPTED: u32 = 1`；`:26` `KNOWN_FLAGS_MASK: u32 = 0`（加密位现被拒，`HeaderError::UnknownFlags`）；`:34-39` `FileHeader { version, flags, page_size }`（Copy，无盐/参数字段）；`:69-76` `encode`（盐/保留区恒写 0）；`:79-110` `decode`（`:99-104` 盐区非零 → `ReservedNonZero("salt")`、`:104` 保留区非零 → `ReservedNonZero("reserved")`）；`:59-66` `FileHeader::current()`。既有 14 单测中 `encrypted_flag_rejected_by_current_build`（`:167-175`）锁定「加密位拒绝」——本 change 语义演进的唯一校准点。
  - **FileStorage 页 I/O 面**：`file_storage.rs:43-95` `open`（锁 `:54-60` → 0 字节写头 `:70-72` → 头校验 `:76-78` → 页长度校验 `:79-86`，长度判定用 `page_size`）；`:101-110` `read_page_blocking`（`HEADER_SIZE + page_id.to_offset(page_size)` 读 `page_size` 字节 → `Page::from_bytes`）；`:112-121` `write_page_blocking`（同偏移整页写）；`:139-156` `allocate_page`（free-list / `set_len(HEADER + offset + page_size)`）；`:158-164` `free_page`（零页经 `write_page`）；`StorageError` 无 InvalidKey 变体（`error.rs` 全枚举已核）。加密接线点即两个 `*_blocking` 函数 + open 分支 + 步长常量替换。
  - **开库咽喉点**：`database.rs:42-121` `open_with_isolation`（`:44` `FileStorage::open(path)` 唯一存储构造点；`:79-94` MS17-T02 水位逻辑原样保留）；`:31-33` `open` 委托。CLI 全部 9 个生产开库点汇聚 `cli/mod.rs:269-282` `execute_command_inner`（`:276` `Database::open(db_path)`）——lifecycle 8 个调用点（`lifecycle.rs:36/110/152/312/413/695/914/980`）+ 主命令臂（`cli/mod.rs:212`）；`mod.rs:542` 结构测试直构点（签名适配预授权）。`mod.rs:27-35` `ExitStatus::InvalidKey` 已存在（`:57-59` exit 5 映射），注释 `:24` 自证「当前无产生路径（密钥 MS12 落地）」；`:250-261` `open_error_status` 仅映射 `DatabaseLocked`，其余落 General。`CliArgs`（`:72-82`）已有 global flag 先例 `format`（`:78` `global = true`）。`list` 不开库（`lifecycle.rs` List 臂纯目录枚举）。
  - **伴生文件**：WAL `<db>.wal`、checkpoint `<db>.checkpoint`（24B，T02 T7 产物）——两者不经 FileStorage 页路径，加密零触碰（design D6）。
  - **依赖可用性**（2026-09-23 `cargo search` 实测，网络可用）：`argon2 0.6.0`、`aes-gcm 0.11.1`、`clap_complete 4.6.11`（T7 用）；`rand 0.8` 已有。
  - **测试基建先例**：新模块 RED 编译演进先例（T02 T7 位点单测）；lib 级 e2e 参考 `tests/isolation_level_test.rs`（tempdir + open_with_isolation + close/reopen 配方——显式作用域 + `close()` 释锁，Iteration 001 Deviations 1a 实证）；cli e2e 参考 `tests/cli_test.rs` 既有 import/锁/退出码用例；env 用例串行先例（T02 T1 合并测试 + EnvGuard）。
- Code and Critical Path:
  - 变更面（产品代码 9 文件）：`Cargo.toml`（+argon2/aes-gcm）、新 `src/storage/crypto.rs`（KDF + PageCipher）、`src/storage/file_header.rs`（mask + 字段 + 条件校验 + 加密头构造）、`src/storage/file_storage.rs`（open_with_key + transform + 步长）、`src/storage/error.rs`（+InvalidKey）、`src/storage/mod.rs`（导出）、`src/database.rs`（open_with_key 核心 + 委托）、`src/cli/mod.rs`（--key + 穿线 + exit 5 映射）、`src/cli/lifecycle.rs`（8 调用点传参）。
  - 数据流：`--key/RTSQL_KEY` → clap 解析（空拒绝）→ `execute_command_inner(key)` → `Database::open_with_key` → `FileStorage::open_with_key` → 头解码 →（加密∧无钥 | 明文∧有钥）→ InvalidKey；加密 → KDF(密码, 盐, 参数) → `PageCipher` → 页读解密/页写加密（4124 步长）。无钥明文库：`open` 委托 `None` → 既有路径逐字节。
  - 错误边界：`CryptoError::AuthFailed` → `StorageError::InvalidKey`（file_storage 映射）→ CLI `ExitStatus::InvalidKey`（exit 5）。KDF 参数非法（头内）在 file_header decode 拒绝（IncompatibleHeader），KDF 调用时不再有非法参数。

**Implementation Guidance**

顺序 T1 → T2 → T3 → T4 → T5 → T6 严格分层（每层以上层为依赖）。T1 新模块先写单测观察编译演进 RED（T02 T7 先例）。T2 注意 `FileHeader` 增字段后所有构造点走 `current()`/新加密构造入口，禁止散落字面量构造。T3 是本 Iteration 最重契约：`open_with_key` 内先保持既有锁→头序，头解码后在返回前插入密钥分支（加密∧无钥/明文∧有钥 → InvalidKey；加密 → KDF 构造 `PageCipher` 存入 `Self`），步长以 `self.cipher.is_some()`（或等价模式字段）区分——`read/write_page_blocking` 关联函数需携带模式参数或改为方法（非实质选择留 Act，但 Preserve 锁定：无 cipher 时字节路径与现状逐字节一致）。T4 的 e2e 重开场景遵守「显式作用域 + close()」锁释放配方。T5 的 `--key` 用 `env = "RTSQL_KEY"` clap 原生声明（无手读 env，规避 I041 类竞态），env 等效用例以 EnvGuard 串行（T02 T1 先例），其余用例走显式 `--key`。每任务先 RED 后实现，验证输出 ≤20 行入 Act Response。

**Behavioral Change**

- 明文库（默认，无密钥）：全部行为逐字节不变（`open`/`open_with_isolation` 委托壳、页路径无 cipher、头 encode/decode 明文形态不变）。
- 加密库（新能力）：`new --key pw`/`open_with_key(path, key)` 创建 4124 步长加密库；带正确密钥全命令面可用，行为与明文库同名命令一致；错误密钥/无钥/损坏 → exit 5；明文带钥 → exit 5。
- 错误面：`StorageError::InvalidKey` 新变体；CLI exit 5 从「枚举留位」变为有产生路径；`file_header` 对加密位从拒绝变为接受（真未知位仍拒绝）。
- 格式头：加密头新增盐/参数语义（明文头两区恒零不变）；伴随 T02 遗留校准——format-header spec R4 位点 16B→24B 文字修正（DA7，spec 侧已随本 change delta 修订）。

**Task Contracts**

### T1: 加密内核模块（依赖 + crypto.rs）

- Requirement/Scenario: database-encryption R1（页加解密往返/AAD）+ R2（KDF 决定性/错误密钥基底）
- Depends on: None
- Targets: `Cargo.toml`（dependencies）、新 `src/storage/crypto.rs`、`src/storage/mod.rs`（导出）
- Current behavior: 无任何加密能力；argon2/aes-gcm 未引入
- Required behavior: `derive_key(password, salt, m_kib, t, p) -> [u8; 32]`（Argon2id，同输入决定性）；`PageCipher::encrypt_page(page_id, &[u8;4096]) -> [u8;4124]`（随机 12B nonce + AES-256-GCM + AAD = page_id u64 LE）；`decrypt_page(page_id, &[u8;4124]) -> Result<[u8;4096], CryptoError>`（`CryptoError::AuthFailed` 认证失败）；KDF 参数值域校验入口（t≥1、p≥1、m_kib ∈ 1024..=2^24，违规 → 参数错误）
- Required changes: 引入 `argon2 = "0.6"`、`aes-gcm = "0.11"`（默认 features）；crypto.rs 实现上述符号 + 模块 doc 注记布局与 AAD 语义；mod.rs 导出
- Preserve: 既有依赖集与版本零变化；`rand 0.8` 原样（nonce 经 aes-gcm 的 OsRng 或等价 OS 随机源）
- Forbidden: 引入 aes-gcm-siv/chacha 等额外密码学依赖；自实现 AEAD/KDF；把 nonce 改为确定性派生（design D1 拒绝方案 d）
- Test witness（RED 先行，编译演进）: crypto.rs 单测——(1) KDF 同参数同盐同密码两次派生相等、不同盐不等；(2) 页往返逐字节相等；(3) 错误密钥解密 `AuthFailed`；(4) 换 page_id 解密 `AuthFailed`（AAD）；(5) 密文单字节翻转 `AuthFailed`；(6) 同页两次加密记录的前 12B nonce 不等；(7) 参数值域拒绝（t=0 / m_kib=512）
- GREEN condition: 上述单测全绿
- Verification: `cargo test --lib storage::crypto`（决定性输出 ≤10 行，退出码 0；RED 记录入 Act Response）
- Stop when: aes-gcm/argon2 API 形态与假设不符（如 nonce 长度/Params 单位）导致契约签名无法保持——记录实际 API 形态返回 Plan（预期内 API 细节差异不阻塞，Act 按 crate 实际类型就近适配并记录）

### T2: 格式头扩展（加密位 + 盐/参数区）

- Requirement/Scenario: database-file-format-header 修改 R1（加密头盐/参数语义）+ R2（未知位收窄/盐参数违规拒绝）+ database-encryption R2 基底
- Depends on: T1（参数值域校验入口）
- Targets: `src/storage/file_header.rs`、`src/storage/mod.rs`（导出同步）
- Current behavior: 加密位被 `KNOWN_FLAGS_MASK=0` 拒绝；盐/保留区必须全零；`FileHeader` 无盐/参数字段
- Required behavior: `KNOWN_FLAGS_MASK = FLAG_ENCRYPTED`；`FileHeader` 加性增 `salt: [u8; 32]`、`kdf_params: [u8; 12]`（明文头恒零，`encode`/`decode` 明文形态逐字节不变）；加密头校验——盐全零 → `ReservedNonZero("salt")`、参数三元组值域违规（经 T1 校验入口）→ `ReservedNonZero("reserved")`；新构造入口 `current_encrypted(salt, m_kib, t, p)`（flags=bit0 + 盐 + 参数编码）；真未知位（bit1 等）仍 `UnknownFlags` 拒绝
- Required changes: 上述符号级变更 + doc 注记更新（`:22-26` 的「MS12 预留/拒绝」注释改写为本语义）
- Preserve: 既有 13 个非校准单测零修改通过（明文头 encode 布局、roundtrip、BadMagic、ZeroVersion、NewerVersion、UnknownFlags bit1、PageMismatch、明文盐/保留区非零拒绝）；`FORMAT_VERSION=1` 不变；`HEADER_SIZE=64` 布局不变
- Forbidden: 改头布局或大小；改明文头任何字节语义；引入版本 2；放宽未知位拒绝
- Test witness（RED 先行）: 加密头 encode/decode 往返（flags/盐/参数保真）；加密头盐全零拒绝；加密头参数违规拒绝（t=0、m_kib=512）；bit1 仍拒绝。RED 形态 = 新断言对现实现失败/编译演进
- 校准面（预授权）: `tests/file_header_test.rs::encrypted_flag_rejected_by_current_build` 改写为「加密头往返」断言（语义演进而非回归——spec delta R2 场景已锁定新语义）；其余 13 用例零修改
- GREEN condition: `cargo test --lib file_header` 全绿（13 既有零修改 + 1 改写 + 新增）
- Verification: `cargo test --lib file_header`（≤10 行，exit 0）
- Stop when: 既有明文头用例出现非预期失败（头结构改动溢出——返回 Plan）

### T3: FileStorage 加密接线 + InvalidKey 错误面

- Requirement/Scenario: database-encryption R1（落盘形态/步长/零页）+ R2（三拒绝面/顺序守卫）+ R4（明文零回归）
- Depends on: T1, T2
- Targets: `src/storage/file_storage.rs`、`src/storage/error.rs`
- Current behavior: `open(path)` 单一签名；页 I/O 明文 4096 步长；错误枚举无 InvalidKey
- Required behavior: `open_with_key(path, key: Option<&str>)` 新核心——锁 → 头校验/初始化（0 字节 + Some(key) → `current_encrypted(OS 随机盐, 19456, 2, 1)`；0 字节 + None → 既有 `current()`）→ **密钥检查**（加密∧None → `InvalidKey("database is encrypted, supply --key or RTSQL_KEY: {path}")`；明文∧Some → `InvalidKey("database is not encrypted: {path}")`；加密∧Some → KDF 构造 `PageCipher`）→ 页长度校验（加密库按 4124 步长整除，违反复用 `PageSizeMismatch{expected: 4124, actual}`）；`read_page_blocking`/`write_page_blocking` 按模式 transform（读 4124 → 解密 → `Page::from_bytes`；页 → 加密 → 4124 落盘）；`allocate_page` 的 `set_len` 与 `page_count` 按模式步长；`free_page` 零页经加密写路径。`StorageError::InvalidKey(String)` 变体（Display `invalid key: {0}`）。既有 `open(path)` 改委托 `open_with_key(path, None)`（签名不变）
- Required changes: 上述符号级变更 + 模块 doc 注记；`error.rs` 变体插入位置就近既有错误分组（非实质）
- Preserve: 无 cipher 时页 I/O 字节路径与现状逐字节一致（`file_storage_io_test`/`file_header_test`/`database_file_lock_test` 既有套件零修改锁定）；锁先于头校验先于密钥检查的顺序（`database_file_lock_test` 锁优先场景零修改）；`AsyncStorage` trait 签名零变化；`NotADatabase`/`NewerFileVersion`/`IncompatibleHeader` 分类与消息零变化；WAL/checkpoint 伴生文件零触碰
- Forbidden: 在 BufferPool/Page/页格式层做任何加密感知；改 `to_offset` 签名；加密头明文页混淆读写（明文库绝不经 cipher 路径，反之亦然——模式在 open 期一次定型）
- Test witness（RED 先行）: file_storage.rs 单测（镜像既有 src 侧 tempdir 基建）——(1) 加密库落盘：建库后裸读文件首页记录 ≠ 明文页镜像且步长 4124；(2) 错误密钥 open 后首读 `InvalidKey`；(3) 明文库 `open_with_key(Some)` → `InvalidKey`；(4) 加密库 `open_with_key(None)` → `InvalidKey`；(5) 密文单字节篡改后读 `InvalidKey`；(6) 加密库 allocate → page_count 步长正确；(7) `open(path)` 与 `open_with_key(path, None)` 行为等价（明文往返）
- GREEN condition: 新单测全绿 + `cargo test --lib file_storage --lib file_header --lib crypto` 全绿 + `cargo test --test file_storage_io_test --test file_header_test --test database_file_lock_test` 零修改全绿
- Verification: 上述命令（各 ≤10 行，exit 0；RED 记录入 Act Response）
- Stop when: 明文路径任何既有断言失败（委托壳不成立或模式泄漏——返回 Plan）；加密库与明文库判别在页 I/O 期出现歧义（模式必须 open 期定型）

### T4: Database lib API + lib 端到端

- Requirement/Scenario: database-encryption R2（建库与重启往返/错误密钥/锁优先）+ R4（明文零回归/伴生明文）
- Depends on: T3
- Targets: `src/database.rs`、新 `tests/encryption_test.rs`
- Current behavior: `Database::open/open_with_isolation` 直调 `FileStorage::open`；无加密 e2e
- Required behavior: `open_with_key(path, isolation, key: Option<&str>)` 新核心（`:44` 存储构造点换 `open_with_key`；MS17-T02 水位逻辑 `:79-94` 原样保留在其体内）；`open`/`open_with_isolation` 改委托壳（签名零变化）。e2e 套件见证：(1) 加密库 CRUD + `close` → 正确密钥重开数据完整（restart 往返，显式作用域 + close 释锁配方）；(2) 错误密钥重开 `InvalidKey`；(3) 明文带钥/加密无钥拒绝；(4) 篡改密文页（reopen 后读触发表）检测；(5) 锁冲突优先——持锁者占用加密库 + 带正确密钥第二开 → `DatabaseLocked`；(6) 加密库伴生 `.wal`/`.checkpoint` 保持既有格式可解析（明文已知限制见证）；(7) dump（明文库）→ `open_with_key` 空目标 + 逐语句执行 dump 文本 → 加密库行集一致（lib 级迁移等价）
- Required changes: database.rs 上述三点 + 新测试文件
- Preserve: `open`/`open_with_isolation` 对既有调用点行为逐字节（全部既有套件零修改锁定）；`checkpoint`/`close` 签名与语义零变化；隔离级别语义零变化
- Forbidden: 在 Database 层缓存或复用跨 open 的密钥；改 recovery/checkpoint 任何逻辑；为加密加新 pub API（超出 `open_with_key` 一个）
- Test witness: 契约 (1)-(7) 为新套件用例；T3 合入后其 lib 拒绝面 (2)(3)(5) 已部分闭合，本契约见证以「新 e2e 先行撰写观察 RED（API 未有→编译演进 / 行为断言先于实现补齐）→ 实现后 GREEN」形态记录（MS13 校准先例）；实际 RED/GREEN 顺序以 Act 实施为准，不得以跳过见证替代
- GREEN condition: `cargo test --test encryption_test` 全绿 + 既有全量相关套件零修改
- Verification: `cargo test --test encryption_test`（≤10 行，exit 0）
- Stop when: restart 往返数据不一致（加密破坏两态一致性——返回 Plan，不得放宽断言）

### T5: CLI 密钥通道（--key / RTSQL_KEY / exit 5）

- Requirement/Scenario: database-encryption R3 全场景 + cli-noninteractive-shell R1 修改（退出码 5 产生路径/空密钥 exit 2）
- Depends on: T4
- Targets: `src/cli/mod.rs`、`src/cli/lifecycle.rs`（8 调用点机械传参）
- Current behavior: 无 --key 参数；`execute_command_inner` 无 key 参数；`open_error_status` 无 InvalidKey 臂；exit 5 无产生路径
- Required behavior: `CliArgs` 增 `#[arg(long, value_name = "KEY", global = true, env = "RTSQL_KEY")] key: Option<String>`；`execute_command` 解析后空密钥检查（`Some("")` → `ExitStatus::Usage("key must not be empty")` exit 2，开库前）；`execute_command_inner` 增 `key: Option<&str>` 参（位置：`db_path` 之后，5 参总数）→ `Database::open_with_key(db_path, IsolationLevel::RepeatableRead, key)`；`open_error_status` 增 `StorageError::InvalidKey(detail) => ExitStatus::InvalidKey(detail)` 臂；`ExitStatus` doc 注释（`:24`「当前无产生路径」）改写。lifecycle 8 调用点 + 主命令臂传 `args.key.as_deref()`；`mod.rs:542` 结构测试直构点签名适配（补 `None` 实参，断言集不变——预授权）。cli_test 新用例组：(1) `new --key pw` 建加密库 + `--key pw` 主命令 CRUD/schema/dump/import/stats 全链路成功；(2) `RTSQL_KEY=pw` env 等效（EnvGuard 串行）；(3) `--key pw` 覆盖 `RTSQL_KEY=wrong`；(4) 错误密钥 exit 5 且 stderr 含 `invalid key`；(5) 加密库无钥 exit 5 且消息含 `--key`；(6) 明文库 `--key pw` exit 5；(7) 空密钥 exit 2；(8) `list` 无钥不受影响
- Required changes: 上述符号级变更
- Preserve: 退出码 0-4 全部分类与消息零变化；无 `--key`/env 时全命令面行为逐字节（既有 cli_test 81 用例零修改锁定）；`--format` global 语义零变化；事务语句/多语句/信号停机路径零变化
- Forbidden: 手工 `std::env::var("RTSQL_KEY")` 读取（必须 clap env 声明——I041 竞态教训）；为密钥新增专用退出码或改变 0-4 映射；lifecycle 各命令语义变化（仅传参）
- Test witness: 上述 8 用例（RED：对现状 `--key` 未实现时 exit 2/未知参数失败 = 编译/用法演进 RED）
- GREEN condition: 新用例全绿 + `cargo test --test cli_test` 既有 81 零修改全绿
- Verification: `cargo test --test cli_test`（≤10 行，exit 0）
- Stop when: `--key` 全局属性对某子命令不生效（clap global 传播失败——返回 Plan）；env 用例在全量并行下复现竞态（回到 EnvGuard 串行结构）

### T6: Iteration 000 收口验证 + 打开延迟实测

- Requirement/Scenario: database-encryption R4（零回归/延迟实测）+ RTM「全部域零回归」
- Depends on: T5
- Targets: change 级验证，无产品代码变更面
- Current behavior: 基线 1065（T02 收口 maintainer 记录，SNAPSHOT current，采信）；clippy/fmt 0、validate PASS
- Required behavior: T1-T5 合入后全量零回归（基线 1065 + T1-T5 新增，预期计数 Act 实测记录，仅预授权校准面 ±1——file_header_test 改写不改变计数）+ 四项静态/结构门 + 打开延迟实测（同规模明文 vs 加密库 `Database::open` 各 ≥3 次取样，数字记录，无阈值断言）+ 结构自检（tasks T1-T6 状态一致、delta spec 与实现一致、Iteration/Cycle 文件齐全、Review Result 与流程状态一致）
- Required changes: 无代码变更
- Preserve: 既有测试套件零修改（T2/T5 预授权校准与适配点除外）
- Forbidden: 为判定验证结果新增脚本/封装/判定器；为延迟数字建 bench 设施或阈值断言；重跑已通过验证增强信心
- Test witness: 各命令决定性输出与退出码 + 延迟数字记入 Act Response（全量输出 ≤20 行）
- GREEN condition: 全量 0 failed、clippy/fmt 0、validate PASS、自检各项一致、延迟数字已记录
- Verification: `cargo test --no-fail-fast`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate`、延迟计时（临时库 + 简单计时，判定 = 数字被记录）
- Stop when: 全量非预期失败（对照 T1-T5 Changed Files 定位归属，无法归属返回 Plan）

**Invariants**

- 明文库全链路行为逐字节不变（无密钥 = 既有路径委托）；`file_storage_io_test`/`file_header_test`/`database_file_lock_test`/`isolation_level_test`/全量套件零修改（预授权校准/适配点除外）。
- 加密 transform 完全封装 FileStorage；AsyncStorage trait、BufferPool、Page、SlottedPage、执行器、WAL、checkpoint 零触碰。
- WAL 帧格式、checkpoint 位点 24B 格式、主库头明文布局（64B、version=1、page_size=4096）零变化。
- 头校验顺序守卫：锁 → 头 → 密钥 → 页长度；锁冲突仍优先于密钥错误。
- 退出码 0-4 分类零变化；exit 5 仅由密钥错误面产生。
- 既有公共 API 签名零变化（新增仅 `FileStorage::open_with_key`、`Database::open_with_key`、`StorageError::InvalidKey`）。
- nonce 每次写入随机生成；AAD = page_id；无确定性 nonce 形态。

**Non-goals**

- T7-T12；WAL/checkpoint 加密；bench 设施与阈值断言；密钥便利层（I054-I056）；密钥内存 zeroize（DA6）；Windows；多用户权限；就地明密转换工具。

**Acceptance**

- T1：crypto 单测 7 组全绿（requirement→scenario→design D1/D2→T1→`crypto.rs`→单测链路）。
- T2：file_header 13 既有零修改 + 1 校准改写 + 加密头新增用例全绿。
- T3：file_storage 新单测 7 组全绿 + 既有存储套件零修改；三拒绝面单测可见。
- T4：`encryption_test` 7 场景全绿（往返/错误密钥/互斥/篡改/锁优先/伴生明文/迁移）。
- T5：cli_test 8 新用例全绿 + 既有 81 零修改；exit 5 三形态 + 空密钥 exit 2 可见。
- T6：全量零回归 + clippy/fmt/validate + 延迟数字记录 + 结构自检。
- Iteration 级：database-encryption R1-R4 与 format-header/cli 两 delta 修改的全部场景均有测试或既有套件见证。

**Verification**

- T1：`cargo test --lib storage::crypto` → 全绿，exit 0；RED（编译演进）记录入 Act Response。
- T2：`cargo test --lib file_header` → 全绿，exit 0。
- T3：`cargo test --lib file_storage` + 既有三存储套件 → 全绿，exit 0；RED 记录同上。
- T4：`cargo test --test encryption_test` → 全绿，exit 0。
- T5：`cargo test --test cli_test` → 全绿，exit 0；RED 记录同上。
- T6：全量 `--no-fail-fast`（≤20 行）+ clippy/fmt/validate + 延迟数字。每项判定以被测命令退出码与原生输出为准，无判定层。

**Gate 2 Readiness**

- 无 Missing requirement：RTM 11 行全部 Covered（PASS；`tasks.md` RTM）。
- 无 Simplified requirement（PASS；proposal Out of Scope 均为范围外已登记候选或用户预裁定，非需求裁剪）。
- 调查完整：格式头预留面、FileStorage 页 I/O 面、开库咽喉点、伴生文件、依赖可用性五点本会话新鲜读码/实测，行号复核命中（PASS；Investigation Facts）。
- 设计闭合：页布局（4124 步长 + 拒绝方案 a-d）、KDF 参数持久化、三拒绝面与顺序守卫、lib/CLI 加性接线、WAL 零触碰推演、验证策略（D1-D6、D9）全部闭合，无 TBD（PASS；design.md）。
- 任务可执行：六任务各有代码位置、行为变化、RED 见证、Preserve/Forbidden 与停止条件（PASS；Task Contracts）。
- 分轮合理：Iteration Plan 三轮依赖有序；000 六任务聚合审计记录（单一成果「加密可用」，lib/CLI 合并理由：咽喉点一次改造 + 避免 lib-only 中间态）（PASS；tasks.md Iteration Plan）。
- 追踪完整：requirement→scenario→design→task→代码→测试链路齐备（PASS；RTM）。
- 验证充分：覆盖全部已批准 scenario（happy：全链路/env 等效/迁移；sad：三拒绝/空密钥/篡改；edge：锁优先/步长校验/参数值域/AAD 换页/零页；兼容：明文零回归 + 预授权校准面点名），每条最简直接判定（PASS）。
- 无身份型证据工程/判定层（PASS；D9——延迟实测为记录性数字，非验收阈值）。
- 无需 Act 决定的实质未知项：crate API 细节差异已入 T1 Stop when（就近适配 + 记录）；非实质选择（错误插入位置、blocking 函数方法化形态、nonce RNG 来源）留 Act（PASS；Risks）。
- OpenSpec tasks/specs/design/当前 Iteration/Cycle 一致（PASS；同源撰写）。
- Persisted Evidence 明确：`none`（PASS；各验证输出 ≤20 行可入 Act Response，延迟数字为记录性输出，无不可低成本重跑项）。
- 计划批准：范围（单 change 合并 T01+T03+T04）经用户 2026-09-23 指令批准；四项场景决策（明密互斥/通道覆盖/页布局/SKILL 落点）经用户同日裁定（proposal 用户决策 2-5）；计划整体经用户 2026-09-23 审计批准（原话「批准」）——Status 已置 `ready`。

**Persisted Evidence**

- Mode: none

（Act Response 承载全部验证输出与延迟数字；无不可重跑项，无 Evidence 预算消耗。）

- Budget: 按公共规则 Evidence 预算执行（0 文件）。

**Risks and Notes**

- crate API 形态（argon2 `Params` 单位、aes-gcm nonce/tag 类型）以实际版本为准就近适配，契约签名语义（32B 密钥/4124 记录/AAD=page_id）不得变——非实质差异记录 Deviations。
- Argon2id 标准参数打开延迟预计数十毫秒级；若实测显著超出（秒级），记录数字并检查是否误用参数单位（m_kib vs MiB）——返回 Plan 前先核对 T1 单测的参数断言。
- e2e 重开场景遵守显式作用域 + `close()`（advisory 锁不随遮蔽 drop——T02 Iter001 Deviations 1a 实证）。
- env 用例必须 EnvGuard 串行；其余密钥用例一律显式 `--key`（避免引入新的 env 竞态面）。
- `PageCipher` 在 `FileStorage` 中的存放形态（`Option<PageCipher>` 字段）与 blocking 关联函数的方法化改写为非实质选择；Preserve 锚点 = 无 cipher 字节路径逐字节不变。
- 全量基线 1065 为 T02 收口采信结论；计数不符时先对照本 change 各任务 Changed Files 与 T02 归档记录复核归属，再判回归或基线漂移。

## Act Response

- Status: reported

**Implemented**

T1-T6 全部按 Task Contract 完成（RED→GREEN 逐任务见证）：

- T1 `Cargo.toml` + `argon2 0.6.0` / `aes-gcm 0.11.1`（默认 features）；新 `src/storage/crypto.rs`——`validate_kdf_params`（t≥1、p≥1、m_kib ∈ 1024..=2^24）+ `derive_key`（Argon2id，`Params::new(m_kib, t, p, Some(32))`，同输入决定性）+ `PageCipher`（`encrypt_page` 随机 12B nonce + AES-256-GCM + AAD=page_id u64 LE → `nonce||ct||tag` 4124B；`decrypt_page` 认证失败 `CryptoError::AuthFailed`）+ 默认参数常量（19456/2/1）。`mod.rs` 挂 `mod crypto`。
- T2 `file_header.rs`——`KNOWN_FLAGS_MASK = FLAG_ENCRYPTED`；`FileHeader` 加性 `salt: [u8;32]`/`kdf_params: [u8;12]`（`current()` 恒零；encode/decode 明文形态逐字节不变）；decode 加密分支（盐全零 → `ReservedNonZero("salt")`、参数值域违规经 T1 入口 → `ReservedNonZero("reserved")`）；`current_encrypted(salt, m_kib, t, p)` 构造入口 + `kdf_params()` 解码 accessor；真未知位（bit1/bit2）仍 `UnknownFlags` 拒绝；模块 doc 布局注释改写。
- T3 `file_storage.rs`——`open_with_key(path, key)` 新核心（`open` 委托 None 签名零变化）；守卫顺序锁 → 头校验 → 密钥检查/KDF → 页长度；三拒绝面（加密无钥/明文带钥 open 期 `InvalidKey`、错误密钥首读经 GCM 认证失败映射 `InvalidKey`）；`read/write_page_blocking` 按模式 transform；`allocate_page`/`page_count` 按模式步长（4124）；`error.rs` 增 `InvalidKey(String)`（Display `invalid key: {0}`，插 IncompatibleHeader 后）。
- T4 `database.rs`——`open_with_key(path, isolation, key)` 新核心（存储构造点换 `FileStorage::open_with_key`；MS17-T02 水位逻辑原样保留体内）；`open`/`open_with_isolation` 转委托壳签名零变化；新 `tests/encryption_test.rs` 7 场景全绿。
- T5 CLI——`CliArgs` 增 `--key`（`global = true, env = "RTSQL_KEY"`，clap 原生 env 声明）；`execute_command` 开库前空密钥 `Usage` exit 2；`execute_command_inner` 增 `key` 参（`db_path` 后第 2 位，共 5 参）→ `Database::open_with_key(db_path, RepeatableRead, key)`；`open_error_status` 增 `InvalidKey` 臂（消息前缀 `invalid key: `，见 Deviations 3）；`ExitStatus` doc 改写；lifecycle 8 函数签名 + 8 调用点 + 主命令臂机械传参；结构测试直构点补 `None`（断言集不变）。
- T6 收口：全量 `--no-fail-fast` **1099 passed / 0 failed / 2 ignored**（修复后重跑，见 Verification Evidence）+ clippy --all-targets -D warnings 0 + fmt --check 0 + `openspec validate --specs` 37 passed / 0 failed + 打开延迟实测记录（下表）+ change tasks T1-T6 状态同步。
- Plan Review 当前 Cycle 修复轮（R-F1/R-F2/R-F3，按 Follow-up Decision 修复契约执行）：
  - R-F1：`read_page_blocking` 解密失败消息改为 `decryption failed (wrong key or corrupted page): page {id}`；`key_wrong_key_exit_5` 追加 stderr 含 `decryption failed` 断言、`tampered_ciphertext_page_detected_on_read` 追加消息含 `wrong key or corrupted page` 断言——两断言先观察 RED（现状消息 `page N authentication failed`），修复后转 GREEN。`invalid key: ` 前缀机制（`open_error_status`）与退出码映射未动。
  - R-F2：file_storage src 单测新增 `encrypted_db_rejects_non_record_aligned_length`——加密头 + 数据区 4124+100 字节，正确密钥 `open_with_key` → `PageSizeMismatch { expected: 4124, .. }`（weak KDF 参数 1024/1/1 仅为测试速度）。步长守护行为先于测试存在（T3 契约），首跑即 GREEN——witness gap 闭合，无 RED 相位。
  - R-F3：`key_encrypted_full_chain` 追加 `--key pw sample app t` 与 `--key pw profile app t` 各 exit 0——R3「全命令面密钥可用」7 面补齐（SELECT/schema/dump/import/stats/restore/sample/profile）。

**Changed Files and Symbols**

- `Cargo.toml`（+argon2 0.6 / +aes-gcm 0.11 / clap features +"env"）、`Cargo.lock`
- 新 `src/storage/crypto.rs`：`validate_kdf_params` / `derive_key` / `PageCipher::{new, encrypt_page, decrypt_page}` / `CryptoError` / `ENCRYPTED_PAGE_RECORD_SIZE` / `DEFAULT_KDF_{M_KIB,T,P}`
- `src/storage/mod.rs`（`mod crypto`）
- `src/storage/file_header.rs`：`KNOWN_FLAGS_MASK`、`FileHeader{salt, kdf_params}`、`current_encrypted`、`kdf_params`、`encode`/`decode` 加密分支、单测 10→13
- `src/storage/file_storage.rs`：`open_with_key`、`FileStorage{record_size, cipher}`、`read/write_page_blocking` transform、`allocate_page` 分模式、单测 +7（初轮）；修复轮——解密失败消息改 `decryption failed (wrong key or corrupted page): page {id}` + 步长负例单测 +1（共 8）
- `src/storage/error.rs`：`StorageError::InvalidKey`
- `src/database.rs`：`open_with_key` 核心 + 两委托壳
- `src/cli/mod.rs`：`CliArgs.key`、空密钥检查、`execute_command_inner` 5 参、`open_error_status` InvalidKey 臂、lifecycle 分发传参
- `src/cli/lifecycle.rs`：8 函数 + 8 调用点 `key` 传参
- `tests/encryption_test.rs`（新 7；修复轮 tampered 用例追加 `wrong key or corrupted page` 断言）、`tests/cli_test.rs`（夹具 env_remove + `spawn_cli_env`/`run_cli_env` + 8 用例，81 既有零修改；修复轮 `key_wrong_key_exit_5` 追加 `decryption failed` 断言、`key_encrypted_full_chain` 追加 sample/profile 两面）、`tests/file_header_test.rs`（校准改写 1 用例 `test_encrypted_flag_rejected` → `test_encrypted_header_roundtrip`）

**Deviations from Plan**

1. **aes-gcm 0.11 API 形态**（Plan 预期内）：`Nonce`/`Key` 经 `aead 0.6`/hybrid-array 0.4——`aead::Nonce<Aes256Gcm>`（cipher 泛型）而非 aes-gcm 顶层 size 泛型别名；`from_slice` deprecated → `TryFrom` + expect。契约签名语义（32B 密钥/4124 记录/AAD=page_id/随机 nonce）零变化。
2. **集成侧校准改写顺延至 T3 步骤**：`test_encrypted_flag_rejected` 改写为 `test_encrypted_header_roundtrip` 需 `open_with_key` 才能表达往返断言（T2 时该 API 未落地）；src 单测层校准（加密头往返）按期在 T2 完成。Cycle 内顺序偏差，T3 GREEN 门覆盖。
3. **`open_error_status` InvalidKey 消息加 `invalid key: ` 前缀**（契约臂原文为 `ExitStatus::InvalidKey(detail)`）：detail 解构丢失 StorageError Display 的 `invalid key` 前缀，而契约测试见证要求 stderr 含 `invalid key`——按 Display 语义补前缀。
4. **clap 启用 `env` feature**（原有依赖加性 feature）：`env = "RTSQL_KEY"` 声明所需，契约"clap env 原生声明"的实现必要条件。
5. **`allocate_page` 加密分支改为直接写加密零页**（Act 补充实现细节，Plan 未预见）：明文语义"新分配页读到全零空页"依赖裸零字节可解析；密文下裸零无法通过 GCM 认证，catalog bootstrap（allocate → get_page 先读后写）在新建加密库时失败。加密分支 pwrite 加密零页承载该语义（读回全零）；明文分支 set_len 字节路径保持原样。实施中曾引入 offset 重复加 HEADER_SIZE 的回退（None 分支多 64B/页），由 T4 迁移用例经既有 `PageSizeMismatch` 守卫抓住后修复并重跑 T3 门套件全绿。
6. **篡改 e2e 锚定页 2**（首个用户数据页，catalog 0/1 之后——架构常量锚点）而非文件最后一字节：实测布局 0/1 catalog、2 数据页、3 PK 索引根，全表扫描不读索引根，末字节翻转不触发读取路径（探针实证后修正）。
7. **修复轮无计划偏差**：R-F1 消息格式串与 Follow-up Decision 修复契约逐字一致；R-F2 为既有守护行为的 witness 补登（首跑 GREEN，非行为变更）；R-F3 为用例追加。修复后 `cargo fmt` 重排新增行（纯空白），cli_test 已重跑确认（89 passed / 0 failed / 2 ignored）。

**Blocker Handoff**

None

**Blocker Resolution**

None（本 Cycle 无阻塞）

**Self-Review**

- Plan compliance: T1-T6 契约逐项实现；Preserve 面（明文逐字节、锁序、退出码 0-4、AsyncStorage 签名、伴生文件、水位逻辑）逐项核对未破坏；Forbidden（页格式层加密感知、额外密码学依赖、手读 env、新退出码、跨 open 密钥缓存）均未触犯。6 项 Deviations 均非实质（API 就近适配/顺序/消息措辞/依赖 feature/实现细节补全），无 Acceptance gap。修复轮按 Plan Review 修复契约执行，Preserve 提醒逐项遵守（明文路径与既有 81 cli 用例零修改、无新依赖、`invalid key: ` 前缀机制与退出码映射不动）。
- Full diff reviewed: 已完成（13 文件 + change tasks 状态）；跨任务交互核对——allocate 修复回灌 T3 门套件重跑全绿；cli_test 夹具 env_remove 对 81 既有用例为零影响面（去除从未依赖的变量）。修复轮 diff（3 文件：file_storage.rs 消息 + 步长负例单测、cli_test.rs 一断言收紧 + sample/profile 两面追加、encryption_test.rs 一断言收紧）已复审——消息改动仅触及 cipher 臂，明文臂字节路径不变；新增断言均观察真实 stderr/错误消息内容，测试不会因错误原因通过。
- Critical findings unresolved: 无
- Important findings unresolved: 无
- Minor findings unresolved: (1) `allocate_page` 两分支机制不同（pwrite 扩展 vs set_len），已在代码注释说明理由；(2) `key_env_equivalent` 在 RED 期因明文路径即通过（exit code 断言不改可观察面），GREEN 后经真实 env 通道承载，加密性由 (1) 的落盘头断言独立见证；(3) 延迟实测为 dev profile（未优化），数字仅记录不设阈值。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T1 RED | `cargo test --lib storage::crypto` | `8 failed; 0 passed`（`not yet implemented` panic） | crypto.rs 全部契约符号 | 预期 RED ✓ |
| T1 GREEN | 同上 | `test result: ok. 8 passed; 0 failed`，exit 0 | KDF 决定性/盐敏感、往返、错误钥/AAD/位翻转拒绝、nonce 唯一性、参数值域 | PASS |
| T2 RED | `cargo test --lib storage::file_header` | `error[E0599]: no function ... current_encrypted`（编译演进） | file_header 加密头新语义 | 预期 RED ✓ |
| T2 GREEN | 同上 | `test result: ok. 13 passed; 0 failed`，exit 0（10 既有〔8 零修改 + 1 改写〕+ 4 新增中的 3 计入） | 加密头往返/盐零拒绝/参数域拒绝/bit1 仍拒/明文形态不变 | PASS |
| T3 RED | `cargo test --lib storage::file_storage` | `error[E0599]: ... open_with_key not found` | FileStorage 加密接线 | 预期 RED ✓ |
| T3 GREEN | `cargo test --lib file_storage / file_header / crypto` + `--test file_storage_io_test / file_header_test / database_file_lock_test` | `7 / 13 / 8 passed` + `4 / 14 / 5 passed`，exit 0 | 落盘形态 4124/三拒绝面/篡改检测/步长/明文等价 + 既有存储套件零修改（校准 1 用例除外） | PASS |
| T4 RED | `cargo test --test encryption_test` | `error[E0599]: Database::open_with_key not found` | Database lib API | 预期 RED ✓ |
| T4 GREEN | 同上（allocate 修复与偏移修复后） | `test result: ok. 7 passed; 0 failed`，exit 0 | 加密 CRUD 重启往返/错误钥/明密互斥/篡改检测/锁优先/伴生明文/dump→restore 迁移 | PASS |
| T5 RED | `cargo test --test cli_test key_` | `6 failed; 2 passed`（`unexpected argument '--key'`） | CLI 密钥通道 | 预期 RED ✓ |
| T5 GREEN | `cargo test --test cli_test` | `test result: ok. 89 passed; 0 failed; 2 ignored`，exit 0 | `--key`/`RTSQL_KEY` 全命令面、exit 5 三形态、空密钥 exit 2、env 等效与覆盖、list 不受影响 + 81 既有零修改 | PASS |
| 修复 RED（R-F1） | `cargo test --test cli_test key_wrong_key_exit_5` + `cargo test --test encryption_test tampered_ciphertext_page_detected_on_read` | 均 FAILED：stderr/消息为 `invalid key: page 0/2 authentication failed`，新断言 `decryption failed` / `wrong key or corrupted page` 对现状不成立 | 两处收紧断言 vs 现状消息 | 预期 RED ✓ |
| 修复 R-F2 见证 | `cargo test --lib storage::file_storage` | `test result: ok. 8 passed; 0 failed`，exit 0（含新 `encrypted_db_rejects_non_record_aligned_length`；守护行为先于测试存在，首跑 GREEN） | 加密步长负例（加密头 + 4124+100 → `PageSizeMismatch { expected: 4124, .. }`） | PASS |
| 修复 GREEN | `cargo test --lib storage::file_storage` + `--test encryption_test` + `--test cli_test` | `8 / 7 / 89 passed; 0 failed`（cli 含 2 ignored），exit 0（cli_test 经 fmt 后复跑同结论） | 新消息三套件 + sample/profile 两面 + 全部既有用例零修改 | PASS |
| T6 全量（修复后重跑） | `cargo test --no-fail-fast` | 78 套件全 ok，合计 **1099 passed / 0 failed / 2 ignored**（= 1065 基线 + 34：crypto 8 / file_header 单测 +3 / file_storage 8〔含步长负例 1〕 / encryption 7 / cli 8），exit 0 | 全部既有套件零回归（预授权校准/适配面除外） | PASS |
| T6 clippy | `cargo clippy --all-targets -- -D warnings` | `Finished` 0 warning，exit 0 | 全 targets 静态门 | PASS |
| T6 fmt | `cargo fmt --check` | 无 diff，exit 0 | 格式门 | PASS |
| T6 validate | `openspec validate --specs` | `Totals: 37 passed, 0 failed`，exit 0（Purpose 占位 warning 为既有 I045 域） | delta specs 结构 | PASS |
| T6 延迟实测 | 临时同规模库（50 行）`Database::open` 计时，各 3 次（一次性探针，已删除） | 明文 419.9µs / 251.1µs / 214.9µs；加密 393.1ms / 358.1ms / 352.2ms（Argon2id 主导；dev profile 未优化） | 打开延迟记录性数字，无阈值断言 | 已记录 ✓ |

**Persisted Evidence**

None required（Mode: none——全部验证输出 ≤20 行可入本 Response，延迟数字为记录性输出，无不可低成本重跑项；`evidence/` 目录未创建）

- Budget: 0 文件

**Experience Candidates**

None（allocate 零页认证语义等实现洞察已沉淀于代码注释与 delta spec R1，属 change 范围内产物；无范围外实质缺陷或可复用高危操作路径）

**Remaining Issues**

无 Critical/Important。Minor：Self-Review 所列 3 项（均已在 Deviations/注释中说明，不阻塞）。

**Commit or Diff Reference**

未 commit（工作区待用户统一提交；本 Cycle 变更面见 Changed Files，对照基线 = 会话起点工作区状态 7364bc9 + MS13/MS17-T02 未提交实施）

## Plan Review

- Review Result: accepted

**Findings**

独立检查范围与方式（修复复核轮）：本 Review 全程只读，独立阅读修复轮触及的三个文件当前形态——`src/storage/file_storage.rs`（解密失败消息 + 新增步长负例单测）、`tests/cli_test.rs`（两处断言收紧 + sample/profile 两面追加）、`tests/encryption_test.rs`（篡改用例断言收紧），并复核 Preserve 面（`invalid key: ` 前缀机制、退出码映射、明文臂字节路径、依赖集）与 change tasks T1-T6 状态行。首轮 Review 已独立读码确认的未触及表面不重复检查；首轮与修复轮之间无其他代码变化——`decryption failed` 与 `wrong key or corrupted page` 在 src/ 与 tests/ 的全仓 grep 仅命中契约指定三处（`file_storage.rs:199` 实现、`cli_test.rs:3119` 断言、`encryption_test.rs:166` 断言），无计划外扩散。

三项修复逐项核验（与 Follow-up Decision 修复契约逐字比对）：

- **R-F1 闭合**：`file_storage.rs:197-202` cipher 臂解密失败消息为 `decryption failed (wrong key or corrupted page): page {id}`，与修复契约格式串逐字一致；`cli_test.rs:3105-3123` `key_wrong_key_exit_5` 在保留既有 `invalid key` 断言（`:3117`）外新增 `decryption failed` 断言（`:3118-3122`）——delta spec R2-S1 THEN 的字面子串满足；`encryption_test.rs:162-169` `tampered_ciphertext_page_detected_on_read` 新增 `wrong key or corrupted page` 断言——R2-S4 语义满足。Act 自报修复 RED（现状消息 `invalid key: page N authentication failed` 对新断言不成立）与消息改形前事实一致。
- **R-F2 闭合**：`file_storage.rs:446-467` 新增 `encrypted_db_rejects_non_record_aligned_length`——加密头 + 4124+100 数据区、正确密钥 `open_with_key` → `PageSizeMismatch { expected: ENCRYPTED_PAGE_RECORD_SIZE, .. }`（`matches!` 携带 expected 值断言）；weak KDF 参数（1024/1/1）仅为测试速度，不触语义。delta spec R1-S3 负面子句获得见证。
- **R-F3 闭合**：`cli_test.rs:3036-3045` `key_encrypted_full_chain` 追加 `--key pw sample app t` 与 `--key pw profile app t` 各断言 exit 0；R3-S1 WHEN 点名的 7 面（SELECT/schema/dump/import/stats/sample/profile）全部被行使。

Preserve 复核：修复轮仅触及 cipher 臂消息格式串与测试文件；`read_page_blocking` None 臂（明文路径 `:187-191`）与 write 路径零变化；`open_error_status` InvalidKey 臂 `invalid key: ` 前缀机制未动（`cli/mod.rs:270`）；退出码 0-4 分类与映射零变化；依赖集零新增（Cargo.toml 仍仅 argon2 0.6 / aes-gcm 0.11 / clap env feature，无其他密码学依赖）；修复后 `cargo fmt` 重排为纯空白（Act 自报并复跑 cli_test 确认同结论）。

验证采信：修复后全量 `cargo test --no-fail-fast` **1099 passed / 0 failed / 2 ignored**（= 1098 + R-F2 新增单测 1）、clippy --all-targets -D warnings 0、fmt --check 0、`openspec validate --specs` 37 PASS——来源本 Cycle Act Response Verification Evidence「修复后重跑」行，覆盖表面（file_storage.rs 与三测试文件）经只读核对自该次运行以来未变化，本 Review 会话未修改任何文件，无重跑触发情形（公共规则 › 验证），予以采信，不重跑。

非阻塞 Minor（记录不修复）：

- M1（沿首轮）：R3「dump→restore 静态加密迁移」的 CLI 明文源形态由 T4 lib 等价用例 + CLI 加密→加密 restore 组合见证，语义链完整，不要求追加 CLI 形态用例。
- M2（沿首轮）：打开延迟数字为 dev profile（Act 已自报）；README 引用时注明 profile（T10 契约已含）。
- M3（沿首轮，maintainer 收尾知悉）：change tasks.md 状态行格式不被 `openspec list` 识别（显示 No tasks）；权威状态以 tasks.md 为准。
- M4（本轮新观察）：design D3 第三形态消息要点为 `... : {path}`，实现为 `... : page {id}`——修复契约自行钉定 page 形态，delta spec 场景（验收权威）只要求 `decryption failed` 子串与 wrong key or corrupted page 语义，均满足；design 为随 change 归档的历史决策记录，不需回改。

**Deviation Classification**

None（修复轮零偏差——R-F1 格式串与契约逐字一致、R-F2 为既有守护行为的 witness 补登、R-F3 为用例追加。首轮分类 F1=ACT-DEVIATION、F2/F3=PLAN-OMISSION 已随修复全部闭合）

**Acceptance Gaps**

None（首轮 4 项 gap——R2-S1 `decryption failed` 字面子串 / R2-S4 消息语义 / R1-S3 步长负面见证 / R3-S1 sample·profile 两面——全部闭合。其余 Acceptance（T1-T6 逐条 GREEN condition、RTM 11 行、Invariants、format-header/cli 两 delta 修改场景）首轮已经独立读码确认满足，相关表面自首轮以来未变化）

**Convergence**

reduced（上一版当前 Cycle Review 4 项 gap → 0）

**Evidence**

- 代码证据：`src/storage/file_storage.rs:197-202`（R-F1 消息）、`:443-467`（R-F2 负例单测）、`:187-191`（明文臂零变化）；`tests/cli_test.rs:3036-3045`（R-F3 sample/profile）、`:3105-3123`（wrong key 断言收紧）；`tests/encryption_test.rs:151-172`（篡改断言收紧，锚定页 2 tag 区末字节）；`src/cli/mod.rs:270`（`invalid key: ` 前缀机制未动）；`Cargo.toml:11/24-25`（依赖面零新增）
- 全仓 grep 证据：`decryption failed`/`wrong key or corrupted page` 在 src/ + tests/ 仅契约指定三处（本轮独立执行）
- 验证采信：修复后全量 1099/0/2、clippy 0、fmt 0、validate 37 PASS——来源本 Cycle Act Response（修复后重跑行），覆盖表面只读核对未变化

**Follow-up Decision**

接受并完成当前 Iteration：三项修复均按 Follow-up Decision 修复契约执行且经本轮独立核验逐字达成；全部 Acceptance（T1-T6 GREEN 条件、RTM 11 行、Invariants、database-encryption/format-header/cli 三个 delta 的全部场景）满足，无阻塞 finding、无新增 gap。Iteration 000（encryption-core）完成，按既有 Iteration Map 展开 Iteration 001（install-surface）。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

`iterations/001-install-surface/000-initial.md`（已展开，Plan Context ready）
