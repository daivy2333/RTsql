# Iteration 002 / Cycle 000: 哈希连接键把负零收成正零

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: Iteration 000 的 `SortMergeJoinExecutor` 与 `JoinExecutor`。Iteration 001 已接受的计划期规则：只有两侧都是基表扫描、且每个连接键都是该表写出的整数 `PRIMARY KEY` 时才发出排序合并。`Float(-0.0)` 与 `Float(0.0)` 在 `Value` 的 `PartialEq` 下是同一键。哈希连接与排序合并都要命中。输出单元格保留输入的原始值。
- Excluded scope: T3 的较小构建侧与时间门、4.2 的全量回归、`Value::Hash`、`ValueRef::Hash`、NaN 排序、聚合、半连接、反连接、计划期分类、目录尾字节。不改加列变更目录。不改 `iterations/001-planner-selection/001-replan.md` 的 Plan Context 和 Review Result。

**Objective**

同一对输入上，`Float(-0.0)` 与 `Float(0.0)` 在哈希连接和排序合并里都命中，多重集合相同。输出里的两个浮点保持输入的位模式。计划期选择不变。哈希连接仍先构建右输入。

**Background**

ISS41 记录 `Value` 的派生 `PartialEq` 把两个零看成相等，`Hash` 使用 `to_bits()`，两个零的哈希不同。Iteration 000 的审查把它留在连接命中上，不在那个 Cycle 修改 `Value` 或 `JoinExecutor`。Iteration 001 只改哪条 SQL 选中排序合并，负零不在那一轮修。`tasks.md` 把 T4 放在本 Iteration。修法已经写在 `tasks.md` 和 design 的 D6：只归一 `JoinExecutor` 交给哈希表的键。

**Investigation Facts**

- Starting point: Iteration 001 的最终 Cycle 是 `iterations/001-planner-selection/001-replan.md`。Act Response 为 `reported`。Review Result 为 `accepted`。该文件的 Plan Context 与这个结论保持不动。那次 Act Response 记录：修改判定前两条锚点退出码 101；修改后第 3 到第 15 条退出码 0、failed 为 0，其中 `cargo test --test sort_merge_join_test` 为 14 passed。审查沿用该结果，没有重跑。审查写明 `src/executor/join.rs`、`src/executor/value.rs`、`src/executor/value_ref.rs` 相对 HEAD 没有未提交差异。本次没有重跑这些命令，没有跑全量，没有跑 criterion。本次只重读了 T4 要碰的哈希键路径、排序合并的相等判断，以及 `tests/sort_merge_join_test.rs` 的执行器夹具。
- Current-State Evidence:
  - `JoinExecutor`（`src/executor/join.rs`）的 `right_hashmap` 类型是 `HashMap<Vec<Value>, Vec<Vec<Value>>>`，由 `HashMap::new()` 构造，没有换成别的 hasher。`next` 先进入 `BuildRight`：`build_hash_key_right` 的返回值作为键，原始 `row` 作为行，一起交给 `charge_bucket_insert`。然后进入 `Output`：`build_hash_key_left` 的返回值放进 `current_match_key`，再 `right_hashmap.get`。命中后 `build_output_row` 从左行和桶里的右行取输出列。键和行是两个值。
  - `build_hash_key_right` 与 `build_hash_key_left` 按条件列把单元格 `clone` 进 `Vec<Value>`。向量里有 `Value::Null` 时返回 `None`，该行不插入、也不探测。两个函数都原样返回克隆出的键，没有改写浮点。它们是私有的，唯一调用者是 `JoinExecutor::next`。
  - `charge_bucket_insert`（`src/executor/query_budget.rs`）用 `hash_key` 做 `HashMap` 的键，把 `row` 放进桶。新桶入账键字节加行字节，已有桶只再入账行字节。本 Cycle 不改这个函数。归一后的键仍是一个 `Value::Float`，行字节口径不变。
  - `Value` 派生 `PartialEq`，所以 `Float(-0.0) == Float(0.0)`。`Eq` 是空实现，比较走这个 `PartialEq`。`Hash` 对 `Value::Float` 调用 `to_bits()`。负零的位模式是 `0x8000_0000_0000_0000`，正零是 `0`。`ValueRef::Hash` 对浮点同样调用 `to_bits()`。`HashMap` 先按哈希分桶，桶内再用 `Eq`。两个零哈希不同，查找通常进不了对方的桶。`RandomState` 下，不同哈希的标签若碰巧相同，`Eq` 为真就会命中。这对键还没有用测试跑过。
  - `SortMergeJoinExecutor` 的 `compare_floats` 对两个非 NaN 浮点使用 `partial_cmp`。负零和正零得到 `Equal`。`merge_product` 还要求键向量 `==`，`group_end` 也用 `==`。这对键会进入同一个乘积。本 Cycle 不改 `compare_floats`、`compare_join_value` 和 `merge_product`。
  - `src/pipeline.rs` 的 `PhysicalPlan::Join` 臂构造 `JoinExecutor::new(...).with_budget(...)`。语句上的哈希连接走同一个 `next`。`PhysicalPlan::SortMergeJoin` 构造的是 `SortMergeJoinExecutor`。一条 SQL 只会发出其中一个节点。
  - `src/executor/semi_join.rs` 有自己的 `build_left_key` 与 `build_right_key`。`anti_join.rs` 和 `aggregate.rs` 各自持有 `HashMap<Vec<Value>, _>`。这些路径用 `Value::Hash`。本 Cycle 不改它们。
  - `src/executor/sort.rs` 的 `compare_values` 对两个浮点使用 `partial_cmp(...).unwrap_or(Ordering::Equal)`。NaN 因此在排序里落到 `Equal`。本 Cycle 不改这个函数。
  - `tests/sort_merge_join_test.rs` 现有 14 个 `tokio::test`。执行器用 `RowSource` 喂行，再分别构造 `JoinExecutor` 和 `SortMergeJoinExecutor`。`join_config` 的输出列是两侧标记，列下标是 1，键不在结果里。`multiset` 用 `HashMap<Vec<Value>, usize>` 计数，哈希同样是 `to_bits()`，相等同样是 `PartialEq`。文件里还没有 `signed_zero_matches_across_hash_and_sort_merge`。
