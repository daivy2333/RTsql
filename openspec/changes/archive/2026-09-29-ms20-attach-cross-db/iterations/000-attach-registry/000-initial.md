# Iteration 000 / Cycle 000: ATTACH 注册面与拒绝矩阵

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `sql-attach-session` R1–R3；proposal 假设 1/2/5/8/9/11；design D1/D2/D8
- Excluded scope: T2–T4（限定名路由、只读边界、CTAS）；DETACH（用户豁免）；lib 路径行为变化

**Objective**

`ATTACH '<path>' AS <alias>` 在 CLI one-shot 会话内注册附加库（`Arc<Database>`，独占锁），同次调用后续语句可消费注册表（本 Iteration 仅以注册成功/失败可观测为验收，查询路由属 Iteration 001）；拒绝矩阵（重复别名/路径缺失/锁冲突含自附加/非字面量路径/lib session-only）全绿且零副作用。

**Investigation Facts**

- Current Baseline: commit bc1d18d 工作区基线，全量 1408 tests / 0 failures / 2 ignored（2026-09-28 新鲜串行结论，覆盖范围此后无代码变化，采信）。当前 `ATTACH` 落入 catch-all：`Unsupported statement type: ATTACH`（exit 3）。
- Current-State Evidence:
  - sqlparser 0.44 原生解析 `ATTACH [DATABASE] '<expr>' AS <ident>` → `Statement::AttachDatabase { schema_name: Ident, database_file_name: Expr, database: bool }`（crate `parser/mod.rs:476,667-677`、`ast/mod.rs:2035-2042`；GenericDialect 无门控）。
  - 分类器先例：`classify_transaction_statement`（`src/parser/planner/mod.rs:46-94`，三值契约 `Ok(None)/Ok(Some)/Err(边界点名)`）；`build_plan` 前置拒绝（`mod.rs:204-213`，session-only 文案、先于 plan cache）；`PlanError::TransactionStatement(String)`（`src/parser/error.rs:61`，Display 裸文案、SQLSTATE 0A000）。
  - CLI 分派：`run_sql`（`src/cli/mod.rs:370-483`）循环前三臂分派（`mod.rs:385-473`），`TransactionSession` 创建于 `mod.rs:382`；`parse_stage` 整体解析多语句（`pipeline.rs:46-52`）；epilogue 事务回滚于 `mod.rs:475-482`；`sql_failure_status`（`mod.rs:503-525`）恒 exit 3 带 `statement k of n failed` 上下文。
  - 打开面：主库 `Database::open_with_key(db_path, IsolationLevel::RepeatableRead, key)`（`mod.rs:341`）；错误分类 `open_error_status`（`mod.rs:304-320`：DatabaseLocked→exit 4、InvalidKey→exit 5、其他→1）。`FileStorage::open_with_key` 锁先于一切（`file_storage.rs:71-77`）；**open 会静默创建 0 字节新库**（`file_storage.rs:87-116`）→ ATTACH 必须 `exists()` 前置。
  - 路径解析：`resolve_db_path`（`src/cli/resolve.rs:30-35`，裸名→`$RTSQL_HOME/db/<name>.db`、含 `/` 直开）；存在性检查先例 `resolve_existing_db`（`src/cli/lifecycle.rs:728-740`，private）。
  - 锁语义：flock 按 inode，同进程二次 open+try_lock 同文件（含 symlink/hardlink）失败（`tests/database_file_lock_test.rs:35-48` 锁定）；`Database` 无 Drop，fd 随进程退出释放。
  - 测试夹具先例：`tests/tx_statement_test.rs`（`run_cli` 158-202：`CARGO_BIN_EXE_rtsql` + `RTSQL_HOME=dir` + 60s watchdog；`cli_fixture` 205-209；lib 段 20-128 含 `plan_cache_len` 断言）。
