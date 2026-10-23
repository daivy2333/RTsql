# Iteration 001 / Cycle 000: 占位符与类型化参数执行

## Plan Context

- Status: ready（Gate 2 PASS，2026-10-22 用户批准「批准」）
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T5, T6, T7, T8（change `tasks.md` Iteration 001）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal What Changes 2 前半（MS31-T02 参数面）+ delta spec `sql-parameter-binding` R1 全部七场景 + design D5/D6/D8；Iteration 000 的 `SqlOutcome`/`SqlError` 形态与 `execute_stage[_in_tx]` 通道
- Excluded scope: 预编译与失效契约（Iteration 002）、DML WHERE 键位参数可达化（design R4——需执行期键求值重构，独立设计，本 change 不预留半成品）、多套占位符方言（`?N`/`$N` 维持拒绝）、字符串替换式绑定、MS32/MS22 范围

**Objective**

SQL 文本位置式裸 `?` 在全可达位（SELECT 谓词/投影项、UPDATE SET 右值、INSERT/UPSERT 值位）绑定原生 `Value` 参数执行；参数个数/类型错误矩阵与 NULL 三值语义锁定；不可达位（DML WHERE 定位键位、标识位、DDL、`?N`）计划期点名拒绝、零副作用；无占位符语句全链行为逐字节不变；全量零回归。

**Background**

MS31-T02 前半（I089 promoted）。依赖 Iteration 000 的类型化结果形态（accepted，2026-10-22）。

**Investigation Facts**

- Current Baseline: Iteration 000 accepted 终态（工作区未提交，HEAD `671f25c` 之上叠加）：typed 入口 `Database::execute_sql`（`src/database.rs:429`）/ `execute_in_tx`（`:538`）返回 `Result<SqlOutcome, SqlError>`；全量 2081/0/2（来源：Iteration 000 最终 Act Response 修复轮验证 + accepted Review 采信）
- Current-State Evidence（2026-10-22 逐项核对；design.md 2026-10-21 调查结论经本轮锚点复核仍成立）:
  - **占位符现状**：tokenizer 无方言门将 `?`（可带数字）tokenize 为 `Token::Placeholder`、`parse_value` 映射 `Value::Placeholder`（sqlparser-0.44 GenericDialect 可达）；现状 `value_from_sqlparser`（`src/parser/value.rs:28`）`_ => Err(PlanError::UnsupportedValue)` 计划期拒绝——含占位符语句的当前行为基线（值位拒绝、无个数概念）
  - **参数执行既有件**：`ParameterExpression { param_name: String, value: Mutex<Value> }`（`src/executor/predicate.rs:427`，`new` :433）owned `evaluate` 返真值（MS30 T03-D2，复合表达式经 owned 路径递归）；`evaluate_ref` 对 String/Date/Timestamp 降级 Null（值借用通道已知限制）；注入链 `Expression::set_parameter_value` / `inject_parameters`（trait 默认 no-op、复合臂实现）+ `inject_correlated_values(plan: &PhysicalPlan, param_values: &[(String, Value)])`（`src/executor/correlated.rs:14`）——Filter/Having/SemiJoin/AntiJoin/SubqueryEval/ScalarSubqueryFilter/DataScan（含下推谓词）/Sort/Limit/Aggregate 等 SELECT 树臂遍历注入
  - **命名空间**：相关参数 `"{table}.{column}"`、标量子查询 `__scalar_subquery_{index}`——`__param_{i}`（0 基、AST 出现序）与两者无碰撞（design D5）
  - **DML 值表示**：`InsertNode.values`（`src/executor/plan.rs:194`）/ `UpsertNode.values`（`:243`）`Vec<Vec<Value>>` 计划期全物化；`UpsertValueExpr = Literal/Excluded/Old`（`:208`）；`UpdateAssignment { column, value: UpdateValue }`、`UpdateValue = Literal(Value) | Expression(ExpressionRef)`（`:278-303`）；UPDATE SET 右值 `Expr::Value(v)` → `UpdateValue::Literal(value_from_sqlparser(v)?)`（`src/parser/planner/ddl_dml.rs:2005`）
  - **DML WHERE 键位**：`build_update`（`ddl_dml.rs:1960`）带 WHERE 经 `extract_pk_from_where`（`:1979` 调用；DELETE 臂 `:2094`）提取 PK，失败报 `UPDATE requires primary key equality in WHERE clause`——`WHERE a = ?` 现状落入该一般性拒绝（非参数点名）；无 WHERE → 全表形态（`key: None`）
  - **执行通道与缓存**：`execute_inner` cache get（`src/pipeline.rs:1226`）→ parse → 单语句检查 → `plan_stage` → execute_stage；`plan_stage` 的 put 门 `is_cacheable(stmt) && routed.is_empty()`（`:291-292`，`is_cacheable` `:3203`——仅 Query 进缓存）；DDL 八臂 + CTAS 执行后 `clear()`。**含占位符语句的计划不得写入 plan cache 也不得消费 cache get**——否则跨执行参数值残留（design D6「plan 为本次新建」的显式化，本契约定为硬边界）
- Code and Critical Path: T5 计数 walk（`src/parser/` AST 面）→ pipeline 校验位点；T6 表达式臂（`src/parser/planner/expression.rs`）+ DML 槽位（`ddl_dml.rs` + `src/executor/plan.rs` 节点加字段）+ pipeline 物化位点；T7 `src/database.rs` 新入口 + `src/pipeline.rs` 通道（parse → 计数校验 → 新建 plan → 注入 → 物化 → execute_stage）。数据流：`params: &[Value]` → `__param_{i}` 注入（SELECT 树）/ 槽位物化（DML values[r][c]）→ 执行器在真实值上跑既有门（类型/NOT NULL/键列/索引维护）

