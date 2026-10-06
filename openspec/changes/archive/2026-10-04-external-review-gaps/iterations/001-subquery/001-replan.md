# Iteration 001 / Cycle 001: 子查询面与 JOIN 装配收口批（replan）

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T4（WHERE 标量子查询比较 + 投影位形状）、T5（FROM 派生表内集合操作）、T11（多表链式 JOIN 装配正确性，2026-10-05 replan 并入）、T10（JOIN 基座输出列序 SELECT 化，2026-10-05 replan 并入）
- Acceptance gaps: None（父 Cycle `000-initial.md` 未执行、零工作区变化——本 Cycle 为其修订版全量替代，非缺口续作）
- Repair items: None（replan 使用更新后的全局 task T10/T11，不建 repair item）
- Inherited scope: proposal R4/R5/R10/R11 与场景 S13–S21、S38–S46；design D4/D5/D10/D11；Iteration 000 交付基线——T1（IndexScanAll 列契约）、T2（WAL 收敛标志链）、T3（JOIN ORDER BY 布局键）、T9（自连接侧身份 `expr_join_side` + `JoinSide`/`output_sides`）全部 accepted（Iteration 000 Cycle 001 Review，2026-10-04）；全量 1882 passed / 0 failed / 2 ignored（Cycle 001 Act Response 全量 Gate + Plan Review 定向复跑采信）
- Excluded scope: IN/EXISTS/标量子查询内的集合操作（spec 维持点名拒绝）；JOIN 基座 WHERE 内标量子查询（维持点名拒绝）；表达式项×子查询解锁；右深/bushy JOIN 重排（代价模型域 I016）；写面与 SQLSTATE（Iteration 002）；文档（Iteration 003）

**Objective**

子查询面 + 遗留 JOIN Issue 四合一清零：① WHERE 比较腿标量子查询（非相关+相关）端到端可用，SELECT 投影位形状与请求一致；② FROM 派生表内集合操作可达且拒绝面收口；③ ≥3 表左深链 JOIN 全形态可达（hash+hash、hash+NLJ 混合、部分表 SELECT 不 panic）、链上同名列不串列；④ JOIN 基座纯列清单输出列序 = SELECT 序（裸路径与 WHERE/ORDER BY 裁剪路径）。全量测试零回归。

**Background**

2026-10-05 用户指令（原话「重新给出00cyc，把issue也计划解决进去」）：把 Iteration 000 Cycle 001 Plan Review 报告的两项范围外 Issue 候选（JOIN 基座 SELECT 序表序、三表链式 JOIN 全形态已坏）并入本 change 解决，不落账独立 Issue。父 Cycle `000-initial.md`（T4/T5，Status: ready）经审计后未交 Act 执行、工作区零变化，按协议 Plan Context 不可改写——本 replan Cycle 以修订后的任务与 Acceptance 全量替代之，`000-initial.md` 冻结保留。迭代主题由「子查询面收口批」扩为「子查询面与 JOIN 装配收口批」（用户已裁定并入 00cyc 不拆分）。

**Investigation Facts**

