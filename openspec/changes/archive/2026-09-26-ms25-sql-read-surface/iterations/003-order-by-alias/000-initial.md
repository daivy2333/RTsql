# Iteration 003 / Cycle 000: ORDER BY 输出别名排序与语料校准

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T7
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `order-by-alias-sorting` R1–R3、`error-reporting` REMOVED+ADDED「ORDER BY 排序键解析与别名排序」、`sql-scalar-functions` MODIFIED R5 全部场景；design D8（OrderByColumn 表达式键 + 别名优先遮蔽 + decorate-sort-undecorate）；proposal 决策记录（用户批准 2026-09-26）；change 级 Gate 2（2026-09-26）覆盖本 Iteration 计划；Iteration 000–002 已交付面（JOIN 输出形状、DISTINCT 链位、CTE/EXPLAIN）作为组合场景输入
- Excluded scope: T8/T9（Iteration 004）；SQL 级 `ORDER BY <表达式>` / `ORDER BY t.col` 限定名 / 位置序号解禁（现状 `ParseError`「ORDER BY only supports column names」保持，query.rs:2420-2427）；NULLS FIRST/LAST；排序 NULL 语义变更；复合语句尾 ORDER BY 的别名键（`build_set_operation` 键 ∈ 复合输出列名，query.rs:579-605，别名键 ColumnNotFound 既有语义保持）；子查询内 ORDER BY 别名排序（子查询表达式项路由被 `building_subquery` 抑制，无别名键面）

**Objective**

顶层语句 `ORDER BY <输出别名>` 按输出列真实排序（表达式项别名 / 普通列别名 / DESC / 多键混合）；输出名优先遮蔽语义（键与别名、输入列均可解析时按输出列排序）；未知键 `ColumnNotFound` 与 DISTINCT 约束、聚合查询既有别名排序零变化；`tests/order_by_alias_sort_test.rs` 全绿（别名静默面 RED→GREEN 见证）、`tests/scalar_function_test.rs` R5/S5 用例按白名单校准、白名单外既有测试零校准、全量串行零回归。

**Investigation Facts**

- Current Baseline: Iteration 002 最终 Act Response（accepted Review 采信，2026-09-27）——CTE/EXPLAIN 落地，全量串行 `cargo test -- --list` 计数 1369（2 ignored）/ 0 failed；`planner_test` 87、`cli_test` 111/2i、`scalar_function_test` 32 零校准。本 Iteration 目标文件中 `src/executor/plan.rs`、`src/executor/sort.rs`、`src/pipeline.rs` 自该验证后零变化（git 只读核实——Iteration 002 验证后唯一改动 `plan_format.rs` 仅被 EXPLAIN 路径消费）；`query.rs` 最后改动为 Iteration 002（本 Review 已核对现态，下列行号为现态）。

