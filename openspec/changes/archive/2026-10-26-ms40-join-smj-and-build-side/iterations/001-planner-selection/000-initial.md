# Iteration 001 / Cycle 000: 整数主键基表扫描选择排序合并

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: Iteration 000 已交付的 `SortMergeJoinExecutor` 与 `PhysicalPlan::SortMergeJoin`。纯等值内连接在两侧都是基表 `Scan`、且每条等值键都是该表已声明整数主键时，改为发出排序合并节点。多重集合与同一输入上的哈希连接相同。`ORDER BY` 不参与选择，连接之上的 Sort 保留。
- Excluded scope: T4 的负零键归一（Iteration 002）、T3 的较小构建侧与时间门、4.2 的全量回归、I016 的代价模型与连接重排、I065、NaN 排序比较、`Value::Hash`、ISS39、ISS40、ISS36 及其他 DDL 缺陷。不改加列变更目录，不改 `.agents/memory/references.md`。

**Objective**

两侧基表扫描且每个连接键都是该表已声明的整数主键时，`execute_sql` 发出排序合并节点，结果多重集合与哈希连接相同。不满足该条件的纯等值仍是哈希。外连接和非等值仍是嵌套循环。

**Investigation Facts**

- Starting point: Iteration 000 的 Review Result 是 `accepted`。该轮 Act Response 记录：`cargo test --test sort_merge_join_test` 退出码 0，3 passed；两个 `--lib` 用例各 1 passed；两个 `nested_loop_join_test` 锚点各 1 passed；`explain_compound_nodes` 退出码 0，`l.id = r.rid` 的计划行仍以 `Join` 开头。本轮规划重读了计划期代码，纯等值内连接仍构造 `PhysicalPlan::Join`。没有重跑这些测试，没有跑全量，没有跑 criterion。工作区相对提交基线 e9cb9d3 仍有未提交产品改动。加列变更的产品改动只作为共同底，不计入本任务要改的行为。
- Current-State Evidence:
  - 分类入口是 `PlanBuilder::build_from_clause_with_projection`（`src/parser/planner/query.rs`）。`OuterJoinKind::None` 且 `is_pure_equi_join_on(on_clause)` 为真时，调用 `extract_join_conditions` 后构造 `PhysicalPlan::Join`。否则构造 `PhysicalPlan::NestedLoopJoin`。外连接三态走后者。右外连接先交换输入，节点类型为 `NestedLoopJoinType::Left`。
  - `is_pure_equi_join_on` 在 `src/parser/planner/ddl_dml.rs`。AND 两侧都成立，且每一腿是 `Eq`、两侧都是裸标识符或两段限定名。字面量、表达式和其他比较符返回假。
  - `extract_join_conditions` 把等值腿收成 `JoinCondition`。限定列经 `resolve_qualified_column` 后，`ColumnRef.table` 是归一后的真实表键，`column` 已小写。裸名列的 `table` 是 `None`。两腿同侧时返回 `PlanError::InvalidQuery`，消息为 `JOIN ON condition must reference both sides of the join`。
  - 左臂基表在 `build_from_clause_with_projection` 里做成 `PhysicalPlan::Scan`，`ScanNode.table_name` 是真实表键（`object_name_parts` 已小写），不是别名。右臂基表同样。别名经 `register_table_alias` 指回真实键。CTE 与视图解析成功时，子计划是 `DerivedScan`，不是 `Scan`。派生表因子在 `extract_join_table_name` 对非 `TableFactor::Table` 返回 `UnsupportedStatement("DERIVED TABLE")`，本任务不改变这个拒绝。
  - 无 WHERE 时把顶层 `Scan` 换成 `DataScan` 的改写只匹配当时的顶层计划。连接节点落入 `_ => base_plan`，两侧子节点保持 `Scan`。`ORDER BY` 在连接装配之后包一层 `PhysicalPlan::Sort`，父节点先于子节点写入 `EXPLAIN` 文本。`describe` 的排序行是 `Sort (order_by=...)`。哈希行是 `Join [Inner] (on=[...])`。排序合并行是 `SortMergeJoin (on=[...])`。条件文本是 `表.列 = 表.列`；自连接两侧真实表名相同，所以条件里出现两次同一个表名，不出现别名。
  - `PlanBuilder.primary_keys` 是真实表键到一个主键列名，键和列名都已小写。没有主键时登记的是空字符串。`register_table_schema` 只在主键列出现在列清单里时调用 `set_pk_column_type`。整数主键在这里是 `Some(crate::storage::page_format::ColumnType::Int)`。未注册是查不到，不是 `Int`。`pk_type_known_non_int` 对未注册返回假，不能用它的否定判断整数主键。字符串主键的类型是 `ColumnType::String(u16)`。`TEXT PRIMARY KEY` 会登记成这个类型。
  - `create_executor_from_plan` 的 `PhysicalPlan::SortMergeJoin` 臂已经构造 `SortMergeJoinExecutor` 并调用 `with_budget`。本任务不改这条臂。`get_subquery_first_column` 对 `SortMergeJoin` 已经返回 `InSubqueryJoinUnsupported`，文案是 `IN subquery with JOIN is not supported`。多列误报文案是 `Subquery returns multiple columns (IN subquery requires single column)`。
  - 查询内存：非字符串值 `value_bytes` 为 16。`QueryBudget::charge` 在 `prev + bytes > limit` 时失败。单列整数行是 16 字节。上限 16 时第一行入账成功，下一行失败。`execute_sql` 的超限错误是 `SqlError`，`message` 为 `Execution error: query memory limit exceeded (limit 16 bytes)`，`sqlstate` 为 `Some("53200")`。上限经 `Database::set_query_memory_limit` 设置。
  - 既有锚点：`tests/nested_loop_join_test.rs` 的 `equi_on_keeps_hash_join_shape` 使用 `r(a INT)`、`s(b INT)`，没有主键，必须保持 `PhysicalPlan::Join`。`non_equi_on_routes_to_nested_loop_join` 是 `r.a < s.b`，必须保持嵌套循环。`tests/explain_test.rs` 的 `explain_compound_nodes` 建 `l(id INT PRIMARY KEY)` 与 `r(rid INT PRIMARY KEY)`，语句是 `l.id = r.rid`，并带 `WHERE`、`ORDER BY` 和 `LIMIT`。当前断言要求某一行 `trim_start().starts_with("Join")` 且包含 `l`。`SortMergeJoin` 不满足这个前缀。同一文件里 `starts_with("Sort")` 也会命中 `SortMergeJoin`。
  - 会改走排序合并、但断言不是行序的既有用例：`tests/pipeline_test.rs` 的 `test_pipeline_join_with_null_keys`（`a.id = b.id`，两表都是 `id INT PRIMARY KEY`，断言 1 行，NULL 不匹配）；`tests/join_test.rs` 的 `execute_sql_returns_err_not_partial_rows_on_join_where_overflow`（同样两表整数主键，断言整句错误且消息含 `Integer overflow in abs`）；`tests/subquery_alias_binding_test.rs` 的 `exists_inner_join_both_arms_aliased`（`j` 与 `u` 都是 `id INT PRIMARY KEY`，别名 `x`、`y`，断言内层结果是 id 1）；`tests/projection_expression_test.rs` 的 `join_with_expression_rejected`（两表 `id INT PRIMARY KEY`，SELECT 含 `CASE`，断言错误消息含 `expression` 或 `Unsupported`）。
  - 必须保持哈希的既有形状断言：`tests/outer_join_test.rs` 的 `outer_join_routes_to_nested_loop_join`。其中 `l(id INT PRIMARY KEY)`，`r(rid INT, val VARCHAR)`，`rid` 不是主键。`l.id = r.rid` 的断言是 `PhysicalPlan::Join`。
