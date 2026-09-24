# RTsql

[English](README.md) | [简体中文](README.zh-CN.md) | [Agent 操作手册](rtsql-docs/SKILL.md)

RTsql 是一个用 Rust 编写的嵌入式关系型数据库。它使用 Tokio 任务执行异步 I/O，以单个主文件保存数据库，并通过一次性执行的 CLI 提供 SQL 执行和管理能力。

当前仓库支持 Linux 和 macOS，不依赖独立数据库服务。

## 快速开始

### 前置条件

- 安装了带 `cargo` 的稳定版 Rust 工具链
- 安装 `bash` 和常用 Unix 用户工具；`strip` 可选
- 从远程仓库获取源码时需要 Git

### 从源码安装

```bash
git clone git@github.com:daivy2333/RTsql.git
cd RTsql
./install.sh
source ~/.bashrc
rtsql --version
```

`install.sh` 会构建 release 二进制，将它安装到 `~/.local/bin`，在当前 shell 和对应工具可用时安装补全，并在需要时将安装目录写入 `~/.bashrc`。脚本不使用 `sudo`，也不调用单独的下载工具；构建时 Cargo 可能获取已声明的 Rust 依赖。

可指定其他安装前缀，或跳过补全安装：

```bash
./install.sh --prefix "$HOME/.local-rtsql"
./install.sh --prefix "$HOME/.local-rtsql" --no-completions
```

`--prefix` 只控制二进制位置；补全文件仍写入各 shell 的标准用户目录。

### 交叉构建 RISC-V 64 Linux（musl）

先安装固定 Rust target，再从仓库根目录构建：

```bash
rustup target add riscv64gc-unknown-linux-musl
./build-riscv64-musl.sh
```

默认输出目录是 `dist/riscv64gc-unknown-linux-musl/`。可指定其他输出根目录：

```bash
./build-riscv64-musl.sh --output-dir /tmp/rtsql-dist
```

成功执行后，该目录包含 `rtsql` 二进制、收录二进制与双语 README 的版本化 `.tar.gz`，以及 `SHA256SUMS`。可在构建宿主机校验归档：

```bash
cd dist/riscv64gc-unknown-linux-musl
sha256sum --check SHA256SUMS
```

脚本依赖 `riscv64-linux-musl-gcc`、GNU tar 与 coreutils `sha256sum`。交叉链接器只传给当前构建子进程，并启用 `+crt-static`，不修改全局 Cargo 配置。脚本不使用 `sudo`、不上传产物，也不执行 RISC-V 二进制。部署时将归档和校验文件复制到目标设备，再在真实 RISC-V 硬件上执行版本、CRUD、加密与恢复检查；仅完成交叉构建不代表这些运行检查已经通过。

### 建库与查询

```bash
rtsql new demo
rtsql demo "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)"
rtsql --format table demo "INSERT INTO people VALUES (1, 'Ada', 36), (2, 'Lin', 41)"
rtsql --format table demo "SELECT id, name, age FROM people WHERE age >= 40"
rtsql schema demo
```

裸数据库名解析为 `$RTSQL_HOME/db/<name>.db`；默认基目录是 `$HOME/.rtsql`。参数中包含 `/` 时，RTsql 将其直接作为文件路径。

### 使用 SQL 事务

每次 CLI 调用只执行一次。需要把 `BEGIN`、事务语句和 `COMMIT` 或 `ROLLBACK` 放在同一个 SQL 参数中：

```bash
rtsql --format table demo "BEGIN; INSERT INTO people VALUES (3, 'Kai', 22); COMMIT;"
rtsql --format table demo "BEGIN; INSERT INTO people VALUES (4, 'Mira', 29); ROLLBACK;"
rtsql --format table demo "SELECT id, name FROM people ORDER BY id"
```

没有显式事务时，分号分隔的语句会逐条自动提交。输入结束时若仍有活动事务，RTsql 会回滚该事务，并在 stderr 提示回滚。

### 备份与恢复

```bash
rtsql dump demo > demo.sql
rtsql new demo-restored
rtsql restore demo-restored demo.sql
rtsql --format table demo-restored "SELECT id, name, age FROM people ORDER BY id"
```

