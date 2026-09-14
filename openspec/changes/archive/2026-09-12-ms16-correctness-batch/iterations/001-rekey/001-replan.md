# Iteration 001 / Cycle 001: replan——BH-3 校准（依赖 rekey 缺陷行为的既有测试）+ Iteration 001 收尾

## Plan Context

- Status: ready
- Iteration: 001-rekey
- Cycle: 001-replan
- Cycle Type: replan
- Parent cycle: 000-initial.md

**Iteration Scope**

- Change tasks: T9（2026-09-13 Plan Review BH-3 裁定新增；T5/T6 已于 000-initial 执行，成果继承为本 Cycle 基线）
- Depends on: Iteration 000（accepted）；同 Iteration 000-initial 已实施成果（T5/T6，见 Investigation Facts）
- Stable baseline: 与 tasks.md Iteration 001 修订后 Stable baseline 一致——rekey 后新键点查可达、旧键点查空集、旧键 INSERT 可用、碰撞写入前拒绝零副作用、恢复两态一致；同键/NULL/非键列分支逐字节保持；全量回归除 T9 校准 6 处外零修改（BH-3 裁定）
- Verification boundary: T9 全绿 + 既有锚点套件零修改（T9 校准 6 处除外）+ 全量 0 failed + clippy/fmt 0 + `openspec validate` PASS
- Diagnostic boundary: `tests/gc_test.rs` + `tests/version_chain_test.rs` + `tests/plan_exec_test.rs`（校准点）；异常时才回看 `src/executor/update.rs`（本 Cycle 零改动面）
- Deferred tasks: None（Iteration 001 为本 change 最后一个 Iteration）

**Cycle Scope**

- Trigger: replan-required
- Acceptance gaps: ① Iteration 001 Stable baseline「全量回归零修改」+ GREEN condition「全量 0 failed」未满足——6 个 M10 时代直连执行器测试依赖 rekey 缺陷行为（旧键条目残留指向新版本），R4 实施后确定性失败（BH-3，proposal 裁定记录 8）
- Repair items: None（replan 使用修订后全局 task T9，不设 repair item）
- Inherited scope: 000-initial 已交付成果全部继承——T5 RED 见证（4 用例）、T6 实现（碰撞预检 + Step 7 三分支，目标套件 9 GREEN + 锚点 26 GREEN）、全部 Invariants；design D4 校准段；delta spec `update-index-maintenance` 校准段
- Excluded scope: 产品代码改动（`src/` 零触碰——T6 实现已完成且经 Review 确认）；新 rekey 场景扩展；I041 修复；性能优化

**Objective**

将 6 个依赖 rekey 缺陷行为的既有直连执行器测试校准至修正后语义（寻址与定位改用行当前键，测试主题与断言语义保持），全量 0 failed、clippy/fmt/validate 收尾——Iteration 001 Acceptance 闭合。

**Background**

000-initial Cycle 实施 T5/T6 后 Plan Review 独立审计裁定 replan-required（见 000-initial.md Plan Review）：BH-3——Plan 的 Iteration 001 兼容性预测「全量回归零修改」被证伪，与 Iteration 000 BH-1 同类。`gc_test`（3 用例）、`version_chain_test`（2 用例）、`plan_exec_test::test_insert_update_scan_flow` 以「SET 键列 = 另一 Int 值」建立版本链，并按**旧键** search/IndexScan 定位 rekey 后版本或寻址后续 UPDATE——该夹具模式依赖 I047 缺陷行为本身；修复生效后这些断言必然失败（失败形态与 T5 RED 预测、I047 缺陷定义逐条一致，恰是修复生效的证据面）。校准裁定与范围见 proposal 裁定记录 8。

**Investigation Facts**

