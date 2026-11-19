# Iteration 001 / Cycle 000: 移走嵌套循环的组合行克隆

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3 的嵌套循环小节
- Inherited scope: Iteration 000 的裁定。胜出者是嵌套循环。组合行仍是左列后接右列。求值返回后，值按原列数回到原来的左右行。SQL 结果、错误文本和 SQLSTATE 与改前一致。
- Excluded scope: 排序、排序合并、哈希桶、`Value::String`。不改谓词 trait、规划器、pipeline 的错误前缀、`RowLockTable`、`Cargo.toml`。不重新比较 10%。

**Objective**

`NestedLoopJoinExecutor::next` 在求值每一对组合时，不再把左右行克隆进新的 `Vec<Value>`。同一 SQL 的结果行、NULL 补齐和谓词错误文本与改前一致。

**Investigation Facts**

- Starting point: 分支 `changes/2026-11-19-ms53-avoidable-copy`。Iteration 000 的执行记录在 `iterations/000-measure-and-decide/000-initial.md`，Act 提交 `37f4a662bc7403f11308c13a73b47970e17ca02f`。裁定只用那次打印：`nlj_clone_ns=61909`，`nlj_executor_ns=386503`，`61909 * 10 = 619090 >= 386503`。其余整组站点和字符串都未达到。该提交相对 merge-base `f940925831e5e78f201bd27a09173cbaa2bad2bc` 没有改 `src/`。本 Cycle 开始时，组合行仍是下面的克隆。
- Current-State Evidence: 克隆发生在 `src/executor/nested_loop_join.rs` 的 `NestedLoopJoinExecutor::next`。右行先物化进 `right_rows`，左行放在 `current_left`。每一对执行 `left_row.iter().cloned().chain(right_row.iter().cloned()).collect()`，再 `self.predicate.evaluate(&combined)`。`Ok(true)` 时调用 `build_output_row(left_row, right_row)`，按 `output_sides` 从左右行按下标克隆输出单元格。`Ok(false)` 只把 `current_right_index` 加一。`Err` 返回 `StorageError::ExecutionError(format!("Predicate evaluation error: {}", e))`。`Left`/`Full` 的未匹配左行走 `build_padded_row`，缺席侧填 `Value::Null`。`Full` 的未匹配右行走 `next_unmatched_right_row`，那里仍克隆整行右行。这些输出路径不在本轮的克隆替换范围内。
- Code and Critical Path: `Predicate::evaluate` 的签名是 `fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>`（`src/executor/predicate.rs`）。返回值不借用 `row`。该文件没有 `unsafe`。`Expression::evaluate` 在调用内部把 `ValueRef` 收成拥有所有权的 `Value` 再返回。因此求值函数返回之后，组合行可以被拆开移回。规划器在 `src/parser/planner/query.rs` 把谓词的 `column_index` 编成组合行绝对下标：左输入 `0..n`，右输入 `n..n+m`。`RIGHT JOIN` 在计划期交换输入，执行器仍按自己的左输入接右输入拼行。构造入口在 `src/pipeline.rs` 的 `PhysicalPlan::NestedLoopJoin` 臂，调用 `NestedLoopJoinExecutor::new(...).with_budget(...)`。本轮不改这个构造。预算在右行物化时 `budget.charge(row_bytes(&row))`，组合求值不再入账。`tests/nested_loop_join_test.rs` 经 `Database::execute_sql` 覆盖不等式连接、混合 AND、NULL 被排除、空输入、`ORDER BY` 和 `WHERE`。`tests/outer_join_test.rs` 经同一入口覆盖 `LEFT`/`RIGHT`/`FULL`、非等值 `LEFT JOIN` 和 NULL 补齐。两个文件都在默认 feature 下编译：前者要求 `network`，后者要求 `network` 和 `cli`，`Cargo.toml` 的 default 含这两项。

**Task Definitions**

### T3: 组合行改为移入再移回