- Current Baseline: 工作区 = Iteration 000 交付后状态（未提交，HEAD 9538647 + T1/T2/T3/T9 改动面）；全量 1882 passed / 0 failed / 2 ignored。行为基线：① WHERE 比较腿标量子查询 → `Unsupported expression type: EXPRESSION` exit 3；② SELECT 投影位子查询可用但形状异常（全表列泄漏 + `__subquery` 按位插列）；③ `FROM (… UNION …)` → `Set operations are not supported inside subqueries`；④ 三表链 JOIN 三形态全坏（hash+hash 与 hash+NLJ 计划期 `Column not found`、仅前两表 SELECT `join.rs` panic exit 101）；⑤ JOIN 基座纯列清单输出恒表序（Scan 基座按 SELECT 序）。
- Current-State Evidence（行号为 Iteration 000 交付后工作区现状，本 replan 展开时按符号核实；Act 以符号定位为准）：
  - **T4——表达式子查询无臂**：`src/parser/planner/expression.rs` 全文件无 `Expr::Subquery` 匹配臂，落入 catch-all `_ => Err(PlanError::UnsupportedExpression("EXPRESSION".into()))`（expression.rs:688）。
  - **T4——WHERE 顶层子查询机制（Preserve 面）**：`try_build_where_subquery`（subquery.rs:19 起）只匹配顶层 `Expr::InSubquery` / `Expr::Exists`；相关参数经 `extract_correlated_params`（subquery.rs:156 起）；表达式子查询上下文置位 `building_subquery`（subquery.rs:36/:85）。
  - **T4——SELECT 投影位机制**：检测臂 query.rs:1320-1356（`UnnamedExpr(Expr::Subquery)` → `__subquery`；`expr AS alias` → 别名；`building_subquery` save/restore 内 `build_query`；产出 `subquery_evals: Vec<(idx, plan, col_name, params)>`）；DISTINCT×子查询拒绝 query.rs:1362-1364；投影位包装 query.rs:2722-2735（从右向左，`result_column_index = proj_idx - subqueries_before`）；**恒等门** query.rs:1785 `if has_aggregates || !subquery_evals.is_empty()`（实际 SubqueryEval 只插列不裁剪 → 全表列泄漏，S18 缺陷点）；聚合×子查询与表达式项×子查询拒绝在投影项分类臂（query.rs:1502/1515/1544/1556）。
  - **T4——形状异常见证（校准对象）**：`tests/subquery_test.rs:487` `name_value_map` helper 与 :513/:540/:565/:656-657 用例按异常形状写断言。
  - **T4——求值同步约束与参数机制（D4 关键约束）**：`Expression::evaluate_ref` 同步禁 `.await`（predicate.rs trait）→ 子查询值须预落 `ParameterExpression`；`inject_parameters`（predicate.rs:43/:152/:245）、`set_parameter_value`（predicate.rs:70/:154-155），消费先例 correlated.rs:68-72。
  - **T4——执行器与构造先例**：`SubqueryEvalExecutor`（subquery_eval.rs:23；语句级缓存 `correlated_cache: HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>` :43；多行 `SubqueryReturnsMultipleRow` :97/:168；首行首列提取 :78-109 区域）；`SemiJoinExecutorV2` 缓存同型；非相关 = 空参数序列 → 单缓存项。下推门：JOIN `filter_projection` 含 `|| !subquery_evals.is_empty()` → 恒等（query.rs:1873-1881），单表 `proj_or_empty` 同规则。构造臂先例 pipeline.rs:1762-1772（快照/路由穿线）；pipeline.rs:2004/:2051 `extract_column_indices` 消费臂。JOIN WHERE 子查询拒绝（Preserve）：query.rs:1844 `UnsupportedStatement("WHERE CLAUSE ON JOIN")`。
  - **T5——拒绝点与置位点 census**：query.rs:925-929（`build_set_operation` 顶部 `building_subquery` 守卫）。置位点：FROM 派生 query.rs:401-410、视图展开 query.rs:1236-1239、SELECT 投影位 query.rs:1344-1354、表达式子查询 subquery.rs:36/:85；CTE `consume_with`（query.rs:1111）子查询内点名拒绝且不改标志。
  - **T5——集合操作全链（零改动可行已核实）**：`SetOperationNode.columns` = 左臂列名（plan.rs:793 起），`get_plan_output_columns` 臂 query.rs:134-135；DerivedScan 包裹 query.rs:398-429；`SetOperationExecutor` 对臂形状零假设；plan_format 渲染臂 plan_format.rs:64/:328 与 correlated 注入臂 correlated.rs:103 在案；括号化拒绝 query.rs:1094-1096/:1280-1282；子查询内 DISTINCT 零消费 query.rs:1300。`tests/set_operations_test.rs` 现有 14 用例。
  - **T11——装配循环结构**（`build_from_clause_with_projection`，query.rs ~335-830）：逐 FROM 因子迭代——右因子建计划 → `all_columns`/`all_qualifiers`/`all_sides` 自 `self.tables` **全宽**重建（current_tables 各臂 + right_table）→ `select_star`/`carry_all_columns` 判定 → **kept 分支（每步执行）**：MS25 T8 严格校验（存在/唯一/歧义点名，作用于当前迭代组合形状）+ 按表序过滤三清单 → Hash 臂（`is_pure_equi_join_on` → `extract_join_conditions`，T9 侧别装配）或 NLJ 臂（`join_column_layout` 全宽段布局 + `build_where` 编译谓词，绝对偏移）→ 循环尾 `current_tables.push` / `combined_qualifiers = output_qualifiers`。
  - **T11——根因三联（代码闭合）**：① 严格校验/kept 每步执行——第三表列在第一步即 `ColumnNotFound`（hash+hash/hash+NLJ 计划失败根因）；中间步 kept 裁剪使左输入行形状 ≠ 下一步 `all_columns` 全宽假设。② `OutputColumn.column_index` within-table 语义 + 执行器 `left_row[column_index]` 寻址（join.rs / nested_loop_join.rs `build_output_row`/`build_padded_row`）——链式左输入（Join 节点作左输入）时链列读串行。③ 条件键裸列名查表：`JoinExecutor::build_hash_key_left/right`（join.rs:79-110）`left/right_column_indices.get(&cond.*_column.column).expect(...)`——链上同名列静默串列、缺列 panic；NLJ 谓词经 `join_column_layout` 绝对偏移编译（index-based），不受 ③ 影响。
  - **T11——执行器构造面**：pipeline.rs Join 臂（~1595-1633）`extract_column_indices(&join_node.left/right)` 建裸名 map（Join/NLJ 输入臂按 `output_columns` 位置枚举，pipeline.rs:1903-1956；Scan 臂按 `columns`）；`JoinConfig` maps 为 `HashMap<String, usize>`（join_config.rs）；`JoinCondition.left_column/right_column: ColumnRef{table: Option<String>（真实名，T9 侧别装配产出）, column}`，执行器查表忽略 `table`。`tests/join_test.rs` 7 处直构 `JoinConfig`（裸名 map + `ColumnRef{table: Some(…)}`）。
  - **T10——两条表序路径**：① kept 分支按表序迭代 `all_columns` + `any()` 匹配（query.rs ~648-668）；② `resolve_join_projection_indices`（query.rs:3263-3316）按 combined 表序过滤收集——WHERE 路径（`filter_projection`/`where_join_projection`）与 ORDER BY 路径（`sort_projection`，query.rs:2518-2535）裁剪序同为表序；校验（0 命中 `ColumnNotFound` / 裸名多命中 `AmbiguousColumn`）在 :3285-3303。对照：Scan 基座 `resolve_projection_indices` 按 SELECT 序（I034）；表达式项顶层 `ProjectionExecutor` 本就 SELECT 序。消费面：`get_plan_output_columns`（query.rs:134-135 SetOperation 臂同区）表头 → CLI/CTAS 列名/INSERT SELECT 映射/DerivedScan。
  - **T10——校准对象**：`tests/self_join_test.rs::self_join_projection_reads_declared_side` 第二形态（三列混排，现按表序断言 `[m1, r1, 1]`，SELECT 序应为 `[m1, 1, r1]`，注记同步）。
