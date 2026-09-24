## Context

当前根 `install.sh` 只服务宿主机架构：它运行本机构建出的二进制，并在宿主机安装程序与补全。RISC-V 目标不能在 x86_64 上执行，因此新入口必须把“构建成功”与“目标机运行验证”分开。仓库已有 `Cargo.lock`、版本 0.1.0、双语 README 和 Unix 脚本惯例；生产依赖不要求本轮新增。

关键外部依赖：Rust stable/rustup、musl std target、`riscv64-linux-musl-gcc`、Bash、GNU tar、coreutils `sha256sum`。本机已安装 GNU 与 bare-metal RISC-V targets；musl target 补齐后，Rust target 默认仍可能调用宿主 `cc`/`/usr/bin/ld`，因此构建命令必须显式使用现有 musl GCC 交叉链接器。

## Goals / Non-Goals

**Goals:**

- 固定生成 `riscv64gc-unknown-linux-musl` release 二进制，不执行它。
- 使用 locked 依赖与 Rust/LLVM strip 能力生成可传递产物。
- 输出内容稳定的版本化 tar.gz 与 SHA-256 校验文件。
- 将交叉编译失败、宿主验证与目标硬件验证三者清楚分开。

**Non-Goals:**

- RISC-V GNU、RV32、裸机、QEMU 或远程硬件运行。
- SSH/SCP、目标机安装器、sudo/systemd 与远端配置。
- Docker/cross、zigbuild、CI、GitHub Release、crates.io 与多架构矩阵。
- 修改 Rust 产品代码、Cargo 依赖、数据库格式或已有 `install.sh` 契约。

## Decisions

### D1: 固定 `riscv64gc-unknown-linux-musl`，不暴露 target override

脚本内固定 target 名称并启用 `+crt-static`，用户只可选择输出目录。这样命令、产物命名与文档不会漂移到 GNU 或 bare-metal ABI，也避免动态 musl 解释器依赖；用户不需要在目标系统额外准备 `/lib/ld-musl-riscv64.so.1`。

**备选方案**：

- `--target` 任意覆盖：灵活但会把 ABI/产物命名变成新的长期契约，拒绝。
- `riscv64gc-unknown-linux-gnu`：依赖目标 glibc 版本，跨发行版兼容性较差，拒绝。
- `riscv64gc-unknown-none-elf`：需要重定义 OS/网络/信号边界，超出本轮，拒绝。

### D2: 前置检查只报告，不隐式安装

构建前一次收集缺失命令和 target 状态，退出并给出安装提示。脚本不运行 `rustup target add`，避免网络、工具链版本与全局环境隐式变化；`riscv64-linux-musl-gcc` 也作为显式前置依赖检查。

target 检测使用 `rustup target list --installed` 的逐行精确匹配；不使用子串匹配，避免相近 target 假阳性。`--help` 在依赖检查前返回，保证无 target 环境也能查看用法。

### D3: `cargo rustc --locked --release` + `-C strip=symbols`

构建命令固定为：

```text
CARGO_TARGET_RISCV64GC_UNKNOWN_LINUX_MUSL_LINKER=riscv64-linux-musl-gcc \
  cargo rustc --locked --release --bin rtsql --target riscv64gc-unknown-linux-musl -- -C strip=symbols -C target-feature=+crt-static
```

`--bin rtsql` 确保额外 rustc 参数作用于最终 CLI binary；`+crt-static` 强制最终 ELF 静态链接。环境变量只注入当前 Cargo 子进程，不写全局或仓库 `.cargo` 配置；这修复了 musl target 默认落到宿主 `/usr/bin/ld` 的实际失败。`--locked` 保证 Cargo.lock 不被改写；`cargo rustc` 将 strip 选项只传给最终 rtsql crate，不污染全局 `RUSTFLAGS`。产物从 `target/riscv64gc-unknown-linux-musl/release/rtsql` 读取。

**备选方案**：

- `cargo build` 后调用宿主 `strip`：宿主 strip 未必支持 RISC-V，失败语义不稳定，拒绝。
- 每次 export `RUSTFLAGS`：影响依赖与用户环境，范围过大，拒绝。
- 只依赖 Rust target 默认 linker：会落到宿主 `cc`/`/usr/bin/ld`，已实证失败，拒绝。
- 只设置 musl linker、不启用 `+crt-static`：会生成依赖 `/lib/ld-musl-riscv64.so.1` 的动态 PIE，已实证不满足静态分发，拒绝。
- Docker/cross/zigbuild：新增外部工具链与维护面，本轮单机脚本不需要，拒绝。

