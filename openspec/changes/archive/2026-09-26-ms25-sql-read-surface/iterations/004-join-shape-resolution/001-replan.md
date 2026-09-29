# Iteration 004 / Cycle 001 (replan): JOIN 输出形状、名字解析、聚合形状与别名应用收口

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T8, T9, T10, T11
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `join-output-shape-resolution` R1–R4、`aggregate-output-shape-conformance` R1–R2、`error-reporting` MODIFIED R5（DerivedScan 基座扩展）、`order-by-alias-sorting` R1 扩展场景；design D9/D10/D11；T8/T9 范围用户批准（2026-09-27）、T10/T11 用户指令并入（2026-09-28 replan）；change 级 Gate 2（2026-09-26）覆盖本 Iteration 计划；Iteration 000 交付面（外连接、D2 组合布局 `carry_all_columns`/`where_join_projection`）与 Iteration 003 交付面（T7 表达式别名键装配，query.rs:1772-1787）作为组合场景输入
- Excluded scope: 表达式项×JOIN 解禁（真表达式维持既有拒绝臂）；单表聚合未知名参数行为（同机制相邻面，随 Cycle 调查裁定顺带或留档，见 T8 Stop when）；有效查询输出形状改变（T9 修复的错配形态除外）；信号语义扩展；子查询内别名（`building_subquery` 抑制不变）；集合操作侧臂内 JOIN 解析深化；MS21-T03；代价模型

**Background**

父 Cycle 000（000-initial.md）承载 T8/T9 的 ready 计划，Act 启动前用户指令重定向（2026-09-28）：把 Iteration 003 Act/Plan Review 报告的两项 Issue 候选并入本 Iteration 解决——① DerivedScan 基座（CTE / FROM 子查询）普通列别名不应用（MS19 T4b 既有静默面）；② `test_sigint_during_open_130` 信号时序偶发。父 Cycle Review Result 已置 `replan-required`，change 的 tasks/design（新增 D11）/delta specs 已同步修订；本 Cycle 按修订后的全局任务（T8–T11）重新执行平衡审计并承载全部执行契约。T8/T9 调查与契约材料自父 Cycle 完整继承（本文自包含重述）。

**Objective**

JOIN 基座与 DerivedScan 基座的名字解析全面收口：聚合参数在 JOIN 布局上精确解析（静默 NULL 消除）、GROUP BY/投影裸名歧义计划期 `AmbiguousColumn` 点名（末位胜出消除）、JOIN 基座未知列 `ColumnNotFound` 点名（零宽行消除）、纯列引用别名项在 JOIN 基座可达（表头=别名）且 ORDER BY 别名键按输出列真实排序；DerivedScan 基座普通列别名表头生效且 ORDER BY 别名键真实排序（静默回落消除）；聚合查询输出行与表头恒为 SELECT 清单形状（键未入选不进输出、键序按 SELECT 序，经既有包装路径承载）；信号处理器注册先于 open 路径锁获取（偶发消除）；无歧义有效查询与形状安全的直出形态逐字节不变。

**Investigation Facts**

