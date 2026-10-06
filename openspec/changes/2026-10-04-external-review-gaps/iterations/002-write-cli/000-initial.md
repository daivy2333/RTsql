# Iteration 002 / Cycle 000: 写面与 CLI 可观测批

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T6（无 WHERE 全表 UPDATE/DELETE）、T7（CLI 错误输出携带 SQLSTATE）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal R6/R7 与场景 S22–S29；design D6/D7；Iteration 000/001 交付基线——T1/T2/T3/T9（Iteration 000 accepted）与 T4/T5/T11/T10（Iteration 001 replan Cycle，2026-10-06 Review accepted）全部交付；全量 **1917 passed / 0 failed / 2 ignored**（Iteration 001 Cycle 001 Act Response 集成 Gate + Plan Review 新鲜度核验采信，见 `iterations/001-subquery/001-replan.md` Plan Review）
- Excluded scope: 一般谓词 UPDATE/DELETE 解锁（第二重门维持拒绝）；TRUNCATE 语义变更；文档同步（Iteration 003 T8）；规格语料合并（docs-maintainer 收尾）

**Objective**

写面 + CLI 可观测两合一：① `UPDATE`/`DELETE` 省略 WHERE 作用于全部行——逐行版本写/墓碑、per 行门序（类型门/FLOAT 升格/NOT NULL/键列类型门 + rekey 碰撞预检）、唯一与二级索引维护照常，语句失败隐式事务回滚全部已写行，`affected_rows` 为实际行数，显式事务可用可回滚，附加库目标经既有路由同语义；② CLI 错误输出机器可读——文本格式行尾 ` [SQLSTATE xxxx]`、`--format json` 错误对象携带 `sqlstate` 字段。全量测试零回归。

**Investigation Facts**

- Current Baseline: 工作区 = Iteration 001 交付后状态（未提交，HEAD 9538647 + 本 change 全部改动面）；全量 1917 passed / 0 failed / 2 ignored。行为基线：① 无 WHERE UPDATE/DELETE → `Missing required field: WHERE clause for UPDATE/DELETE` exit 3；② CLI 错误输出无 SQLSTATE——文本与 JSON 均纯文字（`Response::Error` 已携带可选 `sqlstate`，CLI 渲染未消费）。
- Current-State Evidence（行号为 Iteration 001 交付后工作区现场，Act 以符号定位为准）：
  - **T6——计划节点**：`UpdateNode { table_name, key: Key, assignments: Vec<UpdateAssignment> }`（`src/executor/plan.rs:308-315`）、`DeleteNode { table_name, key: Key }`（plan.rs:317-324）——`key` 必填单键，全表形态需 `Option<Key>` 化。
  - **T6——计划期双门**：`build_update`（`src/parser/planner/ddl_dml.rs:1898`）第一重门 `selection.as_ref().ok_or(MissingField("WHERE clause for UPDATE"))`（:1913-1915）+ 第二重门 `extract_pk_from_where(...).ok_or(ParseError("UPDATE requires primary key equality in WHERE clause"))`（:1916-1921）；`build_delete`（ddl_dml.rs:2005）同构两门（:2019-2028，文案 `...for DELETE` / `"DELETE requires primary key equality in WHERE clause"`）。全仓 tests/ 无「requires primary key equality」「Missing required field」文案断言（grep 实证）——移除第一重门零测试波及。
  - **T6——执行器单点形态**：`UpdateExecutor`（`src/executor/update.rs:43`，`next` :89——单键 search 定位、同旧行快照求值全部右值、per 赋值门序 NOT NULL → 键列类型门 + rekey 碰撞预检 → coerce/类型门/FLOAT 升格、版本写 + 唯一/二级索引维护、Step 7 按 `pk_assigned` 三分支）；`DeleteExecutor`（`src/executor/delete.rs:14`，`next` :99——单键墓碑 + 索引清理）。全表臂 = `IndexManager::scan_all() -> Result<Vec<(Vec<u8>, RowId)>>`（`src/storage/btree/index_manager.rs:339`，现成原语）枚举后逐行复用既有单行逻辑。
  - **T6——DML 事务与路由**：pipeline.rs DML 根事务臂（`PhysicalPlan::Update(_) | Delete(_)` 汇聚点 pipeline.rs:434-449——`routed_db`（pipeline.rs:1306）解析目标库、隐式事务 begin/commit/abort 包裹；显式事务经 `execute_stage_in_tx_routed` 复用）。语句中途失败的整语句回滚由该既有事务包裹承载，零新增机制。readonly 25006 拒绝按计划节点变体匹配——`Option<Key>` 化不改变变体，自动覆盖。视图只读（`reject_view_write`）与 `validate_table` 在两门之前，零变化。
  - **T6——静默失败既有用例**：`tests/subquery_test.rs:262`（`DELETE FROM emp`，test_scalar_subquery_empty_result 前置）、`:423`（`DELETE FROM dept`）——当前响应被丢弃、实际被第一重门拒绝；放开后成为真删除，与用例注释意图一致，验证断言仍成立即可。
  - **T7——协议载荷**：`Response::Error { message, sqlstate: Option<String> }`（`src/network/protocol.rs:41-46`）；Response JSON 序列化已按 None 省略/Some 携带（protocol.rs:134-151 单测在案）；PG 协议面（error-reporting R2）独立于 CLI 渲染。
  - **T7——CLI 渲染现状**：`run()` 顶层以 `emit_stderr(status.message())` 输出一切错误（`src/cli/mod.rs:226-229`）；`sql_failure_status`/`sql_failure_message`（cli/mod.rs:938/:956）从 `&str` 构造失败文案（语句 k of n 定位 + 事务后缀）；`run_sql` 循环内 4 处 `Response::Error { message, .. }` 匹配点（cli/mod.rs:797/:825/:852/:884 区域）**丢弃 sqlstate**；parse 阶段 `Err(e)` 路径无 sqlstate。**CLI 现无 JSON 错误对象**——`QueryPayload` 仅 `Rows/Affected`（`src/cli/render.rs:15-18`），render 无 Error 臂，错误恒为 stderr 文本。
  - **T7——设计 D7 勘误（Review 发现的 Plan 调查缺口）**：D7 原文「JSON 错误对象增加 sqlstate 字段」以既有对象为前提，失实。本 Cycle 闭合发射语义（见 T7 Task Contract Required behavior）。
