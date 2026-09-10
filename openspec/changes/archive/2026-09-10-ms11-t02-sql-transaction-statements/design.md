# Design — MS11-T02 SQL 事务语句

> 采集 revision: c468055（master）；规划日期 2026-09-10。

## 当前行为与目标行为

- **当前**：`BEGIN`/`COMMIT`/`ROLLBACK` 及全部边界子句解析成功（sqlparser 0.44 GenericDialect），经 `build_plan` `_` 臂落 `Plan error: Unsupported statement type`（exit 3；基线探针 2026-09-10 实测 8 种形态）。SQL 面无事务能力；事务仅经 lib API（`Database::begin/commit/rollback/execute_in_tx`，MS07-T04）可用，事务句柄为调用方持有的 owned `Transaction`，`Database` 无会话态字段。
- **目标**：CLI one-shot 调用内 `BEGIN→DML→COMMIT/ROLLBACK` 生效（语义等价 lib API）；边界子句精确拒绝；非会话路径（`execute_sql`/`execute_in_tx`/网络同源）显式拒绝；既有行为零回归。

## 关键选择

### D1 会话态：CLI 循环持有、实现放 lib（`TransactionSession`）

- 拒绝「Database 字段持当前事务」：`Database` 为 `Clone` 且全部字段共享 `Arc`（`src/database.rs:16-25`），网络所有连接共享同一实例——Database 级会话态跨连接污染；且 Database 语义是引擎协调器而非会话。
- 拒绝「SqlHandler 持会话（网络同步支持）」：用户裁定仅 CLI（server 无消费者，MS09 降级先例）；扩大 BDD 与测试面。
- **选择**：`src/transaction/session.rs` 新增 `TransactionSession { tx: Option<Transaction> }`（方法 `begin/commit/rollback(&mut self, &Database) -> Result<(), String>`、`is_active()`、`tx_id()`），CLI `run_sql` 每调用创建。放 lib 层使未来网络复活可直接复用（本 change 范围外）。会话错误返回携带 spec 文案的 `String`（CLI 直接转 `ExitStatus::Sql`）；不新定义错误类型——消息文本即契约。

### D2 plan_stage 前拦截 + 共享分类器；不新增 PhysicalPlan 变体

- 分类器 `classify_transaction_statement(stmt: &Statement) -> Result<Option<TxStatementKind>, PlanError>` 放 `src/parser/planner/mod.rs`，CLI 与 `build_plan` 共用（同一套校验与文案）。`TxStatementKind = { Begin, Commit, Rollback }`。
- 返回语义：`Ok(None)` 非事务语句；`Ok(Some(kind))` 干净的 BEGIN/COMMIT/ROLLBACK；`Err` 边界子句（modes/chain/savepoint/SET TRANSACTION/SAVEPOINT/RELEASE）携点名文案。
- `build_plan` 在既有 match 前先分类：`Err(e)` → 原样传播（非会话路径也获得 R2 精确文案）；`Ok(Some(_))` → `Err(TransactionStatement("transaction statements (BEGIN/COMMIT/ROLLBACK) are only supported in an rtsql CLI session"))`（R4）；`Ok(None)` → 既有分发不变。
- **拒绝新增 PhysicalPlan 变体**：事务语句无行集、无执行器语义；新增变体强制改穷尽 match 的 `get_plan_output_columns`（`query.rs:23-82`，无兜底臂）与 `create_executor_from_plan`（`pipeline.rs:439`，无兜底臂）；且 `execute_stage` 无会话访问点，事务操作无法经 executor 路由。拦截使 plan 层、plan cache（`is_cacheable` 仅 Query，事务语句两条路径都不入缓存）、`extract_column_indices` 全部零改动。

### D3 错误消息目录（契约文案，Act 不得改写）

| 场景 | 文案 |
|---|---|
| 非会话路径遇事务语句（R4） | `transaction statements (BEGIN/COMMIT/ROLLBACK) are only supported in an rtsql CLI session` |
| `SET TRANSACTION` | `SET TRANSACTION is not supported` |
| `SAVEPOINT` / `RELEASE SAVEPOINT` | `SAVEPOINT is not supported` / `RELEASE SAVEPOINT is not supported` |
| BEGIN/START TRANSACTION 带 mode 或 modifier | `transaction modes in BEGIN/START TRANSACTION are not supported` |
| `COMMIT AND CHAIN` | `COMMIT AND CHAIN is not supported` |
| `ROLLBACK AND CHAIN` | `ROLLBACK AND CHAIN is not supported` |
| `ROLLBACK TO [SAVEPOINT] x` | `ROLLBACK TO SAVEPOINT is not supported` |
| 无活跃事务 COMMIT/ROLLBACK | `no active transaction` |
| 活跃中 BEGIN | `transaction already active` |
| 收尾回滚 stderr 提示 | `uncommitted transaction was rolled back at exit` |
| fail-fast 事务上下文后缀（替换 `; previous statement(s) were committed`） | `; previous statement(s) were not committed (rolled back with the transaction)` |