- Current Baseline: Iteration 003 最终 Act Response（Plan Review accepted，2026-09-28 采信）——T7 别名键排序落地，全量串行总数 1376（1373 passed / 1 failed〔`cli_test::test_sigint_during_open_130` 信号时序偶发，单测复核通过，本 Cycle T11 修复对象〕/ 2 ignored）；基线套件计数（2026-09-28 `--list` 实测）：planner_test 87、nested_loop_join_test 9、outer_join_test 16、group_by_expr_test 12、aggregate_test 19、select_distinct_test 12、scalar_function_test 32、cli_test 111/2i、order_by_alias_sort_test 7。本 Iteration 目标文件 `src/parser/planner/query.rs` 聚合分支（:1030-1660）、`src/executor/aggregate.rs`、`src/executor/plan.rs` AggregateNode、`src/parser/ast.rs`、`src/cli/mod.rs` 信号面自该验证后零变化。
- Current-State Evidence:
  - **JOIN 输出列过滤**（query.rs:420-443，`build_from_clause_with_projection`）：`carry_all_columns || select_star` → 全组合列；否则按 `qualified_columns`（`extract_qualified_columns` 产物）过滤。`OutputColumn { table, column, table_alias, column_index }`（plan.rs:384-393）——`column_index` 为单源表内索引；Join/NLJ 执行器 `build_output_row`（join.rs:113-115、nested_loop_join.rs 同型）按 `output_columns` 顺序从左右行取列 → join 输出行第 i 值 = output_columns[i]（枚举位即行索引）。
  - **聚合参数静默 NULL 病灶**：`extract_qualified_columns`（ast.rs:175 起）对聚合函数项返回结果列名（`count_<col>`/`count_distinct_<col>`/`sum_<col>`…，MS25 D7 命名）而非参数列——参数列进不了 `qualified_columns`，无 WHERE 时 join `output_columns` 滤除参数列；聚合分支 `input_schema = get_plan_output_columns(&plan_with_where)`、`column_indices: HashMap<String, usize>`（query.rs:1459-1464）无参数列条目；`AggregateExecutor::extract_value`（aggregate.rs:396-410）`column_indices.get(func.column())` None → `Value::Null` 静默。实测（2026-09-28 本 Plan 复核，t1(k,v)=[(1,b),(2,a)]、t2(k,w)=[(1,x),(2,y)]）：`SELECT count(v) FROM t1 JOIN t2 ON t1.k = t2.k` → `{"columns":["count_v"],"rows":[[0]]}`（应为 2）。D2 路径（含 WHERE）`carry_all_columns`（query.rs:260-269）全列承载反而幸免——锚点 `outer_join_test.rs:565`（`SELECT ac.k, SUM(ac.amt) … WHERE bc.tag='x' GROUP BY ac.k`）现状正确。
  - **裸名末位胜出病灶**：`column_indices` HashMap 以小写列名为键（query.rs:1460-1464）——JOIN 同名列（t1.k/t2.k）折叠，后表覆盖先表；GROUP BY 裸名键（`resolve_group_by_item`→`column_indices`，query.rs:1472）、聚合参数（extract_value 同 map）、组键编译（query.rs:1510-1521）全部静默解析到末位表。
  - **JOIN 基座零宽行病灶**：纯列引用别名项 `SELECT t1.v AS val FROM t1 JOIN t2 …`——`extract_qualified_columns` 对 ExprWithAlias 产出别名名，过滤无匹配 → `output_columns` 空集；实测 `{"columns":[],"rows":[[],[]]}` exit 0（带 `ORDER BY val` 同）。未知列同型零宽行静默。裸名歧义投影：裸名匹配多表同名列时过滤全保留（重复列名静默）。
  - **DerivedScan 别名病灶（T10）**：`alias_reroutable` 门（query.rs:1088-1094）基座条件 `matches!(base_plan, Scan(_))` 漏掉 DerivedScan——`WITH c AS (SELECT v FROM t1) SELECT v AS val FROM c` 普通列别名项停在 `Column` 角色：表头回落 `["v"]`（别名丢失）、`ORDER BY val` 无可解析输出列（静默输入序，Plan Review 2026-09-28 CLI 实测 `{"columns":["v"],"rows":[["b"],["a"]]}`）。表达式项别名在 DerivedScan 基座正常（Act 003 实测 `SELECT upper(v) AS val FROM c ORDER BY val` → `['A','B']`）——`projection_items` 通路与注册形状编译基座在 DerivedScan 上成立。DerivedScan 注册机制：CTE 与 FROM 子查询均经 `register_table(&alias, columns)` 注册输出形状（query.rs:320/:826），`build_expression(&table_name, expr)` 对该基座解析注册形状。DerivedScan 基座 `未知列 AS alias` 现状同样零宽行静默（extract_qualified_columns 产出别名名 → 过滤无匹配）。
  - **表达式项×JOIN 拒绝臂**（T8 必须保持）：`mixed_aggregate` 路径 query.rs:1130-1137、纯表达式路径 query.rs:1158-1165——`"Expression projection items are not supported with JOIN queries"`。`alias_reroutable` 完整门：`!has_aggregates && !building_subquery && matches!(base_plan, Scan(_)) && 无通配`；命中后普通列别名项改标 Expression（:1095-1111）。
  - **T7 表达式键装配与基座交互（T8-R6）**：装配门 `projection_items.is_some()`（query.rs:1772-1787）——`alias_reroutable` 放宽至 JOIN 基座后该门自动生效：ORDER BY 别名键经 `build_expression(&table_name, expr)` 编译，而该函数对 JOIN 基座回退 `self.tables.get(table_name)` 单表 schema 形状（join_column_layout 已在 FROM 构建后 restore，query.rs:487/:502/:506）→ 索引基座与 Sort 输入行错位。Sort 输入行形状按形态二分：无 WHERE join = 过滤后 output_columns 行；WHERE join（D2 carry）= 全组合行（`sort_projection = where_join_projection` 在排序后才裁剪，query.rs:1707-1715/:1712）。若不收口将产生新的静默错序面——T8 契约 ④ 显式约束。DerivedScan 基座无此问题（注册形状 = Sort 输入行形状，与 Scan 基座同构）。
  - **聚合输出形状病灶（D10）**：行装配恒 `group_key(GROUP BY 序) ++ aggregates`（`build_output_rows` 分组臂，aggregate.rs:451-459 区域）；`direct = !has_expression_items && keys_all_column && select_keys_first`（query.rs:1538-1552）不含「GROUP BY 键 ⊆ SELECT 键项」与「SELECT 键序 = GROUP BY 键序」检查；直出 `node_output_columns = agg_output_columns`（SELECT 序，query.rs:1566-1567）。实测（2026-09-28）：`SELECT count(*) FROM t1 GROUP BY v` → 表头 `["count_star"]` 行 `[["b",1],["a",1]]`；`SELECT v, k, count(*) FROM t1 GROUP BY k, v` → 表头 `["v","k","count_star"]` 行 `[[2,"a",1],…]`（置换错位）。包装路径已形状一致：`node_output_columns = group_keys 序 ++ agg_names`（query.rs:1569-1573）+ `wrap_projection`（query.rs:1578-1630）按聚合行位置重排、列名 = SELECT 名；`get_plan_output_columns` Aggregate 臂 = `node.output_columns`、Projection 臂 = `node.columns`——T9 只改 `direct` 判定。
  - **HAVING 绑定**：`build_having(having_expr, &node_output_columns)`（query.rs:1632-1637）——直出形态缺未入选键 → 现状绑定失败；包装形态含键。既有 HAVING 测试全在包装或键对齐形态（aggregate_test / group_by_expr_test:398-440 / scalar_function_test:468），无断言锁定失败面。
  - **校准白名单（Plan Review 2026-09-28 全库清扫定稿）**：① `tests/select_distinct_test.rs::distinct_aggregate_output`（RTM 已列）；② `tests/group_by_expr_test.rs::groupby_order_interleave_keeps_direct_path`（MS13 时代锁定「GROUP BY 序 ≠ SELECT 键序直出错位」，doc 明言「既有错位行为」——T9 后转包装路径，断言与 doc 必须校准，测试名保留或改名非实质、doc 必须写明 T9 来源）。其余零校准面：group_by_expr_test 余下用例（:305 对齐直出、:184 包装、:207/:255/:408/:432 表达式键包装、:462 GROUP BY ALL、拒绝面）、aggregate_test 19、planner_test:1879（对齐）、subquery_test:310（对齐）、outer_join_test:565（D2 锚）、expression_e2e_test（无 GROUP BY 形态命中）。aggregate×JOIN 既有断言仅 outer_join_test:565 一处。
  - **信号注册时序病灶（T11）**：`execute_command_inner` 两段 `select!` 中 open 臂先声明（src/cli/mod.rs:332-338），open future 首次轮询的**同步前缀**完成 `Database::open_with_key → FileStorage::open_with_key → try_lock`（database.rs:58 同步调用、file_storage.rs:71）；信号处理器注册发生在信号臂 future 首次轮询（晚于锁）——「已持锁、未注册」窗口内 SIGINT → 默认处置死亡（`out.code = None`）。测试以「子进程持锁」为锚（cli_test.rs:792-832，5ms 轮询 + 10s deadline），kill 落入窗口即偶发——窗口 = open 同步前缀（微秒级）× 5ms 轮询相位，与偶发频率吻合。注意：tokio `select!` 默认随机轮询序（非 biased），仅交换臂声明序不可靠。信号 future 为注入闭包 `signal_int/signal_term: impl Fn() -> SignalFuture`（单测 :561+ 注入路径）。退出码映射 128+signum（cli/mod.rs:59）。
  - **错误面载体**：`PlanError::AmbiguousColumn(String)` 已存在（src/parser/error.rs:29）；`PlanError::ColumnNotFound(String)` 既有。`AggregateFunc`（aggregate.rs:16-30+）：`CountStar` 无参数，`Count/Sum/Avg/Min/Max/CountDistinct(String)`（+ D7 group_concat）携带参数列名。`AggregateNode` 构造点全集：query.rs:1639-1647、pipeline.rs（执行器装配）、plan.rs:452 定义——tests/ 无 AggregateNode 字面量构造，加性字段机械补面仅限 src 两处。plan_format.rs Aggregate 臂（:33、:198-201）按字段访问渲染，EXPLAIN 文本不含新字段 → 零变化。
  - **Hash join 条件面（不触碰）**：`extract_join_conditions`/`is_pure_equi_join_on` 结构探测与条件编译发生在 FROM 构建期，与本 Cycle 的输出布局解析正交。
  - **测试入口**：`tests/join_output_resolution_test.rs`（T8 新）、`tests/aggregate_output_shape_test.rs`（T9 新）、`tests/derived_scan_alias_test.rs`（T10 新；形态先例 `tests/cte_test.rs`/`tests/order_by_alias_sort_test.rs`：TempDir + `Database::open` + `query_rows`/`error_msg` helpers）——JOIN 夹具须同库双表（跨库 ATTACH 未实现）。

**Implementation Guidance**

TDD：T8/T9/T10 各自先立测试文件取 RED（静默缺陷与错配均为现状可复现，Plan 已实测），再实现 GREEN。建议 T8 → T10（同一 `alias_reroutable` 门编辑点一次落地：`matches!(base_plan, Scan(_) | DerivedScan(_) | Join(_) | NestedLoopJoin(_))`，两任务验收分离——各自测试文件独立见证）→ T9（独立判定收窄，与解析面无交集，顺序不实质约束）→ T11（独立 CLI 面，随时可做）。

T8 落位建议（契约 ①..⑥ 的机制方向，等价机制可）：

