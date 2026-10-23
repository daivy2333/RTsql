# Iteration 000 / Cycle 000: 批量接口

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: 1.1、1.2、1.3、1.4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 假设 A3；design D1；`executor-batch-interface` 两条 requirement
- Excluded scope: 哈希 JOIN、Top-N、查询内存上限、Value 表示、溢写、结果流式消费

**Objective**

`Executor::next_batch` 与连续 `next` 产出相同序列，`max = 0` 不消费。生产路径只有在 10000 行 DataScan 的批量平均时间比逐行基线至少低 10% 时才改为攒批；否则保持逐行。两种结局的查询结果都与现在一致。

**Investigation Facts**

- Current Baseline: 工作区 `a0c1ed4`，本轮读取时工作区干净。`Executor` 只有 `next`。无 WHERE 的单表计划在 `src/parser/planner/query.rs` 约 2354 行变为 `DataScan`。`DataScanExecutor::next`（`src/executor/data_scan.rs` 约 370 行）按页按槽返回一行。`execute_executor`（`src/pipeline.rs` 约 1576 行）循环 `next` 直到 `None`。`benches/data_scan_bench.rs` 已对 1000 与 10000 行比较 `ScanExecutor` 和 `DataScanExecutor` 的 `next` 抽取。没有查询内存预算类型。本轮没有重跑测试，既有全量 2123/0/2 来自 `.agents/docs/tasks.md` 对 MS31 的记录，不作为本 Cycle 的新基线。
- Current-State Evidence: 调用链是 `Database::execute_sql` → `pipeline::execute_stage` → `create_executor_from_plan` → `Box<dyn Executor>` → `execute_executor`。`Filter`、`Projection`、`Limit`、`Having` 每次 `next` 拉一次孩子。整表收齐循环位于排序初始化、哈希 JOIN 两侧、嵌套循环右表、Distinct、集合运算、聚合、半连接、反连接、子查询求值、派生表物化、`materialize_insert_select_source` 和 CTAS 源物化。`StubRowExecutor` 在 `src/executor/mod.rs`，仅测试构造。`async-trait` 版本是 Cargo.toml 的 `0.1`。
- Code and Critical Path: 默认方法加在 `src/executor/executor_trait.rs`。攒批若保留，改 `DataScanExecutor::next_batch`，让同一次调用沿现有页循环收集最多 256 行，不调用 `self.next`。转发层是 Filter、Projection、Limit、Having。生产抽取改为 `next_batch(256)` 的位置就是上面的收齐循环和 `execute_executor`。

**Implementation Guidance**

先写 1.1 的失败测试再加默认方法。基线必须在攒批实现之前保存，且当时 `next_batch` 仍是默认的逐行循环。时间门失败时撤回攒批和所有生产接线，不撤回默认方法和 1.1 的测试。基准参数使用 `--sample-size 10 --measurement-time 2`，criterion 参数放在 `--` 之后。

**Task Contracts**

### 1.1: 默认 next_batch 与逐行同序

- Requirement/Scenario: executor-batch-interface「批量取出与逐行取出同序」；分批取完；批量大小为零
- Depends on: None
- Targets: `src/executor/executor_trait.rs::Executor::next_batch`；`src/executor/mod.rs` 的 `StubRowExecutor` 测试
- Current behavior: trait 没有 `next_batch`。`StubRowExecutor::next` 按构造顺序吐行。
- Required behavior: `next_batch(max)` 返回与连续 `next` 相同的 `ExecResult` 序列。`max == 0` 返回空向量且不调用 `next`。默认实现供未覆盖的执行器使用。
- Preserve: 现有 `next` 的签名和各执行器的逐行语义。
- Forbidden: 在本任务修改 DataScan、JOIN、Sort 或 pipeline 抽取循环。
- First test: 在 `src/executor/mod.rs` 增加 `next_batch_matches_rows`。5 行 `StubRowExecutor`，`next_batch(2)` 三次得到 2、2、1，然后空；另造一个执行器，`next_batch(0)` 后第一次 `next` 仍是第一行。先运行并确认编译失败。命令：`cargo test --lib next_batch_matches_rows -- --test-threads=1`。
- GREEN condition: 同一命令通过。
- Verification: 上述命令退出码 0。
- Stop when: `async-trait` 0.1 不能为该 trait 提供默认异步方法。不要改成 33 份手写转发。

