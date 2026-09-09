# Iteration 000 / Cycle 000-initial: 多语句分片逐条执行与静默截断收口

## Plan Context

- Status: ready（2026-09-08 用户批准 Gate 2 并指令开始实施："更改gate状态，开始实施"）
- Iteration: 000-initial
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3, T4
- Depends on: None
- Stable baseline: CLI 多语句逐条执行+顺序渲染+exit 0；fail-fast 部分生效+序号定位；lib 两路径显式拒绝；单语句零变化（665 基线零回归）
- Verification boundary: `cargo test --all` 全绿（仅 `test_multi_statement_rejected` 重写）；clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: `src/cli/mod.rs`、`src/pipeline.rs`、`tests/cli_test.rs`
- Deferred tasks: None（change 无后续 Iteration）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 全部 What Changes；用户决策 ①CLI 逐条+lib 显式拒绝 ②顺序渲染每条 ③序号+语句文本定位 ④逐条 auto-commit（2026-09-08）
- Excluded scope: SQL 事务语句（MS11-T02）、整脚本隐式事务、网络多结果协议、`execute_in_tx` 多语句支持、真行号预扫描、REPL/T05、`render.rs` 修改、新依赖

**Objective**

`rtsql db "stmt1; stmt2; ..."` 逐条执行并顺序渲染（exit 0）；任一条失败立即停止且错误含序号/语句文本/已生效注明（exit 3）；网络路径与 `execute_in_tx` 遇多语句返回显式 `Response::Error`；单语句行为零变化。

**Background**

lib 层 `statements.first()` 静默截断（网络路径 + 显式事务路径）与 CLI T01 临时护栏（exit 3 拒绝）是同一缺陷的两面（R18 主题 2/3「多语句静默截断——直接脚枪」）。MS10-T04 落地分片执行后护栏退役（spec R5 原文承诺）。

**Current Baseline**

- revision `5c42ec8`（master，工作树干净）
- 基线（2026-09-08 实测）：`cargo test --lib` → 187 passed / 0 failed（exit 0）；`cargo test --test cli_test` → 21 passed / 0 failed / 2 ignored（exit 0）
- CLI 护栏：`run_sql` 对多语句 exit 3（`tests/cli_test.rs::test_multi_statement_rejected` 当前锁定该拒绝语义，本 change 重写）

**Current-State Evidence**

- 调用链：`main.rs` → `cli::run`（`cli/mod.rs:88`）→ `execute_command_inner`（`:162`，两阶段信号 select，work 闭包 = `run_sql`）→ `run_sql(db, sql, format)`（`:198`）
- `run_sql` 现状：`parse_stage(sql)`（`:199`）→ 护栏 `len>1` 返回 `ExitStatus::Sql("one statement at a time: got {} statements; ...")`（`:203-208`）→ `plan_stage(db, sql, &statements[0], false)`（`:210`，完整串作缓存键）→ `columns = PlanBuilder::new().get_plan_output_columns(&plan)`（`:214`）→ `execute_stage(db, plan, false)`（`:216`）→ Response 分支渲染（`:217-221`：QueryResult → `emit(kind, &columns, Rows)`；AffectedRows → `emit(kind, &[], Affected)`；Error → `ExitStatus::Sql(message)`；Pong → Success）
- `emit`（`:224`）= `emit_stdout(render(kind, columns, payload))`，渲染文本追加 `\n` 写 stdout；写失败 → `ExitStatus::General`
- pipeline：`parse_stage`（`pipeline.rs:42-48`）= `parse_sql`（GenericDialect）→ `Vec<Statement>`，空 → `Err("Empty SQL")`；`plan_stage`（`:56-86`）DDL 直接 build，否则 register_table → build_plan → `is_cacheable`（仅 `Statement::Query`，`:986`）时 `plan_cache.put(sql.to_string(), plan)`——`sql` 实参即缓存键；`execute_stage`（`:96-225`）DDL → executor + `plan_cache.clear()`；DML → begin → abort_tables 预取 → executor(Some(tx_id)) → commit/abort（逐条 auto-commit）；查询 → executor(None)
- 截断点：`execute_inner` `statements.first()`（`pipeline.rs:341`）；`execute_in_tx` `statements.first()`（`:237`）。`execute_inner` 的 cache get 在 parse 之前（`:307`，完整串键）；网络路径 `Database::execute_sql`（`database.rs:108`）← `network/handler.rs:27`
- 渲染形状（`cli/render.rs:24-37`）：JSON 行集 `{"columns":[...],"rows":[...]}`；DML `{"affected_rows":N}`；table/csv/tsv 由 `render_rows` 承载
- sqlparser 0.44（已核实源码）：`Parser::parse_sql` 全串解析、按 `SemiColon` token 分片、跳过空语句、尾随分号合法、字符串字面量内 `;` 不分片（`sqlparser-0.44.0/src/parser/mod.rs:401-424`）；错误文本自带 ` at Line: X, Column Y`（`parser_err!` + `Location` Display，`parser/mod.rs:46-49`）；`Statement: Display` 提供 canonical 文本
- `Response::Pong` 仅网络 `Request::Ping` 产生（`network/handler.rs:19`），pipeline 不可达
- 测试入口：`tests/cli_test.rs`（`run_cli`/`fixture`/`seed_users` 夹具；`test_multi_statement_rejected` 在 `:218`）；pipeline stage 单测在 `src/pipeline.rs:996-1159`（8 个，均显式单语句、不断言缓存键文本、不依赖截断——本 change 零修改）；`execute_in_tx` 全部测试调用方传单语句（wal_recovery_large/btree_scale/cli_test/explicit_tx 等，grep 核实）
- 协议测试无多语句依赖（grep `tests/pg_*|network_*` 零命中）

