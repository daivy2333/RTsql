# Iteration 000 / Cycle 000-initial: 版本 0.2 定义与本地部署升级

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 用户指令（2026-09-30）：「把当前版本的数据库定义成 0.2 版，更改对应信息，提交同步，然后把本地编译部署的升级成这个版本」；三项场景缺口裁定（基准节历史标注保留 / riscv 不重跑 / 不加 What's new）。
- Excluded scope: riscv64 交叉构建重跑与 dist 发布；CHANGELOG/What's new；git push；任何功能面；`--uninstall` 路径（升级不依赖卸载）。

**Objective**

版本三处同源落 0.2.0（`Cargo.toml` → `rtsql --version` / `cargo pkgid` 自动跟随 + SKILL frontmatter），本机部署经 install.sh 原地重装升级至 0.2.0 且 0.1 库无迁移可开；升级路径行为进语料（install-script +R3，收尾合并）；实现随收尾提交同步。

**Investigation Facts**

- Current Baseline: HEAD 912df7d 工作区干净，全量 1590 tests / 0 failures（2026-09-30 MS26 收尾批次）；本机 `~/.local/bin/rtsql` → `rtsql 0.1.0`。
- Current-State Evidence（Plan 本会话实读）:
  - `Cargo.toml:3` `version = "0.1.0"` 单一版本源；`rtsql --version` 经 clap 自动输出（本机实跑 `rtsql 0.1.0`）；`build-riscv64-musl.sh:90-98` `cargo pkgid` 运行时派生归档名（零硬编码，本 change 零改动）。
  - 交叉脚本 `:101` `cargo rustc --locked`——Cargo.lock 需与 Cargo.toml 同步（任一 cargo 命令再生）；`install.sh:144` 裸 `cargo build --release` 无 `--locked`。
  - `rtsql-docs/SKILL.md:7` frontmatter `version: "0.1.0"`（唯一文档编辑点）；双 README 硬编码字样仅基准溯源段两处（`README.md:302`/`README.zh-CN.md:302`，裁定保留）；命令示例无期望输出版本。
  - `grep -rn '0\.1\.0' tests/ src/` 零命中——无测试版本断言，见证走命令输出。
  - `install.sh`：默认 `PREFIX=~/.local`；`:144` 构建 → `:149-158` strip → `:163-164` `install -m 0755` 覆盖安装 → 补全再生 → PATH 提示；数据目录零操作。
  - 语料 `install-script` R1 仅全新安装（临时 PREFIX）；升级重装无承载 → 本 change delta spec ADDED R3。
- Code and Critical Path: T1 变更面 = `Cargo.toml`（version 字段一行）+ `Cargo.lock`（cargo 自动再生）+ `SKILL.md:7`（frontmatter 一行）；T2 = 本机操作（install.sh 实跑 + 临时 RTSQL_HOME 冒烟），零代码变更。验证链：`cargo pkgid` → 0.2.0；构建后 `./target/release/rtsql --version` → 0.2.0；升级后 `~/.local/bin/rtsql --version` → 0.2.0；0.1 库（升级前以 0.1.0 二进制在临时 RTSQL_HOME 建库写数）打开查询写回；连续两次 install.sh 幂等。

**Implementation Guidance**

- 执行顺序：T1 先行（改版本 → `cargo build --release` 完成 Cargo.lock 再生并产出 0.2.0 二进制），T2 依赖 T1 的构建产物。
- T2 取证序：升级前以**当前 0.1.0 安装态**在临时 `RTSQL_HOME` 建库写入标记行 → 跑 `./install.sh` → `--version` 断言 → 旧库打开查询（标记行在）+ 新 INSERT → 二次 `./install.sh` 幂等取证（退出码 0 + 补全文件无重复）。
- README 核验：`grep -rn '0\.1\.0' README.md README.zh-CN.md` 仅命中两处基准溯源段（裁定白名单），其余零命中——结论一行入 Act Response。
- 用户 `~/.rtsql/` 数据目录：升级不触碰（脚本无数据面）；不作硬断言（避免并行进程干扰误报），必要时旁观核验。

**Task Contracts**

### T1: 版本定义落点（Cargo.toml + SKILL frontmatter + 核验）

