# Iteration 001 / Cycle 000: 错误可操作化与会话边界锁定

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2（= MS19-T02，能力名携带 + PG 错误码分类）、T3（= MS19-T03，会话边界组合 e2e）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal R2/R3/R4（`error-reporting` ADDED）、R5（`sql-transaction-statements` MODIFIED R2）、design D2/D3/D4、tasks.md Iteration 001 边界
- Excluded scope: StorageError/NetworkError 分类扩展、新错误文案重写、REPL、Iteration 000 的 delete 面、tx 语句语义变化

**Objective**

`UnsupportedStatement`/`UnsupportedExpression` 报错直接点名能力；PG 错误响应按类别携带 SQLSTATE 且 JSON 协议字节不变；既有错误语义（退出码矩阵、文案框架、显式拒绝变体）零回归；会话事务活跃中遇边界子句的组合语义有 e2e 锁定。

**Investigation Facts**

- Current Baseline: Iteration 000 accepted（change `iterations/000-delete-backup-boundary/000-initial.md` Plan Review）：master + T1 交付（`src/cli/mod.rs` Delete 变体、`lifecycle.rs::delete_db`、cli_test +7、README 双语），全量 **1246 tests / 0 failures / 2 ignored**（Act 串行全量 + Plan 对 cli_test 的定向复跑均新鲜）。错误面相关文件（parser/pipeline/network）自基线以来零变化——Plan 调查结论直接适用。
- Current-State Evidence:
  - `PlanError`（`src/parser/error.rs:7-62`，`#[derive(Debug, Clone)]`）：`UnsupportedStatement`（:13）、`UnsupportedExpression`（:17）无载荷；`Display` :69 `Unsupported statement type`、:71 `Unsupported expression type`。当前实测文本（Plan 探针）：`statement 1 of 1 failed: Plan error: Unsupported statement type; statement: MERGE …`。
  - 构造点全集（grep 已清点）：`UnsupportedStatement` ×15——ast.rs :16 :28 :97 :124 :128 :195 :219 :225 :251（extract helper catch-all），planner/mod.rs :226（Drop 非 Table 臂）、:229（`build_plan` 顶层 catch-all），query.rs :778，ddl_dml.rs :590 :937 :944；`UnsupportedExpression` ×16——expression.rs :79 :375 :421 :504 :574 :629 :676 :793，aggregate.rs :88 :125 :161 :173（其中 :161/:676 为 `.ok_or(...)` 形态），subquery.rs :380，ddl_dml.rs :140。
  - `plan_stage`（`src/pipeline.rs:58-88`）返回 `Result<PhysicalPlan, String>`，:67/:81 `format!("Plan error: {e}")`（前缀在此引入）；产品调用方 4 处：`run_sql`（`src/cli/mod.rs:408`）、`execute_in_tx`（`pipeline.rs:271`）、`execute_inner`（`pipeline.rs:394`）、restore（`src/cli/lifecycle.rs:334`）；pipeline 单测若干（:1219-1312 区段，断言 String 错误）。`parse_stage`（`pipeline.rs:44-53`）返回 String；parse 文本存在既有双前缀 quirk（`Parse error: Parse error: sql parser error: …`，实测）——本变更**保持不动**。
  - lib/PG 漏斗：`Database::execute_sql`（`src/database.rs:176`→`pipeline::execute`→`execute_inner` `pipeline.rs:321`）、`Database::execute_in_tx`（`database.rs:231`→`pipeline.rs:255`）；PG 与 JSON 协议均消费 SqlHandler 产出的 `Response`。
  - `Response`（`src/network/protocol.rs:33-39`，`#[derive(Debug, Clone, Serialize, Deserialize)]`）：`Error { message: String }` 单字段；`Response::Error` 构造点约 23 处（pipeline.rs 为主 + network/handler.rs 等——Act 以 grep 全集枚举后机械补字段）。
  - PG 出口：`pg_protocol.rs:306` 硬编码 `let (severity, code) = ("ERROR", "58000");`；`pg_messages::error_response(severity, code, message)`（`pg_messages.rs:225`）已参数化无需改；`pg_messages_test.rs:158` 断言 `NetworkError`（Io/ProtocolWrite）→ 58000——保持（未分类兜底语义）。
  - 测试面：`contains("Unsupported statement type"|"Unsupported expression")` 形态断言 7 处、分布 6 文件（subquery/datetime_function/scalar_function/expression_e2e/nested_loop_join/datetime_type）——文案追加 `: {name}` 后全部存活，无需修改；另有 doc 注释 ~28 处引用该短语（不修改）。`tests/pg_integration_test.rs` 有真 PG 套接字 e2e 基建；`tests/tx_statement_test.rs:158` `run_cli` 夹具（`CARGO_BIN_EXE_rtsql` + TempDir CWD/RTSQL_HOME + 管道捕获）+ `sql_failure_status` 后缀语义（`src/cli/mod.rs:486-508`：k>1 且事务上下文 → `; previous statement(s) were not committed (rolled back with the transaction)`）。
- Code and Critical Path: `parser/error.rs`（两变体载荷 + Display + 新 `sqlstate()`）→ planner 5 文件构造点（payload 字面量）→ `pipeline.rs`（`PlanFailure` 类型 + `plan_stage` 签名 + :271/:394 漏斗臂 + 既有单测机械适配）→ `network/protocol.rs`（`Error` 变体加可选 `sqlstate` 字段 + 全部构造点机械补 `None`）→ `pg_protocol.rs:306`（`unwrap_or("58000")`）→ 测试（error.rs 单测、pg_integration、tx_statement）。T3 只触 `tests/tx_statement_test.rs`，产品代码零改动。

