# Tasks

关联里程碑：MS40 的连接面（I015 的排序合并余项，I016 的降范围起步）。

触发条件已收窄并写入 proposal、design 和增量规格：只有写出的整数 `PRIMARY KEY` 才走排序合并，隐式首列整数键仍走哈希。ISS41 由 Iteration 002 承载。Iteration 001 的 `iterations/001-planner-selection/001-replan.md` Review Result 为 accepted，该文件的 Plan Context 与这个结论不改。目录行末尾的 `explicit_primary_key` 字节仍按已接受的方案：缺该字节的旧行读成没写出 PRIMARY KEY，不删这个字节。`iterations/002-signed-zero-match/000-initial.md` 与 `iterations/003-current-catalog-row-length/000-initial.md` 的 Review Result 都是 accepted，这两个文件不改。当前展开的 Cycle 是 `iterations/004-build-side-and-full-regression/000-initial.md`，Plan Context 为 ready。T3 和全量回归只按这个 Cycle 实施。不改已归档的加列变更。

## 1. 排序合并执行器

- [x] 1.1 T1：增加排序合并连接节点和执行器，经执行器入口证明多重集合与哈希连接相同。依赖：无。验收：`cargo test --test sort_merge_join_test` 退出码 0；`cargo test --lib sort_merge_join_plan_text_names_node` 与 `cargo test --lib sort_merge_join_memory_limit_rejects` 退出码 0；`cargo test --test nested_loop_join_test equi_on_keeps_hash_join_shape` 与 `non_equi_on_routes_to_nested_loop_join` 退出码 0；`cargo test --test explain_test explain_compound_nodes` 退出码 0，`l.id = r.rid` 的计划行仍以 `Join` 开头。本任务不把 SQL 改到排序合并。

## 2. 计划期触发

- [x] 2.1 T2：纯等值内连接的左右子计划都是 `PhysicalPlan::Scan`，每条等值键列都等于该侧 `primary_keys` 登记的列名，两侧 `primary_key_types` 都是 `Some(storage::page_format::ColumnType::Int)`，且两侧表都在 `PlanBuilder.explicit_primary_keys` 里时，发出排序合并节点。`explicit_primary_keys` 只在 `TableMeta.explicit_primary_key` 为真时由 `register_table_schema` 写入。该布尔只在 `CreateTableNode.primary_key.is_some()` 时为真，经 `create_table_with_constraints` 的最后一个参数写入 `TableMeta` 和 `CatalogRow`，并在 kind 段之后追加一个 `u8`（`1` 真，`0` 假；缺字节读为假）。`pk_column` 非空不算写出。类型未注册不成立，不用 `pk_type_known_non_int` 的否定。隐式首列整数键、`orders.customer_id = customers.id`、字符串主键、连接结果再连接仍发出 Hash Join。外连接和非等值仍是嵌套循环。`ORDER BY` 不参与选择，连接之上的 Sort 保留。依赖：T1。验收命令按 `iterations/001-planner-selection/001-replan.md` 的 Verification 逐条执行，不合并成一次无过滤的 `cargo test`，不跑全量。覆盖 `integer_pk_base_scans_select_sort_merge`、`written_integer_primary_key_stays_sort_merge_after_reopen`、`implicit_first_column_integer_key_stays_hash_after_reopen`、`self_join_integer_pk_selects_sort_merge`、`non_pk_equijoin_stays_hash`、`string_pk_equijoin_stays_hash`、`third_table_join_stays_hash`、`order_by_keeps_sort_merge_and_sort`、`explain_integer_pk_join_names_sort_merge`、`sort_merge_join_statement_memory_limit_53200` 和 `in_subquery_integer_pk_join_rejected`；`explain_compound_nodes` 里 `l.id = r.rid` 两侧都写出了整数主键，计划行以 `SortMergeJoin` 开头，其上的 `Sort (` 仍在；`equi_on_keeps_hash_join_shape` 与 `non_equi_on_routes_to_nested_loop_join` 退出码 0。另外这五条既有用例退出码 0：`pipeline_test` 的 `test_pipeline_join_with_null_keys`、`join_test` 的 `execute_sql_returns_err_not_partial_rows_on_join_where_overflow`、`subquery_alias_binding_test` 的 `exists_inner_join_both_arms_aliased`、`projection_expression_test` 的 `join_with_expression_rejected`、`outer_join_test` 的 `outer_join_routes_to_nested_loop_join`。无 `ORDER BY` 的既有用例若只因新选中的语句行序不同而失败，把断言改成多重集合比较，不改 SQL 去躲开排序合并。`equi_on_keeps_hash_join_shape` 必须仍是 `PhysicalPlan::Join`。`outer_join_routes_to_nested_loop_join` 的外连接必须仍是嵌套循环，其中 `l.id = r.rid` 的 `r.rid` 没有写出 `PRIMARY KEY`，内连接必须仍是 `PhysicalPlan::Join`。