- Code and Critical Path:
  - T6：`plan.rs` 两节点 `key` 改 `Option<Key>` → `ddl_dml.rs` 两构建点第一重门移除（None WHERE → `key: None`；Some WHERE 走既有提取链，第二重门原文案保留）→ `plan_format.rs` Update/Delete 渲染臂适配 Option → `update.rs`/`delete.rs` 全表臂（scan_all 枚举 → 逐行复用单行逻辑 → AffectedRows 实数汇总）→ `pipeline.rs` 构造臂机械适配（key 传递）+ `extract_all_table_names`/readonly/视图各消费点核对（节点变体不变，预期零改动）。
  - T7：cli/mod.rs `ExitStatus::Sql` 携带 `Option<String> sqlstate`（或平行载体）→ 4 处 Response::Error 匹配点透传 → `run()` 渲染分叉：非 JSON 文本行尾追 suffix；JSON 模式 stderr 发射单行 JSON 错误对象。`sql_failure_message` 签名随载体机械适配；ATTACH/关闭失败/生命周期子命令面不变。

**Implementation Guidance**

顺序：T6（中-大）→ T7（小）。T6 先 RED（`DELETE FROM t`/`UPDATE t SET v = v + 1` 当前 MissingField 拒绝即 RED 基线；S46 型两表对照先确认不受影响）→ 节点 Option 化与构建点改门 → 执行器全表臂（复用单行逻辑，禁止复制粘贴门序——提取共享路径或逐行构造既有单行执行序列，以 Act 惯用手为准，行为契约以 S22–S26 为准）→ 碰撞回滚与附加库矩阵。T7 在 T6 全绿后独立实施：先 suffix 文本面（readonly_session 既有 25006 簇断言扩展），后 JSON 对象面；`--format` 判定复用 `kind(format)` 既有单点。两任务均不建身份型证据工程。

**Task Contracts**

### T6: 无 WHERE 全表 UPDATE/DELETE