**Relevant Code**

- `src/cli/mod.rs` — CLI 编排：`run_sql`（分片循环宿主）、`emit`/`emit_stdout`（逐段写出）、`ExitStatus`（退出码分类）
- `src/pipeline.rs` — `parse_stage`/`plan_stage`/`execute_stage`（pub，循环直接复用）；`execute_inner`/`execute_in_tx`（显式拒绝宿主）
- `src/cli/render.rs` — 纯函数渲染（零修改，只复用）
- `tests/cli_test.rs` — CLI 集成测试（重写 1 + 新增 5）

**Critical Path**

CLI：`run_sql` 循环体 = `plan_stage(stmt_sql, stmt)` → `get_plan_output_columns` → `execute_stage` → Response 分支渲染写 stdout；任一 Err/Error → 组装定位模板 → `ExitStatus::Sql` → `execute_command_inner` 照常 `close()`。数据流：每条语句独立事务（auto-commit），失败前语句已持久。状态变化：plan_cache 以逐条 canonical 文本为键；DDL 逐条后 clear（既有语义逐条成立）。

**Implementation Guidance**

建议顺序：T1（循环 + 渲染 + 换键，含护栏移除）→ T2（错误分支模板）→ T3（pipeline 两处显式拒绝）→ T4（全量门）。T1 重构时保持错误分支先透传原消息（T2 再接手组装），保证每步可独立验证。`stmt_text` 用 `stmt.to_string()`，`>200` 字符截断加 `...`。定位模板：`statement {k} of {n} failed: {error}; statement: {stmt_text}`，k>1 追加 `; previous statement(s) were committed`。parse 错误分支保持现状透传（不套模板）。json 逐条独立文档由「每条渲染 + emit_stdout 追加换行」自然形成，无需改 render。

**Behavioral Change**

| 场景 | 当前 | 目标 |
|---|---|---|
| CLI 多语句 | exit 3 拒绝，零执行 | 逐条执行、逐段渲染、exit 0 |
| CLI 中间失败 | —（整体拒绝） | 前 k-1 条已生效；exit 3 + `statement k of n failed: ...; statement: <text>`（+已生效注明） |
| CLI 语法错误 | 整体拒绝（parse 全串） | 不变（零执行、行列文本） |
| `execute_sql`/`execute_in_tx` 多语句 | 静默执行第一条 | `Response::Error`（显式拒绝，零执行，put 之前） |
| 单语句（含尾分号） | 正常 | 零变化 |

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T1 | R-分片/S1,S2,S5 | `src/cli/mod.rs::run_sql` | 护栏拒绝 + 单语句执行 | 移除护栏；逐条循环（plan/execute/渲染/写出）；键换 `stmt.to_string()` |
| T2 | R-分片/S3,S4 | `src/cli/mod.rs::run_sql` | 错误透传 | 序号/语句文本/已生效注明模板；parse 分支保持 |
| T3 | R-分片/S6 | `src/pipeline.rs::execute_inner`(:341)、`execute_in_tx`(:237) | first() 静默截断 | len>1 → `Response::Error`（put 之前） |
| T4 | R1 既有场景回归 | 全仓（只读） | 665 基线 | 全量门 + clippy/fmt/validate |