- Code and Critical Path: SQL 进入 `build_from_clause_with_projection` 的纯等值内连接臂。四个条件都成立时构造 `PhysicalPlan::SortMergeJoin(JoinNode { ... })`，字段与现在的 `Join` 节点相同。任一条件失败仍构造 `PhysicalPlan::Join`。`pipeline.rs` 已有的 `SortMergeJoin` 臂执行该节点。`EXPLAIN` 经 `describe` 写出 `SortMergeJoin`。测试用 `Database::execute_sql`，每个测试自己的 `TempDir`。

**Implementation Guidance**

判定放在已经确认为纯等值内连接、并且 `extract_join_conditions` 已经返回的那一臂。四个条件同时成立才构造 `SortMergeJoin`，否则仍构造 `Join`：

1. 左右子计划都是 `PhysicalPlan::Scan`。`DerivedScan`、`Filter`、`DataScan`、`IndexScan` 以及上一层连接都不是。
2. 每一条条件的左列都是左扫描表的主键，右列都是右扫描表的主键。列名与 `primary_keys` 中该表的列名相等。限定列的 `ColumnRef.table` 必须等于该侧 `ScanNode.table_name`。裸名列的 `table` 为 `None`，按该侧扫描表检查。主键登记为空字符串则不成立。
3. 两侧表在 `primary_key_types` 里都是 `Some(ColumnType::Int)`。未注册不成立。不要用 `pk_type_known_non_int` 的否定。
4. 条件列表非空。多一条非主键等值腿时，这一步整步仍走哈希。

