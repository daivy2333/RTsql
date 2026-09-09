# cli-noninteractive-shell Specification

## MODIFIED Requirements

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
