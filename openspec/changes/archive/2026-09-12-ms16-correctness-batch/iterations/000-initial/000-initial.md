# Iteration 000 / Cycle 000: 键位等值全形态可达（路由类型门 + 键列写入类型强制）

## Plan Context

- Status: ready
- Iteration: 000-initial
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3, T4
- Depends on: None
- Stable baseline: 非 Int 键列键位等值全形态正确可达（DataScan 行内求值，restart 保持）；Int 键列路由形状与结果逐字节保持；Int 键列越界写入显式拒绝零副作用、NULL/合规写入与非 Int 键列行为保持；全量回归零修改（基线 867 只增不减）
- Verification boundary: T1-T4 全绿 + 既有锚点套件零修改 + `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` / `openspec validate` 全 0/PASS
- Diagnostic boundary: `src/parser/planner/{mod,query}.rs` + `src/pipeline.rs` 注册点 + `src/executor/{insert,update}.rs` 前置校验 + `src/storage/error.rs` + `tests/keyless_eq_routing_test.rs` + `tests/key_type_conformance_test.rs`
- Deferred tasks: T5, T6（Iteration 001，rekey 索引一致性）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R1-R5 需求基线（proposal）；design D1-D6；既有 MS15-T01/MS15-Rest 修复语义（I036 路由护栏、I037 Step 7 删旧键分支）全部保持
- Excluded scope: rekey 索引维护（T5/T6）；UPDATE/DELETE 非 Int 键列 KeyNotFound 保持；非键列类型校验；存量越界行迁移；性能优化

**Objective**

键位等值 SELECT 对全类型键列（Int / Float / String / Bool 声明类型）与全可写数据形态行集正确：非 Int 键列统一经数据页行内求值（无静默漏行），Int 键列路由可靠（越界写入被强制拒绝）；既有路由形状、I037 语义与全量测试零回归。

**Background**

- I046（MS15-T01 调查新发现）：键位等值腿字面量可键控但键列类型不容纳该键时，`extract_pk_from_where` 成功返回索引键 → IndexScan 点查空索引 → 静默漏行；AND 形态经 `has_pk_eq` → `Filter(Scan)` 同病。用户裁定方向 B（键列类型感知路由）。
- 调查新发现（2026-09-12 探针，d8a244f）：INSERT/UPDATE 不校验值类型与列类型，Float 值可落 Int 键列成为无键行——Int 键列上同类静默漏行，方向 B 修不到；用户裁定并入本 change 直接修复（根因收口 = 键列写入类型强制），不登记 improvement。
- 与 MS15-T01（I036 字面量可键控性护栏）互补：本 change 把路由可靠性判定升级到「键列声明类型」并封堵越界写入源。

**Investigation Facts**

- Current Baseline: master d8a244f（其后仅 docs 提交 e51c4a3 + 工作区 docs 改动，代码面零变化）；867 tests pass / 0 failed / 2 ignored（2026-09-12 MS15-Rest 收尾，Plan Review 独立复跑）；clippy/fmt 0、specs 25 validate PASS。本 Cycle 实施前 Act 做一次只读基线检查（`git status`/`git diff --stat` 确认覆盖范围材料未变）即可采信，不重跑全量。
- 路由现状（`src/parser/planner/query.rs`）：
  - `build_select` WHERE 段（505-609）：子查询臂 → `extract_pk_from_where` 臂（519-544，Some(key) + `is_simple_pk_equality` → `IndexScan`）→ 非 PK 臂（545-608：`has_pk_eq && !has_non_keyable_pk_literal_leg` → `Filter(Scan)` 560-568；`contains_or` → `Filter(DataScan)` 569-587；否则谓词下推 `DataScan(Some(predicate))` 588-607）。
  - `extract_pk_from_where`（929-970）只匹配顶层 `pk = Value` / `Value = pk`，返回 `value.to_key()`（仅 Int → Some，value.rs:82-90）；`has_pk_equality`（845-869）AND 递归结构判定；`has_non_keyable_pk_literal_leg`（882-924，MS15-T01）按字面量 `to_key().is_none()` 分类。
  - `Value::equals`（`src/executor/value.rs:118-137`）：Int↔Float 双向隐式转换 true；Int vs String/Bool false；`(Null, Null) → true`、含 Null 其余 false。
- PlanBuilder（`src/parser/planner/mod.rs:98-129`）：字段 `tables` / `primary_keys` / `inner_table_names` / `building_subquery`；`register_table(name, columns, pk)` lowercase 键。调用点：pipeline.rs:1003（真表，有 `table_meta.columns: Vec<(String, ColumnType)>` 与 `pk_column`）、query.rs:150（派生表别名，pk=""，primary_keys 条目使键位等值判定天然不匹配）、单测 7 处。
- `ColumnType`（`src/storage/page_format/tuple.rs:24-33`）：Int / String(u16) / Float / Bool。
- 写入路径现状：
  - `InsertExecutor::next`（`src/executor/insert.rs:85-112`）：`row_values[self.pk_index]` → `to_key()` → `if let Some(key)` 内 `index_manager.search` 命中即 `Err(StorageError::DuplicateKey)`（102-111，先查后写）；无类型校验。
  - `UpdateExecutor::next`（`src/executor/update.rs:62-143`）：Step 1 `index_manager.search(&self.key)` 未命中 `KeyNotFound`（70-73）→ Step 2-6 读旧 tuple / 改列 / 写新版本 / WAL / record_version → Step 7（129-140）：`column_name == pk_column && new_value.to_key().is_none()` → `index_manager.delete(&self.key)`（I037），否则 `index_manager.update(&self.key, new_row_id)`（rekey 缺陷点，Iteration 001 处理）。
  - `build_update`（`src/parser/planner/ddl_dml.rs:359-411`）：强制单列 SET + PK 等值 WHERE + 字面量/NULL 新值——UPDATE 触达行必经索引（键位等值），非 Int 键列自然 `KeyNotFound`（响亮，保持）。
