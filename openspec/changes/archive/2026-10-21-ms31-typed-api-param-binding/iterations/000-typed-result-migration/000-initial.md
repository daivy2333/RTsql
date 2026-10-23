# Iteration 000 / Cycle 000: 类型化结果基建与调用面迁移

## Plan Context

- Status: ready（Gate 2 PASS，2026-10-21 用户批准「批准」）
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2, T3, T4（change `tasks.md` Iteration 000）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal What Changes 1（T01 全项）+ 两域 delta specs 中 typed-library-results 三条 Requirement；默认假设 A2（签名替换）/A3（错误通道）/A4（列元数据）生效
- Excluded scope: 占位符与参数绑定（Iteration 001）、预编译语句（Iteration 002）、sql-parameter-binding 域全部场景、DML WHERE 键位、MS32/MS22 范围

**Objective**

lib 公共执行入口（`execute_sql`/`execute_in_tx` 与 stage 家族）返回 `Result<SqlOutcome, SqlError>`——查询携带原生 Value 行与逐列元数据（名称 + 类型），DML 携带受影响行数，错误载荷与既有 `Response::Error` 同源；核心路径不再依赖 `network::protocol::Response`；CLI 四格式与 PG/JSON 协议输出逐字节零变化；全量测试（81 文件迁移后）零回归。

**Background**

MS31-T01（I088 promoted）：核心执行结果依赖网络词表、无列元数据、嵌入式使用者无法消费内部 Value。用户裁定（2026-10-21）：单 change 承载 T01+T02；旧签名替换 + 全调用面迁移（不保留兼容层）。

**Investigation Facts**

- Current Baseline: master@34a3ae7 工作区（MS30 交付现场未提交——本 change 在其上叠加）；全量基线 2057 tests / 0 failures / 2 ignored（MS30 终态，采信自 tasks.md 2026-10-21 收尾记录）；分批执行约束见 project-model M19
- Current-State Evidence（2026-10-21 逐项核对）:
  - 入口链：`Database::execute_sql`（`src/database.rs:423`）→ `pipeline::execute`（`src/pipeline.rs:97`）→ `execute_inner`（:1173，cache get → parse → 单语句检查 → plan → execute_stage）；`Database::execute_in_tx`（`src/database.rs:529`）→ `pipeline::execute_in_tx`（`src/pipeline.rs:994`，无 cache get）。database.rs 对 network 的唯一依赖是 `use crate::network::protocol::Response`（:6）
  - Response 词表：`src/network/protocol.rs:34-49`——`QueryResult { rows: Vec<Vec<serde_json::Value>> }` / `AffectedRows { count: u64 }` / `Error { message, sqlstate: Option<String> }` / `Pong`；serde 序列化（JSON 协议直出）
  - 行收集与转换：`execute_executor`（`src/pipeline.rs:1284`）drain 执行器 `ExecResult::Row(Vec<Value>)` → `value_to_json`（:1934，**Float NaN/Inf → JSON Null 有损、Date/Timestamp → DA5 字符串**）→ QueryResult；`ExecResult::AffectedRows(u64)` → AffectedRows（`src/executor/result.rs:8`）
  - 错误漏斗点（文案/SQLSTATE/触发点零变化清单）：parse 失败 `Parse error: {msg}` + 42601（`execute_inner:1220`、`execute_in_tx:998`）；多语句 `multi_statement_rejected`（:650，execute_inner:1232 / execute_in_tx:1004 调用）；plan 失败 `PlanFailure`（:133-157，`Plan error: ` 单层前缀 + `PlanError::sqlstate()`）；执行失败 `Execution error: {e}` sqlstate None（`execute_executor:1299`）；DML abort 失败 `Abort failed: {e}`（:519）/ commit 失败 `Commit failed: {e}`（:533）；只读拒绝 `readonly_rejection`（:588，25006）；附加库事务内写拒绝 `attached_write_in_tx_rejection`（:626，0A000）；InsertSelect 源物化/归一失败（:422/:431 sqlstate None）；CTAS 内部失败面（`execute_ctas_routed` :733）
  - 列名单源：`PlanBuilder::get_plan_output_columns`（`src/parser/planner/query.rs:123`，全计划节点 walk，含 projection 裁剪/Join output_columns/递归臂）；CLI 表头消费点 `src/cli/mod.rs:949`
  - 消费面：CLI 主路径 `src/cli/mod.rs:921-984`（plan_stage_routed → get_plan_output_columns → `execute_stage[_in_tx]_routed` → Response 四臂匹配 → `render(kind(format), &columns, &QueryPayload::Rows(rows))`，`sql_failure_status` 消费 message+sqlstate）；`render`（`src/cli/render.rs:24`）输入 `QueryPayload::Rows(Vec<Vec<serde_json::Value>>)`，四格式纯函数零 IO；PG 协议（`src/network/pg_protocol.rs:249-321`）消费 Response 四臂；JSON 协议序列化 Response；`SqlHandler::execute`（`src/network/handler.rs:14-29`，Ping 本地处理）；`cli/lifecycle.rs:837/888`（restore/import 仅判 Error）；`storage/integrity.rs:644-647`（测试 helper `exec_ok`）；`transaction/session.rs` 测试模块（Response 断言）；tests 81 文件（`grep -rln "execute_sql\|execute_in_tx" tests/` 清单，典型断言 `matches!(resp, Response::Error { .. })` 与 `Response::QueryResult { rows }` 解构 + `serde_json::json!` 对比）
  - 类型推导现成件：`ctas_value_type`（`src/pipeline.rs:965`，Value→StorageColumnType、Null→None）；`infer_ctas_column_types`（:930，含 `unwrap_or(String(255))` 建表兜底——元数据不沿用）；计划节点不携带类型，`TableMeta` 持 `column_types`（`get_table` 可得）
  - PlanCache：`src/plan_cache.rs`（DashMap、归一化 key、全 `&self`）；只有 Query 进缓存（`is_cacheable` :2756）；DDL 八臂 + CTAS 执行后 `clear()`（:340/359/370/381/400/407）——本 Iteration 不改其语义
- Code and Critical Path: 修改集中在 `pipeline.rs` execute 链（execute/execute_inner/execute_in_tx/execute_stage[_routed]/execute_stage_in_tx[_routed]/execute_executor/两个 rejection/CTAS/InsertSelect 臂）→ `database.rs` 两入口签名 → 边界（handler/cli）→ 测试迁移。数据流：执行器原生 Value 行（不再 JSON 化）+ names/types 装配 → SqlOutcome → 边界转换（CLI：value_to_json → QueryPayload；network：to_response → Response）→ 渲染/序列化（零改动）

**Implementation Guidance**

- 建议顺序 T1 → T2 → T3 → T4（T2 是原子面：签名替换迫使 src 与 tests 同轮迁移才能恢复编译；T2 中 types 先装配 names + 全 None，T3 独立补类型）。
- T2 迁移策略：tests 81 文件每处在调用点包 `network::protocol::to_response(db.execute_sql(sql).await)`（或文件内既有 helper 改一行），断言体不动；先跑 `grep -rln` 建清单逐文件处理，迁移完成的判据是 `cargo test` 全目标编译通过。
- `execute_executor` 改为收集 `Vec<Vec<Value>>` 原生行；QueryOutcome 的 names 在 `execute_stage*` 内经 `get_plan_output_columns(&plan)` 装配（与 CLI 表头现状同源同序）。
- `Pong` 不进 SqlOutcome：SqlHandler 在调用库入口前处理 Ping（现状即如此）。
- typed 通道保留原生 `Value::Float`（NaN/Inf 不经 JSON Null 损耗）；CLI/协议边界转换后仍渲染 Null——输出零变化由转换同源保证。