- Requirement/Scenario: R6 / S22–S26
- Depends on: None
- Targets: `src/executor/plan.rs`（`UpdateNode.key`/`DeleteNode.key` → `Option<Key>`）、`src/parser/planner/ddl_dml.rs`（`build_update`/`build_delete` 第一重门移除）、`src/executor/{update,delete}.rs`（全表臂）、`src/executor/plan_format.rs`（渲染臂 Option 适配）、`src/pipeline.rs`（构造臂机械适配）、`tests/full_table_dml_test.rs`（新）
- Current behavior: 无 WHERE UPDATE/DELETE → `Missing required field: WHERE clause for UPDATE/DELETE` exit 3（第一重门）；带 WHERE 非 PK 等值 → `UPDATE/DELETE requires primary key equality in WHERE clause`（第二重门）。
- Required behavior: S22 `DELETE FROM t` → `affected_rows`=实际行数、逐行墓碑、重开一致、dump 无行；S23 `UPDATE t SET v = v + 1` 全部行生效、per 行类型门/NOT NULL/键列门照常（含 FLOAT 升格）；S24 全表 rekey 中途碰撞 → 语句失败、全部已写行回滚零残留（隐式事务）、显式事务内可 ROLLBACK；S25 附加库目标经 `routed_db` 同语义（每文件独立提交）；S26 `TRUNCATE TABLE t` 与带 WHERE 形态（含 PK 等值照常、子查询拒绝面）零变化。
- Preserve: 第二重门文案与语义逐字节（带 WHERE 非 PK 等值维持点名拒绝）；带 WHERE PK 等值单行形态逐字节；`UPDATE SUBQUERY` 与 DELETE WHERE IN 子查询拒绝面；readonly 25006（变体匹配自动覆盖）；视图只读；TRUNCATE checkpoint-first 物理释放与事务内拒绝；per 行门序与既有单行顺序同构（NOT NULL → 键列门 + rekey 碰撞预检 → coerce/类型门/FLOAT 升格，practical-core-gaps T9 语义）；MVCC 可见性按枚举时点快照（与单行同源）；plan cache 既有规则。
- Forbidden: 不得解锁一般谓词 UPDATE/DELETE（第二重门保留）；不得改 TRUNCATE 语义；不得改 `UpdateAssignment`/`UpdateValue` 节点结构；不得为回滚引入新事务机制（复用 DML 根事务包裹）。
- Test witness: 新 `tests/full_table_dml_test.rs`（S22–S26 矩阵：全表 DELETE/UPDATE、逐行碰撞回滚、显式事务回滚、附加库目标、TRUNCATE 对照、子查询拒绝面）——先写 RED（第一重门拒绝即 RED）；`tests/subquery_test.rs:262/:423` 两静默失败用例转真删除验证（断言应保持成立）。
- GREEN condition: 新矩阵全绿 + subquery_test/cross_db_dml_test/cli_test 定向回归零失败。
- Verification: `cargo test --test full_table_dml_test --test subquery_test --test cross_db_dml_test` + 全量 0 failed。
- Stop when: 全表臂需要执行器 trait 语义变化（如需异步枚举键先行物化超出 scan_all 契约）；或逐行门序复用与既有单行文本/顺序出现不可调和偏差 → 返回 Plan。

### T7: CLI 错误输出携带 SQLSTATE

- Requirement/Scenario: R7 / S27–S29
- Depends on: None
- Targets: `src/cli/mod.rs`（`ExitStatus::Sql` 携带 sqlstate、4 处 Response::Error 匹配点透传、`run()` 渲染分叉、`sql_failure_message` 机械适配）、`tests/readonly_session_test.rs`/`tests/cli_test.rs`（错误面扩展）
- Current behavior: 错误输出无 SQLSTATE——所有格式下错误恒为 stderr 纯文本（`run()` → `emit_stderr`）；CLI 无 JSON 错误对象（`QueryPayload` 仅 Rows/Affected）。
- Required behavior: S27 文本格式（table/csv/tsv 与未指定）错误行尾附加 ` [SQLSTATE xxxx]`（`Response::Error` 携带 sqlstate 时），文案与 exit 3 不变；S28 `--format json` 时语句失败错误以**单行 JSON 对象发射到 stderr**（替代该错误的文本行）：`{"error":"<消息文本，不含 suffix>","sqlstate":"<code>"}`，`sqlstate` None 时省略字段；Usage/General/Locked/InvalidKey 类无 sqlstate → JSON 对象省略字段，文本模式不加 suffix；S29 PG 面与正常查询/受影响行渲染零变化、退出码矩阵零变化、stdout 数据通道零变化（错误不进 stdout——dump 纯净纪律同源）。
- Preserve: 退出码矩阵（0/1/2/3/4/5/130/143）；fail-fast 语句定位框架文本（JSON 对象 `error` 值 = 同一 `sql_failure_message` 文本减 suffix）；非错误输出序列化逐字节；PG 协议错误响应（error-reporting R2）；lifecycle 子命令错误面（dump 告警/check 报告/delete 报告不经本通道）；信号退出无消息形态。
- Forbidden: 不得改 `Response::Error` 结构与协议序列化；不得改 PG 协议面；不得把错误发射进 stdout；不得为判定输出新增脚本或工具。
- Test witness: `tests/readonly_session_test.rs` CLI 错误面扩展（`--readonly` 下 INSERT：文本含 `[SQLSTATE 25006]` + `--format json` stderr JSON 含 `"sqlstate":"25006"`）+ `tests/cli_test.rs` 扩展（无 sqlstate 的 General/Usage 错误 JSON 省略字段 + 文本无 suffix）——先写 RED（当前两形态均无代码即 RED）。
- GREEN condition: 扩展矩阵全绿 + readonly_session_test/cli_test/pg_integration_test 定向回归零失败。
- Verification: `cargo test --test readonly_session_test --test cli_test --test pg_integration_test` + 全量 0 failed。
- Stop when: sqlstate 载体需要穿线超出 `ExitStatus` 与 `run()` 渲染点的面（如 lifecycle 子命令大改）→ 返回 Plan。

