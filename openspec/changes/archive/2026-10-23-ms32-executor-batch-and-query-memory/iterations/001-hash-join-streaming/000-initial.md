# Iteration 001 / Cycle 000: 哈希 JOIN 左表逐行

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: 2.1、2.2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 的哈希 JOIN 段落；design D2；`query-memory-budget`「哈希 JOIN 不驻留整表左输入」里关于结果正确、左表不整表驻留的部分。内存上限本身不在本轮。
- Excluded scope: Top-N、查询内存计费、嵌套循环 JOIN 的组合行克隆、选择较小的构建侧、批量生产路径

**Objective**

等值哈希 JOIN 的匹配结果与现在一致。左表不再整表进入执行器缓冲，匹配桶不再整桶克隆。左表在产出匹配之后出错时，`execute_sql` 返回错误，不返回已产出的部分行。

**Investigation Facts**

- Current Baseline: Iteration 000 的 Act Response（2026-10-23）已沿用。生产代码相对 `a0c1ed4` 只增加 `Executor::next_batch` 默认方法、两个同序测试和 `benches/data_scan_bench.rs` 的 `next_batch/10000` 项。DataScan 没有覆盖 `next_batch`。全量 14 批 2125 passed / 0 failed / 2 ignored。本轮没有重跑这组测试。
- Current-State Evidence: `JoinExecutor::next`（`src/executor/join.rs`）仍是三阶段。`BuildRight` 把右行放进 `right_hashmap`。`ScanLeft` 把左行全部放进 `left_rows`。输出时 `right_hashmap.get(&hash_key).cloned()` 复制整桶，再按 `build_output_row` 克隆输出单元格。等值 INNER JOIN 在 `src/parser/planner/query.rs` 约 861 行走 `PhysicalPlan::Join`；外连接走嵌套循环。`execute_executor`（`src/pipeline.rs` 约 1583 行）遇到 `Err` 时直接返回 `SqlError`，不把已经收集的行作为 `Ok` 返回。`tests/scalar_function_test.rs` 记明 `i64::MIN` 不能用 SQL 字面量插入，要用 `0 - 9223372036854775807 - 1`。
- Code and Critical Path: 公共入口 `Database::execute_sql` → `execute_stage` → `create_executor_from_plan` 的 `Join` 臂 → `JoinExecutor`。本轮只改 `src/executor/join.rs` 和 `tests/join_test.rs`。

**Implementation Guidance**

这是保持查询结果的结构调整。先跑现有 JOIN 测试，确认是绿的，再改执行器。输出单元格的克隆保留。NULL 键仍然不匹配。

**Task Contracts**

### 2.1: 左表逐行探测

- Requirement/Scenario: query-memory-budget「哈希 JOIN 不驻留整表左输入」的结果部分。小右表加大左表的上限场景留到 Iteration 003。
- Depends on: Iteration 000
- Targets: `src/executor/join.rs::JoinExecutor`；`tests/join_test.rs`
- Current behavior: 右表建成哈希后，左表全部进入 `left_rows`，然后才输出。每个左行第一次匹配时克隆整个右桶。
- Required behavior: 右表哈希保持。之后每取一行左表就探测，不再保存全部左行，删除 `left_rows` 和把左表收齐的 `ScanLeft` 阶段。查到桶后按下标取当前右行，不 `cloned()` 整桶。输出仍由 `build_output_row` 克隆单元格。等值 JOIN 的行集合与改前一致。`execute_sql` 在投影计算溢出时返回 `Err`，不是只含成功行的 `Ok`。
- Preserve: 右表哈希、NULL 键不匹配、内连接语义、嵌套循环 JOIN、`next_batch` 默认方法。
- Forbidden: 改构建侧选择，改嵌套循环的组合行克隆，加内存计费，改 `execute_executor` 的错误返回形态。
- First test: 先运行 `cargo test --test join_test --test chain_join_test --test self_join_test -- --test-threads=1`，预期在改代码前通过。再在 `tests/join_test.rs` 增加经 `Database::execute_sql` 的测试：`l(id INT PRIMARY KEY)` 含 1 和 2，`r` 同样含 1 和 2，执行 `SELECT abs(CASE WHEN l.id = 2 THEN 0 - 9223372036854775807 - 1 ELSE l.id END) FROM l JOIN r ON l.id = r.id`。这条在改 JOIN 之前就应返回 `Err`，消息包含 `Integer overflow in abs`。
- GREEN condition: 上述三条既有测试加上新测试都通过，且 `join.rs` 中不再有 `left_rows`，输出路径不再 `cloned()` 整个匹配桶。
- Verification: `cargo test --test join_test --test chain_join_test --test self_join_test -- --test-threads=1` 退出码 0。
- Stop when: 新 SQL 在计划期被拒绝，或改完后既有 JOIN 的行集合发生变化。不要另选构建侧来绕过。

