# Iteration 002 / Cycle 000: UPDATE 值表达式、恢复错误面与文档收尾（写面能力 + 遗留并入）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T8、T9、T10、T11、T12
- Acceptance gaps: None
- Repair items: None（T12 为显式 change task，非 rework repair item——Iteration 001 Review accepted 并轮修复，见该 Review Follow-up Decision）
- Inherited scope: R3 全部 scenario（proposal S12–S17）；R6 全部 scenario（S22–S23）；R2 补强场景（S24）；design D5/D6/D7/D8；集中决策 4（UPDATE 值表达式全表面）与 5（展开本 Iteration + 遗留 Issue 并入，原话见 proposal）
- Excluded scope: UPDATE 子查询、聚合右值、DEFAULT 关键字进入 UPDATE SET；恢复重放跳过/丢弃不可重放记录与库修复指引；聚合进入 WHERE/ON、嵌套聚合解锁

**Objective**

`UPDATE ... SET` 接受完整值表达式右值与多列赋值（同旧行快照），门序/索引维护/回滚语义全部保持；恢复重放遇 catalog 缺失表点名指因而打开失败语义不变；聚合显示名重名时全部引用解析到正确注册输出；SKILL.md/README 双语与实现一致。change 全部需求（R1–R6）闭环，可进入收尾归档。

**Investigation Facts**

- Current Baseline: HEAD 9a103c0 + 未提交实施面（Iteration 000/001 已 accepted，Review 采信其改动与验证结论）；全量 **1833 tests / 0 failures / 2 ignored**（Iteration 001 Act Response M19 全量，Plan Review accepted 采信）；fmt/clippy 干净。本 Iteration 新涉及表面：`executor/plan.rs` / `executor/update.rs` / `wal/recovery.rs`（Iteration 000/001 未触碰，调查锚点有效）；`planner/{query,aggregate}.rs`（Iteration 001 刚落地，锚点按当前工作区核实）。
- Current-State Evidence:
  - **T8/T9 计划面与执行器**（design D5 锚点经 2026-10-03 复核）：`UpdateNode { table_name, key, column: String, new_value: Value }`（`src/executor/plan.rs:276`，单列单值）；`build_update`（`src/parser/planner/ddl_dml.rs:1838`）单赋值强制（`:1864` `MULTI-COLUMN UPDATE` 拒绝）、右值仅字面量四形态（Value / TypedString 日期解析 / NULL Identifier / 其余 `UnsupportedValue`）；执行器构造臂 `pipeline.rs:1472`；渲染臂 `plan_format.rs:183`。`executor/update.rs` 写入门链齐备：`coerce_datetime_write`（`:166`）→ 写入类型门 FLOAT 升格（`:254` `ColumnTypeMismatch`）→ NOT NULL（`:114`）→ 键列类型门（`:130`）→ 碰撞预检（`:150`/`:231` `DuplicateKey`）→ Step 7 rekey 三分支（`:299`）——当前全按单 `new_value` 驱动。T4 已交付 `contains_aggregate_expr`（`aggregate.rs`，pub(crate)）可复用于聚合右值拒绝。`UpsertExecutor` 的 `ConflictAction::DoUpdate(assignments)`（`plan_format.rs:149/:175` 可见形态）是独立通道，本 Iteration 不触碰。
  - **T11 恢复错误面**（2026-10-03 调查，design D7）：`redo_record`（`src/wal/recovery.rs`）Insert/Update/Delete 三臂 `:722`/`:820`/`:947` 的 `table_name` 来自 WAL 记录，`get_table` 失败一律 `WalError::RedoFailed(format!("table '{}' lookup failed during redo: {}", …))`；缺失表底层错误 `StorageError::TableNotFound`（`src/storage/error.rs:90`）；`RedoFailed` display 前缀 `WAL redo failed: {}`（`src/wal/record.rs:518`）。恢复期其余 `get_table` 消费点——`build_pk_version_maps`（`:349`）、索引重建（`:1040`）、未提交中止标记（`:1416`）、tail 持久化（`:670`）——全部遍历 catalog 行，表按构造存在，不涉及本场景。场景构造（Iteration 000 Act 已实测触发）：lib open → CREATE TABLE（catalog 脏页，DDL 不写 WAL）→ INSERT 提交（WAL 追加）→ 句柄 drop 不 `close()` → 重开恢复 `get_table` 未命中。
  - **T12 聚合重名解析**（2026-10-03 探针实测，design D8）：wrap Aggregate 臂（`query.rs` 条件包装段）以 `aggregate_display_names.iter().position(eq_ignore_ascii_case)` 首匹配定位；`compile_aggregate_row_expr` 聚合 Function 引用臂（`aggregate.rs`）以 `output_columns.iter().position` 首匹配。显示名互异时正确；重名时歧义——四形态探针错误值：重复别名第三列取 `MAX(a)` 值（20/30 应为 200/300）、别名撞 `result_column_name` 同型、`HAVING MAX(b) > 150` 按 `MAX(a)` 过滤得空组（应两组保留）、嵌入 `MAX(b)+1` 得 21/31（应为 201/301）。HEAD 位置序机制（`agg_seen` 计数）对四形态正确。`SELECT MAX(id), MAX(id)+1` 共享输出列语义必须保持。顶层聚合角色按 SELECT 序注册（分类循环逐项 push）；T4/ORDER BY 项经 `register_aggregate_calls` 在其分类点插入。
  - **测试面**：`tests/aggregate_extension_test.rs` 现 29 用例（Iteration 001 后）；`tests/planner_test.rs` 90；`tests/update_index_maintenance_test.rs` 9；upsert/write_type_conformance/subset_insert 既有矩阵；恢复套件 recovery_tail_persistence（4）/ recovery_replay_convergence（3）/ wal_recovery_large（7）。`tests/update_expression_test.rs` 与 `tests/recovery_error_surface_test.rs` 均不存在（新建）。
