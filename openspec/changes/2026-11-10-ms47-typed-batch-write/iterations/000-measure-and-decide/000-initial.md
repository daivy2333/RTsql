# Iteration 000 / Cycle 000: 测量并裁定

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS47 的测量与 10% 裁定；`specs/typed-batch-write/spec.md` 的测量 requirement，以及未达标时不新增入口的 requirement。达标时的入口 requirement 只作为 T2 的分支记录，不在本 Cycle 实现。
- Excluded scope: T3、`insert_batch`、import、CSV、全量测试、Iteration 001 的目录。

**Objective**

在 release 测试里打印 100 条逐行插入的 `total_ns` 与 `execute_exclusive_ns`，用 `execute_exclusive_ns * 10` 与 `total_ns` 的整数比较裁定建入口或不建入口，并把裁定写进本 Cycle 的 Act Response。产品代码保持不变。

**Investigation Facts**

- Starting point: 分支 `changes/2026-11-10-ms47-typed-batch-write`，基点 `937463c7dfa8ab6bec400e2573b11bdd4b72103b`。当前没有 `insert_batch`。导入仍在 `src/cli/lifecycle.rs` 对每一行 `execute_sql`。本 Cycle 不读另一个工作目录里的未提交内容。
- Current-State Evidence: `openspec/specs/unified-diagnostics/spec.md` 规定语句总耗时是根阶段包含时间，插入的提交是执行的子阶段，执行阶段独占时间加上提交包含时间等于执行阶段包含时间。`src/diagnostics.rs` 的 `Summary.total`、`StageStat.exclusive_total` 和 `StageStat.inclusive_total` 是这些数的字段。`src/pipeline.rs` 的 `execute_staged` 对未命中缓存的语句打开 parse、plan、execute；`src/database.rs` 的 `commit` 在阶段绑定存在时打开名为 `commit` 的子阶段。`benches/common/mod.rs` 的 `insert_rows` 与 `benches/sqlite_compare.rs` 的 `rtsql_insert_100` 使用同一 SQL 形状。变更前命令与结果：在本工作目录执行 `cargo test --offline --test diagnostics_statement_test insert_commit_is_child_of_execute -- --test-threads=1`，得到 `insert_commit_is_child_of_execute ... ok`，`1 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out`。
- Code and Critical Path: 测量不改这条路径。`Database::execute_sql` → `pipeline::execute` → `execute_staged` → `execute_stage` 的 `PhysicalPlan::Insert` 臂 → `InsertExecutor::next`。诊断读回用 `Database::set_diagnostics` 与 `Database::diagnostic_summaries`。`tests/diagnostics_statement_test.rs` 的 `stats_config` 只打开 `Collect.stats`。本测量要把 `summary_limit` 设为 100，因为默认 32 会丢掉最早的摘要。

**Implementation Guidance**

先加测试并跑 release。测试通过就说明当前诊断已经能提供两个时间，不要为了测量去改引擎。T2 只做算术和记录，不第二次运行该测试。无论哪一侧，都不要在本 Cycle 添加 `insert_batch`。

**Task Definitions**

### T1: 打印一百条插入的两个时间

