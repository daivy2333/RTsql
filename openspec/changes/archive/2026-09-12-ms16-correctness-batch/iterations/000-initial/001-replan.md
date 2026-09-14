# Iteration 000 / Cycle 001: replan——INSERT 列清单映射修复 + 既有测试校准 + Iteration 000 收尾

## Plan Context

- Status: ready
- Iteration: 000-initial
- Cycle: 001-replan
- Cycle Type: replan
- Parent cycle: 000-initial.md

**Iteration Scope**

- Change tasks: T7, T8（Iteration 000 修订后剩余任务；T1-T4 已于 000-initial 执行，成果继承为本 Cycle 基线）
- Depends on: None（000-initial 部分成果已在工作区，见 Investigation Facts）
- Stable baseline: 与 tasks.md Iteration 000 修订后 Stable baseline 一致——含 INSERT 列清单映射语义与 T8 校准例外
- Verification boundary: T7/T8 全绿 + 既有锚点套件零修改（T8 校准的 `negative_number_literal_persists` 除外）+ 全量 0 failed + clippy/fmt 0 + `openspec validate` PASS
- Diagnostic boundary: `src/parser/planner/ddl_dml.rs`（build_insert）+ `tests/insert_column_list_test.rs`（新建）+ `tests/expression_e2e_test.rs`（校准一处）+ `tests/key_type_conformance_test.rs`（补一场景）
- Deferred tasks: T5, T6（Iteration 001，rekey 索引一致性——Map 不变）

**Cycle Scope**

- Trigger: replan-required
- Acceptance gaps: ① R5「既有套件零修改」被 `negative_number_literal_persists` 击穿（BH-1）；② R3「显式列序」场景按原契约无法见证（BH-2 列清单无消费点）
- Repair items: None（replan 使用修订后全局 task T7/T8，不设 repair item）
- Inherited scope: 000-initial 已交付成果全部继承——路由类型门（T1/T2）、键位类型强制（T3/T4 的 7 用例与实现）、全部 Invariants；proposal 裁定记录 5-7；design D3 校准方案与 D7 列清单映射
- Excluded scope: partial INSERT（NULL 填充）支持；非键列类型校验；rekey（→001 Iteration）；I041 flaky 消除（独立小 change 建议）；性能优化

**Objective**

关闭 Iteration 000 两个 Acceptance gap：INSERT 列清单映射语义正确（乱序落位、非法清单计划期拒绝、panic 消除）且键位校验作用于映射后键位值（R3 显式列序场景可见证）；既有套件除一处按裁定校准外零修改、全量 0 failed。

**Background**

000-initial Cycle 实施后 Plan Review 独立审计裁定 replan-required（见 000-initial.md Plan Review）：BH-1（既有测试与 R3 强制冲突，PLAN-INVALID——Plan 兼容性预测证伪）与 BH-2（INSERT 列清单全链路无消费点，PLAN-INVALID——Plan 调查事实错误，Plan 独立探针比 Act 报告多证实 panic 与未知列两形态）。用户裁定记录见 proposal（BH-1 校准 / BH-2 并入修复，按推荐方案执行、replan 待批准）。

**Investigation Facts**