**Task Contracts**

### T1: 公共结果类型与边界转换（可验证结果：类型载体与映射就位，独立 GREEN）

- Requirement/Scenario: typed-library-results R1（类型化执行结果——载体）、R2（核心/协议解耦——词表位置）
- Depends on: None
- Targets: `src/result.rs`（新模块：SqlOutcome/ColumnMeta/SqlError + `value_to_json` 迁入）、`src/network/protocol.rs::to_response`（新）、`src/pipeline.rs::value_to_json`（迁移删除，引用面改指新位置）
- Current behavior: `value_to_json` 位于 pipeline（`src/pipeline.rs:1934`）；无公共结果类型；Response 是唯一结果词表
- Required behavior: `SqlOutcome::{Rows{columns: Vec<ColumnMeta>, rows: Vec<Vec<Value>>}, AffectedRows(u64)}`、`ColumnMeta{name, column_type: Option<ColumnType>}`、`SqlError{message, sqlstate: Option<String>}` 公共可用；`value_to_json` 迁入 result.rs 且转换逻辑逐字节不变（Float NaN/Inf→Null、Date/Timestamp DA5 字符串、Bool/Int/String/Null 原样）；`to_response(Result<SqlOutcome, SqlError>) -> Response` 四臂映射（Ok(Rows)→QueryResult〔逐值 value_to_json〕、Ok(AffectedRows)→AffectedRows、Err→Error 原样透传）
- Preserve: Response 词表定义与 serde 序列化形态；`value_to_json` 语义逐字节
- Forbidden: result.rs 引入 `network` 依赖；修改 render/pg_protocol/JSON 序列化；改动 execute 链（T2 范围）
- First test: `src/result.rs` 内 `#[cfg(test)]`（或 `tests/typed_api_test.rs` 首批）：`to_response` 映射矩阵——Rows→QueryResult 值逐项相等（含 Float NaN→Null、Date DA5 字符串）、AffectedRows 透传、Err(SqlError)→Error{message, sqlstate} 逐字段相等。RED 形态：模块不存在，编译失败
- GREEN condition: 上述测试通过；`cargo test --lib` 零回归
- Verification: `cargo test --lib result`（定向，默认并行度可用——M19 只约束全量）
- Stop when: 需要 Response 序列化形态变化，或 value_to_json 无法逐字节保持（如发现既有调用面依赖其副作用）

### T2: 入口签名替换与全调用面迁移（可验证结果：全链 Result 化且既有面零校准通过）

- Requirement/Scenario: typed-library-results R1（四场景全量）、R2（核心无依赖）、R3（零变化前提）
- Depends on: T1
- Targets: `src/pipeline.rs`（execute/execute_inner/execute_in_tx/execute_stage[_routed]/execute_stage_in_tx[_routed]/execute_executor/readonly_rejection/attached_write_in_tx_rejection/multi_statement_rejected/CTAS 臂/InsertSelect 臂）、`src/database.rs::execute_sql`(:423)/`execute_in_tx`(:529) 及 `use Response`(:6)、`src/network/handler.rs::execute`、`src/cli/lifecycle.rs:837/888`、`src/storage/integrity.rs::exec_ok`、`src/cli/mod.rs:921-984` 主路径、`src/transaction/session.rs` 测试、tests 81 文件
- Current behavior: 全链返回 Response；错误经 `Response::Error`（漏斗点清单见 Investigation Facts）；行经 value_to_json JSON 化
- Required behavior: 全链返回 `Result<SqlOutcome, SqlError>`——错误漏斗点逐一改 `Err(SqlError)`（message/sqlstate/触发点逐字节保持）；`execute_executor` 收集原生 `Vec<Vec<Value>>` 行；`execute_stage*` 对 Query 形态装配 `columns`（`get_plan_output_columns` 单源）与 `column_type: None` 占位（T3 补真值）；`database.rs` 两入口签名替换并删除 network 导入；SqlHandler/lifecycle/integrity/CLI 主路径边界转换（CLI：Ok(Rows)→value_to_json→QueryPayload::Rows→render、Ok(AffectedRows)→Affected、Err→`sql_failure_status` 逐字段同源）；tests 81 文件经 `to_response` 保持既有断言
- Preserve: 错误文案与 SQLSTATE 分类逐字节（error-reporting 域契约）；CLI 退出码与 `statement k of n failed` 形态；multi-statement/readonly 25006/attach 0A000 拒绝语义与优先级；plan cache 行为（get/put/clear 位点零变化）；`Pong` 协议本地；ATTACH 会话面（AttachSession 所有权与关闭序）
- Forbidden: 执行器 `next()` 协议与执行器实现；render.rs/pg_protocol.rs/JSON 序列化；保留旧签名或兼容层；改变 tests 断言语义（只允许调用点包转换）
- First test: `tests/typed_api_test.rs`（新文件）——`execute_sql` 返回 `Ok(SqlOutcome::Rows)`：原生值断言（Int/Text/Float/Bool/Date）、列名元数据与 SELECT 清单一致、`SELECT *` 全列序；DML → `AffectedRows` 计数；`Err(SqlError)` 载荷：未知列 message 含列名 + sqlstate 42703、未知表 42P01、语法错 42601。RED 形态：新签名不存在，编译失败
- GREEN condition: 新测试全过；`cargo test` 全部测试目标编译通过且既有断言零校准（迁移只改调用点）
- Verification: `cargo test --test typed_api_test`（定向）+ `cargo test --lib` + 按迁移批次抽验（单目标 `--test <name>` 默认并行度）；全量分批留给 T4
- Stop when: 迁移中发现 Response 消费面存在语义遗漏（如某消费点依赖 QueryResult 的 JSON 形态做非渲染判断）或错误文案无法保持——回 Plan

### T3: 列类型装配（可验证结果：ColumnMeta.column_type 真实值）

- Requirement/Scenario: typed-library-results R1「SELECT 返回原生值与列元数据」「表达式列元数据」两场景
- Depends on: T2
- Targets: `src/pipeline.rs::plan_output_types`（新函数）+ `execute_stage*` 装配点；复用 `ctas_value_type`（:965）
- Current behavior: T2 中间态 column_type 恒 None
- Required behavior: 计划 walk（与 `get_plan_output_columns` 臂结构对称）：扫描族臂（Scan/DataScan/IndexScan/IndexScanAll/ScalarSubqueryFilter/SemiJoin/AntiJoin——节点携 `table_name + columns + projection`）按表名查 `TableMeta` 声明类型、projection 裁剪跟随 names；Join/NestedLoopJoin 按 `output_columns` 逐 (table, column) 查声明类型；Sort/Filter/Limit/Having/Projection/Distinct/SetOperation 递归输入；Aggregate/DerivedScan/SubqueryEval/表达式列 → None。执行后兜底：None 槽对结果行跑 `ctas_value_type` 矩阵推导（同列一致或 Int∪Float→Float；无行/全 NULL/冲突保持 None；**不沿用** infer_ctas_column_types 的 String(255) 兜底与冲突报错——元数据非建表）
- Preserve: names 单源 `get_plan_output_columns`（不加第二命名通道）；空结果声明列经计划 walk 仍报声明类型
- Forbidden: PhysicalPlan 节点类型集变更；执行器改动；binding.rs 消费（types 走 TableMeta 运行期查询，不动 MS30 绑定层）
- First test: `tests/typed_api_test.rs` 增类型断言：`SELECT a, b FROM t`（INT/TEXT 声明）→ Some(Int)/Some(String)；`SELECT a+1` → 值推导 Some(Int)、空结果 None；`SELECT a FROM t` 空结果仍 Some(Int)；`SELECT d, ts`（DATE/TIMESTAMP 列）→ Some(Date)/Some(Timestamp)；全 NULL 表达式列 → None；`SELECT MAX(a)` → 值推导。RED 形态：断言 Some 处实际 None
- GREEN condition: 上述断言通过；`cargo test --lib` + `--test typed_api_test` 零回归
- Verification: 同上定向命令
- Stop when: walk 发现 names 与行形状不对称的节点形态（回 Plan）；或 TableMeta 查询在附加库路由面不可达（Iteration 000 无 attach 路由语句面，若发现可达冲突回 Plan）

