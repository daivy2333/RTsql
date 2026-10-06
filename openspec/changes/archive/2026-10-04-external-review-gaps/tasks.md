# 任务与逻辑迭代

需求基线见 proposal（四项语义决策 2026-10-04 用户批准生效；**replan 2026-10-04**：用户裁定原话「同意，给出replan，把isuee也解决」——自连接双重缺陷以 R9/T9 并入，Issue 随 T9 交付解决，Iteration 000 扩围；**replan 2026-10-05**：用户指令原话「重新给出00cyc，把issue也计划解决进去」——Iteration 000 Review 报告的两项范围外 Issue 候选以 R10/T10（JOIN 基座输出列序）、R11/T11（三表链式 JOIN 装配）并入，Iteration 001 扩围为 T4/T5/T11/T10，以 `iterations/001-subquery/001-replan.md` 执行）。实现调查与设计决策见 design.md D1–D11。复选框表示实施状态。

**工作面划分**（用户要求）：T1–T7、T9–T11 为**代码面**；T8 为**文档面**（README.md / README.zh-CN.md / rtsql-docs/SKILL.md，纯文档无 spec delta）；反馈 8c（WAL 路径拼接）当前版本不复现，**无工作项**；反馈 8a/8b 的消息问题分别随 T4/T6 的语义修复自然消失，无独立文案任务。

## 1. 计划形状与恢复正确性批

- [x] 1.1 **T1〔代码〕** IndexScanAll 列元数据契约统一——`IndexScanAllNode.columns` 恒置 `base_schema`（删除 query.rs:1836-1840 条件分支，design D1）；`pipeline.rs::extract_column_indices` IndexScanAll 臂按 projection 裁剪对齐（唯一反向假设消费方，实际不可达）；PK IndexScan 臂与消费契约不动；聚合/派生/CTAS/INSERT SELECT 列解析随契约自动修正（R1，S1–S5）；依赖 None；以新 `tests/secondary_index_test.rs` 投影矩阵（后位单列不 panic + 乱序对齐 + EXPLAIN 不崩 + PK/范围回落零回归 + 扫描上聚合）+ cli_test 表头用例 RED→GREEN 验收。（Act 2026-10-04 完成）
- [x] 1.2 **T2〔代码〕** 恢复会话 WAL 收敛——`RecoveryResult` 增 `wal_had_tx_records`（任一非 Checkpoint 记录）；`CheckpointManager::new` 增参；零工作守卫加 `!pending_wal_at_open` 条件；lifecycle checkpoint zero_work 取样 OR 同标志（design D2）；`check`/`--readonly`/checkpoint 命令消息随之收敛（R2，S6–S9）；依赖 None；以新 `tests/recovery_wal_convergence_test.rs`（未提交 SIGKILL → 纯读会话收敛 + checkpoint 消息两形态 + marker-only 形态 + 零工作跳过保持 + redo>0 零变化）+ checkpoint_skip/check_command 既有套件零回归 RED→GREEN 验收。（Act 2026-10-04 完成）
- [x] 1.3 **T3〔代码〕** JOIN 基座 ORDER BY 限定键——`carry_all_columns` 条件加 `|| !query.order_by.is_empty()` + 裸 Join/NLJ 基座 `sort_projection` 臂（`resolve_join_projection_indices`）+ 限定键经 `join_row_layout`/`resolve_layout_index` 布局解析产出表达式键（design D3，与 T4 共享 sort_projection 臂，先 T3）；自连接同名列精确解析顺带修复（R3，S10–S12）；依赖 None；以新 `tests/table_alias_resolution_test.rs` JOIN 排序矩阵（e.id/e.salary/emp.id + 自连接限定列 + 单表/带 WHERE JOIN/聚合/DISTINCT/集合尾零回归）RED→GREEN 验收。（Act 2026-10-04：实现完成，S10/S12 GREEN；S11 曾阻塞——测试前提依赖的自连接 Hash/NLJ 输出行被既有执行层双重缺陷破坏〔条件交换判定用归一真实表名 + 输出归属无法区分同表两侧〕，Blocker Handoff 见 Iteration 000 Act Response；replan 裁定：S11 用例解除 `#[ignore]` 转入 T9 验收——**Act 2026-10-04 replan Cycle 随 T9 交付解除 ignore 转 GREEN，S11 闭合，本任务验收完成**）
- [x] 1.4 **T9〔代码〕**（replan 2026-10-04 并入）自连接侧身份正确性——`extract_join_conditions` 交换判定改按书写限定符与声明限定符匹配（签名平行增参，两腿异侧按侧装配、双腿同侧 `InvalidQuery` 点名零副作用、裸名腿位置语义不变、未知限定符 `TableNotFound` 同面）；新 `JoinSide` + `JoinNode`/`NestedLoopJoinNode` 加性 `output_sides`（RIGHT 交换臂反转），两执行器 `build_output_row`/`build_padded_row` 改侧标志归属，`left_table_name` 参数链删除（design D9；R9，S31–S37）；依赖 None（装配面与 T3 相邻，S11 解除依赖本任务）；以新 `tests/self_join_test.rs` 矩阵（Hash 正向/反向配对 + 投影归属 + LEFT/FULL 外连接 + 双腿同侧拒绝 + 非自连接对照）+ `table_alias_resolution_test` S11 解除 ignore 转 GREEN + join/nested_loop_join/join_output_resolution/outer_join/cross_db_query/planner 定向回归 RED→GREEN 验收。（Act 2026-10-04 完成：RED 5 failed/1 passed → GREEN；定向六套件 + 全量 1882/0/2；S33 见证形态按 JOIN 基座表序输出既有语义校准，偏差与范围外观察见 Cycle Act Response）