- Current Baseline: 工作区含 000-initial 已实施改动（7 tracked 文件 + 2 测试文件），未 commit；T1/T2 成果（路由门）与 T4 实现（`KeyTypeMismatch` + 两执行器前置校验）已在工作区并 GREEN（目标套件 14+7+4+5 passed）；全量 880 用例 = 879 passed + 1 failed（BH-1 确定性）+ I041 flaky（偶发，复跑即绿）；clippy 0 / fmt clean / validate PASS（Plan Review 2026-09-12 独立复跑）。Act 在本 Cycle 开工前做只读基线检查（`git status`/`git diff --stat` 对照 000-initial Act Response Changed Files 清单）即可采信，不重跑全量。
- `build_insert`（`src/parser/planner/ddl_dml.rs:70-88`）：取表名 → `validate_table` → 列清单 `columns: Vec<String>`（lowercase）→ `extract_insert_values` 得 `Vec<Vec<Value>>` → 构造 `InsertNode { table_name, columns, values }`。**列清单无任何下游消费**（plan.rs:137-144 字段定义；`InsertExecutor::with_table_manager` 只收 `values` + table_meta，insert.rs:57-92）。
- 表列名来源：`PlanBuilder.tables: HashMap<String, Vec<String>>`（mod.rs:100-101）——pipeline `register_table`（pipeline.rs:998-1002）按 `table_meta.columns` 顺序装入列名，**表列序即注册序**；`build_insert` 是 `PlanBuilder` 方法，可直接读 `self.tables`。列清单大小写：build_insert 已 lowercase。
- BH-2 探针实证（Plan Review，2026-09-12，target/debug/rtsql）：① `(v, id) VALUES (1, 5.0)` → `[1, 5.0]`（错位）；② `(v) VALUES (9)` → `tuple.rs:38` assert panic（`compute_tuple_size` assert_eq values.len vs schema.len）、exit 101；③ `(id, zz) VALUES (7, 1)` → affected 1（未知列忽略）。
- BH-1 冲突点：`tests/expression_e2e_test.rs:509-540`——`CREATE TABLE t (v INT, s STRING)`（隐式键列 v、声明 Int）+ `INSERT INTO t VALUES (-1.5, 'y')` 断言 affected 1（:518）+ 重开断言含 `(-1.5, y)` 行（:534-539）。T4 后该 INSERT 被 `KeyTypeMismatch` 拒绝。
- 键位校验作用点：`InsertExecutor::next` 以 `row_values[self.pk_index]`（表列序位置）取键位值——**重排发生在 plan 期装入 `InsertNode.values` 时**，执行器天然收到映射后的值，无需改动（T7 执行器零改动）。
- 测试夹具先例：`key_type_conformance_test.rs`（expect_error/expect_affected/assert_count helper）；plan 形状断言如需用 `plan_of`（keyless_eq_routing_test.rs:41-47）。

**Implementation Guidance**

建议顺序：T7 RED（`insert_column_list_test.rs`）→ T7 实现（build_insert）→ GREEN → T8 校准 + 场景补写 → 全量收尾。

- T7 形态建议：build_insert 在 `extract_insert_values` 之后构造 InsertNode 之前：取 `self.tables.get(&table_name)` 列名（未知表已被 `validate_table` 拒绝）；`columns` 非空时校验「每项 ∈ 表列 ∧ 无重复 ∧ len == 表列数」，构造映射 `table_pos → requested_pos` 并对每行 values 重排；`columns` 为空时校验每行 `len == 表列数`。拒绝用 `PlanError::ParseError`（或既有等价变体）携带点名文案（未知列名/重复列名/数量 expected vs actual）——变体选择属非实质，文案须可定位。
- T8 校准形态：`negative_number_literal_persists` 中新增 `CREATE TABLE tf (f FLOAT, s STRING)` + `flush_all` + `INSERT INTO tf VALUES (-1.5, 'y')`（affected 1 断言），原 `t` 表断言（`(-1, 'x')` 行与查询）逐字节保留；重开段改为两表各断言行集（`t` 保留原断言、`tf` 断言 `[(-1.5, "y")]`）；文件头与测试 doc-comment 注明 MS16 T8 校准依据（key-column-type-conformance spec 校准段）。R3 补写场景加在 `key_type_conformance_test.rs`（R3-S8：显式列序 `(v, id) VALUES (1, 5.0)` → KeyTypeMismatch + COUNT 0）。

**Behavioral Change**

- INSERT 计划期新增拒绝面（R6）：未知列 / 重复列 / 列清单数量不符 / 无清单行长度不符——此前分别为静默错位 / 静默错位 / panic / panic。
- INSERT 乱序清单值落位修正：按清单映射重排（此前按表列序错位）；键位唯一性预检与键位类型校验因此作用于用户赋给键列的值。
- 测试面：一处既有测试校准（BH-1 裁定，spec 记录）+ 一处场景补写 + 一个新测试文件。产品代码仅 `build_insert` 一处变更。

**Task Contracts**

### T7: INSERT 列清单映射修复（R6）

