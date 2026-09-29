# Iteration 004 / Cycle 000: JOIN 输出形状、名字解析与聚合输出形状收口

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T8, T9
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `join-output-shape-resolution` R1–R4、`aggregate-output-shape-conformance` R1–R2 全部场景；design D9/D10；T8/T9 范围增补用户批准（2026-09-27）；change 级 Gate 2（2026-09-26）覆盖本 Iteration 计划；Iteration 000 交付面（外连接、D2 组合布局 `carry_all_columns`/`where_join_projection`）与 Iteration 003 交付面（T7 表达式别名键装配，query.rs:1772-1787）作为组合场景输入
- Excluded scope: 表达式项×JOIN 解禁（真表达式维持既有拒绝臂）；单表聚合未知名参数行为（同机制相邻面——单表 `column_indices` 布局下参数未知名现状经 `resolve_group_by_item`/`build_having` 各自点名，本 Cycle 只在 JOIN 布局面收紧，单表面随 Cycle 调查裁定顺带或留档，见 Stop when）；有效查询输出形状改变（T9 修复的错配形态除外）；代价模型；MS21-T03（索引条目一致性，独立域）

**Objective**

JOIN 基座的名字解析全面收口：聚合参数在 JOIN 布局上精确解析（静默 NULL 消除）、GROUP BY/投影裸名歧义计划期 `AmbiguousColumn` 点名（末位胜出消除）、JOIN 基座未知列 `ColumnNotFound` 点名（零宽行消除）、纯列引用别名项在 JOIN 基座可达（表头=别名）且 ORDER BY 别名键按输出列真实排序；聚合查询输出行与表头恒为 SELECT 清单形状（键未入选不进输出、键序按 SELECT 序，经既有包装路径承载）；无歧义有效查询与形状安全的直出形态逐字节不变。

**Investigation Facts**

