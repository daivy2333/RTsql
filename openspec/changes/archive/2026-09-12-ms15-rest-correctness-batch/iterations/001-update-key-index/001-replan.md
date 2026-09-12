# Iteration 001 / Cycle 001-replan: T8-R2 语义校准与回归收尾（I037）

## Plan Context

- Status: ready
- Iteration: 001-update-key-index
- Cycle: 001-replan
- Cycle Type: replan
- Parent cycle: 000-initial.md

**Iteration Scope**

- Change tasks: T4, T5, T6（T6 已按 replan 修订：含 T8-R2 语义校准）
- Depends on: Iteration 000（accepted）
- Stable baseline: 键位置 NULL 后旧键 INSERT 成功、点查空集、恢复两态一致、键位等值 UPDATE 对无键行 KeyNotFound；非键列与原值更新行为不变；全量回归通过（T8-R2 按 R1 语义校准除外）
- Verification boundary: T6 全绿 + clippy/fmt 0 + `openspec validate --changes` PASS
- Diagnostic boundary: `tests/keyless_row_test.rs`（T8-R2 校准）+ 全量验证命令
- Deferred tasks: T7-T9（Iteration 002）

**Cycle Scope**

- Trigger: replan-required
- Acceptance gaps: R2「既有 UPDATE 语义零回归」S「既有 UPDATE 行为零回归」——`keyless_row_test::keyless_row_update_recovery_after_crash` 1/4 失败（独立复现确认），根因 = 原 Plan 契约（T8-R2 零修改）与 R1 delete 语义互斥（T8-R2 第二次 UPDATE 依赖的恰是 I037 缺陷的残留索引条目，失败点 Step 1 search，任何 Step 7 形态无法回避）
- Repair items: None（replan 使用更新后的全局 task T6）
- Inherited scope: T4 completed（RED→GREEN，5 用例见证 R1 全场景）；T5 实施完成（新套件 5/5 GREEN、clippy/fmt/validate 干净、Risks 探针实测 serialize 不拒 String 入 Int 列且 delete 分支与恢复一致）——本 Cycle 不回滚、不重做，仅校准既有测试与回归收尾
- Excluded scope: 合成 WAL 测试基建（keyless old_tuple 重放子路径的替代见证，登记 I 项候选归 docs-maintainer）；`wal-recovery-replay-integrity` 主 spec delta（不在本 change delta 面）；rekey 可键控形态（默认假设 2）；I039（Iteration 002）

**Objective**

`keyless_row_test::keyless_row_update_recovery_after_crash` 按 R1 语义校准后全绿：第二次 UPDATE 断言 `KeyNotFound`（键位无键行对键位等值 UPDATE 不可达——R1 新场景），恢复面断言按新链重述（keyless NEW_tuple 重放见证保持）；全量 861 tests / 0 failed / 2 ignored（基线 860+1 只增不减，唯一失败消除）。

**Background**

000-initial Act 遇 Gate 6 阻塞（Blocker Handoff，2026-09-12）：Plan 预期「修复后运行期索引状态与 keyless_row_test 恢复面断言一致」+「4 用例零修改」——调查遗漏 T8-R2 对残留条目的依赖。Plan Review 判定 replan-required：验收边界「全量回归零修改」与 spec R2「SHALL 零修改通过」按语义校准例外修订（已完成于 change 文档），本 Cycle 执行校准与收尾。方向 b（维持残留可达性）被拒：与用户已批准的 R1 矛盾。

**Investigation Facts**

