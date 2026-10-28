# Iteration 000 / Cycle 000: 诊断门面

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS44 的采集口径中，本轮只建立门面、会话槽、三项配置、上限和父子耗时。行为以本文件的 Task Definition 为准。
- Excluded scope: 语句管道接线、WAL 与文件读页、`RTSQL_PROFILING` 旧表、既有 stderr 告警迁移、开销测量、全量回归、尚未发现的缺陷修复。

**Objective**

门面可以按配置记录阶段汇总数、丢弃超出的事件，并算出父子耗时。默认会话执行 `SELECT 1` 仍成功，且不产生摘要。

**Investigation Facts**

- Starting point: 分支 `changes/2026-10-28-ms44-unified-diagnostics`。`src/` 下没有 `diagnostics` 模块。`src/lib.rs` 导出 `profiling`，不导出诊断门面。`src/profiling.rs` 的 `record_time` 在 task-local 未设置时 panic；`is_profiling_enabled` 只判断环境变量 `RTSQL_PROFILING` 是否存在。本轮不改这些函数的行为。
- Current-State Evidence: `Database` 在 `src/database.rs` 派生 `Clone`。`open` 为 `Database::open(path)`，`execute_sql` 为 `Database::execute_sql`。`tests/checkpoint_skip_test.rs` 已调用 `execute_sql("SELECT 1")`。直接写 stderr 的产品调用点有 34 个，本轮一个都不改。变更前不跑测试：新增文件在当前树不存在，没有可沿用的通过结果。
- Code and Critical Path: 新文件 `src/diagnostics.rs`。`src/lib.rs` 增加 `pub mod diagnostics`。`src/database.rs` 增加私有的 `Arc` 会话槽，以及 `set_diagnostics` 和 `diagnostic_summaries`。会话槽随 `Clone` 共享。诊断模块不引用 `database`、`pipeline`、`wal`、`storage`。

**Implementation Guidance**

用一个公开函数接受已经量好的包含耗时，集成测试直接把 100、70、40 传进去。`scope` 结束时走同一套计算。不要在本轮把 `scope` 接到 `execute_inner`。渲染函数返回字符串：`Detail::Off` 为空，`Summary` 只有汇总数，`Full` 才追加事件行，非空文本以 `diagnostics ` 开头。`report` 不写 stderr。

**Task Definitions**

### T1: 门面按配置记录并限制摘要