失败模板其余部分（序号、语句文本、200 字符截断、k>1 才加后缀）不变。注意 sqlparser Display 规范化：`BEGIN` 显示为 `BEGIN TRANSACTION`（探针实测），模板中的语句文本沿用 canonical 形式。

`AND NO CHAIN` 无独立文案：sqlparser 0.44 `parse_commit_rollback_chain`（parser/mod.rs:9041-9051）将 `COMMIT AND NO CHAIN`/`ROLLBACK AND NO CHAIN` 与裸语句解析为同一 AST（`chain: false`），分类器按裸 COMMIT/ROLLBACK 语义处理（2026-09-10 审计裁定，spec R2 同步文档化）。

### D4 响应形状：AffectedRows(0)

（用户裁定）CLI 会话操作成功后直接构造 `QueryPayload::Affected(0)` 渲染，不经 executor/Response——与 DDL 的 `AffectedRows(0)`（`create_table.rs:77`）输出形状一致。JSON/PG/CLI 渲染零变更。

### D5 收尾与失败回滚语义

`run_sql` 两条退出路径统一：循环正常结束或 fail-fast 返回时，`session.is_active()` → 显式 `session.rollback(db)`。正常结束（无其他错误）追加 stderr 提示、`ExitStatus::Success`（exit 0，对齐 psql 隐式回滚先例——写入不持久化是可观察后果，stderr 提示可见）。fail-fast 路径的错误消息按 D3 后缀区分上下文。rollback 本身失败（abort 错误）：stderr 记录错误文本，不掩盖原始错误状态（close() 的 WAL 兜底与恢复期未提交清理仍存在）。

### D6 事务内可见性与 DDL（文档化，不改变）

`execute_stage_in_tx` 查询节点收 `Some(tx_id)` 但扫描保持 `snapshot: None`（MS07-T04 既有语义）：无快照扫描不做 MVCC 逐行检查（`data_scan.rs:429-440`）、墓碑恒跳过（:424-426）、未提交替代者不抑制旧版本（`superseder_suppresses`，:310-326 注释明示"unchanged explicit-tx behavior"）。可观察后果即 R1 的三个文档化场景（未提交 INSERT 可见 / UPDATE 新旧并存 / DELETE 行消失；DDL 立即生效不被回滚）。本 change 不改引擎；隔离级别语义归 MS09-T01（I033 同域）。

## 变更面与责任边界

| 文件 | 变化 |
|---|---|
| `src/parser/error.rs` | +`PlanError::TransactionStatement(String)`（`#[error("{0}")]`，消息即全文） |
| `src/parser/planner/mod.rs` | +`TxStatementKind` +`classify_transaction_statement` + `build_plan` 前置分类 + 单测 |
| `src/transaction/session.rs`（新） | `TransactionSession` + 单测 |
| `src/transaction/mod.rs` | re-export |
| `src/cli/mod.rs` | `run_sql` 会话接线（分类分派、事务内 DML 走 `execute_stage_in_tx`、收尾回滚）；`sql_failure_status` 上下文变体 |
| `tests/tx_statement_test.rs`（新） | R1/R2/R3 e2e + R4 lib 拒绝（`execute_sql`/`execute_in_tx`/`SqlHandler`） |

**禁止修改**：`PhysicalPlan` 枚举及其全部 match 面、`is_cacheable`、`plan_stage`/`execute_stage`/`execute_stage_in_tx` 签名与语义、`TransactionManager`/`Database` 事务 API、`explicit_tx_test.rs`、既有 cli_test 断言、渲染层（`render.rs`）、网络协议。

## 实现顺序

T1 分类器+planner 臂（T2/T3 的文案与判定基元）→ T2 TransactionSession（T4 的状态机）→ T3 lib 拒绝集成测试（R4 关闭）→ T4 CLI 接线 → T5 e2e 测试（R1/R3 + 多语句修订关闭）→ T6 回归清扫。T1→T2 可并行开发但文案目录（D3）先行冻结。

## 风险

- 事务内 SELECT 三个文档化场景依赖无快照扫描语义——若 Act 实测与 R1 场景不符（实质风险低：代码路径已闭合），属 NEW-EVIDENCE 返回 Plan。
- `BEGIN` 经 WAL `BeginTxn` 落记录、收尾回滚经 `AbortTxn`：与 lib API 崩溃语义同源（恢复期 uncommitted 清理），无新恢复面。