- Current Baseline: 000-initial Act Response `blocked`——T4/T5 工作保留且验证有效（独立复核采信：update.rs Step 7 diff 与 D2 逐字同构、新套件 5/5、全量 860 passed / 1 failed / 2 ignored 唯一失败 T8-R2、clippy/fmt/validate 干净——Plan Review 2026-09-12 独立复现 T8-R2 失败与全量计数一致）。
- Current-State Evidence（T8-R2 本体，`tests/keyless_row_test.rs:127-215` 行级）:
  - 链：CREATE TABLE t (a INT, v INT)（隐式 PK=a）+ flush_all（catalog 落盘夹具先例）→ INSERT (NULL,1)/(5,0)/(7,100) → `UPDATE SET a = NULL WHERE a = 5`（v2=(NULL,0)，修复后键 5 条目删除）→ `UPDATE SET v = 42 WHERE a = 5`（`:169-172`，修复后 Step 1 `search(5)`→None→`KeyNotFound`，Act 实测 + Plan 独立复现）→ shutdown + drop 不 close → 重开断言（COUNT=3 `:183` / v=42 可见 `:191` / v=1 可见 `:201` / 可键控行点查+判重守卫 `:204+`）。
  - 测试文档注释 `:127-138` 自述「再次 UPDATE 该行（old_tuple 无键，产生 RedoFailed 形态的 Update 记录）」与注释 `:163`「索引 key5 → v2」——描述的即 I037 缺陷状态本身（Act Remaining #2）。
  - 修复后该链产生的 Update 记录：old_tuple=(5,0) 键控、new_tuple=(NULL,0) 无键——恢复侧 keyless 桶 NEW 版本追踪路径仍被见证；old_tuple 无键的 old-lookup 子路径失去运行期生产者（MS10-T05 语义 + MS15-T01 后 SQL 面不可达，行级论证见 design D2 结构性后果注）。
  - 校准后可用断言面：KeyNotFound 错误变体（`StorageError::KeyNotFound` 经 `Response::Error`）——同文件 `:120-122` 已有错误变体匹配先例（DuplicateKey 守卫）。
- Code and Critical Path: 仅测试文件修改；产品代码零触碰（T5 diff 保持）。

**Implementation Guidance**

校准保持测试骨架与意图（无键行 UPDATE 链 + 崩溃重开的恢复完整性），只重述与新语义矛盾的部分；文档注释按新语义改写（含 Act Remaining #2 的「索引 key5 → v2」句）。建议形态（非实质细节可就地调整）：

1. 注释块 `:127-138` 重述：链 = INSERT 三行 → UPDATE 键位转无键（keyless NEW_tuple Update 记录，keyless 桶重放见证）→ 第二次 UPDATE `KeyNotFound`（I037 修复语义：残留条目消除，键位等值不可达）→ 崩溃重开恢复完整。
2. `:168-172` 第二次 UPDATE 臂改为断言 `Response::Error` 且消息含 `Key not found`（R1 不可达场景直接见证）。
3. 恢复断言：COUNT=3 不变；`:188-194`（v=42）改为「v=0 行恰 1」（键位转无键行 (NULL,0) 经非键谓词可达——keyless NEW_tuple 重放见证）；v=1 与可键控行断言不变；追加重开后 `INSERT (5, 200)` 成功（恢复侧索引无残留条目——R1-S3 一致性收口）。
4. RED 观察点：校准后该测试相对当前工作区应为 GREEN（T5 已实施）——校准本身无 RED 阶段（测试见证的对象行为已由 000-initial T4 RED→GREEN 覆盖），本 Cycle 验证 = 全量回归。

**Behavioral Change**

- 产品行为：零变化（本 Cycle 仅测试文件）。
- 测试断言变化：T8-R2 第二次 UPDATE 期望由成功改为 `KeyNotFound` 错误；恢复面 v=42 断言改为 v=0 行可见 + 重开后旧键 INSERT 成功；注释按新语义改写。
- spec 映射：R1 新增场景「键位无键行对键位等值 UPDATE 不可达」（delta spec 已修订）；R2 校准条款（delta spec 已修订）。

**Task Contracts**

### T6: R2 回归收尾（含 T8-R2 语义校准）——修订后全局 task