### 2.2: 全量分批回归

- Requirement/Scenario: 哈希 JOIN 结果不变的回归面
- Depends on: 2.1
- Targets: 无产品代码
- Current behavior: Iteration 000 结束时 2125 passed / 0 failed / 2 ignored。本轮会多一条测试。
- Required behavior: 按项目模型 M19。`--lib --bins` 一批。`tests/*.rs` 按文件名字典序每 10 个一批。每批 `cargo test --test ... -- --test-threads=1`，0 failed，不漏批。
- Preserve: 不新增测试框架外的判定脚本。
- Forbidden: 一次无过滤的 `cargo test`。
- First test: 无。消费 2.1 已通过的测试。
- GREEN condition: 每一批 failed 为 0。
- Verification: 各批退出码和 passed/failed 计数写入 Act Response。
- Stop when: 某一批失败。本轮范围内的失败先修；范围外的失败回到计划。

**Invariants**

- 等值 INNER JOIN 的匹配行与 Iteration 000 结束时一致。
- 外连接仍走嵌套循环执行器。
- 语句级错误不作为带部分行的成功结果返回。

**Non-goals**

查询内存上限、Top-N、较小构建侧、批量攒批。

**Acceptance**

- 既有 `join_test`、`chain_join_test`、`self_join_test` 通过。
- 新的 `execute_sql` 溢出测试返回 `Err`，消息含 `Integer overflow in abs`。
- `JoinExecutor` 不再保存全部左行，不再克隆整个匹配桶。
- 2.2 各批 0 failed。

**Verification**

- 2.1 的三条既有测试加新测试是直接命令。
- 左表不整表驻留由 `join.rs` 删除 `left_rows` 来核对，不另写内存采样。
- 2.2 沿用 Iteration 000 的分批方式。

**Gate 2 Readiness**

PASS。任务 2.1、2.2 属于 2026-10-23 用户原话「批准」的四轮计划，本 Cycle 没有新增范围。无 BLOCKED，无 WAIVED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

溢出测试在改动前后都应失败返回，它守的是公共入口不交出部分行，不证明左表已经改成逐行。逐行由删除 `left_rows` 核对。若 JOIN 输出顺序把溢出行排到第一行，测试仍然只断言 `Err`，不断言先成功一行。

## Act Response

- Status: blocked

### Blocker Handoff

- 发现位置：Task 2.1 → First test（先行测试建立）→ Phase 3/TDD。命中 Task 2.1 的 Stop when 第一款「新 SQL 在计划期被拒绝」。产品代码未动（`src/executor/join.rs` 未修改）。
- Plan 预期：`SELECT abs(CASE WHEN l.id = 2 THEN 0 - 9223372036854775807 - 1 ELSE l.id END) FROM l JOIN r ON l.id = r.id` 改前即返回 Err 且消息含 `Integer overflow in abs`。
- 实际情况：该 SQL 在计划期被点名拒绝——`Parse error: Expression projection items are not supported with JOIN queries`（SQLSTATE 42601；拒绝点 `src/parser/planner/query.rs:1916` 与 `:1948`）。JOIN 基座表达式投影项是既有能力边界，与本轮目标改动（左表逐行）无关。
- 定向补充事实（one-shot CLI `:memory:` 真实入口，三条探测）：
  - 单表 + 表达式投影：同一表达式形态 `SELECT abs(CASE WHEN id = 2 THEN 0 - 9223372036854775807 - 1 ELSE id END) FROM l` 返回执行期 `Execution error: ... Integer overflow in abs(-9223372036854775808)`——表达式形态与溢出错误面本身与 Plan 预期一致；
  - JOIN + 表达式投影：计划期拒绝（First test SQL 原文，拒绝文案见上）；
  - JOIN + 列投影 + 同表达式入 WHERE：返回执行期 `Predicate evaluation error: Integer overflow in abs(-9223372036854775808)`——JOIN 基座的执行期中途失败经 WHERE 面可达，且左表行 id=1 先通过谓词、行 id=2 才溢出，属同形中途失败。是否把 First test SQL 改写为 WHERE 形态属于测试策略修订，由 Plan 裁定。
