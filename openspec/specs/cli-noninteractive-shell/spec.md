# cli-noninteractive-shell Specification

## Purpose
TBD - created by archiving change 2026-09-06-ms10-t01-cli-shell. Update Purpose after archive.

## Requirements

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

#### Scenario: 主命令缺失库拒绝与 lib 建库边界（原「打开不存在路径创建空库」随 change 2026-09-30-catalog-durability-and-cli-surface T10 校准）

- **GIVEN** 目标路径不存在
- **WHEN** 主命令 `rtsql newdb "CREATE TABLE t (id INT)"`
- **THEN** 以退出码 1 拒绝并指引 `rtsql new newdb`，不创建任何文件（见「主命令打开不存在库显式拒绝」Requirement）
- **AND** lib 层 `Database::open` 对缺失路径仍按既有建库语义创建空库；打开失败（权限、页对齐错误、redo 失败）时报错并以退出码 1 退出

### Requirement: 主命令打开不存在库显式拒绝

主命令（`rtsql <db> <sql>`）在路径解析后、打开前 SHALL 校验库文件存在：不存在（裸名与路径模式同）SHALL 以 `ExitStatus::General`（exit 1）拒绝，错误文案 SHALL 携带库名并指引 `rtsql new <name>`；SHALL NOT 创建任何文件。`new` 子命令（创建通道）SHALL NOT 受此校验影响；其余生命周期子命令既有的缺失拒绝（`resolve_existing_db`）保持不变；lib 层 `Database::open` 对缺失路径的建库行为保持不变。

#### Scenario: 裸名缺失库拒绝

- **GIVEN** `RTSQL_HOME` 下不存在 `typo.db`
- **WHEN** `rtsql typo "SELECT 1"`
- **THEN** exit 1，错误含 `typo` 与 `rtsql new typo` 指引，未创建 `typo.db`

#### Scenario: 路径模式缺失拒绝

- **GIVEN** 路径 `/tmp/nope/x.db` 不存在
- **WHEN** `rtsql /tmp/nope/x.db "SELECT 1"`
- **THEN** exit 1，未创建文件

#### Scenario: new 创建后主命令可用

- **GIVEN** 同上
- **WHEN** `rtsql new typo && rtsql typo "SELECT 1"`
- **THEN** `new` 成功创建，主命令正常执行 exit 0

#### Scenario: lib API 与子命令零回归

- **GIVEN** lib 调用方与既有脚本
- **WHEN** lib `Database::open` 打开缺失路径；`restore`/`import`/`schema`/`dump`/`delete` 操作缺失库
- **THEN** lib 建库行为不变；子命令既有拒绝文案与退出码不变

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

本次追加 table 格式大单元格降级语义：任一单元格（含表头）显示字符数超过固定上限（64）时，table 渲染 SHALL 将该单元格截断为前 61 字符 + `...`（总显示宽度恰 64），列宽 SHALL 按截断后文本计算；table 渲染 SHALL 对任意单元格长度 exit 0，不因单元格长度 panic（修复前：≥65536 字符触发 std::fmt 动态宽度上限 panic，exit 101）。截断 SHALL 按字符（chars）边界执行，多字节字符不产生部分字符。`json`/`csv`/`tsv` SHALL 保持全值保真（不截断、逐字节不变）。

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

#### Scenario: 超限单元格截断且不 panic

- **GIVEN** 某 STRING 值 100,000 字符（≥65536）
- **WHEN** `--format table` 渲染（SELECT 或分析子命令）
- **THEN** exit 0，该单元格显示前 61 字符 + `...`（共 64 字符）（失败边界——修复前：exit 101，panic `Formatting argument out of range`）

#### Scenario: 未超限与截断边界

- **GIVEN** 单元格分别为 64 字符与 65 字符
- **WHEN** `--format table` 渲染
- **THEN** 64 字符原样显示；65 字符截断为 64 显示字符（前 61 + `...`）；既有 ≤64 字符输出逐字节不变

#### Scenario: 三管道格式保真零回归

- **GIVEN** 同一大值
- **WHEN** `--format json` / `--format csv` / `--format tsv` 渲染
- **THEN** 输出含完整原值，长度逐字节一致（不截断）

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

