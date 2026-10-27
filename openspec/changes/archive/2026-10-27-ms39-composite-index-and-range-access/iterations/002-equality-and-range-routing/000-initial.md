# Iteration 002 / Cycle 000: 等值与范围路由

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 规格 `sql-secondary-indexes` 中的等值路由、范围路由和索引选择。世代 1 的哨兵沿用 `Value::secondary_range_bounds`。001 已接受的创建、维护和恢复不在本轮重做。
- Excluded scope: README、取消、只读、内存、加密、附加库和旧库的故障组合，以及全量测试。这些属于 003。

**Objective**

规格里的等值和范围查询能从 `EXPLAIN` 看出所用索引。整键等值仍是 `IndexScanAll`。前缀和范围是 `IndexRangeScan`。可能漏掉非 NULL 行的索引在范围结果里补上键派生为空且谓词成立的可见行，文本含 `omitted=scan`。全整数且不超过 4 列的世代 1 索引不补扫。升级前的单列整数索引上的范围仍是扫描。每种可路由和明确不路由的查询，行多重集合都与同一语句在建索引前相同。

**Investigation Facts**

- Starting point: 001 的 Act Response 在 `645e567`，Review Result 为 accepted。组合索引和新建单列整数索引已经按世代和完整列序维护条目。计划器目前只收到没有额外列的二级索引。验证沿用 001 Review 记录的三条命令，覆盖的代码在该提交之后没有变化。
- Current-State Evidence:
  - `pipeline.rs` 的 `register_table_schema` 用 `extra_column_indexes.is_empty()` 滤掉组合索引，传给计划器的是 `(索引名, 第一列名, 世代)`，顺序是 `secondary_indexes` 的登记顺序。
  - `query.rs` 在主键等值之后调用 `extract_secondary_index_equality`。它只接受一个裸列等值，两侧之一必须是 `Expr::Value`。命中后构造 `IndexScanAll`，`columns` 是基表全列，`projection` 决定输出。`age = -7` 是一元负号，不是字面量，所以今天走扫描。
  - `BETWEEN` 在 `build_where` 里才拆成 `>=` 与 `<=`。路由看到的是原始表达式。`expression.rs` 把「一元负号加整数字面量」编成 `Value::Int(-n)`。
  - `Value::secondary_range_bounds` 只接受前缀：除最后一列外必须是等值，最后一列可以是等值或带包含标记的上下界。它使用世代 1 的分量。单列字符串的世代 0 键就是这段转义再填零，与哨兵的字符串字节相同。世代 0 的整数键仍是没有翻转符号位的大端，和这套哨兵不是同一顺序。
  - `IndexManager` 有 `search_all` 和 `scan_all`。`scan_all` 用内部节点的子页指针做显式栈，不沿叶子链前进。没有按端点取条目的方法。
  - `IndexScanAllExecutor` 用 `search_all` 取行号。有 snapshot 时用 `find_visible_version`，否则用 `read_tuple_from_data_page`。在途窄化时 `ensure_readable` 拒绝。
  - `EXPLAIN` 由 `plan_format.rs` 描述计划，不执行查询。`IndexScanAll` 的文本是 `IndexScanAll <table> (key=..., index=<name>, columns=...)`。`columns` 经投影裁剪。
  - 穷尽匹配 `PhysicalPlan` 且已经有 `IndexScanAll` 臂的位置：`query.rs` 的输出列和 `is_base_scan_chain`，`plan_format.rs` 的叶子臂与 `describe`，`pipeline.rs` 的类型槽、执行器构造、`plan_output_names` 和输出列映射，`subquery.rs` 的标量子查询右支，`correlated.rs` 的叶子臂。`subquery.rs` 的 `_` 会把不认识的节点报成多列。
  - `tests/composite_index_lifecycle_test.rs` 的 `composite_backfill_matches_live_rows` 断言 `WHERE age = 10` 在 `(age, name)` 上是 `DataScan` 且不是 `IndexScanAll`。`tests/string_index_sql_test.rs` 的 `string_index_unrouted_predicates_stay_scan` 把 `name > 'a'` 和 NULL、类型不匹配、带额外谓词的 AND、OR、限定列、参数放在同一组里，断言都没有索引扫描。