## 2. 子查询面与 JOIN 装配收口批

- [x] 2.1 **T4〔代码〕** WHERE 标量子查询比较 + 投影位形状——新 `ScalarSubqueryFilterNode` 计划节点与 `ScalarSubqueryFilterExecutor`（逐行参数注入 + 语句级缓存，非相关退化为单缓存项；多行 `SubqueryReturnsMultipleRow`、零行 NULL 三值语义）；planner WHERE 含 `Expr::Subquery` 递归检测 → 禁止下推 + 谓词编译占位替换 + bindings 登记；JOIN 基座维持点名拒绝；SELECT 投影位恒等门摘除（query.rs:1685-1686）+ `result_column_index` 公式沿用 + subquery_test 参考形状校准（design D4；R4，S13–S18）；依赖 T3（共享 JOIN sort_projection 臂）；以新 `tests/scalar_subquery_comparison_test.rs`（非相关/相关/空/多行/拒绝面/不下推见证）+ subquery_test 形状校准 RED→GREEN 验收。（Act 2026-10-05 完成：新矩阵 14 例 + subquery_test 32 例（name_value_map 系 5 处按预定白名单校准至 SELECT 序形状）+ cli_test 泄漏形状见证 1 处白名单校准；相关参数取值列经附加列 + output_width 收尾截断机制保持既有相关面形状——见 Act Response 偏差记录）
- [x] 2.2 **T5〔代码〕** FROM 派生表内集合操作——`building_subquery` 拆「FROM 派生上下文」标志，仅 FROM 派生放行 `build_set_operation`；视图展开与表达式子查询内维持拒绝；全链其余零改动（design D5；R5，S19–S21）；依赖 None；以新 `tests/set_operations_test.rs` 派生表矩阵（UNION/UNION ALL/INTERSECT/EXCEPT × 外层叠加 + IN 内维持拒绝）RED→GREEN 验收。（Act 2026-10-05 完成：新增 7 例 S19–S21 矩阵 + cte/view/subquery 既有套件零回归）
- [x] 2.3 **T11〔代码〕**（replan 2026-10-05 并入）多表链式 JOIN 装配正确性——装配循环重构（非末步强制链前缀全组合承载、严格校验与 kept 仅末步执行）+ 迭代 ≥2 链前缀列 `column_index` 重定基为左输入行绝对偏移（`output_sides` 机制不变）+ Hash 条件键限定解析（执行器 `table.column` 优先/裸名回退、pipeline Join 构造臂注册限定键）（design D11；R11，S42–S46）；依赖 None；以新 `tests/chain_join_test.rs` 矩阵（hash+hash 链 / hash+NLJ 混合链 / 仅前两表 SELECT 不 panic / 链上同名列不串列 / 两表单步零回归对照）+ join/nested_loop_join/join_output_resolution/outer_join/self_join/cross_db_query 既有套件零回归 RED→GREEN 验收（当前三形态全坏——计划失败 ×2 / panic ×1，即 RED 基线）。（Act 2026-10-05 完成：chain_join_test 9 例 GREEN；RED 基线实测与 replan 调查不同——T9 交付后工作区三形态均为静默错值（串列直击），RED 更强；join/cross_db/alias 等十一套件回归零失败）
- [x] 2.4 **T10〔代码〕**（replan 2026-10-05 并入）JOIN 基座输出列序 SELECT 化——`kept` 分支改按 SELECT 清单序重建（三清单同步、重复项重复产出）+ `resolve_join_projection_indices` 返回序 SELECT 化（WHERE/ORDER BY 裁剪路径随之；命中/歧义错误面不变）（design D10；R10，S38–S41）；依赖 T11（同在末步 kept 分支，后置避免同面双触）；以 join_output_resolution_test 扩展（SELECT 序矩阵 + 重复项 + WHERE/ORDER BY 路径）+ self_join_test S33 校准（表序断言翻转为 SELECT 序）+ ctas/derived_scan_alias/cross_db_query/cli_test 定向回归 RED→GREEN 验收。（Act 2026-10-05 完成：新增 5 例 + self_join S33 第二形态按预定校准翻转为 SELECT 序 + join/self/ctas/derived/cross_db/cli 回归零失败）