`ORDER BY` 和 `WHERE` 包在连接节点之外，不改变判定，也不删除 Sort。自连接的两个 `Scan` 表名相同，条件成立时走排序合并。三表链的第二步左输入已是连接节点，不走排序合并。

`explain_compound_nodes` 把 `("Join", "l")` 改成：某一行去掉缩进后以 `SortMergeJoin` 开头，并且包含 `l`。Sort 的判断改成去掉缩进后以 `Sort (` 开头。不要用 `starts_with("Sort")`，那会把 `SortMergeJoin` 当成 Sort。不要改这条语句的 SQL。`outer_join_routes_to_nested_loop_join` 的哈希断言保持 `PhysicalPlan::Join`。

**Task Definitions**

### T2: 整数主键基表扫描发出排序合并

- Requirement/Scenario: 「整数主键基表扫描走排序合并」的三个场景；「纯等值 JOIN Hash 路径保持」里的非主键列、字符串主键、连接结果再连接；「外连接与非等值仍走嵌套循环」；「IN 子查询继续拒绝连接形状」；「排序合并节点可被计划文本识别」的语句文本；「排序合并连接遵守查询内存上限」的语句入口
- Depends on: T1
- Targets: `src/parser/planner/query.rs` 的 `build_from_clause_with_projection` 里纯等值内连接那一臂；`tests/sort_merge_join_test.rs`；`tests/explain_test.rs` 的 `explain_compound_nodes`
- Current behavior: 纯等值内连接构造 `PhysicalPlan::Join`。`EXPLAIN` 对应行以 `Join [Inner]` 开头。`l(id INT PRIMARY KEY)` 与 `r(rid INT PRIMARY KEY)` 的 `l.id = r.rid` 也是这一行。外连接和非等值是嵌套循环。无主键的 `r.a = s.b` 是哈希。
- Required behavior: 下面四条同时成立时，该连接节点是 `PhysicalPlan::SortMergeJoin`，`EXPLAIN` 去掉缩进后有一行以 `SortMergeJoin` 开头，并含等值条件。任一条件不成立时，该步仍是 `PhysicalPlan::Join`，对应行以 `Join [` 开头。
  - `customers(id INT PRIMARY KEY)` 与 `orders(id INT PRIMARY KEY)`，`customers.id = orders.id`。插入 customers 的 1、2 和 orders 的 2、3。无 `ORDER BY` 的结果多重集合是一行 `(2, 2)`。
  - 自连接：`customers a JOIN customers b ON a.id = b.id`。插入 1 和 2。结果多重集合是 id 1 与 id 2。`EXPLAIN` 条件文本是 `customers.id = customers.id`。
  - `ORDER BY customers.id` 的同一连接：连接节点仍是排序合并，其上仍有 Sort。Sort 行去掉缩进后以 `Sort (` 开头，出现在 `SortMergeJoin` 行之前，缩进更浅。结果次序是 `ORDER BY` 的次序。
  - 语句内存：两张单列整数主键表各一行，`set_query_memory_limit(16)` 后执行该连接。`execute_sql` 返回错误，`sqlstate` 是 `Some("53200")`，`message` 是 `Execution error: query memory limit exceeded (limit 16 bytes)`。没有成功结果行。
  - `orders(id INT PRIMARY KEY, customer_id INT, amount INT)` 与 `customers(id INT PRIMARY KEY, name STRING)`，`orders.customer_id = customers.id`，计划仍是哈希。同一测试里再执行 `customers.id = orders.id AND customers.id = orders.customer_id`，因为 `customer_id` 不是 orders 的主键，整步仍是哈希。
  - 两侧 `TEXT PRIMARY KEY` 的等值内连接仍是哈希。
  - 三张 `id INT PRIMARY KEY` 基表按主键连成三表内连接。`EXPLAIN` 有一行 `SortMergeJoin`，也有一行 `Join [`。第二步不是排序合并。
  - 外层 `IN` 使用上述整数主键连接作为子查询。语句失败，错误含 `IN subquery with JOIN is not supported`，并且不含 `Subquery returns multiple columns`。
  - 无主键的 `r.a = s.b` 仍是哈希。`r.a < s.b` 仍是嵌套循环。`l.id = r.rid` 且 `r.rid` 不是主键时仍是哈希。