- Current Baseline: Iteration 003 最终 Act Response（Plan Review accepted，2026-09-28 本 Review 采信）——T7 别名键排序落地，全量串行 `cargo test --no-fail-fast -- --test-threads=1` 总数 1376（1373 passed / 1 failed〔`cli_test::test_sigint_during_open_130` 信号时序偶发，单测复核通过〕/ 2 ignored）；基线套件计数（2026-09-28 `--list` 实测）：planner_test 87、nested_loop_join_test 9、outer_join_test 16、group_by_expr_test 12、aggregate_test 19、select_distinct_test 12、scalar_function_test 32、cli_test 111/2i。本 Iteration 目标文件 `src/parser/planner/query.rs` 聚合分支（:1030-1660）、`src/executor/aggregate.rs`、`src/executor/plan.rs` AggregateNode、`src/parser/ast.rs` 自该验证后零变化（T7 改动面仅 ORDER BY 装配段 :1672-1795 与 sort.rs/plan.rs OrderByColumn/plan_format.rs，均已入基线）。
- Current-State Evidence:
  - **JOIN 输出列过滤**（query.rs:420-443，`build_from_clause_with_projection`）：`carry_all_columns || select_star` → 全组合列；否则按 `qualified_columns`（`extract_qualified_columns` 产物）过滤。`OutputColumn { table, column, table_alias, column_index }`（plan.rs:384-393）——`column_index` 为**单源表内索引**；Join/NLJ 执行器 `build_output_row`（join.rs:113-115、nested_loop_join.rs 同型）按 `output_columns` 顺序从左右行取列 → **join 输出行第 i 值 = output_columns[i]**（枚举位即行索引）。
  - **聚合参数静默 NULL 病灶**：`extract_qualified_columns`（ast.rs:175 起）对聚合函数项返回**结果列名**（`count_<col>`/`count_distinct_<col>`/`sum_<col>`…，MS25 D7 命名）而非参数列——参数列进不了 `qualified_columns`，无 WHERE 时 join `output_columns` 滤除参数列；聚合分支 `input_schema = get_plan_output_columns(&plan_with_where)`、`column_indices: HashMap<String, usize>`（query.rs:1459-1464）无参数列条目；`AggregateExecutor::extract_value`（aggregate.rs:396-410）`column_indices.get(func.column())` None → **`Value::Null`** 静默。**实测（2026-09-28 本 Review，t1(k,v)=[(1,b),(2,a)]、t2(k,w)=[(1,x),(2,y)]）：`SELECT count(v) FROM t1 JOIN t2 ON t1.k = t2.k` → `{"columns":["count_v"],"rows":[[0]]}`（应为 2）**。D2 路径（含 WHERE）`carry_all_columns`（query.rs:260-269）全列承载反而幸免——锚点 `outer_join_test.rs:565`（`SELECT ac.k, SUM(ac.amt) … WHERE bc.tag='x' GROUP BY ac.k`）现状正确。
  - **裸名末位胜出病灶**：`column_indices` HashMap 以小写列名为键（query.rs:1460-1464）——JOIN 同名列（t1.k/t2.k）折叠，后表覆盖先表；GROUP BY 裸名键（`resolve_group_by_item`→`column_indices`，query.rs:1472）、聚合参数（extract_value 同 map）、组键编译（query.rs:1510-1521）全部静默解析到末位表。
  - **JOIN 基座零宽行病灶**：纯列引用别名项 `SELECT t1.v AS val FROM t1 JOIN t2 …`——`extract_qualified_columns` 对 ExprWithAlias 产出别名名，过滤无匹配 → `output_columns` 空集；**实测 `{"columns":[],"rows":[[],[]]}` exit 0**（带 `ORDER BY val` 同）。未知列同型零宽行静默。裸名歧义投影：裸名匹配多表同名列时过滤**全保留**（重复列名）。
  - **表达式项×JOIN 拒绝臂**（T8 必须保持）：`mixed_aggregate` 路径 query.rs:1130-1137、纯表达式路径 query.rs:1158-1165——`"Expression projection items are not supported with JOIN queries"`。`alias_reroutable` 门（query.rs:1088-1094）：`!has_aggregates && !building_subquery && matches!(base_plan, Scan(_)) && 无通配`——**仅 Scan 基座**；命中后普通列别名项改标 Expression（:1095-1111）。
  - **T7 表达式键装配与 JOIN 基座的交互（本 Iteration 新打开面）**：装配门 `projection_items.is_some()`（query.rs:1772-1787）——T8 放宽 `alias_reroutable` 至 JOIN 基座后该门在 JOIN 基座**自动生效**：ORDER BY 别名键经 `build_expression(&table_name, expr)` 编译，而该函数对 JOIN 基座回退 `self.tables.get(table_name)` 单表 schema 形状（join_column_layout 已在 FROM 构建后 restore 为 None，query.rs:487/:502/:506）→ **索引基座与 Sort 输入行错位**（单表索引 ≠ join 行索引）。Sort 输入行形状按形态二分：无 WHERE join = 过滤后 output_columns 行；WHERE join（D2 carry）= 全组合行（`sort_projection = where_join_projection` 在排序后才裁剪，query.rs:1707-1715/:1712）。**若不收口将产生新的静默错序面**——契约 T8-R6 显式约束。
  - **聚合输出形状病灶（D10）**：行装配恒 `group_key(GROUP BY 序) ++ aggregates`（`build_output_rows` 分组臂，aggregate.rs:451-459 区域）；`direct = !has_expression_items && keys_all_column && select_keys_first`（query.rs:1538-1552）**不含**「GROUP BY 键 ⊆ SELECT 键项」与「SELECT 键序 = GROUP BY 键序」检查；直出 `node_output_columns = agg_output_columns`（SELECT 序，query.rs:1566-1567）。**实测（2026-09-28）：`SELECT count(*) FROM t1 GROUP BY v` → 表头 `["count_star"]` 行 `[["b",1],["a",1]]`**（键未入选，1 列头 2 值行）；**`SELECT v, k, count(*) FROM t1 GROUP BY k, v` → 表头 `["v","k","count_star"]` 行 `[[2,"a",1],…]]`**（键序置换错位，v 头取到 k 值）。
  - **包装路径已形状一致**：`node_output_columns = group_keys 序 ++ agg_names`（query.rs:1569-1573）；`wrap_projection`（query.rs:1578-1630）Column 角色→组键位置、Expression 角色→expr_item 组键位置、Aggregate 角色→`key_count + agg_seen`，列名 = SELECT 名；`get_plan_output_columns` Aggregate 臂 = `node.output_columns`、Projection 臂 = `node.columns`——表头通路已正确，T9 只改 `direct` 判定。
  - **HAVING 绑定**：`build_having(having_expr, &node_output_columns)`（query.rs:1632-1637）——直出形态缺未入选键 → 现状绑定失败；包装形态含键。既有 HAVING 测试全在包装或键对齐形态（aggregate_test / group_by_expr_test:398-440 / scalar_function_test:468），无断言锁定失败面。
  - **校准白名单（本 Cycle 调查定稿，全库 GROUP BY 断言面清扫 2026-09-28）**：① `tests/select_distinct_test.rs::distinct_aggregate_output`（RTM 已列——`SELECT count(*) FROM t GROUP BY g` 错配形状，行宽 2→1 后断言需校准）；② **`tests/group_by_expr_test.rs::groupby_order_interleave_keeps_direct_path`（本 Review 调查新增）**——MS13 时代锁定「GROUP BY 序 ≠ SELECT 键序直出错位」行为（doc 明言「既有错位行为，design R2 记载」），T9 后该形态转包装路径、行重排到 SELECT 序，断言与 doc 必须按新语义校准（测试名可保留或按新语义改名，doc 更新必须写明 T9 来源）。其余面零校准：group_by_expr_test 余下用例（键对齐直出 :305、Aggregate 后置 Column 走包装 :184、表达式键包装 :207/:255/:408/:432、GROUP BY ALL :462、拒绝面 :283/:330-区域外/:372/:389）、aggregate_test 19、planner_test:1879（对齐）、subquery_test:310（对齐）、outer_join_test:565（对齐 D2 锚）、expression_e2e_test（无 GROUP BY 形态命中）。aggregate×JOIN 形态既有断言仅 outer_join_test:565 一处（无锁定破损形态的测试）。
  - **错误面载体**：`PlanError::AmbiguousColumn(String)` 已存在（src/parser/error.rs:29，MS09 join_column_layout 臂消费）；`PlanError::ColumnNotFound(String)` 既有。`AggregateFunc`（aggregate.rs:16-30+）：`CountStar` 无参数，`Count/Sum/Avg/Min/Max/CountDistinct(String)`（+ D7 group_concat）携带参数列名。`AggregateNode` 构造点全集：query.rs:1639-1647（计划）、pipeline.rs（执行器装配）、plan.rs:452 定义——tests/ 无 AggregateNode 字面量构造（grep 证实），加性字段机械补面仅限 src 两处。plan_format.rs Aggregate 臂（:33、:198-201）按字段访问渲染（非穷尽解构），EXPLAIN 文本不含新字段 → 零变化。
  - **Hash join 条件面（不触碰）**：`extract_join_conditions`/`is_pure_equi_join_on` 结构探测与条件编译发生在 FROM 构建期（NLJ 布局 save/restore 配对内），与本 Cycle 的输出布局解析正交——ON 条件解析零变化。
  - **测试入口**：`tests/join_output_resolution_test.rs`（新；形态先例 `tests/outer_join_test.rs`：TempDir + `Database::open` + 单库双表夹具——跨库 ATTACH 未实现，JOIN 夹具须同库两表）、`tests/aggregate_output_shape_test.rs`（新；先例 `tests/group_by_expr_test.rs` 的 `run_cli`/`parse_json` 夹具或 lib 直连同型）、planner_test 追加形状/拒绝矩阵。

