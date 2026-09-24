# Iteration 000 / Cycle 000: RISC-V 64 musl 交叉构建产物初始执行

## Plan Context

- Status: ready
- Iteration: 000-riscv64-musl-artifacts
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: 1.1, 1.2, 1.3, 1.4, 2.1, 3.1, 3.2
- Depends on: None
- Stable baseline: 仓库提供固定 `riscv64gc-unknown-linux-musl` 的 locked 构建脚本；默认 `/dist/` 被忽略；脚本按版本生成 tar.gz + SHA256SUMS 且不执行目标二进制；双语文档与宿主机验证边界一致
- Verification boundary: `bash -n`、help/非法参数、当前缺失 musl target 的停止路径、`git diff --check`、strict OpenSpec validate；成功交叉构建与 RISC-V 运行测试按用户指令显式 SKIPPED
- Diagnostic boundary: 根 `build-riscv64-musl.sh`、`.gitignore`、`README.md`、`README.zh-CN.md` 与本 change Cycle
- Deferred tasks: None

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 全部承诺、riscv64-musl-build 四项 Requirement、design D1–D7、用户选择“riscv64gc-unknown-linux-musl + 仅产物 + 不运行目标测试”
- Excluded scope: Rust 产品代码/依赖/lock、已有 install.sh、SSH/目标机安装、QEMU/真实硬件、CI/Release、多架构矩阵

**Objective**

交付一个固定 RISC-V 64 Linux musl 目标的交叉构建脚本：参数与依赖失败可观察、locked release 构建路径明确、产物安全替换并版本化、双语文档准确，且实现过程绝不执行或解释 RISC-V 二进制。

**Background**

v0.1.0 已在本机安装并完成 release，但仓库只有面向宿主机的 `install.sh`。用户明确选择 RISC-V 64 musl 与“仅产物”，并要求脚本补充后不运行目标测试。规划调查发现默认 `dist/` 尚未被 `.gitignore` 忽略，因此把 `/dist/` 精确增量纳入同一必要范围，避免生成物污染源码状态。

**Investigation Facts**

- Current Baseline: `master` HEAD `145bba4`（v0.1.0 统一提交，1101 tests/clippy/fmt/validate 的已采信结论）；当前用户现场有未跟踪 `demo.sql`，必须保留且不得暂存/删除。新 change 规划文件已创建，Rust 产品代码无改动。
- Current-State Evidence（本会话新鲜读取/执行）：
  - `build-riscv64-musl.sh` 不存在；Gate 3 基线执行 `./build-riscv64-musl.sh --help` 返回非 0 且 stderr 为 `No such file or directory`，未触发 Cargo。
  - `rustup target list --installed` 包含 `riscv64gc-unknown-linux-gnu`、`riscv64gc-unknown-none-elf`，不包含固定 target `riscv64gc-unknown-linux-musl`；可用于验证精确匹配与缺失 target 停止路径。
  - `cargo pkgid` 输出 `path+file:///home/daivy/projects/RTsql#rtsql@0.1.0`，可从现有 package id 尾段派生版本；stderr 的用户级 cargo config 弃用提示不进入 stdout package id。
  - 当前工具可用：cargo/rustup 位于 `~/.cargo/bin`，GNU tar 1.34 与 coreutils sha256sum 8.32 位于 `/usr/bin`。
  - `.gitignore` 仅忽略 `/target` 等，没有 `/dist/`；默认产物目录必须新增精确 ignore。
  - `install.sh` 140 行已收口，模式为严格模式、参数先解析、路径安装、明确错误；新脚本复用其仓库根定位/错误输出风格，但不调用或修改 install.sh。
  - `README.md` 与 `README.zh-CN.md` 各 222 行，Quick start 从第 9 行开始、Known limitations 从第 202 行开始；当前无 RISC-V 构建章节。
- Code and Critical Path:
  - 新入口：脚本绝对路径 → 参数/help → 一次收集式工具与 target 检查 → `cargo pkgid` 版本 → repo root 内 locked cargo rustc → target release binary。
  - 产物流：创建 output root → 临时 staging → 复制 binary + 双语 README → tar.gz → SHA256SUMS → 成功后替换固定 target 子目录 → 绝对路径摘要与未做目标运行验证声明。
  - 错误流：usage error exit 2；工具/target/build/打包失败非 0；失败不发布成功摘要，旧目标子目录在成功替换前保持。
  - 文档流：双语 Quick start 记录 target 安装、默认/自定义输出、产物内容与目标机后续人工步骤；不承诺真实硬件验证。