**Implementation Guidance**

按 design D2/D3/D4 执行，要点（细节以 D 为准）：

- 顺序：T2a 能力名（RED：planner/cli 断言 MERGE 点名）→ T2b SQLSTATE 通道（RED：sqlstate 映射单测 + serde 单测 + PG e2e）→ T3（纯新增测试）。T2a/T2b 同文件交叉（error.rs），按序减少往返。
- D2：payload 用 `String`（顶层 catch-all 的名字来自运行时语句首词，需 owned）；`Display` 追加 `: {0}`（`Unsupported statement type: MERGE`）。`mod.rs:229` 顶层 catch-all 经 ast.rs 新 helper 取 `stmt.to_string()` 首词（sqlparser Display 以大写关键字开头；空/异常回落 `"statement"`）；`mod.rs:226` 传 `format!("DROP {}", object_type)` 大写化形态；其余 29 处按被拒形态传字面量（措辞非实质，Act 定稿后测试断言与之一致）。
- D3：`PlanError::sqlstate(&self) -> &'static str` 映射表见 design（ParseError→42601；UnsupportedValue/UnsupportedStatement/UnsupportedExpression/UnsupportedJoinType/UnsupportedConstraint/UnsupportedSubqueryPosition/InSubqueryJoinUnsupported/TransactionStatement→0A000；TableNotFound→42P01；ColumnNotFound/GroupByColumnNotFound→42703；AmbiguousColumn→42702；其余→58000）。`PlanFailure { message: String, sqlstate: &'static str }`（pipeline.rs）+ `Display`（→ message）+ `From<PlanError>`（`format!("Plan error: {e}")` + `sqlstate()`）；`plan_stage` 返回 `Result<PhysicalPlan, PlanFailure>`；`execute_inner` parse 臂携带 `Some("42601")`、plan 臂携带 `Some(e.sqlstate)`；其余 `Response::Error` 构造点全部补 `sqlstate: None`；`pg_protocol.rs:306` 改 `sqlstate.as_deref().unwrap_or("58000")`；`protocol.rs` Error 字段加 `#[serde(skip_serializing_if = "Option::is_none")]`（None 时 JSON 逐字节不变）。
- 既有 pipeline 单测（:1236 等）对 String 错误的断言做机械类型适配（比较 `Display`/message，不改断言内容）；7 处 contains 断言零修改。

**Task Contracts**

### T2: 能力名携带 + PG 错误码分类（R2/R3/R4）

- Requirement/Scenario: R2 三场景、R3 四场景、R4 两场景（`specs/error-reporting` ADDED delta）
- Depends on: None
- Targets: `src/parser/error.rs`（两变体 `String` 载荷、Display 追加、`sqlstate()`）；`src/parser/ast.rs`（`unsupported_statement_name` helper + 9 站点）；`src/parser/planner/{mod,query,ddl_dml,expression,aggregate,subquery}.rs`（22 站点 payload）；`src/pipeline.rs`（`PlanFailure`、`plan_stage` 签名、:271/:394 臂、parse 臂 42601、既有单测机械适配）；`src/cli/mod.rs:408` 与 `src/cli/lifecycle.rs:334`（调用方机械适配）；`src/network/protocol.rs`（Error 变体字段）+ 全部 `Response::Error` 构造点（grep 全集补 `None`）；`src/network/pg_protocol.rs:306`
- Current behavior: 两文案无能力名（`Plan error: Unsupported statement type`）；PG 全部错误 58000；`plan_stage` 返回 `String`；`Response::Error` 单字段。
- Required behavior: 两文案追加 `: {能力名}`（MERGE 形态可达且点名）；`sqlstate()` 全变体映射成立；`Response::Error` 携带可选 sqlstate（None 时 JSON 输出逐字节不变）；PG 错误按分类出码（能力拒绝 0A000、解析 42601、表 42P01、列 42703/42702、其余 58000）；CLI 退出码矩阵与错误文案框架不变。
- Preserve: parse 双前缀 quirk 与全部既有消息措辞（唯一变化=两变体追加）；显式拒绝变体（`UnsupportedConstraint`/`InSubqueryJoinUnsupported`/`TransactionStatement`/`ColumnNotFound` 等）文案逐字节；既有 contains 断言零修改通过；`pg_messages_test.rs:158`（NetworkError→58000）；`Plan error: ` 前缀框架；T1 的 delete 面行为。
- Forbidden: 不改 StorageError/NetworkError 分类与文案；不重写既有消息；不新增错误类别出口（JSON 协议消费方除可选新字段外零感知）；不动 tx 语句语义与 delete 子命令。
- Test witness: RED 先行——① planner/cli：MERGE 语句报错含 `MERGE`（R2-S1）；可解析的未支持表达式形态报错点名（R2-S2，形态从站点上下文选定）；既有显式拒绝面文案不变断言（R2-S3）；② `error.rs` 单测：`sqlstate()` 全变体映射表（R3）；③ serde 单测：`Response::Error` None 跳过字段 / Some 输出（R3 场景 4 通道 + JSON 兼容）；④ `tests/pg_integration_test.rs`：0A000 / 42601 / 42P01 / 42703 各一（R3 场景 1-3）；⑤ 既有 7 处 contains 断言零修改通过（R4）。
- GREEN condition: `cargo test --test planner_test --test pg_integration_test --test pg_messages_test` 全绿 + 全量串行零回归（1246+新增，既有断言零校准——pipeline 单测的类型机械适配除外）。
- Verification: ① 上述目标测试命令，退出码 0；② `cargo test -- --test-threads=1` 全量（基线 1246+新增）；③ `cargo fmt --check` 对本次触碰文件无新增偏差。失败含义：映射表缺变体→编译期穷尽失败；JSON 字节变化→serde 单测红。
- Stop when: 存在无法机械补 `None` 的 `Response::Error` 构造点（共享签名/宏约束）；sqlparser Display 首词对常见形态不可读（helper 需语义化匹配表——回 Plan 评估规模）；PG e2e 基建不足以断言 SQLSTATE 字段。

