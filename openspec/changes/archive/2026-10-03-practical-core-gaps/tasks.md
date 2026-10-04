# 任务与逻辑迭代

需求于 2026-10-03 获批（Gate 1，四项集中决策见 proposal）；2026-10-03 用户指令追加集中决策 5（展开 Iteration 002 + 遗留 Issue 并入 R6/T11，原话见 proposal），并按 Iteration 001 Plan Review 以 T12 并入聚合重名解析修复（映射 R2/S24）。实现调查与设计见 design.md。复选框表示实施状态。

## 1. 正确性与诚实化小批（bug 修复 + 计划期拒绝 + dump 适配）

- [x] 1.1 T1：建表首数据页初始化——`create_table_with_constraints` 在 `allocate_page()` 后、索引创建前经 `buffer_pool.get_page` + `modify_page` 执行 `SlottedPage::init(page, 0x03)`（镜像 truncate head 重置先例，design D1）；全建表通道单点覆盖；`check` 对 0x00 页 error 分级与损坏注入用例零变化；打开路径不自愈（R1，S1–S5）；依赖 None；以 check_command_test 空表用例（裸建表 check exit 0 单行干净 + CTAS/restore/ATTACH 通道矩阵 + close 重开两态 + 损坏注入零回归 + 存量 0x00 夹具首次写入/TRUNCATE 转绿）RED→GREEN 验收。
- [x] 1.2 T2：非 INTEGER PRIMARY KEY 计划期拒绝——`build_create_table` 在 `extract_primary_key` 后按 PK 列 `data_type != ColumnType::Int` 走 `UnsupportedConstraint("NON-INTEGER PRIMARY KEY")`（design D2；列级与表级 PK 同点；仅显式分支，隐式首列主键与 import 通道零波及）；存量非 INT PK 表打开/读写/dump 不触碰（R4，S18–S19）；依赖 None；以 planner_test 拒绝矩阵（五类型 × 列级/表级）+ constraint_enforcement_test e2e（零副作用、INTEGER PK 全形态保持、隐式主键语义零变化、存量夹具可用）RED→GREEN 验收。
- [x] 1.3 T3：dump DDL 渲染适配——`create_table_sql` PK 渲染条件加 `matches!(col.column_type, ColumnType::Int)`（design D3）；INTEGER PK 表输出逐字节不变；存量非 INT PK 表与隐式非 INT 首列表 DDL 不含 PRIMARY KEY 且 restore 往返可达（R5，S20）；依赖 None（与 T2 语义配套但可独立验证）；以 cli_test dump 用例（INTEGER PK 逐字节 + 存量 TEXT PK/隐式首列表往返）RED→GREEN 验收。

## 2. 聚合表达式嵌入

- [x] 2.1 T4：聚合提取基建 + SELECT 项嵌入——`aggregate.rs` 新增 `contains_aggregate_expr` 递归判定与提取-改写遍历（自顶向下、不进聚合参数、经 `extract_aggregate_func` 规范化、按 `result_column_name` 去重共享输出列、节点改写为聚合输出行 `ColumnExpression`）；`query.rs` SELECT 项分类扩展：顶层命中既有路径、`contains_aggregate_expr` 命中新「聚合嵌入表达式项」经既有条件 Projection 包装通道在聚合输出行求值（输出名 = 别名优先否则 Display）；纯常量项合法混排（design D4；R2，S6/S9/S11）；依赖 None；以 aggregate_extension_test 嵌套矩阵（`MAX(id)+1`/`COALESCE(MAX(id),0)+1`/常量混排 SELECT 与 INSERT SELECT/`SUM(DISTINCT v)+1`）+ planner_test 单元 RED→GREEN 验收。
- [x] 2.2 T5：HAVING 完整表达式编译——`build_having_expression` 扩展完整表达式树臂（BinaryOp 算术/比较、LogicalOp、Nested、CAST、CASE/COALESCE、标量函数；聚合引用既有臂保持），`_` 兜底收窄（design D4；R2，S7）；依赖 T4（共享提取基建）；以 aggregate_extension_test HAVING 矩阵 RED→GREEN 验收。
- [x] 2.3 T6：ORDER BY 聚合表达式键——`query.rs:2186` 聚合分支非裸列键经 T5 编译器产出 `OrderByColumn.expr`（SortExecutor 既有 decorate-sort-undecorate 消费），表达式键豁免列集校验（design D4；R2，S8）；依赖 T5（同一编译器）；以 aggregate_extension_test 排序矩阵（`ORDER BY MAX(id)+1`/`ORDER BY MAX(v)`）RED→GREEN 验收。
- [x] 2.4 T7：拒绝面保持与聚合 e2e 收口——WHERE/JOIN ON 内聚合（未注册函数名既有通道）、嵌套聚合（`InvalidAggregateArgument`）、无 GROUP BY 非聚合列引用（NonAggregated）三拒绝面 e2e 固化；既有聚合套件（aggregate_extension/aggregate_output_shape/join_output_resolution/group_by_expr/order_by_alias_sort）定向回归（R2，S10/S12）；依赖 T4/T5/T6；以新增拒绝面用例 + 定向回归 0 failed 验收。

