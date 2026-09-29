# Iteration 002 / Cycle 000: 列解析静默面收口

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4（投影严格解析 + 别名表头）、T5（列缺失重分类）、T6（ORDER BY 键校验）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 扩围段（2026-09-26 用户批准，范围选项「全收口」）、design D6/D7/D8（D9 记录不实施）、tasks.md Iteration 002 边界、Iteration 001 交付的 `ColumnNotFound`→42703 分类通道（`PlanError::sqlstate` 映射表与 27 变体单测，本 Cycle 零修改直接复用）
- Excluded scope: 无聚合 GROUP BY 整体被忽略（D9——语义缺口，Issue 候选）；JOIN 投影/列解析面改动；表达式查询「按非输出列排序」能力扩展；ORDER BY 输出别名（`ORDER BY n`）支持（现状静默跳过 → 本 Cycle 后计划期拒绝，能力扩展留后续）；UPDATE/INSERT 赋值目标列解析面；聚合路径别名行为

**Objective**

计划期列解析的三个静默错误结果形态（未知投影列 identity 回落、纯查询别名表头退化、ORDER BY 未知键静默跳过）与一处错误建模不一致（值表达式列缺失 → ParseError/42601）全部消除：未知形态计划期 `ColumnNotFound` 点名拒绝（exit 3 / PG 42703），别名表头正确生效；既有可达形态逐字节零回归（白名单内两处文案变化除外）。

**Investigation Facts**

- Current Baseline: Iteration 001 accepted（`iterations/001-error-surface-session-e2e/000-initial.md` Plan Review）；master + MS19 两个 Iteration 交付（工作区未提交），全量 **1261 tests / 0 failures / 2 ignored**（Act 串行全量新鲜；本 Cycle 触碰的 query.rs/expression.rs 自该结论后零变化——Iteration 001 Review 独立 diff 阅读确认其变更面不含静默列解析路径）。
- Current-State Evidence（Plan 真二进制探针 + 代码阅读，2026-09-26；探针库 `t2(id INT PRIMARY KEY, name STRING NOT NULL)` 两行）：
  - **探针矩阵（现状）**：`SELECT nosuchcol FROM t2` / `SELECT id, nosuchcol` / `SELECT t2.nosuch` / `SELECT nosuch AS x` / `SELECT name AS n` / `SELECT name AS n WHERE id=1` → 全部 **exit 0 identity**（非空表返回全列全数据；别名表头退化 `["id","name"]`）；`SELECT id FROM t2 WHERE nosuch = 1` / `SELECT abs(nosuch)` → exit 3 `Plan error: Parse error: Column 'nosuch' not found in table 't2'`；`SELECT id FROM t2 ORDER BY nosuch` / `ORDER BY nosuch, name DESC` / `SELECT id+1 AS d ORDER BY nosuch` → **exit 0 静默不排序**；`SELECT name FROM t2 GROUP BY name`（无聚合）→ exit 0 不分组（D9，不实施）。
  - **可达基线（须保持）**：`SELECT t2.id` → `["id"]`；`SELECT id FROM t2 ORDER BY name DESC` → 正确排序；`SELECT id + 1 AS d FROM t2 ORDER BY name DESC` → 正确排序（D10：表达式查询排序键基础列可达）；`SELECT name AS n, COUNT(*) FROM t2 GROUP BY name` → `["n","count_star"]`（聚合路径别名已正确）；`SELECT id+1 AS d` → `["d"]`（Projection 通路别名表头已正确——T4 别名修复即复用此通路）。
  - **投影路由链**：`projection_columns` = `extract_columns`（`src/parser/ast.rs`——`Identifier`→列名小写、两段 `CompoundIdentifier`→列尾段、`ExprWithAlias`→**别名**、`*`→`"*"`）（`query.rs:534-540`）；角色分类（`query.rs:570-635`）——`is_plain_column_expr`（`query.rs:1584`：非 NULL `Identifier` 或两段 `CompoundIdentifier`）→ `Column` 角色，否则（`!building_subquery`）→ `has_expression_items = true` + `Expression` 角色；投影解析（`query.rs:747-758`）——`has_aggregates || !subquery_evals.is_empty()` → None；`has_expression_items` → None（输入全形状，顶层 Projection 统一求值，`query.rs:704-731` 逐项 `build_expression`）；否则 `base_schema`（仅 `Scan` 臂为 Some，:742-745）`.and_then(resolve_projection_indices)`——**未知名返回 None → identity（缺陷 ①）**。
  - **`resolve_projection_indices`**（`query.rs:1597-1615`）：空清单/`*`/未匹配名 → `None`；注释明写 identity 语义把别名/表达式与未知列混为一桶。
  - **列缺失建模**：`build_expression`（`src/parser/planner/expression.rs`）——单表裸名 `:276-291` 与两段限定名 `:332-348` 构造 `PlanError::ParseError("Column '{}' not found in table '{}'")`（**:283/:340，缺陷 ②**）；同函数多表搜索臂 `:271` 与 NLJ 布局臂 `:324` 已是 `PlanError::ColumnNotFound(column_name)`。`error.rs:81` Display `Column not found: {col}`、`:143` 映射 42703（27 变体单测已锁定，零修改复用）。
  - **ORDER BY**：`query.rs:1126-1137` 构造 `OrderByColumn`（`extract_column_name`，非列名形态既有 ParseError "ORDER BY only supports column names"，:1673）；`sort_columns` 于 `:1145-1149` 计算（聚合 → `projection_columns`；否则 `get_plan_output_columns(plan_with_aggregate)`——基础扫描链 = 全基列、表达式查询 = 全基列〔探针证实基础列可达〕）；`SortExecutor::compare_rows`（`src/executor/sort.rs:75-100`）`position()` `None` → 静默跳过该键（**缺陷 C**）。
  - **测试面清点（grep，2026-09-26）**：`not found in table` 文案断言 **0** 处；纯列别名 SELECT（`SELECT col AS alias`）既有用例 **0** 处；`tests/pg_integration_test.rs::pg_name_resolution_errors` 已见证 JOIN ON 路径 42703（本 Cycle 零修改）；`tests/projection_test.rs`（6）/`projection_expression_test.rs`（16）/`pushdown_test.rs`（16）/`group_by_expr_test.rs`（12）均为可解析形态，预期零校准。