- Code and Critical Path:
  - T4：新 helper（WHERE 表达式递归检测 `Expr::Subquery`，镜像 `contains_or` 模式）→ `build_where` 前置分流：命中则禁止下推 + Subquery 腿替换 `ParameterExpression` 占位 + bindings 登记（子查询经 `building_subquery` 上下文 `build_query`）→ 新节点 `ScalarSubqueryFilterNode { input, predicate, bindings: Vec<(String, PhysicalPlan)> }`（plan.rs + plan_format/correlated/get_plan_output_columns 穷尽臂）→ 新 `ScalarSubqueryFilterExecutor`（逐行参数提取 → 语句级缓存 → miss 时 bindings 逐项 clone + `inject_correlated_values` + 重建 drain 首行首列〔多行 → `SubqueryReturnsMultipleRow`、零行 → NULL〕→ `set_parameter_value` → 谓词求值）→ pipeline 构造臂。投影位：query.rs:1785 恒等门摘除 `|| !subquery_evals.is_empty()`。
  - T5：`building_subquery: bool` 拆「FROM 派生上下文」独立标志，仅 FROM 派生组合放行 query.rs:925 守卫；视图展开/表达式子查询/SELECT 投影位组合维持拒绝；save/restore 严格配对照搬三置位点现有模式。
  - T11：装配循环重构——非末步强制全组合承载（跳过 kept 与严格校验）；末步按查询标志现行逻辑；迭代 ≥2 时链前缀列 `column_index` 重定基为左输入行绝对偏移（累计宽度）、右臂列 within-table 不变，`output_sides` 机制不动；条件键限定解析——执行器查表改 `table.column`（真实名小写）优先、裸名回退，pipeline Join 臂注册限定键（裸键仅无歧义时注册；Scan 单表输入裸=限定同键）；NLJ 链零新机制（谓词绝对偏移与全组合左输入行天然对齐）。
  - T10：kept 分支改按 `qualified_columns` 清单序重建（唯一命中后 push 三清单、重复项重复）；`resolve_join_projection_indices` 改按清单序逐项定位命中索引（校验臂与错误面不变）；消费面自然跟随。

**Implementation Guidance**

顺序：T4（中-大）→ T5（小）→ T11（中-大）→ T10（中）。T4 先 RED（比较腿 `UnsupportedExpression` / S18 泄漏形状）→ 检测 + 占位替换 → 节点 + 执行器 → 构造臂 → 投影位门摘除 → `subquery_test` 形状校准；子查询编译、相关参数、缓存、多行/零行语义一律复用既有机制（SubqueryEval/SemiJoin/Correlated 先例），快照与跨库路由照搬 pipeline.rs:1762 穿线。T5 拆标志时 save/restore 严格配对，错误文案不变。T11 先 RED（三形态当前失败即 RED 基线）→ 循环重构（末步判定）→ 重定基 → 限定键（执行器查表序 + pipeline 注册）→ 链矩阵；两表路径逐字节回归是重构的安全网（join 五套件 + 直构用例零适配预期）。T10 在 T11 的末步 kept 分支上重建（避免同面双触），S33 校准与裁剪函数 SELECT 序化最后收口。四任务均不建身份型证据工程。

**Task Contracts**

### T4: WHERE 标量子查询比较 + 投影位形状

- Requirement/Scenario: R4 / S13–S18
- Depends on: T3（Iteration 000 已交付 accepted——JOIN `sort_projection`/`filter_projection` 共享门已就位）
- Targets: `src/parser/planner/expression.rs`（Subquery 腿占位替换）、`src/parser/planner/query.rs`（WHERE 检测分流、:1785 恒等门）、新计划节点与执行器（`src/executor/plan.rs` + 新模块）、`src/pipeline.rs`（构造臂 + 穷尽消费臂）、`tests/scalar_subquery_comparison_test.rs`（新）、`tests/subquery_test.rs`（形状校准）
- Current behavior: S13 `WHERE salary = (SELECT MAX(salary) FROM emp)` → `Unsupported expression type: EXPRESSION` exit 3；S18 `SELECT name, (SELECT MAX(salary) FROM q6) FROM q6` 全表列泄漏 + `__subquery` 位次插列。
- Required behavior: S13/S14 非相关 + 相关比较返回正确行；S15 空子查询 → NULL → 比较三值语义（Unknown → 行不匹配）；S16 多行 → `SubqueryReturnsMultipleRow` 既有错误面；S17 JOIN 基座 WHERE 内维持点名拒绝且含子查询谓词不下推（计划形状见证）；S18 表头 `["name","__subquery"]`（或别名）、未请求列不泄漏、行形状与请求一致。
- Preserve: 顶层 InSubquery/Exists/SemiJoin/AntiJoin 机制零变化；DISTINCT×子查询、聚合×子查询、表达式项×子查询三拒绝面；JOIN 基座 WHERE 子查询拒绝；`result_column_index` 公式；SubqueryEval 投影位语义；`evaluate_ref` 同步语义；plan cache 既有规则。
- Forbidden: 不得给 `Expression` 增加异步求值变体；不得解锁 JOIN 基座 WHERE 子查询；不得改 InSubquery/Exists 通道语义；不得给 `get_subquery_first_column` 加 SetOperation 臂。
- Test witness: 新 `tests/scalar_subquery_comparison_test.rs`（S13–S18 矩阵 + 非相关单次求值见证 + 不下推计划形状见证）——先写 RED。`subquery_test` 形状见证用例按 S18 校准（diff 记入 Act Response）。
- GREEN condition: 新矩阵全绿 + subquery_test 校准后全绿 + correlated/planner/subquery 既有套件定向回归零失败。
- Verification: `cargo test --test scalar_subquery_comparison_test --test subquery_test` + 定向回归 + 全量 0 failed。
- Stop when: 占位替换需要改 `Expression` trait 异步语义；或投影位公式在裁剪生效后错位；或相关参数注入与谓词求值语义冲突 → 返回 Plan。

### T5: FROM 派生表内集合操作

- Requirement/Scenario: R5 / S19–S21
- Depends on: None
- Targets: `src/parser/planner/query.rs`（`building_subquery` 标志拆分 + :925-929 守卫 + 三置位点配对适配）
- Current behavior: `FROM (SELECT … UNION SELECT …)` → `Set operations are not supported inside subqueries`。
- Required behavior: S19 FROM 派生表集合操作可执行（UNION 去重 / UNION ALL / INTERSECT / EXCEPT，列名取左臂）；S20 外层投影/WHERE/聚合/ORDER BY 叠加照常；S21 IN/EXISTS/标量子查询内集合操作维持点名拒绝。
- Preserve: 视图展开与表达式子查询（IN/EXISTS/标量）上下文拒绝；SELECT 投影位子查询上下文拒绝；括号化操作数拒绝；子查询内 DISTINCT 零消费；CTE `consume_with` 行为；`SetOperationExecutor`/`SetOperationNode` 零改动；维持拒绝上下文的错误文案逐字节不变。
- Forbidden: 不得解锁 IN/EXISTS/标量子查询内集合操作；不得改 `get_subquery_first_column`（不加 SetOperation 臂）。
- Test witness: `tests/set_operations_test.rs` 派生表矩阵（S19–S21）——先写 RED（当前点名拒绝即 RED）。
- GREEN condition: 新矩阵全绿 + set_operations/cte/view/subquery 既有套件零回归。
- Verification: `cargo test --test set_operations_test --test cte_test --test view_test --test subquery_test` + 全量 0 failed。
- Stop when: 拆标志波及视图/表达式子查询上下文语义（既有套件回归命中）→ 返回 Plan。