`rtsql dump <db>` SHALL 向 stdout 输出逻辑 SQL 导出：按 catalog 顺序对每个用户表输出一行 CREATE TABLE DDL，随后按表 schema 列序输出全部行的 INSERT 语句；字符串字面量 SHALL 转义（单引号加倍），NULL 与布尔值 SHALL 输出为 SQL 字面量（NULL / TRUE / FALSE）。DDL 渲染 SHALL 仅在主键列声明类型为 INTEGER 时输出 `PRIMARY KEY`：显式声明 INTEGER PK 的表 SHALL 照常输出；非 INTEGER PK 声明的存量表（声明从未被强制）与隐式主键非 INTEGER 首列表 SHALL NOT 输出 `PRIMARY KEY`（输出与实际强制语义一致，产物在新拒绝面下可 restore 往返）。空库 SHALL 无输出并以退出码 0 退出。

`rtsql restore <db> <file|->` SHALL 从文件（`-` 表示 stdin）读取 SQL 文本，对目标库逐条执行（每条独立 auto-commit，逐条生效），成功时静默退出（退出码 0）。目标库文件不存在、或已含用户表时 SHALL 以退出码 1 拒绝。执行期任一语句失败 SHALL 立即停止：以退出码 3 报错，错误信息含失败语句序号（第 k 条/共 n 条），失败之前的语句已生效。两命令打开数据库的锁冲突均 SHALL 以退出码 4 报错；库文件不存在均 SHALL 以退出码 1 报错。进程退出前 SHALL 完成 close()。

#### Scenario: dump-restore 往返等价

- **GIVEN** 库 `a` 含表 `users (id INT PRIMARY KEY, name STRING)` 及若干行数据
- **WHEN** `rtsql dump a > dump.sql`，随后 `rtsql new b` 与 `rtsql restore b dump.sql`
- **THEN** 全链退出码 0，库 `b` 具有与 `a` 相同的表结构与行数据（SELECT 比对验证）

#### Scenario: INTEGER PK 表 dump 输出逐字节不变

- **GIVEN** 含显式 INTEGER PRIMARY KEY 表的库
- **WHEN** `rtsql dump a`
- **THEN** DDL 含 `PRIMARY KEY`（渲染行为与修改前逐字节一致）

#### Scenario: 非 INTEGER PK 存量表 dump 渲染与往返

- **GIVEN** lib 通道构造的存量 TEXT PRIMARY KEY 表（或隐式主键非 INTEGER 首列表）
- **WHEN** `rtsql dump a > dump.sql`，随后 `rtsql new b` 与 `rtsql restore b dump.sql`
- **THEN** DDL 不含 `PRIMARY KEY`；restore 退出码 0，库 `b` 行数据等价（往返可达，不被非 INTEGER PK 计划期拒绝面阻断）

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

**e2e 验证与缺陷清偿 change（T6/ISS12，2026-10-14 收尾）追加**：`schema` 与 `dump` 的 DDL 列类型 SHALL 渲染标准长名——`INTEGER`、`FLOAT`、`BOOLEAN`、`DATE`、`TIMESTAMP`、`TEXT`（不再输出引擎内部短名 `INT`/`STRING`/`BOOL`）。dump→restore 往返与多代 dump/restore 幂等 SHALL 保持（解析器接受长名）。

#### Scenario: 类型名渲染与声明一致

- **GIVEN** `CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, ok BOOLEAN, score FLOAT)`
- **WHEN** `rtsql schema t` / `rtsql dump t`
- **THEN** DDL 输出 `INTEGER`/`TEXT`/`BOOLEAN`/`FLOAT`（失败边界——修复前：渲染 `INT`/`STRING`/`BOOL` 短名）

#### Scenario: dump→restore 往返与多代幂等

- **GIVEN** 同上库 dump
- **WHEN** restore 到新库后再 dump
- **THEN** 类型名与数据逐字节幂等

### Requirement: dump 数据链完整性预检

`rtsql dump <db>` SHALL 在导出前对每张用户表执行数据页链完整性预检：自 head 沿页链收集，链终止页与 catalog 持久的尾页一致视为完整。链不完整时 SHALL 向 stderr 输出一行告警（点名表名、catalog 尾页与链终止页、以及「备份可能不完整，库可能已损坏」），SHALL 继续产出可达部分并保持退出码 0。完整库的 dump 行为与输出 SHALL 与修复前逐字节一致（零告警）。

#### Scenario: 干净库 dump 零告警

- **GIVEN** 页链完整的库
- **WHEN** `rtsql dump clean`
- **THEN** stderr 无告警，stdout 输出完整 SQL 文本，exit 0

#### Scenario: 断链库 dump 告警并继续

- **GIVEN** 数据页链在 catalog 尾页之前终止的库（历史缺陷损坏形态）
- **WHEN** `rtsql dump broken 2>err.txt`
- **THEN** stdout 仍输出可达行的 SQL 文本；err.txt 含表名与「可能不完整/已损坏」告警；exit 0