**Implementation Guidance**

- 建议顺序 T5 → T6 → T7 → T8（T5 建立拒绝面与计数基建，T6 在其上编译占位符，T7 打通入口，T8 全量）。
- 表达式位占位符经 PlanBuilder 计数器分配 `__param_{i}`；计数器与 `count_placeholders` 的 AST 出现序必须同源同序（同一遍历规则），否则注入错位。
- DML 值位槽位物化时机在 pipeline DML 臂构造执行器前；`Value::Null` 占位只豁免计划期检查，执行期门在物化后的真实值上运行——与字面量语句同源（design D5）。
- `execute_sql`/`execute_in_tx` 语义等价 `params = &[]`：含占位符语句无参执行报个数错误（design D5）——注意这改变了占位符值位的既有错误面（`UnsupportedValue` → 个数点名），属能力新增面的合法收敛。

**Task Contracts**

### T5: AST 占位符计数与不可达位拒绝面（可验证结果：拒绝矩阵成立）

- Requirement/Scenario: sql-parameter-binding R1「参数个数不符」「DML 定位键位占位符点名拒绝」「不可达位置点名拒绝」
- Depends on: None
- Targets: `src/parser/` 新 walk `count_placeholders(&Statement) -> usize`（AST 级，parse 后即可校验；落点 `src/parser/mod.rs` 或 planner 公共面）、`src/pipeline.rs`（`execute_inner`/`execute_in_tx` parse 后校验位点）、`src/parser/planner/ddl_dml.rs`（WHERE 键位点名拒绝——`build_update`/`build_delete` 的 WHERE 面前置探测占位符）
- Current behavior: `?` 在值位经 `value_from_sqlparser` → `UnsupportedValue`（`src/parser/value.rs:28`）；`WHERE a = ?` → `UPDATE requires primary key equality in WHERE clause`（一般性文案）；`?N` 与 `$N` 同走 UnsupportedValue；无个数校验
- Required behavior: parse 后 `count_placeholders` 得 N；`?N`（`Placeholder("?N")`）与 `$N` 点名拒绝（携带能力名，一套位置式方言）；个数不符（含 `execute_sql` 无参路径 N>0）在 plan 前点名「期望 N 实际 M」错误、零副作用；DML WHERE 定位键位占位符计划期点名拒绝（指明「定位键为计划期物化、参数执行期才可得」的不可达原因，区别于一般性 PK 文案）；DDL 语句内占位符点名拒绝（携带能力名）；标识位占位符维持 sqlparser 既有 parse 拒绝面、不静默
- Preserve: 无占位符语句（N=0）全部既有错误面与行为逐字节——计数 walk 对 N=0 语句零副作用；`WHERE a = 1` 等字面量提取链零变化
- Forbidden: 执行器改动；plan cache 语义（T6 契约位除外）；字符串替换式绑定
- First test: `tests/param_binding_test.rs`（新文件）拒绝矩阵先行——①个数不符（2 占位符 1 参数）执行前点名+零副作用；②`WHERE a = ?` UPDATE/DELETE 点名拒绝；③DDL 含 `?` 点名；④`?N` 拒绝。RED：`execute_with_params` 不存在（编译失败）——T5 阶段可先以既有入口断言 `?N`/DDL 拒绝面与键位点名（现文案缺失，真实 RED）
- GREEN condition: 拒绝矩阵全过；`cargo test --lib` 零回归
- Verification: `cargo test --test param_binding_test`（定向）
- Stop when: GenericDialect 对某位形（如标识位）在 parse 层即拒绝导致场景不可构造——按实际可达集记录后收口该子面

### T6: 计划面占位符编译（可验证结果：占位符编译为参数表达式/值槽）

- Requirement/Scenario: sql-parameter-binding R1「占位符等值过滤」「INSERT/UPDATE 值位绑定」（计划面编译部分）
- Depends on: T5（计数器与拒绝面）
- Targets: `src/parser/planner/expression.rs`（`Value::Placeholder("?")` 臂 → `ParameterExpression::new("__param_{i}")`，PlanBuilder 计数器分配、出现序与 `count_placeholders` 同源）；`src/parser/planner/ddl_dml.rs`（INSERT VALUES / UPSERT 待插值与 DO UPDATE SET Literal 臂的槽位登记；UPDATE SET `Expr::Value` 遇 `?` 改路由 `UpdateValue::Expression(ParameterExpression)`）；`src/executor/plan.rs`（`InsertNode`/`UpsertNode` 增 `param_slots: Vec<ValueParamSlot>` 字段——`ValueParamSlot { row: usize, col: usize, param_index: usize }`，Upsert DO UPDATE 赋值独立槽位形态）；`src/pipeline.rs`（DML 臂构造执行器前按参数物化 `values[r][c] = params[i]`）
- Current behavior: 占位符在全部值位被 `value_from_sqlparser` 拒绝（见 T5）；计划节点无参数概念
- Required behavior: 表达式位 `?` 编译为 `ParameterExpression`（owned evaluate 真值通道）；DML 值位槽位置 `Value::Null` 占位 + `param_slots` 登记；**含占位符语句的计划不写入 plan cache（put 门排除）且参数通道不消费 cache get**——杜绝跨执行参数值残留；无占位符语句的计划构造逐字节不变（Placeholder 臂此前不可达，新增臂对既有路径零影响；`param_slots` 空 Vec 对既有计划同构）
- Preserve: 类型门/NOT NULL/键列门/唯一与次级索引维护全部沿用执行器既有语义（占位 Null 豁免计划期检查、执行期门在真实值上跑）；`PhysicalPlan: Clone` 等派生维持；PhysicalPlan 节点类型集不变（仅既有节点加字段）
- Forbidden: 执行器接口重构；执行器 `next()` 协议变更；`?N` 支持；多套方言
- First test: `tests/param_binding_test.rs` 计划面先行——`SELECT … WHERE a = ?` 计划可建成且两次不同参数执行结果各自正确（行为断言锁定无值残留）；`INSERT INTO t VALUES (?, ?)` 计划含槽位（经带参执行后读回验证）
- GREEN condition: 上述测试过；`cargo test --lib` 零回归
- Verification: `cargo test --test param_binding_test` + `cargo test --lib`
- Stop when: 值位物化点需要触及执行器内部（应在 pipeline DML 臂完成——若计划节点形态迫使执行器改动，回 Plan）