- Requirement/Scenario: install-script R3「版本三处同源」场景
- Depends on: None
- Targets: `Cargo.toml`（`version` 字段 → `"0.2.0"`）、`Cargo.lock`（`cargo build` 自动再生）、`rtsql-docs/SKILL.md:7`（frontmatter `version` → `"0.2.0"`）、双 README（只读核验）
- Current behavior: 三处 0.1.0；`rtsql --version` 报 `rtsql 0.1.0`；dist 归档命名派生 0.1.0。
- Required behavior: ① `Cargo.toml` version `0.2.0`；② `cargo pkgid` 输出含 `0.2.0`；③ `cargo build --release` 后 `./target/release/rtsql --version` 输出 `rtsql 0.2.0`；④ `Cargo.lock` 中 rtsql 包版本 0.2.0（`--locked` 交叉构建前置就绪）；⑤ SKILL frontmatter `0.2.0`；⑥ README grep 仅命中基准溯源两处（白名单）。
- Preserve: `--version` 输出格式（clap 默认形态）不变；Cargo.toml 其余字段零改动；SKILL.md 除 frontmatter version 行外零改动；双 README 零编辑（裁定 1/3）；`build-riscv64-musl.sh` 零改动。
- Forbidden: 不新增 CHANGELOG/What's new；不改 `--version` 接线（clap 默认已同源）；不重跑交叉构建；不动版本之外的任何字段或小节。
- Test witness: 无既有测试断言版本（调查事实 4）——见证 = 命令输出：`cargo pkgid`、`--version`、`grep -n 'version' Cargo.lock`（rtsql 条目）、`grep -n '^version' SKILL.md 或 frontmatter 定位`、`grep -rn '0\.1\.0' README.md README.zh-CN.md`（白名单核验）。
- GREEN condition: 上述六项输出全部符合 Required behavior。
- Verification: `cargo pkgid`（退出码 0，含 0.2.0）；`cargo build --release`（退出码 0）后 `./target/release/rtsql --version`（`rtsql 0.2.0`）；grep 三处各一行结论。
- Stop when: bump 后任何下游（`--version`/pkgid/lock）未跟随——版本单一源接线断链属实质问题；README grep 出白名单外的当前版本字样（返回 Plan 重议范围）。

### T2: 本地部署升级与升级语义见证

- Requirement/Scenario: install-script R3 全三场景（0.1 升级保数据 / 版本三处同源落地态 / 重跑幂等）
- Depends on: T1
- Targets: 本机操作——`./install.sh`（默认 prefix）、临时 `RTSQL_HOME` 冒烟库、`~/.local/bin/rtsql` 与补全文件的升级后状态；零代码变更
- Current behavior: `~/.local/bin/rtsql` 为 0.1.0；升级路径无语料承载。
- Required behavior: ① 升级前取证：`rtsql --version` → `rtsql 0.1.0`，临时 `RTSQL_HOME`（0.1.0 安装态二进制）建库写标记行；② `./install.sh` 退出码 0；③ 升级后 `rtsql --version` → `rtsql 0.2.0`；④ 升级前建的 0.1 库可开：标记行可查 + 新 INSERT 成功（无迁移）；⑤ 补全文件存在且内容含 rtsql；⑥ 连续第二次 `./install.sh` 退出码 0、无重复条目/残留备份；⑦ 全量测试零回归兜底。
- Preserve: install.sh 脚本本体零改动（升级走既有路径，正是 R3 的行为语义）；用户 `~/.rtsql/` 数据目录不触碰；`--purge-data` 全程不出现。
- Forbidden: 不先卸载（`--uninstall` 非升级前置）；不改 install.sh；不在用户真实数据目录上做任何写操作；不重跑 riscv 交叉构建。
- Test witness: 全程命令输出（升级前后 `--version`、install.sh 退出码、旧库 SELECT/INSERT 结果、补全文件 grep、二次重跑退出码）；全量 `cargo test` 0 failed。
- GREEN condition: ①–⑦ 全部符合。
- Verification: 各步命令 + 决定性输出 + 退出码入 Act Response；全量 `cargo test -- --test-threads=1` 单批全量（版本字段无行为面，M19 分批不必要——结论注明执行方式）。
- Stop when: 升级后 0.1 库打不开或数据缺失（0.x 兼容声明失实，实质问题）；install.sh 升级路径失败且原因非环境性；用户数据目录出现任何变更痕迹。