## 3. 写面与 CLI 可观测批

- [x] 3.1 **T6〔代码〕** 无 WHERE 全表 UPDATE/DELETE——`UpdateNode/DeleteNode.key` 改 `Option<Key>`；第一重门移除、第二重门（PK 等值）保留；执行器全表臂（`IndexManager::scan_all` 枚举 + 逐行既有单行逻辑复用 + AffectedRows 汇总）；语句失败隐式事务回滚全部已写行；附加库路由继承；TRUNCATE/带 WHERE 形态零变化（design D6；R6，S22–S26）；依赖 None；以新 `tests/full_table_dml_test.rs`（全表 DELETE/UPDATE + 逐行碰撞回滚 + 显式事务回滚 + 附加库目标 + TRUNCATE 对照 + 子查询拒绝面）+ subquery_test 两处静默失败用例转真验证 + cross_db_dml_test 回归 RED→GREEN 验收。（Act 2026-10-06 完成：单行逻辑提取 `update_row`/`delete_row` 共享路径零复制、RED 10 failed/0 passed → GREEN 10 passed；plan_exec/executor/gc/mvcc_record/version_chain/planner_test 八处直构与断言机械适配；subquery_test 两静默用例转真删除后断言仍成立）
- [x] 3.2 **T7〔代码〕** CLI 错误输出携带 SQLSTATE——文本格式错误行尾 ` [SQLSTATE xxxx]`（sqlstate 存在时）；`--format json` 错误对象增 `sqlstate` 字段（None 省略）；退出码与文案零变化；PG 面零变化（design D7；R7，S27–S29）；依赖 None；以 readonly_session_test/cli_test 错误面用例扩展（文本 + JSON + 无 sqlstate 省略）+ pg_integration_test 零回归 RED→GREEN 验收。（Act 2026-10-06 完成：`ExitStatus::Sql` 携带第二载荷 + `emit_error_status` 渲染分叉〔`kind(format)` 单点判定〕+ sql_failure_status 增参五透传点〔classify×2/plan/Response::Error 携带，parse/会话/ATTACH/lifecycle 为 None〕；cli_test 三处 `stderr.starts_with` 锁断言按 S28 定稿形状白名单校准为 contains）

## 4. 文档同步批

- [x] 4.1 **T8〔文档〕** 文档同步——README.md / README.zh-CN.md / rtsql-docs/SKILL.md 按 design D8 七项清单与定稿行为逐条对齐（ORDER BY 限定列、标量子查询边界、无 WHERE 语义声明、SQLSTATE 输出面、FROM 集合操作、恢复收敛话术）；不触碰 spec 语料（随 docs-maintainer 收尾合并）（R8，S30）；依赖 T1–T7 全部行为定稿；以逐条人工核对（对照实测命令输出）验收。（Act 2026-10-06 完成）

## Iteration Plan

### Iteration 000: 计划形状与恢复正确性批

