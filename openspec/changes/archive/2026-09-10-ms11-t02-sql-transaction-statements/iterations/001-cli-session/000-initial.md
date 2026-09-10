# Iteration 001 / Cycle 000: CLI 会话接线与端到端验收

## Plan Context

- Status: ready
- Ready authorization: 用户 2026-09-10 "continue" 指令批准执行本 Cycle（Gate 1 已批、Gate 2 检查已填且各维 PASS）
- Iteration: 001-cli-session
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T4, T5, T6
- Depends on: Iteration 000（分类器、`TransactionSession`、lib 拒绝面已落地并 accepted）
- Stable baseline: MS11-T02 全部 Acceptance 关闭——R1/R3 端到端生效、R2 e2e 锁定、多语句修订场景 S7-S8 落地、R5 全量基线绿
- Verification boundary: T5 e2e 全绿（RED 见证齐全）+ T6 全量基线（781+新增，0 failed；clippy/fmt 0；validate PASS）
- Diagnostic boundary: `src/cli/mod.rs::run_sql`/`sql_failure_status` + `tests/tx_statement_test.rs` CLI 段
- Deferred tasks: None（change 最后一个 Iteration）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R1 S1-S5、R2 S1-S5（e2e 面）、R3 S1-S4、MODIFIED 多语句 S7-S8、R5；design D1-D6；Iteration 000 已实现 API（`TxStatementKind`、`classify_transaction_statement`、`TransactionSession`）
- Excluded scope: 网络路径会话支持、PG 事务状态字节、SAVEPOINT 族实现、隔离级别语义、lib 路径行为任何变化

**Objective**

CLI one-shot 调用内 `BEGIN→语句→COMMIT/ROLLBACK` 按会话语义生效（与 lib 显式事务 API 等价），边界语义（无事务 COMMIT、嵌套 BEGIN、收尾回滚、事务内失败）按 D3/D5 落地，e2e 测试锁定全部 spec 场景，全量回归零失败。

**Background**

tasks MS11-T02；Iteration 000（000-tx-statement-layer/000-initial）已 accepted：分类器与 lib 拒绝面就绪（R2 lib 侧、R4 关闭），`TransactionSession` 基元就绪但未被 CLI 接线。本 Iteration 完成 CLI 会话接线并端到端验收。用户决策（proposal 决策 1-5）：仅 CLI 适用面、`AffectedRows(0)`、边界子句拒绝（`AND NO CHAIN` 按裸语句同义，R2/S5）、边界语义默认包。

**Current Baseline**

- 工作区基于 `c468055`（master）+ Iteration 000 未 commit diff（`src/parser/error.rs` +3、`src/parser/planner/mod.rs` +167、`src/transaction/mod.rs` +2、新增 `src/transaction/session.rs` 与 `tests/tx_statement_test.rs`）。
- 测试基线：**781 pass / 0 failed / 2 ignored**（2026-09-10 Plan Review 独立复跑确认，与 Act Response 一致）。
- CLI 当前可观察行为：`BEGIN; INSERT...; COMMIT` 在语句 1 处 fail-fast（`statement 1 of n failed: Plan error: transaction statements (BEGIN/COMMIT/ROLLBACK) are only supported in an rtsql CLI session`，exit 3）；`COMMIT AND CHAIN` → `Plan error: COMMIT AND CHAIN is not supported`；`COMMIT AND NO CHAIN` → canonical `COMMIT` → session-only 文案（Plan Review 探针 2026-09-10 实测三连）。
- 非事务语句的 CLI 多语句逐条 auto-commit 语义不变（既有 cli_test 断言零修改通过，781 全量含之）。

**Current-State Evidence**

