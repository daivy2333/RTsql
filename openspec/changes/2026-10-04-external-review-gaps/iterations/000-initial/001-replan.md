# Iteration 000 / Cycle 001: 自连接侧身份正确性（replan）

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T9（自连接侧身份正确性，replan 2026-10-04 并入）；T3 的 S11 验收缺口随本 Cycle 关闭（解除 `#[ignore]` 转GREEN，并入 T9 Acceptance）
- Acceptance gaps: R3-S11（父 Cycle `000-initial` Blocker Handoff——自连接 ORDER BY 限定列在该缺陷修复前不可观察）
- Repair items: None（replan 使用更新后的全局 task T9，不建 repair item）
- Inherited scope: proposal R1–R9 与场景 S1–S37；design D1–D9；父 Cycle 已交付且 Review 采信的部分——T1 全部、T2 全部、T3 实现 + S10/S12 见证（Iteration 000 Plan Context 与 Act Response 为基线，本文自包含执行所需事实，Act 不回读父 Cycle）
- Excluded scope: 子查询面（T4/T5，Iteration 001）；写面与 SQLSTATE（T6/T7，Iteration 002）；文档（T8，Iteration 003）；三表链式 JOIN 修复（范围外既有限制，Issue 候选报告，见 Non-goals）

**Objective**

自连接等值 JOIN 全形态正确：ON 条件配对按声明限定符归属（正向/反序书写同结果）、投影与外连接补齐行按显式侧标志归属各读对应侧、双腿同侧计划期点名拒绝；非自连接全部形态逐字节零回归。完成后父 Cycle 的 R3-S11 解除阻塞（`#[ignore]` 移除转 GREEN），Iteration 000 Acceptance（S1–S12 + S31–S37）闭合。

**Background**

父 Cycle Act 阶段 T3 的 S11 用例（自连接 ORDER BY 限定列）暴露两处范围外既有缺陷：①Hash 等值 JOIN 的 ON 交换判定以归一真实表名比较（自连接两腿恒真）→ 行配对错误；②Hash 与 NLJ 执行器输出行归属按真实表名比较（自连接无法区分两侧）→ 投影静默读错表。Plan Review（2026-10-04）独立核实（代码路径 + 改动前 release 二进制行为复现，证据逐字节一致）后按 Blocker Handoff 给出三选一；用户裁定（原话「同意，给出replan，把isuee也解决」）：方案 (a)——replan 并入本 change 修复（R9/T9），Issue 随 T9 交付解决，T3-S11 用例解除 ignore 并入 T9 验收。本 Cycle 即该裁定的执行契约。

**Investigation Facts**