### T11: 多表链式 JOIN 装配正确性

- Requirement/Scenario: R11 / S42–S46
- Depends on: None（T9 的侧身份/`output_sides` 面已交付为前提）
- Targets: `src/parser/planner/query.rs`（装配循环：非末步全组合承载 + 末步判定、链前缀列 `column_index` 重定基）、`src/executor/join.rs`（`build_hash_key_left/right` 限定优先查表）、`src/pipeline.rs`（Join 构造臂限定键注册；NLJ 臂一致性核对）、`tests/chain_join_test.rs`（新）
- Current behavior: 三表链三形态全坏——hash+hash 与 hash+NLJ 计划期 `Column not found`（第三表列在第一步严格校验点名）；仅前两表 SELECT `join.rs` 键查表 `.expect` panic exit 101；链上同名列条件键存在裸名串列静默面（改动前 release 二进制实测，HEAD 9538647 与工作区同）。
- Required behavior: S42 hash+hash 三表链端到端可达且每步配对正确；S43 hash+NLJ 混合链可达且谓词作用全组合行；S44 仅前两表 SELECT 正常裁剪零 panic；S45 链上同名列条件键按（真实表, 列）解析不串列；S46 两表单步全形态（`SELECT *`/WHERE/ORDER BY/聚合/直构 `JoinConfig`/EXPLAIN/附加库）逐字节零变化；真未知列错误面不变。
- Preserve: 两表单步计划形状与执行行为逐字节（链长 1 无重定基、无中间步、kept 逻辑现状）；`output_sides` 侧身份机制（T9）；NLJ 谓词 `join_column_layout` 绝对偏移编译语义；`extract_column_indices` 各臂返回值语义（NLJ 臂链首名输出保留）；SemiJoin/AntiJoin 通道零触及；`build_hash_key_*` 的 `.expect` 防御臂保留（修复后结构上不可达）；真未知列 `ColumnNotFound`/`TableNotFound` 错误面。
- Forbidden: 不得实现右深/bushy 重排；不得改 `JoinCondition`/`JoinConfig` 类型结构（限定键经查表序实现，接口零变化）；不得改 `OutputColumn.table_alias` 语义；不得触碰 IN-subquery/SemiJoin 装配路径。
- Test witness: 新 `tests/chain_join_test.rs`（S42–S46 矩阵，lib `execute_sql` 断言精确行值；含同名列链夹具与两表对照）——先写 RED（三形态当前失败即 RED 基线，S46 对照先确认 GREEN）。
- GREEN condition: 新矩阵全绿 + join_test/nested_loop_join_test/join_output_resolution_test/outer_join_test/self_join_test/cross_db_query_test/table_alias_resolution_test 零回归。
- Verification: `cargo test --test chain_join_test --test join_test --test nested_loop_join_test --test join_output_resolution_test --test outer_join_test --test self_join_test` + 全量 0 failed。
- Stop when: 重定基或限定键引发两表形态行为变化（说明链不变量理解有误）；或链形态仍计划失败（说明循环结构理解有误）→ 返回 Plan。

### T10: JOIN 基座输出列序 SELECT 化

- Requirement/Scenario: R10 / S38–S41
- Depends on: T11（同一末步 kept 分支，T11 重构后单触）
- Targets: `src/parser/planner/query.rs`（kept 分支 SELECT 序重建、`resolve_join_projection_indices` 返回序 SELECT 化）、`tests/join_output_resolution_test.rs`（扩展）、`tests/self_join_test.rs`（S33 第二形态校准）
- Current behavior: JOIN 基座纯列清单输出恒表序——裸路径 kept 按表序过滤；WHERE/ORDER BY 裁剪路径 `resolve_join_projection_indices` 按 combined 表序收集（S38 修复前形态）。
- Required behavior: S38 裸路径表头与行按 SELECT 序（`SELECT d.dname, e.ename` → `[dname, ename]`）；S39/S40 WHERE/ORDER BY 裁剪序 = SELECT 序；S41 重复项重复产出、`SELECT *` 表序不变、CTAS 列名随输出表头（与 Scan 基座一致）；`AmbiguousColumn`/`ColumnNotFound` 既有错误面不变。
- Preserve: `SELECT *` 与 carry-all 全组合承载（聚合/派生/CTAS 输入、Sort 承载全列）语义；表达式项顶层 Projection 路径（本就 SELECT 序）；Scan 基座行为；SortExecutor 零改动；子查询项投影交互面（T4 契约范围，本任务不触）。
- Forbidden: 不得给 JOIN 节点加顶层 Projection 包装（D10 否决）；不得改 `resolve_projection_indices`（Scan 基座）与 `SortExecutor`；不得改校验错误变体与文案。
- Test witness: `join_output_resolution_test` 扩展 SELECT 序矩阵（裸/WHERE/ORDER BY 三路径 + 重复项 + 消费面表头）——先写 RED（表序现状即 RED）；`self_join_test` S33 第二形态断言翻转为 SELECT 序（`[m1, 1, r1]`）+ 注记更新。
- GREEN condition: 新矩阵全绿 + S33 校准 GREEN + ctas/derived_scan_alias/cross_db_query/cli_test/subquery_test 定向回归零失败（表序断言命中则白名单校准记 Act Response）。
- Verification: `cargo test --test join_output_resolution_test --test self_join_test --test ctas_test --test derived_scan_alias_test` + 全量 0 failed。
- Stop when: SELECT 序重建破坏唯一性/歧义校验；或消费面（CTAS/INSERT SELECT/DerivedScan）出现表头与行错位（说明 `output_columns` 同源假设失效）→ 返回 Plan。