- 索引层：`BTree::insert` DuplicateKey 检查已注释禁用（`src/storage/btree/node.rs:127`），唯一性由执行器先查后写承担；`IndexManager::search/insert/delete/update`（index_manager.rs:106-322）spawn_blocking + root 同步；`delete` 内部先 search 清 `row_to_key` 旧映射（245-266）。
- 错误映射：`Response::Error` → CLI `sql_failure_status` → exit 3（`src/cli/mod.rs:405-427` 通用路径）；`StorageError` 为 thiserror 枚举（`src/storage/error.rs:8+`），加性变体有先例（NotADatabase 等）。
- import/restore：`csv_value`（`src/cli/lifecycle.rs:502-524`）按列声明类型转换（Int 列 `parse::<i64>`，"5.0" 本就报 `invalid INT value`）——强制不影响 import；dump/restore 往返由既有套件锁定。
- 测试夹具先例：`tests/keyless_eq_routing_test.rs`（exec_ok/query_rows/plan_of + `wal_buffer.shutdown()`；restart 用例 `buffer_pool.flush_all()` → shutdown → drop → reopen）与 `tests/update_index_maintenance_test.rs`（AffectedRows 模式匹配 + COUNT 断言）。

**Implementation Guidance**

建议顺序：T1（RED 见证）→ T2（路由实现，T1 转 GREEN）→ T3（RED 见证）→ T4（强制实现，T3 转 GREEN + Iteration 收尾）。T2 与 T4 无相互依赖，T3 可与 T2 并行推进；RED 用例须在对应实现前观察失败。

- T2 形态建议：helper `fn pk_type_known_non_int(&self, table_name: &str) -> bool`（`matches!(self.primary_key_types.get(table_name), Some(ColumnType::Int))` 取反，未注册 → false）；extract 臂以 `let extracted = if known_non_int { None } else { self.extract_pk_from_where(...)? };` 前置分流；`Filter(Scan)` 臂条件扩为 `has_pk_eq && !known_non_int && !has_non_keyable_pk_literal_leg(..)?`。等价局部控制流由 Act 定。
- T4 形态建议：`StorageError::KeyTypeMismatch { column, expected, actual }`（Display 如 `key column 'id' expects INT, got Float`）；insert.rs 校验置于 to_key/DuplicateKey 预检之前；update.rs 校验块置于 Step 1 之后、Step 2 之前（与 Iteration 001 碰撞预检同区，本 Cycle 只放类型校验）。判定 helper 可两执行器各自内联（Int 键列 + 值非 Int/Null → Err）。
- 断言风格沿用两测试文件既有先例（模式匹配 Response 变体 + 中文断言消息）；plan 形状断言参照 `simple_non_keyable_equality_plan_is_data_scan_with_predicate`。

**Behavioral Change**

- 路由（SELECT）：Float/String/Bool 声明类型键列的键位等值（简单/AND/反向）由 `IndexScan` / `Filter(Scan)` 改为 `DataScan(Some(predicate))`（含 OR 保持 `Filter(DataScan)`）；行集按 equals 语义由错误空集变为正确行集（Float 键列 + Int 字面量）或保持空集（String/Bool 键列 + Int 字面量，路径变化结果不变）。Int 键列全部形态逐字节不变。
- 写入（INSERT/UPDATE）：Int 键列收到 Float/String/Bool 键位值由静默接受改为 `StorageError::KeyTypeMismatch`（exit 3，任何写入前，零副作用）；NULL 与合规类型行为不变；非 Int 键列不新增拒绝。
- 错误面：新增错误变体与文案（additive）；既有错误（DuplicateKey/KeyNotFound 等）语义与优先级不变（KeyNotFound 仍在类型校验之前）。

**Task Contracts**

### T1: 键列类型感知路由 RED 测试见证

