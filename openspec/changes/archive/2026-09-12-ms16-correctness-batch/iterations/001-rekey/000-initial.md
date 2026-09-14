# Iteration 001 / Cycle 000: rekey 索引一致性（I047）

## Plan Context

- Status: ready
- Iteration: 001-rekey
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T5, T6
- Depends on: Iteration 000（accepted——前置写入校验区已建立；Step 7 既有 I037 分支为三分支基线）
- Stable baseline: rekey 后新键点查可达、旧键点查空集、旧键 INSERT 可用、碰撞写入前拒绝零副作用、恢复两态一致；同键/NULL/非键列分支逐字节保持；全量回归零修改
- Verification boundary: T5/T6 全绿 + 既有锚点套件零修改 + 全量 0 failed + clippy/fmt 0 + `openspec validate` PASS
- Diagnostic boundary: `src/executor/update.rs` + `tests/update_index_maintenance_test.rs`
- Deferred tasks: None（Iteration 001 为本 change 最后一个 Iteration）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R1-R6 已交付成果（Iteration 000 accepted）；update.rs 现有前置校验区（T4 类型校验块）与 Step 7 I037 分支；全部 Invariants
- Excluded scope: 多行 UPDATE 语义（planner 单行契约不变）；索引层 DuplicateKey 机制（node.rs 禁用检查不恢复）；DELETE 执行器；性能优化

**Objective**

`UPDATE SET <键列> = <另一可键控 Int>`（rekey）后索引条目与数据页两态一致：新键点查可达、旧键条目清理（点查空集、旧键 INSERT 可用）、新键撞已有行在任何写入前 `DuplicateKey` 拒绝且零副作用、崩溃恢复后与运行期一致；I037/同键/非键列分支逐字节不变。

**Background**

I047（MS15-Rest 调查新发现 + 同键 rekey 探针实证，improvements 已排期 MS16-T02）：`UpdateExecutor` Step 7 else 臂对键列 SET 为另一可键控值时无条件 `index_manager.update(&self.key, new_row_id)`——旧键条目残留指向新版本（旧键点查返回键位已改的行、旧键 INSERT 被 DuplicateKey 误拒）、新键无索引条目（点查静默空集）、崩溃恢复重建后两态不一致。用户裁定碰撞语义为**写入前拒绝**（Gate 1 集中决策 2）。Iteration 000（accepted）已建立 Step 1 后的前置校验区（类型校验），碰撞预检并入同区。

**Investigation Facts**

- Current Baseline: Iteration 000 最终态（工作区未 commit，对照基线 d8a244f）——全量 **888 passed / 0 failed / 2 ignored**（Plan Review 2026-09-12 独立复跑，001-replan Plan Review accepted 采信）；clippy 0 / fmt clean / validate changes+specs PASS。本 Cycle 开工前 Act 做只读基线检查（`git status`/`git diff --stat` 对照 001-replan Act Response Changed Files）即可采信。
- `UpdateExecutor::next` 现行结构（`src/executor/update.rs`，T4 后）：
  - Step 1：`self.table_meta.index_manager.search(&self.key)` → None 时 `Err(StorageError::KeyNotFound)`（目标行不存在的既有语义）。
  - T4 前置校验块（Step 1 之后、Step 2 之前）：`column_name == pk_column` 时按键列声明类型判定——声明 Int 且 `new_value` 非 `Int|Null` → `Err(KeyTypeMismatch)`（本轮 iter 前已落地，GREEN）。
  - Step 2-6：读旧 tuple（M20 闭包）→ 改列 → 序列化 → `write_tuple_to_data_page` 写新版本（`new_row_id`）→ `clear_all_visible`（新旧页）→ WAL `WalRecord::Update` → `record_version`。
  - Step 7：`if self.column_name == self.table_meta.pk_column && self.new_value.to_key().is_none() { index_manager.delete(&self.key) } else { index_manager.update(&self.key, new_row_id) }`（I037 修复形态；rekey 走 else 臂——本 Cycle 缺陷点）。