- Code and Critical Path: T8 改 `plan.rs::UpdateNode`（字段演进）+ `ddl_dml.rs::build_update`（赋值路由）+ 三消费臂穷尽补齐（enum 变更由编译器强制穷尽）；T9 改 `executor/update.rs`（快照求值 + 门序逐列 + 预检前置）；T11 改 `recovery.rs` 三臂失败分支（变体判别 + 定向文案）；T12 改 `query.rs` wrap 臂与 `aggregate.rs` 引用臂的解析机制（可引入共享解析 helper）；T10 纯文档。T9 依赖 T8；T10 依赖全部；T11/T12 与 T8/T9 无代码级依赖。

**Implementation Guidance**

- 建议执行序：T12（先修复 Iteration 001 遗留回归，恢复干净聚合基线）→ T8 → T9（计划面 → 执行器链）→ T11（独立小任务）→ T10（能力全部落地后一次写准）。仅 T9→T8 与 T10→其余为硬依赖，其余并行可行。
- T8 右值路由形态：`Expr::Value`/TypedString → Literal（既有解析语义逐字节）；`Expr::Identifier` → 表列校验（未知列 `ColumnNotFound`，索引按 `TableMeta.columns` 序）+ Expression；`Expr::Subquery`/`InSubquery`/`Exists` → 显式点名拒绝（沿用既有 UPDATE 无子查询边界文案通道）；`contains_aggregate_expr` 命中 → 点名拒绝；其余（BinaryOp/Cast/Case/Coalesce/Function/Trim/Ceil/Floor/Nested/UnaryOp）→ `build_expression` 既有内核编译为 Expression。`UpdateValue` enum 的 Literal 臂保留 TypedString 日期解析与 NULL 语义。
- T9 快照语义：逐命中行先读全行 `values[]`，全部 Expression 右值基于同一快照求值（`SET a = a + 1, b = a` 中 `b` 取旧 `a`）；随后逐赋值列走既有门序；预检（PK 碰撞 + 变更唯一列碰撞）前置到任何写入前；语句失败经既有隐式事务回滚全部已写行。未赋值列零触碰。
- T11 文案要素契约：表名、catalog 缺失陈述、无 checkpoint 关闭可能原因（如句柄未 `close()` 即 drop、未持久化 DDL 随脏页丢失）、打开拒绝明示。精确措辞留 Act（非实质），四要素缺一不可（测试断言按要素不按全文）。
- T12 两可行机制见 design D8（结构匹配 / 出现序号）；无论何种，直出路径与重名可观察表头必须逐字节不变。
- T10 文档：SKILL.md Aggregate/Write 行改写、README 双语能力清单、非 INTEGER PK 拒绝清单、旧 dump 兼容注记；恢复错误面无用户可见行为变化，仅在错误信息示例如有引用处同步。

**Task Contracts**

### T8: UPDATE 计划面——多列赋值与值表达式右值

- Requirement/Scenario: R3（S12–S14、S17 的计划面；S15/S16 的计划期部分）
- Depends on: None
- Targets: `src/executor/plan.rs::UpdateNode`（`assignments: Vec<UpdateAssignment>` 演进）、`src/parser/planner/ddl_dml.rs::build_update`、`src/executor/plan_format.rs` Update 渲染臂、`correlated.rs` 与 `get_plan_output_columns` 消费臂穷尽补齐
- Current behavior: `SET` 单赋值（多列 `MULTI-COLUMN UPDATE` 拒绝）、右值仅字面量（表达式 `Unsupported value type`）
- Required behavior: 多列赋值可达；右值路由字面量/列引用/表达式三类（子查询与聚合点名拒绝、错误携能力名）；字面量臂语义逐字节保留（含 TypedString 日期解析、NULL）；`plan_format` 渲染新节点形状
- Preserve: `ON CONFLICT DO UPDATE SET` 三形态与 `REPLACE INTO` 独立通道零变化；WHERE 主键等值要求、视图拒绝、附加库路由不动；既有拒绝文案通道不重写
- Forbidden: 不改执行器（T9 范围）；不解锁子查询/聚合右值；不动 UpsertExecutor
- Test witness: `tests/planner_test.rs` 赋值矩阵 RED（多列、列引用、算术/CAST/CASE/标量函数、子查询与聚合拒绝、字面量回归）
- GREEN condition: 矩阵 GREEN + 既有 UPDATE 相关 planner 用例零回归
- Verification: `cargo test --test planner_test -- --test-threads=1`（0 failed）
- Stop when: 计划面无法在不改执行器的前提下表达多列赋值（节点演进前提失效），或右值路由无法区分字面量与表达式形态——返回 Plan