- 影响：Task 2.1 的先行测试无法按 Task Definition 建立；join.rs 重构不得在当前执行依据下开始（TDD 铁律 + Stop when）。Task 2.2 依赖 2.1，未开始。
- 任务状态：已完成——改前基线（三条测试文件 GREEN：join_test 8、chain_join_test 8、self_join_test 6，均 0 failed，`--test-threads=1`）；部分完成——Task 2.1 先行测试已写入 `tests/join_test.rs`，当前为红色（计划期拒绝，非溢出文案）；未开始——join.rs 重构、Task 2.2 全量分批回归。
- 修改文件与工作区：`tests/join_test.rs` 新增 `use rtsql::database::Database` 导入与测试 `execute_sql_returns_err_not_partial_rows_on_join_projection_overflow`（保留红色现场，见上）；工作区其余改动为 Iteration 000 既有未提交交付（`git status` 5 文件），与本轮无关。已通过 Gate：Gate 4 的 Task 2.1 改前基线部分；Gate 3、2.1 实施、2.2 未到达。
- Evidence：None required——阻塞可由单条命令复现（三条探测 SQL 见上，拒绝点源码位置已引用）。
- 恢复条件：Plan 修订 Task 2.1 的 First test（裁定 WHERE 形态或其他执行期可达形态）并同步更新该任务的 GREEN condition、Acceptance、Risks and Notes；用户下达恢复指令后 Act 重写先行测试（RED 按修订后判定），再继续 join.rs 重构与 2.2。

### 自检发现与遗留 Minor 问题

无（未进入实施）。

### 验证命令与实际结果

- `cargo test --test join_test --test chain_join_test --test self_join_test -- --test-threads=1`：改前基线 GREEN（8+8+6 passed，0 failed）。
- `cargo test --test join_test execute_sql_returns_err_not_partial_rows_on_join_projection_overflow -- --test-threads=1`：FAILED——`Parse error: Expression projection items are not supported with JOIN queries`（预期外文案，阻塞依据）。
- CLI 探测三条（`:memory:`）：单表表达式投影 → 执行期 `Integer overflow in abs(-9223372036854775808)`；JOIN 表达式投影 → 计划期 42601 拒绝；JOIN WHERE 形态 → 执行期 `Predicate evaluation error: Integer overflow in abs(-9223372036854775808)`。

### Persisted Evidence

None required。

### Experience Candidates

None。JOIN 基座表达式投影拒绝是既有能力边界（错误点名能力，按设计行为），非缺陷。

### 未解决问题

First test SQL 的替代形态由 Plan 裁定（见 Blocker Handoff 恢复条件）。

## Plan Review

- Review Result: replan-required

**Findings**

偏差原因是 Plan 错误。`query.rs` 约 1916 与 1948 行拒绝 JOIN 上的表达式投影，文案与 Act 记录的 `Expression projection items are not supported with JOIN queries` 一致。先行测试 SQL 走不到 `abs` 溢出。Act 在改 `join.rs` 之前停下，符合 Stop when。

WHERE 包住 JOIN 的路径在 `query.rs` 约 2174 行，`FilterExecutor` 把谓词错误写成 `Predicate evaluation error: `。消息里仍有 `Integer overflow in abs`。这条不改变 JOIN 的表达式投影拒绝，也不扩大本轮范围。

改前 8+8+6 通过沿用 Act Response，没有重跑。`join.rs` 仍含 `left_rows`。红色测试留在 `tests/join_test.rs`，由后继 Cycle 替换。

**Acceptance Gaps**

父 Cycle 规定的投影 SQL 不能作为先行测试。JOIN 重构和全量回归都没有做。行为要求本身不变。

**Evidence**

- `src/parser/planner/query.rs` 的两处 JOIN 表达式投影拒绝
- `src/parser/planner/query.rs` 约 2174 行的 `Filter` 包装
- `src/executor/filter.rs` 的 `Predicate evaluation error`
- Act Response 的三条探测和红色测试失败
- `tests/join_test.rs` 的 `execute_sql_returns_err_not_partial_rows_on_join_projection_overflow`

**Follow-up Decision**

测试约定写错了，当前执行依据不能继续。不在本 Cycle 里改测试。后继 replan 把见证改成 WHERE。

**Iteration Plan Update**

任务 2.1 的见证从 JOIN 投影改为 JOIN 的 WHERE 谓词溢出。重构范围、全量分批和其余轮次不变。`tasks.md` 的 2.1 与 `design.md` D2 已按此改写。

**Next Cycle**

`iterations/001-hash-join-streaming/001-replan.md`

**Next Iteration**

None