### 1.2: 保存逐行批量基线

- Requirement/Scenario: executor-batch-interface「测量未达标」的前置基线
- Depends on: 1.1
- Targets: `benches/data_scan_bench.rs`
- Current behavior: 基准只循环 `next`，没有 `next_batch(256)` 的 10000 行项。
- Required behavior: 增加一个 10000 行 DataScan、调用 `next_batch(256)` 并数满行数的基准项。在攒批实现之前保存名为 `before-ms32-batch` 的 criterion 基线。此时走的是默认逐行循环。
- Preserve: 原有 `scan_via_index` 与 `data_scan` 基准项仍可运行。
- Forbidden: 在保存基线之前修改 `DataScanExecutor::next` 或 `next_batch` 的覆盖实现。
- First test: 不新增断言测试。本任务是变更前观测。
- GREEN condition: 基线命令退出码 0。
- Verification: `cargo bench --bench data_scan_bench -- next_batch/10000 --save-baseline before-ms32-batch --sample-size 10 --measurement-time 2 --noplot`。基准 ID 以实际 `BenchmarkId` 为准，但必须是 10000 行且调用 `next_batch(256)`。Act Response 记录 criterion 打印的平均时间。
- Stop when: 基线命令不能在本机完成。不要改用别的样本数来换一个更容易通过的数字。

### 1.3: 按时间门决定是否保留攒批

- Requirement/Scenario: executor-batch-interface「批量生产路径不改变查询结果」；无 WHERE 的全表查询；测量未达标
- Depends on: 1.2
- Targets: `src/executor/data_scan.rs::DataScanExecutor::next_batch`；`filter.rs`、`projection.rs`、`limit.rs`、`having.rs` 的 `next_batch`；`pipeline.rs` 中 `execute_executor`、`DerivedScan` 物化、`materialize_insert_select_source`、CTAS 源物化；排序、两类 JOIN、Distinct、集合运算、聚合、半连接、反连接、子查询收齐循环
- Current behavior: 这些循环调用 `next`。DataScan 一次调用产一行。
- Required behavior: 先让 DataScan 在一次 `next_batch` 内沿现有页循环收集最多 `max` 行，且不调用 `self.next`。转发层改拉孩子的 `next_batch`。上面的收齐循环改用 `next_batch(256)`。然后重跑 1.2 的同一基准项并对比 `before-ms32-batch`。批量平均时间至少低 10% 时保留这些改动。否则撤回本任务的生产改动，1.1 的默认方法和测试保留。查询行序与逐行路径一致。
- Preserve: `next` 仍可用。页可见性、`scan_cap`、谓词下推和投影语义不变。
- Forbidden: 为了通过时间门更换分配器、改 Value、或只在基准里走一条生产代码不会走到的捷径。
- First test: 在库测试中建一张小表，分别用 `next` 和 `next_batch(64)` 抽干同一个 DataScan，断言行序列相等。实现攒批前这条测试应已通过（默认方法）。命令：`cargo test --lib data_scan_next_batch_matches_next -- --test-threads=1`。
- GREEN condition: 同序测试通过；时间门的保留或撤回已完成，两种结局都记录两个平均时间。
- Verification: 同序命令退出码 0。对比命令：`cargo bench --bench data_scan_bench -- next_batch/10000 --baseline before-ms32-batch --sample-size 10 --measurement-time 2 --noplot`。通过条件是 criterion 报告的批量平均时间 ≤ 基线平均时间的 90%。撤回后再次运行同序测试，确认默认方法仍通过。
- Stop when: 对比输出无法读出两个平均时间，或保留攒批必须改变 `next` 的可见语义。

### 1.4: 全量分批回归