- Current-State Evidence:
  - **主路径 ORDER BY 装配**（`build_query`，query.rs:1669-1763）：`extract_column_name`（:2420-2427）仅接受 `Expr::Identifier` 裸标识符。`OrderByColumn{column, asc}`（plan.rs:11-14）无表达式字段。`sort_columns` = 聚合查询取 `projection_columns`、否则 `get_plan_output_columns(plan_with_aggregate)`（:1688-1692）；`sort_projection` 三分支（`is_base_scan_chain` → projection_indices / `is_identity_filter_over_join` → where_join_projection / 其余空，:1699-1707；helper :2230/:2311）。
  - **别名键静默面（本能力修复对象）**：`alias_columns` = 顶层 `ExprWithAlias` 别名小写集（:1720-1729）；校验循环 resolvable = `sort_columns ∪ alias_columns`（DISTINCT 收窄到输出列集，:1730-1752）——别名键通过校验但 SortNode 携带后 `SortExecutor::compare_rows` 对 `columns` 中不存在的键 `position()` 得 None 静默跳过 → 保持输入序（:1714-1719 注释明文；语料 `sql-scalar-functions` R5 旧语义；`tests/scalar_function_test.rs:477-493` `order_by_expression_alias_keeps_input_order` 期望 'B','A'）。
  - **SortExecutor**（sort.rs）：`compare_rows`（:74-98）键按名对 `self.columns` 大小写不敏感 position，None → Equal 跳过；`compare_values`（:103-133）NULL 恒排尾、日期族同型时间序、跨类型兜底 Equal。排序在投影裁剪前进行、投影在排序后施加（:48-71，D10「排序键可不在投影内」的既有载体）。pipeline Sort 臂 `SortExecutor::new(input, order_by, columns)` + `with_projection`（pipeline.rs:673-679）。
  - **表达式编译上下文**：`build_expression(&table_name, expr) -> Result<ExpressionRef, PlanError>`（expression.rs:232）——列解析经 `join_column_layout` 覆盖臂或 `self.tables.get(table_name)` 全 schema 形状；顶层投影表达式项同上下文编译（query.rs:1181/:1188；no-FORM 路径 :1945/:1952 传 `""`）。`ExpressionRef = Arc<dyn Expression>`，trait 已含 `Send + Sync + Debug`（predicate.rs:50/:76）——`OrderByColumn` 加性 `expr: Option<ExpressionRef>` 与既有 `#[derive(Debug, Clone)]` 编译兼容，无需 trait/derive 变更。
  - **别名键的路由前提**：表达式项路由到顶层 Projection 的前提是 `alias_reroutable`/表达式项通道（query.rs:1084-1091 起，MS19 T5：纯非聚合 Scan 基座、无通配清单；表达式项 × JOIN 维持拒绝）——别名键表达式编译天然限定在「Sort 输入行 = 编译基座行形状」的场景（Scan 基座全形状行 / DerivedScan 基座注册输出形状）；D2 组合布局（Filter×JOIN）不产生表达式别名键（表达式项 × JOIN 拒绝）、聚合查询别名键由 `sort_columns = projection_columns` 命中走列名键（维持既有真实排序，非静默面）。
  - **输出名优先的两个既有事实**：`SELECT name AS name … ORDER BY name`（别名 = 源列名）与 `SELECT x % 2 AS x … ORDER BY x`（别名遮蔽源列）——现状两者都走「别名键通过校验 → compare_rows None 跳过」输入序；目标语义输出别名优先，键解析须先查输出名（alias 命中即表达式键，不再落列名键）。
  - **链位与组合**：基础链 `base → [Sort] → [Limit] → [SubqueryEval] → [Projection]`（Iteration 001 锚点现态）；DISTINCT × 表达式项时 Projection 下移（`Projection → Distinct → Limit`，:1765-1769 注释）——Sort 恒在 Projection 之下，别名键表达式在未投影输入行求值，与编译索引基座一致。DISTINCT 纯列清单时 Distinct 在 Sort 之上（:1765-1766）。
  - **测试入口**：`tests/order_by_alias_sort_test.rs`（新；形态先例 `tests/select_distinct_test.rs`：TempDir + `Database::open` + `query_rows`/`error_msg` helpers）；白名单校准对象 `tests/scalar_function_test.rs::order_by_expression_alias_keeps_input_order`（:477-493）；零校准面 `tests/cli_test.rs`（111/2i，既有排序面）、`tests/planner_test.rs`（87，计划形状）、`tests/group_by_expr_test.rs`（12，聚合别名排序既有面）。

**Implementation Guidance**

实施顺序（TDD）：① `tests/order_by_alias_sort_test.rs` 先行——`alias_sorts_expression` 现状 RED（输入序 'B','A'）、`alias_sorts_plain_desc`/`mixed_keys`/`alias_shadows_source_column`/`input_column_key_unchanged`（现状 GREEN 行为锁定）/`unknown_key_rejected`（现状 GREEN）/`distinct_alias_sort_combo`；② `OrderByColumn` 加性 `expr: Option<ExpressionRef>`（plan.rs，None = 既有列名键）；③ planner 键解析升级——校验循环（:1730-1752）内对 alias_columns 命中的键，按别名回查 `select.projection` 的 `ExprWithAlias` 项取底层 `Expr` 经 `build_expression(&table_name, expr)` 编译置入 `expr` 字段（输出名优先：alias 命中即表达式键，不再落列名键；alias 未命中但 sort_columns 命中维持列名键零变化）；④ `SortExecutor` 表达式键 decorate-sort-undecorate——逐行先求值全部键表达式（求值错误经 `Result` 传播，不吞入比较闭包），按键序比较（列名键按既有 position + 表达式键按求值值，`compare_values` 既有语义）排序后按既有投影裁剪还原行；⑤ 白名单校准 `scalar_function_test::order_by_expression_alias_keeps_input_order`（期望翻转 'A','B'，测试名保留承载语料连续性——与 delta `sql-scalar-functions` R5 校准段互为镜像）+ 定向套件 + 全量串行。

关键实施细节：

- 别名键求值行形状 = Sort 输入行（投影裁剪前的全形状/注册形状行），与 `build_expression` 编译索引同源（同一 `self.tables`/注册形状）——Scan 基座与 DerivedScan 基座两形态均成立（DerivedScan 经 `register_table` 注册输出形状）。
- 多键混合：`order_by: Vec<OrderByColumn>` 逐键独立判别（列名键与表达式键共存，`mixed_keys` 锁定）。
- 表达式键 NULL 语义零变化：求值结果经既有 `compare_values`（NULL 恒排尾）。
- DESC 仅翻转最终 Ordering（含表达式键），与既有列名键 `asc` 臂同构。
- 白名单外既有测试零修改（含 `cli_test` 排序面、`planner_test` 形状、`group_by_expr_test` 聚合别名排序）。

**Task Contracts**

