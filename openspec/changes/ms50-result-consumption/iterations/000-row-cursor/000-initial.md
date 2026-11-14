# Iteration 000 / Cycle 000: 公共行游标

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2, T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `result-consumption` 的四条要求；`execute_sql` 四个整段入口的现有结果和错误；M19 的 `--test-threads=1`
- Excluded scope: DECIMAL 与 BLOB；预编译语句游标；CLI 与协议的边输出；诊断定义；算子内部物化；全量分批

**Objective**

调用方经四个公共入口按批量拿走查询行。一次拉取不预取下一批。写语句在副作用之前被拒绝。整段入口仍一次交回完整结果。

**Investigation Facts**

- Starting point: master `f323d58a452ce34467d2e33fd6dda5cebe798b56`。没有可沿用的前序 Cycle。规划阶段没有跑测试。当前没有 `open_rows`。
- Current-State Evidence:
  - `Database::execute_sql`（`src/database.rs`）委托 `pipeline::execute`。`execute_with_params` 做解析、占位符、计划、参数物化和执行。`execute_in_tx_with_params` 是同一流程，DML 使用调用方的 `tx_id`。
  - 查询臂是 `execute_stage_routed` 和 `execute_stage_in_tx_routed` 的 `_` 分支。自动提交创建执行器时事务参数为 `None`，快照为 `statement_snapshot(None)`。显式事务为 `Some(tx_id)` 和 `statement_snapshot(Some(tx_id))`。
  - `execute_executor` 循环 `executor.next()`，把 `ExecResult::Row` 全部推入 `Vec`。空结果也走 `SqlOutcome::Rows`。随后查询臂调用 `attach_column_metadata`。值推导函数是 `derive_types_from_rows`，只用在整段路径。
  - 执行错误包装为 `Execution error: {e}`。`QueryMemoryExceeded` 的 SQLSTATE 是 `53200`，其余为 `None`。函数是 `storage_error_sqlstate`。
  - `Executor::next_batch` 在 `src/executor/executor_trait.rs`。`max == 0` 返回空且不调用 `next`。这是内部接口。
  - 无 WHERE 的单表查询在 `parser/planner/query.rs` 换成 `DataScan`。`ProjectionExecutor::next` 先取输入的一行，再对这一行求值。
  - 只读拒绝是 `readonly_rejection`，写计划在只读会话返回 SQLSTATE `25006`。
  - 首行计点是 `diagnostics::note_first_delivered_row` 和 `finish_delivered_rows`，只在 `execute_executor` 里调用。
  - 查询预算在语句入口 `QueryBudget::new(database.query_memory_limit())`。交回调用方的结果行不计费。
- Code and Critical Path: 新增 `src/result.rs` 的 `RowCursor`；`src/database.rs` 四个方法；`src/pipeline.rs` 的打开和拉取；`src/lib.rs` 导出。数据从根执行器的一批 `ExecResult::Row` 变成 `Vec<Vec<Value>>` 交给调用方。游标持有执行器和列元数据。丢弃游标丢弃执行器，不触碰调用方的 `Transaction`。

**Implementation Guidance**

四个入口共用一个打开函数，差别只在参数切片和可选 `tx_id`。计划成功后先 `readonly_rejection`，再判断是不是查询臂。拉取对根执行器调用一次 `next_batch(max)`。不要把 `execute_executor` 改成抽干游标。

**Task Definitions**

### T1: 查询可以分批拿走