### T4: 全量分批回归与零变化终验（纯验证任务）

- Requirement/Scenario: typed-library-results R3「CLI 四格式逐字节等价」「协议面零回归」
- Depends on: T1, T2, T3
- Targets: 无代码变更
- Current behavior: N/A
- Required behavior: M19 分批全量回归（测试目标按字典序分组、批内 `--test-threads=1`、批间串行、8-12 目标/批）全部 0 failed；验证记录注明分批方式与各批计数
- Preserve: N/A
- Forbidden: 为通过回归修改既有断言（发现失败按 T2/T3 契约修复后重跑受影响批）
- First test: 不适用（纯验证任务，无代码变更；SKIPPED: 验证任务无先行测试）
- GREEN condition: 全部批次各自 0 failed（缺一批不得声明全量通过）
- Verification: 分批 `cargo test … --test <name> -- --test-threads=1` 序列；CLI/协议零变化由批内既有 e2e 与协议测试承载（render/pg_protocol/序列化零改动 + T1 转换逐字节同源的结构性保证）
- Stop when: 任一批 failed——定位属 T2/T3 契约内则修复重跑该批，契约外回 Plan

**Invariants**

- 错误文案、SQLSTATE 分类、CLI 退出码、`statement k of n failed` 形态逐字节不变（error-reporting 域契约）。
- CLI 四格式、PG/JSON 协议输出逐字节不变（render/pg_protocol/序列化代码零改动）。
- plan cache 语义（key 归一化、Query-only、DDL/CTAS clear 位点）零变化。
- 执行器 `next()` 协议与全部执行器实现零改动。
- ATTACH 会话面（AttachSession 所有权、路由、关闭序）零变化。
- 多语句/只读/附加库写拒绝的语义与优先级零变化。

**Non-goals**

- 占位符、参数绑定、预编译语句（Iteration 001/002）。
- 列类型的表达式静态推导（本 Iteration 值推导兜底已定稿）。
- 旧签名兼容层（用户裁定 A2）。

**Acceptance**

| Requirement | Scenario | 验证载体 |
|---|---|---|
| typed-library-results R1 类型化执行结果 | SELECT 原生值与列元数据 | T2/T3 先行测试（`tests/typed_api_test.rs`） |
| 同上 | 表达式列元数据 | T3 先行测试 |
| 同上 | DML 受影响行数 | T2 先行测试 |
| 同上 | 错误载荷同源 | T2 先行测试（42703/42P01/42601 断言） |
| typed-library-results R2 核心/协议解耦 | 核心无网络词表依赖 | `grep -rn "network::protocol" src/database.rs src/pipeline.rs src/executor/ src/storage/ src/transaction/` 无输出（单步通用工具） |
| typed-library-results R3 边界转换输出零变化 | CLI 四格式逐字节等价 / 协议面零回归 | T4 全量分批回归（render/协议代码零改动 + T1 转换逐字节同源 + 既有断言零校准） |

**Verification**

- 行为类 scenario 全部走真实入口（`Database::execute_sql`/CLI 既有 e2e），先行测试即集成测试。
- 架构断言（R2）用 grep 单步管道直接判定。
- 全量分批回归（M19）在 T4 预算一次；批内失败按契约修复后重跑受影响批。

**Gate 2 Readiness**

PASS（RTM 无 Missing/Simplified；设计闭合 D1-D4/D8 已定稿；Cycle 与 tasks/specs/design 一致；Persisted Evidence = none 满足白名单）。

**Persisted Evidence**

- Mode: none

全部验证为 `cargo test` 集合内行为断言与单步 grep，可低成本重跑；无一次性环境、无 Issue/Blocker 现场保留需求；Act Response 承载命令与结果足够。

**Risks and Notes**

- T2 体量风险：81 文件迁移为机械面，但漏改任一文件即编译失败——编译器完整枚举，无静默遗漏面。
- `Value::Float` NaN/Inf 在 typed 通道保留原生值（不再 JSON 损耗），CLI/协议边界转换后仍渲染 Null——输出零变化；typed 面为行为增强（原生值诚实），无需额外场景。
- T3 的 `plan_output_types` 臂覆盖若与 `get_plan_output_columns` 出现不对称节点形态，属 Plan 遗漏——回 Plan 而非 Act 自行裁剪。
- M37 evaluate_ref 限制不影响本 Iteration（参数面在 Iteration 001，届时先行锁定）。

---

## Act Response

- Status: reported

> 完整快照：含 T1–T4 原始实施 + Plan Review 要求的两项修复（R-a benches 编译面 / R-b 声明来源一致性语义）的最终状态；修复轮的验证记录见文末「修复轮验证」段。

### 实际改动

**T1 公共结果类型与边界转换**

- `src/result.rs`（新增）：`SqlOutcome`（`Rows { columns, rows }` / `AffectedRows(u64)`）、`ColumnMeta { name, column_type: Option<storage::ColumnType> }`、`SqlError { message, sqlstate }`（含 `Display`/`Error` 实现）、`value_to_json`（自 `src/pipeline.rs` 迁入，转换逻辑逐字节不变）。模块只 `use crate::executor::Value` 与 `use crate::storage::ColumnType`——**不依赖 `network`**（核心/协议解耦的结构性落点）。
- `src/network/protocol.rs`（+75 / −0）：新增 `to_response(Result<SqlOutcome, SqlError>) -> Response` 四臂映射（`Ok(Rows)` → `QueryResult` 逐值 `value_to_json`；`Ok(AffectedRows)` → `AffectedRows`；`Err` → `Error` 逐字段透传）+ 3 条映射矩阵单测。**既有 `Response` 词表与 serde 序列化零改动**（diff 无删除行）。
- `src/pipeline.rs`：`value_to_json` 迁出，引用面改指 `crate::result::value_to_json`。
- `src/lib.rs`：`pub mod result;` + 根级重导出 `ColumnMeta` / `SqlError` / `SqlOutcome`。

**T2 入口签名替换与全调用面迁移**