### T9: UPDATE 执行器多列化——同快照求值与门序保持

- Requirement/Scenario: R3（S12–S16 e2e、S17 回归面）
- Depends on: T8
- Targets: `src/executor/update.rs`（逐命中行求值 + 逐列门序 + 预检前置 + 索引维护）
- Current behavior: 执行链按单 `new_value` 驱动（coerce → 类型门 FLOAT 升格 → NOT NULL → 键列类型门 → 碰撞预检 → 版本链写入 + Step 7 rekey）
- Required behavior: 逐命中行旧行快照求值全部表达式右值（S14）；门序 per 列保持（含 coerce、FLOAT 升格、NOT NULL 零副作用、键列 `KeyTypeMismatch`）；PK 与变更唯一列碰撞预检前置（任何写入前，零副作用）；rekey 三分支/唯一条目/次级索引维护照常；语句失败隐式事务回滚全部已写行
- Preserve: 既有门序与错误面优先级、文本不变；`update_index_maintenance_test` 9 用例与 MS16/MS23/MS24 语义零回归；单列字面量 UPDATE 行为逐字节不变
- Forbidden: 不改 InsertExecutor/UpsertExecutor；不改索引原语；不改恢复面
- Test witness: 新 `tests/update_expression_test.rs` RED（表达式形态 × 门序 × rekey/唯一/次级 × 回滚矩阵；先以字面量形态建立 GREEN 基线再观察新形态 RED）
- GREEN condition: 新矩阵 GREEN + update_index_maintenance / upsert / write_type_conformance / subset_insert 定向回归 0 failed
- Verification: `cargo test --test update_expression_test --test update_index_maintenance_test --test upsert_test --test write_type_conformance_test --test subset_insert_test -- --test-threads=1`（0 failed）
- Stop when: 同快照求值需改版本链语义（超出赋值求值层面），或预检前置与既有碰撞预检语义冲突——返回 Plan

### T10: 文档同步——SKILL.md 与 README 双语

- Requirement/Scenario: 设计 D6；依赖 T1–T9、T11、T12（能力全部落地后一次写准）
- Depends on: T8、T9、T11、T12
- Targets: `rtsql-docs/SKILL.md`（Aggregate/Write 两行现状边界改写）、`README.md`、`README.zh-CN.md`（能力清单：聚合嵌套、UPDATE SET 表达式、非 INTEGER PK 拒绝清单、dump PK 渲染与旧 dump 兼容注记）
- Current behavior: SKILL.md 携带本轮写入的现状边界（聚合顶层限定、UPDATE 字面量限定），与 Iteration 001 后实现不一致
- Required behavior: 文档与实现一致——能力面按落地后状态描述，旧 dump `TEXT PRIMARY KEY` restore 失败（exit 3）注记保留
- Preserve: 既有文档结构与其余条目不动；不新增能力声明（只改写与退役）
- Forbidden: 不改产品代码；不写超出已验收能力的承诺
- Test witness: 人工核对——逐条对照文档陈述与测试见证（最短操作：读 SKILL.md Aggregate/Write 两行与 README 能力清单，逐条指出对应测试文件）；结果一句话记入 Response
- GREEN condition: 文档陈述与实现一致、无现状边界残留
- Verification: 人工核对（自动命令无法判定文档语义）；无独立测试变更
- Stop when: 文档需要描述超出 R1–R6 已验收范围的能力——返回 Plan

### T11: 恢复重放目标表缺失定向错误面

- Requirement/Scenario: R6（S22 定向指因 / S23 正常关闭对照）
- Depends on: None
- Targets: `src/wal/recovery.rs::redo_record` Insert/Update/Delete 三臂 `get_table` 失败分支（`:722`/`:820`/`:947`）
- Current behavior: 缺失表与其余 `get_table` 失败统一归并 `table '{}' lookup failed during redo: {}`——「DDL 从未持久化」被诊断为「redo 目标表缺失」
- Required behavior: 底层错误为 `StorageError::TableNotFound` 时输出定向文案（四要素：表名 / catalog 缺失陈述 / 上次关闭未经 checkpoint 的可能原因 / 打开拒绝明示）；其余错误归并文案逐字节不变；打开失败、不跳过不丢弃记录、恢复行为与传播路径零变化
- Preserve: K05 显式报错；`WalError::RedoFailed` 变体与 display 前缀；其余三处 catalog 驱动的 `get_table` 消费点（`:349`/`:1040`/`:1416`）与 tail 持久化（`:670`）不动；既有恢复套件零回归
- Forbidden: 不新增 `WalError` 变体；不改 WAL 格式与恢复行为；不做库修复/补救指引或自动清理
- Test witness: 新 `tests/recovery_error_surface_test.rs` RED——lib open → CREATE TABLE → INSERT 提交 → drop 句柄不 `close()` → 重新 open：断言定向要素（修复前为归并文案）；S23 对照用例（同序列走 `close()` → open 成功行可见）
- GREEN condition: 两用例 GREEN + 既有恢复套件（recovery_tail_persistence / recovery_replay_convergence / wal_recovery_large 等）零回归
- Verification: `cargo test --test recovery_error_surface_test --test recovery_tail_persistence_test --test recovery_replay_convergence_test --test wal_recovery_large_test -- --test-threads=1`（0 failed）
- Stop when: `get_table` 失败无法按错误变体判别（契约前提失效），或定向化需改变打开结果/恢复行为——返回 Plan