- Current Baseline: 工作区 = 000-initial Act 终态（T5/T6 已实施，未 commit，对照基线 d8a244f + Iteration 000 两轮叠加）；全量 892 用例中 6 failed（gc_test 0 passed/3 failed、plan_exec_test 3 passed/1 failed、version_chain_test 1 passed/2 failed）、目标套件 `update_index_maintenance_test` 9 passed、锚点套件 26 passed（key_type_conformance 8 + keyless_eq_routing 14 + keyless_row 4）——Plan Review 2026-09-13 独立复跑确认（`cargo test --no-fail-fast --test update_index_maintenance_test --test gc_test --test plan_exec_test --test version_chain_test`，四套件结果与 Act 报告逐项一致）。Act 在本 Cycle 开工前做只读基线检查（`git status`/`git diff --stat` 对照 000-initial Act Response Changed Files）即可采信，不重跑全量。
- 受影响面（UpdateExecutor 全部测试用法排查，Plan Review 2026-09-13）：引用 `UpdateExecutor` 的测试文件共 7 个——`update_index_maintenance_test.rs`（T5/T6 面，GREEN）、`gc_test.rs`（3 用例全失败）、`version_chain_test.rs`（2 失败 1 通过）、`plan_exec_test.rs`（1 失败 3 通过）、`executor_test.rs`（2 处用法全通过——:255 为 rekey 但仅断言 AffectedRows 不做旧键寻址、:683 为非键列更新）、`mvcc_record_test.rs`（非键列更新）、`storage_test.rs`（仅注释提及）。**受影响用例恰为失败的 6 个，无第 7 个潜伏点。**
- 各失败用例的寻址链（行号对照当前工作区）：
  - `gc_test::test_gc_removes_old_versions`：INSERT id=10 → tx2 UPDATE SET id=20（rekey，键 10 条目删、20 立）→ :85 `search(key=10)` 期望 v2 → panic `v2 should exist`；tx3 UPDATE 仍以键 10 寻址（现会 `KeyNotFound`，该用例先在 :85 panic）。
  - `gc_test::test_gc_preserves_uncommitted_versions`：INSERT id=100 → tx2 未提交 UPDATE SET id=200（rekey）→ :223 `search(key=100)` 期望 v2 → panic `v2 should exist`。
  - `gc_test::test_gc_multiple_keys`：每键链式 10→11→12，tx3 UPDATE 仍以原始键 `key_bytes`（10）寻址 → Step 1 `search(10)` = None → `Error: KeyNotFound`。
  - `plan_exec_test::test_insert_update_scan_flow`：INSERT id=1 → UPDATE SET id=1000（rekey，AffectedRows 断言通过）→ :193 `IndexScanExecutor(key=1)` 期望 Row(Int(1000)) → 实得 None → panic。
  - `version_chain_test::test_version_chain_traversal`：INSERT 10 → tx2 UPDATE SET id=20（rekey，提交）→ :95 `search(key=10)` 期望 v2 → panic `v2 should exist`；tx3 仍以键 10 寻址。
  - `version_chain_test::test_version_chain_skips_invisible`：INSERT 100 → tx2 未提交 UPDATE SET id=200 → tx3 UPDATE 以键 100 寻址 value=300 → `Error: KeyNotFound`。
- 校准语义（design D4 校准段）：rekey 后**行当前键 = 新值键**；测试主题（GC 清理计数与最新版本可达 / 版本链遍历与可见性 / 插改扫流）与断言语义不变，仅寻址/定位键改为当前键。GC 与可见性断言均按 row_id 或数据页遍历（`gc_table` 迭代数据页、`find_visible_version` 按 row_id 链回溯），与索引寻址键解耦——校准不影响其验证力。
- 未提交 UPDATE 的索引可见性：索引维护在 UPDATE 执行时即生效（与既有同键 update 路径一致，非事务性延迟）——`test_gc_preserves_uncommitted_versions` 校准后 `search(200)` 可达 v2（未提交）与既有行为同构。

**Implementation Guidance**

建议顺序：三测试文件逐个校准（gc_test → version_chain_test → plan_exec_test）→ 目标三套件 GREEN → 全量收尾（clippy/fmt/validate）。校准点均为「寻址键/定位键常量」替换：后续 UPDATE 的寻址键改为上一版本的新值键；定位最新版本的 search/IndexScan 键改为最终新值键。每处校准点行内或近旁加简短注释注明 BH-3 校准依据（delta spec 校准段）。同键更新断言（若有）与非键列更新路径零触碰。

**Behavioral Change**