- Requirement/Scenario: R1 键列类型感知路由——Float 简单/AND/反向、String 结果不变、restart 五场景
- Depends on: None
- Targets: `tests/keyless_eq_routing_test.rs`（文件尾追加 I046 段）
- Current behavior: Float 键列 + Int 字面量简单等值经 IndexScan 空集、AND 形态经 Filter(Scan) 空集；`WHERE s = 5`（String 键列）经 IndexScan 空集
- Required behavior: 测试断言目标行为——行集可达（Float 形态）、plan 为谓词下推 DataScan、String 形态空集、restart 保持；实现前这些断言失败（RED）
- Required changes: 仅新增测试函数（含独立文档注释注明 I046 依据与 RED 预测）；不改既有测试
- Preserve: 既有 8 用例逐字节不变；夹具 helper（exec_ok/query_rows/plan_of）复用不修改
- Forbidden: 不改产品代码；不改既有断言；不新增依赖
- Test witness: `cargo test --test keyless_eq_routing_test`——新增用例 RED（Float 简单/AND 达行断言失败、plan 形状断言失败），既有 8 用例 GREEN
- GREEN condition: T2 完成后本任务全部用例转 GREEN
- Verification: `cargo test --test keyless_eq_routing_test` 输出与退出码记录于 Act Response
- Stop when: RED 形态与调查预测不符（如某形态现已可达）——实质基线发现，返回 Plan

### T2: 路由实现——键列类型传递与两处判定门

- Requirement/Scenario: R1 全场景；R2（Int 键列路由保持的负空间约束）
- Depends on: T1（RED 已观察）
- Targets: `src/parser/planner/mod.rs`（PlanBuilder 字段 + 注册方法）、`src/pipeline.rs`（register_table 接线）、`src/parser/planner/query.rs`（WHERE 路由两处门）
- Current behavior: 路由可靠性只按字面量可键控性判定（query.rs:519-568），键列声明类型不可知
- Required behavior: 键列声明类型非 Int 时 extract 臂跳过（视同 None）、`Filter(Scan)` 臂不放行，两形态均落入既有 OR/谓词下推数据页臂；Int 键列与未注册类型行为逐字节不变；pipeline 注册点为真表传递键列 `ColumnType`（从 `table_meta.columns` 按键列名取，pk_column 为空不设置）
- Required changes: PlanBuilder 加性 `primary_key_types: HashMap<String, ColumnType>`（page_format::ColumnType）+ lowercase 键注册方法；query.rs 两处判定门（design D1 语义，等价控制流由 Act 定）
- Preserve: `register_table` 签名不变；`extract_pk_from_where` / `has_pk_equality` / `has_non_keyable_pk_literal_leg` / `is_simple_pk_equality` 函数本体行为不变；`is_simple_pk_equality` 在 Int 键列上的 IndexScan 形状（R2 既有 pushdown 两用例锚点）；子查询/派生表路由不变
- Forbidden: 不改执行器与存储层；不改 plan cache 键；不新增第三条 plan 路径形态（只复用既有 OR/下推臂）
- Test witness: T1 用例转 GREEN；`cargo test --test pushdown_test --test keyless_eq_routing_test` 既有用例零修改通过
- GREEN condition: T1 全部用例 GREEN + 既有锚点 GREEN
- Verification: 两测试套件命令输出 + `cargo test`（受影响面）记录于 Act Response
- Stop when: 判定门无法在不改函数本体的前提下闭合（实质设计偏差），或 plan cache 出现跨语句污染迹象

### T3: Int 键列写入类型强制 RED 测试见证

- Requirement/Scenario: R3 Int 键列写入类型强制——INSERT/UPDATE 拒绝、零副作用、NULL/显式列序/非 Int 键列锚点
- Depends on: None
- Targets: `tests/key_type_conformance_test.rs`（新建）
- Current behavior: `INSERT (5.0, 1)` 入 Int 键列成功落库（2026-09-12 探针实证）；`UPDATE SET id = 5.0` 成功走 I037 分支
- Required behavior: 测试断言目标行为——越界 INSERT/UPDATE 报错（Response::Error，文案含键列名与 INT 期望）且随后 COUNT/点查证明零副作用；NULL、显式列序 `(v, id)`、Float 键列收 Int 值等锚点断言既有/目标行为；实现前拒绝断言 RED
- Required changes: 仅新增测试文件与用例
- Preserve: 不改既有测试文件
- Forbidden: 不改产品代码
- Test witness: `cargo test --test key_type_conformance_test`——拒绝类用例 RED（当前被接受），锚点类用例 GREEN
- GREEN condition: T4 完成后全部转 GREEN
- Verification: 命令输出与退出码记录于 Act Response
- Stop when: 锚点类用例出现非预期 RED（既有行为与调查不符）——实质基线发现，返回 Plan

### T4: 强制实现——执行器前置校验与错误变体

