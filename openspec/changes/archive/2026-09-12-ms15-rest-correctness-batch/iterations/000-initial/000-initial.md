# Iteration 000 / Cycle 000-initial: CLI 表头与投影行形状一致（I034）

## Plan Context

- Status: ready
- Iteration: 000-initial
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3
- Depends on: None
- Stable baseline: 裸 DataScan / 下推 DataScan 子集投影的 CLI 表头等于投影列名，json `columns`/`rows` 字段数一致；聚合与表达式路径表头零回归；全量回归零修改
- Verification boundary: T2/T3 全绿 + clippy/fmt 0 + `openspec validate --changes` PASS
- Diagnostic boundary: `src/parser/planner/query.rs`（`get_plan_output_columns`）+ `tests/cli_test.rs` 追加用例
- Deferred tasks: T4-T6（Iteration 001）、T7-T9（Iteration 002）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: change proposal 全部范围约束；design D1 节点臂裁定表
- Excluded scope: I037（Iteration 001）、I039（Iteration 002）、IndexScan 臂改动、plan 路由、渲染格式族、I037 邻接 rekey 形态

**Objective**

`SELECT` 子集投影经裸 DataScan（无 WHERE）与下推 DataScan（非 PK WHERE）路径执行时，CLI 表头与行形状一致：`SELECT name FROM s` 输出 `{"columns":["name"],"rows":[["Alice"]]}`；聚合与表达式路径表头零回归；既有测试套件除新增见证与 `tests/cli_test.rs:289` 注释校准外零修改通过。

**Background**

tasks MS15-T02 + improvements I034：spec `cli-noninteractive-shell` R6 S1「表头 ["name"]」的 bare-DataScan 分支自 MS10-T01 起未满足（MS10-T04 Plan Review finding 5 实证，2026-09-09）。json 输出 `columns` 与 `rows` 字段数不一致，机器消费需二次裁剪；表格输出表头错位。本 Iteration 为聚合 change 三域中的第一域。

**Investigation Facts**

- Current Baseline: 工作树 = f9e1e1f + MS15-T01 实施与 docs sync（未提交）；853 tests pass / 0 failed / 2 ignored（2026-09-12 Plan Review 独立复跑）。Act 开始前做 `git status`/`git diff` 基线检查；用户若已 commit MS15-T01 则以新 HEAD 为基线。
- Current-State Evidence:
  - `get_plan_output_columns`（`src/parser/planner/query.rs:23-84`）：Filter 臂（28-40）与 Sort 臂（41-53）应用 `node.projection`；`DataScan` 臂（26）`node.columns.clone()` 不应用投影；`Scan` 臂（25）同；`IndexScanAll` 臂（58）同；`IndexScan` 臂（57）返回已收窄 columns。
  - DataScan 构造点 3 处：no-WHERE 臂（`query.rs:578-587`，`projection: proj_or_empty`）、下推臂（`query.rs:572-581`，`projection: proj_or_empty`）、OR 臂（`query.rs:565-570`，`projection: Vec::new()`，Filter 持有裁剪）。
  - Scan 构造点 2 处（`query.rs:108/166`）恒 `projection: Vec::new()`；IndexScanAll 无构造点（grep 实证，plan 变体死路径）；IndexScan 构造时 `columns` 已收窄（`query.rs:527-533`）且 `projection` 索引指向基 schema——不可再应用（越界/双重裁剪）。
  - 消费面 3 处：CLI 表头（`src/cli/mod.rs:330`）；聚合 `input_schema`（`query.rs:634`，聚合/标量子查询/表达式路径 `projection_indices=None`（`query.rs:464-477`）→ scan projection 恒空 → 恒等）；派生表列注册（`query.rs:129`，修复后登记投影列名 = 行形状）。
  - 投影索引语义：DataScan `projection` 恒为指向 `node.columns`（全 schema）的索引——与 Filter/Sort 臂的 `columns[i]` 映射同构，直接复用同一表达式。
- Code and Critical Path: `PlanBuilder::get_plan_output_columns`（query.rs）→ CLI `run_sql` 表头提取（cli/mod.rs:330）→ `render`（cli/render.rs）四格式输出；plan 由 `plan_stage` 构造（可经 plan cache 复用，columns 每次现算不入缓存）。