- Requirement/Scenario: R6 全部 6 场景（insert-column-list-mapping）
- Depends on: None（T1-T4 成果已在工作区）
- Targets: `src/parser/planner/ddl_dml.rs::build_insert`（含新增私有校验/重排 helper，形态由 Act 定）；`tests/insert_column_list_test.rs`（新建）
- Current behavior: 列清单装入 `InsertNode.columns` 后无消费点——乱序清单值按表列序错位落库、部分清单 panic（exit 101）、未知列静默接受、无清单数量不符同源 panic
- Required behavior: 列清单恰为表列排列时值按清单映射重排落位（键位校验作用于重排后键位值）；未知列/重复列/清单数量不符/无清单行长度不符 → 计划期明确错误拒绝（exit 3，零副作用，无 panic）；清单与表列序一致行为逐字节保持
- Required changes: build_insert 校验 + 重排（执行器与存储层零改动）；新测试文件 6 场景见证
- Preserve: 无清单且数量相符的既有 INSERT 行为逐字节不变（含值类型宽松语义——非键列不校验）；`extract_insert_values` 本体行为不变（I040 负数折叠等）；`InsertNode.columns` 字段保留；restore/import 链路行为不变（二者经 execute_sql 走同一 build_insert——import 按表列序产出列清单？不：import 产出无清单 INSERT 文本，restore 重放 dump 文本含 `create_table_sql` 生成的 INSERT 均为无清单形态——数量校验对二者恒满足）
- Forbidden: 不改 `InsertExecutor`/存储层；不加 partial INSERT（NULL 填充）支持；不改 plan cache 键语义
- Test witness: RED 先行——乱序落位用例（修复前错位断言失败）、部分清单用例（修复前 panic）、未知列用例（修复前 accepted）、无清单数量不符（修复前 panic）为 RED；一致清单锚点 GREEN。实现后全绿
- GREEN condition: 6 场景全 GREEN + 既有 `key_type_conformance_test`（7 用例）与全量锚点零回归
- Verification: `cargo test --test insert_column_list_test` 输出 + 退出码记录于 Act Response
- Stop when: 重排需触碰执行器构造面（实质设计偏差）；或 `self.tables` 列名序与 `table_meta.columns` 序不符（注册面事实与调查矛盾）

### T8: 既有测试校准 + R3 显式列序场景 + Iteration 000 收尾（R3/R5）

- Requirement/Scenario: R3 显式列序场景（key-column-type-conformance）；R5 修订后零修改约束（校准例外）；design D3 校准方案
- Depends on: T7（显式列序场景依赖重排后键位校验）
- Targets: `tests/expression_e2e_test.rs::negative_number_literal_persists`（校准）+ `tests/key_type_conformance_test.rs`（补 1 场景）+ Iteration 000 全量收尾
- Current behavior: `negative_number_literal_persists` 确定性失败（BH-1）；R3 显式列序场景缺失（000-initial T3 因 BH-2 未写入）
- Required behavior: 校准后该测试 GREEN——负 Int 行原断言逐字节保留；负 Float 行移入 `tf(f FLOAT, s STRING)`（Float 键列不受 R3 强制，落库为无键行），重开两表各断言行集，I040 负 Int/负 Float 折叠覆盖完整保留；R3-S8 显式列序用例 GREEN（`(v, id) VALUES (1, 5.0)` → `KeyTypeMismatch` 含键列与 INT 文案 + COUNT 0）；全量 0 failed
- Required changes: 仅测试文件两处（校准 + 补场景）；doc-comment 注明校准依据（spec 校准段）
- Preserve: 既有 `expression_e2e_test` 其余 23 用例零修改；`key_type_conformance_test` 既有 7 用例零修改；`t` 表既有断言逐字节保留
- Forbidden: 不改产品代码（T7 已闭合映射面）；不改校准断言以外的既有断言；不改 spec 文件（Plan 已更新）
- Test witness: 校准前该测试 RED（BH-1 确定性失败，000-initial 已观察）；补写场景在 T7 后直接 GREEN（写 RED 无意义——键位校验已存在，场景见证的是「映射后校验」组合语义）
- GREEN condition: 校准测试 GREEN + R3-S8 GREEN + 全量 `cargo test` 0 failed（预期 ≥893 passed：880 + T7 新 6 + T8 补 1，I041 flaky 撞到时复跑确认）+ clippy/fmt/validate 全 0/PASS
- Verification: 全量与静态检查命令输出（决定性片段 ≤20 行/项）+ 退出码记录于 Act Response；I041 flaky 处置：失败用例与 resolve 相关时单独复跑该用例确认 flaky 属性并在 Response 注明，不计入本 change 失败
- Stop when: 校准需改动 `t` 表既有断言才能成立（与裁定方案不符——返回 Plan）；或全量出现 BH-1/I041 之外的新失败（实质基线发现）

**Invariants**