### T3: 会话边界组合场景 e2e（R5）

- Requirement/Scenario: R5——`sql-transaction-statements` MODIFIED R2 新增两组合场景
- Depends on: None（与 T2 无共享面；文案断言不依赖 T2——组合场景的错误载体是 `TransactionStatement`，文案不变）
- Targets: `tests/tx_statement_test.rs`（+2 用例，`run_cli` 夹具）
- Current behavior: 组合路径（`BEGIN; INSERT …; SAVEPOINT sp1`）行为正确（exit 3 + `TransactionStatement` 点名 + `not committed` 后缀 + 收尾回滚，MS11-T02 Plan Review 探针验证，I042 记录）但无 e2e。
- Required behavior: 两用例按 spec 场景断言：`BEGIN; INSERT INTO t VALUES (1); SAVEPOINT sp1` → exit 3、stderr 点名 `SAVEPOINT` 且含 `not committed`；重开查询无行。`BEGIN; INSERT INTO t VALUES (1); SET TRANSACTION ISOLATION LEVEL READ COMMITTED` → 同构断言点名 `SET TRANSACTION`。
- Preserve: 既有 tx_statement 用例零修改；产品代码零改动（本任务）。
- Forbidden: 不改 `run_sql`/`sql_failure_status`/分类器——纯测试加固。
- Test witness: 预期直接 GREEN（I042 记录行为已验证；若 RED 则说明 MS11-T02 后发生回归——按 Stop when 返 Plan）。
- GREEN condition: `cargo test --test tx_statement_test` 全绿。
- Verification: `cargo test --test tx_statement_test`，退出码 0；全量串行由 T2 验证统一覆盖。
- Stop when: 组合行为与 spec 场景不符（exit 码/后缀/残留任一不符——表明回归，返 Plan）。

**Invariants**

- 错误文案框架与退出码矩阵零变化（唯一允许差异=两变体追加能力名）。
- JSON 协议对 None sqlstate 逐字节不变；PG 对未分类错误维持 58000。
- `plan_stage` 错误文本经 `Display` 对所有调用方逐字节等价（仅类型通道变化）。
- T3 零产品代码改动。

**Non-goals**

- StorageError/NetworkError 分类；错误消息措辞重写；REPL；`install.sh`；Iteration 000 面。

**Acceptance**

- R2/R3/R4/R5 全部场景有通过断言（映射见 tasks.md RTM）；全量测试零回归；JSON 兼容与 PG 分类各有直接见证。

**Verification**

- R2-S1：`cargo test --test planner_test unsupported_statement`（或 cli 形态）——断言含 `MERGE`，退出码 0。
- R2-S2/S3：planner/cli 断言 + 既有显式拒绝用例集零修改通过。
- R3-S1..S4：`cargo test --test pg_integration_test` + `error.rs`/serde 单测。
- R4：全量 `cargo test -- --test-threads=1` 零校准（pipeline 单测机械适配除外，属类型适配非文案变化）。
- R5：`cargo test --test tx_statement_test` 两新用例。

**Gate 2 Readiness**

（本 Cycle 创建时为 draft；Gate 2 逐项结论见 Plan 交接报告，用户批准计划后置 `ready`。）

**Persisted Evidence**

- Mode: none

（全部验证为低成本可重跑命令，Act Response 承载结论。）

**Risks and Notes**

- 23 处 `Response::Error` 构造点为机械补字段，编译期穷尽保护，遗漏即编译失败，无静默风险。
- 顶层 catch-all 首词提取依赖 sqlparser Display 大写关键字开头；个别罕见形态若不可读，为非实质措辞问题（Act 调整个别字面量或 helper 局部匹配），不阻塞。
- pipeline 既有单测对 `plan_stage` 错误类型的断言需机械适配（`String` → `PlanFailure` 比较），属类型适配；若 Act 发现断言语义（消息内容）也需变化，按 Stop when 返 Plan。
- PG e2e 断言 SQLSTATE 需要解析 ErrorResponse 字段——`tests/pg_integration_test.rs` 基建已解析错误消息（Act 复用既有解析路径扩展字段断言）；若基建无字段级解析，Stop when 返 Plan 评估。

## Act Response

- Status: reported

**Implemented**

T2（= MS19-T02）与 T3（= MS19-T03）全部交付，Iteration 001 的两个 task 完成；Iteration 完成与否待 Plan Review `accepted` 后由 Plan 判定。

产品代码：

