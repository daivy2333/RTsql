# MS53 可避免的整组复制

来源：`.agents/docs/tasks.md` 的 MS53，承接 I020。分配器和行锁不在本变更。约束：每次测试调用带 `--test-threads=1`。多个测试目标分成多次命令，按下面列出的顺序执行。

Gate 1：PASS。需求、场景和范围来自 MS53 的复制部分，以及本次写死的 10% 门槛。这是批准，不是跳过。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。规格在 `specs/avoidable-copy/spec.md`。
- 000：T1 与 T2 已执行。Review Result 为 `accepted`。胜出者是嵌套循环。`iterations/000-measure-and-decide/000-initial.md` 的 Plan Context 状态仍为 `ready`。
- 001：已展开。执行依据是 `iterations/001-implement-named-site/000-initial.md`，Plan Context 为 `ready`。只做 T3 的嵌套循环小节。
- Persisted Evidence：none。
- 跳过项：无。全量不在测量轮，理由写在 000 的 Verification boundary。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | release 测试打印全部纳秒和行数，产品代码不变 | 已完成 |
| T2 | 000 | T1 | 用 T1 的整数裁定改哪一处或不改，并写入执行记录 | 已完成 |
| T3 | 001 | T2 点名了唯一一处 | 只改嵌套循环的组合行构造，同一输入的结果与改前一致 | 待执行 |

## Iteration Plan

### Iteration 000: 测量整组复制并裁定

- Tasks: T1, T2
- Depends on: None
- Stable result: 一组纳秒数和一次裁定。点名一处时，下一轮只依赖这个名字。未达到时，本变更的稳定结果是不改产品代码。
- Verification boundary: 本轮只跑 `cargo test --offline --release --test ms53_copy_measure -- --test-threads=1` 一次。不跑全量。测量不改产品代码，全量套件读不到这些打印字段。风险：合回主分支之前没有全量；本轮 diff 若只含测试文件，排查范围就是这一个测试。
- Diagnostic boundary: 这一轮的失败只看这个测试的打印、行数和执行器耗时是否为 0。不把分配器、`RowLockTable`、诊断阶段或文件格式纳入排查。
- Non-goals: 不改 `src/`。不展开 Iteration 001。不测量分配器，不测量行锁。

平衡：本轮的稳定结果是裁定。实现与打印数字不是同一个故障域，所以不并进本轮。只有一条测试，它决定下一轮是否存在。

### Iteration 001: 实现被点名的那一处

- Tasks: T3
- Depends on: Iteration 000 的裁定点名了哈希桶、嵌套循环、排序、排序合并或 `Value::String` 之一
- Stable result: 只改点名的那一处。同一输入的行、次序、比较和错误与改前一致。其余复制实现、分配器和 `RowLockTable` 保持原样。
- Verification boundary: 受影响回归，全部带 `--test-threads=1`，按 T3 里该站点的命令顺序执行。不把全量套件放进本轮。`Value::String` 改的是内存负载，磁盘元组标签不改，所以不把这次升到全量。风险：仓外若按 `Value::String(String)` 构造值，将无法编译；本仓库以列出的测试为准。
- Diagnostic boundary: 只查点名的那个文件及其直接调用。分配器依赖、`src/transaction/row_lock.rs`、诊断阶段名不在排查范围。
- Non-goals: 不改第二处复制。不引入分配器。不改 `RowLockTable`。不改谓词 trait。

平衡：一处实现的结果变化和它的既有测试是同一个故障域。按站点分成多轮会把同一次裁定拆开。裁定已点名嵌套循环，本轮已展开，只执行嵌套循环那一小节。

## Requirements Traceability Matrix

| Requirement | Task | First Test |
|---|---|---|
| 整组克隆和字符串克隆的耗时可以打印 | T1 | `avoidable_copy_times_and_row_counts` |
| 达到百分之十才改一处复制 | T2 | `avoidable_copy_times_and_row_counts` 的输出行 |
| 改动后的 SQL 结果与测量前一致 | T3 | 见 T3 中被点名站点的第一条既有测试 |
| 分配器、行锁和诊断保持原实现 | T1 | `avoidable_copy_times_and_row_counts` |