#### Scenario: 告警不进入 stdout 管道

- **GIVEN** 同上断链库
- **WHEN** `rtsql dump broken | grep -c "INSERT INTO"`
- **THEN** stdout 纯 SQL 不含告警文本，管道消费方不受影响

### Requirement: 生命周期子命令 import --csv

`rtsql import <db> <table> <file> --csv` SHALL 将 CSV 数据导入数据库：`file` SHALL 接受文件路径或 `-`（stdin，对齐 `restore` 通道先例）。CSV 首行为表头。目标表**已存在**时沿既有语义：按列名与目标表列匹配（顺序无关），目标表全部列 SHALL 在表头中出现（缺失或未知列 SHALL 以退出码 1 拒绝）；字段值 SHALL 按目标表列类型转换：Int 列按 i64 解析、Float 列按 f64 解析、Bool 列接受 true/false（大小写不敏感）、String 列原样使用；空字段 SHALL 转换为 NULL（非 String 列）或空字符串（String 列）。转换失败 SHALL 以退出码 1 fail-fast 报错并含数据行定位；SQL 执行失败 SHALL 以退出码 3 fail-fast 报错；两者失败前已提交的行保留（逐条 auto-commit）。成功时 SHALL 输出导入行数（沿用受影响行数渲染语义）并以退出码 0 退出。锁冲突 SHALL 以退出码 4 报错。CSV 解析 SHALL 遵循 RFC4180（引号、转义、跨行字段）。

目标表**不存在**时 SHALL 按表头自动建表：列名与列序 SHALL 取 CSV 表头；列类型 SHALL 按全部数据行逐字段猜测（可解析为 i64 → INT、可解析为 f64 → FLOAT、其余字段一律按 STRING 参与推导）并按 CTAS 推导矩阵规则合并（同型保留、INT∪FLOAT → FLOAT、NULL 参与任意类型、全 NULL 或零数据行的列落 STRING）；列内类型冲突 SHALL 以点名列名与冲突类型的错误拒绝（不做 STRING 宽化）。自动建表 SHALL NOT 显式声明主键——新表 SHALL 沿用引擎「未声明主键 → 首列隐式主键」既有语义（与 `create-table-as-select` 物化表同款，`rtsql schema` 渲染首列 `PRIMARY KEY`）：首列声明类型为 INT 时行值重复 SHALL 以 DuplicateKey fail-fast 拒绝（失败前已提交行保留，逐条 auto-commit 既有语义），首列为 FLOAT/STRING 时行落非键行语义、重复值合法。推导 SHALL 在收齐数据行后、建表前完成：任何推导失败 SHALL 以退出码 1 拒绝且零副作用（目标表不存在于失败结果中）；自动建表 SHALL 经常规建表通道执行（保留名/重名视图/重复列名等计划期守卫自然生效），建表失败 SHALL 以既有 SQL 失败通道报错且零副作用。自动建表成功后 SHALL 沿既有导入路径逐行写入（空字段语义沿用目标表导入规则：推断为 STRING 的列空字段插入空字符串，其余列空字段插入 NULL）。

`:memory:` 组合 SHALL 豁免生命周期子命令的内存库拒绝：`rtsql import :memory: <table> <file|-> --csv` SHALL 可达（内存库打开，进程退出即弃，`in-memory-database` 语义零特殊化）；`--key` 组合 SHALL 以退出码 2 拒绝（内存库永不加密，对齐主命令语义）。文件库目标缺失时 SHALL 维持既有退出码 1 拒绝（import 不创建数据库文件，创建通道收敛 `rtsql new`）。

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

- **GIVEN** 库 `missing` 不存在
- **WHEN** `rtsql import missing t data.csv --csv`（或 stdin 形态 `rtsql import missing t - --csv`）
- **THEN** stderr 报错库 `does not exist`，退出码 1，零文件创建（创建通道仍收敛 `rtsql new`）
- **AND** 库存在而目标表缺失的形态不再拒绝：按上方自动建表场景语义建表导入（原「表不存在退出码 1」拒绝面由自动建表语义承接，见「目标表不存在自动建表（类型推导）」等场景）

#### Scenario: CSV 引号转义字段

- **GIVEN** CSV 字段含逗号、双引号与跨行文本（RFC4180 引号转义）
- **WHEN** `rtsql import app t data.csv --csv`
- **THEN** 字段值完整入库（SELECT 比对），退出码 0

#### Scenario: import 锁冲突退出码 4

