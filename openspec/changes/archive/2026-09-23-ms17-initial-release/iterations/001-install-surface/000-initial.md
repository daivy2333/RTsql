# Iteration 001 / Cycle 000: install-surface 初始执行

## Plan Context

- Status: ready
- Iteration: 001-install-surface
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T7, T8, T9
- Depends on: Iteration 000（encryption-core，2026-09-24 Review accepted——全量门稳定 1099/0/2，CLI 面冻结）
- Stable baseline: `rtsql completions <bash|zsh|fish>` 可达且隐藏面正确（三 shell 各生成非空补全脚本含 `rtsql` 与 `--key`，缺参/非三值 exit 2，`--help` 不可见）；`install.sh` 安装/卸载两模式冒烟通过（安装 → 版本 + CRUD + completions 装载 → 程序卸载后数据保留 → 重装 → `--purge-data` 列路径后数据目录消失）；全量门稳定（基线 = Iteration 000 T6 记录 + T7 新增）。
- Verification boundary: cli_test completions 用例全绿 + 既有 89 用例零修改 + 脚本冒烟全轮命令与退出码记录 + T9 全量收口。
- Diagnostic boundary: `src/cli/mod.rs`（Command 变体 + 分发臂 + 生成调用）、`Cargo.toml`、`install.sh`、`tests/cli_test.rs`。
- Deferred tasks: T10-T12（Iteration 002 docs-closeout）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal T7/T8 承诺与用户决策 2-5（本 Iteration 消费决策 5：SKILL.md 不由 install.sh 安装）；design D7（completions 机制与 install.sh 全语义）+ D9（冒烟验证方式）；Iteration 000 冻结的 CLI 面——`--key`/`--format` global、9 子命令分发、退出码 0-5 映射、空密钥守卫位置
- Excluded scope: 双语 README 与 docs/SKILL.md（T10/T11，Iteration 002）；man 页（I058）；CI workflow（I051）；Releases 矩阵（I052）；crates.io（I053）；WAL/checkpoint 加密；密钥便利层（I054-I056）；PowerShell/Elvish 等未收窄 shell；任何产品行为变化

**Objective**

「本机可安装可补全」交付面按 Task Contract 落地：`rtsql completions <bash|zsh|fish>` 隐藏子命令（clap_complete 运行时生成，覆盖主命令、全局参数含 `--key`、全部子命令）与仓库根 `install.sh` 一键编译安装脚本（安装 / `--no-completions` / `--prefix` / `--uninstall` 两模式），全部有 cli_test 用例与脚本冒烟见证；既有 CLI 行为零回归。

**Background**

MS17 初版第二棒（proposal Why 节 2/3）：安装面是用户裁定 v0.1 交付形式「本机一条命令可编译可安装」的唯一承载。completions 子命令是 install.sh 补全安装步骤的内容来源（install-script delta R1 明文「补全内容来自 `rtsql completions <shell>` 同源生成」），故 T7 先于 T8。Iteration 002 的文档以本 Iteration 冻结的安装/补全语义为记载基准。

**Investigation Facts**

