# readonly-open-mode Delta — MS26 T1 只读打开模式

## ADDED Requirements

### Requirement: 只读打开入口与共享锁

系统 SHALL 提供只读打开模式：lib 经 `Database::open_readonly(path)` 与 `Database::open_readonly_with_key(path, key)`；CLI 主命令经 `--readonly` flag。只读打开 SHALL 对主数据库文件获取 advisory 共享锁（`try_lock_shared`），共享锁之间 SHALL 互不排斥（多个只读会话可同时打开同一数据库）。共享锁被其他持有者的独占锁排除（`WouldBlock`）SHALL 返回既有 `StorageError::DatabaseLocked`（CLI exit 4），SHALL NOT 等待或重试。只读打开对不存在的数据库文件 SHALL 报错拒绝，SHALL NOT 创建文件。加密库只读打开 SHALL 按提供的密钥正常解密读取（错误密钥沿既有 `InvalidKey` 拒绝）。共享锁 SHALL 与文件描述符同生命周期（drop/进程退出自动释放），与既有独占锁的生命周期语义同型。

#### Scenario: 两只读会话并发打开与查询

- **GIVEN** 一个 checkpoint-clean 的数据库文件 `x.db` 含表 `t`
- **WHEN** 两个独立的只读连接先后打开 `x.db` 并各自执行 `SELECT * FROM t`
- **THEN** 两个连接均打开成功，查询结果一致且完整；任一连接关闭后另一连接不受影响

#### Scenario: 写者持独占锁时只读者被拒

- **GIVEN** 某持有者已以读写模式打开 `x.db`（独占锁在持）
- **WHEN** 只读模式打开 `x.db`
- **THEN** 打开失败返回 `StorageError::DatabaseLocked`，主文件与伴生文件未被触碰

#### Scenario: 只读者持共享锁时写者被拒

- **GIVEN** 某持有者已以只读模式打开 `x.db`
- **WHEN** 读写模式打开 `x.db`
- **THEN** 打开失败返回 `StorageError::DatabaseLocked`（写写互斥语义同样排除写者）

#### Scenario: 只读打开缺失文件不创建

- **GIVEN** 路径 `missing.db` 不存在
- **WHEN** 只读模式打开（lib 或 CLI `--readonly`）
- **THEN** 打开失败并报错点名路径，目录中 SHALL NOT 出现任何新文件

#### Scenario: 加密库只读打开

- **GIVEN** MS17 加密库
- **WHEN** `open_readonly_with_key(path, 正确密钥)` 后执行 SELECT
- **THEN** 查询结果与明文语义一致；错误密钥沿既有 `InvalidKey` 拒绝

#### Scenario: --readonly 与生命周期子命令组合拒绝

- **GIVEN** CLI
- **WHEN** `rtsql --readonly schema <db>`（`new`/`dump`/`restore`/`import`/`delete` 同型）
- **THEN** Usage 点名拒绝（exit 2），不静默忽略 flag——生命周期子命令维持读写打开（proposal 假设 10）

### Requirement: WAL 未决拒绝

只读打开 SHALL 在任何页写入与恢复重放之前解析 `.wal` 全部记录：`.wal` 不存在、为空、或仅含 `Checkpoint` 标记记录（干净关闭形态）SHALL 允许打开；存在任何其他记录（数据重做记录、未提交事务记录）SHALL 以点名错误拒绝打开，错误信息 SHALL 说明「存在未决 WAL，需先以读写模式打开完成恢复」。拒绝时 SHALL NOT 修改数据库文件与任何伴生文件。

#### Scenario: checkpoint-clean 库只读打开成功

- **GIVEN** `x.db` 经一次读写会话正常 close（WAL 仅含 Checkpoint 标记）
- **WHEN** 只读模式打开
- **THEN** 打开成功，数据为最近一次提交的完整状态

#### Scenario: WAL 未决时只读打开被拒

- **GIVEN** 读写会话写入若干行后进程被强杀（SIGKILL，WAL 含未重放记录）
- **WHEN** 只读模式打开 `x.db`
- **THEN** 打开失败，错误点名未决 WAL 与恢复指引；文件未被修改

#### Scenario: 拒绝后读写打开恢复可用

- **GIVEN** 同上未决状态
- **WHEN** 读写模式打开 `x.db`（执行既有恢复）并正常 close，随后只读模式打开
- **THEN** 只读打开成功且数据完整（未决拒绝不妨碍恢复路径）

### Requirement: 只读会话零写入