- Code and Critical Path: `SELECT` 的 WHERE 在 `query.rs` 里先试主键等值，再试二级等值，否则下推 `DataScan` 或包 `Filter`。执行器由 `pipeline.rs` 按计划节点构造，按索引名从 `TableMeta.secondary_indexes` 取树。范围要在这条链上新增节点：计划器算出哨兵，执行器按内部节点走到起始叶子再按键序取条目，需要时再扫数据页补上派生为空的行。

**Implementation Guidance**

先写 `new_int_range_includes_negatives` 并看到失败：计划里还没有 `IndexRangeScan`。再改登记、路由、计划节点和执行器。整键等值继续调用现有 `search_all`。范围只消费 `secondary_range_bounds` 的返回值，不另写端点。

登记必须带上每一个二级索引的世代和完整列序。单列等值的 `IndexScanAll` 仍然只选择列清单恰好是这一列的索引。组合索引的某一列不能拿单列键去查组合树。

**Task Definitions**

### T4: 等值与范围查询的计划可观察，行集与扫描相同

- Requirement/Scenario: 二级索引等值查询路由可达；范围谓词路由到二级索引扫描。场景是规格里这两条 requirement 的场景。001 已经覆盖的条目维护不在本轮重测。
- Depends on: 001 accepted
- Targets:
  - `src/pipeline.rs` 的 `register_table_schema`，以及构造执行器、类型槽、输出列名和输出位映射的 `PhysicalPlan` 匹配
  - `src/parser/planner/mod.rs` 的二级索引登记，和 `src/parser/planner/query.rs` 的 WHERE 路由、`extract_secondary_index_equality`、输出列、`is_base_scan_chain`
  - `src/parser/planner/subquery.rs` 的标量子查询右支
  - `src/executor/correlated.rs` 的叶子臂
  - `src/executor/plan.rs` 与 `src/executor/plan_format.rs`
  - `src/storage/btree/index_manager.rs` 的范围取条目
  - 新建范围执行器，可见性对齐 `src/executor/index_scan_all.rs`
  - `tests/composite_index_lifecycle_test.rs::composite_backfill_matches_live_rows`
  - `tests/string_index_sql_test.rs::string_index_unrouted_predicates_stay_scan`
- Current behavior: 只有没有额外列的二级索引能进入等值路由，而且只认一个裸列字面量等值。范围、`BETWEEN`、AND、组合整键和一元负号都走 `DataScan` 或 `Filter`。`IndexManager` 不能按端点取条目。`name > 'a'` 被测试断言为不走索引。`(age, name)` 上的 `age = 10` 被测试断言为 `DataScan`。
- Required behavior:
  - 计划器按 `secondary_indexes` 的登记顺序收到每个二级索引的名字、世代和完整列序。创建更早的索引排在前面。
  - 谓词先按 AND 拆开。列在谓词中的书写顺序不影响它是否构成前缀。每一项必须是裸列与字面量的比较，或者裸列的 `BETWEEN`。字面量可以在列的任一侧。比较符是 `=`、`<`、`<=`、`>`、`>=`。`BETWEEN` 的两端都包含。一元负号作用在整数字面量上时，按 `expression.rs` 已有的 `Value::Int(-n)` 解释，只用于范围端点。等值路由仍然只接受 `Expr::Value`，`age = -7` 保持扫描。
  - 限定列、OR、`NOT BETWEEN`、参数、表达式、类型对不上、NULL，以及不能由同一个索引前缀解释的额外谓词，不路由。结果和错误与建索引前相同。
  - 一个索引能解释谓词，当且仅当前面的列只有等值、最后一列是等值或一个范围，而且谓词里没有这个前缀之外的项。只约束非首列不能解释。
  - 多个索引都能解释时，选被约束前缀更长的。长度相同，选不需要补扫的。仍相同，选创建更早的。
  - 同一列的两个下界保留更严的一个：哨兵键更大者更严；键相同则不包含比包含更严。两个上界保留哨兵键更小的；键相同则不包含比包含更严。收紧后如果下界键大于上界键，或者键相等且不是两端都包含，区间为空。空区间仍然是 `IndexRangeScan`，结果没有行。
  - 整键等值且 `derive_secondary_index_key` 返回键时，计划是 `IndexScanAll`，执行器用 `search_all`。编码放不下时不使用这个索引的 `IndexScanAll`，结果只含真实相等的行。
  - 其余可路由谓词使用 `secondary_range_bounds`。世代 0 且列是整数的索引不参与范围路由，`WHERE 列 > 0` 的计划不含 `IndexRangeScan`，等值仍命中原行。世代 0 的单列字符串可以走范围，因为哨兵字节与现有字符串键相同。
  - 范围执行器从内部节点下降，按键序取落在哨兵内的条目，不把叶子链指针当作唯一的前进方式。成员判断用 `Key` 的现有顺序，不把键解码成行值。有 snapshot 时用 `find_visible_version`，否则用 `read_tuple_from_data_page`。在途窄化仍由 `ensure_readable` 拒绝。
  - 全整数且列数不超过 4 的世代 1 索引不补扫。含字符串列，或可能因超过 32 字节漏掉非 NULL 行的索引，范围结果还要加上键派生为 `None` 且原谓词成立的可见行。NULL 不满足范围比较，不得凭补扫返回。树里已经有的行不要再从补扫返回。
  - `EXPLAIN` 不执行查询。范围文本是 `IndexRangeScan <table> (index=<name>, columns=<与 IndexScanAll 相同的投影形状>)`。需要补扫时含 `omitted=scan`，不需要时不含这串字。等值文本保持现有的 `IndexScanAll`。
  - `IndexRangeScan` 的列清单是基表全列，输出由投影决定。输出列、`is_base_scan_chain`、标量子查询右支、相关参数叶子臂和 pipeline 的列映射都按 `IndexScanAll` 的这套契约处理。不要用通配臂把这个节点报成多列或丢掉表名。
  - `composite_backfill_matches_live_rows` 里 `WHERE age = 10` 改为 `IndexRangeScan`，点名 `idx_age_name`，并含 `omitted=scan`。行集仍是原来的两行。它不是 `IndexScanAll`。
  - `string_index_unrouted_predicates_stay_scan` 里 `name > 'a'` 改为 `IndexRangeScan`，点名 `idx_name`，并含 `omitted=scan`。该语句的行结果不变。同函数里的 NULL、类型不匹配、`name = 'dup' AND id = 1`、OR、限定列和参数仍然既不是 `IndexScanAll` 也不是 `IndexRangeScan`。