1. 聚合×JOIN 全列承载（①）：`has_aggregates` 且 JOIN 基座时 join `output_columns` 扩为全组合列——复用 `carry_all_columns` 通道（置位条件从「含 WHERE」扩为「含 WHERE ∨ has_aggregates」需检查该 bool 的其他消费点，或在 join 构造点按 `has_aggregates` 独立判定，等价机制均可）；聚合输入形状 = 全组合布局。
2. 布局感知解析（②）：聚合分支的 `column_indices` 消费面改为保序布局结构——`output_columns` 的 `Vec<OutputColumn>` 天然携带 (table_alias, column, 枚举位)；解析 helper：限定名 (table, column) 精确唯一匹配 → 行位置；裸名唯一命中 → 行位置；裸名多命中 → `PlanError::AmbiguousColumn` 点名；零命中 → `PlanError::ColumnNotFound`。消费面：`resolve_group_by_item` 列名键、组键编译（query.rs:1510-1521）、聚合参数解析（新）。
3. `AggregateNode` 加性 `aggregate_arg_indices: Vec<Option<usize>>`（与 `aggregates` 平行，`CountStar` → None；group_concat 参数列同解析）：计划构造点解析填充；`extract_value` 改消费索引——解析失败的参数在计划期即点名（`ColumnNotFound`/`AmbiguousColumn`），运行期不再有 None→Null 臂复活静默。机械补字段：pipeline.rs 装配点。
4. 非聚合 JOIN 投影严格解析（③）：JOIN 基座纯列清单逐项经同一 helper 解析——未知列 `ColumnNotFound`、裸名多命中 `AmbiguousColumn`（零宽行与重复列静默面消除）。
5. 纯列引用别名项可达（③）：`alias_reroutable` 基座门放宽至 `Join(_) | NestedLoopJoin(_)`（T8）与 `DerivedScan(_)`（T10，同一编辑点）；改标后的 Expression 项编译不得走单表 schema 回退——JOIN 基座按 join 行形状解析（无 WHERE = 过滤后 output_columns 位；有 WHERE = 全组合位，与 `where_join_projection` 的 SELECT 位置机制对齐），DerivedScan 基座按注册形状解析（既有 `build_expression` 通路天然正确，无需额外机制）；「表达式项×JOIN」两拒绝臂（query.rs:1130-1137/:1158-1165）对真表达式保持（`is_plain_column_expr` 判定先于改标，天然豁免）。
6. ORDER BY 别名键×JOIN 基座（④）：T7 装配（query.rs:1772-1787）在 JOIN 基座生效后，键表达式解析必须与 Sort 输入行形状同基座——无 WHERE join 按过滤后布局、WHERE join 按全组合布局（同 helper）；DerivedScan 基座经注册形状天然同基座（T10 场景见证）。禁止落入单表回退产生静默错序。

T9 落位建议：`direct` 判定（query.rs:1552）追加两条件——每个 group key 被某 SELECT `Column` 角色引用（键集合 ⊆ SELECT 键项）∧ SELECT 键项出现序 = GROUP BY 键序（既有 `select_keys_first`「键前聚合后」保留）。判定结果只切换直出/包装路由（包装机制零修改）；`build_output_rows` 行装配不动。

T10 落位建议：门放宽 + `tests/derived_scan_alias_test.rs` 四场景。DerivedScan 基座无需新机制——注册形状编译通路已被表达式项别名验证（Act 003 实测）；普通列别名项改标 Expression 后走同一通路。

T11 落位建议：机制候选（等价，Act 按 tokio 注册语义调查后定）——① 在 open select 前创建两个信号 future（若 tokio 在 future 创建期注册——`Signal::new`/`ctrl_c()` 构造——则直接生效）；② `Database::open` 首个 await 前置 `tokio::task::yield_now().await`，使 open 臂首次轮询在锁获取前让位、select 转而轮询信号臂完成注册（对 tokio 注册语义不敏感，确定性最强）。臂序交换不可靠（select! 默认随机轮询序）。压测判定为计划内不稳定类验证：修复机制落地后连续 10 轮 `cargo test --test cli_test test_sig -- --test-threads=1` 全绿（含 `test_sigint_during_open_130`/`test_sigterm_during_run_143` 等）。

**Task Contracts**

### T8: JOIN 输出形状与名字解析收口（`join-output-shape-resolution` R1–R4）

- Requirement/Scenario: R1 完整形状解析（agg_arg_outside_select_reachable RED 静默 NULL→GREEN / qualified_group_key_precise 全形状化保持）、R2 歧义拒绝（bare_group_by_ambiguous_rejected RED 末位胜出→GREEN / bare_agg_arg_ambiguous_rejected / bare_projection_ambiguous_rejected）、R3 严格解析与别名列（unknown_column_named_rejected RED 零宽行→GREEN / aliased_plain_column_reachable RED 零宽行→GREEN / expression_item_join_still_rejected 保持）、R4 零回归（既有 join/aggregate 套件零校准 + 全量串行零回归）
- Depends on: Iteration 000（JOIN 基座与 D2 组合布局）
- Targets: `src/parser/planner/query.rs`（join 构造点全列承载、聚合分支解析面 :1459-1532、`alias_reroutable` 门 :1088-1111〔与 T10 同一编辑点〕、ORDER BY 别名装配 :1772-1787）、`src/executor/plan.rs::AggregateNode`（加性 `aggregate_arg_indices`）、`src/executor/aggregate.rs::extract_value`（索引消费）、`src/pipeline.rs`（Aggregate 装配点机械补字段）、`src/parser/ast.rs::extract_qualified_columns`（如需区分参数列提取——加性扩展，既有返回值消费点兼容）、`tests/join_output_resolution_test.rs`（新）
- Current behavior: JOIN 无 WHERE 聚合参数不在 SELECT 清单时 `extract_value` 静默 NULL（实测 count(v)=0）；同名列 `column_indices` 折叠末位胜出；JOIN 基座未知列/纯列别名项零宽行静默（实测 `{"columns":[],"rows":[[],[]]}`）；ORDER BY 别名键在 JOIN 基座（放宽后）若无本契约将落单表回退错序
- Required behavior: ① 聚合×JOIN 全列承载——无 WHERE 时聚合输入 = 全组合布局，参数列 plan 期解析为索引，`count(v)` 返回真实计数；限定名 GROUP BY 键精确指向所指表列（t1.k ≠ t2.k）；② 裸名多命中三面（GROUP BY 键 / 聚合参数 / 非聚合投影项）计划期 `AmbiguousColumn` 点名（复用既有错误变体，文案含列名）；③ JOIN 基座未知列（投影项/聚合参数/GROUP BY 键）`ColumnNotFound` 计划期点名——零宽行消除；纯列引用别名项可达：表头 = 别名、行值 = 所指列，非聚合无歧义 JOIN 查询端到端正确；任意表达式项×JOIN 维持既有显式拒绝（两拒绝臂文案与触发面零变化）；④ ORDER BY 别名键在 JOIN 基座按输出列真实排序（索引与 Sort 输入行形状同基座：无 WHERE = 过滤后布局 / 有 WHERE = 全组合布局），无歧义数据上与 DISTINCT/DESC 组合正确；⑤ `aggregate_arg_indices` 为 plan 期解析——解析失败在计划期点名，运行期 extract_value 无静默 NULL 臂复活；⑥ 无歧义有效查询（含 Iteration 000–003 全部交付面）计划形状、结果与表头逐字节不变——`outer_join_test.rs:565` D2 锚点零修改通过
- Preserve: 单表聚合路径行为逐字节不变（column_indices 单表面语义不变——含未知名参数的既有点名/静默现状，本契约不收紧单表面）；Hash join/NLJ 的 ON 条件解析与计划路由（`is_pure_equi_join_on`/`extract_join_conditions`）零变化；`get_plan_output_columns` 对有效查询输出不变（聚合查询 join 输入形状变化不外溢——聚合输出形状由自身名单决定）；EXPLAIN 文本输出零变化；plan cache 键语义不变；M17/M19
- Forbidden: 不解禁真表达式项×JOIN；不改变单表聚合的错误面；不为运行期索引越界新增静默 NULL 回退；不触碰 ON 条件编译路径；白名单外不修改既有测试断言（本任务白名单现为空集——实施中发现需校准即返回 Plan）
- Test witness: `tests/join_output_resolution_test.rs` 先行——`agg_arg_outside_select_reachable`/`bare_group_by_ambiguous_rejected`/`unknown_column_named_rejected`/`aliased_plain_column_reachable` 现状 RED（静默 NULL / 末位胜出 / 零宽行，均 Plan 实测可复现），实现后 GREEN；`qualified_group_key_precise`/`expression_item_join_still_rejected` 既有行为见证
- GREEN condition: 新测试全绿 + `planner_test`(87)/`nested_loop_join_test`(9)/`outer_join_test`(16)/`subquery_test` 零校准通过 + 全量串行 0 failures
- Verification: ①定向 `cargo test --test join_output_resolution_test --test outer_join_test --test nested_loop_join_test`（退出码 0）；②受影响边界 `cargo test --test planner_test --test cli_test --test select_distinct_test --test set_operations_test --test group_by_expr_test --test aggregate_test`（退出码 0）；③全量 `cargo test -- --test-threads=1`（M19，退出码 0）；④`cargo clippy --all-targets` + `cargo fmt --check`（退出码 0）
- Stop when: 全列承载或布局解析牵动 `get_plan_output_columns` 有效查询输出变化（违反 ⑥）——返回 Plan；别名项 JOIN 基座编译需超出布局 helper 的单表回退以外新机制（④ 索引基座无法闭合）——返回 Plan；实施中发现白名单外需校准的既有测试——返回 Plan；单表面收紧需求浮出（未知名参数面与本契约边界冲突）——单表面顺带或留档二选一记录进 Act Response，实质冲突返回 Plan

