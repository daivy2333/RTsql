## Purpose

定义从 x86_64 或其他兼容宿主机生成 RISC-V 64 Linux musl 数据库二进制与可传递归档的前置检查、输出契约和验证边界，使目标架构不可执行时仍能完成可复现的构建交付。

## ADDED Requirements

### Requirement: 固定目标的交叉构建入口

仓库 SHALL 提供 `build-riscv64-musl.sh`，将目标固定为 `riscv64gc-unknown-linux-musl`，接受可选 `--output-dir DIR` 与 `--help`，并以严格模式运行。构建 SHALL 启用 `+crt-static`，产物 SHALL 为静态链接 RISC-V ELF 且不依赖动态解释器。脚本 SHALL NOT 提供 RISC-V GNU、RV32、裸机或任意 target override；未知参数、缺少参数值与非法参数 SHALL 以非零退出码拒绝。脚本 SHALL 支持从任意当前目录调用，并始终以自身所在仓库为构建根。

#### Scenario: 查看交叉构建用法

- **WHEN** 在仓库任意位置执行 `./build-riscv64-musl.sh --help`
- **THEN** stdout 显示固定 target、输出目录、依赖与“不在宿主机执行目标二进制”的说明，退出码 0

#### Scenario: 拒绝未知参数

- **GIVEN** 脚本已存在
- **WHEN** 执行 `./build-riscv64-musl.sh --unknown`
- **THEN** stderr 给出参数错误与用法，退出码非 0，且不启动 Cargo 构建

#### Scenario: 从仓库外目录调用

- **GIVEN** 当前工作目录不是仓库根
- **WHEN** 通过脚本绝对路径执行构建
- **THEN** Cargo 仍以脚本所在仓库为工作目录，产物写入 `--output-dir` 解析后的位置

#### Scenario: 生成静态 RISC-V ELF

- **GIVEN** Rust musl target 与 `riscv64-linux-musl-gcc` 均可用
- **WHEN** 脚本以 `-C target-feature=+crt-static` 完成构建
- **THEN** 目标文件为 RISC-V 64 位静态链接 ELF，不包含 `/lib/ld-musl-riscv64.so.1` 等动态解释器依赖；检查只读取 ELF 元数据，不执行目标文件

### Requirement: 构建前置检查与锁定依赖

脚本 SHALL 在构建前检查 `cargo`、`rustup`、`riscv64-linux-musl-gcc`、`tar` 与 `sha256sum` 可用，并确认 `riscv64gc-unknown-linux-musl` target 已安装。缺失工具或 target 时 SHALL 列出缺失项与对应安装提示并以非零退出码停止；SHALL NOT 自动下载 target、安装系统包、调用 sudo 或修改全局 Rust 配置。构建 SHALL 为当前 Cargo 子进程设置 `CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER=riscv64-linux-musl-gcc`，SHALL 使用仓库已提交的 `Cargo.lock`，SHALL NOT 更新该文件或持久化 linker 配置。

#### Scenario: Rust target 已安装

- **GIVEN** 所需工具与 `riscv64gc-unknown-linux-musl` target 均已安装
- **WHEN** 执行交叉构建
- **THEN** 脚本使用 locked release 构建进入产物阶段，Cargo.lock 保持不变

#### Scenario: Rust target 缺失

- **GIVEN** `rustup target list --installed` 不包含 `riscv64gc-unknown-linux-musl`
- **WHEN** 执行脚本
- **THEN** stderr 点名缺失 target，并提示执行 `rustup target add riscv64gc-unknown-linux-musl`；退出码非 0，且不启动 Cargo 构建

#### Scenario: 构建工具缺失

- **GIVEN** `cargo`、`rustup`、`riscv64-linux-musl-gcc`、`tar` 或 `sha256sum` 任一不可用
- **WHEN** 执行脚本
- **THEN** stderr 列出全部缺失工具，退出码非 0，且不创建成功归档或校验文件

