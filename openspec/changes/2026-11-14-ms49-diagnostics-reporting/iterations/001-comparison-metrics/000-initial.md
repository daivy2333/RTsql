# Iteration 001 / Cycle 000: 比较指标

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 提案、design 和 `tasks.md` 已经锁定的读出口径。首行等待是语句开始到第一行；无行语句没有首行等待。吞吐是保留摘要里的交回行数之和除以这段墙钟，单位是行每秒。P95 是这些语句总耗时的最近秩上界。进程峰值在 Linux 上是 `ru_maxrss * 1024`。
- Excluded scope: T4 的全量；五段 `record_stage` 接线的返工；显式事务路径上的新测量；全局 tasks、SNAPSHOT、M/R/I 和 `openspec/specs/` 正文。

**Objective**

调用方从 `diagnostic_summaries` 读到每条语句的开始时刻、首行等待和交回行数，并用 `segment_metrics` 从这批保留摘要得到吞吐和 P95。Linux 上 `process_memory_peak_bytes` 返回 `ru_maxrss * 1024`。查询内存上限为 0 的语句仍返回原来的行。这些读出不在 SQL 结果列里，也不在诊断渲染文本里。

**Investigation Facts**

- Starting point: Iteration 000 的 Act Response 已把五段旧耗时接到当前语句阶段树。该 Cycle 的 Plan Review 为 accepted。产品代码与 Act 提交 `5531286` 相同。本轮不重跑 000 的定向测试，不跑全量。
- Current-State Evidence:
  - `Summary`（`src/diagnostics.rs`）现有字段是 `sql`、`severity`、`total`、`stages`、`samples`、`events`、`dropped`。没有 `started_at`、`first_row`、`rows_delivered`。全仓库只有 `record_statement` 一处构造 `Summary`。测试用字段访问，不用字面量。
  - `record_statement` 是唯一把摘要推进会话的路径。`total` 取根阶段包含耗时。超过 `summary_limit` 时丢掉最旧的。`Diagnostics::summaries` 与 `Database::diagnostic_summaries` 返回副本，最新在末尾。
  - `StatementScope` 的 `ScopeInner.start` 在 `Diagnostics::statement` 和 `enter_statement` 里设为 `Instant::now()`。`finish` 用 `inner.start.elapsed()` 作为根包含耗时。成功走 `end_ok`（`ScopeEnd::Success`），失败走 `end_err`（`Failed`），未结束析构走 `Dropped`。三条路都进 `StatementScope::record`。
  - `enter_statement` 只在任务已有 `STATEMENT_STAGES` 绑定时返回 `Some`。`execute_with_params` 在 `diagnostics_collecting()` 为真时用 `with_statement_stages` 包住 `execute_inner`。采集全关不建立绑定。
  - `execute_inner` 先 `enter_statement`，再 `execute_staged`。`Ok` 调 `end_ok`，`Err` 调 `end_err`。解析失败因此仍形成摘要。
  - `execute_executor` 是唯一把 `ExecResult::Row` 推进调用方 `SqlOutcome::Rows` 的函数。它先把行放进本地 `rows`，结束时若见过 `AffectedRows` 就返回受影响行数，否则返回 `Rows`。`Err` 直接返回，不构成 `Rows`。
  - 内部物化不经过 `execute_executor`：`materialize_insert_select_source`、`execute_ctas_routed` 的源循环、`create_executor_from_plan` 的 `PhysicalPlan::DerivedScan` 臂，各自对执行器 `next` 并把 `ExecResult::Row` 放进自己的 `Vec`。
  - `record_pre_measured` 直接调用 `record_statement`，不经过 `execute_executor`，也不另开一条摘要。
  - `render` 的格式串是 `diagnostics sql={} severity={:?} total={:?}`，再按阶段循环打印 `stage` 行。`Detail::Off` 返回空字符串。它不读取首行、行数、吞吐、P95 或内存峰值。
  - `QueryBudget::charge` 在 `limit == 0` 或 `bytes == 0` 时立刻 `Ok(())`，不加 `used`。`Database::open` 把 `query_memory_limit` 初始化为 0。`set_query_memory_limit` 写入该原子值。`execute_stage_routed` 与 `execute_stage_in_tx_routed` 用 `QueryBudget::new(database.query_memory_limit())`。
  - 没有 `segment_metrics`、`SegmentMetrics`、`process_memory_peak_bytes`。`Cargo.toml` 已依赖 `libc = "0.2"`。`src/` 里没有 `getrusage` 或 `ru_maxrss`。
  - `tests/diagnostics_facade_test.rs` 用 `record_pre_measured` 和 `Diagnostics::statement` 断言总耗时、阶段、渲染前缀、摘要上限和严重级别。它不构造 `Summary` 字面量。
  - `tests/diagnostics_statement_test.rs` 的 `abandoned_future_summary_is_cancelled` 放弃 `INSERT INTO t2 SELECT id FROM t` 的 future，断言被放弃摘要为 `Severity::Warn`，再插入键 `1` 得到 `AffectedRows(1)`，随后 `SELECT id FROM t2` 只见到这一行。