- `src/parser/error.rs`：`UnsupportedStatement(String)` / `UnsupportedExpression(String)` 携带能力名，`Display` 追加 `: {name}`（`Unsupported statement type: MERGE`）；新增 `pub fn sqlstate(&self) -> &'static str`（D3 映射表：ParseError→42601；UnsupportedValue/Statement/Expression/JoinType/Constraint/SubqueryPosition/InSubqueryJoinUnsupported/TransactionStatement→0A000；TableNotFound→42P01；ColumnNotFound/GroupByColumnNotFound→42703；AmbiguousColumn→42702；其余→58000）。新增 `#[cfg(test)]` 单测 `planerror_sqlstate_mapping`（27 变体全枚举）+ `unsupported_variants_display_capability_name`。
- `src/parser/ast.rs`：新 helper `unsupported_statement_name(&Statement) -> String`（sqlparser Display 首词，空文本回落 `"STATEMENT"`）；9 个 `UnsupportedStatement` 构造点填充能力名（`SET OPERATION` / `DERIVED TABLE` ×2 / 未知聚合函数名（动态，取 `name`）/ `UNSUPPORTED SELECT ITEM EXPRESSION` ×2 / `SELECT ITEM` ×2）。
- `src/parser/planner/`：22 个构造点填充——`mod.rs` Drop 非 Table 臂 `format!("DROP {object_type}")`、顶层 catch-all 走新 helper；`query.rs` `WHERE CLAUSE ON JOIN`；`ddl_dml.rs` `JOIN ON CONSTRAINT` / `INSERT ... SELECT` / `MULTI-COLUMN UPDATE` / `COMPOUND ASSIGNMENT TARGET`；`expression.rs` `COLUMN REFERENCE` / 类型化字面量回退（动态取 `data_type` 大写）/ `ARITHMETIC OPERATOR {op}` / `COALESCE ARGUMENT` / 未注册函数名（动态 `func_name`）/ `EXPRESSION` / `COMPARISON OPERATOR {op}` / `PREDICATE`；`aggregate.rs` 未知聚合名（动态）/ `EXPRESSION` / `COMPARISON OPERATOR {op}` / `HAVING PREDICATE`；`subquery.rs` `COLUMN REFERENCE`。
- `src/pipeline.rs`：新增 `pub struct PlanFailure { message, sqlstate }` + `Display`（→ message，调用方文本逐字节等价）+ `From<PlanError>`（`Plan error: {e}` 前缀框架保持）；`plan_stage` 返回类型 `Result<PhysicalPlan, PlanFailure>`；`register_table` 返回 `PlanFailure`（表缺失文案不变，分类经 `PlanError::TableNotFound(..).sqlstate()` 取自映射表）；两个 parse 臂携带 `Some(PARSE_SQLSTATE)`（新常量 `42601`），两个 plan 臂携带 `Some(failure.sqlstate)`；其余 `Response::Error` 构造点补 `sqlstate: None`。
- `src/network/protocol.rs`：`Error { message, sqlstate: Option<String> }`，`#[serde(skip_serializing_if = "Option::is_none")]`；新增 2 个 serde 单测（None 逐字节不变 / Some 出现）。
- `src/network/pg_protocol.rs`：`Response::Error` 臂改 `sqlstate.as_deref().unwrap_or("58000")`（未分类维持兜底）。
- `src/network/handler.rs`、`src/cli/mod.rs`、`src/cli/lifecycle.rs`：调用方与模式匹配机械适配（`&e` → `&e.to_string()`、模式补 `..`）。

测试：

- `tests/planner_test.rs` +4：`unsupported_statement_names_capability`（MERGE）、`unsupported_statement_names_drop_object_type`（DROP VIEW）、`unsupported_expression_names_capability`（列间取模）、`explicit_rejection_messages_unchanged`（CHECK / FK / 会话外事务语句 / IN 子查询 JOIN 四类逐字节相等）。
- `tests/cli_test.rs` +2：`test_unsupported_statement_names_capability_cli`（exit 3 + 点名 MERGE + 既有文案框架）、`test_unsupported_expression_exit_code_unchanged`（R4-S1 退出码矩阵见证）。
- `src/network/protocol.rs` +2、lib 内 `error.rs` +2（见上）。
- `tests/pg_integration_test.rs` +3：`pg_capability_error_0a000`、`pg_parse_error_42601`、`pg_name_resolution_errors`（42P01 + 42703），并新增最小 PG 简单查询客户端夹具（`PgClient` / `read_until_ready` / `parse_error_fields` / `ErrorFields`）。
- `tests/tx_statement_test.rs` +2（T3）：`active_session_savepoint_rejected_and_rolled_back`、`active_session_set_transaction_rejected_and_rolled_back`。
- 编译强制面：`Response::Error` 模式匹配补 `..` 共 44 处（`tests/` 23 文件 + `tests/subquery_test.rs` 的 `ref message` 形态）、构造点补 `sqlstate: None` 1 处（`tests/pg_protocol_test.rs`）。

**Deviations from Plan**