**Invariants**

- 存储格式、网络协议字节、plan cache key 语义零变化。
- 全部既有 spec 语料行为零回归（本 Iteration delta：`sql-expression-evaluation` ADDED ×2、`sql-set-operations` ADDED、`join-output-shape-resolution` ADDED ×2，均在 change specs/ 在案）。
- 两表 JOIN 单步与 Iteration 000 已交付面（IndexScanAll 契约、WAL 收敛、布局键、侧身份）不在变更面内。
- 不建身份型证据工程/判定层；验证判定使用项目测试框架原生退出码与输出。

**Non-goals**

- IN/EXISTS/标量子查询内集合操作；JOIN 基座 WHERE 内标量子查询解锁；表达式项×子查询。
- 右深/bushy JOIN 重排、代价模型；写面与 SQLSTATE（Iteration 002）；文档（Iteration 003）。

**Acceptance**

- S13–S21、S38–S46（proposal 场景草图）全部可观察通过；R4/R5/R10/R11 的 spec delta 场景与任务测试一一对应（RTM 见 tasks.md）。
- 全量 `cargo test` 0 failed；`cargo fmt --check` 与 `cargo clippy --tests` 干净（fmt 在全量结论之前执行，或执行后对受影响面定向复跑）。

**Verification**

- 直接目标：`cargo test --test scalar_subquery_comparison_test --test set_operations_test --test chain_join_test --test join_output_resolution_test`（各自新增用例 RED→GREEN）。
- 受影响边界：`cargo test --test subquery_test --test cte_test --test view_test --test planner_test --test join_test --test nested_loop_join_test --test outer_join_test --test self_join_test --test ctas_test --test derived_scan_alias_test --test cross_db_query_test --test cli_test`。
- 集成 Gate：全量 `cargo test` 0 failed。
- 判定：各命令退出码 0 + 原生输出行数统计，不新增判定脚本；套件计数按 `Running tests/<name>` 行对应记录。

**Gate 2 Readiness**

PASS——调查完整（T4/T5 锚点自 Iteration 000 交付后工作区按符号核实；T11/T10 为本 replan 新调查：循环结构、根因三联、执行器查表面、裁剪序两路径全部代码闭合，链形态失败基线承自 Iteration 000 Review 的二进制实测且工作区该面未变化）；设计闭合（D4/D5/D10/D11 含否决路线）；任务契约齐备（T4/T5/T11/T10 全字段，自包含）；RTM 对应行 Covered 无 Missing；验证逐 scenario 最简直接判定、无身份型证据工程；Persisted Evidence `none` 满足白名单。计划批准：T4/T5 承 change 级批准（2026-10-04「批准」）；T11/T10 承用户 2026-10-05 replan 指令（原话「重新给出00cyc，把issue也计划解决进去」）；replan 计划整体于 2026-10-05 获用户审计批准（原话「批准」）——Gate 2 全项通过，Status: ready 交 Act。

**Persisted Evidence**

- Mode: none

（全部验证可由测试命令 + Act Response 承载；子查询、集合操作、链式 JOIN 与列序行为均为确定性复现，无需留存现场。）

**Risks and Notes**

- D4/D5/D10/D11 原始行号为规划/调查快照，Act 以符号定位为准。
- T4 投影位恒等门摘除后，`projection_columns` 与 `resolve_projection_indices` 在含子查询项时的交互（`_N` 伪名豁免）按既有 `select_list_is_pure_column_list(select, &subquery_indices)` 通道工作——若 S18 校准发现公式错位，属 Stop when 情形返回 Plan。
- T11 循环重构需末步判定（FROM 因子总数循环前可知）；中间步全组合承载使链形态 EXPLAIN 形状出现此前不存在的中间输出列清单——仅发生于此前必然失败的形态，预期零既有断言波及；若回归命中属实质信号返回 Plan。
- T11 限定键注册的右输入面（Scan 单表裸=限定同键；DerivedScan 右因子表名为空 → 限定键退化为裸名）为机械适配，行为契约以 S42–S46 为准。
- T10 重复项语义（`SELECT e.ename, e.ename` 产出两列）与既有 `any()` 折叠行为不同——对齐 SELECT 语义与 Scan 基座；若消费面（CTAS 重复列名守卫）命中既有拒绝面，属预期行为非回归。
- `subquery_test` 校准范围仅形状见证用例（`name_value_map` 系），语义断言不校准；校准 diff 全文记入 Act Response。
- 相关子查询绑定名与外层列同名冲突由 `extract_correlated_params` 既有机制承载；不够用属实质问题返回 Plan。
- T4 谓词逐行求值性能属 MS22 域，不进本 Cycle。

## Act Response

- Status: reported

### 实际改动

四任务全部交付，符号面与 Task Contract Targets 一致（行号为交付时工作区现场，Act 全程以符号定位）：

**T4（WHERE 标量子查询比较 + 投影位形状）**