**Task Contracts**

### T1: CLI 多语句分片逐条执行与顺序渲染

- Requirement/Scenario: R-多语句分片逐条执行 / S1、S2、S5
- Depends on: None
- Targets: `src/cli/mod.rs::run_sql`
- Current behavior: `len>1` exit 3 拒绝；单语句执行 + 单段渲染
- Required behavior: 每条语句（总数 n）独立 plan（键 = `stmt.to_string()`）→ execute → 渲染 → `emit_stdout` 逐段写出；全部成功 → `ExitStatus::Success`
- Required changes: 护栏块（`:203-208`）移除；单语句执行体泛化为循环；缓存键换逐条 canonical 文本
- Preserve: `render.rs`/`kind()`/`emit_stdout`/信号机制零修改；单语句可观察行为（输出与退出码）零变化；`Pong` 分支保持（Success 无输出）
- Forbidden: 不做整体事务；不改 stage 签名与 plan_cache 逻辑
- Test witness: RED——`tests/cli_test.rs` 重写 `test_multi_statement_rejected` → `test_multi_statement_executes`（双 INSERT：exit 0、stdout 两段受影响行输出、重开查询两行）；新增 `test_multi_statement_sequential_render`（`--format json` 下 INSERT+SELECT → 两个独立 JSON 文档：`{"affected_rows":1}` 后随 `{"columns":..,"rows":..}`）；新增 `test_multi_statement_semicolon_boundaries`（`SELECT 1;; SELECT 'a;b';` → 两语句、exit 0）。先跑 `cargo test --test cli_test` 记录 RED
- GREEN condition: 3 用例绿；既有其余 20 用例零修改绿
- Verification: `cargo test --test cli_test`（exit 0）
- Stop when: 循环语义与 `Response` 单结果契约实质冲突，或逐条键引发缓存行为异常

### T2: fail-fast 错误定位与部分生效语义

- Requirement/Scenario: R-多语句分片逐条执行 / S3、S4
- Depends on: T1
- Targets: `src/cli/mod.rs::run_sql`（错误分支）
- Current behavior: 语句错误透传消息（`ExitStatus::Sql(message)`），无序号定位
- Required behavior: 执行/计划错误 → `statement {k} of {n} failed: {error}; statement: {stmt_text}`（≤200 字符截断）+ k>1 追加 `; previous statement(s) were committed`，exit 3；失败后语句不执行；parse 错误分支保持现状（零执行 + `Parse error:` + 行列文本）
- Required changes: 循环错误分支组装模板
- Preserve: exit 3 语义；parse 分支文案形态；零执行语义
- Forbidden: 不做 token 预扫描/行号计算；不改 parse_stage
- Test witness: RED——新增 `test_multi_statement_fail_fast`（`INSERT INTO t VALUES (1); INSERT INTO missing_table VALUES (2); INSERT INTO t VALUES (3)` → exit 3、stderr 含 `statement 2 of 3` 与 `missing_table`、重开仅 `id=1` 生效）；新增 `test_multi_statement_parse_error_zero_exec`（`INSERT INTO t VALUES (1); SELEC typo` → exit 3、stderr 含 `Line:`、表仍空）。跑 `cargo test --test cli_test` 记录 RED
- GREEN condition: 2 用例绿；T1 用例不回退
- Verification: `cargo test --test cli_test`（exit 0）
- Stop when: 定位内容与 spec S3 契约冲突

### T3: lib 单结果路径显式拒绝多语句

- Requirement/Scenario: R-多语句分片逐条执行 / S6
- Depends on: None
- Targets: `src/pipeline.rs::execute_inner`（`:341`）、`execute_in_tx`（`:237`）
- Current behavior: `statements.first()` 静默截断；网络路径以完整串作键 put 首条 plan
- Required behavior: `len>1` → `Response::Error`（文案说明该路径仅支持单语句、多语句请用 CLI 分片），零执行，发生在 plan_stage/cache put 之前；单语句路径零变化
- Required changes: 两处 first() 消费前加显式拒绝分支
- Preserve: 8 个 pipeline stage 单测零修改；单语句网络/事务行为与 cache get 顺序零变化；`Response` 枚举不变
- Forbidden: 不扩 `Response`；不做网络多结果；不改 `execute_in_tx` 单语句语义
- Test witness: RED——`src/pipeline.rs` tests 模块（复用 `open_db_with_table`）新增 `execute_sql_multi_statement_returns_error`（`db.execute_sql("SELECT 1; SELECT 2")` → `Response::Error`）与 `execute_in_tx_multi_statement_returns_error`（多语句 → `Response::Error`，且事务未终结：随后单语句 `execute_in_tx` 正常执行 + commit 成功）。跑 `cargo test --lib` 记录 RED
- GREEN condition: 2 单测绿；lib 187 基线零回归
- Verification: `cargo test --lib`（exit 0）
- Stop when: 显式拒绝与既有调用方语义实质冲突

