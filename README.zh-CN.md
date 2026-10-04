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

#### 备份边界与数据库文件族

备份首选 `dump` / `restore` 逻辑导出。磁盘上的一个数据库是由同一名字派生的三个文件组成的文件族——`<db>.db`、`<db>.wal`、`<db>.checkpoint`——文件级拷贝是否安全只取决于你拷到的是哪个状态。

**只在写入方干净关闭之后**（one-shot 命令在 close 时 checkpoint，WAL 已截断），单独拷贝 `<db>.db` 才等价于完整备份。写入进行中或崩溃之后，主文件只含已 checkpoint 的前缀：下表数据来自一个提交了 938 行中 937 行后被 `SIGKILL` 的会话，只拷主文件得到 **938 行中的 1 行**——静默不完整，打开时没有任何报错。

| 形态 | 结果 |
|---|---|
| 干净关闭后只拷 `<db>.db` | 完整（1 行 / 1 行） |
| 干净关闭后拷 `<db>.db` + `<db>.wal` | 完整 |
| 干净关闭后拷 `<db>.db` + `<db>.checkpoint` | 完整 |
| 有未 checkpoint 尾部时只拷 `<db>.db` | **不完整：938 行中的 1 行** |
| 有未 checkpoint 尾部时成对拷 `<db>.db` + `<db>.wal` | 完整（938 行，打开时经 WAL 重放） |
| 成对移动到另一目录（文件名不变） | 完整（938 行） |
| 只重命名主文件（`.wal` 留在原处） | **不完整：938 行中的 1 行**，且孤立的 `<db>.wal` 被忽略 |

伴生文件名由主文件名派生，因此任何重命名或移动都必须让 `<db>.wal` 与 `<db>.checkpoint` 随主文件以同一基名同行。写入进行中的文件级拷贝必须包含 `<db>.wal`；否则先让写入方关闭——或运行 `rtsql checkpoint <db>` 将 WAL 折入主文件——再对副本执行 `rtsql <db> "SELECT count(*) FROM t"` 确认。

`rtsql delete <db>` 删除单个数据库文件族；`./install.sh --uninstall --purge-data` 在卸载时清空整个数据目录。两者均不可通过 RTsql 恢复。

### 删除数据库

```bash
rtsql delete demo --dry-run
rtsql delete demo
```

`delete` 删除 `<db>.db` 及其 `.wal`、`.checkpoint` 伴生文件，逐文件报告释放的字节数并以退出码 0 结束。它是文件级管理操作：不开库、不校验内容，加密库也无需 `--key`。库正被其他会话打开时以退出码 4 显式拒绝且不删除任何文件；库不存在时以退出码 1 拒绝。伴生文件本就不存在时报 `absent`，不算错误。

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
| `rtsql import <db> <table> <file\|-> --csv` | 按表头列名导入 CSV，或按表头自动建表；`-` 表示 stdin；接受 `:memory:` 目标 |
| `rtsql delete <db> [--dry-run]` | 删除数据库及其 `.wal` / `.checkpoint` 伴生文件；`--dry-run` 只列出 |
| `rtsql checkpoint <db>` | 将已提交数据刷入主文件（截断 WAL、推进 checkpoint 位点）；文件级备份或交接前的干净同步点 |
| `rtsql check <db>` | 只读结构完整性扫描（页 census、catalog、数据链/溢出链、索引树）；损坏按行报告页位置与损坏语义，干净库输出单行确认。退出码：干净 `0`、存在 error 级 finding `1`（info 级不改退出码）、锁冲突 `4`、密钥错误 `5`；`:memory:` 与 `--readonly` 组合为用法错误（`2`）。不修复、不修改文件；存在未决 WAL 的库须先以读写会话打开完成恢复后再检查 |
| `rtsql stats <db> <table>` | 汇总行数、空值率、去重值、范围和数值分位数 |
| `rtsql sample <db> <table> [n]` | 使用蓄水池抽样；默认 `n` 为 10 |
| `rtsql profile <db> <table> [--top n]` | 输出列画像和 String 高频值；默认 top 为 5，最大 20 |
| `rtsql completions <bash\|zsh\|fish>` | 生成补全脚本；该命令不出现在 `--help` 中 |

### 只读会话

`rtsql --readonly <db> <sql>` 以 advisory 共享锁打开数据库（默认读写打开为独占锁），因此多个只读会话可以同时观察同一个文件，而写会话仍然独占。