- 索引层事实（`src/storage/btree/index_manager.rs`）：`search(key) -> Option<RowId>`（106）；`insert(key, row_id)` 不拒重复（node.rs:127 DuplicateKey 检查已注释禁用——唯一性契约由执行器先查后写承担，INSERT 先例 insert.rs:102-111）；`delete(key)` 内部先 search 并按命中 row_id 清理 `row_to_key` 反向映射（245-266）——**rekey 必须先删后插**（若先 `update(old→new_row_id)` 再删，delete 的 search 命中新 row_id、清错反向映射）；`update(key, new_row_id)` 替换值 + 重录反向映射（306-322）。`Key::as_bytes()` 为既有 API（insert.rs:106 使用）。
- 键比较来源：`self.key: Vec<u8>`（旧键字节，planner `extract_pk_from_where` 生成）与 `self.new_value.to_key()`（`Option<Key>`，仅 Int 有值）——同键判定 = `new_key.as_bytes() == self.key`。
- RED 行为预测（代码级推演，Act RED 观察核对）：
  - 单行 rekey 5→7：else 臂 `update(5→new_row_id)` → `WHERE id = 7` IndexScan 空集（新键无条目）；`WHERE id = 5` 经残留条目返回键位已为 7 的行 `(7, 100)`；`INSERT (5, 200)` 被 DuplicateKey 误拒。
  - 碰撞 rekey（行 (5,100)、(7,200)，`SET id = 7 WHERE id = 5`）：无预检 → 成功 affected 1，行集被静默改写（两行 id 均为 7 的数据态、`WHERE id = 5` 返回 `(7, 100)`）。
  - 崩溃恢复：重建索引以数据页为准（(7,100) 入 7、5 无条目）——运行期（错误态）与恢复面（正确态）不一致。
- 测试夹具先例（`tests/update_index_maintenance_test.rs`）：`db_path` + AffectedRows 模式匹配 + COUNT 断言；恢复用例流程 = CREATE → `buffer_pool.flush_all()`（DDL 无 WAL 记录）→ 写入 → 运行期断言 → `wal_buffer.shutdown()` + drop（不 close）→ 重开断言。
- 键位等值 UPDATE 只触达索引内行（build_update 强制 PK 等值 WHERE，ddl_dml.rs:359-411）——无键行对键位等值 UPDATE 不可达（`KeyNotFound`，update-index-maintenance R1-S4 已验收语义），keyless→keyed 方向在本执行器天然不可达，无需处理。

**Implementation Guidance**

建议顺序：T5（RED 见证，4 新用例）→ T6（实现，转 GREEN + 全量收尾）。

- T6 形态建议：碰撞预检并入 T4 前置校验块（类型校验之后）——`column_name == pk_column` 且 `new_value.to_key()` 为 `Some(new_key)` 且 `new_key.as_bytes() != self.key` 时 `index_manager.search(new_key.as_bytes()).await?` 命中即 `Err(StorageError::DuplicateKey)`；Step 7 改三分支：`to_key().is_none()` → `delete(&self.key)`（I037 原样）；`new_key == self.key`（字节比较）→ `update(&self.key, new_row_id)`（原样）；否则 `delete(&self.key).await?` 后 `insert(new_key.as_bytes(), new_row_id).await?`（顺序固定：先删后插，理由见 Investigation Facts 索引层事实）。等价局部形态由 Act 定。
- 断言风格沿用该测试文件先例（AffectedRows 模式匹配 + COUNT + 点查行集）。

**Behavioral Change**

- UPDATE rekey（新值可键控且 ≠ 旧键）：由「旧键条目残留 + 新键无条目 + 碰撞静默改写」变为「写入前碰撞拒绝（DuplicateKey，零副作用）或删旧键条目 + 插新键条目」；新键点查可达、旧键点查空集、旧键 INSERT 可用、恢复两态一致。
- 错误面：碰撞拒绝复用 `StorageError::DuplicateKey`（INSERT 同文，exit 3）；不新增变体。
- 逐字节不变：I037 分支（SET 键列为 NULL → 删旧键条目）、同键原值更新（update 路径）、非键列更新（else→update 路径）、KeyNotFound 优先级、KeyTypeMismatch 校验（先于碰撞预检）。

**Task Contracts**