### T12: 聚合显示名重名解析去歧义

- Requirement/Scenario: R2（S24 四形态 + 共享输出列对照）
- Depends on: None
- Targets: `src/parser/planner/query.rs` 条件包装 Aggregate 臂（显示名 `position` 首匹配定位）、`src/parser/planner/aggregate.rs::compile_aggregate_row_expr` 聚合 Function 引用臂（`output_columns.iter().position` 首匹配）——可引入共享解析 helper
- Current behavior: 四形态（重复别名 / 别名撞 `result_column_name` 的 wrap 定位、HAVING 引用、嵌入引用）按显示名首匹配解析到第一个同名注册项，静默取错值/错过滤（探针证据见 design D8）
- Required behavior: 每处聚合输出引用解析到与该调用对应的注册聚合输出（机制按 design D8 两路径择一或等价：结构匹配 / 出现序号）；同一调用多次出现仍共享一个输出列；重名可观察表头保持重名输出
- Preserve: 直出路径（无 T4 项、名字互异）逐字节不变；错误面零变化；`result_column_name` 命名与既有聚合五套件行为零回归
- Forbidden: 不改 `executor/aggregate.rs` 求值内核；不改可观察表头命名；不改拒绝面
- Test witness: `tests/aggregate_extension_test.rs` 重名矩阵 RED（S24 四形态按探针错误值断言当前行为 → 修正为正确值 + 共享输出列对照 `SELECT MAX(id), MAX(id)+1`）
- GREEN condition: 矩阵 GREEN + aggregate_extension / aggregate_output_shape / join_output_resolution / group_by_expr / order_by_alias_sort 五套件 0 failed
- Verification: `cargo test --test aggregate_extension_test --test aggregate_output_shape_test --test join_output_resolution_test --test group_by_expr_test --test order_by_alias_sort_test --test planner_test -- --test-threads=1`（0 failed）
- Stop when: 重名歧义无法在不改可观察表头或不破坏同调用共享语义的前提下消除（design D8 两路径均失效）——返回 Plan

**Invariants**

- `ON CONFLICT` / `REPLACE INTO` / UpsertExecutor 通道零变化；单列字面量 UPDATE 行为逐字节不变。
- 恢复重放行为与打开结果零变化：不跳过、不丢弃、不自动修复；缺失表之外的 redo 失败文案逐字节不变。
- 聚合可观察表头（含重名）、同调用共享输出列、直出形状零变化。
- 存量库兼容延续 Iteration 000/001 结论；全量测试基线 1833 passed / 0 failed / 2 ignored 不回归（本 Iteration 净增用例）。
- 文档只描述已验收能力。

**Non-goals**

UPDATE 子查询、聚合右值、DEFAULT 关键字进入 UPDATE SET；恢复重放跳过/丢弃不可重放记录、库修复指引；聚合进入 WHERE/JOIN ON、嵌套聚合、窗口函数；`check` 分级调整；SNAPSHOT/tasks 同步（Maintainer 职责）。

**Acceptance**

- S12–S17：UPDATE 列引用/常量/函数/CAST 表达式生效（S12–S13）；多列同旧行快照（S14）；类型门/NOT NULL/FLOAT 升格/日期 coerce 照常（S15）；键列表达式 rekey 与碰撞预检、非 Int 结果 `KeyTypeMismatch`（S16）；聚合与子查询右值计划期拒绝、`ON CONFLICT`/`REPLACE` 零回归（S17）。
- S22–S23：drop 不 close 重开错误定向指因（S22）；close() 对照 open 成功（S23）。
- S24：聚合重名四形态解析到正确注册输出；共享输出列对照保持。
- 文档与实现一致（T10 人工核对）。
- 映射：R3→T8/T9、R6→T11、R2（S24）→T12、D6→T10（tasks.md RTM）；场景→design D5/D7/D8→Targets→测试文件链路闭合。

**Verification**

- S12–S14：update_expression_test 表达式形态矩阵（自动）。
- S15–S16：门序/升格/coerce/rekey/碰撞矩阵（自动）。
- S17：拒绝面与 upsert/replace 回归（自动）。
- S22–S23：recovery_error_surface_test 两用例（自动）。
- S24：aggregate_extension_test 重名矩阵（自动）。
- T10：人工核对（最短步骤：SKILL.md 两行 + README 能力清单逐条对照测试见证；结果一句话记入 Response）。
- Iteration 收尾：M19 全量分批（`--lib`/`--bins` + 全部集成目标按名字典序分批，串行 `--test-threads=1`，每批 0 failed 方可宣称全量）+ fmt/clippy 干净。

**Gate 2 Readiness**

