# Iteration 000 / Cycle 000: lib 事务语句层（分类器、会话基元、非会话路径拒绝）

## Plan Context

- Status: ready
- Iteration: 000-tx-statement-layer
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3
- Depends on: None
- Stable baseline: 事务语句在 `execute_sql`/`execute_in_tx`/`SqlHandler`（网络同源）三条非会话路径获得 design D3 精确拒绝文案（R2 lib 侧、R4 关闭）；`TransactionSession` 基元可用、单测绿、未被 CLI 接线；CLI 对事务语句的报错文案由 `Plan error: Unsupported statement type` 变为精确文案（`BEGIN`→session-only 文案；边界子句→点名文案），其余 CLI 行为零变化
- Verification boundary: T1/T2 单测 + T3 集成测试绿；全量回归 0 failed；clippy -D warnings 0、fmt 0、openspec validate PASS
- Diagnostic boundary: `src/parser/planner/mod.rs`、`src/parser/error.rs`、`src/transaction/session.rs`
- Deferred tasks: T4, T5, T6（Iteration 001）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R2、R4 全部场景；R5 既有语义零回归约束；design D1-D3、D6
- Excluded scope: CLI 会话接线与 e2e（T4-T6）、网络会话支持、SAVEPOINT 族实现、隔离级别语义

**Objective**

事务语句在所有非会话执行路径被精确拒绝（边界子句点名、事务语句指明仅限 CLI 会话），且不进入计划缓存；`TransactionSession` 会话基元就绪并通过单测；全量回归零失败。

**Background**

tasks MS11-T02（用户 2026-09-10 批准计划）。SQL 面当前无事务语句；lib 显式事务 API（MS07-T04）完备但事务句柄由调用方持有。本 Iteration 落地 lib 层判定与拒绝面（后续 Iteration 001 把 CLI 会话接到这些基元上）。用户裁定：仅 CLI 适用面、边界子句全部显式拒绝、非会话路径显式拒绝。

**Current Baseline**

- revision `c468055`（master），工作区含本 change 目录（无产品代码改动）。
- 基线测试：769 pass / 0 failed / 2 ignored（2026-09-10 MS11-T01 收尾）。
- 基线探针（2026-09-10，真二进制实测）：`BEGIN`/`COMMIT`/`ROLLBACK`/`SET TRANSACTION ...`/`SAVEPOINT sp1`/`BEGIN ISOLATION LEVEL SERIALIZABLE`/`COMMIT AND CHAIN`/`ROLLBACK TO SAVEPOINT sp1` 全部解析成功、统一落 `statement 1 of 1 failed: Plan error: Unsupported statement type; statement: <canonical>`（exit 3）；`BEGIN` 的 canonical 文本为 `BEGIN TRANSACTION`（sqlparser Display 规范化）。

**Current-State Evidence**

- `build_plan` 分发：`src/parser/planner/mod.rs:63-101`，`match stmt` 六类语句 + `_ => Err(PlanError::UnsupportedStatement)`。事务语句全部落 `_` 臂。
- `PlanError` 枚举：`src/parser/error.rs:7`（`UnsupportedStatement` 等；无事务语句变体）。
- `plan_stage`（`src/pipeline.rs:56-86`）：DDL 变体免注册臂；其余 `register_table`（事务语句 `extract_all_table_names`→空表名→Ok，`pipeline.rs:864-881`）→ `build_plan` → `is_cacheable`（仅 `Statement::Query`，`pipeline.rs:1012-1014`）。
- 隐式路径多语句拒绝在 parse 后、plan 前（`pipeline.rs:356-358`）；`execute_in_tx` 同（`pipeline.rs:247-249`）——单条事务语句才到达 `build_plan`。
- `Database::execute_sql` → `pipeline::execute`（`src/database.rs:108-110`）；`execute_in_tx(sql, &tx)` → `pipeline::execute_in_tx(sql, tx.id())`（`database.rs:162-164`）。
- `SqlHandler`（`src/network/handler.rs:6-28`）：`execute(Request)` → `database.execute_sql(&sql)`——网络 JSON/PG 同源。
- 事务句柄：`Transaction { id, snapshot, state }`（`src/transaction/manager.rs:19-45`）；`TransactionManager::begin() -> Transaction` 无失败路径（:80-96）；`commit(tx, &BufferPool) -> Result<()>` 双提交报 `AlreadyCommitted`（:103-129）；`abort(tx, &BufferPool, &HashMap<String, Arc<TableMeta>>) -> Result<()>`（:137-164）。`Database::begin/commit/rollback` 签名见 `database.rs:119-152`。
- `plan_cache.put` 仅在 `is_cacheable` 后（`pipeline.rs:80-82`）——分类器在 `build_plan` 前拒绝，事务语句不触达 put。
- 测试夹具：`tests/cli_test.rs:14-43`（真二进制 `CARGO_BIN_EXE_rtsql` + TempDir + `RTSQL_HOME`）；`tests/explicit_tx_test.rs:14`（`Database::open` tempdir 夹具）。