- **多读单写**：只读会话之间共享锁；读写会话排除只读者与其他写者，只读会话也排除读写会话。锁冲突按可配置预算等待——`--busy-timeout <毫秒>` 或 `RTSQL_BUSY_TIMEOUT`，默认 5000 ms；`0` 仅等待内部 ~8 ms 可见性下限窗口——预算耗尽后才以退出码 `4` 拒绝。该预算对全部开库命令生效，独占锁与只读共享锁同侧；`:memory:` 接受并忽略，`delete` 的锁探测永不等待。
- **无未决 WAL**：只读会话不执行恢复，因此 `.wal` 中仍有未重放记录时拒绝打开，错误文案点名需以读写模式完成恢复，退出码为 `1`。
- **零写入**：不运行恢复也不 checkpoint，`INSERT` / `UPDATE` / `DELETE` / `UPSERT` / `REPLACE` 与全部 DDL 在计划期以 SQLSTATE `25006` 拒绝（退出码 `3`）；`SELECT`、`ATTACH`、`DETACH` 与事务语句不受限。
- **跨库**：只读会话内的 `ATTACH` 将附加库也以只读模式打开。
- **加密库**：照常传入 `--key`；密钥错误仍为退出码 `5`。
- **仅主命令**：`--readonly` 与生命周期或分析子命令组合是用法错误（退出码 `2`）——这些子命令维持读写打开。

### 内存数据库

`rtsql :memory: <sql>` 让整个会话在内存中运行。SQL 面完整可用（含 DDL、事务与 `ATTACH`），且不创建任何文件：没有 `.db`、`.wal` 或 `.checkpoint` 伴生文件，`close()` 与 `checkpoint()` 都是无操作，进程退出即丢弃全部数据。

生命周期子命令、`--key` 与 `--readonly` 对 `:memory:` 都是用法错误（退出码 `2`）。

### 跨库会话

一个 CLI 会话可用 `ATTACH '<路径>' AS <别名>` 注册其他数据库文件，并用限定名（`别名.表`）访问；裸名始终指向主库。会话在其生命周期内持有每个附加文件的 advisory 锁，`--busy-timeout` 预算对这些锁的等待与对主文件完全一致。

```bash
rtsql app "ATTACH 'orders' AS o; \
  INSERT INTO o.recent SELECT id, name FROM t WHERE id > 100; \
  SELECT COUNT(*) FROM o.recent"
```

- **写入路由到附加文件**：对 `别名.表` 的 `INSERT`、`UPDATE`、`DELETE` 及 upsert 形态在该文件上执行，使用其自身的事务、WAL 与缓冲池。每个文件独立提交：某条语句失败时，之前的语句保持已提交——包括写到其他文件的那些。
- **无跨文件事务**：`BEGIN` 内对附加库的写入在任何行被触碰之前以 SQLSTATE `0A000`（退出码 `3`）拒绝。主库写入保持完整的 `BEGIN` / `COMMIT` / `ROLLBACK` 语义，显式主库事务内可以读取附加库。
- **DDL 仅限主库**：附加库只接受查询与 DML——对附加目标的 `CREATE TABLE`、`DROP TABLE`、`CREATE TABLE AS SELECT` 及其他 DDL 在计划期点名拒绝。
- **`INSERT ... SELECT`**：`INSERT INTO <既有表> SELECT ...` 先完整物化源行，再走常规插入路径写入。源可以跨库 JOIN、写回自身目标表（只消费语句开始前读到的行），并支持 `ON CONFLICT DO NOTHING` / `DO UPDATE` 或 `REPLACE INTO`。显式列清单按位置映射，省略列取声明的 `DEFAULT` 或 `NULL`；源失败则目标零变化，空源报告 `affected_rows: 0`。
- **只读会话**以只读模式打开附加库，所有写入照常以 SQLSTATE `25006` 拒绝。

### 按 WAL 大小自动 checkpoint

读写会话在「自上次 checkpoint 以来累计追加的字节数」达到水位时自动 checkpoint，使长会话的 `.wal` 保持有界。检查发生在每个提交边界（auto-commit 语句提交后、显式 `COMMIT` 后）；同一会话内仍有其他活跃事务时顺延到后续提交边界。