**Implementation Guidance**

TDD：两任务各自先立测试文件取 RED（T8 三面静默缺陷 + T9 两类错配均为现状可复现 RED），再实现 GREEN。建议 T8 先行（其 join 布局解析 helper 是 T9 无直接依赖但同文件聚合分支的先行收敛，避免 query.rs 同段两轮改序）——两者亦可并行实现（T9 只动 `direct` 判定与白名单，与 T8 的解析面无交集），顺序不作实质约束。

T8 落位建议（契约 T8-R1..R6 的机制方向，等价机制可）：

1. 聚合×JOIN 全列承载（R1）：`has_aggregates` 且 JOIN 基座时 join `output_columns` 扩为全组合列——复用 `carry_all_columns` 通道（把置位条件从「含 WHERE」扩为「含 WHERE ∨ has_aggregates」需检查该 bool 的其他消费点，或在 join 构造点按 `has_aggregates` 独立判定，等价机制均可）；聚合输入形状 = 全组合布局。
2. 布局感知解析（R2）：聚合分支的 `column_indices` 消费面改为保序布局结构——`output_columns` 的 `Vec<OutputColumn>` 天然携带 (table_alias, column, 枚举位)；解析 helper：限定名 (table, column) 精确唯一匹配 → 行位置；裸名唯一命中 → 行位置；裸名多命中 → `PlanError::AmbiguousColumn` 点名；零命中 → `PlanError::ColumnNotFound`。消费面：`resolve_group_by_item` 列名键、组键编译（query.rs:1510-1521）、聚合参数解析（新）。
3. `AggregateNode` 加性 `aggregate_arg_indices: Vec<Option<usize>>`（与 `aggregates` 平行，`CountStar` → None；group_concat 参数列同解析）：计划构造点解析填充；`extract_value` 改消费索引（签名加 `&[Option<usize>]`，名字查表退役或保留为回退——二选一非实质，但 JOIN 面单表回退禁止复活静默 NULL：解析失败的参数在**计划期**即 `ColumnNotFound` 点名，运行期不再有 None→Null 臂）。机械补字段：pipeline.rs 装配点、plan_format 若有穷尽解构（现状无）。
4. 非聚合 JOIN 投影严格解析（R3）：JOIN 基座纯列清单逐项经同一 helper 解析——未知列 `ColumnNotFound`、裸名多命中 `AmbiguousColumn`（零宽行与重复列静默面消除）。
5. 纯列引用别名项可达（R3）：`alias_reroutable` 基座门放宽至 `Join(_) | NestedLoopJoin(_)`（含外连接 NLJ；子查询/通配/聚合条件不变）；改标后的 Expression 项编译**不得**走单表 schema 回退——按 join 行形状解析（无 WHERE = 过滤后 output_columns 位；有 WHERE = 全组合位，与 `where_join_projection` 的 SELECT 位置机制对齐）；「表达式项×JOIN」两拒绝臂（query.rs:1130-1137/:1158-1165）对真表达式保持（`is_plain_column_expr` 判定先于改标，天然豁免）。
6. ORDER BY 别名键×JOIN 基座（R6）：T7 装配（query.rs:1772-1787）在 JOIN 基座生效后，键表达式解析必须与 Sort 输入行形状同基座——无 WHERE join 按过滤后布局、WHERE join 按全组合布局（同 helper）；禁止落入 `build_expression` 单表回退产生静默错序。见证：`join_output_resolution_test.rs` 增非单调可区分的别名排序组合用例（如 `SELECT t1.v AS val FROM t1 JOIN t2 ON t1.k = t2.k ORDER BY val` 用使 v 序 ≠ 行序的数据），映射 delta `join-output-shape-resolution` R3「别名列可达」场景族。