- `run_sql`（`src/cli/mod.rs:270-311`）：`parse_stage` 全串解析 → 逐条 `plan_stage(db, &statement_text, stmt, false)`（缓存键 = `stmt.to_string()` canonical 文本，:281）→ `PlanBuilder::new().get_plan_output_columns(&plan)`（:286）→ `execute_stage(db, plan, false)`（:288）→ Response match 渲染（`QueryResult`→Rows / `AffectedRows`→Affected / `Error`→fail-fast / `Pong`→跳过，:289-307）。事务语句当前走 `Ok(None)` 分类路径之外的 plan_stage → build_plan → session-only 错误。
- `sql_failure_status(k, n, error, statement_text)`（`cli/mod.rs:316-326`）：`statement {k} of {n} failed: {error}; statement: {≤200 字符}`；`k>1` 追加 `; previous statement(s) were committed`；parse 错误不套此模板。
- `execute_stage_in_tx(database, plan, tx_id)`（`src/pipeline.rs:274-299`）：DML 消费调用方 tx_id、无隐式包裹；DDL 立即执行并清缓存；查询节点收 `Some(tx_id)` 但扫描 `snapshot: None`（`data_scan.rs:429-440` 无快照不做 MVCC 检查；:424-426 墓碑恒跳过；`superseder_suppresses` :310-326 未提交不抑制旧版本）——R1/S3-S5 文档化语义的代码依据。
- `TransactionSession`（`src/transaction/session.rs`）：`new()` / `is_active()` / `tx_id() -> Option<u64>` / `tx() -> Option<&Transaction>` / `async begin(&mut self, &Database) -> Result<(), String>`（active → `"transaction already active"`）/ `async commit`（idle → `"no active transaction"`）/ `async rollback`（同上）。`classify_transaction_statement(&Statement) -> Result<Option<TxStatementKind>, PlanError>`（`src/parser/planner/mod.rs:44`，`TxStatementKind::{Begin,Commit,Rollback}` :30）；`build_plan` 前置分类（:137-141）。
- 渲染：`render(kind, &[], &QueryPayload::Affected(0))` 与 DDL 的 `AffectedRows(0)` 输出同构（`src/cli/render.rs:26-34`，JSON `{"affected_rows":0}`）；`emit_stderr`（`cli/mod.rs:354-357`）；`kind(format)`（:329-343）。
- 退出路径：`run_sql` 正常结束返回 `ExitStatus::Success`；任一语句失败经 `sql_failure_status` 提前返回（fail-fast）；`execute_command_inner` 在 work 结束后 `db.close()`（checkpoint，:255-267）——收尾回滚必须发生在 `run_sql` 内部（close 之前）。
- 测试夹具：`tests/cli_test.rs:14-43`（真二进制 `CARGO_BIN_EXE_rtsql`、TempDir、`RTSQL_HOME`、管道非 TTY 默认 JSON、60s 挂起 kill）；`tests/tx_statement_test.rs:16-22` `open_db()`（lib 段，Arc<Database>）。
- Plan Review 独立验证（2026-09-10）：`tx_statement_test` 3 passed；planner 单测 17 / session 单测 6；全量 781/0/2；clippy/fmt 0；探针三连（BEGIN / COMMIT AND CHAIN / COMMIT AND NO CHAIN）文案与 exit 逐条吻合。

**Relevant Code**

- `src/cli/mod.rs` — `run_sql`（接线主体）、`sql_failure_status`（上下文后缀）、`emit_stderr`（收尾提示）。
- `tests/tx_statement_test.rs` — 追加 CLI e2e 段（R1/R2/R3 + 多语句修订）。
- 不修改：`pipeline.rs`、`planner/*`、`transaction/session.rs`（Iteration 000 产物）、`render.rs`、网络层。

**Critical Path**

`run_sql` 每语句：`classify_transaction_statement(stmt)`
- `Err(e)` → 若 session active：先 `session.rollback(db)`，再 `sql_failure_status`（active 时后缀换未提交文案）→ exit 3
- `Ok(Some(Begin))` → `session.begin(db)`：Ok → 渲染 Affected(0)；Err(msg) → 若 session active 先 rollback，`sql_failure_status`（含 msg）→ exit 3
- `Ok(Some(Commit))` / `Ok(Some(Rollback))` → `session.commit/rollback(db)`：Ok → 渲染 Affected(0)；Err(msg) → fail-fast（idle 时无 rollback）
- `Ok(None)` → 既有 plan_stage → columns → `if session.is_active() { execute_stage_in_tx(db, plan, session.tx_id().expect(...)) } else { execute_stage(db, plan, false) }` → 既有 Response 渲染

循环正常结束：`if session.is_active() { session.rollback(db)（失败仅 stderr 记录）; emit_stderr("uncommitted transaction was rolled back at exit") }` → `ExitStatus::Success`。事务语句成功响应不经 executor/Response——直接构造 `QueryPayload::Affected(0)` 渲染（D4）。

**Implementation Guidance**

- `sql_failure_status` 上下文变体（D3）：事务上下文（session active 且 k>1）后缀用 `; previous statement(s) were not committed (rolled back with the transaction)` 替换 `; previous statement(s) were committed`；模板其余（序号/语句文本/截断/k>1 才加后缀）不变。实现形态（加参数 vs 调用点分支）非实质。
- 会话实例放 `run_sql` 函数体顶部，每次调用新建；不放 `Database`、不放全局（D1）。
- 缓存键行为保持：非事务语句仍用 `stmt.to_string()`；事务语句不触达 plan_stage。
- R2 拒绝在 CLI 的到达路径：分类器 `Err(e)` 分支（会话态不变，拒绝发生在任何语句执行前——与 spec R2 一致；注意 `Err` 分支的 rollback-if-active 只在 session active 时发生且随后 fail-fast，满足"会话态不变"的语义是"被拒语句不改变会话态"，回滚是收尾规则的结果，不是被拒语句的效果）。
- 建议顺序：T4 先行（行为面）→ T5 逐场景落测试（先 RED 后 GREEN 只对行为变化场景适用，见 T5 契约）→ T6 收口。