- Preserve: 嵌套循环的算法与外连接结果；哈希连接仍先构建右输入；`SortMergeJoinExecutor` 的比较与乘积；`Value::Hash`；NaN 的排序比较；`IN` 子查询对哈希连接和嵌套循环的既有拒绝文案。每个新测试使用自己的 `TempDir`。
- Forbidden: 不改 `src/executor/join.rs`、`src/executor/sort_merge_join.rs`、`src/executor/value.rs`、`src/executor/value_ref.rs`。不把 SQL 改写成躲开排序合并。不把 `outer_join_routes_to_nested_loop_join` 的哈希断言改成排序合并。不跑全量，不跑 criterion。不实现 T4 的负零归一。
- First test: 在 `tests/sort_merge_join_test.rs` 增加 `integer_pk_base_scans_select_sort_merge`。它经 `Database::execute_sql` 执行上面的 customers/orders 语句。`EXPLAIN` 行应以 `SortMergeJoin` 开头。修改前运行 `cargo test --test sort_merge_join_test integer_pk_base_scans_select_sort_merge`，预期失败：计划行以 `Join [Inner]` 开头。看到这个失败之后再改计划期分类。
- GREEN condition: 同一命令退出码 0。随后 `cargo test --test sort_merge_join_test` 退出码 0，并且覆盖本任务列出的其余测试名。
- Verification: 见本 Cycle 的 Verification。任一命令退出码非 0，或 `failed` 不为 0，本任务未完成。
- Stop when: 为了通过测试需要改执行器算法、`Value::Hash` 或 NaN 排序；某个应保持哈希或嵌套循环的语句变成了另一种节点，而且不是「无 `ORDER BY` 时只是行序不同」；`outer_join_routes_to_nested_loop_join` 变成排序合并；`join_with_expression_rejected` 的消息不再含 `expression` 或 `Unsupported`；内存超限的错误不是 `53200` 或消息与上面的句子不一致。这些情况返回计划，不在本 Cycle 改算法。

**Invariants**

纯等值 SQL 只有在四条条件都成立时才离开哈希。外连接和非等值仍是嵌套循环。连接键相等仍与 `Value` 的 `PartialEq` 一致；本 Cycle 不修改这个相等的实现。无 `ORDER BY` 的排序合并结果不保证行序。打开数据库的新测试各自使用自己的 `TempDir`。

