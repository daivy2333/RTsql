# Iteration 000 / Cycle 000: 测量整组复制并裁定

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2
- Inherited scope: `avoidable-copy` 的测量与 10% 裁定。产品代码保持测量前的行为。
- Excluded scope: T3 与 Iteration 001。不改 `src/`、`Cargo.toml`。不测量分配器，不测量 `RowLockTable`，不新增诊断指标。

**Objective**

一次 release 测试打印规格里的纳秒数和行数。用这些整数裁定：要么点名唯一一处留到下一轮，要么写明保留原实现并结束产品改动。本 Cycle 不实现那一处。

**Investigation Facts**

- Starting point: 分支 `changes/2026-11-19-ms53-avoidable-copy`，基点 `f940925831e5e78f201bd27a09173cbaa2bad2bc`（父提交 `8063854d3c7dfd721bb018b35eb189c585006bad`）。没有其他活跃变更。变更前没有跑过 `tests/ms53_copy_measure.rs`，该文件尚不存在。
- Current-State Evidence: `Value::String` 的负载是 `String`（`src/executor/value.rs`）。`JoinExecutor::next` 用 `right_hashmap.get` 取桶再按下标读行，不克隆整桶（`src/executor/join.rs`）。`NestedLoopJoinExecutor::next` 对每一对执行 `left_row.iter().cloned().chain(right_row.iter().cloned())` 再 `predicate.evaluate`（`src/executor/nested_loop_join.rs`）。`SortExecutor::next` 克隆 `sorted_rows[position]`（`src/executor/sort.rs`）。`SortMergeJoinExecutor::next` 克隆 `outputs[output_index]`（`src/executor/sort_merge_join.rs`）。`RowLockTable::get_lock` 没有 SQL 调用者。`Cargo.toml` 没有全局分配器。诊断没有克隆计数。
- Code and Critical Path: 测试只构造执行器，不经过 `pipeline::execute`。行源是测试文件里的本地 `Executor`。哈希连接和排序合并用 `JoinConfig`：条件两侧都是 `ColumnRef { table: None, column: "k" }`，列映射 `"k" -> 0`，另外左侧还有 `"name" -> 1`，右侧还有第二列 `"v" -> 1`。输出四列，侧别依次为 Left、Left、Right、Right，列下标依次为 0、1、0、1。嵌套循环使用 `Arc<dyn Predicate>` 形式的 `ConstantPredicate { value: true }`，输出列与侧别同样是四列。排序的 `columns` 是 `["k", "name"]`，`OrderByColumn { column: "k", asc: true, expr: None }`。

**Task Definitions**

### T1: 打印克隆耗时和行数

- Requirement/Scenario: `avoidable-copy` 的「整组克隆和字符串克隆的耗时可以打印」，以及「分配器、行锁和诊断保持原实现」。
- Depends on: None
- Targets: `tests/ms53_copy_measure.rs` 的 `avoidable_copy_times_and_row_counts`。不修改 `src/`。
- Current behavior: 没有这个测试。四个执行器的克隆行为见 Investigation Facts。
- Required behavior: 测试在 release 下退出码 0，并打印规格列出的全部字段。`hash_rows=50`，`nlj_rows=2500`，`sort_rows=100`，`smj_rows=50`。排序输出首行第 0 列是 `Int(0)`，末行第 0 列是 `Int(99)`。四个执行器耗时都大于 0。`hash_bucket_clone_ns=0`。测试不断言 10% 比较的真假。
- Preserve: 产品代码、诊断阶段名、`Cargo.toml`、`RowLockTable`。
- Forbidden: 修改 `src/`；把空循环的耗时印成 `hash_bucket_clone_ns`；在测试里判断胜出者；新增诊断计数。
- First test: 文件不存在时，`cargo test --offline --release --test ms53_copy_measure -- --test-threads=1` 找不到该测试目标，退出码非 0。这是变更前的观察。写入测试后再跑同一条命令。
- GREEN condition: 同一条命令退出码 0，failed 为 0，输出含全部字段且行数符合 Required behavior。
- Verification: 只跑这一条命令一次。失败表示负载、行数或打印字段与规格不一致，不是「低于 10%」。
- Stop when: 哈希连接的探测既不是按下标取行，也不是克隆整桶；嵌套循环的组合次数不是 2500；任一执行器耗时为 0。停止并回到计划，不要为了非 0 重跑，也不要改口径。