- Current Baseline: master `7364bc9` + 未提交叠层（MS13 实施 + MS09/MS13 收尾 docs sync + MS17-T02 实施 + T02 收尾 docs sync + 本 change T1-T6）；**全量 1099 passed / 0 failed / 2 ignored**（来源：Iteration 000 最终 Act Response 修复后重跑，其 accepted Review 已采信；覆盖表面自该次运行未变化，本 Cycle 开工前无需重跑）；clippy --all-targets 0、fmt --check 0、`openspec validate --specs` 37 PASS。cli_test 现 89 用例（81 既有 + 8 key 组）+ 2 ignored。change tasks T1-T6 done、T7-T12 pending。
- Current-State Evidence（本会话新鲜读码/核对）：
  - **依赖面**：`Cargo.toml:11` `clap = { version = "4", features = ["derive", "env"] }`；`:24-25` `argon2 = "0.6"` / `aes-gcm = "0.11"`（Iteration 000 T1 落地）；clap_complete 未引入（grep 零命中）。clap_complete 4.x 可用性已由 Iteration 000 调查实测（`cargo search`：`clap_complete 4.6.11`，网络可用）。
  - **CLI 结构**：`src/cli/mod.rs:66-85` `CliArgs`（`#[derive(Parser)]`，`#[command(name = "rtsql", version, about = ...)]`；db/sql/format/key 字段 + `#[command(subcommand)] command: Option<Command>`）——Parser derive 同时给出 `CliArgs::command()`（CommandFactory），即 completions 生成所需的命令树来源；`:89-153` `Command` 枚举 9 变体（New/List/Schema/Dump/Restore/Import/Stats/Sample/Profile）；`:173-203` `execute_command`（`:175-177` 空密钥守卫在 match 之前；`:178-202` 逐变体分发臂；`None` → `execute_main_command`）；`:164-171` `run()`（`CliArgs::parse()` → execute → message → ExitCode）。`ExitStatus`（`:28-36`）含 `Success` 变体（exit 0、无 stderr 消息）——completions 成功臂的返回形态。
  - **clap 用法错误出口**：`CliArgs::parse()` 对缺参/非法枚举值自行以退出码 2 终止（clap 默认错误退出码 2）——completions 缺 shell 与非三值 shell 的 exit 2 由该出口承载，不经 `ExitStatus::Usage` 手工构造（与既有 `--format bogus` 同型）。
  - **测试基建**：`tests/cli_test.rs` 夹具 `fixture()` / `run_cli()` / `spawn_cli_env` / `run_cli_env`（T5 修复轮引入，env 用例经 EnvGuard 串行）；既有断言模式 = exit code 断言 + stdout/stderr `contains`。
  - **install.sh**：全新文件（仓库根 ls 实证不存在）；语义权威 = design D7 + delta spec `install-script` R1（3 场景）/ R2（2 场景）；无产品代码交互（脚本消费 `rtsql completions` 输出与 `cargo build` 产物）。
  - **结构测试直构点**：`cli/mod.rs` tests 模块的 `execute_command_inner` 直构点已因 Iteration 000 T5 五参适配；completions 臂不开库、不触 `execute_command_inner`，预期本 Iteration 零适配。
- Code and Critical Path:
  - T7 变更面：`Cargo.toml`（+`clap_complete = "4"`）+ `src/cli/mod.rs`（`Command::Completions { shell }` 隐藏变体 + `execute_command` 新分发臂 + stdout 生成调用）。生成路径不触库、不持锁、不消费密钥、不触信号 future。
  - T8 变更面：新 `install.sh`（bash 严格模式）；零产品代码。
  - T9：验证收口，无代码。
  - 测试面：`tests/cli_test.rs` 新 completions 用例组。

**Implementation Guidance**

顺序 T7 → T8 → T9：completions 子命令是 install.sh 补全安装步骤的内容来源，先有产物来源再有消费者。T7 注意点：(1) **三值收窄是场景级硬约束**——`clap_complete::Shell` 枚举含 PowerShell/Elvish 等变体，直接作为参数类型会让 `completions powershell` 成功生成、违反 delta spec「未收窄值 exit 2」场景；按 D7 二选一（`value_parser` 过滤三值，或自定义三值 `ValueEnum` 映射到 `clap_complete::Shell`），机制非实质、行为（powershell → exit 2）必须达成。(2) `#[command(hide = true)]` 保证 `--help` 隐藏面。(3) 生成调 `clap_complete::generate(shell, &mut CliArgs::command(), "rtsql", &mut io::stdout())`，成功臂返回 `ExitStatus::Success`。(4) 空密钥守卫现位置保持（见 Risks）。T8 注意点：`set -euo pipefail`；cargo 检查先于 build；strip 失败跳过并提示、不失败；`--purge-data` 先逐行列出后删除、无交互提示；`--prefix` 安装/卸载对称；completions 目录不受 `--prefix` 影响（DA4）；PATH 提示仅当 `$PREFIX/bin` 不在 PATH 时输出。冒烟全程临时 `PREFIX` + 临时 `RTSQL_HOME` 全自动命令化（D9）。

**Behavioral Change**

- 新能力：`rtsql completions <bash|zsh|fish>`（隐藏）——stdout 输出对应 shell 的补全脚本（覆盖主命令、全局参数含 `--key`、全部子命令），exit 0；`<shell>` 缺失或非三值（如 powershell）→ clap 用法错误 exit 2；`rtsql --help` 输出不含 completions；生成路径不开数据库、不消费密钥。
- 新交付物：仓库根 `install.sh`——安装（cargo 检查 → `cargo build --release` → strip → `$PREFIX/bin/rtsql` → 按 `$SHELL` 检测安装 completions → PATH 提示）/ `--no-completions` / `--prefix <DIR>` / `--uninstall`（只清程序：二进制 + 三 completions 文件，数据目录保留）/ `--uninstall --purge-data`（先列后清数据目录）/ `--help`；严格模式，任一安装步骤失败非零终止。
- 既有行为零变化：CLI 退出码 0-5、9 子命令、主命令、`--key`/`--format` global 语义、加密链路全部不动（cli_test 既有 89 用例零修改锁定）；产品代码变更面仅 `Cargo.toml` + `cli/mod.rs`。