- **默认值**：16 MiB。常规会话远低于该值，行为与既有语义一致——只有 `close()` 与显式 checkpoint 才截断 WAL。
- **显式命令**：`rtsql checkpoint <db>` 按需执行同样的截断并输出一行确认——干净库报告 `no pending work` 且仍以退出码 `0` 结束。外部备份或文件交接前以它为同步点。
- **环境变量**：`RTSQL_WAL_CHECKPOINT_BYTES=<字节数>` 覆盖水位；库内经 `Database::set_checkpoint_bytes_threshold` 可设置同一值。取 `0` 表示禁用自动 checkpoint——WAL 持续增长，`close()` 仍执行完整 checkpoint。
- **非法值**：非数字、负数与溢出在打开数据库**之前**拒绝，报错文案点名变量名并回显非法值（退出码 `2`），不创建任何文件。
- **失败传播**：自动 checkpoint 失败时，触发该边界的语句收到点名 checkpoint 失败的错误（退出码 `3`）。该语句已提交的数据保持可用——提交先于 checkpoint 完成 fsync，后续提交边界会自然重试。
- **只读与内存会话**：两者都不持有 WAL 通道，该设置对其无效。

### 输出格式

`--format` 接受 `table`、`json`、`csv` 和 `tsv`。未显式指定时，交互式终端默认使用 `table`，其他环境默认使用 `json`。

### 退出码

| 退出码 | 含义 |
|---:|---|
| 0 | 成功 |
| 1 | 一般 I/O、存储或格式错误 |
| 2 | CLI 用法错误 |
| 3 | SQL 解析或执行错误 |
| 4 | 数据库已被其他进程锁定（`--busy-timeout` 预算耗尽后） |
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
- **聚合表达式：** 聚合可嵌套于 SELECT 值表达式（`SELECT MAX(id) + 1`）、`HAVING` 条件（`HAVING MAX(id) + 1 > 2`）与聚合查询 `ORDER BY` 键（`ORDER BY MAX(v)`）；选择清单可与纯常量混排（`SELECT MAX(id), 1`）；同一聚合调用多次出现共享一个输出列。`WHERE`/`JOIN ... ON` 内聚合、聚合嵌套、无 `GROUP BY` 的非聚合列引用在计划期点名拒绝。
- **常量查询：** 支持 `SELECT 1 + 1` 等无 `FROM` 表达式。
- **事务：** 显式库 API 事务，以及 CLI `BEGIN` / `COMMIT` / `ROLLBACK` 会话。
- **MVCC：** 默认 Repeatable Read，也可通过 `Database::open_with_isolation` 选择 Read Committed。
- **存储：** 持久化 schema、B-Tree 主键索引、带帧校验和的 WAL、checkpoint、崩溃恢复和页复用。
- **长值存储：** 超过单页内联容量（约 4KB）的行值自动经溢出页链存取，无人为长度上限。溢出对 SQL 层透明：`INSERT`/`UPDATE`/`SELECT`、`dump`/`restore`/`import`、崩溃恢复、GC 与 `DROP TABLE`/`TRUNCATE` 释放、加密库、`:memory:` 与只读会话同等可用（此前超过页容量的行以 `No enough space in page` 失败，该失败形态已终止）。
- **运行形态：** 共享锁上的只读会话（多个并发读者、零写入），以及全程驻留内存的 `:memory:` 数据库。

引擎使用显式类型检查和三值谓词逻辑，不会隐式转换不兼容的 SQL 类型。

### 约束

`CREATE TABLE` 接受的每一条约束要么被执行面强制，要么在计划期点名拒绝——声明的约束绝不会被静默忽略。

- **NOT NULL** 在 `INSERT` 与 `UPDATE` 强制。违反的写入以 `NOT NULL constraint violation: column '<列名>'` 失败，零副作用。
- **UNIQUE** 对 `INT` 列强制（主键列除外），经专属内部唯一索引实现；重复值以 `Duplicate key` 拒绝，`NULL` 豁免，且重启与崩溃恢复后强制保持。声明在主键列上的 `UNIQUE` 由主键既有唯一性消费，不建第二索引。
- **PRIMARY KEY** 必须声明在 `INTEGER` 列上——这是引擎强制的唯一形态。显式非整数 `PRIMARY KEY` 计划期点名拒绝；未声明主键的表沿用首列隐式主键既有语义，零变化。
- **计划期点名拒绝（表不创建）：** `CHECK`、`FOREIGN KEY`、方言特定列选项、显式非 `INTEGER` `PRIMARY KEY`、非 `INT` 列的 `UNIQUE`、组合（多列）`UNIQUE`。
- **存量兼容：** 旧版本创建的、含非 `INT` `UNIQUE` 列的数据库可正常打开（这些列不被强制）；但把这类表 dump 后 restore 进新库会被上述计划期规则拒绝。 以非整数 `PRIMARY KEY` 声明的存量表同样可正常打开、读取与 dump，且 dump DDL 不渲染 `PRIMARY KEY`（`dump` → `restore` 往返可达）；旧版本生成的、内嵌非整数 `... PRIMARY KEY` DDL 的 dump 在新库 restore 失败（该声明从未被强制）。在 `NOT NULL` 强制落地前写入的、「NOT NULL 列含 NULL」的存量行同样会在 restore 时被拒。请先调整 schema（把非 `INT` `UNIQUE` 改为 `INT` 或去掉声明）并清理或回填此类数据，再执行 restore。

