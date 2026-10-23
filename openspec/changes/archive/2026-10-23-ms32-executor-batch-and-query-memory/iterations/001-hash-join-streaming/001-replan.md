# Iteration 001 / Cycle 001: 哈希 JOIN 左表逐行（修订见证 SQL）

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: 2.1、2.2
- Acceptance gaps: 父 Cycle 的投影溢出 SQL 在计划期被拒绝，JOIN 重构没有开始
- Repair items: None
- Inherited scope: 父 Cycle 的 JOIN 重构要求、改前基线 8+8+6 通过、`join.rs` 尚未修改
- Excluded scope: 与父 Cycle 相同。不放宽 JOIN 的表达式投影拒绝。

**Objective**

与父 Cycle 相同：等值哈希 JOIN 结果不变，左表不再整表驻留，匹配桶不再整桶克隆。公共入口在 WHERE 谓词溢出时返回错误，不返回部分行。

**Background**

父 Cycle `000-initial` 把先行测试写成 JOIN 的 SELECT 表达式。`src/parser/planner/query.rs` 约 1916 与 1948 行对 JOIN 上的表达式投影返回 `Expression projection items are not supported with JOIN queries`。这是既有拒绝，不是本轮要打开的能力。Act 在 `:memory:` 上看到同一溢出表达式放进 WHERE 后，执行期返回 `Predicate evaluation error: Integer overflow in abs(-9223372036854775808)`。用户 2026-10-23 要求审计并解决阻塞，见证改为这条 WHERE。

**Investigation Facts**

- Current Baseline: `src/executor/join.rs` 仍是父 Cycle 调查时的三阶段实现，`left_rows` 还在。`tests/join_test.rs` 里的 `execute_sql_returns_err_not_partial_rows_on_join_projection_overflow` 当前失败，原因是计划期拒绝。改前三条测试文件为 8+8+6 passed，来自父 Cycle Act Response，本轮未重跑。
- Current-State Evidence: JOIN 的 WHERE 在 `query.rs` 约 2116 行编成包住 JOIN 的 `Filter`。`FilterExecutor::next` 对每一输出行求值谓词，错误文案前缀是 `Predicate evaluation error: `。`execute_executor` 遇到 `Err` 不返回已收集行。`i64::MIN` 仍用 `0 - 9223372036854775807 - 1` 构造。
- Code and Critical Path: 与父 Cycle 相同，只改 `src/executor/join.rs` 和 `tests/join_test.rs`。

**Implementation Guidance**

先替换那条红色测试，确认 WHERE 形态在改 `join.rs` 之前就已经返回溢出错误，再改执行器。不要保留投影形态的测试。

**Task Contracts**

### 2.1: 左表逐行探测

- Requirement/Scenario: 与父 Cycle 相同。见证从投影改为 WHERE。
- Depends on: Iteration 000
- Targets: `src/executor/join.rs::JoinExecutor`；`tests/join_test.rs`
- Current behavior: 与父 Cycle 相同。红色测试断言投影 SQL 含 `Integer overflow in abs`，实际是计划期拒绝。
- Required behavior: 与父 Cycle的重构要求相同。测试改为：`l`、`r` 各有主键 1 和 2，执行 `SELECT l.id FROM l JOIN r ON l.id = r.id WHERE abs(CASE WHEN l.id = 2 THEN 0 - 9223372036854775807 - 1 ELSE l.id END) > 0`。结果是 `Err`，消息包含 `Integer overflow in abs`，不是 `Ok`。
- Preserve: 与父 Cycle 相同，另加 JOIN 表达式投影的计划期拒绝。
- Forbidden: 与父 Cycle 相同。不得为了让投影 SQL 通过而去掉 `query.rs` 的拒绝。
- First test: 删除或改写 `execute_sql_returns_err_not_partial_rows_on_join_projection_overflow`，使文件里不再断言那条投影 SQL。新测试在修改 `join.rs` 之前运行，预期已经失败在溢出文案上（`Err` 且含 `Integer overflow in abs`）。命令：`cargo test --test join_test execute_sql_returns_err_not_partial_rows_on_join_where_overflow -- --test-threads=1`。测试名以这个为准。
- GREEN condition: 该测试与 `join_test`、`chain_join_test`、`self_join_test` 一起通过，且 `join.rs` 不再有 `left_rows`，输出路径不再 `cloned()` 整个匹配桶。
- Verification: `cargo test --test join_test --test chain_join_test --test self_join_test -- --test-threads=1` 退出码 0。
- Stop when: WHERE 形态也在计划期被拒绝，或改完后既有 JOIN 行集合变化。不要改投影拒绝来绕过。