### T7: 参数执行通道与错误/NULL/TEXT 真值矩阵（可验证结果：入口与语义矩阵成立）

- Requirement/Scenario: sql-parameter-binding R1「占位符等值过滤」「INSERT/UPDATE 值位绑定」「参数类型不符」「NULL 参数三值」
- Depends on: T5, T6
- Targets: `src/database.rs`（`execute_with_params(&self, sql: &str, params: &[Value])` / `execute_in_tx_with_params(&self, sql, params, tx)`）；`src/pipeline.rs`（通道：parse → `count_placeholders` 校验 → 新建 plan（不经 cache）→ SELECT 树 `inject_correlated_values` 同款 walk 注入 `__param_{i}` → DML 臂槽位物化 → `execute_stage[_in_tx]`）
- Current behavior: 无参数入口；占位符语句被拒（T5 前）
- Required behavior: 等值过滤与值位绑定的结果/写入/读回与等价字面量语句一致；类型不符拒绝语义与字面量类型门同源（`ColumnTypeMismatch`）；NULL 参数三值（等值比较 Unknown 折叠不返回行、`IS NULL` 按真实 NULL 生效）；TEXT 参数按真实值求值（owned evaluate 通道——MS30 T03-D2 面）；顶层投影位 `SELECT ?` 的求值通道由先行测试锁定：TEXT 参数真值可达即断言真值，若实际触发 `evaluate_ref` String 降级则该位点名拒绝并在 Act Response 记录（design R1/D6 预设处置）；显式事务 `execute_in_tx_with_params` 的 DML 事务语义与既有 `execute_in_tx` 一致（design D8）
- Preserve: 既有 `execute_sql`/`execute_in_tx` 语义（除「占位符值位 UnsupportedValue → 个数点名」的既定收敛外零变化）；错误文案与 SQLSTATE 分类既有契约；plan cache 对无参语句零变化
- Forbidden: 新隔离语义；字符串替换式绑定；prepare/PreparedStatement（Iteration 002）
- First test: `tests/param_binding_test.rs` 行为矩阵（真实入口 `Database::execute_with_params`）：等值过滤、值位绑定（INSERT/UPDATE 各含类型门/NOT NULL 拒绝形态）、类型不符、NULL 三值、TEXT 真值、事务内执行。RED：入口不存在（编译失败）
- GREEN condition: 矩阵全过；`cargo test --lib` + `cargo test --test param_binding_test` 零回归
- Verification: 同上定向命令
- Stop when: 注入 walk 对某计划臂不可达（含参语句构造出 walk 未覆盖的节点形态）——回 Plan 定臂面

### T8: 全量分批回归（M19）预算点（纯验证任务）

- Requirement/Scenario: 全量零回归（change 级稳定基线）
- Depends on: T5, T6, T7
- Targets: 无代码变更
- Current behavior: N/A
- Required behavior: M19 分批全量回归（字典序分组、每批 ≤12 目标、批内 `--test-threads=1`、批间串行、限并发编译 `cargo build --lib --tests -j 2`、可用内存 <1.4 GiB 等待守卫——Iteration 000 实测定型纪律）全部 0 failed；验证记录注明分批方式与各批计数
- Preserve: N/A
- Forbidden: 为通过回归修改既有断言（发现失败按 T5-T7 契约修复后重跑受影响批）
- First test: 不适用（纯验证任务；SKIPPED: 验证任务无先行测试）
- GREEN condition: 全部批次各自 0 failed（缺一批不得声明全量通过）
- Verification: 分批 `cargo test --test <name> -- --test-threads=1` 序列 + `cargo test --lib`
- Stop when: 任一批 failed——契约内定位修复重跑该批，契约外回 Plan

**Invariants**

- 无占位符语句全链行为逐字节不变（错误面、计划形状、plan cache 行为）。
- `render.rs`、`pg_protocol.rs`、JSON 序列化零改动；执行器 `next()` 协议与全部执行器实现零改动（Insert/Upsert 经槽位物化后消费既有形态）。
- plan cache：无参语句语义零变化；含占位符语句不入不消费（跨执行值残留杜绝）。
- 错误文案与 SQLSTATE 分类沿 error-reporting 域既有契约；新增错误点名携带能力名/位置/期望实际个数。
- 绑定不以字符串替换实现（参数经类型化 `Value` 注入）。

