# RTsql

[English](README.md) | [简体中文](README.zh-CN.md) | [Agent 操作手册](rtsql-docs/SKILL.md)

RTsql 是一个用 Rust 编写的嵌入式关系型数据库。它使用 Tokio 任务执行异步 I/O，以单个主文件保存数据库，并通过一次性执行的 CLI 提供 SQL 执行和管理能力。

当前仓库支持 Linux 和 macOS，并可交叉构建静态 RISC-V 64 Linux（musl）二进制；不依赖独立数据库服务。

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

在 Linux 构建机上，RTsql 可交叉编译静态 RISC-V 64 二进制并打包待部署——脚本自身不执行 RISC-V 二进制：

```bash
rustup target add riscv64gc-unknown-linux-musl
./build-riscv64-musl.sh
```

目标固定为 `riscv64gc-unknown-linux-musl`；构建使用 `riscv64-linux-musl-gcc` 链接并启用 `+crt-static`，产物是完全静态的 RISC-V ELF，不依赖动态解释器。前置条件（`cargo`、`rustup`、`riscv64-linux-musl-gcc` 和 GNU `tar`）在构建前统一检查：缺失项会连同安装提示一起报告并停止——脚本不会自动安装、不使用 `sudo`、不上传任何产物，也不写全局 Cargo 配置。

成功执行后，产物发布到 `dist/riscv64gc-unknown-linux-musl/`：

- `rtsql`：strip 后的静态二进制
- `rtsql-v<version>-riscv64gc-unknown-linux-musl.tar.gz`：收录二进制与双语 README

`--output-dir DIR` 可指定其他输出根目录，`--help` 查看内置说明。将归档复制到目标设备后部署。交叉构建成功不等于运行验证：版本、CRUD、加密与恢复检查必须在真实 RISC-V 硬件上执行，本仓库不因交叉构建成功而声明这些运行检查通过。

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

- **DDL 与 DML：** `CREATE TABLE`、`DROP TABLE`、`SELECT`、`INSERT`（含 `ON CONFLICT DO NOTHING` / `DO UPDATE` 与 `REPLACE INTO`）、`UPDATE`、`DELETE`。
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

### 约束

`CREATE TABLE` 接受的每一条约束要么被执行面强制，要么在计划期点名拒绝——声明的约束绝不会被静默忽略。

- **NOT NULL** 在 `INSERT` 与 `UPDATE` 强制。违反的写入以 `NOT NULL constraint violation: column '<列名>'` 失败，零副作用。
- **UNIQUE** 对 `INT` 列强制（主键列除外），经专属内部唯一索引实现；重复值以 `Duplicate key` 拒绝，`NULL` 豁免，且重启与崩溃恢复后强制保持。声明在主键列上的 `UNIQUE` 由主键既有唯一性消费，不建第二索引。
- **计划期点名拒绝（表不创建）：** `CHECK`、`FOREIGN KEY`、方言特定列选项、非 `INT` 列的 `UNIQUE`、组合（多列）`UNIQUE`。
- **存量兼容：** 旧版本创建的、含非 `INT` `UNIQUE` 列的数据库可正常打开（这些列不被强制）；但把这类表 dump 后 restore 进新库会被上述计划期规则拒绝。在 `NOT NULL` 强制落地前写入的、「NOT NULL 列含 NULL」的存量行同样会在 restore 时被拒。请先调整 schema（把非 `INT` `UNIQUE` 改为 `INT` 或去掉声明）并清理或回填此类数据，再执行 restore。

### 写面

`INSERT` 与 `UPDATE` 接受 SQLite 子集的写面。被拒绝的写入零副作用：`INSERT`、`UPDATE`、`DO NOTHING` 与 `DO UPDATE` 在触行之前完成全部校验；`REPLACE INTO` 在删除冲突行之后才校验失败时经语句回滚，被删行的主键与唯一索引条目一并还原。