- Code and Critical Path: T4 触 `query.rs`（路由臂 :747-758 改严格解析 + 角色分类后处理）；T5 触 `expression.rs` 两构造点；T6 触 `query.rs` Sort 构造点校验。全部为计划期（`build_select` 内），不触执行器与存储；错误经既有 `PlanFailure`/`Response::Error` 通道流出（Iteration 001 交付面，零改动）。

**Implementation Guidance**

顺序：T4 RED（拒绝矩阵 + 别名表头）→ T5 RED（文案/分类）→ T6 RED（排序键）→ 各 GREEN → 全量串行。T4 与 T5 同文件族交叉（未知别名项经 T5 的 `build_expression` 得 `ColumnNotFound`），按序减少往返。

- T4a 严格解析：`resolve_projection_indices` 保留 `None` 仅用于 `*`/空清单；纯列分支（:754-758）改为——`base_schema` 为 `Some(schema)` 时逐项解析，任一非 `*` 项不可解析 → `Err(PlanError::ColumnNotFound(该项))`；`base_schema` 为 `None`（非 Scan 基座）维持现状 identity（保守，不扩范围）。
- T4b 别名路由：角色分类循环后、混合聚合裁决（:644）前，若 `!has_aggregates && !building_subquery`，把 `ExprWithAlias(plain column)` 项从 `Column` 角色改标 `Expression(alias)` 并置 `has_expression_items = true`——路由进入既有顶层 Projection 通路（:704-731 逐项 `build_expression`，别名项表头 = 别名、行 = 投影列）。聚合查询不改标（`agg_output_columns` 已含别名；混合投影校验不受影响——`!has_aggregates` 条件保证）。`item_roles` 的 `Column` 角色在聚合路径的其他消费点保持原序（仅纯非聚合路径翻转）。
- T5：`:283`/`:340` 两处 `PlanError::ParseError(format!("Column '{}' not found in table '{}'", ...))` → `PlanError::ColumnNotFound(column_name)`（两处作用域内变量名即列名，与 `:271`/`:324` 同形）。`:277`/`:334` 的 `Table '{}' not found` ParseError 不动。
- T6：`sort_columns` 计算后（:1149）、`SortNode` 构造（:1156）前，对 `order_by` 每键校验 `sort_columns` 可解析（大小写不敏感，同 `compare_rows` 的匹配规则），不可解析 → `Err(PlanError::ColumnNotFound(key))`。`SortExecutor` 不改。

**Task Contracts**

### T4: 投影列严格解析 + 纯查询别名表头生效（R6/R7）

- Requirement/Scenario: R6 四场景、R7 三场景（`specs/error-reporting` ADDED）
- Depends on: None（T5 的 ColumnNotFound 文案由本任务 RED 断言依赖——若 T5 未先行，未知别名项断言按 `ColumnNotFound` 变体存在性写，不依赖其文案定稿；执行序 T4 先行、其别名拒绝用例与 T5 联合 GREEN）
- Targets: `src/parser/planner/query.rs`（`resolve_projection_indices` 消费臂 :747-758、角色分类后处理 :635-650 区段）；`tests/planner_test.rs`（+5）、`tests/cli_test.rs`（+2）、`tests/pg_integration_test.rs`（+1）
- Current behavior: 见探针矩阵——未知裸列/限定列/混合与 `col AS alias`（纯非聚合）全部 exit 0 identity。
- Required behavior: 未知裸列/限定列/混合 → exit 3 `ColumnNotFound` 点名该项（PG 42703）；`name AS n` → exit 0 表头 `["n"]` 行 = name 列；`nosuch AS x` → exit 3 点名 `nosuch`；可解析形态（`*`/子集/重排/重复/`t.id`）与聚合别名输出逐字节零回归。
- Preserve: 非 Scan 基座（DerivedScan 可达形态）identity 行为不变；JOIN 面投影解析（`qualified_columns`/布局）不变；聚合查询角色分类与输出（含 `name AS n, COUNT(*) GROUP BY name` → `["n","count_star"]`）逐字节；`has_aggregates`/`subquery_evals` 优先分支不变；`building_subquery` 抑制语义不变。
- Forbidden: 不改 `resolve_projection_indices` 对 `*`/空清单的 None 语义；不改执行器（Projection/DataScan/Sort）；不动聚合路径别名装配；不扩展 DerivedScan 严格解析。
- Test witness: RED 先行——`tests/planner_test.rs` 新增 `unknown_projection_column_rejected`（断言 `Err` 且含 `nosuchcol`，现状 `Ok`/identity → RED）、`mixed_known_unknown_rejected`、`unknown_qualified_column_rejected`、`resolvable_projection_shapes_unchanged`（`*`/子集/重排/重复/限定有效列输出形状断言，现状即 GREEN 作为回归锚）、`alias_header_plan_shape`（纯查询别名项路由后 plan 含 Projection 臂或等价形状）；`tests/cli_test.rs` `test_unknown_projection_column_cli`（exit 3）、`test_alias_header_cli`（`["n"]` + 行）；`tests/pg_integration_test.rs` `pg_unknown_column_42703`。
- GREEN condition: 上述用例全绿 + `cargo test --test projection_test --test projection_expression_test --test pushdown_test --test group_by_expr_test` 零校准。
- Verification: ① `cargo test --test planner_test -- --test-threads=1` 退出码 0；② `cargo test --test cli_test --test pg_integration_test` 退出码 0；③ 真二进制探针复跑探针矩阵六形态（4 拒绝 exit 3 + 2 可达不变）。失败含义：可解析形态断言红 = 严格解析误伤可达形态（回 Plan）。
- Stop when: 纯列分支存在无法从 AST 区分「别名」与「未知列」的形态（`extract_columns` 信息不足需改返回结构——回 Plan 评估）；角色改标破坏聚合混合校验（`name AS n, COUNT(*)` 形态红——回 Plan）。

### T5: 值表达式列缺失重分类 ColumnNotFound（R8）

