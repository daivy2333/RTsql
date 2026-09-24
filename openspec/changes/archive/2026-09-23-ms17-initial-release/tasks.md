# MS17 初版分发收口 — Tasks

> 全局任务编号 T1–T12；Iteration 规划见文末 Iteration Plan。状态：`pending` / `in-progress` / `done` / `skipped`。

## T1: 加密内核模块（依赖 + crypto.rs）

- 状态: done
- `Cargo.toml`：新增 `argon2 = "0.6"`、`aes-gcm = "0.11"`（默认 features）。
- 新模块 `src/storage/crypto.rs`：`derive_key(password: &[u8], salt: &[u8; 32], m_kib: u32, t: u32, p: u32) -> [u8; 32]`（Argon2id）；`PageCipher`（持派生密钥）`encrypt_page(page_id: PageId, page: &[u8; 4096]) -> [u8; 4124]`（随机 12B nonce + AES-256-GCM + AAD = page_id u64 LE）与 `decrypt_page(page_id: PageId, record: &[u8; 4124]) -> Result<[u8; 4096], CryptoError>`（认证失败 → `CryptoError::AuthFailed`）；`KdfParamsError` 值域校验。`src/storage/mod.rs` 导出。
- 测试见证（RED 先行，新模块编译演进先例）：KDF 同参数决定性 / 不同盐不同密钥；页加解密往返；错误密钥认证失败；AAD 篡改（换 page_id）拒绝；密文位翻转拒绝；同页两次加密 nonce 不同。
- 验证: `cargo test --lib storage::crypto`（决定性输出 + 退出码记 Act Response）。

## T2: 格式头扩展（加密位 + 盐/参数区）

- 状态: done
- `src/storage/file_header.rs`：`KNOWN_FLAGS_MASK` `0` → `FLAG_ENCRYPTED`；`FileHeader` 加性增 `salt: [u8; 32]`、`kdf_params: [u8; 12]` 字段（`FileHeader::current()` 明文头恒零、encode/decode 明文形态逐字节不变）；加密头条件校验（盐全零或参数值域违规 t<1 / p<1 / m_kib ∉ 1024..=2^24 → 既有 `ReservedNonZero` 分类）；加密头构造入口 `current_encrypted(salt, m_kib, t, p)`。`src/storage/mod.rs` 导出同步。
- 校准面（预授权）：`tests/file_header_test.rs::encrypted_flag_rejected_by_current_build` 按语义演进而改写（加密位现被接受——改断言加密头往返 + 未知位仍拒绝）；其余 13 既有用例零修改。
- 测试见证（RED 先行）：加密头 encode/decode 往返（盐/参数保真）；明文头盐/保留区非零仍拒绝（既有用例锁定）；加密头参数值域拒绝矩阵；未知 flag 位（bit1）仍拒绝。
- 验证: `cargo test --lib file_header` 全绿。

## T3: FileStorage 加密接线 + InvalidKey 错误面

- 状态: done
- `src/storage/file_storage.rs`：`open_with_key(path, key: Option<&str>)` 新核心（既有 `open` 委托 `None`，签名零变化）；0 字节文件 + 密钥 → 写加密头（OS 随机盐 + DA1 默认参数）；打开顺序守卫扩展——锁 → 头校验 → 密钥检查/KDF（加密无钥 / 明文带钥 → `InvalidKey`，页解析与 WAL 触碰前）→ 页长度校验（加密库按 4124 步长）；`read_page_blocking`/`write_page_blocking` 经 `PageCipher` transform；`allocate_page`/`page_count` 按模式步长。
- `src/storage/error.rs`：`InvalidKey(String)` 变体（Display `invalid key: {0}`）。
- 测试见证（RED 先行）：明文库 `open` 与 `open_with_key(None)` 字节级等价（既有 file_storage_io 套件零修改锁定）；加密库落盘形态（首字节非明文页 + 4124 步长）；错误密钥 open 拒绝；明文带钥 / 加密无钥拒绝；页密文篡改读取拒绝；allocate/页数步长正确。
- 验证: `cargo test --lib file_storage` + `cargo test --lib crypto` 全绿。

## T4: Database lib API + lib 端到端