**Behavioral Change**

- 当前：CLI 中事务语句一律 fail-fast 报 session-only 错误（exit 3）；多语句逐条 auto-commit；无会话概念。
- 目标：`BEGIN`/`COMMIT`/`ROLLBACK` 驱动会话事务态（成功输出 `{"affected_rows":0}` 同构形状）；事务内语句经 `execute_stage_in_tx`（不再逐条 auto-commit）；边界子句错误文案不变（R2，Iteration 000 已落地）；无活跃事务 COMMIT/ROLLBACK → `no active transaction`；嵌套 BEGIN → `transaction already active`（原事务保持）；收尾未提交 → 显式回滚 + stderr 提示 + exit 0；事务内失败 → fail-fast + 未提交后缀 + 回滚。
- 接口：`run_sql`/`sql_failure_status` 为 crate 私有，无公共 API 变化；lib 行为零变化。

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T4 | R1/S1-S5, R3/S1-S4, MODIFIED S7-S8 | `src/cli/mod.rs::run_sql` | 逐条 auto-commit 执行循环 | 会话分类分派 + `execute_stage_in_tx` 接线 + 收尾回滚 |
| T4 | R3/S4, MODIFIED S7 | `src/cli/mod.rs::sql_failure_status` | fail-fast 定位模板 | 事务上下文后缀变体（D3） |
| T5 | R1/R2/R3/S*、MODIFIED S7-S8 | `tests/tx_statement_test.rs`（CLI 段追加） | 仅 lib 段（R4） | e2e 场景锁定（真二进制） |
| T6 | R5/S1-S3 | 全量套件 | 769→781 基线 | 只增不减收口 |

**Task Contracts**

### T4: CLI 会话接线（事务语句生效 + 边界语义 + 收尾回滚）

- Requirement/Scenario: R1/S1-S5；R3/S1-S4；MODIFIED 多语句 S7-S8（行为面）
- Depends on: Iteration 000 API（已就绪）
- Targets: `src/cli/mod.rs::run_sql`、`src/cli/mod.rs::sql_failure_status`
- Current behavior: 事务语句经 plan_stage 落 build_plan session-only 错误 fail-fast；多语句逐条 auto-commit；无收尾回滚
- Required behavior: 按 Critical Path 分派——事务语句驱动 `TransactionSession` 并渲染 `Affected(0)`；事务内非事务语句走 `execute_stage_in_tx(db, plan, tx_id)`；无活跃事务 COMMIT/ROLLBACK → `no active transaction`；嵌套 BEGIN → `transaction already active`（原事务保持开启）；任意错误返回路径 session active 时先显式 rollback；循环正常结束 session active 时 rollback + stderr `uncommitted transaction was rolled back at exit` + exit 0；事务上下文 fail-fast 后缀 `; previous statement(s) were not committed (rolled back with the transaction)`
- Required changes: `run_sql` 会话分派；`sql_failure_status` 上下文后缀；渲染 Affected(0) 直连（不经 executor）
- Preserve: 非事务语句执行路径与缓存键逐字节等价；`execute_stage`/`plan_stage` 调用形态不变；既有 cli_test 断言零修改；parse 错误零执行路径不变；退出码分类（0/1/2/3/4/5）不变
- Forbidden: 改 lib 层（pipeline/planner/transaction）签名或行为；改渲染层；事务语句进入 plan cache；实现 SAVEPOINT/嵌套/mode 支持
- Test witness: T5 e2e（本 Iteration 内 T4 先落、T5 随后 RED→GREEN 见证；T4 完成时以手工二进制探针记录行为：`BEGIN; INSERT; COMMIT` 三段输出 + 重开可见，写入 Act Response）
- GREEN condition: T5 全部场景绿
- Verification: 手工探针（命令+输出+exit 写入 Act Response）+ T5 套件
- Stop when: 会话语义需要改 lib 签名、或 `execute_stage_in_tx` 对某语句形态行为与 spec 场景冲突（实质：Blocker Handoff）

### T5: CLI e2e 测试（真二进制）

