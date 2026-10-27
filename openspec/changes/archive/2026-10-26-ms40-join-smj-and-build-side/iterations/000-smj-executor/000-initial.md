# Iteration 000 / Cycle 000: 排序合并连接执行器

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 排序合并连接被执行时，多重集合与哈希连接相同；计划文本能认出该节点；驻留行超过查询内存上限时失败；外连接和非等值仍走嵌套循环；纯等值 SQL 在本 Iteration 仍发出 Hash Join
- Excluded scope: T2 的计划期谓词（两侧基表扫描且每个连接键都是该表已声明的整数主键）、T3 的较小构建侧与时间门、3.2 的全量回归、I016 的代价模型与连接重排、I065、ISS39、ISS40、ISS36 及其他 DDL 缺陷

**Objective**

直接执行一棵排序合并连接计划时，输出行的多重集合与同一输入上的哈希连接相同。现有 SQL 的计划形状不变。

**Investigation Facts**

- Starting point: 工作区相对 e9cb9d3 有未提交产品改动。调查读的是当前工作区。规划阶段没有跑测试，也没有跑全量。嵌套循环连接已在 MS09-T02 实施。规格 `openspec/specs/join-executor-selection/spec.md` 要求纯等值走哈希、非等值走嵌套循环。本变更不补做嵌套循环。触发条件已由用户选定，写入 design 的 D5；本 Cycle 不实现它。
- Current-State Evidence:
  - 分类入口是 `PlanBuilder::build_from_clause_with_projection`。内连接且 `is_pure_equi_join_on(on_clause)` 为真时得到 `PhysicalPlan::Join`，否则得到 `PhysicalPlan::NestedLoopJoin`。外连接三态都走后者。右外连接先交换输入，节点类型为 `NestedLoopJoinType::Left`。
  - `is_pure_equi_join_on` 在 `src/parser/planner/ddl_dml.rs`。AND 两侧都成立，且每一腿是 `Eq`、两侧都是裸标识符或两段限定名。字面量、表达式和其他比较符返回假。
  - `extract_join_conditions` 把等值腿收成 `JoinCondition`。两腿同侧时返回 `PlanError::InvalidQuery`，消息为 `JOIN ON condition must reference both sides of the join`。裸名列的 `ColumnRef.table` 为 `None`，限定列的 `table` 是归一后的真实表键。
  - 基表臂在这一步是 `PhysicalPlan::Scan`。无 WHERE 时把顶层 `Scan` 换成 `DataScan` 的改写不走进连接子节点。CTE、视图、派生表是 `DerivedScan`。
  - `JoinExecutor::next` 的相位是 `BuildRight` 然后 `Output`。右行进 `HashMap<Vec<Value>, Vec<Vec<Value>>>`。`build_hash_key_right` / `build_hash_key_left` 在键中有 `Value::Null` 时返回 `None`，该行不匹配。输出用 `output_sides` 决定取左行还是右行。`JoinExecutor::new` 接受公开的 `JoinConfig`，`with_budget` 是 `pub(crate)`。
  - `Value` 派生 `PartialEq`。`Int(2)` 不等于 `Float(2.0)`。`compare_values` 对这两个值、不兼容类型和 NaN 返回 `Equal`。哈希命中不能改用这个比较。`compare_values` 是 `sort.rs` 的私有函数。同类型次序是：`Int` 用 `i64::cmp`，`Float` 用 `partial_cmp` 的 `unwrap_or(Equal)`，`String` 与 `Bool` 用 `cmp`，`Date` 与 `Timestamp` 用 `cmp`。
  - `Value::to_key` 只为 `Int` 生成 `i64::to_be_bytes`。`ScanExecutor` 对有键表调用 `index_manager.scan_all()`。本 Cycle 不依赖扫描序。
  - `PlanBuilder.primary_keys` 是表名到单个主键列名，键和列名都已小写。`register_table_schema` 调用 `set_pk_column_type`，把 `TableMeta.columns` 里的 `crate::storage::page_format::ColumnType` 放进 `primary_key_types`。整数主键在这里是 `Some(ColumnType::Int)`。未注册是 `None`，不满足后续 Iteration 的类型条件。`pk_type_known_non_int` 对 `None` 返回假，不能用它的否定判断整数主键。`extract_primary_key` 在候选多于一列时返回 `MultiplePrimaryKey`。本 Cycle 不读这些表来改计划。
  - 计划变体必须补臂的位置：`pipeline.rs` 的 `plan_type_slots`、`collect_scan_tables`、`create_executor_from_plan`、`collect_join_key_pairs`、`extract_column_indices`。`plan_output_names` 对 `Join` 落入 `_ => None`，新节点保持这个回落。`query.rs` 的 `get_plan_output_columns`、连接结果表名 `"join_result"`、别名改路由、两处表达式项拒绝、`join_combined_columns`。`plan_format.rs` 的递归渲染和 `describe`。`correlated.rs` 的 `inject_correlated_values`。`subquery.rs` 的 `get_subquery_first_column` 对 `Join` 与 `NestedLoopJoin` 返回 `InSubqueryJoinUnsupported`，文案是 `IN subquery with JOIN is not supported`；新节点若落入末尾 `_`，会误报多列。
  - 查询内存入账是 `QueryBudget::charge`。行字节数是 `row_bytes`：非字符串值 16 字节，字符串 16 加字节长度。超限错误是 `StorageError::QueryMemoryExceeded`。`pipeline.rs` 的 `storage_error_sqlstate` 把该变体映射为 `53200`。`SortExecutor` 对每条保留的输入行调用 `budget.charge(row_bytes(&row))`，驻留到语句结束，没有对应的出账。
  - 既有形状锚点是 `tests/nested_loop_join_test.rs` 的 `equi_on_keeps_hash_join_shape` 和 `non_equi_on_routes_to_nested_loop_join`。前者的表是 `r(a INT)`、`s(b INT)`，没有主键。
  - `create_executor_from_plan` 与 `JoinExecutor::with_budget` 是 `pub(crate)`。`render_plan_tree` 在私有模块 `plan_format` 里，没有再导出。`tests/` 调用不到这三处。`QueryBudget` 是公开类型。`Executor`、`JoinExecutor`、`JoinConfig`、`PhysicalPlan`、`Value` 对 `tests/` 可见。