- 000-initial 全部 Invariants 继续有效（Int 键列路由形状、I037 分支、错误优先级、plan cache、`register_table` 签名）。
- 无清单且数量相符的 INSERT（含 restore/import 产出形态）行为逐字节不变。
- 执行器与存储层在本 Cycle 零改动（T7 仅 planner、T8 仅测试）。
- 既有测试除 T8 校准一处外零修改；基线 867 只增不减。

**Non-goals**

- partial INSERT（未列列 NULL 填充）支持；`InsertNode.columns` 字段移除；rekey（→001 Iteration）；I041 修复；非键列类型校验；性能优化。

**Acceptance**

Requirements Traceability Matrix（本 Cycle 范围 R6 + R3/R5 gap 闭合；R1/R2 已于 000-initial 覆盖，R4 Deferred）：

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R6 INSERT 列清单映射与校验 | 乱序落位/乱序键位拒绝/部分清单/未知列/无清单数量/一致锚点 | D7 | T7 | 000 | `ddl_dml.rs::build_insert` | `insert_column_list_test` 6 用例 | None | Covered |
| R3 Int 键列写入类型强制（显式列序场景闭合） | R3-S8 显式列序 | D3/D7 | T7, T8 | 000 | 同上（映射后键位值进执行器校验） | `key_type_conformance_test` R3-S8 | None | Covered |
| R5 既有语义零回归（修订：校准例外） | 全量零修改除 T8 校准 | D3 校准段/D5 | T8 | 000 | `expression_e2e_test::negative_number_literal_persists` | 校准后 GREEN + 全量 0 failed | 校准例外（BH-1 裁定，spec 校准段记录） | Covered |
| R4 键位 rekey 后索引条目一致 | （Iteration 001） | D4 | T5, T6 | 001 | `update.rs` Step 7 | `update_index_maintenance_test` 扩展 | None | Deferred（Iteration 001） |

**Verification**

- T7：目标套件 + RED 先行观察；T8：校准测试 + R3-S8 + 全量（预期 ≥893 passed / 0 failed / 2 ignored）+ `cargo clippy --all-targets -- -D warnings`（0）+ `cargo fmt --check`（clean）+ `openspec validate`（PASS，4 delta specs）。
- 验证直接观察行集/错误文案/退出码/panic 消失，不引入身份型证据机制。

**Gate 2 Readiness**

- 无 Missing requirement：PASS——R6 新增后 RTM Covered（本 Cycle Acceptance 表）；R3/R5 gap 映射到 T7/T8。
- 无未批准 Simplified：PASS——R5 校准例外为 BH-1 裁定（proposal 记录 5）+ spec 校准段，非需求裁剪。
- 调查完整：PASS——build_insert 结构、列名注册序、BH-2 探针、BH-1 冲突点、键位校验作用点均有代码级证据（Investigation Facts）。
- 设计闭合：PASS——design D7（含选择理由与被否备选）、D3 校准段；无 TBD。
- 任务可执行：PASS——T7/T8 契约含 Targets/Current/Required/Preserve/Forbidden/RED/GREEN/停止条件。
- 分轮合理：PASS——T7/T8 同属 Iteration 000 修订后 Acceptance（tasks.md 平衡审计已更新）。
- 追踪完整：PASS——RTM 链路闭环。
- 验证充分：PASS——RED→GREEN + 全量 + 静态检查；无身份型证据工程；Persisted Evidence none。
- 无实质未知项：PASS——非实质选择（错误变体选择、helper 拆分）留 Act。
- tasks/specs/design/Cycle 一致：PASS——T7/T8 已入 tasks.md；4 个 delta specs；design D7；validate 待 Act 收尾复跑（本 Cycle 创建后 Plan 已跑 changes/specs PASS）。
- 用户批准：**PASS**——2026-09-12 用户批准 replan 计划并授权执行（原话：「更改gate状态，开始实施」），含 MS11-T01 已验收测试校准授权（proposal 裁定记录 5，BH-1 推荐方案随批准生效）。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果；无不可低成本重跑的验收面。

- Budget: 不适用（none）。

**Risks and Notes**

- I041 flaky 可能撞全量收尾（约 1/6）——T8 契约已写明处置（单独复跑确认 + Response 注明）。
- import/restore 产出均为无清单 INSERT 文本（dump `create_table_sql`/`sql_literal` 路径、import 逐条 auto-commit）——数量校验恒满足，零影响；Act 全量回归即覆盖。
- `self.tables` 列名序与表列序的一致性由 pipeline 注册顺序保证（pipeline.rs:998-1002 map 保序）；若 T7 实施发现不一致属实质发现，停止条件已列。