- Requirement/Scenario: 「详细程度、观察类别和采集内容分开配置」；「语句阶段与父子耗时」中的固定包含耗时；「输出限量与慢输出」中的两条上限。真实 SQL 阶段、等待类别和取消行可见性不在本任务。
- Depends on: None
- Targets: `src/diagnostics.rs`（新建）、`src/lib.rs` 的模块导出、`src/database.rs` 的私有会话槽与 `set_diagnostics` / `diagnostic_summaries`、`tests/diagnostics_facade_test.rs`（新建）
- Current behavior: 没有诊断配置、摘要或上限。`execute_sql("SELECT 1")` 在文件库上可以成功。
- Required behavior: `rtsql::diagnostics` 提供公开函数，供集成测试传入已经量好的包含耗时。调用方可以分别设置 `Detail`（`Off` / `Summary` / `Full`）、`ModuleSet`（`All` 或只含 `pipeline` / `wal` / `storage` / `index`）和 `Collect` 的 `events`、`stats`、`trace`。`stats` 为真且 `events`、`trace` 为假时，摘要有阶段汇总数，没有逐条追踪样本。`Detail::Off` 时摘要仍可从会话槽读回，渲染字符串为空。观察类别不含 `pipeline` 时，不记录 `parse` / `plan` / `execute`。`event_limit` 默认 64，`summary_limit` 默认 32，SQL 保留 80 个字符。超出事件上限后 `dropped` 大于 0，汇总数仍在。摘要超过上限时只留最新的。根包含耗时 100、执行包含耗时 70、提交子阶段包含耗时 40 时，总耗时为 100，执行独占耗时为 30。成功结束的严重级别是 Info，失败是 Error，未结束被丢弃是 Warn。公开记录没有操作标识、运行标识或内容哈希字段。默认配置下 `execute_sql("SELECT 1")` 成功，`diagnostic_summaries` 为空。
- Preserve: `Database::open` 与 `execute_sql` 的结果类型。`Clone` 仍共享原有会话状态。`profiling` 的环境变量开关和 panic 条件。现有 stderr 调用点的文本。
- Forbidden: 修改 `src/pipeline.rs`、`src/profiling.rs`、`src/wal/`、`src/storage/`、`src/transaction/`、`src/executor/`。新增 crate。新增 CLI 或 SQL 语法。给记录加操作标识、运行标识或内容哈希。在 `report` 里写 stderr。把本轮扩展成语句阶段接线。跑全量 `cargo test`。
- First test: `tests/diagnostics_facade_test.rs` 的 `stats_without_trace_records_totals`。先只加这个测试。RED 命令是 `cargo test --offline --test diagnostics_facade_test stats_without_trace_records_totals`。预期结果：退出码非 0，编译失败，原因是诊断类型或模块还不存在。看到该失败后再写实现。
- GREEN condition: 同一文件补齐下列测试后，`cargo test --offline --test diagnostics_facade_test` 退出码 0，且输出里的 failed 为 0。测试为：`stats_without_trace_records_totals`、`detail_off_keeps_summary_and_renders_empty`、`module_filter_skips_pipeline_stages`、`root_total_is_not_parent_plus_child`、`event_limit_increments_dropped`、`summary_limit_drops_oldest`、`sql_text_truncates_at_80`、`public_record_has_no_identity_field`、`ended_scope_uses_info_error_warn`、`default_session_execute_has_empty_summaries`。
- Verification: 上一条 GREEN 命令，默认并行度。通过表示门面口径成立。失败表示配置、上限或耗时公式与本任务不符。`default_session_execute_has_empty_summaries` 使用 `tempfile` 的临时目录、`Database::open` 和 `execute_sql("SELECT 1")`，断言 `Result` 为成功且摘要为空。
- Stop when: 要让上述测试通过，就必须修改管道、WAL、存储，或给公开记录增加标识字段。这时停止，不继续改那些文件。

**Invariants**

诊断模块不依赖数据库各层。采集全关时不分配摘要。`report` 不写 stderr。公开摘要不含操作标识。默认 `SELECT 1` 的结果不因新字段而失败。

**Non-goals**

不在本 Cycle 记录真实语句的 parse/plan/execute，不区分等待类别，不比较三种开销，不迁移告警，不跑全量。

**Acceptance**

- `stats_without_trace_records_totals` 证明只开统计时有汇总数、无逐条样本。
- `detail_off_keeps_summary_and_renders_empty` 证明输出关闭仍可读回，且渲染为空。
- `module_filter_skips_pipeline_stages` 证明类别过滤。
- `root_total_is_not_parent_plus_child` 使用 100、70、40，总耗时 100，执行独占 30。
- `event_limit_increments_dropped` 与 `summary_limit_drops_oldest` 证明两个上限。
- `sql_text_truncates_at_80` 证明第 81 个字符被截掉。
- `public_record_has_no_identity_field` 用结构体字面量只填严重级别、类别、消息和数据；编译通过即表示没有标识字段。
- `ended_scope_uses_info_error_warn` 证明三种结束状态的严重级别。
- `default_session_execute_has_empty_summaries` 证明默认会话不产生摘要，`SELECT 1` 仍成功。

**Verification**

本轮场景都在 `tests/diagnostics_facade_test.rs` 内直接调用门面或 `execute_sql`。不解析进程 stderr，不使用诊断字段作为运行身份。命令是 `cargo test --offline --test diagnostics_facade_test`。退出码 0 且 failed 为 0 为通过。

**Gate 2 Readiness**

PASS。检查项均通过，无 BLOCKED，无 SKIPPED。批准来自本次 goal 对计划的授权，不是跳过。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

