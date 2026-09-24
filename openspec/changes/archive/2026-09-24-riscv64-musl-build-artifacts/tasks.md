# RISC-V 64 musl 交叉构建产物 — Tasks

> 里程碑：MS17 后续分发扩展（不重开 completed MS17；正式编号待 milestone planner 分配）。
> 验证边界：只做宿主机 shell 语法、帮助/参数错误、差异与 OpenSpec 校验；按用户要求不执行交叉构建脚本的成功路径、不运行 RISC-V 二进制、不补目标硬件测试。

## 1. 交叉构建脚本

- [x] 1.1 新建可执行根脚本 `build-riscv64-musl.sh`，实现 `set -euo pipefail`、仓库根定位、固定 target、`--output-dir DIR`、`--help` 与统一参数错误；验证 `bash -n build-riscv64-musl.sh`、`./build-riscv64-musl.sh --help` 和未知参数非零退出。
- [x] 1.2 实现一次收集式前置检查：逐项检查 `cargo`/`rustup`/`riscv64-linux-musl-gcc`/`tar`/`sha256sum`，精确检查 musl target 已安装，缺失时只报告安装提示、不自动安装；验证代码路径先处理 help/参数再进入检查，且当前已安装的 GNU/bare-metal RISC-V target 不会误匹配 musl target。
- [x] 1.3 实现 locked release 构建与安全产物发布：`.gitignore` 精确新增 `/dist/`；从 `cargo pkgid` 派生版本，为当前 Cargo 子进程设置 `CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER=riscv64-linux-musl-gcc`，以 `cargo rustc --locked --release --bin rtsql --target riscv64gc-unknown-linux-musl -- -C strip=symbols -C target-feature=+crt-static` 构建静态 RISC-V ELF，在 staging 中收集二进制/双语 README，生成 tar.gz + SHA256SUMS，再安全替换固定目标子目录；验证输出路径保护、保留无关文件、Cargo.lock 不变及失败时不输出成功摘要。
- [x] 1.4 输出最终二进制/归档/校验文件绝对路径与“cross-build succeeded; target execution not verified”声明；验证脚本不存在任何执行 `$TARGET_TRIPLE` 二进制、SSH/QEMU/binfmt 或自动安装依赖的路径。

## 2. 双语文档

- [x] 2.1 在 `README.md` 与 `README.zh-CN.md` 增加同构 RISC-V 64 musl 章节，覆盖 `rustup target add`、默认/自定义输出目录、tar.gz/SHA256SUMS、目标设备部署属于后续人工步骤及未做目标硬件运行验证；验证两文档命令与 `build-riscv64-musl.sh --help` 一致。

## 3. 收尾验证

- [x] 3.1 执行宿主机可判定门：`bash -n build-riscv64-musl.sh`、帮助/非法参数检查、musl linker 前置检查、`git diff --check`、`openspec validate 2026-09-24-riscv64-musl-build-artifacts --strict`；记录成功交叉构建与 RISC-V 运行测试为 `SKIPPED: 用户要求且当前宿主非 RISC-V`，不得声明通过。
- [x] 3.2 审查完整 diff，确认仅新脚本、`.gitignore` 精确 `/dist/` 增量、两份 README 与本 change 产物变化，Cargo 源码/依赖和已有 `install.sh` 零修改；保留用户已有未跟踪 `demo.sql`，不暂存/删除；同步任务状态后交 Plan Review。

## Requirements Traceability Matrix

| Requirement | Scenario | Design | Task | Code Surface | Verification | Status |
|---|---|---|---|---|---|---|
| 固定目标的交叉构建入口 | help / 未知参数 / 仓库外调用 / 静态 ELF | D1/D6 | 1.1/1.3/1.4 | `build-riscv64-musl.sh` 参数、repo-root 定位、crt-static | `bash -n`、help、非法参数、`file` 静态元数据；不执行目标 | Covered |
| 构建前置检查与锁定依赖 | target 已装 / target 缺失 / 工具缺失 / musl linker | D2/D3 | 1.2/1.3 | preflight + linker env + `cargo rustc --locked` | 精确匹配审查；不伪造缺失 target 成功证据 | Covered |
| 版本化分发产物 | 默认输出 / 归档内容 / 自定义目录 / 重复构建 | D4/D5 | 1.3 | `.gitignore`、`build-riscv64-musl.sh` staging/tar/sha256/固定子目录替换 | 安全路径与流程审查；成功构建按用户要求不执行 | Covered |
| 宿主机验证边界 | 宿主静态验证 / 未做目标机运行 | D6/D7 | 1.4/2.1/3.1 | 成功摘要、README、验证记录 | 静态门通过并显式 SKIPPED 目标运行 | Covered |

## Iteration Plan

### Iteration 000: riscv64-musl-artifacts

- Tasks: 1.1, 1.2, 1.3, 1.4, 2.1, 3.1, 3.2
- Depends on: None
- Stable baseline: 仓库提供固定 `riscv64gc-unknown-linux-musl` 的 locked 构建脚本；默认 `dist/` 被忽略；脚本可生成版本化 tar.gz + SHA256SUMS 且不执行目标二进制；双语文档与宿主机验证边界一致
- Verification boundary: `bash -n`、help/非法参数、缺失 target 停止路径的宿主机可判定检查、`git diff --check`、strict OpenSpec validate；成功交叉构建与 RISC-V 运行测试显式 SKIPPED
- Diagnostic boundary: 根 `build-riscv64-musl.sh`、`.gitignore`、`README.md`、`README.zh-CN.md` 与本 change Cycle
- Non-goals: Rust 产品代码、Cargo 依赖/lock、已有 `install.sh`、SSH/目标机安装、QEMU/真实硬件测试、CI/Release 自动化
- 平衡审计: 七个任务共同形成一个“脚本 + 使用文档 + 宿主验证”的单一交付面；脚本各层必须按参数→preflight→build→artifact→summary 顺序集成，文档依赖最终 CLI，拆成多 Iteration 会产生无法独立验收的半成品。规模集中且诊断面一致，单 Iteration 合理。