只读会话的生命周期 SHALL NOT 对数据库文件与伴生文件（`.wal`/`.checkpoint`）产生任何写入：不运行崩溃恢复、不执行 checkpoint（`close()` 与显式 `checkpoint()` 对只读会话 SHALL 为不产生文件写入的无操作）、不创建伴生文件。只读会话内的查询结果 SHALL 来自磁盘现有页内容（含 Checkpoint 标记之前已刷盘的全部已提交状态）。

#### Scenario: 只读会话前后文件字节一致

- **GIVEN** checkpoint-clean 的 `x.db`（记录打开前主文件、`.wal`、`.checkpoint` 的字节 checksum）
- **WHEN** 只读会话执行若干 SELECT（含聚合、索引点查）后 `close()` 并退出
- **THEN** 全部文件字节与打开前一致（零写入见证）

#### Scenario: 只读会话执行显式 checkpoint 无操作

- **GIVEN** 只读会话
- **WHEN** 调用 lib `checkpoint()` / `close()`
- **THEN** 成功返回且不产生文件写入（同上 checksum 见证）

### Requirement: 只读拒绝写操作

只读会话中的 INSERT / UPDATE / DELETE / UPSERT / REPLACE / CREATE TABLE / DROP TABLE / ALTER TABLE / TRUNCATE / CREATE INDEX / DROP INDEX / CREATE VIEW / DROP VIEW / CTAS SHALL 在计划期（解析后、任何写入生效前）以点名错误拒绝，错误信息 SHALL 点名只读状态；该错误 SHALL 归类 SQLSTATE `25006`（read_only_sql_transaction），CLI 退出码 3。SELECT、`ATTACH`、`DETACH` 与事务控制语句（`BEGIN`/`COMMIT`/`ROLLBACK` 等）SHALL NOT 受只读限制（只读会话内显式事务包住只读 SELECT 合法；事务内写语句同样被本 Requirement 拒绝）。

#### Scenario: 只读会话 DML 拒绝

- **GIVEN** 只读会话
- **WHEN** 执行 `INSERT INTO t VALUES (...)`（UPDATE/DELETE/UPSERT/REPLACE 同矩阵）
- **THEN** 报错点名只读（SQLSTATE 25006，CLI exit 3），表数据无变化

#### Scenario: 只读会话 DDL 拒绝

- **GIVEN** 只读会话
- **WHEN** 执行 `CREATE TABLE ...`（DROP/ALTER/TRUNCATE/INDEX/VIEW/CTAS 同矩阵）
- **THEN** 报错点名只读，schema 无变化

#### Scenario: 只读会话只读语句不受限

- **GIVEN** 只读会话
- **WHEN** 执行 SELECT（含 JOIN/聚合/子查询）、`ATTACH`、事务语句包住 SELECT
- **THEN** 全部正常执行，行为与读写会话同形态一致

### Requirement: 只读会话 ATTACH 附加库只读

只读会话中的 `ATTACH '<path>' AS <alias>` SHALL 以只读模式打开附加库（共享锁 + WAL 未决拒绝，同主库语义）；会话内对附加库的写操作引用 SHALL 按 MS20 只读边界与本域只读拒绝面点名拒绝。锁冲突与 WAL 未决的附加库 SHALL 使 ATTACH 语句失败且零副作用（同 MS20 ATTACH 失败收口语义）。

#### Scenario: 只读会话跨库只读分析

- **GIVEN** 只读会话已打开主库 `a.db`，另一进程正以读写模式打开 `b.db` 并持有写锁
- **WHEN** 在只读会话中 `ATTACH 'b.db' AS b` 后查询 `b.t`
- **THEN** ATTACH 失败（`DatabaseLocked`）——附加库被写者独占；写者 close 后重新 ATTACH 成功，跨库只读查询可用

#### Scenario: 附加库 WAL 未决拒绝

- **GIVEN** 只读会话，附加库 `b.db` 处于 WAL 未决态
- **WHEN** `ATTACH 'b.db' AS b`
- **THEN** ATTACH 失败并点名附加库未决 WAL，主库会话状态不变

### Requirement: 既有打开语义零回归

默认读写打开（`Database::open`/`open_with_isolation`/`open_with_key` 与 CLI 无 `--readonly` 的全部命令）的加锁、恢复、checkpoint、行为 SHALL 逐字节不变。既有 `database_file_lock_test.rs` 全矩阵 SHALL 保持通过。

#### Scenario: 默认路径零回归

- **WHEN** 运行 `tests/database_file_lock_test.rs` 与完整测试套件
- **THEN** 全部通过；除本 change 新增见证外既有测试零修改