### 2.2: 全量分批回归

- Requirement/Scenario: 与父 Cycle 相同
- Depends on: 2.1
- Targets: 无产品代码
- Current behavior: 父 Cycle 没有跑全量。Iteration 000 结束时 2125/0/2，本轮会替换一条测试而不是净增两条。
- Required behavior: 与父 Cycle 相同的 M19 分批。
- Preserve: 不新增测试框架外的判定脚本。
- Forbidden: 一次无过滤的 `cargo test`。
- First test: 无。
- GREEN condition: 每一批 failed 为 0。
- Verification: 各批计数写入 Act Response。
- Stop when: 某一批失败。范围内先修；范围外回到计划。

**Invariants**

与父 Cycle 相同。

**Non-goals**

与父 Cycle 相同。不实现 JOIN 上的表达式投影。

**Acceptance**

- 既有三条 JOIN 测试文件通过。
- WHERE 溢出测试返回 `Err`，消息含 `Integer overflow in abs`。投影形态的测试不再留在文件里。
- `JoinExecutor` 不再保存全部左行，不再克隆整个匹配桶。
- 2.2 各批 0 failed。

**Verification**

- WHERE 测试走 `Database::execute_sql`。
- 左表不整表驻留仍由删除 `left_rows` 核对。
- 全量沿用 Iteration 000 的分批方式。

**Gate 2 Readiness**

PASS。用户 2026-10-23 要求解决阻塞；见证改为 Act 已在真实入口看到的 WHERE 溢出，范围没有扩大。无 BLOCKED，无 WAIVED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

WHERE 的 `Filter` 在 JOIN 之上。溢出发生在输出行的谓词上，不是左表扫描内部。测试守公共入口不返回部分成功，不证明逐行。逐行仍由删除 `left_rows` 核对。

## Act Response

- Status: reported

### 实际改动

- `src/executor/join.rs::JoinExecutor`：
  - `JoinPhase` 删除 `ScanLeft`，收敛为 `BuildRight → Output` 两阶段。
  - 结构体状态 `left_rows: Vec<Vec<Value>>`、`current_left_index`、`current_right_matches: Vec<Vec<Value>>` 删除；替换为 `current_left: Option<Vec<Value>>`（当前探测中的左表行，`None` 表示需从左执行器取下一行）与 `current_match_key: Option<Vec<Value>>`（首探时缓存的命中桶键）；`current_right_index` 保留。
  - `next()` Output 阶段逐行拉取左表：取行 → 首探（`current_right_index == 0`）计算哈希键并缓存；键含 NULL 直接跳到下一左行；命中桶经键按需取回、按下标取当前右行，不再 `cloned()` 整桶；桶内匹配耗尽后推进到下一左行。左执行器非 `Row` 结果继续跳过（与原 `ScanLeft` 行为一致）。
  - `build_hash_key_right`/`build_hash_key_left`/`build_output_row`/`condition_column_index`、右表构建路径、`build_output_row` 逐单元格克隆全部未动；`Executor::next_batch` 默认方法未动。