- Requirement/Scenario: R8 三场景（`specs/error-reporting` ADDED）
- Depends on: None
- Targets: `src/parser/planner/expression.rs`（:283、:340 两构造点）；`tests/planner_test.rs`（+3）；`tests/pg_integration_test.rs`（+1）
- Current behavior: WHERE/值表达式列缺失 → `ParseError`，文案 `Parse error: Column 'x' not found in table 't'`，PG 42601。
- Required behavior: 同形态 → `ColumnNotFound(x)`，文案 `Column not found: x`（经既有 Display），PG 42703；JOIN 布局路径与 `Table 'x' not found` ParseError 不变。
- Preserve: `:271`/`:324` 既有 ColumnNotFound 形态；`Table '{}' not found`（:277/:334）文案与分类；`explicit_rejection_messages_unchanged`（Iteration 001 用例）零修改通过；`planerror_sqlstate_mapping` 27 变体单测零修改（ColumnNotFound→42703 已在表内）。
- Forbidden: 不改 `PlanError` 枚举/Display/映射表；不改多表臂；不动 UPDATE/INSERT 赋值目标解析。
- Test witness: RED——`where_column_missing_column_not_found`（断言文案 `Column not found: nosuch`，现状红）、`expression_arg_column_missing`（abs 形态）、`join_layout_zero_regression`（现状 GREEN 锚）；`tests/pg_integration_test.rs` `pg_where_column_missing_42703`（现状 42601 → RED）。
- GREEN condition: 上述全绿 + `explicit_rejection_messages_unchanged` 与 `pg_name_resolution_errors` 零修改通过。
- Verification: `cargo test --test planner_test --test pg_integration_test -- --test-threads=1` 退出码 0；真二进制探针两形态文案。失败含义：JOIN ON 用例红 = 误伤多表臂（回 Plan）。
- Stop when: 列缺失在该两站点存在非「列缺失」的复用语义（同一 ParseError 承载多种失败——需回 Plan 拆分）。

### T6: ORDER BY 排序键计划期校验（R9）

- Requirement/Scenario: R9 两场景（`specs/error-reporting` ADDED）
- Depends on: None
- Targets: `src/parser/planner/query.rs`（SortNode 构造点 :1150-1162 区段）；`tests/planner_test.rs`（+2）；`tests/cli_test.rs`（+1）
- Current behavior: 未知排序键静默跳过（单键与混合键均不排序不报错）。
- Required behavior: 不可解析键 → 计划期 `ColumnNotFound(key)`（exit 3，PG 42703）；既有可达形态（基础列/表达式查询基础列/聚合投影名）排序结果逐字节不变。
- Preserve: `sort_columns` 计算逻辑（:1145-1149）不变；`ORDER BY only supports column names`（非列名形态）不变；`SortExecutor` 全部代码不变；聚合查询 `ORDER BY count_star` 可达性不变。
- Forbidden: 不改 `sort.rs`；不实现 ORDER BY 输出别名解析（能力扩展）；不重构 Sort/Projection 链。
- Test witness: RED——`unknown_order_by_key_rejected`（单键 + `nosuch, name DESC` 混合键，现状 `Ok` → RED）、`reachable_order_by_shapes_unchanged`（三形态排序断言，现状 GREEN 锚）；`tests/cli_test.rs` `test_unknown_order_by_key_cli`（exit 3）。
- GREEN condition: 上述全绿 + 既有排序相关用例零校准。
- Verification: `cargo test --test planner_test --test cli_test -- --test-threads=1` 退出码 0；真二进制探针三可达形态排序复跑。
- Stop when: `sort_columns` 在某可达形态与 `SortExecutor` 实际可见列集不一致（校验会误拒现可排序形态——回 Plan；探针证据表明三形态一致）。

**Invariants**

- `PlanError` 枚举、Display、`sqlstate()` 映射表与 27 变体单测零修改（全部复用既有变体与分类）。
- 执行器（Projection/DataScan/Sort/Aggregate）零改动——全部修复在计划期。
- 既有可达形态（探针可达基线全表）输出逐字节零回归；错误文案变化仅 R4 白名单两处。
- 聚合查询的角色分类、混合投影校验与别名输出零变化。

**Non-goals**

D9 无聚合 GROUP BY；JOIN 列解析面；ORDER BY 别名/非输出列排序能力；UPDATE/INSERT 赋值目标解析；DerivedScan 严格解析；错误文案措辞优化（白名单外）。

**Acceptance**

- R6 四场景、R7 三场景、R8 三场景、R9 两场景各有通过断言（映射见 tasks.md RTM R6-R9 行）。
- R4 白名单修订（两处文案变化）与全量零回归成立：全量串行（M19 约束）基线 1261 + 新增用例，除白名单外零校准。
- 探针矩阵可达基线六形态真二进制复跑逐字节一致。

**Verification**

- R6-S1..S3：`cargo test --test planner_test unknown_projection` 等 + `pg_unknown_column_42703`——exit 3 点名 / 42703。
- R6-S4/R7-S3：`cargo test --test projection_test --test projection_expression_test --test group_by_expr_test` 零校准。
- R7-S1/S2：`test_alias_header_cli`（`["n"]`）/ alias 拒绝用例。
- R8-S1..S3：`where_column_missing_column_not_found` 等三 planner 用例 + `pg_where_column_missing_42703` + 既有 JOIN ON 用例零修改。
- R9-S1/S2：`unknown_order_by_key_rejected` + `reachable_order_by_shapes_unchanged`。
- 全量：`cargo test -- --test-threads=1`（串行，M19 约束）零回归。

**Gate 2 Readiness**

全部 PASS（2026-09-26）：需求与范围经用户批准（范围选项「全收口」AskUserQuestion 确认 + 计划批准原话「批准」）；调查完整（真二进制探针矩阵 + 代码阅读，本文件自包含）；设计闭合（D6/D7/D8，含替代方案与拒绝理由）；任务可执行（三契约含 Targets/行为变化/Preserve/Forbidden/RED-GREEN/Stop when）；分轮合理（平衡审计通过）；追踪完整（R6-R9 → T4-T6 → 测试见证 RTM）；验证为最简直接判定、无身份型证据工程；Persisted Evidence `none` 满足白名单；无 TBD 与实质未知项（非实质项在 Risks and Notes）。无 BLOCKED/WAIVED 项。

**Persisted Evidence**

- Mode: none

（全部验证为低成本可重跑命令与真二进制探针，Act Response 承载结论。）

**Risks and Notes**