- **GIVEN** 某持有者已锁定目标库文件
- **WHEN** `rtsql import <db> <table> data.csv --csv`
- **THEN** stderr 输出以 `database is locked` 开头的错误信息，退出码 4

#### Scenario: stdin 导入既有表

- **GIVEN** 表 `t (a INT, b STRING)`；进程管道 `printf 'a,b\n1,x\n2,y\n' | rtsql import app t - --csv`
- **WHEN** import 从 stdin 读取 CSV
- **THEN** 导入 2 行，stdout 输出 `{"affected_rows":2}`，退出码 0；SELECT 比对值正确

#### Scenario: 目标表不存在自动建表（类型推导）

- **GIVEN** 库 `app` 无表 `t`；CSV 首行 `id,score,label`、数据行含整数、浮点与文本
- **WHEN** `rtsql import app t data.csv --csv`（或 stdin 形态）
- **THEN** 表 `t` 按 `(id INT, score FLOAT, label STRING)` 创建（INT∪FLOAT → FLOAT 提升矩阵生效），全部行导入；`rtsql schema app` 可见该表；SELECT 比对值与类型正确

#### Scenario: 同型列推导与全 NULL 列

- **GIVEN** CSV 表头 `a,b`：`a` 列数据全为整数文本、`b` 列数据全为空字段
- **WHEN** 对无表目标自动建表导入
- **THEN** `a` 推导为 INT、`b`（全 NULL）落 STRING；导入成功退出码 0

#### Scenario: 零数据行（仅表头）自动建表

- **GIVEN** CSV 仅含表头行 `a,b,c`
- **WHEN** 对无表目标自动建表导入
- **THEN** 表按三列全 STRING 创建，导入 0 行，退出码 0

#### Scenario: 类型冲突点名拒绝零副作用

- **GIVEN** CSV 表头 `a`：数据行既有 `1`、`2` 又有 `abc`
- **WHEN** 对无表目标自动建表导入
- **THEN** 退出码 1，stderr 点名列名 `a` 与冲突类型（INT 与 STRING）；目标表不存在（`rtsql schema` 零输出），零副作用

#### Scenario: 自动建表首列隐式主键（引擎语义沿用）

- **GIVEN** CSV 表头 `id,v`：`id` 列数据含重复整数值（如 `1` 出现两行）；对照 CSV 首列推导为 FLOAT/STRING 且含重复值
- **WHEN** 对无表目标自动建表导入
- **THEN** INT 首列形态于首个重复值处以 DuplicateKey fail-fast 拒绝（exit 3，失败前已提交行保留——引擎「未声明主键 → 首列隐式主键」语义，与 `create-table-as-select` 同款；`rtsql schema` 渲染首列 `PRIMARY KEY`）；FLOAT/STRING 首列重复值全部导入成功（非键行语义，不触发 DuplicateKey）

#### Scenario: 自动建表经保留名/重复列名守卫拒绝

- **GIVEN** CSV 表头含重复列名（如 `a,a`）
- **WHEN** 对无表目标自动建表导入
- **THEN** 经常规建表通道计划期守卫拒绝（SQL 失败通道，退出码 3），零副作用

#### Scenario: `:memory:` import 免建表可达

- **WHEN** `printf 'a,b\n1,x\n' | rtsql import :memory: t - --csv`
- **THEN** 内存库自动建表并导入，stdout 输出 `{"affected_rows":1}`，退出码 0（进程退出即弃，无任何文件产生）

#### Scenario: `:memory:` import `--key` 拒绝

- **WHEN** `rtsql import :memory: t - --csv --key secret`
- **THEN** 退出码 2，stderr 点名内存库不支持加密（对齐主命令语义）

#### Scenario: `:memory:` 其他生命周期子命令维持拒绝

- **WHEN** `rtsql :memory: schema`、`rtsql :memory: dump` 等（import 以外的生命周期子命令）
- **THEN** 维持既有退出码 2 拒绝（豁免面仅 import）

### Requirement: 标量子查询输出列的表头形状

SELECT 清单含标量子查询项（`SELECT col, (SELECT …) AS alias FROM t` 形态）时，CLI 输出表头 SHALL 在该标量列的输出位置（`result_column_index`）携带其列名（别名或表达式名），SHALL NOT 返回未插入标量列名的输入计划列名。json 输出 `columns` 数组长度 SHALL 与 `rows` 每行值数一致；table/csv/tsv 表头 SHALL 与值列对齐。标量列名来源与既有执行器插入语义一致（`SubqueryEvalExecutor` 在 `result_column_index` 插入标量值，索引越界时追加）。不含标量子查询项的既有形态（含 I034 已锁定的扫描投影表头）SHALL 逐字节保持。来源：MS17-T02（ISS03；change `2026-09-23-ms17-t02-defect-closeout`）。

