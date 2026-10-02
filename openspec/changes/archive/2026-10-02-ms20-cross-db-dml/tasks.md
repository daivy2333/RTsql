# 任务与逻辑迭代

需求和完整计划已于 2026-10-02 获批，完整计划批准原话：「批准」。复选框表示实施状态。Iteration 000/001/002 均已完成并经 Plan Review accepted（000：第二次审计；001：实施后第三次审计；002：第四次审计）；2026-10-02 用户指令追加 Iteration 002 收口两个实施期发现的 Issue 候选（原话：「额外给出一个iter，我们顺便把issue解决了」，范围选择「两个都修」），R9/R10 增入 proposal、design D6/D7 增补、delta specs 增两处修改。

## 1. 附加库普通写入（关联 MS20-T02）

- [x] 1.1 T1：普通附加 DML 的目标事务、metadata/WAL/缓冲池与回滚路由（R1/R2/R4）；依赖 None；以 cross_db_dml_test 路由/约束/独立提交用例 GREEN 验收。
- [x] 1.2 T2：所有退出路径的附加库关闭与目标提交水位（R5）；依赖 T1；以 fail-fast 重开、关闭失败、Notify 注入信号和目标水位用例 GREEN 验收。
- [x] 1.3 T3：显式事务/只读/DDL/语法边界、写面组合及恢复验证（R1–R5/R8）；依赖 T1/T2；以边界、恢复和当前 Cycle 列出的受影响回归全部 0 failed 验收。

## 2. 查询结果写入（关联 MS20-T02）

- [x] 2.1 T4：INSERT SELECT 查询计划、源表注册、目标列映射与枚举消费面（R7）；依赖 T3 accepted；以 planner/source 路由/空源结构拒绝单测 GREEN 与编译通过验收。
- [x] 2.2 T5：原生物化、单事务写入、冲突动作、失败及主库显式事务（R2/R3/R4/R5/R7）；依赖 T4；以 insert_select_test 行/计数/回滚/重开与既有 SELECT 组合矩阵 GREEN 验收。
- [x] 2.3 T6：README 双语、仓库 rtsql-docs 使用面同步及最终分批回归（R6，全部 requirement）；依赖 T5；以隔离 TempDir 内文档示例运行、逐条内容 Review、fmt/clippy 通过及 M19 全量每批 0 failed 验收。

## 3. 缺陷收口（2026-10-02 用户指令追加，关联 Iteration 000/001 Issue 候选）

- [x] 3.1 T7：两段附加 FROM 项的隐式短名限定引用可达（R9）；依赖 None；以 cross_db_query_test 短名面用例与 planner 隐式登记单测 RED→GREEN 验收。
- [x] 3.2 T8：崩溃恢复 redo 按 WAL logical_id 强制收敛（R10）；依赖 None；以 abort→重插→崩溃恢复用例 RED→GREEN、对照序列与既有恢复套件零回归验收。

## Iteration Plan

### Iteration 000：附加库普通写入与生命周期

- Tasks：T1、T2、T3。
- Depends on：None。
- 阶段结果：VALUES/UPSERT/REPLACE、主键等值 UPDATE/DELETE 正确写附加目标；每文件独立提交；所有退出路径处理附加关闭；显式事务/只读/DDL 边界有测试。
- Stable baseline：读路由不变；目标事务与资源本地一致；重开/恢复、fail-fast 与信号后已提交附加数据保持。
- Verification boundary：新 cross_db_dml_test + attach/query/tx/readonly/memory/busy-timeout/单库写面定向回归 + CLI 信号关闭单测；直接输出和退出码判定。
- Diagnostic boundary：cli/mod.rs 编排/AttachSession、lifecycle.rs 单个 mode 适配、pipeline 根事务与四写构造臂、ddl_dml.rs 三个 DML 拒绝点及对应测试。
- Non-goals：INSERT SELECT、文档最终更新和全量最终 Gate；不扩展 UPDATE/DELETE 语法。
- 平衡审计：T1 的路由、T2 的关闭、T3 的事务边界共同形成可安全持久化的附加写能力，不能把关闭留到后续；以同库错误归属/生命周期为诊断边界。SELECT 源是独立查询计划/物化故障域，拆到 001。
- 当前轮：`iterations/000-initial/000-initial.md`，Review Result=accepted（2026-10-02 第二次审计）。

### Iteration 001：查询结果写入与完整使用说明