### T7: ORDER BY 输出别名排序与语料校准（`order-by-alias-sorting` R1–R3 + `error-reporting` REMOVED/ADDED + `sql-scalar-functions` MODIFIED）

- Requirement/Scenario: R1 别名真实排序三场景（alias_sorts_expression RED→GREEN / alias_sorts_plain_desc / mixed_keys）、R2 遮蔽两场景（alias_shadows_source_column / input_column_key_unchanged）、R3 校验与组合三场景（unknown_key_rejected 既有 / distinct_alias_sort_combo / 既有排序套件零校准——scalar_function_test R5/S5 白名单校准除外）；`error-reporting` ADDED「ORDER BY 排序键解析与别名排序」三场景与 R1/R3 同源见证；`sql-scalar-functions` R5 校准段场景（原「静默保持输入序」期望翻转）
- Depends on: Iteration 000（JOIN 输出形状排序键）+ Iteration 001（DISTINCT×别名排序组合场景）
- Targets: `src/parser/planner/query.rs::build_query`（ORDER BY 键解析升级：alias 命中键编译表达式置入 `OrderByColumn.expr`，输出名优先）、`src/executor/plan.rs::OrderByColumn`（加性 `expr: Option<ExpressionRef>`）、`src/executor/sort.rs::SortExecutor`（表达式键 decorate-sort-undecorate）、`tests/order_by_alias_sort_test.rs`（新）+ `tests/scalar_function_test.rs`（R5/S5 一处白名单校准）；`build_expression`（expression.rs:232）只复用不修改
- Current behavior: 别名键计划期通过校验（`sort_columns ∪ alias_columns`）但 SortNode 携带后 `compare_rows` None 跳过——静默保持输入序（语料 `sql-scalar-functions` R5 旧语义锁定，scalar_function_test :480 见证）
- Required behavior: ① 表达式项别名与普通列别名键按输出列真实排序（ASC 默认 / DESC）；② 输出名优先遮蔽：键与别名、输入列均可匹配时按输出列排序（含别名 = 源列名与别名遮蔽源列两形态）；③ 仅输入列可匹配的键维持既有列名键逐字节不变（sort_projection 三分支与 D10 排序后投影原样）；④ 未知键 `ColumnNotFound` 保持（error-reporting ADDED「SHALL NOT 静默跳过」——既有校验循环语义不变）；⑤ DISTINCT × 别名排序组合可达（键 ∈ 输出列集，别名天然满足）；⑥ 聚合查询既有别名排序零变化（sort_columns = projection_columns 命中 → 列名键）；⑦ 白名单：`scalar_function_test` R5/S5 用例期望翻转（'B','A' → 'A','B'），测试名保留
- Preserve: 非别名键全路径逐字节不变（列名键、`compare_values` NULL/日期族/跨类型语义、D10 排序后投影、sort_projection 三分支）；复合语句尾 ORDER BY（query.rs:579-605）与子查询 ORDER BY 零变化；`extract_column_name` 拒绝面（表达式/限定名/位置序号 `ParseError`）不变；plan cache 键语义不变；M17/M19
- Forbidden: 不解禁 SQL 级 `ORDER BY` 表达式/限定名/位置序号；不实现 NULLS FIRST/LAST；不改变排序 NULL 语义；不触碰复合/子查询 ORDER BY 路径；白名单外不修改既有测试断言
- Test witness: `tests/order_by_alias_sort_test.rs` 先于实现运行——`alias_sorts_expression` 现状 RED（输入序），实现后 GREEN；`input_column_key_unchanged`/`unknown_key_rejected` 现状 GREEN（行为锁定）；`scalar_function_test` R5/S5 校准后 GREEN
- GREEN condition: 新测试全绿 + 白名单校准用例绿；`planner_test`(87)/`cli_test`(111)/`scalar_function_test`(32)/`group_by_expr_test`(12) 零校准（R5/S5 白名单除外）通过；全量串行 0 failures
- Verification: ①定向 `cargo test --test order_by_alias_sort_test --test scalar_function_test`（退出码 0）；②受影响边界 `cargo test --test planner_test --test cli_test --test group_by_expr_test`（退出码 0）；③全量 `cargo test -- --test-threads=1`（M19，退出码 0）；④`openspec validate 2026-09-26-ms25-sql-read-surface --strict`（退出码 0）
- Stop when: 表达式键编译与 Sort 输入行形状出现索引基座错位（某基座形状下编译索引 ≠ 输入行索引——返回 Plan）；别名键编译与聚合/D2 组合布局产生交互冲突（返回 Plan）；白名单外既有测试需要校准才能通过（返回 Plan）

**Invariants**