## 3. UPDATE 值表达式与文档收尾

- [x] 3.1 T8：UPDATE 计划面——`UpdateNode` 改 `assignments: Vec<UpdateAssignment>`（`UpdateValue::Literal | Expression` enum，字面量臂逐字节保留含 TypedString 日期解析）；`build_update` 解除单赋值限制，逐赋值右值路由（字面量/列引用/子查询点名拒绝/聚合点名拒绝/表达式编译），`plan_format`/`correlated.rs`/`get_plan_output_columns` 消费臂穷尽补齐（design D5；R3）；依赖 None；以 planner_test 赋值矩阵 RED→GREEN 验收。
- [x] 3.2 T9：UPDATE 执行器多列化——逐命中行旧行快照求值全部表达式右值（S14 同快照语义）；门序保持（coerce→类型门 FLOAT 升格→NOT NULL→键列类型门）per 列；预检前置（PK 碰撞 + 变更唯一列碰撞，全部写入前，零副作用）；索引维护（rekey 三分支/唯一条目/次级索引既有路径）；语句失败隐式事务回滚全部已写行（design D5；R3，S12–S17）；依赖 T8；以新 `tests/update_expression_test.rs`（表达式形态 × 门序 × rekey/唯一/次级 × 回滚）+ update_index_maintenance_test 定向回归 RED→GREEN 验收。
- [x] 3.3 T10：文档同步——SKILL.md Aggregate/Write 两行边界改写为新能力（本轮现状边界描述退役）；README.md/README.zh-CN.md 能力清单同步（聚合嵌套、UPDATE SET 表达式、非 INTEGER PK 拒绝清单、dump PK 渲染与旧 dump 兼容注记）（design D6）；依赖 T1–T9；以文档与实现一致性人工核对验收。

## 4. 遗留并入（恢复错误面诚实化 + 聚合重名解析修复）

- [x] 4.1 T11：恢复重放目标表缺失定向错误面——`src/wal/recovery.rs` `redo_record` Insert/Update/Delete 三臂（`:722`/`:820`/`:947`）`get_table` 失败分支按底层错误变体判别：`StorageError::TableNotFound` → 定向文案（点名表名 + catalog 缺失事实 + 最可能原因〔上次关闭未经 checkpoint，如 lib 句柄未 `close()` 即 drop、未持久化 DDL 随脏页丢失〕+ 打开拒绝），其余错误维持既有归并文案逐字节不变；仍走 `WalError::RedoFailed(String)`，不跳过/不丢弃不可重放记录，K05 与打开失败传播路径零变化（design D7；R6，S22–S23）；依赖 None；以新 `tests/recovery_error_surface_test.rs`（drop 不 close 定向指因 RED→GREEN + close() 对照）+ 既有恢复套件零回归验收。
- [x] 4.2 T12：聚合显示名重名解析去歧义——`src/parser/planner/query.rs` 条件包装 Aggregate 臂（显示名 `position` 首匹配）与 `src/parser/planner/aggregate.rs::compile_aggregate_row_expr` 聚合 Function 引用臂按 design D8 消除重名歧义（结构匹配或出现序号，机制留 Act；可观察表头、同调用共享输出列、直出路径、错误面零变化）（design D8；R2，S24）；依赖 None（Iteration 001 变更面自身回归修复）；以 `tests/aggregate_extension_test.rs` 重名矩阵（四形态 + 共享输出列对照）RED→GREEN + 既有聚合五套件零回归验收。