### T5: rekey 索引一致性 RED 测试见证（R4）

- Requirement/Scenario: R4 前 4 场景（新键可达 / 旧键清理 / 碰撞写入前拒绝 / 恢复两态一致）
- Depends on: None（Iteration 000 成果已在工作区）
- Targets: `tests/update_index_maintenance_test.rs`（文件尾追加 I047 段，4 新用例）
- Current behavior: 见 Investigation Facts「RED 行为预测」（三条缺陷形态均代码级推演成立）
- Required behavior: 测试断言目标行为——rekey 新键点查返回 rekeyed 行、旧键点查空集且旧键 INSERT 成功、碰撞 UPDATE 报 Error 且两行零副作用、恢复两态一致；实现前全部 RED
- Required changes: 仅新增测试函数（doc-comment 注明 I047 依据与 RED 预测）；不改既有测试
- Preserve: 既有 5 用例逐字节不变（含 I037 三用例与两锚点）；夹具 helper 复用不修改
- Forbidden: 不改产品代码；不改既有断言
- Test witness: `cargo test --test update_index_maintenance_test`——4 新用例 RED（形态与预测一致），既有 5 用例 GREEN
- GREEN condition: T6 完成后全部转 GREEN
- Verification: 命令输出与退出码记录于 Act Response
- Stop when: RED 形态与预测不符（如某形态现已正确）——实质基线发现，返回 Plan

### T6: rekey 实现——写入前碰撞预检 + Step 7 三分支（R4）+ 全量收尾

- Requirement/Scenario: R4 全部场景；R5 负空间约束（I037/同键/非键列/KeyNotFound/KeyTypeMismatch 优先级）
- Depends on: T5（RED 已观察）
- Targets: `src/executor/update.rs`（前置校验块扩碰撞预检 + Step 7 三分支）
- Current behavior: Step 7 else 臂对 rekey 无条件 `update(&self.key, new_row_id)`（Investigation Facts）
- Required behavior: 前置块内（类型校验后）碰撞预检——新值可键控且新键 ≠ 旧键时 `search(new_key)` 命中即 `Err(DuplicateKey)`（任何写入前，零副作用）；Step 7 三分支——NULL → delete（I037 原样）/ 同键 → update（原样）/ rekey → 先 `delete(old)` 后 `insert(new_key, new_row_id)`（顺序固定）
- Required changes: 仅 `update.rs` 两处（前置块 + Step 7）；全量收尾（`cargo test` 全量、clippy --all-targets -- -D warnings、fmt --check、`openspec validate`）
- Preserve: I037 分支、同键分支、非键列 else 臂、KeyNotFound 与 KeyTypeMismatch 校验位置与语义、WAL/版本链写入时序（碰撞拒绝在任何写入前）、`row_to_key` 反向映射一致性（先删后插）
- Forbidden: 不改 planner/存储层/索引层；不改 restore/import；不引入索引层 DuplicateKey 机制
- Test witness: T5 4 用例转 GREEN；既有 `update_index_maintenance_test` 5 用例 + `keyless_row_test` 4 用例零修改通过
- GREEN condition: T5 全绿 + 锚点全绿 + 全量 0 failed（预期 ≥892 passed：888 + T5 新 4）+ clippy/fmt/validate 全 0/PASS
- Verification: 全量与静态检查输出（≤20 行/项）+ 退出码记录于 Act Response
- Stop when: 三分支无法在不触碰索引层的前提下闭合（实质设计偏差）；或 delete/insert 顺序导致反向映射断言失败（实质发现）

**Invariants**

- I037（SET 键列为 NULL → 删旧键条目）逐字节不变；同键原值更新路径不变；非键列更新路径不变。
- 错误优先级：KeyNotFound（Step 1）→ KeyTypeMismatch（类型校验）→ DuplicateKey（碰撞预检）→ 写入。
- 碰撞拒绝零副作用（行集、索引条目、WAL 不变）。
- 既有测试零修改通过（T5 新增见证除外）；基线 888 只增不减。
- 执行器外（planner/存储/索引层）零改动。

**Non-goals**

