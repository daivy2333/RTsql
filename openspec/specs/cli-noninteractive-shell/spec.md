# cli-noninteractive-shell Specification

## Purpose
TBD - created by archiving change 2026-09-06-ms10-t01-cli-shell. Update Purpose after archive.
## Requirements
### Requirement: 参数化 CLI 入口与主命令

`rtsql` 二进制 SHALL 提供 `rtsql <db> <sql>` one-shot 主命令：解析参数、打开数据库、执行单条 SQL、渲染结果到 stdout、以分类退出码退出。进程正常退出前 SHALL 调用 `Database::close()`（checkpoint + WAL 截断）。参数缺失或非法时 SHALL 以退出码 2 报用法错误。

#### Scenario: one-shot SELECT 执行成功

- **GIVEN** 集中存储或指定路径下存在含数据的库
- **WHEN** `rtsql <db> "SELECT id, name FROM t WHERE id = 1"`
- **THEN** stdout 输出查询结果（含列名表头），退出码 0
- **AND** 进程退出前完成了 checkpoint（WAL 被截断，重开无 redo 负担）

#### Scenario: 用法错误退出码 2

- **GIVEN** 任意环境
- **WHEN** `rtsql`（无参数）或 `rtsql --format bogus db "SELECT 1"`（非法选项值）
- **THEN** stderr 输出用法信息，退出码 2，不打开任何数据库

#### Scenario: SQL 错误退出码 3

- **GIVEN** 已打开的库
- **WHEN** `rtsql <db> "SELEC typo"` 或 `rtsql <db> "SELECT * FROM missing_table"`
- **THEN** stderr 输出错误信息，退出码 3

#### Scenario: 退出码枚举为后续任务留位

- **GIVEN** CLI 退出码枚举（0/2/3/4/5）
- **WHEN** 本 capability 落地后的代码审查
- **THEN** 退出码 4（锁冲突）与 5（密钥）已存在于枚举与映射表中，但尚无产生路径（T02/MS12 落地）

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

### Requirement: 多语句显式拒绝（临时护栏）

T01 阶段 CLI SHALL 对包含多条语句的 SQL 显式报错（parse 后语句数 > 1），退出码 3，错误信息说明当前版本每次只执行一条语句；不得静默截断。MS10-T04 落地分片执行后本护栏退役。

#### Scenario: 多语句被拒绝

- **GIVEN** 任意库
- **WHEN** `rtsql db "INSERT INTO t VALUES (1); INSERT INTO t VALUES (2)"`
- **THEN** stderr 报"每次只执行一条语句"类错误，退出码 3
- **AND** 两条 INSERT 均未执行（不是只执行第一条）

### Requirement: 扫描执行器真投影（Iteration 001）

`SELECT` 的投影列表 SHALL 决定扫描路径返回行的形状：四个扫描执行器（Scan / DataScan / IndexScan / IndexScanAll）SHALL 按投影裁剪产出行，plan 节点的 `columns` 元数据与行形状一致。谓词求值（WHERE / 下推谓词 / MVCC 可见性）SHALL 在全 schema 行上先行完成，投影只发生在行产出最后一步。`SELECT *` 的投影等于全 schema，行为不变。

#### Scenario: 子集投影在全部扫描路径返回投影列

- **GIVEN** 表 `s(id INT PRIMARY KEY, name STRING)` 含一行 `(1, 'Alice')`
- **WHEN** 分别执行 `SELECT name FROM s`（DataScan 路径）与 `SELECT name FROM s WHERE id = 1`（IndexScan 路径）
- **THEN** 两条查询都返回单列：表头 `["name"]`、行 `[["Alice"]]`
- **AND** 表头列数与每行字段数一致（任何路径无错位）

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

#### Scenario: 既有测试按投影语义校准

- **GIVEN** 既有测试套件中假设"子集投影返回全 schema 行"的断言
- **WHEN** 本 Requirement 落地
- **THEN** 受影响断言按投影语义校准（只改行形状期望，不改测试意图），校准清单记录于 Act Response
- **AND** `cargo test --all` 全绿

