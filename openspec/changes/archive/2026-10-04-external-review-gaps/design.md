# design — 外部评测反馈批

实现调查于 2026-10-04 完成（本会话三路并行调查 + 复现实测，HEAD 9538647）。全部 file:line 以该 revision 为准，Act 以符号定位为准。

## D1 IndexScanAll 列元数据契约统一（R1/T1）

**根因**：`IndexScanAllNode.columns` 存在两套互斥契约。构造点唯一（`src/parser/planner/query.rs:1824-1843`，二级索引等值路由臂，引入提交 2f83dd9f）：`!sort_due && projection_indices.is_some()` 分支赋 `projection_columns`（已裁剪 SELECT 形状），否则 `base_schema`。消费方两种口径并存：

| 消费方 | 位置 | 假设 |
|---|---|---|
| `get_plan_output_columns`（CLI 表头唯一来源 cli/mod.rs:861；派生/CTE/set-op/聚合输入/CTAS/视图/INSERT SELECT 宽度） | query.rs:95 `projected_columns(&node.columns, &node.projection)` | **基表全列**（再裁剪） |
| EXPLAIN 渲染 | plan_format.rs:124-134 → `columns()` 344-351 | **基表全列**（再裁剪；panic 点 348） |
| 执行器构造 | pipeline.rs:1411-1437（只传 `node.projection`） | 无关（行永远正确） |
| JOIN 侧列映射 `extract_column_indices` | pipeline.rs:2028-2037 | **输出形状**（唯一反向假设；JOIN 侧不经单表 WHERE 路由、实际不可达，经 DerivedScan 间接出现时 DerivedScan 用自身 columns） |

PK 路由 `IndexScanNode` 臂（query.rs:1804-1813）装配同型，但其消费方（query.rs:88-94、plan_format.rs:111-119）持「columns 已收窄、不再裁剪」契约——两契约并存即缺陷。

**决策**：`columns` 恒置 `base_schema.clone().unwrap_or_default()`（删除 query.rs:1836-1840 条件分支）。理由：与 sort_due 分支（今天已是 base_schema）、plan_format.rs:120-121 文档契约、query.rs:95 消费逻辑三方一致；行零变化；顺带修好聚合输入 schema（query.rs:1956，当前会错映射 GROUP BY/聚合列）、CTAS/视图/派生/CTE/INSERT SELECT 宽度各面。善后唯一一处：pipeline.rs:2028-2037 `extract_column_indices` 的 IndexScanAll 臂改为按 `node.projection` 对 `node.columns` 裁剪（对齐 IndexScan 臂口径；实际不可达、风险低）。PK IndexScan 臂与其消费契约**不动**。否决：反向统一为「已收窄」契约（需改全部消费方、波及面大且违背 plan_format 注释口径）。

## D2 恢复会话 WAL 收敛（R2/T2）

**根因**：未提交事务 SIGKILL 后，未提交元组只存在于 WAL 不在数据页。后续纯读读写打开的恢复全程零页写入、零 WAL 追加：`full_recover` 中 `will_redo=false`（recovery.rs:607-609）→ `redo_count=0` → `persist_replayed_tails`/`rebuild_pk_indexes` 全部跳过、`mark_uncommitted_aborted` 扫盘无命中且只读不置脏。close 时零工作守卫（checkpoint.rs:150-156：`get_write_count()==0 && !has_dirty_pages()`）成立 → WAL 原样 → `assert_no_pending_wal`（database.rs:37-54，拒绝一切非 Checkpoint 记录）永久楔死。`RecoveryResult.redo_count` 在 open 消费点（database.rs:331-340）根本未被消费。

