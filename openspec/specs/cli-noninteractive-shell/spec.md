# cli-noninteractive-shell Specification

## Purpose
TBD - created by archiving change 2026-09-06-ms10-t01-cli-shell. Update Purpose after archive.

## Requirements

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

### Requirement: 数据库名称解析

CLI SHALL 把 `<db>` 参数解析为数据库主文件路径：参数含 `/` 时视为路径原样使用；裸名 `foo` 时解析为 `$RTSQL_HOME/db/foo.db`，其中 `RTSQL_HOME` 未设置时默认 `~/.rtsql/`。`~` 展开 SHALL 依赖环境变量语义（unix `HOME`），不自行解析 `~` 字面量。

#### Scenario: 裸名解析到集中存储

- **GIVEN** `RTSQL_HOME` 未设置、`$HOME/.rtsql/db/foo.db` 存在
- **WHEN** `rtsql foo "SELECT ..."`
- **THEN** 打开 `$HOME/.rtsql/db/foo.db`

#### Scenario: RTSQL_HOME 覆盖默认根

- **GIVEN** `RTSQL_HOME=/tmp/myroot` 且 `/tmp/myroot/db/bar.db` 存在
- **WHEN** `RTSQL_HOME=/tmp/myroot rtsql bar "SELECT ..."`
- **THEN** 打开 `/tmp/myroot/db/bar.db`

#### Scenario: 含斜杠参数按路径直开

- **GIVEN** 工作目录或绝对路径下存在库文件
- **WHEN** `rtsql ./local.db "SELECT ..."` 或 `rtsql /abs/path/x.db "SELECT ..."`
- **THEN** 打开该路径原样，不拼集中存储前缀

#### Scenario: 打开不存在路径创建空库

- **GIVEN** 目标路径不存在
- **WHEN** `rtsql newdb "CREATE TABLE t (id INT)"`
- **THEN** 沿用现有 `Database::open` 语义静默创建空库并执行成功（不提示"已创建"）
- **AND** 打开失败（权限、页对齐错误、redo 失败）时报错并以退出码 1 退出

### Requirement: 查询结果列名表头

查询结果渲染 SHALL 携带真实列名表头：CLI 通过 `PlanBuilder::get_plan_output_columns` 从 PhysicalPlan 提取列名，`get_plan_output_columns` 对 Join/SemiJoin/AntiJoin 节点 SHALL 返回其 `output_columns` 的列名（此前返回空 Vec）。

#### Scenario: 普通查询表头

- **GIVEN** 表 `t(a INT, b TEXT)` 有一行数据
- **WHEN** `rtsql db "SELECT a, b FROM t"`
- **THEN** 表格输出表头为 `a | b`，JSON 输出 `columns` 为 `["a","b"]`

#### Scenario: JOIN 查询表头

- **GIVEN** 两表 JOIN 查询（如 `SELECT t.a, u.b FROM t JOIN u ON ...`）
- **WHEN** 以任意格式渲染结果
- **THEN** 表头为投影列名（如 `a | b`），而非 `col0 | col1` 或空

#### Scenario: 别名与聚合表头

- **GIVEN** `SELECT COUNT(*) AS cnt, AVG(price) FROM sales`
- **WHEN** 渲染结果
- **THEN** 表头为 `cnt | avg_price`（别名优先；无别名聚合用引擎现有 `result_column_name` 文本，如 `count_star`）

#### Scenario: JOIN 臂补齐不影响既有派生表路径

- **GIVEN** `get_plan_output_columns` 的现有唯一消费方（派生表列注册）
- **WHEN** 该函数对 JOIN 臂返回真列名
- **THEN** 既有 585 测试零修改全绿（JOIN 不可作为派生表输入，行为无实际变化）

### Requirement: 输出格式四态

CLI SHALL 支持 `--format table|json|csv|tsv`；未指定时 TTY stdout 默认 table、非 TTY 默认 json。CSV/TSV SHALL 用 RFC 4180 风格转义（引号包裹含分隔符/引号/换行的值，引号翻倍）。NULL 的文本渲染 SHALL 为空（与 PG DataRow 语义一致）。

#### Scenario: TTY 默认表格

