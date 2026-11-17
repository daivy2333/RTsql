# Iteration 001 / Cycle 000: 按估计更换叶序

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `specs/query-cost-and-join-order/spec.md` 的「估计只更换合格查询的叶序」，`specs/explain-plan-output/spec.md` 里因此多出来的 `JoinOrder` 行，以及 `specs/sql-parameter-binding/spec.md` 里统计替换后预编译语句重计划的那一场景。000 已交付的统计保存和 `choose_left_deep_order` 直接使用。
- Excluded scope: T4 的全量。不改代价公式、权重、8 表上限和溢出宽度。不交换单个连接的左右输入。不改 `sort_unstable_by`。

**Objective**

合格查询在每张基表都有可用统计时，按 000 的代价函数更换左深叶序。EXPLAIN 在首行 `EXPLAIN` 之后多一行 `JoinOrder`。最外层交给 WHERE、ORDER BY 和投影的列序仍与书写装配相同。不合格或没有可用统计时，节点行与书写装配逐字节一致，并且不出现 `JoinOrder`。

**Investigation Facts**

- Starting point: 000 的 Act Response 在 `iterations/000-session-statistics/000-initial.md`，HEAD `9139878df480a3da57500d245cb9591964b723e5`。`ANALYZE TABLE` 已能保存会话统计。`choose_left_deep_order` 对规格里的星型、排序合并、平手、9 表和溢出输入返回规定决定。`build_from_clause_with_projection` 仍按书写顺序装配。变更前的定向结果沿用该 Act Response：`table_statistics_test` 12 passed，`table_stats` 过滤 14 passed，都带 `--test-threads=1`。那是书写叶序仍在时的起点。实现改到连接装配之后，GREEN 再跑这两条。本 Cycle 不跑全量。
- Current-State Evidence:
  - 书写装配在 `src/parser/planner/query.rs` 的 `build_query`。它调用 `build_from_clause_with_projection`。`from` 只有一项时，`from[0].joins` 收成左深树，右子节点是下一张表。有 `ORDER BY`、WHERE 或聚合投影时，第三参为真，连接节点携带全部列。返回的 `combined_qualifiers` 与最外层 `output_columns` 同序，后面的 WHERE、ORDER BY 和投影按这个顺序解析。
  - 叶是 `PhysicalPlan::Scan` 时才是基表扫描。派生表、CTE 引用和视图展开都是 `DerivedScan`。视图展开和派生表会把 `building_subquery` 与 `building_from_derived` 置真。标量子查询只把 `building_subquery` 置真。CTE 定义在 `consume_with` 里调用 `build_query`，当时 `pending_cte_names` 非空，并且不把 `building_subquery` 置真。
  - 内连接且 `is_pure_equi_join_on` 为真时走 `Join` 或 `SortMergeJoin`。`integer_pk_base_scan_join` 仅在左右都是基表 `Scan`、条件非空、每条等值键都是两侧已登记的整数主键时返回真。外连接走 `NestedLoopJoin`。
  - `query.limit`、`query.offset`、`query.fetch` 和 `query.limit_by` 是 LIMIT、OFFSET、FETCH、LIMIT BY 的语法字段。
  - EXPLAIN 在 `PlanBuilder::build_plan` 里对 `Statement::Query` 调用 `build_query`，再包 `ExplainNode`。`ExplainExecutor::new` 对内层计划调用 `render_plan_tree`。该函数首行固定是 `EXPLAIN`，随后每个节点一行，缩进为深度乘两个空格。`describe` 对扫描的文本是 `Scan <表名> (<列>)`。`ExplainExecutor` 的现有测试期望第二行就是根节点，不能因为没有决定而多出一行。
  - 直接执行和 `plan_prepared` 都进入 `plan_stage_impl`。查询和 EXPLAIN 落在它最后的 `_` 臂，那里新建 `PlanBuilder`。`plan_prepared` 由 `PreparedStatement::resolve_plan` 在 `schema_version` 不一致时调用。
  - 统计在 `Database::session_stats`。`snapshot` 按基表名返回 `ColumnStat`，含 `name`、`row_count`、`ndv`。成功的 `ANALYZE` 已经把 `schema_version` 加一。`CREATE INDEX` 等既有 DDL 已经调用 `bump_schema_version`，这一步会清空全部统计。关闭后再打开是新的空快照。
- Code and Critical Path: 查询计划从 `plan_stage_impl` 的 `_` 臂进入 `build_plan` / `build_query`。书写装配成功之后，才允许读取快照并调用 `choose_left_deep_order`。决定写在 `ExplainNode` 上，由 `ExplainExecutor` 插到渲染结果的第二行。执行仍走原来的连接节点。

**Implementation Guidance**

先写会失败的星型 EXPLAIN 测试，看到第一条扫描仍是 `d1`，再改计划器。

统计快照在 `plan_stage_impl` 创建 `PlanBuilder` 之后、`build_plan` 之前交给计划器。`ANALYZE` 自己的计划臂不用这份快照做重排。