- Code and Critical Path:
  - `src/parser/planner/mod.rs`：新增 `classify_attach_statement`（`pub(crate)`，同模块分类器先例）+ `build_plan` 前置拒绝臂；新 `PlanError` 变体或复用既有（随 Act 按 error.rs 模式裁定，文案即契约）。
  - `src/cli/mod.rs`：`run_sql` 创建 `AttachSession`（`HashMap<String, Arc<Database>>`）、三臂扩四臂、epilogue best-effort close 附加库（只读无脏页，MS18 零工作守卫保证无操作）。
  - `src/cli/resolve.rs`：复用 `resolve_db_path`；存在性检查逻辑（promote 或复用 `resolve_existing_db` 语义，等价选择留 Act）。

**Implementation Guidance**

建议顺序：分类器 + build_plan 拒绝 + planner 单元测试（RED→GREEN，lib 层先行）→ CLI 四臂分派 + AttachSession → e2e 拒绝矩阵。附加库 open 参数与主库完全同源（RR + 同一 key）。ATTACH 成功渲染 `Affected(0)` 直连（事务语句先例 `mod.rs:396-403`）。

**Task Contracts**

### T1: ATTACH 会话语句与注册面

- Requirement/Scenario: `sql-attach-session` R1（注册/裸名路径/路径缺失/锁冲突）、R2（重复别名/自附加/不进 plan cache）、R3（lib session-only）
- Depends on: None
- Targets: `src/parser/planner/mod.rs::classify_attach_statement`（新）、`src/parser/planner/mod.rs::build_plan`（前置臂）、`src/parser/error.rs::PlanError`（拒绝变体/文案）、`src/cli/mod.rs::run_sql`（AttachSession + 第四臂 + epilogue）
- Current behavior: `ATTACH 'x.db' AS o` 经 lib/CLI 均报 `Unsupported statement type: ATTACH`（exit 3）；CLI 多语句中含 ATTACH 整体可解析但逐句拒绝。
- Required behavior:
  - CLI 会话：`ATTACH '<path>' AS <alias>` 成功注册（别名小写归一化），渲染 `Affected(0)`；同调用后续语句可经注册表取附加库实例（本 Iteration 的消费仅限注册表内部一致性与生命周期可观测）。
  - 路径解析复用 `resolve_db_path`；目标不存在 → 点名拒绝（exit 3，语句上下文），SHALL NOT 创建文件。
  - 打开错误映射：`DatabaseLocked` → exit 4、`InvalidKey` → exit 5、其他 → exit 3（`sql_failure_status` 同型带语句上下文）。
  - 重复别名 → 点名拒绝（exit 3），原注册保持；自附加（同 inode）→ `DatabaseLocked` exit 4（锁机制，无 canonicalize）。
  - `database_file_name` 非字符串字面量（表达式/标识符等）→ 计划期点名拒绝。
  - lib 路径（`Database::execute*`/`execute_in_tx`）→ session-only 点名拒绝；ATTACH 不进 plan cache（`plan_cache_len` 不变）。
  - epilogue：进程正常退出前对附加库 best-effort close（忽略结果）；fail-fast 中途退出随进程释放锁。
- Preserve: 既有三分派行为逐字节不变（tx 臂/普通臂/边界臂）；`parse_stage` 无自定义分片；主库 open/close 路径零变化；lib 公开 API 签名零变化。
- Forbidden: DETACH 任何形式实现；预解析文本拦截；canonicalize 路径比较；`Database` 新增字段；plan cache key 结构变化。
- Test witness: `tests/attach_session_test.rs`（新，RED：文件不存在）——注册成功+多语句可见、裸名解析（RTSQL_HOME）、路径缺失、重复别名、锁冲突（预开第二进程持锁）、自附加、lib session-only + plan cache 不变、退出后锁释放（同进程重开成功）；planner 单元测试（`planner/mod.rs` 分类矩阵，tx_statement_tests 343-432 先例）。命令：`cargo test --test attach_session_test` 与 `cargo test planner`；预期 RED：测试文件不存在/断言失败。
- GREEN condition: 上述测试全绿；全量 `cargo test -- --test-threads=1` 零回归（基线 1408 + 新增）。
- Verification: `cargo test --test attach_session_test`（exit 0）；`cargo test -- --test-threads=1`（exit 0，failures=0）；失败含义：契约语义未达成或既有面回归。
- Stop when: ATTACH 解析需要 sqlparser 行为超出已查证范围（如 `DATABASE` 关键字形态差异）；锁/错误映射与 `open_error_status` 语义冲突无法调和；需要改动 `Database` 结构或 plan cache key 结构。