- Requirement/Scenario: R2 S「既有 UPDATE 行为零回归」（校准条款）+ R1 S「键位无键行对键位等值 UPDATE 不可达」（新场景）
- Depends on: T5（000-initial，已完成且验证有效）
- Targets: `tests/keyless_row_test.rs::keyless_row_update_recovery_after_crash`（链断言 + 文档注释）
- Current behavior: 测试在 `:171` 失败（第二次 UPDATE 期望成功，实际 `Key not found`）——锁定了 I037 缺陷行为（残留条目可达性）
- Required behavior: 校准后测试全绿——第二次 UPDATE 断言 KeyNotFound；恢复面按新链断言（COUNT=3、v=0 行可见、v=1 行可见、可键控行点查+判重守卫、重开后旧键 INSERT 成功）；注释按新语义改写
- Required changes: 仅该测试函数体与其文档注释；同文件其余 3 用例零修改
- Preserve: 测试意图（无键行 UPDATE 链 + 崩溃恢复完整性）；`flush_all` catalog 落盘夹具先例；错误变体匹配先例（`:120-122`）
- Forbidden: 修改产品代码（T5 diff 保持）；修改同文件其他用例；引入合成 WAL 构造
- Test witness: `cargo test --test keyless_row_test` → 4 passed / 0 failed；全量 `cargo test --no-fail-fast` → 861 passed / 0 failed / 2 ignored（基线 860+1 只增不减）
- GREEN condition: 上述两命令退出码 0
- Verification: 全量 + `cargo clippy --all-targets -- -D warnings`（0）+ `cargo fmt --check`（0）+ `openspec validate --changes`（PASS）
- Stop when: 校准后恢复面断言无法在 R1 语义下成立（如 keyless NEW_tuple 重放后 v=0 行不可见——恢复路径与运行期语义矛盾，实质，返回 Plan）

**Invariants**

- T5 实施面（update.rs Step 7 分支）零触碰；无键行存储语义（落库不入索引）与恢复重放语义（MS10-T02 R7/R8、MS10-T05 keyless 桶）不变；既有测试意图保持（校准只重述与已批准 R1 矛盾的断言）。

**Non-goals**

- 合成 WAL 见证基建；`wal-recovery-replay-integrity` 主 spec 修订（docs-maintainer 收尾面：I 项候选登记）；rekey 形态；I039；性能。

**Acceptance**

RTM（replan 增量；T4/T5 行见 000-initial RTM，保持有效）：

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 | 键位无键行对键位等值 UPDATE 不可达（新场景） | D2 结构性后果 | T6 | 001 | 产品零修改（T5 已交付行为） | 校准后 T8-R2 第二次 UPDATE 断言 | None | Covered |
| R1 | 崩溃恢复两态一致（keyless NEW_tuple 重放见证保持） | D2 | T6 | 001 | 产品零修改 | 校准后 T8-R2 恢复面断言 + 000-initial T4 恢复用例 | None | Covered |
| R2 | 既有 UPDATE 行为零回归（校准条款） | D2 | T6 | 001 | 产品零修改 | keyless_row 4/4 + 全量 861/0/2 | None | Covered |

**Verification**

- `cargo test --no-fail-fast` 全量：861 passed / 0 failed / 2 ignored（唯一失败 T8-R2 消除），决定性输出记入 Act Response。
- `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` / `openspec validate --changes` 全 0/PASS。
- Persisted Evidence 为 none（全量可低成本重跑，Act Response 足够）。

**Gate 2 Readiness**

- 无 Missing requirement：PASS（replan RTM 全 Covered；R1 新场景与 R2 校准条款已入 delta spec）
- Simplified requirement 已批准：PASS（无 Simplified 项）
- 调查完整：PASS（T8-R2 全文行级 + 冲突机制独立复现 + 恢复路径影响面论证，见 Investigation Facts 与 design D2）
- 设计闭合：PASS（校准形态、保留面、禁止面明确；恢复一致性论证保持有效——T5 验证未失效）
- 任务可执行：PASS（T6 契约完整：位置/行为/见证/停止条件）
- 分轮合理：PASS（replan 单 Cycle 收口单 gap；Iteration Plan 结构不变，仅验收边界措辞修订）
- 追踪完整：PASS（RTM + spec 场景 + 契约映射闭合）
- 验证充分：PASS（全量回归 + 目标套件 + 静态检查）
- 无身份型证据工程：PASS
- 无实质未知项/TBD：PASS（keyless old_tuple 重放子路径见证缺失已定性为 legacy 兼容面 + I 项候选，不阻塞）
- OpenSpec 产物一致：PASS（spec R1/R2 修订、tasks T6 修订、design D2 注记与本 Cycle 交叉一致——validate 于校准实施后复跑确认）
- Persisted Evidence 模式明确：PASS（none）
- 用户批准计划：PASS（2026-09-12 用户原话"批准"——批准 T8-R2 语义校准（修改既有测试 `keyless_row_test::keyless_row_update_recovery_after_crash` + spec R2 零修改措辞修订 + R1 新场景），replan 方向 a 确认、合成 WAL 备选维持排除）