- `src/pipeline.rs` execute 链全量 Result 化：`execute` / `execute_inner` / `execute_in_tx` / `execute_stage` / `execute_stage_routed` / `execute_stage_in_tx` / `execute_stage_in_tx_routed` / `execute_executor` / `execute_ctas_routed` / `readonly_rejection` / `attached_write_in_tx_rejection` / `multi_statement_rejected`。错误漏斗点逐一改 `Err(SqlError { … })`——message / sqlstate / 触发点逐字保持（`Parse error: ` + 42601、多语句点名、plan 失败 `Plan error: `、`Execution error: ` + None、`Abort failed: ` / `Commit failed: `、只读 25006、附加库事务内写 0A000、CTAS 内部失败面）。`execute_executor` 改收集**原生 `Vec<Vec<Value>>` 行**（JSON 化退到边界）。
- `src/database.rs`：`execute_sql` / `execute_in_tx` 签名换为 `Result<SqlOutcome, SqlError>`；删除 `use crate::network::protocol::Response`。
- `src/network/handler.rs`：`SqlHandler::execute` 经 `to_response` 边界转换（`Ping` 仍在调用库入口前本地处理，不进类型化结果）。
- `src/cli/mod.rs` 主路径：`Ok(Rows)` → `value_to_json` → `QueryPayload::Rows` → `render`；`Ok(AffectedRows)` → `QueryPayload::Affected` → `render`；`Err` → `sql_failure_status`（message + sqlstate 逐字段同源）。`Response::Pong` 臂移除（typed 结果无该变体，Ping 不经此路径）。
- `src/cli/lifecycle.rs`：dump / restore / import 三处改 typed 匹配；dump 与 analyze 的行转换经 `value_to_json`（与原执行链同源，逐字节一致）。
- `src/storage/integrity.rs`、`src/transaction/session.rs`、`src/pipeline.rs`、`src/cli/mod.rs` 的内建测试模块：改用 typed 断言（`resp.is_ok()` / `Ok(SqlOutcome::Rows { … })` / `Value::Int(…)`）。
- `tests/` 81 文件：1458 处 `execute_sql` / `execute_in_tx` 调用点包 `to_response(…)`（**既有断言原样未动**），另 6 处直调 `execute_stage_routed` / `execute_stage_in_tx_routed` 的点同样包转换；`tests/cross_db_dml_test.rs` 与 `tests/insert_select_test.rs` 的 `routed_exec` helper 改返回 `to_response(…)`。
- `benches/` 7 文件 + `examples/bench_minimal.rs`：32 处丢弃返回值的调用点改 `let _ = …`（签名变更后 `Result` 为 `#[must_use]`，语义与原「忽略 Response」一致）。

**T3 列类型装配**

- `src/pipeline.rs`：`plan_type_slots`（**同步**槽位 walk，臂结构与 `get_plan_output_columns` 对称）+ `resolve_type_slots`（**异步**解析，表元数据按表名缓存，同表只查一次）+ `collect_scan_tables`（JOIN 别名兜底候选表收集）+ `projected_column_names`（投影裁剪，语义同 `planner::query::projected_columns`）+ `derive_types_from_rows`（值推导兜底）+ `attach_column_metadata`（装配点：列名 + 声明类型 → `None` 槽值推导）。声明类型取自 `TableMeta.columns`（`storage::ColumnType`），值推导复用 `ctas_value_type` 矩阵。
- `tests/typed_api_test.rs`：新增 10 条类型断言（声明类型 / 日期族 / 空结果仍报声明类型 / 表达式值推导 / 字符串表达式值推导（`COALESCE`）/ 表达式无行保持 None / 全 NULL 保持 None / 聚合值推导 / JOIN 声明类型 / 别名 JOIN 声明类型）。

**T4**：无代码变更（纯验证任务）。

**R-a（Plan Review F1 / G1 关闭）—— benches 面编译修复**

- `benches/sqlite_compare.rs:154`：`RTsqlDirect::execute` 返回类型 `rtsql::network::protocol::Response` → `std::result::Result<rtsql::SqlOutcome, rtsql::SqlError>`（包装方法与库入口签名对齐）。
- 同文件 18 处 `rtsql.execute(…)` 丢弃点加 `let _ =`；`:747` `runtime.block_on(async { … })` 丢弃点加 `let _ =`。
- `benches/common/mod.rs` 6 处 `db.execute_sql(…)` 丢弃点加 `let _ =`。
- 产品代码零改动；bench / example 面无行为变更（纯编译面修复）。

**R-b（Plan Review F2 / G2 关闭）—— 声明来源解析的一致性语义**

- `src/pipeline.rs`：
  - 新增 `ResolvedType { Declared(ColumnType), Unknown, Conflicted }`——声明来源解析结论三态。
  - `resolve_type_slots` 重写：限定符可解析 → 直取该表声明类型；限定符不可解析（别名 / 无限定）→ 按列名在候选表集合匹配，**含该列的全部候选表声明类型一致**才取该类型，不一致 → `Conflicted`（不再取首个候选冒充声明来源）。同表元数据只查一次（新增 `load_cached_meta` 缓存助手）。
  - `attach_column_metadata` 入参改 `Vec<ResolvedType>`：`Declared` → 声明类型；`Unknown` → 保持未知并参与值推导兜底；`Conflicted` → 保持未知且**不参与**值推导（不以值形状冒充声明类型）。
  - `derive_types_from_rows` 增 `derivable` 掩码参数；两处文档注释随实现校准（`plan_output_types` → `plan_type_slots` + `resolve_type_slots`；值推导作用域表述）。
- `src/pipeline.rs` 内建测试模块新增 2 条单测（解析器层一致性语义 + 冲突槽不参与值推导）。
- `tests/typed_api_test.rs` 新增 1 条入口见证（别名 JOIN 列类型与实际值一致）。

### Deviations from Plan

1. **T3 落地为「同步槽位 + 异步解析」两段式（Plan 写单一 `plan_output_types`）**：Plan Context 目标写 `src/pipeline.rs::plan_output_types`（新函数）。Act 落地为 `plan_type_slots`（同步）+ `resolve_type_slots`（异步）。理由：递归 async 函数须 `Box::pin` 装箱才能编译，而两段式同时避免装箱、让同一张表的元数据只查一次。**walk 形状与槽位语义与 Plan 描述等价**，仅函数名与切分点不同。
2. **SemiJoin / AntiJoin 归 Join 族臂（Plan 将其列入扫描族臂）**：Plan Context 写「扫描族臂（…SemiJoin/AntiJoin——节点携 `table_name + columns + projection`）」。实测 `SemiJoinNode` / `AntiJoinNode`（`src/executor/plan.rs:680` / `:701`）**无 `table_name` 与 `columns` 字段**，只有 `output_columns` + `projection`——按节点实际形状归入 Join 族臂（`output_columns` 逐 (table, column) 查声明类型 + 投影裁剪）。语义与 Plan 描述的「按表名查 `TableMeta` 声明类型」一致；属可直接定位的路径差异，非实质偏离。
3. **JOIN 别名形态的列名兜底（Plan 未指定）**：`OutputColumn.table` 是**声明限定符**——无别名形态即真实表名，直接命中；别名形态（`FROM a AS x`）不可直接查表。Act 增列名兜底：在左右输入的扫描族表集合中按列名匹配（唯一命中取其类型；多个候选类型不一致 → 保持未知）。理由：否则别名 JOIN 的列类型恒为未知，与「列元数据对嵌入使用者可用」的目标相悖。
   - **〔修复轮更正〕本条对 `OutputColumn.table` 的描述有误**：该字段在计划构造三处均写入**真实表名**（`table_alias` 同值），声明限定符不落入计划节点——「别名形态不可直接查表」不成立，别名形态走直查即命中真实表。本条同时是 Plan Review F2 的前提来源（见 Deviations 6）；兜底代码保留（不可达分支），语义已按一致性改造。