- **GIVEN** stdout 为 TTY（集成测试中用 pty 或以显式 --format 验证渲染函数）
- **WHEN** 未指定 `--format` 执行 SELECT
- **THEN** 输出对齐表格（含表头与行分隔）

#### Scenario: 非 TTY 默认 JSON

- **GIVEN** stdout 为管道（非 TTY）
- **WHEN** 未指定 `--format` 执行 SELECT
- **THEN** 输出 `{"columns":[...],"rows":[[...],...]}` 形状的合法 JSON

#### Scenario: CSV 转义

- **GIVEN** 某字符串值含逗号、引号或换行（如 `a"b,c`）
- **WHEN** `--format csv` 渲染
- **THEN** 该字段以引号包裹并正确转义（`"a""b,c"`），可被标准 CSV 解析器还原

#### Scenario: TSV 转义

- **GIVEN** 某字符串值含制表符、换行或引号
- **WHEN** `--format tsv` 渲染
- **THEN** 字段内制表符/换行转义（`\t`/`\n`），字段不以引号包裹，行结构可还原

#### Scenario: DML 与 DDL 的输出

- **GIVEN** INSERT/UPDATE/DELETE/CREATE TABLE 语句
- **WHEN** 以任意格式执行
- **THEN** 输出受影响行数（table/json：`AffectedRows` 语义；csv/tsv：同值单字段），退出码 0

### Requirement: 扫描执行器真投影（Iteration 001）

`SELECT` 的投影列表 SHALL 决定扫描路径返回行的形状：四个扫描执行器（Scan / DataScan / IndexScan / IndexScanAll）SHALL 按投影裁剪产出行，plan 节点的 `columns` 元数据与行形状一致。谓词求值（WHERE / 下推谓词 / MVCC 可见性）SHALL 在全 schema 行上先行完成，投影只发生在行产出最后一步。`SELECT *` 的投影等于全 schema，行为不变。

CLI 表头 SHALL 与行形状一致：`get_plan_output_columns` 对携带投影的 plan 节点 SHALL 返回投影后的列名（含 DataScan / Scan / IndexScanAll 节点自身的 `projection` 裁剪，与 Filter / Sort 臂既有模式一致）；任何查询路径下 CLI 输出的表头列数 SHALL 等于每行字段数（table / json / csv / tsv 各格式同契约）。

#### Scenario: 子集投影在全部扫描路径返回投影列

- **GIVEN** 表 `s(id INT PRIMARY KEY, name STRING)` 含一行 `(1, 'Alice')`
- **WHEN** 分别执行 `SELECT name FROM s`（DataScan 路径）与 `SELECT name FROM s WHERE id = 1`（IndexScan 路径）
- **THEN** 两条查询都返回单列：表头 `["name"]`、行 `[["Alice"]]`
- **AND** 表头列数与每行字段数一致（任何路径无错位）

#### Scenario: 裸 DataScan 子集投影 CLI 表头按投影裁剪

- **GIVEN** 表 `s(id INT PRIMARY KEY, name STRING)` 含一行 `(1, 'Alice')`（修复前表头为 `["id","name"]`、行 `[["Alice"]]`）
- **WHEN** `rtsql <db> "SELECT name FROM s"`（`--format json`）
- **THEN** 输出 `{"columns":["name"],"rows":[["Alice"]]}`（修复前 `columns` 为 `["id","name"]`，与 `rows` 字段数不一致）

#### Scenario: 下推 DataScan 子集投影 CLI 表头按投影裁剪

- **GIVEN** 表 `t(id INT, n INT, s STRING)` 含行 `(1, 10, 'a')`
- **WHEN** `rtsql <db> "SELECT s FROM t WHERE n > 5"`（谓词下推 DataScan 路径，`--format json`）
- **THEN** 输出 `{"columns":["s"],"rows":[["a"]]}`，表头列数与行字段数一致

#### Scenario: PK 点查聚合返回正确值

- **GIVEN** 表 `s(id INT PRIMARY KEY, price INT)` 含行 `(1,10), (2,20)`
- **WHEN** `SELECT SUM(price) FROM s WHERE id = 2`
- **THEN** 返回 `20`（而非 `null`）
- **AND** 聚合输入的列映射与投影后的行形状一致（无静默 Null 兜底路径）