- Current Baseline: 工作区含父 Cycle 未提交改动（T1/T2/T3 已交付）；测试现状 1875 passed / 0 failed / 3 ignored（父 Cycle Act Response 全量 Gate 采信，Plan Review 复核工作区未变化）；S11 用例 `#[ignore]` 在案。自连接缺陷在 HEAD 9538647 与当前工作区同成立（非本 change 引入）。
- Current-State Evidence:
  - **注册模型**：`PlanBuilder.tables` 以真实表名（小写）为键；别名经 `table_aliases: HashMap<alias, real_key>`（`mod.rs:219`）映射，`resolve_table_qualifier`（mod.rs:292-294）单点归一并**抹除别名身份**；声明限定符（别名优先否则真实名）以 `current_qualifiers`（左链，每臂一条）与 `right_qualifier`（右臂）形态存在于 `build_from_clause_with_projection`（query.rs:335 起）——自连接两侧的声明限定符天然唯一（`e1`/`e2` 各占一段），这是本修复的身份基础。`qualifier_matches_declared`（mod.rs:315-320）：原文相等或归一命中（隐式短名）。
  - **缺陷①链（条件交换）**：JOIN 循环（query.rs:651）调 `extract_join_conditions(&current_tables, &right_table, on_clause)`（ddl_dml.rs:175-226，只收真实名）。腿解析 `resolve_column_ref`（expression.rs:24-47）：两段腿经 `resolve_qualified_column`（:87-105）把书写限定符归一为真实键填入 `ColumnRef.table`（注释明示「ON 腿左右交换判定按真实名工作」）；裸名腿在传入侧的表清单内搜索（位置语义）。交换判定 `left_ref.table.as_deref() == Some(right_table)`（ddl_dml.rs:205-212）——自连接两腿归一同名恒真，左腿先写形态恒误判交换。`JoinCondition { left_column, right_column }`（plan.rs:527-532）的执行消费只按**列名**从侧索引映射取键（join.rs:80-100 `right_column_indices.get(&cond.right_column.column)`），`ColumnRef.table` 不参与执行。接受面 `is_pure_equi_join_on`（ddl_dml.rs:113-117）：腿仅裸名/两段（三段走 NLJ）。AND 链在 :188-189 递归。
  - **缺陷②链（输出归属）**：`all_columns` 构造（query.rs:525-553）`OutputColumn { table_alias: 真实表名, column_index: 侧内索引 }`（D4 注释明示「OutputColumn 的真实表名字段不变」，计划期归属走并行的 `all_qualifiers`）。执行器侧归属却用 `table_alias`：`join.rs::build_output_row`（:112-122）与 `nested_loop_join.rs::build_output_row`（:79-102）均 `col.table_alias == self.left_table_name`，`build_padded_row`（nested_loop_join.rs，外连接补齐）同型——自连接全部列恒读左行。`left_table_name` 来源：`JoinConfig`（pipeline.rs:1595-1632，`extract_column_indices(&node.left)?.1` = 首条件 `left_column.table`）与 NLJ 构造（pipeline.rs:1634-1668，`extract_column_indices` NLJ 臂 = 首 output column `table_alias`，pipeline.rs:1940-1956）。
  - **NLJ 谓词路径自连接已正确**（基线实测）：非等值/外连接 ON 经 `join_column_layout`（声明限定符分段，query.rs:692-710）编译，`resolve_layout_index`（query.rs:3142-3178）按声明段键匹配——自连接两别名各占一段，谓词配对正确；实测 `LEFT JOIN` 行多重性正确、仅归属错位（缺陷②）。RIGHT 交换：布局按交换后输入序、`output_columns` 按原表序（query.rs:714-723 注释），归属「与输入侧无关」依赖真实名比较——同被缺陷②覆盖。
  - **构造点 census**：`JoinNode`/`NestedLoopJoinNode` 字面构造仅 query.rs:654（Hash）与 query.rs:724/732（NLJ 两分支）；执行器构造 pipeline.rs:1622（`JoinConfig`）与 pipeline.rs:1658（`NestedLoopJoinExecutor::new`）；测试直接构造 `tests/join_test.rs` 7 处 `JoinConfig`、`tests/nested_loop_join_test.rs:297` 1 处节点字面量。`get_plan_output_columns` Join/NLJ 臂与 `plan_format.rs` 只消费 `output_columns` 名字面，加性字段零影响。
  - **三表链式 JOIN 既有边界**（范围外，不修）：单 FROM ≥3 表因子全部形态已坏——hash+hash 链 `Column not found: vc` 计划失败、hash+NLJ 混合链同、仅 SELECT 前两表形态 `join.rs:100` panic（replan 调查实测，HEAD 9538647 与工作区同）；本 Cycle 不触及链式装配，错误面不变。
- Code and Critical Path:
  - 修复臂 1（交换判定）：`extract_join_conditions` 签名平行增参 `left_qualifiers: &[String]` + `right_qualifier: &str`（调用点 query.rs:651 传 `&current_qualifiers`/`&right_qualifier`；AND 递归臂 ddl_dml.rs:188-189 透传）。腿侧别：限定腿按书写限定符 `qualifier_matches_declared` 依次匹配左链各臂 → 左侧；匹配 `right_qualifier` → 右侧；均未命中 → `TableNotFound(qualifier)`；裸名腿按位置（左腿左、右腿右）。两腿异侧 → 按侧装配（`left_column` = 左侧腿）；同侧 → `PlanError::InvalidQuery` 点名。`ColumnRef` 仍经 `resolve_column_ref` 解析（真实名不变）。
  - 修复臂 2（输出归属）：`plan.rs` 新 `JoinSide { Left, Right }`（derive Debug/Clone/Copy/PartialEq/Eq）；`JoinNode`/`NestedLoopJoinNode` 加性 `pub output_sides: Vec<JoinSide>`（与 `output_columns` 同序同长，执行器输入项语境）。`all_columns` 构造点同步产出 sides（左段 Left、右臂 Right）并随 SELECT 裁剪同步裁剪（`kept` 分支）；Hash 臂直用；NLJ RIGHT 交换臂按输入交换反转（Left↔Right）。`join.rs`/`nested_loop_join.rs` 的 `build_output_row`/`build_padded_row` 改按 `output_sides[i]` 取行；`JoinExecutor.left_table_name`/`JoinConfig.left_table_name` 与 NLJ 执行器 `left_table_name` 参数删除（`JoinConfig.right_table_name` 现状 dead-by-underscore 非本 change 产物，不动）；pipeline 两构造臂机械适配（`extract_column_indices` 对左输入调用保留——嵌套链形态既有错误传播面，名字结果弃置）。