- Code and Critical Path: 右行进入 `build_hash_key_right`，返回的键进入 `charge_bucket_insert`，原始行进入桶。左行进入 `build_hash_key_left`，返回的键用于 `HashMap::get`。输出从原始左行和桶中的原始右行复制。归一只发生在这两个函数返回、即将交给哈希表的键向量上。`f == 0.0` 对负零和正零都为真，对 NaN 为假。换成 `Value::Float(0.0)` 之后，两个零的 `to_bits()` 都是 `0`，查找进入同一个桶。桶里的行和输出行仍是输入原值。

**Implementation Guidance**

两个键函数在克隆出向量之后、返回 `Some` 之前，把其中每个 `Value::Float(f)` 且 `f == 0.0` 的元素换成 `Value::Float(0.0)`。含 `Value::Null` 的向量仍返回 `None`。不要把替换写回 `charge_bucket_insert` 的 `row` 参数，也不要写回 `build_output_row` 使用的左右行。`next` 继续把这两个函数的返回值交给哈希表，继续把原始行交给桶和输出。

两个函数都要换。只换构建键或只换探测键，另一个方向的查找仍然对不上。是否抽一个私有函数由实施决定，名字不构成完成条件。

**Task Definitions**

### T4: 负零与正零在两条连接路径上都命中