- Code and Critical Path: 采集打开时，`execute_with_params` 建立阶段绑定，`enter_statement` 记下 `ScopeInner.start`。查询行只在 `execute_executor` 进入结果 `rows`。语句 `Ok` 时 `end_ok` 经 `record_statement` 形成摘要。调用方用 `diagnostic_summaries` 的切片调用 `segment_metrics`。内存峰值单独读进程，不进这条语句路径。

**Implementation Guidance**

交付记录放在 `StatementScope` 已经持有的阶段绑定上，使 `execute_executor` 能写下、`record_statement` 能读走。`enter_statement` 把同一个 `ScopeInner.start` 放进这条绑定，供第一行计算时长。`Diagnostics::statement` 没有阶段绑定，不放。不要把记录放进 `SqlOutcome`。绑定上的字段名由实施决定。

语句测试先引用尚不存在的字段和读出函数，确认编译或断言失败，再改形成路径。五段 `record_stage` 调用点保持 000 的接线。

**Task Definitions**

### T3: 从保留摘要和进程读出比较指标

- Requirement/Scenario: 「比较指标从保留摘要读出」的全部场景：非空查询、插入与空结果、出错、被放弃、只含保留摘要、P95 最近秩、墙钟为零或行数为零、进程峰值、比较指标不进入 SQL 结果。另含 `record_pre_measured` 的开始时刻回推。
- Depends on: Iteration 000 accepted
- Targets: `src/diagnostics.rs` 的 `Summary`、`record_statement`、`StatementScope::record`、`render`；`src/pipeline.rs` 的 `execute_executor`；新测试 `tests/diagnostics_comparison_test.rs`；`tests/diagnostics_statement_test.rs` 的 `abandoned_future_summary_is_cancelled`。
- Current behavior: 摘要没有开始时刻、首行等待和交回行数。没有 `segment_metrics` 或 `process_memory_peak_bytes`。`execute_executor` 把 `ExecResult::Row` 收进 `SqlOutcome::Rows`，不记时长。`render` 只打印现有格式串。上限为 0 时 `QueryBudget::charge` 直接返回。被放弃插入的摘要是 `Warn`，该键可以再次插入。
- Required behavior:
  - `Summary` 增加公开字段 `started_at: Instant`、`first_row: Option<Duration>`、`rows_delivered: u64`。现有字段的含义不改。`total` 仍是根包含耗时。
  - 经 `StatementScope` 形成的摘要，`started_at` 就是该作用域的 `ScopeInner.start`。成功、失败、放弃都写这个开始时刻。
  - 只有 `ScopeEnd::Success` 才抄写交付记录。失败和放弃的 `first_row` 为 `None`，`rows_delivered` 为 0。
  - `execute_executor` 把第一行 `ExecResult::Row` 放进将要返回的 `rows` 时，若阶段绑定存在，记下从 `started_at` 到此刻的时长。最终结果是非空 `SqlOutcome::Rows` 时保留该时长，`rows_delivered` 等于 `rows.len()`。最终结果是空 `Rows`、`AffectedRows` 或 `Err` 时清掉时长，交回行数为 0。没有阶段绑定时不记。
  - 成功且至少交回一行时，`first_row` 为 `Some`，且不大于 `total`。插入、空结果、出错、被放弃的 `first_row` 为 `None`，`rows_delivered` 为 0。
  - `materialize_insert_select_source`、CTAS 源循环、`DerivedScan` 物化不调用这个记录。它们内部的行不产生首行等待，也不计入交回行数。
  - `record_pre_measured` 仍只形成一条摘要。`started_at` 等于记录时刻减去根包含耗时；`Instant::checked_sub` 得到 `None` 时，`started_at` 用记录时刻。`first_row` 为 `None`，`rows_delivered` 为 0。
  - 公开读出，不做会话内的第二套存储：

    ```text
    pub struct SegmentMetrics {
        pub throughput_rows_per_sec: Option<f64>,
        pub slow_p95: Option<Duration>,
    }

    pub fn segment_metrics(summaries: &[Summary]) -> SegmentMetrics
    pub fn process_memory_peak_bytes() -> Option<u64>
    ```

  - 没有摘要时，吞吐和 P95 都是 `None`。`t0` 是这些摘要里最早的 `started_at`。每条的结束时刻是 `started_at + total`。`t1` 是这些结束时刻里最晚的一个。墙钟是 `t1.saturating_duration_since(t0)`。
  - 墙钟为 0 时不给出吞吐。否则吞吐等于交回行数之和除以 `wall.as_secs_f64()`，单位是行每秒。行数之和为 0 且墙钟大于 0 时，吞吐为 `0.0`。
  - P95：总耗时升序，下标从 1 计。`N = 0` 时不给出。否则秩 `r = (95 × N + 99) / 100`，P95 等于第 `r` 条。不插值，不另设最小样本数。因此一条时等于该总耗时，两条时等于较大者，二十条时等于升序后的第 19 条。
  - Linux 上 `getrusage(RUSAGE_SELF)` 返回 0 且 `ru_maxrss >= 0` 时，`process_memory_peak_bytes` 返回 `ru_maxrss` 的非负值乘以 1024。调用失败或 `ru_maxrss < 0` 时返回 `None`。其他操作系统返回 `None`。
  - `render` 仍使用现在的格式串，不追加开始时刻、首行等待、交回行数、吞吐、P95 或内存峰值。阶段行仍走现有循环。
  - 查询内存上限为 0 时，已知查询仍返回原来的行，不报查询预算超限。`QueryBudget::charge` 的短路保持。
  - 诊断打开与关闭时，同一查询的结果行相同。行里没有上述读出。同一条无法解析的语句，两次的错误文本和 SQLSTATE 相同。
