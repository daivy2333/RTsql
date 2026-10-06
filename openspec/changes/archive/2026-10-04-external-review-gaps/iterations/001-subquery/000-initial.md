# Iteration 001 / Cycle 000: 子查询面收口批

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4（WHERE 标量子查询比较 + 投影位形状）、T5（FROM 派生表内集合操作）
- Acceptance gaps: None（initial）
- Repair items: None
- Inherited scope: proposal R4/R5 与场景 S13–S21；design D4/D5；Iteration 000 交付基线——T1（IndexScanAll 列契约）、T2（WAL 收敛标志链）、T3（JOIN ORDER BY 布局键：`resolve_order_by_layout_key` + `carry_all_columns` 加序 + `sort_projection` JOIN 臂）、T9（自连接侧身份：`expr_join_side` + `JoinSide`/`output_sides`）全部 accepted（Iteration 000 Cycle 001 Review，2026-10-04）；全量 1882 passed / 0 failed / 2 ignored（Cycle 001 Act Response 全量 Gate + Plan Review 定向复跑采信）
- Excluded scope: IN/EXISTS/标量子查询内的集合操作（spec 维持点名拒绝）；JOIN 基座 WHERE 内标量子查询（维持点名拒绝）；表达式项×子查询解锁；写面与 SQLSTATE（Iteration 002）；文档（Iteration 003）

**Objective**

WHERE 比较腿中的标量子查询（非相关 + 相关）端到端可用——空结果 NULL 三值语义、多行点名拒绝、含子查询谓词不下推有见证；SELECT 投影位标量子查询输出形状与请求一致（未请求列不泄漏）；FROM 派生表内 UNION [ALL]/INTERSECT/EXCEPT 可作派生源（外层组合照常、IN/EXISTS 内维持拒绝）。全量测试零回归。

**Investigation Facts**