`SELECT 1` 的成功形态沿用 `tests/checkpoint_skip_test.rs` 的既有调用。本轮不测量它的耗时。渲染前缀 `diagnostics ` 只供以后的语句测试区分新旧 stderr，本轮只断言 `Off` 得到空字符串。

## Act Response

- Status: reported
- Task：T1（change tasks.md 1.1）

### 实际改动

- 新建 `src/diagnostics.rs`：门面模块。公开类型 `Detail` / `Module` / `ModuleSet` / `Collect` / `DiagnosticsConfig`（默认 event_limit 64、summary_limit 32）、`Record`（仅严重级别、类别、消息、数据四字段）、`ScopeEnd` / `Severity` / `Category`、`StageSample` / `StageStat` / `TraceSample` / `Summary`；公开函数 `render`、`Diagnostics::record_pre_measured`（预量包含耗时入口）、`Diagnostics::statement`（作用域，`end_ok` / `end_err` / 析构未结束按 Dropped）。摘要形成收敛在私有 `Diagnostics::record_statement` 单点：采集全关不分配摘要、事件超限保留最新并计 `dropped`、SQL 截 80 字符、阶段按（类别,阶段名）聚合、独占 = 包含 − 直接子阶段包含之和（saturating）、摘要超限丢最旧。
- `src/lib.rs`：新增 `pub mod diagnostics;`。
- `src/database.rs`：私有字段 `diagnostics: Arc<crate::diagnostics::Diagnostics>`（随 `Clone` 共享，open 内构造），公开方法 `set_diagnostics` 与 `diagnostic_summaries`。`Database::open` / `execute_sql` 签名与行为未动。
- 新建 `tests/diagnostics_facade_test.rs`：GREEN condition 列出的 10 个测试，全部使用固定耗时树（根 100 / parse 10 / plan 5 / execute 70 / commit 40）或门面直接断言；`default_session_execute_has_empty_summaries` 用 tempdir + `Database::open` + `execute_sql("SELECT 1")`。

### 与计划的偏差

无实质偏差。两处记录性说明：测试固定树比 Required behavior 的最小三元组多一个 `plan` 5ms 叶子——`module_filter_skips_pipeline_stages` 需要树中存在 `plan` 才能断言其被过滤；`detail_off_keeps_summary_and_renders_empty` 除断言 Off 渲染为空外，加了同摘要 Summary 档渲染以 `diagnostics ` 开头的对照断言，防止空字符串因渲染恒空而通过。

### 自检发现与修复

- Gate 3（spec compliance + code quality）逐项通过：Preserve 项（open/execute_sql 结果类型、Clone 共享、profiling panic 条件、34 个 stderr 调用点）未被触碰；Forbidden 项（pipeline/profiling/wal/storage/transaction/executor、新 crate、CLI/SQL 语法、标识字段、stderr 渲染、语句接线、全量测试）均未违反；诊断模块 import 只有 `std`，不依赖数据库各层；diff 无计划外修改；无新增编译警告（编译输出仅环境级 `~/.cargo/config` 弃用提示，与本项目无关）；测试断言均为具体值（精确 Duration、精确字符串、精确计数），不存在因错误原因通过。
- 无 Critical / Important 发现；无遗留 Minor 问题。

### 验证命令与实际结果

- RED（先行测试）：`cargo test --offline --test diagnostics_facade_test stats_without_trace_records_totals` → 退出码 101，`error[E0432]: unresolved import rtsql::diagnostics`（诊断模块不存在，符合 Task Definition 预期失败）。
- GREEN（Verification）：`cargo test --offline --test diagnostics_facade_test`（默认并行度）→ 退出码 0，`10 passed; 0 failed; 0 ignored`，覆盖 GREEN condition 全部 10 个测试。
- 未跑全量（全量预算在 Iteration 003 / T4，遵守 M19 分批串行约束）。

### Persisted Evidence

None required（Mode: none，无 required 项，结论可低成本重跑）。

### Experience Candidates

None（未发现 change 范围外实质缺陷；测试命令为项目常规命令，无新 Runbook 价值）。

### 未解决问题

None。

## Plan Review

- Review Result: pending