PASS——2026-10-03 依据用户集中决策 5 授权（原话「给出下一轮iter并把遗留issue记入一起解决」，proposal 已录）；R3 契约为 Gate 1 已批准计划 tasks 3.1–3.3 的展开，R6/T11 为用户指令并入，R2/T12 为 Iteration 001 Review finding 按同一指令并轮修复（机制见 design D8，行为契约已闭合）。Gate 2 检查：无 Missing（RTM R1–R6 全 Covered）、调查完整（T8/T9 锚点 2026-10-03 复核、T11/T12 锚点本会话实测核实）、设计闭合（D5–D8 无 TBD）、任务契约自包含（T8–T12 含变更面/见证/停止条件）、验证直接（无身份型证据工程）、Persisted Evidence none、无 BLOCKED/WAIVED。已知风险：详细计划尚未经用户事后审计——实施未开始前该风险可逆，审计后可经 replan 修订。

**Persisted Evidence**

- Mode: none

（全部验证可低成本重跑，Act Response 命令 + 决定性输出 + 退出码足以承载。）

**Risks and Notes**

- T9 预检前置与既有「写入前碰撞预检」的合并面：既有预检按单值驱动，多列化后预检需覆盖全部赋值列（含表达式求值结果）——实现顺序（先求值后预检后写入）在契约内，若与版本链快照语义冲突按 Stop-when 返回。
- T12 结构匹配路径需对 `AggregateFunc` 与归一实参做等价比较——若 `AggregateFunc` 不便派生相等性，出现序号路径为备选；两路径行为契约相同。
- T11 场景构造依赖「句柄 drop 不 close」——与既有测试夹具纪律（checkpoint+wal shutdown+drop）相反，属故意的负向夹具；测试文件头注明，避免后续被「纠正」。
- 非实质留 Act：`UpdateValue`/`UpdateAssignment` 的精确类型形态、定向错误文案措辞（四要素约束内）、T12 解析 helper 的落位、测试夹具形态。

## Act Response

- Status: reported

**Implemented**

- T12（R2/S24）`src/parser/planner/aggregate.rs`：聚合输出行编译器 `compile_aggregate_row_expr` 增 `canonical: &[String]` 参数（与 `output_columns` 聚合段平行的注册项规范名清单，`build_having_expression`/`build_having`/`compile_ceil_floor` 及全部递归臂穿线）；聚合 Function 引用臂由显示名 `position` 首匹配改为按注册项规范名（`AggregateFunc::result_column_name`，func 身份）解析，输出列索引 = 聚合段基址 `output_columns.len() - canonical.len()` + 规范名命中位；未注册引用既有拒绝通道（`HavingNonAggregatedReference` / `NonAggregatedColumn`）与文案逐字节保持。`register_aggregate_calls` 去重键由显示名改为规范名——别名遮蔽下按显示名去重会把未注册的不同调用误判为已注册（S24 形态④取错值的根源之一：该调用不进共享清单、执行器不计算）。`src/parser/planner/query.rs`：`SelectItemRole::Aggregate(String)` → `Aggregate(String, usize)` 携注册序号（两分类臂注册前记录 `aggregates.len()`），wrap Aggregate 臂按注册序号直接定位（消除显示名首匹配歧义）；聚合分支新增 `aggregate_canonical_names` 快照（`aggregates` 移入 AggregateNode 前采集，ORDER BY 编译点在分支后仍可消费）；wrap 嵌入项编译、`build_having`、ORDER BY 表达式键三消费点接 canonical。测试 `tests/aggregate_extension_test.rs` +5（S24 四形态 + 共享输出列对照 `SELECT MAX(id), MAX(id)+1`；夹具显式 INT PK——首列隐式主键撞重复组键）。
- T8（R3 计划面）`src/executor/plan.rs`：新 `UpdateValue { Literal(Value), Expression(ExpressionRef) }`（手写 `PartialEq`：字面量按值相等，表达式臂不可比较）与 `UpdateAssignment { column, value }`；`UpdateNode` 改 `assignments: Vec<UpdateAssignment>`。`src/parser/planner/ddl_dml.rs::build_update`：单赋值限制解除，逐赋值右值路由——`Value`/`TypedString`（日期解析）/NULL Identifier → `Literal`（既有语义逐字节）；非 NULL Identifier → `build_expression` 编译为 `Expression`（未知列 `ColumnNotFound`，索引按注册表列序）；`Subquery`/`InSubquery`/`Exists` → `UnsupportedStatement("UPDATE SUBQUERY")` 点名拒绝；其余（算术/CAST/CASE/COALESCE/标量函数）→ `build_expression` 既有内核（聚合调用不在标量注册表，经既有 `UnsupportedExpression(函数名)` 通道点名，与 WHERE 聚合同源）。`src/executor/plan_format.rs` Update 渲染臂改 `set col=…` 多赋值形态；`src/executor/mod.rs` 导出 `UpdateAssignment`/`UpdateValue`；`correlated.rs`/`get_plan_output_columns` 消费臂零变化（Update 仍为叶节点——表达式无关联参数面、DML 无输出列，穷尽性由编译保证）。测试 `tests/planner_test.rs` +7（多列/列引用/表达式五形态/未知列/子查询/聚合/字面量回归）+ `test_update` 校准到新节点形状。
- T9（R3 执行器）`src/executor/update.rs`：`UpdateExecutor` 改持 `assignments: Vec<UpdateAssignment>`；执行链重排——Step 1 search（`KeyNotFound` 优先不变）→ Step 2 读旧行前移（width-aware + DEFAULT 覆盖层保持；表达式求值输入）→ Step 3 逐赋值列位解析（`ColumnNotFound`）+ 全部右值按同一旧行快照求值（语句序；表达式求值错误沿 `ExecutionError("Expression evaluation error: …")` 映射，与 ProjectionExecutor 同款）→ Step 4 per-assignment 门序（语句序：NOT NULL → 键列类型门 + rekey 碰撞预检 → 日期族 coerce——既有单列顺序与错误文本逐字节）→ 唯一/二级旧值快照 → Step 5 赋值落位（语句序、同列后写胜出）→ UNIQUE 碰撞预检（逐字节保持）→ 类型门 + FLOAT 升格（per 赋值列语句序）→ serialize/write/WAL/record_version（逐字节不变）→ Step 7 PK 索引维护（PK 未赋值 → 条目 update；赋值 → 按落位后终值三分支）+ 唯一四分支 + 二级维护（values 驱动，逐字节不变）。`src/pipeline.rs` 过渡桥拆除直传 assignments；5 个既有测试文件（plan_exec/executor/gc/mvcc_record/version_chain）10 处 `UpdateExecutor::new` 直构机械校准，断言零变化。新 `tests/update_expression_test.rs` 14 用例（字面量基线/S12–S17 矩阵/次级索引/重启两态）。
- T11（R6）`src/wal/recovery.rs`：新 `redo_table_lookup_error` 分类 helper——`StorageError::TableNotFound` → 定向文案（四要素：表名 / catalog 缺失陈述 / 上次关闭未经 checkpoint 的可能原因含 lib 句柄未 `close()` 示例 / 打开拒绝且未跳过记录明示）；其余错误维持既有归并文案逐字节。Insert/Update/Delete 三臂 `get_table` 失败分支接线；仍走 `WalError::RedoFailed`（不新增变体、display 前缀不变），K05、打开失败传播零变化。新 `tests/recovery_error_surface_test.rs` 3 用例（S22 定向要素 / 打开仍拒绝 / S23 close() 对照；文件头注明「drop 不 close」为故意负向夹具）。
- T10 文档：`rtsql-docs/SKILL.md` Aggregate/Write 两行改写为新能力（聚合嵌套/UPDATE 表达式，拒绝面表述保留）、Schema 行非 INTEGER PK 改计划期拒绝 + 存量可用注记、refuse 清单增 explicit non-integer PRIMARY KEY、Dump behavioral notes 增 PK 渲染与旧 dump restore 失败注记；`README.md`/`README.zh-CN.md` 能力清单增「聚合表达式 / Aggregate expressions」条目、写面增「`UPDATE ... SET` 表达式」条目、约束面增 PRIMARY KEY 条目并把非 INTEGER PK 入计划期拒绝清单、存量兼容注记扩 PK 渲染与旧 dump 失败。
- 语义校准（预期后果白名单，2 处既有测试，均带 change 注记）：`tests/cross_db_dml_test.rs::update_delete_subquery_shapes_still_rejected` 子查询右值拒绝文案 `Unsupported value type` → `UPDATE SUBQUERY`（T8「错误携能力名」契约的直接后果；exit 3、计划期拒绝、数据完好断言零变化）；`tests/planner_test.rs::test_update` 与 ddl_dml 内嵌 `test_build_update` 校准到 `assignments` 形状（单字面量语义重断言）。