**Invariants**

- 全量测试零回归（1408 基线 + 新增净增）。
- 无自研 SQL 分片/预解析层。
- lib 公开 API 与既有 CLI 行为（除 ATTACH 臂新增外）零变化。
- 附加库一期只读消费：本 Iteration 不提供任何对附加库的数据访问面（查询路由属 001）。

**Non-goals**

- T2/T3/T4 全部；DETACH；附加库 close 的显式 checkpoint 语义（只读零工作跳过已覆盖）。

**Acceptance**

- `sql-attach-session` R1–R3 全部场景有测试见证且 GREEN。

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| sql-attach-session R1（注册/路径/锁） | T1 | `tests/attach_session_test.rs` 注册与路径/锁场景 | Covered |
| sql-attach-session R2（重复别名/自附加/不进 cache） | T1 | `tests/attach_session_test.rs` 拒绝矩阵 + `plan_cache_len` 断言 | Covered |
| sql-attach-session R3（lib session-only） | T1 | `tests/attach_session_test.rs` lib 段 + planner 单元测试 | Covered |
- 拒绝矩阵（重复别名/路径缺失/锁冲突/自附加/非字面量路径/lib session-only）exit 码与文案符合 Task Contract。
- 全量零回归。

**Verification**

- `cargo test --test attach_session_test` → exit 0。
- `cargo test planner` → exit 0（分类器单元测试）。
- `cargo test -- --test-threads=1` → exit 0，0 failures（决定性输出：test result 行）。

**Gate 2 Readiness**

PASS（2026-09-29 用户批准计划）——无 BLOCKED/WAIVED 项；无 Missing/Simplified requirement；调查证据齐备；无实质未知项；Persisted Evidence: none。

**Persisted Evidence**

- Mode: none

（Act Response 承载命令、输出与退出码；无用户明示留存需求，结果可低成本重跑。）

**Risks and Notes**

- `PlanError` 新变体 vs 复用 `TransactionStatement(String)` 承载 session-only 文案：等价选择留 Act（文案即契约，须在测试中锁定）。
- ATTACH 语句失败时 `sql_failure_status` 的「previous statement(s) committed」后缀语义对 exit 4/5 映射路径的适配方式：等价实现选择留 Act。
- 附加库在 `run_sql` 正常结束路径的 close 顺序（先 tx 回滚检查后附加库 close）：非实质。

## Act Response

- Status: reported

**Delivered**

`ATTACH '<path>' AS <alias>` 在 CLI one-shot 会话内注册附加库（`Arc<Database>`，独占锁，密钥通道与主库同源），拒绝矩阵全绿且零副作用；lib 路径 session-only 拒绝。注册面已就绪，Iteration 001 消费之（查询路由）。

**Changed files and symbols**

| File | Change |
|---|---|
| `src/parser/error.rs` | 新增 `PlanError::AttachStatement(String)`（Display 裸文案、`sqlstate()` → 0A000）；`planerror_sqlstate_mapping` 表补该变体 |
| `src/parser/planner/mod.rs` | 新增 `AttachInfo { path, alias }`（pub(crate)）与 `classify_attach_statement()`（pub(crate)，`classify_transaction_statement` 同型三值契约）；`build_plan` 在事务语句前置拒绝之后追加 ATTACH 前置臂；新增 `mod attach_statement_tests` 6 例 |
| `src/cli/mod.rs` | 新增 `AttachSession`（`HashMap<String, Arc<Database>>`）+ `AttachFailure`（Sql/Locked/InvalidKey 三类）+ `attach_database()` + `attach_open_failure()`；`run_sql` 增 `key: Option<&str>` 参数与第四臂（成功 `Affected(0)` 直连、失败经 `rollback_session` 后按语义类映射 4/5/3）；epilogue 追加附加库 best-effort close；`sql_failure_status` 拆出共用构造点 `sql_failure_message`（输出逐字节不变） |
| `tests/attach_session_test.rs` | 新增 11 例（e2e 9 + lib 2） |