- **列清单：** `INSERT INTO t (col, ...) VALUES ...` 接受表列的任意子集且顺序任意。省略列取声明 `DEFAULT`，无声明则取 `NULL`；NOT NULL 列这样被填入 `NULL` 时点名拒绝。`VALUES` 中的 `DEFAULT` 等价于省略该列。声明默认值经 catalog 持久化、重启后生效，并由 `rtsql dump` 与 `rtsql schema` 渲染。
- **值类型：** 写入值必须与列声明类型一致。`NULL` 豁免，日期列接受其类型化字面量，`FLOAT` 列接受整数值（以等价浮点存储），其余跨类型写入以 `column '<列名>' expects <类型>, got <类型>` 拒绝。`dump` / `restore` / `import` 通道经同一规则。
- **`ON CONFLICT DO NOTHING`：** 冲突行被跳过，不计入受影响行数。
- **`ON CONFLICT (col) DO UPDATE SET ...`：** 冲突行原位更新。右值接受字面量、`DEFAULT`、`excluded.<列名>`（待插行值）或裸列名（冲突行旧值），可一次赋值多列。仅被赋值列发生变化。
- **冲突目标：** 省略目标时仲裁主键与全部唯一索引（先主键，再按声明序的唯一列）。显式目标必须是承载唯一性的单列——`INT` 主键列或 `UNIQUE` 列。
- **`REPLACE INTO`：** 删除全部冲突行后插入新行；每个插入行计一个受影响行。
- **计划期点名拒绝（CLI exit 3）：** 组合与非唯一冲突目标，报 `ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint`；`ON CONFLICT ON CONSTRAINT`；`DO UPDATE WHERE`；MySQL 的 `ON DUPLICATE KEY UPDATE`；`REPLACE INTO` 与 `ON CONFLICT` 子句并存；赋值右值除字面量、`DEFAULT`、`excluded.<列名>` 与列引用之外的一切形态（含算术与函数表达式）。`INSERT OR ...` 方言在本引擎 SQL 方言下不可达。
- 三个动作都维护主键与唯一索引条目，结果在重启与崩溃恢复后保持。

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

仓库的 `benches/` 目录还包含 Criterion 基准和 SQLite 对比基准。RISC-V 64 Linux（musl）产物由 `build-riscv64-musl.sh` 生成，见上文「交叉构建 RISC-V 64 Linux（musl）」章节。

## 性能与资源对比

2026-09-24 在 Intel i9-13900HX（32 线程）Linux 机器上实测：RTsql 0.1.0（release 构建）对比 SQLite 3.37.2（`rusqlite` 链接的系统 `libsqlite3`，以及 `sqlite3` CLI）。两个引擎均使用默认配置，作用于同一张合成表 `bench (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)`；RTsql 每条语句经 WAL 自动提交，SQLite 使用默认回滚日志。以下为单机现场观察，不是基准保证。

引擎级操作（Criterion 进程内基准、直接调用引擎 API，20 样本均值；复现命令 `cargo bench --bench sqlite_compare`）：

| 操作 | RTsql | SQLite | 更快 |
|---|---|---|---|
| INSERT 100 行（逐条提交） | 4.8 ms（约 20,800 行/秒） | 251.3 ms（约 400 行/秒） | RTsql ~52x |
| 主键点查（1k 行表） | 1.57 µs | 6.80 µs | RTsql ~4.3x |
| 1k 行全表扫描 | 297 µs | 98 µs | SQLite ~3x |

CLI 级资源占用（一次性进程，加载 2,000 行后执行 `SELECT COUNT(*)`）：

| 指标 | RTsql | sqlite3 |
|---|---|---|
| 加载耗时（2,000 条 INSERT） | 3.2 s | 5.3 s |
| 峰值内存（RSS） | ~16.7 MiB | ~4.1 MiB |
| 关闭后主数据库文件 | 300 KiB | 48 KiB |
| 单次 `SELECT 1` 时延（50 次均值） | 10.8 ms | 1.2 ms |
| 二进制体积 | 6.7 MB | 1.6 MB |

解读：RTsql 的逐条写入路径与索引点查较快，但全表扫描目前慢于 SQLite，一次性调用还承载异步运行时与关闭时 checkpoint 的固定开销。SQLite 成熟三十余年，以上仅为 RTsql 当前位置的快照；下结论前请在自己的硬件上复跑基准。

## 已知限制

- 支持 Linux 和 macOS 构建宿主；尚未实现 Windows 文件 I/O。RISC-V 64 Linux 以静态 musl 交叉构建产物交付，尚未在真实 RISC-V 硬件上验证。
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