`build_from_clause_with_projection` 先完成现在的书写装配。它返回错误时原样返回，不读统计，不写 `JoinOrder`。成功之后只在下面每一条都成立时继续：

- `building_subquery` 为假，`building_from_derived` 为假，并且 `pending_cte_names` 为空。
- `FROM` 只有一项，连接步至少两步。
- 每一张叶都是主库基表 `Scan`，表名不在 `attached_tables`。
- 每一步都是内连接，并且 `is_pure_equi_join_on` 为真。
- `query.order_by` 非空。
- `query.limit`、`query.offset`、`query.fetch` 都是 `None`，`query.limit_by` 为空。
- 链上每一张基表都有可用统计。可用指快照里有这张基表，且统计列名的顺序与计划时该表的列名顺序相同。有一列对不上，这张表就没有可用统计。行数取 `row_count`。

叶的限定符用声明限定符。统计的查找键是基表名。代价函数的 `sort_merge_first` 只反映这两个基表扫描作为第一步时 `integer_pk_base_scan_join` 是否为真。不要另写 8 表上限或溢出判断。

`LeftDeepChoice::Insufficient` 保持书写树，EXPLAIN 行是 `JoinOrder written`。`Estimated` 且叶序与书写顺序相同，也保持书写树，行是 `JoinOrder estimated`。叶序不同时重造左深树：每一步的右子节点是一张基表扫描；内层行按选中叶序拼接各叶的全部列；最外层 `output_columns` 与返回给 `build_query` 的限定符序仍是书写装配的那一份。`output_sides` 和 `column_index` 指向重造后实际持有该列的子行。每个新连接节点形成后调用现有的 `integer_pk_base_scan_join`，用它的结果选择 `SortMergeJoin` 或 `Join`。不要按行数交换左右。

`JoinOrder` 行只放在 `ExplainNode` 上。`ExplainExecutor` 在 `render_plan_tree` 的结果里，把 `  JoinOrder estimated` 或 `  JoinOrder written` 插到首行 `EXPLAIN` 之后。没有这个标签时，输出与现在的 `render_plan_tree` 逐字节相同。`render_plan_tree` 本身不增加行，避免没有决定的计划改变现有渲染测试。

星型夹具不要声明主键。`d1.id = fact.d1` 不是两侧整数主键，第一步不走排序合并，代价函数才会选出 `fact, d1, d2`。

**Task Definitions**

### T3: 合格查询更换左深叶序

- Requirement/Scenario: 「估计只更换合格查询的叶序」的全部场景，EXPLAIN 因此出现的 `JoinOrder` 行，以及「统计替换后重计划」。
- Depends on: Iteration 000 accepted
- Targets: `src/parser/planner/query.rs` 的 `build_query` 与 `build_from_clause_with_projection`；`src/parser/planner/mod.rs` 的 `Explain` 臂和 `ExplainNode` 的构造；`src/executor/plan.rs` 的 `ExplainNode`；`src/executor/explain.rs` 的 `ExplainExecutor::new`；`src/pipeline.rs` 的 `plan_stage_impl`；`tests/join_reorder_test.rs`。
- Current behavior: 统计可以保存，连接仍按书写顺序装配。EXPLAIN 没有 `JoinOrder` 行。两表及更多表的第一条 `Scan ` 是书写顺序的左表。
- Required behavior:
  - 三张表都统计之后，`SELECT fact.id, d1.id, d2.id FROM d1 JOIN fact ON d1.id = fact.d1 JOIN d2 ON fact.d2 = d2.id ORDER BY fact.id, d1.id, d2.id` 的 EXPLAIN 有一行，去掉行首空白后等于 `JoinOrder estimated`。去掉行首空白后，第一条以 `Scan ` 开头的行以 `Scan fact` 开头。数据是 `d1(id INT)` 与 `d2(id INT)` 各有 1 和 2，`fact(id INT, d1 INT, d2 INT)` 有 100 行，`fact.id` 为 1 到 100，`fact.d1` 与 `fact.d2` 都是 `(id - 1) % 2 + 1`。结果行与同一数据上从未统计的会话相同。
  - 同一数据和连接上，`SELECT *` 加 `ORDER BY fact.id` 时，表头列序与未统计时相同，行也相同。
  - 加上 `WHERE fact.id = 1` 和 `ORDER BY fact.id` 时，只返回 `fact.id = 1` 的那一行，且与未统计时相同。
  - `SELECT fact.d1, COUNT(*) ... GROUP BY fact.d1 ORDER BY fact.d1` 的分组和计数与未统计时相同。
  - 去掉 ORDER BY、只连接 `d1` 与 `fact`、改成 `d1 LEFT JOIN fact ... JOIN d2` 并带 `ORDER BY fact.id`，或星型内连接加上 `ORDER BY fact.id LIMIT 1` 时，没有 `JoinOrder` 行。前三种的第一条扫描仍是 `d1`。外连接的计划含有 `NestedLoopJoin`。带 LIMIT 的那一行与未统计时相同。
  - `a`、`b`、`c` 各 10 行，连接列的不同值个数都是 10，且没有主键使第一步变成排序合并。`a JOIN b JOIN c` 加 `ORDER BY` 时，EXPLAIN 含有 `JoinOrder estimated`，第一条扫描仍是 `a`。
  - 九张已统计基表的等值链，带 `ORDER BY` 时，EXPLAIN 含有 `JoinOrder written`，第一条扫描仍是书写顺序的左表。
  - 星型查询先 `prepare`，再统计三张表，然后执行该预编译语句，并直接执行同一文本。两次结果行相同。`schema_version` 比这三次 `ANALYZE` 之前大。
  - 星型查询已经出现 `JoinOrder estimated` 之后，成功 `CREATE INDEX`，再 EXPLAIN 时没有 `JoinOrder` 行，第一条扫描回到书写顺序的左表。
  - 关闭再打开之后，同一 EXPLAIN 没有 `JoinOrder` 行。