**Implementation Guidance**

顺序：臂 1（ddl_dml.rs 条件判定 + query.rs 调用点）→ 臂 2（plan.rs 字段 + query.rs 装配 + 两执行器 + pipeline + 测试机械适配）→ S11 解除 ignore。测试先行：`tests/self_join_test.rs` 矩阵按 S31–S36 先写 RED（当前缺陷确定性复现：S31 得 `[["r1","r1"],…]`）；S35 的拒绝面为新增行为，RED = 当前静默误判执行。机械适配（JoinConfig 7 处 + 节点字面量 1 处）以编译错误为引导，断言零变化。聚合查询的 Sort 在聚合输出行（父 Cycle 偏差 1 的门）与本修复无交互——T9 只改 JOIN 节点归属与条件判定，不动 ORDER BY 装配。`InvalidQuery` 文案点名能力（如 `JOIN ON condition must reference both sides`），映射既有 42601 类。

**Task Contracts**

### T9: 自连接等值 JOIN 配对与投影归属正确

- Requirement/Scenario: R9 / S31–S37（含 T3-S11 解除阻塞）
- Depends on: None（装配面与 T3 相邻；S11 解除依赖本任务）
- Targets: `src/parser/planner/ddl_dml.rs::extract_join_conditions`（+AND 递归臂）、`src/parser/planner/query.rs` JOIN 臂调用点与 `all_columns`/sides 装配（含 RIGHT 交换反转）、`src/executor/plan.rs`（`JoinSide` + 两节点加性字段）、`src/executor/join.rs`（JoinExecutor 归属 + JoinConfig）、`src/executor/nested_loop_join.rs`（归属 + 签名）、`src/pipeline.rs` 两执行器构造臂、`tests/join_test.rs`/`tests/nested_loop_join_test.rs` 机械适配
- Current behavior: S31 `[["r1","r1"],["r2","r2"]]`、S32 `[["m1","m1"],["m2","m2"]]`、S34 全部 `[x,x]`、S35 静默误判执行（全部实测，HEAD 9538647 二进制）；S11 用例 `#[ignore]`。
- Required behavior: S31/S32 同结果 `[["m1","r1"],["m2","r2"]]`；S33 任意投影组合各读对应侧；S34 `[["m1","r1"],["m2","r2"],["r1",NULL],["r2",NULL]]`（FULL/非等值形态同归属语义）；S35 计划期 `InvalidQuery` 点名零副作用；S36 非自连接矩阵逐字节零变化；S37 S11 解除 ignore 转 GREEN。
- Preserve: 非自连接全部形态行为逐字节（判定结果论证见 design D9）；`is_pure_equi_join_on` 接受面；`JoinCondition` 执行语义（按列名取键）；`ColumnRef.table` 真实名语义与 `extract_column_indices` 各臂返回值语义（NLJ 臂链首名输出保留）；NLJ RIGHT 交换的 `output_columns` 原表序注释语义；三表链式 JOIN 既有失败面（错误形态不变）；聚合查询 ORDER BY 限定键既有裸名键路径（父 Cycle 偏差 1 的门）。
- Forbidden: 不得改 `OutputColumn.table_alias` 语义（D9 否决路线 i）；不得改为不归一的写限定符穿透（否决路线 ii）；不得触碰三表链式装配逻辑；不得改 `JoinConfig.right_table_name`（现状 dead，非本 change 产物）；不得扩大到 IN/EXISTS/SemiJoin/AntiJoin 通道（半连接只输出左列，归属语义不同面）。
- Test witness: 新 `tests/self_join_test.rs`（S31–S36 矩阵，lib `execute_sql` 断言精确行值；S35 断言错误面与零副作用）——先写 RED；S37 以移除 `tests/table_alias_resolution_test.rs::self_join_same_name_columns_resolve_by_qualifier` 的 `#[ignore]` 属性转 GREEN 为见证。
- GREEN condition: 新矩阵全绿 + S11 GREEN + join_test/nested_loop_join_test/join_output_resolution_test/outer_join_test/cross_db_query_test/planner_test/table_alias_resolution_test 零回归。
- Verification: `cargo test --test self_join_test --test table_alias_resolution_test --test join_test --test nested_loop_join_test --test join_output_resolution_test --test outer_join_test` 决定性输出 + 全量 `cargo test` 0 failed（预期 1875 − 1 ignore 转正 + 净增新用例）+ `cargo fmt --check`/`cargo clippy --tests` 干净。
- Stop when: 修复引发非自连接形态行为变化（说明侧身份理解有误）；或三段/复合键腿形态进入 Hash 路径（说明接受面理解有误）→ 返回 Plan。