### T4: 回归门与全量验证

- Requirement/Scenario: R1 既有场景回归 + change 验证边界
- Depends on: T1, T2, T3
- Targets: 全仓（只读验证）
- Current behavior: 基线 lib 187/0、cli_test 21/0/2（2026-09-08）
- Required behavior: `cargo test --all` 全绿（唯一重写：`test_multi_statement_rejected`）；clippy/fmt/validate 全 0/PASS
- Required changes: 无代码改动
- Preserve: 既有测试断言语义零修改
- Forbidden: 不放宽断言换通过
- Test witness: 变更前基线已留档（本文件 Current Baseline）
- GREEN condition: 全量门通过
- Verification: `cargo test --all && cargo clippy --all-targets -- -D warnings && cargo fmt --check && openspec validate 2026-09-08-ms10-t04-multi-statement-execution`
- Stop when: 计划外既有测试破坏（→ Blocker Handoff，不得静默改断言）

**Invariants**

- `parse_stage`/`plan_stage`/`execute_stage`/`Response`/`ExitStatus`/`render()` 签名与语义不变；不新增依赖与模块。
- 单语句执行（含尾分号）、网络与事务路径单语句行为零变化。
- 信号两阶段停机不变：循环被信号取消时已提交语句保留、`close()` checkpoint 照常执行。
- `plan_cache::normalize_sql_key` 与容量/淘汰逻辑不变。

**Non-goals**

SQL 事务语句（MS11-T02）；整脚本隐式事务；网络多结果协议；`execute_in_tx` 多语句支持；真行号预扫描；REPL/T05；`render.rs` 修改。

**Acceptance**

- S1/S2/S5：`test_multi_statement_executes`、`test_multi_statement_sequential_render`、`test_multi_statement_semicolon_boundaries` GREEN。
- S3/S4：`test_multi_statement_fail_fast`、`test_multi_statement_parse_error_zero_exec` GREEN。
- S6：`execute_sql_multi_statement_returns_error`、`execute_in_tx_multi_statement_returns_error` GREEN。
- 回归：`cargo test --all` 全绿（除重写用例零修改）；clippy/fmt/validate 全 0/PASS。
- 映射链：proposal → delta spec（R1 修正 + R-分片 6 场景）→ design D1-D8 → T1-T4 → 上述测试。

**Verification**

- `cargo test --test cli_test`（T1/T2 面）、`cargo test --lib`（T3 面）、`cargo test --all && cargo clippy --all-targets -- -D warnings && cargo fmt --check && openspec validate 2026-09-08-ms10-t04-multi-statement-execution`（T4 门）。
- 基线对照：lib 187/0、cli_test 21/0/2 → 目标 lib 189+/0（+2 单测）、cli_test 26/0/2（-1 重写 +6 新增）、全量 671/0/2。

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | Current-State Evidence（file:line 全链 + sqlparser 源码核实 + 基线实测 187/0、21/0/2） |
| Design | PASS | design.md D1-D8（层级/渲染/定位/事务/键/拒绝/见证/解析器事实） |
| Iteration Plan | PASS | tasks.md 单 Iteration + 平衡审计（单一验收域、约百行级、同诊断域） |
| Cycle Scope | PASS | initial，Acceptance gaps None，Excluded scope 明确 |
| Task Contracts | PASS | T1-T4 含 Current/Required/Preserve/Forbidden/Witness/Stop；Act 只读本 Cycle 即可建立测试见证 |
| Traceability | PASS | RTM 7 行全 Covered（唯一简化已获用户批准并登记） |
| Verification | PASS | 见证命令 + GREEN 条件 + 基线对照数值 |

**Persisted Evidence**

- Mode: none

