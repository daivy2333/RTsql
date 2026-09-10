# sql-transaction-statements Specification

## ADDED Requirements

### Requirement: 事务语句 CLI 会话往返

CLI one-shot 调用 SHALL 维护会话事务态：`BEGIN`（含 `BEGIN TRANSACTION`/`START TRANSACTION`，不带任何 mode）开启显式事务，`COMMIT` 提交，`ROLLBACK` 回滚。事务开启后、终结前的语句 SHALL 在该事务内执行（等价于 lib `begin` → `execute_in_tx` → `commit/rollback` 路径），SHALL NOT 逐条 auto-commit。事务语句成功时 SHALL 返回受影响行数 0（与其他语句同构渲染，如 `{"affected_rows":0}`）。COMMIT 后的写入 SHALL 对后续调用可见（跨进程重开可查）；ROLLBACK 后的写入 SHALL 完全不可见。事务内 SELECT 与 DDL SHALL 沿用引擎既有可见性与生效语义（SELECT 走无快照扫描：本事务未提交 INSERT 可见、DELETE 后行不再出现、UPDATE 后新旧版本同时出现；DDL 立即生效且不被 ROLLBACK 撤销）——本文档化既有引擎行为，SHALL NOT 改变。

#### Scenario: BEGIN→INSERT→COMMIT 提交可见

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "BEGIN; INSERT INTO t VALUES (1); COMMIT"`
- **THEN** 退出码 0，stdout 顺序输出 BEGIN 的受影响行数 0、INSERT 的受影响行数 1、COMMIT 的受影响行数 0
- **AND** 重开查询可见 `id=1`（提交已持久化）

#### Scenario: BEGIN→INSERT→ROLLBACK 无残留

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "BEGIN; INSERT INTO t VALUES (1); ROLLBACK"`
- **THEN** 退出码 0
- **AND** 重开查询 `t` 为空（回滚无残留）