- Requirement/Scenario: 公共行游标分批交回 / 分批拼接等于整段结果、空结果仍有列元数据、批量大小为零不推进；非行语句在副作用之前拒绝 / 可写会话中的插入不落行、只读会话仍是只读拒绝
- Depends on: None
- Targets: `src/result.rs::RowCursor`；`src/database.rs::open_rows`；`src/database.rs::open_rows_with_params`；`src/pipeline.rs` 查询臂打开路径；`src/lib.rs` 的导出
- Current behavior: 查询只有 `execute_sql` / `execute_with_params`，返回前收齐全部行。没有公共游标。
- Required behavior: `open_rows` 对查询返回游标。`columns()` 在首拉前返回计划列名和声明类型，未知类型保持 `None`，不调用 `derive_types_from_rows`。`next_batch(2)` 连续拉到空批，拼接后与 `execute_sql` 的行序和行值相同。空结果仍有列元数据，第一次拉取为空。`next_batch(0)` 为空且随后的 `next_batch(1)` 仍是第一行。可写会话对 INSERT 返回消息 `streaming consumption requires a row-producing statement`、SQLSTATE `None`，行数不变。只读会话对 CREATE TABLE 返回 SQLSTATE `25006`，不建表。解析和参数错误文本与对应整段入口相同。
- Preserve: `execute_sql` 仍返回完整 `SqlOutcome`。`Executor::next_batch` 仍是内部方法。
- Forbidden: 修改 CLI、协议、诊断计点、页格式、值类型枚举。把 `SqlOutcome::Rows` 改成流。
- First test: `tests/result_consumption_test.rs::batches_match_execute_sql`。改前不能编译或失败，因为没有 `open_rows`。命令 `cargo test --test result_consumption_test batches_match_execute_sql -- --test-threads=1`。
- GREEN condition: 该测试通过，并且同文件的空结果、零批量和插入拒绝测试通过。
- Verification: 上一条命令不带过滤再跑整个新文件，退出码 0 且 failed 为 0。失败表示拉取合同或拒绝文本不对。
- Stop when: 查询臂产出 `AffectedRows` 或 `RowId`，或者不改 `execute_sql` 的结果形状就无法分批。

### T2: 未索取的行不提前失败

- Requirement/Scenario: 结果交接处的背压 / 未索取的行不会提前失败、提前丢弃不改变表；整段入口与诊断口径保持 / 整段失败不交回部分行、直接全表扫描仍不受结果大小拒绝
- Depends on: T1
- Targets: `src/pipeline.rs` 的拉取循环；`src/diagnostics.rs::note_first_delivered_row`；`src/diagnostics.rs::finish_delivered_rows`；`execute_executor`
- Current behavior: `execute_executor` 在返回前跑完执行器。中途错误时调用方看不到任何行。首行计点发生在第一行进入结果 `Vec` 时。
- Required behavior: 新建表插入三行短字符串，值为 `1`、`2`、`nope`。`SELECT CAST(s AS INTEGER) FROM t` 没有 WHERE、ORDER BY、聚合或连接。游标每次 `next_batch(1)`：前两批是整数 1 和 2，第三批是 `Err`，消息和 SQLSTATE 与这条语句的 `execute_sql` 错误相同，再拉一次仍是这个错误。同一条语句的 `execute_sql` 返回 `Err` 且没有行。只拉一批后丢弃游标，再 `execute_sql` 全表，行数与内容不变。上限 `set_query_memory_limit(16)` 时，无 WHERE 的整数 `SELECT *` 经两个入口都成功返回全部行。游标的实现里不调用 `note_first_delivered_row` 和 `finish_delivered_rows`。
- Preserve: `execute_executor` 的收集循环和它的诊断调用保持。查询预算的计费函数不改。
- Forbidden: 为了让第三行晚一点失败而关闭投影或改 DataScan 顺序。新增诊断字段。把结果行计入查询预算。
- First test: `tests/result_consumption_test.rs::later_cast_failure_is_not_prefetched`。T1 完成后、拉取仍一次抽干时，这个测试失败。命令 `cargo test --test result_consumption_test later_cast_failure_is_not_prefetched -- --test-threads=1`。
- GREEN condition: 该测试、`execute_sql_cast_returns_no_rows`、提前丢弃测试和上限 16 的全表扫描测试通过。
- Verification: `cargo test --test result_consumption_test -- --test-threads=1` 退出码 0、failed 为 0。另跑 `cargo test --test diagnostics_comparison_test nonempty_query_records_first_row -- --test-threads=1`，退出码 0。后者失败表示整段诊断被带上了游标。
- Stop when: 这条 CAST 查询被规划成排序或聚合，导致第一批就必须算完全部行。停止并回到计划，不要改规划来迁就测试。