- Tasks：T4、T5、T6。
- Depends on：Iteration 000 的 accepted Review。
- 阶段结果：主库/附加目标 INSERT SELECT 完整可用，支持现有 SELECT 与冲突动作；部分提交边界有使用说明，全量回归通过。
- Stable baseline：原生 Value 映射、同表预物化、当前目标单事务、失败还原、主库显式事务与附加目标拒绝正确；可进入最终 Review/收尾。
- Verification boundary：新 insert_select_test + planner/写面/CTAS/跨库与只读受影响回归 + cargo fmt/clippy + M19 全量目标分批串行；不运行性能 bench。
- Diagnostic boundary：ddl_dml/build_insert、query/get_plan_output_columns、pipeline 源表提取/物化/execute 两叶子、plan/mod/plan_format/correlated 穷尽消费、README 与仓库技能文档。
- Non-goals：附加 DDL/CTAS、跨文件原子事务、SQL 面新扩展、存储格式、性能优化。
- 平衡审计：T4 编译与 T5 执行是同一 INSERT SELECT 结果的两层，不能单独当作可用能力。T6 文档/全量是该完整写面验收的一部分，与 001 合并，避免建立只验证或只写文档的过薄迭代。
- 后续 Cycle：无（本 Iteration 已于 2026-10-02 实施后审计 accepted；Cycle 文件 `000-initial.md` 冻结）。

### Iteration 002：缺陷收口（附加短名限定引用 + 恢复重放收敛）

- Tasks：T7、T8。
- Depends on：None（两 task 互相独立；Iteration 000/001 accepted 仅为基线前提）。
- 阶段结果：`SELECT u.tag FROM t JOIN o.u …` 类短名限定列引用全子句面可达，显式别名/纯主库/字面含点行为零变化；「语句部分写入回滚 → 重插 → 崩溃」序列重开恢复成功，重放按 WAL logical_id 收敛，既有恢复矩阵零回归。
- Stable baseline：R9 全子句面短名可达且计划形状与全限定等价；R10 探针序列（Iteration 000 Act Response 记载）RED→GREEN，对照组保持，恢复重跑幂等；全量按 M19 分批 0 failed（收尾轮）。
- Verification boundary：cross_db_query_test 短名面 + planner 单测；新恢复收敛测试 + 既有恢复套件定向；fmt/clippy；M19 全量分批。
- Diagnostic boundary：T7——`src/parser/planner/query.rs` FROM 基表臂与 JOIN 右臂别名注册点、PlanBuilder 附加标记读取；T8——`src/wal/recovery.rs::redo_tuple_at_row_id`、`src/storage/page_format/slotted_page.rs` 落位原语、恢复测试。
- Non-goals：运行期页分配行为与磁盘格式变化、GC 激活、UPDATE/DELETE 语法扩展、三段以上命名空间、性能优化。
- 平衡审计：两项均为本轮 change 实施期发现并已实证的缺陷收口（非新能力），共享「使已批准承诺可达」主题；T7 体量小、T8 体量中，合并避免「单缺陷过薄迭代」，且同为收尾轮共享 M19 最终全量。
- 当前轮：`iterations/002-defect-closeout/000-initial.md`，Review Result=accepted（2026-10-02 第四次审计——用户指令「开始审计」；Gate 2 PASS 记录保留于 Plan Context：2026-10-02 用户批准 Iteration 002 计划，原话「批准」；Plan 独立审计技术面全部 PASS）。Cycle 冻结；Map 无后继 Iteration，change 实施面完成。

## Requirements Traceability Matrix

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| R1 附加普通写面 | T1/T3 | cross_db_dml_test::attached_values_upsert_update_delete_route_and_reopen；attached_defaults_constraints_and_indexes | Covered |
| R2 文件独立提交 | T1/T3/T5 | independent_file_commits_survive_later_error；insert_select_late_constraint_error_rolls_back_target | Covered |
| R3 显式事务边界 | T3/T5 | attached_dml_in_active_tx_rejected_and_main_rolled_back；insert_select_main_explicit_tx_commit_and_rollback | Covered |
| R4 只读/DDL/名字 | T1/T3/T4/T5 | readonly_attached_dml_rejected；existing_attached_ddl_and_ctas_rejections；routing_names_and_zero_match | Covered |
| R5 生命周期/恢复 | T2/T3/T5 | attached_close_after_fail_fast；phase2_signal_closes_attached_databases；attached_routed_writes_crash_recovery；insert_select_target_auto_checkpoint_failure_preserves_commit | Covered |
| R6 使用说明 | T6 | 实际完成的最短 README 示例运行 + 文档逐条 Review | Covered |
| R7 INSERT SELECT | T4/T5 | insert_select_cross_db_join_and_main_target；column_mapping_defaults_and_empty_source；self_source_materializes_once；select_conflict_actions；source_error_no_target_write | Covered |
| R8 既有 UPDATE/DELETE 边界 | T3 | update_delete_subquery_shapes_still_rejected | Covered |
| R9 附加短名限定引用 | T7 | cross_db_query_test 短名 JOIN/单表/显式别名优先/字面含点不误登记 + planner 隐式登记单测 | Covered |
| R10 恢复重放收敛 | T8 | abort→重插→崩溃恢复 RED→GREEN；无 abort / abort 后不重插对照组；恢复重跑幂等；既有恢复套件零回归 | Covered |