- Preserve: 主键和唯一索引的 `to_key` 与点查。世代 0 整数等值的字节。单列字符串等值的 `IndexScanAll`。页类型和 32 字节键。`CREATE INDEX` 的世代赋值。DML、回填、恢复和删列的派生。
- Forbidden: 修改 `Value::to_key`、`Key` 的比较、`derive_secondary_index_key` 的分量规则、`secondary_range_bounds` 的哨兵规则和页格式。不要把世代 0 的整数范围送进 `IndexRangeScan`。不要用单列键探测组合树。不要写 README，不要跑全量。不要为了单列 `1` 与 `(1, i64::MIN)` 相等去改编码。
- First test: 新增 `tests/index_range_routing_test.rs` 的 `new_int_range_includes_negatives`。表含负数、0 和正数，`CREATE INDEX` 建在新建的单列整数列上。实现前运行 `cargo test --offline --test index_range_routing_test new_int_range_includes_negatives -- --test-threads=1`，预期失败：`EXPLAIN` 不含 `IndexRangeScan`。
- GREEN condition: 下面 Verification 里的命令都是 passed、failed 为 0。
- Verification: 见本 Cycle 的 Verification。新测试失败表示计划形状、哨兵、补扫或选择与本任务不一致。两条既有测试失败表示前缀等值或字符串范围的计划没有从 001 的扫描断言改过来，或者把不该路由的谓词改路由了。
- Stop when: 不修改 `Key` 比较或 `derive_secondary_index_key` 就无法让范围结果与扫描的多重集合相同；或者不把世代 0 的整数索引进 `IndexRangeScan` 就无法通过某个场景；或者现有哨兵与树上的世代 1 键或单列字符串键对不上，必须改哨兵规则。这些情况都返回计划。

**Invariants**

- 键仍是 32 字节。超长和 NULL 不入树，不截断。
- 范围成员由键的字节序决定。补扫只补派生为空且谓词成立的可见行。
- 行比较按多重集合。测试在比较前排序，不要求返回顺序与全表扫描相同。
- 主键和唯一索引的字节不变。

**Non-goals**

README、取消与只读、内存库、加密库、附加库、升级前库文件的组合，以及全量回归。

**Acceptance**