**Non-goals**

- 预编译语句与 schema 版本失效契约（Iteration 002）。
- DML WHERE 键位参数可达化（design R4——执行期键求值重构，独立设计）。
- `?N`/`$N` 方言、多占位符方言（维持拒绝）。
- 顶层投影位参数的静态推导（求值通道以先行测试实测锁定，design R1 预设两种处置）。

**Acceptance**

| Requirement | Scenario | 验证载体 |
|---|---|---|
| sql-parameter-binding R1 | 占位符等值过滤 | T6/T7 先行测试（`tests/param_binding_test.rs`，真实入口） |
| 同上 | INSERT/UPDATE 值位绑定 | T6/T7 先行测试（含类型门/NOT NULL/索引维护一致性形态） |
| 同上 | DML 定位键位占位符点名拒绝 | T5 拒绝矩阵 |
| 同上 | 参数个数不符 | T5 拒绝矩阵（零副作用断言） |
| 同上 | 参数类型不符 | T7 行为矩阵（`ColumnTypeMismatch` 同源） |
| 同上 | NULL 参数三值 | T7 行为矩阵 |
| 同上 | 不可达位置点名拒绝 | T5 拒绝矩阵（DDL/标识位/`?N`） |
| 全量零回归 | — | T8 M19 分批 |

**Verification**

- 行为类 scenario 全部走真实入口（`Database::execute_with_params` / `execute_in_tx_with_params`），先行测试即集成测试。
- 全量分批回归（M19）在 T8 预算一次；批内失败按契约修复后重跑受影响批。

**Gate 2 Readiness**

PASS（RTM 无 Missing/Simplified；D5/D6/D8 设计闭合；锚点经 2026-10-22 实际代码复核；Cycle 与 tasks/specs/design 一致；Persisted Evidence = none 满足白名单）。Status 维持 draft，等待用户批准后转 ready。

**Persisted Evidence**

- Mode: none

全部验证为 `cargo test` 集合内行为断言，可低成本重跑；无一次性环境与现场保留需求；Act Response 承载命令与结果足够。

**Risks and Notes**

- 顶层投影位 `SELECT ?` 的求值通道为预设锁定点（design R1）：先行测试实测二选一（TEXT 真值断言或该位点名拒绝），处置已定稿、不阻塞 Gate 2。
- plan cache 排除（put 门 + 参数通道不走 cache get）为 design D6「plan 为本次新建」的 Plan 侧显式化——防跨执行参数值残留，属契约明确化而非设计变更。
- 计数器与 `count_placeholders` 出现序同源是注入正确性的前提（Implementation Guidance 已记）；若 Act 发现两遍历无法同序，回 Plan。
- Iteration 000 遗留 MF-1（`join_type_slots` 文档注释）不在本 Cycle 契约内——仅当 T6/T7 合法触达该函数邻域时可顺带校准，不单独立项。

## Act Response

- Status: reported
- Cycle: Iteration 001 / Cycle `000-initial`（initial）
- Tasks: T5, T6, T7, T8 —— 全部完成（Gate 3 逐任务 Review 通过、Gate 4 验证满足）
- Date: 2026-10-22

### 实际改动

**新增模块**

| 文件 | 符号 | 职责 |
|---|---|---|
| `src/parser/placeholder.rs`（新） | `PARAM_PREFIX` / `assign_placeholder_indices(&mut Statement) -> Result<usize, PlanError>` / `count_placeholders(&Statement) -> usize` / `expr_has_placeholder(&Expr) -> bool` / `placeholder_index` / `placeholder_param_name` / `is_placeholder_value` / `unsupported_placeholder_message` | AST 级占位符遍历：裸 `?` 按 **SQL 文本序** 改写为 `__param_{i}` 标记并计数（同源同序）；非裸形态（`?N`/`$N`/`:name`/`@name`）与 DDL 内占位符点名拒绝；标识位维持 sqlparser 既有 parse 拒绝面 |
| `src/parser/mod.rs` | `pub mod placeholder` + 三个 re-export | 模块挂载与公共面 |
| `tests/param_binding_test.rs`（新，28 条） | T5 拒绝矩阵 + T6 计划面 + T7 行为矩阵 | 真实入口（`Database::execute_with_params` / `execute_in_tx_with_params`）断言 |

**计划面编译（T6）**

| 文件 | 改动 |
|---|---|
| `src/parser/planner/expression.rs` | `Expr::Value(v)` 臂：占位符 → `ParameterExpression::new("__param_{i}")`（owned evaluate 真值通道）；未编号形态点名拒绝 |
| `src/parser/planner/ddl_dml.rs` | `InsertValue::Param(usize)` 新形态；`extract_insert_values` 识别占位符；`map_insert_values` 登记 `ValueParamSlot`（行位 × 列位 → 参数序号，计划期该位置 `Value::Null` 占位）；`build_upsert_value` → `UpsertValueExpr::Param(index)`；`build_update` SET 右值 → `UpdateValue::Expression(ParameterExpression)`；`build_update`/`build_delete` 前置 `reject_parameterized_key_clause`（DML WHERE 定位键位点名拒绝，指明「定位键为计划期物化、参数执行期才可得」）；`build_upsert_action`/`build_insert` 传 `param_slots` |
| `src/parser/planner/query.rs` | `extract_pk_from_where` / `extract_secondary_index_equality` / `has_non_keyable_pk_literal_leg` 遇占位符位改为「计划期无值 → 不提取键 / 不路由索引 / 非字面量腿」（原经 `value_from_sqlparser` → `UnsupportedValue`，致参数化等值过滤不可达） |
| `src/executor/plan.rs` | `ValueParamSlot { row, col, param_index }` 新类型；`InsertNode.param_slots` / `UpsertNode.param_slots` 新字段；`UpsertValueExpr::Param(usize)` 新变体 |
| `src/executor/upsert.rs` | `UpsertValueExpr::Param` 臂——执行器只见物化后形态，未物化即 `StorageError::Internal` 点名 |
| `src/executor/plan_format.rs` | `UpsertValueExpr::Param` 的 EXPLAIN 渲染臂（`?{i}`） |