- Code and Critical Path: SQL 文本进入 `build_from_clause_with_projection` 的连接循环，纯等值内连接变成 `JoinNode`，`create_executor_from_plan` 的 `PhysicalPlan::Join` 臂构造 `JoinExecutor` 并调用 `with_budget`。排序合并节点在本 Iteration 不从这条 SQL 路径产生。多重集合测试在 `tests/` 里构造两个执行器，输入由测试自己的 `Executor` 提供。计划文本和内存上限在 crate 内的 `#[cfg(test)]` 观察，因为渲染函数和预算注入不是公开 API。

**Implementation Guidance**

执行器读完左右输入。键的任一列是 NULL 的行不参与匹配，也不入账。其余行按连接键排序。比较时先按 `Value` 变体区分，同类型再按上面列出的同类型次序。两组键只有在 `PartialEq` 下相等时才合并，合并结果是两组行的乘积。变体不同就不是同一组，即使 `compare_values` 会返回 `Equal`。组间次序不进入验收。输出列按 `output_sides` 从左右行取值，取法与 `JoinExecutor` 相同。保留的行用 `QueryBudget::charge(row_bytes(&row))` 入账。超限返回 `QueryMemoryExceeded`，不交出已经产生的结果行。未设置上限时 `charge` 对 limit 0 直接成功。

新计划变体的字段与 `JoinNode` 相同：左右子计划、`conditions`、`output_columns`、`output_sides`。每个已列出的、对 `Join` 有专门行为的匹配臂按 `Join` 臂处理列、类型、注入和拒绝。`describe` 的文本包含 `SortMergeJoin` 和等值条件。`is_pure_equi_join_on` 为真的 SQL 仍构造 `PhysicalPlan::Join`。执行器与 `JoinExecutor` 一样从 `executor/mod.rs` 导出。不要为了测试把 `pub(crate)` 的预算或渲染函数改成公开。