4. **M19 分批执行的环境约束（Plan 写「8-12 目标/批」）**：本机 WSL 内存 7.6 GiB / 32 核。一次性并行编译 126 个测试二进制并以默认并行度跑测试导致内存耗尽（WSL 崩溃，首轮尝试失败）。Act 改用限并发编译（`cargo build --tests -j 2`）+ **12 批 × 每批 ≤12 目标 × 批内 `--test-threads=1` × 批间串行** + 可用内存 <1.4 GiB 时等待回收的内存守卫。分批粒度落在 Plan 的 8-12 目标/批区间内，串行纪律按 M19 执行。
5. **`cargo fmt` 校准**：迁移引入的换行偏差经 `cargo fmt` 收敛（全仓 `cargo fmt --check` 零 diff；格式化后重跑定向套件全绿，行为零差异）。
6. **修复轮：F2 的前提与实际代码不符 → 先行测试落点由入口面改为解析器层**（Plan Review 分类修正为「Plan 错误」）：F2 描述「别名形态限定符不可解析 → 候选首个命中」并给出 `SELECT x.k FROM t1 JOIN t2 AS x` 预期「元数据报 INT、实际值 TEXT」。实际核对：`OutputColumn.table` 在三处构造点（`src/parser/planner/query.rs:695` / `:710` / `:3476`）均写入**真实表名**（`table_alias` 同值——`tests/self_join_test.rs` 与 MS20 归档 carrier 记载「构造恒真实名」），声明限定符只存在于未落入计划节点的 `output_qualifiers` 平行向量。故别名形态的元数据解析走**限定符直查**并命中真实表——实测四类形态（别名→后位表 / 别名→前位表 / 自连接 / 裸名唯一命中）元数据与实际行值类型全部一致，**F2 所述症状未复现**；F2 的前提源自 Deviation 3 对 `OutputColumn.table` 的错误描述（本条同时更正 Deviation 3）。
   - 处理：一致性语义改造照 Review 要求实施（去掉首个候选冒充声明来源的隐患，代码与自身文档一致）；Review 规定的入口断言「该形态 `column_type` 为 None」**不予照写**——按 Review 自身关闭条件的前半句「限定符可解析时直取声明类型」，该形态属可解析，断言 None 会把准确的声明类型（TEXT）降级为未知。
   - 先行测试改落**解析器层**（`src/pipeline.rs` 内建单测，直接构造冲突候选槽）：该层即改动点本身，且在旧实现下真实观察到 RED（`[Declared(Int)]`，断言失败）→ 恢复一致性语义后 GREEN；入口面另保留一条见证锁定「别名形态元数据与实际值一致」的诚实行为。
7. **修复轮：分批粒度 11 批（原轮 12 批）**：分批规则不变（字典序、每批 ≤12 目标、批内 `--test-threads=1`、批间串行、内存 <1.4 GiB 等待守卫）；126 目标按 ≤12 分组得 10×12 + 1×6 = 11 批（原轮为 12 批、各批目标数不等）——均在 Plan 的 8-12 目标/批区间内。

### Gate 3 自检

- **Spec compliance**
  - R1「SELECT 返回原生值与列元数据」：查询经 `execute_sql` 返回 `Ok(SqlOutcome::Rows)`，行为**原生 `Value` 行**（Int / Text / Float / Bool / Date / Timestamp 断言）+ 逐列元数据（列名与 SELECT 清单一致、`SELECT *` 全列序）——`tests/typed_api_test.rs` 6 条见证。
  - R1「DML 受影响行数」：`Ok(SqlOutcome::AffectedRows(n))`，INSERT / UPDATE / DELETE 计数断言——1 条见证。
  - R1「错误载荷同源」：未知列 message 含列名 + 42703、未知表 42P01、语法错 42601——1 条见证。
  - R1「表达式列元数据」（T3）：声明类型 / 日期族 / 空结果仍报声明类型 / 表达式值推导 / 表达式无行保持 None / 全 NULL 保持 None / 聚合值推导 / JOIN / 别名 JOIN——9 条见证。
  - R2「核心/协议解耦」：`grep -rn "network::protocol" src/database.rs src/pipeline.rs src/executor/ src/storage/ src/transaction/` → **零命中**；`src/result.rs` 的 `use` 仅 `executor::Value` 与 `storage::ColumnType`。
  - R3「边界转换输出零变化」：`src/cli/render.rs`、`src/network/pg_protocol.rs`、`src/executor/` 全部**零改动**（`git status` 空）；`src/network/protocol.rs` diff 为 +75 / −0（既有词表与 serde 序列化零改动）；`value_to_json` 转换逻辑逐字节迁入（2 条单测锁定）。
  - Preserve 面：plan cache 位点（get / put / clear）零改动；执行器 `next()` 协议与全部执行器实现零改动；ATTACH 会话面零改动；多语句 / 只读 25006 / 附加库写 0A000 的拒绝语义与优先级零改动（`tests/cross_db_dml_test.rs` 覆盖 25006 优先于 0A000 的形态，迁移后原断言通过）。
  - Forbidden 面：未保留旧签名或兼容层（`Database::execute_sql` 旧签名已消失）；`tests/` 断言语义未改（仅调用点包转换）。
- **Code quality**
  - 改动面 = `src/result.rs`（新）+ `src/pipeline.rs` + `src/database.rs` + `src/network/protocol.rs` + `src/network/handler.rs` + `src/cli/mod.rs` + `src/cli/lifecycle.rs` + `src/storage/integrity.rs` + `src/transaction/session.rs` + `src/lib.rs`（src 9 文件）+ `tests/` 81 文件 + `benches/` 8 文件（修复轮新增 `benches/common/mod.rs`）+ `examples/bench_minimal.rs`。无计划外修改。
  - `cargo build --lib --tests` **零警告**；`cargo fmt --check` 全仓**零 diff**。
  - 无身份型证据机制、无判定层新增；无新增 Hash / 指纹 / revision pin / run-id 类机制。
  - 测试不会因错误原因通过：T3 的 6 条 RED 断言在实现前确实失败于「Some 处实际 None」（RED 形态已采信并留痕），实现后全部 GREEN。
  - `let _ = …` 仅为消除 `#[must_use]` 警告，语义与原「忽略 Response」一致（不新增断言、不改断言语义）。
- **已修复**：Deviation 5（rustfmt 校准）；T3 编译期的 `OutputColumn` 未导入、Join/NestedLoopJoin 无 `projection` 字段、async 递归装箱三处（实现期定位，落在 T3 契约内）。
- **遗留 Minor**（不阻塞，交 Plan Review 处置）
  1. **M37 编号悬空**：`design.md` D6/R1、本 Cycle Plan Context 的 Risks and Notes、`tasks.md` Iteration 001 段三处引用的 **M37**（`ParameterExpression::evaluate_ref` 对 String/Date/Timestamp 降级 Null）在 `.agents/memory/project-model.md` 中**不存在**——现役 M 编号止于 M19，M37 属旧编号体系。限制的实质内容已内联写明，不构成实施阻塞；编号需 Plan 侧修正或补录（Act 不改 Plan Context）。
  2. **测试二进制编译并发未受 M19 约束**：M19 只约束「测试执行」的分批串行，未覆盖「测试二进制编译」阶段；本机 7.6 GiB 内存下 32 路并发链接 126 个测试二进制会耗尽内存。属 Runbook 面（见 Experience Candidates），不阻塞本 Cycle。