- Current Baseline: 工作区 = Iteration 000 交付后状态（未提交，HEAD 9538647 + T1/T2/T3/T9 改动面）；全量 1882 passed / 0 failed / 2 ignored。行为基线：① WHERE 比较腿标量子查询 → `Unsupported expression type: EXPRESSION` exit 3（`Expr::Subquery` 在 `build_expression` 无臂）；② SELECT 投影位子查询可用但形状异常——全表列泄漏 + `__subquery` 按位插列（请求的投影被忽略）；③ `FROM (… UNION …)` → `Set operations are not supported inside subqueries` exit 3。
- Current-State Evidence（行号为 Iteration 000 交付后工作区现状，本 Iteration 展开时按符号重新定位核实）：
  - **表达式子查询无臂**：`src/parser/planner/expression.rs` 全文件无 `Expr::Subquery` 匹配臂，落入 catch-all `_ => Err(PlanError::UnsupportedExpression("EXPRESSION".into()))`（expression.rs:688）。`build_where` 与 `build_expression` 共享该编译入口。
  - **WHERE 顶层子查询机制（Preserve 面）**：`try_build_where_subquery`（subquery.rs:19 起）只匹配顶层 `Expr::InSubquery` / `Expr::Exists` 两形态；相关参数经 `extract_correlated_params`（subquery.rs:156 起）判相关并登记外层参数；表达式子查询上下文置位 `building_subquery`（subquery.rs:36/:85）。
  - **SELECT 投影位机制**：检测臂 query.rs:1320-1356（`UnnamedExpr(Expr::Subquery)` → 列名 `__subquery`；`expr AS alias` → 别名小写；`building_subquery` save/restore 内 `build_query` 编译子查询计划；`extract_correlated_params` 登记相关参数；产出 `subquery_evals: Vec<(idx, plan, col_name, params)>`）；DISTINCT×子查询点名拒绝 query.rs:1362-1364；投影位包装 query.rs:2722-2735（从右向左处理，`result_column_index = proj_idx - subqueries_before`）；**恒等门** query.rs:1785 `if has_aggregates || !subquery_evals.is_empty()`（注释 :1772 "SubqueryEval owns those shapes"——实际 SubqueryEval 只插列不裁剪 → 全表列泄漏，此即 S18 缺陷点）；混合聚合×子查询与表达式项×子查询拒绝在投影项分类臂（query.rs:1502/1515/1544/1556，`!building_subquery` 守卫）。
  - **形状异常见证（校准对象）**：`tests/subquery_test.rs:487` `name_value_map` helper 与 :513/:540/:565/:656-657 用例按异常形状写断言——T4 落地后按 S18 形状校准（白名单，仅形状见证用例）。
  - **表达式求值同步约束（D4 关键约束）**：`Expression::evaluate_ref` 同步且禁止 `.await`（predicate.rs trait 定义）→ 子查询值不能作为表达式变体懒求值；参数占位机制 `ParameterExpression` 已存在——`inject_parameters`（predicate.rs:43/:152/:245 trait 臂）与 `set_parameter_value`（predicate.rs:70/:154-155），消费先例 correlated.rs:68-72。
  - **执行器先例（复用机制）**：`SubqueryEvalExecutor`（subquery_eval.rs:23；语句级缓存 `correlated_cache: HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>` :43；多行 `SubqueryReturnsMultipleRow` :97/:168；首行首列提取逻辑 :78-109 区域）；SemiJoinExecutorV2 缓存同型（semi_join.rs）；非相关形态 = 空参数序列 → 单缓存项（每语句一次求值）。
  - **下推门（不下推见证锚点）**：JOIN 基座 `filter_projection` 含 `|| !subquery_evals.is_empty()` → 恒等承载（query.rs:1873-1881）；单表 `proj_or_empty` 同规则——SELECT 投影位子查询已不下推；T4 的 WHERE 检测命中后同样禁止谓词下推（镜像 `contains_or` 臂模式）。
  - **构造臂先例**：pipeline.rs:1762-1772 `PhysicalPlan::SubqueryEval` 构造臂（快照/跨库路由穿线方式）；pipeline.rs:2004/:2051 `extract_column_indices` 消费臂（新节点需加穷尽臂）。
  - **JOIN 基座 WHERE 子查询拒绝（Preserve 面）**：query.rs:1844 `UnsupportedStatement("WHERE CLAUSE ON JOIN")`——T4 不解锁该面（S17 断言对象）。
  - **T5 拒绝点**：query.rs:925-929（`build_set_operation` 顶部 `building_subquery` 守卫）。`building_subquery` 置位点 census：FROM 派生 query.rs:401-410、视图展开 query.rs:1236-1239、SELECT 投影位 query.rs:1344-1354、表达式子查询 subquery.rs:36/:85；CTE `consume_with`（query.rs:1111）在子查询内点名拒绝且不改标志。T5 拆「FROM 派生上下文」标志，仅该组合放行 :925 守卫。
  - **集合操作全链（D5 已核实零改动可行）**：`SetOperationNode.columns` = 左臂列名（plan.rs:793 起），`get_plan_output_columns` SetOperation 臂 query.rs:134-135 直取；DerivedScan 包裹 query.rs:398-429（物化行宽 = 左臂宽自洽）；`SetOperationExecutor` 对臂计划形状零假设（set_operation.rs）；plan_format 渲染臂 plan_format.rs:64/:328 与 correlated 参数注入臂 correlated.rs:103 均已存在；括号化操作数拒绝 query.rs:1094-1096/:1280-1282 保持；子查询内 DISTINCT 零消费 query.rs:1300（`Some(Distinct) => !building_subquery`）保持。
  - **测试面**：`tests/set_operations_test.rs` 现有 14 用例（MS25）；`tests/subquery_test.rs` 现有 28 用例；T4 依赖的 T3 交付面（`sort_projection` 的 `subquery_evals.is_empty()` 门 query.rs:2527）已就位。
- Code and Critical Path:
  - T4：新 helper（WHERE 表达式递归检测含 `Expr::Subquery`，镜像 `contains_or` 七变体扩展模式）→ `build_where` 前置分流：命中则禁止下推 + 谓词编译时 Subquery 腿替换为 `ParameterExpression` 占位 + bindings 登记（子查询计划经 `building_subquery` 上下文 `build_query` 编译，同 SELECT 投影位检测臂模式）→ 新计划节点 `ScalarSubqueryFilterNode { input, predicate, bindings: Vec<(String, PhysicalPlan)> }`（plan.rs + plan_format/correlated/get_plan_output_columns 穷尽消费臂）→ 新 `ScalarSubqueryFilterExecutor`：逐行提取参数值 → 语句级缓存（键 = 参数值序列，非相关恒空 = 单缓存项）→ miss 时 bindings 逐项 `plan.clone()` + `inject_correlated_values`（correlated.rs）+ 重建执行器 + drain 取首行首列（多行 → `SubqueryReturnsMultipleRow`、零行 → NULL，逻辑自 subquery_eval.rs 摘出复用）→ `set_parameter_value` 注入占位 → 对当前行求值谓词 → 匹配输出 → pipeline 构造臂（快照/路由照搬 SubqueryEval 先例）。投影位：query.rs:1785 恒等门摘除 `|| !subquery_evals.is_empty()`（`result_column_index` 公式 :2722-2735 不变，裁剪生效后恰好正确）。
  - T5：`building_subquery: bool` 拆双标志（FROM 派生上下文单独承载），仅 FROM 派生组合放行 query.rs:925 守卫；视图展开 / 表达式子查询（IN/EXISTS/标量）/ SELECT 投影位组合维持拒绝。全链其余零改动。

