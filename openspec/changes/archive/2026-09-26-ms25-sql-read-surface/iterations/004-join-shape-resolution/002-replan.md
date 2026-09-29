# Iteration 004 / Cycle 002 (replan): 聚合参数计划期解析统一与限定通配文案可操作化

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T12, T13（T8 的参数面残留收口项并入 T12 契约）
- Acceptance gaps: T8 ③ 部分缺口（JOIN 基座聚合参数未知列静默 NULL，Cycle 001 Plan Review Finding 1）
- Repair items: None（收口项并入 T12 全局任务，不建本地 repair item）
- Inherited scope: `error-reporting` MODIFIED R6（聚合参数列缺失分类）、MODIFIED R1（限定通配点名场景）；design D12；T12/T13 用户指令并入（2026-09-28 二次 replan）；**已完成基线继承**——T8 主体、T9、T10、T11 已在 Cycle 001 交付并经 Plan Review 核对通过（Act Response 验证：全量串行 1401 passed / 0 failed / 2 ignored，两处白名单校准、10 轮信号压测、clippy/fmt/validate 干净），本 Cycle 不重做，其交付面作为零回归锚
- Excluded scope: `SELECT t.*` 限定通配投影能力本身（T13 只修文案，能力支持为独立方向）；GROUP BY 分组键列缺失的措辞变更（维持既有 `NonAggregatedColumn` 终态，Cycle 001 Review Finding 2 裁定）；表达式项×JOIN 解禁；信号语义扩展；MS21-T03；代价模型

**Background**

父 Cycle 001（001-replan.md）交付 T8 主体/T9/T10/T11，Plan Review 核对通过但发现一项实质缺口（T8 ③：JOIN 基座聚合参数未知列静默 NULL，实测 `count(nosuch)` → `[[0]]`、`sum(t1.nosuch)` → `[[null]]`），原定「当前 Cycle 修复」。用户指令（2026-09-28）扩大范围：把该收口项与两项 Issue 候选（单表面聚合未知名参数静默 NULL、`SELECT t.*` 拒绝文案泛化）合并为一轮 replan 直接解决——统一为 T12（聚合参数计划期解析，两基座一次收口，避免同一构造点两轮触碰）与 T13（限定通配文案可操作化）。父 Cycle Review Result 已置 `replan-required`，change 的 tasks/design（新增 D12）/delta specs（error-reporting MODIFIED R1/R6）已同步修订。

**Objective**

聚合函数参数在 JOIN 与单表面均计划期解析：未知列、限定名未知列、别名引用一律 `ColumnNotFound` 计划期点名（静默 0/NULL 终止），JOIN 裸名歧义 `AmbiguousColumn` 保持，既有聚合形态零回归；`SELECT t.*` 三基座拒绝文案点名限定通配能力（泛化 `SELECT ITEM` 终止）；已交付面（Iteration 000–003 与 Cycle 001 全部交付）零回归。

**Investigation Facts**