**执行通道（T7）**

| 文件 | 改动 |
|---|---|
| `src/database.rs` | `Database::execute_with_params(sql, &[Value])` 与 `Database::execute_in_tx_with_params(sql, &[Value], &Transaction)` 新入口 |
| `src/pipeline.rs` | `execute_with_params` / `execute_in_tx_with_params` 公共函数；`execute` 与 `execute_in_tx` 改走无参等价路径（`params = &[]`）；`execute_inner` 增加「占位符编号 → 个数校验 → plan → 参数物化 → execute」段；`materialize_params`（表达式位注入 + DML 值位槽位物化 + UPSERT DO UPDATE 赋值物化）与 `materialize_value_slots`；`parameter_count_mismatch`（点名「期望 N 实际 M」）；参数化通道不消费 plan cache get；plan cache put 门增「含占位符语句不入缓存」 |
| `src/executor/correlated.rs` | 注入 walk 增 `PhysicalPlan::Update` 臂（UPDATE 赋值右值位）与 `Sort` 臂（排序键表达式位） |

### Deviations from Plan（偏差与原因）

1. **占位符序号来源由 PlanBuilder 计数器改为 AST 级单次遍历编号**（design D5「PlanBuilder 计数器分配 `__param_{i}`」→ 实现为 `assign_placeholder_indices` 把裸 `?` 改写为 `__param_{i}` 标记，计划面读标记序号）。原因：计划期遍历序 ≠ SQL 文本序——FROM/JOIN（ON 条件）先于投影项、聚合分支的投影项晚于 WHERE、标量子查询项先行；计数器按计划序分配会使 `SELECT ? FROM t WHERE a = ?` 的两个参数绑定错位。改为单次 AST 遍历后，计数与编号同源同序（Plan Context Implementation Guidance 的前提要求），且计划面不再需要计数器。副作用：计划面读到未编号裸 `?`（CLI 等不经编号的入口）时点名拒绝，不静默按 NULL 处理。
2. **先行测试落点**：Plan 的 First test 指向 `tests/param_binding_test.rs`（新文件）。实施初期先行测试写入 `tests/typed_api_test.rs`，收口时按契约迁至 `tests/param_binding_test.rs`（26 条）；`tests/typed_api_test.rs` 恢复为 Iteration 000 终态（17 条）。先行测试顺序偏差：AST 遍历模块（`src/parser/placeholder.rs`）先于其单测落地——该模块的行为断言随 T7 端到端用例（26 条）一并覆盖，计划面与执行面行为仍为先行测试驱动。
3. **错误面收敛**（Plan 已预告，Implementation Guidance 第 4 条）：占位符值位既有错误面 `UnsupportedValue`（"Unsupported value type"）→ 个数点名（"Parameter count mismatch: expected N placeholder(s), got M parameter(s)"，SQLSTATE 42601）；计划期未编号裸 `?` 新增点名文案（"unbound placeholder: bare '?' is only supported through the parameterized execution entry point (execute_with_params)"）。
4. **`count_placeholders` 的实现方式**：契约为 `count_placeholders(&Statement) -> usize`（只读）。遍历需可变借用（编号改写），故计数模式克隆语句后走同一遍历——保证计数与编号同源同序；代价为每次计数一次语句克隆（调用点仅 plan cache put 门与测试路径）。
5. **注入面扩展**：`inject_correlated_values` 新增 `Sort` 臂（排序键表达式位，防御性覆盖 JOIN 基座别名键）与 `Update` 臂（赋值右值位，T7 必需）。

### 自检发现与修复（Gate 3）

| # | 级别 | 发现 | 处置 |
|---|---|---|---|
| 1 | Important | SELECT 谓词位参数在计划期被 `extract_pk_from_where` / `extract_secondary_index_equality` / `has_non_keyable_pk_literal_leg` 的字面量转换拒绝（`UnsupportedValue`）→ 参数化等值过滤不可达 | 已修复：三处遇占位符位返回「计划期无值 → 不提取键 / 不路由索引 / 非字面量腿」，谓词回落扫描路径，执行期按参数值求值；`t7_where_placeholder_filters_rows` 见证 |
| 2 | Important | `ON CONFLICT ... DO UPDATE SET` 右值位的占位符未被遍历（计数少算）→ 第二个参数被误判为多余 | 已修复：遍历补入 ON 冲突子句（`DoUpdate` 赋值位 + 过滤位、MySQL `DuplicateKeyUpdate`）；`t7_upsert_value_and_do_update_bind_parameters` 见证 |
| 3 | Minor | 计划面读到未编号裸 `?` 的错误文案误导（对裸形态说 "only bare '?' is supported"） | 已修复：文案分形态点名（`unsupported_placeholder_message`） |
| 4 | Minor | `UpsertValueExpr::Param` 在 EXPLAIN 渲染面未覆盖（非穷尽匹配） | 已修复：补臂渲染 `?{i}` |
| 5 | Minor | `PhysicalPlan::Update` 的赋值表达式位不在既有注入 walk 覆盖内（参数会静默求值为 Null） | 已修复：`inject_correlated_values` 增 Update 臂 |
| 6 | Minor | 排序键表达式位不在既有注入 walk 覆盖内 | 已修复：增 Sort 臂 |
| — | 遗留 Minor | 无。MF-1（`join_type_slots` 文档注释）不在本 Cycle 触达邻域，未校准 | 沿用 Iteration 000 记录 |

