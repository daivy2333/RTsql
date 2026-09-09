# cli-noninteractive-shell Specification

## Purpose
TBD - created by archiving change 2026-09-06-ms10-t01-cli-shell. Update Purpose after archive.
## Requirements
### Requirement: 参数化 CLI 入口与主命令

`rtsql` 二进制 SHALL 提供 `rtsql <db> <sql>` one-shot 主命令：解析参数、打开数据库、执行 SQL（单条语句，或以 `;` 分隔的多条语句逐条执行）、渲染结果到 stdout、以分类退出码退出。进程正常退出前 SHALL 调用 `Database::close()`（checkpoint + WAL 截断）。参数缺失或非法时 SHALL 以退出码 2 报用法错误。打开时遇跨进程锁冲突 SHALL 以退出码 4 报 `database is locked`。

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

CLI SHALL 对以 `;` 分隔的多条 SQL 语句逐条执行：每条语句独立 plan/execute、独立 auto-commit 事务（逐条生效）。每条语句的结果 SHALL 顺序渲染到 stdout：DML/DDL 输出受影响行数，SELECT 输出查询结果（列名表头等既有渲染语义不变）；`json` 格式下每条语句输出一个独立 JSON 文档。任一语句失败时 SHALL 立即停止执行（fail-fast）：以退出码 3 报错，错误信息 SHALL 包含失败语句的序号（第 k 条/共 n 条）与失败语句文本，并注明失败之前的语句已生效；失败之后的语句 SHALL NOT 执行。SQL 语法错误 SHALL 在任何语句执行前整体拒绝（零执行），错误信息保留解析器的行/列定位。lib 单结果执行路径（`pipeline::execute` 网络路径、`execute_in_tx` 显式事务路径）遇多语句 SHALL 显式报错，SHALL NOT 静默截断。

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