T9 落位建议：`direct` 判定（query.rs:1552）追加两条件——每个 group key 被某 SELECT `Column` 角色引用（键集合 ⊆ SELECT 键项）∧ SELECT 键项出现序 = GROUP BY 键序（既有 `select_keys_first`「键前聚合后」保留）。判定结果只切换直出/包装路由（包装机制零修改）；`build_output_rows` 行装配不动（D10 否决「执行器去键」替代方案的两消费面理由维持）。

**Task Contracts**

### T8: JOIN 输出形状与名字解析收口（`join-output-shape-resolution` R1–R4）

- Requirement/Scenario: R1 完整形状解析（agg_arg_outside_select_reachable RED 静默 NULL→GREEN / qualified_group_key_precise 全形状化保持）、R2 歧义拒绝（bare_group_by_ambiguous_rejected RED 末位胜出→GREEN / bare_agg_arg_ambiguous_rejected / bare_projection_ambiguous_rejected）、R3 严格解析与别名列（unknown_column_named_rejected RED 零宽行→GREEN / aliased_plain_column_reachable RED 零宽行→GREEN / expression_item_join_still_rejected 保持）、R4 零回归（既有 join/aggregate 套件零校准 + 全量串行零回归）
- Depends on: Iteration 000（JOIN 基座与 D2 组合布局）
- Targets: `src/parser/planner/query.rs`（join 构造点全列承载、聚合分支解析面 :1459-1532、`alias_reroutable` 门 :1088-1111、ORDER BY 别名装配 :1772-1787）、`src/executor/plan.rs::AggregateNode`（加性 `aggregate_arg_indices`）、`src/executor/aggregate.rs::extract_value`（索引消费）、`src/pipeline.rs`（Aggregate 装配点机械补字段）、`src/parser/ast.rs::extract_qualified_columns`（如需区分参数列提取——加性扩展，既有返回值消费点兼容）、`tests/join_output_resolution_test.rs`（新）
- Current behavior: JOIN 无 WHERE 聚合参数不在 SELECT 清单时 `extract_value` 静默 NULL（实测 count(v)=0）；同名列 `column_indices` 折叠末位胜出；JOIN 基座未知列/纯列别名项零宽行静默（实测 `{"columns":[],"rows":[[],[]]}`）；ORDER BY 别名键在 JOIN 基座（T8 放宽后）若无本契约将落单表回退错序
- Required behavior: ① R1 聚合×JOIN 全列承载——无 WHERE 时聚合输入 = 全组合布局，参数列 plan 期解析为索引，`count(v)` 返回真实计数；限定名 GROUP BY 键精确指向所指表列（t1.k ≠ t2.k）；② R2 裸名多命中三面（GROUP BY 键 / 聚合参数 / 非聚合投影项）计划期 `AmbiguousColumn` 点名（复用既有错误变体，文案含列名）；③ R3 JOIN 基座未知列（投影项/聚合参数/GROUP BY 键）`ColumnNotFound` 计划期点名——零宽行消除；纯列引用别名项可达：表头 = 别名、行值 = 所指列，非聚合无歧义 JOIN 查询端到端正确；任意表达式项×JOIN 维持既有显式拒绝（两拒绝臂文案与触发面零变化）；④ R6 ORDER BY 别名键在 JOIN 基座按输出列真实排序（索引与 Sort 输入行形状同基座：无 WHERE = 过滤后布局 / 有 WHERE = 全组合布局），无歧义数据上与 DISTINCT/DESC 组合正确；⑤ `aggregate_arg_indices` 为 plan 期解析——解析失败在计划期点名（`ColumnNotFound`/`AmbiguousColumn`），运行期 extract_value 无静默 NULL 臂复活；⑥ R4 无歧义有效查询（含 Iteration 000–003 全部交付面）计划形状、结果与表头逐字节不变——`outer_join_test.rs:565` D2 锚点零修改通过
- Preserve: 单表聚合路径行为逐字节不变（column_indices 单表面语义不变——含未知名参数的既有点名/静默现状，本契约不收紧单表面）；Hash join/NLJ 的 ON 条件解析与计划路由（`is_pure_equi_join_on`/`extract_join_conditions`）零变化；`get_plan_output_columns` 对有效查询输出不变（聚合查询 join 输入形状变化不外溢——聚合输出形状由自身名单决定）；EXPLAIN 文本输出零变化；plan cache 键语义不变；M17/M19
- Forbidden: 不解禁真表达式项×JOIN；不改变单表聚合的错误面；不为运行期索引越界新增静默 NULL 回退；不触碰 ON 条件编译路径；白名单外不修改既有测试断言（本任务白名单现为空集——实施中发现需校准即返回 Plan）
- Test witness: `tests/join_output_resolution_test.rs` 先行——`agg_arg_outside_select_reachable`/`bare_group_by_ambiguous_rejected`/`unknown_column_named_rejected`/`aliased_plain_column_reachable` 现状 RED（静默 NULL / 末位胜出 / 零宽行，均本 Review 实测可复现），实现后 GREEN；`qualified_group_key_precise`/`expression_item_join_still_rejected` 既有行为见证
- GREEN condition: 新测试全绿 + `planner_test`(87)/`nested_loop_join_test`(9)/`outer_join_test`(16)/`subquery_test` 零校准通过 + 全量串行 0 failures
- Verification: ①定向 `cargo test --test join_output_resolution_test --test outer_join_test --test nested_loop_join_test`（退出码 0）；②受影响边界 `cargo test --test planner_test --test cli_test --test select_distinct_test --test set_operations_test --test group_by_expr_test --test aggregate_test`（退出码 0）；③全量 `cargo test -- --test-threads=1`（M19，退出码 0）；④`cargo clippy --all-targets` + `cargo fmt --check`（退出码 0）
- Stop when: 全列承载或布局解析牵动 `get_plan_output_columns` 有效查询输出变化（违反 Preserve④）——返回 Plan；别名项 JOIN 基座编译需超出布局 helper 的单表回退以外新机制（R6 索引基座无法闭合）——返回 Plan；实施中发现本白名单外需校准的既有测试——返回 Plan；单表面收紧需求浮出（未知名参数面与本契约边界冲突）——单表面顺带或留档二选一记录进 Act Response，实质冲突返回 Plan