**Implementation Guidance**

严格按 1.1 → 1.2 → 1.3 → 1.4 → 2.1 → 3.1/3.2。`--help` 与参数错误必须先于依赖检查。target 检查逐行精确相等，不能用子串；当前 musl 缺失时，GREEN 应在 Cargo build 前停止。产物先在 output root 内 staging，全部成功后才替换固定子目录；失败 trap 只清自己的临时目录。版本从 `cargo pkgid` 尾段读取，不硬编码。脚本只可复制/检查 target 文件，绝不可执行它。

**Behavioral Change**

当前仓库没有 RISC-V 构建入口。完成后新增固定 musl target 的可调用脚本与双语说明；成功路径会生成 `rtsql-v<version>-riscv64gc-unknown-linux-musl.tar.gz` 与 `SHA256SUMS`，但不修改系统安装目录、不上传远端、不执行目标二进制。默认 `/dist/` 被 Git 忽略。

**Task Contracts**

### 1.1: 固定参数入口与仓库根定位

- Requirement/Scenario: 固定目标入口 / help / 未知参数 / 仓库外调用
- Depends on: None
- Targets: `build-riscv64-musl.sh`
- Current behavior: 文件不存在
- Required behavior: 严格模式；可从任意 cwd 调用；仅支持 `--help`、`--output-dir DIR`；help 写 stdout/exit 0，缺值/未知参数写 stderr/exit 2；usage 写明固定 target、依赖和不执行目标二进制
- Required changes: 新建可执行脚本，解析参数后使用脚本目录作为 repo root；默认 output root 为 repo `dist/`，显式 relative output root 按调用者 cwd 解析
- Preserve: 不读取/依赖项目 `.cargo` 配置，不要求 bash 之外的语言运行器
- Forbidden: target override、自动安装、自动 SSH/deploy、交互 prompt
- Test witness: RED 已观察为脚本不存在；GREEN 为 `bash -n`、help exit 0、未知参数和缺值 exit 2
- GREEN condition: 语法/帮助/参数错误符合契约，脚本不进入构建
- Verification: 三条命令原生输出与退出码；失败表示参数入口未完成
- Stop when: 契约需要可变 target 或自动安装才能继续

### 1.2: 一次收集式前置检查

- Requirement/Scenario: target 已安装 / musl 缺失 / 工具缺失 / help 先于检查
- Depends on: 1.1
- Targets: `build-riscv64-musl.sh`
- Current behavior: 无 musl target 检查；本机只安装 GNU/bare-metal RISC-V targets
- Required behavior: 收集检查 bash/cargo/rustup/tar/sha256sum；再逐行精确检查 musl target。缺工具与缺 target 一次性报告，给出 `rustup target add riscv64gc-unknown-linux-musl`，非 0 停止；不运行 rustup target add/sudo
- Required changes: preflight 位于参数/help 后、build 前
- Preserve: Cargo 原生输出与失败退出码；不修改 Rust 工具链
- Forbidden: 子串匹配、自动下载、PATH 全局修改
- Test witness: 当前环境运行默认脚本路径应只报告精确 musl target 缺失，且输出不含 Cargo 编译开始信息；工具缺失分支由代码审查
- GREEN condition: 当前 GNU/bare-metal target 不误匹配，musl 缺失在 Cargo build 前停止
- Verification: 默认脚本调用 stderr 点名缺失 musl target，exit 非 0；help/unknown 仍走参数出口
- Stop when: 需要伪造已安装 target 或执行 build 才能判断 preflight

### 1.3: locked 构建与安全版本化产物