- Requirement/Scenario: R1/S1-S5；R2/S1-S5；R3/S1-S4；MODIFIED S7-S8（锁定面）
- Depends on: T4
- Targets: `tests/tx_statement_test.rs` CLI 段（复用 cli_test 夹具模式：TempDir + `RTSQL_HOME` + 管道）
- Current behavior: 文件仅有 lib 段 3 测试；CLI 行为未锁定
- Required behavior: 逐场景命名测试（建议映射，命名可调整但须保留 requirement-scenario 对应注释）：R1 `tx_commit_roundtrip_visible_after_reopen` / `tx_rollback_leaves_no_residue` / `in_tx_select_sees_own_uncommitted_insert` / `in_tx_select_after_update_yields_both_versions` / `in_tx_ddl_survives_rollback`；R2 `reject_set_transaction` / `reject_savepoint_and_release` / `reject_begin_with_modes` / `reject_chain_forms` / `and_no_chain_behaves_as_bare`；R3 `commit_without_tx_errors` / `nested_begin_errors_and_rolls_back_at_exit` / `open_tx_implicitly_rolled_back_with_notice` / `in_tx_fail_fast_notes_uncommitted`；MODIFIED S7（与 R3/S4 同观测定名 `multi_statement_in_tx_fail_fast`）S8（与 R3/S3 同观测定名 `multi_statement_open_tx_rolled_back_at_exit`）
- Required changes: 仅新增测试；每测试含 stdout/stderr/exit code 断言（文案断言用 D3 子串）
- Preserve: 不修改既有测试与 lib 段测试
- Forbidden: 断言 D3 之外的文案细节；为凑 RED 修改既有行为断言
- Test witness: **RED 仅适用于行为变化场景**——R1/R3/S7/S8 测试在 T4 之前运行必须失败（当前事务语句 fail-fast、无会话语义）；**R2 测试在 T4 之前即 GREEN**（Iteration 000 已落地的既有行为锁定），契约以"T4 后全绿 + R1/R3 组在 T4 前至少一条 RED 记录"为见证纪律，不伪造 RED
- GREEN condition: 全部新测试通过且全量无回归
- Verification: `cargo test --test tx_statement_test`
- Stop when: 场景实测与 spec 断言矛盾（如 R1/S4 双版本行为不符——NEW-EVIDENCE 返回 Plan）

### T6: 全量回归清扫

- Requirement/Scenario: R5/S1-S3
- Depends on: T4, T5
- Targets: 全仓
- Current behavior: 781 pass / 0 failed / 2 ignored
- Required behavior: 测试总数只增不减（≥781+新增），0 failed；clippy -D warnings 0；fmt 0 diff；`openspec validate 2026-09-10-ms11-t02-sql-transaction-statements` PASS
- Required changes: 无（清扫性任务；发现回归即修复并记录）
- Preserve: 既有测试零修改（R5/S1-S2 断言）
- Forbidden: 用跳过/ignore 换绿；修改既有断言
- Test witness: 全量命令输出（Act Response 摘录）
- GREEN condition: 四项命令全 PASS
- Verification: `cargo test` + `cargo clippy -- -D warnings` + `cargo fmt --check` + `openspec validate ...`
- Stop when: 既有测试因本 change 行为需要修改（R5/S1-S2 违约——实质：Blocker Handoff）

**Invariants**

- design D3 文案目录为契约常量；`AND NO CHAIN` 按裸语句语义（无独立文案，R2/S5）。
- 事务语句永不进入 plan cache；lib 路径行为与 Iteration 000 逐字节一致。
- 退出码分类不变；收尾回滚发生于 `db.close()` 之前（`run_sql` 内部）。
- 既有测试套件零修改（R5）。

**Non-goals**

- 网络路径会话支持、PG 事务状态字节、REPL、多连接会话
- SAVEPOINT 族、SET TRANSACTION、隔离级别（仅拒绝面）
- 引擎事务/可见性语义改变（D6 文档化）

**Acceptance**

- R1/S1-S5：T5 五测试（提交可见、回滚无残留、事务内 SELECT 三文档化场景）
- R2/S1-S5：T5 五测试（含 AND NO CHAIN 等价）
- R3/S1-S4：T5 四测试（嵌套 BEGIN 场景同时锁定收尾回滚）
- MODIFIED S7-S8：T5 两测试
- R5/S1-S3：T6 四命令
- 全链路映射见 RTM（Plan Review 汇报层）

**Verification**

- `cargo test --test tx_statement_test`（全部场景）
- `cargo test`（全量 ≥781+新增，0 failed）
- `cargo clippy -- -D warnings`（0）/ `cargo fmt --check`（0）/ `openspec validate 2026-09-10-ms11-t02-sql-transaction-statements`（PASS）
- 直接观察目标状态：stdout 顺序输出、stderr 文案、exit code、重开持久化——不使用身份型证据

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | Current-State Evidence 全带行号；Iteration 000 产物签名经 Plan Review 独立读取核实；`execute_stage_in_tx`/`run_sql`/`sql_failure_status`/渲染/退出路径定位完整 |
| Design | PASS | D1-D6 已闭合（000 已实施验证 D1/D2/D3）；本 Iteration 无新设计选择，全部继承 |
| Iteration Plan | PASS | tasks.md 两 Iteration Map 不变；T4-T6 全分配、依赖有序；平衡审计已过 |
| Cycle Scope | PASS | initial；gaps None；行为面=CLI 接线+e2e，lib 层排除 |
| Task Contracts | PASS | T4-T6 契约自包含（Act 只读本 Cycle 可实施）；RED 纪律按行为变化面如实划定（R2 组预期 pre-T4 GREEN，不伪造 RED） |
| Traceability | PASS | R1/R2/R3/R5、MODIFIED S7-S8 → T4/T5/T6 → 测试命名映射闭环 |
| Verification | PASS | 场景级 e2e + 全量 + 静态三命令；无身份型证据工程 |