**Task Contracts**

### T7: completions 隐藏子命令

- Requirement/Scenario: cli-noninteractive-shell delta 新增 Requirement「Shell 补全脚本生成」全场景（三 shell 补全脚本生成 / 隐藏面与用法错误）
- Depends on: None（Iteration 000 已交付 CLI 面）
- Targets: `Cargo.toml`（dependencies）、`src/cli/mod.rs`（`Command` 枚举 + `execute_command`）
- Current behavior: 无 completions 子命令；`rtsql completions ...` 被按主命令位置参数解析（db="completions"）报缺 SQL 用法错误
- Required behavior: `Command::Completions { shell }` 变体（`hide = true`）：bash/zsh/fish 三值 → stdout 非空补全脚本（含 `rtsql` 命令名与 `--key`）+ `ExitStatus::Success` exit 0；缺 shell、非三值（powershell 等）→ exit 2（clap 用法错误出口）；`rtsql --help` 不含 completions；生成路径不开库、不消费密钥、不触信号 future
- Required changes: `Cargo.toml` +`clap_complete = "4"`；`mod.rs` 变体 + 分发臂 + 生成调用（命令树经 `CliArgs::command()`）
- Preserve: 既有 9 变体分发臂与主命令臂逐字节；`--key`/`--format` global 语义；空密钥守卫现位置；退出码 0-5 映射表；lifecycle 各函数零触碰
- Forbidden: 为 completions 新开退出码或改 0-5 映射；生成路径开库/手工读 env；生成到文件而非 stdout；扩大 shell 收窄集（bash/zsh/fish 之外）
- Test witness（RED 先行）: cli_test 新用例组——(1) 三 shell 各生成非空脚本且含 `rtsql` 与 `--key`、exit 0；(2) `rtsql completions powershell` exit 2；(3) `rtsql completions`（缺参）exit 2；(4) `rtsql --help` 输出不含 completions；(5) 生成路径不开库（无库文件环境即成功，夹具即空环境）。RED 形态 = 现状无子命令时 (1) 失败（completions 被当 db 名）、(4) 失败（无隐藏面可言）
- GREEN condition: 新用例全绿 + `cargo test --test cli_test` 既有 89 零修改全绿
- Verification: `cargo test --test cli_test`（决定性输出 ≤10 行，exit 0；RED 记录入 Act Response）
- Stop when: `hide = true` 后 `--help` 仍出现 completions（clap 版本行为与假设不符）；或非三值拒绝无法在 clap 层达成（value_parser 收窄失效）——返回 Plan

### T8: install.sh 一键安装脚本