**决策（方案 A，最小面）**：`RecoveryResult` 增加 bool（`wal_had_tx_records`：`records` 中存在任一非 `Checkpoint` 记录，recovery.rs:558-559 手握全量 records，三集合非空即等价——marker-only 空事务形态三集合为空但 WAL 仍有 Begin/Commit 记录，故按记录类别判定而非集合）；`CheckpointManager::new` 增参接收（构造点 database.rs:352-362 恰在恢复之后）；零工作守卫改为 `!pending_wal_at_open && get_write_count()==0 && !has_dirty_pages()`。`close()`、`Database::checkpoint`、CLI checkpoint 调用方自动收敛，零签名穿线。lifecycle.rs:226-243 的 zero_work 取样同 OR 该标志（否则消息仍打 "no pending work"）。否决：方案 B（open 内联 checkpoint——把「读写 open 写 WAL」变成新语义、CheckpointManager 构造需前移、次序约束多）；方案 C（close 传 force 标志——Database 加字段两处穿线）。既有测试核对：checkpoint_skip_test S1/S2/S6（marker-free 干净库）语义保持；S5（redo>0 崩溃恢复）本就收敛、零变化；check_command_test `test_check_pending_wal_exit_1_then_recovers` 种子为已提交 INSERT（redo>0）不受影响。无任何测试锁定楔死形态——新增回归为纯增量。

## D3 JOIN 基座 ORDER BY 限定键（R3/T3）

**失败链**：`extract_column_name`（query.rs:3354-3364）解出 `(Some("e"), "id")` → 主 ORDER BY 装配点（query.rs:2319-2364）`qualifier_in_scope` 检查通过后**丢弃限定符**产出裸名键 → 键校验集 `sort_columns = get_plan_output_columns(plan_with_aggregate)`（query.rs:2382）——无 WHERE 的 JOIN 基座 output_columns 已被过滤为 SELECT 清单列（query.rs:617-640，`carry_all_columns` 判定 query.rs:1321 不含 ORDER BY）→ 裸名查不到 → `ColumnNotFound`。带 WHERE 的 JOIN 正常：`carry_all_columns` 置位 → JOIN 承载全组合列 → Sort 经 `where_join_projection` 裁剪（query.rs:1773-1782、2397-2398）。单表正常：基座 Scan/DataScan 输出全 schema，Sort 拥有裁剪权。

**决策（三处修复面）**：
1. `build_from_clause_with_projection` 调用点（query.rs:1318-1323）：`carry_all_columns` 条件加 `|| !query.order_by.is_empty()`（镜像 WHERE 的既有规则）。
2. `sort_projection` 装配（query.rs:2390-2401）：为裸 Join/NLJ 基座（无 WHERE）补一臂，经 `resolve_join_projection_indices`（query.rs:3122-3178）解析 SELECT 裁剪索引。
3. 限定键经布局解析：ORDER BY 限定键在 `join_row_layout`（query.rs:3032-3042）上经 `resolve_layout_index`（query.rs:3048-3078）解析出组合行绝对索引，产出 `OrderByColumn.expr = Some(ColumnExpression { column_index })` 表达式键（SortExecutor decorate-sort-undecorate 通路既有，装配先例 query.rs:2464-2487）；限定符解析复用 `resolve_table_qualifier`/`qualifier_matches_declared`（mod.rs:292-320）。顺带修复自连接同名列裸名 first-hit 局限（MS20 Iteration 001 review 项 1 登记）。
回归面：DISTINCT（query.rs:2434-2438）、集合操作尾 ORDER BY（query.rs:848-871）、聚合基座（query.rs:2328-2344）保持零变化；`get_plan_output_columns` Sort 臂（query.rs:73-85）已按 projection 描述收窄形状，表头自动一致。与 D4 的 JOIN sort_projection 臂共享（T3/T4 同 Iteration 相邻实施，先 T3）。

## D4 WHERE 标量子查询比较 + 投影位形状（R4/T4）

**当前面**：`Expr::Subquery` 在 `build_expression` 无臂（expression.rs:688 兜底 `UnsupportedExpression("EXPRESSION")`）；`try_build_where_subquery`（subquery.rs:19-125）只匹配顶层 InSubquery/Exists；SELECT 投影位由 query.rs:1232-1260 检测 + query.rs:2580-2597 包装（`result_column_index = proj_idx - subqueries_before`），但 query.rs:1685-1686 把含子查询的 `projection_indices` 恒等化（注释声称 "SubqueryEval owns those shapes"，实际只插列不裁剪）→ 全表列泄漏。该异常形状被 tests/subquery_test.rs:481-493 `name_value_map` 固化为参考语义（需校准）。