**Persisted Evidence**

- Mode: none

Act Response（命令、决定性输出、退出码）足以承载验证结果；全部验证可低成本复现。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- R1/S3-S5（事务内 SELECT 文档化语义）依据 `data_scan.rs` 无快照扫描代码路径预判；若 e2e 实测与场景不符 → NEW-EVIDENCE 返回 Plan（Act 不得改引擎代码凑场景）。
- `run_sql` 的失败路径与收尾路径都要 rollback——遗漏任一路径会让未提交版本依赖 close() 后的恢复清理（功能等价但 stderr 缺失），T5 的 R3/S3-S4 测试会锁定。
- cargo 配置弃用告警（`/home/daivy/.cargo/config`）为环境既有噪声，与本 change 无关，不计入 clippy/fmt 结论。

## Act Response

- Status: reported

**Implemented**

- **T4 CLI 会话接线（R1/R3/S7/S8 行为面）**：`src/cli/mod.rs::run_sql` 每调用新建 `TransactionSession`（D1），循环内每条语句先经 `classify_transaction_statement` 分派——`Ok(Some(kind))` → `session.begin/commit/rollback(db)`，成功直接渲染 `QueryPayload::Affected(0)`（D4，不经 executor）；`Ok(None)` → 既有 `plan_stage` → columns → 会话活跃时 `execute_stage_in_tx(db, plan, tx_id)`（tx_id 取自 `session.tx_id()`，无 expect）、空闲时既有 `execute_stage(db, plan, false)`（逐字节等价保留）；`Err(e)`（边界子句）→ fail-fast，原始 D3 文案直达。`sql_failure_status` 增 `in_transaction: bool` 参数：k>1 时后缀按上下文二选一（D3 目录），k>1 门槛与模板其余不变。错误返回路径统一先 `rollback_session`（新 helper：session active → 显式回滚、失败仅 stderr 记录不掩盖原始错误、返回事务上下文标志）；stdout 写失败（General）路径同样回滚。循环正常结束 session 仍活跃 → 显式回滚 + stderr `uncommitted transaction was rolled back at exit` + exit 0（D5）。
- **T5 CLI e2e 测试（R1/R2/R3/S7/S8 锁定面）**：`tests/tx_statement_test.rs` 追加 CLI 段 16 测试 + 夹具（`run_cli`/`cli_fixture`/`run_cli_ok`/`json_docs`/`select_rows`，复用 cli_test 模式：真二进制 `CARGO_BIN_EXE_rtsql` + 独立 TempDir + `RTSQL_HOME` + 管道默认 JSON + 60s kill），场景命名与映射注释逐条对应契约（文件头映射表）。文案断言全部使用 D3 子串；重开可见性经第二次 CLI 调用验证（纯黑盒）。
- **T6 全量回归清扫（R5）**：无清扫性修复需求；四项命令全 PASS（见 Verification Evidence）。

**Changed Files and Symbols**

| 文件 | 变化 |
|---|---|
| `src/cli/mod.rs` | `run_sql` 会话分派重写（分类三臂 + 事务内执行 + 收尾回滚）；新 `rollback_session` helper；`sql_failure_status` +`in_transaction` 参数与上下文后缀；+`classify_transaction_statement`/`TxStatementKind`/`TransactionSession`/`execute_stage_in_tx` imports |
| `src/cli/lifecycle.rs` | restore 路径两处 `sql_failure_status` 调用点补 `false`（签名变化的必要后果；restore 为 auto-commit 上下文，行为逐字节不变） |
| `tests/tx_statement_test.rs` | +CLI e2e 段 16 测试与 5 夹具 helper（lib 段 3 测试零修改）；+imports（`Read`/`Path`/`Command`/`Stdio`/`Duration`/`Instant`/`TempDir`） |

Iteration 000 产物（`planner/mod.rs`、`error.rs`、`transaction/session.rs`、`transaction/mod.rs`）本 Iteration 零修改。

**Deviations from Plan**