**Deviations from Plan**

- T8 节点演进与 `build_update` 路由编译耦合（构造点必须同步改写才可编译）——RED 以「新形状测试编译失败」（`UpdateNode` 无 `assignments` 字段，E0026）形式捕获，路由落地后 GREEN；无独立行为 RED 观察点（契约的 Current behavior 由 plan 调查探针与既有用例锚定）。
- T8 阶段短暂引入 pipeline 构造臂过渡桥（单字面量直通既有构造、其余 `StorageError::Internal` 显式拒绝）作为 T9 的 RED 观察点，T9 落地后拆除——「T8 不改执行器」以执行器文件零修改履行（桥位于 pipeline 消费臂）。
- T9 Step 2 读旧行前移至 NOT NULL/键列门之前（表达式求值需旧行快照）——仅当页读失败与门失败叠加时错误优先级有别于修复前（IO 故障角落，无测试锚定）；单列字面量形态的可观察门序与文本逐字节保持。
- `UpdateExecutor::new` 8 参 → 7 参（assignments）——计划未枚举 lib 级直构点，10 处机械校准断言零变化。
- T11 helper 落位于 `impl` 内共享（Plan 只约束三臂行为）；`TableNotFound(String)` 携表名载荷，定向文案以记录表名为准。

**Self-Review**