- 全部新逻辑挂在「顶层主路径 ORDER BY 键命中 `alias_columns`」形态之下——非别名键、复合、子查询路径逐字节不变。
- 表达式键求值在排序（投影裁剪前）的输入行上进行，编译索引与求值行形状同源（`build_expression` 与顶层 ProjectionItem 同上下文同基座）。
- 排序比较沿用既有 `compare_values`（NULL 恒排尾、跨类型兜底 Equal）；无新比较语义。
- 输出行形状不变量（I034 纪律）：Sort 投影后输出与表头恒一致；表达式键不改变输出列集。

**Non-goals**

- SQL 级 `ORDER BY <表达式>` / `ORDER BY t.col` / 位置序号；NULLS FIRST/LAST；排序 NULL 语义变更；递归 CTE/窗口函数；T8/T9（Iteration 004）。

**Acceptance**

`order-by-alias-sorting` R1–R3 全部场景可观察满足；`error-reporting` REMOVED+ADDED 与 `sql-scalar-functions` MODIFIED 的 delta 承载语义与实现一致（镜像关系：delta 校准段 ↔ `scalar_function_test` R5/S5 校准，合并语料时核对）；别名静默面修复有 RED→GREEN 见证；白名单外既有测试零校准；全量串行零回归。Scenario→Task→Test 链路：RTM `order-by-alias-sorting` 三行 + `error-reporting` REMOVED+ADDED 一行 + `sql-scalar-functions` MODIFIED 一行（tasks.md）。

**Verification**

- `alias_sorts_expression`（RED→GREEN）/`alias_sorts_plain_desc`/`mixed_keys`/`alias_shadows_source_column`/`input_column_key_unchanged`/`unknown_key_rejected`/`distinct_alias_sort_combo`（tests/order_by_alias_sort_test.rs）。
- 白名单校准：`tests/scalar_function_test.rs::order_by_expression_alias_keeps_input_order` 期望翻转后 GREEN。
- 全量：`cargo test -- --test-threads=1` 退出码 0（串行，M19）。
- 判定均为测试自身断言 + cargo 退出码；无身份型证据工程。

**Gate 2 Readiness**

PASS——无 Missing requirement（RTM 全 Covered）；调查完整（主路径 ORDER BY 装配、别名静默机制、SortExecutor 比较与投影结构、表达式编译上下文与 derive 兼容性、路由前提、输出名优先既有事实、复合/子查询边界、白名单对象全部 file:line 实证，行号为 Iteration 002 后现态）；设计闭合（D8 字段/解析序/求值上下文/语料镜像明确）；任务契约自包含（Act 无需回读外部资料）；验证直接判定（测试断言 + 退出码，无身份型证据工程）；无实质未知项。change 级 Gate 2（2026-09-26 用户批准）覆盖本 Iteration 计划；Iteration 000–002 交付面作为组合场景输入已纳入契约。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果；无一次性环境、无不可复现结论。

**Risks and Notes**

- `OrderByColumn` 加性 `expr: Option<ExpressionRef>` 与既有 `#[derive(Debug, Clone)]` 编译兼容（Expression trait 已含 Debug + Send + Sync，predicate.rs:50/:76）——无需 trait/derive 变更；等价 newtype 包装为非实质选择。
- 输出名解析序的最小面：本引擎「输出名」按 `ExprWithAlias` 别名承载；Display 列名（如聚合 `count_star`）的排序已由聚合查询 `sort_columns = projection_columns` 命中走列名键真实排序（非静默面），无需表达式键通道——delta spec「`ExprWithAlias` 的别名与 Display 列名」中 Display 列名形态由既有列名键承载，实现时按此边界；若 Act 调查发现超出边界的形态，按非实质留档或返回 Plan。
- 白名单唯一性：本 Iteration 唯一允许的既有测试修改是 `scalar_function_test::order_by_expression_alias_keeps_input_order` 期望翻转（delta 语料校准段的镜像见证）——其余任何校准返回 Plan。
- DISTINCT × 别名排序组合依赖 Iteration 001 D3 链位（表达式项 + DISTINCT 时 `Projection → Distinct → Limit`，Sort 恒在其下）——别名键表达式在 Sort 输入行求值与该链位天然一致；`distinct_alias_sort_combo` 锁定。
- 语料修订（`error-reporting` REMOVED+ADDED、`sql-scalar-functions` MODIFIED）已作为 delta specs 随 change 入库，合并由 Maintainer 收尾时执行；本 Cycle 的测试校准与 delta 文字互为镜像，验收时核对一致。

## Act Response

- Status: reported

### 已交付内容（T7）

输出别名键由「计划期可解析、执行期静默跳过（保持输入序）」改为**按输出列真实排序**（R1/R2），校验与组合面零回归（R3）。