## Act Response

- Status: reported

**Implemented**

- T7（完成）：`build_insert` 在 `extract_insert_values` 之后、构造 `InsertNode` 之前调用新增私有方法 `map_insert_values`——列清单非空时校验「恰为表列排列」（逐项小写定位到注册列序位置，未知列 `PlanError::ColumnNotFound` 点名、重复列 `ParseError` 点名、清单长度 ≠ 表列数 `ParseError` 点名 expected/got），随后每行按 `table_pos` 映射重排（`ordered[target] = row[requested]`，排列保证全覆盖）；列清单为空时保持既有表列序逐位置语义，仅新增行长度校验。数量不符文案收敛为共享 helper `insert_count_error`（`INSERT INTO '<t>' expects N values, got M`）。执行器与存储层零改动，`InsertNode.columns` 字段保留、`extract_insert_values` 本体零改动。R3-S8 语义（映射后键位校验）经 `key_type_conformance_test` 新用例证实。
- T8（完成）：BH-1 校准——`negative_number_literal_persists` 负 Float 行移入 `CREATE TABLE tf (f FLOAT, s STRING)`（`flush_all` 后 INSERT affected 1），负 Int 行 `t` 表建表/INSERT/WHERE 查询/重开断言逐字节保留，重开段改两表各断言行集（`t` → `[(-1,"x")]`、`tf` → `[(-1.5,"y")]`），文件头与测试 doc-comment 注明校准依据；R3-S8 补写——`explicit_column_list_key_violation_rejected`（`(v, id) VALUES (1, 5.0)` → `KeyTypeMismatch` 点名键列 + COUNT 0）。Iteration 000 全量收尾：全量回归 0 failed、clippy/fmt/validate 全 0/PASS。

**Changed Files and Symbols**

- `src/parser/planner/ddl_dml.rs`：自由函数 `insert_count_error`（新增）+ `PlanBuilder::map_insert_values`（新增私有方法）+ `build_insert` 接线一行。
- `tests/insert_column_list_test.rs`：新建 7 用例（R6 六场景 + 重复列 SHALL 覆盖），夹具 helper 复用 key_type_conformance 先例。
- `tests/expression_e2e_test.rs`：文件头校准注记 + `negative_number_literal_persists` 校准（唯一改动测试，其余 23 用例零修改）。
- `tests/key_type_conformance_test.rs`：追加 R3-S8 `explicit_column_list_key_violation_rejected`（既有 7 用例零修改）。
- `iterations/000-initial/001-replan.md`：本 Response（Plan Context 状态变更与 Gate 2 批准记录见该文件上部）。

**Deviations from Plan**

1.（非实质）T7 测试文件为 7 用例而非契约 Test witness 所写「6 场景」：追加 `duplicate_column_in_list_rejected_at_plan_time`——R6 Requirement SHALL 明列重复列拒绝、T7 契约 Required behavior 亦明列「重复列 → 计划期明确错误拒绝」，编号场景未覆盖该 SHALL 项，补需求面见证（覆盖增强，非范围扩大）。
2.（非实质）数量错误文案三处 format 收敛为共享 helper `insert_count_error`（Gate 4 code review 发现的重复实现，修复后复跑套件与 fmt）。
3.（非实质）Plan GREEN condition 数字预测「≥893 passed」算术偏高：000-initial 全量计数 880 + T7 新 7 + T8 补 1 = 888 为实际全量计数（879 passed + BH-1 校准转绿 1 + 新增 8），binding 条件「0 failed」满足。

**Blocker Handoff**

None

**Blocker Resolution**

None（000-initial 的 BH-1/BH-2 由用户批准的 replan 裁定闭合，处置记录见其 Plan Review Follow-up Decision 与 proposal 裁定记录 5-7）

**Self-Review**