**Relevant Code**

- `src/parser/planner/mod.rs` — `PlanBuilder::build_plan` 分发；本 Iteration 加 `TxStatementKind`、`classify_transaction_statement`、前置分类。
- `src/parser/error.rs` — 加 `TransactionStatement(String)` 变体（`#[error("{0}")]`，消息即全文）。
- `src/transaction/session.rs`（新）— `TransactionSession`；`src/transaction/mod.rs` re-export。
- `tests/tx_statement_test.rs`（新）— R4 集成段（本 Iteration）+ R1/R2/R3 CLI 段（Iteration 001 扩展）。

**Critical Path**

`execute_sql("BEGIN")` → `pipeline::execute` → `parse_stage` → 单语句 → `plan_stage` → `build_plan` → **分类器 `Ok(Some(Begin))`** → `Err(TransactionStatement("...only supported in an rtsql CLI session"))` → `Response::Error`（文案经 `format!("Plan error: {}", e)`，`pipeline.rs:77-79`）。
CLI 单语句 `"SAVEPOINT sp1"` → 同链 → 分类器 `Err(TransactionStatement("SAVEPOINT is not supported"))` → exit 3。
会话路径（Iteration 001）CLI `run_sql` 循环 → `classify_transaction_statement` → `Ok(Some(kind))` → `TransactionSession` 操作；`Ok(None)` → 既有 plan/execute。

**Implementation Guidance**

- 分类器签名与语义（design D2）：`pub enum TxStatementKind { Begin, Commit, Rollback }`（planner/mod.rs 导出，供 CLI Iteration 001 复用）；`pub(crate) fn classify_transaction_statement(stmt: &Statement) -> Result<Option<TxStatementKind>, PlanError>`。匹配：`Statement::StartTransaction { modes, begin, modifier }`——`begin` 任意（`BEGIN`/`START TRANSACTION` 同义）、`modes` 空、`modifier` `None` → `Ok(Some(Begin))`；否则 `Err(TransactionStatement("transaction modes in BEGIN/START TRANSACTION are not supported"))`。`Commit { chain }`——chain=false → `Ok(Some(Commit))`；chain=true → `Err("COMMIT AND CHAIN is not supported")`。`Rollback { chain, savepoint }`——savepoint `Some` → `Err("ROLLBACK TO SAVEPOINT is not supported")`；chain=true → `Err("ROLLBACK AND CHAIN is not supported")`；chain=false/savepoint=None → `Ok(Some(Rollback))`。`SetTransaction { .. }` → `Err("SET TRANSACTION is not supported")`；`Savepoint { .. }` → `Err("SAVEPOINT is not supported")`；`ReleaseSavepoint { .. }` → `Err("RELEASE SAVEPOINT is not supported")`。其余 → `Ok(None)`。
- `build_plan` 改法：函数体首行调用分类器——`Err(e)` 返回 `Err(e)`；`Ok(Some(_))` 返回 `Err(PlanError::TransactionStatement("transaction statements (BEGIN/COMMIT/ROLLBACK) are only supported in an rtsql CLI session".into()))`；`Ok(None)` 落既有 match（不变）。
- `TransactionSession`（design D1）：字段 `tx: Option<Transaction>`；`pub fn new() -> Self`；`pub fn is_active(&self) -> bool`；`pub fn tx_id(&self) -> Option<u64>`；`pub async fn begin(&mut self, db: &Database) -> Result<(), String>`（active → `Err("transaction already active")`；否则 `db.begin().await` 存入）；`pub async fn commit(&mut self, db: &Database) -> Result<(), String>`（None → `Err("no active transaction")`；否则 `db.commit(tx).await.map_err(|e| e.to_string())`，成功后 `tx=None`）；`pub async fn rollback(&mut self, db: &Database) -> Result<(), String>`（None → `Err("no active transaction")`；否则 `db.rollback(tx).await.map_err(|e| e.to_string())`，成功后 `tx=None`）。错误为 `String`——消息文本即契约（design D3）。
- 既有 `sql_failure_status`、`run_sql`、渲染层本 Iteration 不动（文案变化仅来自 planner）。