**Invariants**

- 版本单一源不破：Cargo.toml 是唯一版本编辑点，三下游自动跟随。
- 升级零数据面：install.sh 重装不触碰数据目录；冒烟全部在临时 RTSQL_HOME。
- 文档裁定稳态：README 基准节 0.1.0 历史标注为白名单长期形态。
- 行为零变化：版本字段与 frontmatter 之外，产品代码、脚本、测试零改动。

**Non-goals**

riscv dist 重跑与发布；CHANGELOG/What's new；git push；功能面变更；`--version` 输出格式；`--uninstall` 路径。

**Acceptance**

- install-script R3 场景 1（升级保数据）：T2 ①–④。
- R3 场景 2（版本三处同源）：T1 ①–④ + 归档命名派生面确认（`build-riscv64-musl.sh` 派生逻辑零改动即跟随）。
- R3 场景 3（重跑幂等）：T2 ⑤–⑥。
- 文档同步：SKILL frontmatter 0.2.0 + README 白名单核验（T1 ⑤⑥）。

**Verification**

- R3 场景映射的命令序列（T1 六项 + T2 七项）全部以命令 + 决定性输出 + 退出码判定；全量 `cargo test -- --test-threads=1` 单批 0 failed 兜底。
- 收尾提交同步（用户指令）：实现 + 文档 + 语料合并随收尾批次入库（不含 push）。

**Gate 2 Readiness**

PASS——2026-09-30 用户批准计划（需求/范围 Gate 1 与计划 Gate 2 经 AskUserQuestion 明确批准；三项场景缺口裁定与四项默认假设记录于 proposal）。调查完整（版本单一源接线、Cargo.lock 联动、文档字样分布、安装面机制、语料缺口全部实读定位）；设计闭合（D1 单一源 / D2 原地重装 / D3 裁定归档）；RTM Covered 无 Missing；Persisted Evidence none。无 BLOCKED/WAIVED 项。

**Persisted Evidence**

- Mode: none

（全部见证为命令输出与退出码，Act Response 承载；升级操作是一次性本机操作，结果可由 `--version` 与临时库状态低成本复核，无独立 Evidence 必要。）

**Risks and Notes**

- install.sh 运行会按 Cargo.toml 拉取依赖（既有行为）；本机工具链 Rust 版本需 ≥ 既有基线（MS26 批次已同链构建成功，无新风险）。
- `rtsql --version` 的 PATH 解析：升级验证用 `~/.local/bin/rtsql --version` 显式路径，避免 shell hash 缓存指向旧二进制的假象（bash `hash -r` 或绝对路径）。
- 0.1 库兼容的预期支撑：MS19–MS26 catalog 尾段（unique/DEFAULT/secondary/kind）全部「旧行读为空」向后兼容设计，页格式未变；若实证失败按 Stop when 返回 Plan，不现场修补。
- 用户数据目录旁观核验（可选）：升级前后 `ls -la ~/.rtsql/` 快照对比，仅记录不作硬断言。

## Act Response

- Status: reported
- Cycle: `000-initial`（initial）| 实施日期：2026-09-30 | 基线：HEAD 485a020

**Implemented**

- **T1 版本定义落点**（3 文件 3 行，`git diff --stat`：3 files changed, 3 insertions(+), 3 deletions(-)）：
  - `Cargo.toml:3` `version = "0.1.0"` → `"0.2.0"`（唯一版本编辑点，design D1 单一源）。
  - `Cargo.lock:1120` rtsql 包版本随 `cargo build --release` 自动再生为 `0.2.0`（交叉脚本 `build-riscv64-musl.sh:101` `--locked` 的前置就绪）。
  - `rtsql-docs/SKILL.md:7` frontmatter `version: "0.1.0"` → `"0.2.0"`（唯一文档编辑点）。
  - 双 README 零编辑（裁定 1/3）：`grep -n '0\.1\.0' README.md README.zh-CN.md` 仅命中 `README.md:302` / `README.zh-CN.md:302` 基准溯源段各 1 处，与裁定白名单一致。
  - `build-riscv64-musl.sh` 零改动，归档名经 `cargo pkgid` 派生自动跟随（裁定 2）。
