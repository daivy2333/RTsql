# MS17 初版分发收口：最小加密 + 可安装面 + 双语文档（T01 + T03 + T04）

## Why

tasks.md MS17（★初版达成★，2026-09-23 用户批准规划）定义初版交付形式 =「本机一条命令可编译可安装 + 最基本的加密 + 有一份能读的文档」。T02 缺陷清账已由 change `2026-09-23-ms17-t02-defect-closeout` 完成（3 Iteration 全部 accepted，2026-09-23 归档，specs 37 域 / 1065 tests）；本 change 收口剩余三项工作，同属「初版可交付状态」单一验收主题：

1. **MS17-T01 最小加密**——数据库文件可选整库加密（SQLCipher 模型）：格式头加密 flag 激活（`KNOWN_FLAGS_MASK` 扩展 + 32B 盐槽 + 12B 保留区启用为 Argon2id 参数区）+ Argon2id KDF（密码 + 头随机盐）+ 页级 AES-256-GCM transform（FileStorage 读写路径，明/密库 flag 区分）+ 错误密钥显式拒绝（GCM 认证失败面，`StorageError::InvalidKey`，退出码 5 既有留位获得产生路径）+ 明文库行为零回归 + 打开延迟顺带实测记录（Act Response，不建 bench 设施）。密钥通道 `--key` / `RTSQL_KEY` 两条（MS12 裁剪裁定保留面）。
2. **MS17-T03 安装面**——`rtsql completions <bash|zsh|fish>` 隐藏子命令（clap_complete 运行时生成）+ 根目录 `install.sh` 一键编译安装脚本（`cargo build --release` + strip + 安装到用户目录〔默认 `~/.local`，`--prefix` 覆盖〕+ 按检测 shell 安装 completions + `--no-completions` 开关 + PATH 提示 + `--uninstall` 卸载两模式〔只清理程序 / `--purge-data` 带数据清理须显式 flag 并列出将删除路径〕）。
3. **MS17-T04 双语文档**——README.md（英文全量重写，现文件为 2026-08-25 中文旧版、测试计数 481 已过期）+ README.zh-CN.md（中文）+ `docs/SKILL.md`（agent 用说明书：安装部署 / 使用 / 管理 / 卸载，供 AI agent 加载后直接照做）。

三块独立故障域（加密格式 / 安装脚本 / 文档）合并依据与 MS15/MS16/T02 批处理先例相同：同一验收主题「初版可交付」，且用户 2026-09-23 明确指令「把 MS17 的剩余工作规划到一个 change 里面」（取代 tasks.md Workload 的 2-4 change 预估）。

**T02 收尾遗留校准（顺带）**：MS17-T02 Iteration 002 T7 已将 checkpoint 位点扩为 24B（lsn + timestamp + tx watermark），但主 spec `database-file-format-header` R4 场景仍写「`.checkpoint` 于 checkpoint 时写入 16B 位点」——T02 delta 未触及该域，形成 spec-实现漂移。本 change 本就修改该域（加密位/盐区/参数区），顺带校准该场景文字，消除语料库失真。

## 用户决策（2026-09-23 Gate 1 前裁定）