1. **T5 测试先于 T4 落地（顺序与"建议顺序：T4 先行"相反）**：契约的绑定见证纪律要求"R1/R3 组在 T4 前至少一条 RED 记录"，只有测试先于 T4 存在才能取得 pre-T4 RED；"建议"非绑定，按 Gate 3"先写测试再观察 RED"执行。实测 pre-T4：12 failed（R1×5、R3×4、S7、S8、R2/S5）/ 7 passed（3 lib + R2/S1-S4），与"R2 测试在 T4 之前即 GREEN"的预期有一处出入——R2/S5 断言 `no active transaction`（会话语义）本身即 T4 行为，pre-T4 RED 属预期修正（记录不伪造）。非实质。
2. **`sql_failure_status` 加参波及 `lifecycle.rs` 两处调用点**：实现形态（加参数 vs 调用点分支）契约已声明非实质；选择加参后 restore/import 两处 auto-commit 调用点显式传 `false`，行为逐字节等价。Change Surface 未列出 lifecycle.rs，属签名变化的必要最小后果。非实质。
3. **回滚失败 stderr 文案在 D5 授权内自选**：D3 只冻结成功提示文案；D5 要求"stderr 记录错误文本，不掩盖原始错误"。实际采用收尾路径 `uncommitted transaction rollback failed: {e}`、错误路径 `session rollback failed: {e}`。不可 e2e 观察（需 WAL abort 错误注入），非实质。
4. **stdout 写失败（General）路径亦先回滚**：契约"任意错误返回路径 session active 时先显式 rollback"按字面执行；该路径无法在管道夹具下稳定触发，无专测。非实质。
5. `run_sql` 分派绑定命名 `tx_kind`（避免遮蔽 `kind(format)` 函数，编译修正）。非实质。

**Blocker Handoff**

None

**Blocker Resolution**

None（未发生阻塞）

**Self-Review**

- Spec review（逐任务）：T4——R1/S1-S5、R3/S1-S4、S7/S8 行为面全部落地并经 T5 锁定；Preserve 面逐项核对：`plan_stage(db, &statement_text, stmt, false)` 调用形态与缓存键逐字节不变、空闲会话走原 `execute_stage(db, plan, false)`、parse 零执行路径不变、退出码分类零变化、lib 层与渲染层零触碰（diff 证实）、既有 cli_test 断言零修改通过（全量 797 含之）。T5——16 测试命名与契约逐一对应、requirement-scenario 映射注释保留、D3 子串断言、R2/S1-S4 pre-T4 即 GREEN 锁定既有拒绝行为、RED 仅取自行为变化场景（12 条，含预期修正后的 R2/S5，见 Deviation 1）。T6——四命令新鲜全绿，测试总数 781→797 只增不减。
- 完整 diff review：变更面 = Changed Files 全集，无计划外文件/符号；无新增警告（clippy 0）、无死代码（`rollback_session` 为全部错误路径共用 helper）、无重复实现；测试不因错误原因通过（断言 exit code + D3 子串 + 行集内容 + 重开持久化，非捕获工具自证）。跨任务交互核对：分类器 Err + 会话活跃 → 先回滚再 fail-fast（R2 拒绝不改变会话态的语义按 Implementation Guidance 解释落地，S2/R3 嵌套 BEGIN 测试覆盖 Begin-Err-with-active-session 路径）；事务内 DDL 经 `execute_stage_in_tx` 立即生效不被回滚（R1/S5 绿）。
- 已修复发现：`rollback_session` doc-comment 笔误（"返回返回时"→"返回"），修复后重跑 fmt/clippy/全量（覆盖范围不变，结论见 Verification Evidence 末轮）。
- 遗留 Minor 问题：无。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T5 pre-T4 RED 见证 | `cargo test --test tx_statement_test`（T4 未合入时） | `test result: FAILED. 7 passed; 12 failed`；失败集 = R1×5 + R3×4 + S7 + S8 + R2/S5，报错均为 `Plan error: transaction statements (BEGIN/COMMIT/ROLLBACK) are only supported in an rtsql CLI session`（exit 3） | 行为变化场景变更前基线；R2/S1-S4 既有拒绝行为同期锁定 GREEN | RED ✅ |
| T4 手工探针① | `rtsql p1.db "BEGIN; INSERT INTO t VALUES (1); COMMIT"`（真二进制） | `{"affected_rows":0}` / `{"affected_rows":1}` / `{"affected_rows":0}`，exit 0 | 三段顺序输出（R1/S1 THEN） | PASS |
| T4 手工探针② | `rtsql p1.db "SELECT id FROM t"`（重开） | `{"columns":["id"],"rows":[[1]]}`，exit 0 | 提交持久可见（R1/S1 AND） | PASS |
| T4 手工探针③ | `rtsql p1.db "BEGIN; INSERT INTO t VALUES (2); ROLLBACK"` + 重开 | 三段输出 exit 0；重开仅 `[[1]]` | ROLLBACK 无残留（R1/S2） | PASS |
| T4 手工探针④ | `rtsql p1.db "BEGIN; INSERT INTO t VALUES (9)"`（无终结） + 重开 | stderr `uncommitted transaction was rolled back at exit`，exit 0；重开仅 `[[1]]` | 收尾回滚 + 提示 + exit 0（R3/S3） | PASS |
| T4 手工探针⑤ | `rtsql p3.db "BEGIN; INSERT INTO t VALUES (1); INSERT INTO missing_table VALUES (2)"` | `statement 3 of 3 failed: ...; previous statement(s) were not committed (rolled back with the transaction)`，exit 3 | 事务上下文 fail-fast 后缀（R3/S4、S7） | PASS |
| T5 GREEN | `cargo test --test tx_statement_test` | `test result: ok. 19 passed; 0 failed`（3 lib + 16 CLI） | R1/R2/R3/S7/S8 全场景 + R4 lib 段回归 | PASS |
| 全量回归（R5） | `cargo test`（Review 修复后新鲜复跑） | `passed=797 failed=0 ignored=2`（基线 781+16，exit 0） | 全部既有行为零回归 | PASS |
| 静态分析 | `cargo clippy --all-targets -- -D warnings` | `Finished ... 0 warning`（exit 0） | 全 crate（lib+bins+tests） | PASS |
| 格式化 | `cargo fmt --check` | 0 diff（exit 0） | 全 crate | PASS |
| OpenSpec | `openspec validate 2026-09-10-ms11-t02-sql-transaction-statements` | `Change '...' is valid`（exit 0） | change 产物一致性 | PASS |

