# install-script Specification

## Purpose
定义仓库根 `install.sh` 的本机编译安装、shell 补全、PATH 提示、程序/数据两模式卸载与原地升级重装契约。来源：MS17-T03 change `2026-09-23-ms17-initial-release`；升级重装与版本同源由 v0.2 change `2026-09-30-version-0-2-bump-and-deploy` 增补。

## Requirements

### Requirement: 一键编译安装面

仓库根目录 SHALL 提供 `install.sh` 脚本，在 Linux/macOS 本机以一条命令完成从源码安装：cargo 存在性检查 → `cargo build --release` → strip 目标二进制（strip 不可用时 SHALL 跳过并提示、不失败）→ 安装二进制到 `$PREFIX/bin/rtsql`（默认 `~/.local`，`--prefix <DIR>` 覆盖）→ 按 `$SHELL` 与命令存在性检测安装 shell 补全（bash → `~/.local/share/bash-completion/completions/rtsql`、zsh → `~/.zsh/completions/_rtsql` 并输出 fpath 追加提示、fish → `~/.config/fish/completions/rtsql.fish`；补全内容来自 `rtsql completions <shell>` 同源生成）→ `$PREFIX/bin` 不在 PATH 时输出 export 提示。`--no-completions` SHALL 跳过补全安装。`--help` SHALL 输出用法。脚本 SHALL 以严格模式运行（`set -euo pipefail`），任一安装步骤失败 SHALL 以非零退出码终止。

#### Scenario: 临时 PREFIX 一键安装可用

- **GIVEN** 本机装有 Rust 工具链的干净目录（临时 `$PREFIX`）
- **WHEN** `PREFIX=<tmp> ./install.sh` 后执行 `<tmp>/bin/rtsql --version` 与建库 CRUD（临时 `RTSQL_HOME`）
- **THEN** 安装退出码 0，版本输出正常，CRUD 全部成功；`$PREFIX/bin/rtsql` 存在且经 strip

#### Scenario: 补全安装与装载

- **GIVEN** 临时 PREFIX 安装完成
- **WHEN** 检查对应 shell 的补全文件并装载
- **THEN** 补全文件存在于约定路径且内容含 `rtsql`；bash 补全文件 `source` 装载无错误；`--no-completions` 安装时不产生补全文件

#### Scenario: PATH 提示

- **GIVEN** `$PREFIX/bin` 不在当前 PATH
- **WHEN** 安装完成
- **THEN** stdout 输出包含 `$PREFIX/bin` 的 PATH export 提示；`$PREFIX/bin` 已在 PATH 时不输出

### Requirement: 卸载两模式

`install.sh --uninstall` SHALL 只清理程序面：删除 `$PREFIX/bin/rtsql` 与三 shell 补全文件（存在才删），SHALL NOT 触碰数据目录；卸载后 `rtsql` 命令不可用而 `$RTSQL_HOME`（或默认 `~/.rtsql/`）数据完整保留。`--uninstall --purge-data` SHALL 在程序清理之外追加清除数据目录：删除前 SHALL 逐行列出将删除的路径，显式 flag 组合即确认（无交互提示）；清除后数据目录不存在。`--prefix` SHALL 与安装/卸载对称定位二进制。

#### Scenario: 程序卸载后数据保留

- **GIVEN** 临时 PREFIX 安装且 `RTSQL_HOME` 临时目录含建库数据
- **WHEN** `--uninstall` 执行
- **THEN** 二进制与补全文件消失、`rtsql` 不可用；`RTSQL_HOME` 数据目录原样保留

#### Scenario: 带数据清理须显式 flag 并列出路径

- **GIVEN** 程序已安装且数据目录存在
- **WHEN** `--uninstall --purge-data` 执行
- **THEN** stdout 先逐行列出将删除的数据路径，随后删除；数据目录不再存在；不带 `--purge-data` 的卸载 SHALL NOT 产生数据删除

### Requirement: 升级重装

对已存在安装的 `install.sh` 重跑 SHALL 作为原地升级路径：新构建的二进制 SHALL 原子替换 `$PREFIX/bin/rtsql`（`install -m 0755` 覆盖语义），shell 补全 SHALL 幂等再生（内容同源覆盖、不产生重复条目），SHALL NOT 触碰 `$RTSQL_HOME`（或默认 `~/.rtsql/`）数据目录。升级后 `rtsql --version` SHALL 反映新构建的包版本——版本单一来源为 `Cargo.toml` 的 `version` 字段，`rtsql --version`（clap 自动接线）与 `cargo pkgid`（分发归档命名派生）三者 SHALL 同源一致。同一 0.x 系列内升级 SHALL NOT 要求既有数据库文件任何迁移即可打开与查询（存储格式兼容）；升级路径 SHALL NOT 依赖先卸载（`--uninstall` 非升级前置）。

#### Scenario: 0.1 安装态原地升级保数据

- **GIVEN** `~/.local/bin/rtsql` 为 0.1.0 且临时 `RTSQL_HOME` 内有 0.1.0 建库并写入的表
- **WHEN** 源码版本改为 0.2.0 后重跑 `./install.sh`（退出码 0），随后 `rtsql --version` 并打开既有库执行 SELECT 与 INSERT
- **THEN** 版本输出 `rtsql 0.2.0`；既有库可开、旧数据可查、新写入成功，数据目录无任何删除或迁移痕迹

#### Scenario: 版本三处同源

- **WHEN** 分别读取 `Cargo.toml` 的 `version`、`cargo pkgid` 输出与构建产物 `rtsql --version` 输出
- **THEN** 三者版本段一致（如均为 `0.2.0`）；riscv64 分发归档命名（`rtsql-v<version>-<target>.tar.gz`）随同源自动跟随

#### Scenario: 重跑幂等

- **GIVEN** 同一版本下连续两次 `./install.sh`
- **THEN** 两次均退出码 0；二进制与补全文件内容一致（无重复条目、无残留备份文件）；第二次不产生额外副作用