- Requirement/Scenario: install-script delta R1 全场景（临时 PREFIX 一键安装可用 / 补全安装与装载 / PATH 提示）+ R2 全场景（程序卸载后数据保留 / 带数据清理须显式 flag 并列出路径）
- Depends on: T7（completions 产物来源）
- Targets: 新 `install.sh`（仓库根）
- Current behavior: 无脚本
- Required behavior: D7 全语义——`set -euo pipefail`；用法 `./install.sh [--prefix DIR] [--no-completions]` / `./install.sh --uninstall [--purge-data] [--prefix DIR]` / `--help`；安装流 = cargo 存在性检查 → `cargo build --release` → strip 目标二进制（不可用跳过+提示不失败）→ 安装 `$PREFIX/bin/rtsql`（默认 `~/.local`）→ 按 `$SHELL`+命令存在性检测安装 completions（bash `~/.local/share/bash-completion/completions/rtsql`、zsh `~/.zsh/completions/_rtsql` 并 stdout 提示 fpath 追加、fish `~/.config/fish/completions/rtsql.fish`；补全内容 = `rtsql completions <shell>` stdout 同源生成）→ `$PREFIX/bin` 不在 PATH 时输出 export 提示；`--uninstall` = 删二进制 + 三 completions 文件（存在才删）、SHALL NOT 触数据目录；`--uninstall --purge-data` = 追加清除 `$RTSQL_HOME`（未设默认 `~/.rtsql/`），**先逐行列出将删除路径再删**、无交互提示；任一安装步骤失败非零退出
- Required changes: 新脚本一个（零产品代码）
- Preserve: 脚本不修改仓库内任何文件、不写产品代码路径；`~/.rtsql`/`$RTSQL_HOME` 仅在 `--purge-data` 路径被触碰
- Forbidden: 交互式确认（破坏性确认由显式 flag 承担，tasks.md 原文）；网络下载（curl/wget）；sudo；安装 man 页或 README/SKILL.md（用户决策 5）
- Test witness（脚本冒烟，临时 PREFIX + 临时 RTSQL_HOME 全自动命令化）: (1) `PREFIX=<tmp> ./install.sh` exit 0；(2) `<tmp>/bin/rtsql --version` exit 0；(3) 临时 `RTSQL_HOME` 建库 CRUD exit 0；(4) 对应 shell 补全文件存在、内容含 `rtsql`、`bash -c 'source <file>'` 装载无错误；(5) PATH 提示出现（临时 bin 不在 PATH）；(6) `--no-completions` 重装不产生补全文件；(7) `--uninstall` 后二进制与补全消失、`RTSQL_HOME` 保留；(8) 重装后 `--uninstall --purge-data`：stdout 先列数据路径、执行后数据目录消失
- GREEN condition: 冒烟命令逐条退出码与文件断言全部达成并记 Act Response
- Verification: 冒烟逐条命令（每条 ≤5 行输出摘要 + exit code 记录；判定 = 命令原生输出与退出码，无判定层）
- Stop when: 本机 cargo/strip 环境缺失致冒烟无法执行（能力边界，记录环境事实返回用户）；zsh/fish 检测目录与 D7 约定冲突需环境事实裁定——返回 Plan

### T9: Iteration 001 收口验证