计划范围外修改：无（`git diff --stat -- src/` 仅本 Cycle 目标文件）。

### 验证（全部真实运行）

- **T5/T6/T7 定向**：`cargo test --test param_binding_test -- --test-threads=1` → **28 passed / 0 failed**（拒绝矩阵、个数错误零副作用、计划面编译、值位绑定、类型门/NOT NULL 门同源、唯一与次级索引维护照常、NULL 三值、TEXT 真值、事务内执行、无值残留、plan cache 不入）。
- **库单测**：`cargo test --lib -- --test-threads=1` → **414 passed / 0 failed / 0 ignored**。
- **CLI 面**：`cargo test --test cli_test -- --test-threads=1` → **124 passed / 0 failed / 2 ignored**。
- **类型化结果面**：`cargo test --test typed_api_test -- --test-threads=1` → **17 passed / 0 failed**。
- **构建 / 静态面**：`cargo build --lib --tests -j 2` → 零错误零警告；`cargo check --benches --examples` → 零错误零警告；`cargo fmt --check` → 全仓零 diff。
- **全量（M19 分批，串行）**：前置 `cargo build --lib --tests -j 2`（限并发编译）→ 14 批（lib 1 批 + 127 个集成目标按**字典序**分组，每批 ≤10 目标），批内 `cargo test --test <name> -- --test-threads=1`、**批间串行**、每批前可用内存守卫（< 1434 MiB 则等待，本轮实测 3.6–4.2 GiB，未触发）。分批计数：

| 批 | 目标数 | 范围（字典序） | passed/failed/ignored |
|---|---|---|---|
| 1 | — | lib（单元测试） | 414/0/0 |
| 2 | 10 | aggregate_extension_test … busy_timeout_test | 148/0/0 |
| 3 | 10 | cache_perf_test … concurrent_test | 182/0/2 |
| 4 | 10 | connection_limit_test … ddl_crash_recovery_test | 147/0/0 |
| 5 | 10 | default_declaration_test … explicit_tx_test | 83/**1**/0 |
| 6 | 10 | expression_e2e_test … insert_select_test | 100/0/0 |
| 7 | 10 | isolation_level_test … mvcc_commit_test | 89/0/0 |
| 8 | 10 | mvcc_record_test … param_binding_test | 117/0/0 |
| 9 | 10 | parser_test … prefetch_test | 193/0/0 |
| 10 | 10 | projection_expression_test … recovery_wal_convergence_test | 77/0/0 |
| 11 | 10 | runtime_test … sort_test | 134/0/0 |
| 12 | 10 | sql_differential_test … truncate_test | 147/0/0 |
| 13 | 10 | tx_statement_test … wal_auto_checkpoint_test | 152/0/0 |
| 14 | 7 | wal_buffer_test … write_type_conformance_test | 48/0/0 |
| — | — | doctest | 0 用例 |

- **批 5 的失败与补跑**：`e2e_test::e5_concurrency_lock_journey` 首跑失败——断言「等待者应阻塞到持锁者释放」（实测 elapsed 21.3ms < 800ms 门槛）。该用例为**文件锁竞争 + 进程计时断言**，与本 Cycle 改动面（解析器/计划器/参数通道/注入 walk）无触达关系。复核：`cargo test --test e2e_test -- --test-threads=1` → **27 passed / 0 failed**（按通过处理，注明首跑失败）。
- **被中止目标的补跑**：cargo 在批内首个失败目标后中止该批后续目标——批 5 中 e2e_test 之后的 4 个目标未执行，按覆盖清单补跑：`encryption_test` **7/0/0**、`executor_test` **39/0/0**、`explain_test` **10/0/0**、`explicit_tx_test` **21/0/0**。
- **覆盖核对**：ran 127 = want 127（集成目标不重不漏）；全量合计 **lib 414 + 集成 1695 = 2109 passed / 0 failed / 2 ignored**（基线 Iteration 000 终态 2081/0/2 + 本轮新增 param_binding_test 28 条，无回归；ignored 2 与基线一致）。
- **文案校准后的定向复核**（`unsupported_placeholder_message` 改动触及错误面）：`cargo test -j 2 --lib --test param_binding_test --test typed_api_test --test cli_test -- --test-threads=1` → 414/0/0 + 28/0/0 + 17/0/0 + 124/0/2 全绿；其余目标的产品代码面未变，其批次结论继续采信。

- **补充见证后的定向复核**（追加「唯一与次级索引维护照常」与「拒绝零副作用」两条 spec 场景细节见证）：`cargo test --test param_binding_test -- --test-threads=1` → **28 passed / 0 failed**；仅该测试文件变更，其余目标产品代码面未变，其批次结论继续采信（全量口径相应由 2107 更新为 2109）。

### Persisted Evidence