#### Scenario: 使用 musl 交叉链接器

- **GIVEN** Rust musl target、`riscv64-linux-musl-gcc`、`tar` 与 `sha256sum` 均可用
- **WHEN** 执行脚本
- **THEN** 当前 Cargo 子进程使用 `CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER=riscv64-linux-musl-gcc`，不写入全局 Cargo 配置；链接器为 RISC-V ELF 目标而非宿主机 `/usr/bin/ld`

### Requirement: 版本化分发产物

成功构建 SHALL 从固定 target 的 release 输出复制 `rtsql`，连同 `README.md` 与 `README.zh-CN.md` 组成版本化 `.tar.gz`；版本 SHALL 来自当前 Cargo package。脚本 SHALL 在归档旁生成该归档的 `SHA256SUMS`，并在 stdout 报告二进制、归档和校验文件的绝对路径。默认输出根 SHALL 为仓库 `dist/`，目标子目录 SHALL 为 `riscv64gc-unknown-linux-musl/`；`--output-dir DIR` SHALL 将该目标子目录放在 DIR 下。重复构建 SHALL 只替换本脚本拥有的目标子目录，SHALL NOT 删除输出根中的其他文件。

#### Scenario: 生成默认 musl 产物

- **GIVEN** 前置检查通过且交叉构建成功
- **WHEN** 执行 `./build-riscv64-musl.sh`
- **THEN** `dist/riscv64gc-unknown-linux-musl/` 包含可执行 `rtsql`、`rtsql-v<version>-riscv64gc-unknown-linux-musl.tar.gz` 与 `SHA256SUMS`，退出码 0

#### Scenario: 归档内容与校验文件

- **GIVEN** 交叉构建生成目标二进制
- **WHEN** 检查版本化 tar.gz 与 SHA256SUMS
- **THEN** tar.gz 顶层包含 `rtsql`、`README.md`、`README.zh-CN.md`；SHA256SUMS 恰记录该归档，校验命令退出码 0

#### Scenario: 自定义输出目录

- **GIVEN** 用户传入存在的父目录或可创建的新目录
- **WHEN** 执行 `./build-riscv64-musl.sh --output-dir /tmp/rtsql-dist`
- **THEN** 产物写入 `/tmp/rtsql-dist/riscv64gc-unknown-linux-musl/`，不写入默认 `dist/`

#### Scenario: 重复构建保留无关文件

- **GIVEN** 自定义输出根中存在不属于本脚本的 `keep.txt`
- **WHEN** 再次执行脚本完成构建
- **THEN** `keep.txt` 保持不变，脚本仅替换 `riscv64gc-unknown-linux-musl/` 内容

### Requirement: 宿主机验证边界

脚本 SHALL NOT 执行、加载或解释 RISC-V 目标二进制，也 SHALL NOT 通过 QEMU、binfmt、容器或远程设备补做运行测试。实现验证 SHALL 仅使用宿主机可判定项：shell 语法检查、`--help`、参数错误与路径保护；本机不是 RISC-V 架构 SHALL NOT 成为跳过脚本实现验证的理由。脚本在缺少真实 RISC-V 验证时 SHALL 明确把产物标记为“已交叉构建、未在目标硬件运行验证”，不得声称数据库功能已在 RISC-V 上通过。

#### Scenario: 宿主机完成脚本验证

- **GIVEN** 当前宿主机不是 RISC-V
- **WHEN** 对脚本执行 `bash -n`、`--help` 与非法参数检查
- **THEN** 命令按预期退出，且不需要也不尝试运行目标二进制

#### Scenario: 构建成功但未做目标机运行验证

- **GIVEN** Cargo 成功生成 RISC-V musl 二进制
- **WHEN** 脚本报告成功摘要
- **THEN** 摘要明确区分“交叉构建成功”和“未在 RISC-V 硬件运行验证”，不把编译成功表述为功能验证