- Requirement/Scenario: RTM「全部域零回归」行 + install-script/cli 两 delta 场景汇总
- Depends on: T7, T8
- Targets: change 级验证，无产品代码变更面
- Current behavior: 基线 = Iteration 000 T6 记录（1099/0/2）+ T7 新增
- Required behavior: 全量 `cargo test --no-fail-fast` 零回归（预期计数 = 1099 + completions 用例数，Act 实测记录）+ `cargo clippy --all-targets -- -D warnings` 0 + `cargo fmt --check` 0 + `openspec validate --specs` PASS + 结构自检（tasks T7-T9 状态一致、delta spec 与实现一致、Iteration/Cycle 文件齐全、Review Result 与流程状态一致）
- Required changes: 无
- Preserve: 既有测试套件零修改
- Forbidden: 为判定验证结果新增脚本/封装/判定器；重跑已通过验证增强信心
- Test witness: 各命令决定性输出与退出码记 Act Response（全量输出 ≤20 行）
- GREEN condition: 全量 0 failed、clippy/fmt 0、validate PASS、自检各项一致
- Verification: `cargo test --no-fail-fast`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate --specs`
- Stop when: 全量非预期失败（对照 T7/T8 Changed Files 定位归属，无法归属返回 Plan）

**Invariants**

- 既有 CLI 全链路行为零变化（cli_test 89 用例零修改锁定）；产品代码变更面仅 `Cargo.toml`（+clap_complete）与 `cli/mod.rs`（completions 变体 + 分发臂）。
- completions 生成路径不开数据库、不持锁、不消费密钥、不触信号 future；`execute_command_inner` 签名零变化。
- 退出码 0-5 映射零变化；completions 用法错误经 clap 既有 exit 2 出口。
- install.sh 不修改仓库内文件、无网络、无 sudo；数据目录删除仅存在于 `--purge-data` 路径且先逐行列出后删除。

**Non-goals**

T10-T12（Iteration 002）；man 页（I058）；CI workflow（I051）；Releases 矩阵（I052）；crates.io（I053）；SKILL.md 安装（用户决策 5）；PowerShell/Elvish 等未收窄 shell；Windows；密钥便利层（I054-I056）。

**Acceptance**

- T7：cli_test 新用例组全绿 + 既有 89 零修改；三 shell 生成 / 隐藏面 / 非三值与缺参 exit 2 可见。
- T8：冒烟 8 步全轮通过并记录（安装可用 / 补全装载 / PATH 提示 / `--no-completions` / 两模式卸载语义 / purge-data 列路径）。
- T9：全量零回归 + clippy/fmt/validate + 结构自检。
- Iteration 级：install-script delta R1/R2 全场景与 cli delta completions Requirement 全场景均有见证；RTM 对应行维持 Covered。

**Verification**

- T7：`cargo test --test cli_test` → 全绿 exit 0；RED 记录入 Act Response。
- T8：冒烟逐条命令 + 退出码 + 文件存在性（判定 = 命令原生输出与退出码；人工步骤无——冒烟全自动命令化）。
- T9：全量 `--no-fail-fast`（≤20 行）+ clippy/fmt/validate。每项以被测命令退出码与原生输出为准，无判定层。

**Gate 2 Readiness**

- 无 Missing requirement：cli delta completions Requirement（2 场景）与 install-script R1（3 场景）/ R2（2 场景）全部映射 T7/T8，RTM 行 Covered（PASS；tasks.md RTM）。
- 无 Simplified requirement（PASS；proposal Out of Scope 均为已登记 I 候选或用户预裁定，非需求裁剪）。
- 调查完整：CLI 结构（CliArgs/Command/execute_command/ExitStatus/clap 错误出口）、依赖面（clap_complete 未引入、可用性已实测）、测试基建（夹具与断言模式）、install.sh 全新面——本会话新鲜读码/核对（PASS；Investigation Facts）。
- 设计闭合：D7 已钉定 completions 机制二选一边界与 install.sh 全语义（含目录约定、卸载两模式、purge-data 列表语义、strip 失败兜底）；三值收窄的行为约束为场景级硬约束、机制留 Act（PASS；design D7）。
- 任务可执行：T7/T8/T9 各有 Targets、行为变化、RED/冒烟见证、Preserve/Forbidden 与停止条件（PASS；Task Contracts）。
- 分轮合理：Iteration Map 三轮依赖有序（000 accepted → 001 → 002）；001 内 T7→T8 消费关系明确、T9 收口；tasks.md 平衡审计已记录（PASS）。
- 追踪完整：requirement→scenario→design→task→代码→测试链路齐备（PASS；RTM）。
- 验证充分：覆盖全部已批准 scenario（happy：三 shell 生成 / 临时 PREFIX 安装 / 补全装载；sad：非三值与缺参 exit 2 / 程序卸载数据保留；edge：`--no-completions` / PATH 已在 PATH 不提示 / purge-data 列路径；兼容：既有 89 cli 用例 + 全量门零回归），每条最简直接判定（PASS）。
- 无身份型证据工程/判定层（PASS；冒烟为命令 + 退出码 + 文件存在性，D9）。
- 无需 Act 决定的实质未知项：三值收窄机制、`ExitStatus::Success` 臂形态、completions 用例组织方式均为非实质；场景级行为（powershell → exit 2、隐藏面、stdout 生成、不开库不消费密钥）已钉定（PASS；Risks and Notes）。
- OpenSpec tasks/specs/design/当前 Iteration/Cycle 一致（PASS；同源撰写）。
- Persisted Evidence 明确：`none`（PASS；冒烟与测试输出均 ≤20 行可入 Act Response，全部可低成本重跑）。
- 计划批准：范围（单 change 合并 T01+T03+T04）与 Iteration Map（含 001-install-surface = T7/T8/T9）经用户 2026-09-23 批准（Iteration 000 cycle「计划批准」段，原话「批准」）；本 Cycle 按既有 Map 展开，无新增决策（PASS）。

**Persisted Evidence**

- Mode: none

（冒烟逐条输出与测试输出均可入 Act Response；无不可低成本重跑项，无 Evidence 预算消耗。）

- Budget: 按公共规则 Evidence 预算执行（0 文件）。

**Risks and Notes**

- 空密钥守卫（`execute_command:175-177`）位于 match 分发之前——`RTSQL_KEY="" rtsql completions bash` 会 exit 2。任何已批准场景不涉及该组合；保持现序（守卫语义统一、改动最小），不为此加绕过分支。
- `clap_complete::Shell` 直接作参数类型会让 powershell/elvish 成功生成——三值收窄必须达成（value_parser 过滤或自定义枚举映射，机制留 Act；行为 powershell → exit 2 是场景级硬约束）。
- zsh 补全目录 D7 约定 `~/.zsh/completions/`（非 Oh My Zsh 路径），脚本输出 fpath 追加提示即可；fish `~/.config/fish/completions/` 为默认装载数路径。
- 冒烟中 `bash -c 'source <file>'` 仅验证脚本语法可装载，不依赖 bash-completion 运行时包。
- 本机（WSL2 Linux x86_64）cargo/strip 预期可用；strip 缺失走「跳过+提示」分支，冒烟仍通过。
- PATH 检测建议 `case ":$PATH:"` 形态或等价，避免引入 `which` 依赖；提示输出 stdout。
- 全量基线 1099 为 Iteration 000 修复后重跑采信结论；T9 计数不符时先对照本 Iteration Changed Files 复核归属，再判回归或基线漂移。

## Act Response

- Status: reported

**Implemented**

- T7：引入 `clap_complete`，新增隐藏的 `completions <bash|zsh|fish>` 子命令；通过 `CliArgs::command()` 生成脚本，生成路径不开库、不进入信号或数据库执行路径。
- T8：新增可执行 `install.sh`，覆盖 `cargo build --release`、strip、`--prefix`、当前 shell 补全安装、`--no-completions`、PATH 提示、程序卸载和显式 `--purge-data`。
- T9：完成 CLI 定向测试、脚本冒烟、全量测试、clippy、rustfmt、OpenSpec validate 与 change 结构自检；T7–T9 状态已同步为 `done`。

**Changed Files and Symbols**

- `Cargo.toml`、`Cargo.lock`：新增 `clap_complete = "4"`；Cargo 同步解析 `clap` 4.6.7、`clap_complete` 4.6.11 及 `syn` 3.0.6。
- `src/cli/mod.rs`：新增 `CommandFactory` 导入、`CompletionShell`、`Command::Completions`、`execute_command` 生成臂。
- `tests/cli_test.rs`：新增三 shell 生成、隐藏面、缺参和非法 shell 用例。
- `install.sh`：新增安装/补全/卸载脚本。
- `openspec/changes/2026-09-23-ms17-initial-release/tasks.md`：T7–T9 更新为 `done`。

**Deviations from Plan**

1. `clap_complete::generate` 在当前依赖版本返回 `()` 而非 `Result`；生成成功直接返回 `ExitStatus::Success`，契约中的 stdout 与退出码语义不变。
2. fish 补全脚本以 shell 原生形式输出 `-l key`，测试接受该等价表示；bash/zsh 仍断言 `--key`。
3. 首次临时 `HOME` 冒烟因 rustup 未找到默认工具链失败；显式继承真实 `CARGO_HOME`/`RUSTUP_HOME` 后同一冒烟通过。这是测试环境配置问题，不是实现偏差。
4. 新增依赖触发 Cargo 更新 clap 补丁版本及 `syn` 依赖锁定；全量测试和 clippy 均通过，未观察到既有行为回归。

**Blocker Handoff**

None

**Blocker Resolution**

None

**Self-Review**

- Spec compliance：T7 三 shell、隐藏面和 exit 2 场景均有测试；T8 安装、补全、PATH、`--no-completions`、程序卸载和数据清理均有冒烟见证；T9 四项验证与结构自检完成。
- Scope review：未修改数据库、生命周期、信号、密钥或退出码既有路径；`install.sh` 不联网、不使用 sudo、不触碰数据目录，除非显式 `--purge-data`。
- Code quality：Rust 代码通过 clippy 和 rustfmt；脚本通过 `bash -n`，参数错误路径返回 exit 2；`git diff --check` 无输出。
- Critical/Important findings：None。
- Minor findings：环境未安装 `shellcheck`，因此仅执行 `bash -n`；fish 的长选项文本差异已由测试覆盖。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T7 RED | `cargo test --test cli_test completions_` | `2 failed`，现状将 `completions bash` 当 SQL 解析 | 新增 completions 用例 | 预期 RED |
| T7 GREEN | `cargo test --test cli_test completions_` | `2 passed; 0 failed`，exit 0 | 三 shell、隐藏面、非法/缺参 | PASS |
| T7 回归 | `cargo test --test cli_test` | `91 passed; 0 failed; 2 ignored`，exit 0 | 既有 CLI + 新增用例 | PASS |
| T8 RED | `./install.sh --help` | `No such file or directory` | 安装脚本初始状态 | 预期 RED |
| T8 GREEN | 临时 `HOME`/`RTSQL_HOME`/`PREFIX` 冒烟 | `rtsql 0.1.0`、CRUD JSON 输出、最终 `install/uninstall smoke: PASS`，exit 0 | 编译安装、版本、CRUD、补全装载、PATH、两种卸载 | PASS |
| T8 边界 | `bash -n install.sh`；非法参数命令 | `bash_n=0 purge_without_uninstall=2 unknown_option=2 executable=yes` | 严格模式、参数错误、可执行位 | PASS |
| T9 全量 | `cargo test --no-fail-fast` | 78 suites；`1101 passed; 0 failed; 2 ignored`，exit 0 | 全部 Rust 测试套件 | PASS |
| T9 clippy | `cargo clippy --all-targets -- -D warnings` | `Finished`，0 warning，exit 0 | 全 targets 静态门 | PASS |
| T9 fmt | `cargo fmt --check` | 无 diff，exit 0 | Rust 格式门 | PASS |
| T9 validate | `openspec validate --specs` | `Totals: 37 passed, 0 failed`；仅既有 Purpose/长文本 warning | 主 specs | PASS |
| 结构自检 | 读取 tasks、Cycle 与目录清单 | T1–T9=`done`、T10–T12=`pending`；000/001 各有 `000-initial`，000 Review=`accepted`、001 Review=`pending` | change 结构与流程状态 | PASS |

**Persisted Evidence**

None required（验证输出可低成本重跑；未创建 `evidence/` 目录）。

**Experience Candidates**

None

**Remaining Issues**

None blocking。`shellcheck` 不在当前环境，未执行该项；`bash -n` 已通过。

**Commit or Diff Reference**

未 commit。当前 Cycle 变更见 `Cargo.toml`、`Cargo.lock`、`src/cli/mod.rs`、`tests/cli_test.rs`、`install.sh` 及 change `tasks.md`；工作区仍包含此前未提交的 MS13/MS17-T02 叠层，未回滚或覆盖。

## Plan Review

- Review Result: accepted

**Findings**

独立检查范围与方式：本 Review 全程只读。独立阅读本 Cycle 全部变更面——`install.sh` 全文（140 行）、`git diff` 的 `Cargo.toml`（+1 行 clap_complete）、`src/cli/mod.rs`（completions 变体 + 分发臂 + CompletionShell 映射）、`tests/cli_test.rs`（+42 行纯增量，2 个新用例）——并对照 delta specs（cli-noninteractive-shell「Shell 补全脚本生成」Requirement 2 场景、install-script R1 3 场景 / R2 2 场景）与 design D7 逐条核对；实测运行 `target/release/rtsql --help`（exit 0）独立确认隐藏面（completions 不出现在 Commands 列表）；核对 change tasks T7-T9 状态行与结构（T1-T9 done / T10-T12 pending、000/001 Cycle 文件齐全）。

T7 核验：`Command::Completions { shell: CompletionShell }` 带 `#[command(hide = true)]`（`src/cli/mod.rs` diff）；`CompletionShell` 自定义三值 ValueEnum（Bash/Zsh/Fish）映射 `clap_complete::Shell`——powershell 等未收窄值由 clap 用法错误出口 exit 2 承载（与契约「三值收窄为场景级硬约束」一致，D7 二选一中的自定义枚举路径）；分发臂经 `CliArgs::command()` + `clap_complete::generate(..., "rtsql", stdout)` 返回 `ExitStatus::Success`；生成臂不开库、不消费密钥、不触信号 future。测试 diff 纯增量（2 新用例覆盖契约 5 项见证点：三 shell 生成/非空/含 `rtsql` 与 `--key`〔fish 等价 `-l key`〕、缺参 exit 2、powershell exit 2、`--help` 隐藏面；「不开库」由空夹具环境成功隐含承载）。