None required —— 全部验证为 `cargo test` 集合内的行为断言（可低成本重跑），无一次性环境、无需保留现场；命令与逐批计数已在本 Response 承载。

### Experience Candidates

1. **Runbook 候选——全量分批回归中「批内首个失败目标导致 cargo 中止后续目标」的处置**：失败目标复核 + 被中止目标补跑 + ran-vs-want 覆盖清单核对（本轮批 5：e2e_test 首跑失败 → 4 目标被中止 → 补跑 7/39/10/21 全绿 → ran 127 = want 127）。证据：本 Response「验证」段批 5 与补跑记录。
2. **Runbook 候选——M19 分批回归的限并发预编译 + 批间内存守卫 + 逐批计数模板**：本轮实跑 14 批 127 目标稳定通过（前置 `cargo build --lib --tests -j 2`，批内 `--test-threads=1`，批间串行，每批前 MemAvailable ≥ 1434 MiB 守卫）。证据：本 Response 分批计数表与迭代 000 同纪律记录。
3. **Issue 候选——`tests/e2e_test.rs:969` 锁等待计时断言在机器负载下偶发不达**（首跑 elapsed 21.3ms < 800ms 门槛，复核通过）：属既有测试稳定性缺陷，change 范围外且有证据；本 Cycle 不落账，仅报告。

### 需 Plan Review 裁定的设计回折

Deviation 1（占位符序号来源由 PlanBuilder 计数器改为 AST 级单次遍历编号）触及 design D5 的机制描述——Act 只记录偏差与理由，不改写 Plan 产物。Plan Review 需裁定：design.md D5 的「PlanBuilder 计数器分配 `__param_{i}`」是否回折为「AST 级单次遍历编号（计数与编号同源同序）」，以及 `count_placeholders` 的克隆实现方式是否入规。

### 未解决问题

- DML WHERE 定位键位参数化仍不可达（design R4——需执行期键求值重构，非本 Cycle 范围）：已按契约点名拒绝并指明原因。
- `Merge` / `Explain` / `Prepare` 等语句形态的占位符位置不在 AST 遍历面内（计划期即被既有语句分派拒绝，参数化执行不可达）——已在 `src/parser/placeholder.rs` 模块头标注为已知边界。
- 预编译语句与 schema 版本失效契约属 Iteration 002，未启动。

## Plan Review

- Review Result: accepted（2026-10-22 复审——T5-T8 独立核对通过；设计回折请求裁定接受并已回写 design.md D5/D6）

**Findings（复审）**