- Preserve: 语句总耗时仍是根包含耗时。错误文本、SQLSTATE、SQL 结果不变。五段子指标的接线不改。页读仍经 `record_page_read`。摘要上限仍丢弃最旧。`Detail::Off` 仍渲染空字符串。被放弃语句的 `Warn`、再次插入成功、以及 `t2` 只含已完成语句的行，保持现有断言。
- Forbidden: 把指标写入 `SqlOutcome` 或结果列；读 `QueryBudget` 的 `used` 或任何分配器统计；解析 `VmHWM` 或 `/proc/self/status`；新建场景对象或在会话里另存一段指标；修改 `QueryBudget::charge`；跑全量；重跑 Iteration 000 已通过的整文件定向测试。
- First test: 在 `tests/diagnostics_comparison_test.rs` 增加 `nonempty_query_records_first_row` 与 `p95_uses_nearest_rank_upper_bound`。给 `abandoned_future_summary_is_cancelled` 追加：被放弃摘要的 `first_row` 为 `None`，`rows_delivered` 为 0。改产品代码前运行下面两条，结果是非零退出码（字段或函数尚不存在时为编译失败，否则为断言失败）。

  ```text
  cargo test --offline --test diagnostics_comparison_test -- --test-threads=1 nonempty_query_records_first_row p95_uses_nearest_rank_upper_bound
  cargo test --offline --test diagnostics_statement_test abandoned_future_summary_is_cancelled -- --test-threads=1
  ```