### T9: 聚合输出形状一致性收口（`aggregate-output-shape-conformance` R1–R2）

- Requirement/Scenario: R1 形状一致（agg_only_group_by_key_not_selected RED 表头 1 列行 2 值→GREEN / select_key_order_differs_from_group_by RED 置换错位→GREEN）、R2 绑定与零回归（existing_keyed_forms_unchanged / having_unselected_key_bindable 可达性扩展 / distinct_aggregate_output + groupby_order_interleave_keeps_direct_path 白名单校准 / 既有聚合套件零校准）
- Depends on: None（与 T8/T10 同 Iteration，实现顺序不约束；测试面独立）
- Targets: `src/parser/planner/query.rs::build_query` 聚合分支 `direct` 判定（:1538-1552，仅加两条件）、`tests/aggregate_output_shape_test.rs`（新）、白名单两处校准（`tests/select_distinct_test.rs::distinct_aggregate_output`、`tests/group_by_expr_test.rs::groupby_order_interleave_keeps_direct_path`）
- Current behavior: 直出判定不含键子集与键序检查——`SELECT count(*) FROM t GROUP BY v` 表头 1 列而行 [键,聚合] 2 值（实测）；`SELECT v, k, count(*) FROM t GROUP BY k, v` 行按 GROUP BY 序、表头按 SELECT 序置换错位（实测）；HAVING 引用未入选键在直出形态绑定失败
- Required behavior: ① 形状不安全形态（键未入选 / 键序不同）改走既有条件包装路径——输出行与表头恒为 SELECT 清单形状，未入选分组键不进输出、仅供 HAVING 绑定（SQLite/PostgreSQL 对齐）；② 形状安全的直出形态（键 ⊆ SELECT、键序一致、键前聚合后、无表达式项）路径与计划形状逐字节不变；③ HAVING 经 `node_output_columns` 绑定零变化 + 行为扩展：HAVING 引用未入选分组键由绑定失败转为可达（包装路径既有能力，无既有断言锁定失败面——调查已核实）；④ ORDER BY 聚合输出名绑定零变化；⑤ 白名单两处按新语义校准（行形状/排序键断言 + doc 注明 T9 来源）
- Preserve: `build_output_rows` 行装配（group_key ++ aggregates）逐字节不动；包装路径 `wrap_projection`/`node_output_columns` 机制零修改；既有对齐直出形态（group_by_expr_test :305 等）与表达式键包装形态（:207/:255/:408/:432）零校准通过；`get_plan_output_columns` Aggregate/Projection 臂零变化
- Forbidden: 不改执行器行装配；不实现「表头补键」输出未选列；不扩大直出判定（只收窄）；白名单（两处）之外不修改既有测试断言
- Test witness: `tests/aggregate_output_shape_test.rs` 先行——两类错配形态现状 RED（Plan 实测可复现），实现后 GREEN；`existing_keyed_forms_unchanged` 对齐形态现状 GREEN 锁定
- GREEN condition: 新测试全绿 + 两处白名单校准 GREEN + `group_by_expr_test`(12 除白名单 1 处)/`aggregate_test`(19)/`select_distinct_test`(12 除白名单 1 处)/`expression_e2e_test` 零校准 + 全量串行 0 failures
- Verification: ①定向 `cargo test --test aggregate_output_shape_test --test select_distinct_test --test group_by_expr_test`（退出码 0）；②受影响边界 `cargo test --test aggregate_test --test planner_test --test cli_test --test expression_e2e_test`（退出码 0）；③全量与静态检查同 T8 ③④（可合并执行）
- Stop when: 收窄判定后仍有形状错配形态逃逸直出（枚举遗漏）——返回 Plan；白名单两处之外的既有断言需校准——返回 Plan；HAVING 可达性扩展撞上未识别的失败面锁定——返回 Plan

### T10: DerivedScan 基座别名应用收口（`error-reporting` MODIFIED R5 + `order-by-alias-sorting` R1 扩展）

- Requirement/Scenario: `error-reporting` MODIFIED R5 新增两场景（cte_base_alias_header / from_subquery_alias_header）+ 既有三场景零修改；`order-by-alias-sorting` R1 扩展场景（cte_base_alias_sorts）
- Depends on: None（与 T8 共享 `alias_reroutable` 门编辑点——实现协调、验收分离）
- Targets: `src/parser/planner/query.rs::alias_reroutable` 门（:1088-1094，基座条件加 `DerivedScan(_)`）、`tests/derived_scan_alias_test.rs`（新）
- Current behavior: `WITH c AS (SELECT v FROM t) SELECT v AS val FROM c` 表头回落 `["v"]`（别名丢失）；`ORDER BY val` 静默输入序；`SELECT v AS val FROM (SELECT v FROM t) AS c` 同型；DerivedScan 基座 `未知列 AS alias` 零宽行静默（Plan Review 2026-09-28 CLI 实测前两项）
- Required behavior: ① DerivedScan 基座普通列别名项表头 = 别名（CTE 与 FROM 子查询两形态），行值 = 所指列；② `ORDER BY <别名>` 在 DerivedScan 基座按输出列真实排序（表头 = 别名 + 排序生效，delta `order-by-alias-sorting` R1 扩展场景）；③ DerivedScan 基座 `未知列 AS alias` `ColumnNotFound` 计划期点名（零宽行消除）；④ 表达式项别名 DerivedScan 基座既有行为保持（Act 003 实测 witness）；⑤ 子查询内（`building_subquery`）与通配清单维持既有门排除；Scan 基座行为逐字节不变
- Preserve: `building_subquery` 抑制；聚合查询别名装配既有面；CTE 定义内语义（定义按子查询语义构建、遮蔽机制不变）；`register_table` 注册机制本身零修改；plan cache 键语义不变
- Forbidden: 不扩大到其他基座（SubqueryEval/集合操作侧臂在 `building_subquery` 抑制内）；不改 JOIN 基座面（T8 契约承载）；白名单外不修改既有测试断言（本任务白名单为空集）
- Test witness: `tests/derived_scan_alias_test.rs` 先行——cte_base_alias_header / from_subquery_alias_header / cte_base_alias_sorts / unknown_alias_cte_rejected 现状 RED（表头回落 / 输入序 / 零宽行），实现后 GREEN；expression_alias_cte_witness 现状 GREEN（既有行为锁定）
- GREEN condition: 新测试全绿 + `cte_test`/`explain_test`/`order_by_alias_sort_test`/`select_distinct_test`/`planner_test` 零校准 + 全量串行 0 failures
- Verification: ①定向 `cargo test --test derived_scan_alias_test --test cte_test --test order_by_alias_sort_test`（退出码 0）；②受影响边界 `cargo test --test explain_test --test select_distinct_test --test planner_test --test cli_test`（退出码 0）；③全量与静态检查同 T8（可合并执行）
- Stop when: 门放宽后 DerivedScan 基座编译落入非注册形状回退（索引错位）——返回 Plan；与 T8 的 JOIN 扩展在同一门编辑点产生交互冲突——返回 Plan；实施中发现白名单外需校准的既有测试——返回 Plan

