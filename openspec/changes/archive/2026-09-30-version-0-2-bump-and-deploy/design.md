# v0.2 版本定义与本地部署升级 — Design

调查基线：HEAD 912df7d 工作区（MS26 收尾 + R31 runbook 提交后），全量 1590 tests / 0 failures；本机 `~/.local/bin/rtsql` 报 `rtsql 0.1.0`。

## Current-State Evidence（Plan 本会话实读）

1. **版本单一源**：`Cargo.toml:3` `version = "0.1.0"`。下游全部自动派生：`rtsql --version` 经 clap 默认接线输出 `rtsql 0.1.0`（本机实跑）；`build-riscv64-musl.sh:90-98` 以 `cargo pkgid` 派生 `VERSION` 并命名归档 `rtsql-v$VERSION-$TARGET_TRIPLE.tar.gz`（零硬编码）。bump 只需改 Cargo.toml 一处。
2. **Cargo.lock 联动**：交叉脚本 `:101` 以 `--locked` 构建——Cargo.lock 中 rtsql 包版本条目必须与 Cargo.toml 同步（任一 cargo 命令自动再生）；`install.sh:144` 为裸 `cargo build --release`（无 `--locked`，自然再生）。T1 的构建步骤天然完成同步。
3. **文档版本字样**：`rtsql-docs/SKILL.md:7` frontmatter `version: "0.1.0"`（唯一需编辑点）。双 README 硬编码字样仅 `README.md:302` / `README.zh-CN.md:302` 基准溯源段「RTsql 0.1.0（2026-09-24 实测）」——用户裁定保留为历史标注；`README.md:24`、`SKILL.md:51/71/102` 的 `rtsql --version` 与 `README.md:52` 的 `rtsql-v<version>-…` 均无期望输出/占位符，零编辑。
4. **无测试版本断言**：`grep -rn '0\.1\.0' tests/ src/` 零命中（2026-09-30）——版本字段无行为面，见证以命令输出承载。
5. **安装面**：`install.sh` 默认 `PREFIX=~/.local`，`cargo build --release` → strip → `install -m 0755 "$SOURCE_BIN" "$BIN_PATH"`（`install(1)` 覆盖语义 = 原子替换）→ 三 shell 补全再生 → PATH 提示；`--uninstall [--purge-data]` 独立。本机部署点 `~/.local/bin/rtsql`（0.1.0）。
6. **语料缺口**：`install-script` 域 R1 只覆盖全新安装（临时 PREFIX），升级重装（对既有安装重跑）无承载——本 change T2 实际操作即该路径，ADDED R3 补入。

## 设计决策

### D1 版本单一源，一处修改

`Cargo.toml` 是唯一编辑的版本源；`--version`、`cargo pkgid`、dist 归档名三下游自动跟随。替代方案（各处硬编码同步）否决：多源漂移风险，且现状本就是单一源接线。`Cargo.lock` 再生由 T1 的 `cargo build` 步骤自然完成——作为交叉脚本 `--locked` 的前置写入契约，不单独造同步脚本。

### D2 升级路径 = install.sh 原地重装

对既有安装重跑 install.sh 即升级（`install -m 0755` 覆盖 + 补全幂等再生），不卸载先、不 `--purge-data`（该 flag 删数据目录，与升级目标相反）。0.1 库兼容性以实跑见证（0.1.0 建库 → 升级 → 0.2.0 打开查询写回）——若失败即 Stop（0.x 内格式兼容声明失实属实质问题，MS19–MS26 catalog 尾段全部向后兼容读的设计预期支持兼容）。

### D3 三项用户裁定（2026-09-30 AskUserQuestion）

基准节历史标注保留（历史事实准确）；riscv 交叉构建不重跑（dist 为 git 忽略本地产物、交叉构建非运行时验证）；不加 What's new（特性清单已随 MS26 T7 同步）。裁定与理由随 change 归档。

## Iteration 顺序

单 Iteration（000）：T1（版本落点）→ T2（依赖 T1 的新版本构建）。

## 风险与边界

- `install.sh` 运行会拉取依赖（Cargo 声明内）、执行 strip——既有安装面行为，非新风险。
- 升级冒烟一律用临时 `RTSQL_HOME`，用户 `~/.rtsql/` 只做「不触碰」的旁观验证（升级前后目录 mtime/内容不变可选核验，不作硬断言——避免并行进程干扰误报）。
- 版本字符串在 README 基准节的保留意味着「README 中存在 0.1.0 字样」是长期稳态，未来 grep 核验按此白名单解释。