1. **`sqlstate()` 提前到 T2a 落地**：error.rs 单文件一次改完，`sqlstate` 与两变体载荷同时写入。T2b 的映射表 RED 仍有效（见证映射正确性），仅「方法存在性」不经 RED——非实质顺序偏差。
2. **能力名取值分两类**：被拒形态的名称在作用域内时取动态值（未知聚合函数名、未注册函数名、`DROP {对象}`、比较/算术算子、类型化字面量的 `data_type`），其余 catch-all 臂取描述形态的静态字面量（Plan 已授权「措辞非实质，Act 定稿」）。
3. **PG e2e 夹具为本次新建**（Plan Risks 假设「基建已解析错误消息」不成立——基线 `tests/pg_integration_test.rs` 只有启动握手用例，无查询与错误响应路径）。按契约点名的三个见证补最小简单查询客户端与 ErrorResponse 字段解析（消息类型 + 4 字节长度逐条读到 `Z`），未新增依赖与生产代码。判为 Plan 事实误设而非实质阻塞：见证目标（SQLSTATE 字段断言）不变，补的是测试夹具本身。
4. **42703 见证改取 JOIN ON 路径**：Plan 设想「不存在列」形态未指明。实际探测发现投影与 WHERE 中的未知列由既有代码构造成 `PlanError::ParseError`（文案 `Column 'x' not found in table 't'`）→ 分类 42601；真正产出 `ColumnNotFound` 的是 JOIN ON 条件列解析路径，故 42703 见证用 `SELECT t.id FROM t JOIN r ON t.nosuchcol = r.a`。修正分类需改既有文案（`Column not found: x`），R4/Preserve 与契约 Forbidden 明令禁止，故不动——见 Experience Candidates 的 Issue 候选 ②。
5. **`register_table` 纳入类型化通道**：Plan 只列 `plan_stage` 签名；因表缺失错误经 `?` 直接冒泡，必须一并改签名才能让 plan 臂携带分类。分类码经映射表取（`PlanError::TableNotFound(..).sqlstate()`），不在漏斗点硬编码字面量。
6. **44 处模式匹配机械适配**：字段新增的编译强制后果（Plan 调查已列 23 处构造点，模式面在其外）。全部为 `..` 补齐，无断言内容改动。
7. **`multi_statement_rejected` 维持 `None`（58000）**：多语句拒绝不在 D3 映射表内，不自造分类。
8. **rustfmt 顺带改写 `protocol.rs` 枚举两个既有单行变体**：为 `Error` 变体加文档注释后 rustfmt 强制整个枚举多行展开，纯格式、无语义。
9. **`cargo fmt` 既有偏差未清理**：`src/executor/delete.rs`、`src/storage/data/table_manager.rs`、`src/wal/recovery.rs`、`tests/cli_test.rs:3346-3823`、`tests/constraint_enforcement_test.rs` 的偏差均为 MS23/MS24 批次已提交内容（与 Iteration 000 Deviations 6 同一集合），按 Surgical Changes 不动。

**Blocker Handoff**

None.

**Blocker Resolution**

None.

**Self-Review**

Gate 3（spec compliance）：R2 三场景各有断言（MERGE 点名 + 退出码 3 由 CLI 用例判定；表达式点名由 planner 用例判定；显式拒绝变体逐字节相等断言 + 既有 5 处 CHECK/FK 用例零修改）。R3 四场景各有见证：0A000 / 42601 / 42P01 / 42703 四个 PG e2e 断言 SQLSTATE 字段；未分类兜底由映射表单测 14 个 58000 条目 + `pg_messages_test.rs:158`（NetworkError→58000，零修改）承担；JSON 兼容由 serde 单测逐字节断言承担。R4 两场景：退出码矩阵既有用例集零校准通过（含新增 exit 3 见证），既有 7 处 `contains("Unsupported statement type"|"Unsupported expression")` 断言零修改通过。R5 两组合场景按 spec 断言 exit 3、点名子句、`not committed` 后缀、重开无残留。契约 Targets 全部落地：31 个 `Unsupported*` 构造点、`plan_stage` 与两个调用方、`Response::Error` 全部构造点、PG 出口。Preserve 项复核：parse 双前缀 quirk 保持（实测 `Parse error: Parse error: sql parser error: …`）；表缺失、事务语句、约束、IN 子查询 JOIN 文案逐字节不变；`Plan error: ` 前缀框架保持；T1 的 delete 面行为不变（`cli_test` 108 passed 含 7 个 delete 用例）。Forbidden 项核对：未改 StorageError/NetworkError 分类与文案；未重写既有消息（唯一变化=两变体追加能力名）；未新增错误变体（JSON 消费方仅感知可选字段）；未动 tx 语句语义与 delete 子命令产品代码。

Gate 3（code quality）：无计划外修改（`git diff --stat` 41 文件，产品 13 + 测试 28，测试侧 24 个文件为字段适配）。错误与边界路径：`WouldBlock`/EOF/畸形 ErrorResponse 载荷均有显式臂（`parse_error_fields` 对缺 NUL 终止符回落 `tail.len()`，`split_first` 对空载荷退出）。无死代码（`unsupported_statement_name` 单调用点、`PlanFailure::from` 三个构造点）。无新增 warning（`cargo clippy --all-targets` exit 0）。无身份型证据机制或判定层：测试经真二进制（CLI 用例）与项目自身测试框架判定；PG 用例解析的是服务端真实协议输出，未引入判定包装。命名与局部结构沿用既有风格（`sqlstate`/`PlanFailure` 与 design D3 命名一致；测试夹具 `PgClient`/`ErrorFields` 局部私有）。

自检发现（均已处理或非阻塞）：