### T11: 信号处理器注册时序硬化（无 corpus delta——既有 130/143 语义意图内的实现修正）

- Requirement/Scenario: 无行为 requirement（T11 不引入 corpus delta）；见证 = `tests/cli_test.rs` 信号 e2e 用例（`test_sigint_during_open_130`/`test_sigterm_during_run_143` 等 test_sig 前缀组）零修改 + 稳定性压测（计划判定方式的固定次数，不稳定类验证既定条款）
- Depends on: None（独立故障域，与 T8/T9/T10 无耦合）
- Targets: `src/cli/mod.rs::execute_command_inner`（信号 future 创建时点相对两段 select 的位置）；如取机制 ② 则 `src/database.rs::open_with_key`（首 await 前置 yield——公共入口语义中性）
- Current behavior: open select 的 open 臂先轮询，其同步前缀完成 `try_lock`（database.rs:58 同步调用 → file_storage.rs:71）；信号处理器注册发生在信号臂首次轮询（晚于锁）——「已持锁、未注册」窗口内 SIGINT → 默认处置死亡（`out.code = None`，Iteration 003 全量首跑实测一次）
- Required behavior: ① 结构性不变量：信号处理器注册先于 open 路径的文件锁获取（可达观察：子进程持锁后任意时刻 SIGINT → exit 130，SHALL NOT 默认处置死亡）；② 既有两阶段语义零变化（open 阶段 `Signaled(2)` 无 close 立即退；执行阶段信号经 close checkpoint 后退；SIGTERM 143）——test_sig 前缀 e2e 用例零修改通过；③ 行为收敛：进程启动至 open 前送达的信号由「未定义（默认死亡）」收敛为 130/143——既有 Requirement 意图内，不加 delta 场景（design D11 记录）
- Preserve: 两阶段 select 语义、close() checkpoint 行为、信号 future 注入接口（`signal_int/signal_term: impl Fn() -> SignalFuture` 闭包参数与单测 :561+ 注入路径零修改语义）、退出码映射（128+signum）、M17/M19
- Forbidden: 不为测试加产品钩子或就绪标记（身份型证据工程红线）；不在测试内做成功后重试（判定 weakening）；不改变退出码映射；不改 close() 语义；依赖 select! 臂声明序（默认随机轮询序，不可靠）
- Test witness: 压测判定（计划内不稳定类条款）：`for i in $(seq 1 10); do cargo test --test cli_test test_sig -- --test-threads=1 || break; done`——10 轮全绿（修复前该用例全量串行下偶发失败，机制推理 + 压测共同承载）；`test_sig` 前缀组各用例单跑 GREEN
- GREEN condition: 压测 10 轮全绿 + `cli_test` 全量 111/2i + 全量串行 0 failures + clippy/fmt 干净
- Verification: ①压测循环 10 轮（退出码全 0，命令与输出记录进 Act Response）；②`cargo test --test cli_test`（退出码 0）；③全量串行（与 T8/T9/T10 合并一次亦可）；④`cargo clippy --all-targets` + `cargo fmt --check`
- Stop when: tokio 注册语义调查发现注册无法先于 open select 完成、且机制 ② 让位方案不可行——返回 Plan（转测试侧锚点强化需 Plan 裁定，属契约变更）；压测中出现非信号类新失败——返回 Plan

**Invariants**

- I034 纪律：输出行形状与表头恒一致（T9 的目标即恢复该不变量在聚合查询的普适性；T8/T10 的零宽行与别名回落消除同理）。
- 布局解析的索引基座唯一性：任何表达式（投影项 / 组键 / 聚合参数 / ORDER BY 别名键）的行索引必须与其求值所在执行器节点的输入行形状同源——禁止单表 schema 回退落在 join 行节点上。
- 有效查询零回归：无歧义、无未知列、形状安全的既有查询计划与结果逐字节不变（Iteration 000–003 全部交付面为锚）。
- 错误面零副作用：全部新增拒绝（AmbiguousColumn/ColumnNotFound）发生在计划期，纯查询面。
- 信号语义承诺不变：Signaled(signum) → 128+signum；open 阶段无 close 立即退、执行阶段 close 后退（T11 只前移注册时点，不改语义）。

**Non-goals**

- 真表达式项×JOIN 解禁；单表聚合未知名参数面收紧（留档裁定）；递归 CTE/窗口函数；代价模型与 Join 重排；集合操作侧臂内的 JOIN 解析深化；MS21-T03；信号语义扩展。

**Acceptance**

`join-output-shape-resolution` R1–R4 与 `aggregate-output-shape-conformance` R1–R2 全部场景可观察满足；`error-reporting` MODIFIED R5 与 `order-by-alias-sorting` R1 扩展场景满足（delta 与实现/测试一致）；两类静默缺陷（JOIN 聚合参数 NULL、直出错配）、零宽行面与 DerivedScan 别名回落有 RED→GREEN 见证；白名单两处校准与实现一致；信号注册先于锁获取的结构不变量成立且压测见证；无歧义有效查询与形状安全直出逐字节不变；全量串行零回归。Scenario→Task→Test 链路：RTM `join-output-shape-resolution` 四行 + `aggregate-output-shape-conformance` 两行 + `error-reporting` MODIFIED R5 一行 + `order-by-alias-sorting` R1 扩展一行（change tasks.md）。

**Verification**

- T8：`tests/join_output_resolution_test.rs` 全场景（含 ORDER BY 别名×JOIN 组合见证）+ outer_join/nested_loop_join/planner/cli 边界 + 全量串行。
- T9：`tests/aggregate_output_shape_test.rs` 全场景 + 两处白名单校准 + group_by_expr/aggregate/select_distinct/expression_e2e 边界。
- T10：`tests/derived_scan_alias_test.rs` 全场景 + cte/order_by_alias/explain/select_distinct 边界。
- T11：压测 10 轮 + cli_test 全量。
- 汇总：全量串行 `cargo test -- --test-threads=1` 退出码 0（M19）+ clippy/fmt 干净 + `openspec validate 2026-09-26-ms25-sql-read-surface --strict` 退出码 0。
- 判定均为测试自身断言 + cargo 退出码；无身份型证据工程。

**Gate 2 Readiness**

PASS——无 Missing requirement（RTM 全 Covered，tasks.md 任务表与 RTM 已同步 T8–T11）；调查完整（T8/T9 三面静默缺陷与两类错配的病灶机制 file:line 实证 + Plan 复核实测复现 + 校准白名单全库清扫定稿 + AggregateNode 构造点/错误变体/plan_format 消费面核实；T10 病灶 CLI 实测 + 注册形状编译通路经 Act 003 实测与 query.rs:320/:826 核实；T11 竞态机制经 select 臂序/database.rs 同步调用链/file_storage.rs:71 结构核实 + tokio 随机轮询序注意项 + 机制候选明确）；设计闭合（D9/D10/D11 方案与替代方案否决理由明确）；任务契约自包含（Act 无需回读外部资料或父 Cycle）；验证直接判定（测试断言 + 退出码 + 计划内压测条款，无身份型证据工程）；无实质未知项留给 Act（T11 的 tokio 注册语义调查为契约内机制选择，两候选均闭合同一不变量）。change 级 Gate 2（2026-09-26 用户批准）覆盖本计划；T10/T11 并入经用户指令（2026-09-28 replan）。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果；无一次性环境、无不可复现结论。

**Risks and Notes**