- 遗留 Minor（记录不返工）：① T9 多列为「先全部求值 → 逐列门序」两阶段——求值期错误（如标量函数类型错）先于所有门序错误可见；计划未定义多列错误优先级，属新行为面（单列形态优先级不变）。② T12 规范名解析下 `SELECT MAX(v) AS total … HAVING MAX(v)` 由修复前 `HavingNonAggregatedReference` 变为按注册输出正确求值；别名遮蔽单命中形态（`SELECT MAX(a) AS max_b … HAVING MAX(b)` 且 MAX(b) 未注册）由静默取错值变为点名拒绝——两者均无既有用例锚定，方向与 R2「解析到各自正确的注册聚合输出」一致。③ 同调用跨别名重复注册收敛（`SELECT MAX(v) AS x, MAX(v)+1` 两注册项 → 一个，最终结果不变、Aggregate 节点中间形状更紧凑）——「同一调用多次出现共享一个输出列」契约的直接兑现。④ T12 Identifier 臂（HAVING 引用别名）仍按显示名解析，别名与组键重名角落首匹配未变——S24 范围为 Function 引用与 wrap 角色，该角落无既有锚定。
- 无未解决 Critical/Important。

**Verification Evidence**

- T12 RED：`cargo test --test aggregate_extension_test -- --test-threads=1 duplicate_alias alias_shadowing having_disambiguates embedded_reference shared_output` → 4 FAILED（重复别名第三列 20/30 应 200/300；HAVING 空组；嵌入 21/31——与 Plan Review 探针一致）+ 1 passed（共享输出对照）。
- T12 GREEN：`cargo test --test aggregate_extension_test -- --test-threads=1` → `test result: ok. 39 passed; 0 failed`；五套件 + planner（aggregate_output_shape 8 / group_by_expr 12 / join_output_resolution 14 / order_by_alias_sort 7 / planner_test 100）全 0 failed。
- T8 RED：planner_test 新矩阵编译失败（`UpdateNode` 无 `assignments` 字段）——节点形状缺位的编译级见证。
- T8 GREEN：`cargo test --test planner_test -- --test-threads=1` → `107 passed; 0 failed`；UPDATE 相关 e2e（update_index_maintenance / key_type_conformance / write_type_conformance / subset_insert / explicit_tx）全 0 failed。
- T9 RED：`cargo test --test update_expression_test -- --test-threads=1` → `2 passed; 12 failed`（字面量基线与 NOT NULL 用例过，12 表达式/多列用例经过渡桥显式拒绝）。
- T9 GREEN：同目标 → `14 passed; 0 failed`；定向回归（planner 107 / update_expression 14 / update_index_maintenance / upsert / write_type_conformance / subset_insert / constraint_enforcement / explicit_tx / key_type_conformance）全 0 failed。
- T11 RED：`cargo test --test recovery_error_surface_test -- --test-threads=1` → `2 passed; 1 failed`（S22 实际输出 `WAL error: WAL redo failed: table 't' lookup failed during redo: table not found: t`，缺 catalog/checkpoint 要素）。
- T11 GREEN：同目标 → `3 passed; 0 failed`；恢复套件（recovery_tail_persistence / recovery_replay_convergence / wal_recovery_large / check_command）全 0 failed。
- Cycle 收尾（M19）：`cargo fmt --check` 干净；`cargo clippy --all-targets` 0 警告；114 个集成目标按名字典序 6 批 + `--lib` + `--bins`，串行 `--test-threads=1`，全部退出码 0——集成 1468 passed + lib 394 passed，全量 **1862 tests / 0 failures / 2 ignored**（1833 基线 + 净增 29：aggregate_extension +5、planner +7、update_expression +14、recovery_error_surface +3；cli_test 2 ignored 不变）。批次 2 首跑经 cross_db_dml 文案校准后重跑全绿（见语义校准）。
- `openspec validate --changes 2026-10-03-practical-core-gaps` → `✓ 1 passed, 0 failed`。

**Persisted Evidence**

- None required（plan 模式 none；全部验证可低成本重跑，Act Response 命令 + 输出 + 退出码足以承载）。

**Experience Candidates**

- None（全部验证为既有 cargo 命令常轨；close() 与 wal_buffer.shutdown() 的重启语义差异属既有文档化行为，非范围外缺陷）。

**Remaining Issues**

- 无未解决问题。Minor 项与语义校准如上；change 收尾归档与 SNAPSHOT/tasks 同步为 Maintainer 职责（本 Response 不触碰）。

## Plan Review

- Review Result: accepted

**Findings**