- Plan compliance: T7 逐条符合契约——Targets 仅 `ddl_dml.rs::build_insert`（+私有 helper，契约授权「形态由 Act 定」）与新测试文件；Preserve 全部满足（无清单数量相符 INSERT 逐字节保持、`extract_insert_values` 本体不变、`InsertNode.columns` 保留、restore/import 无清单形态零影响——数量校验恒满足且全量回归证实）；Forbidden 全部未触（执行器/存储层零改动、无 partial INSERT、plan cache 键语义不变）；停止条件未命中（重排未触执行器构造面、`self.tables` 注册序与表列序一致——R6-S6 锚点与 S1 映射均按注册序正确落位）。T8 逐条符合契约——校准后 `t` 表既有断言逐字节保留（stop condition 未命中）、R3-S8 GREEN、全量 0 failed（I041 flaky 未触发，无需复跑）。
- Full diff reviewed: 是——`git diff` 全文审读 ddl_dml.rs 与 expression_e2e_test.rs（本 Cycle 仅上述两 tracked 文件变更）+ 两个测试文件全文复核；无计划外修改；000-initial 已实施面（planner mod/query、pipeline、insert/update、error、keyless_eq_routing_test）本 Cycle 零触碰；用户 docs 待提交改动（SNAPSHOT/tasks/improvements）未触碰。
- Critical findings unresolved: 无。
- Important findings unresolved: 无。
- Minor findings unresolved: 1——`openspec validate --changes` 输出 `⚠ [WARNING] overview: Purpose section is still a placeholder`：指向 change overview（proposal.md，Plan 产物）缺 `## Purpose` 段（两个新增 delta spec 的 Purpose 均为实文、specs 目录无 TBD/TODO）；WARNING 不影响 validate 计数（1 passed / 0 failed），按 T8 契约「不改 spec 文件」未处理，留 Plan Review 裁定（修复 proposal 或确认为既有噪声）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T7 RED 见证 | `cargo test --test insert_column_list_test` | `FAILED. 1 passed; 6 failed`——S1 错位 `[[1,2]]`≠`[[2,1]]`、S2/S4/S7 `AffectedRows { count: 1 }` 未拒绝、S3/S5 panic 于 `src/storage/page_format/tuple.rs:38`（`left: 1, right: 2`）；S6 一致锚点 GREEN | R6 全部形态修复前 RED + 一致清单锚点 | PASS（预期 RED，形态与 BH-2 探针一致） |
| T7 GREEN + 锚点 | `cargo test --test insert_column_list_test --test key_type_conformance_test` | `ok. 7 passed; 0 failed` + `ok. 7 passed; 0 failed` | R6 七用例 + R3 键位强制锚点零回归 | PASS |
| T8 校准 + R3-S8 | `cargo test --test expression_e2e_test --test key_type_conformance_test` | `ok. 24 passed; 0 failed`（含校准用例与 23 既有用例零修改）+ `ok. 8 passed; 0 failed`（含 R3-S8） | BH-1 校准转绿 + I040 覆盖保持 + R3-S8 映射后键位校验 | PASS |
| 全量回归（fmt 修正后） | `cargo test --no-fail-fast` | `TOTAL passed=888 failed=0 ignored=2`（聚合全部套件） | 全仓测试面 | PASS |
| 静态检查 | `cargo clippy --all-targets -- -D warnings`；`cargo fmt --check` | clippy exit 0（仅 cargo config 弃用环境提示，非代码 warning）；fmt clean（修正 map_insert_values 两处 + 测试三处换行后 FMT_CLEAN） | 全 workspace | PASS |
| OpenSpec | `openspec validate --changes`；`openspec validate --specs` | `1 passed, 0 failed`（1 WARNING 见 Minor）；`25 passed, 0 failed` | change + 语料库 | PASS |

**Persisted Evidence**

（None required——Plan Context Persisted Evidence: none；全部验证可低成本重跑，决定性输出已入本 Response）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook / Incident | None | None | None |

**Remaining Issues**

- Minor：`openspec validate --changes` 的 overview Purpose WARNING（proposal.md，Plan 产物）——见 Self-Review；不阻塞 Acceptance。
- Iteration 001（T5/T6 rekey）未展开——按 Map 待 Iteration 000 accepted 后由 Plan 展开。

**Commit or Diff Reference**

未 commit（工作区含 000-initial 与 001-replan 两轮改动叠加 + 用户既有 docs 待提交改动，未触碰；对照基线 d8a244f）。

## Plan Review

- Review Result: accepted

**Findings**

独立核查（不依赖 Act Self-Review）——T7 diff 全文审读、三测试文件核对、二进制探针（退出码与落位）、全量/clippy/fmt/validate 独立复跑：