- 隐藏校准风险：Plan 已 grep 清点（`not found in table` 断言 0、纯列别名 SELECT 用例 0），但 identity 回落可能被间接依赖（如某用例借 `SELECT nosuch` 断言全列形状）——Act RED 阶段先跑 `projection_test`/`projection_expression_test`/`cli_test`/`aggregate_test`/`group_by_expr_test` 基线，发现红即按「计划内行为变更 + 白名单」记录校准清单；校准面超出投影/排序/列缺失形态时回 Plan。
- T4b 角色改标的边界（`!has_aggregates && !building_subquery`）为探针钉死的事实条件；若 Act 发现第三种消费 `Column` 角色的路径受翻转影响（如 group key 归属校验），按 Stop when 返 Plan，不自行扩条件。
- T6 校验列集与 `SortExecutor` 可见列集的一致性由探针钉死（三形态）；聚合查询 `ORDER BY <未投影基础列>` 现状即静默跳过，T6 后拒绝——属显式化目标而非回归（该形态从未排序成功）。
- ORDER BY 输出别名（`ORDER BY n`）现状静默跳过，T6 后拒绝——已知边界（SQLite 支持该形态；能力扩展留后续，可入 improvements）。

## Act Response

- Status: reported

**Implemented**

T4、T5、T6 全部交付并通过各自 GREEN 条件；T6 的 Blocker 已由 Plan Review 裁定（收窄校验谓词）并在当前 Cycle 修复完成，语料 `sql-scalar-functions` R5 e2e 零修改恢复 GREEN，Cycle 级全量零失败。Iteration 002 的三个 task 全部完成。

产品代码（2 文件，无执行器改动）：

- `src/parser/planner/query.rs`
  - **T4a 严格解析**：`resolve_projection_indices`（:1674）签名由 `Option<Vec<usize>>` 改为 `Result<Option<Vec<usize>>, PlanError>`——`None` 只保留给空清单与 `*`；不可解析的名字改为 `Err(PlanError::ColumnNotFound(该项))`（原与别名/表达式共用的 identity 回落桶是「未知列静默返回全列全数据」的唯一来源）。消费臂（:790-825）新增 `pure_column_list` 门：仅当每个非子查询项都是普通列引用（通配放行）时才严格解析，否则维持恒等。
  - **T4b 别名路由**：`build_select` 角色分类后新增 `alias_reroutable` 门（:639-673）——`!has_aggregates && !building_subquery && Scan 基座 && 清单无通配符` 时，把「普通列引用 + 别名」项的 `SelectItemRole` 由 `Column` 改标 `Expression(alias)` 并置 `has_expression_items`，交既有顶层 Projection 通路（表头 = 别名、行 = 该列）。
  - **T6 排序键校验**：`sort_columns` 计算后、`SortNode` 构造前（:1226-1253）逐键校验，匹配规则与 `SortExecutor::compare_rows` 同源（大小写不敏感）；不可解析 → `PlanError::ColumnNotFound(键)`。校验谓词按 Plan Review 修订后的契约为「键在 `sort_columns` 内 **或** 与顶层投影项输出别名（`select.projection` 中任一 `ExprWithAlias` 的 `alias.value`，:1237-1245 预收集为 `alias_columns`）相等」——别名键不拒绝也不解析为排序列，由 `SortExecutor` 既有 None 跳过臂保持输入序。`src/executor/sort.rs` 零改动。
- `src/parser/planner/expression.rs`：**T5 重分类**两处单表列缺失构造点（:279、:334）由 `ParseError("Column '{}' not found in table '{}'")` 改为 `ColumnNotFound(column_name.clone())`，与同函数多表臂（:271）与 NLJ 布局臂（:324）同模。`Table '{}' not found` 的 ParseError（:277/:334）不动。

测试（3 文件，+16 用例）：

- `tests/planner_test.rs`（+11）：T4 `unknown_projection_column_rejected` / `mixed_known_unknown_rejected` / `unknown_qualified_column_rejected` / `resolvable_projection_shapes_unchanged`（7 形态零校准锚）/ `alias_header_plan_shape` / `unknown_alias_column_rejected`；T5 `where_column_missing_column_not_found` / `expression_arg_column_missing` / `join_layout_zero_regression`（JOIN ON 既有 ColumnNotFound 锚）；T6 `unknown_order_by_key_rejected`（单键 + 混合键）/ `reachable_order_by_shapes_unchanged`（三形态 Sort 节点形状锚）。
- `tests/cli_test.rs`（+3）：`test_unknown_projection_column_cli`（exit 3 点名）、`test_alias_header_cli`（`["n"]` + 行）、`test_unknown_order_by_key_cli`（exit 3 点名）。
- `tests/pg_integration_test.rs`（+2）：`pg_unknown_column_42703`、`pg_where_column_missing_42703`。

**Deviations from Plan**

1. **T4b 改标范围收窄（Gate 3 发现的回归修复）**：契约文字为「`!has_aggregates && !building_subquery` 即改标」。按此实施后真二进制探针立即显示两处新拒绝——`SELECT *, name AS n FROM t` 撞「`*` 不能与表达式项混用」拒绝臂、JOIN 基座别名查询撞「表达式项 + JOIN」拒绝臂，二者变更前均 exit 0。收窄为「+ Scan 基座 + 清单无通配符」后两形态恢复既有行为。收窄方向与契约自身的 Preserve/Invariants 一致（JOIN 投影解析面不变、既有可达形态零回归），属契约内处理。
2. **T4a 增加 `pure_column_list` 门（契约遗漏的既有可达形态）**：全量首次运行暴露 `subquery_test` 4 项失败（`Column not found: _1`）。根因：`extract_columns` 对字面量投影项产出 `_N` 伪名（`src/parser/ast.rs:112` `Expr::Value(v) => Ok(format!("_{}", v))`），子查询内的 `SELECT 1 FROM dept`（EXISTS/IN 形态）经 `building_subquery` 路径进入纯列分支并被判为未知列。字面量投影不是 delta R6 所称「投影名」，且 `SELECT 1 FROM t` 是既有可达形态，delta R6 要求「既有可解析形态 SHALL 逐字节保持」——故严格解析只作用于纯列清单。这是 delta 自身蕴含的范围，非新增设计。
3. **planner_test 用例数 6 而非契约的 +5**：契约 Required behavior 与 tasks.md RTM 均要求 `未知列 AS alias` 有见证（`alias_unknown_column_rejected`），契约 Test witness 清单漏列该用例。计数 `+5` 为 Plan 概数。
4. **`reachable_order_by_shapes_unchanged` 首版断言形态错误（测试侧，非产品）**：首版按顶层即 `Sort` 断言，RED 阶段暴露表达式查询的顶层是 `Projection(Sort(...))`（别名表头由 Projection 拥有）。改为解包后比对 Sort 节点——修正的是见证本身，非产品行为。
5. **`cargo fmt` 既有偏差集合较 Iteration 000 记录多一处**：`src/parser/planner/mod.rs` 亦存在既有格式偏差（Iteration 001 批次引入的 helper 导入简化）。本轮新增代码自身 fmt-clean；按 Surgical Changes 不动无关代码。既有偏差文件集合：`src/executor/delete.rs`、`src/parser/planner/mod.rs`、`src/storage/data/table_manager.rs`、`src/wal/recovery.rs`、`tests/cli_test.rs:3346-3823`、`tests/constraint_enforcement_test.rs`。
6. **别名匹配支未按 `has_aggregates` 分门（Blocker 修复轮）**：Review 修订契约把「顶层投影输出别名」写为无条件匹配面，实现照此落地。核对聚合路径后确认无放宽效果——聚合查询的 `sort_columns` 即 `projection_columns`，其中本就含别名（探针 `ORDER BY count_star` 与 `agg_output_columns` 装配），新增分支对聚合形态是超集且不改变判定。分门会引入无证据的额外条件（Review 明令不自行扩条件），故不加。此处记录以备 Review 核对。