- Requirement/Scenario: `typed-batch-write` / 「一百条插入给出两个时间且行可见」；「总耗时为零时测量无效」由测试直接失败来表达，不把 0 印成一份可用裁定。
- Depends on: None
- Targets: `tests/typed_batch_write_test.rs` 的 `row_insert_share_of_hundred_inserts`。不改 `src/`。
- Current behavior: 单条 `INSERT INTO t VALUES (1)` 在只开性能统计时产生一条摘要，含 `Module::Pipeline` 的 parse、plan、execute 和 `Module::Wal` 的 commit，并且 `execute.exclusive_total + commit.inclusive_total == execute.inclusive_total`。没有 100 条 `bench` 插入的合计打印。
- Required behavior: 测试函数使用 `#[tokio::test]`。使用文件库 `Database::open`。先 `execute_sql` 建表 `CREATE TABLE bench (id INT PRIMARY KEY, name STRING, value INT)`，再 `set_diagnostics`。配置为 `collect.stats = true`，`events` 与 `trace` 为 false，`modules` 为 `ModuleSet::All`，`summary_limit` 为 100，其余字段用 `DiagnosticsConfig::default()`。然后对 `i` 从 0 到 99 把整数字面量写进 SQL 文本，执行 `INSERT INTO bench VALUES ({i}, 'user_{i}', {i * 10})`，不用参数绑定。立刻 `diagnostic_summaries()`，在任何后续 SQL 之前完成断言和打印。断言：长度为 100；每条 `severity` 为 `Severity::Info`；每条 `sql` 含 `INSERT INTO bench VALUES`；每条 `total > 0`；每条能取到 `Module::Pipeline` 的 parse、plan、execute 和 `Module::Wal` 的 commit；每条满足 `execute.exclusive_total + commit.inclusive_total == execute.inclusive_total`。`total_ns` 是 100 条 `summary.total.as_nanos()` 的 `u128` 和。`execute_exclusive_ns` 是 100 条 execute 阶段 `exclusive_total.as_nanos()` 的 `u128` 和。测试用 `println!` 打印恰好一行：`ms47_insert_share statements=100 total_ns={total_ns} execute_exclusive_ns={execute_exclusive_ns}`。打印后再执行 `SELECT COUNT(*) FROM bench`，结果为 `SqlOutcome::Rows`，唯一单元格是 `Value::Int(100)`。测试不断言这两个整数的大小关系。
- Preserve: 产品代码、导入、既有诊断测试的断言。
- Forbidden: 修改 `src/`；用测试外的脚本求和或判断 10%；把 `DELETE`、打开数据库或建表算进两个整数；把提交时间加进分子；把各阶段包含时间相加当作语句总耗时；为了让数字更大而改 SQL 形状或行数。
- First test: 本测试就是对当前行为的观察。命令：`cargo test --offline --release --test typed_batch_write_test row_insert_share_of_hundred_inserts -- --test-threads=1 --nocapture`。预期：退出码 0，输出含上述一行，`bench` 的行数断言通过。当前引擎已有阶段数据，测试应直接通过。若编译失败或断言失败，停止，不要改产品代码去凑绿。
- GREEN condition: 同一命令退出码 0，failed 为 0，输出含 `ms47_insert_share statements=100 total_ns=` 与 `execute_exclusive_ns=`。
- Verification: 上一条命令。通过表示 100 条摘要和 100 行插入成立。失败表示诊断合同或负载与计划不一致。
- Stop when: 必须修改 `src/` 才能让摘要含有 execute 与 commit，或 `total_ns` 为 0。这两种情况都回到计划，不把比例记成低于 10%。

### T2: 用整数比较裁定入口

- Requirement/Scenario: 测量 requirement 的两个时间；「低于百分之十时不新增入口」。达到 10% 时只记录「建入口」，不实现入口 requirement。
- Depends on: T1
- Targets: 本文件的 Act Response。低于 10% 时同时在 Act Response 写明产品代码没有 `insert_batch`。不改 `src/`。
- Current behavior: T1 的 stdout 有一行两个纳秒数。尚无裁定。
- Required behavior: 从 T1 那一次命令的输出读取 `total_ns` 与 `execute_exclusive_ns`，不重新运行。`total_ns` 为 0 则本任务阻塞，不裁定。`execute_exclusive_ns.checked_mul(10)` 为 `None` 则阻塞，不裁定。否则当 `execute_exclusive_ns * 10 >= total_ns` 时，Act Response 写「建入口」；当 `execute_exclusive_ns * 10 < total_ns` 时写「不建入口」。两种写法都带上两个整数和比较式。不建入口时写明 `src/` 无改动。建入口时也写明本 Cycle 没有添加 `insert_batch`。
- Preserve: T1 的测试和产品代码。
- Forbidden: 重跑测试来换一侧结果；改门槛；在本 Cycle 实现 `insert_batch` 或展开下一轮目录；把浮点比例四舍五入后再比较。
- First test: 无新测试。沿用 T1 的一次输出。
- GREEN condition: Act Response 含有两个整数、比较式和「建入口」或「不建入口」之一。
- Verification: 对照 T1 命令输出中的那一行。整数与输出一致，比较式与整数一致。不一致则裁定无效。
- Stop when: 输出里找不到该行，或 `total_ns` 为 0，或乘法溢出。写 Blocker Handoff，不要猜一个比例。