- **修复轮自检（R-a / R-b）**
  - **G1 关闭**：`cargo check --benches --examples -j 4` → 零错误、零 unused-Result 警告（`benches/sqlite_compare.rs` 与 `benches/common/mod.rs` 均在该检查集内——以 `-v` 输出确认 `--crate-name sqlite_compare` 实际参与编译）。既有 `Response` 词表与 serde 序列化零改动（`src/network/protocol.rs` diff 仍为 +75 / −0）。
  - **G2 关闭**：解析器层单测在旧实现下真实 RED（`[Declared(Int)]`——正是「首个候选冒充声明来源」的错型形态），一致性语义落地后 GREEN；既有 16 条 typed 断言零回归（`tests/typed_api_test.rs` 17/0）。
  - **Spec compliance**：R1 的四场景与 R3 的零变化面均未受修复影响——修复只改「声明来源不可判定时如何表达」，不改变结果行、列名、错误载荷与边界转换。R2 结构断言复验：核心路径 `network::protocol` 零命中；`src/cli/render.rs`、`src/network/pg_protocol.rs`、`src/executor/` 仍零改动。
  - **Code quality**：新增 `ResolvedType` 三态 + `load_cached_meta` 缓存助手，无新增依赖、无身份型证据机制、无判定层；`cargo build --lib --tests -j 2` 零警告、`cargo fmt --check` 零 diff。
  - **已修复**：G1（benches 面编译破损）、G2（候选首个命中冒充声明来源）。
  - **遗留 Minor**（不阻塞）：Plan Review F3——`execute_stage_routed` / `execute_stage_in_tx_routed` 对 DML / DDL / CTAS 臂也预计算 `output_columns` / `output_types` 后弃用（无害浪费，Review 已明示不要求返工）；修复轮新增同一处的一处次要浪费（`Conflicted` 槽的候选表元数据查询）。
- **Experience Candidates**：2 项（Runbook——M19 分批回归的编译与执行纪律；Runbook——bench / example 编译面须纳入「全目标」检查），见下。

### 验证（全部真实运行）

- **T1 先行测试（RED）**：`cargo test --lib result` → 编译失败 13 处（`to_response` 未定义、`ColumnMeta` / `SqlError` / `SqlOutcome` 未解析）——RED 形态：模块不存在。
- **T1 GREEN（定向）**：`cargo test --lib result` → 2 passed / 0 failed；`cargo test --lib protocol` → 5 passed / 0 failed（含既有 2 条 JSON 序列化见证）。
- **T2 先行测试（RED）**：`cargo test --test typed_api_test` → 编译失败（`no method named unwrap found for enum Response`、`mismatched types`：`Response` vs `Result<SqlOutcome, SqlError>`）——RED 形态：新签名不存在。
- **T2 GREEN（定向）**：`cargo test --test typed_api_test` → 6 passed / 0 failed（T2 阶段断言集）。
- **T3 先行测试（RED）**：`cargo test --test typed_api_test` → **6 FAILED / 3 passed**——失败点全部为「断言 Some 处实际 None」（`column_meta_carries_name_and_declared_type`、`empty_result_keeps_declared_column_types`、`declared_datetime_types_come_from_table_metadata`、`declared_column_types_come_from_table_metadata`、`expression_column_type_is_derived_from_values`、`aggregate_column_type_is_derived_from_values`），与 Plan 预设 RED 形态一致。
- **T3 GREEN（定向）**：`cargo test --test typed_api_test` → **16 passed / 0 failed**（含新增 JOIN、别名 JOIN 与 `COALESCE` 字符串表达式三条）。
- **库单测**：`cargo test --lib` → **412 passed / 0 failed / 0 ignored**。
- **全量（M19 分批一次，最终代码）**：12 批 = 126 个集成目标按**字典序**分组，每批 ≤12 目标，批内 `cargo test --test <name> -- --test-threads=1`、**批间串行**、无默认并行调用；前置 `--lib` 与 doctest 单独串行执行。**全部批次 rc=0**，合计 **2116 passed / 0 failed / 2 ignored**（= lib 412 + doctest 39 + 集成 1665；ignored 2 与基线一致）。该次运行后仅追加 `tests/typed_api_test.rs` 一条 `COALESCE` 见证（产品代码零改动——`git diff --stat -- src/` 仍为 9 files / 676+ / 193−，与本轮运行面逐字节一致），追加后定向重跑该目标 16/0，全量口径相应为 **2117 passed / 0 failed / 2 ignored**。目标覆盖经 ran-vs-want 清单 diff 核对：**ran 126 = want 126、唯一 126——不重不漏**。分批计数：
  | 批 | 目标数 | 范围（字典序） | passed/failed/ignored |
  |---|---|---|---|
  | 1 | 12 | aggregate_extension_test … catalog_durability_test | 153/0/0 |
  | 2 | 10 | chain_join_test … constraint_enforcement_test | 206/0/2 |
  | 3 | 10 | cross_db_dml_test … default_precheck_test | 137/0/0 |
  | 4 | 11 | derived_scan_alias_test … file_storage_io_test | 184/0/0 |
  | 5 | 11 | full_table_dml_test … key_type_conformance_test | 102/0/0 |
  | 6 | 10 | keyless_eq_routing_test … network_protocol_test | 75/0/0 |
  | 7 | 11 | network_server_test … plan_cache_test | 115/0/0 |
  | 8 | 11 | plan_exec_test … recovery_replay_convergence_test | 202/0/0 |
  | 9 | 8 | recovery_tail_persistence_test … secondary_index_equality_test | 74/0/0 |
  | 10 | 11 | secondary_index_test … subset_insert_test | 185/0/0 |
  | 11 | 11 | sync_loader_test … version_chain_test | 149/0/0 |
  | 12 | 10 | view_test … write_type_conformance_test | 83/0/0 |
  | — | lib | 单元测试 | 412/0/0 |
  | — | doctest | 文档测试 | 39/0/0 |
  | | **合计** | **127 次调用 / 126 集成目标** | **2116/0/2**（追加 1 条见证后 2117/0/2） |

  基线核对：MS30 收尾基线 2057（不含 doctest）+ 本次新增 21 条（`src/result.rs` 2 + `src/network/protocol.rs` 3 + `tests/typed_api_test.rs` 16）= 2078 = lib 412 + 集成 1666 ✓；doctest 39 条为既有常量。
  - **〔修复轮更正〕doctest 计数有误**：修复轮实测 `cargo test --doc` → `Doc-tests rtsql` **0 用例**（`src/` 内无 rustdoc 代码块，crate 无 `include_str!` 文档引入）——原轮「doctest 39 条为既有常量」的归因不成立。原轮实测成立的成分为 lib 412 与集成 1665（分批表逐批计数），原轮全量口径应为 **2077**（= 412 + 1665，追加 `COALESCE` 后 2078），而非 2116 / 2117。修复轮的全量口径按实测记录（见下）。
- **构建 / 格式化**：`cargo build --lib --tests -j 2` → 零警告；`cargo fmt --check` → 全仓零 diff。
- **结构断言（R2）**：`grep -rn "network::protocol" src/database.rs src/pipeline.rs src/executor/ src/storage/ src/transaction/` → 零命中；`git status --porcelain -- src/cli/render.rs src/network/pg_protocol.rs src/executor/` → 空。