- Requirement/Scenario: target 已装成功路径 / 归档内容 / 自定义目录 / 重复构建保留无关文件
- Depends on: 1.2
- Targets: `build-riscv64-musl.sh`、`.gitignore`
- Current behavior: 无 musl 构建与产物；`/dist/` 未忽略
- Required behavior: `.gitignore` 精确新增 `/dist/`；repo root 内运行 `cargo rustc --locked --release --target riscv64gc-unknown-linux-musl -- -C strip=symbols`；从 cargo package id 派生版本；staging 收集 executable rtsql + 两 README，生成版本化 tar.gz 与 SHA256SUMS；成功后只替换固定 target 子目录，保留 output root 其他文件
- Required changes: 上述脚本构建/打包段与 ignore 增量
- Preserve: Cargo.toml/Cargo.lock 零变化；已有 install.sh 零变化；旧目标子目录在打包成功前不被删除
- Forbidden: 改 Cargo.lock、RUSTFLAGS 全局导出、硬编码版本、执行 target binary、引入 manifest/build-id 身份工程
- Test witness: 用户明确不执行成功交叉构建；以 Task Contract +完整 diff 代码审查见证命令、路径保护、staging/替换顺序；`git diff --check` 验证文本
- GREEN condition: 成功路径的每条命令、文件与错误边界均在脚本中闭合，/dist/ 被忽略，未执行产物
- Verification: 逐路径审查 + `git diff --check`；不得声称 tar/checksum 实际生成
- Stop when: 实现必须运行目标或改变 ABI 才能继续

### 1.4: 成功摘要与禁止执行边界

- Requirement/Scenario: 构建成功但未做目标机验证 / 宿主静态验证
- Depends on: 1.3
- Targets: `build-riscv64-musl.sh`
- Current behavior: 无摘要
- Required behavior: 成功后输出 binary/archive/checksum 绝对路径和精确声明 `cross-build succeeded; target execution not verified`；脚本不存在执行 target、QEMU、binfmt、SSH/SCP 的路径
- Required changes: 成功摘要与静态禁止路径
- Preserve: 所有非零失败不打印成功声明
- Forbidden: 以 checksum、ELF/file 元数据或交叉编译成功宣称 RISC-V 功能通过
- Test witness: 独立搜索脚本确认无执行/远端路径；成功摘要由代码审查
- GREEN condition: 声明区分 cross-build 与 hardware verification
- Verification: 脚本全文审查；无运行目标测试
- Stop when: 需要新增 QEMU/硬件执行才能满足摘要

### 2.1: 双语交叉构建说明

- Requirement/Scenario: 固定入口 / 版本化产物 / 自定义输出 / 宿主验证边界
- Depends on: 1.1, 1.4
- Targets: `README.md`、`README.zh-CN.md`
- Current behavior: 两文档只有宿主安装与限制，无 musl 交叉构建
- Required behavior: 同构章节覆盖 rustup target add、默认/自定义命令、tar/SHA256SUMS、目标机部署为后续人工步骤、未做 RISC-V 运行验证
- Required changes: Quick start 安装段附近精准插入章节
- Preserve: 现有 v0.1.0 用法、链接和能力清单
- Forbidden: 宣称已在 RISC-V 通过、引入 SSH 自动部署或硬编码测试/版本结果
- Test witness: 两文档人工逐条对照 help 与脚本；无 orphan link
- GREEN condition: 中英文命令、路径、警告一致且可复制
- Verification: 文本对照 + `git diff --check`
- Stop when: 文档需要承诺未实现的硬件验证

### 3.1: 宿主机静态门与显式跳过

- Requirement/Scenario: 宿主完成脚本验证 / 未做目标运行
- Depends on: 1.1, 1.2, 1.4, 2.1
- Targets: Cycle verification
- Current behavior: 脚本尚不存在
- Required behavior: 执行 bash -n、help/unknown、当前 musl 缺失停止路径、git diff check、strict validate；记录成功 cross-build 与 RISC-V execution 为 SKIPPED
- Required changes: 仅验证记录
- Preserve: 用户未跟踪 demo.sql
- Forbidden: 安装 musl target、运行 cargo cross build、运行 RISC-V binary、重复 Rust 全量测试
- Test witness: 命令原生退出码/输出
- GREEN condition: 所有宿主门通过；skip 原因明确且无伪成功声明
- Verification: 表中命令
- Stop when: 宿主门需要改变产品行为或安装全局 target

### 3.2: 完整 diff Review 与状态同步

- Requirement/Scenario: 全部 requirement 的范围保持
- Depends on: 1.1, 1.2, 1.3, 1.4, 2.1, 3.1
- Targets: 完整 worktree diff、change tasks/Cycle
- Current behavior: Act 待写
- Required behavior: 对照 Requirement/Task Contract 审查完整 diff；仅允许新脚本、.gitignore /dist/、双 README、change 文件；同步 tasks/Act Response
- Required changes: 状态与反馈写入
- Preserve: demo.sql 与所有范围外用户现场
- Forbidden: 暂存/删除 demo.sql、修改 Cargo/install.sh/产品代码、同步全局状态
- Test witness: git status/diff/name list
- GREEN condition: 无 Critical/Important，范围外零修改
- Verification: `git status --short`、`git diff --check`、变更文件审查
- Stop when: 发现计划外产品修改或需要新增需求