**Non-goals**

负零与正零的哈希查找、较小构建侧、全量回归、NaN 排序、聚合与半连接和反连接的分桶、DDL 缺陷。

**Acceptance**

- `integer_pk_base_scans_select_sort_merge`：整数主键基表等值的 `EXPLAIN` 含 `SortMergeJoin`，结果多重集合是 `(2, 2)`。
- `self_join_integer_pk_selects_sort_merge`：自连接的 `EXPLAIN` 含 `SortMergeJoin` 和 `customers.id = customers.id`，结果多重集合是 id 1 与 id 2。
- `order_by_keeps_sort_merge_and_sort`：`EXPLAIN` 同时有 `Sort (` 与 `SortMergeJoin`，结果次序由 `ORDER BY` 决定。
- `explain_integer_pk_join_names_sort_merge`：`EXPLAIN` 含 `SortMergeJoin` 和等值条件。
- `sort_merge_join_statement_memory_limit_53200`：上限 16 时语句失败，SQLSTATE `53200`，消息与 Task Definition 中的句子一致，没有成功结果行。
- `non_pk_equijoin_stays_hash`：`orders.customer_id = customers.id` 以及多一条非主键腿的语句，`EXPLAIN` 行以 `Join [` 开头。
- `string_pk_equijoin_stays_hash`：`TEXT PRIMARY KEY` 的等值内连接以 `Join [` 开头。
- `third_table_join_stays_hash`：三表链的 `EXPLAIN` 同时有 `SortMergeJoin` 和 `Join [`。
- `in_subquery_integer_pk_join_rejected`：错误含 `IN subquery with JOIN is not supported`，不含多列误报。
- `explain_compound_nodes`：`l.id = r.rid` 的计划行以 `SortMergeJoin` 开头，`Sort (` 仍在，`Limit` 与 `Filter` 仍在。
- `equi_on_keeps_hash_join_shape` 仍是哈希。`non_equi_on_routes_to_nested_loop_join` 仍是嵌套循环。
- `test_pipeline_join_with_null_keys` 仍是 1 行。`execute_sql_returns_err_not_partial_rows_on_join_where_overflow` 仍整句失败并含溢出文案。`exists_inner_join_both_arms_aliased` 的内层结果仍是 id 1。`join_with_expression_rejected` 的错误消息仍含 `expression` 或 `Unsupported`。`outer_join_routes_to_nested_loop_join` 的内连接断言仍是 `PhysicalPlan::Join`。
- 上述既有用例若只因为无 `ORDER BY` 的行序改变而失败，把该断言改成多重集合比较。不改 SQL。

**Verification**

下列命令按写出的顺序逐条执行。定向测试使用默认并行度。不跑全量，不跑 criterion。每条退出码 0 且 failed 为 0。

1. 修改前：`cargo test --test sort_merge_join_test integer_pk_base_scans_select_sort_merge`。预期失败，计划仍是 `Join [Inner]`。
2. 修改后：`cargo test --test sort_merge_join_test`。
3. `cargo test --test explain_test explain_compound_nodes`。
4. `cargo test --test nested_loop_join_test equi_on_keeps_hash_join_shape`。
5. `cargo test --test nested_loop_join_test non_equi_on_routes_to_nested_loop_join`。
6. `cargo test --test pipeline_test test_pipeline_join_with_null_keys`。
7. `cargo test --test join_test execute_sql_returns_err_not_partial_rows_on_join_where_overflow`。
8. `cargo test --test subquery_alias_binding_test exists_inner_join_both_arms_aliased`。
9. `cargo test --test projection_expression_test join_with_expression_rejected`。
10. `cargo test --test outer_join_test outer_join_routes_to_nested_loop_join`。

第 6 到第 9 条若只因行序失败，按 Acceptance 改成多重集合后重跑该条一次。第 10 条若变成排序合并，停止。

**Gate 2 Readiness**