`Database` 零字段变化、plan cache key 零变化、`parse_stage` 与既有三分派逐字节不变、lib 公开 API 签名零变化；`sqlparser` 无自研分片/预解析层。

**Deviations from Plan**（均为 Cycle 显式留给 Act 的等价选择，非实质问题）

1. **新增 `PlanError::AttachStatement` 变体**，未复用 `TransactionStatement(String)`（Cycle Risks 第 1 条）。理由：后者是事务语句专用变体，承载 ATTACH 拒绝会把能力面报错成事务错，违 MS19 错误面诚实纪律。Display 裸文案、SQLSTATE 0A000 与事务变体同型。
2. **`run_sql` 增 `key` 参数**（private fn，lib 公开 API 不变）：密钥通道须抵达 attach 臂。`Option<String>` 在 HRTB 工厂闭包外克隆，使 work future 不借用 `args`（保持 `for<'a>` 约束成立）。
3. **非字面量路径的拒绝优先级**：`build_plan` 透出分类器点名消息 `ATTACH database path must be a string literal`，而非 session-only 文案——该形态在任意执行面都非法，session-only 文案会误导为「换个面就能用」。契约只要求「计划期点名拒绝」未固定优先级；`build_plan_reports_path_form_rejection_for_non_literal_path` 锁定该行为。
4. **exit 4/5 消息复用多语句上下文**（Cycle Risks 第 2 条）：三类退出码共用 `sql_failure_message` 构造点，消息形状与 exit 3 逐字节一致，仅语义类（退出码）不同。
5. **epilogue close 忽略返回**（design D1）：一期只读无脏页，MS18 零工作守卫使 close 无操作；进程退出时 flock 随 fd 释放兜底。
6. **新增 Cycle 未列举的 exit 5 场景** `attach_invalid_key_rejected_with_exit_5`：契约 Required behavior 明确要求 `InvalidKey → exit 5`，无见证则该分支未经 TDD 见证。构造为「明文主库无密钥 + 加密附加库」，失败只可能来自附加库打开面（不会因主库带钥而假通过）。

**Self-review findings**

- Critical / Important：0 未解决。Gate 3 两类检查（spec compliance + code quality）已过：R1–R3 全部场景有 GREEN 见证；既有三分派未改；无新增警告（clippy 干净）、无死代码、无重复实现；未引入身份型证据机制或判定层。
- Minor 1（保留记录）：`attach_locked_database_rejected_with_exit_4` 用独立 OS 线程 + 私有 Tokio runtime 持 flock，CLI 侧同步驱动，跨进程锁序依赖 500ms settle 等待 + 3s 持锁窗口。属真实锁等待时序，非身份型判定层；已把持锁窗口由 5s 收到 3s。
- Minor 2（保留记录）：`ATTACH 'a.db' AS x; ATTACH 'a.db' AS y`（同文件两个别名）以 exit 4 `database is locked` 诚实拒绝——独占锁模型下同进程不可重复打开，不静默给别名；spec 场景表未列该形态，无需新决定。

**Verification**