**Invariants**

- 每次 `cargo test` 都带 `--test-threads=1`。本 Cycle 只有一个测试目标。
- 不跑全量，不并行跑多个测试目标。
- 诊断只提供时间，测试的通过条件不使用 10% 门槛，也不使用操作标识、运行标识或内容哈希。
- `execute_sql` 的插入行为在本 Cycle 结束时与开始时相同。

**Non-goals**

- 不实现 `Database::insert_batch`。
- 不修改 `src/cli/lifecycle.rs`、CSV 或导入。
- 不创建 Iteration 001 的目录。
- 不新增放弃 future 的测试，不改 `InsertExecutor`。

**Acceptance**

- T1 的 release 命令退出码 0，输出含 `ms47_insert_share statements=100` 以及两个纳秒字段，测试断言 100 条摘要和 100 行。
- T2 的 Act Response 用这两个整数做了 `* 10` 比较，并写下「建入口」或「不建入口」。
- `src/` 相对本 Cycle 开始时没有产品改动。
- 低于 10% 时，Act Response 同时满足「低于百分之十时不新增入口」的记录要求。达到 10% 时，入口代码仍不在本 Cycle。

**Verification**

测量场景的判定就是 T1 的那一条 `cargo test --release` 命令：退出码、`test result` 的 failed 计数，以及 `--nocapture` 打印的那一行。不另写求和脚本。裁定场景不重新执行命令，只核对 Act Response 与该行。没有跨进程的端到端命令；负载走的是库的 `execute_sql` 入口，与现有插入基准相同。

**Gate 2 Readiness**

PASS。无 BLOCKED。无 SKIPPED。

**Persisted Evidence**

- Mode: none

两个纳秒数和裁定写入 Act Response。测试可再跑，不把 stdout 另存进 `evidence/`。

**Risks and Notes**

- 靠近 10% 时另一次运行可能落到另一侧。本 Cycle 不重跑，不改门槛。
- 分子不含提交。提交占主导而得到「不建入口」是预定结果。
- 全量不在本变更任何一轮。用户原话：「这次自动化把全量测试放到最后来限制全量的次数」。风险：本变更收尾前没有全量；合回后那一次失败时，排查范围包含 MS46 与 MS47。
- 行宽、视图和空批次的可见行为属于 T3。本 Cycle 不实现它们。T3 的检查顺序、签名和错误对照在 `design.md` 与 `tasks.md` 的 T3，供达标后的下一轮抄入新的 Cycle，不要求本 Cycle 的 Act 去读聊天记录。

## Act Response

- Status: reported

**Implemented**

`tests/typed_batch_write_test.rs` 新增 `row_insert_share_of_hundred_inserts`。它打开文件库，建表后再打开统计，对 0 到 99 各执行一条整数字面量插入，在后续 SQL 之前打印两个 `u128` 和，再断言 `bench` 有 100 行。`src/` 无改动，产品代码没有 `insert_batch`。

裁定：不建入口。`total_ns = 332234286`，`execute_exclusive_ns = 9804119`，`9804119 * 10 < 332234286`。`9804119 * 10 = 98041190`，未溢出；`total_ns` 不是 0。

**Deviations from Plan**

第一次调用测量命令时，测试把 `PathBuf` 传给 `Database::open`，编译失败，没有打出纳秒行。测试改为传入 `&Path` 后，下面记录的这一次命令才是测量。没有在打出该行之后重跑。

**Verification Evidence**

命令：

```text
cargo test --offline --release --test typed_batch_write_test row_insert_share_of_hundred_inserts -- --test-threads=1 --nocapture
```

退出码 0。`test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.34s`。

`ms47_insert_share statements=100 total_ns=332234286 execute_exclusive_ns=9804119`

**Persisted Evidence**

None required。

**Experience Candidates**

None

**Open issues**

无。本 Cycle 不添加入口代码，不展开下一轮。

## Plan Review

- Review Result: pending

**Findings**

尚未 Review。

**Evidence**

尚未 Review。

**Follow-up Decision**

尚未 Review。

**Next Cycle**

None

**Next Iteration**

None