**Behavioral Change**

- 当前：事务语句（含边界子句）在所有路径统一 `Plan error: Unsupported statement type`。
- 目标（本 Iteration）：非会话路径——`BEGIN/COMMIT/ROLLBACK` → `Plan error: transaction statements (BEGIN/COMMIT/ROLLBACK) are only supported in an rtsql CLI session`；边界子句 → `Plan error: <点名文案>`（D3 目录）。两条路径（`execute_sql`/`execute_in_tx`）与网络同源；不进 plan cache。会话语义（Iteration 001）不变。
- 接口：`PlanError` 新变体（additive）；`TxStatementKind`/分类器/`TransactionSession` 为新 pub(crate)/pub API（additive）。既有 API 零签名变化。

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R2/S1-S4, R4/S1-S2 | `src/parser/planner/mod.rs`（新 `classify_transaction_statement` + `build_plan`） | AST→Plan 分发 | 前置分类 + 精确拒绝；新 `TxStatementKind` 导出 |
| T1 | R2/S1-S4 | `src/parser/error.rs` `PlanError` | 错误分类 | +`TransactionStatement(String)` 变体 |
| T2 | R3/S1-S2（lib 基元） | `src/transaction/session.rs`（新）+ `transaction/mod.rs` | 无 | `TransactionSession` 状态机 + 单测 |
| T3 | R4/S1-S3 | `tests/tx_statement_test.rs`（新） | 无 | 三路径拒绝 + cache 断言集成测试 |

**Task Contracts**

### T1: 事务语句分类器与 planner 精确拒绝

- Requirement/Scenario: R2/S1-S4；R4/S1-S2（文案与拒绝面）
- Depends on: None
- Targets: `src/parser/planner/mod.rs`（`classify_transaction_statement`、`build_plan`）、`src/parser/error.rs`（`PlanError`）
- Current behavior: 事务语句与边界子句统一 `Plan error: Unsupported statement type`（基线探针证据）
- Required behavior: 见 design D3 文案目录——`execute_sql("BEGIN")` → `Response::Error` 含 `only supported in an rtsql CLI session`；`SET TRANSACTION`/`SAVEPOINT sp1`/`RELEASE SAVEPOINT sp1`/`BEGIN ISOLATION LEVEL SERIALIZABLE`/`COMMIT AND CHAIN`/`ROLLBACK TO SAVEPOINT sp1` → `Response::Error` 含各自点名文案
- Required changes: 新 `TxStatementKind` + 分类器（校验 modes/modifier/chain/savepoint）；`build_plan` 前置分类（Err 传播、Some→session-only 拒绝、None 走既有 match）；新 `PlanError::TransactionStatement(String)`（`#[error("{0}")]`）
- Preserve: 非事务语句分发路径逐字节等价；`plan_stage` 免注册臂与 `register_table` 流程不变；DDL/DML/Query 计划输出不变
- Forbidden: 新增 `PhysicalPlan` 变体；改 `is_cacheable`；改边界子句语义（不做部分支持）；改 D3 文案
- Test witness: `src/parser/planner/mod.rs` `#[cfg(test)]` 表驱动单测——先写测试观察 RED（新 API 未实现即编译失败属 RED 形态；或以现有探针输出为变更前基线），分类矩阵：`BEGIN`/`START TRANSACTION`/`BEGIN TRANSACTION` → `Ok(Some(Begin))`；`COMMIT`/`ROLLBACK` → 对应 kind；8 类边界子句 → `Err` 含 D3 文案；`SELECT 1`/`INSERT`/`CREATE TABLE` → `Ok(None)`
- GREEN condition: 单测全绿 + T3 集成段 R4/S1-S2 断言通过
- Verification: `cargo test --lib planner`（或 `cargo test planner`）+ 全量回归；失败含义=分发或文案回归
- Stop when: 分类器需要改 `plan_stage`/缓存语义，或 sqlparser 0.44 AST 与本契约字段不符（实质：Blocker Handoff）

### T2: TransactionSession 会话基元