T8 核验：`install.sh` 逐行对照 D7 与 install-script R1/R2——`set -euo pipefail`；`--purge-data` 须配 `--uninstall`（exit 2，冒烟 `purge_without_uninstall=2` 实证）；安装流 cargo 检查 → `cargo build --release`（于 SCRIPT_DIR）→ strip 缺失/失败仅警告不失败 → `install -m 0755` 至 `$PREFIX/bin`；completions 按 `$SHELL` + 命令存在性检测、内容来自 `$BIN_PATH completions <shell>` 同源生成；zsh fpath 提示；`--no-completions` 跳过；PATH 提示 case 语句双分支；`--uninstall` 仅删二进制 + 三补全文件（`rm -f` 存在才删）不触数据目录；`--purge-data` 先逐行列出（`will remove data directory: ...`）后 `rm -rf`，含无效数据目录守卫（`''|/|.|..` 拒绝）；`--prefix` 安装/卸载对称；无网络/sudo/交互；可执行位确认（`-rwxr-xr-x`）。8 步冒烟全轮记录采信（见下）。

T9 核验：全量 1101/0/2、clippy 0、fmt 0、validate 37 PASS、结构自检（T1-T9 done、000 Review accepted / 001 pending）——与变更面一致。

验证采信：T7 定向/全量 cli_test、T8 冒烟、T9 四项门均来源本 Cycle Act Response，覆盖表面（Cargo.toml/Cargo.lock、src/cli/mod.rs、tests/cli_test.rs、install.sh）经本 Review 只读核对自该次运行以来未变化，无重跑触发情形（公共规则 › 验证），予以采信，不重跑。本 Review 新增的 `--help` 隐藏面实测为补充独立检查（上述）。