### T9: 聚合输出形状一致性收口（`aggregate-output-shape-conformance` R1–R2）

- Requirement/Scenario: R1 形状一致（agg_only_group_by_key_not_selected RED 表头 1 列行 2 值→GREEN / select_key_order_differs_from_group_by RED 置换错位→GREEN）、R2 绑定与零回归（existing_keyed_forms_unchanged / having_unselected_key_bindable 可达性扩展 / distinct_aggregate_output + groupby_order_interleave_keeps_direct_path 白名单校准 / 既有聚合套件零校准）
- Depends on: None（与 T8 同 Iteration，实现顺序不约束；测试面独立）
- Targets: `src/parser/planner/query.rs::build_query` 聚合分支 `direct` 判定（:1538-1552，仅加两条件）、`tests/aggregate_output_shape_test.rs`（新）、白名单两处校准（`tests/select_distinct_test.rs::distinct_aggregate_output`、`tests/group_by_expr_test.rs::groupby_order_interleave_keeps_direct_path`）
- Current behavior: 直出判定不含键子集与键序检查——`SELECT count(*) FROM t GROUP BY v` 表头 1 列而行 [键,聚合] 2 值（实测）；`SELECT v, k, count(*) FROM t GROUP BY k, v` 行按 GROUP BY 序、表头按 SELECT 序置换错位（实测）；HAVING 引用未入选键在直出形态绑定失败
- Required behavior: ① 形状不安全形态（键未入选 / 键序不同）改走既有条件包装路径——输出行与表头恒为 SELECT 清单形状，未入选分组键不进输出、仅供 HAVING 绑定（SQLite/PostgreSQL 对齐）；② 形状安全的直出形态（键 ⊆ SELECT、键序一致、键前聚合后、无表达式项）路径与计划形状逐字节不变；③ HAVING 经 `node_output_columns` 绑定零变化 + 行为扩展：HAVING 引用未入选分组键由绑定失败转为可达（包装路径既有能力，无既有断言锁定失败面——调查已核实）；④ ORDER BY 聚合输出名绑定零变化；⑤ 白名单两处按新语义校准（行形状/排序键断言 + doc 注明 T9 来源）
- Preserve: `build_output_rows` 行装配（group_key ++ aggregates）逐字节不动；包装路径 `wrap_projection`/`node_output_columns` 机制零修改；既有对齐直出形态（group_by_expr_test :305 等）与表达式键包装形态（:207/:255/:408/:432）零校准通过；`get_plan_output_columns` Aggregate/Projection 臂零变化
- Forbidden: 不改执行器行装配；不实现「表头补键」输出未选列；不扩大直出判定（只收窄）；白名单（两处）之外不修改既有测试断言
- Test witness: `tests/aggregate_output_shape_test.rs` 先行——两类错配形态现状 RED（本 Review 实测可复现），实现后 GREEN；`existing_keyed_forms_unchanged` 对齐形态现状 GREEN 锁定
- GREEN condition: 新测试全绿 + 两处白名单校准 GREEN + `group_by_expr_test`(12 除白名单 1 处)/`aggregate_test`(19)/`select_distinct_test`(12 除白名单 1 处)/`expression_e2e_test` 零校准 + 全量串行 0 failures
- Verification: ①定向 `cargo test --test aggregate_output_shape_test --test select_distinct_test --test group_by_expr_test`（退出码 0）；②受影响边界 `cargo test --test aggregate_test --test planner_test --test cli_test --test expression_e2e_test`（退出码 0）；③全量与静态检查同 T8 ④（可合并执行）
- Stop when: 收窄判定后仍有形状错配形态逃逸直出（枚举遗漏）——返回 Plan；白名单两处之外的既有断言需校准——返回 Plan；HAVING 可达性扩展撞上未识别的失败面锁定——返回 Plan