#### Scenario: 表头在标量位置携带列名

- **GIVEN** 表 `emp(id INT, name VARCHAR, salary INT)` 与 `dept(rid INT, region VARCHAR)` 存在关联数据（标量子查询可返回单行）
- **WHEN** `SELECT id, (SELECT region FROM dept WHERE dept.rid = emp.id) AS region FROM emp`，输出格式 json
- **THEN** `columns` 为 `["id", "region", "name", "salary"]`（别名在 index 1，输入计划列名保持原序列于其后），`columns` 长度等于每行 `rows` 值数，标量值位于行内 index 1

#### Scenario: 标量子查询位于中间位置时 table 表头对齐

- **GIVEN** 同上数据
- **WHEN** `SELECT id, name, (SELECT region FROM dept WHERE dept.rid = emp.id) AS region FROM emp`，输出格式 table
- **THEN** 表头为 `id | name | region | salary` 四列，与四值行对齐，标量列名位于标量值所在位置（index 2）；change 前表头为 `[id, name, salary]` 三列，对四值行错位且无 `region` 列名

#### Scenario: 非标量子查询形态零回归

- **GIVEN** 既有投影表头用例（I034 锁定的裸 DataScan/IndexScan 表头断言与 cli_test 全部既有用例）
- **WHEN** 全量测试运行
- **THEN** 全部零修改通过；不含标量子查询的查询表头行为逐字节不变

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

### Requirement: 生命周期子命令 delete

`rtsql delete <db>` SHALL 删除指定数据库的主文件与伴生文件（`<db>.wal`、`<db>.checkpoint`，与引擎 `with_extension` 命名一致）。定位 SHALL 复用既有数据库名称解析（裸名 → `$RTSQL_HOME/db/<name>.db`；含 `/` 形态按路径直开）。删除 SHALL 无交互确认直接执行，并 SHALL 逐文件报告释放结果（路径与释放字节数）；伴生文件不存在时 SHALL 跳过并在报告中说明，SHALL NOT 视为错误。`--dry-run` SHALL 列出将删除的文件与大小且 SHALL NOT 执行任何删除、SHALL NOT 修改文件系统状态。目标库正被打开（advisory 独占锁被其他会话持有）时 SHALL 以 `DatabaseLocked` 显式拒绝（退出码 4）且 SHALL NOT 删除任何文件；锁探测与删除 SHALL 在同一锁持有窗口内完成。库不存在时 SHALL 以既有生命周期命令的缺失语义拒绝（点名路径，退出码 1）。delete SHALL 为文件级管理操作，SHALL NOT 依赖库内容有效性：主文件头部无效或为零字节时 SHALL 同样可删（不初始化、不校验、不要求密钥——加密库的删除与明文库无差别）。

#### Scenario: 删除裸名库并报告释放

- **GIVEN** 集中存储区存在库 `mydb`（主文件与 `.wal` 伴生存在，无 checkpoint 位点）
- **WHEN** `rtsql delete mydb`
- **THEN** 退出码 0，输出逐文件报告主文件与 `mydb.wal` 已删除及释放字节数
- **AND** 文件系统中三个可能路径均不存在；`rtsql list` 不再枚举该库

#### Scenario: dry-run 只列出不删除

- **GIVEN** 同上
- **WHEN** `rtsql delete mydb --dry-run`
- **THEN** 退出码 0，输出与实际删除相同的逐文件清单（含将释放字节数）
- **AND** 所有文件仍在，`rtsql db "SELECT 1"`（指向同库）仍可执行

#### Scenario: 库打开中显式拒绝且零删除

- **GIVEN** 库 `mydb` 已被另一进程打开（持锁中）
- **WHEN** `rtsql delete mydb`
- **THEN** 退出码 4，stderr 点名库被占用（`DatabaseLocked` 语义）
- **AND** 主文件与伴生文件全部原样保留

#### Scenario: 库不存在点名拒绝

- **WHEN** `rtsql delete nosuchdb`
- **THEN** 退出码 1，stderr 点名解析后的路径不存在

#### Scenario: 路径形态与伴生清理

- **GIVEN** 自定义路径 `/tmp/data/x.db` 存在且伴生 `/tmp/data/x.wal`、`/tmp/data/x.checkpoint` 均存在
- **WHEN** `rtsql delete /tmp/data/x.db`
- **THEN** 退出码 0，三个文件全部删除并逐文件报告