1. **单 change 合并**：用户指令「开始规划下一个 change，把 MS17 的剩余工作规划到一个 change 里面」——T01 + T03 + T04 单 change 收口（本 proposal）；拆分留 Iteration 层。
2. **明密互斥方向**：对明文库提供密钥（`--key`/`RTSQL_KEY`）→ 显式拒绝 `StorageError::InvalidKey`，CLI 退出码 5（`database is not encrypted`）。防呆：脚本误带密钥不静默；与「加密库无密钥」错误面对称。
3. **密钥通道覆盖面**：全命令面统一——全局 `--key` flag（clap `env = "RTSQL_KEY"`，`--key` 覆盖 env 为 clap 原生语义）作用于主命令 + `new`/`schema`/`dump`/`restore`/`import`/`stats`/`sample`/`profile` 全部开库命令；`new`/`restore` 带密钥即创建加密库。实现近零额外成本（全部 9 个生产开库点汇聚于 `execute_command_inner` 单一咽喉点）；`dump`（明文或加密）→ `restore --key` 即静态加密迁移路径。
4. **加密页布局**：扩展步长 4124B——每页磁盘记录 = 12B nonce + 4096B 密文 + 16B GCM tag，加密库页 N 偏移 = `64 + N × 4124`（明文库 `64 + N × 4096` 不变）。SlottedPage / BufferPool / 页格式层零触碰，transform 完全封装在 FileStorage；nonce 每次写入随机生成，AAD 绑定 page_id 防整页搬移。SQLCipher 同型模型。
5. **SKILL.md 交付形态**：仓库 `docs/SKILL.md`（英文，代码块为主），README 双语互链；install.sh 不安装（v0.1 最小面）。

## What Changes

1. **加密内核模块（T1）**——`Cargo.toml` 新增依赖 `argon2`（0.6，KDF）、`aes-gcm`（0.11，页加密；默认 features）；新模块 `src/storage/crypto.rs`：`derive_key`（Argon2id，密码字节 + 32B 盐 + 参数三元组 → 32B AES-256 密钥）与 `PageCipher`（`encrypt_page`/`decrypt_page`，随机 nonce、AAD = page_id u64 LE、GCM 认证失败显式错误）。单测先行（RED 编译演进先例）。
2. **格式头扩展（T2）**——`src/storage/file_header.rs`：`KNOWN_FLAGS_MASK` 0 → `FLAG_ENCRYPTED`；`FileHeader` 加性增 `salt: [u8; 32]` 与 `kdf_params: [u8; 12]` 字段（明文头恒零、encode/decode 逐字节不变）；加密头条件校验（盐非全零、参数值域 t≥1 / p≥1 / m_kib ∈ 1024..=2^24，违规 → `IncompatibleHeader`）；加密头构造入口。既有明文头 14 单测零修改（`encrypted_flag_rejected_by_current_build` 一个用例按语义演进而改写——校准面预授权）。
3. **FileStorage 加密接线（T3）**——`open_with_key(path, key: Option<&str>)` 加性 API（既有 `open` 委托 None，既有调用点零波及）；0 字节文件 + 密钥 → 写加密头（随机盐 + 默认参数）；加密库页 I/O 经 `PageCipher` transform（读 4124 → 解密验证 → 4096B 页；写 → 随机 nonce 加密 → 4124B 落盘）；`allocate_page`/`page_count`/`PageSizeMismatch` 按模式步长（4124）；打开顺序守卫扩展：锁 → 头校验 → 密钥检查/KDF → 页长度校验（加密无密钥在页解析与 WAL 触碰前拒绝）。`StorageError::InvalidKey(String)` 新变体。
4. **Database lib API + lib 端到端（T4）**——`Database::open_with_key(path, isolation, key)` 为核心实现（既有 `open`/`open_with_isolation` 委托，签名零变化）；新 `tests/encryption_test.rs` 套件：加密库 CRUD/restart 往返、错误密钥拒绝、明密互斥、损坏密文检测、锁冲突优先、dump→restore 加密迁移。
5. **CLI 密钥通道（T5）**——`CliArgs` 增全局 `--key`（`env = "RTSQL_KEY"`）；空密钥 → Usage exit 2；`execute_command_inner` 增 key 参数穿线（lifecycle 8 个调用点 + 主命令 = 9 个生产开库点 + mod.rs 结构测试 1 个直构点机械适配）；`open_error_status` 增 `InvalidKey` → `ExitStatus::InvalidKey`（exit 5）映射。cli_test 新用例组。
6. **Iteration 000 收口（T6）**——全量 `--no-fail-fast` 零回归（基线 1065）+ clippy/fmt/validate + 加密库打开延迟实测记录（Act Response，不建 bench 设施）。
7. **completions 子命令（T7）**——`Cargo.toml` 增 `clap_complete`（4.x）；`Command::Completions { shell }` 隐藏子命令（bash/zsh/fish，clap_complete 运行时生成到 stdout）；未知 shell → 用法错误 exit 2。cli_test 用例组。
8. **install.sh（T8）**——根目录新脚本：依赖检查（cargo）→ `cargo build --release` → strip → 安装二进制到 `$PREFIX/bin`（默认 `~/.local`）→ 按检测 shell 安装 completions（bash/zsh/fish 用户级目录）→ `--no-completions` 开关 → PATH 提示 → `--uninstall`（只清理程序：二进制 + completions）/ `--uninstall --purge-data`（追加清除 `RTSQL_HOME` 数据目录：先列出将删除路径；破坏性动作以显式 flag 为确认）；`--prefix <dir>` 覆盖。脚本冒烟（临时 PREFIX 全自动命令化）。
9. **Iteration 001 收口（T9）**——全量验证 + 脚本冒烟完整轮记录。
10. **双语 README（T10）**——README.md 英文全量重写 + README.zh-CN.md 中文：项目定位 / 快速开始（脚本安装与卸载、建库 CRUD、事务、加密用法）/ 能力清单 / 架构概览一段 / 已知限制与非目标。
11. **agent SKILL.md（T11）**——`docs/SKILL.md`（英文）：安装部署 / 使用（one-shot 主命令、子命令、格式与退出码、事务、备份恢复、分析命令）/ 管理（集中存储目录、伴生文件、锁、格式头、加密）/ 卸载；与实现逐条核对。
12. **change 收尾全量验证（T12）**——全量 + 静态门 + validate + change 结构自检。

