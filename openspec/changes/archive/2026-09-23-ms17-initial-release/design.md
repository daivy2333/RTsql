# MS17 初版分发收口 — Design

> 决策记录随 change 归档，为长期技术选择的权威记录。D1-D9。

## D1: 加密总体架构与页布局（扩展步长 4124B）

**模型**：SQLCipher 同型——头明文（flag/盐/参数可见，非机密）、页密文、per-page 随机 IV + 认证 tag 随页存储。加密 transform 完全封装在 `FileStorage`（`src/storage/file_storage.rs`）的 `read_page_blocking`/`write_page_blocking` 两点，`AsyncStorage` trait、`BufferPool`、`Page`、SlottedPage、执行器零触碰：解密产物与加密输入均为标准 4096B 页镜像，上层全链路无感知。

**布局**（用户决策 4）：

```text
明文库（现状逐字节不变）：  页 N 偏移 = 64 + N × 4096，记录 = 4096B 页镜像
加密库：                    页 N 偏移 = 64 + N × 4124，记录 = 4124B
                            [ 12B nonce | 4096B 密文 | 16B GCM tag ]
AAD = page_id u64 LE（防整页搬移：页 A 密文移到页 B 位置认证失败）
nonce：每次 write_page 经 OS 随机源新生成（12B = GCM 标准 96-bit）
```

- 偏移公式由头 flag 区分；`PageId::to_offset` 纯数学不动（步长合成仍在 FileStorage 调用点，与既有 HEADER_SIZE 合成同型）。
- `allocate_page` 的 `set_len`、`page_count`、打开期页长度校验按模式步长计算；加密库非 4124 整除 → 既有 `PageSizeMismatch`（expected 携带本模式步长）。
- `free_page` 的零页写入走同一加密写路径（零页密文化，无明文旁路）。

**拒绝的替代方案**：

- (a) 步长不变、页内承载 nonce+tag（页载荷 -28B）——解密产物 4068B ≠ `Page::PAGE_SIZE`，SlottedPage/Page 需双格式变体，页格式层被侵入，与「明文库零回归」的隔离目标相悖；否决。
- (b) AES-GCM-SIV + 确定性 nonce（file_nonce, page_id）——tag 仍须随页存储，步长问题同在；且引入第二个 AEAD 依赖；随机 nonce 方案无 nonce 重用风险（每次写新生成），SIV 无必要；否决。
- (c) 文件级整体加密——页粒度随机读写需整文件重加密，性能模型不成立；否决。
- (d) 页级确定性 nonce（如 page_id 直接作 nonce）——同页重写即 GCM nonce 重用（灾难性）；否决（随机 nonce 是硬约束）。

**崩溃安全推演**：WAL 帧格式与明文语义零变化（D6）；页写撕裂 → 读取认证失败 → 与明文模式同由 WAL redo 兜底（redo 重放经加密写路径重写页）；checkpoint 九步次序不变（位点/截断不涉页格式）；干净 close 先刷页后截断 WAL，无「截断后撕裂页」窗口。加密不改变任何既有崩溃窗口语义。

## D2: KDF 与参数（Argon2id + 头内参数区）

- **KDF**：Argon2id（argon2 crate 0.6），输入 = 密码 UTF-8 字节（CLI `--key`/`RTSQL_KEY` 原值；lib API `key: &str`）+ 32B 盐 + 参数三元组，输出 32B = AES-256-GCM 密钥。
- **默认参数**（DA1）：m=19456 KiB、t=2、p=1（OWASP 2023 首选档）。**参数持久化于头保留区 12B**（`m_kib u32 LE + t u32 LE + p u32 LE`——恰合 64B 头布局的 52..64 保留区），打开时按头内参数派生：未来调默认参数不破坏既有加密库。
- **盐**：头 20..52 盐区，建库时 OS 随机源生成 32B。
- **头结构**（`FileHeader` 加性扩展）：`salt: [u8; 32]` + `kdf_params: [u8; 12]` 字段，明文头恒全零（encode/decode 逐字节不变）；加密头（flags bit0 置位）携带非零盐与合法参数。
- **decode 条件校验**：flags 含加密位时，盐区全零 → `HeaderError::ReservedNonZero("salt")` 既有分类；保留区参数违反值域（t≥1、p≥1、m_kib ∈ 1024..=2^24）→ `HeaderError::ReservedNonZero("reserved")`（复用既有分类与映射面）。明文头盐/保留区非零仍拒绝（既有语义逐字节）。
- **`KNOWN_FLAGS_MASK`**：0 → `FLAG_ENCRYPTED`。真正未知位仍拒绝（前向防护保持，`unknown_flag_bit_rejected` 语义不变）；既有 `encrypted_flag_rejected_by_current_build` 用例按语义演进而改写（校准面预授权，proposal 兼容段）。

## D3: 错误面与明密互斥（InvalidKey + exit 5 + 顺序守卫）