#### Scenario: 投影外排序键正确排序

- **GIVEN** 表 `s(id INT PRIMARY KEY, name STRING)` 含多行
- **WHEN** `SELECT id FROM s WHERE price > 15 ORDER BY name DESC`（排序键 `name` 不在投影内）
- **THEN** 输出行按 `name` 降序排列（而非静默保持原序）

#### Scenario: SELECT 与全 schema 行为不变

- **GIVEN** 任意含数据的表
- **WHEN** `SELECT * FROM t` 或投影覆盖全部列
- **THEN** 返回行与投影改造前的全 schema 行完全一致（旧行为保留）

#### Scenario: 聚合与表达式路径表头零回归

- **GIVEN** 聚合查询（`SELECT COUNT(*) AS cnt FROM t`）与表达式投影查询（`SELECT COALESCE(n, 0) AS x FROM t`；表达式项支持面见 `sql-expression-evaluation`——二元算术不在 SELECT 表达式项之列，本场景以受支持的 Projection 定形形态为锚）
- **WHEN** 渲染结果
- **THEN** 表头分别来自 Aggregate `output_columns` 与 Projection 节点 `columns`，与本 change 前一致（聚合查询 scan 输入投影恒为空、表达式路径由顶层 Projection 节点定形，本 change 的 scan 臂投影裁剪不触及）

#### Scenario: 既有测试按投影语义校准

- **GIVEN** 既有测试套件中假设"子集投影返回全 schema 行"的断言
- **WHEN** 本 Requirement 落地
- **THEN** 受影响断言按投影语义校准（只改行形状期望，不改测试意图），校准清单记录于 Act Response
- **AND** `cargo test --all` 全绿

### Requirement: 优雅停机（信号接线 close）

CLI SHALL 处理 SIGINT 与 SIGTERM：信号到达时中止当前阶段。数据库已打开时 SHALL 执行 `Database::close()`（checkpoint + WAL 截断）后以 `128 + 信号编号` 退出（SIGINT=130、SIGTERM=143）；数据库尚未打开（打开阶段被中断）时 SHALL 立即以相同退出码退出（无需 close——WAL 未被截断，恢复可完整重放）。无信号时 SHALL 行为与本 Requirement 落地前完全一致。

#### Scenario: 执行阶段 SIGINT 优雅停机

- **GIVEN** 数据库含大量未 checkpoint 事务（重开需显著恢复时间）
- **WHEN** `rtsql <db> "SELECT ..."` 执行期间收到 SIGINT
- **THEN** 进程完成 `close()`（checkpoint）后以退出码 130 退出，不挂起
- **AND** 已提交数据完整（后续打开可查）

#### Scenario: 打开阶段 SIGINT 立即退出

- **GIVEN** 恢复耗时较长的数据库（大 WAL）
- **WHEN** `Database::open` 完成前收到 SIGINT
- **THEN** 进程立即以退出码 130 退出，无挂起，无 close

#### Scenario: SIGTERM 同语义

- **GIVEN** 执行阶段或打开阶段的 `rtsql` 进程
- **WHEN** 收到 SIGTERM
- **THEN** 行为与 SIGINT 一致，退出码 143

#### Scenario: 无信号正常路径回归

- **GIVEN** 任意库与单条合法 SQL
- **WHEN** 正常执行（无信号）
- **THEN** 退出码、stdout/stderr 输出与 checkpoint 语义与本 Requirement 落地前完全一致（既有 cli 集成测试零修改全绿）

### Requirement: 多语句分片逐条执行