## 3. 负零与正零的连接命中

- [x] 3.1 T4：哈希连接与排序合并对 `Float(-0.0)` 和 `Float(0.0)` 的匹配结果一致，并且与 `PartialEq` 一致。键向量里每个 `Value::Float`，若 `f == 0.0`，在进入 `JoinExecutor` 哈希表之前换成 `Value::Float(0.0)`；存入表中的行和输出行保持输入原值。`f == 0.0` 对 NaN 为假，不得把 NaN 收成同一键。不改 `Value::Hash`、`ValueRef::Hash`、`compare_floats`、`compare_join_value` 和 `sort.rs` 的 `compare_values`。不改聚合、半连接和反连接。依赖：T1。验收：`cargo test --test sort_merge_join_test signed_zero_matches_across_hash_and_sort_merge` 退出码 0。该测试在修改前必须失败：哈希连接缺少这对行，排序合并有这行。修改前若一次就通过，停止并返回计划。测试同时覆盖探测负零对构建正零、探测正零对构建负零，以及构建侧同时有两个零时的两行乘积。输出列包含两个键，并用 `to_bits()` 断言各自的原始位模式。

## 4. 当前代目录行长度

- [x] 4.3 T5：`legacy_db_write_upgrades_touched_table_row_to_current_layout` 与 `legacy_g2_db_opens_queries_and_writes` 中，被写到的表目录行长度从 `g1_tables_row_len + 9` 改为 `+ 10`。紧随其后的尾段从 `00×8 01` 改为 `00×8 01 00`。末字节 `0` 是这次升级写出的 `explicit_primary_key`，表示没写出 PRIMARY KEY。不改 `serialize_catalog_row` 和 `deserialize_catalog_row`。未被写到的表仍按原代际长度断言。全库没有第三条把当前代长度写成 `+ 9` 的断言。依赖：T2 已接受的尾字节。验收：改断言前这两条测试因长度多 1 字节失败；改完后 `cargo test --test legacy_compat_test legacy_db_write_upgrades_touched_table_row_to_current_layout` 与 `cargo test --test legacy_compat_test legacy_g2_db_opens_queries_and_writes` 退出码 0；`cargo test --lib legacy_row_without_explicit_primary_key_byte_reads_false` 与 `cargo test --lib catalog_row_explicit_primary_key_roundtrip` 退出码 0。

## 5. 哈希构建侧与收尾验证

- [x] 4.1 T3：在始终构建右输入的基线保存之后，按行数选择构建侧，并用 1000 对 100000 的 criterion 对比。语句是 `customers INNER JOIN orders ON orders.customer_id = customers.id`，留在哈希路径。依赖：T2、T4。平均时间不超过基线的 90% 则保留接线，否则撤回接线。基线与对比之间不得再改其他产品代码，因此 T4 的键归一必须已经在树里。保留接线时，探测侧非 NULL 键行按驻留计费，`small_right_large_left_join_succeeds_under_small_limit` 改为期望 64 字节上限下超限；撤回时该测试仍期望成功。验收：Act Response 同时有基线和对比的平均时间；撤回后 `cargo test --test hash_join_build_side_test` 仍证明生产路径构建右输入。
- [x] 4.2 按 M19 做一次分批全量回归。依赖：T1、T2、T3、T4 的生产代码已经停在本变更的最终形态（T3 若撤回，全量跑的是撤回后的树，其中仍包含 T4 的键归一），并且 T5 的断言已经改完。执行时按当时的 `tests/*.rs` 重算 N，不沿用历史个数。批次数或批大小越界则停止并返回计划，不得声明通过。验收：每一批退出码 0 且 failed 为 0。缺一批，或任一批 failed 不是 0，本任务失败。

## Iteration Plan

### Iteration 000: 排序合并连接执行器

- Tasks: T1
- Depends on: None
- Stable result: 直接执行排序合并节点时，多重集合与同一输入上的哈希连接相同。现有 SQL 仍不发出该节点。
- Verification boundary: `cargo test --test sort_merge_join_test`、`cargo test --lib sort_merge_join_plan_text_names_node`、`cargo test --lib sort_merge_join_memory_limit_rejects`、`cargo test --test nested_loop_join_test equi_on_keeps_hash_join_shape`、`cargo test --test nested_loop_join_test non_equi_on_routes_to_nested_loop_join`、`cargo test --test explain_test explain_compound_nodes`。定向测试可用默认并行度。不跑全量，不跑 criterion。`explain_compound_nodes` 在本 Iteration 仍要求计划行以 `Join` 开头。
- Diagnostic boundary: 新执行器、新计划变体，以及为编译和列、类型、拒绝行为补上的匹配臂。计划期分类和哈希构建侧的失败不在这里修。
- Non-goals: 计划期谓词、较小构建侧、全量回归。