Delta specs：

- 新增 `database-encryption`：R1 加密库磁盘格式 / R2 密钥派生与打开面（错误密钥·明密互斥·损坏检测）/ R3 密钥通道与 CLI 映射 / R4 明文库零回归与已知限制。
- 修改 `database-file-format-header`：R1 布局扩展（加密位 + 盐/参数区 + 加密库页偏移公式分支）；R2「未知特性 flag 拒绝」场景改写（加密位被本构建接受，仅真正未知位拒绝）；R4「伴生文件行为不变」场景校准（`.checkpoint` 16B → 24B，T02 T7 已实施）。
- 修改 `cli-noninteractive-shell`：R1 增全局 `--key` 参数一句 + 两处退出码场景 AND 行改写（exit 5 获得产生路径）；新增 Requirement「completions 子命令」。
- 新增 `install-script`：R1 安装面（编译安装 / completions / PATH 提示）/ R2 卸载两模式（程序清理 / `--purge-data` 数据清理）。

## BDD 场景草图（缺口扫描结论）

覆盖面按 delta spec 场景落定；要点：

- **Happy**：`new --key pw` 建加密库 → 主命令/子命令带 `--key` 全链路 CRUD/schema/dump/import/stats 可用；`close` 后带正确密钥重开数据完整；`RTSQL_KEY` 环境变量与 `--key` 等效（`--key` 优先）；`dump` 明文库 → `restore --key` 产出加密库（迁移路径）；`rtsql completions bash|zsh|fish` 输出非空补全脚本；`install.sh` 临时 PREFIX 一键安装后 `rtsql --version` 与基础 CRUD 可用、卸载两模式行为正确。
- **Sad**：错误密钥打开加密库 → exit 5（GCM 认证失败）；加密库无密钥 → exit 5 点名 `--key`/`RTSQL_KEY`；明文库带密钥 → exit 5（用户决策 2）；篡改密文页字节 → 读取时认证失败拒绝（损坏检测）；空密钥（`--key ""` / `RTSQL_KEY=""`）→ exit 2；未知 shell → exit 2；`--uninstall` 后 `rtsql` 不可用而数据目录保留；`--purge-data` 列出路径后清除 `RTSQL_HOME`。
- **Edge**：0 字节文件 + 密钥 = 加密新库唯一入口（既有 0 字节契约的密钥分支）；首列标量子查询…（无关域略）；加密库第 0/1 catalog 页与数据页同机制；AAD 换页（把页 A 密文搬到页 B 位置）→ 认证失败；页数校验（加密库非 4124 整除 → `PageSizeMismatch`）；Argon2 参数区非法值域 → `IncompatibleHeader`；锁冲突优先于密钥错误（顺序守卫）；`--prefix` 自定义目录安装与卸载对称；`--no-completions` 跳过补全安装。
- **兼容**：既有 1065 测试零修改通过（预授权校准面除外：`file_header_test` 加密位拒绝用例按语义演进而改写、`checkpoint_test`/`recovery_test` 无涉、`cli/mod.rs` 结构测试 `execute_command_inner` 直构点签名机械适配）；明文库全路径行为逐字节不变（无密钥打开 = 既有路径委托）；WAL 帧格式、checkpoint 位点格式、主库头明文库布局零变化；RR/RC 隔离语义、MVCC、退出码 0-4 分类零变化。