- Current Baseline: Cycle 001 最终 Act Response（Plan Review 核对采信，2026-09-28）——T8 主体/T9/T10/T11 落地，全量串行 `cargo test -- --test-threads=1` 1401 passed / 0 failed / 2 ignored（总数 1403 = Iteration 003 基线 1376 + 27 新用例）；`join_output_resolution_test` 12、`aggregate_output_shape_test` 8、`derived_scan_alias_test` 7、`planner_test` 87、`aggregate_extension_test`、`subquery_test` 32、`cli_test` 111/2i。本 Cycle 目标文件 `aggregate_arg_indices` 构造段（query.rs:1808-1826）与 ast.rs 兜底臂自该验证后零变化。
- Current-State Evidence:
  - **参数收集面**：SELECT 项循环同步收集 `aggregate_args: Vec<Option<Expr>>`（query.rs:1097/:1133/:1155，`aggregate_arg_expr` 取聚合调用首个普通实参——`COUNT(*)` → `None`；字面量实参在 `extract_aggregate_func` 的 `expr_to_column_name_static` 门即拒绝，故 `Some` 恒为 `Identifier` 或 2 段 `CompoundIdentifier`）。
  - **病灶构造点**（query.rs:1808-1826）：`aggregate_arg_indices` 仅在 `join_layout = Some` 时逐参 `resolve_layout_index` 解析；零命中返回 `Ok(None)` 直接落入索引 `None`（`map_err` 的 `ColumnNotFound` 分支为死代码——该 helper 零命中返回 `Ok(None)`、歧义返回 `Err(AmbiguousColumn)`，query.rs:2498-2528）；`join_layout = None`（单表面）→ `vec![None; aggregates.len()]` 全回落。运行期 `extract_value`（aggregate.rs:414-435）`arg_index.or_else(|| column_indices.get(func.column()))`，两级皆无 → `Value::Null` 静默。
  - **实测现状**（2026-09-28，本 Plan，夹具 t(a,b)=[(1,'x'),(2,'y')]、t2(c)=[(9)]）：单表面 `SELECT count(nosuch) FROM t` → `{"columns":["count_nosuch"],"rows":[[0]]}` exit 0；别名参数 `SELECT upper(b) AS u, count(u) FROM t GROUP BY u` → `count_u` 全 0 静默（"u" 非输入列）；JOIN 面 `count(nosuch)` → `[[0]]`、`sum(t1.nosuch)` → `[[null]]`（Cycle 001 Review 已实测）。
  - **单表面解析基座**：`column_indices: HashMap<String, usize>`（query.rs:1459-1464，键 = `get_plan_output_columns(&plan_with_where)` 小写列名）——单表无歧义概念，命中即输入行索引；限定名参数现状经 `expr_to_column_name_static` 取末段名消费（`count(t.v)` → "v"），T12 维持末段名语义（可解析形态零变化，仅查无转点名）。
  - **T13 病灶**：`SELECT t.*`（sqlparser 0.44 `SelectItem::QualifiedWildcard(ObjectName, WildcardAdditionalOptions)`，query.rs:2123 既有 no-FROM 拒绝臂先例）落入 ast.rs 两处兜底臂 `_ => Err(PlanError::UnsupportedStatement("SELECT ITEM".into()))`（extract_columns ~:163、extract_qualified_columns ~:298）——实测单表面与 JOIN 基座均 `Unsupported statement type: SELECT ITEM` exit 3。文案框架 `Unsupported statement type: {能力名}`（planner_test:1591 `MERGE`、:1611 `DROP VIEW` 先例）；no-FROM 路径已有能力指称先例 `"Qualified wildcard is not supported without FROM"`（query.rs:2123-2127）。
  - **零校准清扫**（2026-09-28，本 Plan）：全 tests/ 聚合参数形态扫描——`count(v)/max(v)/min(v)/sum(v)`（aggregate_extension_test）、`min(n)`（cli_test）、`count(k)/count(v)/max(v)`（join_output_resolution_test）均为基列引用；`visibility_test` 的 `current_count` 为 Rust 局部变量非 SQL 参数；`subquery_test` 32 用例无聚合调用；**无任何测试使用限定名聚合参数或别名聚合参数**——T12 的行为变化面（静默 → 点名）零既有断言锁定。GROUP BY 键解析链（`resolve_group_by_item` 四级 + `NonAggregatedColumn` 终态）不在本 Cycle 触碰面。
  - **错误面载体**：`PlanError::ColumnNotFound(String)` / `PlanError::AmbiguousColumn(String)` 既有（src/parser/error.rs:25/:29）；`UnsupportedStatement` → PG SQLSTATE 0A000（MS19 R2 分类面，文案变更不影响分类）。
  - **测试入口**：`tests/join_output_resolution_test.rs`（T12 JOIN 面 2 场景追加；先例与夹具同文件）、`tests/planner_test.rs`（T12 单表面 2 场景 + T13 文案 2 断言；错误面矩阵先例 :1591/:1611）。
- Code and Critical Path: `aggregate_arg_indices` 构造（query.rs 聚合分支）→ `AggregateNode.aggregate_arg_indices`（plan.rs）→ `AggregateExecutor::extract_value`（aggregate.rs）；ast.rs `extract_columns`/`extract_qualified_columns` 兜底臂 → planner 各 `extract_*` 消费点（拒绝发生在提取期，早于任何形状装配）。