- Minor 1（已修）：初版 `plan_stage` 内对已是 `PlanFailure` 的 `register_table` 结果写 `.map_err(PlanFailure::from)`（靠 `From<T> for T` 恒等 impl 通过编译）——改为直接 `?`，消除阅读歧义。
- Minor 2（已修）：`tests/pg_integration_test.rs` 初版把 ErrorResponse 解析结果结构命名为 `Response`，与协议 `Response` 同名易混淆——改名 `ErrorFields` 并派发 `unwrap_or_default()`。
- Minor 3（已修）：本次触碰的 7 个文件存在 fmt 偏差（`protocol.rs` / `error.rs` / `aggregate.rs` / `ddl_dml.rs` / `expression.rs` / `pipeline.rs` / `pg_integration_test.rs`），均在本 Cycle 新增行上——对这 7 个文件单独 `rustfmt`（不跑全量 `cargo fmt`，避免触碰他处既有偏差）。
- Minor 4（保留，Plan 已预告）：全量串行首跑 `tests/cli_test.rs` 报 1 项失败 `test_sigint_during_open_130`（信号用例，断言拿到非 130 退出码且 stdout/stderr 为空）。按公共规则的一次偶发复核：同命令复核通过（1261/0/2），另单独复跑 `cli_test` 108 passed。属「子进程在信号送达前已完成查询」的既有竞态（`SIGNAL_DELAY_MS` 时间敏感族，与 Iteration 000 Minor 3 同族），未复现故不落 Issue，不在本 change 范围。
- 未发现 Critical / Important 问题。

**Verification Evidence**

① T2a RED（planner 面）：`cargo test --test planner_test -- --test-threads=1`

```
---- unsupported_expression_names_capability stdout ----
不支持表达式必须点名能力，实际: Unsupported expression type
---- unsupported_statement_names_capability stdout ----
不支持语句必须点名能力，实际: Unsupported statement type
---- unsupported_statement_names_drop_object_type stdout ----
非 Table 的 DROP 必须点名对象类型，实际: Unsupported statement type
test result: FAILED. 73 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out
```

② T2a GREEN：`cargo test --test planner_test -- --test-threads=1` → `test result: ok. 76 passed; 0 failed; 0 ignored`（退出码 0；既有 7 处 `contains` 断言零修改通过，见 ③）。

③ 既有文案断言面零修改：`cargo test --lib --test subquery_test --test datetime_function_test --test scalar_function_test --test expression_e2e_test --test nested_loop_join_test --test datetime_type_test -- --test-threads=1` → 6 个二进制全 `ok`（311 / 19 / 16 / 24 / 9 / 32 passed，0 failed）。

④ T2b RED：

- serde 单测编译失败（`variant Response::Error has no field named sqlstate`）；
- PG e2e：`pg_capability_error_0a000` / `pg_parse_error_42601` / `pg_name_resolution_errors` 三项 `left: "58000"` 对 `right: "0A000"/"42601"/"42P01"` 失败。

⑤ T2b GREEN：`cargo test --lib --test pg_integration_test --test pg_messages_test -- --test-threads=1` → `315 passed`（lib，含映射表与 serde 单测）/ `4 passed`（pg_integration）/ `9 passed`（pg_messages，58000 兜底零修改）退出码 0。

⑥ T3：`cargo test --test tx_statement_test -- --test-threads=1` → `test result: ok. 21 passed; 0 failed`（退出码 0；两新组合用例首次即 GREEN，与契约「行为已存在」一致）。

⑦ 全量零回归（R4-S1）：`cargo test -- --test-threads=1`（串行）

```
passed=1261 failed=0 ignored=2        (81 个测试二进制)
```

基线 1246（Iteration 000 accepted）+ 本次新增 15（planner 4 / cli 2 / pg_integration 3 / tx_statement 2 / lib 4）= 1261。**首跑失败记录**：同命令首跑 `tests/cli_test.rs` 报 `107 passed; 1 failed`（`test_sigint_during_open_130`，见 Self-Review Minor 4），复核通过。

⑧ 静态检查与规格校验：

- `cargo clippy --all-targets` → 退出码 0（仅 `~/.cargo/config` 弃用提示，非本项目告警）
- `cargo fmt -- --check` → 本次触碰文件零偏差（残余清单见 Deviations 9，均为既有）
- `openspec validate 2026-09-26-ms19-cli-admin-hardening --strict` → `Change '2026-09-26-ms19-cli-admin-hardening' is valid`，退出码 0

⑨ CLI 实际运行（真二进制，`RTSQL_HOME=/tmp/opencode/ms19c`）：

```
$ rtsql cli "MERGE INTO t USING t ON t.id=t.id WHEN MATCHED THEN UPDATE SET name='x'"
statement 1 of 1 failed: Plan error: Unsupported statement type: MERGE; …          exit=3
$ rtsql cli "SELECT id % name FROM t"
statement 1 of 1 failed: Plan error: Unsupported expression type: ARITHMETIC OPERATOR %; …   exit=3
$ rtsql cli "SELECT (1 FROM t"
Parse error: Parse error: sql parser error: Expected ), found: FROM …              exit=3   （双前缀 quirk 保持）
$ rtsql cli "SELECT id FROM nosuch"
statement 1 of 1 failed: Table 'nosuch' not found: table not found: nosuch; …      exit=3   （文案不变）
$ rtsql cli "SAVEPOINT sp1"
statement 1 of 1 failed: SAVEPOINT is not supported; …                            exit=3   （文案不变）
```