**Invariants**

- I034 纪律：输出行形状与表头恒一致（T9 的目标即恢复该不变量在聚合查询的普适性；T8 的零宽行消除同理）。
- 布局解析的索引基座唯一性：任何表达式（投影项 / 组键 / 聚合参数 / ORDER BY 别名键）的行索引必须与其求值所在执行器节点的输入行形状同源——禁止单表 schema 回退落在 join 行节点上。
- 有效查询零回归：无歧义、无未知列、形状安全的既有查询计划与结果逐字节不变（Iteration 000–003 全部交付面为锚）。
- 错误面零副作用：全部新增拒绝（AmbiguousColumn/ColumnNotFound）发生在计划期，任何写入路径无关（纯查询面）。

**Non-goals**

- 真表达式项×JOIN 解禁；单表聚合未知名参数面收紧（留档裁定）；递归 CTE/窗口函数；代价模型与 Join 重排；`INTERSECT/EXCEPT` 侧臂内的 JOIN 解析深化（侧臂经 `building_subquery` 语义构建，现状拒绝/抑制臂内维持）；MS21-T03。

**Acceptance**

`join-output-shape-resolution` R1–R4 与 `aggregate-output-shape-conformance` R1–R2 全部场景可观察满足；两类静默缺陷（JOIN 聚合参数 NULL、直出错配）与零宽行面有 RED→GREEN 见证；白名单两处校准与实现一致（镜像核对）；无歧义有效查询与形状安全直出逐字节不变；全量串行零回归。Scenario→Task→Test 链路：RTM `join-output-shape-resolution` 四行 + `aggregate-output-shape-conformance` 两行（change tasks.md）。