#### 修复轮验证（Plan Review F1 / F2 关闭，2026-10-21）

- **R-a GREEN（G1 关闭条件）**：`cargo check --benches --examples -j 4` → **零错误、零 unused-Result 警告**（唯一输出为 `~/.cargo/config` 弃用告警——环境项，Review 明示不计）。检查集确认：`cargo check --benches --examples -j 4 -v` 输出含 `--crate-name sqlite_compare`（及 `data_scan_bench` / `micro_bench` / `visibility_bench`），证明 bench 目标确在编译面内（原 F1 的暴露缺口即在此）。
- **R-b 先行测试（RED，解析器层）**：临时将候选匹配回退为「首个候选命中」后 `cargo test --lib type_slot_resolution_requires_candidate_agreement` → **FAILED**，失败信息 `[Declared(Int)]`（冲突候选被解析为声明类型——正是 F2 所述错型形态）。恢复一致性语义后同一命令 → **1 passed / 0 failed**。
- **R-b 冲突槽语义**：`cargo test --lib conflicted_column_type_is_not_derived_from_values` → **1 passed / 0 failed**（冲突槽保持未知且不参与值推导；不可推导槽仍由值推导兜底）。
- **R-b 入口面回归**：`cargo test --test typed_api_test -j 2` → **17 passed / 0 failed**（既有 16 条零回归 + 新增 1 条别名 JOIN 见证）。
- **库单测**：`cargo test --lib -j 2` → **414 passed / 0 failed / 0 ignored**（基线 412 + 本轮新增 2 条解析器层单测）。
- **全量（M19 分批一次，修复后最终代码）**：11 批 = 126 个集成目标按**字典序**分组，每批 ≤12 目标（10×12 + 1×6），批内 `cargo test --test <name> -- --test-threads=1`、**批间串行**、无默认并行调用；限并发编译（`cargo build --lib --tests -j 2`）+ 可用内存 <1.4 GiB 时等待守卫。**全部批次 rc=0，失败目标数 0**，合计 **集成 1667 passed / 0 failed / 2 ignored**（ignored 2 与基线一致）。
  - 逐批范围：1 aggregate_extension_test … catalog_durability_test ｜ 2 chain_join_test … cross_db_query_test ｜ 3 ctas_test … e2e_test ｜ 4 encryption_test … index_manager_test ｜ 5 insert_column_list_test … mvcc_commit_test ｜ 6 mvcc_record_test … pg_messages_test ｜ 7 pg_protocol_test … recovery_e2e_test ｜ 8 recovery_error_surface_test … select_distinct_test ｜ 9 self_join_test … table_manager_test ｜ 10 truncate_test … wal_buffer_test ｜ 11 wal_handle_test … write_type_conformance_test（6 目标）。
  - **全量口径**：lib 414 + doctest 0（`Doc-tests rtsql` 无用例，见上方更正）+ 集成 1667 = **2081 passed / 0 failed / 2 ignored**。与基线对账：MS30 收尾基线 2057 + 原轮新增 21 条（`src/result.rs` 2 + `src/network/protocol.rs` 3 + `tests/typed_api_test.rs` 16）= 2078（原轮末态 = lib 412 + 集成 1666；原轮写成 2117 是多算了 39 条不存在的 doctest）+ 本轮新增 3 条（lib 单测 2 + `typed_api_test.rs` 1）= **2081** ✓ 与实测逐项相加吻合（lib 414 + 集成 1667）。
- **构建 / 格式化**：`cargo build --lib --tests -j 2` → 零警告；`cargo fmt --check` → 全仓零 diff。
- **结构断言（R2 / R3 复验）**：`grep -rn "network::protocol" src/database.rs src/pipeline.rs src/executor/ src/storage/ src/transaction/` → 零命中；`git status --porcelain -- src/cli/render.rs src/network/pg_protocol.rs src/executor/` → 空；`git diff --stat -- src/network/protocol.rs` → +75 / −0（既有词表与 serde 序列化零改动）。
- **文档注释校准后的定向复核**：两处文档注释（`attach_column_metadata` 的函数名引用、`derive_types_from_rows` 的作用域表述）随本轮实现校准，属非行为面改动；按公共规则「被覆盖表面变化后只做定向审计，不重跑全量」执行：`cargo fmt --check` 零 diff + `cargo build --lib --tests -j 2` 零警告 + `cargo test --lib` 414/0/0 + `cargo test --test typed_api_test` 17/0/0 + `cargo check --benches --examples -j 4` 零错误零警告。

### Persisted Evidence

None required（计划 Mode: none；全部验证为 `cargo test` 集合内行为断言 + 单步 grep / git 管道，可低成本重跑；无一次性环境、无 Issue / Blocker 现场保留需求；Act Response 承载命令与逐批计数足够）。

### Experience Candidates

1. **Runbook 候选——M19 分批回归的编译与执行纪律**：M19 约束「测试执行」分批串行，但未覆盖**测试二进制编译**阶段。实测（本机 WSL 7.6 GiB / 32 核）：一次性并行编译 126 个集成测试二进制并以默认并行度跑测试 → 内存耗尽、WSL 崩溃（首轮尝试失败，无残留结果）；改用 `cargo build --tests -j 2` + 12 批 × ≤12 目标 × 批内 `--test-threads=1` × 批间串行 + 可用内存 <1.4 GiB 时的等待守卫 → 连续两轮（T2 后、T3 后）均 0 failed 稳定通过。属「已端到端验证成功 + 可重复 + 风险较高」的操作路径，值得固化为 Runbook。证据：本 Act Response 的「验证（全部真实运行）」全量分批表与 Deviation 4。
2. **Runbook 候选——签名替换后须覆盖 bench / example 编译面**：F1 的暴露缺口是「验证命令只覆盖 `--lib --tests`，未覆盖 bench / example 目标」，致 `benches/sqlite_compare.rs` 的 E0308 在两轮 Act 后仍不可见（`cargo bench` / `cargo check --all-targets` 在该工作区不可用）。可复用路径：签名替换类改动的验证清单须含 `cargo check --benches --examples -j 4`（零错误 + 零 unused-Result 警告）并用 `-v` 确认目标确在编译集内。证据：本 Act Response「修复轮验证」的 R-a GREEN 条目与 Plan Review F1。

### 未解决问题

- **F2 的前提与实际代码不符——请 Plan Review 复核**（修复轮发现）：F2 所述「别名指向后位表 + 前位候选表同名列异型 → 元数据报 INT、实际值为 TEXT」在当前代码**未复现**——`OutputColumn.table` 承载真实表名，别名形态走限定符直查即命中真实表；实测四类形态元数据与实际值类型全部一致（证据见 Deviations 6 与修复轮验证）。本轮已按关闭条件实施一致性语义改造（去掉「首个候选冒充声明来源」），先行测试落解析器层（旧实现下真实 RED）。**F2 的入口断言未照写**——按其自身关闭条件前半句「限定符可解析时直取声明类型」，该形态属可解析，断言 None 会把准确声明类型降级为未知。请 Review 复核该判定：接受解析器层锁定 + 入口面诚实行为见证，或另行裁定。
- **M37 编号悬空**（Gate 3 Minor 1）：`design.md` D6/R1、本 Cycle Plan Context、`tasks.md` Iteration 001 段引用的 M37 在 project-model 中不存在（M 编号止于 M19）。Act 不改 Plan Context 与 design——请 Plan Review 处置（修正编号或补录模型条目）。不影响 Iteration 000 的任何验收。
- **SNAPSHOT 仓库现场过期**（范围外，交维护者）：`.agents/docs/SNAPSHOT.md` 的「仓库现场」段仍写基线 `34a3ae7` / ahead 4 / 「MS30 交付现场未提交」，实际 HEAD 已是 `671f25c`（MS30 已提交）、ahead 5；「权威文档」段仍写「无活跃变更」（现有 1 个）；「目录约定」写 tests 126 个文件（实际 125 + 本次新增 `typed_api_test.rs` = 126）。Act 不维护全局文档（阶段边界），留待 `openspec-docs-maintainer` 收尾时刷新。