- Requirement/Scenario: R3/S1-S2 的 lib 基元（Iteration 001 消费）；R1/S1-S2 的语义等价载体
- Depends on: None（与 T1 无代码依赖；文案目录 D3 先行冻结）
- Targets: `src/transaction/session.rs`（新）、`src/transaction/mod.rs`（re-export）
- Current behavior: 无会话抽象——调用方手工持有 `Transaction` 并调 `Database::begin/commit/rollback`
- Required behavior: `begin` 二次调用 → `Err("transaction already active")`；未 begin 的 `commit`/`rollback` → `Err("no active transaction")`；begin→commit 后 `is_active()==false` 且写入持久可见；begin→rollback 后写入不可见；`tx_id()` 在 active 时返回 `Some(id)`（与 `db.begin()` 分配一致）
- Required changes: 新模块 + 单测（tempdir `Database::open` 夹具，tokio::test）
- Preserve: `Database`/`TransactionManager` 签名与语义零变化；`Transaction` 语义（消耗型、状态机）不变
- Forbidden: 会话方法返回类型不用 `String` 消息以外的新错误类型；不改 D3 文案；不实现 savepoint/nested
- Test witness: `src/transaction/session.rs` `#[cfg(test)]`——先写测试观察 RED（模块不存在）；GREEN 后保留为常驻单测
- GREEN condition: 单测全绿（状态机 + 两条错误文案 + 持久化可见性）
- Verification: `cargo test session`；失败含义=会话状态机或文案回归
- Stop when: 需要 Database 新增方法或改事务 API 才能实现（实质：Blocker Handoff）

### T3: lib 非会话路径拒绝集成测试

- Requirement/Scenario: R4/S1、R4/S2、R4/S3
- Depends on: T1（分类器与文案）；T2 不依赖
- Targets: `tests/tx_statement_test.rs`（新，lib 段）
- Current behavior: 无该测试文件；`BEGIN` 经 `execute_sql` 落 `Unsupported statement type`（基线探针）
- Required behavior: ①`db.execute_sql("BEGIN")` → `Response::Error` 含 `only supported in an rtsql CLI session` 且 `plan_cache_len()` 不变；②`db.begin()` 后 `execute_in_tx("COMMIT", &tx)` → `Response::Error`（事务语句拒绝），随后同事务 `execute_in_tx("INSERT ...")` 成功且 `db.commit(tx)` 成功（拒绝不终结事务）；③`SqlHandler::new(Arc<Database>).execute(Request::Query{sql:"COMMIT"})` → `Response::Error` 同源文案
- Required changes: 新集成测试文件（复用 `explicit_tx_test` 的 tempdir 夹具模式）
- Preserve: 既有测试套件零修改
- Forbidden: 不经真 TCP 验证网络路径（handler 层即同源证明）；不断言 D3 之外的文案子串
- Test witness: 先运行确认 RED（文件不存在/断言落空——T1 未合入时 `BEGIN` 返回 Unsupported 文案），T1 合入后 GREEN
- GREEN condition: 三场景断言全部通过
- Verification: `cargo test --test tx_statement_test`；失败含义=非会话路径拒绝面回归
- Stop when: `SqlHandler` 不可从集成测试构造（可见性问题）——改用 `pipeline::execute` 直测并在测试注释记录等价性（非实质，记录进 Act Response）

**Invariants**

- `PhysicalPlan` 枚举及其全部 match 面、`is_cacheable`、`plan_stage`/`execute_stage`/`execute_stage_in_tx`、`TransactionManager`/`Database` 事务 API、渲染层、网络协议：零修改。
- 事务语句永不进入 plan cache（两条路径均不触达 `put`）。
- design D3 文案为契约常量；spec 断言引用其子串。
- 既有测试套件零修改通过（R5）。

**Non-goals**

- CLI 会话接线与端到端事务（T4-T6，Iteration 001）
- 网络路径会话支持、PG 事务状态字节
- SAVEPOINT 族、SET TRANSACTION、隔离级别（全部仅拒绝）
- 引擎事务/可见性语义任何改变（design D6 文档化面）

**Acceptance**

- R2/S1-S4：分类器单测矩阵 + Iteration 001 e2e 复验（本 Iteration 先以单测+T3 锁 lib 面）
- R4/S1-S3：T3 集成测试三场景
- R5/S1-S3：全量回归（769+新增，0 failed）、clippy/fmt 0、validate PASS
- Stable baseline 达成：非会话路径精确拒绝 + 会话基元就绪且未接线

**Verification**