- Requirement/Scenario: executor-batch-interface 两条 requirement 的回归面
- Depends on: 1.3
- Targets: 无产品代码。验证 1.1–1.3 结束后的工作区。
- Current behavior: MS31 记录的全量终态是 2123/0/2，那是归档时的数字，不是本任务的预期总数。
- Required behavior: 按项目模型 M19 跑完当前测试目标。`--lib --bins` 单独一批。`tests/*.rs` 按文件名字典序，每 10 个文件一批。每批命令形如 `cargo test --test <file> ... -- --test-threads=1`。全部批次 0 failed。
- Preserve: 不新增测试框架外的判定脚本。
- Forbidden: 一次无过滤的 `cargo test`，或去掉 `--test-threads=1` 的全量批次。
- First test: 无新测试。本任务消费 1.1 和 1.3 已通过的测试。
- GREEN condition: 每一批的 failed 计数为 0，且没有漏批。
- Verification: 上述命令的退出码和 failed 计数。Act Response 记录批次数和各批 passed/failed，不粘贴完整日志。
- Stop when: 某一批失败。先修本 Iteration 范围内的失败；范围外的失败回到计划。

**Invariants**

- 未启用攒批时，查询结果与 `a0c1ed4` 的逐行执行一致。
- 启用攒批后，同一查询的行序列仍与逐行执行一致。
- `dyn Executor + Send` 继续是执行器的公共装配方式。

**Non-goals**

哈希 JOIN 的左表缓存、Top-N、查询内存上限、EXPLAIN、CLI 环境变量。

**Acceptance**

- 1.1 的测试证明分批同序和 `max = 0` 不消费。
- 1.3 的同序测试证明 DataScan 批量与逐行一致；时间门决定生产接线是否保留，两个平均时间写在 Act Response。
- 1.4 的各批 0 failed 证明接线没有破坏既有查询。

**Verification**

- 同序与零批量：`cargo test --lib next_batch_matches_rows -- --test-threads=1`，以及 1.3 的 `data_scan_next_batch_matches_next`。
- 时间门：1.2 的 `--save-baseline before-ms32-batch` 和 1.3 的 `--baseline before-ms32-batch`。判定是平均时间，不是基准是否「看起来更快」。
- 全量：1.4 的分批命令。无 WHERE 全表查询由既有扫描测试在全量中覆盖，不另写脚本比较输出文件。

**Gate 2 Readiness**

PASS。用户 2026-10-23 原话「批准」。无 BLOCKED，无 WAIVED。

**Persisted Evidence**

- Mode: none

基准平均时间写入 Act Response 即可。criterion 的 `target/criterion` 产物可以重跑，不另存进变更目录。

**Risks and Notes**

时间门可能因噪声撤回攒批。这是 D1 的预定结局，不是失败。撤回后 1.1 和全量仍要通过。批量大小 256 是接线常数；改它会让基线失去意义，本 Cycle 不改。

## Act Response

- Status: reported
- Cycle Type: initial
- Iteration: 000 批量接口
- 实施日期: 2026-10-23
- 起点 revision: `a0c1ed4`

### 实际改动

保留的文件与符号：

| 文件 | 符号 | 内容 |
|---|---|---|
| `src/executor/executor_trait.rs` | `Executor::next_batch` | 新增默认方法：逐行调用 `next` 直到攒满 `max` 行或输入耗尽；`max == 0` 返回空集合且不调用 `next`，不推进游标。`async-trait` 0.1 支持默认方法体，无需 33 份手写转发。 |
| `src/executor/mod.rs` | `batch_interface_tests::next_batch_matches_rows` | 1.1 先行测试：5 行 `StubRowExecutor` 按 2 分批得 2/2/1 后空批；`next_batch(0)` 后第一次 `next` 仍是第一行；分批序列与逐行序列逐项相同。 |
| `src/executor/data_scan.rs` | `tests::data_scan_next_batch_matches_next` | 1.3 同序测试：同一张 3 行表，两个独立 DataScan 执行器分别用 `next` 与 `next_batch(64)` 抽干，行序列逐项相同。 |
| `benches/data_scan_bench.rs` | `data_scan_vs_scan/next_batch/<n>` | 1.2 基准项：10000 行 DataScan 调 `next_batch(256)` 并数满行数；原有 `scan_via_index` 与 `data_scan` 项未改。 |

已撤回的文件与符号（1.3 的生产改动，按 D1 时间门判定）：