- **T2 本地部署升级**（零代码变更，纯本机操作）：
  - 升级前取证：安装态 `~/.local/bin/rtsql --version` → `rtsql 0.1.0`；以 0.1.0 二进制在临时 `RTSQL_HOME=/tmp/rtsql-smoke-WvHcaX` 建库 `smokebase` 并写入标记行（`{"affected_rows":1}`），文件头 `RTSQLDB.`（64B 头确认）、伴生 `.wal`/`.checkpoint` 齐备。
  - `./install.sh` 默认路径原地重装 → **退出码 0**（未卸载、未用 `--purge-data`、脚本本体零改动）。
  - 升级后 `~/.local/bin/rtsql --version` → `rtsql 0.2.0`（`~/.local/bin/rtsql` 7706104 字节，20:43 新落盘）。
  - 0.1 库免迁移可开：旧标记行可查（`rows":[[1,"written-by-0.1.0"]]`）、0.2 写入 0.1 库成功、合并读回两行（0.1.0 + 0.2.0）；数据目录三文件仍在，无删除/迁移痕迹。
  - bash 补全再生（`~/.local/share/bash-completion/completions/rtsql`，20249 字节 / 602 行）。
  - 幂等：第二次 `./install.sh` 退出码 0；`cmp` 证实二进制与补全文件两次安装**逐字节一致**；`~/.local/bin/` 无 `.bak`/`.old`/临时残留。
  - 用户真实数据目录 `~/.rtsql/` 全程未触碰（冒烟全在临时 `RTSQL_HOME`）。

**Deviations from Plan**

- 无实质偏差。三项非实质事项记录：
  1. **全量测试按单批执行**（契约 T2 Verification 明文授权：「版本字段无行为面，M19 分批不必要——结论注明执行方式」），未走 M19 分批。
  2. **验证命令的输出捕获范围**：全量测试命令的 `grep + tail` 组合截断了前半段 `test result` 行，故本 Response 不引用总用例数（基线 1590 未在本轮重新计数）；判定依据为 `cargo test` 自身退出码 0，非截断输出。
  3. **补全覆盖面为单 shell**：契约 ⑤ 表述为「补全文件存在且内容含 rtsql」，实际 `install.sh:170-187` 的 bash/zsh/fish 三分支以 `command -v` 为条件，本机 zsh/fish 未安装 → 仅 bash 补全落地。属既有条件安装行为，非本 change 引入，契约未要求三 shell 齐备。

**Blocker Handoff**

None required —— 未命中 Gate 5。

**Self-Review**

Gate 3（T1 / T2 各一次）通过，**无未解决的 Critical 或 Important 问题**。

- **Spec compliance**：`install-script` R3 三场景全覆盖——场景 1（0.1 安装态原地升级保数据）= T2 ①–④ 全绿；场景 2（版本三处同源）= T1 ①–④ 全绿 + 归档命名派生面确认（交叉脚本零改动、随 `pkgid` 派生）；场景 3（重跑幂等）= T2 ⑤–⑥ 全绿。Preserve 项全部守住：`--version` 输出格式、Cargo.toml 其余字段、SKILL.md 除 frontmatter 一行、双 README、install.sh 本体、交叉脚本均零改动；不变量「版本单一源不破」「升级零数据面」成立。
- **Code quality**：diff 为 3 行版本字面量，无计划外修改、无新增警告/死代码/重复实现/无依据复杂度、无身份型证据机制或判定层。T2 零代码变更。
- **自检发现（均判定为非缺陷，记录留档）**：
  1. 补全文件 `sort | uniq -d` 报重复行 → 逐条查看为 bash `case` 脚本固有结构行（`;;`、`return 0`、`COMPREPLY=...`、缩进空白），非安装叠加产生的重复条目。决定性证据：两次安装 `cmp` 逐字节一致、602 行无增长。R3「不产生重复条目」要求满足。
  2. `install.sh` 零 `echo`（`grep -c 'echo'` = 0）→ 安装过程静默属既有设计；不影响判定，安装效果已用安装点文件状态（版本、字节数、时间戳、补全文件）独立验证。

**Verification Evidence**