**Invariants**

- 存储格式、网络协议字节、plan cache key 语义零变化（T7 的 stderr JSON 为 CLI 渲染面新增，不触协议）。
- 全部既有 spec 语料行为零回归（本 Iteration delta：`sql-write-surface` ADDED、`cross-db-dml` MODIFIED、`cli-noninteractive-shell` ADDED，均在 change specs/ 在案）。
- 不建身份型证据工程/判定层；验证判定使用项目测试框架原生退出码与输出。

**Non-goals**

- 一般谓词 UPDATE/DELETE；TRUNCATE 语义；REPLACE/UPSERT 全表形态（未在反馈面，维持现状）。
- 文档同步（Iteration 003）；spec 语料合并（docs-maintainer 收尾）。

**Acceptance**

- S22–S29（proposal 场景草图）全部可观察通过；R6/R7 的 spec delta 场景与任务测试一一对应（映射见各 Task Contract Requirement/Scenario 行）。
- 全量 `cargo test` 0 failed；`cargo fmt --check` 与 `cargo clippy --tests` 干净。

**Verification**

- 直接目标：`cargo test --test full_table_dml_test --test readonly_session_test`（新增用例 RED→GREEN）。
- 受影响边界：`cargo test --test subquery_test --test cross_db_dml_test --test cli_test --test pg_integration_test --test explicit_tx_test`。
- 集成 Gate：全量 `cargo test` 0 failed。
- 判定：各命令退出码 0 + 原生输出行数统计，不新增判定脚本。

**Gate 2 Readiness**

调查完整（T6 节点/双门/执行器/事务路由/静默用例、T7 协议载荷/渲染链/无 JSON 错误对象事实，全部按当前工作区符号核实）；设计闭合（D6 直接可用；D7 勘误后发射语义本 Cycle 闭合——stderr JSON 对象、形状与省略规则已 pin，见 T7 Required behavior）；任务契约齐备自包含；RTM 对应行 Covered 无 Missing；验证逐 scenario 最简直接判定、无身份型证据工程；Persisted Evidence `none` 满足白名单。计划批准：T6/R6 承 2026-10-04 集中决策 #1（原话「批准」）；T7/R7 承集中决策 #2（「文本后缀 + JSON 字段」）；D7 发射细节（stderr 通道 + 对象形状）为 Review 展开时 Plan 闭合的实现级完成——用户于 2026-10-06 审计批准本 Cycle 计划（原话「批准，另外，提交保存一下当前工作」），全项通过，Status: ready 交 Act。

**Persisted Evidence**

- Mode: none

（全部验证可由测试命令 + Act Response 承载；全表 DML 与 CLI 渲染行为均为确定性复现，无需留存现场。）

**Risks and Notes**

- T6 全表 UPDATE 的逐行碰撞预检在「新键与本次语句已写行」间的语义与单行形态一致（预检以索引实时状态为准，写后行参与后续预检）——S24 回滚见证兜底；若回滚后索引残留属实质问题返回 Plan。
- T6 附加库目标的每文件独立提交语义（cross-db-dml 既有边界）对全表形态自然成立，非原子 v1 语义文档化面不扩。
- T7 JSON 错误对象的 stderr 通道与 `{"error":…}` 扁平形状为本 Cycle 定稿（delta spec 场景只 pin 字段存在与省略规则，不 pin 通道/形状）；如用户审计要求改判（如 stdout 或嵌套形状），属 Plan 修订非 Act 偏差。
- T7 4 处匹配点行号为调查快照，Act 以 `Response::Error {` 符号定位为准。