PASS。无 BLOCKED，无 SKIPPED。用户本轮指令批准把 ISS41 纳入本变更，并批准展开下一个 Iteration。T2 的需求与触发条件沿用 proposal 记载的 2026-10-26 批准。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 整数主键在 SQL 里不能重复，所以本 Cycle 的语句测试看的是匹配键的多重集合，不重复 Iteration 000 的 2×2 乘积。
- `test_pipeline_join_with_null_keys` 会改走排序合并。它插入 NULL。若主键拒绝这次插入，该测试在修改前就已经失败；那时沿用修改前的失败并停止，不把主键约束改掉。
- 扫描执行器若也向同一预算入账，上限 16 仍应得到 `53200` 和同一句消息。若错误变成别的 SQLSTATE 或另一句消息，停止。
- 负零漏匹配还没有用这对键跑过。本 Cycle 不修。先行测试留在 T4。

## Act Response

- Status: blocked

**实际改动**

- `PlanBuilder::build_from_clause_with_projection` 在纯等值内连接已经抽出条件之后，调用 `integer_pk_base_scan_join`。四个条件都成立时构造 `PhysicalPlan::SortMergeJoin`，否则仍构造 `PhysicalPlan::Join`。四个条件是：左右子计划都是 `Scan`，条件列表非空，每条左列等于左表 `primary_keys` 里的列名、右列等于右表的列名，限定列的表名等于该侧 `ScanNode.table_name`，裸名列按该侧扫描表检查，并且两侧 `primary_key_types` 都是 `Some(ColumnType::Int)`。空字符串主键不成立。没有使用 `pk_type_known_non_int` 的否定。
- `tests/sort_merge_join_test.rs` 增加本任务列出的语句测试。`tests/explain_test.rs` 的 `explain_compound_nodes` 把连接行改成以 `SortMergeJoin` 开头，排序行改成以 `Sort (` 开头。
- 没有改 `src/executor/join.rs`、`src/executor/sort_merge_join.rs`、`src/executor/value.rs`、`src/executor/value_ref.rs`。没有做负零归一，没有改 `Value::Hash`，没有做构建侧选择。

**与计划的偏差**

`string_pk_equijoin_stays_hash` 不能用 `CREATE TABLE ... TEXT PRIMARY KEY`。这条 DDL 在计划期被拒绝，错误是 `Unsupported constraint: NON-INTEGER PRIMARY KEY`。测试改为用 `create_table_with_constraints` 写入两张存量字符串主键表，关闭后重新打开，再经 `execute_sql` 看 `EXPLAIN`。断言仍是计划行以 `Join [` 开头。

**自检**

Gate 3 未通过。已声明整数主键的语句按验收走了排序合并，但无显式主键的 `r.a = s.b`，以及 `outer_join_routes_to_nested_loop_join` 里 `r.rid` 不是显式主键的内连接，也变成了 `SortMergeJoin`。这命中本 Cycle 的停止条件。没有改执行器算法。

无已修复的 Critical 或 Important。遗留 Minor：`PhysicalPlan::SortMergeJoin` 的注释仍写着 SQL 计划路径发出 `Join`。本 Cycle 没有改那个文件。

**验证**

| 命令 | 退出码 | 结果 |
|---|---|---|
| `cargo test --test sort_merge_join_test integer_pk_base_scans_select_sort_merge`（改计划前） | 101 | 0 passed，1 failed。计划行是 `Join [Inner] (on=[customers.id = orders.id])` |
| `cargo test --test sort_merge_join_test`（分类改完后的第一次） | 101 | 11 passed，1 failed。`string_pk_equijoin_stays_hash` 在 `CREATE TABLE ... TEXT PRIMARY KEY` 被拒绝 |
| `cargo test --test sort_merge_join_test`（存量字符串主键建表之后） | 0 | 12 passed，0 failed |
| `cargo test --test explain_test explain_compound_nodes` | 0 | 1 passed，0 failed |
| `cargo test --test nested_loop_join_test equi_on_keeps_hash_join_shape` | 101 | 0 passed，1 failed。实际节点是 `SortMergeJoin` |
| `cargo test --test nested_loop_join_test non_equi_on_routes_to_nested_loop_join` | 0 | 1 passed，0 failed |
| `cargo test --test pipeline_test test_pipeline_join_with_null_keys` | 0 | 1 passed，0 failed |
| `cargo test --test join_test execute_sql_returns_err_not_partial_rows_on_join_where_overflow` | 0 | 1 passed，0 failed |
| `cargo test --test subquery_alias_binding_test exists_inner_join_both_arms_aliased` | 0 | 1 passed，0 failed |
| `cargo test --test projection_expression_test join_with_expression_rejected` | 0 | 1 passed，0 failed |
| `cargo test --test outer_join_test outer_join_routes_to_nested_loop_join` | 101 | 0 passed，1 failed。`l.id = r.rid` 的内连接实际是 `SortMergeJoin` |