**Implementation Guidance**

TDD：T12/T13 各自先立见证取 RED（P1/P2/P11/P12/P9/P10 现状形态均 Plan 实测），再实现 GREEN。T12 与 T13 相互独立，顺序不约束。

T12 落位建议（契约 ①..⑤ 的机制方向，等价机制可）：`aggregate_arg_indices` 构造从「JOIN 布局独享」改为统一逐参解析——`Some(Expr::Identifier(ident))`：JOIN 布局面 `resolve_layout_index(layout, None, …)` / 单表面 `column_indices.get(ident.value.to_lowercase())`；`Some(Expr::CompoundIdentifier(parts)) if parts.len() == 2`：JOIN 布局限定名解析 / 单表面按末段名 `column_indices.get(parts[1])`（既有末段语义，可解析形态零变化）；命中 → `Some(idx)`，**零命中 → `Err(PlanError::ColumnNotFound(名))`**（裸名 = `ident.value`、限定名 = 末段名），JOIN 布局裸名多命中 → `Err(AmbiguousColumn)`（既有透传）；`None`（`COUNT(*)`）→ `Ok(None)`。现 `map_err` 死分支随之清理。`extract_value` 的名字查表回退对列参数成为死路径——保留为防御（非实质，不删）。

T13 落位建议：ast.rs 两处兜底臂 `"SELECT ITEM"` → `"QUALIFIED WILDCARD"`（能力名 token 与 no-FROM 路径 "Qualified wildcard…" 的指称一致；跟随 R1 文案框架）。若两处 `_` 臂未来可能承接其他变体，可显式 `SelectItem::QualifiedWildcard(..)` 匹配臂 + 保留泛化兜底（等价，非实质）——但**拒绝语义与覆盖面零变化**。

**Task Contracts**

### T12: 聚合参数计划期解析统一（`error-reporting` MODIFIED R6；含 T8 ③ 残留收口）

- Requirement/Scenario: delta R6 新增 4 场景——聚合参数列缺失点名（单表面 + JOIN 基座）/ 限定名列缺失点名 / 别名引用点名 / 既有聚合零回归；RTM witness 见 Task List
- Depends on: Cycle 001 交付面（JOIN 布局解析 helper 与 `aggregate_args` 收集已在位）
- Targets: `src/parser/planner/query.rs::aggregate_arg_indices` 构造（:1808-1826 统一两面解析）、`tests/join_output_resolution_test.rs`（+2）、`tests/planner_test.rs`（+2）
- Current behavior: 单表面未知参数 `count(nosuch)` → `[[0]]` exit 0；别名参数 `count(u)` → 全组 0 静默；JOIN 面未知参数 `count(nosuch)` → `[[0]]`、`sum(t1.nosuch)` → `[[null]]` exit 0（均 Plan 实测）
- Required behavior: ① 单表面：聚合参数经 `column_indices` 计划期解析（`Identifier` 小写名 / `CompoundIdentifier` 末段名），命中 → `Some(idx)`，查无 → `ColumnNotFound` 点名；② JOIN 面：零命中（`Ok(None)`）→ `ColumnNotFound` 点名（裸名 `ident.value` / 限定名末段名），裸名多命中 `AmbiguousColumn` 保持；③ `None` 参数（`COUNT(*)`）维持 `Ok(None)`；④ 行为变化明示：未知列与别名引用由静默 0/NULL 转计划期点名（PG 42703 同类，exit 3）——零既有断言锁定（清扫核实）；⑤ 可解析形态零变化：单表面 `count(v)`/`count(t.v)`、JOIN 面 `sum(p.x)`/`count(k)` 唯一命中等既有正确形态逐字节不变
- Preserve: GROUP BY 键解析面（`resolve_group_by_item` 四级链与 `NonAggregatedColumn` 终态）零变化；`extract_value` 对 `Some(idx)` 的求值路径与既有越界防御零变化；Cycle 001 已交付全部面（T8 主体/T9/T10/T11）零回归；M17/M19
- Forbidden: 不改 GROUP BY 键面措辞；不实现聚合参数的别名解析（别名引用按列缺失点名）；不为运行期查表新增静默 NULL 分支；白名单外不修改既有测试断言（本任务白名单空集）
- Test witness: `join_output_resolution_test` 增 `unknown_bare_agg_arg_rejected`/`unknown_qualified_agg_arg_rejected`（现状 RED：P1/P2 形态）+ `planner_test` 增 `unknown_agg_arg_single_table_rejected`/`alias_agg_arg_rejected`（现状 RED：P11/P12 形态）——先 RED 后 GREEN
- GREEN condition: 4 新见证全绿 + `aggregate_extension_test`/`aggregate_test`/`group_by_expr_test`/`expression_e2e_test`/`subquery_test`/`cli_test` 零校准 + 全量串行 0 failures
- Verification: ①定向 `cargo test --test join_output_resolution_test --test planner_test`（退出码 0）；②受影响边界 `cargo test --test aggregate_extension_test --test aggregate_test --test group_by_expr_test --test expression_e2e_test --test subquery_test --test cli_test --test aggregate_output_shape_test --test derived_scan_alias_test`（退出码 0）；③全量 `cargo test -- --test-threads=1`（退出码 0）；④clippy/fmt（退出码 0）
- Stop when: 单表面解析撞上未识别的合法形态（既有测试需校准）——返回 Plan；GROUP BY 键面被牵动——返回 Plan；别名引用存在正当语义需求（如关联子查询聚合参数引用外层列的正确消费形态）——返回 Plan