| 命令 | 结果 | 退出码 |
|---|---|---|
| `cargo test --test attach_session_test` | `11 passed; 0 failed`（注册/裸名/路径缺失/锁冲突/重复别名/自附加/exit 5/事务内/退出释放/lib×2） | 0 |
| `cargo test --lib planner` | `23 passed; 0 failed`（含 6 例 attach 分类器/前置拒绝） | 0 |
| `cargo test -- --test-threads=1` | 全量 1425 tests / 0 failures（基线 1408 + 新增 17） | 0 |
| `cargo fmt --check` | 无 diff | 0 |
| `cargo clippy --all-targets` | 无 warning（仅 cargo config 弃用提示，与本次改动无关） | 0 |
| `openspec validate 2026-09-29-ms20-attach-cross-db --strict` | `Change '2026-09-29-ms20-attach-cross-db' is valid` | 0 |

基线采信：Cycle 记录的 commit `bc1d18d` / 1408 tests 结论（2026-09-28 串行，覆盖范围此后无代码变化）。本次基线核对发现工作树含 MS23+MS24+MS25 未提交实现（索引内 162 文件、+15561/-3542），全量验证即在该工作面上运行，与 Cycle 声明的「工作区基线」一致。

**Persisted Evidence**

- None required（Mode: none，按 Plan）。全部结论可由上述命令低成本重跑，无用户明示留存需求，无环境即将消失的一次性现场。

**Experience Candidates**

- None。本 Cycle 未发现 change 范围外的实质缺陷（无 Issue 候选），亦无「已端到端验证且可重复或高风险」的操作路径满足 Runbook 准入——锁夹具属测试实现细节，不构成可复用运行流程。

**Unresolved**

- 无阻塞项。
- **部分场景覆盖（须 Review 知悉）**：`sql-attach-session` R1/S1 的规格可观察量是「第二条语句可经别名 `o` 读取附加库表」，其中「读取」属 T2 路由面。本 Cycle 按 Plan 的显式切分只交付注册面，故该场景当前的见证是「注册成功 + 同次调用后续语句继续执行」，`SELECT * FROM o.u` 仍按既有扁平名语义查主库 catalog（`TableNotFound`）——Iteration 001 交付路由后该场景才完整闭合。R1 其余场景（裸名解析/路径缺失/锁冲突）、R2 全部场景、R3 全部场景本 Cycle 已完整见证。
- Iteration 001（T2/T3）以本 Cycle 的注册面为前置：T2 消费 `AttachSession.dbs` 做路由查找，预期无需改动本 Cycle 的注册面结构。
- Plan Review 未执行（Act 不填 Review，`Review Result` 仍 `pending`）。

## Plan Review

- Review Result: accepted

**Findings**

Plan 独立阅读了全部变更面（`src/parser/error.rs`、`src/parser/planner/mod.rs`、`src/cli/mod.rs`、`tests/attach_session_test.rs`）并与 Act Response 声明逐条核对一致；mtime 核对显示 4 个文件最后一次修改（12:39–12:42）均早于 Act Response 写入（12:47），覆盖范围未变化，Act 验证结论按公共规则 › 验证 采信。

非阻塞 Minor findings：

1. **Acceptance 措辞宽于契约切分（Plan 遗漏，非实质）**：Cycle Acceptance 写「R1–R3 全部场景有测试见证且 GREEN」，而 Task Contract 显式把 T1 可观察量切分为注册面（「本 Iteration 的消费仅限注册表内部一致性与生命周期可观测」）。`sql-attach-session` R1/S1 的 THEN（「可经别名 `o` 读取附加库表」）须待路由面才能完整见证。Act 已在 Unresolved 如实披露；RTM 见证列实际按「注册与路径/锁场景」取证，与契约一致。**处置**：闭合义务转入 Iteration 001 Task Contract T2（`ATTACH ... AS o; SELECT * FROM o.u` e2e 同时闭合本域 R1/S1 与 cross-db-namespace-query R1/S1）。
2. **proposal.md What-Changes T02 与用户裁定矛盾（Plan 错误，非实质，已修正）**：proposal 正文原写「未命中 → 计划期点名拒绝（不静默回落主库同名表）」「三段及以上限定名点名拒绝」，与同文件假设 6 用户裁定（2026-09-29：未命中回落扁平名；三段仅首段命中别名才拒绝）及 delta spec `cross-db-namespace-query` R1 相矛盾。本次 Review 已将正文两处修正为与裁定/spec 一致；不影响 Cycle 000 验收。
3. **Act Minor 1（锁夹具时序）复核接受**：独立 OS 线程持 flock + 500ms settle + 3s 持锁窗口属真实锁等待时序，非身份型判定层，不违反公共规则 › 行为约束。
4. **Act Minor 2（同文件双别名 `ATTACH 'a.db' AS x; ATTACH 'a.db' AS y` → exit 4）复核接受**：与 design D8 flock 按 inode 拒绝语义一致，spec 场景表未列举该形态，诚实拒绝无契约冲突。