1. **（Minor，记录不返工）** `src/executor/plan_format.rs` Update 渲染臂注释与实现不符——注释称「字面量显示 SQL 形态」，实际两臂均按 `{:?}` Debug 形态渲染（如 `set v=Int(11)`）。`EXPLAIN` 对 DML 的计划期拒绝面（语料 `explain-plan-output` R2）使该臂自 CLI 不可达，纯注释层瑕疵、无行为后果；随下一个触碰该文件的 change 顺带修正即可。
2. **（Minor 复核，同意不返工）** Act Self-Review 四项逐一复核成立：① 多列「先全部求值 → 逐列门序」使求值期错误先于门序错误可见——同快照语义（S14）的必然结果，计划未定义多列错误优先级，属新行为面且单列形态优先级不变；②③ 为 T12 规范名解析机制的直接预期后果（`HAVING MAX(v)` 按注册输出求值、别名遮蔽单命中由静默取错转点名拒绝、同调用跨别名注册收敛），方向与 R2「解析到各自正确的注册聚合输出」一致；④ Identifier 臂显示名首匹配角落无既有用例锚定，在 S24 声明范围（Function 引用与 wrap 角色）之外。
3. **（偏差分类，全部非阻塞）** T8 节点演进与路由编译耦合（RED 以 E0026 编译失败形态捕获——Plan 遗漏，见证仍有效：形状缺位的编译级证据）；T8 阶段 pipeline 过渡桥（T9 RED 观察点，T9 落地后拆除——「T8 不改执行器」以执行器文件零修改履行，契约内 Act 实现选择）；T9 Step 2 读旧行前移至门序之前（表达式求值需旧行快照——Plan 遗漏，仅 IO 故障与门失败叠加角落错误优先级有别，无测试锚定，已声明）；`UpdateExecutor::new` 8 参 → 7 参 10 处机械校准（Plan 遗漏——计划未枚举 lib 级直构点，断言零变化，Review 已核对五文件全部改动均为构造器换装）；T11 helper 落位 impl 内共享（契约内非实质选择）。
4. **（语义校准核实）** 2 处既有测试校准均带 change 注记且行为断言未弱化：`cross_db_dml_test::update_delete_subquery_shapes_still_rejected` 文案 `Unsupported value type` → `UPDATE SUBQUERY`（T8 能力名契约直接后果，exit 3 / 计划期 / 零副作用断言保持）；`planner_test::test_update` 与 `ddl_dml` 内嵌 `test_build_update` 校准到 `assignments` 形状（单字面量语义重断言）。

偏差结论：无 Act 偏离、无基线变化、无计划外修改面（30 个修改 + 3 个新增文件全部有 Iteration 归属声明，逐文件核对无遗漏）；无身份型证据机制。Act Response 声明与代码逐项核对一致（详见 Evidence）。

**Acceptance Gaps**

无——S12–S17（update_expression_test 14 用例矩阵 + planner_test 拒绝面 + 五套件定向回归）、S22–S23（recovery_error_surface_test 3 用例，含 K05 打开拒绝对照与 close() 正常恢复对照）、S24（aggregate_extension_test 5 用例，四形态期望值与 Plan 探针证据逐值吻合 + 共享输出列对照）全部有 GREEN 见证；T10 文档经 Review 独立读 diff 核对（README 双语三处 + SKILL.md 四处与实现一致，能力陈述未超出 R1–R6 已验收范围，旧 dump 兼容注记在位）。M19 全量 1862 tests / 0 failures / 2 ignored、fmt/clippy 干净、`openspec validate` 通过。

**Evidence**

- 独立审读完整 diff（2026-10-04，HEAD 9a103c0 + 工作区）：`src/executor/plan.rs`（`UpdateValue` 手写 `PartialEq` 字面量按值 / `UpdateAssignment` / `UpdateNode.assignments`）、`src/parser/planner/ddl_dml.rs::build_update`（单赋值限制解除、五臂路由——Value/TypedString/NULL Literal 逐字节保留、非 NULL Identifier 与其余形态经 `build_expression`、Subquery/InSubquery/Exists 点名 `UPDATE SUBQUERY`、聚合经未注册函数通道）、`src/executor/plan_format.rs`（多赋值渲染）、`src/executor/mod.rs`（导出）、`src/pipeline.rs`（构造臂直传 assignments）、`src/executor/update.rs`（Step 重排全链——同快照求值、per-assignment 门序 NOT NULL → 键列门 + rekey 预检 → coerce 与既有顺序逐字节同构、赋值落位同列后写胜出、类型门 per 赋值列、Step 7 按 `pk_assigned` 三分支派生自落位后终值、唯一/二级维护未触碰）、`src/wal/recovery.rs`（`redo_table_lookup_error` 分类——`TableNotFound` 四要素定向文案、其余归并文案逐字节不变、三臂接线、无新变体）、`src/parser/planner/aggregate.rs`（`compile_aggregate_row_expr` canonical 穿线 + 规范名解析臂、`register_aggregate_calls` 去重键改规范名）、`src/parser/planner/query.rs`（`SelectItemRole::Aggregate(String, usize)` 注册序号两分类臂、`aggregate_canonical_names` 快照、wrap/HAVING/ORDER BY 三消费点接线、`key_count + reg_idx` 与直出路径零变化）；测试面（update_expression 14 / recovery_error_surface 3 / aggregate_extension +5 / planner +7 + 校准 / cross_db_dml 校准 / 五机械换装文件断言零变化）；文档三处。
- Act 的测试运行结论采信（来源：用户授权「测试可信」2026-10-04 + Act Response Verification Evidence 2026-10-03；Review 独立 diff 审读与声明一致、被覆盖代码表面在结论产生后未变化，未触发公共规则 › 验证 的任何定向重跑情形）。

**Follow-up Decision**

接受：Acceptance 全部满足，无阻塞 finding；Finding 1 仅记录（注释层瑕疵，不构成本 Cycle 返工理由）。Iteration 002 完成，change 全部 Iteration（000/001/002）accepted，R1–R6 全部闭环，可进入 Maintainer 收尾（增量规格合并、change 归档、SNAPSHOT/tasks 刷新）与提交入库。

**Iteration Plan Update**

（无——Map 不变）

**Next Cycle**

None

**Next Iteration**

None（Map 末位 Iteration——change 全部需求闭环）