Gate 2 结论：全部 PASS（含 1 项由 BLOCKED 转 PASS 的用户批准项），Plan Context 状态改 `ready`，交接 `openspec-act` 恢复执行（000-initial blocked → 本 replan Cycle T6）。

**Persisted Evidence**

- Mode: none

Budget: 不适用。

**Risks and Notes**

- 校准后 T8-R2 的「UPDATE 链」语义变化（第二次 UPDATE 由成功变 KeyNotFound）是已批准 R1 的直接后果，非回归；测试标题与注释需如实反映。
- 全量已知偶发 I041（resolve env 竞态）——失败先重跑再判定。
- keyless old_tuple 重放子路径从此无运行期见证（legacy WAL 兼容面）——若用户希望保留 e2e 见证，备选为合成 WAL 测试（本 Cycle 排除，可作 I 项后续独立实施）。

## Act Response

- Status: reported

**Implemented**

T6 按 replan 契约完成（2026-09-12）：`keyless_row_update_recovery_after_crash` 按 I037 修复语义校准 + 回归收尾。产品代码零触碰（T5 diff 保持，未回滚不重做）。具体：

1. 测试 doc 注释按新链重述（keyed→keyless Update 记录 + keyless 桶重放见证 + 第二次 UPDATE KeyNotFound + 恢复完整）。
2. 第二次 UPDATE 臂改为断言 `Response::Error` 且消息含 `Key not found`（R1 新场景「键位无键行对键位等值 UPDATE 不可达」直接见证）。
3. 恢复面断言按新链：COUNT=3 保持；v=42 断言改为 v=0 行恰 1（键位转无键行 (NULL,0) 经非键谓词可达——keyless NEW_tuple 重放见证）；v=1 行、可键控行点查与判重守卫不变；追加重开后 `INSERT (5, 200)` 成功（恢复侧索引无残留条目——R1-S3 一致性收口）。
4. 校准无 RED 阶段（契约既定）；修改前基线见证确认旧断言在 `:171` 失败且错误消息逐字符合 Plan Context 预测。

**Changed Files and Symbols**

- `tests/keyless_row_test.rs`（仅此 1 文件）：
  - `keyless_row_update_recovery_after_crash`——第二次 UPDATE 断言改 `Response::Error { message }` + `contains("Key not found")`；恢复面 `WHERE v = 42` → `WHERE v = 0`；追加 `INSERT INTO t VALUES (5, 200)` 成功断言。
  - 模块 doc「R2 见证」行与「UPDATE 可达性注记」、测试 doc 注释、`:169`/`:171`/`:178` 三处行内注释按新语义改写。
  - 同文件其余 3 用例（`string_first_column_table_accepts_inserts` / `null_key_position_row_insert_visible` / `keyed_duplicate_still_rejected`）零修改（diff 确认）。

**Deviations from Plan**

1. 注释重述范围含模块级 doc（「R2 见证」行 + 「UPDATE 可达性注记」段），契约点名的是测试 doc 块与「索引 key5 → v2」句——模块 doc 描述的「keyed → keyless → 再次 UPDATE」可达路径与已批准 R1 矛盾（再次 UPDATE 自此 KeyNotFound），按 Implementation Guidance「只重述与新语义矛盾的部分」一并改写。comment-only，非实质。
2. 实施中发现并即时修复一处编辑引入的变量笔误（重开后 `(8, 0)` INSERT 断言行 `db` → `db2`，`db` 已被 drop 会编译失败）；修复发生于首次 GREEN 运行前，最终 diff 正确，无验证结论受影响。

**Blocker Handoff**

None（本 Cycle 正常完成；000-initial 的 Blocker 已由本 replan 解决）。

**Blocker Resolution**