非阻塞 Minor（记录不修复）：

- M1：install-script R1-S3 THEN 负向子句（`$PREFIX/bin` 已在 PATH 时不输出提示）未在冒烟中行使（T8 契约 8 步未含该分支）；脚本第 137-140 行 case 语句双分支直接可读，正向分支已经冒烟实证。非阻塞。
- M2：`RTSQL_KEY="" rtsql completions bash` → exit 2（空密钥守卫位于分发之前）——Plan Risks 已记录并裁定保持现序（DA2 统一守卫）；Iteration 002 文档不得将该组合记载为受支持用法。
- M3：`--purge-data` 无效数据目录守卫（第 72-77 行）位于 `remove_program` 之后——该失败路径上程序面已被移除但数据保留；守卫为契约外附加防御， Acceptance 场景未涉及。非阻塞。
- M4：fish 补全断言 `-l key` 等价形态（Deviation 2）——clap_complete fish 原生语法，bash/zsh 仍断言字面 `--key`，与 delta spec「含 rtsql 命令名与 --key 等全局参数」语义一致。

**Deviation Classification**

- Deviation 1（`clap_complete::generate` 返回 `()` 非 `Result`）→ ACT-DEVIATION（非实质 API 就近适配；生成成功即返回 `ExitStatus::Success`，stdout/退出码契约语义不变）。
- Deviation 2（fish `-l key` 等价断言）→ ACT-DEVIATION（非实质测试等价；生成器原生语法差异）。
- Deviation 3（冒烟首跑 rustup 默认工具链缺失，继承真实 CARGO_HOME/RUSTUP_HOME 后通过）→ BASELINE-CHANGED（测试环境事实，非实现偏差）。
- Deviation 4（新增依赖致 Cargo.lock 中 clap 4.6.7 / syn 3.0.6 锁定漂移）→ BASELINE-CHANGED（非实质依赖锁定漂移；全量 + clippy 已证零回归）。