**Invariants**

- 存储格式、网络协议字节、plan cache key 语义零变化。
- 全部既有 spec 语料行为零回归（本 replan 的 delta 为 `sql-table-alias-resolution` ADDED Requirement）。
- 不建身份型证据工程/判定层；验证判定使用项目测试框架原生退出码与输出。
- T1/T2/T3 父 Cycle 已交付面（IndexScanAll 契约、WAL 收敛标志链、ORDER BY 布局键装配）不在本 Cycle 变更面内。

**Non-goals**

- 三表链式 JOIN 修复（范围外既有限制——replan 调查实测全部形态已坏；作为独立 Issue 候选报告，待用户裁定处置，不落账）。
- SemiJoin/AntiJoin 归属语义（只输出左列，不同面）；嵌套 JOIN 列映射重构（链式边界的一部分）。
- 子查询面、写面、SQLSTATE、文档（后续 Iteration 不变）。

**Acceptance**

- S31–S37（proposal 场景草图）全部可观察通过；R9 的 spec delta 场景与 T9 测试一一对应（RTM 见 tasks.md）；父 Cycle R3-S11 缺口关闭。
- 全量 `cargo test` 0 failed；`cargo fmt --check` 与 `cargo clippy --tests` 干净。

**Verification**

- 直接目标：`cargo test --test self_join_test`（新增矩阵 RED→GREEN）+ `cargo test --test table_alias_resolution_test self_join`（S11 转 GREEN）。
- 受影响边界：join_test / nested_loop_join_test / join_output_resolution_test / outer_join_test / cross_db_query_test / planner_test / table_alias_resolution_test 全量零回归。
- 集成 Gate：全量 `cargo test` 0 failed。
- 判定：各命令退出码 0 + 原生输出行数统计，不新增判定脚本。

**Gate 2 Readiness**

PASS——调查完整（注册模型/缺陷双链/NLJ 路径基线/构造点 census/链式边界均为本会话实际代码阅读与二进制实测，HEAD 9538647 + 工作区现状）；设计闭合（D9 双臂含否决路线与非自连接零变化论证）；任务契约齐备（T9 全字段，自包含）；RTM 9/9 Covered 无 Missing；验证逐 scenario 最简直接判定、无身份型证据工程；Persisted Evidence `none` 满足白名单；计划于 2026-10-04 获用户批准（原话「批准」），Status 转 `ready`。

**Persisted Evidence**

- Mode: none

（全部验证可由测试命令 + Act Response 承载；自连接缺陷为确定性复现，无需留存现场。）

**Risks and Notes**

- `output_sides` 为加性字段，`JoinNode`/`NestedLoopJoinNode` 的其它消费面（`get_plan_output_columns`、`plan_format`、`correlated` 参数注入）只读既有字段——机械适配以编译错误为引导；若出现构造点之外的语义消费（编译不报但行为依赖 left_table_name），属实质问题返回 Plan。
- 双腿同侧拒绝为新增计划期错误面：既有语料若有 `ON t.a = t.b` 同表腿形态（非自连接单表无 JOIN 不受影响；自连接形态语料仅 `ON a.id = b.id` 不触发），预期零命中；若全量回归命中，按语料校准白名单处理并记入 Act Response。
- 链式 JOIN 既有边界的三种实测形态（计划失败 ×2、panic ×1）为本 Cycle 的回归对照基线：T9 后三类形态的可观察结果 SHALL 不变。
- 行号查询为 9538647 + 父 Cycle 工作区现状的混合快照，Act 以符号定位为准。

## Act Response

- Status: reported

### 实际改动

**修复臂 1（ON 交换判定按声明限定符归属）**

- `src/parser/planner/ddl_dml.rs`：`extract_join_conditions` 签名平行增参 `left_qualifiers: &[String]` + `right_qualifier: &str`（AND 递归臂透传）；单等值腿先侧别判定——新增私有 `expr_join_side`（限定腿按书写限定符经 `qualifier_matches_declared` 依次匹配左链各臂 → 左侧、右臂 → 右侧、均未命中 `TableNotFound(qualifier)`；裸名腿返回 `None` 由调用方按书写位置定侧）；两腿同侧 `PlanError::InvalidQuery("JOIN ON condition must reference both sides of the join")` 点名零副作用；两腿异侧按侧装配（`left_column` 恒为左输入腿，书写反序形态归位）；`ColumnRef` 仍经既有 `resolve_column_ref`（裸名按侧解析上下文、限定腿真实名不变）。
- `src/parser/planner/query.rs`：Hash 臂调用点穿线 `&current_qualifiers` / `&right_qualifier`。