- 状态: done
- `src/database.rs`：`open_with_key(path, isolation, key: Option<&str>)` 新核心实现（storage 构造点换 `open_with_key`）；既有 `open`/`open_with_isolation` 委托，签名零变化（MS17-T02 D8 水位逻辑原样保留在其体内）。
- 新 `tests/encryption_test.rs` 套件：加密库 CRUD + `close` → 正确密钥重开数据完整（restart 往返）；错误密钥重开 `InvalidKey`；加密无钥 / 明文带钥拒绝；页密文篡改（reopen 后读触发表）检测；锁冲突优先于密钥错误（持有者占锁 + 无钥打开加密库 → `DatabaseLocked`）；dump（明文库）→ `restore --key` lib 面（经 CLI 属 T5，此处锁 lib 等价路径）。
- 测试见证（RED 先行）：`open_with_key` 对加密库 CRUD 前不可用（编译演进 / API 未有即 RED）；各拒绝形态 RED 于现状（无加密能力）。
- 验证: `cargo test --test encryption_test` 全绿 + 既有 `file_header_test`/`isolation_level_test` 零修改。

## T5: CLI 密钥通道（--key / RTSQL_KEY / exit 5）

- 状态: done
- `src/cli/mod.rs`：`CliArgs` 增 `#[arg(long, value_name = "KEY", global = true, env = "RTSQL_KEY")] key: Option<String>`；空密钥 → `ExitStatus::Usage("key must not be empty")`（exit 2，开库前）；`execute_command_inner` 增 `key: Option<&str>` 参数 → `Database::open_with_key(db_path, RepeatableRead, key)`；`open_error_status` 增 `InvalidKey` → `ExitStatus::InvalidKey`（exit 5）臂；`ExitStatus` doc 注释「InvalidKey 当前无产生路径」改写。lifecycle 8 个调用点 + 主命令臂传参（机械适配）；`mod.rs` 结构测试直构点签名适配（断言集不变，预授权）。
- 测试见证（RED 先行）：cli_test 新用例组——`new --key` 建加密库 + 带钥 CRUD/schema/dump/import/stats 全链路；`RTSQL_KEY` env 等效 + `--key` 覆盖 env（EnvGuard 串行先例，显式 flag 用例不经 env）；错误密钥 exit 5 且消息点名；加密无钥 exit 5；明文带钥 exit 5；空密钥 exit 2；无钥 `list` 不受影响。
- 验证: `cargo test --test cli_test` 全绿（既有 81 零修改 + 新增）。

## T6: Iteration 000 收口验证 + 打开延迟实测

- 状态: done
- 全量 `cargo test --no-fail-fast` 零回归（基线 1065 + 新增，预期计数 Act 实测记录）+ `cargo clippy --all-targets -- -D warnings` 0 + `cargo fmt --check` 0 + `openspec validate` PASS。
- 加密库 vs 明文库 `Database::open` 计时实测（标准参数，各 ≥3 次取样记录 Act Response；不建 bench 设施、不设阈值断言）。
- change 结构自检（本 Iteration 范围）：tasks T1-T6 状态一致、delta spec 与实现一致、Iteration/Cycle 文件齐全。
- 验证: 各命令决定性输出 + 退出码记 Act Response。

## T7: completions 隐藏子命令

- 状态: done
- `Cargo.toml`：新增 `clap_complete = "4"`。
- `src/cli/mod.rs`：`Command::Completions { shell: clap_complete::Shell }`（`hide(true)`；bash/zsh/fish 三值收窄，未知 shell clap 用法错误 exit 2）；`execute_command` 分发臂 → `CliArgs::command()` + `clap_complete::generate(shell, &mut cmd, "rtsql", &mut stdout)`。
- 测试见证（RED 先行）：cli_test 新用例——三 shell 各生成非空脚本且含 `rtsql` 与 `--key`；未知 shell 值 exit 2；`rtsql --help` 不出现 completions（隐藏面）；`rtsql completions`（缺 shell）exit 2。
- 验证: `cargo test --test cli_test` 全绿。

## T8: install.sh 一键安装脚本

- 状态: done
- 新文件 `install.sh`（bash `set -euo pipefail`；design D7 全语义）：安装（cargo 检查 → `cargo build --release` → strip → `$PREFIX/bin` 安装 → completions 三 shell 检测安装 → PATH 提示）/ `--no-completions` / `--prefix` / `--uninstall` 两模式（`--purge-data` 先列路径后删）/ `--help`。
- 测试见证（脚本冒烟，临时 PREFIX 全自动命令化）：安装 → `$PREFIX/bin/rtsql --version` 0 → 临时 `RTSQL_HOME` 建库 CRUD → completions 文件存在且 bash source 可装载 → `--uninstall` 后二进制与 completions 消失、`RTSQL_HOME` 保留 → 重装 → `--uninstall --purge-data` 后数据目录消失。
- 验证: 冒烟命令逐条退出码 + 文件存在性记 Act Response。

## T9: Iteration 001 收口验证