- User instruction: 2026-09-12 用户批准 001-replan（Gate 2 Readiness 记录原话"批准"——T8-R2 语义校准方向 a、spec R2 校准条款与 R1 新场景；合成 WAL 备选维持排除）
- Resolution: replan Cycle 001 执行 T6 校准 + 回归收尾；000-initial 的 T4/T5 工作保留且验证有效（独立复核采信，见 Investigation Facts）
- Accepted risk: 「old_tuple 无键」old-lookup 重放子路径自此无 SQL 运行期 e2e 见证（legacy WAL 兼容面；I 项候选登记归 docs-maintainer 收尾面）
- Resume point: 000-initial blocked → 本 replan Cycle 恢复执行（T6 校准 + 收尾）——已完成
- Required verification: T6 契约验证清单——全部通过（见 Verification Evidence）

**Self-Review**

- Spec review（先于 code review）：T6 契约逐项核对——R1 新场景见证 + R2 校准条款断言面完整；Targets/Preserve/Forbidden 面零违反（产品代码零触碰、其余 3 用例零修改、无合成 WAL）；测试意图保持（无键行 UPDATE 链 + 崩溃恢复完整性）；Gate 证据覆盖 GREEN condition。
- Code quality review：diff 无计划外行为修改（注释范围扩展记入 Deviations 1）；断言按行为证据设计（错误消息 `contains` 抗前缀变化、v=0 行恰 1、旧键 INSERT 成功均为可观察行为）；测试不会因错误原因通过（KeyNotFound 消息点名）；命名与 match/panic 风格同文件既有惯例；无新增警告、死代码、重复实现或身份型证据机制。
- 已修复发现：`db`→`db2` 变量笔误（Deviations 2，GREEN 前修复）。
- 遗留 Minor：无。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| 基线见证（修改前） | `cargo test --test keyless_row_test` | `test result: FAILED. 3 passed; 1 failed`——`:171` panic "对无键版本的 UPDATE 应成功，实际 Error { message: "Execution error: Key not found" }" | T8-R2 旧断言锁定 I037 缺陷行为 | 与 Plan Context「Current behavior」逐字一致 |
| 目标套件（修改后） | `cargo test --test keyless_row_test` | `test result: ok. 4 passed; 0 failed`（exit 0） | T8-R2 校准 + 同文件 3 用例 | PASS |
| 全量回归 | `cargo test --no-fail-fast` | `TOTAL passed=861 failed=0 ignored=2`（exit 0） | 全仓库测试面 | PASS（基线 860+1 只增不减，唯一失败消除） |
| 静态检查 | `cargo clippy --all-targets -- -D warnings` | 0 warning（exit 0） | 全 targets | PASS |
| 格式 | `cargo fmt --check` | 0 diff（exit 0） | 全仓库 | PASS |
| OpenSpec | `openspec validate --changes` | `Totals: 1 passed, 0 failed` | change 产物一致性 | PASS |

**Persisted Evidence**

None required（Plan 模式 none；全量可低成本重跑，Act Response 足够，未创建 evidence/ 目录）。

**Experience Candidates**

None。

**Remaining Issues**

1. 本文件 Plan Context 头部状态字段仍为 `draft`，而 Gate 2 结论与 change tasks.md 均为 `ready`——Plan 区域字段漏改，Act 不代改，留 Plan Review 一并处理（非阻塞）。
2. 「old_tuple 无键」old-lookup 重放子路径无运行期 e2e 见证——I 项候选（replan Non-goals 既定，归 docs-maintainer 收尾时登记）。

**Commit or Diff Reference**

实施未提交（待用户触发；本 Cycle diff = `tests/keyless_row_test.rs` 1 文件，对照基线 f9e1e1f + 000-initial 已保留工作区）。

## Plan Review

- Review Result: accepted

**Findings**

基于实际代码、diff 与独立复跑的检查（Act Self-Review 未代替独立检查）：