- 新建单列整数索引上，`列 > 0`、`列 < 0`、`列 >= -1`、`列 BETWEEN -2 AND 2` 和字面量在左侧的同义比较，`EXPLAIN` 含 `IndexRangeScan` 和索引名，不含 `omitted=scan`。负数只出现在数值序落入区间的结果里。结果与建索引前相同。`SELECT` 单列投影的 `columns` 是该输出列。
- 用世代 0 建出的单列整数索引上，`列 > 0` 的计划不含 `IndexRangeScan`。等值仍点名该索引并命中原行。
- 全整数组合 `(a, b)` 上，`a > 1` 含 `(2, i64::MIN)`、不含 `(1, i64::MAX)`。`a = 1` 含第一行、不含第二行。`a = 1 AND b > 2` 含第一行、不含第二行。三条都是 `IndexRangeScan`，不含 `omitted=scan`，且与扫描的多重集合相同。`b > 2 AND a = 1` 同样路由。
- 组合整键等值在列顺序对调和交换后都点名该索引的 `IndexScanAll`。整键超过 32 字节的字面量不是该索引的 `IndexScanAll`，查询仍返回真实相等的行。
- 字符串范围的 `EXPLAIN` 含 `IndexRangeScan`、索引名和 `omitted=scan`。短字符串和超过 30 字节的长字符串都在结果中。
- 只约束组合的非首列、OR、额外谓词、`NOT BETWEEN`、限定列、参数、NULL 和类型不匹配不使用 `IndexRangeScan`。矛盾区间 `列 > 5 AND 列 < 1` 仍是 `IndexRangeScan`，结果为空。
- 前缀更长者优先。长度相同则不补扫者优先。再相同则创建更早者优先。
- 不跑全量。

**Verification**

这些测试的查询和 `EXPLAIN` 都从 `Database::execute_sql` 进入。行在比较前按输出排序，比较的是多重集合。

1. `cargo test --offline --test index_range_routing_test -- --test-threads=1`
   - `new_int_range_includes_negatives`
   - `legacy_int_range_stays_scan`：表和行用 SQL 写入。索引用 `CreateIndexExecutor` 和 `generation: 0` 建在整数列上，因为现在的 `CREATE INDEX` 会把新建整数索引写成世代 1。查询和 `EXPLAIN` 仍走 SQL。
   - `composite_prefix_bounds`：`i64::MIN` 用 `0 - 9223372036854775807 - 1` 插入。`i64::MAX` 用 `9223372036854775807`。
   - `composite_equality_names_index`
   - `string_range_includes_overlong`
   - `unrouted_predicates_stay_scan`
   - `index_selection_prefers_longer_then_exact_then_earlier`
2. `cargo test --offline --test composite_index_lifecycle_test composite_backfill_matches_live_rows -- --test-threads=1`
3. `cargo test --offline --test string_index_sql_test string_index_unrouted_predicates_stay_scan -- --test-threads=1`

三条命令按顺序执行。失败表示计划、行集或既有不路由断言与本任务不一致。

**Gate 2 Readiness**

PASS。计划审查的 11 项均通过，RTM 无 Missing，没有未批准的裁剪。批准来自本次目标对已写 Iteration Plan 的授权，不是跳过。

**Persisted Evidence**

- Mode: none

测试可以重跑。命令、结果和改动的符号写入 Act Response 即可。

**Risks and Notes**

- `composite_backfill_matches_live_rows` 和 `string_index_unrouted_predicates_stay_scan` 里的扫描断言是 001 和 MS38 留下的。前缀等值和字符串范围改由本轮接管，所以这两条断言要改。其余行集断言保持。
- 标量子查询的右支如果落进 `subquery.rs` 的通配臂，会在计划期被报成返回多列。范围节点要走和 `IndexScanAll` 相同的投影裁剪。
- 不要为单列 `1` 与 `(1, i64::MIN)` 在忽略长度后比较相等去改 `Key` 或整数分量。

## Act Response

- Status: reported

**Implemented**

T4。计划器按 `secondary_indexes` 的登记顺序收到每个二级索引的名字、世代和完整列序。等值拆成 AND 后，裸列与字面量的比较以及裸列 `BETWEEN` 可以路由。字面量可以在列的任一侧。一元负号只用于范围端点；`n = -1` 仍走扫描。限定列、OR、`NOT BETWEEN`、参数、类型不匹配、NULL 和同一索引解释不了的额外谓词不路由。