**Acceptance Gaps**

None。T7 GREEN（新用例全绿 + 既有 89 零修改——diff 纯增量实证）、T8 冒烟 8 步、T9 四项门全部满足；RTM 行（cli completions Requirement → D7 → T7 → `src/cli/mod.rs` → cli_test 新用例；install-script R1/R2 → D7 → T8 → `install.sh` → 冒烟两模式轮）维持 Covered；Invariants（既有 CLI 零变化、退出码 0-5 映射、completions 不开库不消费密钥、脚本无网络/sudo/交互、数据目录仅 `--purge-data` 路径且先列后删）逐项核对未破坏。

**Convergence**

N/A（initial Cycle，无父 Cycle 缺口可比；本版 Review 首次即零 gap）。

**Evidence**

- 代码证据：`src/cli/mod.rs` diff（`CompletionShell` 三值枚举 + `hide = true` + generate 分发臂）；`install.sh:1-140`（严格模式/守卫顺序/卸载两模式/补全同源生成/PATH 双分支）；`tests/cli_test.rs` diff（+42 行纯增量，`completions_supported_shells` / `completions_hidden_and_usage_errors`）。
- 本 Review 实测：`target/release/rtsql --help` exit 0，Commands 列表不含 completions（隐藏面独立确认）。
- 文件系统证据：`install.sh` 可执行位 `-rwxr-xr-x`。
- 验证采信：T7 cli_test 91/0/2、T8 冒烟全轮、T9 全量 1101/0/2 + clippy/fmt/validate——来源本 Cycle Act Response，覆盖表面只读核对未变化。

**Follow-up Decision**

接受并完成当前 Iteration：T7/T8/T9 全部 Acceptance 满足，无阻塞 finding，四项 Deviation 均非实质且已分类；既有执行契约未暴露任何缺口。Iteration 001（install-surface）完成，按既有 Iteration Map 展开 Iteration 002（docs-closeout）。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

`iterations/002-docs-closeout/000-initial.md`（本次展开，Plan Context ready）