- Requirement/Scenario: R3 全场景；R5（I037 分支、KeyNotFound 优先级、非 Int 键列不拒绝的负空间约束）
- Depends on: T3（RED 已观察）；T2 无依赖（可并行）
- Targets: `src/storage/error.rs`（加性变体）、`src/executor/insert.rs`（键位类型预检）、`src/executor/update.rs`（写入前类型校验）
- Current behavior: 两执行器均无类型校验（Investigation Facts「写入路径现状」）
- Required behavior: 键列声明类型 Int 且键位值非 Int/Null 时，在任何写入（数据页/WAL/索引）之前 `Err(StorageError::KeyTypeMismatch)`；INSERT 校验先于 DuplicateKey 预检；UPDATE 校验位于 Step 1 之后（KeyNotFound 优先保持）；NULL 与非 Int 键列（Float/String/Bool 声明）不拒绝；显式列序经既有 pk_index 解析天然覆盖
- Required changes: error.rs 加性变体（thiserror，Display 点名键列/期望 INT/实际类型）；两执行器判定逻辑（Int 键列 + 非 Int/Null 值 → Err，等价局部形态由 Act 定）
- Preserve: I037 Step 7 分支行为逐字节不变（`SET id = NULL` 仍删旧键条目）；KeyNotFound / DuplicateKey / 非键列更新 / 同键更新语义不变；WAL 与版本链写入时序不变（校验在最前）
- Forbidden: 不改 Step 7 分支结构（rekey 归 Iteration 001 T6）；不加非键列校验；不改 restore/import 流程
- Test witness: T3 全部用例转 GREEN；`cargo test --test keyless_row_test --test update_index_maintenance_test` 既有用例零修改通过
- GREEN condition: T3 全绿 + I037/keyless 锚点全绿 + Iteration 000 全量收尾（`cargo test` 全量、clippy --all-targets -- -D warnings、fmt --check、openspec validate）
- Verification: 全量命令输出（决定性片段 ≤20 行/项）+ 退出码记录于 Act Response
- Stop when: 既有测试依赖越界键位写入而失败（实质基线发现），或校验点无法先于 WAL/版本写入安置

**Invariants**

- Int 键列全部既有路由形状与结果逐字节不变（R2）；`Filter(DataScan)` OR 臂与谓词下推臂既有语义不变。
- I037（SET 键列为 NULL → 删旧键条目）逐字节不变；keyless 落库不入索引语义不变。
- 错误优先级：`KeyNotFound` 先于类型校验（UPDATE 目标行不存在仍报 KeyNotFound）；`DuplicateKey` 语义不变。
- 既有测试套件零修改通过（新增见证除外）；基线 867 只增不减。
- plan cache 键与失效语义不变；`register_table` 签名不变。

**Non-goals**

- rekey 索引维护（Iteration 001 T5/T6）；UPDATE/DELETE 非 Int 键列 KeyNotFound 保持；非键列与非 Int 键列类型强制；存量越界行迁移；restore 历史越界 dump 的兼容处理；性能优化。

**Acceptance**

Requirements Traceability Matrix（Iteration 000 范围 R1/R2/R3/R5；R4 属 Iteration 001，此处仅列占位映射）：

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 键列类型感知路由 | Float 简单/AND/反向、String 结果不变、restart | D1/D2 | T1, T2 | 000 | `query.rs` build_select WHERE 路由两门 + `mod.rs` primary_key_types + `pipeline.rs` 接线 | `keyless_eq_routing_test` 新增 5-6 用例 | None | Covered |
| R2 可键控字面量路由保持（收窄 Int 键列） | IndexScan / Filter(Scan) 两锚点 | D1 | T2 | 000 | 同上（未注册/Int 类型不分流） | 既有 `pushdown_test` 两用例 + `keyless_eq_routing_test` 既有 8 用例零修改 | None | Covered |
| R3 Int 键列写入类型强制 | INSERT/UPDATE 拒绝×2、NULL×2、显式列序、非 Int 键列锚、import/restore 见证 | D3 | T3, T4 | 000 | `insert.rs` 预检 + `update.rs` 前置校验 + `error.rs::KeyTypeMismatch` | `key_type_conformance_test` 全部用例 + 既有 keyless_row/update_index_maintenance 锚点 | None | Covered |
| R5 既有语义零回归 | 全量零修改 + I037 分支保持 | D5/D6 | T2, T4 | 000 | 全部变更面 | `cargo test` 全量 + clippy/fmt/validate | None | Covered |
| R4 键位 rekey 后索引条目一致 | （Iteration 001） | D4 | T5, T6 | 001 | `update.rs` Step 7 | `update_index_maintenance_test` 扩展 | None | Deferred（Iteration 001） |

**Verification**

- 每任务：目标测试套件命令 + 决定性输出（≤20 行/项）+ 退出码；RED 先于实现观察（T1/T3）。
- Iteration 收尾（T4）：`cargo test`（全量，预期 ≥867+新增，0 failed, 2 ignored）、`cargo clippy --all-targets -- -D warnings`（0）、`cargo fmt --check`（0 diff）、`openspec validate`（PASS，含本 change 3 个 delta specs）。
- 验证直接观察行集/错误变体/plan 形状/退出码，不引入身份型证据机制。

**Gate 2 Readiness**

Gate 2 于 2026-09-12 经用户批准通过，各检查项：