- **占位符遍历与拒绝面（T5）核对通过**：`src/parser/placeholder.rs` 的 `Walker` 对 sqlparser `Expr` 穷尽匹配（编译器保证）、SQL 文本序遍历（Query: WITH→主体→ORDER→LIMIT…；Select: 投影→FROM/JOIN ON→WHERE→GROUP→HAVING…；Insert 含 ON 冲突子句赋值与过滤位）；DDL（CTAS 源/视图/索引列位）命中占位符点名拒绝；非裸形态（`?N`/`$N`/`:name`/`@name`）分配模式点名拒绝；重复分配幂等（已编号标记保持原样）。`count_placeholders` 克隆走同一遍历——计数与编号同源同序由构造保证。
- **计划面编译（T6）核对通过**：`expression.rs` `Expr::Value` 臂只在 `Value::Placeholder` 分流（既有值路径逐字节不变）；未编号形态经 `unsupported_placeholder_message` 分形态点名；`ddl_dml.rs` 值槽登记两路径列位语义正确（全宽路径 col=VALUES 行内位 = 表列位；列清单路径 col=清单→表列映射后的表序位，`map_insert_values` 重构对 Val/DefaultKeyword 等价）；`reject_parameterized_key_clause` 在 `build_update`/`build_delete` 的 PK 提取链前置；`query.rs` 三处守卫（键提取/二级索引路由/字面量腿）均只在 `is_placeholder_value` 触发——SELECT `WHERE a = ?` 回落扫描路径执行期求值，既有字面量路径零变化。
- **执行通道（T7）核对通过**：`execute_inner`/`execute_in_tx_with_params` 同序（编号→个数校验→plan→物化→执行，计划前拒绝零副作用）；cache 契约双面落实——get 旁路（`!params.is_empty()` 时跳过）+ put 门 `count_placeholders(stmt) == 0` 排除，且经 `plan_stage → plan_stage_routed` 委托覆盖全部执行路径；`materialize_params`（槽位物化 + UPSERT DO UPDATE `Param→Literal` 改写 + `inject_correlated_values` 表达式位注入）；个数错误「expected N placeholder(s), got M parameter(s)」+ 42601 点名。`execute`/`execute_in_tx` 等价 `params = &[]`——无占位符语句仅多一次空遍历（零改写、零行为变化）。
- **执行器面加性核对通过**：`ValueParamSlot` 新类型 + `InsertNode`/`UpsertNode` 新字段 + `UpsertValueExpr::Param` 新变体（`plan_format.rs` 渲染臂与 `upsert.rs` 防漏臂为编译器穷尽性强制的新增臂；upsert 臂对未物化形态 `StorageError::Internal` 点名——「执行器只见物化后形态」不变量以错误面强制）。`correlated.rs` 增 `Update` 臂（T7 必需——SET 右值参数注入）与 `Sort` 臂（防御性）；`InsertSelect` 源子树与 CTAS 源子树在既有 walk 臂内——`INSERT INTO t SELECT … WHERE x = ?` 参数可达且正确注入（Act 有专门见证 `t7_insert_select_source_placeholder_binds`）；CTAS 源占位符在编号期被 DDL 拒绝面拦截。
- **测试覆盖核对通过**：`tests/param_binding_test.rs` 28 条 = 23 `#[tokio::test]` + 5 plain `#[test]`——delta spec R1 七场景逐一覆盖（等值过滤/值位绑定含类型门与 NOT NULL 门同源与唯一次级索引维护/键位点名/个数零副作用/类型不符/NULL 三值/不可达位点名），另覆盖契约硬边界（cache 不入 + 同文本不同参数无值残留）、design R1 锁定点（顶层投影位 TEXT 真值——真值通道实测可达，无需位点名拒绝，按预设前半处置）、事务内执行、JOIN ON 位、ORDER BY 别名键位。
- **验证采信与对账**：全量算术逐项吻合——批表 passed 合计 1617 + 补跑 4 目标 77 + e2e 复核 1 = 集成 1695 = 基线 1667 + 28；lib 414；全量 **2109/0/2**（Act Response 记录的 2109 为追加两条见证后的最终口径）。批 5 e2e 锁等待计时断言首跑失败按验证规则处理（一次复核通过、注明首跑失败、被中止 4 目标补跑、ran 127 = want 127 覆盖清单核对）——处置合规；该测试稳定性缺陷确认为 Issue 候选（Act 已报告未落账，处置权在 Recorder）。
- **Deviation 裁定**：①AST 单遍编号替代 PlanBuilder 计数器——**接受**：契约硬要求是「计数与编号同源同序」；计划期遍历序 ≠ SQL 文本序，按计划序分配会使 `SELECT ? FROM t WHERE a = ?` 绑定错位，AST 改写以构造方式满足同源要求且计划面免计数器；design.md D5 已由本 Review 回折（机制描述、文本序表述、SELECT 键提取「计划期无值」前提、D6 流程与 cache 门落地形态）。②测试落点先 typed_api 后迁回 param_binding——接受（契约落点已满足，typed_api 恢复 17 条终态）。③错误面收敛为契约预告项。④count 克隆实现——接受（调用点仅 put 门与测试路径；附 MF-A 观察项）。⑤注入面扩展（Update 必需、Sort 防御）——接受，对非参数表达式零影响（`set_parameter_value` 对无名参数为 no-op）。
- **MF-A（非阻塞 Minor，性能观察）**：put 门 `count_placeholders(stmt) == 0` 对每次可缓存计划（cache miss 与显式事务路径）克隆一次完整 AST。相对 parse+plan 成本为小头，且仅 miss 路径；备选优化（`sql.contains('?')` 预检或参数存在标志穿线）留 MS22 实测纪律评估，本 Cycle 不处理。
- **MF-B（非阻塞 Minor，文案归因）**：`reject_parameterized_key_clause` 检查整条 WHERE——`UPDATE … WHERE a = 1 AND c > ?`（占位符在非键谓词位）报「parameterized primary key is not supported」文案；该语句本就被「UPDATE 仅支持 PK 等值」拒绝，两条路径均零副作用，仅文案归因偏差。后续触达时可收窄为键位判定。
- **MF-C（非阻塞 Minor，已修正）**：change tasks.md 两处「全量终态 2107/0/2」为追加两条见证前的口径（最终 2109）——本 Review 同步 tasks.md 时已修正。

**Acceptance Gaps**

None——R1 七场景、无占位符语句零回归（2109 = 2081 + 28 对账吻合）、plan cache 双面契约、Invariants（render/pg_protocol/序列化零改动、执行器只加新变体臂、无字符串替换绑定）均满足。

**Evidence**

- 独立读全部 diff：`placeholder.rs`（新，761 行全文）、`parser/mod.rs`、`expression.rs`、`ddl_dml.rs`、`query.rs`、`executor/{plan,upsert,plan_format,correlated,mod}.rs`、`database.rs`、`pipeline.rs`。
- 关键路径独立验证：put 门覆盖面（`plan_stage → plan_stage_routed` 委托核对）；`InsertSelect`/`CTAS` 源子树注入臂（`correlated.rs:128`/`:122`）覆盖核对；`map_insert_values` 两路径列位语义逐行核对；`Database` 结构（全 Arc 字段，`#[derive(Clone)]`——Iteration 002 prepare 持引用的前提成立）。
- 测试清点：`param_binding_test.rs` 属性计数 23+5=28 与声明一致；七场景逐一映射到具体测试函数。
- 采信 Act Response 验证（未失效——其后仅 tasks.md 文档面变更）：定向 28/0、lib 414/0、cli 124/0/2、typed_api 17/0、benches/examples check 零告警、fmt 零 diff、全量 14 批（算术对账见 Findings）。

**Follow-up Decision**

全部契约点满足、偏差均有据且已裁定、无阻塞 finding——Review Result 置 **accepted**，Iteration 001 完成。设计回折按裁定回写 design.md D5/D6（AST 单遍编号 + cache 门落地形态）。MF-A/B 记录不返工；MF-C 已随本 Review 修正。按 Map 展开下一逻辑 Iteration 002（Plan Context 状态 draft，等待用户批准后转 ready）。

**Iteration Plan Update**

design.md D5/D6 回折（见 Follow-up Decision）；Iteration Map 本身无变化。

**Next Cycle**

None

**Next Iteration**

`iterations/002-prepared-statements/000-initial.md`（2026-10-22 展开，Plan Context 状态 draft）