- 状态: done
- 全量 + clippy/fmt/validate（基线 = T6 记录 + T7 新增）+ 结构自检（T7/T8 范围）。
- 验证: 各命令决定性输出记 Act Response。

## T10: 双语 README

- 状态: done
- `README.md` 英文全量重写（现 2026-08-25 中文旧版退役）+ `README.zh-CN.md` 中文同构：定位 / 快速开始（脚本安装与卸载、建库 CRUD、事务、加密用法〔`--key`/`RTSQL_KEY`、dump→restore 迁移〕）/ 能力清单 / 架构概览一段 / 已知限制与非目标（D8 清单：WAL/checkpoint 明文、脚本逐语句 KDF 延迟、无就地转换、Unix-only、密钥轮换无等）。
- 测试见证（内容一致性，人工最短步骤）：两文档快速开始段命令块逐条执行，输出/退出码与文档声称一致（一句话记录比对结论）；能力清单与 `rtsql --help` 输出核对。
- 验证: 比对记录记 Act Response；无 orphan 链接（`README.zh-CN.md` 互链有效）。

## T11: agent SKILL.md 说明书

- 状态: done
- 新文件 `docs/SKILL.md`（英文，D8 结构）：Install/Deploy、Usage（one-shot、子命令、格式与退出码 0-5、事务、备份恢复、分析命令、加密）、Management（存储目录、伴生文件、锁、格式头、加密语义）、Uninstall；README 双语互链。
- 测试见证（内容一致性）：SKILL.md 命令块逐条执行比对（同 T10 方式）。
- 验证: 比对记录记 Act Response。

## T12: change 收尾全量验证与结构自检

- 状态: done
- 全量 `cargo test --no-fail-fast` + clippy + fmt + `openspec validate`；结构自检——tasks T1-T12 状态一致、specs/design 与已实现行为一致、3 Iteration × Cycle 文件齐全、`Review Result` 与流程状态一致。
- 验证: 各命令决定性输出 + 退出码记 Act Response。

---

## Iteration Plan

### Iteration 000: encryption-core（最小加密：内核 + 格式头 + lib/CLI 通道 + 收口）

- Tasks: T1, T2, T3, T4, T5, T6
- Depends on: None
- Stable baseline: 可选整库加密全链路可用——加密库建/开/读写/重启两态一致，错误密钥·明密互斥·损坏检测三拒绝面 exit 5，明文库零回归，`--key`/`RTSQL_KEY` 全命令面生效；全量门稳定（1065 + 新增）。
- Verification boundary: crypto/file_header/file_storage 单测 + encryption_test + cli_test 新用例全绿 + 既有套件零修改（预授权校准面除外）+ T6 全量收口与延迟实测记录。
- Diagnostic boundary: `src/storage/{crypto,file_header,file_storage,error,mod}.rs`、`src/database.rs`、`src/cli/{mod}.rs`（+ lifecycle 传参机械面）、`tests/{file_header_test,encryption_test,cli_test}.rs`。
- Non-goals: completions/install.sh（Iter 001）；文档（Iter 002）；WAL/checkpoint 加密；性能 bench；密钥便利层（I054-I056）。
- 平衡审计: 六任务构成单一成果「可选整库加密可用」，链路严格分层递进（内核 → 头 → 存储 → lib → CLI），Task Contract 各自独立 RED→GREEN 与验收面；CLI 通道与 lib 面合并因共享同一咽喉点改造（拆开会产生 lib-only 中间态无用户可见面，且 `execute_command_inner` 签名改造一次完成最省）；不拆。

### Iteration 001: install-surface（completions + install.sh + 收口）

- Tasks: T7, T8, T9
- Depends on: Iteration 000（全量门稳定；变更面零重叠——cli/mod.rs 新子命令分发臂与 T5 密钥穿线互不触碰）
- Stable baseline: `rtsql completions <bash|zsh|fish>` 可达且隐藏面正确；`install.sh` 安装/卸载两模式冒烟通过；全量门稳定。
- Verification boundary: cli_test completions 用例全绿 + 脚本冒烟全轮记录 + T9 全量收口。
- Diagnostic boundary: `src/cli/mod.rs`（分发臂 + 生成函数）、`Cargo.toml`、`install.sh`、`tests/cli_test.rs`。
- Non-goals: man 页（I058）/ CI（I051）/ Releases（I052）/ crates.io（I053）；SKILL.md 安装（用户决策 5）。
- 平衡审计: 两任务同属「本机可安装可补全」单一成果；completions 是 install.sh 安装内容的组成部分（脚本安装 completions 消费同一子命令产物），合并验收闭环；与加密域变更面零重叠、独立诊断面。不与 000 合并：加密是重活需独立验收边界与诊断隔离（MS09 Iter000 先例）。