### D4: 输出根与可替换目标子目录分层

默认输出根为 `$SCRIPT_DIR/dist`；`--output-dir DIR` 指向输出根，relative path 按调用者当前目录解析。`.gitignore` 精确新增 `/dist/`，避免默认构建产物污染源码状态，自定义输出目录仍由用户管理。脚本只拥有 `$OUTPUT_ROOT/riscv64gc-unknown-linux-musl/` 子目录，重复构建可原子式替换该子目录，但保留输出根内其他文件。删除前校验目标子目录的 basename 与非空父路径，禁止 `/`、空串、`.`、`..`。

构建成功前不发布成功摘要。归档与校验在临时 staging 目录完成，再替换目标子目录，避免失败留下看似完整的旧/新混合产物。

### D5: 版本与归档内容由现有仓库来源派生

版本从 `cargo pkgid` 的 package id 尾段读取，不在脚本中硬编码 `0.1.0`。归档名为：

```text
rtsql-v<version>-riscv64gc-unknown-linux-musl.tar.gz
```

tar 顶层固定包含 `rtsql`、`README.md`、`README.zh-CN.md`。`SHA256SUMS` 只记录该 tar.gz，不引入 build-id、manifest、时间戳证明或专用审计工具。README 是随产品交付的静态材料，不增加额外生成协议。

### D6: 成功摘要显式声明未做目标机运行验证

脚本在成功后报告绝对路径与“cross-build succeeded; target execution not verified”。它不调用目标文件，不用 `file` 输出、Hash、ELF 元数据或 checksum 冒充功能验证。

### D7: 文档只增加宿主机可执行说明

英文/中文 README 在安装与已知限制附近增加同一内容的交叉构建段：安装 target、运行脚本、自定义输出目录、产物位置、下载/部署由用户在 RISC-V 主机完成，并明确本仓库未做目标硬件运行验证。

## Flow

```text
用户
  │ --help / --output-dir
  ▼
参数解析（先于依赖检查）
  │
  ▼
工具与 target 精确检查 ──缺失──> 错误 + 安装提示 + 非零退出
  │ 全部存在
  ▼
cargo rustc --locked --release --target riscv64gc-unknown-linux-musl
  │ 失败 → 保留 Cargo 原生错误，不生成成功摘要
  ▼
临时 staging：rtsql + 双语 README
  │
  ▼
tar.gz + SHA256SUMS
  │
  ▼
安全替换目标子目录 → 路径摘要 + 未做目标机验证声明
```

## Risks / Trade-offs

- [musl target 未安装] → 前置检查精确报错并给出单条 `rustup target add` 命令；不自动下载。
- [生产依赖出现 target-specific C/linker 需求] → Cargo 原生错误直接透传；本轮计划不通过 Docker/cross 掩盖，后续按真实错误独立规划。
- [宿主 `sha256sum` 不可用] → 明确列为前置依赖；本轮不增加 macOS `shasum` 分支，避免未验证的跨平台工具选择。
- [Rust musl target 默认调用宿主 linker] → 预检 `riscv64-linux-musl-gcc`，并只为 Cargo 子进程注入 target linker 环境变量；已用诊断构建实证可完成 RISC-V 链接。
- [无法在 x86_64 运行目标二进制] → 只声明构建与归档成功；目标 CRUD/加密/恢复由真实 RISC-V 设备另行验证。
- [重复构建覆盖输出] → 仅替换固定子目录，先 staging 后替换；输出根其他文件保留。
- [Cargo.lock 与 target 组件不兼容] → `--locked` 立即失败，禁止自动更新依赖。

## Migration Plan

1. 用户安装固定 musl target。
2. 在仓库根执行脚本，生成默认或指定输出目录。
3. 将 tar.gz 与 SHA256SUMS 传递给目标设备；校验后解压并按目标系统策略安装。
4. 真实 RISC-V 设备另行执行版本、CRUD、事务、加密、恢复与卸载冒烟；结果不由本脚本伪造。

回滚只涉及删除新脚本、README 增量与 change 归档；默认/自定义 `dist` 产物可独立删除。现有 v0.1.0 本机安装和数据库文件不受影响。

## Open Questions

无。会影响 spec、ABI、产物格式与任务拆分的问题已由用户选择固定：RISC-V 64 musl、仅产物、目标机不执行。