- `src/parser/planner/query.rs`：新自由函数 `contains_scalar_subquery`（遍历变体集镜像 `contains_or` + Subquery/Cast/Case/Function/Trim/Ceil/Floor，InSubquery/Exists 腿与 Query 体内不进入）；单表 WHERE 链新臂（`try_build_where_subquery` 后、PK 提取前）：`bind_where_scalar_subqueries` 绑定 → `build_where` 编译改写表达式 → `ScalarSubqueryFilter` 节点；JOIN 基座拒绝条件扩展 `|| contains_scalar_subquery(where_expr)`（同文案 `WHERE CLAUSE ON JOIN`）；投影恒等门改组（`has_aggregates || (子查询项 ∧ 清单全为子查询项)` 恒等，混排形态按过滤后清单解析）；**相关参数取值列补充机制**（见偏差 3）：`subquery_param_columns`/`subquery_param_extras`、SELECT 投影尾部追加参数取值列、wrap 循环最外层节点 `output_width` 收尾截断、IndexScan「已收窄」列名同步附加；`get_plan_output_columns` 两臂（SubqueryEval 截断镜像、ScalarSubqueryFilter 直出 `columns`）；`is_base_scan_chain` 增 ScalarSubqueryFilter 臂。
- `src/parser/planner/subquery.rs`：`bind_where_scalar_subqueries`/`build_scalar_subquery_binding`（IN/EXISTS 同款 `building_subquery` + 相关参数提取）/`rewrite_scalar_subqueries`（深度优先左→右，`Expr::Subquery` → 保留名占位 `Expr::Function`，遍历集与检测一致）/`scalar_subquery_placeholder`。
- `src/parser/planner/expression.rs`：`build_expression` Function 臂顶部保留名臂——`__SCALAR_SUBQUERY_*` → `ParameterExpression`（同名小写）。
- `src/executor/plan.rs`：新变体 `ScalarSubqueryFilter(ScalarSubqueryFilterNode)` + 节点 `ScalarSubqueryFilterNode { input, predicate, bindings, columns, table_name, projection }` + `ScalarSubqueryBinding { param_name, subquery, correlated_params }`；`SubqueryEvalNode` 加性字段 `output_width`（T4 相关参数收尾截断，0 = 恒等）。
- `src/executor/scalar_subquery_filter.rs`（新）：`ScalarSubqueryFilterExecutor`——逐行绑定求值（非相关单次缓存/相关按参数值序缓存，错误不缓存）→ `inject_parameters` 按名注入 → 谓词求值 → `apply_projection` 裁剪；多行 `SubqueryReturnsMultipleRow` 既有错误面、零行 NULL。
- `src/executor/subquery_eval.rs`：`with_output_width` 构造 + 参数提取后按宽度截断再插列（0 = 既有行为逐字节）。
- `src/executor/plan_format.rs`：render/describe 两臂（下探输入与绑定子查询；`ScalarSubqueryFilter <table> (bindings=N, correlated_params=M, columns=[…])`）。
- `src/executor/correlated.rs`：注入臂（输入链 + 各绑定子查询；节点自身谓词占位由执行器逐行注入不在此消费）。
- `src/executor/mod.rs`：mod + `ScalarSubqueryFilterExecutor`/`ScalarSubqueryBinding`/`ScalarSubqueryFilterNode` 导出。
- `src/pipeline.rs`：`ScalarSubqueryFilter` 构造臂（外层列位映射按输入链全形状——谓词与相关参数在裁剪前行上取值）；`extract_column_indices` 新臂（ScalarSubqueryFilter 按 `columns`、Sort 投影感知、Limit 透传）与防御消息更新；`plan_output_names` 助手 + SubqueryEval 构造臂列位映射按收窄后行形状（`with_output_width` 接线）。
- 测试：新 `tests/scalar_subquery_comparison_test.rs` 14 例（S13–S18 矩阵：非相关 Eq/Gt/左腿/AND 混合、相关比较、重复参数一致、空子查询三值、多行拒绝、JOIN 拒绝、不下推计划见证、投影形状 ×4）。

**T5（FROM 派生表内集合操作）**

- `src/parser/planner/mod.rs`：PlanBuilder 加性字段 `building_from_derived`（init false）。
- `src/parser/planner/query.rs`：FROM 派生臂唯一置位（save/restore 与 `building_subquery` 严格配对，错误路径同恢复）；`build_set_operation` 守卫改 `building_subquery && !building_from_derived`（维持拒绝文案逐字节）；视图展开、SELECT 投影位两点复位 false。
- `src/parser/planner/subquery.rs`：IN/EXISTS 两臂与 T4 `build_scalar_subquery_binding` 复位 false（六入口 save/restore 全配对）。
- 测试：`tests/set_operations_test.rs` 扩展 7 例（S19 四操作派生源/外层叠加 + S21 IN/EXISTS/标量三维持拒绝）。

**T11（多表链式 JOIN 装配正确性）**

- `src/parser/planner/query.rs`（`build_from_clause_with_projection`）：循环 enumerate + `is_last` 判定；严格校验与 kept 收窄门 `is_last`（非末步强制全组合承载）；链前缀列 `column_index` 重定基为左输入行绝对偏移（累计宽度，链长 1 数值同 within-table——两表逐字节），右臂列保持 within-table（执行器按右输入行寻址）。
- `src/executor/join.rs`：`condition_column_index`——`(真实表, 列)` 限定键优先、裸名回退；`.expect` 防御臂保留。
- `src/pipeline.rs`：Join 构造臂注册限定键（`register_join_qualified_keys`/`collect_join_key_pairs`——限定键唯一命中才注册，`or_insert` 不覆盖既有条目；自连接歧义回落裸名既有语义）。
- 测试：新 `tests/chain_join_test.rs` 9 例（S42 hash+hash 链/链中间列绝对偏移、S43 hash+NLJ 混合链、S44 部分表 SELECT、S45 同名列链（三表同名 `id` 载荷错位夹具）、S46 四对照 + 真未知列错误面、EXPLAIN 渲染见证）。

**T10（JOIN 基座输出列序 SELECT 化）**

- `src/parser/planner/query.rs`：末步 kept 分支改按 SELECT 清单序重建（三清单同步、重复项重复产出、`*` 项与未命中项维持静默跳过）；`resolve_join_projection_indices` 改逐 SELECT 项定位命中索引（校验臂与错误面不变，`matches_select` 闭包删除）。
- 测试：`tests/join_output_resolution_test.rs` 扩展 5 例（S38 裸路径/S39 WHERE 路径/S40 ORDER BY 路径/S41 重复项 + `SELECT *`/S41 CTAS 列名随输出序 `SELECT *` 见证）。

### 与计划的偏差及原因