| 文件 | 符号 | 改动 |
|---|---|---|
| `src/executor/plan.rs` | `OrderByColumn` | 加性字段 `expr: Option<ExpressionRef>`（`Some` = 输出名解析的表达式键；`None` = 既有列名键）。`ExpressionRef` 已在文件头导入，`#[derive(Debug, Clone)]` 编译兼容，无 trait/derive 变更 |
| `src/parser/planner/query.rs` | `build_query` ORDER BY 装配段（:1759-1787） | 校验循环之后新增表达式键装配：键命中 `select.projection` 的 `ExprWithAlias` 别名（大小写不敏感）时回查底层 `Expr`，经 `build_expression(&table_name, …)` 编译置入 `ob.expr`——输出名优先遮蔽。范围门 `projection_items.is_some()`（纯非聚合表达式项通道） |
| `src/parser/planner/query.rs` | 复合语句尾 ORDER BY（:586）/ 主路径 `OrderByColumn` 构造（:1682） | 补 `expr: None`；MS19 T6 旧注释（「别名键静默保持输入序、SHALL NOT 报错」）更新为 MS25 T7 语义 |
| `src/executor/sort.rs` | `SortExecutor::initialize` / `compare_rows` | decorate-sort-undecorate：含表达式键时逐行在**未投影输入行**上经 owned `Expression::evaluate` 求值键表达式（错误经 `Result` 传播，不吞入比较闭包），按「表达式键先后序」对齐后与列名键混合比较，排序后 undecorate 回输入行，既有投影裁剪与 `compare_values` 语义不变。无表达式键时 `keys = None`，比较路径与变更前逐字节相同 |
| `src/executor/plan_format.rs` | `describe` Sort 臂 / 渲染器测试夹具 | 解构模式补 `..`；夹具字面量补 `expr: None`（EXPLAIN 文本输出零变化） |
| `tests/order_by_alias_sort_test.rs` | 新建 7 场景 | R1×3、R2×2、R3×2 |
| `tests/scalar_function_test.rs` | `order_by_expression_alias_keeps_input_order` | **白名单唯一校准**：期望 'B','A' → 'A','B'，测试名与 doc 注释按 delta 校准段更新（承载语料连续性） |
| `tests/sort_test.rs` | 6 处 `OrderByColumn` 字面量 | 机械补 `expr: None`，**断言零改动** |

实现要点落位与契约一致：别名键求值行 = Sort 输入行（顶层 Projection 恒在 Sort 之上，Sort 之下 `projection_indices = None` 使 `sort_projection` 为恒等），编译基座与顶层 `ProjectionItem` 同为 `build_expression(&table_name, expr)`，两侧索引同源。`compare_values`（NULL 恒排尾、跨类型兜底 Equal）、DESC 翻转、并列序不稳定语义（`sort_unstable_by`，与既有列名键一致）均未改动。

### 测试见证（RED → GREEN）

新建 `tests/order_by_alias_sort_test.rs` 先行运行取得 RED：**2 passed / 5 failed**——`alias_sorts_expression` 返回输入序 ['B','A']、`alias_sorts_plain_desc` ['a','b']、`mixed_keys` / `alias_shadows_source_column` / `distinct_alias_sort_combo` 同样保持输入序（别名键被 `compare_rows` 的 `position()` None 跳过臂丢弃）。`input_column_key_unchanged`（列名键真实排序）与 `unknown_key_rejected`（`ColumnNotFound`）现状 GREEN，作为既有行为锁定见证。实现后同文件 **7 passed / 0 failed**。

见证有效性修正（实施中）：首版 `mixed_keys`（组内输入序恰好已排序）与 `alias_shadows_source_column`（`x * 2` 单调整数保序）**偶然通过**，无法区分「按别名表达式排序」与「按同名基列排序」。已改为 delta 场景形态且非单调的见证：`mixed_keys` 用 delta R1 的 `(2,1),(1,2),(1,1)` + 代理主键列（本引擎无显式 PK 时首列即键，`a` 重复触发 Duplicate key）；`alias_shadows_source_column` 改用 `SELECT 10 - x AS x`（按输出列 → `[-10,5,7]`，按基列 → `[7,5,-10]`，两序可区分）。修正后两条均为真实 RED。

### 与计划的偏差

1. **契约 Implementation Guidance 的 RED/现状标注偏保守**（非实质）：计划括注 `alias_sorts_plain_desc` / `mixed_keys` / `alias_shadows_source_column` 为「现状 GREEN 行为锁定」，实测三条现状均 RED（别名键在所有非聚合表达式项通道下静默跳过）。实施按契约 Required behavior ①②执行，见证强度高于计划预期，无范围影响。
2. **既有测试字面量的机械补字段**（非实质）：契约「白名单外不修改既有测试断言」——`tests/sort_test.rs` 6 处与 `plan_format.rs` 测试夹具 1 处 `OrderByColumn { … }` 因加性字段必须补 `expr: None`，**断言与期望值零改动**，不属校准。
3. **求解实现取「按别名回查 `Expr` + `build_expression` 编译」**（契约明列路径），未复用 `projection_items` 中已编译的 `ProjectionItem.expr`：两者是同一函数同一参数的同值结果，复用会与契约文本不符且需按名字匹配 `UnnamedExpr` 项（其 `name` 是表达式文本），回查 `ExprWithAlias` 的解析边界更窄、无歧义。

