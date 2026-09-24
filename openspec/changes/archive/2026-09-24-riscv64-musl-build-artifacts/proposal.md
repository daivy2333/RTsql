## Why

RTsql v0.1.0 已具备 Linux 本机安装与完整交付文档，但尚没有面向 RISC-V 64 Linux 设备的可复现交叉构建入口。用户需要在 x86_64 开发机上生成 `riscv64gc-unknown-linux-musl` 静态二进制及可传递产物，而不把“宿主机无法执行目标架构二进制”误判为构建失败。

## What Changes

- 新增仓库根脚本 `build-riscv64-musl.sh`，固定目标为 `riscv64gc-unknown-linux-musl`，提供 `--output-dir DIR` 与 `--help`。
- 脚本执行前检查 `bash`、`cargo`、`rustup`、`riscv64-linux-musl-gcc`、`tar`、`sha256sum` 与目标 target 是否已安装；缺失时给出可操作错误，不自动下载或修改全局 Rust 环境。
- 使用 `cargo rustc --locked --release --bin rtsql --target riscv64gc-unknown-linux-musl -- -C strip=symbols -C target-feature=+crt-static` 构建，并仅为该命令设置 `CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER=riscv64-linux-musl-gcc`；复用现有 `Cargo.lock`，不增加依赖或持久 `.cargo` 配置。
- 将 `rtsql`、英文 README 与中文 README 收进版本化 `.tar.gz`，生成归档的 `SHA256SUMS`；输出目录可通过参数指定，默认 `dist/` 加入 `.gitignore`，避免二进制产物污染源码状态。
- 在英文/中文 README 增加交叉构建与“产物不会在 x86_64 宿主机执行”的说明。
- 验证仅做宿主机可判定项：脚本语法、`--help`、参数错误路径；本 change 不执行 RISC-V 二进制，也不把本机架构限制作为跳过理由。
- 不实现 SSH/SCP 部署、目标机安装、systemd、交叉运行测试、RISC-V GNU/裸机/RV32、多架构矩阵或 CI/Release 自动化。

## Capabilities

### New Capabilities

- `riscv64-musl-build`: 定义 RISC-V 64 Linux musl 交叉构建前置检查、静态二进制与版本化分发归档生成、校验和及宿主机验证边界。

### Modified Capabilities

无。现有 `install-script` 继续只负责本机源码安装；RISC-V 产物生成是独立能力，不改变其 requirement。

## Impact

- 代码：新增根目录 shell 脚本，并精确修改 `.gitignore` 忽略默认 `/dist/`；不修改 Rust 产品代码、Cargo 依赖或数据库文件格式。
- 文档：修改 `README.md`、`README.zh-CN.md`。
- 产物：运行时生成 `dist/riscv64gc-unknown-linux-musl/`（默认）或用户指定目录，不纳入源码提交。
- 环境：需要已安装的 Rust stable toolchain、`rustup target add riscv64gc-unknown-linux-musl` 与可执行的 `riscv64-linux-musl-gcc`；不要求 sudo、SSH 或 RISC-V 硬件。
- 关联记录：M14（源码/脚本组织边界）、R18（安装分发与 CLI 产品形态）、I051（CI 不在本轮）、I052（预编译矩阵的单架构切片；不自动关闭完整矩阵候选）。本轮不新增 D/K。
- 里程碑归属：MS17 后续分发扩展；不重开已完成的 MS17，正式 milestone 编号若需要由 milestone planner 后续分配。

## Rollback

删除 `build-riscv64-musl.sh`、`.gitignore` 的 `/dist/` 增量、两处 README 增量与本 change 归档即可；脚本不修改系统安装目录、数据库或远端设备。已生成的 `dist/` 产物由用户单独删除，不影响源码与现有 v0.1.0 本机安装。