**Task Definitions**

### T1: 排序合并连接的多重集合与哈希连接相同

- Requirement/Scenario: 「排序合并连接的多重集合与哈希连接相同」及其四个场景；「外连接与非等值仍走嵌套循环」；「排序合并节点可被计划文本识别」的节点文本场景；「排序合并连接遵守查询内存上限」的失败变体；「纯等值 JOIN Hash 路径保持」里无主键的形状场景
- Depends on: None
- Targets: 新执行器模块；`PhysicalPlan` 的新变体；`src/executor/mod.rs` 的导出；`src/pipeline.rs` 的 `create_executor_from_plan`、`plan_type_slots`、`collect_scan_tables`、`collect_join_key_pairs`、`extract_column_indices`；`src/executor/plan_format.rs`；`src/executor/correlated.rs`；`src/parser/planner/query.rs` 的 `get_plan_output_columns`、`"join_result"` 臂、别名改路由、两处表达式项拒绝、`join_combined_columns`；`src/parser/planner/subquery.rs` 的 `get_subquery_first_column`
- Current behavior: 没有排序合并节点。纯等值 SQL 得到 `PhysicalPlan::Join`，非等值与外连接得到 `PhysicalPlan::NestedLoopJoin`。哈希连接对 NULL 键不匹配，并用 `PartialEq` 比较键。
- Required behavior: 对同一输入和同一组列=列等值条件，排序合并执行器的输出多重集合与 `JoinExecutor` 相同。重复键按乘积计数。NULL 键不匹配，NULL 与 NULL 也不匹配。`Int(2)` 与 `Float(2.0)` 不匹配。任一侧为空则结果为空。计划文本含 `SortMergeJoin` 和条件。保留行超过预算时返回 `QueryMemoryExceeded`，没有已交出的结果行。`ON r.a < s.b` 以及左外、右外、全外的计划和结果不变。`ON r.a = s.b` 且两列都不是主键时，计划仍是 Hash Join。`IN` 子查询若内部是新节点，仍返回 `InSubqueryJoinUnsupported`。
- Preserve: `extract_join_conditions` 的同侧拒绝；半连接和反连接；嵌套循环的谓词语义；哈希连接当前的右构建行为（本任务不改 `JoinPhase::BuildRight`）；`equi_on_keeps_hash_join_shape` 使用的无主键表
- Forbidden: 改 `is_pure_equi_join_on` 的真值集合；在 SQL 路径发出排序合并节点；按整数主键或 `ORDER BY` 选择连接算法；选择哈希构建侧；增加统计信息或连接重排；修改索引编码、DDL 或 ISS39、ISS40、ISS36 涉及的文件行为；为身份、批次或运行归属增加校验字段；把 `compare_values` 的跨类型 `Equal` 当作连接命中
- First test: 新建 `tests/sort_merge_join_test.rs`。先写 `sort_merge_join_matches_hash_multiset`：测试自己实现 `Executor`，给哈希执行器和排序合并执行器各一份相同的行。行里含重复键和 NULL。两个执行器的输出按多重集合比较。变更前该类型和执行器不存在，测试不能编译，这就是 RED。命令：`cargo test --test sort_merge_join_test sort_merge_join_matches_hash_multiset`。预期：编译失败。这个用例不打开数据库。
- GREEN condition: 同一命令退出码 0。随后 `cargo test --test sort_merge_join_test` 覆盖空输入和整数对浮点，退出码 0。`cargo test --lib sort_merge_join_plan_text_names_node` 退出码 0。`cargo test --lib sort_merge_join_memory_limit_rejects` 退出码 0。`cargo test --test nested_loop_join_test equi_on_keeps_hash_join_shape` 与 `cargo test --test nested_loop_join_test non_equi_on_routes_to_nested_loop_join` 退出码 0。`cargo test --test explain_test explain_compound_nodes` 退出码 0，计划行仍以 `Join` 开头。
- Verification: 上面几条定向命令可用默认并行度。通过条件是 cargo 退出码 0。`equi_on_keeps_hash_join_shape` 的两表没有主键，整数主键语句改走排序合并时它仍然通过。本任务用 `explain_compound_nodes` 守住 `l.id = r.rid`：该行仍以 `Join` 开头，其上的 `Sort` 仍在。这一行变成 `SortMergeJoin` 表示 SQL 路径被提前改掉，本任务失败。内存用例在 crate 内构造很小的 `QueryBudget`，断言错误是 `QueryMemoryExceeded` 且没有交出结果行。语句路径的 `53200` 由既有 `storage_error_sqlstate` 映射这个变体；经 `execute_sql` 观察 SQLSTATE 属于 T2，因为本任务的 SQL 还不发出该节点。不跑全量，不跑 criterion。
- Stop when: 为了让多重集合相同，必须把异类型或 NULL 当成相等；新节点不补臂就无法让既有 SQL 保持现在的计划；或者必须改哈希构建侧、计划期分类、索引或 DDL 才能让这些测试通过。停止并回到计划。不要在本任务里按整数主键改 SQL 计划。