**Implementation Guidance**

实现顺序：T1 先写 RED（两个缺陷形态断言 + 一个零回归锚点），确认 RED 后 T2 改 `get_plan_output_columns`，最后 T3 回归收尾。T2 形态建议（非实质细节可由 Act 就地调整）：DataScan/Scan/IndexScanAll 三臂统一套用 Filter/Sort 臂既有模式——

```rust
let mut columns = node.columns.clone();
if !node.projection.is_empty() {
    columns = node.projection.iter().map(|&i| columns[i].clone()).collect();
}
columns
```

IndexScan 臂（57）保持原样，加一行注释说明其 columns 构造时已收窄（防将来误改）。`tests/cli_test.rs:289` 注释按新语义改写。

**Behavioral Change**

- 当前：`SELECT name FROM s`（裸 DataScan）CLI 表头 `["id","name"]`、行 `[["Alice"]]`；下推 DataScan 同病；json `columns`/`rows` 字段数不一致。
- 目标：表头 = 投影列名，列数恒等于行字段数（四格式同契约）。
- 接口：`get_plan_output_columns` 返回值变化（对携带投影的 DataScan/Scan/IndexScanAll 输入）——crate 内 pub(crate)，消费面仅 3 处（Investigation Facts 已列）。
- 错误语义：无新增错误路径；派生表列注册收窄后，外层引用被投影掉列名从错误结果变为列不存在报错（方向正确，见 Risks）。

**Task Contracts**

### T1: RED 测试见证——R1 缺陷形态与零回归锚点

- Requirement/Scenario: R1（delta spec `specs/cli-noninteractive-shell/spec.md`）S「裸 DataScan 子集投影 CLI 表头按投影裁剪」、S「下推 DataScan 子集投影 CLI 表头按投影裁剪」、S「聚合与表达式路径表头零回归」
- Depends on: None
- Targets: `tests/cli_test.rs`（追加测试，沿用 `fixture`/`run_cli` helper）
- Current behavior: `SELECT name FROM s`（json）`columns=["id","name"]`、`rows=[["Alice"]]`；`SELECT s FROM t WHERE n > 5` 同病
- Required behavior: 测试断言 `columns=["name"]`/`["s"]` 且 `columns.len() == rows[0].len()`；聚合（`COUNT(*) AS cnt`）与表达式（`a + 1 AS x`）表头锚点断言（当前已满足，锁定不回归）
- Required changes: 仅新增测试；不修改产品代码
- Preserve: 既有测试断言与意图零修改
- Forbidden: 修改 `src/`；改既有用例
- Test witness: `cargo test --test cli_test`，新增用例缺陷形态断言 RED（表头为全 schema）、锚点断言 GREEN
- GREEN condition: T2 后缺陷形态断言转 GREEN
- Verification: `cargo test --test cli_test <new_test_names>`，退出码 0；失败即契约失效
- Stop when: 新用例无法稳定复现缺陷（表头已正确）或锚点断言在基线上失败（基线与预期不符，返回 Plan）

### T2: R1 实现——scan 臂投影裁剪（design D1）

- Requirement/Scenario: R1 全部场景（含 S「子集投影在全部扫描路径返回投影列」既有断言转真）
- Depends on: T1
- Targets: `src/parser/planner/query.rs::PlanBuilder::get_plan_output_columns`（DataScan/Scan/IndexScanAll 三臂）；`tests/cli_test.rs:289` 注释
- Current behavior: 三臂返回 `node.columns` 全 schema
- Required behavior: 三臂对非空 `node.projection` 按索引裁剪列名（与 Filter/Sort 臂同构）；IndexScan 臂保持并注释防误改
- Required changes: `get_plan_output_columns` 返回值变化（见 Behavioral Change）；注释校准
- Preserve: Filter/Sort/IndexScan/Aggregate/Projection/Join 各臂行为；聚合 `input_schema` 恒等（projection 恒空）；`SELECT *` 与全投影恒等
- Forbidden: 修改执行器（`with_projection` 语义）、plan 路由、IndexScan 臂
- Test witness: T1 用例转 GREEN；`tests/projection_test.rs` 6 用例零修改通过
- GREEN condition: `cargo test --test cli_test --test projection_test` 全绿
- Verification: 同上 + `cargo test --lib parser`（planner 单测含 `test_get_plan_output_columns_join` 零回归）
- Stop when: 裁剪后任一消费面（聚合/派生表）出现形状不匹配错误（实质，返回 Plan）