**关键约束**：`Expression::evaluate_ref` 同步且禁止 `.await`（predicate.rs:62-64）→ 子查询值不能作为表达式变体懒求值；值必须预先落进 `ParameterExpression`（predicate.rs:427-484，`Mutex<Value>` 占位 + `inject_parameters`）。

**决策（统一 Filter 型执行器路线，非相关与相关同一机制）**：
- Planner：WHERE 表达式递归检测含 `Expr::Subquery`（新 helper，镜像 `contains_or`）→ 命中则整个谓词**禁止下推**（镜像 contains_or 臂 query.rs:1874 模式），装配新计划节点 `ScalarSubqueryFilterNode { input, predicate, bindings: Vec<(String /*param 名*/, PhysicalPlan)> }`；谓词编译时把 Subquery 腿替换为 `ParameterExpression` 占位并登记 binding（子查询经 `building_subquery` 上下文 `build_query` 编译，同 query.rs:1247-1257 模式）；`extract_correlated_params`（subquery.rs:156-172）判相关并登记外层参数。
- Executor：新 `ScalarSubqueryFilterExecutor`——逐行：提取参数值（非相关为空序列）→ 语句级缓存（键 = 参数值序列，镜像 SubqueryEvalExecutor subquery_eval.rs:131-193 与 SemiJoinExecutorV2 semi_join.rs:209-267 的既有机制）→ miss 时 `plan.clone()` + `inject_correlated_values`（correlated.rs:14-118）+ 重建执行器 + drain 取首行首列（多行 → `SubqueryReturnsMultipleRow`、零行 → NULL，逻辑自 subquery_eval.rs:78-109 摘出复用）→ `set_parameter_value` 注入占位 → 对当前行求值谓词 → 匹配输出/过滤。非相关形态参数序列恒空 → 单缓存项 → 每语句一次求值，天然高效。快照/跨库路由/多行错误语义照搬 SubqueryEval 既有臂（pipeline.rs:1761-1782 构造臂先例）。
- 否决：a) 表达式层 SubqueryEval 变体（异步不可行）；b) 改写为与单行 DerivedScan 的 JOIN（空结果/多行基数/JOIN 形状裁剪均是新坑，且恰在 D3 的薄弱层上，invasive）。
- 投影位形状：query.rs:1685-1686 摘出 `!subquery_evals.is_empty()`（纯列清单时走既有 `resolve_projection_indices` 裁剪，`projection_columns` 现成）；`result_column_index` 公式（query.rs:2589）裁剪生效后恰好正确，无需改；JOIN 基座 `filter_projection` 同门（query.rs:1773-1781）与 D3 的 sort_projection 臂共享；tests/subquery_test.rs:481-493 参考形状见证按新形状校准（白名单）。组合面已收窄利于修复：DISTINCT×子查询拒绝（query.rs:1265-1269）、混合聚合×子查询拒绝（query.rs:1562-1570）、表达式项×子查询拒绝（query.rs:1594-1602）保持。
- JOIN 基座 WHERE 内标量子查询：维持显式拒绝（query.rs:1739-1746 同型文案点名），不加能力。

## D5 FROM 派生表内集合操作（R5/T5）

拒绝点在 planner（query.rs:828-832 `building_subquery` 守卫）。`building_subquery` 是单 bool，三种置位来源（FROM 派生 query.rs:401-410 / 视图展开 query.rs:1139-1142 / 表达式子查询 query.rs:1247-1257 与 subquery.rs:36、85）。

**决策**：拆「FROM 派生上下文」标志——仅 FROM 派生表上下文放行 `build_set_operation`；视图展开与表达式子查询（IN/EXISTS/标量）维持拒绝。全链其余零改动：`SetOperationNode.columns`（= 左臂列名，plan.rs:776-789）→ `get_plan_output_columns`（query.rs:134-135）→ DerivedScan 包裹（query.rs:398-429，物化行宽 = 左臂宽自洽）→ `SetOperationExecutor`（set_operation.rs:24+，对臂计划形状零假设）→ plan_format/correlated 穷尽臂已有（plan_format.rs:64-67/328-337、correlated.rs:103-108）。子查询内 DISTINCT 的既有零消费行为（query.rs:1200-1218）与 UNION 去重正交，保持。括号化操作数拒绝（query.rs:997-999）保持。`get_subquery_first_column` 不加 SetOperation 臂（IN/EXISTS 内维持拒绝的范围收口）。