- `cargo test`（全量，决定性输出：test result 行）
- `cargo clippy -D warnings`（0 warning）、`cargo fmt --check`（0 diff）、`openspec validate 2026-09-10-ms11-t02-sql-transaction-statements`（PASS）
- 探针复核（可选）：真二进制 `rtsql <tmpdb> "SAVEPOINT sp1"` stderr 含 `SAVEPOINT is not supported`、exit 3

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | Current-State Evidence 全部给出文件/符号/行号；基线探针实测（2026-09-10）；分类器两侧调用点、缓存写点、多语句拒绝点已定位 |
| Design | PASS | design.md D1-D6 闭合（含备选与拒绝理由）；文案目录冻结；无 TBD |
| Iteration Plan | PASS | tasks.md 两 Iteration + 平衡审计；T1-T6 全分配、依赖有序 |
| Cycle Scope | PASS | initial；Acceptance gaps None；R2/R4 + 基元纳入，CLI 行为面排除 |
| Task Contracts | PASS | T1-T3 契约含目标符号、当前/目标行为、RED/GREEN、验证、停止条件；Act 只读本 Cycle 即可实施 |
| Traceability | PASS | RTM（change 汇报层）R2/R4→D1-D3→T1-T3→测试面闭环；R1/R3/R5 由 Iteration 001 承接 |
| Verification | PASS | 单测/集成/全量/静态四层命令与通过条件明确；无身份型证据工程 |

**Persisted Evidence**

- Mode: none

Act Response（命令、≤20 行决定性输出、退出码）足以承载验证结果；无不可复现结果。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- R4/S3 网络同源拒绝以 `SqlHandler` 层测试证明（T3 契约）——JSON TCP 夹具已整体注释退役（`tests/network_server_test.rs:1-4`，M8 切 PG），活跃网络夹具为 `pg_integration_test.rs`；不为单条错误断言复活 JSON TCP 夹具或搭 PG 服务（非实质测试策略选择，已在契约声明）。
- 事务内 SELECT 文档化场景（R1）属 Iteration 001 验收；若实测与 spec 场景不符，届时按 NEW-EVIDENCE 返回 Plan（本 Iteration 不涉及）。
- sqlparser 0.44 `StartTransaction` 的 `modifier` 字段（SQLite-only）与 `TransactionMode` 枚举成员未逐一枚举——分类器按"非空即拒绝"处理，无需逐一识别（非实质）。
- `PlanError::TransactionStatement` 的 Display 若与既有 `format!("Plan error: {}", e)` 拼接产生双前缀观感，文案以 D3 全文为准（`#[error("{0}")]`），不做前缀装饰（非实质）。

## Act Response

- Status: reported

**Implemented**

- **T1 分类器与 planner 精确拒绝（R2 lib 侧 + R4 拒绝面）**：`src/parser/planner/mod.rs` 新增 `pub enum TxStatementKind { Begin, Commit, Rollback }` 与 `pub(crate) fn classify_transaction_statement(&Statement) -> Result<Option<TxStatementKind>, PlanError>`（D2 语义：`StartTransaction` modes 空且 modifier None → Begin；`Commit { chain }` chain=true → Err；`Rollback { chain, savepoint }` savepoint 优先、其次 chain、否则干净 Rollback；`SetTransaction`/`Savepoint`/`ReleaseSavepoint` → 点名 Err；其余 → Ok(None)）。`build_plan` 函数体首行前置分类：`Err` 经 `?` 传播（边界子句 D3 文案直达）、`Some` → session-only 拒绝、`None` 落既有 match（逐字节不变）。`src/parser/error.rs` 新增 `PlanError::TransactionStatement(String)` + Display 臂（消息即全文）。事务语句在 plan 构建前被拒，`is_cacheable`（仅 Query）天然保证不进 plan cache。
- **T2 TransactionSession 会话基元（R3 lib 基元）**：`src/transaction/session.rs` 新模块——`tx: Option<Transaction>` 状态机，`new/is_active/tx_id/tx/begin/commit/rollback`；begin 双开 → `Err("transaction already active")` 且原事务保持；无事务 commit/rollback → `Err("no active transaction")`；commit/rollback 先 `take()` 消费 owned 句柄再调 `Database` API；`Default` 实现规避 clippy `new_without_default`。`src/transaction/mod.rs` +`mod session;` +`pub use session::TransactionSession;`。未被任何非测试路径消费（Iteration 001 接线）。
- **T3 lib 非会话路径拒绝集成测试（R4/S1-S3）**：`tests/tx_statement_test.rs` 三场景——①`execute_sql("BEGIN")` → Error 含 `only supported in an rtsql CLI session` 且 `plan_cache_len()` 不变（先 SELECT 预热缓存使断言有意义）；②`begin()` 后 `execute_in_tx("COMMIT")` → Error、同事务 INSERT 仍成功、显式 commit 持久可见（拒绝不终结事务）；③`SqlHandler::new(Arc<Database>).execute(Request::Query{sql:"COMMIT"})` 与 `execute_sql("COMMIT")` 错误消息逐字节相等（网络同源）。