- `tests/join_test.rs`：删除投影形态测试 `execute_sql_returns_err_not_partial_rows_on_join_projection_overflow`（父 Cycle 的红色现场），替换为 `execute_sql_returns_err_not_partial_rows_on_join_where_overflow`——`SELECT l.id FROM l JOIN r ON l.id = r.id WHERE abs(CASE WHEN l.id = 2 THEN 0 - 9223372036854775807 - 1 ELSE l.id END) > 0`，断言整句 `Err` 且消息含 `Integer overflow in abs`；新增 `use rtsql::database::Database` 导入。测试文件中不再有投影形态 SQL 断言。

### 与计划的偏差

- 无实质偏差。测试名与 SQL 按 Task Contract 逐字落地。
- 记录更正（父 Cycle 000-initial 已冻结，在此更正其 Act Response 的计数口径）：父 Cycle 改前基线写「join_test 8、chain_join_test 8、self_join_test 6」，实际 chain_join_test 基线为 9（该文件本轮未改动、现计数 9；被截断输出中可见的 "8 passed" 属 join_test）。基线合计 23 而非 22，不影响任何结论。

### 自检发现与遗留 Minor 问题

- clippy 命中测试中 `.err().expect(...)`，按建议改为 `.expect_err(...)`（`SqlOutcome` 已 derive `Debug`）；两行 INSERT 的 rustfmt 违规已按格式重排。修复后本轮两个目标文件 `cargo fmt --check` 干净、clippy 无 finding。
- 遗留 Minor（本轮范围外，不修，留 Plan Review 处置）：`src/executor/data_scan.rs:687`（Iteration 000 未提交交付的测试代码）`DataScanExecutor::new(...)` 换行不符合 rustfmt，`cargo fmt --check` 在该文件报 Diff。

### 验证命令与实际结果

- 改前（重构基线）：`cargo test --test join_test execute_sql_returns_err_not_partial_rows_on_join_where_overflow -- --test-threads=1` → 1 passed，见证在 `join.rs` 改动前即返回整句 `Err` 含 `Integer overflow in abs`。
- 改后（Task 2.1 Verification）：`cargo test --test join_test --test chain_join_test --test self_join_test -- --test-threads=1` → 9 + 9 + 6 = 24 passed / 0 failed，退出码 0。
- 结构核对：`grep "left_rows|ScanLeft|current_right_matches|current_left_index|cloned()" src/executor/join.rs` 零命中；JOIN 表达式投影计划期拒绝未触碰（`query.rs` 未修改）。
- Task 2.2 全量 M19 分批（`--test-threads=1` 逐批）：`--lib --bins` 一批 416 passed（bins 0）；`tests/*.rs` 128 文件字典序 13 批（12×10 + 8），各批 0 failed，批合计 1710 passed / 2 ignored（ignored 集中在 concurrent 批，与基线同源）。总计 **2126 passed / 0 failed / 2 ignored** = Iteration 000 终态 2125 + 1（替换后的见证测试），对账吻合。
- `cargo clippy --all-targets`：本轮改动文件无剩余 finding。

### Persisted Evidence

None required（Plan Context Mode: none；全部验证命令可从本 Response 复现）。

### Experience Candidates

None。

### 未解决问题

None。遗留 Minor（data_scan.rs 格式）已在自检发现中上报。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读 `git diff src/executor/join.rs` 与当前代码，重构与 Act 声明一致：`JoinPhase` 收敛为 `BuildRight → Output` 两阶段；`grep "left_rows|ScanLeft|current_right_matches|current_left_index" src/executor/join.rs` 零命中；输出路径 `right_hashmap.get(hash_key)` 按下标取当前右行（join.rs:195–201），无整桶 `cloned()`；`build_output_row` 逐单元格克隆保留（join.rs:129–138）。