## D6 无 WHERE 全表 UPDATE/DELETE（R6/T6）

**实现事实**：`UpdateNode { table_name, key: Key, assignments }`（plan.rs:306-313）、`DeleteNode { table_name, key: Key }`（plan.rs:317-322）——`key` 必填单键；执行器单点操作（update.rs:43-100 search 单点写版本、delete.rs:14-115 单点墓碑）；拒绝门 `build_update` ddl_dml.rs:1852-1859、`build_delete` :1962-1969（MissingField → `extract_pk_from_where` PK 等值二连门）。SELECT 侧「无 WHERE → DataScan」通路未接 DML。

**决策**：
- 计划面：`UpdateNode.key`/`DeleteNode.key` 改 `Option<Key>`（None = 全表）；`build_update`/`build_delete` 的第一重门（MissingField）移除——无 WHERE 时 key=None 走全表形态；**第二重门保留**：带 WHERE 但非 PK 等值的既有拒绝语义零变化（范围收口：本 change 不解锁一般谓词 UPDATE/DELETE，`UPDATE ... WHERE v > 5` 仍拒绝——拒绝文案维持点名）。
- 执行面：`UpdateExecutor`/`DeleteExecutor` 扩展全表臂——`IndexManager::scan_all() -> Vec<(Vec<u8>, RowId)>`（index_manager.rs:339，现成原语）枚举 → 逐行复用既有单行逻辑（UPDATE：旧行快照求值全部右值 → per 行门序 coerce→类型门 FLOAT 升格→NOT NULL→键列类型门 + rekey 碰撞预检 → 版本写 + 唯一/二级索引维护；DELETE：墓碑 + 索引清理）→ 汇总 `AffectedRows(实际行数)`（替换单行形态硬编码的 1/0）。
- 语义边界：语句中途失败（任一行碰撞预检/类型门命中）→ 隐式事务回滚全部已写行（既有 DML 事务包裹 pipeline.rs 语义，零新增机制）；显式事务内可用、可手动 ROLLBACK（与 TRUNCATE 的 in-tx 拒绝 + checkpoint-first 物理释放语义形成对照，二者并存）；MVCC 可见性按枚举时点快照（与单行语义同源）。附加库目标经既有 routed 通道（pipeline.rs:1472-1497 构造臂路由绑定），无 WHERE 形态自然继承；cross-db-dml spec「UPDATE DELETE 保持已有语法边界」MODIFIED（「主键等值条件」→「主键等值条件或省略 WHERE 的全表形态」；子查询拒绝面保持）。
- 测试面：无文案断言锁定（全仓 grep 无 "Missing required field" 测试）；`tests/subquery_test.rs:262/:423` 两处 `DELETE FROM emp` 返回值被丢弃、当前实际被拒绝的静默失败用例，放开后成为真删除（与本用例注释意图一致，验证即可）；`tests/cross_db_dml_test.rs:1121` 子查询拒绝面簇不受影响（第二重门）。

## D7 SQLSTATE CLI 渲染（R7/T7）

`Response::Error` 已携带可选 `sqlstate`（MS19 R2：0A000/42601/42P01/42703/42702/58000 兜底 + 25006）。CLI 侧未渲染。

**决策**：文本格式——`sql_failure_status`/render 错误路径在 `sqlstate` 存在时行尾附加 ` [SQLSTATE xxxx]`；JSON 格式——错误对象增加 `"sqlstate"` 字段（None 时省略字段）；退出码与既有文案零变化；PG 协议面零变化（pg_integration 断言不涉及 CLI 渲染）。注意 MS19 曾约束「JSON 序列化字节不变」——该约束是当时的零回归护栏，本 change 显式放宽（新字段），受影响 JSON 错误断言用例按白名单校准（readonly_session_test/cli_test 错误面用例）。