- 无 Missing requirement：PASS——RTM R1/R2/R3/R5 Covered，R4 显式 Deferred→Iteration 001（Cycle Acceptance 表 + tasks.md Iteration Plan）。
- 无未批准 Simplified：PASS——RTM Simplification 列全 None。
- 调查完整：PASS——Investigation Facts 覆盖入口/调用者/数据流/错误路径/测试入口/索引层/import-restore 面（含文件行号）；基线 867（d8a244f 后代码面零变化）+ Act 只读基线检查指令。
- 设计闭合：PASS——design D1-D6，无 TBD；D2 记录备选否决理由；D5 记录用户决策与兼容性。
- 任务可执行：PASS——T1-T4 契约含 Targets/Current/Required/Preserve/Forbidden/RED 见证/GREEN 条件/停止条件。
- 分轮合理：PASS——2 Iteration + 平衡审计（tasks.md）。
- 追踪完整：PASS——R→S→D→T→代码面→测试见证链路闭环（Acceptance RTM）。
- 验证充分：PASS——RED→GREEN + 全量回归 + clippy/fmt/validate；直接观察行集/错误变体/plan 形状/退出码。
- 无身份型证据工程：PASS——验证直接观察目标行为；Persisted Evidence none（无不可低成本重跑面）。
- 无实质未知项：PASS——非实质选择（等价控制流、Display 措辞）留给 Act 且不涉契约语义。
- tasks/specs/design/Cycle 一致：PASS——编号与映射一致；`openspec validate --changes` PASS + 单 change `--strict` valid（2026-09-12）。
- 用户批准：PASS——2026-09-12 用户批准计划执行（「批准，交 Act 执行」）。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果（命令、决定性输出、退出码）；无不可低成本重跑的验收面。

- Budget: 不适用（none）。

**Risks and Notes**

- 既有测试若隐式依赖越界键位写入，RED 阶段即暴露 → 实质基线发现返回 Plan（预计不存在）。
- 类型未知（未注册）降级路径只有派生别名（pk=""）可达，键位等值判定天然不匹配——惰性已确认（query.rs:150）。
- `equals(Int, Float)` 的 f64 转换精度语义与既有非键谓词求值一致，不新增语义面。
- 工作区含用户待提交 docs 改动（SNAPSHOT/tasks/improvements）——Act 只读基线检查时注意区分，不触碰。

## Act Response

- Status: blocked

**Implemented**

- T1（完成）：`tests/keyless_eq_routing_test.rs` 追加 I046 段 6 用例（Float 键列 Int 字面量简单/AND/反向/plan 形状/String 锚点/restart），RED 按 Plan Context 预测观察（5 RED / 9 GREEN：Float 简单/AND/反向/plan 形状/restart 失败于空集或 IndexScan；String 锚点 + 既有 8 用例 GREEN）。
- T2（完成）：路由类型门落地，T1 全部转 GREEN（14 passed）；`pushdown_test` 16 passed（Int 键列 IndexScan/Filter(Scan) 形状锚点零修改）。
- T3（部分完成，7/8 场景）：新建 `tests/key_type_conformance_test.rs` 7 用例（拒绝矩阵 INSERT Float/String/Bool + UPDATE Float/String + 零副作用 + NULL insert / UPDATE NULL I037 / Float 键列收 Int 值锚点），RED 按预测观察（4 拒绝用例 RED 当前被接受 / 3 锚点 GREEN）。**R3「显式列序」场景未写入套件**——Plan 调查事实与实测行为冲突，见 Blocker Handoff BH-2。
- T4（完成）：`StorageError::KeyTypeMismatch` + InsertExecutor 键位类型预检（先于 DuplicateKey 预检）+ UpdateExecutor 写入前类型校验（Step 1 之后、KeyNotFound 优先保持、先于任何写入），T3 全部转 GREEN（7 passed）+ keyless_row 4 + update_index_maintenance 5 锚点全绿。
- Iteration 000 全量收尾（Gate 5）：**BLOCKED**——全量回归 879 passed / **1 failed** / 2 ignored，唯一失败为既有测试 `expression_e2e_test::negative_number_literal_persists`，见 Blocker Handoff BH-1。

**Changed Files and Symbols**

- `src/parser/planner/mod.rs`：`PlanBuilder.primary_key_types: HashMap<String, ColumnType>`（加性字段）+ `PlanBuilder::set_pk_column_type`（lowercase 键，`register_table` 签名不变）。
- `src/pipeline.rs`：`register_table`（自由函数）——真表按键列名从 `table_meta.columns` 传递 `ColumnType`，键列不在列清单不注册。
- `src/parser/planner/query.rs`：`PlanBuilder::pk_type_known_non_int`（判定 helper）+ `PlanBuilder::extract_pk_from_where_gated`（D1 门 1，`extract_pk_from_where` 本体零改动）+ `build_select` WHERE 非 PK 臂条件扩展（D1 门 2）。
- `src/storage/error.rs`：`StorageError::KeyTypeMismatch { column, expected, actual }`（加性变体，Display 形态 `key column 'id' expects INT, got Float`）。
- `src/executor/insert.rs`：`key_value_type_name`（私有 helper）+ `InsertExecutor::next` 键位类型预检（`schema[pk_index]` 判 Int 声明，先于 `to_key()`/DuplicateKey 预检）。
- `src/executor/update.rs`：`key_value_type_name`（私有 helper）+ `UpdateExecutor::next` Step 1 后键列写入类型校验（`column_name == pk_column` + 声明 Int 判定，先于 Step 2）。
- `tests/keyless_eq_routing_test.rs`：+6 用例（既有 8 用例逐字节不变）。
- `tests/key_type_conformance_test.rs`：新建 7 用例。

**Deviations from Plan**