**Invariants**

- Rust 产品代码、Cargo.toml/Cargo.lock、数据库格式和已有 install.sh 零变化。
- `/dist/` 之外的用户文件不删除；用户未跟踪 demo.sql 原样保留。
- 脚本不执行 RISC-V 文件、不自动安装、不使用 sudo/SSH/QEMU。
- 成功产物路径固定为 `riscv64gc-unknown-linux-musl`，ABI 不漂移。
- 失败不产生成功摘要；构建/打包成功前不删除旧目标子目录。

**Non-goals**

- 实际交叉构建、目标硬件运行、真实功能/性能/恢复结论。
- target 选择器、RISC-V GNU/RV32/bare-metal、多架构矩阵。
- 自动部署、CI/Release/crates.io、签名、manifest 或验证工具。

**Acceptance**

- 1.1：help/usage/仓库根/固定 target 行为完整。
- 1.2：当前环境精确报告 musl 缺失，GNU/bare-metal 不误匹配，Cargo build 未启动。
- 1.3：`/dist/` 忽略；locked build、版本派生、staging、tar/checksum、安全替换路径完整，Cargo/lock/install.sh 零变化。
- 1.4：成功摘要声明未做 target execution，脚本无执行/远端路径。
- 2.1：双语章节与 help/script 一致。
- 3.1/3.2：宿主静态门、strict validate、完整 diff 通过；cross-build/硬件测试明确 SKIPPED。

**Verification**

- 参数：`bash -n build-riscv64-musl.sh`；`--help` exit 0；unknown/missing value exit 2。
- 缺失 target：默认调用 stderr 精确点名 musl target 并提示 rustup add，exit 非 0，未启动编译。
- 范围：`git diff --check`；status/name review 确认仅 4 个实现/文档文件 + change，且 demo.sql 未触碰。
- OpenSpec：`openspec validate 2026-09-24-riscv64-musl-build-artifacts --strict` exit 0。
- SKIPPED：成功 `cargo rustc`、tar/checksum 实际生成、RISC-V binary/CRUD/恢复运行；用户要求不运行且当前未安装 musl target。以上不声明 PASS。

**Gate 2 Readiness**

- Missing requirement: PASS（RTM 四项均 Covered）。
- Simplified requirement: PASS（无审批外裁剪；实际 cross-build/HW 测试是用户显式接受的验证 waiver，不是 requirement 简化）。
- Investigation: PASS（脚本缺失、target 状态、cargo package id、工具、gitignore、README、用户 demo.sql 均新鲜读取/执行）。
- Design: PASS（D1–D7 与备选/错误/产物路径闭合，Open Questions 无）。
- Tasks: PASS（七项均有目标、依赖、见证、GREEN、停止条件）。
- Iteration balance: PASS（单一脚本+文档+宿主门，独立成果，不拆分）。
- Traceability: PASS（spec scenario→D→task→surface→verification）。
- Verification: WAIVED（成功 cross-build 与 RISC-V 执行，用户明确“脚本补充之后不用运行测试”；风险为打包/目标运行未实证，已在 Acceptance/Verification/Skipped 明示）。其余宿主门 PASS。
- No identity evidence/judgment layer: PASS（SHA256SUM 是分发完整性目标行为，不用于证明测试/构建身份；无 manifest/build-id/run-id）。
- Unknowns: PASS（ABI、产物、部署、验证边界均由用户固定）。
- Consistency: PASS（strict validate 通过；tasks/spec/design/Iteration Map 一致）。
- Persisted Evidence: PASS（none；全部允许验证低成本，Act Response 足够）。
- Approval: PASS（用户审阅 proposal/spec/design/tasks 后于 2026-09-24 明确“开始实施”）。

**Persisted Evidence**

- Mode: none

所有宿主允许验证均可低成本重跑并写入 Act Response；用户明确不运行实际 cross-build/hardware verification，不创建证据占位目录。

- Budget: 0 文件。

**Risks and Notes**