> **Review 勘误（2026-10-06，Iteration 001 Review / Iteration 002 展开时）**：本节「JSON 错误对象增加字段」以既有 JSON 错误发射为前提，失实——CLI 现无 JSON 错误对象（错误恒经 `run()` → `emit_stderr` 纯文本；`QueryPayload` 仅 Rows/Affected）。T7 的 JSON 错误对象为**新增发射行为**：通道（stderr）、形状（`{"error":"…","sqlstate":"…"}` 扁平单行）与省略规则在 `iterations/002-write-cli/000-initial.md` T7 Task Contract 定稿。

## D8 文档同步（R8/T8，文档面）

统一在行为定稿后一轮完成（README.md + README.zh-CN.md + rtsql-docs/SKILL.md 三处，双语逐条对齐）：
1. ORDER BY 限定列：SKILL.md:191/249 声明随 R3 成真，补 JOIN 基座说明（无 WHERE 形态已覆盖）。
2. 标量子查询：SKILL.md:249「scalar and IN subqueries, correlated included」随 R4 成真；README.md:253 同步；补「多行拒绝、空结果 NULL 语义、JOIN WHERE 维持拒绝」边界。
3. UPDATE/DELETE：README.md:282（双语）/SKILL.md:251 示例随 R6 成真；写面行显式声明「无 WHERE = 全表，行级语义，TRUNCATE 仍为物理释放替代」。
4. SQLSTATE：README.md:192（双语）/SKILL.md:141/437 随 R7 成真；补 CLI 输出面说明（文本后缀 + JSON 字段）。
5. FROM 集合操作：SKILL.md:249 Read 行补「FROM 派生表内集合操作可用；IN/EXISTS 内维持拒绝」。
6. 恢复收敛：SKILL.md:156/441、README.md:180 话术随 R2 成真；补「纯读恢复会话同样收敛」一句。
7. 不改 spec 语料（spec delta 随 docs-maintainer 收尾合并）；不新增身份型证据或验证工具。

## D9 自连接侧身份正确性（R9/T9，replan 2026-10-04）

**根因（单句）**：JOIN 装配层的侧身份以真实表名承载——ON 交换判定比较归一真实名（`ColumnRef.table` 携带 `resolve_table_qualifier` 归一键）、执行器输出归属比较 `OutputColumn.table_alias`（构造恒真实名）与链首名——自连接两侧真实名相同使两类判定全部失效。

**修复臂 1（交换判定按声明限定符归属）**：`extract_join_conditions`（`ddl_dml.rs`）签名平行增参 `left_qualifiers: &[String]`（链声明限定符）与 `right_qualifier: &str`（调用点 `query.rs` JOIN 臂传 `current_qualifiers`/`right_qualifier`，AND 递归臂透传）。每腿侧别判定（接受面 `is_pure_equi_join_on` 不变，腿形仅裸名/两段）：限定腿按书写限定符与声明限定符匹配（`qualifier_matches_declared` 同款：别名原文相等 / 归一命中 / 隐式短名），命中左链 → 左侧、命中右 → 右侧、均未命中 → `TableNotFound(qualifier)` 与 `resolve_qualified_column` 未知限定符同面；裸名腿按位置判定（左腿即左侧，既有按侧解析语义不变）。两腿异侧 → 按侧装配 `JoinCondition`（`left_column` = 左侧腿、`right_column` = 右侧腿，书写序无关）；两腿同侧 → `InvalidQuery` 新点名拒绝零副作用。`JoinCondition` 内容语义不变（执行器按列名从侧索引映射取键）。列解析仍经既有 `resolve_column_ref`（`ColumnRef.table` 真实名不变，链首命名与键提取不受影响）。**非自连接零变化论证**：无别名臂的声明限定符即真实表名，逐形态判定结果与现状一致（正向不交换/反向交换/裸名位置判定）。

