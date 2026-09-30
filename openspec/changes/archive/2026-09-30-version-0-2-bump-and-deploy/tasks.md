# v0.2 版本定义与本地部署升级 — Tasks

## 任务清单

| Task | 目标 | Iteration | Requirement 映射 |
|---|---|---|---|
| T1 | 版本定义落点：`Cargo.toml` version → `0.2.0` + `Cargo.lock` 再生（任一 cargo 命令）+ `rtsql-docs/SKILL.md` frontmatter → `"0.2.0"` + 双 README 版本字样核验（零编辑裁定，核验结论入 Act Response）+ `cargo pkgid`/`--version` 同源见证 | 000 | install-script R3（升级重装）之「版本三处同源」场景 |
| T2 | 本地部署升级：0.1.0 现状取证（`--version` + 临时库建库写数）→ `./install.sh` 默认路径重装 → `--version` 0.2.0 + 既有 0.1 库可开可查可写 + 补全幂等 + 连续重跑幂等 + 全量测试零回归兜底 | 000 | install-script R3（升级重装）全三场景 |

## Iteration Plan

### Iteration 000: 版本定义与部署升级

- Tasks: T1, T2
- Depends on: None
- Stable baseline: `rtsql --version`/`cargo pkgid`/SKILL frontmatter 三处同源 0.2.0；本机 `~/.local/bin/rtsql` 为 0.2.0 且 0.1 库无迁移可开；升级路径行为进语料（收尾合并 install-script +R3）；全量测试零回归。
- Verification boundary: T1/T2 命令输出见证（pkgid/--version/grep/安装退出码/冒烟 CRUD）+ 全量 `cargo test` 零回归（M19 分批或全量单批——版本字段无行为面，容忍单批全量）+ 收尾提交同步。
- Diagnostic boundary: `Cargo.toml`、`Cargo.lock`、`rtsql-docs/SKILL.md` frontmatter、`install.sh` 执行面（不改脚本本体）、`~/.local` 安装点。
- Non-goals: riscv dist 重跑；What's new/CHANGELOG；push；任何功能面（见 proposal Non-goals）。

## Requirements Traceability Matrix

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| install-script R3 升级重装（ADDED，本 change） | T1, T2 | T1 版本三处同源见证（pkgid/`--version`/归档命名派生面）+ T2 升级实跑（0.1 库可开/幂等重跑/退出码 0） | Covered |

平衡审计：单 Iteration 两任务形成同一阶段成果「版本 0.2 定义与部署同步」，T1 是 T2 的直接前置（同域、共享版本同源验证面），无过碎或过重；改动面小（2 文件 + 1 本机操作）符合小 change 单 Iteration 先例。

## 与 OpenSpec Changes 同步

- 实施经 `openspec/changes/2026-09-30-version-0-2-bump-and-deploy/` 承载；收尾时 install-script 增量合并（+R3），change 归档，提交随收尾批次（用户指令「提交同步」；不含 push）。