- Requirement/Scenario: 「纯等值 JOIN Hash 路径保持」的「负零与正零在哈希连接中命中」；「排序合并连接的多重集合与哈希连接相同」的「负零与正零两边一致」
- Depends on: T1
- Targets: `src/executor/join.rs` 的 `JoinExecutor::build_hash_key_left` 与 `JoinExecutor::build_hash_key_right`；`tests/sort_merge_join_test.rs` 的 `signed_zero_matches_across_hash_and_sort_merge`
- Current behavior: 两个键函数把条件列原样放进键向量。`Float(-0.0)` 与 `Float(0.0)` 的 `PartialEq` 为真，`Hash` 不同。哈希连接的 `HashMap::get` 因此通常找不到对方的桶。排序合并在 `partial_cmp` 与 `==` 下把这对键乘积。输出函数复制的是原始行。现有 14 个测试没有这对键。
- Required behavior: 交给 `right_hashmap` 插入和 `get` 的键向量里，每个 `Value::Float(f)` 在 `f == 0.0` 时变为 `Value::Float(0.0)`。负零和正零都满足这个比较。NaN 不满足，键里的 NaN 保持原值。含 `Null` 的键仍然不入表、不匹配。桶里保存的行，以及 `build_output_row` 交出的单元格，保持输入原值。
  - 哈希连接仍先读完右输入并建表，再按左输入逐行探测。
  - 测试直接构造两个执行器，使用与现有执行器用例相同的 `RowSource`。左右各一行或右输入两行，唯一列是键。输出列是左键和右键，`column_index` 都是 0。不复用 `join_config`，因为它不把键放进输出。
  - 三个情形都要出现在 `signed_zero_matches_across_hash_and_sort_merge` 里。探测侧是左输入，构建侧是右输入。
    - 左键 `Float(-0.0)`，右键 `Float(0.0)`。两边都产出一行。左键位模式是 `0x8000_0000_0000_0000`，右键位模式是 `0`。
    - 左键 `Float(0.0)`，右键 `Float(-0.0)`。两边都产出一行。左键位模式是 `0`，右键位模式是 `0x8000_0000_0000_0000`。
    - 左键 `Float(-0.0)`，右输入同时有 `Float(-0.0)` 和 `Float(0.0)`。两边都产出两行。位模式多重集合是 `(0x8000_0000_0000_0000, 0x8000_0000_0000_0000)` 与 `(0x8000_0000_0000_0000, 0)`。组内顺序不作要求，比较前把 `(左位, 右位)` 排成稳定顺序。
  - 每个情形用 `to_bits()` 读取输出里的两个 `Value::Float`。`Value` 的 `==` 会把负零和正零看成相等，不能用来证明位模式还在。不要用文件里的 `multiset` 给含有负零的结果计数；那个 `HashMap<Vec<Value>, _>` 与连接哈希表有同一对 `Hash` / `Eq`。
  - 每个情形在同一次断言里比较排序合并的位模式多重集合、哈希连接的位模式多重集合和期望。修改前的失败输出要同时呈现两边：排序合并与期望一致，哈希连接缺少这对行。
- Preserve: 计划期选择、目录尾字节、排序合并的比较和乘积、`Value::Hash`、`ValueRef::Hash`、`sort.rs` 的 `compare_values`、聚合与半连接和反连接的分桶、哈希连接先构建右输入、NULL 键不匹配、整数与同数值浮点不匹配。已有 14 个测试的 SQL 和断言保持原样。`charge_bucket_insert` 的计费口径保持原样。
- Forbidden: 不改 `src/executor/value.rs`、`src/executor/value_ref.rs`、`src/executor/sort_merge_join.rs`、`src/executor/sort.rs`、`src/executor/semi_join.rs`、`src/executor/anti_join.rs`、`src/executor/aggregate.rs`、`src/executor/query_budget.rs`。不改计划期、pipeline 的节点选择和加列变更目录。不改 `001-replan.md`。不跑全量，不跑 criterion。不把输出行里的负零改写成正零。不把 NaN 收成同一个键。
- First test: 先只增加 `signed_zero_matches_across_hash_and_sort_merge`，不要先改 `src/executor/join.rs`。测试能编译并运行之后，执行一次 `cargo test --test sort_merge_join_test signed_zero_matches_across_hash_and_sort_merge`。预期退出码 101：排序合并有这对行，哈希连接缺少这对行。修改前若这一次退出码是 0，停止并返回计划，不要再跑一次来换一个失败，也不要开始改产品代码。失败若是编译错误，只改测试，直到同一次运行呈现出上述两边的结果。失败若是排序合并也缺少这对行，或者断言在比较两个执行器之前就返回，停止并返回计划。
- GREEN condition: 同一条过滤命令退出码 0，failed 为 0。随后 `cargo test --test sort_merge_join_test` 退出码 0，failed 为 0，passed 为 15。
- Verification: 见本 Cycle 的 Verification。过滤命令在修改前按 First test 失败；修改后两条命令都退出码 0 且 failed 为 0。
- Stop when: 修改前该测试一次就通过；修改前的失败看不出「排序合并有这对行、哈希连接缺少」；为了通过需要改 `Value::Hash`、`ValueRef::Hash`、排序合并比较、`compare_values`、半连接、反连接、聚合或计划期；为了让 `to_bits()` 通过需要改写输出行；`f == 0.0` 把 NaN 也收成了 `0.0`；键向量里只有第一个浮点被归一。这些情况返回计划。

**Invariants**

连接键是否命中与 `Value` 的 `PartialEq` 一致。负零和正零命中。NaN 不因为这次归一变成同一个哈希键。输出单元格保持进入执行器时的值。NULL 键不匹配。整数与同数值浮点不匹配。哈希连接仍先构建右输入、再探测左输入。计划期仍只在写出的整数 `PRIMARY KEY` 上选择排序合并。

**Non-goals**

较小构建侧、全量回归、NaN 的排序比较、聚合与半连接和反连接的分桶、改 `Value::Hash`、计划期选择、目录字节。