**Implementation Guidance**

顺序：T4（中-大）→ T5（小）。T4 内：先 RED（比较腿当前 `Unsupported expression type: EXPRESSION`）→ 检测 helper + 占位替换 → 节点 + 执行器 → 构造臂 → 投影位门摘除（形状用例先 RED）→ `subquery_test` 形状校准。子查询编译、相关参数、缓存与多行/零行语义一律复用既有机制（SubqueryEval / SemiJoin / Correlated 先例），不新造命名或缓存规则；快照与跨库路由照搬 pipeline.rs:1762 构造臂穿线。T5 独立：拆标志时 save/restore 严格配对（三置位点现有配对模式照搬），错误文案不变。两任务均不建身份型证据工程；测试用项目既有测试栈。

**Task Contracts**

### T4: WHERE 标量子查询比较 + 投影位形状

- Requirement/Scenario: R4 / S13–S18
- Depends on: T3（Iteration 000 已交付——JOIN `sort_projection`/`filter_projection` 共享门已就位）
- Targets: `src/parser/planner/expression.rs`（Subquery 腿占位替换）、`src/parser/planner/query.rs`（WHERE 检测分流、:1785 恒等门）、新计划节点与执行器（`src/executor/plan.rs` + 新模块）、`src/pipeline.rs`（构造臂 + 穷尽消费臂）、`tests/scalar_subquery_comparison_test.rs`（新）、`tests/subquery_test.rs`（形状校准）
- Current behavior: S13 `WHERE salary = (SELECT MAX(salary) FROM emp)` → `Unsupported expression type: EXPRESSION` exit 3；S18 `SELECT name, (SELECT MAX(salary) FROM q6) FROM q6` 全表列泄漏 + `__subquery` 位次插列。
- Required behavior: S13/S14 非相关 + 相关比较返回正确行；S15 空子查询 → NULL → 比较三值语义（Unknown → 行不匹配）；S16 多行 → `SubqueryReturnsMultipleRow` 既有错误面；S17 JOIN 基座 WHERE 内维持点名拒绝且含子查询谓词不下推（计划形状见证）；S18 表头 `["name","__subquery"]`（或别名）、未请求列不泄漏、行形状与请求一致。
- Preserve: 顶层 InSubquery/Exists/SemiJoin/AntiJoin 机制零变化；DISTINCT×子查询、聚合×子查询、表达式项×子查询三拒绝面；JOIN 基座 WHERE 子查询拒绝；`result_column_index` 公式；SubqueryEval 投影位语义；`evaluate_ref` 同步语义；plan cache 既有规则（含子查询语句行为与现状一致）。
- Forbidden: 不得给 `Expression` 增加异步求值变体；不得解锁 JOIN 基座 WHERE 子查询；不得改 InSubquery/Exists 通道语义；不得给 `get_subquery_first_column` 加 SetOperation 臂（T5 范围收口）。
- Test witness: 新 `tests/scalar_subquery_comparison_test.rs`（S13–S18 矩阵 + 非相关单次求值见证 + 不下推计划形状见证）——先写 RED（当前 `UnsupportedExpression` 即 RED；S18 当前泄漏形状即 RED）。`subquery_test` 形状见证用例按 S18 校准（diff 记入 Act Response）。
- GREEN condition: 新矩阵全绿 + subquery_test 校准后全绿 + correlated/planner/subquery 既有套件定向回归零失败。
- Verification: `cargo test --test scalar_subquery_comparison_test --test subquery_test` + 定向回归 + 全量 `cargo test` 0 failed。
- Stop when: 占位替换需要改 `Expression` trait 异步语义；或投影位公式在裁剪生效后错位（说明 D4 形状分析失效）；或相关参数注入与谓词求值语义冲突 → 返回 Plan。

### T5: FROM 派生表内集合操作