- musl target 当前缺失：这是 preflight 负向见证，不得为 GREEN 擅自安装。
- 成功 cargo rustc/tar/checksum 路径只做代码审查，Act 必须明确“未执行”，不能使用假设补齐。
- 用户 `demo.sql` 是手工测试残留，范围外且未跟踪；不得删除、修改、暂存或纳入 diff Review 结论。
- GNU tar/coreutils 是已声明宿主依赖；不增加 macOS shasum 分支。
- shell 局部 helper/变量名与等价控制流留给 Act；不得改变错误优先级、路径保护或成功声明语义。

## Act Response

- Status: reported

**Implemented**

- T1.1：新增可执行 `build-riscv64-musl.sh`（156 行），严格模式、脚本目录即仓库根、固定 `riscv64gc-unknown-linux-musl`、`--help` / `--output-dir DIR`、缺值与未知参数 stderr + exit 2，help/参数错误先于依赖检查。
- T1.2：一次收集式 preflight 检查 bash/cargo/rustup/`riscv64-linux-musl-gcc`/tar/sha256sum；musl target 经 `rustup target list --installed` 逐行精确匹配（GNU/bare-metal 不误匹配）；缺失只报告并提示 `rustup target add` / 安装提示，非 0 停止，不自动安装、不动 PATH。
- T1.3：`.gitignore` 精确新增 `/dist/`；版本从 `cargo pkgid` 尾段派生（不硬编码）；以 `CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER=riscv64-linux-musl-gcc`（仅当前 Cargo 子进程）+ `cargo rustc --locked --release --bin rtsql --target riscv64gc-unknown-linux-musl -- -C strip=symbols -C target-feature=+crt-static` 构建；output root 内 mktemp staging 收集 binary + 双语 README，生成版本化 tar.gz + SHA256SUMS 后安全替换固定 target 子目录（backup/trap 保护，输出根其他文件保留）。
- T1.4：成功摘要输出 binary/archive/checksum 绝对路径及精确声明 `cross-build succeeded; target execution not verified`；脚本不存在执行目标文件、自动安装、sudo、SSH/SCP、QEMU 或 binfmt 路径；按用户审计指令移除脚本内 `sha256sum --check SHA256SUMS` 自引用验证层（身份工程），SHA256SUMS 作为 spec 要求的分发完整性产物保留，校验由使用方/Act 侧命令执行。
- T2.1：`README.md` / `README.zh-CN.md` 增加同构 RISC-V 64 musl 交叉构建章节：target 安装、默认/自定义输出、tar.gz/SHA256SUMS、宿主侧校验命令、linker 依赖与 `+crt-static`、目标机部署为后续人工步骤、未做硬件运行验证声明。
- T3.1/T3.2：宿主静态门、完整脚本实跑（含静态 ELF/tar 内容/checksum 校验）、strict OpenSpec validate、完整 diff Review 与任务状态同步完成；RISC-V 目标机运行按契约保持 SKIPPED（无硬件，脚本不执行目标）。

**Changed Files and Symbols**

- `build-riscv64-musl.sh`：新增；参数解析、preflight、版本派生、locked 静态构建、staging/tar/SHA256SUMS、安全发布与成功摘要。
- `.gitignore`：新增 `/dist/`（第 2 行）。
- `README.md`、`README.zh-CN.md`：新增 RISC-V 64 musl 交叉构建章节；另承载范围外工作流 hunk（见 Deviations 4）。
- `openspec/changes/2026-09-24-riscv64-musl-build-artifacts/`：proposal/spec/design/tasks、Iteration Map 与本 Cycle 文件。
- Rust 产品代码、`Cargo.toml`、`Cargo.lock`、`src/`、`tests/`、已有 `install.sh`：本 change 零修改。

**Deviations from Plan**

1. 实现把设计中的 `cargo rustc --locked --release --target ...` 收紧为显式 `--bin rtsql`：package 同时含 lib/bin，显式选择保证额外 rustc 参数作用于预期 binary target；不改变 ABI、产物或用户接口（Gate 4 局部实现修正）。
2. 按 Plan Review Follow-up Decision 增加 linker preflight、仅当前子进程的 `CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER` 注入与 `-C target-feature=+crt-static`；Plan 已同步修订 proposal/spec/design/tasks，README 依赖说明同步。修复宿主机 `/usr/bin/ld` 链接失败与动态 musl PIE 两个 Acceptance gap。
3. 按用户审计指令移除脚本内 `sha256sum --check SHA256SUMS` 自引用验证（本轮新增修复）：该步骤只证明 capture 工具自身输出一致性，sha256sum 写失败已由 `set -e` 覆盖，属公共规则禁止的二级验证/身份型证据层；移除后与 design D5（SHA256SUMS 仅为分发完整性产物）一致，spec「校验命令退出码 0」由 Act 侧/使用方命令满足。
4. 范围外工作流共居工作区（非本 change 内容，未触碰、未还原）：`install.sh` bashrc PATH 自动写入/卸载移除、`docs/SKILL.md` → `rtsql-docs/SKILL.md` 迁移、`.claude/docs/SNAPSHOT.md` 与 `tasks.md` 的 SKILL.md 路径勘误；README 两文件中对应 hunk（SKILL.md 链接更新 ×2、install.sh bashrc 描述 ×1）同属该工作流。提交切分由用户决定。Plan Context 记录的未跟踪 `demo.sql` 已不在工作区（用户自行处置）。