**采信说明**：全量 1261/0/2 结论产生后仅有两处后续编辑——`tests/pg_integration_test.rs` 的 `Response`→`ErrorFields` 重命名（该二进制已复跑 4 passed）与 `planner/mod.rs` 的 helper 导入路径简化（无语义变化，`planner_test` 76 passed + `cli_test` 108 passed + clippy exit 0 已在该编辑后运行）。覆盖范围内既有结论按公共规则 › 验证直接采信。

**Persisted Evidence**

None required（Plan 设定 Mode: none，予以维持）。全部验证为低成本可重跑命令与真二进制运行，结论已落在本节；无一次性环境依赖，不建 `evidence/` 目录。

**Experience Candidates**

- Runbook 候选：None。（本 Cycle 无「结果无法低成本复现」或「高风险且非显然」的操作路径；PG 简单查询夹具已作为测试代码入库，复用方式在测试文件内自解释。）
- Issue 候选（均在本 change 范围外，附证据；由 Recorder 按用户指令决定是否落账）：
  ① **投影项未知列被静默丢弃**（ISS03 同族）——`rtsql <db> "SELECT nosuchcol FROM t2"`（表 `t2(id INT PRIMARY KEY)` 存在）返回退出码 0、列头 `["id"]`、0 行，无任何错误；`SELECT nosuchcol, id FROM t2` 同样静默只出 `id`。证据：Verification ⑨ 之前的实测输出（`{"columns":["id"],"rows":[]}` exit=0），与 `rtsql p "SELECT t3.id FROM t2"` 同形。属静默错误结果面，修复需动投影列解析路径（本 change Forbidden）。
  ② **列缺失被建模为 `ParseError`，PG 分类落到 42601**——`SELECT id FROM t WHERE nosuch = 1` 与 `SELECT abs(nosuch) FROM t` 的列缺失走 `src/parser/planner/expression.rs:284/341` 的 `PlanError::ParseError`，绕开 `ColumnNotFound`（42703）。本 change 按 R4/Preserve 未改文案故未改分类（见 Deviations 4）。证据：同上实测输出 + `planerror_sqlstate_mapping` 单测的变体映射。
  ③ 投影静默丢弃与列缺失分类两项同源于「投影/谓词列解析的错误建模」，建议合并为一条 Issue 交由 Recorder 评估。

**Remaining Issues**

- Critical / Important：无。
- Minor：见 Self-Review 1-4（1-3 已修，4 为未复现的既有信号用例竞态，不落账）。

## Plan Review

- Review Result: accepted

**Findings**

独立审查（未暂存 diff 41 文件全量阅读 + Act 声明逐条 grep/代码核对 + delta specs 场景映射核对），非阻塞结论如下；无阻塞 Acceptance 的发现。