- 测试面：6 个既有直连执行器测试的夹具寻址方式校准（M10 时代模式 → 修正后语义）；无产品行为变化（`src/` 零改动）。
- 测试主题与验证力保持：GC 清理计数断言、版本链 next_version/可见性断言、插改扫流值断言全部逐字节保留。

**Task Contracts**

### T9: BH-3 校准——6 个依赖 rekey 缺陷行为的既有测试按当前键寻址 + Iteration 001 全量收尾（R4 负空间 + 校准例外）

- Requirement/Scenario: R4 全场景的既有套件零修改约束修订（校准例外 6 处，proposal 裁定记录 8）；delta spec `update-index-maintenance` 校准段
- Depends on: None（T5/T6 成果已在工作区）
- Targets: `tests/gc_test.rs`（3 用例）、`tests/version_chain_test.rs`（2 用例）、`tests/plan_exec_test.rs::test_insert_update_scan_flow`——仅测试文件
- Current behavior: 6 用例确定性失败（形态与寻址链见 Investigation Facts；Plan + Act 多轮观察一致）
- Required behavior: 校准后 6 用例 GREEN——
  - `test_gc_removes_old_versions`：v2 定位 `search(20)`；tx3 UPDATE 寻址键 20；v3 定位 `search(30)`；GC 后最终断言 `search(30)` 返回 row_v3 且值 30
  - `test_gc_preserves_uncommitted_versions`：v2 定位 `search(200)`；GC cleaned_count ≥ 1 与 v2 未提交头断言不变
  - `test_gc_multiple_keys`：tx3 UPDATE 寻址键 `versions[i][1]`；最终验证循环 `search(versions[i][2])` 定位并断言值
  - `test_insert_update_scan_flow`：UPDATE 后 `IndexScanExecutor(key=1000)` 返回 Row 且 `values[0] == Int(1000)`
  - `test_version_chain_traversal`：v2 定位 `search(20)`；tx3 UPDATE 寻址键 20；v3 定位 `search(30)`；next_version/提交头/可见性断言（按 row_id）逐字节不变
  - `test_version_chain_skips_invisible`：tx3 UPDATE 寻址键 200；v3 定位 `search(300)`；Tx4 可见性断言不变