**Blocker Handoff**

None

**Blocker Resolution**

None

**Self-Review**

- Plan compliance: PASS。四个 Requirement 的参数、preflight、产物与宿主验证边界均映射到实现；用户 waiver 仅限 RISC-V 目标机运行，不被误写为 PASS；cross-build 已按 Follow-up Decision 实跑并取证。
- Spec review: PASS。固定 target、help/usage、错误优先级、精确 target 检查、linker preflight 与子进程注入、locked build、`+crt-static` 静态 ELF、版本归档、安全替换与禁止执行边界完整；`/dist/` 忽略为生成物不污染源码状态的必要后果；无 manifest/build-id/run-id/时间证明等身份机制（脚本内自引用 checksum 校验已移除）。
- Code quality review: PASS。失败前不删除旧输出；staging/backup trap 保护发布边界；`set -e` 覆盖 sha256sum 写失败；无范围外代码、无自动安装/远端执行。Gate 4 曾发现并修复 `--bin rtsql` target 选择问题。
- Full diff reviewed: PASS。逐行审查 156 行脚本、`.gitignore`、两份 README 差异与 change 文件；范围外工作流 hunk 已归因（Deviations 4），未纳入本 change 结论。
- Critical findings unresolved: 0
- Important findings unresolved: 0
- Minor findings unresolved: 1（RISC-V 目标机运行未验证，属用户明确接受的验证 waiver；非代码发现。）

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T1.1 语法/帮助 | `bash -n build-riscv64-musl.sh`；`./build-riscv64-musl.sh --help` | 语法无输出 exit 0；help 显示固定 target、依赖、不执行目标，exit 0 | 严格模式、参数入口、usage | PASS |
| T1.1 参数错误 | `./build-riscv64-musl.sh --unknown`；`./build-riscv64-musl.sh --output-dir` | 均 exit 2；stderr 分别含 unknown option / requires a directory | 非法参数与缺值 | PASS |
| T1.1 仓库外入口 | 从 `/tmp/opencode` 以绝对路径执行 `--help` | exit 0，无构建输出 | repo-root 定位与 help 复用 | PASS |
| T1.2 缺失 target | `./build-riscv64-musl.sh`（musl target 未装时观察，前次 Response 记录） | exit 1；精确点名缺失 `riscv64gc-unknown-linux-musl` 并提示 `rustup target add`；无 Cargo Compiling | GNU/bare-metal 不误匹配、Cargo 前停止；该代码路径（逐行精确匹配）本轮未改动，结论沿用 | PASS |
| T1.2/T1.3 preflight 正路径 | `./build-riscv64-musl.sh`（本轮完整实跑） | preflight 通过（含 `riscv64-linux-musl-gcc`），进入构建 | 工具收集检查 + target 精确匹配 | PASS |
| 完整交叉构建 | `./build-riscv64-musl.sh` | exit 0；`Finished release profile`；摘要输出 binary/archive/checksum 绝对路径 + `cross-build succeeded; target execution not verified` | preflight→locked 构建→staging→tar→SHA256SUMS→安全替换→摘要 全链路 | PASS |
| 静态 RISC-V ELF | `file dist/riscv64gc-unknown-linux-musl/rtsql` | `ELF 64-bit LSB executable, UCB RISC-V, RVC, double-float ABI, statically linked, stripped` | 目标产物只读元数据（未执行） | PASS |
| 归档内容 | `tar -tzf dist/.../rtsql-v0.1.0-riscv64gc-unknown-linux-musl.tar.gz` | `rtsql` / `README.md` / `README.zh-CN.md`（恰三项） | 归档顶层内容 | PASS |
| 校验和（Act 侧） | `(cd dist/riscv64gc-unknown-linux-musl && sha256sum --check SHA256SUMS)` | `rtsql-v0.1.0-riscv64gc-unknown-linux-musl.tar.gz: OK`，exit 0 | 分发完整性（spec「校验命令退出码 0」） | PASS |
| 身份工程移除 | `grep -nE 'sudo|ssh|scp|qemu|binfmt|sha256sum --check' build-riscv64-musl.sh` | 无匹配，exit 1 | 脚本全文禁止面 | PASS |
| dist ignore | `git check-ignore -v dist/riscv64gc-unknown-linux-musl/rtsql` | `.gitignore:2:/dist/` | 默认生成物不污染 Git 状态 | PASS |
| T3.1 差异 | `git diff --check` | 无输出，exit 0 | 全部 tracked 文本差异 | PASS |
| T3.1 OpenSpec | `openspec validate 2026-09-24-riscv64-musl-build-artifacts --strict` | `Change '2026-09-24-riscv64-musl-build-artifacts' is valid`，exit 0 | proposal/spec/design/tasks/Iteration | PASS |
| T3.2 范围 | `git status --short` + `git diff --stat -- Cargo.toml Cargo.lock src/ tests/` + 完整 diff Review | 产品面 diff 为空；仅 `.gitignore`、双 README、新脚本/change 属本 change；范围外工作流 hunk 已归因 | 计划范围与用户现场保护 | PASS |
| RISC-V 目标机运行 | 未执行 | `SKIPPED: 无 RISC-V 硬件；契约禁止脚本执行目标二进制` | binary/CRUD/加密/恢复 | 不声明 PASS |