`StorageError::InvalidKey(String)` 新变体，Display `invalid key: {0}`，`{0}` 携带路径与原因。三形态（全在页解析与 WAL 触碰之前判定）：

| 形态 | 判定点 | 消息要点 |
|---|---|---|
| 加密库无密钥 | 头解码后（flag=1 ∧ key=None） | `database is encrypted, supply --key or RTSQL_KEY: {path}` |
| 明文库带密钥 | 头解码后（flag=0 ∧ key=Some） | `database is not encrypted: {path}`（用户决策 2） |
| 错误密钥 / 损坏密文 | 首次页解密认证失败 | `decryption failed (wrong key or corrupted page): {path}` |

- 打开顺序守卫扩展（衔接 format-header spec R3）：锁 → 头校验/初始化 → **密钥检查 + KDF** → 页长度校验 → 返回。锁仍优先于一切格式/密钥错误（`DatabaseLocked` exit 4 语义不变）；密钥错误不触碰 WAL / checkpoint / 任何页数据。
- CLI 映射：`open_error_status` 增 `StorageError::InvalidKey` 臂 → `ExitStatus::InvalidKey`（exit 5，既有枚举留位获得产生路径，`cli/mod.rs:22-26` 注释同步改写）。
- KDF 本身无验证器（不存密码 hash）——GCM tag 即验证器：错误密钥必在首页读取时暴露，无「成功打开坏库」窗口。
- 运行期（非打开期）页认证失败 → `InvalidKey` 沿既有存储错误链传播（DA5，非守卫主面）。

## D4: lib API 加性面（既有签名零变化）

```rust
// src/storage/file_storage.rs
pub fn open(path: &Path) -> Result<Self>                      // 保持，委托 open_with_key(path, None)
pub fn open_with_key(path: &Path, key: Option<&str>) -> Result<Self>  // 新核心

// src/database.rs
pub async fn open(path) / open_with_isolation(path, iso)      // 保持，委托 open_with_key
pub async fn open_with_key(path, isolation, key: Option<&str>) -> Result<Self>  // 新核心
```

- 既有全部调用点（`database.rs` 内部 + 30+ 处测试 `FileStorage::open`）零波及——`open` 变委托壳，明文路径逐字节等价（T02 先例：旧签名委托壳）。
- 密钥参数类型 `Option<&str>`：None = 明文库打开；Some = 密钥（空串由 CLI 层拒绝，lib 层 Some("") 为合法但注定失败的输入——Argon2 接受空密码，行为确定，不加 lib 层校验）。
- lib 面无内存模式（`:memory:` 无实现，grep 实证），无密钥 × 内存模式组合面。

## D5: CLI 密钥通道（单一咽喉点穿线）

- `CliArgs` 增 `#[arg(long, value_name = "KEY", global = true, env = "RTSQL_KEY")] key: Option<String>`——global 使其对主命令与全部子命令生效；clap 原生语义 `--key` 覆盖 `RTSQL_KEY`。
- 空密钥拒绝（DA2）：解析后 `key.as_deref() == Some("")` → `ExitStatus::Usage("key must not be empty")` exit 2，在任何开库之前。
- 穿线路径：`run` → `execute_command` 解析一次 key → 9 个生产开库点全部经 `execute_command_inner(db_path, key, work, sigint, sigterm)` → `Database::open_with_key(db_path, IsolationLevel::RepeatableRead, key)`。`list` 不开库、不消费 key（无害携带）。
- 机械适配面：`execute_command_inner` 签名增参 → lifecycle 8 个调用点（`lifecycle.rs:36/110/152/312/413/695/914/980`）+ 主命令臂（`cli/mod.rs:212`）+ `mod.rs` 结构测试直构点（`cli/mod.rs:542`，预授权断言集不变）。
- **拒绝的替代方案**：每子命令独立 `--key` 定义——重复声明且易漂移；全局环境读取散落在各函数——测试竞态面（I041 教训），clap env 集中解析无 env 竞态。

## D6: WAL / checkpoint / 恢复交互（零触碰推演）

- WAL 帧格式、`WalRecord` 结构、`WalWriter`/`WALBuffer`/`RecoveryManager` 签名与行为零变化——WAL 与 checkpoint 位点文件保持明文（用户决策 2 沿用 T02 预裁定；README 已知限制 + improvement 登记随收尾）。
- 恢复重放的页写入经 BufferPool → `FileStorage::write_page` → 加密写路径：重放产物与运行期落盘形态一致（两态一致性由同一 transform 保证，无需恢复侧感知加密）。
- 恢复读页（版本链重建、索引重建）经统一 `read_page` 解密路径。
- checkpoint 位点 24B 格式（T02 T7 产物）与加密正交；`checkpoint` 闭包水位捕获不受影响。
- 密钥生存期 = `FileStorage` 存续期（`PageCipher` 持派生密钥）；`Database::close`/drop 后随进程消亡，无落盘。

## D7: completions 子命令与 install.sh