#### Scenario: 头部无效文件仍可删

- **GIVEN** 集中存储区存在 `corrupt.db`（内容为随机字节，非有效 RTsql 头）
- **WHEN** `rtsql delete corrupt`
- **THEN** 退出码 0，文件被删除（delete 不做库内容校验，不触发头初始化或校验拒绝）

#### Scenario: 既有生命周期子命令与主命令零回归

- **WHEN** 重复执行既有 `new/list/schema/dump/restore/import` 与主命令用例集
- **THEN** 行为与退出码逐字节一致；裸名与子命令名冲突时子命令优先的既有语义不变（名为 `delete` 的库经主命令形态不可达为既有语义的既知边界）

### Requirement: 锁等待 busy-timeout 通道

CLI SHALL 提供全局 `--busy-timeout <MS>` flag（对齐 `--key` 的 global 先例，作用于全部开库命令）与 env 通道 `RTSQL_BUSY_TIMEOUT`（显式 flag 优先），以毫秒为单位配置 `database-file-lock` 域定义的锁等待预算。值 SHALL 为非负十进制整数：非数字、负数或空值 SHALL 在任何开库动作前以 exit 2 拒绝（对齐 `RTSQL_WAL_CHECKPOINT_BYTES` 校验先例）。未配置时使用默认预算 5000ms。显式配置 0 表示不额外等待（仅保留 `database-file-lock` 域的内部可见性窗口下限）。`:memory:` 主命令组合 SHALL 接受该 flag/env 并忽略（内存库无文件锁；env 全局导出 SHALL NOT 破坏 `:memory:` 使用）。`delete` 子命令的锁探测路径 SHALL NOT 受等待预算影响（立即探测、锁占用即拒绝，行为不变）。

#### Scenario: flag 等待后成功

- **GIVEN** 另一进程持有一个已建库的写锁并将在 1 秒内释放
- **WHEN** `rtsql --busy-timeout 5000 x.db "INSERT ..."`
- **THEN** 第二方等待至锁释放，语句执行成功 exit 0

#### Scenario: env 生效且 flag 优先

- **GIVEN** `RTSQL_BUSY_TIMEOUT=100`，另一进程持有锁并将在 1 秒内释放
- **WHEN** `rtsql x.db "SELECT 1"`（仅 env）
- **THEN** 约 100ms 后 exit 4（`DatabaseLocked`）；同场景加 `--busy-timeout 5000` 则等待至释放后成功（flag 优先）

#### Scenario: 非法值 open 前拒绝

- **GIVEN** 任意库状态（含不存在的库）
- **WHEN** `rtsql --busy-timeout abc x.db "SELECT 1"` / `--busy-timeout -1 ...` / `RTSQL_BUSY_TIMEOUT=`（空）
- **THEN** exit 2，错误点名非法值与通道；未创建文件、未执行语句

#### Scenario: 内存库组合接受并忽略

- **GIVEN** 无前置条件
- **WHEN** `RTSQL_BUSY_TIMEOUT=3000 rtsql :memory: "SELECT 1"` 及 `rtsql --busy-timeout 3000 :memory: "SELECT 1"`
- **THEN** 均 exit 0 正常执行（忽略等待配置，无拒绝）

#### Scenario: 超时错误面不变

- **GIVEN** 另一进程长期持有 `x.db` 写锁
- **WHEN** `rtsql --busy-timeout 200 x.db "SELECT 1"`
- **THEN** 约 200ms 后 exit 4，错误信息维持既有 `DatabaseLocked` 文案（含库名）

**e2e 验证与缺陷清偿 change（T6/ISS13，2026-10-14 收尾）修正**：env 通道非法值报错面——经 `RTSQL_BUSY_TIMEOUT` 设置非法值（非 u64）SHALL 在任何开库前 exit 2，报错文案 SHALL 点名环境变量 `RTSQL_BUSY_TIMEOUT` 与实际值（不再只指向 `--busy-timeout` flag）；flag 显式值优先语义与退出码 SHALL 不变。`RTSQL_KEY` / `RTSQL_WAL_CHECKPOINT_BYTES` 同族 env 校验报错 SHALL 同样点名各自 env 名（如有同病一并收敛）。

#### Scenario: env 非法值报错点名环境变量

- **WHEN** `RTSQL_BUSY_TIMEOUT=abc rtsql demo "SELECT 1"`
- **THEN** exit 2，报错含 `RTSQL_BUSY_TIMEOUT` 与 `abc`（失败边界——修复前：`error: invalid value 'abc' for '--busy-timeout <MS>'`——env 设置者被误导去查不存在的命令行参数）