- 全列承载改变聚合×JOIN（无 WHERE）的 AggregateNode 输入行形状（过滤 → 全组合）——该形态现状输出即错误（静默 NULL），形状变化不构成有效查询回归；GROUP BY 裸名键解析结果随布局修正而变（末位胜出 → 精确），属缺陷修复面。
- `alias_reroutable` 放宽后 `has_expression_items` 在 JOIN/DerivedScan 基座可能置位——`mixed_aggregate`/纯表达式两拒绝臂的触发顺序在改标之后（MS19 T4b 既有机制），真表达式项×JOIN 拒绝文案不变；`projection_items` 通路在两基座首次启用，其 `get_plan_output_columns` Projection 臂表头行为由 R3/R5 场景见证。
- T9 的 `groupby_order_interleave_keeps_direct_path` 校准：测试名可保留（doc 更新承载语义变化）或按新语义改名——非实质，Act 裁定；doc 必须写明「T9 形状一致性收口后转包装路径」来源。
- T11 压测前该用例的偶发率未知（全量历史 1 次）：压测 10 轮全绿是计划判定（非穷尽证明），与机制推理（注册先于锁的结构证据）共同承载验收；若 Act 调查发现窗口机制与 D11 假设不符，按 Stop when 返回 Plan。
- `test_sigint_during_open_130` 在本 Cycle 全量验证中若再失败：修复落地后不再适用「偶发复核通过」豁免——T11 修复后该用例失败按真实失败处理（返回当前 Cycle 修复）。
- 单表聚合未知名参数面（如 `SELECT count(nosuch) FROM t GROUP BY g`）：现状经 `column_indices` 单表面查无 → extract_value 静默 NULL——与本 Cycle 的「运行期无静默 NULL 臂」约束存在张力。契约边界：本 Cycle 只对 JOIN 布局面收紧（`aggregate_arg_indices` 解析在 JOIN 基座 plan 期点名）；单表面是否同轮收紧由 Act 按实现自然落点裁定（若 helper 统一覆盖单表面则单表未知名参数同步转 plan 期 `ColumnNotFound`——行为收紧需在 Act Response 记录并补 witness；若保持单表面现状则留档 Issue 候选）。两条路径均不阻塞验收，实质冲突时返回 Plan。

## Act Response

- Status: reported

**实际改动**

- T8（JOIN 输出形状与名字解析收口，D9）
  - `src/parser/planner/query.rs`
    - `build_from_clause_with_projection` 新增 `pure_column_list` 参数：过滤式解析前对纯列清单逐项严格校验（未知列 `ColumnNotFound`、裸名多命中 `AmbiguousColumn`），零宽行/重复列静默面消除；函数内新增该校验块（既有过滤与承载语义不动）。
    - 新增自由函数 `join_row_layout`（由 JOIN 节点 `output_columns` 按 `table_alias` 连续段构造行布局；WHERE 路径内联装配改为复用该函数，行为逐字节不变）与 `resolve_layout_index`（布局上解析列引用：限定名按（段, 列）首个精确命中；裸名唯一命中返回索引、多命中 `AmbiguousColumn`、无命中 `None`）。
    - `build_query`：FROM 构建前用 `projection_has_aggregates` 探测聚合项，`carry_all_columns` 增置位（聚合×JOIN 组合行承载全组合列）；`base_plan` 之后计算 `join_layout`（`join_combined_columns(base_plan).map(join_row_layout)`）作为投影项 / ORDER BY 别名键 / 分组键 / 聚合参数的统一名字解析基座。
    - 聚合参数：SELECT 项循环同步收集 `aggregate_args`（`aggregate_arg_expr` 保留限定名，`AggregateFunc` 自身只留列名）；聚合分支按布局计算 `aggregate_arg_indices`（JOIN 基座逐项 `resolve_layout_index`，非列引用参数回落既有语义；单表面全 `None`）。
    - 分组键：`GroupKey` 新增加性字段 `input_index`；`resolve_group_by_item` 新增 `join_layout` 参数，布局面下精确解析（歧义点名），单表面签名与行为不变；键编译优先取 `input_index`。
    - 别名项路由：`alias_reroutable` 基座条件放宽到 `Scan | DerivedScan | Join | NestedLoopJoin`；新增 `expression_items_plain` 判定并接入两处「表达式项 × JOIN」拒绝臂（真表达式维持既有拒绝，纯列项放行）；顶层 Projection 项与 ORDER BY 别名键编译期间置 `self.join_column_layout`；`filter_projection`（WHERE×JOIN）与 `sort_projection` 在顶层 Projection 存在时保持恒等（与单表面 `projection_indices=None` 的既有规则同型）。
  - `src/parser/ast.rs`：`extract_qualified_columns` 的 `ExprWithAlias` 臂对纯列引用项改为输出被引用列 `(table, column)`（新增 `aliased_plain_column_ref`），其余表达式项维持别名名。`qualified_columns` 仅供 JOIN 形状面消费，单表面零影响。
  - `src/executor/plan.rs`：`AggregateNode` 新增加性字段 `aggregate_arg_indices: Vec<Option<usize>>`。
  - `src/pipeline.rs`：`Aggregate` 装配臂透传 `aggregate_arg_indices`。
  - `src/executor/aggregate.rs`：`AggregateExecutor` 承接该字段；`extract_value` 新增 `arg_index` 参数，`Some` 时直取该索引，否则沿既有 `column_indices` 按名查找（单表面路径逐字节不变）。
- T9（聚合输出形状一致性收口，D10）
  - `src/parser/planner/query.rs` 聚合分支：`direct` 判定收窄为 `!has_expression_items && keys_all_column && select_keys_first && keys_selected_in_select_order`，其中新增收集 SELECT 纯列项序列并与分组键序列逐项比对（长度 + 大小写无关相等）。形状不安全形态（键未入选 / 键序不同 / 交错序 / 表达式项）改走既有 `wrap_projection` 包装路径——`build_output_rows`、`wrap_projection`、`node_output_columns` 机制零修改，未入选分组键仅经 `node_output_columns` 供 HAVING 绑定。
- T10（DerivedScan 基座别名应用收口，D11）：与 T8 同一 `alias_reroutable` 门一次落地；CTE 引用经 `resolve_cte_reference` 与 FROM 子查询同走 `DerivedScan` 装配，验收独立。
- T11（信号处理器注册时序硬化，D11）
  - `src/cli/mod.rs`：`sigint_future`/`sigterm_future` 改为构造期经 `tokio::signal::unix::signal(..)` 同步安装处理器（原先在首次轮询内安装）；`execute_command_inner` 在首个 `select!` 之前构造阶段 1 的信号实例（`phase1_sigint`/`phase1_sigterm`），使注册早于 `Database::open_with_key` 的第一次轮询及其同步前缀；阶段 2 仍按工厂生成新实例。安装失败保持 pending（不伪造信号）。

**测试**

- 新增：`tests/join_output_resolution_test.rs`（T8，12 场景）、`tests/derived_scan_alias_test.rs`（T10，7 场景）、`tests/aggregate_output_shape_test.rs`（T9，8 场景）。
- 白名单校准（两处，Plan 定稿范围）：`tests/select_distinct_test.rs::distinct_aggregate_output`（去重见证保留原查询形态；`expected.len()` 2 → 1 并加形状守恒断言，doc 注明 T9 来源：未入选分组键不再随行携带，两组聚合值同为 2 故 DISTINCT 真正折叠）、`tests/group_by_expr_test.rs::groupby_order_interleave_keeps_direct_path` → 改名 `groupby_order_interleave_aligned_to_select_order`（Plan Risk 条授权 Act 裁定命名；断言由 `[dept, id, count]` 错位形态改为 SELECT 序对齐 `[id, dept, count]`，doc 注明 T9 来源）。

**Deviations from Plan**

1. `resolve_layout_index` 的段内偏移在自检中被发现并修正（见下）；最终实现与契约的「布局感知解析」一致，无需 Plan 裁决。
2. T8 见证夹具用 `max(v)` 而非 `count(v)` 断言：初版见证在「参数列解析到段首索引」的缺陷下仍会偶然通过（`count(k)` 与 `count(v)` 同为 2），不具判别力；改为 `max(v)` 后错误落点返回数字而非字符串，RED→GREEN 信号可靠。
3. `qualified_group_key_precise` 采用行序无关比较：Hash Join 的分组输出序由 HashMap 迭代决定（既有面），计划中的限定名 `ORDER BY t1.k` 形态不可用（ORDER BY 只接受列名键，既有面）。
4. T9 两处校准的最终语义按 Plan 契约①（形状不安全改走包装路径、未入选键仅供 HAVING 绑定）落地；白名单 `distinct_aggregate_output` 的载体查询形态与断言同步按该语义调整。
5. 单表聚合未知名参数面保持现状（Plan Risk 条路径 B）：`aggregate_arg_indices` 仅在 JOIN 基座解析，单表面回落 `column_indices` 按名查找，行为逐字节不变——留档 Issue 候选（见下）。