没有 Simplified。没有 Missing。T3 的第一条测试取决于裁定点名的站点，不在 000 执行。

## T1 与 T2

执行依据是 `iterations/000-measure-and-decide/000-initial.md` 的 Task Definition。本文件不另写一套步骤。

## T3: 实现点名的那一处

本任务只在 Iteration 001 展开之后执行。000 的 Cycle 不执行本任务。只执行与裁定名字相符的那一小节，其余小节不执行。

### 嵌套循环

- Requirement/Scenario: `avoidable-copy` 的「达到百分之十才改一处复制」中嵌套循环达到的场景，以及「改动后的 SQL 结果与测量前一致」的非等值连接场景。
- Depends on: T2 记录的胜出者是嵌套循环。
- Targets: `src/executor/nested_loop_join.rs` 的 `NestedLoopJoinExecutor::next`。测试保持 `tests/nested_loop_join_test.rs` 与 `tests/outer_join_test.rs`。
- Current behavior: 每一对组合都把左右行克隆进一个新的 `Vec<Value>`，再调用 `predicate.evaluate`。
- Required behavior: 组合行仍是左列后接右列。值从左右行移入，求值返回后按原列数移回。输出单元格的克隆、外连接的 NULL 补齐和错误文本保持现状。
- Preserve: 谓词签名、SQL 结果、SQLSTATE、诊断阶段名。
- Forbidden: 修改谓词 trait；修改哈希连接、排序、排序合并或 `Value::String`；修改 `RowLockTable` 或 `Cargo.toml` 依赖。
- First test: 改代码前执行 `cargo test --offline --test nested_loop_join_test -- --test-threads=1`，退出码 0。这是重构前的 GREEN。
- GREEN condition: 同一条命令再次退出码 0，failed 为 0。随后 `cargo test --offline --test outer_join_test -- --test-threads=1` 退出码 0。最后再跑一次测量测试，命令与 T1 相同，行数仍然正确。
- Verification: 上面三条按这个顺序执行，不并成一次 cargo 调用。外连接失败表示 NULL 补齐被移动弄坏。测量测试失败表示负载行数变了。
- Stop when: 谓词在 `evaluate` 返回后仍借用组合行，或为了保持结果必须修改谓词 trait 的错误文本。停止并回到计划。

### 排序

- Requirement/Scenario: 「改动后的 SQL 结果与测量前一致」的排序场景。
- Depends on: T2 记录的胜出者是排序。
- Targets: `src/executor/sort.rs` 的 `SortExecutor::next`。
- Current behavior: `sorted_rows[position].clone()` 后位置加一。
- Required behavior: 用 `std::mem::take` 移出该槽位的行。已交出的槽位不再读取。次序和投影与改前一致。
- Preserve: 比较、`ORDER BY` 的升序和降序、有界排序的驻留上限。
- Forbidden: 改其他执行器、`Value::String`、分配器和行锁表。
- First test: 改代码前 `cargo test --offline --test sort_test -- --test-threads=1` 退出码 0。
- GREEN condition: 同一命令再次退出码 0。然后 `cargo test --offline --test order_by_alias_sort_test -- --test-threads=1` 退出码 0。最后跑 T1 的测量命令，`sort_rows` 仍为 100，首行键仍为 0，末行键仍为 99。
- Verification: 三条命令按顺序执行。别名排序失败表示表达式键路径被交出改动影响。
- Stop when: 交出之后的代码还会读取该槽位。停止并回到计划。

### 排序合并