**Blocker Handoff**

**发现位置**：T6，Cycle 级验证（全量串行）。

**Plan 预期与实际情况**

Plan Context 的 T6 契约与 delta R9 要求「每个排序键 SHALL 在 Sort 节点实际可见列集内解析；不可解析的键 SHALL 点名拒绝」，Risks 段并将 `ORDER BY <输出别名>` 明列为「已知边界——T6 后拒绝，能力扩展留后续」。按此实施后，全量串行出现 1 项既有测试失败：

```
test order_by_expression_alias_keeps_input_order ... FAILED
---- test_order_by_expression_alias_keeps_input_order stdout ----
expected QueryResult, got Error: Plan error: Column not found: u
```

该用例是**已验收语料规格** `openspec/specs/sql-scalar-functions/spec.md` R5 的场景「ORDER BY 引用表达式项别名静默保持输入序（既有语义锁定，SHALL NOT 报错、SHALL NOT 按别名列排序）」的 e2e 见证（`SELECT upper(name) AS u FROM t ORDER BY u`）。因此 T6 的字面实施与两处已批准文本冲突：

- 与语料 `sql-scalar-functions` R5 场景直接矛盾（要求 SHALL NOT 报错）；
- 与本 change delta `error-reporting` R4「新增的显式拒绝面 SHALL NOT 影响白名单外任何既有可达形态」矛盾；
- 本 change 携带的 delta 只覆盖 `cli-noninteractive-shell`/`error-reporting`/`sql-transaction-statements`，**无 `sql-scalar-functions` 的 MODIFIED delta**——即本 change 无权修改该已验收行为。

**需要决定**（Act 不自行选择，两条路都改变已批准文本）：

1. **收窄校验谓词（Act 倾向，代价最小）**：键既不在 Sort 输入列集、也不等于任一顶层投影项输出名（别名/表达式文本）时才算「不可解析」。`ORDER BY nosuch` 两形态仍点名拒绝（R9 目标达成），`ORDER BY <别名>` 维持变更前行为（R5/R4 保持）。契约 Forbidden「不实现 ORDER BY 输出别名解析（能力扩展）」仍然满足——不按别名排序，只是不拒绝。需 Plan 修订 T6 契约文字与 delta R9 措辞。
2. **接受行为变更并修订语料**：保留当前实现，为 `sql-scalar-functions` R5 增加 MODIFIED delta（把「SHALL NOT 报错」改为「输出别名键 SHALL 点名拒绝」），并相应修改 `tests/scalar_function_test.rs` 既有见证。属范围扩大，需用户批准。

**影响面**：当前工作区全量 `1276 passed / 1 failed / 2 ignored`，唯一失败项即上述用例；无其他连带。T4/T5 的交付面全部通过（含修复后 `subquery_test` 32/32）。T6 的两个新增见证（planner 单键/混合键拒绝、cli exit 3）均已 GREEN，三形态可达排序零回归见证亦通过。

**已完成 / 部分完成 / 未开始**（阻塞时的原始记录，保留）

- 已完成并验证：T4（全部 4 个 Requirement 场景）、T5（全部 3 个场景）。
- 部分完成：T6——实现、RED→GREEN 见证、可达形态零回归锚均就绪，仅「输出别名键」的处置待裁定。
- 未开始：无（T6 为本 Iteration 末任务）。

**已通过的 Gate**：Gate 3（T4 复审后修复 1 项 Important 回归并复跑；T5/T6 各自 spec compliance 与 code quality 通过）、Gate 4（见 Verification 全部结论，唯一失败项已定位为上述待裁定冲突而非未知原因）。

**修改文件**：`src/parser/planner/query.rs`、`src/parser/planner/expression.rs`、`tests/planner_test.rs`、`tests/cli_test.rs`、`tests/pg_integration_test.rs`（工作区未提交，与 Iteration 000/001 的交付同处一个工作树）。

**Evidence**

None required。失败现象可由既有测试命令低成本复现（`cargo test --test scalar_function_test`），决定性输出已在本节与 Verification 中；未创建 `evidence/` 目录。

**Blocker Resolution**