## Iteration Plan

### Iteration 000：正确性与诚实化小批（存储 bug + 计划期拒绝 + dump 适配）

- Tasks：T1、T2、T3。
- Depends on：None。
- 阶段结果：新建即含零行表的库 check 干净（全建表通道）；非 INTEGER 显式 PK 计划期点名拒绝且隐式主键/import 通道零波及；dump DDL 渲染与实际强制语义一致且往返可达；损坏检测与既有面零回归。
- Stable baseline：存储正确性 bug 关闭、诚实化缺口关闭，为后续表达式能力批提供干净基线（后续 Iteration 的 check/dump 观测不再有噪音）。
- Verification boundary：check_command_test 扩展 + planner_test/constraint_enforcement_test/cli_test 扩展 + 既有 check/约束/dump 套件定向回归 + fmt/clippy。
- Diagnostic boundary：`src/storage/data/table_manager.rs::create_table_with_constraints`、`src/parser/planner/ddl_dml.rs::{build_create_table, extract_primary_key}`、`src/cli/lifecycle.rs::create_table_sql`、对应测试文件。
- Non-goals：聚合表达式嵌入与 UPDATE 表达式（Iteration 001/002）；文档同步（Iteration 002）。
- 平衡审计：三个任务同属「声明与可观测状态诚实化」单一故障域（存储页状态、约束声明、dump 输出三者都是「表达的状态与真实状态不一致」），各自改动面小（单点 ± 数行到数十行）、验证面独立但共享 check/约束/dump 套件回归边界；拆开会形成三个过薄 Iteration，合为一轮形成单一可验收结果。

### Iteration 001：聚合表达式嵌入（读面能力）

- Tasks：T4、T5、T6、T7。
- Depends on：Iteration 000（干净 check/dump 基线；无代码级硬依赖）。
- 阶段结果：聚合可嵌套于 SELECT 清单值表达式、HAVING 条件、聚合查询 ORDER BY 键；常量项可与聚合混排（SELECT 与 INSERT SELECT 源同语义）；DISTINCT 量化词在表达式中保持；拒绝面（WHERE/ON 聚合、嵌套聚合、NonAggregated）固化；既有聚合输出形状零回归。
- Stable baseline：读面聚合表达式能力完整，可支撑 next-id 等标准 SQL 形态。
- Verification boundary：aggregate_extension_test 嵌套/HAVING/排序/拒绝面矩阵扩展 + planner_test 单元 + 既有聚合五套件定向回归 + fmt/clippy。
- Diagnostic boundary：`src/parser/planner/aggregate.rs`（提取基建/HAVING 编译器）、`src/parser/planner/query.rs`（SELECT 项分类循环/聚合 ORDER BY 分支）、`src/executor/aggregate.rs` 与条件 Projection 包装消费。
- Non-goals：聚合进入 WHERE/JOIN ON；嵌套聚合；窗口函数；UPDATE 表达式（Iteration 002）。
- 平衡审计：T4 是基建且承载主场景，T5/T6 是同一编译器向两个子句的推广（依赖 T4/T5 单向链），T7 是四者的共享验收收口（拒绝面回归矩阵）；同属「聚合输出行上的表达式求值」单一机制域，共享 aggregate_extension_test 验证边界。拆分会把共享基建与收口割裂，合为一轮。

### Iteration 002：UPDATE 值表达式、恢复错误面与文档收尾（写面能力 + 遗留并入）