**修复臂 2（输出归属按显式侧标志）**

- `src/executor/plan.rs`：新 `JoinSide { Left, Right }`（Debug/Clone/Copy/PartialEq/Eq）；`JoinNode`/`NestedLoopJoinNode` 加性 `pub output_sides: Vec<JoinSide>`（与 `output_columns` 同序同长，执行器输入项语境）。
- `src/executor/mod.rs`：导出 `JoinSide`。
- `src/parser/planner/query.rs`：`all_columns` 构造点同步产出 `all_sides`（左链各臂列 Left、右臂列 Right，原始表序语境），SELECT 裁剪分支（`kept`）三清单同步裁剪；Hash 臂直用；NLJ RIGHT 交换臂按输入交换反转（Left↔Right，`output_columns` 原表序不变），两节点构造点接线；RIGHT 交换处既有 `table_alias` 归属注释更新为侧标志语义。
- `src/executor/join.rs`：`JoinExecutor` 删 `left_table_name` 字段、增 `output_sides`；`build_output_row` 改 `output_sides[i]` 侧标志取行；`_right_table_name` 保留。
- `src/executor/join_config.rs`：删 `left_table_name`、增 `output_sides`（`right_table_name` 现状 dead-by-underscore 非本 change 产物，不动）。
- `src/executor/nested_loop_join.rs`：`NestedLoopJoinExecutor` 删 `left_table_name` 字段与 `new` 参数、增 `output_sides`；`build_output_row`/`build_padded_row`（外连接补齐行）均改侧标志归属。
- `src/pipeline.rs`：Join 构造臂 `left_column_indices` 名字结果弃置（调用保留——嵌套链形态既有错误传播面），`JoinConfig` 传 `output_sides`；NLJ 构造臂 `extract_column_indices(&node.left)` 调用保留、结果弃置，`NestedLoopJoinExecutor::new` 传 `node.output_sides`。

**测试见证（TDD：先 RED 后 GREEN）**

- 新 `tests/self_join_test.rs` 6 用例（S31–S36 矩阵，lib `execute_sql` 断言精确行值）：Hash 正向/反向配对、投影归属（同名双列 + 三列混排）、LEFT/FULL 外连接与非等值 NLJ 归属、双腿同侧（左/右）点名拒绝 + 零副作用（COUNT 不变）、非自连接全形态对照（正向/反向/裸名/无别名/LEFT 补齐）。
- `tests/join_test.rs`：7 处 `JoinConfig` 机械适配（`left_table_name` 字段 → `output_sides`，按各处 `output_columns` 侧序填写；断言零变化）。
- `tests/nested_loop_join_test.rs`：1 处节点字面量加 `output_sides: Vec::new()`（断言零变化）。
- `tests/table_alias_resolution_test.rs`：S11 `self_join_same_name_columns_resolve_by_qualifier` 解除 `#[ignore]` 转常规验收（S37），注释同步为修复后状态。

### 与计划的偏差

1. **S33 见证形态校准（非实质，测试面）**：proposal S33 示例形态 `SELECT e2.id, e1.id` 隐含「JOIN 基座按 SELECT 序输出」假设。实施期探针实测（当前工作区，含父 Cycle 交付面）：JOIN 基座纯列清单输出列序恒为**表序**——Hash 与 NLJ、自连接与非自连接同语义（`SELECT d.dname, e.ename FROM emp3 e JOIN dept d …` 返回 `[ename, dname]`），Scan 基座才按 SELECT 序；表头与行同源于 join `output_columns`，自洽无错位。该语义 T9 前后不变（RED 期 S33 同形态即得表序 `[[10,10],[11,11]]`），非本修复引入。S33 用例改用与表序一致的 SELECT 形态，归属见证力等价（修复前同形态两列同读左行 `[[10,10],[11,11]]` → 修复后各读对应侧 `[[1,10],[2,11]]`）。JOIN 基座 SELECT 序缺失作为范围外观察记入 Experience Candidates（Issue 候选），不落账。
2. **双腿同侧拒绝的错误载体**：经既有 `PlanError::InvalidQuery` 承载（渲染 `Invalid query: JOIN ON condition must reference both sides of the join`，SQLSTATE 沿该变体既有映射，无新错误变体、未改映射表）。契约句「映射既有 42601 类」按「既有变体既有分类」理解执行。