- GREEN condition: Verification 的三条命令都是退出码 0、failed 为 0。
- Verification:
  - `cargo test --offline --test diagnostics_comparison_test -- --test-threads=1`
  - `cargo test --offline --test diagnostics_statement_test abandoned_future_summary_is_cancelled -- --test-threads=1`
  - `cargo test --offline --test diagnostics_facade_test -- --test-threads=1`
  - 比较指标文件还包含下面这些测试，随第一条命令一起跑。失败表示读出口径或结果列变了。门面命令失败表示总耗时、阶段汇总或现有渲染格式被改动。被放弃命令失败表示放弃语义或新增的空首行断言不成立。
  - `insert_empty_and_error_omit_first_row`：只开 stats。成功插入的受影响行数与诊断关闭时相同，摘要没有首行等待，交回行数为 0。再查一个不存在的键，摘要没有首行等待，交回行数为 0，结果为空。同一条无法解析的语句在诊断关闭和只开 stats 下各执行一次，错误文本与 SQLSTATE 相同；只开 stats 的摘要没有首行等待，交回行数为 0。
  - `nonempty_query_records_first_row`：表里至少一行。诊断关闭与只开 stats 的查询行相同。stats 摘要的 `first_row` 为 `Some` 且不大于 `total`，`rows_delivered` 等于结果行数。入口是 `Database::execute_sql`。
  - `segment_metrics_uses_only_retained_summaries`：`summary_limit` 为 1，只开 stats，连续两条都会交回行且行数不同的成功查询。`diagnostic_summaries` 只剩后一条。该摘要的 `rows_delivered` 等于后一条的结果行数，不等于两条行数之和。`segment_metrics` 的吞吐和 P95 等于只用这一条摘要计算的结果。两条查询各自的结果行都与诊断关闭时相同。
  - `p95_uses_nearest_rank_upper_bound`：不执行 SQL，用构造的摘要。空切片的 P95 为 `None`。一条时等于该总耗时。两条时等于较大的总耗时。二十条互不相同的总耗时，P95 等于升序后的第 19 条。
  - `throughput_handles_zero_wall_and_zero_rows`：不执行 SQL。墙钟为 0 时吞吐为 `None`。交回行数之和为 0 且墙钟大于 0 时，吞吐为 `Some(0.0)`。另用两段重叠的整秒区间证明墙钟是最早开始到最晚结束，而不是总耗时之和：一条从 `T` 起、总耗时 2 秒、交回 3 行，另一条从 `T + 1 秒` 起、总耗时 1 秒、交回 1 行，墙钟 2 秒，吞吐为 `Some(2.0)`。
  - `pre_measured_summary_has_no_delivery`：`record_pre_measured` 一条成功摘要。`first_row` 为 `None`，`rows_delivered` 为 0。`started_at + total` 落在调用前后的两个 `Instant::now()` 之间。
  - `linux_memory_peak_matches_ru_maxrss`：Linux 上函数返回值等于同一次测试里 `getrusage(RUSAGE_SELF)` 读到的非负 `ru_maxrss` 乘以 1024。同一测试打开数据库后把查询内存上限设为 0，执行一条已知查询，结果行与设诊断之前相同，且不是查询预算超限。非 Linux 上函数返回 `None`，这条查询断言仍执行。
  - `comparison_metrics_absent_from_sql_rows`：只开 stats 的成功查询，结果行与诊断关闭时逐值相同。对这条摘要调用 `render(summary, Detail::Summary)`，结果等于用现有格式串从 `sql`、`severity`、`total` 和 `stages` 拼出的文本，其中没有首行等待、交回行数、吞吐、P95 或内存峰值。