### T3: R1 回归收尾

- Requirement/Scenario: R1 S「既有测试按投影语义校准」+ R6 零回归面
- Depends on: T2
- Targets: 全量验证命令（无代码修改）
- Current behavior: —
- Required behavior: 全量 853+新增 全绿；clippy/fmt 0；validate PASS
- Required changes: 无代码修改；记录派生表列注册消费面复核结论（外层引用被投影列的行为实测）于 Act Response
- Preserve: —
- Forbidden: 为凑绿修改既有测试（除 `cli_test.rs:289` 注释）
- Test witness: `cargo test`（全量）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate --changes`
- GREEN condition: 全部退出码 0；基线 853 只增不减
- Verification: 同上
- Stop when: 全量出现无法归因于本变更面的失败（返回 Plan）

**Invariants**

- 谓词求值先于投影裁剪（MS10-T01 真投影机制）；`SELECT *` 恒等；plan cache 键语义不变；执行器层零修改；四格式渲染函数（`src/cli/render.rs`）零修改。

**Non-goals**

- I037/I039（后续 Iteration）；IndexScan 臂；IndexScanAll 接线；渲染格式族；plan 路由与 I046/I036 域。

**Acceptance**

R1 delta spec 场景全部满足且既有套件零回归。RTM（Iteration 000 范围）：

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R6（修改） | 裸 DataScan 表头裁剪 | D1 | T1/T2 | 000 | `query.rs::get_plan_output_columns` DataScan 臂 | `cli_test.rs` 新增裸 DataScan 用例 | None | Covered |
| R6（修改） | 下推 DataScan 表头裁剪 | D1 | T1/T2 | 000 | 同上 | `cli_test.rs` 新增下推用例 | None | Covered |
| R6（修改） | 聚合/表达式表头零回归 | D1 | T1 | 000 | Aggregate/Projection 臂（不改） | `cli_test.rs` 新增锚点 | None | Covered |
| R6（修改） | 全扫描路径投影列（既有 S1 转真） | D1 | T2/T3 | 000 | DataScan 臂 | `projection_test.rs` 既有 6 用例 + 全量 | None | Covered |
| R6（修改） | 全 schema 行为不变 | D1 | T2/T3 | 000 | scan 臂恒等面 | 全量既有套件 | None | Covered |
| R6（修改） | 既有测试校准 | D1 | T2/T3 | 000 | `cli_test.rs:289` 注释 | Act Response 校准清单 | None | Covered |

**Verification**

- `cargo test`（全量，基线 853 只增不减）、`cargo clippy --all-targets -- -D warnings`（0）、`cargo fmt --check`（0 diff）、`openspec validate --changes`（PASS）。
- 缺陷形态 CLI 探针：`SELECT name FROM s` json 输出 `{"columns":["name"],"rows":[["Alice"]]}`（决定性输出记入 Act Response，≤20 行）。
- Persisted Evidence 为 none：验证命令与决定性输出写入 Act Response 即可，无不可低成本重跑项。

**Gate 2 Readiness**

- 无 Missing requirement：PASS（RTM 全 Covered，delta spec 7 场景均有 task/代码/测试映射）
- Simplified requirement 已批准：PASS（无 Simplified 项）
- 调查完整：PASS（入口/消费面/构造点/测试入口行级实证，见 Investigation Facts；未确认项仅 serialize 类型校验时序，属 Iteration 001 非实质未知项，不在本 Iteration 面）
- 设计闭合：PASS（节点臂逐臂裁定 + 消费面核查 + 注释校准点定位）
- 任务可执行：PASS（T1-T3 均有位置/行为/见证/停止条件）
- 分轮合理：PASS（平衡审计见 tasks.md；本 Iteration 单域单文件面）
- 追踪完整：PASS（RTM 链路闭合）
- 验证充分：PASS（RED→GREEN + 既有套件零修改 + 全量收尾）
- 无身份型证据工程：PASS（验证直接观察 CLI 输出与测试退出码）
- 无实质未知项/TBD：PASS（实现形态细节明确标注非实质可调）
- OpenSpec 产物一致：PASS（proposal/design/tasks/delta spec/cycle 交叉一致）
- Persisted Evidence 模式明确：PASS（none，无 required 项）
- 用户批准计划：PASS（2026-09-12 用户原话"批准"——Gate 1 需求与范围、三项默认假设（proposal「默认假设」节：① I039 解析侧归一化 ② I037 邻接 rekey 形态排除 ③ 工作区不提交）与 Gate 2 计划一并批准，无推翻项）

Gate 2 结论：全部 PASS（含 1 项由 BLOCKED 转 PASS 的用户批准项），Plan Context 状态改 `ready`，交接 `openspec-act`。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果（命令、决定性输出、退出码）；无满足 Evidence 白名单的不可重跑项。Budget: 不适用。

**Risks and Notes**

- 派生表列注册行为变化：外层查询引用被投影掉的列名从"错误结果"变为"列不存在报错"——若全量出现既有用例锁定旧行为，按 R6 既有「按投影语义校准」先例处理并记录校准清单，不视为回归。
- plan cache 命中的旧 plan 对象在新代码进程中重建，无跨进程持久化，无污染面。
- 若用户在 Act 开始前 commit 了 MS15-T01，基线 HEAD 变化属 BASELINE-CHANGED 非阻塞（验证基线以实施时点实测为准）。

## Act Response

- Status: reported

**Implemented**

- T1（RED 见证）：`tests/cli_test.rs` 追加 3 用例——裸 DataScan（`SELECT name FROM s` json，断言 `columns==["name"]`、行 `[["Alice"]]`、表头/行字段数一致）、下推 DataScan（`SELECT s FROM t WHERE n > 5` json，断言 `columns==["s"]`）、聚合/表达式表头锚点（`COUNT(*) AS cnt` + `COALESCE(n,0) AS x`）。实测 RED：2 缺陷形态断言失败（表头为全 schema `["id","name"]` / `["id","n","s"]`，行已裁剪）、锚点 GREEN——与契约预期形态逐字一致。
- T2（design D1）：`get_plan_output_columns` 三 scan 臂收窄——新模块级 helper `projected_columns(columns, projection)`（空投影恒等，非空按索引映射，与 Filter/Sort 臂同构）；`Scan`/`DataScan` 臂应用（DataScan 必需面，Scan 对称加固）；`IndexScanAll` 臂对称加固（无构造点死路径）；`IndexScan` 臂保持原行为 + 防误改注释（columns 构造时已收窄，再应用将双重裁剪/越界）。`tests/cli_test.rs:288` 顺序渲染注释按投影语义校准（T2 契约点名项）。T1 缺陷形态断言转 GREEN。
- T3（回归收尾）：消费面复核（结论见 Self-Review）+ 全量 `cargo test` / clippy / fmt / `openspec validate --changes` 全绿（见 Verification Evidence）。

**Changed Files and Symbols**

- `src/parser/planner/query.rs`：新 `fn projected_columns(&[String], &[usize]) -> Vec<String>`（模块级，query.rs:24）；`PlanBuilder::get_plan_output_columns` 的 `Scan`/`DataScan`/`IndexScanAll` 三臂改经 helper 裁剪 + `IndexScan` 臂注释（query.rs:37-38/69-76）。执行器、plan 路由、其余臂零修改。
- `tests/cli_test.rs`：追加 `test_bare_datascans_projection_header_matches_rows` / `test_pushdown_datascans_projection_header_matches_rows` / `test_aggregate_and_expression_header_anchors`（文件尾部，MS15-Rest Iteration 000 节）；`test_multi_statement_sequential_render` 文档注释按投影语义校准（断言零修改）。

**Deviations from Plan**

1. T1 锚点表达式形态等价替换（非实质）：Plan Context 命名 `SELECT n + 1 AS x FROM t`，基线实测报 `Plan error: Unsupported expression type`——SELECT 列表二元算术不在 MS11-T01 表达式项支持面（七变体门：CASE/COALESCE/CAST/常量等），属预存能力边界而非 I034 回归。评估契约停止条件（锚点基线失败 → 返回 Plan）后判定：锚点的 Acceptance 意图（表达式路径表头由顶层 Projection 节点定形、本 change scan 臂裁剪不触及）可由受支持的等价形态完整达成，按契约内等价调整改用 `SELECT COALESCE(n, 0) AS x FROM t`（同走顶层 Projection 定形，表头 `["x"]`），已用注释在测试处记录。delta spec 场景文本中的 `a + 1` 示例未改动（Plan 产物），Plan Review 可裁量是否修订措辞。
2. 过程记录：首次全量验证后 `cargo fmt` 将 IndexScanAll 臂重排为单行（纯格式），按 Gate 5 新鲜性对最终状态重跑完整验证链，结论不变。

**Blocker Handoff**

None required

**Blocker Resolution**

None

**Self-Review**

Spec compliance review（逐任务）：

- R1 场景覆盖：S2 裸 DataScan（新用例）、S3 下推 DataScan（新用例）、S4 锚点（新用例，COALESCE 等价形态）、S1「全部扫描路径返回投影列」（新用例 + `projection_test` 6 用例零修改转真）、S5「SELECT * 行为不变」（`select_star_unchanged` + 全量零回归）、S6「既有测试校准」（唯一校准点 = T2 契约点名的 `cli_test.rs:288` 注释；无既有断言被修改）。RTM 六行全 Covered。
- 不变量：谓词求值先于投影裁剪（执行器零修改）；`SELECT *` 恒等（空/全投影恒等路径）；plan cache 键语义不变（未触碰）；四格式渲染函数零修改。
- 禁止项：未修改执行器、plan 路由、IndexScan 臂行为、既有用例。

Code quality review（完整 diff）：

- diff 无计划外修改；`git status` 文件集 = 基线（MS15-T01 未提交产物，本 cycle 未触碰）+ `tests/cli_test.rs`。
- `projected_columns` 索引安全性：projection 索引由 `resolve_projection_indices` 对同一 `node.columns` schema 生成（与 Filter/Sort 臂及执行器裁剪共用同一不变量）；空投影恒等使全部既有零投影构造点（OR 臂 / ORDER BY / 聚合 / 表达式 / Scan 构造点）行为逐字节不变。
- 无新增警告（clippy `-D warnings` exit 0）、无死代码、无重复实现；测试以正确原因通过（RED 观察到确切缺陷形态，行内容断言证明行从未变化、仅表头元数据修复）。
- 发现问题：无 Critical / 无 Important；Minor 2 项记入 Remaining Issues。

T3 消费面复核结论（契约要求记录项）：

- 聚合 `input_schema`（query.rs:652）：聚合路径 `projection_indices=None` → scan 投影恒空 → `projected_columns` 恒等；聚合套件（含 `pk_point_aggregate_real_value`）全绿实证。
- 派生表列注册（query.rs:148）：子查询带子集投影时现登记投影列名 = 实际行形状（修复前登记全 schema 名而行已裁剪，同族错位）；外层引用被投影掉的列由"错误值/Null"变为"列不存在报错"（Plan Risks 预期方向）；全量绿——无既有用例锁定旧行为，校准清单仅注释 1 处。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T1 RED 见证 | `cargo test --test cli_test -- test_bare_datascans… test_pushdown_datascans… test_aggregate_and_expression_header_anchors`（实现前） | `2 failed; 1 passed`——bare: `columns ["id","name"] ≠ ["name"]`；pushdown: `["id","n","s"] ≠ ["s"]`；锚点绿 | 缺陷形态观察（I034 两路径） | PASS（预期 RED） |
| T2 GREEN | `cargo test --test cli_test --test projection_test` | `59 passed; 0 failed; 2 ignored` + `6 passed; 0 failed` | 新用例转绿 + 执行器投影 6 锚点零修改 | PASS |
| 全量回归（最终状态） | `cargo test` | `passed: 856 failed: 0 ignored: 2`（基线 853 + 3，只增不减） | 全仓库 | PASS |
| 静态分析 | `cargo clippy --all-targets -- -D warnings` | exit 0（输出仅 `~/.cargo/config` 弃用提示 2 行，非 lint） | 全目标 | PASS |
| 格式 | `cargo fmt --check` | 0 diff | 全仓库 | PASS |
| OpenSpec | `openspec validate --changes` | `Totals: 1 passed, 0 failed` | change 产物 | PASS |
| CLI 探针 | `./target/debug/rtsql <tmp>/s.db "SELECT name FROM s" --format json`；同法 t.db 下推 | `{"columns":["name"],"rows":[["Alice"]]}`；`{"columns":["s"],"rows":[["a"]]}` | 缺陷形态端到端（与 delta spec THEN 逐字一致） | PASS |

**Persisted Evidence**

None required（Plan 模式 none；验证命令与决定性输出已全部载入本 Response，无可采集白名单项）

**Experience Candidates**

None

**Remaining Issues**（均 Minor，不阻塞 Acceptance）

1. `IndexScanAllNode.columns` 的节点文档注释写"与 `projection` 一致的实际输出形状"（与 `IndexScanNode` 同款），而 D1 裁定其 `get_plan_output_columns` 臂按"columns = 基 schema + projection 索引"裁剪——plan 变体当前无构造点（死路径，grep 实证），无行为影响；若将来接线且沿用 IndexScan 构造先例（构造时收窄 columns），该臂与节点文档将矛盾，需届时统一语义。
2. SELECT 列表二元算术（`n + 1`）不在 MS11-T01 表达式项支持面（`Unsupported expression type`）——预存能力边界，与本 change 无关；建议 docs-maintainer 收尾时评估登记 I 项。

**Commit or Diff Reference**

未提交（工作区；基线 HEAD f9e1e1f，同工作区含 MS15-T01 实施与 docs sync 未提交产物，本 cycle 未触碰）。本 cycle diff = `tests/cli_test.rs`（+110 行：3 用例 + 1 注释校准）与 `src/parser/planner/query.rs`（helper + 三臂 + 注释；该文件另含 pre-cycle 的 MS15-T01 未提交改动，本 cycle 未触碰）。

## Plan Review

- Review Result: accepted

**Findings**

基于实际代码、diff 与独立验证的检查（Act Self-Review 未代替独立检查）：

- **F1（PLAN-INVALID，非阻塞，本 Review 已由 Plan 修正）**：delta spec R1 S6「聚合与表达式路径表头零回归」的 GIVEN 示例 `SELECT a + 1 AS x FROM t` 不可达——SELECT 列表二元算术不在 MS11-T01 表达式项支持面（Act Deviation 1 实证 `Unsupported expression type`）。Plan 产物缺陷：场景文本命名了不可执行的形态。已按 Act 的契约内等价调整修订 spec 文案为 `SELECT COALESCE(n, 0) AS x FROM t`（同走顶层 Projection 定形，Acceptance 意图不变），并加注表达式支持面引用。测试与 spec 文案现逐字一致。
- **F2（Minor，接受不返工）**：`IndexScanAllNode.columns` 节点文档注释（`src/executor/plan.rs:114`「与 `projection` 一致的实际输出形状」）与 D1 的臂裁定（按基 schema + projection 索引裁剪）存在潜在语义矛盾——plan 变体当前无构造点（死路径，本次 Review grep 复核成立），现行为恒等无影响；将来接线时需先统一语义（IndexScan 构造收窄先例 vs DataScan 全 schema + 裁剪臂先例）再定构造形态。记录为未来决策点，不构成本 Acceptance 问题。
- **F3（Minor，接受不返工，Review 独立证据）**：本 Review 独立复跑 3 次全量——1 次瞬态失败（累计 349 passed 后 1 failed，cargo 中止后续目标，未留存失败名）、2 次干净全绿。失败签名与已登记 **I041**（`src/cli/resolve.rs` 两 env 测试并发改写进程全局 HOME/RTSQL_HOME，偶发约 1/6 全量、重跑即绿、`--lib` 稳定）一致，机制为既有 lib 内 env 竞态，与本 change diff 无关（新增 3 用例无 env/共享全局操作）。注：新增 e2e 子进程负载对 I041 窗口有边际放大（同 MS11-T02 新增 16 e2e 的既有先例），治理归 I041 已登记方案。非阻塞。
- **F4（Minor，接受）**：Act Remaining Issues #2（SELECT 二元算术能力边界建议登记 I 项）——属 docs-maintainer 收尾登记面，Follow-up 记录。
- 实现核对：`projected_columns` helper 与 D1 契约逐字同构（空投影恒等/非空索引映射）；DataScan/Scan/IndexScanAll 三臂应用、IndexScan 臂保持 + 防双重裁剪注释——与 Task Contract T2 的 Preserve/Forbidden 面完全一致；`git status` 文件集（query.rs + cli_test.rs，另含 pre-cycle MS15-T01 未提交产物）与 Changed Files 声明一致，执行器/渲染/plan 路由零触碰。`cli_test.rs:288` 注释校准位置与内容正确（断言零修改）。
- 偏差核对：Deviation 1（锚点形态 `n + 1` → `COALESCE(n,0)` 等价替换）为 ACT-DEVIATION 非实质——Act 正确评估了契约停止条件（锚点基线失败 vs 预存能力边界），等价形态完整达成 Acceptance 意图，测试处已注释；Deviation 2（fmt 重排后按最终状态重跑完整验证链）过程正确，新鲜性保持。

**Deviation Classification**

- ACT-DEVIATION ×1（Deviation 1 锚点等价替换，非实质，接受）
- PLAN-INVALID ×1（F1 spec 示例不可达，非阻塞，本 Review 修正 spec 文案）
- BASELINE-CHANGED ×0；PLAN-OMISSION ×0；NEW-EVIDENCE ×0

**Acceptance Gaps**

None——R1 delta spec 7 场景全部满足（S2/S3/S4 新增见证、S1 经既有 `projection_test` 6 用例 + 新用例转真、S5/S6/S7 零回归面全量锁定）；全量 856（基线 853 + 3）只增不减。

**Convergence**

N/A（首次 Review）

**Evidence**

- 独立代码检查：`src/parser/planner/query.rs:24-31`（helper）、`:35-36/46-53/70-76`（三臂 + IndexScan 注释）；`tests/cli_test.rs` diff（3 新用例 + 1 注释校准，断言与 delta spec THEN 逐字一致）。
- 独立复跑：`cargo test` 全量 exit 0——`total passed: 856 failed: 0 ignored: 2`（67 个 test target；另 1 次瞬态失败见 F3）；`cargo clippy --all-targets -- -D warnings` exit 0；`cargo fmt --check` exit 0；`openspec validate --changes` 1 passed / 0 failed。
- 独立 CLI 探针（RTSQL_HOME=/tmp/i034probe，2026-09-12）：`SELECT name FROM s` → `{"columns":["name"],"rows":[["Alice"]]}`；`SELECT s FROM t WHERE n > 5` → `{"columns":["s"],"rows":[["a"]]}`；`SELECT COUNT(*) AS cnt FROM t` → `{"columns":["cnt"],...}`；`SELECT COALESCE(n, 0) AS x FROM t` → `{"columns":["x"],...}`——与 delta spec THEN 逐字一致。
- 采信 Act Response 未失效结论：T1 RED 见证（2 failed / 1 passed 与契约预期形态一致，覆盖范围未变化）、T3 消费面复核结论（聚合/派生表面，本次 Review 代码复核一致）。

**Follow-up Decision**

既有 Acceptance 已满足、无阻塞项且无当前 Cycle 修复需求 → `accepted`。Minor 项处置：F1 已由 Plan 修正（spec 文案）；F2/F4 记录在案（F2 为将来 IndexScanAll 接线时的决策点；F4 的 I 项登记与 F3 的 I041 治理归 docs-maintainer 收尾面，不属本 change 实施范围）。收尾时建议 docs-maintainer：登记 Act Remaining #2（SELECT 二元算术能力边界）为新 I 项；I034 转 promoted。

**Iteration Plan Update**

None

**Next Cycle**

None（本 Iteration 一次通过，无 rework/replan）

**Next Iteration**

`iterations/001-update-key-index/000-initial.md`（已按 Map 展开，Plan Context `ready`——T4-T6，I037 UPDATE 键位无键值索引条目清理）