`dump` 输出 SQL 文本，不会加密该文件；应按明文数据库导出数据保护。`restore` 要求目标数据库为空；文件参数使用 `-` 时从 stdin 读取 dump。

### 创建加密数据库

```bash
rtsql new secure --key 'replace-with-a-password'
rtsql --key 'replace-with-a-password' secure "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT)"
rtsql --key 'replace-with-a-password' secure "INSERT INTO notes VALUES (1, 'private')"
rtsql --key 'replace-with-a-password' secure "SELECT id, body FROM notes"
```

`RTSQL_KEY` 与 `--key` 等效；两者同时存在时，显式 `--key` 优先：

```bash
RTSQL_KEY='replace-with-a-password' rtsql secure "SELECT id, body FROM notes"
```

通过导出和导入迁移数据库：

```bash
rtsql dump demo > demo.sql
rtsql new secure-copy --key 'replace-with-a-password'
rtsql --key 'replace-with-a-password' restore secure-copy demo.sql
```

未提供密钥打开加密库、向明文库提供密钥、使用错误密钥，都会返回退出码 5。空密钥会在打开数据库前以退出码 2 拒绝。

### 卸载

在源码检出目录执行：

```bash
./install.sh --uninstall
```

该命令删除已安装的二进制和补全文件，保留 `$RTSQL_HOME`；未设置时默认保留 `$HOME/.rtsql`。

```bash
./install.sh --uninstall --purge-data
```

`--purge-data` 会先打印并删除数据目录。RTsql 无法恢复被该命令删除的数据。

## CLI 命令面

运行 `rtsql --help` 可查看内置帮助。主命令格式为：

```text
rtsql [OPTIONS] [DB] [SQL] [COMMAND]
```

| 命令 | 用途 |
|---|---|
| `rtsql <db> <sql>` | 执行一条 SQL 或分号分隔的 SQL 脚本 |
| `rtsql new <target>` | 创建空的裸名数据库或路径数据库 |
| `rtsql list` | 列出集中存储目录中的数据库 |
| `rtsql schema <db>` | 输出用户表 DDL |
| `rtsql dump <db>` | 将 DDL 和数据导出为 SQL 文本 |
| `rtsql restore <db> <file>` | 将 dump 恢复到空数据库；`-` 表示 stdin |
| `rtsql import <db> <table> <file> --csv` | 按表头列名导入 CSV |
| `rtsql stats <db> <table>` | 汇总行数、空值率、去重值、范围和数值分位数 |
| `rtsql sample <db> <table> [n]` | 使用蓄水池抽样；默认 `n` 为 10 |
| `rtsql profile <db> <table> [--top n]` | 输出列画像和 String 高频值；默认 top 为 5，最大 20 |
| `rtsql completions <bash\|zsh\|fish>` | 生成补全脚本；该命令不出现在 `--help` 中 |

### 输出格式

`--format` 接受 `table`、`json`、`csv` 和 `tsv`。未显式指定时，交互式终端默认使用 `table`，其他环境默认使用 `json`。

### 退出码

| 退出码 | 含义 |
|---:|---|
| 0 | 成功 |
| 1 | 一般 I/O、存储或格式错误 |
| 2 | CLI 用法错误 |
| 3 | SQL 解析或执行错误 |
| 4 | 数据库已被其他进程锁定 |
| 5 | 加密密钥错误 |
| 130 / 143 | 被 SIGINT / SIGTERM 终止 |

## SQL 与引擎能力