**修复臂 2（输出归属按显式侧标志）**：`plan.rs` 新 `JoinSide { Left, Right }`；`JoinNode`/`NestedLoopJoinNode` 加性字段 `output_sides: Vec<JoinSide>`（与 `output_columns` 同序同长，**执行器输入项语境**：`Left` = 从执行器左输入行取值）。`query.rs` `all_columns` 构造点同步产出（左段列 = Left、右臂列 = Right，原始表序语境）并随 SELECT 裁剪同步裁剪；Hash 臂直用；NLJ RIGHT 交换臂按输入交换反转（Left↔Right），`output_columns` 序不变（既有注释语义保持）。两执行器 `build_output_row`/`build_padded_row` 改按 `output_sides[i]` 取行；`JoinExecutor.left_table_name`/`JoinConfig.left_table_name` 与 NLJ 执行器 `left_table_name` 参数删除（归属唯一消费点消失；`JoinConfig.right_table_name` 现状 dead-by-underscore 非本 change 产物，不动）；pipeline 两构造臂机械适配（`extract_column_indices` 对左输入的调用保留以维持嵌套链形态既有错误传播面，名字结果弃置）；`tests/join_test.rs` 7 处 `JoinConfig` 与 `tests/nested_loop_join_test.rs` 1 处节点字面构造机械适配。

**否决路线**：(i) `OutputColumn.table_alias` 改声明限定符——语义波及 `extract_column_indices` NLJ 臂链首名与全部真实名消费面，blast radius 大；(ii) 交换判定改为不归一的写限定符穿透——破坏 D4 单点归一化纪律，三段/复合键形态回归。选择理由：加性字段 + 判定点修复，非自连接行为逐形态可论证零变化。

**已知边界（2026-10-05 replan 起改为本 change 修复目标，见 D11）**：三表链式 JOIN（单 FROM ≥3 表因子）当前版本全部形态已坏——hash+hash 链 `Column not found` 计划失败、hash+NLJ 混合链同、仅 SELECT 前两表形态 `join.rs:100` panic（replan 调查实测，HEAD 9538647 与本 change 代码同）；T9 不触及链式装配，错误面在 T9 交付时保持不变。

## D10 JOIN 基座输出列序 SELECT 化（R10/T10，replan 2026-10-05）

**现状（两条表序路径）**：① 裸纯列清单的 `kept` 分支（query.rs 装配循环内）按表序迭代 `all_columns`、以 `any()` 匹配 SELECT 清单——输出（`output_columns`/`output_qualifiers`/`output_sides`）恒表序；② `resolve_join_projection_indices`（query.rs:3263 起）按 combined 表序过滤收集命中索引——WHERE 路径（`filter_projection`/`where_join_projection`）与 ORDER BY 路径（`sort_projection`）的裁剪序同为表序。对照：Scan 基座 `resolve_projection_indices` 按 SELECT 序（I034 语义）；JOIN 基座表达式项走顶层 `ProjectionExecutor` 本就 SELECT 序。表头（`get_plan_output_columns`）与行（`build_output_row`）同源于 `output_columns`，自洽但与标准 SELECT 序不符。

**决策（单点语义：JOIN 基座纯列清单输出 = SELECT 序）**：

- ① `kept` 重建：改按 `qualified_columns` 清单序迭代，每项在 `all_columns` 上唯一命中（MS25 T8 严格校验已保证存在/唯一/歧义点名）后 push `(col, qualifier, side)` 三清单同步；重复项（`SELECT e.ename, e.ename`）重复产出（对齐 SELECT 语义）。
- ② `resolve_join_projection_indices` 返回序 SELECT 化：改按 `qualified_columns` 迭代、逐项定位 combined 命中索引（命中 0 → 既有 `ColumnNotFound`；裸名多命中 → 既有 `AmbiguousColumn`）；重复项重复索引。WHERE/ORDER BY 两裁剪路径随之 SELECT 序（Sort/Filter 承载裁剪的既有机制零改动）。
- 范围收口：纯列清单且无子查询项（子查询项的投影交互归 T4 面）；`SELECT *` 与 carry-all 全组合承载（聚合/派生/CTAS 输入、Sort 承载全列）不变；表达式项路径零变化。消费面自然跟随：`get_plan_output_columns` 表头 → CLI 渲染、CTAS 列名、INSERT SELECT 映射、DerivedScan 列清单（CTAS 列名随输出序 = 与 Scan 基座 CTAS 一致）。
- 否决：给 JOIN 节点统一加顶层 Projection 包装（改 plan 形状、EXPLAIN 面、两表性能路径，blast radius 大）；仅改 kept 分支不改裁剪函数（WHERE/ORDER BY 路径残留表序，语义分裂）。
- 校准白名单预期：`tests/self_join_test.rs` S33 第二形态（三列混排断言从表序翻转为 SELECT 序 + 注记更新）；`join_output_resolution`/`derived_scan_alias`/`ctas`/`cross_db_query`/`cli_test` 若有表序断言命中，白名单校准记 Act Response。