#### Scenario: flag 优先与既有锁矩阵零回归

- **WHEN** flag 与 env 同时设置（flag 合法 env 非法 / 两者均合法不同值）；既有锁等待/超时/信号中断用例
- **THEN** flag 优先、锁矩阵行为与退出码不变

### Requirement: 生命周期子命令 checkpoint

`rtsql checkpoint <db>` SHALL：经 `resolve_existing_db` 解析目标库（缺失以既有 General 语义 exit 1 拒绝）→ 以读写模式独占打开（锁冲突经 `--busy-timeout`/env 等待预算后 `DatabaseLocked` exit 4）→ 执行 `Database::checkpoint()` → 向 stdout 输出一行确认并 exit 0；WAL 无未决工作（无追加且无脏页）时 SHALL 输出明示零工作早退的确认并 exit 0（`Database::checkpoint()` 零工作跳过语义不变）。`--readonly` 组合与 `:memory:` 目标 SHALL 以 exit 2 拒绝（对齐既有子命令拒绝先例）。加密库 SHALL 经既有全局 `--key`/`RTSQL_KEY` 打开，密钥错误沿用 exit 5。checkpoint 的执行 SHALL 与 close/水位自动 checkpoint 的全量九步路径同源，落盘后外部备份可见完整已提交数据。

#### Scenario: 写入后显式 checkpoint

- **GIVEN** `x.db` 存在且有已提交写入（WAL 含未 checkpoint 记录）
- **WHEN** `rtsql checkpoint x.db`
- **THEN** exit 0，stdout 输出确认行；随后 `rtsql dump x.db` 可见全部已提交数据，WAL 文件被截断/缩减

#### Scenario: 零工作早退明示

- **GIVEN** `x.db` 干净（无 WAL 未决记录且无脏页）
- **WHEN** `rtsql checkpoint x.db`
- **THEN** exit 0，stdout 确认行明示零工作（无 WAL 截断等副作用）

#### Scenario: 缺失库拒绝

- **GIVEN** `RTSQL_HOME` 下不存在 `nope.db`
- **WHEN** `rtsql checkpoint nope`
- **THEN** exit 1，错误含库名（`resolve_existing_db` 既有文案），未创建文件

#### Scenario: 锁冲突经等待后拒绝

- **GIVEN** 另一进程长期持有 `x.db` 写锁
- **WHEN** `rtsql --busy-timeout 300 checkpoint x.db`
- **THEN** 约 300ms 等待耗尽后 exit 4（`DatabaseLocked` 错误面），未产生 checkpoint 副作用

#### Scenario: 只读与内存库组合拒绝

- **GIVEN** `x.db` 存在
- **WHEN** `rtsql --readonly checkpoint x.db`；`rtsql checkpoint :memory:`
- **THEN** 均 exit 2，错误点名组合不支持

#### Scenario: 加密库 checkpoint

- **GIVEN** 以 `--key secret` 创建的加密库 `enc.db` 有未决 WAL
- **WHEN** `rtsql --key secret checkpoint enc.db`；随后 `rtsql --key wrong checkpoint enc.db`
- **THEN** 正确密钥 exit 0 完成落盘；错误密钥 exit 5（既有解密失败错误面）

### Requirement: CLI 错误输出携带 SQLSTATE

CLI 错误输出 SHALL 在 `Response::Error` 携带 `sqlstate` 时渲染该代码：文本格式在错误行尾附加 ` [SQLSTATE xxxx]`，`--format json` 的错误对象 SHALL 包含 `sqlstate` 字段（`sqlstate` 缺失的错误——Usage/General 类——SHALL 省略该字段与后缀）。既有错误文案、退出码矩阵、非错误输出的序列化 SHALL 零变化；PG 协议错误响应的 SQLSTATE 携带（error-reporting R2）SHALL 零变化。

#### Scenario: 只读拒绝文本携带 SQLSTATE

- **GIVEN** `--readonly` 会话
- **WHEN** `INSERT` 被计划期拒绝
- **THEN** 错误行含 `[SQLSTATE 25006]`，文案与 exit 3 不变（失败边界——修复前：纯文字无代码）

#### Scenario: JSON 格式携带字段

- **GIVEN** 同上
- **WHEN** `--format json`
- **THEN** 错误 JSON 含 `"sqlstate":"25006"`；无 sqlstate 的错误（如缺库 General）省略字段

#### Scenario: PG 面与非错误输出零变化