1.（非实质）T2 extract 臂门：Plan Implementation Guidance 建议 `let extracted = if known_non_int { None } else { ... }` 前置分流；Act 采用等价包裹方法 `extract_pk_from_where_gated`（惰性求值位置逐字节不变、diff 最小、镜像文件内既有 helper 风格）。契约授权「等价控制流由 Act 定」。
2.（实质，见 BH-2）T3 显式列序场景未按契约写入：Plan 事实「显式列序经既有 pk_index 解析天然覆盖」与实测行为不符，契约 stop condition 命中。
3.（非实质）`key_value_type_name` 在 insert.rs/update.rs 各自内联（契约明示「两执行器各自内联」授权；共享位置在 T4 Targets 之外）。

**Blocker Handoff**

发现位置：T3 测试设计（BH-2）/ T4 全量收尾 Gate 5（BH-1）；均命中 Gate 6（实际代码与契约实质冲突，Act 不可在契约内修复）。

**BH-1：既有测试依赖「Int 键列收 Float 值」写入，与 D3 强制不变量冲突（T4 stop condition：既有测试依赖越界键位写入）**

- Plan 预期：design Risks「预计不存在：既有套件数据均合规」；Acceptance 要求「既有套件零修改通过、基线 867 只增不减」。
- 实际：`tests/expression_e2e_test.rs::negative_number_literal_persists`（MS11-T01 R5/S1，I040 负数字面量）建表 `CREATE TABLE t (v INT, s STRING)`（无显式主键 → 首列 v 为隐式键列、声明 Int），断言 `INSERT INTO t VALUES (-1.5, 'y')` 成功落库。T4 后被拒绝：`Error { message: "Execution error: key column 'v' expects INT, got Float" }`（tests/expression_e2e_test.rs:516-518 断言处）。全量 `cargo test --no-fail-fast`：879 passed / 1 failed / 2 ignored，**唯一失败即此用例**（其余含 13 个新用例与全部锚点全绿）。
- 影响：R5「既有语义零回归」（零修改）与 R3 强制不变量在「Int 隐式键列 + Float 负数字面量」形态上直接冲突——两个已批准约束无法同时成立。裁定属 Plan/用户：校准该测试（Float 字面量移入非键列或 Float 键列表；涉 MS11-T01 已验收测试修改，需批准）或收窄 D3（如仅显式 PRIMARY KEY 强制——但 MS15-T01 全部夹具为隐式键列表，收窄将使 I046 主形态不设防，Act 判断不可取，仅供 Plan 参考）。Act 未改动该测试、未收窄实现、未回滚。
- 恢复条件：用户/Plan 对 BH-1 裁定（测试校准方案或设计修订）后，Act 在当前 Cycle 消费裁定（实现侧不动或按新契约调整），校准后重跑全量收尾。

**BH-2：INSERT 显式列清单从未被应用（值按表列序位置解释）——Plan 调查事实错误 + 潜在缺陷（T3 stop condition：锚点类与调查不符）**

- Plan 预期：Investigation Facts / T4 Required behavior「显式列序经既有 pk_index 解析天然覆盖」——`(v, id) VALUES (1, 5.0)` 中 5.0 落到键列 id，T4 类型预检将拒绝。
- 实际（代码核对 + 二进制探针，探针时点 T2 已落地、T4 未落地——列清单映射面二者均未触碰）：`build_insert` 将列清单装入 `InsertNode.columns` 但**无任何消费点**；`create_executor_from_plan` 将 `node.values` 直传执行器，`row_values[pk_index]` 按表列序取值。探针：`INSERT INTO p (v, id) VALUES (1, 5.0)` → 落库 `[1, 5.0]`（id=1, v=5.0，列清单被静默忽略，5.0 落在非键列 v）；对照 `INSERT INTO p2 VALUES (5.0, 1)` → 落库 `[5.0, 1]`。
- 影响：① R3「显式列序」场景按契约无法见证 GREEN（该形态键位收到的是合规 `Int(1)`，T4 不拒绝），场景未写入套件（写入将永久 RED，毒化共享测试面）；② 新发现潜在用户可见缺陷——列清单与表列序不一致时值被静默错列存储（既有测试列清单均按表列序书写，缺陷潜伏），属 INSERT 列映射域，超出本 change 批准范围。
- 恢复条件：Plan 裁定场景与缺陷处置（本 change 内修复列清单映射（范围扩展，需 replan）/ 登记新 improvement 后改写场景 / 将场景移出 R3）后，Act 按修订契约补齐见证。

任务状态：已完成 T1、T2；部分完成 T3（7/8 场景 + RED 见证）；T4 实现完成、目标套件 GREEN，收尾阻塞于 BH-1；未开始无（T5/T6 本就 Deferred 至 Iteration 001）。已通过 Gate：Gate 3（四任务测试见证齐备）、Gate 4（逐任务两段 Review + 完整 diff Review）、Gate 5 部分（目标套件 + clippy 0 + fmt clean + validate PASS；全量回归项 BLOCKED）。修改文件：上列 8 个；工作区另有用户既有 docs 待提交改动（SNAPSHOT/tasks/improvements，未触碰）。