- **DDL 与 DML：** `CREATE TABLE`、`DROP TABLE`、`SELECT`、`INSERT`、`UPDATE`、`DELETE`。
- **查询：** `WHERE`、`JOIN`、`GROUP BY`、`HAVING`、`ORDER BY`、`LIMIT`、`OFFSET`。
- **表达式：** `IN`、`BETWEEN`、`LIKE`、`IS NULL`、`NOT`、`CASE`、`COALESCE`、`CAST` 和算术运算符。
- **标量函数：** `upper`、`lower`、`length`、`substr`、`replace`、`trim`、`abs`、`round`、`floor`、`ceil`。
- **日期时间：** `DATE`、`TIMESTAMP`、`now`、`date`、`year`、`month`、`day`、`hour`、`minute`、`second`、`date_trunc`、`datediff` 和 `INTERVAL` 算术。
- **子查询：** 标量子查询、`IN`、`EXISTS`、派生表和关联子查询；相同关联参数可复用语句级结果。
- **分组：** 可按列、别名、表达式文本或序号执行 `GROUP BY`。
- **常量查询：** 支持 `SELECT 1 + 1` 等无 `FROM` 表达式。
- **事务：** 显式库 API 事务，以及 CLI `BEGIN` / `COMMIT` / `ROLLBACK` 会话。
- **MVCC：** 默认 Repeatable Read，也可通过 `Database::open_with_isolation` 选择 Read Committed。
- **存储：** 持久化 schema、B-Tree 主键索引、带帧校验和的 WAL、checkpoint、崩溃恢复和页复用。

引擎使用显式类型检查和三值谓词逻辑，不会隐式转换不兼容的 SQL 类型。

## 加密模型

加密数据库以 64 字节明文头开始，后续保存加密页记录。文件头包含随机 32 字节 Argon2id 盐和持久化 KDF 参数。每个 4096 字节页写为 4124 字节记录，包含 12 字节 nonce、密文和 16 字节 AES-GCM 认证 tag；page ID 作为附加认证数据绑定到记录。

`.wal` 和 `.checkpoint` 伴生文件保持原有明文格式。加密主数据库不会加密 dump 输出或这些伴生文件。

## 库 API

主要入口如下：

- `Database::open(path)`：以 Repeatable Read 打开或创建数据库。
- `Database::open_with_isolation(path, level)`：选择 Repeatable Read 或 Read Committed。
- `Database::open_with_key(path, isolation, key)`：打开明文或加密存储。
- `Database::execute_sql(sql)`：执行 SQL。
- `Database::execute_in_tx(...)`：在显式库事务中执行。
- `Database::checkpoint()`：刷写 checkpoint。
- `Database::close()`：刷写、执行 checkpoint 并释放文件锁。

## 架构

SQL 文本由 `sqlparser-rs` 解析，转换为物理计划，再由 Volcano 风格执行器树运行。执行管道通过基于 DashMap 的缓冲池和 B-Tree 主键索引读取符合 MVCC 可见性的行。固定大小的 slotted page 保存序列化行和版本链。WAL 记录提供 redo 恢复，checkpoint 持久化安全重放位置并限制 WAL 增长。页加密封装在 `FileStorage` 中；缓冲池和执行器只处理普通 4096 字节页镜像。

## 构建与测试

```bash
cargo build --release
cargo test --no-fail-fast
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo bench
```

仓库的 `benches/` 目录还包含 Criterion 基准和 SQLite 对比基准。

## 已知限制

- 支持 Linux 和 macOS；尚未实现 Windows 文件 I/O。
- 只加密主数据库文件；WAL、checkpoint 和 dump 输出仍为明文。
- 不支持原地切换明文/加密状态；请使用 `dump` 和 `restore`。
- 不包含密钥轮换、密钥管理子命令、密钥缓存和 `--password-file`。
- 打开加密数据库时会执行 Argon2id。2026-09-24 记录的一次小库 dev profile 样本约为 0.35–0.39 秒，明文打开低于 1 毫秒；该数字仅是现场观察，不是基准保证。
- 支持 Repeatable Read 和 Read Committed，不支持 serializable。
- 不包含窗口函数、自定义函数、时区类型、`TIMESTAMPTZ` 和 `INTERVAL` 存储列。
- 不提供交互式 REPL、多用户角色模型或远程访问控制。
- 当前交付面不包含 CI、预编译 Release、`cargo install` 发布和生成的 man page。

## 文档

- [English README](README.md)
- [Agent 操作手册](rtsql-docs/SKILL.md)
- [项目快照](.claude/docs/SNAPSHOT.md)
- [项目路线](.claude/docs/tasks.md)
- [OpenSpec 行为规格](openspec/specs/)

Cargo 包元数据声明的许可为 `MIT OR Apache-2.0`。