**Verification**

- T8：`tests/join_output_resolution_test.rs` 全场景（含 ORDER BY 别名×JOIN 组合见证）+ outer_join/nested_loop_join/planner/cli 边界 + 全量串行。
- T9：`tests/aggregate_output_shape_test.rs` 全场景 + 两处白名单校准 + group_by_expr/aggregate/select_distinct/expression_e2e 边界 + 全量串行（与 T8 合并一次全量亦可）。
- 判定均为测试自身断言 + cargo 退出码；无身份型证据工程。

**Gate 2 Readiness**

PASS——无 Missing requirement（RTM 全 Covered，tasks.md 四+二行）；调查完整（三面静默缺陷与两类错配的病灶机制 file:line 实证 + 本 Review 四组 CLI 实测复现 + 校准白名单全库清扫定稿 + AggregateNode 构造点/错误变体/plan_format 消费面核实，D9 预留三个调查点已闭合：alias_reroutable×拒绝臂交互 = `is_plain_column_expr` 先于改标天然豁免 + R6 显式约束、Hash join 条件解析 = FROM 构建期正交不触碰、ORDER BY 别名×聚合组合 = T7 门在聚合基座不生效〔projection_items None〕无交互）；设计闭合（D9/D10 方案与替代方案否决理由明确）；任务契约自包含（Act 无需回读外部资料）；验证直接判定（测试断言 + 退出码，无身份型证据工程）；无实质未知项留给 Act。change 级 Gate 2（2026-09-26 用户批准）与本 Iteration 增补批准（2026-09-27 T8/T9）覆盖本计划。

**Persisted Evidence**

- Mode: none

Act Response 承载全部验证结果；无一次性环境、无不可复现结论。