**Acceptance**

- `signed_zero_matches_across_hash_and_sort_merge` 的三个情形里，哈希连接和排序合并的位模式多重集合都与 Task Definition 的期望相同。
- 探测负零对构建正零、探测正零对构建负零，各有一行，且两个键的 `to_bits()` 分别是输入的负零位和正零位。
- 构建侧同时有负零和正零、探测侧是负零时，两边都是两行。左键位保持负零，右键位一个是负零、一个是正零。
- 修改前这条测试退出码 101，且失败呈现为排序合并有这对行、哈希连接缺少。修改前一次通过则本 Cycle 不实施产品修改。
- `cargo test --test sort_merge_join_test` 在修改后退出码 0，failed 为 0，passed 为 15。原有多重集合、空输入、整数与浮点不匹配，以及 Iteration 001 的语句用例仍通过。
- `src/executor/value.rs`、`src/executor/value_ref.rs`、`src/executor/sort_merge_join.rs`、`src/executor/sort.rs` 没有为本任务改动。

**Verification**

命令按顺序执行。每条是一次独立的 `cargo test`。定向测试使用默认并行度。不跑全量，不跑 criterion。

1. 修改产品代码前：`cargo test --test sort_merge_join_test signed_zero_matches_across_hash_and_sort_merge`。预期退出码 101。排序合并有这对行，哈希连接缺少这对行。这一次若退出码 0，停止并返回计划。
2. 修改后：`cargo test --test sort_merge_join_test signed_zero_matches_across_hash_and_sort_merge`。退出码 0，failed 为 0。
3. 修改后：`cargo test --test sort_merge_join_test`。退出码 0，failed 为 0，passed 为 15。

第 1 条只跑一次。不用第二次运行来确认失败是不是碰巧。第 3 条若 failed 不为 0，或 passed 不是 15，任务未完成。

**Gate 2 Readiness**

PASS。无 BLOCKED，无 SKIPPED。2026-10-26 用户指令展开本 Iteration，T4 的修法以 `tasks.md` 已写的为准，检查标准不降低；修改前 `signed_zero_matches_across_hash_and_sort_merge` 必须失败，一次通过则实施停止并返回计划。需求、场景和 D6 已在本变更中获批。本次重读后，任务、规格、设计与本 Cycle 的执行依据一致。RTM 中 T4 那一行保持 Covered。Persisted Evidence 为 none。全量回归仍在 Iteration 003，本轮不跑。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- `HashMap::new()` 使用 `RandomState`。修改前两个零的哈希不同，查找仍可能因为标签相同且 `Eq` 为真而命中。该测试一次通过只说明这次分桶撞上了，不说明缺陷不存在。按 First test 停止并返回计划。
- 半连接、反连接和聚合仍用 `Value::Hash`。两个零在那些路径上的分桶本 Cycle 不改，也不另开缺陷；ISS41 的连接命中由 T4 处理。
- `f == 0.0` 对 NaN 为假。不要另加把 NaN 收成同一键的条件。
- 这条测试不打开数据库，不使用 `TempDir`。

## Act Response

- Status: reported

`JoinExecutor` 交给哈希表的键里，每个 `f == 0.0` 的浮点收成 `Value::Float(0.0)`。同一对负零和正零上，哈希连接和排序合并都命中，输出单元格仍是输入的位模式。

### 实际改动

- `JoinExecutor::normalize_signed_zero_key`：遍历键向量，把每个 `Value::Float(f)` 且 `f == 0.0` 的元素换成 `Value::Float(0.0)`。NaN 不满足这个比较，保持原值。
- `JoinExecutor::build_hash_key_right` 与 `JoinExecutor::build_hash_key_left`：含 `Value::Null` 时仍返回 `None`；否则把归一后的键交给哈希表。`charge_bucket_insert` 的 `row` 和 `build_output_row` 使用的左右行不经过这个函数。
- `tests/sort_merge_join_test.rs` 新增 `signed_zero_matches_across_hash_and_sort_merge`。左右输入经 `RowSource` 直接构造两个执行器。输出列是左键和右键，`column_index` 都是 0。三个情形各用一次 `assert_eq` 比较排序合并的位模式、哈希连接的位模式和期望。位模式来自 `to_bits()`，比较前按 `(左位, 右位)` 排序。

### 与计划的偏差

两个键函数共用 `normalize_signed_zero_key`。计划允许抽出私有函数。