- **F1（非阻塞，Plan 区域字段，本 Review 已修复）**：Act Remaining #1 属实——Plan Context Status 字段漏改（`draft`，Gate 2 已通过时的应然状态为 `ready`）。Plan 于本 Review 补正；Gate 2 结论行当时已写明 `ready`，属字段遗漏非流程问题。
- **F2（非实质，接受）**：Deviation 1 注释重述范围扩展至模块级 doc——被改写的「keyed → keyless → 再次 UPDATE」可达性描述与已批准 R1 矛盾，落在 Implementation Guidance「只重述与新语义矛盾的部分」授权内；comment-only。
- **F3（非实质，接受）**：Deviation 2 `db`→`db2` 变量笔误在首次 GREEN 运行前修复——过程正确，无验证结论受影响（最终 diff 独立复核确认引用正确）。
- **实现核对**：diff 与 T6 契约逐项吻合——第二次 UPDATE 断言 `Response::Error` + `contains("Key not found")`（R1 新场景「键位无键行对键位等值 UPDATE 不可达」直接见证，错误消息 `contains` 抗前缀变化）；恢复面 COUNT=3 保持、v=42→v=0 行恰 1（keyless NEW_tuple 重放见证）、v=1 与可键控行断言不变、追加重开后 `INSERT (5, 200)` 成功（R1-S3 一致性收口）；模块/测试/行内注释按新语义改写（含「索引 key5 → v2」缺陷句，F4/Act Remaining #2 并入完成）；同文件其余 3 用例零修改（diff 确认）；产品代码零触碰（`git status` 本 Cycle 唯一新增修改面 = `tests/keyless_row_test.rs`）。
- **验证核对（独立复跑）**：目标套件 `cargo test --test keyless_row_test` → 4 passed / 0 failed；全量 `cargo test --no-fail-fast` → 861 passed / 0 failed / 2 ignored、零 FAILED 行（基线 860+1 只增不减，唯一失败消除）；clippy `-D warnings` 0、fmt 0、`openspec validate --changes` PASS。

**Deviation Classification**

- ACT-DEVIATION ×2（均非实质：注释范围扩展、GREEN 前笔误修复）
- PLAN-OMISSION ×0、PLAN-INVALID ×0、BASELINE-CHANGED ×0、NEW-EVIDENCE ×0

**Acceptance Gaps**

None——R1 新场景经校准后 T8-R2 直接见证；R2 校准条款满足（T8-R2 校准 + 其余 3 用例零修改）；全量 861/0/2 全绿。父 Cycle（000-initial）唯一 gap（T8-R2 失败 + 验收边界冲突）全部关闭。

**Convergence**

reduced（父 Cycle gap 全部关闭：keyless_row 1/4 失败 → 4/4；全量 860/1 → 861/0）

**Evidence**

- 独立复跑（2026-09-12）：`cargo test --test keyless_row_test` → `4 passed; 0 failed`；`cargo test --no-fail-fast` → `passed: 861 failed: 0 ignored: 2`（FAILED 行计数 0）；`cargo clippy --all-targets -- -D warnings` exit 0；`cargo fmt --check` 0；`openspec validate --changes` 1 passed / 0 failed。
- 独立 diff 检查：`git diff tests/keyless_row_test.rs`（+46/-21，仅 T8-R2 函数体 + 注释面 + 模块 doc）。
- 采信 Act 未失效结论：基线见证（修改前 `:171` 失败形态与 Plan Context 预测逐字一致）、Risks 探针与 T4/T5 验证（000-initial，覆盖未变化）。

**Follow-up Decision**

既有 Acceptance 已满足、无阻塞项、无当前 Cycle 修复需求 → `accepted`，Iteration 001 完成（000-initial replan-required 链收口）。Minor 项处置：F1 已由 Plan 修复；「old_tuple 无键」old-lookup 重放子路径运行期见证缺失（Act Remaining #2 / replan 既定）与 Iteration 000 遗留的 SELECT 二元算术 I 项候选，一并归 docs-maintainer 收尾登记。按 Map 展开 Iteration 002。

**Iteration Plan Update**

None（replan 契约边界内完成；Iteration Map 不变）

**Next Cycle**

None

**Next Iteration**

`iterations/002-table-name-normalization/000-initial.md`（已按 Map 展开，Plan Context `ready`——T7-T9，I039 表名解析归一化与 dump 保真）