### 自检发现与修复

- **Critical（已修复，Cycle 内）**：`initialize` 的 decorate 元组顺序错误——`rows.into_iter().zip(keys)` 产出 `(row, keys)`，而比较闭包按 `(keys, row)` 解构，导致**键值被当作行下推**、undecorate 取回键值。症状：`alias_shadows_source_column` 返回 `[3,5,20]`（`10 - x` 被二次求值还原为 `x`）、`mixed_keys` 报 `Column index 1 out of bounds (row has 1 columns)`。修复：显式 `.map(|(row, row_keys)| (row_keys, row))`。修复后两场景 GREEN，全量零回归。
- **Important（已修复）**：clippy `redundant_pattern_matching`（`if let Some(_)` → `is_some()`）。
- 复查项（无问题）：表达式键与列名键混合下标对齐（独立计数器 `key_idx`，列名键不占位）、求值错误不落入比较闭包、`evaluate_ref` 未被使用（String 结果会报错，同 ProjectionExecutor 约束）、`projection_items` 在本装配点仍为 `Some`（`.take()` 只发生在其后的 DISTINCT 分支）。

### 遗留 Minor

1. **普通列别名在 DerivedScan 基座（CTE / FROM 子查询）不应用别名**（MS25 前既有，非本 Cycle 引入）：实测 `WITH c AS (SELECT v FROM t) SELECT v AS val FROM c ORDER BY val` 表头为 `v` 而非 `val`、序为输入序；根因是 MS19 T4b 的别名重路由门 `matches!(&base_plan, PhysicalPlan::Scan(_))` 只覆盖 Scan 基座，DerivedScan 基座的纯列别名项停在 `Column` 角色——别名本身未应用，故 ORDER BY 无可解析的输出列。表达式项别名在 DerivedScan 基座正常（实测 `SELECT upper(v) AS val FROM c ORDER BY val` → `['A','B']`）。本 Cycle 契约范围（design D8）为 Scan 基座表达式项通道，未扩展；作为 Issue 候选报告（见下）。
2. delta `order-by-alias-sorting` R2「别名遮蔽源列」场景示例写作 `SELECT x % 2 AS x FROM t ORDER BY x`——本引擎值表达式算术只支持 `+ - * /`（`%` 在计划期按 `PlanError::UnsupportedExpression("ARITHMETIC OPERATOR %")` 点名拒绝，`expression.rs` BinaryOp 臂 :411-428），该示例不可执行。要求文字本身不含运算符，实现语义已满足并由 `10 - x` 见证；delta 措辞校正需 Plan 裁定（Act 不改 delta specs）。同样仅见于示例，`error-reporting` ADDED 与 `sql-scalar-functions` R5 校准段的示例均为 `upper()`/`+ 1`，均可执行。

### 验证

1. 定向（RED 见证首跑）：`cargo test --test order_by_alias_sort_test` → `2 passed; 5 failed`（`alias_sorts_expression` 实测 `[["B"],["A"]]` vs 期望 `[["A"],["B"]]`）。退出码 101（预期 RED）。
2. 定向（GREEN）：`cargo test --test order_by_alias_sort_test` → `test result: ok. 7 passed; 0 failed`，退出码 0。
3. 白名单校准：`cargo test --test order_by_alias_sort_test --test scalar_function_test` → `7 passed` + `32 passed; 0 failed`（含 `order_by_expression_alias_keeps_input_order` 校准后 GREEN，与基线 32 零校准一致），退出码 0。
4. 受影响边界：`cargo test --test planner_test --test cli_test --test group_by_expr_test --test sort_test --test select_distinct_test --test set_operations_test --test explain_test` → 7 套件全 `ok. 0 failed`，退出码 0。
5. 静态：`cargo clippy --all-targets` → 0 warning（退出码 0）；`cargo fmt --check` → 无 diff（退出码 0）。
6. 全量（M19 串行）：`cargo test --no-fail-fast -- --test-threads=1` → 90 个测试目标，**1373 passed / 1 failed / 2 ignored**（总数 1376 = Iteration 002 基线 1369 + 本 Cycle 新增 7）。唯一失败 `cli_test::test_sigint_during_open_130`（`out.code` 为 `None`，子进程在打开阶段被信号终止）：本改动不触及 `src/cli/mod.rs` 信号路径，查询 `SELECT COUNT(*) FROM t` 无 ORDER BY 无别名，计划面零变化。**首跑失败，按「失败后判定是否偶发错误的一次复核」执行**：`cargo test --test cli_test -- --test-threads=1 test_sigint_during_open_130` → `test result: ok. 1 passed; 0 failed`，退出码 0 —— 按通过处理（该用例在第 4 项受影响边界批次中亦为 `111 passed; 0 failed`）。复核未重复全量。
7. OpenSpec：`openspec validate 2026-09-26-ms25-sql-read-surface --strict` → `Change '2026-09-26-ms25-sql-read-surface' is valid`，退出码 0。
8. change 结构自检：`iterations/003-order-by-alias/000-initial.md` 为 003 唯一 Cycle、Plan Context 完整未改写、Act Response 首次填写；change `tasks.md` 无 per-task 状态字段（T7 状态以本 Response 为准），Iteration 状态与后继 Iteration（004）由 Plan Review 更新。