**Blocker Resolution**

（未恢复时写 None）

**Self-Review**

- Plan compliance: T1/T2/T4 逐条符合契约（Targets/Preserve/Forbidden 全部满足）；T3 按 stop condition 处置显式列序偏差；未扩大变更面、未回滚既有工作、未改既有测试。
- Full diff reviewed: 是——7 tracked 文件 diff + 新建测试文件全文复核；无计划外修改；既有函数本体（`extract_pk_from_where`/`has_pk_equality`/`has_non_keyable_pk_literal_leg`/`is_simple_pk_equality`/`register_table`/I037 Step 7 分支）零改动。
- Critical findings unresolved: 无（两处实质发现均属 Plan 裁定面，已按 Gate 6 阻塞交接，非 Act 契约内可修复项）。
- Important findings unresolved: 无。
- Minor findings unresolved: 2——① `key_value_type_name` 双文件内联（契约授权；共享需触达 T4 Targets 外文件）；② T2 包裹方法与 Plan 建议形态的等价替换（Deviation 1）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T1 RED 见证 | `cargo test --test keyless_eq_routing_test` | `FAILED. 9 passed; 5 failed`（Float 简单/AND/反向/plan/restart，退出码非零） | I046 五场景修复前 RED + 既有 8 + String 锚点 GREEN | PASS（预期 RED） |
| T2 GREEN | `cargo test --test keyless_eq_routing_test --test pushdown_test` | `ok. 14 passed; 0 failed` + `ok. 16 passed; 0 failed`（含 `simple_pk_equality_still_index_scan`） | I046 全场景 + Int 键列路由形状锚点 | PASS |
| T3 RED 见证 | `cargo test --test key_type_conformance_test` | `FAILED. 3 passed; 4 failed`（拒绝用例「应被拒绝为 Error，实际 AffectedRows { count: 1 }」） | D3 拒绝矩阵修复前 RED + 3 锚点 GREEN | PASS（预期 RED） |
| T4 GREEN + 锚点 | `cargo test --test key_type_conformance_test --test keyless_row_test --test update_index_maintenance_test` | `7 passed` + `4 passed` + `5 passed`，全 0 failed | D3 强制 + keyless/I037 语义保持 | PASS |
| 全量回归 | `cargo test --no-fail-fast` | `passed=879 failed=1 ignored=2`；唯一失败 `negative_number_literal_persists`（BH-1） | 全仓测试面 | **FAIL（→ BH-1 阻塞）** |
| 静态检查 | `cargo clippy --all-targets -- -D warnings`；`cargo fmt --check` | clippy 0 warning；fmt 修正 update.rs 一处超长行后 0 diff（FMT_CLEAN） | 全 workspace | PASS |
| OpenSpec | `openspec validate --changes`；`openspec validate --specs` | `1 passed, 0 failed`；`25 passed, 0 failed` | change + 语料库 | PASS |
| 基线采信 | `git status` / `git diff --stat`（实施前只读检查） | 仅用户 docs 改动 + change 目录，代码面与 d8a244f 一致 | Plan Context Current Baseline | 采信（867 基线，Plan Review 2026-09-12 独立复跑） |

**Persisted Evidence**

（None required——Blocker 探针命令与决定性输出已全文入本 Handoff，可低成本复现；Plan Context Persisted Evidence: none）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook / Incident | None | None | None |

**Remaining Issues**

- BH-1、BH-2（见 Blocker Handoff）——均为 Plan/用户裁定面；Iteration 000 的 Gate 5 全量回归项与 R3 显式列序场景在其裁定前无法收口。

**Commit or Diff Reference**

未 commit（工作区 7 tracked 文件修改 + `tests/key_type_conformance_test.rs` 新建；对照基线 d8a244f，未触 docs 与既有测试）。

## Plan Review

- Review Result: replan-required

**Findings**

独立核查（不依赖 Act Self-Review）——全文 diff 审读、失败测试源码核对、BH-2 代码级核实与二进制探针、全量/clippy/fmt/validate 独立复跑：

