## MODIFIED Requirements

### Requirement: 边界子句显式拒绝

以下事务边界子句 SHALL 显式拒绝为 SQL 错误（CLI 退出码 3），错误信息 SHALL 点名不被支持的子句，SHALL NOT 静默忽略或部分执行：`SET TRANSACTION`；`SAVEPOINT <名>`；`RELEASE SAVEPOINT <名>`；`BEGIN`/`START TRANSACTION` 携带任何 transaction mode（如 `ISOLATION LEVEL`、`READ ONLY/WRITE`）；`COMMIT AND CHAIN`；`ROLLBACK AND CHAIN`；`ROLLBACK [TO [SAVEPOINT] <名>]`（带 savepoint 的回滚）。`AND NO CHAIN` 形态 SHALL 按裸语句语义执行：sqlparser 0.44 将 `COMMIT AND NO CHAIN`/`ROLLBACK AND NO CHAIN` 与裸 `COMMIT`/`ROLLBACK` 解析为同一 AST（`chain: false`），二者不可区分，SHALL NOT 为其产生独立拒绝。拒绝 SHALL 发生在任何语句执行之前（会话态不变）。上述语义 SHALL 同样成立于同一 CLI 调用内会话事务已开启的组合形态：`BEGIN`（或 `START TRANSACTION`）与先行写入语句之后遇边界子句时，该子句 SHALL 以相同退出码与点名文案拒绝，错误信息 SHALL 按既有收尾规则注明先前语句未提交并已随事务回滚，且重开后 SHALL 无部分生效（组合行为由既有 `run_sql` 错误分支与收尾规则组合产生，本 Requirement 将其纳入锁定面）。

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

#### Scenario: 活跃会话事务中遇 SAVEPOINT 拒绝并整体回滚

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "BEGIN; INSERT INTO t VALUES (1); SAVEPOINT sp1"`
- **THEN** 退出码 3，stderr 点名 `SAVEPOINT` 不支持，并注明先前语句未提交、已随事务回滚
- **AND** 重开查询 `t` 为空（无部分生效）

#### Scenario: 活跃会话事务中遇 SET TRANSACTION 拒绝并整体回滚

- **GIVEN** 表 `t(id INT)` 为空
- **WHEN** `rtsql db "BEGIN; INSERT INTO t VALUES (1); SET TRANSACTION ISOLATION LEVEL READ COMMITTED"`
- **THEN** 退出码 3，stderr 点名 `SET TRANSACTION` 不支持，并注明先前语句未提交、已随事务回滚
- **AND** 重开查询 `t` 为空（无部分生效）