所有验证由命令输出 + Act Response 承载（命令、≤20 行决定性输出、退出码）；无不可复现结果，无一次性环境。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB；超限需要用户明确批准。

**Risks and Notes**

- `Statement::to_string()` Display 保真度未逐语法验证——非实质（键仅作进程内缓存身份，同 AST 必产同 plan；不同写法归并同键属期望行为）。
- table/csv/tsv 多段拼接使整段 stdout 非单一文档——机器消费以 json/JSONL 为准（proposal Impact 已注明）。
- sqlparser 个别非 `expected` 系错误可能无行列文本——测试只对典型语法错误断言 `Line:`。
- 错误文案具体措辞为非实质项；序号、语句文本、已生效注明三项内容为契约（S3）。

## Act Response

- Status: reported（2026-09-08，Cycle 000-initial 全部 task 完成，Gate 3-5 + 完整 diff Review 通过）

**Implemented**

- **T1**：`run_sql` 多语句护栏移除，改为分片逐条循环——每条语句独立 `plan_stage`（缓存键 = 该语句 canonical 文本 `stmt.to_string()`，D5）→ `get_plan_output_columns` → `execute_stage`（逐条 auto-commit，D4）→ 按 Response 分支 `render` + `emit_stdout` 顺序写出（D2）；全部成功 → `ExitStatus::Success`；孤儿 `emit()` 随改写清理。
- **T2**：新增 `sql_failure_status` 定位模板——执行/计划错误 → `statement {k} of {n} failed: {error}; statement: {stmt_text}`（200 字符 chars 截断加 `...`），k>1 追加 `; previous statement(s) were committed`（D3）；parse 错误分支保持透传（零执行 + 行列文本）。
- **T3**：`execute_inner` 与 `execute_in_tx` 在 `parse_stage` 成功后、`plan_stage`/cache put 之前对 `len > 1` 返回 `Response::Error`（D6），文案经共享私有 `multi_statement_rejected(count)` 产出。
- **T4**：全量门通过（见 Verification Evidence）。
- delta spec S5 场景按用户批准修订（no-FROM → 含 FROM 等价见证，见 Blocker Resolution）。

**Changed Files and Symbols**

- `src/cli/mod.rs`：`run_sql`（循环化）；新增 `sql_failure_status`；删除 `emit`。
- `src/pipeline.rs`：`execute_in_tx`、`execute_inner`（显式拒绝分支）；新增私有 `multi_statement_rejected`。
- `tests/cli_test.rs`：重写 `test_multi_statement_rejected` → `test_multi_statement_executes`（S1）；新增 `test_multi_statement_sequential_render`（S2）、`test_multi_statement_semicolon_boundaries`（S5）、`test_multi_statement_fail_fast`（S3）、`test_multi_statement_parse_error_zero_exec`（S4）。
- `tests/file_header_test.rs`：2 处 `repeat().take()` → `repeat_n()`（基线 clippy 债务，见 Deviations 3）。
- `openspec/changes/2026-09-08-ms10-t04-multi-statement-execution/specs/cli-noninteractive-shell/spec.md`：S5 场景 GIVEN/WHEN 修订（用户批准）。

**Deviations from Plan**

1. **S5 见证 SQL 修订（用户批准，2026-09-08）**：`SELECT 1;; SELECT 'a;b';` 因引擎 plan 阶段要求 FROM 子句不可达 exit 0（实测 `Plan error: Missing required field: FROM clause`，exit 3）→ spec 场景与见证改为含 FROM 等价 SQL；三个分号边界语义验证力不变。全记录见 Blocker Handoff/Resolution。
2. **sequential_render 用例 SELECT 改两列投影**：全表扫描子集单列投影的 columns 表头返回全 schema（既有单语句行为，实施中实测 `SELECT id FROM users` → `{"columns":["id","name"],"rows":[[1]]}`）；两列形式与既有单语句渲染语义逐字一致，S2 契约（两个独立文档/顺序/形状）验证力不变，避免在多语句用例锁死表头形状。
3. **file_header_test 2 处机械 lint 修复（超出原 Change Surface）**：T4 clippy 门（`-D warnings`）被基线债务阻塞——`tests/file_header_test.rs`（MS10-T03 遗留、本 change 计划外文件）2 处 `manual_repeat_n` 错误；修复为 `repeat_n` 等价机械替换，语义零变化，file_header_test 14 用例零断言修改。
4. **pipeline.rs 新增私有 `multi_statement_rejected` helper**：Self-Review（Gate 4 code quality）发现两处拒绝分支 5 行文案重复，合并为共享函数。
5. **计划数值勘误（非实质）**：Iteration Plan 目标「cli_test 26」应为 25（重写净 0 + 新增 4；与「全量 671 = 665 + 6」自洽，其中 2 为 lib 用例）；实测 25，其余数值（lib 189、全量 671/0/2）与计划一致。