**Risks and Notes**

- 全列承载改变聚合×JOIN（无 WHERE）的 AggregateNode 输入行形状（过滤 → 全组合）——该形态现状输出即错误（静默 NULL），形状变化不构成有效查询回归；但 GROUP BY 裸名键在该形态下的解析结果会随布局修正而变（末位胜出 → 精确），属缺陷修复面而非回归。
- `alias_reroutable` 放宽后 `has_expression_items` 在 JOIN 基座可能置位——`mixed_aggregate`/纯表达式两拒绝臂的触发顺序在改标之后（MS19 T4b 既有机制），真表达式项×JOIN 拒绝文案不变；`projection_items` 通路在 JOIN 基座首次启用，其 `get_plan_output_columns` Projection 臂表头行为由 R3 场景见证。
- T9 的 `groupby_order_interleave_keeps_direct_path` 校准：测试名可保留（doc 更新承载语义变化）或按新语义改名——非实质，Act 裁定；doc 必须写明「T9 形状一致性收口后转包装路径」来源。
- `test_sigint_during_open_130` 偶发（Iteration 003 全量首跑失败、复核通过）：本 Cycle 全量若再遇该用例失败，按「失败后判定是否偶发错误的一次复核」处理并记录，不作为本 Iteration 阻塞。
- 单表聚合未知名参数面（如 `SELECT count(nosuch) FROM t GROUP BY g`）：现状经 `column_indices` 单表面查无 → extract_value 静默 NULL——与本 Cycle 的「运行期无静默 NULL 臂」约束存在张力。契约边界：本 Cycle 只对 JOIN 布局面收紧（`aggregate_arg_indices` 解析在 JOIN 基座 plan 期点名）；单表面是否同轮收紧由 Act 按实现自然落点裁定（若 helper 统一覆盖单表面则单表未知名参数同步转 plan 期 `ColumnNotFound`——行为收紧需在 Act Response 记录并补 witness；若保持单表面现状则留档 Issue 候选）。两条路径均不阻塞验收，实质冲突时返回 Plan。

## Act Response

- Status: pending

## Plan Review

- Review Result: replan-required

**Findings**

本 Cycle 的 Plan Context（ready）在 Act 启动前由用户指令重定向（2026-09-28）：把 Plan Review（Iteration 003）报告的两项 Issue 候选并入本 Iteration 解决——① DerivedScan 基座（CTE / FROM 子查询）普通列别名不应用（MS19 T4b 既有静默面）；② `test_sigint_during_open_130` 信号时序偶发（测试基建 + 产品注册时序窗口）。目标、范围与任务清单发生实质变化，按公共规则 › Iteration 与 Cycle 线程 与 references/iteration-planning.md › Replan Cycle 处理：更新 change 的 tasks、design（新增 D11）与 delta specs，创建本目录后继 replan Cycle 承载修订后的执行契约。

本 Cycle 的 T8/T9 调查与契约材料全部有效，由 replan Cycle 完整继承（自包含重述，不要求 Act 回读本 Cycle）。

**Acceptance Gaps**

None——非缺陷驱动的范围变更，Act 未启动、无实现偏差。

**Evidence**

用户指令原话：「重新规划iter4，把issue提到的问题也规划解决」（2026-09-28，本会话）。

**Follow-up Decision**

`replan-required`：范围扩大（新增 T10 DerivedScan 别名应用收口、T11 信号注册时序硬化），需要修订后的 Task Contract、delta specs 与 Gate 2——创建后继 replan Cycle，本 Cycle 不执行。

**Iteration Plan Update**

change `tasks.md` 已同步更新：Iteration 004 任务集 T8/T9 → T8/T9/T10/T11；RTM 增补 T10 相关行（`error-reporting` MODIFIED R5 + `order-by-alias-sorting` R1 场景扩展）；design.md 新增 D11；delta specs 增补（error-reporting MODIFIED R5、order-by-alias-sorting 场景扩展）。

**Next Cycle**

`openspec/changes/2026-09-26-ms25-sql-read-surface/iterations/004-join-shape-resolution/001-replan.md`

**Next Iteration**

None（仍在 Iteration 004 内）