负载细节：

- 哈希连接左行 `(Int(i), String(format!("customer_{i}")))`，右行 `(Int(i), Int(i * 100))`，`i` 为 0..50。等值键是第 0 列。排空后应有 50 行。
- 嵌套循环左右行都是 `(Int(i), String(format!("customer_{i}")))`，`i` 为 0..50。谓词恒真。克隆循环是双重循环，共 2500 次，每次克隆对应的左行和右行。排空后应有 2500 行。
- 排序输入按 `i` 从 99 到 0 送入 `(Int(i), String(format!("user_{i}")))`。克隆循环把这 100 行各克隆一次。
- 排序合并的行形状与哈希连接相同，`i` 为 0..50。克隆循环把排空得到的 50 个输出行再各克隆一次。这个循环测的是交出时的那一次整行克隆，不是 `prepare` 里的键克隆。
- 字符串循环把 `customer_0` 到 `customer_49` 各包进 `Value::String` 再克隆一次。
- 每个克隆结果都交给 `std::hint::black_box`。每段计时只有前后各一次 `Instant`。纳秒用 `as_nanos()` 转成 `u128` 再打印。
- 打印行的名字必须与规格一致，一行一个 `name=value`。

### T2: 用整数裁定唯一一处

- Requirement/Scenario: 「达到百分之十才改一处复制」及其三个场景。
- Depends on: T1
- Targets: 本 Cycle 的 Act Response。不修改其他文件。
- Current behavior: 还没有裁定。
- Required behavior: 用 T1 打印的 `u128` 做一次比较，在执行记录里写明每个站点是否达到，以及最终是哪一个名字或「保留原实现」。达到的定义是 `clone_ns.checked_mul(10)` 得到 `Some(product)` 且 `product >= executor_ns`。字符串的分母是 `hash_executor_ns`。整组四处都未达到时才看字符串。整组有达到的，取 `clone_ns` 最大者；相等时依次取嵌套循环、排序合并、排序、哈希桶。点名之后写明 Iteration 001 可以展开，但本 Cycle 不展开、不改产品代码。
- Preserve: T1 的打印原文。不要改写数字。
- Forbidden: 看到数字后改 10% 门槛；因字符串也达到就同时点名两处；在本 Cycle 修改 `src/`。
- First test: 无新测试。输入是 T1 那一次命令的输出。
- GREEN condition: 执行记录里的比较与打印数字一致，并且只点名一处或写明保留。
- Verification: 对照 T1 的输出重算一次。乘法溢出或执行器耗时为 0 时，本任务改为阻塞，不写成未达到。
- Stop when: T1 的字段缺失，或任一执行器耗时为 0，或 `checked_mul(10)` 为 `None`。停止，不要重跑测试来换一组数字。

**Invariants**

SQL 结果、错误文本、SQLSTATE、诊断阶段名、磁盘元组标签、`RowLockTable` 和全局分配器在本 Cycle 结束时与起点相同。测试可以新增。

**Acceptance**

- T1 的命令退出码 0，行数和字段符合 T1 的 Required behavior。
- `git diff` 相对本 Cycle 开始时的产品代码，不含 `src/` 与 `Cargo.toml`。起点提交是本文件所属变更里 Gate 1 之后、本 Cycle 提交之前的变更分支 HEAD；实施开始前用 `git status` 确认工作区没有已经改过的 `src/`。
- T2 的裁定与打印数字一致，只含一个胜出者或「保留原实现」。
- 测量无效时 Act Response 为 `blocked`，不出现「保留原实现」或点名。

**Verification**

`cargo test --offline --release --test ms53_copy_measure -- --test-threads=1`

退出码 0，failed 为 0，输出含九个纳秒字段和四个行数字段。这一个测试覆盖测量场景。裁定场景由 T2 对这次输出做整数比较，不另跑命令。10% 比较不放在测试断言里。

**Gate 2 Readiness**