| # | 命令 | 决定性输出 | 退出码 |
|---|---|---|---|
| 1 | `cargo pkgid`（bump 前，见证基线） | `path+file:///home/daivy/projects/RTsql#rtsql@0.1.0` | 0 |
| 2 | `cargo build --release` | `Compiling rtsql v0.2.0` / `Finished \`release\` profile ... in 1m 24s` | 0 |
| 3 | `cargo pkgid`（bump 后） | `path+file:///home/daivy/projects/RTsql#rtsql@0.2.0` | 0 |
| 4 | `./target/release/rtsql --version` | `rtsql 0.2.0` | 0 |
| 5 | `grep -A1 '^name = "rtsql"' Cargo.lock` | `name = "rtsql"` / `version = "0.2.0"` | 0 |
| 6 | `grep -n 'version: "0' rtsql-docs/SKILL.md` | `7:  version: "0.2.0"` | 0 |
| 7 | `grep -n '0\.1\.0' README.md README.zh-CN.md` | 仅 `README.md:302` / `README.zh-CN.md:302` 基准溯源段（白名单，各 1 处） | 0 |
| 8 | `git diff --stat` | `Cargo.lock`/`Cargo.toml`/`rtsql-docs/SKILL.md` 各 1 行，3 insertions / 3 deletions | 0 |
| 9 | `~/.local/bin/rtsql --version`（升级前） | `rtsql 0.1.0` | 0 |
| 10 | 0.1.0 建库写标记行（临时 `RTSQL_HOME`） | `{"affected_rows":0}` → `{"affected_rows":1}`；文件头 `RTSQLDB.`；`.db`/`.wal`/`.checkpoint` 齐备 | 0 |
| 11 | `./install.sh`（第一次） | `Finished \`release\` profile ... in 0.09s` | **0** |
| 12 | `~/.local/bin/rtsql --version`（升级后） | `rtsql 0.2.0` | 0 |
| 13 | 0.1 库 SELECT（0.2 打开） | `{"columns":["id","note"],"rows":[[1,"written-by-0.1.0"]]}` | 0 |
| 14 | 0.1 库 INSERT（0.2 写入） | `{"affected_rows":1}` | 0 |
| 15 | 合并读回 + 目录检查 | `rows":[[1,"written-by-0.1.0"],[2,"written-by-0.2.0"]]`；目录仍为 `.db`/`.wal`/`.checkpoint` 三文件 | 0 |
| 16 | 补全文件核验 | `~/.local/share/bash-completion/completions/rtsql` 20249 字节 / 602 行，内容含 rtsql 子命令 | 0 |
| 17 | `./install.sh`（第二次，幂等） | `Finished \`release\` profile ... in 0.08s` | **0** |
| 18 | `cmp` 两次安装产物 | 二进制逐字节一致；补全逐字节一致；无 `.bak`/`.old`/临时残留 | 0 |
| 19 | `cargo test -- --test-threads=1`（单批全量） | 输出中各 `test result: ok. ... 0 failed`（本命令 grep+tail 截断前半段行，故不引用总用例数） | **0** |
| 20 | `openspec validate 2026-09-30-version-0-2-bump-and-deploy --strict` | `Change '...' is valid` | **0** |
| 21 | `openspec validate install-script --type spec --strict` | `Specification 'install-script' is valid`（附 INFO：R3 文本 >500 字符建议拆分，不构成失败） | **0** |
| 22 | `openspec validate --all --strict` | `46 passed, 14 failed (60 items)` —— 14 项全部为 MS06–MS10 时代 spec 的 Purpose 占位（`storage-io-optimization` / `wal-recovery-frame-parsing` / `wal-recovery-replay-integrity` / `wal-writer-handle-reuse` 等），SNAPSHOT「OpenSpec」段已记录为既有状态，非本轮引入 | 1 |

**Persisted Evidence**

None required —— 沿用 Cycle 声明的 `Persisted Evidence: none`。全部见证为可低成本复核的命令输出与退出码（`--version` 复读、临时库状态、文件字节对比），已由本 Response 的 Verification Evidence 表承载；一次性本机操作无长期审计价值，未创建 `evidence/` 目录。

**Experience Candidates**