### Iteration 001: 计划期选择

- Tasks: T2
- Depends on: Iteration 000
- Stable result: 两侧基表扫描且每个连接键都是该表写出的整数 `PRIMARY KEY` 时，SQL 发出排序合并节点，多重集合与哈希连接相同。隐式首列整数键、`orders.customer_id = customers.id`、字符串主键和连接结果再连接仍是哈希。外连接和非等值仍是嵌套循环。关闭再打开后，写出与否保持建表时的值。
- Verification boundary: T2 的定向测试经 `execute_sql` 与 `EXPLAIN` 观察计划形状、多重集合和错误文案，包含重新打开后的两条语句。目录尾字节用 `catalog.rs` 里点名的往返和缺字节用例。另外跑 `explain_compound_nodes`、两个 `nested_loop_join_test` 锚点，以及会改走排序合并或必须保持哈希形状的既有用例：`pipeline_test` 的 `test_pipeline_join_with_null_keys`、`join_test` 的 `execute_sql_returns_err_not_partial_rows_on_join_where_overflow`、`subquery_alias_binding_test` 的 `exists_inner_join_both_arms_aliased`、`projection_expression_test` 的 `join_with_expression_rejected`、`outer_join_test` 的 `outer_join_routes_to_nested_loop_join`。命令逐条执行，定向测试可用默认并行度。不跑全量，不跑 criterion。
- Diagnostic boundary: 显式主键位的写入、目录尾字节和分类谓词，以及该谓词使既有断言失效的计划文本或无 `ORDER BY` 行序。执行器多重集合错误回到后续 Iteration，不在本 Iteration 里改算法。隐式首列仍写入 `pk_column` 的行为不在这里改。
- Non-goals: 负零键归一、较小构建侧、全量回归。

### Iteration 002: 负零与正零的连接命中

- Tasks: T4
- Depends on: Iteration 001
- Stable result: 同一对 `Float(-0.0)` 与 `Float(0.0)` 输入上，哈希连接与排序合并都命中，多重集合相同，输出浮点保持输入位模式。计划期选择规则不变。
- Verification boundary: `cargo test --test sort_merge_join_test signed_zero_matches_across_hash_and_sort_merge`，以及 `cargo test --test sort_merge_join_test` 确认既有多重集合用例仍通过。定向测试可用默认并行度。不跑全量，不跑 criterion。修改前该新测试必须失败；一次就通过则停止并返回计划。
- Diagnostic boundary: `JoinExecutor::build_hash_key_left` 与 `build_hash_key_right` 交给哈希表的键。排序合并的比较函数、`Value::Hash`、计划期分类和构建侧选择的失败不在这里修。
- Non-goals: 改 NaN 排序；改聚合、半连接、反连接的分桶；较小构建侧；全量回归。

### Iteration 003: 当前代目录行长度

- Tasks: T5
- Depends on: Iteration 001
- Stable result: 被写到的 G1 `users` 行和 G2 `people` 行，长度等于 `g1_tables_row_len + 10`，尾段以表类型字节 `0x01` 和 `explicit_primary_key` 字节 `0x00` 结束。未被写到的表仍是原代际长度。缺尾字节的行仍读成没写出 PRIMARY KEY。`serialize_catalog_row` 仍在 kind 之后写出这个字节。
- Verification boundary: 改断言前先跑两条集成测试，确认失败点仍是长度多 1 字节。改完后逐条跑这两条，以及 `legacy_row_without_explicit_primary_key_byte_reads_false` 和 `catalog_row_explicit_primary_key_roundtrip`。定向测试可用默认并行度。不跑全量，不跑 criterion。全量预算仍只在最后一个 Iteration。
- Diagnostic boundary: `tests/legacy_compat_test.rs` 里这两条测试的当前代长度和尾段内容断言，以及 `g1_tables_row_len` 上说明当前布局尾段组成的注释。序列化、反序列化、负零键和构建侧的失败不在这里修。
- Non-goals: 删掉尾字节；改加列变更；改 Iteration 002 的 Cycle；负零键归一；较小构建侧；全量回归；未被写到的表的代际断言。

### Iteration 004: 较小构建侧与全量回归