1. **T11 RED 基线形态与 replan 调查不符**：计划记载「三形态全坏——计划失败 ×2 / panic ×1」（HEAD 9538647 release 二进制实测）。Act 在 T9 交付后的工作区实测：三形态均为**静默错值**（`SELECT t1.a, t2.b, …` 的 t2.b 读到 t1.a——裸名键串列缺陷直击），无计划失败与 panic。差异源于 Iteration 000 T9 交付改变条件提取与输出归属面。RED 更强（正确性红线直击形态），修复目标与 Acceptance 不变，不影响契约执行。
2. **T4 增加相关参数取值列补充机制（计划未预见的必要实现）**：投影恒等门摘除后，SELECT 投影裁剪会裁掉相关子查询的取值列（参数按列名从外层行取值），既有相关面（subquery_test 缓存见证 5 例）当场回归。契约 Required（S18 形状）与 Preserve（既有相关面零回归）必须同时成立——最小实现为：SELECT 投影外的相关取值列**尾部附加**进输入投影，`SubqueryEvalNode` 加性 `output_width` 由链最外层节点收尾截断（参数列只到最外节点为止不泄漏进输出）。全部既有形态（无子查询项、全子查询项清单、`*` 混排、JOIN 基座恒等承载）走恒等路径零变化。属 Act 在契约内处理的非实质局部设计（不改变任何行为契约与 Preserve 项），非 Stop-when 情形（S18 公式无错位）。
3. **校准清单**：Plan 预定白名单（subquery_test `name_value_map` 系 5 处 + self_join_test S33 第二形态）之外，`tests/cli_test.rs::test_scalar_subquery_header_carries_alias_json` 1 处断言旧泄漏形状（表头 `[id, region, name, salary]` + 全行值）——该断言正是 S18 缺陷面的见证，按 T4 契约「未请求列不泄漏」校准为 `[id, region]`（diff 全文：列清单与行值收窄，语义断言零变化）。记入本 Response 作为白名单扩展。
4. **S46 预期内形态核实**：`t.*` 项在 kept/裁剪两路径维持既有静默跳过（镜像旧 `any()` 过滤语义），契约 Preserve 逐字节成立。

### 自检发现与修复（Gate 3）

1. T11 实现首轮将右臂列一并重定基 → 两表 9 例回归（S46 对照 + join 套件）当场暴露；修正为「左链前缀重定基 + 右臂 within-table」（与契约文字一致），复跑全绿。根因：执行器对 Right 侧列按右输入行寻址，Plan Investigation Facts 的「右臂列 within-table 不变」本已写明。
2. T10 resolve 首轮复用按**列**的 `matches_select` 闭包（匹配任意 SELECT 项）→ find 恒命中首个被任意项选中的列（WHERE/ORDER BY 路径得 `[1,1]`）；改为按**项**匹配后全绿，闭包删除。
3. 测试自身错误 3 处（非实现错误）：S17 计划见证误设 EXPLAIN 单行（树渲染每节点一行）且断言过宽（绑定子查询自身 DataScan 合法存在，改断言 `predicate=` 下推标记）；S18-别名用例期望值算错（全局 MAX=65000 非 Dave 本行值）；CTAS 用例首版断言不区分列序（改 `SELECT *` 直接暴露物化列序）。

### 遗留 Minor 问题

- Self-join × 三表链的限定键在该输入内歧义（`emp.id` 两臂同名）时不注册、回落裸名首匹配——契约明示的限定键注册规则内行为（「裸键仅无歧义时注册」同源），非回归；真实多表链场景键名跨表可区分，S45 覆盖。
- `ScalarSubqueryFilter` 的 EXPLAIN 渲染形状（columns 直出 + bindings/相关参数计数）为本 Cycle 新面，无既有断言约束，形状自定已记录。

### 验证

TDD 全程：每任务先 RED（T4 新矩阵 14 例全失败 `Unsupported expression type: EXPRESSION`/形状泄漏；T5 派生源 4 例失败/维持拒绝 3 例既有绿；T11 三链形态静默错值/两表对照先 GREEN；T10 四例表序失败/CTAS 对照）→ 实现 → GREEN。

- T4：`cargo test --test scalar_subquery_comparison_test --test subquery_test` → **14 passed + 32 passed**（exit 0）；定向回归 planner_test 107 / projection_expression_test 16 零失败。
- T5：`cargo test --test set_operations_test --test cte_test --test view_test --test subquery_test` → **21 + 14 + 15 + 32**（exit 0）；planner/explain 107/10 零失败。
- T11：`cargo test --test chain_join_test` → **9 passed**（exit 0）；join_test 14 / nested_loop_join_test 7 / join_output_resolution_test 9 / outer_join_test 16 / self_join_test 6 / cross_db_query_test 21 / table_alias_resolution_test 17 / cte_test 14 / view_test 18 / derived_scan_alias_test 19 / ctas_test 15 零失败。
- T10：`cargo test --test join_output_resolution_test` → **19 passed**（exit 0）；cli_test 119 / self_join_test 6 / ctas_test 15 / derived_scan_alias_test 19 / cross_db_query_test 17 / subquery_test 32 / chain_join_test 9 / join_test 14 零失败。
- 集成 Gate（分批串行，用户指令）：`cargo test --test …` 六批（批 1 本 Cycle 目标与 join/planner 族 323；批 2 恢复/WAL/checkpoint/索引 134；批 3 CLI 面 222；批 4 写面 171；批 5 执行器/表达式/读面 362；批 6 存储/MVCC/btree/网络 311）+ `cargo test --lib` 394 + `cargo test --bins` 0 → **全量 1917 passed / 0 failed / 2 ignored**，各批退出码 0（1882 基线 + 本 Cycle 净增 35：T4 新 14 + T5 新 7 + T11 新 9 + T10 新 5，精确吻合）。
- `cargo fmt --check` exit 0；`cargo clippy --tests` 0 新增警告（实施中引入的 6 条 Copy-clone/identical-if 警告已清零）。
- `openspec validate 2026-10-04-external-review-gaps` → valid；`openspec validate --specs` → 64 passed / 0 failed（WARNING 为 MS06–10 时代 spec 占位 Purpose 既有状态，非本轮引入）。

### Persisted Evidence

None required（Mode: none——全部验证由测试命令原生输出 + 本 Response 承载，输出可低成本复现）。

### Experience Candidates