- Resolved: 2026-09-26
- 用户指令：「开始实施」（在助手完成上下文恢复、报告 Cycle 状态与唯一下一步后直接授权实施；未附加条件或范围变更）。
- 裁定来源：Plan Review「Blocker Handoff 处理」已采纳路径 1 并给出**修订后的 T6 契约**（以该节为准，Plan Context 不可改写）。Act 不做路径选择，只执行契约。
- 恢复点：T6 校验循环（`src/parser/planner/query.rs:1230-1253`）。恢复前工作区自阻塞结论产生后未变化（T4/T5 面与测试文件零编辑），T4/T5 的既有见证与全量结论按公共规则 › 验证 采信。
- 解决办法落地：按契约在 `sort_columns` 匹配支之后增加顶层投影输出别名匹配支——`alias_columns` 预收集 `select.projection` 中 `ExprWithAlias` 的 `alias.value`（小写），`resolvable = sort_columns 命中 || alias_columns.contains(&key)`；两者皆不命中才 `ColumnNotFound(键)`。SortNode 构造与 `sort.rs` 零改动（别名键仍由 `compare_rows` 既有 None 跳过臂保持输入序）。
- 风险与接受：按别名排序仍不实现（能力扩展，Non-goals）；`ORDER BY <别名>` 保持输入序而非报错——即语料 R5 与本 change delta R4 要求的既有语义。R9-S1（未知键点名拒绝）不受影响，单键与混合键两形态探针复核仍 exit 3。
- 所需验证（Review 指定，已全部执行）：`unknown_order_by_key_rejected`（单键 + 混合键）、`test_unknown_order_by_key_cli`、`reachable_order_by_shapes_unchanged`（三形态）仍 GREEN；`cargo test --test scalar_function_test` 32/0（语料 e2e 零修改）；全量串行零失败。实际结果见 Verification ⑧-⑫，全部达成（1277/0/2，与 Review 预期一致）。

**Self-Review**

Gate 3（spec compliance）：T4 逐场景核对——R6 四形态（未知裸列/混合/限定名 → `ColumnNotFound` 42703）有 planner/CLI/PG 三层断言，`*`、子集、重排、重复、有效限定名、大小写变体七形态零校准锚全绿；R7 别名表头 `["n"]` 与行值正确、未知别名项点名拒绝、聚合别名 `["n","count_star"]` 零回归。T5 三场景全绿，JOIN ON 既有 `ColumnNotFound` 锚与 `Table '{}' not found` 文案零校准，`explicit_rejection_messages_unchanged` 与 `pg_name_resolution_errors` 未修改通过。T6 单键/混合键拒绝与三形态可达排序成立，输出别名键按修订契约保持输入序不报错（R9-S2 闭合）。不变量核对：`PlanError` 枚举/Display/`sqlstate()` 映射表与 27 变体单测零修改；执行器（Projection/DataScan/Sort/Aggregate）零改动（`git diff --stat -- src/executor/sort.rs` 空）；聚合查询角色分类与别名装配零变化。Preserve/Forbidden 逐条成立，唯一例外（改标范围收窄）已按契约内处理记录并有探针证据。

Gate 3（code quality）：无计划外修改——`git diff` 中 `expression.rs` 除两处 `.ok_or_else` 外的差异均属 Iteration 001 已验收交付（`UnsupportedExpression` 能力名），非本轮改动。`resolve_projection_indices` 无重复实现、无死代码（唯一调用点）、无新增 warning（`cargo clippy --all-targets` exit 0，本轮修复见下）。测试经真二进制（`CARGO_BIN_EXE_rtsql`）与真实 PG socket 判定退出码/SQLSTATE/原生输出，无判定层、无身份型证据机制。新增代码 fmt-clean。

自检发现（均已处理）：

- **Important 1（已修）**：`SELECT *, name AS n FROM t` 与 JOIN 基座别名查询被 T4b 改标触发既有拒绝臂而新变为拒绝（探针 exit 3）。已按契约 Preserve 收窄 `alias_reroutable` 并复跑全部相关测试二进制与探针形态。
- **Important 2（已修）**：T4a 使子查询内字面量投影（`_N` 伪名）被误判为未知列，`subquery_test` 4 项失败。已加 `pure_column_list` 门，复跑 `subquery_test` 32/32 通过。
- **Important 3（Blocker 修复轮，已修）**：`ORDER BY <输出别名>` 被新拒绝面误伤，1 项已验收语料场景红。由 Plan Review 裁定后按修订契约修复，见 Blocker Resolution 与 Verification ⑧-⑫。
- **Minor 1（已修，Blocker 修复轮）**：新增别名匹配支初版写 `alias_columns.iter().any(|a| *a == key)`，`cargo clippy --all-targets` 报 `manual_contains` 告警（`query.rs:1250`）——本轮唯一新增告警。改为 `alias_columns.contains(&key)` 后 clippy 项目告警归零。
- **Minor 2（保留，文档精度）**：`resolve_projection_indices` 的文档注释已随 `pure_column_list` 门改写，避免「别名永不到达此处」在 DerivedScan/通配形态下失准。
- **Minor 3（记录）**：`tests/planner_test.rs` 计数由基线 76 增至 87（+11），`cli_test` 110→111（+3，其中 2 ignored 为既有）、`pg_integration_test` 5→6（+2）、`scalar_function_test` 31→32（既有语料用例，零修改）；全量 1261 → 1277（+16），与新增用例数一致。
- **Minor 4（Plan 侧，本轮零改动）**：`ORDER BY <输出别名>` 的「不按别名排序」为已知能力边界（Non-goals），可入 improvements 留后续——由 Plan/用户在收尾时裁定，Act 不落账。
- 未发现其他 Critical / Important 问题。

**Verification Evidence**

① T4 RED（`cargo test --test planner_test -- --test-threads=1`）——5 项按预期失败，症状与 Plan 探针矩阵逐条一致：

```
---- unknown_projection_column_rejected stdout ----
Expected unknown projection column rejection, got DataScan(DataScanNode { table_name: "users", columns: ["id", "name"], predicate: None, scan_cap: None, projection: [] })
---- alias_header_plan_shape stdout ----
Expected Projection for aliased column, got DataScan(DataScanNode { ... projection: [] })
test result: FAILED. 77 passed; 5 failed; 0 ignored
```

② T4 GREEN（同一命令 + T4 契约零校准集，串行）：

```
test result: ok. 82 passed; 0 failed; 0 ignored   (planner_test)
test result: ok. 110 passed; 0 failed; 2 ignored (cli_test)
test result: ok. 5 passed; 0 failed; 0 ignored    (pg_integration_test)
test result: ok. 16 passed / 12 passed / 6 passed / 16 passed  (projection_expression / aggregate / projection / pushdown)
```

（`unknown_alias_column_rejected` 首轮即通过——其文案断言只要求点名未知列，与 T5 前的 `ParseError` 文案和 T5 后的 `Column not found` 文案都相容；T5 落地后随 T5 一并复跑，见 ④。）