- 多行 UPDATE / 表达式 SET（planner 单行字面量契约不变）；DELETE 执行器 rekey 面键位等值对非 Int 键列 KeyNotFound 保持；partial INSERT；I041 修复；性能优化。

**Acceptance**

Requirements Traceability Matrix（Iteration 001 范围 R4；R1-R6 已于 Iteration 000 覆盖）：

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R4 键位 rekey 后索引条目一致 | 新键可达/旧键清理/碰撞写入前拒绝/恢复两态一致/同键保持 | D4 | T5, T6 | 001 | `update.rs` 前置块碰撞预检 + Step 7 三分支 | `update_index_maintenance_test` 新增 4 用例 + 既有 5 用例零修改 | None | Covered |

**Verification**

- T5：目标套件 RED 观察；T6：转 GREEN + 全量（预期 ≥892 passed / 0 failed / 2 ignored）+ clippy/fmt/validate 全 0/PASS。
- 验证直接观察行集/错误变体/退出码/恢复面行为，不引入身份型证据机制。

**Gate 2 Readiness**

- 无 Missing requirement：PASS——R4 全场景映射 T5/T6（Acceptance RTM）；R1-R6 已于 Iteration 000 Covered。
- 无未批准 Simplified：PASS——Simplification 列全 None。
- 调查完整：PASS——update.rs 现行结构（T4 后）、索引层语义（delete 先删后插依据、insert 无重复拒绝）、RED 行为推演、夹具先例均有代码级证据。
- 设计闭合：PASS——design D4（含顺序约束理由、错误路径、索引层事实）；无 TBD。
- 任务可执行：PASS——T5/T6 契约含 Targets/Current/Required/Preserve/Forbidden/RED/GREEN/停止条件。
- 分轮合理：PASS——Iteration 001 为 Map 最后一个 Iteration，tasks.md 平衡审计已通过。
- 追踪完整：PASS——R→S→D→T→代码面→测试见证闭环。
- 验证充分：PASS——RED→GREEN + 全量 + 静态检查；无身份型证据工程；Persisted Evidence none。
- 无实质未知项：PASS——非实质选择（分支内局部形态）留 Act。
- tasks/specs/design/Cycle 一致：PASS——T5/T6 为 Map 既有任务；delta spec `update-index-maintenance` ADDED Requirement 就绪；design D4。
- 用户批准：PASS——change 级 Gate 2 2026-09-12 用户批准（「批准，交 Act 执行」，计划含 Iteration 001 范围）；Iteration 000 已 accepted（001-replan Plan Review），Current Baseline 经 Plan Review 独立复跑采信。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果；无不可低成本重跑的验收面。

- Budget: 不适用（none）。

**Risks and Notes**

- I041 flaky 可能撞全量收尾（约 1/6）——处置同 T8 契约先例：与 resolve 相关的失败单独复跑确认 flaky 属性并在 Response 注明，不计入本 change 失败。
- rekey 先删后插的 IO 失败窗口（insert 失败时旧键条目已删，直至重开重建）——与既有 I037 delete 暴露同类，不新增风险类别（design Risks 已记录）。
- 碰撞预检与写入之间的窗口与 INSERT 先查后写同构（进程内单写者 + 文件锁独占），不新增并发面。

## Act Response

- Status: blocked

**Implemented**