None（本轮无 change 范围外、有证据的实质缺陷发现；T11 串列缺陷已随本任务解决不另立）。

### 未解决问题

None。Iteration 002（T6 无 WHERE 全表 UPDATE/DELETE、T7 CLI SQLSTATE）与 Iteration 003（T8 文档同步）待展开。

## Plan Review

- Review Result: accepted

**Findings**

独立审计范围：本 Cycle 四任务全部变更面独立读码与 diff 核对——`query.rs`（WHERE 链臂、`contains_scalar_subquery`、投影恒等门改组、参数取值列与 `output_width` 机制、装配循环 `is_last` 重构与重定基、kept 重建、`resolve_join_projection_indices`）、`subquery.rs`（绑定/改写/占位与六入口 save/restore）、`expression.rs`（保留名臂）、`plan.rs`/`scalar_subquery_filter.rs`/`subquery_eval.rs`/`plan_format.rs`/`correlated.rs`/`executor mod.rs`、`pipeline.rs`（构造臂、`plan_output_names`、限定键注册、`extract_column_indices` 扩展）、`join.rs`/`join_config.rs`/`nested_loop_join.rs`（侧归属与限定键查表）及 scalar_subquery_comparison/chain_join 两新测试文件与 set_operations/join_output_resolution/subquery/cli/self_join 校准面。Act Response 各项声明与代码逐一相符。

1. **偏差 1（T11 RED 基线）核实采信**：差异源于 T9 交付改变条件提取与输出归属面，工作区三形态静默错值（裸名键串列直击）较计划记载的「计划失败/panic」更强，修复目标与 Acceptance 不变。分类：基线变化，非阻塞。
2. **偏差 2（T4 相关参数取值列 + `output_width` 机制）独立核验通过**：补充列在输入投影尾部保持参数可达（`extract_param_values` 按收窄前行取值，pipeline `plan_output_names` 映射含补充列）、最外层 SubqueryEval 截断收尾（`get_plan_output_columns` 截断镜像同步）、取值列不可解析回落恒等——无 Preserve 项违反（`result_column_index` 公式、既有相关面、S18 形状三者同时成立）。分类：Act 契约内非实质局部设计，采纳。
3. **偏差 3（cli_test 校准）核实**：`test_scalar_subquery_header_carries_alias_json` 旧断言即 S18 泄漏面见证，校准为 `[id, region]` 与契约「未请求列不泄漏」一致，语义断言零变化。白名单扩展采纳。
4. **Minor（非阻塞）**：检测/改写遍历集不对称——`contains_scalar_subquery` 递归 `FunctionArg::Named` 而 `rewrite_scalar_subqueries` 不改写 Named 载荷；命名参数函数内嵌子查询的 WHERE 谓词将进入标量通道但未被改写，终以既有 `UnsupportedExpression` 拒绝（与变更前错误面逐字相同，安全拒绝零回归；S13–S18 场景集外）。仅记录，不构成修复项。
5. **Minor（spec 语料边界，非阻塞）**：SELECT 清单仅含标量子查询项（无基列混排）时维持恒等承载（全外层行 + 标量按位插列）——既有行为，且被既有 accepted 测试 `cte_test::two_scalar_refs_same_rows`（`[2,2,9]`）锁定；delta spec R4 第二 Requirement 正文「未请求的基表列 SHALL NOT 泄漏」按字面可覆盖该形态，但其场景与「SHALL 与其余纯列清单项共同构成输出形状」限定均指向混排形态。docs-maintainer 收尾合并时按场景限定口径处理；纯形态裁剪属范围扩展需另行决策，不在本 change。
6. **Minor（记录日期，非阻塞）**：Act Response 与 tasks.md 复选框标注「Act 2026-10-05 完成」，工作区证据（源文件末次写入 2026-10-06 14:31–14:56、cargo test 构建指纹 15:12–15:14、本 Response 落盘 15:16）显示最终验证实际于 2026-10-06 代码定稿后执行——验证新鲜成立（见 Evidence），日期标注偏差一天；历史现场字段不改写，仅记录。

**Acceptance Gaps**

None——S13–S21、S38–S46 全部有 GREEN 测试见证；R4/R5/R10/R11 spec delta 场景与测试一一对应；全量 1917 passed / 0 failed / 2 ignored；fmt/clippy 干净；`openspec validate` 通过。

**Evidence**

- 验证采信来源：本 Cycle Act Response「验证」节（T4 14+32、T5 21+14+15+32、T11 9 及十一套件、T10 19 及八套件、六批集成 Gate + lib 394 + bins 0 = 1917/0/2、fmt/clippy/validate）——新鲜度核验：源文件末次 mtime 2026-10-06 14:56 早于 cargo test 构建指纹 15:12–15:14，早于 Response 落盘 15:16，验证在最终代码状态之后执行，采信成立（公共规则 › 验证 异常审计「代码表面变化后定向核对」项，只读证据链核验，未重跑）。
- 独立代码核对（不以 Act 自检代替）：T4 占位名大小写链（`__scalar_subquery_<n>` → `__SCALAR_SUBQUERY_` 前缀 → lowercase ParameterExpression）闭合；`apply_projection` 空=恒等语义与 proj_or_empty 约定一致；`register_join_qualified_keys` 歧义集合在重复键二次插入情形下仍正确跳过；T11 重定基在链长 1 时数值等同 within-table（两表逐字节不变量成立）；`plan_output_names` 的 Filter 回落臂与既有 name→pos 映射等价；限定键收集与 `extract_column_indices` Scan 臂同约定（JOIN 输入 Scan 无投影）。

**Follow-up Decision**

accepted——既有 Acceptance 全部满足，四项偏差均为非阻塞 Minor 或已核实采纳的 Act 契约内处理；Iteration Plan 不变。finding 4/5/6 随本 Review 留档，不要求当前 Cycle 修复。

**Next Cycle**

None

**Next Iteration**

`iterations/002-write-cli/000-initial.md`（T6/T7，已按 Map 展开；Plan Context Status: draft，含 D7 发射语义勘误与闭合，待用户审计批准后转 ready 交 Act）