### T13: 限定通配拒绝文案可操作化（`error-reporting` MODIFIED R1）

- Requirement/Scenario: delta R1 新增 1 场景（限定通配点名拒绝，单表面 + JOIN 基座同断言）
- Depends on: None
- Targets: `src/parser/ast.rs` 两处兜底臂（extract_columns ~:163、extract_qualified_columns ~:298）、`tests/planner_test.rs`（+1 用例含两断言）
- Current behavior: `SELECT t.* FROM t` 与 `SELECT t.* FROM t JOIN t2 ON …` 均 `Unsupported statement type: SELECT ITEM` exit 3（Plan 实测 P9/P10）
- Required behavior: ① 两处兜底臂文案改为能力点名 `Unsupported statement type: QUALIFIED WILDCARD`（exit 3，PG 0A000 分类不变）；② 拒绝语义与覆盖面零变化（单表面/派生表/JOIN 三基座同拒，不实现 `t.*` 投影）；③ 其余错误文案零变化（`MERGE`/`DROP VIEW`/no-FROM 路径 "Qualified wildcard is not supported without FROM" 等不动）
- Preserve: `UnsupportedStatement` → 0A000 分类与 JSON 序列化字节不变（仅 message 载荷变化）；no-FROM 路径既有拒绝臂零变化；M17/M19
- Forbidden: 不实现限定通配投影；不改动通配相关其他文案（`SELECT * is not supported without FROM`、`SELECT * cannot be mixed…` 等）；白名单外不修改既有测试断言（本任务白名单空集）
- Test witness: `planner_test` 增 `qualified_wildcard_capability_named`（单表面 + JOIN 基座两断言，`msg.starts_with("Unsupported statement type: QUALIFIED WILDCARD")`，跟随 :1591 断言模式）——现状 RED（"SELECT ITEM"）后 GREEN
- GREEN condition: 新见证绿 + `planner_test`/`cli_test` 零校准 + 全量串行 0 failures
- Verification: ①定向 `cargo test --test planner_test`（退出码 0）；②受影响边界 `cargo test --test cli_test --test pg_integration_test`（退出码 0）；③全量与静态检查同 T12 ③④（合并执行）
- Stop when: 兜底臂存在 `QualifiedWildcard` 以外的可达变体（改文案会误伤其他形态的语义）——返回 Plan；既有测试锁定 "SELECT ITEM" 文案——返回 Plan

**Invariants**

