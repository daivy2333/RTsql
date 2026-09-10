# cli-noninteractive-shell Specification

## MODIFIED Requirements

### Requirement: 参数化 CLI 入口与主命令

`rtsql` 二进制 SHALL 提供 `rtsql <db> <sql>` one-shot 主命令：解析参数、打开数据库、执行 SQL（单条语句，或以 `;` 分隔的多条语句逐条执行）、渲染结果到 stdout、以分类退出码退出。进程正常退出前 SHALL 调用 `Database::close()`（checkpoint + WAL 截断）。主命令参数缺失或非法时 SHALL 以退出码 2 报用法错误。打开时遇跨进程锁冲突 SHALL 以退出码 4 报 `database is locked`。

`rtsql` SHALL 同时提供生命周期子命令 `new` / `list` / `schema` / `dump` / `restore` / `import`：第一个位置参数命中子命令名时 SHALL 分发到对应子命令（子命令优先），未命中时按主命令位置参数解析。子命令参数缺失或非法时 SHALL 以退出码 2 报用法错误。裸名与子命令名同名的数据库 SHALL 以含 `/` 的路径形式经主命令打开（冲突行为文档化）。

#### Scenario: one-shot SELECT 执行成功

- **GIVEN** 集中存储或指定路径下存在含数据的库
- **WHEN** `rtsql <db> "SELECT id, name FROM t WHERE id = 1"`
- **THEN** stdout 输出查询结果（含列名表头），退出码 0
- **AND** 进程退出前完成了 checkpoint（WAL 被截断，重开无 redo 负担）

#### Scenario: 用法错误退出码 2

- **GIVEN** 任意环境
- **WHEN** `rtsql`（无参数）或 `rtsql --format bogus db "SELECT 1"`（非法选项值）或 `rtsql <db>`（缺 SQL 参数）
- **THEN** stderr 输出用法信息，退出码 2，不打开任何数据库

#### Scenario: SQL 错误退出码 3

- **GIVEN** 已打开的库
- **WHEN** `rtsql <db> "SELEC typo"` 或 `rtsql <db> "SELECT * FROM missing_table"`
- **THEN** stderr 输出错误信息，退出码 3

#### Scenario: 锁冲突退出码 4

- **GIVEN** 某持有者已锁定目标库文件
- **WHEN** `rtsql <db> "SELECT 1"`
- **THEN** stderr 输出以 `database is locked` 开头的错误信息（含目标路径），退出码 4，且不执行任何 SQL
- **AND** 密钥错误退出码 5 仍为枚举留位（MS12 落地）

#### Scenario: 退出码枚举为后续任务留位

- **GIVEN** CLI 退出码枚举（0/2/3/4/5）
- **WHEN** 本 capability 落地后的代码审查
- **THEN** 退出码 4（锁冲突）已由本 Requirement 的锁冲突场景获得产生路径（T02 落地）
- **AND** 退出码 5（密钥）已存在于枚举与映射表中，但尚无产生路径（MS12 落地）

#### Scenario: 子命令分发与主命令零回归

- **GIVEN** 任意环境
- **WHEN** 分别执行 `rtsql list`、`rtsql new foo`、`rtsql schema foo`、`rtsql dump foo`、`rtsql restore foo dump.sql`、`rtsql import foo t data.csv --csv`
- **THEN** 各调用分发到对应子命令（而非按主命令位置参数解析），行为满足各自 Requirement
- **AND** `rtsql <db> "<sql>"` 主命令行为与本 Requirement 既有场景零偏差

#### Scenario: 裸名与子命令名冲突

- **GIVEN** 集中区存在名为 `list` 的数据库文件（`$RTSQL_HOME/db/list.db`）
- **WHEN** `rtsql list "SELECT 1"`
- **THEN** 第一个参数按子命令名分发（list 子命令执行，忽略多余参数报用法错误），SHALL NOT 打开该库
- **AND** `rtsql ./<path-to>/list.db "SELECT 1"`（含 `/` 路径形式）可正常打开该库执行 SQL

## ADDED Requirements