多个索引都能解释时，先取被约束前缀更长的，再取不需要补扫的，最后取创建更早的。整键等值且 `derive_secondary_index_key` 返回键时，计划是 `IndexScanAll`，执行器调用 `search_all`。编码放不下时不用该索引的 `IndexScanAll`。其余可路由谓词使用 `secondary_range_bounds` 的哨兵，计划是 `IndexRangeScan`。世代 0 的整数索引不参与范围路由。全整数且不超过 4 列的世代 1 索引不补扫。含字符串列，或可能超过 32 字节的索引，范围结果再加上键派生为 `None` 且原谓词成立的可见行。`EXPLAIN` 文本是 `IndexRangeScan <table> (index=<name>, columns=...)`，需要补扫时含 `omitted=scan`。

`IndexManager::scan_range` 从内部节点下降，用显式栈按键序取落在哨兵内的条目，不沿叶子链前进。可见性与 `IndexScanAllExecutor` 相同：有 snapshot 时用 `find_visible_version`，否则用 `read_tuple_from_data_page`。在途窄化仍由 `ensure_readable` 拒绝。

改动文件：`src/parser/planner/mod.rs` 的 `SecondaryIndexRegistration`，`src/pipeline.rs` 的登记、类型槽、输出列名、列映射和执行器构造，`src/parser/planner/query.rs` 的 `route_secondary_index`，`src/parser/planner/subquery.rs` 的标量子查询右支，`src/executor/correlated.rs` 的叶子臂，`src/executor/plan.rs` 的 `IndexRangeScanNode`，`src/executor/plan_format.rs` 的渲染，`src/executor/index_range_scan.rs` 的 `IndexRangeScanExecutor`，`src/storage/btree/index_manager.rs` 的 `scan_range`。测试：新建 `tests/index_range_routing_test.rs`，并改 `tests/composite_index_lifecycle_test.rs` 的 `composite_backfill_matches_live_rows` 与 `tests/string_index_sql_test.rs` 的 `string_index_unrouted_predicates_stay_scan`。

**Deviations from Plan**

`composite_prefix_bounds` 用 `UPDATE t SET b = 0 - 9223372036854775807 - 1 WHERE id = 2` 写入 `i64::MIN`。`INSERT ... VALUES` 不接受这个减法表达式。更新后 `SELECT b FROM t WHERE id = 2` 读回 `i64::MIN`。

`string_index_unrouted_predicates_stay_scan` 不再调用 `assert_no_index_scan`。`name > 'a'` 单独断言 `IndexRangeScan`、`idx_name` 和 `omitted=scan`。同函数其余语句改为既不含 `IndexScanAll` 也不含 `IndexRangeScan`。这个只被该循环使用的辅助函数已删除。

**Self-review**

Gate 3 没有未解决的 Critical 或 Important 问题。标量子查询右支、`is_base_scan_chain`、相关参数叶子臂和 pipeline 的列映射都按 `IndexScanAll` 的投影契约补了 `IndexRangeScan` 臂。本轮三条命令没有执行带范围扫描的标量子查询或 `ORDER BY`。

**Verification Evidence**

先行测试 `new_int_range_includes_negatives` 在实现前失败。`EXPLAIN` 是 `DataScan`，不含 `IndexRangeScan`。

实现后，以下命令按顺序执行，failed 都是 0。没有并行，没有跑全量。

- `cargo test --offline --test index_range_routing_test -- --test-threads=1`：7 passed，0 failed。
- `cargo test --offline --test composite_index_lifecycle_test composite_backfill_matches_live_rows -- --test-threads=1`：1 passed，0 failed，6 filtered out。
- `cargo test --offline --test string_index_sql_test string_index_unrouted_predicates_stay_scan -- --test-threads=1`：1 passed，0 failed，7 filtered out。

**Persisted Evidence**

None required。

**Experience Candidates**

None。

**Unresolved**

None。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读了 `39f804d..2fd6b80` 的 diff 和当前源码。Act Response 与代码一致。本轮 Acceptance 已满足。没有阻塞项。