- 静默错误结果红线：聚合参数的列缺失在任何执行发生前计划期点名（T12 的目标即把 Cycle 001 的 JOIN 面收口扩展到全部基座）。
- 已交付面零回归：Iteration 000–003 与 Cycle 001 全部交付（T8 主体/T9/T10/T11）的计划形状、结果、表头、文案逐字节不变。
- 错误分类一致性：列缺失恒 `ColumnNotFound`（42703），能力拒绝恒 `UnsupportedStatement`（0A000）且携带能力名。
- 白名单纪律：本 Cycle 白名单为空集——任何既有测试需校准即返回 Plan。

**Non-goals**

- `SELECT t.*` 投影能力；GROUP BY 键措辞；关联子查询聚合参数的外层列正确消费语义（T12 按列缺失点名，正确关联语义为独立方向）；表达式项×JOIN；MS21-T03；代价模型。

**Acceptance**

`error-reporting` MODIFIED R6 四场景与 MODIFIED R1 新场景可观察满足；T8 ③ 缺口关闭（JOIN 面聚合参数未知列点名）；单表面与别名参数静默面消除；既有聚合形态与全部已交付面零回归；全量串行零回归。Scenario→Task→Test 链路：RTM `error-reporting` MODIFIED R6 一行 + MODIFIED R1 一行（change tasks.md）。

**Verification**

- T12：`tests/join_output_resolution_test.rs` 14 场景（12 既有 + 2 新）+ `tests/planner_test.rs` 91（87 + 2 T12 + 2 T13）+ 聚合/表达式/子查询/cli 边界 + 全量串行。
- T13：`tests/planner_test.rs` 文案断言 + cli/pg 边界。
- 汇总：全量 `cargo test -- --test-threads=1` 退出码 0（预期总数 1403 + 6 = 1409，2 ignored）+ clippy/fmt 干净 + `openspec validate 2026-09-26-ms25-sql-read-surface --strict` 退出码 0。
- 判定均为测试自身断言 + cargo 退出码；无身份型证据工程。

**Gate 2 Readiness**

PASS——无 Missing requirement（RTM 两行 Covered，tasks.md 已同步）；调查完整（病灶构造点 file:line 实证 + 六组实测复现 P1/P2/P9/P10/P11/P12 + 零校准全库清扫〔聚合参数形态、别名参数形态、限定名参数形态、关联子查询聚合形态〕+ sqlparser 变体与文案框架先例核实）；设计闭合（D12 方案明确，T12 吸收 Cycle 001 修复项避免同构造点两轮触碰）；任务契约自包含（Act 无需回读父 Cycle）；验证直接判定（测试断言 + 退出码）；无实质未知项。change 级 Gate 2（2026-09-26 用户批准）覆盖本计划；T12/T13 并入经用户指令（2026-09-28 二次 replan）。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果；无一次性环境、无不可复现结论。

**Risks and Notes**

- T12 行为变化面：聚合参数引用 SELECT 输出别名/未知列由静默转拒绝——清扫核实零既有断言锁定；若 Act 在实施中发现隐藏依赖（如关联子查询聚合参数经名字查表现状「碰巧正确」的形态），按 Stop when 返回 Plan。
- 关联子查询的聚合参数现状语义：内层聚合参数按名字绑定内层输入行（外层列引用不经关联机制替换，内层缺同名列即静默 NULL）——T12 后转为点名。该形态的正确关联消费属独立方向（Non-goal），本 Cycle 不实现。
- `extract_value` 的名字查表回退保留为防御性死路径——删除与否为非实质选择，Act 裁定（删除需同步收紧 `AggregateNode` 字段语义注释）。
- T13 文案 token `QUALIFIED WILDCARD` 为契约指定值（与 no-FROM 路径指称一致）；若 Act 认为其他 token 更贴切，属非实质偏差，记录进 Act Response 即可（测试断言须与实现一致）。
- 预期全量总数 1409 = 1403 + 6 新见证（T12×4 + T13×2；planner_test 87→91、join_output_resolution_test 12→14）；若计数不符需在 Act Response 说明。

## Act Response

- Status: reported

### 交付

T12、T13 全部交付。Iteration 004 六任务（T8 主体/T9/T10/T11 由 Cycle 001 交付 + T12/T13 本轮）实现面闭合。

**T12——聚合参数计划期解析统一**（`src/parser/planner/query.rs:1804-1845`）

