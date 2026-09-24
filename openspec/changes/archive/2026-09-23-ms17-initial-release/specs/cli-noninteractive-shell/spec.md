# cli-noninteractive-shell Specification（delta）

## MODIFIED Requirements

### Requirement: 参数化 CLI 入口与主命令

`rtsql` 二进制 SHALL 提供 `rtsql <db> <sql>` one-shot 主命令：解析参数、打开数据库、执行 SQL（单条语句，或以 `;` 分隔的多条语句逐条执行）、渲染结果到 stdout、以分类退出码退出。进程正常退出前 SHALL 调用 `Database::close()`（checkpoint + WAL 截断）。主命令参数缺失或非法时 SHALL 以退出码 2 报用法错误。打开时遇跨进程锁冲突 SHALL 以退出码 4 报 `database is locked`；打开时遇密钥错误 SHALL 以退出码 5 报 `invalid key` 语义消息（密钥语义见 `database-encryption` spec）。

`rtsql` SHALL 同时提供生命周期子命令 `new` / `list` / `schema` / `dump` / `restore` / `import` 与分析子命令 `stats` / `sample` / `profile`：第一个位置参数命中子命令名时 SHALL 分发到对应子命令（子命令优先），未命中时按主命令位置参数解析。子命令参数缺失或非法时 SHALL 以退出码 2 报用法错误。裸名与子命令名同名的数据库 SHALL 以含 `/` 的路径形式经主命令打开（冲突行为文档化）。

`rtsql` SHALL 提供全局参数 `--key <KEY>`（含环境变量 `RTSQL_KEY` 通道）作用于全部开库命令，并 SHALL 提供 `completions` 隐藏子命令（见「Shell 补全脚本生成」Requirement）；两者的完整语义分别在 `database-encryption` spec 与本文件 completions Requirement 权威记录。

#### Scenario: one-shot SELECT 执行成功

- **GIVEN** 集中存储或指定路径下存在含数据的库
- **WHEN** `rtsql <db> "SELECT id, name FROM t WHERE id = 1"`
- **THEN** stdout 输出查询结果（含列名表头），退出码 0
- **AND** 进程退出前完成了 checkpoint（WAL 被截断，重开无 redo 负担）

#### Scenario: 用法错误退出码 2

- **GIVEN** 任意环境
- **WHEN** `rtsql`（无参数）或 `rtsql --format bogus db "SELECT 1"`（非法选项值）或 `rtsql <db>`（缺 SQL 参数）或 `rtsql --key "" <db> "SELECT 1"`（空密钥）
- **THEN** stderr 输出用法信息，退出码 2，不打开任何数据库

#### Scenario: SQL 错误退出码 3

- **GIVEN** 已打开的库
- **WHEN** `rtsql <db> "SELEC typo"` 或 `rtsql <db> "SELECT * FROM missing_table"`
- **THEN** stderr 输出错误信息，退出码 3

#### Scenario: 锁冲突退出码 4

- **GIVEN** 某持有者已锁定目标库文件
- **WHEN** `rtsql <db> "SELECT 1"`
- **THEN** stderr 输出以 `database is locked` 开头的错误信息（含目标路径），退出码 4，且不执行任何 SQL
- **AND** 锁冲突 SHALL 优先于密钥错误（加密库被占用时无密钥打开仍报退出码 4）

#### Scenario: 退出码枚举为后续任务留位

- **GIVEN** CLI 退出码枚举（0/1/2/3/4/5）
- **WHEN** 本 capability 落地后的代码审查
- **THEN** 退出码 4（锁冲突）已由锁冲突场景获得产生路径（T02 落地）
- **AND** 退出码 5（密钥）已由 `database-encryption` spec 的错误面获得产生路径（本 change 落地——错误密钥/加密无钥/明文带钥，场景见该 spec「退出码 5 产生路径」）

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

### Requirement: Shell 补全脚本生成

`rtsql` SHALL 提供隐藏子命令 `rtsql completions <shell>`，`<shell>` SHALL 接受 `bash` / `zsh` / `fish` 三值；执行 SHALL 向 stdout 输出对应 shell 的补全脚本（clap_complete 运行时生成，覆盖主命令、全局参数与全部子命令）。该子命令 SHALL NOT 出现在 `--help` 输出中（隐藏面）。`<shell>` 缺失或取值非法时 SHALL 以退出码 2 报用法错误。生成路径 SHALL NOT 打开数据库、不消费密钥。

#### Scenario: 三 shell 补全脚本生成

- **GIVEN** 任意环境（无需任何数据库）
- **WHEN** 分别执行 `rtsql completions bash`、`rtsql completions zsh`、`rtsql completions fish`
- **THEN** stdout 各输出非空补全脚本，内容含 `rtsql` 命令名与 `--key` 等全局参数，退出码 0

#### Scenario: 隐藏面与用法错误

- **GIVEN** 任意环境
- **WHEN** 执行 `rtsql --help`；再执行 `rtsql completions`（缺参数）与 `rtsql completions powershell`（未收窄值）
- **THEN** `--help` 输出不含 completions；后两者以退出码 2 报用法错误