## D11 多表链式 JOIN 装配正确性（R11/T11，replan 2026-10-05）

**根因三联（调查实测 + 代码闭合）**：① 装配循环每步把「当前迭代组合形状」当全作用域——第三表列在第一步的严格校验即 `ColumnNotFound`（hash+hash/hash+NLJ 计划失败根因）；中间步 `kept` 裁剪使左输入行形状 ≠ 下一步 `all_columns` 全宽假设。② 链式左输入的 `column_index` 为 within-table 语义，`build_output_row`/`build_padded_row` 以 `left_row[within-table index]` 寻址——链列读串行。③ `JoinConfig`/执行器条件键按裸列名查表（`HashMap<String, usize>` + `.expect`）——链上同名列静默串列、缺列 panic（`join.rs` `build_hash_key_*`；NLJ 谓词路径经 `join_column_layout` 绝对偏移编译，本就 index-based 不受 ③ 影响）。

**决策（左深链不变量：每个 JOIN 步的左输入行恒为其链前缀全组合行）**：

- a) 循环重构：非末步强制全组合承载（跳过 `kept` 与严格校验——中间步输出 = 链前缀全组合列）；末步按查询标志现行逻辑（`carry_all_columns`/`select_star`/kept——此时形状 = 全作用域，校验与裁剪正确）。
- b) 索引重定基：迭代 ≥2 时链前缀列（`current_tables` 各臂）的 `column_index` 重定为左输入行绝对偏移（累计宽度），右臂列保持 within-table；`output_sides`（T9 D9）机制不变（链列 Left、右臂 Right）——`build_output_row` 按（side, index）寻址即正确。两表单步（链长 1）无重定基，逐字节不变。
- c) 条件键限定解析：`JoinExecutor` 的 `build_hash_key_left/right` 查表改「`table.column`（真实名小写）优先、裸 `column` 回退」；pipeline `Join` 构造臂为 `left/right_column_indices` 增注册限定键（裸键仅在输入内无歧义时注册；Scan 单表输入裸=限定同键）；`join_test` 直构 `JoinConfig`（裸键 map + `ColumnRef{table: …}`）经回退臂零适配。T9 侧别装配产出的 `ColumnRef.table`（真实名）恰为限定键供源。
- d) NLJ 链零新机制：谓词经 `join_column_layout` 绝对偏移编译——a+b 后左输入行 = 链前缀全组合行，`left_row ++ right_row` 与布局偏移天然对齐。
- e) 可观察面：中间节点 EXPLAIN 形状变化仅发生于此前必然计划失败的链形态（零既有断言波及）；两表路径（含 `SELECT *`/WHERE/ORDER BY/聚合/直构）逐字节零回归；真未知列错误面不变。
- 否决：右深/bushy 重排（代价模型域 I016）；`JoinCondition` 键类型改 `(table, column)` 结构化重构 executor 接口（blast radius 大——qualified-first 查表等效且接口零变化）；中间步 kept + 重放裁剪（两次寻址叠加复杂度高于全组合承载）。

## 实现顺序与依赖

T1 → T3 → T9（replan 增入：T3 的 S11 验收依赖 T9 的执行器归属修复，相邻装配先形状后身份）→ T4（T3/T4 共享 JOIN sort_projection 臂）；T2、T5、T6、T7 相互独立。Iteration 划分见 tasks.md。**Iteration 001 执行序（2026-10-05 replan）：T4 → T5 → T11 → T10**——T4 依赖 T3 交付面（已就位）；T11 重构装配循环的末步 kept 分支，T10 的 SELECT 序重建恰在该分支上，故 T10 依赖 T11、后置避免同面双触；T4/T5 与 T11/T10 分属子查询与 JOIN 装配两故障域，批内顺序执行、验证面独立。全部任务不改存储格式、不改网络协议字节（除 T7 显式放宽的 JSON 错误对象）、不建身份型证据工程。