### Requirement: 生命周期子命令 new

`rtsql new <name|path>` SHALL 在解析后的路径创建空数据库：裸名按名称解析约定映射到集中区，含 `/` 的参数按路径使用。SHALL 创建缺失的目标父目录（仅 `new` 享有此语义；主命令与其余子命令维持不建目录契约）。目标文件已存在（含 0 字节文件）时 SHALL 以退出码 1 报错且不修改该文件。成功时 SHALL 静默退出（退出码 0，无 stdout 输出），创建结果 SHALL 立即可被主命令与其他子命令使用。进程退出前 SHALL 完成 close()（checkpoint + WAL 截断）。

#### Scenario: 裸名新建成功并自动创建集中区目录

- **GIVEN** `RTSQL_HOME` 指向一个不含 `db/` 子目录的目录
- **WHEN** `rtsql new app`
- **THEN** `$RTSQL_HOME/db/app.db` 被创建为合法空库，退出码 0，无 stdout 输出
- **AND** `db/` 目录被自动创建

#### Scenario: 路径新建并创建父目录

- **GIVEN** 文件系统上不存在 `/tmp/x/y/data.db`（`/tmp/x` 也不存在）
- **WHEN** `rtsql new /tmp/x/y/data.db`
- **THEN** 父目录 `/tmp/x/y` 与文件 `data.db` 被创建，退出码 0

#### Scenario: 已存在文件拒绝

- **GIVEN** 目标路径已存在文件（任意内容，含 0 字节）
- **WHEN** `rtsql new <name|path>`
- **THEN** stderr 报错且信息含 `already exists`，退出码 1，该文件内容不变

#### Scenario: 建库结果立即可用

- **GIVEN** `rtsql new app` 已成功
- **WHEN** `rtsql app "CREATE TABLE t (id INT PRIMARY KEY)"` 与后续 INSERT/SELECT
- **THEN** 主命令正常执行，退出码 0

### Requirement: 生命周期子命令 list

`rtsql list` SHALL 枚举集中存储目录（`$RTSQL_HOME/db/`，默认 `~/.rtsql/db/`）下的 `*.db` 常规文件，按名称排序，输出为行集（列 `name` 与 `size_bytes`），渲染遵循既有输出格式语义（`--format` 与 TTY/非 TTY 默认）。枚举 SHALL NOT 打开任何数据库文件（不校验内容有效性）。目录不存在或不含 `*.db` 文件时 SHALL 输出空行集并以退出码 0 退出。`RTSQL_HOME` 与 `HOME` 均未设置时 SHALL 以退出码 1 报错。

#### Scenario: 枚举集中区数据库文件

- **GIVEN** 集中区 `db/` 下存在 `a.db`（100 字节）与 `b.db`（200 字节），另有一个 `notes.txt`
- **WHEN** `rtsql list`（非 TTY，默认 json）
- **THEN** stdout 输出 `{"columns":["name","size_bytes"],"rows":[["a.db",100],["b.db",200]]}`，退出码 0
- **AND** `notes.txt` 不出现在结果中

#### Scenario: 空目录与目录缺失输出空行集

- **GIVEN** 集中区 `db/` 目录不存在或不含任何 `*.db` 文件
- **WHEN** `rtsql list`
- **THEN** stdout 输出空行集（`rows` 为空数组），退出码 0

#### Scenario: 基目录无法解析报错

- **GIVEN** `RTSQL_HOME` 与 `HOME` 环境变量均未设置
- **WHEN** `rtsql list`
- **THEN** stderr 输出错误信息，退出码 1

### Requirement: 生命周期子命令 schema

`rtsql schema <db>` SHALL 对每个用户表输出一行 `CREATE TABLE` DDL 语句（含列类型、PRIMARY KEY、NOT NULL、UNIQUE 等持久化约束），写入 stdout；空库 SHALL 无输出并以退出码 0 退出。schema 数据 SHALL 取自持久化 catalog（系统表不可经 SQL 查询）。目标库文件不存在时 SHALL 以退出码 1 报错；打开遇锁冲突 SHALL 以退出码 4 报错。输出的 DDL SHALL 可被主命令执行（在另一库重建同类表结构）。DEFAULT 约束不在 catalog 持久化面，SHALL NOT 出现在输出中。