行为等价独立核对：新实现输出序列 = 左执行器行序 × 命中桶内序，与旧三阶段「ScanLeft 收齐 + 索引推进」逐一对应；NULL 键跳行（join.rs:180–185）、非 `Row` 结果跳过（join.rs:171）、左表耗尽返回 `None`（join.rs:173）语义保持。`current_match_key` 跨行残留安全：该字段只在首探块（`current_right_index == 0`）之后被读，首探要么覆写键要么跳行（join.rs:178–188），`expect("match key cached while probing")` 不可达。错误传播位置从「首调用 ScanLeft 期」移到「到达失败行时」，公共入口 `execute_executor` 收集后 `Err` 不交部分行（父 Cycle 已核），由本轮见证测试守住。

`git diff tests/join_test.rs`：投影形态 SQL 断言不在文件中，替换为 WHERE 形态，SQL 与 Task Contract 逐字一致，`.expect_err` 断言消息含 `Integer overflow in abs`。相对 HEAD 净 +1 条测试，与 2125 + 1 = 2126 对账吻合。

沿用 Act 验证结论（覆盖未失效——只读 `git status` 核对工作区 7 文件与 Act 声明一致，Act 后无变化）：定向 `join_test`/`chain_join_test`/`self_join_test` = 9 + 9 + 6 = 24 passed / 0 failed；全量 M19 分批 2126 passed / 0 failed / 2 ignored；clippy 本轮两文件无 finding；`cargo fmt --check` 在 join.rs、join_test.rs 无 Diff。

Act 记录更正确认：chain_join_test 改前基线为 9（父 Cycle Act 误记 8），基线合计 23，改后 24 = 23 + 净增 1，自洽。父 Cycle 已冻结，本轮 Act 在 Response 内更正，无需动作。分类：现状澄清。

Minor finding（不阻塞，本轮范围外）：`cargo fmt --check` 独立复核，全库仅 `src/executor/data_scan.rs:687` 一处 Diff——Iteration 000 交付的 next_batch 同序测试代码 `DataScanExecutor::new(...)` 换行不合 rustfmt。Act 已如实上报。

工作区其余改动归因（不在本轮范围，无发现）：`executor_trait.rs`、`data_scan.rs`、`mod.rs`、`benches/data_scan_bench.rs` 属 Iteration 000 已 accepted 交付；`AGENTS.md` 为 openspec-init 规则升级（新增「轻量工作」段等），非产品代码。

**Acceptance Gaps**

无。四项 Acceptance 全部满足：三条既有 JOIN 测试文件通过（沿用 Act）；WHERE 溢出测试 `Err` 且含 `Integer overflow in abs`、投影形态测试不在文件（独立核对 diff）；`JoinExecutor` 不再保存全部左行、不再克隆整桶（独立核对代码与 grep）；2.2 各批 0 failed（沿用 Act，对账吻合）。

**Evidence**

- `git diff src/executor/join.rs`、`git diff tests/join_test.rs`、`grep "left_rows|ScanLeft|current_right_matches|current_left_index" src/executor/join.rs`（零命中）
- `src/executor/join.rs:163–207`（Output 阶段逐行探测）、`:129–138`（build_output_row）
- `cargo fmt --check`：仅 data_scan.rs:687 一处 Diff
- Act Response 的定向与全量验证结论（沿用，来源已注明）

**Follow-up Decision**

accepted。重构目标（左表不整表驻留、匹配桶不整桶克隆）由代码结构证实，行为等价由既有 JOIN 矩阵、见证测试与全量回归证实，无阻塞项。

data_scan.rs:687 格式 Minor 不阻塞任何 Acceptance，按公共规则不为之返工，也不并入 Iteration 002 范围。它是本 change 产生的孤儿格式问题，change 收尾前需清除：纯格式修改属轻量工作（AGENTS › 轻量工作），待用户授权执行 `cargo fmt` 一行修复即可。

**Iteration Plan Update**

无。四轮 Map 不变。

**Next Cycle**

None

**Next Iteration**

`iterations/002-bounded-sort/000-initial.md`（按既有 Map 展开，Status: ready，Gate 2 沿 2026-10-23 用户批准的四轮计划与 design D3）