**Blocker Handoff**

- **发现位置**：Task T1 / Test witness（Gate 3）建立前核实；Act Response 置 blocked，终止实施。
- **Plan 预期**：T1 见证 `test_multi_statement_semicolon_boundaries` 以 `SELECT 1;; SELECT 'a;b';` 达成两语句 exit 0（delta spec `cli-noninteractive-shell` S5 场景同文本；design D7 同口径）。
- **实际情况**：本引擎 plan 阶段要求 FROM 子句。实测（2026-09-08，revision 5c42ec8 工作树）：`./target/debug/rtsql /tmp/rtsql-t04-probe.db "SELECT 1"` → stderr `Plan error: Missing required field: FROM clause`，退出码 3。与 `tests/cli_test.rs:481-482` 既有 T02 会话注释（「契约草拟的 `SELECT 1` 无 FROM 子句在 plan 阶段即报错」）一致。分片实现落地后，该 SQL 将产出 `statement 1 of 2 failed: Plan error: Missing required field: FROM clause; statement: SELECT 1` 退出码 3——见证按契约字面永远无法 GREEN。
- **影响**：S5 场景（分号边界语义）按字面不可达。修复路径需要 planner 支持 no-FROM SELECT——新引擎能力，超出本 change Change Surface（`src/cli/mod.rs::run_sql` + `src/pipeline.rs` 两处拒绝点）与 Non-goals。其余见证（S1/S2/S3/S4/S6 全部含 FROM 语句）不受影响；T1 其余部分、T2、T3、T4 均可独立执行。
- **任务状态**：T1/T2/T3/T4 全部未开始（零代码/测试修改）；已通过 Gate：Gate 1、Gate 2（用户 2026-09-08 批准）；Gate 3 起阻塞。
- **恢复条件**（三选一）：① 修订 delta spec S5 场景 GIVEN/WHEN 为含 FROM 的等价 SQL（连续分号、字面量分号、尾随分号三个边界语义不变）；② 扩范围支持 no-FROM SELECT（需返回 openspec-plan replan，超出 T04）；③ 用户显式豁免：以含 FROM 等价 SQL 建立见证，spec 文本修订留 Plan Review（须保留豁免原话与风险）。
- **Evidence**：None required（单命令可复现，决定性输出已录于上，≤20 行）。

**Blocker Resolution**

- **用户指令**（2026-09-08，AskUserQuestion 应答原文选项）：选择「修订 S5 场景 SQL（推荐）」——把 delta spec S5 的 WHEN 改为等价含 FROM 语句，GIVEN 补种子行；连续分号/字符串内分号/尾随分号三个边界语义全部保留，测试同形；修订经用户当场批准记录于此。
- **解决办法**：delta spec `specs/cli-noninteractive-shell/spec.md` S5 场景修订为 GIVEN `users(id INT PRIMARY KEY, name STRING)` 含一行（`id=1`，`name='Alice'`）、WHEN `SELECT id FROM users;; SELECT name FROM users WHERE name = 'a;b';`、THEN 两语句 exit 0 且第一查询 1 行、第二查询空行集。T1 见证 `test_multi_statement_semicolon_boundaries` 按修订后场景建立。
- **风险**：S5 与原字面 SQL 的差异仅为语句形状（no-FROM → 含 FROM）；分号边界验证力等价。no-FROM SELECT 仍不受支持（既有引擎行为，本 change 未承诺改变）。
- **恢复点**：从 T1 测试见证建立继续（T1/T2/T3/T4 原契约其余部分不变）。
- **所需验证**：不变（T1/T2 `cargo test --test cli_test`；T3 `cargo test --lib`；T4 全量门）。

**Self-Review**