**Changed Files and Symbols**

| 文件 | 变化 |
|---|---|
| `src/parser/planner/mod.rs` | +`TxStatementKind`、+`classify_transaction_statement`、`build_plan` 前置分类（3 行守卫）、+`mod tx_statement_tests`（4 单测：分类矩阵 5 形态 / 8 类边界子句点名文案 / 非事务语句 Ok(None) 6 形态 / build_plan 三语句 session-only 消息） |
| `src/parser/error.rs` | +`PlanError::TransactionStatement(String)` + Display 臂（`write!(f, "{}", msg)`，消息即全文） |
| `src/transaction/session.rs`（新） | `TransactionSession`（7 方法 + Default）+ `mod tests`（5 单测：commit 往返持久可见 / rollback 丢弃 / 双 begin 报错且原事务保持 / 无事务 commit+rollback 报错 / tx_id 与 db.begin() 分配器一致） |
| `src/transaction/mod.rs` | +`mod session;` +`pub use session::TransactionSession;` |
| `tests/tx_statement_test.rs`（新） | R4/S1-S3 三集成测试（lib 段；CLI 段 Iteration 001 扩展） |

**Deviations from Plan**

1. `PlanError::TransactionStatement` 的 Display 用手写 `write!(f, "{}", msg)` 而非契约所写 thiserror `#[error("{0}")]` 形态——`PlanError` 实际是手写 `fmt::Display`（非 thiserror），语义等价（消息即全文）。非实质。
2. `TransactionSession` 增加 `pub fn tx(&self) -> Option<&Transaction>`（契约方法集未列出）——契约自身要求的单测行为（事务内写入持久可见/不可见）需要把会话事务传给 `execute_in_tx`，且 Iteration 001 T4 的 `execute_stage_in_tx` 接线同样依赖它；additive、不触碰 Forbidden（非新错误类型）。非实质。
3. 单测矩阵不含 `COMMIT AND NO CHAIN` / `ROLLBACK AND NO CHAIN` 独立拒绝行——真二进制探针实测 sqlparser 0.44 将两者 canonical 化为 `COMMIT`/`ROLLBACK`（AST `chain: bool` 无法区分显式 NO CHAIN 与缺省），按契约分类语义 `chain=false → Ok(Some(kind))` 处理，矩阵保留恰好 8 类可区分边界子句（与契约"8 类"一致）。契约 D3 目录中两条 NO CHAIN 文案在 AST 层不可达。影响见 Remaining Issues。
4. 契约验证命令写作 `cargo clippy -D warnings`，实际可执行形式为 `cargo clippy -- -D warnings`（`-D` 须置于 `--` 后）。等价调整，非实质。

**Blocker Handoff**

None

**Blocker Resolution**

None（未发生阻塞）

**Self-Review**