### 自检发现（Gate 3）

- 编辑期引入一处注释错字（「语效」），即时修复。
- 全量回归零命中：双腿同侧新增拒绝面在既有语料零命中（计划风险预判成立）；非自连接全部形态逐字节零变化（S36 + join/nested_loop_join/join_output_resolution/outer_join/cross_db_query 既有套件全绿见证）。
- 遗留 Minor：无。Critical/Important：无。

### 验证（Gate 4）

- **RED（实施前）**：`cargo test --test self_join_test` → `5 failed; 1 passed`（S31 `[["r1","r1"],…]`、S33 `[[10,10],…]`、S34 全 `[x,x]`、S35 静默执行 `[["r1"],["r2"]]`，均与 Plan Context 预测逐字节一致；S36 非自连接对照 GREEN 为零回归基线），退出码非零。
- **GREEN（定向）**：`cargo test --test self_join_test --test table_alias_resolution_test --test join_test --test nested_loop_join_test --test join_output_resolution_test --test outer_join_test` → 六套件 `14 / 7 / 9 / 16 / 6 / 6 passed; 0 failed; 0 ignored`，退出码 0（table_alias_resolution 14 含 S11 转正）。
- **全量 Gate**：`cargo test` → `TOTAL passed: 1882 failed: 0 ignored: 2`，退出码 0。与计划预测一致（基线 1875/0/3：S11 转正 −1 ignored +1 passed，新增 self_join 6 用例）。
- **fmt/clippy**：`cargo fmt --check` 干净（`cargo fmt` 仅触及本任务编辑面 4 文件，语义保持格式化，在全量结论之后执行）；`cargo clippy --tests` 无告警（仅既有 `~/.cargo/config` 弃用提示，与本 change 无关）。

### Persisted Evidence

None required（Plan Context Mode: `none`；全部验证由测试命令 + 本 Response 承载）。

### Experience Candidates

- **Issue 候选（范围外，报告不落账）**：JOIN 基座纯列清单 SELECT 输出列序恒为表序——SELECT 序重排在 Join/NLJ 基座不生效（Scan 基座生效），表头与行自洽但与标准 SELECT 序语义不符；自连接/非自连接、Hash/NLJ 同语义，T9 前后不变。证据：本 Cycle Act Response 偏差 1 的探针实测与 S33 校准记录。
- 三表链式 JOIN 既有边界（hash+hash 计划失败 / hash+NLJ 同 / 仅前两表形态 `join.rs` panic）为 Plan Context 既有报告的 Issue 候选，本 Cycle 未新增证据，维持待用户裁定。

### 未解决问题

无阻塞。Iteration 000 的场景面（S1–S12 + S31–S37）已全部交付，等 Plan Review 独立核验；T9 复选框与 T3（S11 闭合）已按实际完成状态更新至 change `tasks.md`。R9 spec delta 已在 change `specs/sql-table-alias-resolution/` 在案，语料合并归 docs-maintainer 收尾职责。

## Plan Review

- Reviewed: 2026-10-04（Plan 独立审计：T9 完整 diff 逐文件阅读〔ddl_dml/query/plan/mod/join/join_config/nested_loop_join/pipeline + 4 测试文件〕+ 新测试断言逐条核对 + 构造点 census 独立 grep + 定向复跑）
- Inputs: 本 Cycle Plan Context、Act Response（reported）、工作区完整 diff（T9 面）、`tests/self_join_test.rs` 全文、D9、R9 spec delta；父 Cycle Plan Review（T1/T2/T3 面已独立核实，本次不重复）
- Persisted Evidence: Mode `none` 满足白名单（未因 Evidence 缺失提出问题）

### 代码与声明核对