- `src/executor/data_scan.rs`：`DataScanExecutor::next_batch` 覆盖实现与 `next_row` 私有方法拆分。
- `src/executor/filter.rs`、`projection.rs`、`limit.rs`、`having.rs`：转发层 `next_batch` 覆盖实现。
- `src/executor/sort.rs`、`join.rs`、`nested_loop_join.rs`、`distinct.rs`、`set_operation.rs`、`aggregate.rs`、`semi_join.rs`、`anti_join.rs`、`subquery_eval.rs`、`scalar_subquery_filter.rs`：收齐循环改批量。
- `src/pipeline.rs`：`execute_executor`、`materialize_insert_select_source`、CTAS 源物化、DerivedScan 物化改批量。
- `EXECUTOR_BATCH_SIZE` 接线常数：1.3 引入，撤回后无其他引用，作为孤儿一并删除。

### 测量结果

| 运行 | criterion mean | slope | median |
|---|---|---|---|
| 基线（1.2，默认逐行实现，攒批实现之前保存） | 19.467 ms | 19.049 ms | 19.139 ms |
| 对比（1.3，DataScan 攒批 + 生产接线） | 19.061 ms | 18.307 ms | 18.668 ms |

criterion 报告 `change: [-7.0631% -2.0860% +3.1362%] (p = 0.48 > 0.05)`，`No change in performance detected`。

判定：批量平均时间未达到基线平均时间的 90%（门槛 17.520 ms），时间门未通过。按 D1 与 Task Definition 撤回 1.3 的生产改动，保留 1.1 的默认方法与测试。这是 Plan Context「Risks and Notes」写明的预定结局，不是失败。

### 与计划的偏差

1. 1.3 的收尾是撤回而非保留——计划已声明的结局，不计为偏差。
2. 撤回时删除了 1.3 引入的 `EXECUTOR_BATCH_SIZE` 常量（撤回后无引用）。
3. 1.3 的同序测试在撤回后保留：它是默认方法的同序守护，不属生产改动；Plan Context 的 First test 已要求它在攒批实现前就通过，撤回后仍通过。
4. 实施 1.3 时曾修正 `distinct.rs`、`aggregate.rs` 两处由改动形状引入的 `clippy::single_match`；撤回后这两处随生产改动回到基线状态。

### 自检发现与遗留 Minor 问题

- Gate 3 逐任务执行：1.1 覆盖分批同序与 `max = 0` 不消费；1.2 覆盖基线命令退出码与基准 ID；1.3 覆盖两个平均时间与撤回后同序测试重跑；1.4 覆盖各批 failed 计数。
- 遗留 Minor（未修复）：`Executor` trait 新增一个 async 方法后，`async-trait` 宏生成的 `clippy::double_must_use` 实例由 3 个增至 6 个。同一 lint 类在 `src/storage/async_storage.rs`（12 个）与 `src/network/protocol.rs`（6 个）已存在且未被抑制，与项目现状保持一致；未新增 lint 类别。

### 验证命令与实际结果

| 命令 | 结果 |
|---|---|
| `cargo test --lib next_batch_matches_rows -- --test-threads=1` | 退出码 0；1 passed / 0 failed |
| `cargo test --lib data_scan_next_batch_matches_next -- --test-threads=1` | 退出码 0；1 passed / 0 failed（撤回后重跑） |
| `cargo bench --bench data_scan_bench -- next_batch/10000 --save-baseline before-ms32-batch --sample-size 10 --measurement-time 2 --noplot` | 退出码 0；基线 mean 19.467 ms |
| `cargo bench --bench data_scan_bench -- next_batch/10000 --baseline before-ms32-batch --sample-size 10 --measurement-time 2 --noplot` | 退出码 0；change -2.09%，未达门槛 → 撤回 |

1.4 全量分批（M19：`--lib --bins` 单独一批，`tests/*.rs` 按文件名字典序每 10 个一批，批间顺序执行，每批 `--test-threads=1`）：

| 批 | 目标 | passed | failed |
|---|---|---|---|
| A | `--lib --bins` | 416 | 0 |
| 1 | aggregate_extension_test … busy_timeout_test（10） | 148 | 0 |
| 2 | cache_perf_test … concurrent_test（10） | 182 | 0 |
| 3 | connection_limit_test … ddl_crash_recovery_test（10） | 147 | 0 |
| 4 | default_declaration_test … explicit_tx_test（10） | 161 | 0 |
| 5 | expression_e2e_test … insert_select_test（10） | 100 | 0 |
| 6 | isolation_level_test … mvcc_commit_test（10） | 89 | 0 |
| 7 | mvcc_record_test … param_binding_test（10） | 117 | 0 |
| 8 | parser_test … prefetch_test（10） | 193 | 0 |
| 9 | prepared_statement_test … recovery_test（10） | 86 | 0 |
| 10 | recovery_wal_convergence_test … set_operations_test（10） | 133 | 0 |
| 11 | sort_test … table_manager_test（10） | 146 | 0 |
| 12 | truncate_test … visibility_test（10） | 148 | 0 |
| 13 | wal_auto_checkpoint_test … write_type_conformance_test（8） | 59 | 0 |