6 项 Deviations 逐一复核（`PlanError::AttachStatement` 新变体、`run_sql` key 参数、非字面量路径文案优先级、exit 4/5 消息复用 `sql_failure_message`、epilogue 忽略返回、补 exit 5 见证）——全部落在 Cycle 显式留给 Act 的等价选择内或属 TDD 见证补齐，无实质偏差。其中 Deviation 1（新变体）经代码复核成立：`error.rs:63,124,145` 变体/Display/SQLSTATE 0A000 与事务变体同型，`planerror_sqlstate_mapping` 表已补（`:174` 测试锁定）。

Forbidden 边界核对通过：`Database` 零字段变化（`AttachSession` 为 `cli/mod.rs` 本地结构）；plan cache key 零变化（CLI 路径 ATTACH 分类消费先于 `plan_stage`，lib 路径 `build_plan` 前置拒绝先于 cache put）；lib 公开 API 零变化（`run_sql` 为 private fn）；无 DETACH、无预解析文本拦截、无 canonicalize。变更面 = 恰好声明的 4 个文件（src/tests 全树 mtime 扫描无其他近期改动）。

**Acceptance Gaps**

None（按 Task Contract 切分后的 Iteration 000 Acceptance 全部满足；`sql-attach-session` R1/S1 完整闭合按计划归 Iteration 001，见 Findings 1）。

**Evidence**

- 代码独立阅读：`src/parser/planner/mod.rs:104-140`（AttachInfo / classify_attach_statement 三值契约）、`:260-269`（build_plan ATTACH 前置臂，先于 plan cache）、`:400-500`（6 例单元测试）；`src/cli/mod.rs:384-453`（AttachSession / attach_database——重复别名先于打开判定、`exists()` 前置、`attach_open_failure` 三分类）、`:478-526`（四臂分派 + 失败先回滚后映射 4/5/3）、`:616-629`（epilogue best-effort close）、`:650-690`（`sql_failure_status` 委托 `sql_failure_message`，消息形状逐字节一致）、`:263-277`（key 经工厂闭包克隆穿线，HRTB 约束保持）；`src/parser/error.rs:63,124,145,174`。
- 测试独立阅读：`tests/attach_session_test.rs` 头部场景映射表与 11 例实现逐条对应（R1/S1-S4、R2/S1-S2、R3/S1×2、D1/D2/假设 11 补充见证）。
- 验证新鲜度：mtime 核对（4 文件 12:39–12:42 < Act Response 写入 12:47:46；src/tests 无其他 6 小时内改动文件）。
- 采信 Act Response Verification 表（来源：Cycle 000 Act Response，运行于同一工作面，覆盖范围此后未变化）：attach_session_test 11 passed / planner lib 23 passed / 全量串行 1425 tests 0 failures / fmt、clippy、`openspec validate --strict` 全过。

**Follow-up Decision**

接受（accepted）：按 Task Contract 切分的 Acceptance 全部有 GREEN 见证，无阻塞项；四项 Minor 均不触及契约语义或 Acceptance。`sql-attach-session` R1/S1 的读取可观察量闭合义务随 Iteration 001 交付，已写入其 Task Contract T2。Iteration Plan 不变。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-09-29-ms20-attach-cross-db/iterations/001-cross-db-query/000-initial.md`（已展开，Plan Context ready）