**Invariants**

纯等值 SQL 的计划形状在本 Iteration 结束时仍是 Hash Join。非等值与外连接仍是嵌套循环。哈希连接仍先构建右输入。连接键相等与 `Value` 的 `PartialEq` 一致。打开数据库的新测试各自使用自己的 `TempDir`。

**Non-goals**

计划期触发谓词、较小构建侧、时间门、全量回归、代价模型、连接重排、结果流式化、DDL 缺陷。不为尚不存在的缺陷建 Iteration。

**Acceptance**

- 重复键、NULL、整数对浮点、空输入四个场景的多重集合与哈希连接一致，由 `tests/sort_merge_join_test.rs` 观察。
- 渲染含新节点的计划时，文本含 `SortMergeJoin` 和等值条件。
- 预算小于两侧保留行时，执行器返回 `QueryMemoryExceeded`，没有已交出的结果行。
- `equi_on_keeps_hash_join_shape` 与 `non_equi_on_routes_to_nested_loop_join` 仍通过。
- `explain_compound_nodes` 仍通过，`l.id = r.rid` 的计划行以 `Join` 开头。

**Verification**

执行器多重集合用同一进程里的两个执行器比较，输入由测试自己构造。这是执行器入口，不经过 SQL 谓词。外连接和非等值走现有 `execute_sql` 测试，命令见 T1 的 GREEN condition。计划文本调用 crate 内的 `render_plan_tree`。不新增判定脚本。

**Gate 2 Readiness**

通过。检查项全部 PASS。无 BLOCKED，无 SKIPPED。

用户 2026-10-26 的 goal 已选定触发条件并批准需求与计划。设计里的选择、接口和错误语义已经写明。RTM 没有 Missing。本 Cycle 只覆盖 T1，与 tasks、specs、design 的 Iteration 000 一致。Persisted Evidence 为 none。

**Persisted Evidence**

- Mode: none

命令、结果、修改的文件和符号写入 Act Response。T3 的两个 criterion 平均时间也写入当时的 Act Response，基准可以重跑。

**Risks and Notes**

`compare_values` 的兜底 `Equal` 不能当连接命中。组内行序不要求稳定。负数主键的 `scan_all` 字典序与有符号升序不一致；本 Cycle 的执行器自己排序，不读取扫描序。

另一名实施者正在改加列路径，其中包括 `pipeline.rs`。实施前若本 Cycle 点名的符号行为已经变了，按 Stop when 返回计划。加列目录面的改动不是本任务的需求。

`explain_compound_nodes` 的整数主键语句在 T2 才改成 `SortMergeJoin`，并保留其上的 `Sort`。本 Cycle 的通过条件是这条断言维持现状。

当前 Iteration 不修实施中才发现、且尚未写入本 Iteration 的缺陷。与排序合并执行器、哈希构建侧或本变更选择规则同一故障域的，交给后续 Iteration。其他故障域留在 MS40，不新建里程碑。

## Act Response

- Status: reported

**实际改动**