CLI SHALL 对以 `;` 分隔的多条 SQL 语句逐条执行：每条语句独立 plan/execute；无活跃会话事务时每条语句独立 auto-commit 事务（逐条生效）。事务语句（`BEGIN`/`COMMIT`/`ROLLBACK`，语义见 `sql-transaction-statements`）SHALL 改变会话事务上下文：`BEGIN` 之后的语句 SHALL 在打开的事务内执行（不再逐条 auto-commit），直至 `COMMIT`/`ROLLBACK` 终结该事务。每条语句的结果 SHALL 顺序渲染到 stdout：DML/DDL/事务语句输出受影响行数，SELECT 输出查询结果（列名表头等既有渲染语义不变）；`json` 格式下每条语句输出一个独立 JSON 文档。任一语句失败时 SHALL 立即停止执行（fail-fast）：以退出码 3 报错，错误信息 SHALL 包含失败语句的序号（第 k 条/共 n 条）与失败语句文本；失败之前语句的生效状态 SHALL 如实注明——无会话事务时注明已生效（已提交），有会话事务时注明未提交且已随事务回滚（此时 CLI SHALL 在返回前显式回滚会话事务）；失败之后的语句 SHALL NOT 执行。CLI 调用结束时 SHALL NOT 存在未提交的会话事务：语句循环正常结束仍有打开事务时 SHALL 显式回滚并向 stderr 提示，退出码仍为 0。SQL 语法错误 SHALL 在任何语句执行前整体拒绝（零执行），错误信息保留解析器的行/列定位。lib 单结果执行路径（`pipeline::execute` 网络路径、`execute_in_tx` 显式事务路径）遇多语句 SHALL 显式报错，SHALL NOT 静默截断。

#### Scenario: 多条 DML 逐条执行全部生效

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "INSERT INTO t VALUES (1); INSERT INTO t VALUES (2)"`
- **THEN** 退出码 0，stdout 顺序输出两段受影响行数结果
- **AND** 重开查询可见两行（两条独立事务均已提交）

#### Scenario: 顺序渲染混合语句结果

- **GIVEN** 表 `t(id INT)` 已有数据
- **WHEN** `rtsql db "INSERT INTO t VALUES (9); SELECT id FROM t"`（`--format json`，非 TTY）
- **THEN** stdout 先输出 INSERT 的受影响行文档（与单语句 DML 输出形状一致），再输出 SELECT 的 rows 文档（两个独立 JSON 文档）
- **AND** 退出码 0

#### Scenario: 中间语句失败 fail-fast（部分已生效）

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "INSERT INTO t VALUES (1); INSERT INTO missing_table VALUES (2); INSERT INTO t VALUES (3)"`
- **THEN** 退出码 3，stderr 错误信息包含失败语句序号（第 2 条/共 3 条）与失败语句文本
- **AND** 第 1 条 INSERT 已生效（重开可查 `id=1`），第 3 条未执行

#### Scenario: 语法错误整体拒绝（零执行）

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "INSERT INTO t VALUES (1); SELEC typo"`
- **THEN** 退出码 3，stderr 错误信息含解析器行/列定位
- **AND** 任何语句均未执行（`t` 仍为空）

#### Scenario: 分号边界语义

- **GIVEN** 表 `users(id INT PRIMARY KEY, name STRING)` 已有一行（`id=1`，`name='Alice'`）
- **WHEN** `rtsql db "SELECT id FROM users;; SELECT name FROM users WHERE name = 'a;b';"`（连续分号、字符串字面量内分号、尾随分号）
- **THEN** 按两条语句执行（字符串字面量不被分片），退出码 0
- **AND** 第一条查询输出 1 行，第二条查询输出空行集（`'a;b'` 不匹配任何行）

#### Scenario: lib 单结果路径显式拒绝多语句

- **GIVEN** 已打开的 `Database`
- **WHEN** `Database::execute_sql("SELECT 1; SELECT 2")` 或 `execute_in_tx` 传入多语句
- **THEN** 返回 `Response::Error`，错误信息说明该路径仅支持单语句
- **AND** 不执行任何语句（无静默截断）

#### Scenario: 事务上下文内失败注明未提交并回滚（区别于 auto-commit 上下文）

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "BEGIN; INSERT INTO t VALUES (1); INSERT INTO missing_table VALUES (2)"`
- **THEN** 退出码 3，stderr 错误信息包含失败语句序号（第 3 条/共 3 条）并注明先前语句未提交、已随事务回滚
- **AND** 重开查询 `t` 为空（无部分生效，与 auto-commit 上下文的"已生效"注明不同）

#### Scenario: 调用收尾未提交事务隐式回滚并提示

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "BEGIN; INSERT INTO t VALUES (1)"`（无 COMMIT/ROLLBACK）
- **THEN** 退出码 0，stderr 输出未提交回滚提示
- **AND** 重开查询 `t` 为空

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