`aggregate_arg_indices` 构造从「`join_layout` 独享 / 单表面回落 `vec![None; n]`」改为两基座统一逐参解析：

- `Identifier`：JOIN 面经 `resolve_layout_index(layout, None, …)`；单表面经 `column_indices.get(小写名)`。
- `CompoundIdentifier(2)`：JOIN 面经 `resolve_layout_index(layout, Some(段), 列)`；单表面按末段名查 `column_indices`（既有末段语义逐字节保持）。
- 零命中一律 `PlanError::ColumnNotFound(名)`（裸名 `ident.value`、限定名末段名）；JOIN 面裸名多命中 `AmbiguousColumn` 透传不变。
- `None`（`COUNT(*)`）维持 `Ok(None)`；非列引用参数（字面量等）维持既有 `Ok(None)` 防御臂。
- Plan 预判的 `map_err` `ColumnNotFound` 死分支（`resolve_layout_index` 零命中恒返 `Ok(None)`）随统一解析消除。
- `aggregate_args` 与 `aggregates` 在 SELECT 项循环（query.rs:1133/1134、1155/1156）同步 push，长度恒等——原 `None` 分支按 `aggregates.len()` 填充的前提由逐参映射等价承接。

**T13——限定通配拒绝文案可操作化**（`src/parser/ast.rs:162-168`、`:301-307`）

`extract_columns` / `extract_qualified_columns` 两处兜底臂 `"SELECT ITEM"` → `"QUALIFIED WILDCARD"`。核实 `SelectItem`（sqlparser 0.44）仅四变体且前三个已显式匹配（`UnnamedExpr` / `ExprWithAlias` / `Wildcard`），兜底臂只承接 `QualifiedWildcard`——采用契约指定的最小改法（改文案），未拆显式匹配臂。拒绝语义与覆盖面零变化，`UnsupportedStatement` → 0A000 分类与 JSON 序列化字节不变。

### 测试见证

新增 5 个测试函数 / 6 个新见证断言（`tests/join_output_resolution_test.rs` +41 行、`tests/planner_test.rs` +78 行）：

| 见证 | 现状（RED 实测形态） | 现行为 |
|---|---|---|
| `unknown_bare_agg_arg_rejected` | `QueryResult { rows: [[0]] }` | `Column not found: nosuch`（T8 ③ 缺口） |
| `unknown_qualified_agg_arg_rejected` | `QueryResult { rows: [[Null]] }` | `Column not found: nosuch` |
| `unknown_agg_arg_single_table_rejected` | `Aggregate(aggregate_arg_indices: [None], aggregates: [Count("nosuch")])` | `ColumnNotFound` |
| `alias_agg_arg_rejected` | 同上全组 count 0 | `ColumnNotFound: u` |
| `qualified_wildcard_capability_named` | `Unsupported statement type: SELECT ITEM` | `…: QUALIFIED WILDCARD`（单表面 + JOIN 两断言） |

RED 观察发生在产品代码改动之前，5 失败形态与 Plan P1/P2/P9/P10/P11/P12 预测逐一致。

### 验证

| 验证 | 结果 | 退出码 |
|---|---|---|
| `cargo test --test join_output_resolution_test --test planner_test` | 14 + 90 passed / 0 failed | 0 |
| `cargo test --test aggregate_extension_test --test aggregate_test --test group_by_expr_test --test expression_e2e_test --test subquery_test --test aggregate_output_shape_test --test derived_scan_alias_test` | 13+19+12+24+32+8+7 = 115 passed / 0 failed（零校准） | 0 |
| `cargo test --test cli_test --test pg_integration_test` | 111 passed / 2 ignored + 6 passed | 0 |
| `cargo test -- --test-threads=1`（全量串行） | 退出码 0，零 failures | 0 |
| `cargo clippy --all-targets` | 无警告 | 0 |
| `cargo fmt --check` | 通过（首次运行后应用 `cargo fmt`，复检通过） | 0 |
| `openspec validate 2026-09-26-ms25-sql-read-surface --strict` | `Change '…' is valid` | 0 |