- T5（完成）：`tests/update_index_maintenance_test.rs` 文件尾追加 I047 段（段注释 + 4 用例）——R4-S1 `rekey_new_key_point_query_reachable`（新键点查返回 rekeyed 行）、R4-S2 `rekey_old_key_cleaned_insert_available`（旧键点查空集 + 旧键 INSERT 可用 + COUNT 2）、R4-S3 `rekey_collision_rejected_before_write_zero_side_effects`（碰撞 UPDATE 报 Error 含 duplicate 文案 + 两行原样零副作用）、R4-S4 `recovery_matches_runtime_after_rekey`（运行期/恢复面同断言：新键可达、旧键空集、恢复后旧键 INSERT 可用）。断言风格沿用该文件内联 match 先例，既有 5 用例与 `db_path` helper 逐字节未动。RED 观察：4 用例全 RED 且形态与 Plan Context 预测逐条一致（新键 `left: []`、旧键点查 `left: 1`、碰撞 `AffectedRows { count: 1 }`、恢复运行期 `left: []`），既有 5 用例 GREEN。
- T6（实现完成，收尾 step 阻塞）：`src/executor/update.rs` 恰两处——①前置校验块（`KeyTypeMismatch` 之后、同一 `pk_column` 守卫内）并入碰撞预检：`new_value.to_key()` 为 `Some` 且 `new_key.as_bytes() != self.key` 时 `search(new_key)` 命中即 `Err(DuplicateKey)`（任何写入前，与 INSERT 先查后写同模式；同键字节相等不做预检）；②Step 7 三分支：非键列 → `update(&self.key, new_row_id)`（原样）；键列 + `to_key()` 为 None → `delete(&self.key)`（I037 原样）；键列 + 新键与旧键字节相等 → `update`（原样）；键列 + 新键不等 → 先 `delete(&self.key)` 后 `insert(new_key.as_bytes(), new_row_id)`（顺序固定，依据 delete 的 search 反向映射清理语义）。目标套件 9 passed / 0 failed（4 新用例转 GREEN + 既有 5 用例零回归），锚点套件 26 passed 全绿（key_type_conformance 8 + keyless_eq_routing 14 + keyless_row 4）。全量回归出现 6 个 Plan 未预见的确定性失败（见 Blocker Handoff），GREEN condition「全量 0 failed」不满足，Gate 5 未通过。

**Changed Files and Symbols**

- `src/executor/update.rs`：`UpdateExecutor::next`——前置校验块扩碰撞预检（`Value::to_key` / `Key::as_bytes` / `index_manager.search`，新增约 17 行含注释）+ Step 7 重构为「非键列 else + 键列 match 三分支（None / 同键 / rekey）」。函数签名、Step 1-6、KeyNotFound/KeyTypeMismatch 位置与语义、WAL/版本链写入时序均未改动。
- `tests/update_index_maintenance_test.rs`：文件尾追加 I047 段注释 + 4 测试函数（约 240 行）；既有内容逐字节未动。

**Deviations from Plan**

- 无契约内偏差——实现形态与 Plan Context Implementation Guidance 建议一致（碰撞预检位置与条件、三分支结构、先删后插顺序、同键字节比较、锚点与夹具复用）。
- 阻塞为 Plan 层面缺口（见 Blocker Handoff），非 Act 执行偏差。

**Blocker Handoff**

- 发现位置：T6 全量收尾 step（Gate 5 / GREEN condition「全量 0 failed」）。
- Plan 预期：Iteration 001 Stable baseline「全量回归零修改」；T6 契约 Test witness 将全量影响面锁定为「`update_index_maintenance_test` 5 用例 + `keyless_row_test` 4 用例零修改通过」；Invariant「既有测试零修改通过（T5 新增见证除外）；基线 888 只增不减」。
- 实际：全量 `cargo test --no-fail-fast` 出现 **6 个确定性失败**（两次完整失败清单一致，单套件复跑复现），全部为直连构造 `UpdateExecutor` 的 M10 时代单测，以「SET 键列 = 另一 Int 值」建立版本链，并**以旧键 search/IndexScan 定位新版本或寻址后续 UPDATE**——依赖 I047 缺陷行为（旧键条目残留指向新版本）。修复后语义正确（旧键条目删除、新键条目建立），断言必然失败：
  1. `gc_test::test_gc_removes_old_versions`——panic gc_test.rs:85 `v2 should exist`（rekey 10→20 后 search 旧键 10）
  2. `gc_test::test_gc_preserves_uncommitted_versions`——panic gc_test.rs:223 `v2 should exist`（rekey 100→200 后 search 旧键 100）
  3. `gc_test::test_gc_multiple_keys`——`Error: KeyNotFound`（链式 rekey 10→11→12，tx3 仍以旧键 10 寻址 → Step 1 正确 KeyNotFound）
  4. `plan_exec_test::test_insert_update_scan_flow`——panic plan_exec_test.rs:193 `Expected Row with Int(1000) after update, got None`（rekey 1→1000 后以旧键 1 IndexScan）
  5. `version_chain_test::test_version_chain_traversal`——panic version_chain_test.rs:95 `v2 should exist`（rekey 10→20 后 search 旧键 10）
  6. `version_chain_test::test_version_chain_skips_invisible`——`Error: KeyNotFound`（链式 rekey 100→200→300，tx3 以旧键 100 寻址）