- **GIVEN** serve/PG 客户端与正常查询
- **WHEN** 既有 pg_integration 断言与正常结果渲染
- **THEN** 零回归

### Requirement: 多语句 JSONL 与 DDL affected_rows 输出形态（锁定）

多语句输入以 `--format json` 执行时，输出 SHALL 为逐语句一个独立 JSON 对象（JSONL 流，含既有 fail-fast 序号定位）；DDL 语句成功 SHALL 输出 `{"affected_rows":0}`。两者为文档化设计形态（2026-10-13 用户裁定维持现状），以测试与文档锁定；退出码与渲染语义 SHALL 零变化。

#### Scenario: 多语句 JSONL 形态

- **GIVEN** 库与两条查询语句以 `;` 连接
- **WHEN** `rtsql demo --format json "SELECT 1; SELECT 2"`
- **THEN** stdout 两行独立 JSON 对象（每行可独立解析）

#### Scenario: DDL affected_rows 形态

- **WHEN** `rtsql demo --format json "CREATE TABLE t (id INTEGER)"`
- **THEN** 输出 `{"affected_rows":0}`、exit 0

#### Scenario: help 文本无内部开发标记

- **WHEN** `rtsql --help` / `rtsql import --help`
- **THEN** 帮助文本不含内部任务编号（`MS2x` 类）与提案标记（`proposal`）（失败边界——修复前：`--busy-timeout` help 含「MS28 T1：」、`--readonly` help 含「MS26 T1：」「proposal 假设 10」）

### Requirement: 大值行集输出组合（锁定）

主命令 SELECT 与分析子命令 stats/sample/profile 对含超限大值（≥65536 字符）的行集 SHALL 全格式可用：`table` exit 0 且超限单元格截断；`json`/`csv`/`tsv` 全值保真。stats 的 String 列 min/max、sample 行渲染、profile top-k 等携带原值的输出路径 SHALL 同受 table 截断保护、json 保真。本 Requirement 锁定 2026-10-14 组合空格清偿后的输出组合行为。

#### Scenario: 大值 × 主命令四格式

- **GIVEN** 表含 100,000 字符 STRING 值（E8 大值旅程表）
- **WHEN** `SELECT body FROM docs` 分别以四格式执行
- **THEN** json/csv/tsv 全值保真；table exit 0、数据行恰 64 字符以 `...` 结尾（失败边界——修复前 table exit 101）

#### Scenario: 大值 × 分析子命令（用户实测 panic 路径）

- **GIVEN** 表含 70,000 字符 STRING 列值（stats min/max、sample 行渲染、profile top-k 均携带原值）
- **WHEN** stats/sample/profile 以 `--format table` 执行
- **THEN** 三命令 exit 0，超限值截断显示；stats 以 `--format json` 执行时 min/max SHALL 携带完整原值（失败边界——修复前 exit 101，与用户 2026-10-14 实测一致）

### Requirement: import 与分析子命令非默认运行形态（锁定）

`import --csv` 目标为加密文件库时 SHALL 经 `--key` 打开写入：对钥 exit 0 且导入数据可查；错钥 exit 5（与主命令密钥错误面一致）。stats/sample/profile 对加密文件库 SHALL 经 `--key` 可用（exit 0）。stats/sample/profile 与 `--readonly` 组合 SHALL Usage exit 2 且报错点名 `--readonly` 归属主命令（与 schema/dump/delete/restore/import 同一拒绝面，`src/cli/mod.rs` 通用拒绝臂）。本 Requirement 锁定既有通道与拒绝面的组合行为（此前零见证）。

#### Scenario: import × 加密库对钥

- **GIVEN** `--key pw` 创建的加密库含目标表，CSV 文件就绪
- **WHEN** `rtsql --key pw import <enc-db> <table> <file> --csv`
- **THEN** exit 0，`--key pw` 查询导入行数正确（数据真实落库可查）

#### Scenario: import × 加密库错钥

- **GIVEN** 同上加密库
- **WHEN** `rtsql --key wrong import <enc-db> <table> <file> --csv`
- **THEN** exit 5，零行写入

#### Scenario: 分析子命令 × 加密库

- **GIVEN** `--key pw` 创建并写入的加密库
- **WHEN** `rtsql --key pw stats <enc-db> <table> --format json`
- **THEN** exit 0，统计输出正常

#### Scenario: 分析子命令 × --readonly 拒绝

- **WHEN** `rtsql --readonly stats|sample|profile <db> <table>`
- **THEN** 三者均 Usage exit 2，报错点名 `--readonly` 仅属主命令（失败边界——此前拒绝矩阵无此三项见证）