- Spec review（逐任务）：T1——R2 八类边界子句点名文案逐条断言（`assert_eq` 全文匹配）、R4 session-only 消息全文断言、非事务语句 `Ok(None)` 矩阵覆盖 Query/INSERT/UPDATE/DELETE/CREATE/DROP；Preserve 面：既有 `build_plan` match 与全部既有 planner 单测零修改通过（17 passed），`plan_stage`/`is_cacheable`/`PhysicalPlan` 零触碰（diff 证实）。T2——R3 双 begin 报错+原事务保持、无事务 commit/rollback 报错、commit 持久可见、rollback 不可见、tx_id 同分配器；`Database`/`TransactionManager`/`Transaction` 零修改。T3——R4 三场景断言与契约 ①②③ 一一对应，`explicit_tx_test.rs` 等既有测试零修改。
- 完整 diff review：变更面 = Change Surface 全集，无计划外文件/符号；无新增警告（clippy 0）、无死代码（`TxStatementKind`/`tx()` 均为 Iteration 001 契约消费面，pub API additive）、无重复实现；测试不因错误原因通过（全部断言 D3 消息文本或行集内容，非捕获工具自证）。
- 已修复发现：单测矩阵初版含 2 条 NO CHAIN 拒绝行，与契约分类语义矛盾——经真二进制探针证实 AST 不可区分后修正为 8 类可区分行（Deviation 3）。
- 遗留 Minor 问题：无。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T3 RED 见证 | `cargo test --test tx_statement_test`（T1 未合入时） | `test result: FAILED. 0 passed; 3 failed`，三测试均报 `expected message containing "only supported in an rtsql CLI session", got "Plan error: Unsupported statement type"` | R4 三场景变更前基线 | RED ✅ |
| T1 RED 见证 | `cargo test --lib planner`（API 未实现时） | `error[E0412]: cannot find type TxStatementKind`、`error[E0425]: cannot find function classify_transaction_statement`、`error[E0599]: no variant TransactionStatement`（exit 101） | 分类器/错误变体缺失 | RED（编译形态）✅ |
| T2 RED 见证 | `cargo test --lib session`（模块仅含测试） | `error[E0432]: unresolved import session::TransactionSession` + `E0433` ×5（exit 101） | 会话基元缺失 | RED（编译形态）✅ |
| T1 GREEN | `cargo test --lib planner` | `test result: ok. 17 passed; 0 failed`（含 4 新单测） | 分类矩阵 + build_plan 拒绝 + 既有 planner 回归 | PASS |
| T2 GREEN | `cargo test --lib session` | `test result: ok. 6 passed; 0 failed`（5 session 新单测 + 1 过滤断言重叠的 planner 单测） | 会话状态机 + D3 文案 + 持久化可见性 | PASS |
| T3 GREEN | `cargo test --test tx_statement_test` | `test result: ok. 3 passed; 0 failed` | R4/S1-S3 非会话拒绝 + 缓存不变 + 同源性 | PASS |
| 全量回归（R5） | `cargo test` | `passed=781 failed=0 ignored=2`（基线 769 + 新增 12；总数只增不减；exit 0） | 全部既有行为零回归 | PASS |
| 静态分析 | `cargo clippy -- -D warnings` | `Finished dev profile ... in 4.33s`（0 warning；exit 0） | 全 crate（lib+bins+tests） | PASS |
| 格式化 | `cargo fmt --check` | 0 diff（exit 0） | 全 crate | PASS |
| OpenSpec | `openspec validate 2026-09-10-ms11-t02-sql-transaction-statements` | `Change '...' is valid`（exit 0） | change 产物一致性 | PASS |
| 真二进制探针 | `rtsql /tmp/tx_probe.db "<sql>"` ×5 | `SAVEPOINT sp1`→`Plan error: SAVEPOINT is not supported`、`COMMIT`/`BEGIN`→session-only 文案（BEGIN canonical `BEGIN TRANSACTION`）、`SET TRANSACTION ...`→`SET TRANSACTION is not supported`、`ROLLBACK TO SAVEPOINT sp1`→点名文案；全部 exit 3 | CLI 非会话路径精确拒绝文案（Iteration stable baseline：文案由 Unsupported 变精确） | PASS |

**Persisted Evidence**

None required（Mode: none；全部验证可低成本复现，Act Response 摘录已足够）

**Experience Candidates**

None

**Remaining Issues**

1. **spec R2/S4 的 `ROLLBACK AND NO CHAIN` 场景在 AST 层不可满足（NEW-EVIDENCE，归 Plan Review 裁定）**：sqlparser 0.44 将 `COMMIT AND NO CHAIN`/`ROLLBACK AND NO CHAIN` canonical 化为 `COMMIT`/`ROLLBACK`（`chain: bool` 不区分显式 NO CHAIN），契约分类语义下它们等同干净语句——CLI 会话中将作为正常 ROLLBACK 执行（exit 0），而 spec R2/S4 期望 exit 3 点名文案。属 Iteration 001 e2e 验收面；请 Plan 裁定：修订 spec R2/S4 场景措辞（移除 NO CHAIN 形态或改述为不可区分语义），或接受"NO CHAIN 即 clean 语句"语义并在 design D3 目录标注两条 NO CHAIN 文案不可达。
2. 其余无。

**Commit or Diff Reference**

未 commit（待用户触发）；工作区 diff 基于 `c468055`（master）：`src/parser/error.rs` +3、`src/parser/planner/mod.rs` +167（含测试）、`src/transaction/mod.rs` +2、新增 `src/transaction/session.rs` 与 `tests/tx_statement_test.rs`。