- **F1（实质，阻塞，BH-1 确认）**：`tests/expression_e2e_test.rs::negative_number_literal_persists`（MS11-T01 R5/S1，I040）建表 `CREATE TABLE t (v INT, s STRING)`（首列 v 为隐式键列、声明 Int）并断言 `INSERT INTO t VALUES (-1.5, 'y')` 落库（:518 panic 断言）——与 R3 强制直接冲突。Plan Review 独立复跑全量：880 用例 = 879 passed + 该 1 failed（确定性）+ I041 flaky 假失败 1 次（`cli::resolve::test_db_dir_env_cases`，复跑即绿，与本次 diff 无关、既有登记项）。Act 的 879/1 failed 报告与独立复跑一致（Act 那轮未撞 I041 flake）。
- **F2（实质，阻塞 + 范围裁定，BH-2 确认且严重度更高）**：`InsertNode.columns`（plan.rs:137-144）全链路无消费点——`InsertExecutor` 构造只收 `values` + table_meta（insert.rs:57-92），值按表列序位置解释。Plan Review 独立探针三形态：① 乱序清单静默错位（`INSERT INTO p (v, id) VALUES (1, 5.0)` → 落库 `[1, 5.0]`，id=1、v=5.0）；② **部分清单 `INSERT INTO p (v) VALUES (9)` 触发 `tuple.rs:38` 断言 panic、exit 101**（Act 未报此形态，Plan 独立探针追加证实）；③ 未知列静默接受（`INSERT INTO p (id, zz) VALUES (7, 1)` → affected 1）。属 INSERT 列映射域既有潜伏缺陷（非本次引入），Plan 调查事实「显式列序经既有 pk_index 解析天然覆盖」错误。
- **F3（非阻塞 Minor，接受）**：T2 以 `extract_pk_from_where_gated` 包裹方法替代 Plan 建议的前置分流——契约授权「等价控制流由 Act 定」，diff 审读确认惰性求值位置与语义逐字节等价、`extract_pk_from_where` 本体零改动。
- **F4（非阻塞 Minor，接受）**：`key_value_type_name` 双执行器各自内联——契约明示授权，共享 helper 需触达 T4 Targets 外文件。
- **F5（独立确认项）**：实现与契约一致性——两处路由门（门 1 包裹方法返回 None 分流 + 门 2 条件扩展）语义正确；`register_table` 签名未动；I037 Step 7 分支未动；INSERT 校验先于 DuplicateKey 预检；UPDATE 校验在 Step 1 之后（KeyNotFound 优先保持）且先于任何写入；`KeyTypeMismatch` 加性、Display 点名键列/INT 期望/实际类型；既有 8+16 锚点用例零修改；clippy 0（仅 cargo config 弃用环境提示）、fmt clean、validate changes PASS / specs 25 PASS。

**Deviation Classification**

- BH-1 → **PLAN-INVALID**（Plan 的兼容性预测「预计不存在：既有套件数据均合规」被证伪；「既有套件零修改」验收约束与已批准的 R3 强制在该测试形态上不可同时成立）。
- BH-2 → **PLAN-INVALID**（Plan 调查事实「显式列序经既有 pk_index 解析天然覆盖」错误——列清单根本未被消费）。
- Deviation 1 / 3 → ACT-DEVIATION（契约授权内，非实质）。

**Acceptance Gaps**

- R5「既有测试套件零修改通过」：`negative_number_literal_persists` 确定性失败（F1）——验收约束需修订为「除按 BH-1 裁定校准的该测试外零修改」。
- R3「显式列序」场景：按原契约无法见证（该形态键位收到合规值，T4 不拒绝）——需 T7 列清单映射修复后方可见证（F2）。

**Convergence**

N/A（首次 Review）

**Evidence**

- 全量独立复跑：880 用例（867 基线 + 13 新增）= 879 passed / 1 failed（BH-1）/ 2 ignored；`--lib` 单独复跑 `cli::resolve` 3 passed（I041 flaky 复跑即绿）。
- BH-2 探针（target/debug/rtsql 重建后）：三形态输出与退出码（错位 `[1, 5.0]` exit 0 / panic `tuple.rs:38` exit 101 / 未知列 affected 1 exit 0）。
- 静态：`cargo fmt --check` clean；`cargo clippy --all-targets -- -D warnings` exit 0；`openspec validate --changes` PASS / `--specs` 25 PASS（含既有 I045 Purpose WARNING 噪声）。
- 采信 Act 未失效结论：T1/T3 RED 见证（9+4 failed 形态与预测一致）、目标套件 GREEN 计数——来源 Act Response Verification Evidence 表，diff 审读与独立复跑未发现矛盾。

**Follow-up Decision**

两个 Acceptance gap 均需验收边界或范围变化，普通返工不可覆盖 → **replan-required**：更新 change 计划产物（proposal/design/tasks/delta specs）并在同 Iteration 创建 `001-replan` Cycle。裁定内容（用户未即时应答，按 Plan Review 推荐方案执行、replan 计划待用户批准）：

1. BH-1 → 校准 `negative_number_literal_persists`（负 Int 行保持原表原断言；负 Float 行移入 Float 键列表 `tf`，I040 覆盖完整保留）——校准已记入 `key-column-type-conformance` delta spec（T8-R2 先例）。收窄 D3 的备选被否（隐式键列是 I046 主形态，收窄后不设防）。
2. BH-2 → 并入本 change 修复（新 task T7：`build_insert` plan 期列清单校验 + 重排，新 delta spec `insert-column-list-mapping`；三探针形态全收口，panic 消除；R3 显式列序场景修复后可见证）。
3. T8：执行校准 + 补写 R3 显式列序场景 + Iteration 000 全量收尾。

**Iteration Plan Update**

- Iteration 000 Tasks 扩为 T1-T4 + T7/T8（T7/T8 契约见 `001-replan`）；Stable baseline / Verification boundary / Diagnostic boundary 相应扩展（tasks.md 已同步）；新增 delta spec `insert-column-list-mapping`（R6）。Iteration Map 编号不变。

**Next Cycle**

`iterations/000-initial/001-replan.md`（replan，Plan Context draft——Gate 2 待用户批准）

**Next Iteration**

None（Iteration 000 未 accepted；Iteration 001 保持 Deferred）