③ T4 真二进制探针矩阵（`/tmp/opencode/ms19-i002`，表 `t2(id INT PRIMARY KEY, name STRING NOT NULL)` 两行）：

```
SELECT nosuchcol FROM t2            → exit=3 Plan error: Column not found: nosuchcol
SELECT id, nosuchcol FROM t2        → exit=3 Plan error: Column not found: nosuchcol
SELECT t2.nosuch FROM t2            → exit=3 Plan error: Column not found: nosuch
SELECT name AS n FROM t2            → exit=0 {"columns":["n"],"rows":[["a"],["b"]]}
SELECT name AS n FROM t2 WHERE id=1 → exit=0 {"columns":["n"],"rows":[["a"]]}
SELECT t2.id FROM t2                → exit=0 {"columns":["id"],"rows":[[1],[2]]}
```

④ T5 RED→GREEN（`cargo test --test planner_test --test pg_integration_test -- --test-threads=1`）：RED 两项失败于文案 `left: "Parse error: Column 'nosuch' not found in table 'users'"`；GREEN 后

```
test result: ok. 85 passed; 0 failed; 0 ignored   (planner_test)
test result: ok. 5 passed; 0 failed; 0 ignored    (pg_integration_test)
```

T5 探针两形态与表缺失对照：

```
SELECT id FROM t2 WHERE nosuch = 1  → exit=3 Plan error: Column not found: nosuch
SELECT abs(nosuch) FROM t2          → exit=3 Plan error: Column not found: nosuch
SELECT id FROM nosuchtable          → exit=3 Table 'nosuchtable' not found: table not found: nosuchtable  （既有 ParseError 文案不变）
```

⑤ T6 RED→GREEN（planner 两项；`reachable_order_by_shapes_unchanged` 首版断言形态错误，修正后作锚）：GREEN 后 `test result: ok. 87 passed; 0 failed`。T6 探针（真实排序结果）：

```
SELECT id FROM t2 ORDER BY nosuch                      → exit=3 Column not found: nosuch
SELECT id FROM t2 ORDER BY nosuch, name DESC           → exit=3 Column not found: nosuch
SELECT id FROM t2 ORDER BY name DESC                   → exit=0 rows=[[2],[1]]
SELECT id + 1 AS d FROM t2 ORDER BY name DESC          → exit=0 rows=[[3],[2]]
SELECT name, COUNT(*) FROM t2 GROUP BY name ORDER BY count_star DESC → exit=0 rows=[["a",1],["b",1]]
SELECT name AS n FROM t2 ORDER BY n                    → exit=3 Column not found: n   ← Blocker 冲突形态
```

⑥ 全量串行（**修复前**，M19 约束，`cargo test --no-fail-fast -- --test-threads=1`；`--no-fail-fast` 用于取得完整失败集合，非重复验证）：

```
passed=1276 failed=1 ignored=2
test order_by_expression_alias_keeps_input_order ... FAILED
```

（基线 1261 + 本轮新增 16 = 1277 = 1276 + 1。修复 Important 2 之前同一命令的失败集合为 5 项：`scalar_function_test` 1 + `subquery_test` 4。）

⑦ 静态检查与规格校验（**修复前**，随本轮重跑，结论见 ⑪）：

- `cargo clippy --all-targets` → exit 0（无本项目告警；仅 `~/.cargo/config` 弃用提示）
- `cargo fmt --check` → 本轮新增/触碰代码零偏差；既有偏差文件集合见 Deviations 5（本轮新增代码经 `rustfmt` 修正的是 `query.rs`/`planner_test.rs`/`pg_integration_test.rs` 的新代码段，`cli_test.rs` 新增段 4001 行起无偏差）
- `openspec validate 2026-09-26-ms19-cli-admin-hardening --strict` → `Change '2026-09-26-ms19-cli-admin-hardening' is valid`，exit 0

⑧ **Blocker 修复轮 RED 见证**（恢复点重新建立见证；`cargo test --test scalar_function_test order_by_expression_alias_keeps_input_order`）：

```
test order_by_expression_alias_keeps_input_order ... FAILED
Expected QueryResult, got Error { message: "Plan error: Column not found: u", sqlstate: Some("42703") }
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 31 filtered out
```

（观察值与 Plan Review 定向复跑一致，证实覆盖未变化、失败可归因于字面实现而非偶发。）

⑨ **Blocker 修复轮 GREEN**（`cargo test --test scalar_function_test`）——语料 R5 e2e 零修改恢复：