- **修复臂 1（核实一致）**：`extract_join_conditions` 签名平行增参（AND 递归臂透传）；新 `expr_join_side`——限定腿按书写限定符经 `qualifier_matches_declared` 匹配左链 → 左侧、右臂 → 右侧、均未命中 `TableNotFound(qualifier)`；裸名腿 `None` → 调用方按书写位置定侧（`unwrap_or(true)`/`unwrap_or(false)`）；两腿同侧 `InvalidQuery("JOIN ON condition must reference both sides of the join")` 先于解析零副作用；异侧按侧装配后仍经既有 `resolve_column_ref`（`ColumnRef.table` 真实名语义不变）。与 D9 修复臂 1 逐条相符。语义推演核对：非自连接正向（判定不交换）/反向（按限定符交换）/裸名（位置定侧）与现状一致；自连接裸名混合腿（如 `ON id = e2.manager_id`）从旧代码静默误交换转为同侧点名拒绝——旧行为即缺陷面，符合 R9 delta「两腿同侧 SHALL 点名拒绝」。
- **修复臂 2（核实一致）**：`plan.rs` 新 `JoinSide`（derive 集与契约一致）；`JoinNode`/`NestedLoopJoinNode` 加性 `output_sides`；`all_sides` 与 `all_columns`/`all_qualifiers` 同构同序（`current_tables` flat_map → chain right_table，`expect` 先例与邻接代码一致），`kept` 分支三清单同步裁剪；Hash 臂直用；NLJ RIGHT 交换臂反转（Left↔Right、`output_columns` 原表序不变，注释更新为侧标志语义）；两执行器 `build_output_row`/`build_padded_row` 改 `zip(output_columns, output_sides)` 侧标志取行；`JoinExecutor.left_table_name`/`JoinConfig.left_table_name`/NLJ `new` 参数删除，`_right_table_name` 保留（dead-by-underscore 现状，契约禁止不动遵守）；pipeline 两构造臂调用保留、名字弃置。与 D9 修复臂 2 逐条相符。
- **Forbidden/Preserve 边界（核实满足）**：`OutputColumn.table_alias` 语义未动；写限定符未穿透（侧别判定只用于定侧）；三表链式装配未触及；`is_pure_equi_join_on` 接受面、`JoinCondition` 执行语义（按列名取键）、`extract_column_indices` 返回值语义（NLJ 臂链首名）均未动；SemiJoin/AntiJoin 零触及（grep 证实构造点仅 subquery.rs 且无 `output_sides`）。
- **构造点 census（独立 grep 证实）**：`JoinNode` 字面构造仅 query.rs:686；`NestedLoopJoinNode` 仅 query.rs:769/778——与 Act Response census 声明一致，编译期穷尽性由全量构建背书。
- **测试见证（非空洞）**：`tests/self_join_test.rs` 6 用例断言精确行值（S31/S32 配对、S33 双列同名 + 三列混排归属、S34 LEFT/FULL 补齐 + 非等值 NLJ、S35 双侧同侧拒绝 + COUNT 零副作用、S36 非自连接五形态对照）；S33 校准注记与「JOIN 基座输出恒表序」既有语义一致，归属见证力等价（修复前同形态 `[[10,10],[11,11]]`）。`join_test` 7 处 `JoinConfig` 按各处 `output_columns` 侧序机械填写（含 `Right`-only/`Left`-only 空表用例）、断言零变化；NLJ 节点字面量加空 `output_sides`；S11 `#[ignore]` 解除、注释同步为修复后状态。
- **S11 闭合（核实）**：`table_alias_resolution_test::self_join_same_name_columns_resolve_by_qualifier` 已转常规验收；全仓 `#[ignore]` 实际测试 2 个（均在 cli_test，第 3 处 grep 命中为 S11 注释文本），与「ignore 回落 2」一致。

### 偏差分类（Plan 遗漏 / Plan 错误 / Act 偏离 / 基线变化 / 新证据）

1. **S33 见证形态校准**（Act 偏离，非实质，测试面）：JOIN 基座纯列清单 SELECT 输出恒表序为既有共享行为（T9 前后不变，Act 探针实测 + 本 Review 核对断言值与表序自洽）；proposal S33 的 SELECT 序假设失实属 proposal 措辞遗漏。归属见证力等价，可接受。
2. **双腿同侧错误载体**（Act 偏离，契约内，非实质）：经既有 `PlanError::InvalidQuery` 承载（渲染 `Invalid query: …`，SQLSTATE 沿既有映射），无新变体——契约句「映射既有 42601 类」的合理执行。
3. **GREEN 计数误标（Act 偏离，记录面，非阻塞 Minor finding）**：Act Response 验证段「六套件 `14 / 7 / 9 / 16 / 6 / 6 passed`（table_alias_resolution 14 含 S11 转正）」与实测套件映射不符——正确映射为 join_output_resolution=**14** / join=7 / nested_loop_join=9 / outer_join=**16** / self_join=**6** / table_alias_resolution=**19**（Act 把末位误写 6、把 14 误归属 table_alias_resolution，实际 14 是 join_output_resolution、table_alias_resolution 为 19=18 既有 + S11 转正）。判定性结论（六套件 0 failed、0 ignored、S11 转正、全量 1882/0/2）不受影响：全量算术自洽（1875 + S11 转正 1 + self_join 6 = 1882；ignored 3−1=2），且本 Review 独立复跑证实（见 Evidence）。
4. 其余无偏差；无身份型证据工程；无范围外改动。