**Persisted Evidence**

None required（Mode: none；全部验证可低成本复现，Act Response 摘录已足够）

**Experience Candidates**

None

**Remaining Issues**

1. **既有 lib 测试 env 竞态 flake（非本 change 行为回归，归 Plan Review 裁定路由）**：`cli::resolve::tests::test_db_dir_env_cases` 在全量 `cargo test` 中偶发失败（约 6 次全量运行中观察到 1 次，本 change 收尾前）；根因是它与 `test_bare_name_env_cases` 为两个并发改写进程全局 `HOME`/`RTSQL_HOME` 的测试（各自持有独立 `EnvGuard`，`src/cli/resolve.rs:79-127`），guard drop 恢复 HOME 的窗口撞上对方的断言窗口。本 change 未触碰 `resolve.rs` 与两个测试，且 diff 无任何 env 变更——新增 16 个子进程型 e2e 测试提高全量并行负载，放大了该既有窗口。单独 `--lib` 稳定通过；修复需改动既有测试（本 change R5 禁止），建议后续登记 Improvement（env 用例合并单测试或串行化）。
2. 其余无。

**Commit or Diff Reference**

未 commit（待用户触发）；工作区 diff 基于 `c468055`（master），含 Iteration 000（已 accepted）与 Iteration 001 全部改动：`src/cli/mod.rs`（run_sql 重写 +2 helpers）、`src/cli/lifecycle.rs`（2 调用点）、`tests/tx_statement_test.rs`（+16 CLI 测试），及 Iteration 000 的 `src/parser/error.rs`/`src/parser/planner/mod.rs`/`src/transaction/mod.rs`/`src/transaction/session.rs`/lib 测试段。

## Plan Review

- Review Result: accepted

**Findings**

1. **F1（非阻塞 Minor，ACT-DEVIATION，Act 已声明）**：T5 先于 T4 落地，与契约"建议顺序：T4 先行"相反——契约 RED 见证纪律（R1/R3 组在 T4 前至少一条 RED）要求测试先行，属绑定纪律对建议顺序的必要反转；pre-T4 RED 12 条（R1×5、R3×4、S7、S8、R2/S5）与 R2/S5 会话语义预期修正已如实记录，不伪造 RED。实现与契约逐条核对无偏离。维持非实质。
2. **F2（非阻塞 Minor，PLAN-OMISSION）**：Change Surface 未列 `src/cli/lifecycle.rs`——`sql_failure_status` 加参（契约授权的实现形态选择："加参数 vs 调用点分支非实质"）的必要最小后果。Plan Review 独立核实 diff：仅 2 处 restore 调用点显式传 `false`，restore 为 auto-commit 上下文，行为逐字节等价。
3. **F3（非阻塞 Minor，覆盖注记）**：边界子句拒绝 × 会话事务活跃的组合路径（如 `BEGIN; INSERT ...; SAVEPOINT sp1`）无 spec 场景、无 e2e 用例——无 Acceptance 缺口（R2 各场景均为独立调用、空闲会话；R2"会话态不变"语义按 Plan Context Implementation Guidance 解释为"被拒语句不改变会话态"，回滚属 D5 收尾规则）。Plan Review 独立二进制探针验证该路径行为正确（见 Evidence ④）：exit 3、D3 点名文案、事务上下文后缀、重开无残留。不要求返工；如需锁定该组合路径，可由 docs-maintainer 登记 Improvement（可选，非本 change 义务）。
4. **F4（非阻塞 Minor，Act Remaining Issue #1 采信并裁定路由）**：`src/cli/resolve.rs` 两个 env 测试（`test_db_dir_env_cases` / `test_bare_name_env_cases`）各自持有独立 `EnvGuard` 并发改写进程全局 `HOME`/`RTSQL_HOME` 的既有竞态窗口（Act 观察约 1/6 全量运行；本次 Plan Review 独立全量运行未复现）。本 change 未触碰 `resolve.rs` 与两测试，diff 无任何 env 变更；修复需修改既有测试（本 change R5 禁止）。裁定：非本 change Acceptance 问题，不阻塞收尾；收尾时由 docs-maintainer 登记 Improvement（env 用例合并单测试或串行化）。
5. **非阻塞确认**：回滚失败 stderr 文案（`uncommitted transaction rollback failed: {e}` / `session rollback failed: {e}`）在 D5 授权内自选（D3 未冻结该两处）；`tx_kind` 分派绑定命名为避免遮蔽 `kind(format)` 的编译性局部修正；stdout 写失败（General）路径先回滚属"任意错误返回路径"契约的字面执行（管道夹具下不可稳定触发，无专测）——均非实质。