- 影响判断：与 Iteration 000 BH-1 同类——既有测试编码了被修复的缺陷行为；测试主题（GC / 版本链 / 插改扫流）本身不受影响，但其「以旧键定位 rekey 后行」的夹具模式在新语义下必须改为按当前键寻址。非实现缺陷：失败形态与 T5 RED 预测、I047 缺陷定义完全一致，恰是修复生效的证据面。
- Plan 依据缺口：design.md 无 gc_test / version_chain_test / plan_exec_test 相关内容（grep 零命中）；Investigation Facts 未覆盖直连执行器 MVCC/GC 测试面。
- 契约冲突：T6 Invariant「既有测试零修改通过」与「全量 0 failed」在不修改该 6 个测试或回退修复的前提下不可同时成立。Act 无权校准既有测试（超出契约 Preserve/Forbidden 授权；禁止自行补全 Plan 遗漏设计或扩大变更面）。
- 任务状态：T5 **done**；T6 实现完成但收尾 step 阻塞（GREEN condition 未满足，未标记 done）。
- 已通过 Gate：Gate 1/2（cycle 前置授权）；Gate 3（T5 RED 见证 + T6 RED→GREEN 见证）；T5 的 Gate 4/5（目标套件范围）；T6 Gate 4/5 未达。
- 工作区状态：本 Cycle 改动 = `src/executor/update.rs` + `tests/update_index_maintenance_test.rs`（未 commit，未回滚）；工作区同时含 Iteration 000 两轮改动与用户 docs 待提交改动（未触碰）。
- 恢复条件（二选一）：① 调用 `openspec-plan` Review 本 Cycle——预计裁定 replan-required，由 Plan 按 BH-1 先例裁定校准方案（6 测试改为按当前键寻址）并记录校准依据，创建后继 Cycle 执行校准 + 收尾；② 用户明确授权 Act 在当前 Cycle 内校准该 6 测试（扩大执行契约，需保留原话与风险记录）。恢复后自「全量收尾」恢复点继续：T6 实现与 T5 见证无需重做。

**Blocker Resolution**

None（未恢复）

**Self-Review**

- Plan compliance: T5 逐条符合契约（Targets/Required/Preserve/Forbidden/RED 形态全对齐，stop condition 未命中）；T6 实现逐条符合契约（仅 update.rs 两处、Preserve 全部保持、Forbidden 未触、两条 stop condition 均未命中）——阻塞来自契约未覆盖的全量失败面，非契约执行偏差。
- Full diff reviewed: 是——本 Cycle diff 仅上述两文件；`update.rs` 两处改动与测试追加段全文审读；Iteration 000 既有改动与用户 docs 待提交改动零触碰。
- Critical findings unresolved: 无。
- Important findings unresolved: 1——即 Blocker：6 个既有测试依赖 I047 缺陷行为，全量收尾无法闭环（已按 Gate 6 交接）。
- Minor findings unresolved: 无。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T5 RED 见证 | `cargo test --test update_index_maintenance_test` | `test result: FAILED. 5 passed; 4 failed`——4 用例失败形态与 Plan Context 预测逐条一致（新键 `left: []`、旧键 `left: 1`、碰撞 `AffectedRows { count: 1 }`、恢复运行期 `left: []`） | I047 三缺陷形态 + 两态不一致，修复前 RED | PASS（预期 RED） |
| T6 GREEN（目标套件） | `cargo test --test update_index_maintenance_test` | `test result: ok. 9 passed; 0 failed; 0 ignored` | R4 全场景（4 新 + 同键锚点）+ I037 三用例 + 两行为保持锚点 | PASS |
| 锚点套件 | `cargo test --test keyless_row_test --test key_type_conformance_test --test keyless_eq_routing_test` | `4 passed` + `8 passed` + `14 passed`，全 0 failed | 键位类型强制 / 路由类型门 / 无键行写入面零回归 | PASS |
| 全量回归（阻塞面） | `cargo test --no-fail-fast`（多轮） | 完整失败清单两次一致：gc_test `0 passed; 3 failed`、plan_exec_test `3 passed; 1 failed`、version_chain_test `1 passed; 2 failed`；另一轮另见 `cli::resolve::tests::test_db_dir_env_cases` 偶发（I041 已知项） | 全仓测试面——暴露 Plan 未预见的直连执行器测试依赖 | FAIL（阻塞，见 Blocker Handoff） |
| clippy / fmt / validate | 未运行 | — | — | 未达（T6 收尾按序阻塞于全量步骤） |