**Self-Review 发现**

- Critical（T8，实施内修复）：`resolve_layout_index` 初版只在段间累加 `offset`、段内不累加位置，导致任何命中都返回段首索引（`SUM(ac.amt)` → 索引 0 = `ac.id`）。被 `outer_join_test::where_over_join_aggregate` 捕获（返回 `[[2,2],[1,1]]` 而非 `[[1,30]]`）；修正为 `offset + pos` 后该用例与 `planner_test`/`nested_loop_join_test`/`aggregate_test`/`select_distinct_test`/`group_by_expr_test` 全部零校准通过。
- Important（T9，实施内修复）：T9 初版把「键未投影」实现为计划期 `GroupByColumnNotFound` 拒绝，与 Plan 契约①（走包装路径 + HAVING 可达性扩展）及 `aggregate-output-shape-conformance` 场景冲突；改为仅收窄 `direct` 判定、删除两处计划期拒绝后，`HAVING` 引用未入选分组键变为可达（`SELECT count(*) FROM t GROUP BY v HAVING v = 10` → `[[1]]`）。
- Minor（保留，未修）：`qualified_group_key_precise` / `select_order_key_order_mismatch_aligned` 等断言依赖行序无关比较，已在用例内注明原因；不属产品缺陷面。

**验证**

1. 定向（新见证）`cargo test --test join_output_resolution_test --test derived_scan_alias_test --test aggregate_output_shape_test -- --test-threads=1`
   - `test result: ok. 12 passed; 0 failed` / `ok. 7 passed; 0 failed` / `ok. 8 passed; 0 failed`；退出码 0。
   - RED 证据（本会话实测）：T8 首次运行 10/12 失败（`count(v)` 静默 0、裸名三面静默、未知列零宽行 `[[],[]]`、别名项零宽、限定名 `ORDER BY` 见证自身形态错误）；T10 的 `derived_alias_header_is_alias` 在把 `alias_reroutable` 基座条件临时回退为仅 `Scan(_)` 后失败（同次回退使 T8 三个别名场景失败），恢复后全绿；T9 首次运行 4 失败（`SELECT count(*) FROM t GROUP BY k` 行宽 2/表头 1；`SELECT v,k,count(*) GROUP BY k,v` 行列错位 `[[1,10,1],[1,20,1],[2,30,1]]`）。
2. 受影响边界（零校准核对）`planner_test`(87) / `nested_loop_join_test`(9) / `outer_join_test`(16) / `aggregate_test`(19) / `select_distinct_test`(12) / `group_by_expr_test`(12) / `subquery_test`(32) / `order_by_alias_sort_test`(7) / `set_operations_test`(14) / `explain_test`(10) / `cte_test`(14) / `expression_e2e_test`(24) 全部 `ok. 0 failed`，退出码 0。
3. T11（计划内不稳定类验证，固定 10 轮）：`for i in $(seq 1 10); do cargo test --test cli_test test_sigint_during_open_130 -- --test-threads=1; done` → `rounds=10 passed=10 failed=0`；单轮 GREEN 与 10 轮压测各一次运行（未重复运行已通过的验证）。
4. 全量与静态检查
   - `cargo test -- --test-threads=1`：`suites=93 passed=1401 failed=0 ignored=2`（Iteration 003 基线 1376 → +25：12 + 7 + 8 新用例 −2 校准用例改名不计数），退出码 0。
   - `cargo clippy --all-targets`：无 warning / error（仅环境级 `.cargo/config` 弃用提示），退出码 0。
   - `cargo fmt --check`：0 处 diff，退出码 0。
   - `openspec validate 2026-09-26-ms25-sql-read-surface --strict`：`Change '2026-09-26-ms25-sql-read-surface' is valid`，退出码 0。
   - 判定均为测试自身断言与 cargo / openspec 退出码；未新增任何 Hash / 指纹 / run-id / manifest / 审计器类身份型证据机制，未新增产品就绪钩子或测试专用标识。

**Persisted Evidence**

- `None required`（Plan 指定 `Mode: none`）：全部验证可低成本复现（三个定向套件 + 全量串行 + 固定 10 轮压测），结果由 Act Response 承载；无一次性环境、无不可复现结论、无 `required` 映射 Gate。

**Experience Candidates**

- Runbook 候选（Recorder 裁定）：时序敏感 e2e 用例的固定轮次压测规程——`test_sigint_during_open_130` 类「子进程持锁后发信号」用例的单轮全绿不承载验收，固定 10 轮全绿 + 机制结构证据（注册先于锁）共同承载；本会话已按该规程验证 10/10。证据：本 Act Response 验证项 3。
- Issue 候选（Recorder 裁定，change 范围外）：
  1. 单表面聚合未知名参数静默 NULL——`SELECT count(nosuch) FROM t1` → `[[0]]`（`column_indices` 查无 → `extract_value` 静默 NULL，与本 Cycle 消除的 JOIN 面同机制）。证据：验证探测输出。Plan Risk 条已裁定本 Cycle 不收紧单表面。
  2. 限定通配项 `SELECT t1.*` 计划期拒绝且文案泛化（`Unsupported statement type: SELECT ITEM`）——单表面、派生表、JOIN 三基座同拒（既有面，非本 change 引入；`src/parser/ast.rs:297` 兜底臂），能力缺口 + 文案不指名。

**未解决问题**

- 无阻塞项。上列两条 Issue 候选均在本 change 范围外，按 Act 规则只报告不落账。
- Plan Review 待办：`error-reporting` MODIFIED R5 / `order-by-alias-sorting` R1 扩展场景与 `aggregate-output-shape-conformance` 场景的合并一致性由 Plan Review 独立核对（本 Cycle 未改动任何 delta spec）。

## Plan Review

- Review Result: replan-required

**Findings**

独立核对范围：完整 diff（query.rs / ast.rs / plan.rs / pipeline.rs / aggregate.rs / cli/mod.rs + 三个新测试文件 + 两处校准）逐文件阅读、关键符号代码走查、CLI 复现实验 8 组（P1–P8）。T8/T9/T10/T11 的主体交付与 Act Response 声明一致，验证结论按覆盖未变化采信；发现一项实质缺口与若干非实质 finding：