**Deviation Classification**

ACT-DEVIATION ×2（T5/T4 顺序反转——绑定 RED 纪律的必要结果；`tx_kind` 命名）+ PLAN-OMISSION ×1（Change Surface 漏列 `lifecycle.rs`，行为面无影响）。全部非实质、非阻塞；无 NEW-EVIDENCE、无 BASELINE-CHANGED、无实质 Act 偏离、无身份型证据工程。

**Acceptance Gaps**

None——R1/S1-S5、R2/S1-S5（含 R2/S5 `AND NO CHAIN` 等价）、R3/S1-S4、MODIFIED S7-S8 全部经 T5 e2e 锁定（`tx_statement_test` 19 测试：3 lib + 16 CLI，场景命名与契约逐一对应）；R5/S1-S3 经独立复跑确认（既有测试套件零修改 + 全量基线只增不减 + 静态三命令）。稳定基线达成：MS11-T02 全部 Acceptance 关闭。

**Convergence**

N/A（首次 Review，无上一版 gap 可比）

**Evidence**

Plan Review 独立核实（2026-09-10，不采信 Act Self-Review 作为替代）：①代码读取 `src/cli/mod.rs`（`run_sql` 分类三臂重写、`QueryPayload::Affected(0)` 直渲染、`session.tx_id()` Some→`execute_stage_in_tx`/None→`execute_stage` 逐字节等价保留、`rollback_session` helper、收尾回滚在 `db.close()` 之前、`sql_failure_status` +`in_transaction` 后缀二选一）与 Plan Context Critical Path 逐条一致；`lifecycle.rs` 2 调用点；`tests/tx_statement_test.rs` 19 测试；Iteration 000 产物（`planner/mod.rs`/`error.rs`/`transaction/session.rs`/`transaction/mod.rs`）与 accepted 状态一致、本 Iteration 零修改；既有测试文件零修改（git status 证实）；②独立复跑 `cargo test` 全量 `passed=797 failed=0 ignored=2`（797 ok 行逐一计数）；③`cargo clippy --all-targets -- -D warnings` 0 warning（`~/.cargo/config` 弃用提示为环境既有噪声，不计入）、`cargo fmt --check` 0 diff、`openspec validate 2026-09-10-ms11-t02-sql-transaction-statements` PASS（三命令 exit 0）；④独立二进制探针（F3 组合路径，真二进制）：`rtsql <db> "BEGIN; INSERT INTO t VALUES (1); SAVEPOINT sp1"` → exit 3，stderr `statement 3 of 3 failed: SAVEPOINT is not supported; statement: SAVEPOINT sp1; previous statement(s) were not committed (rolled back with the transaction)`；随后重开 `SELECT id FROM t` → 空行集、exit 0（D5 收尾回滚生效，无部分提交残留）。

**Follow-up Decision**

accepted——实现满足 Iteration 001 全部既有 Acceptance；无当前 Cycle 修复项；无 rework/replan 需求。F3/F4 两项可选后续（组合路径 e2e 锁定、env 竞态 Improvement 登记）不属本 change 返工面，由用户决定是否在收尾时交 docs-maintainer 登记。

**Iteration Plan Update**

None（Map 不变）

**Next Cycle**

None

**Next Iteration**

None（change 最后一个 Iteration 已 accepted；收尾——SNAPSHOT/tasks 同步、spec 合并、归档——由用户触发 openspec-docs-maintainer）