- Stop when: 要得到首行等待就必须改错误文本、SQLSTATE 或结果列时，停止并回到计划。峰值只有改读 `VmHWM`、`QueryBudget` 或分配器统计才能得到时，停止并回到计划。同一次测试里函数返回值与 `ru_maxrss * 1024` 不一致时，停止并回到计划，不改乘法，也不反复读取直到相等。

**Invariants**

- 诊断只观察。SQL 结果、错误文本、SQLSTATE 与改前相同。
- 语句总耗时仍是根阶段包含耗时，不是子阶段之和。
- 比较指标只从传入的摘要切片和进程 `ru_maxrss` 读出。
- 上限为 0 时查询预算入账仍立即返回且不计字节。
- 测试调用都带 `--test-threads=1`。本轮不跑全量。

**Non-goals**

第二套计时、缓冲池计数、全量、改 SQL 结果列、解析 `VmHWM`、显式事务路径上的新测量、改渲染格式串、返工五段子指标。

**Acceptance**

- 非空查询的首行等待和交回行数由 `nonempty_query_records_first_row` 覆盖。
- 插入、空结果、出错由 `insert_empty_and_error_omit_first_row` 覆盖。
- 被放弃语句由 `abandoned_future_summary_is_cancelled` 覆盖。
- 保留摘要上的吞吐和 P95 由 `segment_metrics_uses_only_retained_summaries` 与 `p95_uses_nearest_rank_upper_bound` 覆盖。
- 墙钟为 0、行数为 0，以及整秒重叠区间的吞吐由 `throughput_handles_zero_wall_and_zero_rows` 覆盖。
- `record_pre_measured` 的空交付和回推开始时刻由 `pre_measured_summary_has_no_delivery` 覆盖。
- Linux 进程峰值和上限 0 的原行由 `linux_memory_peak_matches_ru_maxrss` 覆盖。
- 结果列与渲染不含这些读出，由 `comparison_metrics_absent_from_sql_rows` 覆盖。门面既有总耗时和渲染前缀由 `diagnostics_facade_test` 覆盖。
- 正确性看 `SqlOutcome`、错误文本和 SQLSTATE。渲染文本只证明格式串没有追加这四项读出。

**Verification**

按 T3 的三条命令执行。每条都带 `--test-threads=1`。通过条件是退出码 0 且 failed 为 0。语句类场景走 `Database::execute_sql`。P95 和吞吐的构造摘要不执行 SQL。这三条不是全量。

**Gate 2 Readiness**

Gate 2：PASS。无 BLOCKED。无 SKIPPED。

批准来源是 goal「很好，按你推荐的来就行，默认全局最优」对 `tasks.md` 已写 T3 范围的确认。检查项均已对照本 Cycle、规格和 design 的锁定公式通过，不是跳过。

**Persisted Evidence**

- Mode: none

定向测试可以按上面的命令重跑。本轮不需要把输出存成文件。

**Risks and Notes**

- 阶段绑定上交付记录的字段名是非实质选择。成功抄写、失败清零、以及非空 `Rows` 才保留的规则已写死。
- 吞吐断言使用整秒墙钟和可精确表示的行速率。不要用需要容差的浮点比较代替 `Some(2.0)` 或 `Some(0.0)`。
- `ru_maxrss` 只增不减。峰值测试只比较一次。不一致就回到计划。
- 000 已让阶段行变多。渲染测试锁定的是格式串没有追加比较指标，不禁止已有阶段行出现子指标名。
- 索引查询的 `first_row` 可以远小于 `execute` 的包含耗时。不要把它写成比例阈值。