### T3: 带参和事务使用同一游标

- Requirement/Scenario: 公共行游标分批交回 / 带参与事务内可见
- Depends on: T1
- Targets: `src/database.rs::open_rows_with_params`；`src/database.rs::open_rows_in_tx`；`src/database.rs::open_rows_in_tx_with_params`；`pipeline::execute_in_tx_with_params` 的查询臂参数
- Current behavior: 带参和事务内查询都由整段入口收齐行。事务内查询的执行器带 `Some(tx_id)` 和 `statement_snapshot(Some(tx_id))`。
- Required behavior: `open_rows_with_params` 的拼接结果与 `execute_with_params` 相同。显式事务里插入一行但不提交，`open_rows_in_tx` 能读到它。丢弃游标后这行仍未提交；调用方 `commit` 后提交，`rollback` 的另一条用例里该行不可见。事务内的 INSERT 走游标时同样是 T1 的拒绝文本，并且不使用调用方事务把行写进去。
- Preserve: `execute_in_tx` 的提交权仍在调用方。失败语句不结束事务的既有合同保持。
- Forbidden: 游标析构时调用 `commit` 或 `rollback`。
- First test: `tests/result_consumption_test.rs::transaction_cursor_sees_uncommitted_row`。命令 `cargo test --test result_consumption_test transaction_cursor_sees_uncommitted_row -- --test-threads=1`。改前失败。
- GREEN condition: 带参、事务可见、丢弃后回滚仍可回滚，这几个测试通过。
- Verification: `cargo test --test result_consumption_test -- --test-threads=1` 退出码 0、failed 为 0。
- Stop when: 事务内查询不把 `tx_id` 传进 `create_executor_from_plan` 就看不见未提交行，同时又会让游标拥有提交权。

**Invariants**

- 任何 `cargo test` 带 `--test-threads=1`。
- 不跑无目标过滤的 `cargo test`。
- `execute_sql` 四个整段入口的成功结果、整句失败和诊断计点保持。
- 不新增跨层依赖环。游标留在结果与 pipeline，不把 `Executor` 导出到库外。

**Non-goals**

DECIMAL 与 BLOB。预编译游标。CLI 与协议边输出。修改诊断定义。取消排序、聚合、连接的内部驻留。全量分批。

**Acceptance**

- T1 的新文件测试证明分批拼接、列元数据、零批量和两类拒绝。
- T2 证明第三行 CAST 失败不出现在前两批，`execute_sql` 仍无部分行，丢弃游标不改表，上限 16 的整数全扫两条路径都成功，既有首行等待测试仍通过。
- T3 证明带参结果一致，事务内看得见未提交行，丢弃游标不提交也不妨碍回滚。

**Verification**

行为经 `Database` 的真实方法观察，不另做判定程序。

```text
cargo test --test result_consumption_test -- --test-threads=1
cargo test --test diagnostics_comparison_test nonempty_query_records_first_row -- --test-threads=1
```

两条都要退出码 0 且 failed 为 0。这不是全量。

**Gate 2 Readiness**

PASS。无 BLOCKED，无 SKIPPED。审查项均已通过：无 TBD；无未批准裁剪；tasks、spec、design 与本 Cycle 一致；证据来自上述符号的代码阅读；三个任务的执行依据足够让 Act 写第一个测试；调用者和测试入口已经点名；没有把类型系统划进来；Iteration 只有这一轮且通过平衡审计；全量预算不在本变更；Persisted Evidence 为 none。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

全量分批不在本 Cycle。本里程碑的那一次写在 `ms50-decimal-and-blob` 的 Iteration 001。本变更与它没有代码依赖。