### Iteration 002: docs-closeout（双语 README + SKILL.md + change 收尾）

- Tasks: T10, T11, T12
- Depends on: Iteration 000 + 001（加密用法与安装用法均定稿——文档记载面冻结前提）
- Stable baseline: README.md / README.zh-CN.md / docs/SKILL.md 与实现一致且命令块全部可执行；change 全部收口。
- Verification boundary: 文档命令块逐条执行比对记录 + T12 全量与结构自检。
- Diagnostic boundary: `README.md`、`README.zh-CN.md`、`docs/SKILL.md`（纯文档，无产品代码变更面）。
- Non-goals: 文档自动化测试设施；man 页（I058）；Purpose 占位清理等语料库杂项（I045 域，随 maintainer 收尾顺带与否由其裁定）。
- 平衡审计: 三文档同属「初版可读」单一成果，内容互链、验证方式同构（命令块逐条比对）；T12 收尾并入形成 change 级验证闭环（T02 T6/T8 同构先例）。独立成轮因依赖前两轮交付面冻结——先写文档会在加密/脚本语义变化时返工。

## Requirements Traceability Matrix

| Requirement (delta spec) | Scenario 代表 | Design | Task | Iter | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| database-encryption R1 加密库磁盘格式 | 4124 步长落盘形态 / allocate / 页数校验 / AAD | D1 | T1, T3 | 000 | `crypto.rs::PageCipher`、`file_storage.rs::{open_with_key,read/write_page_blocking,allocate_page}` | crypto 往返与 nonce 单测 + file_storage 落盘形态用例 | None | Covered |
| database-encryption R2 密钥派生与打开面 | KDF 决定性 / 错误密钥 / 明密互斥 / 损坏检测 / 顺序守卫 | D1, D2, D3 | T1, T2, T3, T4 | 000 | `crypto.rs::derive_key`、`file_header.rs` 条件校验、`file_storage.rs` 守卫、`error.rs::InvalidKey` | crypto/header/file_storage 单测 + encryption_test e2e | None | Covered |
| database-encryption R3 密钥通道与 CLI 映射 | --key/RTSQL_KEY 全命令面 / 空密钥 exit 2 / exit 5 三形态 / new·restore 建加密库 | D3, D5 | T4, T5 | 000 | `database.rs::open_with_key`、`cli/mod.rs::{CliArgs,execute_command_inner,open_error_status}` + lifecycle 传参 | cli_test 新用例组（exit code 断言） | None | Covered |
| database-encryption R4 明文库零回归与已知限制 | 明文路径逐字节 / 既有套件零修改 / WAL·位点明文 | D1, D6 | T3, T4, T6 | 000 | `file_storage.rs::open` 委托壳 | 既有 file_storage/file_header/isolation 套件零修改 + 全量门 | None | Covered |
| database-file-format-header 修改（R1 布局扩展 / R2 场景改写 / R4 位点 24B 校准） | 加密头往返 / 未知位仍拒 / 明文头零变化 / 24B 校准 | D2, DA7 | T2 | 000 | `file_header.rs` | file_header_test（13 零修改 + 1 改写校准 + 新增） | None | Covered |
| cli-noninteractive-shell R1 修改（--key 提及 + 退出码场景改写） | exit 5 产生路径 / 锁冲突场景 AND 行 | D3, D5 | T5 | 000 | `cli/mod.rs::ExitStatus/From` | cli_test exit 5 用例 | None | Covered |
| cli-noninteractive-shell 新增 Requirement completions | 三 shell 生成 / 隐藏面 / 未知 shell exit 2 | D7 | T7 | 001 | `cli/mod.rs::Command::Completions` | cli_test completions 用例 | None | Covered |
| install-script R1 安装面 | 临时 PREFIX 冒烟 / completions 装载 / PATH 提示 | D7 | T8 | 001 | `install.sh` | 脚本冒烟命令记录 | None | Covered |
| install-script R2 卸载两模式 | 程序清理后数据保留 / --purge-data 列路径后清除 | D7 | T8 | 001 | `install.sh` | 冒烟两模式轮 | None | Covered |
| （文档交付物一致性，无行为 requirement）README ×2 + SKILL.md | 命令块逐条可执行 | D8 | T10, T11 | 002 | `README.md`、`README.zh-CN.md`、`docs/SKILL.md` | Act 逐条执行比对记录 | None | Covered |
| 全部域零回归 | 全量零修改（预授权校准面除外） | D9 | T6, T9, T12 | 000/001/002 | — | 全量 `--no-fail-fast` + clippy/fmt/validate | None | Covered |