- Requirement/Scenario: R5 / S19–S21
- Depends on: None
- Targets: `src/parser/planner/query.rs`（`building_subquery` 标志拆分 + :925-929 守卫 + 三置位点配对适配）
- Current behavior: `FROM (SELECT … UNION SELECT …)` → `Set operations are not supported inside subqueries`。
- Required behavior: S19 FROM 派生表集合操作可执行（UNION 去重 / UNION ALL / INTERSECT / EXCEPT，列名取左臂）；S20 外层投影/WHERE/聚合/ORDER BY 叠加照常；S21 IN/EXISTS/标量子查询内集合操作维持点名拒绝。
- Preserve: 视图展开与表达式子查询（IN/EXISTS/标量）上下文拒绝；SELECT 投影位子查询上下文拒绝；括号化操作数拒绝；子查询内 DISTINCT 零消费；CTE `consume_with` 行为；`SetOperationExecutor`/`SetOperationNode` 零改动；`get_subquery_first_column` 不加臂。
- Forbidden: 不得解锁 IN/EXISTS/标量子查询内集合操作；不得改错误文案（既有 `Set operations are not supported inside subqueries` 对维持拒绝的上下文逐字节不变）。
- Test witness: `tests/set_operations_test.rs` 派生表矩阵（S19–S21）——先写 RED（当前点名拒绝即 RED）。
- GREEN condition: 新矩阵全绿 + set_operations/cte/view/subquery 既有套件零回归。
- Verification: `cargo test --test set_operations_test --test cte_test --test view_test --test subquery_test` + 全量 `cargo test` 0 failed。
- Stop when: 拆标志波及视图/表达式子查询上下文语义（既有套件回归命中）→ 返回 Plan。

**Invariants**

- 存储格式、网络协议字节、plan cache key 语义零变化。
- 全部既有 spec 语料行为零回归（本 Iteration delta：`sql-expression-evaluation` ADDED ×2、`sql-set-operations` ADDED，均在 change specs/ 在案）。
- 不建身份型证据工程/判定层；验证判定使用项目测试框架原生退出码与输出。
- Iteration 000 已交付面（IndexScanAll 契约、WAL 收敛标志链、JOIN 布局键与侧身份）不在本 Cycle 变更面内。

**Non-goals**

- IN/EXISTS/标量子查询内集合操作（范围收口，spec 明示拒绝）。
- JOIN 基座 WHERE 内标量子查询解锁。
- 表达式项×子查询、窗口函数、递归 CTE。
- 写面与 SQLSTATE（Iteration 002）；文档（Iteration 003）。

**Acceptance**

- S13–S21（proposal 场景草图）全部可观察通过；R4/R5 的 spec delta 场景与任务测试一一对应（RTM 见 tasks.md）。
- 全量 `cargo test` 0 failed；`cargo fmt --check` 与 `cargo clippy --tests` 干净（fmt 在全量结论之前执行，或执行后对受影响面定向复跑）。

**Verification**

- 直接目标：`cargo test --test scalar_subquery_comparison_test --test set_operations_test`（新增矩阵 RED→GREEN）。
- 受影响边界：`cargo test --test subquery_test --test cte_test --test view_test --test correlated_subquery_test --test planner_test`（subquery_test 含形状校准白名单）。
- 集成 Gate：全量 `cargo test` 0 failed。
- 判定：各命令退出码 0 + 原生输出行数统计，不新增判定脚本；套件计数按 `Running tests/<name>` 行对应记录（避免 Iter000 Cycle001 的计数误标形态）。

**Gate 2 Readiness**

PASS——调查完整（本展开按符号重新定位全部锚点至 Iteration 000 交付后工作区现状；基线引用 Cycle 001 Act Response + accepted Review）；设计闭合（D4/D5 含否决路线 a/b 与范围收口）；任务契约齐备（T4/T5 全字段，自包含）；RTM 对应行 Covered 无 Missing；验证逐 scenario 最简直接判定、无身份型证据工程；Persisted Evidence `none` 满足白名单。计划批准来源：change 级批准（2026-10-04 用户原话「批准」，含 proposal 四项决策与 Iteration Plan）+ Iteration 000 accepted Review 基线，无新增待批范围。

**Persisted Evidence**

- Mode: none

（全部验证可由测试命令 + Act Response 承载；标量子查询与集合操作行为均为确定性复现，无需留存现场。）

**Risks and Notes**

- D4/D5 原始行号为 change 规划时快照；本 Plan Context 的行号已按 Iteration 000 交付后工作区重新定位，Act 仍以符号定位为准。
- T4 投影位恒等门摘除后，`projection_columns` 与 `resolve_projection_indices` 在含子查询项时的交互（子查询伪名 `_N` 豁免）按既有 `select_list_is_pure_column_list(select, &subquery_indices)` 通道工作——若 S18 校准发现公式错位，属 Stop when 情形返回 Plan，不在 Act 内改公式。
- `subquery_test` 校准范围仅形状见证用例（`name_value_map` 系），语义断言不校准；校准 diff 全文记入 Act Response。
- 相关子查询绑定名与外层列同名冲突由 `extract_correlated_params` 既有机制承载，不新造命名规则；若该机制对比较腿占位形态不够用，属实质问题返回 Plan。
- T4 谓词逐行求值位于热路径：非相关形态单缓存项使其与 SubqueryEval 每语句一次同量级；性能量化属 MS22 域，不进本 Cycle。

## Act Response

- Status: pending

## Plan Review

- Review Result: pending