- **F1（确认项）**：`map_insert_values` 语义与 design D7/R6 逐条一致——排列校验（未知列 `ColumnNotFound` 点名 → 重复列 `ParseError` 点名 → 清单长度校验）→ 每行按 `table_pos` 映射重排（排列校验保证全覆盖，`ordered[target] = row[requested]`）→ 空清单仅行长校验；接线在 `extract_insert_values` 之后、`InsertNode` 构造之前；执行器与存储层零改动；`InsertNode.columns` 保留；`extract_insert_values` 本体未动。Plan 独立探针：五种拒绝形态全部 exit 3 且文案可定位（数量 `expects 2 values, got 1`、未知列 `Column not found: zz`、重复列 `Duplicate column 'id'`、键位 `key column 'id' expects INT, got Float`）、乱序清单落位 `[2, 1]` 正确——panic 形态（此前 exit 101）全部消除。
- **F2（确认项）**：T8 校准与 BH-1 裁定逐条一致——`t` 表建表/INSERT/WHERE 查询/重开断言逐字节保留；`tf` 表遵循 DDL-flush 夹具先例（`flush_all` 后 INSERT）；重开段两表各断言行集；文件头与 doc-comment 注明校准依据（key-column-type-conformance spec 校准段）。R3-S8 按契约补写（点名键列 + INT 文案 + COUNT 0）。
- **F3（确认项）**：全量独立复跑 **888 passed / 0 failed / 2 ignored**（880 基线 + T7 新 7 + T8 补 1，与 Act 报告一致；无 FAILED 行）。
- **F4（确认项）**：`cargo clippy --all-targets -- -D warnings` 0（仅 cargo config 弃用环境提示）、`cargo fmt --check` clean、`openspec validate --changes` PASS（无 WARNING）/ `--specs` 25 PASS。
- **F5（Act Minor finding 处置：误归因，无需行动）**：Act 报告的「overview Purpose WARNING 指向 proposal.md」经复核不成立——该 WARNING 来自 `openspec validate --specs`，指向主 specs（cli-noninteractive-shell、database-file-format-header 等）的 archive 占位 Purpose，即既有登记项 **I045** 的持续噪声；change 自身 `--changes` validate 干净。proposal.md 无需修改。
- **F6（非实质偏差，接受）**：① 7 用例（6 编号场景 + 重复列 SHALL 见证）——R6 Requirement SHALL 明列重复列拒绝而编号场景遗漏，Act 补需求面见证属覆盖增强；② `insert_count_error` 共享 helper——等价收敛；③ Plan GREEN 数字预测「≥893」算术错误（正确值 888），binding 条件「0 failed」满足。

**Deviation Classification**

- 偏差 1 / 2 → ACT-DEVIATION（契约授权内非实质）。
- 偏差 3 → PLAN-INVALID（Plan 数字预测算术错误，非实质、不阻塞）。
- F5 → 既有 I045 噪声（非本 change 产物，无行动）。

**Acceptance Gaps**

None——000-initial Plan Review 的两个 gap 均闭合：① R5 校准例外落地（`negative_number_literal_persists` GREEN，I040 覆盖保留，其余既有测试零修改）；② R3 显式列序场景可见证（R3-S8 GREEN，映射后键位校验生效）。R6 六场景 + 重复列 SHALL 全部 Covered。

**Convergence**

reduced → closed（对比 000-initial Plan Review：R5 校准 gap 与 R3 显式列序 gap 均已消除，无新 gap）

**Evidence**

- 全量独立复跑：888 passed / 0 failed / 2 ignored（后台 `cargo test --no-fail-fast` 全量聚合，无 FAILED）。
- 二进制探针（target/debug/rtsql 重建）：五拒绝形态退出码与文案（均 exit 3）+ 乱序落位 `SELECT id, v` → `[[2,1]]`。
- 静态：clippy 0 / fmt FMT_CLEAN / validate changes PASS + specs 25 PASS。
- 采信 Act 未失效结论：T7 RED 见证（1 passed / 6 failed，形态与 BH-2 探针一致）、目标套件计数——来源 Act Response Verification Evidence 表，diff 审读与独立复跑未发现矛盾。

**Follow-up Decision**

既有 Acceptance 已满足且无阻塞项 → `accepted`：Iteration 000 完成（T1-T8 全部 done）。按 Map 展开下一 Iteration `001-rekey`（T5/T6，Plan Context ready）。

**Iteration Plan Update**

None（Map 不变）

**Next Cycle**

None（001-replan accepted，Iteration 000 完成）

**Next Iteration**

`iterations/001-rekey/000-initial.md`（已创建，Plan Context ready）