#### Scenario: 表结构 DDL 输出

- **GIVEN** 库 `app` 含表 `users (id INT PRIMARY KEY, name STRING NOT NULL)`
- **WHEN** `rtsql schema app`
- **THEN** stdout 输出含 `CREATE TABLE` 与表名、全部列及约束的 DDL 文本，退出码 0

#### Scenario: 空库无输出

- **GIVEN** 库 `app` 存在且无用户表
- **WHEN** `rtsql schema app`
- **THEN** stdout 无输出，退出码 0

#### Scenario: 库文件不存在报错

- **GIVEN** 集中区或指定路径不存在目标库文件
- **WHEN** `rtsql schema missing`
- **THEN** stderr 报错且信息含 `does not exist`，退出码 1，且不创建任何文件

#### Scenario: 锁冲突退出码 4

- **GIVEN** 某持有者已锁定目标库文件
- **WHEN** `rtsql schema <db>`
- **THEN** stderr 输出以 `database is locked` 开头的错误信息，退出码 4

### Requirement: 生命周期子命令 dump 与 restore

`rtsql dump <db>` SHALL 向 stdout 输出逻辑 SQL 导出：按 catalog 顺序对每个用户表输出一行 CREATE TABLE DDL，随后按表 schema 列序输出全部行的 INSERT 语句；字符串字面量 SHALL 转义（单引号加倍），NULL 与布尔值 SHALL 输出为 SQL 字面量（NULL / TRUE / FALSE）。空库 SHALL 无输出并以退出码 0 退出。

`rtsql restore <db> <file|->` SHALL 从文件（`-` 表示 stdin）读取 SQL 文本，对目标库逐条执行（每条独立 auto-commit，逐条生效），成功时静默退出（退出码 0）。目标库文件不存在、或已含用户表时 SHALL 以退出码 1 拒绝。执行期任一语句失败 SHALL 立即停止：以退出码 3 报错，错误信息含失败语句序号（第 k 条/共 n 条），失败之前的语句已生效。两命令打开数据库的锁冲突均 SHALL 以退出码 4 报错；库文件不存在均 SHALL 以退出码 1 报错。进程退出前 SHALL 完成 close()。

#### Scenario: dump-restore 往返等价

- **GIVEN** 库 `a` 含表 `users (id INT PRIMARY KEY, name STRING)` 及若干行数据
- **WHEN** `rtsql dump a > dump.sql`，随后 `rtsql new b` 与 `rtsql restore b dump.sql`
- **THEN** 全链退出码 0，库 `b` 具有与 `a` 相同的表结构与行数据（SELECT 比对验证）

#### Scenario: dump 空库无输出

- **GIVEN** 库 `a` 存在且无用户表
- **WHEN** `rtsql dump a`
- **THEN** stdout 无输出，退出码 0

#### Scenario: restore 经 stdin 管道

- **GIVEN** 库 `a` 含表与数据，库 `b` 为 `new` 创建的空库
- **WHEN** `rtsql dump a | rtsql restore b -`
- **THEN** 退出码 0，库 `b` 与 `a` 数据等价

#### Scenario: restore 目标非空库拒绝

- **GIVEN** 库 `b` 已含用户表
- **WHEN** `rtsql restore b dump.sql`
- **THEN** stderr 报错，退出码 1，库内容不变

#### Scenario: restore 中途失败 fail-fast

- **GIVEN** dump 产物中第二条语句执行必然失败（如手工构造的重复主键）
- **WHEN** `rtsql restore b dump.sql`
- **THEN** 退出码 3，stderr 含失败语句序号；第一条语句的成效保留（重开可见），其后语句未执行

#### Scenario: dump/restore 库文件不存在报错

- **GIVEN** 目标库文件不存在
- **WHEN** `rtsql dump missing` 或 `rtsql restore missing dump.sql`
- **THEN** stderr 报错且信息含 `does not exist`，退出码 1，不创建任何文件