- **diff 一致性（Plan 独立阅读）**：本 Cycle 变更面恰为未暂存的 41 个代码文件（产品 14：`src/parser/error.rs`、`src/parser/ast.rs`、planner 6 文件、`src/pipeline.rs`、`src/network/{protocol,pg_protocol,handler}.rs`、`src/cli/{mod,lifecycle}.rs`；测试 27）+ Act Response 所在的 Iteration 文件本身。产品侧逐文件核对与 Act Implemented 段一致；测试侧其余 20 文件 diff 全部为纯机械 `Response::Error { message, .. }` 模式适配（字段新增的编译强制后果），无断言内容改动。
- **零载荷残余清零（grep 全量）**：`PlanError::UnsupportedStatement/UnsupportedExpression` 不带载荷的构造形态在 `src/` 全量 0 处；`src/` 内 29 处 `Response::Error {` 构造全部携带 `sqlstate`、12 处模式匹配全部带 `..`——编译穷尽保护成立，无遗漏。
- **sqlstate 映射完备性**：`PlanError` 枚举恰 27 变体，`planerror_sqlstate_mapping` 单测 27 条目全枚举（13 分类 + 14 兜底），与 design D3 映射表逐条一致；`pg_messages_test.rs:158`（NetworkError→58000）零修改（不在 diff 文件集）。
- **文案与框架保持（R4）**：既有 7 处 `contains("Unsupported statement type"|"Unsupported expression")` 断言零修改通过（diff 中仅新增用例含 contains 行）；parse 双前缀 quirk、表缺失/事务语句/CHECK/FK/IN 子查询 JOIN 文案逐字节（`explicit_rejection_messages_unchanged` 以 `assert_eq!` 锁定）；`Plan error: ` 前缀经 `PlanFailure::From` 保持；CLI 退出码矩阵全量零校准。
- **测试映射（RTM）**：planner_test +4、cli_test +2、pg_integration_test +3（最小 `PgClient` 夹具，无新增依赖、无产品代码）、tx_statement_test +2、error.rs lib +2、protocol.rs serde +2，与 delta specs R2（3 场景）/R3（4 场景）/R4（2 场景）/R5（MODIFIED 2 组合场景）逐一对应；T3 两用例与 `sql-transaction-statements` MODIFIED R2 的 S9/S10 场景断言逐条吻合（exit 3、点名子句、`not committed` 后缀、重开无残留）。
- **偏差分类（Act 偏差 1-9）**：1（sqlstate 提前落地）非实质顺序；2（动态/静态能力名两类）为 Plan 授权「措辞非实质」；3（PG 夹具新建）判为 Plan 事实误设——Plan Risks 假设「基建已解析错误消息」不成立（基线只有启动握手用例），Act 补最小夹具且见证目标不变，正确处置；4（42703 取 JOIN ON 路径）为计划未指明形态下的实测修正——投影/WHERE 未知列经既有代码构造为 `ParseError`（文案 `Column 'x' not found`），按 R4/Preserve 禁改文案故分类维持 42601，修正路径选择正确且根因已报 Issue 候选；5（`register_table` 纳入类型化通道）为签名通道的必要连带，分类码取自映射表非硬编码；6（44 处模式适配）编译强制、独立清点零遗漏；7（multi_statement 维持 None）正确——不在映射表不自造分类；8（rustfmt 枚举展开）纯格式；9（fmt 既有偏差不动）Surgical Changes 正确应用，集合与 Iteration 000 Deviations 6 一致。全部非实质、有记录。
- **事后编辑的采信链完整**：全量 1261/0/2 之后的两处编辑（`pg_integration_test.rs` `Response`→`ErrorFields` 重命名、`planner/mod.rs` 导入路径简化）各自影响面均有编辑后复跑（pg_integration 4 passed；planner_test 76 + cli_test 108 + clippy exit 0），其余二进制覆盖未变化——按公共规则 › 验证 采信成立。
- **Minor 1-4（Act 自检）复核同意**：1-3 已修且终态正确（`plan_stage` 直连 `?` 于 pipeline.rs:105 确认；`ErrorFields` 命名落位；触碰文件 fmt 干净）；4（`test_sigint_during_open_130` 首跑失败）与 Iteration 000 Minor 3 同族同文件，各一次、复核均通过、未复现，按偶发复核条款处理正确。
- **Minor 5（计数簿记，Plan 遗漏）**：design D2 与 Plan Context 的构造点聚合计数（31 = 15+16，expression.rs ×10）与实际不符——实际 29 = UnsupportedStatement 15（ast ×9 + mod ×2 + query ×1 + ddl_dml ×3）+ UnsupportedExpression 14（expression ×8 + aggregate ×4 + subquery ×1 + ddl_dml ×1）；两文档同段落中的逐文件行号枚举清单准确，仅聚合数偏大 2，Act Self-Review 沿用「31」。覆盖完整性不受影响（零载荷残余 0），不阻塞。
- **Minor 6（簿记，Act 偏差）**：Act Response 汇总行「产品 13 + 测试 28」与实际「产品 14 + 测试 27」互异（Act 自列产品清单含 14 文件），总数 41 正确。纯报告文字，不阻塞。
- **Minor 7（措辞，Plan 遗漏）**：tasks.md Iteration 001 稳定基线句「JSON 协议输出字节不变」较 design D3 的精确语义（None 时逐字节不变；parse/plan 错误携带 `Some` 时字段出现）更宽——design 为该选择的权威记录，serde 两分支单测与 D3 一致，delta spec 未作无条件 JSON 承诺。随 change 归档，无需动作。
- **工作区观察（非本 Cycle 变更面，无动作）**：工作区存在未暂存的 `.agents/memory/project-model.md` M19 登记（全量测试串行执行约束，引用 2026-09-26 用户指令与两 Cycle 串行全量结论）——不在 Act 声明面内，内容与两 Cycle 实际执行一致；本 Review 未运行全量（采信既有新鲜结论，且受该约束），不涉及。

**Acceptance Gaps**

None.（R2 三场景、R3 四场景、R4 两场景、R5 两组合场景逐一有独立核对或注明来源的通过证据；全量 1261/0/2；JSON 兼容与 PG 分类各有直接见证；T3 零产品代码改动经 diff 证实——tx_statement_test 之外仅 `..` 机械适配。）

**Evidence**

- 独立 diff 阅读：`git diff`（未暂存 41 代码文件全量）+ `git status`（区分既有暂存迁移与本 Cycle 面）。
- grep 清点（本 Review 实测）：`PlanError::Unsupported*` 零载荷残余 0；`Response::Error` 构造 29 处全部携带 `sqlstate`、模式 12 处全部带 `..`；既有 contains 断言 7 处零修改；`PlanError` 变体 27 = 映射测试条目 27。
- 采信 Act 未失效结论（来源：Act Response Verification ①-⑨ 及其采信说明，覆盖范围与当前代码一致）：T2a/T2b RED→GREEN、全量 `cargo test -- --test-threads=1` 1261/0/2、clippy exit 0、触碰文件 fmt 干净、`openspec validate --strict` 通过、真二进制 CLI 探针（能力名/双前缀/表缺失/SAVEPOINT 文案）。
- 本 Review 未重复运行任何已通过验证（无公共规则 › 验证 列出的重跑情形；受 M19 串行约束，全量不重跑）。

**Follow-up Decision**

`accepted`：T2/T3 全部 Acceptance 满足，偏差与 Minor（5/6/7 为记录性）均非阻塞，无需当前 Cycle 修复或 rework。Iteration 001 完成；本 Iteration 为 Map 最后一个，change 实施面全部交付，可进入收尾流程。

**Iteration Plan Update**

None（Map 保持不变，无剩余 Iteration）。

**Next Cycle**

None。

**Next Iteration**

None。（change `2026-09-26-ms19-cli-admin-hardening` 两个 Iteration 均 accepted；收尾由 openspec-docs-maintainer 按用户指令执行——合并增量规格、同步 tasks/SNAPSHOT、归档 carrier。Experience Candidates 中的 Runbook 候选与 Issue 候选 ①②（建议合并）由 Recorder 按用户指令处置。）