**Persisted Evidence**

None required（Mode: none）。全部宿主验证可低成本重跑并由本 Response 承载；未创建 `evidence/`。

**Experience Candidates**

- Runbook 候选：RISC-V musl 交叉构建与产物校验流程——前置检查（Rust target + `riscv64-linux-musl-gcc`）、per-subprocess `CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER` 注入（musl target 默认落宿主 `cc`/`/usr/bin/ld` 报 `EM: 243` 的实证坑）、`+crt-static` 静态化、staging/安全替换与宿主侧 `sha256sum --check`。证据：本 Response Verification Evidence 完整交叉构建行 + Plan Review Evidence 段；端到端成功两次（16:04、16:17）。候选登记与否由 Recorder 按用户指令执行。

**Remaining Issues**

- 当前未在真实 RISC-V 硬件执行版本/CRUD/加密/恢复冒烟；产物已在宿主生成并校验（静态 ELF、tar 三项、SHA256SUMS OK），目标机验证需在具备硬件的环境另行进行。
- 范围外工作流（install.sh bashrc PATH、SKILL.md 迁移、SNAPSHOT/tasks 路径勘误）未提交且与本 change 共居 README 文件；提交切分由用户决定。
- 无阻塞项。

**Commit or Diff Reference**

未 commit。本 change 差异为 `build-riscv64-musl.sh`、`.gitignore`、两份 README（RISC-V 章节）与本 change 目录；对照基线 `145bba4`。`dist/` 产物由最终脚本于 16:17 重新生成（Git 已忽略）。

## Plan Review

- Review Result: accepted

**Findings**