#### Scenario: 事务内 SELECT 见本事务未提交写入

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "BEGIN; INSERT INTO t VALUES (1); SELECT id FROM t; COMMIT"`
- **THEN** SELECT 输出 1 行 `id=1`（本事务未提交写入可见）

#### Scenario: 事务内 SELECT 的 UPDATE 新旧版本并存（既有语义文档化）

- **GIVEN** 表 `t(id INT PRIMARY KEY, name VARCHAR)` 含一行 `id=1, name='a'`
- **WHEN** `rtsql db "BEGIN; UPDATE t SET name='b' WHERE id=1; SELECT name FROM t; ROLLBACK"`
- **THEN** SELECT 输出 `a`、`b` 两行（无快照扫描：未提交新版本不抑制旧版本，既有引擎语义）
- **AND** 重开查询仍为 `name='a'`（ROLLBACK 生效）

#### Scenario: 事务内 DDL 立即生效且不被 ROLLBACK 撤销（既有语义文档化）

- **GIVEN** 数据库无表 `t2`
- **WHEN** `rtsql db "BEGIN; CREATE TABLE t2 (id INT PRIMARY KEY); ROLLBACK"`
- **THEN** 退出码 0
- **AND** 重开 `rtsql db "SELECT * FROM t2"` 可执行（表存在，DDL 未被回滚；返回空行集）

### Requirement: 边界子句显式拒绝

以下事务边界子句 SHALL 显式拒绝为 SQL 错误（CLI 退出码 3），错误信息 SHALL 点名不被支持的子句，SHALL NOT 静默忽略或部分执行：`SET TRANSACTION`；`SAVEPOINT <名>`；`RELEASE SAVEPOINT <名>`；`BEGIN`/`START TRANSACTION` 携带任何 transaction mode（如 `ISOLATION LEVEL`、`READ ONLY/WRITE`）；`COMMIT AND CHAIN`；`ROLLBACK AND CHAIN`；`ROLLBACK [TO [SAVEPOINT] <名>]`（带 savepoint 的回滚）。`AND NO CHAIN` 形态 SHALL 按裸语句语义执行：sqlparser 0.44 将 `COMMIT AND NO CHAIN`/`ROLLBACK AND NO CHAIN` 与裸 `COMMIT`/`ROLLBACK` 解析为同一 AST（`chain: false`），二者不可区分，SHALL NOT 为其产生独立拒绝。拒绝 SHALL 发生在任何语句执行之前（会话态不变）。

#### Scenario: SET TRANSACTION 拒绝

- **WHEN** `rtsql db "SET TRANSACTION ISOLATION LEVEL READ COMMITTED"`
- **THEN** 退出码 3，stderr 信息含 `SET TRANSACTION` 且说明不支持

#### Scenario: SAVEPOINT 与 RELEASE SAVEPOINT 拒绝

- **WHEN** `rtsql db "SAVEPOINT sp1"`；随后 `rtsql db "RELEASE SAVEPOINT sp1"`
- **THEN** 两次均退出码 3，stderr 信息分别点名 `SAVEPOINT` / `RELEASE SAVEPOINT` 不支持

#### Scenario: BEGIN 携带 transaction mode 拒绝

- **WHEN** `rtsql db "BEGIN ISOLATION LEVEL SERIALIZABLE"`；随后 `rtsql db "START TRANSACTION READ ONLY"`
- **THEN** 两次均退出码 3，stderr 信息说明 BEGIN/START TRANSACTION 不支持事务模式

#### Scenario: CHAIN 与 ROLLBACK TO SAVEPOINT 拒绝

- **WHEN** `rtsql db "COMMIT AND CHAIN"`；随后 `rtsql db "ROLLBACK AND CHAIN"`；随后 `rtsql db "ROLLBACK TO SAVEPOINT sp1"`
- **THEN** 三次均退出码 3，stderr 信息分别点名 `AND CHAIN` / `ROLLBACK TO SAVEPOINT` 语义不支持

#### Scenario: AND NO CHAIN 与裸语句同义（AST 等价）

- **GIVEN** 无会话事务
- **WHEN** `rtsql db "COMMIT AND NO CHAIN"`
- **THEN** 退出码 3，stderr 信息含 `no active transaction`（SHALL NOT 点名 AND CHAIN——sqlparser 0.44 中该形态与裸 `COMMIT` 解析为同一 AST，按裸语句语义执行）

### Requirement: 会话状态边界

无活跃会话事务时 `COMMIT` 或 `ROLLBACK` SHALL 报 SQL 错误（退出码 3，信息含 `no active transaction`）。会话事务活跃中再次 `BEGIN` SHALL 报 SQL 错误（退出码 3，信息含 `transaction already active`），且已开启的事务 SHALL 保持开启。CLI 调用的语句循环结束时若会话事务仍开启（用户未 COMMIT/ROLLBACK），CLI SHALL 显式回滚该事务、向 stderr 输出未提交回滚提示、仍以退出码 0 结束（与 psql/静默回滚先例一致，写入不持久化是可观察后果）。事务内语句失败时 fail-fast 语义不变（退出码 3），但 CLI SHALL 在返回前显式回滚会话事务，且错误信息 SHALL 注明先前语句未提交并已随事务回滚（区别于 auto-commit 上下文的"已提交"注明）。

#### Scenario: 无活跃事务时 COMMIT/ROLLBACK 报错

- **GIVEN** 表 `t(id INT)` 已存在，无会话事务
- **WHEN** `rtsql db "COMMIT"`；随后 `rtsql db "ROLLBACK"`
- **THEN** 两次均退出码 3，stderr 信息含 `no active transaction`
- **AND** `t` 数据不变

#### Scenario: 嵌套 BEGIN 报错且原事务保持

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "BEGIN; INSERT INTO t VALUES (1); BEGIN; COMMIT"`
- **THEN** 第二条 `BEGIN` 处失败：退出码 3，stderr 信息含 `transaction already active`
- **AND** 事务在调用收尾被回滚，重开查询 `t` 为空