- Requirement/Scenario: `avoidable-copy` 的「嵌套循环达到且克隆耗时最大」，以及「嵌套循环的非等值连接结果不变」。
- Depends on: Iteration 000 记录的胜出者是嵌套循环。
- Targets: `src/executor/nested_loop_join.rs` 的 `NestedLoopJoinExecutor::next`。既有测试保持 `tests/nested_loop_join_test.rs` 与 `tests/outer_join_test.rs`。
- Current behavior: 每一对组合都克隆左右行，拼成新的 `Vec<Value>`，再调用 `predicate.evaluate`。求值返回后，输出、继续下一对和外连接补齐都读原来的左右行。
- Required behavior: 每一对仍把左行各列接在右行各列之前交给现有谓词。值从 `current_left` 和 `right_rows[index]` 移进这个向量，不克隆这两行。调用前记下两侧列数。`evaluate` 返回后，无论 `Ok(true)`、`Ok(false)` 还是 `Err`，先按记下的列数把值移回原来的左右行，再执行现有分支。`Ok(true)` 仍调用现有的 `build_output_row`。`Err` 的格式字符串仍是 `Predicate evaluation error: {}`，变体仍是 `StorageError::ExecutionError`。`build_padded_row` 和 `next_unmatched_right_row` 保持现有克隆与 NULL 补齐。移出和移回之间不 `.await`。
- Preserve: 谓词签名、规划器的组合行下标、SQL 结果、`Execution error:` 前缀、该错误的 SQLSTATE（`storage_error_sqlstate` 对这个变体返回 `None`）、诊断阶段名、右行物化时的预算入账。
- Forbidden: 修改谓词 trait；修改哈希连接、排序、排序合并或 `Value::String`；修改 `RowLockTable` 或 `Cargo.toml`；用 `catch_unwind` 或别的 panic 恢复把值搬回；在本轮重跑测量来改胜出者；改 pipeline 的错误漏斗。
- First test: 改 `nested_loop_join.rs` 之前执行 `cargo test --offline --test nested_loop_join_test -- --test-threads=1`，退出码 0。这是重构前的 GREEN。改前若不是 0，停止，不要把失败当成这次移动的修复对象。
- GREEN condition: 同一条命令再次退出码 0，failed 为 0。随后 `cargo test --offline --test outer_join_test -- --test-threads=1` 退出码 0。最后 `cargo test --offline --release --test ms53_copy_measure -- --test-threads=1` 退出码 0。测量命令不带 `--nocapture`。通过条件是测试里已有的行数断言，不读取新的纳秒数，也不重新裁定。
- Verification: 上面三条按这个顺序执行，每次单独一个 cargo 进程。外连接失败表示 NULL 补齐或未匹配行被移动弄坏。测量测试失败表示负载行数变了。
- Stop when: 为了让移回通过编译，必须修改谓词 trait，或必须修改 `Predicate evaluation error:` 这段格式字符串。停止并回到计划。不要改 trait，也不要改错误文本。

**Invariants**

SQL 结果、错误文本、SQLSTATE、诊断阶段名、磁盘元组标签、`RowLockTable` 和全局分配器与本 Cycle 开始时相同。哈希连接、排序、排序合并和 `Value::String` 的实现不改。

**Acceptance**

- `nested_loop_join.rs` 的组合求值不再克隆左右行。求值返回后，左右行按原列数、原顺序恢复，然后才产出、继续或返回错误。
- `cargo test --offline --test nested_loop_join_test -- --test-threads=1` 在修改后退出码 0。这覆盖非等值连接的结果行。
- `cargo test --offline --test outer_join_test -- --test-threads=1` 退出码 0。这覆盖外连接的 NULL 补齐。
- 测量测试退出码 0，`hash_rows`、`nlj_rows`、`sort_rows`、`smj_rows` 仍由该测试断言为 50、2500、100、50。
- 产品 diff 只出现在 `src/executor/nested_loop_join.rs`。

**Verification**

按 GREEN condition 的三条命令顺序执行。每条都带 `--test-threads=1`。不跑全量套件。测量测试不重新解释 10%。

**Gate 2 Readiness**

PASS。无 BLOCKED。无 SKIPPED。goal 原文「那么请你开始计划并实施ms53吧，要求遵守各种规范」是本 Cycle 的批准。检查项本身没有跳过。

**Risks and Notes**

- 用整行 `take` 还是按元素移出，由实施选择。验收只看谓词看到的顺序，以及返回后两侧行的列数和顺序。
- `evaluate` 中途 panic 时，值可能留在组合向量里。不要加恢复协议。现有克隆路径在 panic 时也会丢掉当前这次求值用的副本。
- 当前 `let left_row = self.current_left.as_ref()...` 的借用跨过整个组合循环。移出之前要结束这份共享借用。外连接补齐仍读恢复后的 `current_left`。
- `evaluate` 拿到的是 `&[Value]`，不能改向量长度。按调用前记下的列数拆回。不要为「长度不一致」新增错误。
- Persisted Evidence：none。三条命令的退出码写在 Act Response 里即可。

## Act Response

- Status: 待填写

### 实际改动

待 Act 填写。

### 验证

待 Act 填写。

## Plan Review

- Review Result: pending

**Findings**

待 Act 提交后填写。

**Follow-up Decision**

待 Act 提交后填写。

**Next Cycle**

None

**Next Iteration**

None
