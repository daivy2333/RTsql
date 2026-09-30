# install-script Delta — v0.2 升级重装语义

## ADDED Requirements

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