自检写过：标量子查询右支、`is_base_scan_chain`、相关参数叶子臂和 pipeline 的列映射都补了 `IndexRangeScan` 臂，但三条命令没有执行带范围扫描的标量子查询或 `ORDER BY`。这不是 Acceptance 缺口。任务要求这些位置按 `IndexScanAll` 的投影契约处理，避免通配臂把节点报成多列或丢掉表名。`get_plan_output_columns`、`is_base_scan_chain`、`subquery.rs` 的标量右支、`correlated.rs` 的叶子臂，以及 `pipeline.rs` 的类型槽、执行器构造、`plan_output_names` 和 `extract_column_indices` 都按这个契约处理：投影裁剪与 `IndexScanAll` 相同，表名保留。标量右支在通配臂之前取投影后的首列。Acceptance 的八条可观察条件不包含标量子查询，也不包含排序键落在输出列之外的 `ORDER BY`。Verification 就是已通过的三条命令。

`composite_backfill_matches_live_rows` 执行了 `SELECT id, age, name FROM t WHERE age = 10 ORDER BY id`，并断言结果是原来的两行。该语句选出全部列。EXPLAIN 打在没有 `ORDER BY` 的同一 WHERE 上。自检把「没有执行 ORDER BY」说满了，但不构成另一条必须补跑的验收。

- 非阻塞，与计划一致。`composite_prefix_bounds` 用 `UPDATE` 写入 `i64::MIN`，因为 `INSERT ... VALUES` 不接受那个减法表达式。`SELECT b FROM t WHERE id = 2` 读回 `i64::MIN`。`a > 1` 只含 id 2，`a = 1` 和 `a = 1 AND b > 2` 只含 id 1。
- 非阻塞，与计划一致。`string_index_unrouted_predicates_stay_scan` 删掉了只被该循环使用的 `assert_no_index_scan`。`name > 'a'` 单独断言 `IndexRangeScan`、`idx_name` 和 `omitted=scan`。同函数其余语句既不含 `IndexScanAll` 也不含 `IndexRangeScan`。

**Evidence**

沿用 Act Response 的验证，覆盖范围在 `2fd6b80` 之后没有变化，本次不重跑：

- `cargo test --offline --test index_range_routing_test -- --test-threads=1`：7 passed，0 failed。
- `cargo test --offline --test composite_index_lifecycle_test composite_backfill_matches_live_rows -- --test-threads=1`：1 passed，0 failed，6 filtered out。
- `cargo test --offline --test string_index_sql_test string_index_unrouted_predicates_stay_scan -- --test-threads=1`：1 passed，0 failed，7 filtered out。

代码核对：计划器按 `secondary_indexes` 的登记顺序收到名字、世代和完整列序。谓词按 AND 拆开。裸列与字面量的比较，以及裸列 `BETWEEN`，可以路由；字面量可以在列的任一侧。等值不接受一元负号，`n = -1` 的测试断言既不是 `IndexScanAll` 也不是 `IndexRangeScan`。范围端点接受整数字面量上的一元负号。限定列、OR、`NOT BETWEEN`、参数、类型不匹配、NULL 和索引解释不了的额外谓词使该索引不能成为候选。选择顺序是更长前缀、不补扫、创建更早。整键等值且 `derive_secondary_index_key` 返回键时构造 `IndexScanAll`，执行器调用 `search_all`。编码放不下时该索引不产生候选。其余可路由谓词把 `secondary_range_bounds` 的哨兵放进 `IndexRangeScan`。世代 0 的整数索引不参与范围。全整数且不超过 4 列的世代 1 索引不补扫。`IndexManager::scan_range` 从内部节点下降，子页用显式栈按键序展开，成员判断用 `Key` 的现有顺序。子树按闭区间剪枝，这个闭区间盖住了 `equality_candidate_children` 写明的子树键域，剪枝不会丢掉落在哨兵内的键。可见性与 `IndexScanAllExecutor` 相同。补扫跳过派生不为空的行，再保留谓词成立的行。EXPLAIN 文本是 `IndexRangeScan <table> (index=<name>, columns=...)`，需要补扫时含 `omitted=scan`。矛盾区间 `a > 5 AND a < 1` 仍是 `IndexRangeScan`，结果为空。没有为单列 `1` 与 `(1, i64::MIN)` 改 `Key`。

**Follow-up Decision**

本轮 Acceptance 已满足。不要求当前 Cycle 修复，不创建 rework 或 replan。Iteration Plan 不变。展开 003。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-10-27-ms39-composite-index-and-range-access/iterations/003-fault-boundary-and-full-suite/000-initial.md`