**None** —— 升级路径行为本身已由本 change 的 `install-script` R3 增量规格承载（收尾合并进语料），无需另立 Runbook；本次未发现当前 change 范围外的实质缺陷（install.sh 静默输出与条件补全均为既有行为，不构成 Issue 候选）；0.1→0.2 免迁移兼容性的设计声明已获实证支持，但属本 change 内的验收项而非独立经验产物。

**Remaining Issues**

None —— 无未解决问题，无阻塞项，无跳过项。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（强制项：独立阅读实际代码与 diff，核对 Act Response 声明与代码/状态一致；本会话补充独立重建见证）：

- **diff 面核对**：工作区改动恰为 3 文件各 1 行（`Cargo.toml` version / `Cargo.lock` rtsql 条目 / `SKILL.md:7` frontmatter），与 Act 声明一致；`build-riscv64-musl.sh`、`install.sh`、双 README、产品代码零改动（git status 确认）。
- **版本三处同源独立复核**：`Cargo.toml` 0.2.0 / `cargo pkgid` → `rtsql@0.2.0` / `~/.local/bin/rtsql --version` → `rtsql 0.2.0` / `Cargo.lock` 0.2.0 / SKILL frontmatter 0.2.0——全部与 Act Verification Evidence #3–#7 一致。
- **README 白名单独立复核**：`grep -rn '0\.1\.0'` 恰命中 `README.md:302` / `README.zh-CN.md:302` 两处历史标注，零遗漏零多余。
- **幂等独立复核**：本 Review 重跑 `./install.sh` → 退出码 0（增量 0.51s）、安装后仍 `rtsql 0.2.0`——与 Act #17/#18 相互印证。
- **兼容性独立重建见证**：`git worktree`（912df7d）构建 0.1.0 → 临时 `RTSQL_HOME` 建库写 2 行 → 已安装 0.2.0 打开同库：SELECT 返回 0.1 写入行、INSERT exit 0、count=3——与 Act #9–#15 的升级时序见证相互印证（双重覆盖）；worktree 与临时目录已清理。
- **回归独立复核**：`cargo test --no-run` 全目标编译通过；抽样批 `cli_test` 111 / `readonly_session_test` 14 / `memory_db_test` 15 = 140 passed / 0 failed——补足 Act #19 因输出截断未引用的具体计数（Act 以退出码 0 判定，合规；本 Review 提供计数佐证）。
- **偏差分类**：三项偏差均非实质——① 单批全量为契约 T2 Verification 明文授权；② 截断输出以退出码判定属诚实披露，计数已由本 Review 补齐；③ 条件补全（bash only）为 install.sh 既有 `command -v` 分支行为，非本 change 引入，R3「幂等再生」在已安装 shell 上成立（cmp 逐字节一致）。
- **Act #22（validate --all 14 failed）**：全部为 MS06–MS10 时代 spec Purpose 占位——SNAPSHOT 既有的已知状态，非本 change 引入，不阻塞。

非阻塞 Minor finding：`openspec validate install-script --type spec` 的 INFO（R3 文本 >500 字符建议拆分）——规格可读性建议，SHALL 结构清晰，不拆分。

**Acceptance Gaps**

None——install-script R3 三场景全部有独立核验的见证对应（场景 1 = Act #9–#15 + 本 Review worktree 重建；场景 2 = 版本三处同源 + 交叉脚本派生面零改动；场景 3 = Act #17/#18 + 本 Review 重跑）。

**Evidence**

- 本 Review 新增检查：3 文件 diff 核对、版本五点独立复核、README 白名单重跑、install.sh 幂等重跑、0.1→0.2 worktree 重建见证、全目标编译 + 140 例抽样回归。
- 采信 Act Response 验证结论（2026-09-30）：升级时序原始见证（#9–#18，含 cmp 逐字节幂等）、全量测试退出码 0（#19）、validate（#20/#21）。覆盖面未失效（工作区与 Act 声明一致），不重跑全量。

**Follow-up Decision**

Acceptance 全部满足、无阻塞 finding → `accepted`。Next Iteration: None（单 Iteration change），进入收尾（install-script +R3 合并、归档、提交同步——用户 2026-09-30 指令授权串联）。

**Iteration Plan Update**

None。

**Next Cycle**

None。

**Next Iteration**

None。