**completions**：

- `Cargo.toml` 增 `clap_complete = "4"`（与 clap 4 同族；二进制依赖，非 dev——运行时生成）。
- `Command::Completions { shell: clap_complete::Shell }`（`Shell` 已实现 ValueEnum；按 tasks.md 收窄 bash/zsh/fish——`value_parser` 限三值或自定义枚举映射，未知 shell 由 clap 用法错误 exit 2 兜底）；子命令 `hide(true)`（不出现在 `--help`，`rtsql completions` 裸调用 → clap 报缺参数 exit 2）。
- 生成：`CliArgs::command()` → `clap_complete::generate(shell, &mut cmd, "rtsql", &mut stdout)`——补全脚本覆盖主命令、全局 flag（含 `--key`）与全部子命令。
- 生成路径不触库、无密钥语义；`--format` 对其无意义（忽略，既有 global flag 行为）。

**install.sh**（仓库根目录，`bash` 严格模式 `set -euo pipefail`）：

- 用法：`./install.sh [--prefix DIR] [--no-completions]` / `./install.sh --uninstall [--purge-data] [--prefix DIR]` / `--help`。
- 安装流程：cargo 存在性检查 → `cargo build --release` → `strip` 目标二进制（strip 不可用时跳过并提示，不失败）→ 安装到 `$PREFIX/bin/rtsql`（默认 `~/.local`，DA4）→ 逐 shell 检测安装 completions（`$SHELL` + 命令存在性探测；bash `~/.local/share/bash-completion/completions/rtsql`、zsh `~/.zsh/completions/_rtsql`〔stdout 提示 fpath 追加〕、fish `~/.config/fish/completions/rtsql.fish`）→ `$PREFIX/bin` 不在 PATH 时输出 export 提示。
- 卸载两模式（tasks.md 原文要求）：`--uninstall` = 删二进制 + 删三 shell completions 文件（只清程序，数据目录保留）；`--uninstall --purge-data` = 追加清除数据目录（`$RTSQL_HOME` 或默认 `~/.rtsql/`）——**先逐行列出将删除路径再删除**，显式 flag 即确认（无交互提示，保持脚本非交互可自动化；破坏性动作的显式性由 flag 承担，tasks.md 原文「须显式 flag 确认并列出将删除路径」）。
- `--prefix` 与安装/卸载对称（卸载也按 prefix 定位二进制）。

## D8: 文档面（README ×2 + docs/SKILL.md）

- **README.md**（英文，全量重写——现文件 2026-08-25 中文旧版，测试计数/能力清单过期）：定位段、快速开始（install.sh 安装与卸载、建库 CRUD、事务语句、加密用法〔`--key`/`RTSQL_KEY`、dump→restore 迁移〕）、能力清单（以 37 spec 语料库与 CLI 面为准）、架构概览一段、已知限制与非目标（WAL/checkpoint 明文、脚本逐语句 KDF 延迟、无就地转换、Unix-only 等）。
- **README.zh-CN.md**（中文同构）。
- **docs/SKILL.md**（英文，DA3；用户决策 5）：面向 AI agent 的操作手册——Install/Deploy（脚本、PREFIX、PATH）、Usage（one-shot 主命令、六生命周期子命令、三分析命令、四格式与退出码 0-5 表、事务、备份恢复、加密）、Management（集中存储目录布局、伴生文件 `.wal`/`.checkpoint`、文件锁、格式头、加密语义）、Uninstall；命令块全部可直接照做。
- **一致性验证方式**：文档任务（T10/T11）的测试见证 = Act 逐条执行文档中的命令块并与文档声称的输出/退出码比对（人工步骤最短化：每文档一组冒烟命令，一句话记录比对结论）；不建判定脚本（公共规则 › 验证）。

## D9: 验证策略

- 逐 scenario 最简直接判定（既有全量门 + 新增定向套件 + CLI e2e + 脚本冒烟），无身份型证据工程、无判定层、无 bench 设施。
- **脚本冒烟**（T8/T9）：临时 PREFIX 全自动命令化——`PREFIX=$(mktemp -d) ./install.sh` → `rtsql --version` + 建库 CRUD + completions 文件存在且 `bash -c 'source <file>'` 可装载 → `--uninstall` 后二进制消失、数据保留 → 重装 → `--purge-data` 后数据目录消失。全部为命令 + 退出码 + 文件存在性断言，Act Response 承载。
- **打开延迟实测**（T6）：加密库 vs 明文库 `Database::open` 计时对比（标准参数下预期数十毫秒级），数字记入 Act Response——不建 bench、不设阈值断言（仅记录，README 引用）。
- 全量门基线 1065（T02 收口 maintainer 记录）；本 change 新增测试计数在各 Iteration 收口任务中按 Act 实测记录。
- Persisted Evidence 全 Iteration `none`：全部验证命令可低成本重跑，Act Response 承载决定性输出（≤20 行/项）。