- Tasks: T1, T3, T2, T9（T9 为 2026-10-04 replan 并入——自连接双重缺陷为 T3-S11 阻塞根因，用户裁定方案 (a) 并入修复、Issue 随 T9 解决）
- Depends on: None
- Stable baseline: 二级索引等值扫描任意投影形状/表头正确、EXPLAIN 不崩；JOIN 基座 ORDER BY 限定键全形态可达（自连接形态依赖 T9 解除 S11 阻塞）；崩溃恢复后纯读会话 check 收敛；自连接等值 JOIN 配对与投影归属正确——为子查询批（Iter 001 共享 JOIN 形状层）与写面批提供干净基线。
- Verification boundary: secondary_index/table_alias_resolution/recovery_wal_convergence/self_join 新增测试 + checkpoint_skip/check_command/cli_test/join_test/nested_loop_join_test/join_output_resolution_test/outer_join_test/cross_db_query_test/planner_test 既有套件定向回归 + fmt/clippy。
- Diagnostic boundary: `src/parser/planner/query.rs`（计划形状与 JOIN 装配）、`src/parser/planner/ddl_dml.rs`（JOIN 条件提取）、`src/executor/{plan,join,nested_loop_join}.rs`、`src/executor/plan_format.rs`、`src/pipeline.rs::extract_column_indices` 与执行器构造臂、`src/database.rs`/`src/wal/{recovery,checkpoint}.rs`、`src/cli/lifecycle.rs` 及对应测试文件。
- Non-goals: 子查询面（Iter 001）；写面与 CLI 渲染（Iter 002）；文档（Iter 003）；三表链式 JOIN 修复（范围外既有限制，Issue 候选报告）。
- 平衡审计: 三个任务同属「外部评测正确性红线——代码违反既有承诺」单一主题（崩溃/静默错位、恢复不收敛、JOIN 排序违反 spec），各自改动面小-中（单点条件分支/守卫条件/装配臂），验证面独立但共享 plan-shape + check 套件回归边界；T3 与 T1 同文件相邻（query.rs 计划装配），T2 独立故障域但体量小、单独成 Iteration 过薄；合为一轮形成「红线清零」单一可验收结果。**replan 追加审计（2026-10-04）**：T9 与 T1/T3 同属正确性红线主题、代码面相邻（query.rs 装配 + 条件提取 + 两执行器归属）、验证面共享 join 套件回归边界，且为 T3-S11 验收的必要前提（「该工作是否是达到当前 Iteration 原有 Acceptance 的必要条件」答案为是）；并入后 Iteration 000 仍为单轮可验收结果，体量中-大但边界清晰，不再拆分。

### Iteration 001: 子查询面与 JOIN 装配收口批

- Tasks: T4, T5, T11, T10（T11/T10 为 2026-10-05 replan 并入——Iteration 000 Review 报告的两项范围外 Issue 候选，用户裁定并入解决、不落账独立 Issue；执行序 T4 → T5 → T11 → T10）
- Depends on: Iteration 000（T4 依赖 T3 的 JOIN sort_projection 臂——已交付 accepted；基线 1882 passed / 0 failed / 2 ignored）
- Stable baseline: WHERE 标量子查询比较（非相关+相关）与 FROM 派生表集合操作端到端可用；SELECT 投影位形状与请求一致；≥3 表左深链 JOIN 全形态可达且链上同名列不串列；JOIN 基座纯列清单输出列序 = SELECT 序；两表 JOIN 全形态与子查询/集合既有面零回归。
- Verification boundary: scalar_subquery_comparison/chain_join 新增测试 + set_operations 派生表矩阵 + subquery_test 形状校准 + join_output_resolution/self_join 扩展与校准 + join/nested_loop_join/outer_join/cte/view/cross_db_query/ctas/derived_scan_alias/planner/cli 既有套件定向回归 + 全量。
- Diagnostic boundary: `src/parser/planner/{query,expression,subquery}.rs`、`src/parser/planner/ddl_dml.rs`、新计划节点与执行器、`src/executor/{join,join_config}.rs`、`src/pipeline.rs` 构造臂、对应测试文件。
- Non-goals: IN/EXISTS/标量子查询内集合操作（spec 明示维持拒绝）；JOIN 基座 WHERE 子查询解锁；右深/bushy JOIN 重排；写面与 CLI 渲染（Iter 002）；文档（Iter 003）。
- 平衡审计: 原两任务（T4 中-大、T5 小）同属「子查询能力面收口」，共享 DerivedScan/投影形状测试面。**replan 追加审计（2026-10-05）**：T11/T10 为用户裁定并入的 Issue 修复——两者同属「JOIN 装配面正确性/语义」且共享末步 kept 分支与 join 套件回归边界（T10 依赖 T11 重构面，后置单触）；与 T4/T5 同在 planner 装配层但故障域独立。并入后 Iteration 001 为单轮「子查询面 + 遗留 JOIN Issue 清零」可验收结果，体量大但四任务契约独立、验证面清晰、执行序无交叉触摸；用户已裁定并入 00cyc 不拆分。