- Tasks：T8、T9、T10、T11、T12。
- Depends on：Iteration 000（T2 的 PK 类型判定与 T9 键列门共享语义面）；Iteration 001（T12 修复其引入的聚合重名解析回归、T10 文档覆盖其能力面）。T11/T12 相互独立、与 T8/T9 无代码级依赖。
- 阶段结果：`UPDATE ... SET` 接受完整值表达式右值与多列赋值（同旧行快照）；类型门/NOT NULL/键列门/碰撞预检/rekey/唯一与次级索引维护/回滚语义全部保持；`ON CONFLICT`/`REPLACE` 零回归；恢复重放对 catalog 缺失表点名指因（打开失败语义不变）；聚合显示名重名时各引用解析到正确注册输出；SKILL.md/README 双语与实现一致。
- Stable baseline：写面表达式能力完整、遗留缺陷两项闭环，change 全部需求闭环，文档与实现一致，可进入收尾归档。
- Verification boundary：新 tests/update_expression_test.rs + tests/recovery_error_surface_test.rs + aggregate_extension_test 重名矩阵 + planner_test 赋值矩阵 + update_index_maintenance/upsert/write_type_conformance/subset_insert/aggregate 五套件定向回归 + fmt/clippy + M19 全量分批收尾。
- Diagnostic boundary：`src/parser/planner/ddl_dml.rs::build_update`、`src/executor/plan.rs::UpdateNode`、`src/executor/update.rs`、`src/wal/recovery.rs::redo_record` 三臂、`src/parser/planner/{query,aggregate}.rs` 引用解析、文档三处。
- Non-goals：UPDATE 子查询、聚合右值、DEFAULT 关键字进入 UPDATE SET；恢复重放跳过/丢弃不可重放记录与库修复指引。
- 平衡审计（2026-10-03 修订）：原三轮（T8/T9 写面、T10 文档）审计成立；T11（恢复错误面单点文案 + lib 测试）与 T12（聚合引用解析去歧义）各自改动面小、与写面无共享故障域，但均为本 change 遗留收口——单独成轮过碎（各为单点小任务），并入收尾轮共享 M19 全量分批验证边界；T10 依赖全部能力落地后一次写准，仍居末。修订后五任务同属「change 收口」单一阶段成果，工作量适中、验证边界清晰。

## Requirements Traceability Matrix

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| R1 建表首数据页初始化 | T1 | check_command_test 空表干净/通道矩阵/两态/损坏注入零回归 + 存量夹具转绿 | Covered |
| R2 聚合表达式嵌入 | T4、T5、T6、T7、T12 | aggregate_extension_test 嵌套/常量混排/DISTINCT/HAVING/ORDER BY/拒绝面/重名解析（S24）矩阵 + 既有聚合套件回归 | Covered |
| R3 UPDATE SET 值表达式 | T8、T9 | update_expression_test 形态×门序×索引×回滚矩阵 + update_index_maintenance 回归 | Covered |
| R4 非 INTEGER PK 计划期拒绝 | T2 | planner_test 拒绝矩阵 + constraint_enforcement_test e2e（零副作用/INTEGER 保持/隐式零变化/存量可用） | Covered |
| R5 dump DDL 渲染适配 | T3 | cli_test INTEGER PK 逐字节 + 存量非 INT PK/隐式首列表往返 | Covered |
| R6 恢复重放目标表缺失的错误面诚实 | T11 | recovery_error_surface_test drop 不 close 定向指因 + close() 对照 + 既有恢复套件回归 | Covered |

R1–R6 全部 Covered，无 Simplified/Missing。

## Phase / Gate 与验证状态

- CLARIFY completed；Gate 1 PASS（2026-10-03 用户批准需求与范围；四项集中决策见 proposal）。
- INVESTIGATE completed；Current-State Evidence 见 design（四区域入口/调用链/机制事实 file:line 核实；探针实测闭合「0x00 head 页可写入」未知项）。
- DESIGN / TASKS / RTM completed；实质未知项 None（非实质选择：提取-改写的内部函数形态、错误文案最终措辞——在「点名能力」契约内由 Act 定）。
- Gate 2 PASS——2026-10-03 依据用户原始指令授权（原话见 Iteration 000 Cycle Gate 2 Readiness；四项实质决策 Gate 1 用户亲选，计划无新增用户面向选择）；已知风险：详细计划尚未经用户事后审计，实施前可逆。当前 Cycle Status=ready，无 BLOCKED/WAIVED；`openspec validate` valid。
- Persisted Evidence：none（全部验证可低成本重跑，Act Response 足以承载）。
- SKIPPED：产品代码修改（Act 职责）、Iteration 001/002 目录创建（Map 中保留，展开待前序 accepted）。

最终回归按 M19：`--lib`/`--bins` 与全部集成测试目标按名字典序分批，串行独立 `cargo test <明确目标> -- --test-threads=1`；每批 0 failed 方可宣称全量。不使用无目标过滤的全量调用，不创建辅助判定脚本、身份字段或证据协议。