1. Follow-up Decision 落实核实（独立读码）：`build-riscv64-musl.sh:7/19/53` 将 `riscv64-linux-musl-gcc` 纳入 usage 依赖声明与一次收集式 preflight；`:101` 仅对当前 Cargo 子进程注入 `CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER` 并传入 `-C strip=symbols -C target-feature=+crt-static`；design D2/D3 与 delta spec R1/R2 已同步收录。上轮 2 个 Acceptance gap 的决定性证据本轮独立复核：`file dist/riscv64gc-unknown-linux-musl/rtsql` → 静态 RISC-V ELF（无动态解释器）；`tar -tzf` 顶层恰三项；产物 mtime（16:17）晚于脚本 mtime（16:16:37），当前产物由当前脚本版本生成。
2. 身份工程移除核实：脚本全文无 `sha256sum --check` 自引用校验，无 sudo/ssh/scp/qemu/binfmt/自动安装路径；spec「校验命令退出码 0」由使用方/Act 侧命令满足（本轮独立复核 exit 0），符合公共规则 › 行为约束；design D5 明确 SHA256SUMS 仅为分发完整性产物。
3. 范围保持核实：`git diff --stat` 显示 Cargo 产品面（Cargo.toml/Cargo.lock/src/tests）零修改；`.gitignore` 恰 1 行 `/dist/`；README 增量为 RISC-V 章节 + 归因于范围外工作流的既有 hunk（SKILL.md 链接 ×2、install.sh bashrc 描述与 Quick start source ×1），Act 未触碰未还原，符合 Task 3.2 Preserve/Forbidden；install.sh/docs-SKILL.md 迁移/SNAPSHOT-tasks 勘误属范围外工作流，与 Act Response Deviations 4 归因一致；`demo.sql` 已不在工作区（用户自行处置，Act 已报告）。tracked diff 自 Act 验证结论以来未变化（README 16:04 / 脚本 16:16:37 均早于 16:17 产物），宿主门结论采信。
4. 双语 README 与当前脚本行为一致（独立文本对照）：固定 target、`--output-dir`、默认 `dist/` 路径、`riscv64-linux-musl-gcc`、GNU tar、`sha256sum`、`+crt-static`、子进程级 linker 注入、不执行目标声明、目标机人工部署与「未做硬件运行验证」声明，中英同构。
5. Minor（非阻塞）：发布成功与 trap 移除之间的极窄失败窗内（`mv` 成功后 `rm -rf` backup 失败），输出根可能残留隐藏 `.riscv64gc-unknown-linux-musl.backup.*` 旧产物目录；不涉及任何 Acceptance 场景，「失败不发布成功摘要」与路径保护语义不受影响，不要求修复。
6. Minor（非阻塞）：spec R2 工具清单未列 bash，脚本额外检查 bash——超集无害（脚本自身即 bash 运行），不要求修复。
7. 采信与补跑：Act 已报告且覆盖面未变的宿主门（bash -n/help/参数错误/`git diff --check`）直接采信；T3.1 的 strict validate 因 T3.2 tasks/cycle 状态同步改变了其覆盖面（change 目录文件），按新鲜度规则本轮补跑 `openspec validate 2026-09-24-riscv64-musl-build-artifacts --strict` → `valid`，exit 0。

**Deviation Classification**

- ACT-DEVIATION（Deviation 1，`--bin rtsql`）：已由首轮 Review 吸纳进 plan/spec/design（D3），与当前计划一致，不再构成偏差。
- ACT-DEVIATION（Deviation 3，经用户审计指令授权）：移除脚本内 checksum 自引用验证层，符合公共规则身份工程禁令；spec 校验语义由使用方命令满足并经独立复核。
- 上轮两项 PLAN-OMISSION（linker preflight / `+crt-static`）：修复已按 Follow-up Decision 落实并有决定性证据，计划产物已同步。
- 范围外工作流共居（Deviation 4）：非本 change 偏差，现场归因清楚；提交切分由用户决定。

**Acceptance Gaps**

None（上轮 2 项 gap——成功构建路径不可达、动态 musl PIE——均已闭合：完整脚本两次成功运行（16:04、16:17）、静态 ELF 元数据、tar/checksum 独立复核。）

**Convergence**

closed（上轮 expanded 2 gap → 本轮 0 gap。）

**Evidence**

- `file dist/riscv64gc-unknown-linux-musl/rtsql` → `ELF 64-bit LSB executable, UCB RISC-V, RVC, double-float ABI, statically linked, stripped`（本轮独立复核）。
- `tar -tzf dist/riscv64gc-unknown-linux-musl/rtsql-v0.1.0-riscv64gc-unknown-linux-musl.tar.gz` → `rtsql` / `README.md` / `README.zh-CN.md` 恰三项（本轮独立复核）。
- `(cd dist/riscv64gc-unknown-linux-musl && sha256sum --check SHA256SUMS)` → `rtsql-v0.1.0-...tar.gz: OK`，exit 0（本轮独立复核）。
- `openspec validate 2026-09-24-riscv64-musl-build-artifacts --strict` → `valid`，exit 0（本轮补跑）。
- `git status --short` / `git diff --stat` → 仅本 change 4 文件 + 范围外工作流 hunk（已归因）；产品面零修改。
- 脚本全文独立读码（156 行）+ mtime 基线：script 16:16:37 < dist 16:17。
- 既有采信：Act Response Verification Evidence 表（T1.1 参数面、T1.2 缺失 target 停止路径、preflight 正路径、`git check-ignore`、`git diff --check`）。

**Follow-up Decision**

None

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None