- **Spec compliance（逐 task）**：S1-S6 六场景见证齐备且 GREEN；RTM 7 行全部有对应 GREEN 测试；Invariants（stage/`Response`/`ExitStatus`/`render` 签名语义不变、单语句含尾分号零变化、信号两阶段停机不变、`normalize_sql_key` 不变、零新增依赖模块）逐项核对无违反；Forbidden（整体事务包裹、token 预扫描、网络多结果、`render.rs` 修改、放宽断言）未触碰；既有 cli_test 其余 20 用例与 pipeline 8 个 stage 单测零修改通过。
- **Code quality（完整 diff）**：无未登记的计划外修改（2 项计划外触碰均已登记：spec S5 修订经批准、file_header_test 基线 lint）；错误路径（stdout 写失败 → `General` 并停止、逐条 auto-commit 沿 `execute_stage` 既有语义、事务路径拒绝后事务保持存活）；边界（200 字符按 `chars` 截断 UTF-8 安全；`;;`/尾分号/字符串内分号经 sqlparser 语义 + 用例锁定）；测试不因错误原因通过（T1/T2/T3 均先观察 RED——lib 用例 RED 时分别捕获「静默截断执行第一条」与「首条无 FROM 的误导性 plan 错误」两种形态）。
- **已修复发现**：两处拒绝文案重复 → `multi_statement_rejected` helper（修复后重跑全门）。
- **遗留 Minor**：`kind(format)` 在循环内每条语句重算（`is_terminal()` 每语句一次，语义无影响、开销可忽略）——未处理，不阻塞。
- **e2e 抽查**（Gate 5 补充，真实二进制）：`INSERT;INSERT;SELECT` → 两段 affected + rows 文档 exit 0；fail-fast → `statement 2 of 3 failed: Table 'missing' not found: table not found: missing; statement: INSERT INTO missing VALUES (4); previous statement(s) were committed` exit 3。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| T1/T2 测试 | `cargo test --test cli_test` | `test result: ok. 25 passed; 0 failed; 2 ignored` | PASS |
| T3 测试 | `cargo test --lib` | `test result: ok. 189 passed; 0 failed` | PASS |
| T4 全量 | `cargo test --all` | 合计 `671 passed`、`FAILED` 计数 0（+2 ignored 既有设计项） | PASS |
| T4 clippy | `cargo clippy --all-targets -- -D warnings` | `Finished`（exit 0） | PASS |
| T4 fmt | `cargo fmt --check` | exit 0（无 diff） | PASS |
| T4 validate | `openspec validate 2026-09-08-ms10-t04-multi-statement-execution` | `Change '...' is valid` | PASS |

以上为最终代码状态（含 helper 重构）的新鲜运行；四门以单条链式命令执行，退出码 `test=0 clippy=0 fmt=0 validate=0`。

**Persisted Evidence**

None required（Plan Mode: none；全部验证可复现，命令与决定性输出已录于上）

**Experience Candidates**

None

**Remaining Issues**

- 全表扫描子集单列投影的 columns 表头返回全 schema（既有行为，非本 change 引入；`SELECT id FROM t` 表头 `["id","name"]` 而 rows 为真投影）——是否修正属独立引擎议题，供 Plan Review / 用户决定（若修正需同步校准 `test_multi_statement_sequential_render` 注释）。
- no-FROM SELECT（`SELECT 1`）不受支持为既有能力边界（S5 修订已绕开）；是否立项由路线规划决定。
- `tests/file_header_test.rs` 的 clippy 基线债务已顺手清偿（源文件非归档 carrier，MS10-T03 carrier 无需变动）。

**Commit or Diff Reference**

未 commit——工作树含 4 个源/测试文件改动 + change 文档（含 delta spec S5 修订）；等待用户审计与 Plan Review 后由后续流程收尾。基线 HEAD `5c42ec8`。

## Plan Review

- Review Result: accepted（2026-09-09，Plan 独立检查与复跑全过；findings 全部非阻塞，见 Findings/Follow-up Decision）

**Findings**

（独立检查日期 2026-09-09；全部非阻塞）

