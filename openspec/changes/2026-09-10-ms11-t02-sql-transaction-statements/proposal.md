# MS11-T02: SQL 事务语句 BEGIN/COMMIT/ROLLBACK

## Why

tasks MS11-T02：agent 是写 SQL 的（R18 主题 7），事务是日常件，但当前 SQL 面没有事务语句——事务只能经 lib API（`begin`/`execute_in_tx`/`commit`/`rollback`，MS07-T04）使用，CLI 一次调用只能逐条 auto-commit。本 change 把 `BEGIN`/`COMMIT`/`ROLLBACK` 接入 SQL 面，复用既有显式事务 API，实现面小、同域验收（"agent 写 SQL 的日常件"）。

用户决策（2026-09-10，集中裁定）：

1. **适用面：仅 CLI**——事务语句只在 CLI one-shot 调用内的会话生效；lib 隐式路径（`execute_sql`，含网络 JSON/PG）遇事务语句给出明确错误（planner 语句臂精确报错）。server 无活跃消费者（MS09 降级先例）。
2. **响应形状：复用 `AffectedRows(0)`**——与 CREATE/DROP TABLE 响应一致（`create_table.rs:77`/`drop_table.rs:61`），JSON/PG/CLI 渲染零变更。
3. **边界子句：全部显式拒绝**——`SET TRANSACTION`、`SAVEPOINT`、`RELEASE SAVEPOINT`、`BEGIN/START TRANSACTION` 带 TransactionMode、`COMMIT/ROLLBACK AND CHAIN`、`ROLLBACK TO SAVEPOINT`。
4. **边界语义默认包**——无活跃事务 COMMIT/ROLLBACK → SQL 错误；事务活跃中 BEGIN → SQL 错误（不支持嵌套）；CLI 调用结束时事务仍开启 → 显式 rollback + stderr 提示（exit 0）；事务内语句失败 fail-fast exit 3，错误消息在事务上下文改为"先前语句未提交、已随事务回滚"。
5. **审计修正（2026-09-10，Plan 审计 + Act Review 同发现）**——`COMMIT/ROLLBACK AND NO CHAIN` 在 sqlparser 0.44 AST 中与裸语句不可区分（`parse_commit_rollback_chain` 返回 `chain=false`），R2 收窄为仅拒 `AND CHAIN` 形态；`AND NO CHAIN` 文档化为按裸语句语义执行（新增 R2/S5 场景锁定）。用户批准。

## What Changes

- 新 capability spec `sql-transaction-statements`（5 Requirement）：
  - R1 事务语句 CLI 会话往返：BEGIN→DML→COMMIT/ROLLBACK 语义与 lib 显式事务 API 等价；成功响应为受影响行数 0；事务内 SELECT/DDL 沿既有引擎语义（文档化，不改变）
  - R2 边界子句显式拒绝（6 类，文案点名）
  - R3 会话状态边界（无事务 COMMIT/ROLLBACK、嵌套 BEGIN、收尾未提交隐式回滚、事务内失败 fail-fast 注明未提交并回滚）
  - R4 非会话路径（`execute_sql`/`execute_in_tx`/网络）显式拒绝事务语句
  - R5 既有语义零回归
- 修改 spec `cli-noninteractive-shell`「多语句分片逐条执行」：事务上下文内语句不再逐条 auto-commit；失败消息按上下文如实注明（已提交 vs 未提交已回滚）；调用收尾回滚开启中的事务
- 实现：planner 事务语句臂（识别 + 子句校验 + 精确报错，非会话路径拒绝面）、CLI 会话事务态（lib 层可复用）、事务内语句改走 `execute_stage_in_tx`