- Tasks: T3，以及 4.2 的全量回归
- Depends on: Iteration 002、Iteration 003
- Stable result: 哈希连接要么仍构建右输入，要么在 1000 对 100000 的基准上平均时间至少低 10% 后按行数选择构建侧。排序合并连接、负零键归一和 T5 的目录行长度断言仍在。
- Verification boundary: T3 的定向测试和 criterion 命令。全量回归是本 Iteration 的完成判据，不是单独的产品结果。方式写在 `iterations/004-build-side-and-full-regression/000-initial.md`：执行时用 `LC_ALL=C ls tests/*.rs | sort` 重算 N，不沿用历史个数。第 00 批是 `cargo test --lib --bins -- --test-threads=1`。B = ceil(N / 12)。大小为 11 的批次数是 B×12−N，大小为 12 的批次数是 N−11×B。这两个数小于 0、任一已产生批次的文件数不在 8 到 12、或文件总数对不上 N，就停止并返回计划，不改分组，也不跑全量。文件批是 `cargo test --test … -- --test-threads=1`，批间串行。每一批退出码 0 且 failed 为 0。缺一批、任一批 failed 不是 0、或使用一次不带目标的 `cargo test`，都不得声明全量通过。定向测试可用默认并行度。新增用例各自使用独立 TempDir，不依赖顺序、并发度或批次边界。
- Diagnostic boundary: `JoinExecutor` 的构建侧选择、`benches/scale_bench.rs` 的新基准项，撤回是否把生产路径恢复为构建右输入，以及接线保留时 `small_right_large_left_join_succeeds_under_small_limit` 的超限断言。排序合并的错误结果、负零键归一和 T5 的目录行长度断言不在本 Iteration 修改。
- Non-goals: 代价模型、连接重排、结果流式化、DDL 缺陷、NaN 排序。不改 T5 已经对齐的目录行长度断言，也不改序列化。

## 平衡审计

五个 Iteration 分开。000 只交付可独立执行的新算法，计划期还碰不到它，故障在执行器。001 只改哪条 SQL 选中它。选出条件要能分辨写出的 `PRIMARY KEY` 和隐式首列，所以显式主键位和它的目录尾字节留在 001，故障仍在分类及其输入；`explain_compound_nodes` 的节点名变化属于这个分类结果。001 写明执行器多重集合错误不在该轮改算法，所以 ISS41 不放进 001，也不拿到 001 前面。001 当时不把目录字节单独拆出去：没有分类结果时，这个字节没有可以单独验收的语句行为。字节已经按 001 接受并写在序列化里。003 只改两条仍把当前代长度写成 kind 字节为止的断言，以及这两条测试里少写末尾 `0` 的尾段字面量。它不并进 002：002 的故障在哈希键，那个 Cycle 保持 ready，不改写。它不并进全量那一轮：全量要在这两条已经对齐之后跑，失败时才不会先撞上这条已知的长度差。002 只改哈希连接键对负零的归一，故障在 `HashMap` 查找；它不改计划期，也不改构建侧。004 只改哈希连接的构建侧，并带上本变更唯一的一次全量回归。时间门的基线要在 T4 已经进入代码之后保存，失败时撤回的是接线，不是 000 的执行器和 002 的键归一。负零这一轮单独成立：它有自己的稳定结果和验证，和分类、构建侧不是同一个故障域。没有为尚不存在的缺陷留空 Iteration。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 排序合并连接的多重集合与哈希连接相同 | T1 | `sort_merge_join_matches_hash_multiset` | Covered |
| 外连接与非等值仍走嵌套循环 | T1 | `non_equi_on_routes_to_nested_loop_join` | Covered |
| 排序合并节点可被计划文本识别 | T1 | `sort_merge_join_plan_text_names_node` | Covered |
| 排序合并连接遵守查询内存上限 | T1 | `sort_merge_join_memory_limit_rejects`（语句入口在 T2 的 `execute_sql`，SQLSTATE `53200`） | Covered |
| 纯等值 JOIN Hash 路径保持（无主键形状） | T1 | `equi_on_keeps_hash_join_shape` | Covered |
| 整数主键基表扫描走排序合并 | T2 | `integer_pk_base_scans_select_sort_merge` | Covered |
| 纯等值 JOIN Hash 路径保持（非主键列、字符串主键、三表第二步） | T2 | `non_pk_equijoin_stays_hash` | Covered |
| IN 子查询继续拒绝连接形状 | T2 | `in_subquery_integer_pk_join_rejected` | Covered |
| 纯等值 JOIN Hash 路径保持（不等行数与时间门） | T3 | `hash_join_probes_larger_side` | Covered |
| 接线保留时探测侧驻留并计费；撤回时左输入仍不整表驻留 | T3 | `small_right_large_left_join_succeeds_under_small_limit` | Covered |
| `Float(-0.0)` 与 `Float(0.0)` 在哈希连接和排序合并中都命中 | T4 | `signed_zero_matches_across_hash_and_sort_merge` | Covered |
| 被写到的旧目录行升级为当前布局时带 `explicit_primary_key` 字节；缺该字节的旧行仍读成没写出 PRIMARY KEY | T5 | `legacy_db_write_upgrades_touched_table_row_to_current_layout` | Covered |