#### Scenario: dump/restore 锁冲突退出码 4

- **GIVEN** 某持有者已锁定目标库文件
- **WHEN** `rtsql dump <db>` 或 `rtsql restore <db> dump.sql`
- **THEN** stderr 输出以 `database is locked` 开头的错误信息，退出码 4

### Requirement: 生命周期子命令 import --csv

`rtsql import <db> <table> <file> --csv` SHALL 将 CSV 文件数据导入已存在的表：首行为表头，按列名与目标表列匹配（顺序无关），目标表全部列 SHALL 在表头中出现（缺失或未知列 SHALL 以退出码 1 拒绝）。字段值 SHALL 按目标表列类型转换：Int 列按 i64 解析、Float 列按 f64 解析、Bool 列接受 true/false（大小写不敏感）、String 列原样使用；空字段 SHALL 转换为 NULL（非 String 列）或空字符串（String 列）。转换失败 SHALL 以退出码 1 fail-fast 报错并含数据行定位；SQL 执行失败 SHALL 以退出码 3 fail-fast 报错；两者失败前已提交的行保留（逐条 auto-commit）。成功时 SHALL 输出导入行数（沿用受影响行数渲染语义）并以退出码 0 退出。库文件或表不存在时 SHALL 以退出码 1 报错；锁冲突 SHALL 以退出码 4 报错。CSV 解析 SHALL 遵循 RFC4180（引号、转义、跨行字段）。

#### Scenario: 基本导入

- **GIVEN** 表 `users (id INT PRIMARY KEY, name STRING)`；CSV 文件首行 `id,name`、数据两行
- **WHEN** `rtsql import app users data.csv --csv`（非 TTY）
- **THEN** 表内新增 2 行且值正确，stdout 输出 `{"affected_rows":2}`，退出码 0

#### Scenario: 表头乱序匹配

- **GIVEN** 同上表；CSV 首行 `name,id`（顺序与表定义相反）
- **WHEN** `rtsql import app users data.csv --csv`
- **THEN** 各值按列名正确映射入库，退出码 0

#### Scenario: 类型转换与空字段语义

- **GIVEN** 表 `t (a INT, b FLOAT, c BOOL, d STRING)`；CSV 数据行含空字段与合法类型文本
- **WHEN** `rtsql import app t data.csv --csv`
- **THEN** `a` 空字段为 NULL、`b`/`c` 按类型解析、`d` 空字段为空字符串；SELECT 比对验证，退出码 0

#### Scenario: 转换失败 fail-fast

- **GIVEN** 表 `t (a INT)`；CSV 数据第 2 行 `a` 列为 `abc`
- **WHEN** `rtsql import app t data.csv --csv`
- **THEN** 退出码 1，stderr 含失败数据行定位；第 1 行已入库（重开可见）

#### Scenario: 表头列不匹配拒绝

- **GIVEN** 表 `t (a INT, b INT)`；CSV 表头缺 `b` 列或含表外列
- **WHEN** `rtsql import app t data.csv --csv`
- **THEN** stderr 报错，退出码 1，表内无新行

#### Scenario: 目标表或库不存在拒绝

- **GIVEN** 库 `app` 存在但无表 `t`；或库 `missing` 不存在
- **WHEN** `rtsql import app t data.csv --csv` 或 `rtsql import missing t data.csv --csv`
- **THEN** stderr 报错（表缺失 / 库 `does not exist`），退出码 1

#### Scenario: CSV 引号转义字段

- **GIVEN** CSV 字段含逗号、双引号与跨行文本（RFC4180 引号转义）
- **WHEN** `rtsql import app t data.csv --csv`
- **THEN** 字段值完整入库（SELECT 比对），退出码 0

#### Scenario: import 锁冲突退出码 4

- **GIVEN** 某持有者已锁定目标库文件
- **WHEN** `rtsql import <db> <table> data.csv --csv`
- **THEN** stderr 输出以 `database is locked` 开头的错误信息，退出码 4
