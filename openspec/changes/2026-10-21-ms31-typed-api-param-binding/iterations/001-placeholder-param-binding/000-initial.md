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