全量总数 1408（`cargo test -- --list` 静态枚举，`grep -c ": test$"` = 1408）。Plan 预期 1409，差 1：Plan 按 6 个见证函数计数，实际 T13 的单表面 + JOIN 两断言合并在**同一个**测试函数内（`qualified_wildcard_capability_named`），故为 5 函数 + 1 额外断言 = 1408。断言覆盖与 Plan 要求一致，仅函数计数口径差异。

### Deviations from Plan

1. **T13 未拆显式 `QualifiedWildcard(..)` 匹配臂**（Plan 标注为「等价，非实质」二选一）。采用改文案最小实现——已核实兜底臂只承接该变体，行为等价。
2. **全量总数 1408 ≠ Plan 预期 1409**，原因见上（T13 两断言合一函数）。断言覆盖无缺口。
3. **无实质偏差**：T12 落位与 Plan Implementation Guidance 的机制方向一致（逐参统一解析 + 零命中点名 + `None`/`COUNT(*)` 保持），仅错误传播写法由 `map_err` 改为 `and_then(..ok_or_else(..))` 以消除死分支。

### 自检发现（Gate 3）

- **Spec compliance**：`error-reporting` MODIFIED R6 四场景与 MODIFIED R1 新场景均实现并有见证；`join-output-shape-resolution` R1-R4 未触碰；GROUP BY 键解析面（`resolve_group_by_item` 四级链 + `NonAggregatedColumn` 终态）逐字未动；M17/M19 错误分类面未动（`ColumnNotFound`→42703、`UnsupportedStatement`→0A000 为既有映射，复用未改）。
- **可解析形态零变化**实测：单表面 `SELECT count(b), count(t.b) FROM t` → `{"columns":["count_b","count_b"],"rows":[[1,1]]}` exit 0，限定名与裸名两形态逐字节一致；CLI 两个新错误面均 exit 3（`statement 1 of 1 failed: Plan error: Column not found: nosuch` / `… Unsupported statement type: QUALIFIED WILDCARD`）。
- **Code quality**：diff 无计划外修改（产品码 2 文件 +47/-24，测试 2 文件 +119）；无新增死代码——Plan 预判的 `map_err` 死分支已删；`extract_value` 的名字查表回退按 Plan 风险条目裁定**保留**为防御路径（`arg_index` 恒 `Some` 或 `COUNT(*)` 的 `None`，该臂对列参数不再可达，但删除需同步收紧 `AggregateNode` 字段语义注释，非实质选择）。
- **无既有断言需校准**：本 Cycle 白名单为空集，实际零校准（与 Plan 清扫结论一致）。

**未解决 Critical / Important 问题：无。** 遗留 Minor：0。

### Persisted Evidence

`None required`——验证全部由测试断言与 cargo 退出码直接判定，可低成本重跑，无一次性环境或不可复现结论。

### Experience Candidates

`None`——两项均在 change 范围内收口，无范围外实质缺陷；既有操作流程（Runbook）无新增或需更新面。

### 未解决问题

无。T8 ③ 缺口（JOIN 基座聚合参数未知列静默 NULL）关闭；单表面与别名参数静默面消除；T13 泛化 `SELECT ITEM` 文案终止。

## Plan Review

- Review Result: accepted

**独立核对范围**

完整阅读本 Cycle 全部触及面（不以下列 Act Self-Review 代替）：`query.rs:1804-1847` 统一解析构造 + `resolve_layout_index`（:2515-2541）语义核实 + `aggregate_args`/`aggregates` 收集同步性（:1133-1134/:1155-1156）+ `ast.rs:162-166/:301-303` 两处兜底臂及其余臂（:157/:286）状态 + `extract_value` 防御路径（aggregate.rs:417-435）+ 单一构造点与消费链（grep 全 src：query.rs:1811 → plan.rs:461 → aggregate.rs:382/:396，pipeline.rs:649/:661 透传）+ 5 个新测试函数逐一阅读（`join_output_resolution_test.rs:327/:346`、`planner_test.rs:1921/:1943/:1965`）+ 静态计数独立复核（全量 `--list` 1408、`planner_test` 90、`join_output_resolution_test` 14，本 Review 会话实测）。

**Findings**