PASS。无 BLOCKED。无 SKIPPED。

**Risks and Notes**

- 哈希桶写 0 是因为当前探测不克隆整桶。这不是空循环的测量值。
- 排序合并的克隆循环使用排空后的输出行。它衡量交出克隆，不衡量 `prepare` 里的键克隆。
- 本 Cycle 即使裁定点名了一处，也不创建 Iteration 001 的目录，也不改产品代码。
- Persisted Evidence：none。数字写在 Act Response 里即可。

## Act Response

- Status: reported

### 实际改动

- `tests/ms53_copy_measure.rs`：新增 `avoidable_copy_times_and_row_counts`。`RowSource` 按给定行序把行移出。哈希连接和排序合并使用 `JoinConfig`：条件两侧都是 `ColumnRef { table: None, column: "k" }`，左列映射 `k -> 0`、`name -> 1`，右列映射 `k -> 0`、`v -> 1`，输出四列，侧别依次为 Left、Left、Right、Right，列下标依次为 0、1、0、1。嵌套循环使用 `Arc<dyn Predicate>` 形式的 `ConstantPredicate { value: true }` 和 `NestedLoopJoinType::Inner`，输出列与侧别与上面相同。排序的 `columns` 是 `["k", "name"]`，`OrderByColumn { column: "k", asc: true, expr: None }`。`hash_bucket_clone_ns` 写成常量 0。
- `openspec/changes/2026-11-19-ms53-avoidable-copy/tasks.md`：T1、T2 标为已完成。
- 没有修改 `src/` 或 `Cargo.toml`。

### 与计划的偏差

验证命令在测试参数后加了 `--nocapture`。计划里的命令不带这个参数。测试通过时，libtest 不会把 `println!` 打进那条命令的输出，T2 就没有可核对的整数。`--offline`、`--release`、`--test ms53_copy_measure` 和 `--test-threads=1` 保持原样。

带未使用导入的那一次 release 运行退出码是 0，编译警告是 `unused import: Predicate`。删掉该导入后重跑。下面的整数和裁定来自重跑。第一次打印不参与比较。

`RowSource` 用 `IntoIter` 把行移出。计划只要求测试文件里有本地 `Executor`。

### 自检

Gate 3 对照了测试和 diff。断言覆盖行数、排序首行 `Int(0)`、末行 `Int(99)`、四个执行器耗时大于 0，以及 `hash_bucket_clone_ns` 为 0。测试里没有 10% 比较。哈希探测仍是 `right_hashmap.get` 之后按下标取行，桶克隆因此记 0。嵌套循环的克隆循环是左右各 50 行的双重循环，每次 `iter().cloned().chain(...).collect()` 后交给 `black_box`。排序和排序合并的克隆循环各克隆排空得到的输出行。字符串循环把 `customer_0` 到 `customer_49` 包进 `Value::String` 再克隆。重跑后这条测试没有编译警告。

已修复：未使用的 `Predicate` 导入。

遗留 Minor：无。

### 验证

文件还不存在时：

```
cargo test --offline --release --test ms53_copy_measure -- --test-threads=1
```

退出码 101。输出为 `error: no test target named ms53_copy_measure in default-run packages`。

删掉未使用导入之后：

```
cargo test --offline --release --test ms53_copy_measure -- --test-threads=1 --nocapture
```

退出码 0。`test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`。这次编译输出里没有 `unused import`。

打印原文：

```
hash_executor_ns=19421
hash_bucket_clone_ns=0
nlj_executor_ns=386503
nlj_clone_ns=61909
sort_executor_ns=21324
sort_clone_ns=2038
smj_executor_ns=25135
smj_clone_ns=1198
string_clone_ns=1685
hash_rows=50
nlj_rows=2500
sort_rows=100
smj_rows=50
```

### 裁定

四个执行器耗时都大于 0。`checked_mul(10)` 都得到 `Some`。比较用的就是上面这组整数。

| 站点 | clone_ns × 10 | 分母 | 达到 |
|---|---|---|---|
| 哈希桶 | 0 | `hash_executor_ns` 19421 | 否 |
| 嵌套循环 | 619090 | `nlj_executor_ns` 386503 | 是 |
| 排序 | 20380 | `sort_executor_ns` 21324 | 否 |
| 排序合并 | 11980 | `smj_executor_ns` 25135 | 否 |
| 字符串 | 16850 | `hash_executor_ns` 19421 | 否 |