- Preserve: 没有可用统计时，既有 EXPLAIN 节点行逐字节不变。000 的 `analyze_keeps_written_join_order_in_explain` 仍通过。哈希连接仍构建右输入。书写装配返回的错误文本不变。
- Forbidden: 改 `sort_unstable_by`；把两表连接对调；重排外连接；按行数交换任何一个连接的左右输入；在计划器里另写 8 表上限或溢出判断；修改 `openspec/specs/` 或 `.agents/`；跑全量或无过滤的 `cargo test`。
- First test: 在 `tests/join_reorder_test.rs` 写 `star_join_explain_puts_fact_first`。实现之前运行 `cargo test --offline --test join_reorder_test star_join_explain_puts_fact_first -- --test-threads=1`。预期失败，因为第一条扫描仍是 `d1`。同一文件写上上面其余观察。本计划没有运行这条命令。
- GREEN condition: 下面三条依次执行，退出码都是 0，failed 为 0。
  1. `cargo test --offline --test join_reorder_test -- --test-threads=1`
  2. `cargo test --offline --test table_statistics_test -- --test-threads=1`
  3. `cargo test --offline --lib table_stats -- --test-threads=1`
- Verification: 每条观察都从 `Database::execute_sql` 或 `Database::prepare` 进入。EXPLAIN 看结果行里的计划文本。查询结果与同一进程里从未统计的另一个 `Database` 比较，或者与统计前保存的行比较。预编译比较 `PreparedStatement::execute` 与随后的直接执行。
- Stop when: 最外层列序无法与书写装配一致，或为了结果一致必须交换单个连接的左右输入，或必须把排序改成稳定排序。停止并回到计划。不要改结果语义。

**Invariants**

- 未收集统计时，既有语句的结果和 EXPLAIN 节点行不变。
- `EXPLAIN` 不执行被观察的查询，不读取目标表的行。
- 任何测试命令都带 `--test-threads=1`，目标分开、依次执行。本 Cycle 不跑全量。

**Non-goals**

不改 000 的计数和代价函数。不持久化统计。不交换两表连接的构建侧。不跑 M19 全量。

**Acceptance**

- 星型查询在统计之后把 `fact` 放在左叶，`JoinOrder estimated` 出现，结果与未统计时相同。
- `SELECT *`、WHERE、`GROUP BY` 的列序或分组与未统计时一致。
- 无 ORDER BY、两表、外连接和 LIMIT 保持书写叶序，且没有 `JoinOrder` 行。
- 九张表是 `JoinOrder written` 且叶序不变。最小代价等于书写顺序时是 `JoinOrder estimated` 且叶序不变。
- 预编译语句在统计之后与直接执行的结果相同，版本戳已增加。
- `CREATE INDEX` 或关闭再打开之后，不再出现 `JoinOrder` 行。

**Verification**

先行命令必须先失败。GREEN 的三条命令分开执行，都带 `--test-threads=1`。不使用无过滤的 `cargo test`。全量留在 Iteration 002，本轮返工也不得加入全量。

**Gate 2 Readiness**

PASS。无 BLOCKED。无 SKIPPED。

**Persisted Evidence**

- Mode: none

定向测试的命令和结果写入 Act Response 即可。

**Risks and Notes**

- 标签的字段名和重造时的辅助函数名可以由实施确定。`JoinOrder` 的两句文本、插入位置，以及最外层列序必须与书写装配相同，这些不能改。
- 最外层列如果按物理叶序交出去，WHERE 会读错列。`WHERE fact.id = 1` 和 `SELECT *` 的表头就是这条边界。
- 000 的代价函数对规格中的六个输入已经给出规定决定。本轮调用它，不调整它的整数。

## Act Response

尚未执行。

## Plan Review

- Review Result: pending

**Findings**

尚未实施。

**Follow-up Decision**

等待实施。

**Next Cycle**

None

**Next Iteration**

None