**Persisted Evidence**

（None required——Plan Context Persisted Evidence: none；失败确定性且单套件命令即可低成本复现，根因与形态已在本 Handoff 完整记录，无不可低成本重跑的验收面）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook / Incident | None | None | None（Plan 兼容性预测缺口属 change 内 Plan-Act 线程事务，非可复用操作路径或重要故障） |

**Remaining Issues**

- Blocker（见 Handoff）：6 个既有直连执行器测试依赖 I047 缺陷行为，需 Plan 裁定校准方案（BH-1 同类）后方可全量收尾。
- I041（resolve env 测试竞态）：全量运行中偶发 1 次，已知既有项，单独复跑未现，不计入本 change 失败。

**Commit or Diff Reference**

未 commit（本 Cycle 改动：`src/executor/update.rs` + `tests/update_index_maintenance_test.rs`；对照基线 d8a244f + Iteration 000 两轮工作区改动叠加，用户 docs 待提交改动未触碰）。

## Plan Review

- Review Result: replan-required

**Findings**

独立核查（不依赖 Act Self-Review）——T6 diff 全文审读、四套件独立复跑、6 失败用例代码级根因确认、UpdateExecutor 全部测试用法排查：

- **F1（确认项）**：T5 RED 见证成立——4 新用例失败形态与 Plan Context RED 预测逐条一致（新键空集 `left: []`、旧键点查 `left: 1`、碰撞 `AffectedRows { count: 1 }`、恢复运行期 `left: []`），既有 5 用例 GREEN；断言风格沿用目标文件先例，既有内容逐字节未动。Plan 独立复跑目标套件 9 passed / 0 failed（T6 后 GREEN）。
- **F2（确认项）**：T6 实现与 design D4 逐条一致——碰撞预检位于 `pk_column` 守卫内、`KeyTypeMismatch` 之后（错误优先级 KeyNotFound → KeyTypeMismatch → DuplicateKey → 写入保持）；条件为 `to_key()` Some + 新旧键字节不等 + `search` 命中；Step 7 三分支重构与既有行为逐路径等价（非键列 → update / pk+None → delete〔I037 原样〕/ pk+同键 → update〔原样〕/ pk+异键 → 先删后插，顺序依据 delete 的 search 反向映射清理语义）；`src/` 其余零改动，Preserve/Forbidden 全部满足。
- **F3（确认项）**：BH-3 属实且为 PLAN-INVALID——Plan 独立复跑四套件与 Act 报告逐项一致（gc_test 0 passed/3 failed、plan_exec_test 3 passed/1 failed、version_chain_test 1 passed/2 failed、目标套件 9 passed）；6 用例寻址链代码级定性（gc_test.rs:85/:223 panic `v2 should exist`、multiple_keys tx3 旧键寻址 `KeyNotFound`、plan_exec_test.rs:193 IndexScan None、version_chain_test.rs:95/:skips_invisible 同模式）——全部为「SET 键列 = 另一 Int 后按旧键定位/寻址」的缺陷行为依赖，与 T5 RED 预测、I047 缺陷定义一致，恰是修复生效的证据面；测试主题（GC/版本链可见性/插改扫流）本身不受影响。
- **F4（确认项）**：受影响面排查闭合——引用 `UpdateExecutor` 的 7 个测试文件逐一定性：`executor_test.rs:255` rekey 但仅断言 AffectedRows（修复后 GREEN，无需校准）、`:683` 与 `mvcc_record_test.rs` 非键列更新、`storage_test.rs` 仅注释；**受影响用例恰为失败的 6 个，无第 7 个潜伏点**。
- **F5（确认项）**：Act Response 与实际代码、工作区一致——git status 对照 Changed Files 无漂移；Persisted Evidence none 符合模式；Self-Review 的偏差记录（无契约内偏差）与 Plan 独立审读一致。
- **F6（非阻塞 Minor，无需行动）**：000-initial 全量运行中 I041（`cli::resolve` env 竞态）偶发 1 次——已知既有登记项，复跑即绿，不计入。