### Iteration 002: 写面与 CLI 可观测批

- Tasks: T6, T7
- Depends on: None（与 Iter 001 无依赖；Iteration 顺序按执行序编排）
- Stable baseline: 无 WHERE 全表 UPDATE/DELETE 可用且行级语义/回滚/附加库路由完整；CLI 错误输出机器可读（SQLSTATE）。
- Verification boundary: full_table_dml 新增测试 + readonly_session/cli_test 错误面校准 + cross_db_dml/pg_integration 零回归。
- Diagnostic boundary: `src/parser/planner/ddl_dml.rs`、`src/executor/{update,delete}.rs`、`src/executor/plan.rs` 节点字段、`src/cli/mod.rs` 错误渲染、对应测试文件。
- Non-goals: 一般谓词 UPDATE/DELETE 解锁（维持拒绝）；TRUNCATE 语义变更。
- 平衡审计: 两任务同属「评测反馈的写面与可观测面」——T6 中（新执行形态）、T7 小（渲染附加）；验证面共享 CLI e2e（exit code/错误面/affected_rows）边界，均不改存储格式；合并为一轮「面向使用者的写面与可观测性收口」。

### Iteration 003: 文档同步批

- Tasks: T8
- Depends on: Iteration 000/001/002 全部行为定稿
- Stable baseline: README 双语与 SKILL.md 与 R1–R7 定稿行为逐条一致（design D8 七项清单）。
- Verification boundary: 逐条人工核对（对照实测命令输出），不建判定工具。
- Diagnostic boundary: `README.md`、`README.zh-CN.md`、`rtsql-docs/SKILL.md`。
- Non-goals: spec 语料合并（docs-maintainer 收尾职责）；新增能力声明。
- 平衡审计: 单一文档任务形成「文档与行为一致」稳定结果；前置依赖全部代码批完成，独立成轮避免行为未定稿时返工文档。

## Requirements Traceability Matrix

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| R1 IndexScanAll 列元数据契约 | T1 | secondary_index_test 投影矩阵 + cli_test 表头 | Covered |
| R2 恢复会话 WAL 收敛 | T2 | recovery_wal_convergence_test + checkpoint_skip/check_command 零回归 | Covered |
| R3 JOIN 基座 ORDER BY 限定键 | T3 | table_alias_resolution_test JOIN 排序矩阵 | Covered |
| R9 自连接侧身份正确性 | T9 | self_join_test 矩阵 + table_alias_resolution S11 转 GREEN + join 套件回归 | Covered |
| R10 JOIN 基座输出列序 SELECT 化 | T10 | join_output_resolution_test 扩展 + self_join_test S33 校准 + ctas/derived_scan_alias/cross_db_query/cli 回归 | Covered |
| R11 多表链式 JOIN 装配正确性 | T11 | chain_join_test 矩阵 + join/nested_loop_join/outer_join/self_join/join_output_resolution 零回归 | Covered |
| R4 WHERE 标量子查询 + 投影形状 | T4 | scalar_subquery_comparison_test + subquery_test 校准 | Covered |
| R5 FROM 派生表集合操作 | T5 | set_operations_test 派生表矩阵 | Covered |
| R6 无 WHERE 全表 UPDATE/DELETE | T6 | full_table_dml_test + cross_db_dml_test 回归 | Covered |
| R7 CLI SQLSTATE | T7 | readonly_session/cli_test 错误面 + pg_integration 零回归 | Covered |
| R8 文档一致性 | T8 | 人工逐条核对（S30） | Covered |