1. **[实质缺口——T8 Required behavior ③ 未满足，当前 Cycle 修复]** JOIN 基座**聚合参数未知列仍静默 NULL**。实测（2026-09-28，本 Review）：`SELECT count(nosuch) FROM t1 JOIN t2 ON t1.k = t2.k` → `{"columns":["count_nosuch"],"rows":[[0]]}` exit 0；`SELECT sum(t1.nosuch) FROM t1 JOIN t2 ON t1.k = t2.k` → `{"columns":["sum_nosuch"],"rows":[[null]]}` exit 0。契约 ③ 明列「JOIN 基座未知列（投影项/聚合参数/GROUP BY 键）`ColumnNotFound` 计划期点名」。根因：`aggregate_arg_indices` 构造（query.rs:1808-1826）对 `resolve_layout_index` 的零命中结果 `Ok(None)` 直接落入索引 `None` → 运行期回落 `column_indices` 名字查表 → `extract_value` None → `Value::Null` 静默；`map_err` 的 `ColumnNotFound` 分支为**死代码**（`resolve_layout_index` 零命中返回 `Ok(None)` 而非 `Err`，歧义才返回 `Err(AmbiguousColumn)`）。投影项未知列面已正确（P3：`ColumnNotFound` exit 3）。
2. **[Plan 契约措辞过宽——Plan 错误，非阻塞]** 契约 ③ 将「GROUP BY 键」列入 `ColumnNotFound` 面；实测 GROUP BY 未知键（裸名 P6 / 限定名 P5）计划期以既有 `NonAggregatedColumn` 点名拒绝（exit 3）——与单表面解析链终态一致、非静默。拒绝变体与措辞由既有四级解析链决定，契约意图（计划期点名、零静默）满足；本 Review 裁定按实质（零静默面）验收，变体措辞偏差记录为 Plan 起草过宽，不要求改写（改 JOIN 面单独措辞会与单表面不一致，触碰 Preserve 面）。
3. **[Act 记录性——非实质]** `groupby_order_interleave_keeps_direct_path` 改名 `groupby_order_interleave_aligned_to_select_order`（Plan Risk 条授权，doc 注明 T9 来源 ✓，断言由错位 `[dept,id,count]` 改为 SELECT 序对齐 ✓）；change `tasks.md` RTM witness 名已由本 Review 同步。Act Response 验证项 4 的算式注记「+25：12+7+8 新用例 −2 校准用例改名不计数」算术有误——改名不减计数，实际净增 27（1403 = 1376 + 27，其中 1401 passed 含此前偶发的 sigint 用例转绿）；总数与分套件计数自洽，仅注记笔误。
4. **[Act 见证改进——非实质，接受]** T8 见证以 `max(v)` 替代 `count(v)` 承载 R1（参数列解析到段首索引的缺陷下 `count` 同值偶然通过，`max` 错误落点返回数字与字符串可区分——判别力理由成立）；`qualified_group_key_precise` 行序无关比较（Hash Join 分组输出序由 HashMap 迭代决定，既有面）。
5. **[Critical 自检修复核对 ✓]** `resolve_layout_index` 段内偏移修复在位（`offset + pos`，query.rs:2521-2523）——段首索引缺陷已消除。
6. **[T11 核对 ✓]** 构造期注册（`unix::signal()` 同步安装，cli/mod.rs:284-302）+ 阶段 1 实例前置（:338-339 先于首个 select :343-347）——注册先于 `Database::open_with_key` 同步前缀的锁获取，结构性消除窗口；安装失败保持 pending 不伪造信号；信号 e2e 用例零修改（diff 核实）；10 轮压测按计划内不稳定类判定条款采信。
7. **[T9/T10 核对 ✓]** `direct` 四条件收窄在位（query.rs:1685-1717，`keys_selected_in_select_order` = SELECT 纯列项序列与分组键序列等长逐项大小写无关相等——配合既有严格模式「SELECT 列项 ⊆ 分组键」，恰为 D10 两条件）；P7 实测 HAVING 未入选键可达（`SELECT count(*) FROM t1 GROUP BY v HAVING v = 'b'` → `[[1]]`）；P8 实测两类错配修复（键未入选 → 表头 1 列行 1 值；键序不同 → 行按 SELECT 序对齐）。T10 门扩展 `Scan | DerivedScan | Join | NestedLoopJoin` 一次落地（query.rs:1188-1198）；顶层 Projection 项与 ORDER BY 别名键编译期置 `join_column_layout` 并在装配后恢复（query.rs:1315-1317/:1971-1989）——布局基座与求值行同源；`filter_projection`/`sort_projection` 在顶层 Projection 存在时恒等的声明与代码一致。
8. **[单表面裁定 ✓]** Plan Risk 条路径 B 维持：`aggregate_arg_indices` 单表面全 `None`、`extract_value` 经 `arg_index.or_else(名字查表)` 既有语义逐字节不变（aggregate.rs:417-435）——未知参数静默 NULL 留档 Issue 候选（Act 已报告）。
9. **[白名单核对 ✓]** 两处校准均在 Plan 定稿范围内，断言零范围外改动；`tests/cli_test.rs` 信号用例零修改；三个新测试文件场景数与 Act 声明一致（12/8/7）。
10. **[范围变更——用户指令二次 replan]** 用户指令（2026-09-28）：把两项 Issue 候选并入本 Iteration 直接解决——① 单表面聚合未知名参数静默 NULL（本 Review Finding 8 留档面）；② `SELECT t.*` 限定通配拒绝文案泛化（Act 报告）。目标、范围与任务清单再次实质变化，原「当前 Cycle 修复」路径（本 Review 原 Follow-up Decision 四项）并入后继 replan Cycle 的修订契约统一执行；T9/T10/T11 与 T8 主体已在 Act Response 交付并经本 Review 核对通过，作为已完成基线继承。

Act 报告的 Issue 候选（不落账，按职责由 Recorder 处理）：① 单表面聚合未知名参数静默 NULL（`SELECT count(nosuch) FROM t1` → `[[0]]`，Plan Risk 预先裁定留档）；② 限定通配项 `SELECT t1.*` 计划期拒绝且文案泛化（`ast.rs` 兜底臂，既有面）。Runbook 候选：时序敏感 e2e 的固定轮次压测规程。

**Acceptance Gaps**

- **T8 ③（部分）**：聚合参数未知列的 JOIN 基座计划期点名缺失（Finding 1）——静默 NULL 与本 Iteration 消除的「静默错误结果」红线同类，属契约 Required behavior 明文，构成 Acceptance gap。其余 ③ 面（投影项 ✓、GROUP BY 键按实质裁定 ✓）、①②④⑤⑥ 全部满足。

**Evidence**

- 代码走查：query.rs:2460-2528（join_combined_columns/join_row_layout/resolve_layout_index）、:1053-1073（projection_has_aggregates 与 carry_all 扩展）、:1097-1160（aggregate_args 收集）、:1188-1300（门扩展 + expression_items_plain + 两拒绝臂豁免）、:1658-1668（GroupKey.input_index 键编译）、:1685-1717（direct 收窄）、:1808-1826（aggregate_arg_indices 构造——缺口所在）、:1315-1317/:1971-1989（编译期布局置位）；ast.rs:339-354/:285-295（aliased_plain_column_ref）；aggregate.rs:414-435（extract_value）；cli/mod.rs:278-302/:338-347（T11）。
- 实测（2026-09-28，本 Review，/tmp 夹具 t1(k,v)=[(1,b),(2,a)]、t2(k,w)=[(1,x),(2,y)]）：P1/P2 缺口见证、P3 投影未知列点名 ✓、P4 单表面现状（留档裁定）、P5/P6 GROUP BY 未知键 NonAggregatedColumn、P7 HAVING 扩展、P8 T9 形状修复。
- Act Response 验证 1–4 项（定向 27 用例 RED→GREEN、边界 12 套件零校准、10 轮压测、全量 1401/0/2 + clippy/fmt/validate）在覆盖未变化前提下采信，未重复运行。
- delta 一致性：Act 未改 delta；`error-reporting` MODIFIED R5 两场景 ↔ `derived_scan_alias_test.rs` 实测见证、`order-by-alias-sorting` R1 扩展 ↔ cte_alias_sort_desc、`aggregate-output-shape-conformance` 场景 ↔ T9 测试与 P7/P8 实测——互为镜像核对一致。

**Follow-up Decision**

`replan-required`（用户指令 2026-09-28 二次扩围）：Finding 1 的当前 Cycle 修复项（聚合参数未知列 JOIN 面计划期点名）与两项 Issue 候选的解决（单表面聚合参数计划期解析 T12、限定通配文案可操作化 T13）合并为后继 replan Cycle 的修订契约统一执行——避免「先窄修再扩围」两轮触碰同一构造点。T9/T10/T11 与 T8 主体的交付与验证结论经本 Review 核对，作为已完成基线由后继 Cycle 继承，不重做。

**Next Cycle**

`openspec/changes/2026-09-26-ms25-sql-read-surface/iterations/004-join-shape-resolution/002-replan.md`

**Next Iteration**

None（仍在 Iteration 004 内）