采集时现场：HEAD `bc1d18d`（MS24 收尾文档），MS25 Iterations 000–002 代码与本次改动同处未提交工作树（index 已暂存既有部分，本次改动为工作树增量）。

### Persisted Evidence

None required——验证结论均为测试自身断言 + cargo 退出码直接判定，可低成本重跑；无一次性环境、无不可复现结论。`Persisted Evidence: Mode: none` 保持。

### Experience Candidates

1. **Issue 候选（change 范围外，MS19 T4b 既有）**：普通列别名在 DerivedScan 基座（CTE / FROM 子查询）不应用别名——`WITH c AS (SELECT v FROM t) SELECT v AS val FROM c ORDER BY val` 表头返回 `v`、别名丢失（静默错误结果同类）。证据：本 Cycle CLI 实测（`src/parser/planner/query.rs:1084-1090` 的 `alias_reroutable` 门 `matches!(&base_plan, PhysicalPlan::Scan(_))`）。诊断面：别名重路由门 + `build_from_clause_with_projection` 派生基座注册形状。
2. **Runbook/Issue 候选（测试基建）**：`tests/cli_test.rs::test_sigint_during_open_130` 存在偶发失败——全量串行跑时 `out.code` 为 `None`，隔离复核通过（证据见验证第 6 项；同批次受影响边界运行中为 `111 passed; 0 failed`）。属信号投递时序竞态，非产品缺陷；Recorder 按 Issue 准入（实质缺陷）自行判定是否登记，或作为 R27 诊断 Runbook 的已知偶发项记录。

### 未解决 / 交由 Plan Review

1. delta `order-by-alias-sorting` R2 场景示例的 `%` 运算符措辞不可执行（见遗留 Minor 2）——delta 校正超出 Act 职责，请 Plan 裁定。
2. Iteration 003 的 Iteration 状态、后续 Cycle（`rework-required` 时）与 Iteration 004（T8/T9）的展开，均由 Plan Review 依 Map 处理；Act 未创建任何后继 Cycle。
3. `test_sigint_during_open_130` 偶发性未经第二次独立全量确认（复核仅覆盖该用例自身）；如 Review 要求，可在下个 Cycle 的全量验证中一并观察。

## Plan Review

- Review Result: accepted

**Findings**

独立核对范围：完整 diff（src/executor/{plan,sort,plan_format}.rs、src/parser/planner/query.rs、tests/{order_by_alias_sort,scalar_function,sort}_test.rs）逐文件阅读 + delta specs 三份核对 + 复现实验四组。偏差分类与结论：

1. **[Plan 错误——已由本 Review 修正]** delta `order-by-alias-sorting` R2「别名遮蔽源列」场景示例 `SELECT x % 2 AS x` 引用本引擎不支持的 `%` 算子（值表达式算术仅 `+ - * /`，Act 遗留 Minor 2 报告属实——计划起草时未核对算子支持面）。**已裁定并修正**：示例改为可执行的非单调形态 `SELECT 10 - x AS x FROM t ORDER BY x`（GIVEN (5),(3),(20)、THEN 输出值 -10,5,7 与源列序 7,5,-10 两序可区分），与见证测试 `alias_shadows_source_column` 互为镜像；修正注记已写入 delta 场景（specs/order-by-alias-sorting/spec.md，2026-09-28）。要求文字本身不含运算符，无需其他 delta 改动。
2. **[Act 偏离（记录性）——非实质]** Act Response「`tests/sort_test.rs` 6 处 `OrderByColumn` 字面量」实为 **7 处**（单列 asc/desc、多列 2 处、null_at_end、empty_input、with_float）——「断言与期望值零改动、机械补 `expr: None`」的实质声明经 diff 核对成立，计数笔误不构成偏差。
3. **[Plan 遗漏——非实质]** 契约 Implementation Guidance 将 `alias_sorts_plain_desc`/`mixed_keys`/`alias_shadows_source_column` 标注为「现状 GREEN 行为锁定」，实测三条现状均 RED（别名键在所有非聚合表达式项通道下静默跳过）——Act 已报告，见证强度高于计划预期，实施按 Required behavior 执行无范围影响。接受。
4. **[新证据——非阻塞]** 工作区存在非本 Cycle 契约面的纯 rustfmt 行重排（`src/executor/delete.rs`、`src/storage/data/table_manager.rs`、`src/wal/recovery.rs`、`tests/constraint_enforcement_test.rs`）：逐 token 核对零语义变化，属 `cargo fmt` 全局重排副产物（使 `cargo fmt --check` 通过的必要重排），不属校准、不属范围违规，不要求还原。
5. **[新证据——移交 Iteration 004]** 本 Review 调查确认 T7 装配门（`projection_items.is_some()`，query.rs:1772-1787）将随 Iteration 004 T8 的 `alias_reroutable` 放宽在 JOIN 基座自动生效，而 `build_expression` 对 JOIN 基座回退单表 schema 形状（join_column_layout 已 restore）——若不收口将产生新的静默错序面。已作为 T8 契约 Required behavior ④ 与 stop condition 写入 Iteration 004 Plan Context（含基线实测：JOIN+纯列别名现状零宽行 `{"columns":[],"rows":[[],[]]}`）。非本 Cycle 缺陷，T7 在其声明域（Scan 基座）内语义正确。
6. **[采信成立]** `test_sigint_during_open_130` 信号时序偶发：Act 按「失败后判定是否偶发错误的一次复核」单测复核通过，同批次受影响边界批次 `cli_test` 111/0 failed——按通过处理成立；不要求第二次独立全量。偶发观察移交 Iteration 004 全量验证（已写入其 Risks and Notes）。