整组里只有嵌套循环达到。`nlj_clone_ns` 是 61909，也是达到者里最大的克隆耗时。相等时的次序没有用上。

胜出者：嵌套循环。

字符串未达到。整组已经有达到者，所以这一轮不改 `Value::String`。

Iteration 001 可以展开，只改嵌套循环的组合行构造。本 Cycle 不创建该目录，不改产品代码。

### Experience Candidates

None

### 未解决问题

无。

## Plan Review

- Review Result: accepted

**Findings**

对照 `tests/ms53_copy_measure.rs`，以及 merge-base `f940925831e5e78f201bd27a09173cbaa2bad2bc` 到 `37f4a662bc7403f11308c13a73b47970e17ca02f` 的 diff。diff 新增变更文档和这一个测试，没有 `src/`，没有 `Cargo.toml`。

测试打印九个纳秒字段和四个行数字段。断言是 `hash_rows=50`、`nlj_rows=2500`、`sort_rows=100`、`smj_rows=50`，排序首行 `Int(0)`、末行 `Int(99)`，四个执行器耗时大于 0，`hash_bucket_clone_ns` 为常量 0。克隆结果交给 `black_box`。每段计时只有前后各一次 `Instant`。测试里没有 10% 比较。嵌套循环克隆是左右各 50 行的双重循环。字符串循环是 `customer_0` 到 `customer_49`。

`src/executor/join.rs` 的探测仍是 `right_hashmap.get` 之后按下标取行。`Value::String` 的负载仍是 `String`。嵌套循环的组合行仍是 `left_row.iter().cloned().chain(right_row.iter().cloned())`。这三个文件不在本次 diff 里。

用本 Cycle Act Response 的打印重算。来源是删掉未使用导入之后的那次命令：`cargo test --offline --release --test ms53_copy_measure -- --test-threads=1 --nocapture`，退出码 0，`1 passed; 0 failed`。工作区在该提交之后没有再改测试文件，所以不重跑。

| 站点 | clone_ns × 10 | 分母 | 达到 |
|---|---|---|---|
| 哈希桶 | 0 | 19421 | 否 |
| 嵌套循环 | 619090 | 386503 | 是 |
| 排序 | 20380 | 21324 | 否 |
| 排序合并 | 11980 | 25135 | 否 |
| 字符串 | 16850 | 19421 | 否 |

四个执行器耗时都大于 0。五个乘积都落在 `u128` 里。整组里只有嵌套循环达到，它也是达到者里克隆耗时最大的一处。字符串未达到。

非阻塞，计划遗漏：计划里的命令没有 `--nocapture`。测试通过时，libtest 不把 `println!` 打进那条命令的输出。Act 加上这个参数，测量口径没有改。

非阻塞：带未使用 `Predicate` 导入的第一次 release 运行退出码也是 0。裁定只用删掉该导入之后的第二次打印。重跑原因是去掉编译警告。

`RowSource` 用 `IntoIter` 移出已有行。计划允许测试文件里的本地 `Executor`。

**Evidence**

- 测试与打印：本 Cycle Act Response，提交 `37f4a662bc7403f11308c13a73b47970e17ca02f`。
- diff：`git diff --name-status f940925831e5e78f201bd27a09173cbaa2bad2bc..37f4a662bc7403f11308c13a73b47970e17ca02f`，八个新增文件，无 `src/`。
- 探测与字符串负载：`src/executor/join.rs` 的 `right_hashmap.get`，`src/executor/value.rs` 的 `String(String)`，`src/executor/nested_loop_join.rs` 的组合行克隆。

**Follow-up Decision**

T1 和 T2 的验收都满足。上面两项不挡住验收，不留在当前 Cycle 修，也不另开返工。裁定点名嵌套循环，任务表里的下一轮因此展开，只执行嵌套循环那一小节。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-11-19-ms53-avoidable-copy/iterations/001-implement-named-site/000-initial.md`