**Deviation Classification**

- BH-3 → **PLAN-INVALID**：Plan 的 Iteration 001 兼容性预测「全量回归零修改」被证伪——调查将全量影响面锁定为 `update_index_maintenance_test` + `keyless_row_test`，未排查直连执行器 MVCC/GC 测试面对 Step 7 语义变化的依赖（BH-1 同类：Plan 兼容性预测错误，非 Act 问题）。
- Act 实现 → 无偏差（契约逐条执行，两条 stop condition 均未命中；阻塞交接本身符合 Gate 6 程序）。

**Acceptance Gaps**

- Iteration 001 GREEN condition「全量 0 failed」未满足：6 个既有直连执行器测试确定性失败（gc_test ×3 / version_chain_test ×2 / plan_exec_test ×1，双源多轮一致）。
- Iteration 001 Stable baseline「全量回归零修改」被证伪，需修订为「除校准 6 处外零修改」。
- R4 本体（5 场景）已由 T5/T6 覆盖且 GREEN——gap 仅在既有套件负空间，不在 R4 交付面。

**Convergence**

N/A（本 Cycle 首次 Review，无上一版 gap 比较项）

**Evidence**

- Plan 独立复跑（2026-09-13）：`cargo test --no-fail-fast --test update_index_maintenance_test --test gc_test --test plan_exec_test --test version_chain_test` → `9 passed; 0 failed` / `0 passed; 3 failed` / `3 passed; 1 failed` / `1 passed; 2 failed`——与 Act 报告逐项一致。
- 代码级：T6 diff（`src/executor/update.rs` 前置块 + Step 7）全文审读；6 失败用例寻址链行号与 panic 文案逐一核对；UpdateExecutor 测试用法 7 文件排查（F4）。
- 采信 Act 未失效结论：T5 RED 形态、锚点套件 26 GREEN——来源 Act Response Verification Evidence 表，工作区自 Act 终态零变化，独立复跑未发现矛盾。

**Follow-up Decision**

校准 6 个既有测试超出当前执行契约——T6 Invariants 明文「既有测试零修改通过（T5 新增见证除外）」，Preserve/Forbidden 无此授权；且 Acceptance 基线「全量回归零修改」本身需修订（验证契约变化）。按 iteration-planning Review 分类表，范围/验证契约变化必须使用 `replan-required`，不得伪装为当前 Cycle 修复或 rework。已按 BH-1 → 001-replan 先例创建同 Iteration 后继 Cycle（T9 校准 + 全量收尾，使用修订后全局 task、不设 repair item），并同步修订 change tasks、design D4 校准段、delta spec 校准段与 proposal 裁定记录 8-9。T5/T6 成果经独立核查全部有效，replan Cycle 直接继承，无需重做。

**Iteration Plan Update**

- Iteration 001（tasks.md 已修订）：Tasks 扩为 T5/T6/T9（T9 = BH-3 校准 + 全量收尾）；Stable baseline 修订为「全量回归除 T9 校准 6 处外零修改」；Verification/Diagnostic boundary 扩入三校准测试文件。目标、范围、依赖、R4 requirement 本体无变化；平衡审计维持（校准属同一验收成果的测试面闭合）。proposal 裁定记录 8-9、design D4 校准段、delta spec `update-index-maintenance` 校准段已同步。

**Next Cycle**

`iterations/001-rekey/001-replan.md`（已创建；Plan Context `draft`——Gate 2 前十项自评 PASS，末项「用户批准计划」待用户批准后由 Plan 置 `ready` 交 Act）

**Next Iteration**

None（Iteration 001 未完成；本 change 无后续 Iteration）