- Required changes: 仅上述三文件中列明的寻址键/定位键常量与派生变量 + 校准点注释（注明 BH-3 依据）；随后全量收尾（`cargo test --no-fail-fast` 全量、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate` changes+specs）
- Preserve: 各测试建表/事务结构、GC 计数断言、可见性断言、值断言逐字节不变；`version_chain_test::test_all_versions_invisible`、`plan_exec_test` 其余 3 用例、`executor_test.rs`、`mvcc_record_test.rs`、`storage_test.rs` 零修改；T5 4 用例与 T6 实现（`src/` 全部）零触碰
- Forbidden: 不改产品代码（`src/`）；不改校准点以外的既有断言或结构；不新增测试用例（校准非扩展）；不引入身份型证据机制
- Test witness: 校准前 6 用例 RED 已确立（000-initial Act 全量 + Plan Review 独立复跑，双源一致）——本任务为校准，直接观察校准后 GREEN，无需重写 RED（BH-1/T8 先例）
- GREEN condition: 目标三套件全绿（gc 3 + version_chain 3 + plan_exec 4）+ 全量 `cargo test --no-fail-fast` 0 failed（预期 892 passed / 0 failed / 2 ignored；I041 偶发按 T8 契约先例处置——与 resolve 相关的失败单独复跑确认并在 Response 注明）+ clippy 0 + fmt clean + `openspec validate` changes 1 PASS / specs 25 PASS
- Verification: 命令输出（每项 ≤20 行决定性片段）与退出码记录于 Act Response
- Stop when: 校准需改动断言语义（GC 计数/可见性/值断言）才能成立——测试主题本身受影响的实质发现，返回 Plan；或全量出现第 7 个同模式失败——覆盖调查遗漏的实质发现，返回 Plan

**Invariants**

- T5/T6 已交付成果（路由门、写入类型强制、碰撞预检、Step 7 三分支）零触碰；`src/` 全部零改动。
- 错误优先级：KeyNotFound（Step 1）→ KeyTypeMismatch（类型校验）→ DuplicateKey（碰撞预检）→ 写入（T6 既定，校准不得破坏）。
- 既有测试除 T9 校准 6 处外零修改；基线 888 + T5 新 4 = 892 只增不减。
- 校准后的测试仍验证原主题：GC 清理语义、版本链可见性、插改扫流。

**Non-goals**

- 产品代码任何改动；新 rekey 场景或测试扩展；partial INSERT；I041 修复；性能优化；MS16 范围外缺陷（I033/I032/I038 等）。

**Acceptance**

Requirements Traceability Matrix（Iteration 001 修订后范围 R4 + 校准例外；R1-R3/R5/R6 已于 Iteration 000 覆盖）：

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R4 键位 rekey 后索引条目一致 | 新键可达/旧键清理/碰撞写入前拒绝/恢复两态一致/同键保持 | D4 | T5, T6 | 001 | `update.rs` 前置块碰撞预检 + Step 7 三分支 | `update_index_maintenance_test` 4 新用例 + 既有 5 用例零修改（000-initial 已 GREEN） | None | Covered（T5/T6 继承） |
| R4 负空间——既有套件零修改（修订：T9 校准例外 6 处） | gc/version_chain/plan_exec 六用例校准后 GREEN | D4 校准段 | T9 | 001 | 三测试文件寻址键/定位键 | 目标三套件全绿 + 全量 0 failed | 校准例外（BH-3 裁定，proposal 记录 8；delta spec 校准段记录） | Covered |

**Verification**

- T9：目标三套件 GREEN + 全量 `cargo test --no-fail-fast`（预期 892 passed / 0 failed / 2 ignored）+ `cargo clippy --all-targets -- -D warnings`（0）+ `cargo fmt --check`（clean）+ `openspec validate --changes`（1 PASS）+ `openspec validate --specs`（25 PASS）。
- 验证直接观察行集/错误变体/退出码/恢复面行为，不引入身份型证据机制。

**Gate 2 Readiness**

- 无 Missing requirement：PASS——R4 已 Covered（T5/T6 继承，000-initial Plan Review 独立确认）；校准例外映射 T9（修订后 RTM）。
- 无未批准 Simplified：PASS——校准例外为 BH-3 裁定（proposal 记录 8）+ delta spec 校准段，非需求裁剪；待用户批准本 replan 计划（Gate 2 末项）。
- 调查完整：PASS——6 失败用例的寻址链均有代码级证据（行号 + panic 文案 + Plan 独立复跑）；受影响面经 UpdateExecutor 全部测试用法排查闭合（7 文件逐一定性，无潜伏点）；GC/可见性断言与索引寻址解耦已确认。
- 设计闭合：PASS——design D4 校准段（校准语义 + 受影响面 + 先例）；无 TBD。
- 任务可执行：PASS——T9 契约含 Targets/Current/Required（逐用例）/Preserve/Forbidden/GREEN/停止条件。
- 分轮合理：PASS——Iteration 001 修订后仍为单一可验收成果（rekey 索引一致性 + 其测试面校准），Map 不变。
- 追踪完整：PASS——R4 → S → D4 → T5/T6/T9 → 代码面 → 测试见证闭环。
- 验证充分：PASS——校准后 GREEN + 全量 + 静态检查；无身份型证据工程；Persisted Evidence none。
- 无实质未知项：PASS——非实质选择（注释位置、局部变量形态）留 Act。
- tasks/specs/design/Cycle 一致：PASS——T9 已入 tasks.md；delta spec `update-index-maintenance` 校准段已记；design D4 校准段已记；本 Cycle 创建（draft）。
- 用户批准：**PASS**——2026-09-13 用户批准 replan 计划（原话「批准」），T9 契约、校准例外（proposal 裁定记录 8）与修订后 Iteration 001 基线随批准生效；000-initial 的 Plan Context 已冻结，本 Cycle 为其 Plan Review `replan-required` 的后继产物。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果；无不可低成本重跑的验收面。

- Budget: 不适用（none）。

**Risks and Notes**

- I041 flaky 可能撞全量收尾（约 1/6）——处置同 T8 契约先例：与 resolve 相关的失败单独复跑确认 flaky 属性并在 Response 注明，不计入本 change 失败。
- 校准点均为常量替换，改动机械；若实施中发现某用例的断言语义（非寻址面）也依赖缺陷行为，属 stop condition 实质发现，返回 Plan。
- `test_gc_preserves_uncommitted_versions` 校准后依赖「未提交 UPDATE 的索引条目即时生效」——与既有同键 update 路径一致（T6 前即如此），非新增语义面。

## Act Response

- Status: reported

**Implemented**

T9 全部完成（本 change 最后一个 task）。BH-3 校准：6 个依赖 rekey 缺陷行为（旧键条目残留指向新版本）的既有直连执行器测试，按 design D4 校准段将寻址/定位键改用行当前键，测试主题与断言语义逐字节保持：

- `tests/gc_test.rs` 3 用例——`test_gc_removes_old_versions`：v1 定位与 tx2 寻址保持键 10，tx2 rekey 10→20 后 v2 定位与 tx3 寻址改键 20，tx3 rekey 20→30 后 v3 定位与 GC 后最终定位改键 30；`test_gc_preserves_uncommitted_versions`：tx2 寻址保持键 100，rekey 100→200（未提交）后 v2 定位改键 200；`test_gc_multiple_keys`：tx3 寻址键 `key_bytes` → `versions[i][1]`，最终验证循环定位键 → `versions[i][2]`。
- `tests/version_chain_test.rs` 2 用例——`test_version_chain_traversal`：v2 定位与 tx3 寻址改键 20、v3 定位改键 30（next_version/提交头/可见性断言按 row_id 逐字节不变）；`test_version_chain_skips_invisible`：tx3 寻址改键 200（未提交 UPDATE 索引条目执行时即生效）、v3 定位改键 300。
- `tests/plan_exec_test.rs` 1 用例——`test_insert_update_scan_flow`：UPDATE 寻址保持键 1，rekey 1→1000 后 IndexScan 定位改键 1000。

每处校准点近旁加 BH-3 校准注释（与 delta spec `update-index-maintenance` 校准段及 expression_e2e_test BH-1 先例措辞一致）。合计 10 处寻址/定位键替换 + 4 处 `mut` 声明 + 10 条注释，零断言/结构改动，`src/` 零触碰。

**Changed Files and Symbols**

- `tests/gc_test.rs`：`test_gc_removes_old_versions`（`key` 改 `mut` + 2 处重赋值 + 2 注释）、`test_gc_preserves_uncommitted_versions`（`mut` + 1 重赋值 + 1 注释）、`test_gc_multiple_keys`（tx3 寻址参数 + 最终循环 `key_bytes` 初始化 + 2 注释）。
- `tests/version_chain_test.rs`：`test_version_chain_traversal`（`mut` + 2 重赋值 + 2 注释）、`test_version_chain_skips_invisible`（`mut` + 2 重赋值 + 2 注释）；`test_all_versions_invisible` 零修改。
- `tests/plan_exec_test.rs`：`test_insert_update_scan_flow`（IndexScan 构造键改 1000 + 1 注释）；其余 3 用例零修改。
- 合计 3 文件 +30/−7 行；无产品代码改动。

**Deviations from Plan**

- 无契约内偏差——校准点与 T9 契约 6 用例 Required 行为逐条对应，实现形态与 Plan Context Investigation Facts 的寻址链一致。
- 实现细节（非实质，契约「寻址键/定位键常量与派生变量」授权面）：`test_gc_removes_old_versions`、`test_gc_preserves_uncommitted_versions`、`test_version_chain_traversal`、`test_version_chain_skips_invisible` 以 `key` 变量 `mut` + 重赋值表达「行当前键随 rekey 演进」（替代逐点展开多个键常量变量）；`test_gc_multiple_keys` 与 `test_insert_update_scan_flow` 按契约逐点替换。

**Blocker Handoff**

None

**Blocker Resolution**

None

**Self-Review**

- Plan compliance: PASS——T9 契约 6 用例 Required 行为逐条落地；Preserve 面（建表/事务结构、GC 计数断言、可见性断言、值断言、`test_all_versions_invisible`、plan_exec 其余 3 用例、`executor_test.rs`/`mvcc_record_test.rs`/`storage_test.rs`、T5 4 用例、`src/` 全部）零触碰（git diff 逐 hunk 核对）；Forbidden 面（产品代码、校准点外断言/结构、新增用例、身份型证据机制）零违反；stop condition 未触发——无第 7 个同模式失败（全量 0 failed 证实受影响面闭合）。
- Full diff reviewed: PASS——本 Cycle 完整 diff（3 文件 +30/−7）逐 hunk 审查：无计划外修改；无死变量（`key_bytes` 在 `test_gc_multiple_keys` 夹具循环仍服务 tx2 寻址、plan_exec 仍服务 UPDATE 寻址）；无 mut 死赋值（每次重赋值后均有读取）；注释语言与 BH-1 校准先例一致。
- Critical findings unresolved: None
- Important findings unresolved: None
- Minor findings unresolved: `openspec validate --changes` 输出先存 WARNING（change Purpose 占位提示；validate 仍 1 passed / 0 failed，属归档前 docs 收尾面，非本 Cycle 范围）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| RED 基线（校准前） | `cargo test --no-fail-fast --test gc_test --test version_chain_test --test plan_exec_test` | `gc_test: 0 passed; 3 failed`、`plan_exec_test: 3 passed; 1 failed`（`Expected Row with Int(1000) after update, got None`）、`version_chain_test: 1 passed; 2 failed`（`:95 v2 should exist` / `Error: KeyNotFound`） | 6 用例缺陷依赖面 | FAIL（预期 RED，与 Plan Context 预测及 000-initial Act/Plan Review 双源结论一致） |
| 目标套件 GREEN（校准后） | 同上命令 | `gc_test: 3 passed; 0 failed`、`plan_exec_test: 4 passed; 0 failed`、`version_chain_test: 3 passed; 0 failed` | 6 校准用例 + 同套件未触碰用例 | PASS |
| 全量回归 | `cargo test --no-fail-fast` | `TOTAL passed: 892, failed: 0, ignored: 2`（exit 0） | 全部 892 用例 | PASS（I041 flaky 未触发，首跑即干净） |
| 静态检查 | `cargo clippy --all-targets -- -D warnings` | `Finished dev profile ... in 2.04s`（exit 0） | 全 targets lint | PASS |
| 格式 | `cargo fmt --check` | clean（exit 0） | 全仓格式 | PASS |
| OpenSpec | `openspec validate --changes` / `--specs` | `1 passed, 0 failed`（含先存 Purpose WARNING）/ `25 passed, 0 failed` | change + specs 语料库 | PASS |

**Persisted Evidence**

None required（Plan Context Mode: none；Act Response 承载全部验证结果，无不可低成本重跑的验收面）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook / Incident | None | None | None |

**Remaining Issues**

- `openspec validate --changes` 的 Purpose 占位 WARNING（先存，Minor，归档前 docs 收尾面）。
- I041（resolve env 测试竞态）本次未触发；仍按 MS16 Workload 建议留独立小 change 随带消除。

**Commit or Diff Reference**

未 commit（待用户触发）；本 Cycle diff = `tests/{gc_test,version_chain_test,plan_exec_test}.rs` +30/−7（对照基线：000-initial Act 终态工作区，Investigation Facts 只读基线检查一致）。

## Plan Review

- Review Result: accepted

**Findings**

独立核查（不依赖 Act Self-Review）——T9 diff 逐 hunk 审读、三校准套件 + 全量独立复跑、`src/` 漂移对照：

- **F1（确认项）**：T9 diff 与契约逐用例对应——10 处寻址/定位键替换全部落位且无多余改动：`test_gc_removes_old_versions`（tx2 寻址保持键 10、rekey 10→20 后 v2 定位与 tx3 寻址键 20、rekey 20→30 后 v3 定位与 GC 后最终定位键 30）、`test_gc_preserves_uncommitted_versions`（v2 定位键 200）、`test_gc_multiple_keys`（tx3 寻址 `versions[i][1]`、终验定位 `versions[i][2]`，tx2 寻址保持原键）、`test_insert_update_scan_flow`（UPDATE 寻址保持键 1、IndexScan 定位键 1000）、`test_version_chain_traversal`（v2 定位与 tx3 寻址键 20、v3 定位键 30）、`test_version_chain_skips_invisible`（tx3 寻址键 200、v3 定位键 300）；断言与结构零改动，BH-3 校准注释齐备（与 delta spec 校准段措辞一致）。
- **F2（确认项）**：校准语义正确——GC 计数、`next_version`、可见性、值断言均按 row_id/数据页，与索引寻址键解耦，验证力逐字节保持；未提交 UPDATE 的索引条目即时生效与既有同键 update 路径同构（T6 前即如此）。
- **F3（确认项）**：Plan 独立复跑全绿——三校准套件 `3 passed` + `4 passed` + `3 passed`（全 0 failed）；全量 `cargo test --no-fail-fast` **892 passed / 0 failed / 2 ignored**（与 Act 报告一致，I041 未触发）；`cargo clippy --all-targets -- -D warnings` 0、`cargo fmt --check` clean、`openspec validate --changes` 1 PASS / `--specs` 25 PASS。
- **F4（确认项）**：`src/` 零改动——diffstat 对照 000-initial 终态（含 T6 的 update.rs 84 行），本 Cycle 无任何 src hunk；Preserve 面（`test_all_versions_invisible`、plan_exec 其余 3 用例、`executor_test.rs`/`mvcc_record_test.rs`/`storage_test.rs`、T5 4 用例）零触碰。
- **F5（确认项）**：全量 0 failed 证实受影响面闭合——000-initial Plan Review F4 的「恰 6 用例」排查结论经受全量验证，无第 7 个同模式失败。
- **F6（Minor，无需行动）**：`openspec validate` 的 Purpose 占位 WARNING——既有 I045 噪声（000-initial 001-replan Review F5 已定案归属），validate 计数 PASS 不受影响，归档前 docs 收尾面处理。

**Deviation Classification**

- Act 实现细节（`key` 变量 `mut` + 重赋值表达「行当前键随 rekey 演进」，替代逐点展开键常量）→ **ACT-DEVIATION**（T9 契约「寻址键/定位键常量与派生变量」授权面内，非实质，已记录）。
- 无 PLAN-OMISSION / PLAN-INVALID / BASELINE-CHANGED / NEW-EVIDENCE。

**Acceptance Gaps**

None——R4 全场景 Covered（T5/T6 继承，000-initial Review 独立确认）；校准例外闭合（T9 六用例 GREEN，proposal 裁定记录 8 + delta spec 校准段）；全量 0 failed（892 passed）；clippy/fmt/validate 全 0/PASS。Iteration 001 修订后 Stable baseline 全部满足。

**Convergence**

closed（对比 000-initial Plan Review：全量 gap 已消除——6 失败用例校准转绿，无新 gap）

**Evidence**

- Plan 独立复跑（2026-09-13）：三校准套件 `3/4/3 passed`；全量聚合 `TOTAL passed=892 failed=0 ignored=2`；clippy exit 0 / fmt exit 0 / validate changes `1 passed, 0 failed` + specs `25 passed, 0 failed`。
- 代码级：T9 diff（3 文件 +30/−7）逐 hunk 审读；`git diff --stat -- src/` 对照 000-initial 终态确认零漂移。
- 采信 Act 未失效结论：RED 基线（000-initial Act + Plan Review 双源已定案，且本 Cycle GREEN 前提下重写 RED 无意义）。

**Follow-up Decision**

既有 Acceptance 已满足且无阻塞项 → `accepted`：Iteration 001 完成（T5/T6/T9 全部 done）。本 change（2026-09-12-ms16-correctness-batch）全部 Iteration（000、001）accepted，R1-R6 Covered——满足收尾条件，待用户调用 `openspec-docs-maintainer` 收尾（delta specs 合并主语料库、change 归档、SNAPSHOT/tasks 同步、I046/I047 处置落账）。

**Iteration Plan Update**

None（Map 不变——001-replan 为 Iteration 001 最后 Cycle）

**Next Cycle**

None（001-replan accepted，Iteration 001 完成）

**Next Iteration**

None（本 change 最后一个 Iteration 已完成；全部 Iteration accepted，change 达收尾条件）