## Act Response

- Status: reported

**实际改动**

- `src/diagnostics.rs`：`Summary` 增加 `started_at`、`first_row`、`rows_delivered`。`record_statement` 写入这三项。`record_pre_measured` 用记录时刻减去根包含耗时得到 `started_at`（`checked_sub` 为 `None` 时用记录时刻），`first_row` 为 `None`，`rows_delivered` 为 0。阶段绑定 `StageStack` 增加同名交付字段。`enter_statement` 把同一个 `ScopeInner.start` 写入绑定。`note_first_delivered_row` 与 `finish_delivered_rows` 供执行器写下交付记录。`StatementScope::record` 始终使用作用域开始时刻；只有 `ScopeEnd::Success` 抄写交付记录，失败和放弃的 `first_row` 为 `None`、`rows_delivered` 为 0。新增 `SegmentMetrics`、`segment_metrics`、`process_memory_peak_bytes`。`render` 未改。
- `src/pipeline.rs`：`execute_executor` 在第一行 `ExecResult::Row` 进入将返回的 `rows` 时记下首行等待；非空 `SqlOutcome::Rows` 保留该时长且 `rows_delivered` 等于 `rows.len()`；空 `Rows`、`AffectedRows` 和 `Err` 清掉时长，交回行数为 0。错误文本和 SQLSTATE 未改。
- `tests/diagnostics_comparison_test.rs`：比较指标读出测试。
- `tests/diagnostics_statement_test.rs`：`abandoned_future_summary_is_cancelled` 追加被放弃摘要的 `first_row` 为 `None`、`rows_delivered` 为 0。
- `openspec/changes/2026-11-14-ms49-diagnostics-reporting/tasks.md`：T3 标为完成。

**与计划的偏差**

阶段绑定上的字段名用 `started_at`、`first_row`、`rows_delivered`。计划把该命名留给实施。

`process_memory_peak_bytes` 直接调用平台 `getrusage(RUSAGE_SELF)`，没有把 `libc` 升成库依赖。`Cargo.toml` 里的 `libc` 只在 dev-dependencies，库代码不能引用。返回值仍是非负 `ru_maxrss` 乘以 1024。Linux 测试里同一次比较一致。

**自检**

`rustfmt` 曾把 `record_measured_submetric` 和语句测试里原有断言重新折行。这些折行已改回。产品行为与下面三条命令运行时相同。无遗留 Minor。Gate 3：PASS。Gate 4：PASS。Gate 5 未触发。

全量：SKIPPED。全量属于尚未展开的 Iteration 002，本批只执行 T3。

**验证**

先行测试（改产品代码前）：

- `cargo test --offline --test diagnostics_comparison_test -- --test-threads=1 nonempty_query_records_first_row p95_uses_nearest_rank_upper_bound`：退出码 101。编译失败，`segment_metrics` 与 `process_memory_peak_bytes` 不存在，`Summary` 没有 `started_at`、`first_row`、`rows_delivered`。18 个错误。
- `cargo test --offline --test diagnostics_statement_test abandoned_future_summary_is_cancelled -- --test-threads=1`：退出码 101。编译失败，`Summary` 没有 `first_row` 和 `rows_delivered`。2 个错误。

GREEN，各跑一次：

- `cargo test --offline --test diagnostics_comparison_test -- --test-threads=1`：退出码 0。8 passed；0 failed。
- `cargo test --offline --test diagnostics_statement_test abandoned_future_summary_is_cancelled -- --test-threads=1`：退出码 0。1 passed；0 failed。
- `cargo test --offline --test diagnostics_facade_test -- --test-threads=1`：退出码 0。10 passed；0 failed。

**Persisted Evidence**

None required。

**Experience Candidates**

None

**未解决问题**

无。

## Plan Review

- Review Result: pending