R1–R10 全部有已执行见证（R6 为文档示例实跑 + 逐条内容 Review；R8「既有支持形态」经源码和原生命令确认为空，不扩展其原本拒绝面；R9/R10 见证随 Iteration 002 实施完成并经第四次审计 accepted——R9：cross_db_query_test +4 e2e 与 planner +4 单测，RED `2 failed`→GREEN `21 passed`；R10：recovery_replay_convergence_test 3 例 RED→GREEN + 既有恢复套件零回归）。未简化需求。

## Phase / Gate 与验证状态

- CLARIFY completed；Gate 1 PASS，批准原话见 proposal。
- INVESTIGATE completed；定向基线 8 个目标 133 passed/0 failed，3 个原生命令观察既有拒绝，见当前轮事实。
- DESIGN / TASKS / RTM completed；实质未知项 None；非实质局部实现选择由 Act 处理。
- PLAN REVIEW completed；Gate 2 PASS：2026-10-02 用户批准完整计划；三轮 Iteration 的 Cycle 审计均 PASS。
- 结构自检 PASS：proposal/specs/design/tasks 相互对应，T1–T6 唯一归属两轮；只有 000-initial/000-initial.md 已展开；无 TBD/TODO、无 Missing/Simplified，实施框均未勾选，Act/Review 均 pending。`openspec validate 2026-10-02-ms20-cross-db-dml --strict` 输出 `Change '2026-10-02-ms20-cross-db-dml' is valid`，exit 0。初次校验指出旧场景名称遗漏，已保留相应名字并更新行为，复验通过。
- IMPLEMENT completed（T1–T3，Act Response `reported`，含 F1–F3 当前 Cycle 修复）；ACT REVIEW 第二次审计完成（2026-10-02，用户指令「开始审计」）：Review Result `accepted`，Gate 4 PASS，Iteration 000 完成并展开 Iteration 001（`iterations/001-query-result-writes/000-initial.md`）。Iteration 001 IMPLEMENT completed（T4–T6，Act Response `reported` 2026-10-02：M19 全量 13 批 1726 passed / 0 failed / 2 ignored）；第三次审计（2026-10-02，用户指令「实施完了，进行审计」）：Review Result `accepted`，Gate 4 PASS，两个 Iteration 均完成。2026-10-02 用户指令追加 Iteration 002（范围选择「两个都修」）：R9/R10 增入，Map 扩展，`iterations/002-defect-closeout/000-initial.md` 已创建；Gate 2 PASS（2026-10-02 用户批准 Iteration 002 计划，原话「批准」；Plan 独立审计调查事实与产物一致性全部吻合、无 BLOCKED/WAIVED），Status=ready，待 Act 实施 T7/T8。Iteration 002 IMPLEMENT completed（T7/T8，Act Response `reported` 2026-10-02：T7 e2e 21 passed 含 +4、planner 单测 +4；T8 recovery_replay_convergence_test 3 例 + slotted_page 单测 +2；fmt/clippy clean；M19 收尾轮 11 批 + `--lib` + `--bins` 合计 1739 passed / 0 failed / 2 ignored，较 Iteration 001 基线 1726 净增 13）；第四次审计（2026-10-02，用户指令「开始审计」）：Review Result `accepted`，Gate 4 PASS，两个 Minor finding（显式别名 × 裸名同表混用角落语义移位、测试注释场景编号不同轴）记录不返工，Next Iteration: None，全部三个 Iteration 完成；待用户指令收尾（增量规格 6 域合并与 change 归档）。
- Persisted Evidence：none；结果可低成本重跑且 Act Response 足以保存决定性输出，不创建目录。
- SKIPPED：性能 bench（无性能 requirement）、Plan 阶段全量测试（定向基线足够）、经验产物/全局状态/归档（非本技能职责）、后续目录创建（需当前迭代 accepted）。

最终回归按 M19：`--lib`/`--bins` 与全部集成测试目标按名字典序分批，每批约 8–12 个目标，串行独立 `cargo test <明确目标> -- --test-threads=1`；每批 0 failed 方可宣称全量。不要用无目标过滤的全量调用，不创建辅助判定脚本、身份字段或证据协议。