#### Scenario: 调用收尾未提交事务隐式回滚并提示

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "BEGIN; INSERT INTO t VALUES (1)"`（无 COMMIT/ROLLBACK）
- **THEN** 退出码 0，stderr 输出未提交回滚提示
- **AND** 重开查询 `t` 为空（写入不持久化）

#### Scenario: 事务内失败 fail-fast 注明未提交并回滚

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "BEGIN; INSERT INTO t VALUES (1); INSERT INTO missing_table VALUES (2)"`
- **THEN** 退出码 3，stderr 错误信息含失败语句序号（第 3 条/共 3 条）且注明先前语句未提交、已随事务回滚
- **AND** 重开查询 `t` 为空（无部分生效）

### Requirement: 非会话路径显式拒绝

lib 单结果执行路径 SHALL 显式拒绝事务语句：`Database::execute_sql`（网络 JSON/PG 协议同源）与 `execute_in_tx` 遇 `BEGIN`/`START TRANSACTION`/`COMMIT`/`ROLLBACK` SHALL 返回 `Response::Error`，错误信息 SHALL 指明事务语句仅在 rtsql CLI 会话受支持。拒绝 SHALL 发生在 plan 构建与计划缓存写入之前：事务语句 SHALL NOT 进入计划缓存。

#### Scenario: execute_sql 拒绝 BEGIN

- **GIVEN** 已打开的 `Database`
- **WHEN** `db.execute_sql("BEGIN").await`
- **THEN** 返回 `Response::Error`，信息指明事务语句仅在 rtsql CLI 会话受支持
- **AND** `plan_cache` 长度不变

#### Scenario: execute_in_tx 拒绝 COMMIT

- **GIVEN** 已打开的 `Database` 与经 `begin()` 获得的活跃事务
- **WHEN** `db.execute_in_tx("COMMIT", &tx).await`
- **THEN** 返回 `Response::Error`（事务语句经会话语义处理，不经语句路径执行）
- **AND** 该事务仍可继续执行 DML 并正常提交（拒绝不终结事务）

#### Scenario: 网络路径同源拒绝

- **GIVEN** 运行中的 `Server`（JSON 协议）与已连接客户端
- **WHEN** 客户端发送 `Request::Query { sql: "COMMIT" }`
- **THEN** 收到错误响应，信息与 `execute_sql` 拒绝面一致

### Requirement: 既有语义零回归

本变更 SHALL NOT 改变任何既有行为：显式事务 API（`begin`/`commit`/`rollback`/`execute_in_tx`）语义、错误分类与可见性行为保持逐字节等价；不含事务语句的多语句脚本逐条 auto-commit 语义保持不变；既有测试套件 SHALL 零修改通过（允许的例外：为修正错误信息断言而做的等价校准，须逐处列出）。

#### Scenario: 显式事务 API 语义不变

- **GIVEN** 既有 `tests/explicit_tx_test.rs` 全部场景
- **WHEN** 全量回归
- **THEN** 8 测试零修改通过（多表提交可见、DDL+DML 同事务、回滚无残留、回滚恢复更新值、失败语句后事务存活、双 commit/rollback 报错、隐式路径不受影响、tx_id 复用）

#### Scenario: 无事务语句的多语句脚本语义不变

- **GIVEN** 既有 cli_test 多语句场景（逐条 auto-commit、fail-fast 已提交注明、语法错误零执行、分号边界）
- **WHEN** 全量回归
- **THEN** 全部既有断言零修改通过

#### Scenario: 全量基线通过

- **WHEN** `cargo test`、`cargo clippy -D warnings`、`cargo fmt --check`、`openspec validate`
- **THEN** 全部通过且 warning 为 0（测试总数只增不减）