```
test result: ok. 32 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

⑩ **Review 指定的定向见证复核**（`cargo test --test planner_test --test cli_test --test pg_integration_test --test scalar_function_test --test tx_statement_test`，定向运行按 M19 允许默认并行度）：

```
test result: ok. 111 passed; 0 failed; 2 ignored  (cli_test，含 test_unknown_order_by_key_cli exit 3)
test result: ok. 6 passed; 0 failed; 0 ignored     (pg_integration_test)
test result: ok. 87 passed; 0 failed; 0 ignored    (planner_test，含 unknown_order_by_key_rejected 单键+混合键、reachable_order_by_shapes_unchanged 三形态)
test result: ok. 32 passed; 0 failed; 0 ignored    (scalar_function_test)
test result: ok. 21 passed; 0 failed; 0 ignored    (tx_statement_test，Iteration 001 交付零回归)
```

⑪ **Blocker 修复轮真二进制探针**（`/tmp/opencode/ms19-i002b`，表 `t2(id INT PRIMARY KEY, name STRING NOT NULL)` 两行，插入序 a、b）：

```
SELECT id FROM t2 ORDER BY nosuch                      → exit=3 Plan error: Column not found: nosuch
SELECT id FROM t2 ORDER BY nosuch, name DESC           → exit=3 Plan error: Column not found: nosuch
SELECT name AS n FROM t2 ORDER BY n                    → exit=0 {"columns":["n"],"rows":[["a"],["b"]]}      ← 别名键保持输入序
SELECT upper(name) AS u FROM t2 ORDER BY u             → exit=0 {"columns":["u"],"rows":[["A"],["B"]]}      ← 语料 R5 形态
SELECT id FROM t2 ORDER BY name DESC                   → exit=0 {"columns":["id"],"rows":[[2],[1]]}
SELECT id + 1 AS d FROM t2 ORDER BY name DESC          → exit=0 {"columns":["d"],"rows":[[3],[2]]}
SELECT name, COUNT(*) FROM t2 GROUP BY name ORDER BY count_star DESC → exit=0 rows=[["b",1],["a",1]]
SELECT id, name AS n FROM t2 ORDER BY n                → exit=0 rows=[[1,"a"],[2,"b"]]                       ← 混合清单别名键
```

（别名两形态输出为插入序 a→b 而非按键排序——即「不拒绝、不按别名排序」的契约语义；未知键两形态仍点名拒绝。）

⑫ **Blocker 修复轮 Cycle 级验证与静态检查**：

- 全量串行（`cargo test --no-fail-fast -- --test-threads=1`，M19 约束）→ `passed=1277 failed=0 ignored=2`（82 个测试二进制，退出码 0，无 FAILED 行）。与 Review 预期 1277/0/2 一致：1261 基线 + 16 新增，无白名单外失败。
- `cargo clippy --all-targets` → 项目告警 0（Minor 1 修复后归零；仅 `~/.cargo/config` 弃用提示）
- `rustfmt --check --edition 2021 src/parser/planner/query.rs` → exit 0
- `git diff --stat -- src/executor/sort.rs` → 空（Forbidden「不改 sort.rs」成立）
- `openspec validate 2026-09-26-ms19-cli-admin-hardening --strict` → `Change '2026-09-26-ms19-cli-admin-hardening' is valid`，exit 0

**Persisted Evidence**

None required（Plan 设定 Mode: none，予以维持）。全部验证为低成本可重跑命令与真二进制探针，结论已落在本节；探针现场 `/tmp/opencode/ms19-i002`（修复前）与 `/tmp/opencode/ms19-i002b`（修复后）均为一次性环境，无保留价值，不建 `evidence/` 目录。

**Experience Candidates**

- Runbook 候选：None（本 Cycle 无「结果无法低成本复现」或「高风险且非显然」的操作路径；探针库两行数据 + one-shot 查询即全部现场）。
- Issue 候选：None。唯一失败项是本 change 范围内的待裁定冲突（见 Blocker Handoff），已由 Plan Review 裁定并在当前 Cycle 修复，按记录边界不作为 Issue 落账。无范围外已证据的实质缺陷（D9 无聚合 GROUP BY 忽略仍按 Excluded scope 记录为候选，未变）。

**Remaining Issues**

- Critical / Important：无。Important 1-3 全部已修（Important 3 = Blocker 冲突，由 Plan Review 裁定后在当前 Cycle 修复，见 Blocker Resolution）。
- Minor：见 Self-Review——Minor 1（clippy `manual_contains`，已修）、Minor 2（文档注释精度，已改写）、Minor 3（计数簿记，保留在案）、Minor 4（`ORDER BY` 输出别名不参与排序的能力边界，Plan/用户裁定项，Act 不落账）。

## Plan Review

- Review Result: accepted
- Reviewed: 2026-09-26（第 2 轮：Blocker 修复后重新 Review。第 1 轮审计已独立核实 T4/T5/T6 字面实现与全部声明、裁定 Blocker 为 Plan 起草错误并采纳收窄谓词路径，结论摘要由 Act Response › Blocker Resolution 保留；本轮覆盖为修复后最新完整审查）

**独立检查结果（修复轮）**

- 修复代码与修订契约逐字一致：`query.rs:1237-1254`——`alias_columns` 预收集 `select.projection` 中 `ExprWithAlias` 的 `alias.value`（小写）；`resolvable = sort_columns 命中（大小写不敏感）|| alias_columns.contains(&key)`；两者皆不命中 → `ColumnNotFound(键)`。SortNode 构造不变，`src/executor/sort.rs` 零改动（`git diff --stat` 空），注释携带语料 R5 依据。
- diff 范围核查：query.rs 仍 5 hunks（T6 hunk +19→+36 即修复增量）、expression.rs hunk 集不变；`tests/scalar_function_test.rs` 的 ` M` 仅为 Iteration 001 的 `Response::Error { message, .. }` 机械适配，语料 e2e `order_by_expression_alias_keeps_input_order` 本体零修改——「零修改恢复 GREEN」声明成立。
- Deviation 6（别名匹配支未按 `has_aggregates` 分门）核对成立：聚合查询 `sort_columns = projection_columns` 本就含别名，新增支为无害超集，不产生放宽；分门才是无证据的额外条件。接受。
- Act 验证 ⑧-⑫ 采信（修复后全量新鲜，此后工作区零变化）：修复轮 RED 见证 → 语料 e2e `scalar_function_test` 32/0 → Review 指定定向见证（planner 87 / cli 111 / pg_integration 6 / scalar 32 / tx_statement 21）→ 真二进制探针 8 形态（`ORDER BY n`/`ORDER BY u` 保持输入序、未知键单/混合拒绝、三可达形态、混合清单别名键）→ 全量串行 **1277 passed / 0 failed / 2 ignored**（与第 1 轮 Review 预期一致：1261 基线 + 16 新增）→ clippy 项目告警 0 / fmt clean / `openspec validate --strict` 通过。

**Findings（第 2 轮）**

1. 修复完全落在 Follow-up Decision 契约内，无新偏差；Act 修复轮自检 Minor 1（clippy `manual_contains`）已修并复跑归零。
2. 非阻塞 Minor 保留：计数簿记（Minor 3）；`ORDER BY <输出别名>` 不参与排序的能力边界（Minor 4）——属可选改进候选（SQLite 支持按别名排序），收尾时由用户裁定是否入 improvements，不阻塞本 Review。
3. 无身份型证据工程、无判定层、无范围外改动。

**Acceptance Gaps**: None（R9-S2 已闭合——语料 e2e 零修改 GREEN + 探针别名两形态输入序；R6 四场景、R7 三场景、R8 三场景、R9-S1 前轮已由见证满足）

**Iteration Plan Update**: None
**Next Cycle**: None
**Next Iteration**: None（Iteration 002 为本 change 末 Iteration——000/001/002 三轮全 accepted，change 实施面完成，待收尾）

**Issue 候选**: None（D9 无聚合 GROUP BY 忽略维持 Excluded scope 候选记录，不落账）

**Runbook 候选**: Iteration 000「文件对备份边界实测」候选维持（父 Cycle Review 已确认合格），由 Recorder 按用户指令建立产物。