## Out of Scope / Non-goals

- WAL 帧与 checkpoint 位点文件加密（用户决策 2026-09-23，MS17-T01 范围预裁定沿用：本轮仅加密主库文件）——README 已知限制 + 随本 change 收尾由 docs-maintainer 登记 improvement（登记候选 I 编号随收尾定）。
- TTL 密钥缓存（I054）/ `rtsql key` 子命令（I055）/ `--password-file` 通道（I056）/ 正式加密 bench 基线（I057）。
- CI workflow（I051）/ Releases 预编译矩阵（I052）/ crates.io（I053）/ man 页（I058）。
- 密钥轮换；就地明密转换工具（迁移路径 = dump → restore ± `--key`，README 记载）；多用户/角色权限；列级加密；key agent 常驻进程。
- Windows（页 I/O 层限 Unix，`install.sh` 目标 Linux/macOS）；Homebrew/deb/AUR。
- IN×JOIN 能力解锁、多列 IN 首列静默语义裁决（T02 登记候选，无关域）。
- 页级快路径与加密叠加的性能量化（I057 域）。

## 默认假设（用户未显式裁定，按合理默认补齐，可否决）

- **DA1** Argon2id 参数默认 m=19456 KiB、t=2、p=1（OWASP 2023 首选档，打开延迟预计数十毫秒级）；参数三元组持久化于头保留区 12B（m_kib u32 LE + t u32 LE + p u32 LE），打开时按头参数派生——未来调参不破坏旧库。
- **DA2** 空密钥（`--key ""` / `RTSQL_KEY=""`）→ Usage exit 2 拒绝（防误设空环境变量静默降级）。
- **DA3** SKILL.md 英文；README.md 英文 + README.zh-CN.md 中文（tasks.md 已定双语形态，语言分派为本 DA）。
- **DA4** install.sh 数据清理 flag 名 `--purge-data`；安装 PREFIX 默认 `~/.local`（二进制 → `~/.local/bin`）；completions 恒装用户级目录（bash `~/.local/share/bash-completion/completions/`、zsh `~/.zsh/completions/`〔输出 fpath 提示〕、fish `~/.config/fish/completions/`），不受 `--prefix` 影响。
- **DA5** 运行期（非打开期）页解密失败走既有 statement 错误链（打开期 InvalidKey → exit 5 为守卫主面；语句执行期页损坏属存储错误路径既有分类）。
- **DA6** nonce 12B 每次 `write_page` 经 OS 随机源新生成；AAD = page_id u64 LE；不引入密钥内存 zeroize（v0.1 最小集，README 不承诺内存驻留防护）。
- **DA7** `database-file-format-header` R4 场景 16B → 24B 位点校准随本 change 顺带（T02 T7 已实施的事实性修正）。