## Plan Review

- Review Result: accepted

**Findings**

1. **F1（NEW-EVIDENCE 由 Act 上报 + PLAN-INVALID 在 Plan spec 文本，已闭合）**：sqlparser 0.44 `parse_commit_rollback_chain`（parser/mod.rs:9041-9051）将 `COMMIT/ROLLBACK AND NO CHAIN` 与裸语句解析为同一 AST（`chain: false`）——Plan 原 spec R2 的 `AND [NO] CHAIN` 拒绝面与 R2/S4 场景不可实现。Act Remaining Issue #1 与 Plan 审计独立发现同一问题；实现本身正确（`classify_transaction_statement` chain=false → clean，`mod.rs:59-83` 经读取核实，无 Act 偏离）。处置（用户批准 2026-09-10）：spec R2 收窄为仅拒 `AND CHAIN`、`AND NO CHAIN` 文档化为裸语句同义并新增 R2/S5 场景；design D3 删除两条 NO CHAIN 文案并补记；tasks T5 场景清单同步 S1-S5；proposal 增决策 5。R2/S5 e2e 归 T5（Iteration 001）；Iteration 000 无返工项。
2. **F2（PLAN-INVALID，Minor，已闭合）**：R3/S4 GIVEN 病句（"无唯一约束外的其他表"）修正为"表 `t(id INT)` 为空"（用户批准）。
3. **非阻塞**：①session `tx()` 访问器为契约外 additive——非实质局部差异，Iteration 001 T4 消费；②冻结 Plan Context 中 T1 契约"8 类边界子句"计数与修正后 D3 目录（6 类拒绝 + `AND NO CHAIN` 同义）表述有出入，以 design D3 修正版为准；③R4/S3 以 `SqlHandler` 层测试证明网络同源——契约已声明（JSON TCP 夹具退役，`network_server_test.rs:1-4`），维持。

**Deviation Classification**

NEW-EVIDENCE（Act 上报 AND NO CHAIN AST 限制）+ PLAN-INVALID（Plan 的 spec R2 原文与 R3/S4 GIVEN 缺陷；已按用户批准修正，无 Act 偏离）

**Acceptance Gaps**

None——Iteration 000 既有 Acceptance 全部满足：R2 lib 侧（分类器单测矩阵 17 passed）、R4/S1-S3（tx_statement_test 3 passed + cache 不变断言）、稳定基线（CLI 文案由 Unsupported 变精确、其余零变化）、全量 781/0/2、clippy/fmt 0、validate PASS。

**Convergence**

N/A（首次 Review，无上一版 gap 可比）

**Evidence**

Plan Review 独立核实（2026-09-10，不采信 Act Self-Review 作为替代）：①代码读取 `planner/mod.rs:30/44-90/137-141`（分类器+前置分类）、`transaction/session.rs`（D1 全签名+D3 文案）、`error.rs:57/109`（变体+Display）；②独立复跑 `cargo test --test tx_statement_test` 3 passed、`--lib planner` 17 passed、`--lib session` 6 passed；③全量 `cargo test` 汇总 781 passed / 0 failed / 2 ignored；④`cargo clippy -- -D warnings` 0 warning、`cargo fmt --check` 0 diff；⑤真二进制探针三连：`BEGIN`→session-only 文案（canonical `BEGIN TRANSACTION`）、`COMMIT AND CHAIN`→点名文案、`COMMIT AND NO CHAIN`→canonical `COMMIT`→session-only 文案（R2/S5 AST 等价在真二进制确认），全部 exit 3——与修正后 spec/design D3 逐条一致。

**Follow-up Decision**

accepted——实现满足 Iteration 000 既有 Acceptance；两处 PLAN-INVALID spec 文本缺陷已按用户批准修正并对齐实现（修正属 change 级文档，不动已交接 Plan Context）；R2/S5 场景映射 T5（Iteration 001）承接，无需本 Iteration 返工或后继 Cycle。

**Iteration Plan Update**

None（Map 不变；T5 场景清单 S1-S4→S1-S5 为 planning 文档对齐修正，非 Iteration Map 变化）

**Next Cycle**

None

**Next Iteration**

`iterations/001-cli-session/000-initial.md`（已展开；Status: draft，Gate 2 检查项已填，待用户批准后置 ready 交 Act）
