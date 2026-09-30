# v0.2 版本定义与本地部署升级 — Proposal

## Why

- MS19–MS26 大批能力（约束强制、UPSERT、读面代数、ATTACH、二级索引/视图、只读并发、`:memory:`、WAL 水位）落地后，包版本仍停在初始 `0.1.0`：`rtsql --version` 输出、`rtsql-docs/SKILL.md` frontmatter、riscv64 分发归档命名（`cargo pkgid` 派生）全部滞后于能力面。用户裁定（2026-09-30）：当前版本的数据库**定义为 0.2 版**。
- 本机部署（`~/.local/bin/rtsql`，install.sh 默认 prefix）仍运行 0.1.0，需随版本定义升级到 0.2.0——该升级路径（对既有安装重跑 `install.sh`）在语料 `install-script` 域无承载，本次实际操作正好补入行为规格。

## What Changes

- **T1 版本定义落点**：`Cargo.toml` `version` → `0.2.0`（单一版本源——`rtsql --version` 经 clap 自动跟随、`cargo pkgid` 派生 dist 归档名 `rtsql-v0.2.0-…` 自动跟随）+ `Cargo.lock` 随任一 cargo 命令再生（交叉脚本 `--locked` 构建的前置）+ `rtsql-docs/SKILL.md` frontmatter `version` → `"0.2.0"` + 双 README 版本字样核验。
- **T2 本地部署升级**：`./install.sh` 默认路径重装（`~/.local/bin` + 三 shell 补全幂等再生），`rtsql --version` → `rtsql 0.2.0`、冒烟（临时 `RTSQL_HOME` 建库 CRUD）+ **升级语义进语料**：`install-script` 域 ADDED「升级重装」Requirement（原地替换保数据、版本同源、重跑幂等、0.1 库 0.2 可开）。
- **提交同步**：实现与文档随 change 收尾批次提交入库（沿用仓库收尾惯例；不含 push——未指令）。

## 用户裁定（2026-09-30，三项场景缺口）

1. **双 README 基准节的「RTsql 0.1.0（2026-09-24 实测）」保留**——历史事实准确标注，未来重测时再刷新。
2. **riscv64 交叉构建不重跑**——`dist/` 为 git 忽略本地产物；交叉构建非运行时验证（spec riscv64-musl-build 已声明），留给未来发布动作。脚本本身**零改动**（版本运行时派生，`build-riscv64-musl.sh:90-98`）。
3. **双 README 不新增 What's new 小节**——特性清单已随 MS26 T7 同步至当前能力面。

## 默认假设（Gate 1 审阅项；可逐项推翻）

1. **版本号形态 0.2.0**：Cargo semver 要求三段 `x.y.z`，用户「0.2 版」落为 `0.2.0`。
2. **README 零编辑**：双 README 唯一硬编码版本字样即基准节历史标注（裁定保留）；`rtsql --version`/`rtsql-v<version>-…` 均为无期望输出的命令示例或占位符。T1 对 README 的动作 = 核验无遗漏的当前版本字样（零编辑结论入 Act Response），非内容修改。
3. **升级走 install.sh 默认路径**：默认 prefix `~/.local` + 补全幂等再生；**绝不使用 `--purge-data`**（该 flag 删除数据目录）。升级不触碰 `~/.rtsql/` 用户数据（脚本本身无数据面操作）。
4. **0.1 库 0.2 兼容**：0.1.0（MS17 初始发布）含 64B 格式头与加密面；MS19–MS26 的 catalog 尾段扩展全部向后兼容读（旧行读为空），页格式未变——升级后既有库可直接打开。T2 以「0.1.0 建库 → 升级 → 0.2.0 打开查询」实证该声明，若实证失败即 Stop 返回 Plan（0.x 内格式兼容声明失实属实质问题）。

## 范围外（Non-goals）

- riscv64 交叉构建重跑与 dist 产物发布（裁定 2）。
- CHANGELOG / What's new 小节（裁定 3）；版本号之外的任何文档内容改写。
- git push（未指令；本地提交随收尾）。
- MS27 及任何功能面变更；`--version` 输出格式变更（既有 clap 默认形态保持）。
- 旧版本卸载重装（`--uninstall` 路径）——原地重装即升级，无需先卸载。

## Impact

- **受影响文件**：`Cargo.toml`（version 字段）、`Cargo.lock`（自动再生）、`rtsql-docs/SKILL.md`（frontmatter version）、`openspec/specs/install-script/spec.md`（收尾合并 +R3）、双 README 仅核验。
- **本机状态**：`~/.local/bin/rtsql` 0.1.0 → 0.2.0；`~/.local/share/bash-completion/completions/rtsql` 等补全文件再生（幂等）；用户数据目录不触碰。
- **测试**：无既有测试断言版本号（`grep -rn '0\.1\.0' tests/ src/` 零命中，2026-09-30 调查）——见证以命令输出承载（`cargo pkgid`/`--version`/grep + 升级操作实跑）；全量测试零回归兜底（版本字段不影响行为面）。

## 验证边界

- **T1**：`cargo pkgid` 输出 0.2.0；`cargo build` 后 `./target/release/rtsql --version` 输出 `rtsql 0.2.0`；SKILL frontmatter grep 0.2.0；双 README grep 确认仅存历史标注；`Cargo.lock` 中 rtsql 包版本 0.2.0。
- **T2**：升级前 `rtsql --version` 0.1.0 + 临时库建库；`./install.sh` 退出码 0；升级后 `rtsql --version` 0.2.0；原库可开可查数据完整；补全文件存在且幂等；全量测试零回归。