- Requirement/Scenario: 「改动后的 SQL 结果与测量前一致」的排序场景。
- Depends on: T2 记录的胜出者是排序合并。
- Targets: `src/executor/sort_merge_join.rs` 的 `SortMergeJoinExecutor::next`。
- Current behavior: `outputs[output_index].clone()` 后下标加一。
- Required behavior: 用 `std::mem::take` 移出该输出行。已交出的槽位不再读取。组间乘积和输出单元格与改前一致。
- Preserve: NULL 键不入组、负零与正零的相等、与哈希连接相同的多重集合。
- Forbidden: 改 `prepare` 的收集方式；改其他执行器、`Value`、分配器和行锁表。
- First test: 改代码前 `cargo test --offline --test sort_merge_join_test -- --test-threads=1` 退出码 0。
- GREEN condition: 同一命令再次退出码 0。然后跑 T1 的测量命令，`smj_rows` 仍为 50。
- Verification: 两条命令按顺序执行。排序合并测试失败表示多重集合或零值相等变了。
- Stop when: 交出之后还会读取该输出槽位，或必须改 `prepare` 才能移出。停止并回到计划。

### 哈希桶

- Requirement/Scenario: 测量场景里哈希桶克隆耗时为 0。本站点没有单独的实现，除非 T2 把胜出者记成哈希桶。
- Depends on: T2 记录的胜出者是哈希桶。
- Targets: `src/executor/join.rs` 的 `JoinExecutor::next`。
- Current behavior: 探测按下标取桶中的行，不克隆整桶。
- Required behavior: 保持按下标取行。多匹配的多重集合、NULL 键不匹配、负零归一与改前一致。
- Preserve: 输出列克隆和哈希键克隆。
- Forbidden: 为了测量而加回整桶克隆；改其他执行器。
- First test: 若 T2 仍把胜出者写成哈希桶，而探测已经是按下标取行，则不要改 `join.rs`。在执行记录写明「该处已经是目标行为，001 不改代码」。改代码前的检查命令是 `cargo test --offline --test join_test -- --test-threads=1`，退出码 0。
- GREEN condition: 同一命令退出码 0，且测量测试的 `hash_rows` 仍为 50。
- Verification: 只在实际修改了 `join.rs` 时跑 `join_test`。没有修改则不跑，执行记录说明原因。
- Stop when: 探测既不是按下标取行，也不是克隆整桶。停止并回到计划。

### Value::String

- Requirement/Scenario: 「整组克隆都未达到而字符串克隆达到」，以及「字符串值的比较不变」。
- Depends on: T2 记录的胜出者是 `Value::String`。
- Targets: `src/executor/value.rs` 的 `Value::String`；仓内构造或匹配该负载的位置。磁盘编解码在 `src/storage/page_format/tuple.rs`，只在负载类型变化导致无法编译时改调用，不改标签。
- Current behavior: `Value::String(String)`。克隆会复制字符。
- Required behavior: 负载为 `Arc<str>`。相等、哈希、顺序和显示按字符内容。读回的元组字节与改前一致。其他变体和 `ValueRef::Text` 不变。
- Preserve: 磁盘标签、SQL 比较结果、连接和分组。
- Forbidden: 改 `Blob`；新增一套平行的值 API；改分配器或行锁；同时改四个执行器的整组克隆。
- First test: 改代码前 `cargo test --offline --test value_test -- --test-threads=1` 退出码 0。
- GREEN condition: 按顺序执行，每条退出码 0，failed 为 0：`value_test`、`join_test`、`sort_merge_join_test`、`nested_loop_join_test`、`sort_test`，然后 T1 的测量命令。
- Verification: 这些命令顺序执行，不合并。`value_test` 失败表示比较或显示变了。连接测试失败表示字符串键的相等变了。
- Stop when: 不新增公共构造函数就无法让仓内调用编译，或磁盘标签必须改变才能写回相同字节。停止并回到计划。

## 验证命令汇总

000 的命令只在 Cycle 文件里。001 的命令见 T3 被点名的那一小节。全部带 `--test-threads=1`。没有全量命令。