### 验证采信与定向复跑

- Act 全量 Gate（1882 passed / 0 failed / 2 ignored）**采信**——同会话新鲜运行；但 Act 自述 `cargo fmt` 在全量结论之后触及 T9 编辑面 4 文件，触发公共规则 › 验证 的「被覆盖表面在结论后变化」定向审计例外。
- **定向复跑（本 Review 执行，2026-10-04）**：`cargo test --test self_join_test --test table_alias_resolution_test --test join_test --test nested_loop_join_test --test join_output_resolution_test --test outer_join_test` → `14 / 7 / 9 / 16 / 6 / 19 passed; 0 failed; 0 ignored`（六套件全绿，退出码 0）——fmt 后置影响排除，GREEN 结论成立。`cargo fmt --check` 干净（`~/.cargo/config` 弃用提示为环境既有，与本 change 无关）。clippy 结论（格式化不影响 lint 结果）采信 Act。

### Findings

- 阻塞 Acceptance 的 finding：无。
- 非阻塞 Minor：① GREEN 计数误标（偏差 3，Act Response 记录面，事实已由本 Review 复跑更正在案）；② S33 proposal 措辞遗漏（偏差 1，已在测试注记与本 Review 在案）。
- 无身份型证据工程；无范围外新缺陷。

### Acceptance Gaps

- 无。S31–S37 全部见证（S37 = S11 解除 ignore 转 GREEN，19 passed 含之）；R9 spec delta 场景与 `tests/self_join_test.rs` 一一对应；父 Cycle R3-S11 缺口关闭；全量 0 failed。

### Blocker 处理结果

- 父 Cycle Blocker Handoff（自连接双重缺陷阻塞 R3-S11）经本 Cycle T9 交付解除：缺陷 ①（交换判定按归一真实名）由修复臂 1 关闭，缺陷 ②（输出归属按真实表名）由修复臂 2 关闭；非自连接零回归有 S36 对照 + join 五套件见证。Issue 随 T9 交付解决（用户裁定），不落账。

### Evidence

- 本 Review 独立复跑：`cargo test --test self_join_test --test table_alias_resolution_test --test join_test --test nested_loop_join_test --test join_output_resolution_test --test outer_join_test` → 六套件 `14/7/9/16/6/19 passed; 0 failed; 0 ignored`，退出码 0；`cargo fmt --check` 退出码 0。
- 全量 1882/0/2：采信 Act Response（同会话新鲜运行、基线算术自洽、fmt 后置影响经定向复跑排除）。
- diff 核对：T9 面 8 源文件 + 4 测试文件与 Act Response「实际改动」逐条一致，无未声明改动（`git status` 文件清单与两 Cycle 交付面并集吻合）。

### Follow-up Decision

- 接受：既有 Acceptance（S1–S12 + S31–S37、全量 0 failed、fmt/clippy 干净）全部满足；两项 Minor finding 不阻塞且已更正在案；Iter000 扩围后的平衡审计（T1/T3/T2/T9 单轮「红线清零」）成立。
- 无需当前 Cycle 修复；无需 rework/replan。

### Iteration Plan Update

- 无（Map 保持：Iter 001 T4/T5、Iter 002 T6/T7、Iter 003 T8）。

### Issue 候选（范围外，只报告不落账）

1. **JOIN 基座纯列清单 SELECT 输出列序恒为表序**（Scan 基座按 SELECT 序，Join/NLJ 基座不生效；表头与行自洽无错位；自连接/非自连接、Hash/NLJ 同语义，T9 前后不变）。证据：本 Cycle Act Response 偏差 1 探针实测 + S33 校准记录。处置待用户裁定（Recorder 落账或并入后续 change）。
2. **三表链式 JOIN 全形态已坏**（维持父 Cycle Review 原报告：hash+hash 计划失败 / hash+NLJ 同 / 仅前两表形态 panic；T9 Preserve 边界，错误面不变）。处置待用户裁定。

### Next Cycle

- None（Iteration 000 完成）。

### Next Iteration

- `iterations/001-subquery/000-initial.md`（子查询面收口批：T4、T5——已展开，Status: ready）。

- Review Result: accepted