- 新增 `SortMergeJoinExecutor`（`src/executor/sort_merge_join.rs`），并由 `src/executor/mod.rs` 导出。`new` 接收 `JoinConfig`，`with_budget` 注入查询内存预算。
- `PhysicalPlan::SortMergeJoin` 复用 `JoinNode`。SQL 仍在 `build_from_clause_with_projection` 里对纯等值内连接构造 `PhysicalPlan::Join`。
- 与 `Join` 相同的匹配臂：`plan_type_slots`、`collect_scan_tables`、`create_executor_from_plan`、`collect_join_key_pairs`、`extract_column_indices`、`render_plan_tree` / `describe`、`inject_correlated_values`、`get_plan_output_columns`、`"join_result"`、别名改路由、两处表达式项拒绝、`join_combined_columns`、`get_subquery_first_column`。
- `plan_output_names` 对 `Join` 仍落入 `_ => None`，新节点保持这个回落。
- 测试：`tests/sort_merge_join_test.rs`（重复键与 NULL、空输入、整数对浮点）；`sort_merge_join_plan_text_names_node`；`sort_merge_join_memory_limit_rejects`。

执行器读完两侧后，键中任一列是 NULL 的行不匹配、不入账。其余行按连接键排序：先按 `Value` 变体，同类型再按整数、浮点、字符串、布尔、日期、时间戳的既有次序。只有 `PartialEq` 相等的键做乘积。`Int(2)` 与 `Float(2.0)` 不是同一组。输出列按 `output_sides` 取值。保留行以 `QueryBudget::charge(row_bytes(&row))` 入账；超限时 `next` 返回 `QueryMemoryExceeded`，在此之前不交出结果行。

**与计划的偏差**

浮点 NaN 与有序数在排序比较中不相等（NaN 排在有序数之后，NaN 与 NaN 相等）。计划写的是 `partial_cmp` 的 `unwrap_or(Equal)`。那个结果不是全序，不能交给 `sort_by`。合并仍只认 `PartialEq`，NaN 不与任何键合并，组间次序也不进入验收。

**自检**

Gate 3 通过。四个多重集合场景、计划文本、内存失败、外连接与非等值、无主键纯等值仍为 Hash Join，以及 `IN` 子查询对新节点返回 `InSubqueryJoinUnsupported`，都在当前代码里。先行测试先观察到编译失败，再改产品代码。验证命令覆盖通过条件。这些编译没有新增 rustc 警告。没有为身份或批次增加校验字段。

无 Critical 或 Important。遗留 Minor：无。

**验证**

| 命令 | 退出码 | 结果 |
|---|---|---|
| `cargo test --test sort_merge_join_test sort_merge_join_matches_hash_multiset`（实现前） | 101 | 编译失败：`SortMergeJoinExecutor` 不存在 |
| 同一命令（实现后） | 0 | 1 passed，0 failed |
| `cargo test --test sort_merge_join_test` | 0 | 3 passed，0 failed |
| `cargo test --lib sort_merge_join_plan_text_names_node` | 0 | 1 passed，0 failed |
| `cargo test --lib sort_merge_join_memory_limit_rejects` | 0 | 1 passed，0 failed |
| `cargo test --test nested_loop_join_test equi_on_keeps_hash_join_shape` | 0 | 1 passed，0 failed |
| `cargo test --test nested_loop_join_test non_equi_on_routes_to_nested_loop_join` | 0 | 1 passed，0 failed |
| `cargo test --test explain_test explain_compound_nodes` | 0 | 1 passed，0 failed。`l.id = r.rid` 的计划行仍以 `Join` 开头 |

定向命令按写出的顺序逐条执行，没有改成一次无过滤的 `cargo test`。未跑全量，未跑 criterion。

**Persisted Evidence**

None required。

**Experience Candidates**

None。

**Issue 候选**

None。

**未解决问题**

无。

## Plan Review

- Review Result: accepted

**Findings**

验收已满足。独立读过执行器、计划匹配臂、测试和本轮 diff。Act Response 里的定向测试结论沿用：这些文件的修改时间早于本 Cycle 文件写入，其后没有新的提交改写它们。