若无 WHERE 的 CAST 查询不被规划成按行投影的 DataScan，按 T2 的停止条件回到计划。

## Act Response

- Status: reported

四个入口共用 `pipeline::open_rows`。差别只有参数切片和可选 `tx_id`。计划成功后先 `readonly_rejection`，非查询臂在创建执行器之前拒绝。拉取对根执行器调用一次 `next_batch(max)`。列元数据只保留声明类型。`execute_executor` 仍一次收齐整段结果，并保留原来的诊断计点。

被替代的做法有两件。打开时抽干执行器再切批，第三行 `CAST` 会在第一批之前失败。把 `execute_executor` 改成游标，会改掉整段入口的收集和诊断。

**实际改动**

- `src/result.rs`：`RowCursor`。`columns()` 返回计划列名和声明类型。执行器字段不进入公共签名。
- `src/pipeline.rs`：`open_rows`、`prepare_cursor_plan`、`is_row_producing`、`declared_column_metadata`，以及 `RowCursor::next_batch`。
- `src/database.rs`：`open_rows`、`open_rows_with_params`、`open_rows_in_tx`、`open_rows_in_tx_with_params`。
- `src/lib.rs`：导出 `RowCursor`。
- `tests/result_consumption_test.rs`：本轮 16 个测试。
- `tasks.md`：T1、T2、T3 标为完成。

**与计划的偏差**

T2 的先行测试没有出现 RED。T1 的拉取已经是一次 `next_batch(max)`，`SELECT CAST(s AS INTEGER) FROM t` 的前两批是整数 1 和 2，第三批才返回与 `execute_sql` 相同的错误。这条语句没有被规划成必须先算完全部行的排序或聚合，T2 停止条件没有命中。

可重复读且快照为 `None` 时，数据页扫描能看见其他语句尚未提交的行。事务测试用事务仍在活跃集里证明丢弃游标没有提交，再用调用方的 `commit` 和另一条 `rollback` 证明提交权仍在调用方。

打开路径不读 plan cache，每次重新解析和计划。这样错误文本与整段入口相同，也不改 `execute_sql` 的缓存命中路径。

**自检**

Gate 3：T1、T2、T3 的要求、场景和禁止项都已核对。`execute_executor` 的收集循环和 `note_first_delivered_row`、`finish_delivered_rows` 仍只在整段路径上。游标不调用这两处，也不调用 `derive_types_from_rows`。

遗留 Minor：一批里若出现 `AffectedRows` 或 `RowId`，游标记下 `streaming consumption received a non-row result` 并不再继续。本轮查询没有打到这条路径。

**验证**

未跑全量。命令都带 `--test-threads=1`，多个目标逐个执行。

- `cargo test --test result_consumption_test batches_match_execute_sql -- --test-threads=1`：改前编译失败，没有 `RowCursor` 和 `open_rows`。实现后 1 passed，0 failed。
- `cargo test --test result_consumption_test -- --test-threads=1`：当时文件内 7 passed，0 failed。
- `cargo test --test result_consumption_test later_cast_failure_is_not_prefetched -- --test-threads=1`：1 passed，0 failed。
- `cargo test --test result_consumption_test -- --test-threads=1`：11 passed，0 failed。
- `cargo test --test diagnostics_comparison_test nonempty_query_records_first_row -- --test-threads=1`：1 passed，0 failed。此后没有再改 `execute_executor` 的诊断调用。
- `cargo test --test result_consumption_test transaction_cursor_sees_uncommitted_row -- --test-threads=1`：改前编译失败，没有 `open_rows_in_tx`。补上入口后 1 failed。失败点是事务外 `SELECT` 仍能看见未提交行，产品代码未改。
- `cargo test --test result_consumption_test -- --test-threads=1`：断言改为事务仍在活跃集后，16 passed，0 failed。

**Persisted Evidence**

None required。

**Experience Candidates**

None。

**未解决问题**

无。

## Plan Review

- Review Result: pending