这些命令按上表顺序逐条执行。没有改成一次无过滤的 `cargo test`。没有跑全量，没有跑 criterion。

**Persisted Evidence**

None required。

**Experience Candidates**

None。

**Issue 候选**

None。

**未解决问题**

T2 未完成。无显式主键的整数首列被登记成整数主键后，会和已声明主键走同一条排序合并条件。

**Blocker Handoff**

- 任务与位置：T2，验证第 4 条和第 10 条，Gate 5。
- 计划预期：`primary_keys` 在没有主键时是空字符串，类型未登记就不是整数主键。`r(a INT)` 与 `s(b INT)` 的 `r.a = s.b` 保持哈希。`r(rid INT, val VARCHAR)` 的 `rid` 不是主键，`l.id = r.rid` 保持哈希。
- 实际情况：`CreateTableExecutor` 在 `primary_key` 为 `None` 时把第一列写入 `pk_column`（`src/executor/create_table.rs`）。`register_table_schema` 把这个列名放进 `primary_keys`，列存在时再把类型放进 `primary_key_types`。因此 `CREATE TABLE r (a INT)` 的登记主键是 `a`，类型是 `Int`；`CREATE TABLE r (rid INT, val VARCHAR)` 的登记主键是 `rid`，类型是 `Int`。四个条件对这两句都成立，计划变成 `SortMergeJoin`。`TableMeta` 和 `PlanBuilder` 都只保存列名和类型，没有保存这个键是不是显式 `PRIMARY KEY`。
- 影响：验收要求保持哈希的两句变成了排序合并。停止条件要求把这种情况交回计划，本 Cycle 不改执行器算法。
- 已完成：先行测试先失败，失败文本是 `Join [Inner]`。分类谓词、新语句测试和 `explain_compound_nodes` 已写入。已声明整数主键、自连接、`ORDER BY`、内存上限 `53200`、非主键列、字符串主键、三表第二步和 `IN` 子查询拒绝，在 `sort_merge_join_test` 里通过。
- 部分完成：T2 的验证。上表里退出码 0 的命令通过；第 4 条和第 10 条失败。
- 未开始：没有未写的任务步骤。验收没有达到。
- 修改文件：`src/parser/planner/query.rs`，`tests/sort_merge_join_test.rs`，`tests/explain_test.rs`。工作区保留这些改动，没有回滚。
- 已通过的 Gate：Gate 2 在计划里已经是 PASS。本 Cycle 的 Gate 3 和 Gate 4 未通过。
- Evidence：None required。
- 恢复条件：计划改写判定，使只有显式 `PRIMARY KEY` 的整数键走排序合并，或改验收。当前 `primary_keys` 不能区分显式主键和隐式首列键。恢复后不要在本 Cycle 改 `SortMergeJoinExecutor`、`Value::Hash` 或 NaN 排序。

## Plan Review

- Review Result: replan-required

**Findings**