### 写面

`INSERT` 与 `UPDATE` 接受 SQLite 子集的写面。被拒绝的写入零副作用：`INSERT`、`UPDATE`、`DO NOTHING` 与 `DO UPDATE` 在触行之前完成全部校验；`REPLACE INTO` 在删除冲突行之后才校验失败时经语句回滚，被删行的主键与唯一索引条目一并还原。

- **列清单：** `INSERT INTO t (col, ...) VALUES ...` 接受表列的任意子集且顺序任意。省略列取声明 `DEFAULT`，无声明则取 `NULL`；NOT NULL 列这样被填入 `NULL` 时点名拒绝。`VALUES` 中的 `DEFAULT` 等价于省略该列。声明默认值经 catalog 持久化、重启后生效，并由 `rtsql dump` 与 `rtsql schema` 渲染。
- **值类型：** 写入值必须与列声明类型一致。`NULL` 豁免，日期列接受其类型化字面量，`FLOAT` 列接受整数值（以等价浮点存储），其余跨类型写入以 `column '<列名>' expects <类型>, got <类型>` 拒绝。`dump` / `restore` / `import` 通道经同一规则。
- **声明默认值：** `CREATE TABLE` 或 `ALTER TABLE ADD COLUMN` 声明的 `DEFAULT` 字面量在计划期按同一规则与列类型校验——`NULL` 豁免，`FLOAT` 列整数字面量升格（以浮点持久化），`DATE`/`TIMESTAMP` 列解析字符串（非法日期拒绝），其余跨类型声明以 `DEFAULT type mismatch: column '<列名>': expected <类型>, got <类型>` 在建表前拒绝、零副作用。类型化日期字面量（如 `DEFAULT DATE '2026-01-02'`）可接受，含日期默认值的表 `dump` → `restore` 往返保真。`DEFAULT` 字面量长度另受 catalog 列行页内容量约束（约 4KB 量级）：超长默认值在建表时按既有页满错误点名拒绝、零副作用；写入值本身不受此限制（超页行走溢出页链）。
- **`UPDATE ... SET` 表达式：** 可一次赋值多列，右值接受完整值表达式——列引用按旧行值同快照求值（`SET a = a + 1, b = a` 中 `b` 读旧 `a`），另支持算术、`CAST`、`CASE`、`COALESCE` 与标量函数。写入类型门、`NOT NULL`、键列类型门、碰撞预检、rekey 与唯一/次级索引维护全部作用于求值结果；任一赋值失败整行保持原状。右值为聚合调用或子查询时计划期点名拒绝。
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
- `Database::open_with_lock_wait(path, isolation, key, lock_wait)`：以可配置锁等待预算打开（`None` = 默认 5000 ms）。
- `Database::open_readonly_with_lock_wait(path, key, lock_wait)`：同一预算的只读打开。
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
| 峰值内存（RSS） | ~16.4 MiB | ~4.1 MiB |
| 关闭后主数据库文件 | 300 KiB | 48 KiB |
| 单次 `SELECT 1` 时延（50 次均值） | 6.5 ms | 1.2 ms |
| 二进制体积 | 6.7 MB | 1.6 MB |

单次时延与峰值内存为 2026-09-26 重测值（零工作关闭跳过落地后：只读会话关闭不再执行 checkpoint；经实测评估保留默认多线程运行时）。加载耗时行保留 2026-09-24 数值，写入路径未变化。

解读：RTsql 的逐条写入路径与索引点查较快，但全表扫描目前慢于 SQLite，一次性调用还承载异步运行时的固定开销，只读关闭不再叠加 checkpoint 开销（零工作关闭整体跳过）。SQLite 成熟三十余年，以上仅为 RTsql 当前位置的快照；下结论前请在自己的硬件上复跑基准。

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