合计 14 批、全部退出码 0，2125 passed / 0 failed / 2 ignored。相对 MS31 归档时的 2123/0/2，多出的 2 个是本 Cycle 新增的 `next_batch_matches_rows` 与 `data_scan_next_batch_matches_next`，ignored 数不变。`tests/` 共 128 个 `.rs` 文件且无子目录测试目标，`src/` 无 doctest，因此 14 批覆盖当前全部测试目标，无漏批。

### Persisted Evidence

None required。Plan Context 模式为 `none`；两个平均时间写入本响应。criterion 产物在 `target/criterion/data_scan_vs_scan/next_batch/10000/`，可重跑，不另存进变更目录。

### Experience Candidates

- Runbook：无新增候选。criterion 基线采集与前后对比的流程已由 `.agents/runbooks/ms08-bench-comparison.md`（references.md R17）覆盖，本 Cycle 沿用其 `--save-baseline` / `--baseline` 形态并验证成功，不另立。
- Issue 候选（范围外，只报告不登记）：`.agents/docs/tasks.md` 的 MS22 段把 bench 基线纪律 Runbook 标为「R26/ms08-bench runbook」，实际编号是 R17（references.md:197）；R26 是 RTsql vs SQLite 跨引擎对比 Runbook（references.md:271）。本 Cycle 实际沿用的是 `.agents/runbooks/ms08-bench-comparison.md`。
- Issue 候选（范围外，只报告不登记）：`.agents/docs/SNAPSHOT.md` 的「仓库现场」段仍写 MS31 交付现场未提交、「变更：无活跃 change」，与当前工作区不符，由 Maintainer 收尾时刷新。

### 未解决问题

无。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读了相对 `a0c1ed4` 的 diff 和当前 `Executor::next_batch`。产品改动与 Act Response 一致：默认方法在 `max == 0` 时直接返回空向量；`src` 里没有 `next_batch` 的覆盖实现，也没有 `EXECUTOR_BATCH_SIZE`。`tests/*.rs` 共 128 个文件，与 12 批整十个加最后 8 个的分批方式相符。

`target/criterion/data_scan_vs_scan/next_batch/10000/` 的 estimates 与响应里的两个平均时间一致：基线 19.467 ms，对比 19.061 ms，变化约 -2.09%。90% 门槛是 17.520 ms，撤回生产攒批符合 D1。验证命令没有重跑，沿用 Act Response，并用这份 estimates 核对了时间门。

两条不阻塞的记录：

- `benches/data_scan_bench.rs` 的注释仍写对比运行会换成 DataScan 自己的攒批实现。实现已撤回，基准现在测的是默认方法。注释过时，不改变基准行为。
- 工作区里 `AGENTS.md` 有规则措辞改动。Act Response 没有声称它，它也不在本轮验收范围内。

`double_must_use` 计数增加是 Act 已记录的 Minor，现有文件里同类 lint 未被抑制，不要求修。

**Acceptance Gaps**

无。

**Evidence**

- `src/executor/executor_trait.rs` 的默认 `next_batch`
- `src/executor/mod.rs` 的 `next_batch_matches_rows`
- `src/executor/data_scan.rs` 的 `data_scan_next_batch_matches_next`
- `benches/data_scan_bench.rs` 的 `next_batch` 项
- criterion estimates：基线 point 19466934.69 ns，对比 point 19060854.06 ns
- Act Response 的 14 批计数，本轮未重跑

**Follow-up Decision**

时间门未通过是计划写明的结局，Acceptance 已满足。不要求当前 Cycle 再改。Iteration Plan 不变。

**Iteration Plan Update**

**Next Cycle**

None

**Next Iteration**

`iterations/001-hash-join-streaming/000-initial.md`