1. **ACT-DEVIATION（已经阻塞流程正当处置）**：S5 见证 SQL 修订——原契约 `SELECT 1;; SELECT 'a;b';` 因引擎 plan 阶段要求 FROM 子句而按字面不可达 exit 0（Plan 独立复现确认既有行为）。Act 经 Blocker Handoff → 用户批准（「修订 S5 场景 SQL（推荐）」）→ Blocker Resolution，将 delta spec S5 场景与见证改为含 FROM 等价 SQL；三个分号边界语义（连续分号跳空语句、字符串字面量内分号不分片、尾随分号合法）全部保留。Plan 复核：spec.md:74-79 与 `test_multi_statement_semicolon_boundaries` 一致；独立探针确认 `'a;b'` 字面量不被分片（WHERE 匹配 exit 0）、空语句跳过（两段输出）。
2. **ACT-DEVIATION（非阻塞）**：`tests/file_header_test.rs` 2 处 `repeat().take()` → `repeat_n` 机械替换——计划外文件触碰，由 T4 clippy `-D warnings` 门暴露的基线债务触发；断言语义零修改（全量门独立复跑 671/0/2 确认）。符合既有先例（MS06-T02 T0 机械 clippy 收编）。
3. **ACT-DEVIATION（非实质）**：`test_multi_statement_sequential_render` 的 SELECT 采用两列投影——规避 finding 5 的既有表头行为，测试内注释已登记；S2 契约（两个独立 JSON 文档 / 顺序 / 形状）验证力不变。
4. **PLAN-OMISSION（非阻塞数值勘误）**：计划 Verification「cli_test 26/0/2（-1 重写 +6 新增）」算术错误（6 个新增含 2 个 lib 用例）；实测 cli_test 25/0/2、lib 189/0、全量 671/0/2，Act 勘误正确且自洽。
5. **NEW-EVIDENCE（既有缺口，非本 change 引入）**：裸 DataScan（无 WHERE）子集投影的 CLI 表头为全 schema——Plan 独立探针复现：`SELECT name FROM t` → `{"columns":["id","name"],"rows":[["Alice"]]}`（行真投影、表头未按 projection 裁剪），而 `SELECT name FROM t WHERE id = 1` → `["name"]` 正确。机制推断：CLI 表头经 `get_plan_output_columns`（plan 节点 columns 元数据），对裸 DataScan 节点未按 `projection` 索引裁剪；`tests/projection_test.rs`（MS10-T01）只断言 lib 行形状，未覆盖 CLI 表头——spec `cli-noninteractive-shell` R6 S1 场景「表头 ["name"]」的 bare-DataScan 分支自 MS10-T01 起未满足。非本 change Acceptance（S2 用两列投影、S5 只断言行）；收尾时登记 improvement 候选。
6. **Minor（Act 自报）**：`kind(format)` 循环内每语句重算 `is_terminal()`——开销可忽略、语义无影响，不处理。

**Deviation Classification**

ACT-DEVIATION（findings 1/2/3）+ PLAN-OMISSION（finding 4）+ NEW-EVIDENCE（finding 5）；无 PLAN-INVALID、无 BASELINE-CHANGED。

**Acceptance Gaps**

None——S1-S6 六场景见证全 GREEN（Plan 独立复跑：cli_test 25/0/2、lib 189/0）；全量 671 passed / 0 failed / 2 ignored；clippy / fmt / openspec validate 全过；RTM 7 行 Covered；Invariants / Forbidden 逐项核对无违反（`git status` 恰为 4 个登记文件 + change 文档；`render.rs` / `Cargo.toml` / `plan_cache` / 信号两阶段机制零触碰）。

**Convergence**

N/A（首次 Review）

**Evidence**

Plan 独立复跑（2026-09-09）：`cargo test --all` → 61 个测试二进制全 ok，`passed:671 failed:0 ignored:2`；`cargo clippy --all-targets -- -D warnings` exit 0；`cargo fmt --check` exit 0；`openspec validate 2026-09-08-ms10-t04-multi-statement-execution` → valid。独立 e2e 探针（真实二进制）：`INSERT;INSERT;SELECT` → 三个顺序 JSON 文档 exit 0；fail-fast → `statement 2 of 3 failed: Table 'nope' not found: table not found: nope; statement: INSERT INTO nope VALUES (4); previous statement(s) were committed` exit 3；空串 → `Empty SQL` exit 3（不变）；`SELECT name FROM t WHERE name = 'a;b'` → exit 0（字面量不被分片）。

**Follow-up Decision**

接受——全部 Acceptance 满足、无阻塞 finding。findings 处置：①③ 记录保留（S5 修订经用户批准，等效性成立）；② 已完成、无需动作；④ 以本 Review 记录为准；⑤ 连同 Act Remaining Issues 的 no-FROM SELECT 能力边界（`SELECT 1` 不受支持为既有引擎能力边界）共两项 improvement 候选，由 openspec-docs-maintainer 收尾时登记 Ixx（超出 Plan 授权范围，不自行写入）；⑥ 不处理。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None
