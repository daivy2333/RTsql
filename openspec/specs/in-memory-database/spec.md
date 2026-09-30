# in-memory-database Specification

## Purpose
约束 `:memory:` 内存库形态：lib `Database::open_in_memory()` 与 CLI 主命令路径实参 `:memory:`（优先于文件名解析）经内存存储后端（`MemoryStorage`，实现既有 `AsyncStorage` 接缝）承载全部页数据——零文件（`.db`/`.wal`/`.checkpoint` 均不产生）、checkpoint/WAL 无操作、进程退出即弃、会话内全形态（CRUD/DDL/事务/CTAS/ATTACH/多语句分片）与文件库同语义。来源：MS26 T3/T4（change `2026-09-30-ms26-embedded-runtime`）。

## Requirements

### Requirement: 内存库打开与全形态会话

系统 SHALL 提供 `:memory:` 内存库形态：lib 经 `Database::open_in_memory()`；CLI 主命令经路径实参 `:memory:` 特判（优先于 `resolve_db_path` 的文件解析）。内存库 SHALL 经内存存储后端承载全部页数据（实现既有 `AsyncStorage` 接缝），SHALL NOT 创建、要求或触碰任何文件（主文件、`.wal`、`.checkpoint` 均不产生）。会话内全形态 SHALL 可用：CRUD、DDL（CREATE/DROP TABLE、索引、视图、ALTER、TRUNCATE）、事务、CTAS 物化、多语句 `;` 分片（MS10-T04）——行为与文件库同语义（约束、错误面、plan cache、渲染一致）。进程退出（或 Database 弃置）后数据即弃，SHALL NOT 有任何持久化或恢复语义。

#### Scenario: 多语句计算管道

- **GIVEN** one-shot CLI 调用 `rtsql :memory: "CREATE TABLE t(id INT PRIMARY KEY, v INT); INSERT INTO t VALUES (1,10),(2,20); SELECT sum(v) FROM t"`
- **THEN** 三条语句依次执行，输出 `30`；进程退出后文件系统无任何新文件

#### Scenario: lib 内存库全 CRUD

- **GIVEN** `Database::open_in_memory()`
- **WHEN** 建表、INSERT、UPDATE、DELETE、SELECT、建索引、建视图
- **THEN** 全部成功且语义与文件库一致（约束拒绝、错误面同型）

#### Scenario: 会话内状态完整可查

- **GIVEN** 内存库写入若干行
- **WHEN** 同会话内执行 `SELECT`、`list` 型查询、plan cache 复用路径
- **THEN** 数据与 schema 完整（内存存储承载全部页，行为不可区分于文件库）

#### Scenario: 进程退出即弃

- **GIVEN** 内存库会话写入数据后正常 close 并退出
- **WHEN** 新进程再次 `rtsql :memory: "SELECT ..."` 或 `open_in_memory()`
- **THEN** 空库（无任何表），且不产生任何文件

### Requirement: 内存库 checkpoint 与 WAL 无操作

内存库 SHALL NOT 运行崩溃恢复，`close()` 与显式 `checkpoint()` SHALL 为不产生任何文件与 WAL I/O 的无操作。事务提交路径 SHALL 保持既有会话内语义（版本可见性、回滚还原），WAL 载体 SHALL 为进程内形态或等效无操作通道（机制随 design 定稿），SHALL NOT 落盘。

#### Scenario: close 与显式 checkpoint 无文件副作用

- **GIVEN** 内存库写入若干行
- **WHEN** 调用 `checkpoint()` 后 `close()`
- **THEN** 成功返回，工作目录与数据目录无任何文件产生或修改

#### Scenario: 事务语义保持

- **GIVEN** 内存库会话 `BEGIN` 写入后 `ROLLBACK`
- **THEN** 回滚语义与文件库一致（行消失、索引条目还原、可重插）

### Requirement: 内存库边界拒绝面

生命周期子命令（`new`/`schema`/`dump`/`restore`/`import`/`delete`）对 `:memory:` 实参 SHALL 点名拒绝（无文件语义可承载）；`:memory:` 与 `--key`/`RTSQL_KEY` 组合 SHALL 点名拒绝（无持久化，加密无意义）；加密库相关错误面 SHALL NOT 适用于内存库。

#### Scenario: 生命周期子命令拒绝

- **GIVEN** CLI 调用 `rtsql :memory: dump`（`schema`/`restore`/`import`/`new`/`delete` 同矩阵）
- **THEN** 点名错误拒绝（`:memory:` 不承载文件语义），exit 码非 0，不产生文件

#### Scenario: 加密组合拒绝

- **GIVEN** `rtsql :memory: "SELECT 1" --key pw`（或 RTSQL_KEY 环境）
- **THEN** 打开期点名拒绝，不进入执行

### Requirement: ATTACH 与跨库组合零特殊化

内存库会话中的 `ATTACH '<path>' AS <alias>` SHALL 按文件库既有语义打开附加库（读写会话独占锁 / 只读会话共享锁，MS20 + MS26 语义）；附加库路由、只读边界与 CTAS 物化进内存主库 SHALL 与文件主库行为一致，SHALL NOT 为内存主库引入特殊分支。

#### Scenario: ATTACH 文件库跨库查询

- **GIVEN** 内存库会话 `ATTACH 'file.db' AS f`（`file.db` 为既有文件库含表 `t`）
- **WHEN** `SELECT * FROM f.t` 与 `CREATE TABLE copy AS SELECT * FROM f.t`
- **THEN** 跨库只读查询返回 `file.db` 数据；CTAS 将副本物化进内存主库，本会话内可查

#### Scenario: 文件库语义零回归

- **WHEN** 运行 `tests/attach_session_test.rs`、`tests/cross_db_query_test.rs`、`tests/ctas_test.rs` 与完整测试套件
- **THEN** 全部通过；除本 change 新增见证外既有测试零修改