Plan 错误，阻塞当时的 Acceptance。Task Definition 把「没有主键」写成 `primary_keys` 为空字符串、类型未登记。当前代码不是这样。`extract_primary_key` 在 SQL 没有 `PRIMARY KEY` 时返回 `None`，`CreateTableNode.primary_key` 保存这个 `Option`。`CreateTableExecutor` 在 `None` 时把第一列写入 `pk_column`。`register_table_schema` 再把列名放进 `primary_keys`，类型是 `Int` 时放进 `primary_key_types`。`integer_pk_base_scan_join` 只看 Scan、条件非空、列名和 `Int`，所以隐式首列也满足。

Act 按该依据实现。已写出整数 `PRIMARY KEY` 的语句测试通过。`equi_on_keeps_hash_join_shape` 与 `outer_join_routes_to_nested_loop_join` 的内连接变成 `SortMergeJoin`，退出码都是 101。这是计划把完成条件钉在分不开的字段上，随后用户收窄了触发条件。不是 Act 偏离，也不是执行器算法错误。`src/executor/join.rs`、`src/executor/value.rs`、`src/executor/value_ref.rs` 相对 HEAD 没有未提交差异。

没有不阻塞 Acceptance 的额外 finding 要留在本 Cycle 里修。`PhysicalPlan::SortMergeJoin` 注释仍写着 SQL 计划路径发出 `Join`，Act 记为 Minor，本轮不修。

**Acceptance Gaps**

- `equi_on_keeps_hash_join_shape`：`CREATE TABLE r (a INT)` 与 `CREATE TABLE s (b INT)` 的 `r.a = s.b` 应是 `PhysicalPlan::Join`，实际是 `SortMergeJoin`。来源是父 Cycle Act Response 的验证表，本次重读 `integer_pk_base_scan_join` 与 `CreateTableExecutor` 后与该结果一致，没有重跑。
- `outer_join_routes_to_nested_loop_join`：`l(id INT PRIMARY KEY)` 与 `r(rid INT, val VARCHAR)` 的内连接 `l.id = r.rid` 应是 `PhysicalPlan::Join`，实际是 `SortMergeJoin`。外连接三态的断言写在同一测试的前面。失败记录指向这个内连接。

**Evidence**

- `src/parser/planner/ddl_dml.rs` 的 `extract_primary_key`：零个候选返回 `None`。
- `src/executor/plan.rs` 的 `CreateTableNode.primary_key: Option<String>`。
- `src/executor/create_table.rs`：`primary_key` 为 `None` 时取第一列，再调用 `create_table_with_constraints`。
- `src/pipeline.rs` 的 `register_table_schema`：登记 `table_meta.pk_column` 和对应列类型。
- `src/parser/planner/query.rs` 的 `integer_pk_base_scan_join`：没有显式主键位。
- `src/storage/catalog.rs` 的 `CatalogRow` 与 `src/storage/data/table_manager.rs` 的 `TableMeta`：只有 `pk_column` 和 `pk_index`。
- 验证沿用父 Cycle Act Response：两条锚点退出码 101；`sort_merge_join_test` 在字符串主键改法之后退出码 0；`explain_compound_nodes` 退出码 0。本次没有重跑，也没有跑全量。

**Follow-up Decision**

用户收窄了触发条件，T2 的完成条件变了。父 Cycle 的 Task Definition 无法要求 Act 区分写出的 `PRIMARY KEY` 和隐式首列。不在本 Cycle 上继续，也不做 rework。

**Iteration Plan Update**

Iteration 001 的稳定结果改为：排序合并只在两侧都是基表扫描、且每个连接键是该表写出的整数 `PRIMARY KEY` 时发出。隐式首列整数键不满足。字符串主键不满足。外连接和非等值仍走嵌套循环。其余纯等值仍走哈希。两条锚点的计划形状保持哈希或嵌套循环。验证增加重新打开后的语句，以及目录尾字节的往返和缺字节读取。全量仍只在 Iteration 003，按既有分批方式。Iteration 002 与 003 的任务不变。

**Next Cycle**

`iterations/001-planner-selection/001-replan.md`

**Next Iteration**

None

**Issue 候选**

None。这次偏差留在本变更的触发条件里，由后继 Cycle 承载。