修改前第一次 `cargo test --test sort_merge_join_test signed_zero_matches_across_hash_and_sort_merge` 没有运行测试：`pos` 在第二次情形移动后，第三次情形再次使用，编译错误 `E0382`，退出码 101。只给该测试补了 `clone`。下一次同一条命令才是行为失败，这次失败没有重跑。

### 验证

1. 测试能编译后、修改 `src/executor/join.rs` 前：`cargo test --test sort_merge_join_test signed_zero_matches_across_hash_and_sort_merge`。退出码 101。0 passed，1 failed，14 filtered out。断言左侧 `sort_merge: [(9223372036854775808, 0)]`、`hash_join: []`，右侧两项都是 `[(9223372036854775808, 0)]`。`9223372036854775808` 是 `0x8000_0000_0000_0000`。排序合并有这对行，哈希连接没有。
2. 修改后：同一条过滤命令。退出码 0。1 passed，0 failed，14 filtered out。
3. 修改后：`cargo test --test sort_merge_join_test`。退出码 0。15 passed，0 failed，0 filtered out。

没有跑全量，没有跑 criterion。

### Persisted Evidence

None required。

### Experience Candidates

None。

### Issue 候选

None。

### 未解决问题

无。

## Plan Review

- Review Result: accepted

**Findings**

无阻塞项，无 Minor。

`build_hash_key_right` 与 `build_hash_key_left` 在键含 `Value::Null` 时仍返回 `None`，否则调用 `normalize_signed_zero_key`。该函数遍历键向量，把每个 `Value::Float(f)` 且 `f == 0.0` 的元素换成 `Value::Float(0.0)`。NaN 不满足这个比较，留在键里。`next` 仍先进入 `BuildRight`：归一后的键和原始右行交给 `charge_bucket_insert`，探测用左键做 `get`，`build_output_row` 从原始左行和桶里的原始右行复制。相对 HEAD，`join.rs` 只改了这两处返回和这个新函数。

`Value::Hash` 与 `ValueRef::Hash` 仍对浮点调用 `to_bits()`。`sort.rs` 的 `compare_values` 对两个浮点仍是 `partial_cmp(...).unwrap_or(Ordering::Equal)`。`sort_merge_join.rs` 没有把零收成正零。`semi_join.rs`、`anti_join.rs`、`aggregate.rs`、`query_budget.rs` 没有本任务的改动。

`signed_zero_matches_across_hash_and_sort_merge` 用 `RowSource` 和 `signed_zero_config` 直接构造两个执行器。输出列下标都是 0。三个情形分别是左负零对右正零、左正零对右负零，以及左负零对右负零加正零。比较前按 `(左位, 右位)` 排序，位模式来自 `to_bits()`，没有用 `multiset`。文件里现有 15 个 `tokio::test`。

修改前第一次过滤命令是编译错误 `E0382`：`pos` 被移动后再次使用。Act 只给该测试补了 `clone`。下一次同一命令才是行为失败，且没有重跑。任务定义允许编译错误只改测试，直到同一次运行呈现排序合并有这对行、哈希连接缺少这对行。这次 `clone` 复制的仍是原来的浮点，三个情形的输入位模式没有变。行为失败已经观察到，验收不因此改变。这是非阻塞记录。

**Evidence**

验证沿用本 Cycle Act Response，没有重跑。`join.rs` 的修改时间是 2026-10-26 21:59:37，测试文件是 21:59:10，本 Cycle 文件在审查前是 22:02:00。两份源文件都早于 Act Response 写入，其后没有新的 git 提交。当前 `join.rs` diff 与 Act 声明的改动一致。

沿用的结果：

1. 改 `join.rs` 前，`cargo test --test sort_merge_join_test signed_zero_matches_across_hash_and_sort_merge` 退出码 101。0 passed，1 failed，14 filtered out。断言左侧 `sort_merge: [(9223372036854775808, 0)]`、`hash_join: []`，右侧两项都是 `[(9223372036854775808, 0)]`。`9223372036854775808` 是 `0x8000_0000_0000_0000`。
2. 改后同一过滤命令退出码 0。1 passed，0 failed，14 filtered out。
3. 改后 `cargo test --test sort_merge_join_test` 退出码 0。15 passed，0 failed。

Persisted Evidence 为 none。未要求证据目录。

**Follow-up Decision**

既有验收在当前代码里成立，没有缺口，所以接受。Iteration Plan 不改。

`tasks.md` 的 Map 里还有 Iteration 003 和 Iteration 004。这次审查只结束本 Cycle，不展开下一 Iteration，也不写后继 Cycle。

**Next Cycle**

None

**Next Iteration**

None