## Plan Review

- Review Result: accepted（2026-10-22 复审——R-a/R-b 修复轮独立核对通过，F2 前提复核请求裁定 Act 判定成立；本节为最新完整反馈，覆盖 2026-10-21 首轮 Review。首轮 F1-F5 与 Follow-up Decision 的原文要点并入下文处置结论，完整过程见 Act Response Deviations 4-7 与「修复轮验证」段）

**Findings（复审）**

- **G1/F1 关闭确认（R-a）**：独立读 diff——`benches/sqlite_compare.rs:154` 包装方法返回 `std::result::Result<rtsql::SqlOutcome, rtsql::SqlError>`；18 处 `rtsql.execute(…)` 丢弃点 + `:747` `block_on` 均已包 `let _ =`；`benches/common/mod.rs` 6 处齐全。`:732` 的 `block_on` 为 `create_table(...).await.unwrap()` 块内消费，非 Result 丢弃点——无遗漏。采信修复轮 `cargo check --benches --examples -j 4` 零错误零警告（来源：Act Response 修复轮验证；其后 bench 面与产品代码零改动——本轮 diff 复核一致）。
- **G2/F2 关闭确认（R-b）**：`resolve_type_slots`（`src/pipeline.rs:1684-1755`）与关闭条件逐句吻合——限定符可解析直取（:1703-1716）；不可解析按列名匹配候选表，含该列的全部候选声明类型一致才 `Declared`，否则 `Conflicted`（:1717-1752）；冲突槽经 `derivable` 掩码排除值推导（`attach_column_metadata:1379-1383` + `derive_types_from_rows:1406-1410`）。两条解析器层单测（`src/pipeline.rs:3411` / `:3462`）锁定三态语义，RED 形态与旧实现「首个候选命中」错型一致（`[Declared(Int)]`）；入口见证 `aliased_join_column_type_matches_the_actual_value`（`tests/typed_api_test.rs:386`）锁定别名指向后位表时报告**该表**声明类型且与行值一致。采信修复轮定向与全量验证（lib 414/0、typed_api 17/0、全量 2081/0/2）。
- **F2 前提复核请求——Act 判定成立（偏差重分类：Plan 错误）**：独立核对 `OutputColumn.table` 三处构造点（`src/parser/planner/query.rs:695` / `:710` / `:3476`）均写 `Some(真实表名)` 且 `table_alias` 同值；`self.tables` 以真实表名为键（`src/parser/planner/mod.rs:149`），别名经 `register_table_alias`（mod.rs:305）独立映射——基表别名形态限定符恒可解析，F2 所述「元数据报 INT、实际值 TEXT」症状在入口层不可达；首轮 F2 的症状前提源自 Deviation 3 对该字段的错误描述（Act 已在 Deviation 6 自行更正）。**入口 None 断言拒绝照写正确**：该形态按 G2 关闭条件前半句属「限定符可解析」，断 None 会把准确的声明类型降级为未知；解析器层 RED 锁定 + 入口诚实行为见证构成本 Gap 的合法关闭。首轮 F2 中真实存在的缺陷面（候选首个命中冒充声明来源）已由 R-b 消除。
- **F4 确认（Plan 侧已处置）**：`design.md:28` 已改描述性表述并注明「M37」为 predicate.rs 注释沿用的旧编号、现役 project-model 无此条目；tasks.md 无残留 M37 引用。
- **F3（非阻塞，维持记录）**：`execute_stage[_in_tx]_routed` 对 DML/DDL/CTAS 臂预计算 `output_columns`/`output_types` 后弃用——无害浪费，首轮已明示不要求返工。
- **F5（非阻塞，范围外，维持）**：SNAPSHOT 仓库现场/权威文档段过期——交 `openspec-docs-maintainer` 收尾刷新。
- **MF-1（新，非阻塞 Minor）**：`join_type_slots` 文档注释（`src/pipeline.rs:1600-1602`）仍表述「`OutputColumn.table` 是声明限定符——别名形态不可直接查表」，与 Deviation 6 更正后的事实不符（基表别名形态恒载真实表名；兜底的真实可达形态是 CTE/视图 effective 限定符与 `None` 无限定形态）。纯文档准确性问题，无行为影响；后续 Cycle 合法触达该函数邻域时可顺带校准，不构成返工事由。
- **MF-2（新，非阻塞 Minor）**：Act Response「（src 9 文件）」实际列出 10 个文件（9 个已跟踪修改 + `src/result.rs` 新增；`git diff --stat -- src/` 的 9 files 只计已跟踪修改）。计数口径偏差，无实质影响。

**Acceptance Gaps**

None——G1/G2 均关闭；R1 四场景 / R2 解耦 / R3 零变化 / 全量零回归在首轮已独立核对满足，修复轮未触及结果行、列名、错误载荷与边界转换面。

**Evidence**

- 独立读 diff：`benches/sqlite_compare.rs`（签名 + 19 处丢弃点）、`benches/common/mod.rs`（6 处）、`src/pipeline.rs` R-b 全部函数体（`ResolvedType` / `resolve_type_slots` / `load_cached_meta` / `derive_types_from_rows` / `attach_column_metadata` / `join_type_slots` / `collect_scan_tables`）与两条单测。
- F2 前提独立核对：`src/parser/planner/query.rs` 三处构造点 + `mod.rs:149`（tables 键）/ `:305`（别名映射）；`benches/sqlite_compare.rs:732` block_on 消费形态核对。
- 改动面核对：`git diff --stat` 99 个已跟踪修改 + 3 个未跟踪（`src/result.rs`、`tests/typed_api_test.rs`、change 目录）与 Act 声明一致；`src/network/protocol.rs` +75/−0；`src/cli/render.rs`、`src/network/pg_protocol.rs`、`src/executor/` 零改动（git 核对）；R2 结构断言静态复核（grep 核心路径 `network::protocol` 零命中）。
- `tests/typed_api_test.rs` 17 条测试清点（含 R-b 新增见证）；`cargo test --lib` 414 与两条新单测在位相符。
- 采信 Act Response 修复轮验证（其后代码零变化，本轮 diff 复核一致）：`cargo check --benches --examples -j 4` GREEN、`cargo test --lib` 414/0、typed_api 17/0、全量 M19 分批 11 批 2081/0/2、fmt/build 零告警。

**Follow-up Decision**

G1/G2 修复经独立核对满足关闭条件，F2 复核请求裁定 Act 判定成立，无新的阻塞 finding——Review Result 置 **accepted**，Iteration 000 完成。MF-1/MF-2 仅记录不返工。按 Iteration Map 展开下一逻辑 Iteration 001（Plan Context 状态 draft，等待用户批准后转 ready）。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

`iterations/001-placeholder-param-binding/000-initial.md`（2026-10-22 展开，Plan Context 状态 draft）
