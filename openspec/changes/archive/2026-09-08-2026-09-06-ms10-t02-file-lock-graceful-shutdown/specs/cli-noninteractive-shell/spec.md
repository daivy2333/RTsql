# cli-noninteractive-shell Specification

## MODIFIED Requirements

### Requirement: 参数化 CLI 入口与主命令

`rtsql` 二进制 SHALL 提供 `rtsql <db> <sql>` one-shot 主命令：解析参数、打开数据库、执行单条 SQL、渲染结果到 stdout、以分类退出码退出。进程正常退出前 SHALL 调用 `Database::close()`（checkpoint + WAL 截断）。参数缺失或非法时 SHALL 以退出码 2 报用法错误。打开时遇跨进程锁冲突 SHALL 以退出码 4 报 `database is locked`。

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

## ADDED Requirements

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