1. **[计数差 1408 vs 1409——Plan 口径误差，非阻塞]** Plan 预期 1409 按「T13×2 见证函数」计数；实际 `qualified_wildcard_capability_named` 为 1 函数 2 断言（for 循环单表面 + JOIN 两基座断言，planner_test.rs:1965-1987），5 新函数 = 1403 + 5 = 1408。Act 解释经本 Review 独立计数核实成立；断言覆盖与契约一致（两基座均有断言），无覆盖缺口。分类：Plan 错误（计数口径），不阻塞。
2. **[T12 核对 ✓]** 契约 ①..⑤ 逐项成立：单表面 `Identifier` 小写名 / `CompoundIdentifier` 末段名查 `column_indices`、零命中 `ColumnNotFound`（query.rs:1822-1826/:1838-1842）；JOIN 面 `resolve_layout_index` 零命中 `ColumnNotFound`（裸名 `ident.value` / 限定名末段名，:1815-1820/:1830-1836），歧义 `AmbiguousColumn` 透传（resolve_layout_index :2536-2539）；`None`（COUNT(*)）→ `Ok(None)`（:1814）与非列引用防御臂（:1845）保持；Plan 预判的 `map_err` 死分支已随 `and_then(..ok_or_else)` 改写消除。
3. **[T13 核对 ✓]** ast.rs:166/:303 两处兜底臂均 `"QUALIFIED WILDCARD"`，携带 design D12 注释；:157/:286 的 `UNSUPPORTED SELECT ITEM EXPRESSION` 为另一既有消息（表达式变体臂），未触碰。采用改文案最小实现（Plan 预标注等价非实质二选一）——Act 核实 sqlparser 0.44 `SelectItem` 四变体前三已显式匹配、兜底臂只承接 `QualifiedWildcard`，与本 Review 阅读一致；拒绝语义与覆盖面零变化，`UnsupportedStatement` → 0A000 分类不变。
4. **[Minor——`alias_agg_arg_rejected` 断言强度，非阻塞]** planner_test.rs:1952 `msg.contains('u')` 为单字符匹配、判别力弱（几乎任意消息含 `u`）；核心断言（Err 分支 + `"Column not found"`）已承载验收目标——静默形态经 `Ok(plan) → panic` 分支不可通过，拒绝发生与错误类别均有见证。断言精度属非实质，记录不返工。
5. **[采集/消费一致性 ✓]** `aggregate_args` 与 `aggregates` 在 `UnnamedExpr` 与 `ExprWithAlias` 两分支同步 push（长度恒等）；名字查表回退保留为防御（列参数 `arg_index` 恒 `Some`、COUNT(*) 恒 `None`，该臂对列参数不再可达），与 Plan Risk 条裁定一致。
6. **[验证采信]** Act 全量串行 1408/0/2、定向 14+90、边界 115 零校准、clippy/fmt/validate 干净——本 Review 独立复核静态计数与实现面后按覆盖未变化采信；RED 见证声明与 Plan Investigation Facts 的 P1/P2/P9/P10/P11/P12 实测形态逐一致（GREEN 为真实运行），TDD 见证成立。

**Acceptance Gaps**

无。delta `error-reporting` MODIFIED R6 四场景（JOIN 裸名 / JOIN 限定名 / 别名引用 / 既有零回归）与 MODIFIED R1 新场景（限定通配点名，单表面 + JOIN）均有测试见证；T8 ③ 缺口（JOIN 基座聚合参数未知列静默 NULL）关闭；单表面与别名参数静默面消除；白名单空集零校准、全部已交付面零回归。Iteration 004 六任务（T8 主体/T9/T10/T11 + T12/T13）Acceptance 全部满足。

**Iteration Plan Update**: None——Map 不变，Iteration 004 为最后 Iteration，T1–T13 全部交付。

**Next Cycle**: None。**Next Iteration**: None——change 五个 Iteration 全部 accepted，无剩余 Iteration；change 具备收尾条件（最终 Review Result accepted），待用户指令交 `openspec-docs-maintainer` 收尾（合并 delta specs、归档、刷新 SNAPSHOT/tasks）。

**Issue 候选**：None——Act Experience Candidates 已报 None，本 Review 核对无 change 范围外实质缺陷。