非阻塞 Minor：浮点 NaN 的排序比较。计划写的是 `partial_cmp` 的 `unwrap_or(Equal)`。这个关系不是全序，不能交给 `sort_by`。实现把 NaN 排在有序数之后，NaN 与 NaN 在排序比较中相等。合并仍只认 `PartialEq`，所以 NaN 不与任何键合并，包括另一个 NaN。组间次序不在验收里。偏差原因是计划给出的同类型次序不能直接用于排序；Act 的处理与这条限制一致。

**Evidence**

- `SortMergeJoinExecutor`（`src/executor/sort_merge_join.rs`）读完两侧。键里有 `Value::Null` 的行不进入排序，也不调用 `charge`。其余行先按变体序号，再按同类型次序排序。只有键向量 `==` 才做乘积。`Int(2)` 与 `Float(2.0)` 变体序号不同，不会相乘。输出按 `output_sides` 取列，取法与 `JoinExecutor::build_output_row` 相同。
- 超限发生在 `prepare` 交出任何行之前。`QueryBudget::charge` 在已用量加上本次字节数超过上限时返回 `QueryMemoryExceeded`。单列 `Int` 行是 16 字节；预算 16 时第二行入账失败。`sort_merge_join_memory_limit_rejects` 对第一次 `next` 使用 `expect_err`。
- 纯等值内连接仍由 `PlanBuilder::build_from_clause_with_projection` 构造 `PhysicalPlan::Join`。哈希节点的 `describe` 以 `Join [Inner]` 开头。`tests/explain_test.rs` 的 `explain_compound_nodes` 要求某行 `trim_start().starts_with("Join")`。`SortMergeJoin` 不满足这个前缀。
- 计划点名的匹配臂已与 `Join` 使用同一处理：`plan_type_slots` 与 `collect_scan_tables`，`create_executor_from_plan`，`collect_join_key_pairs` 与 `extract_column_indices`，`render_plan_tree` 与 `describe`，`inject_correlated_values`，`get_plan_output_columns` 与 `"join_result"`，别名改路由，两处表达式项拒绝，`join_combined_columns`，以及 `get_subquery_first_column`。`plan_output_names` 对两个节点都落入 `_ => None`。`Join` 执行臂仍构造 `JoinExecutor` 并调用 `with_budget`。`with_budget` 保持 `pub(crate)`。
- `src/executor/join.rs` 未改，`JoinPhase::BuildRight` 仍在。`pipeline.rs` 相对 HEAD 还有本任务以外的未提交差异；本轮只核对了排序合并匹配臂。
- 沿用本 Cycle「验证」表：实现前 `sort_merge_join_matches_hash_multiset` 退出码 101；实现后退出码 0，1 passed。`cargo test --test sort_merge_join_test` 退出码 0，3 passed。两个 `--lib` 用例各 1 passed。两个 `nested_loop_join_test` 锚点各 1 passed。`explain_compound_nodes` 退出码 0，1 passed，`l.id = r.rid` 的计划行仍以 `Join` 开头。Persisted Evidence 为 none。未因缺少证据目录记问题。

**Follow-up Decision**

五个验收点都有对应代码，以及沿用通过的测试。NaN 的排序比较不改变本 Iteration 的验收边界，也不需要新的执行依据。Iteration Plan 不改。不创建后继 Cycle。按本轮审查指令，不展开下一 Iteration。

**Next Cycle**

None

**Next Iteration**

未展开。`tasks.md` 的下一逻辑单元仍是 Iteration 001（计划期选择）。

**Issue 候选**

`Value` 派生 `PartialEq`，`Float(-0.0)` 与 `Float(0.0)` 相等；`Hash` 使用 `to_bits()`，二者哈希不同。哈希连接经 `HashMap` 查找时，负零可能对不上正零。排序合并按 `PartialEq` 会合并这对键。本轮四个多重集合场景没有这对值，SQL 也仍不发出排序合并节点。不在本 Cycle 修改 `Value` 或 `JoinExecutor`。未登记。