Act 报告的 Issue 候选（不落账，按职责由 Recorder 处理）：① DerivedScan 基座普通列别名不应用（本 Review 已用 CLI 复现确认：`WITH c AS (SELECT v FROM t1) SELECT v AS val FROM c ORDER BY val` → 表头 `["v"]`、输入序——与 Act 观察一致）；② `test_sigint_during_open_130` 测试基建时序竞态。

**Acceptance Gaps**

None——`order-by-alias-sorting` R1–R3、`error-reporting` REMOVED+ADDED、`sql-scalar-functions` MODIFIED R5 全部场景满足：

- 代码与声明一致：`OrderByColumn.expr` 加性字段（None = 既有列名键路径逐字节不变）；装配段输出名优先（别名命中即编译表达式、未命中维持列名键）、范围门在 DISTINCT `.take()` 之前；decorate-sort-undecorate 求值在未投影输入行、求值错误经 `Result` 传播不落入比较闭包、Critical 自检修复（`.map(|(row, row_keys)| (row_keys, row))`，sort.rs:99）在位；无表达式键路径与变更前逐字节相同。
- 见证有效：`mixed_keys`（代理 PK + 组内非平凡序）与 `alias_shadows_source_column`（`10 - x` 两序可区分）经代码核对为真实可区分见证，排除「按同名基列排序」的偶然通过形态。
- 语料镜像：`sql-scalar-functions` R5 校准段 ↔ `scalar_function_test::order_by_expression_alias_keeps_input_order` 期望翻转（测试名保留）一致；`error-reporting` ADDED 三场景与 R1/R3 同源。
- 零回归：全量串行 1373 passed / 1 偶发复核通过 / 2 ignored（总数 1376 = Iteration 002 基线 1369 + 7）；白名单外既有测试零校准（除 sort_test/plan_format 机械补字段——非断言改动）。

**Evidence**

- 独立 diff 阅读：`git diff -- src/executor/{plan,plan_format,sort}.rs src/parser/planner/query.rs tests/{scalar_function,sort}_test.rs`（未暂存工作树增量，与 Act 采集时现场一致）+ `tests/order_by_alias_sort_test.rs` 全文 + `src/executor/sort.rs` 现态。
- delta specs：`specs/{order-by-alias-sorting,error-reporting,sql-scalar-functions}/spec.md` 三份全文核对；示例可执行性逐一验证（唯一不可执行项已修正）。
- 复现实验（2026-09-28，本 Review，target/debug/rtsql + /tmp 夹具）：JOIN 基座纯列别名 `{"columns":[],"rows":[[],[]]}`（finding 5 基线）、DerivedScan 基座别名丢失（Issue 候选①复现）。
- Act Response 验证第 1–8 项在覆盖未变化前提下直接采信（工作区与采集时一致，只读 git 核实）；未重复运行任何已采信验证。

**Follow-up Decision**

`accepted`：既有 Acceptance 全部满足，仅存非阻塞 Minor findings（finding 1 已由 Plan 修正完毕，finding 2–4 记录接受，finding 5–6 移交后继 Iteration/Recorder）。无当前 Cycle 修复项、无 rework/replan 需求。Iteration 003 完成。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-09-26-ms25-sql-read-surface/iterations/004-join-shape-resolution/000-initial.md`（已按 Map 展开，Plan Context Status: ready，Gate 2 PASS——T8/T9 执行契约、本 Review 移交的 JOIN 基座交互约束与校准白名单均已自包含写入）
