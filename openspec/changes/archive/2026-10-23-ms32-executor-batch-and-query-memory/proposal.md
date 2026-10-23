## Why

执行器以 `async next()` 逐行返回，`async-trait` 把每次调用装箱；哈希 JOIN 在输出前缓存全部左表行并克隆匹配桶，`ORDER BY` 加 `LIMIT` 仍先收集全部输入再排序。这些路径没有查询内存上限。来源是里程碑 MS32（`.agents/docs/tasks.md`）及其已提升条目 I081、I082。现在做，是因为 MS30、MS31 已完成，MS32 是路线上下一项。

## What Changes

- 为 `Executor` 增加默认的 `next_batch`。先用现有 DataScan 基准测量；只有 `n=10000` 的批量抽取平均时间比逐行抽取至少低 10% 时，才把 DataScan 的批量实现和把它接到生产抽取循环的转发层留下来。达不到就只保留默认方法。
- 哈希 JOIN 改为左表逐行探测，不再克隆整桶匹配行。输出单元格仍按值复制。公共 `execute_sql` 在中途错误时仍不返回部分结果。
- `LIMIT` 直接包住 `Sort`，或中间只隔一个投影再包住 `Sort` 时，排序只保留 `offset + limit` 行。`DISTINCT`、聚合、集合运算仍全量排序。键值相等时的相对次序不保证，与现在的 `sort_unstable` 一样。
- 查询内存上限默认 0（不限制）。库设置器和 `RTSQL_QUERY_MEMORY_BYTES` 可设上限；超限整句失败，SQLSTATE `53200`，不返回部分行，写语句在产生副作用前失败。

用户在 2026-10-23 确认的假设：

- **A1** 默认上限 0。测试用显式小上限验证拒绝。
- **A2** 计入扫描缓冲、两类 JOIN 的构建侧、Sort、Distinct、聚合、集合运算、子查询物化，以及 `INSERT SELECT` / `CTAS` 在返回前持有的源行。不计入最终 `SqlOutcome::Rows`，不计单行在途。
- **A3** 总是加上默认 `next_batch`。DataScan 及把它接入生产路径所需的转发和抽取循环，仅在基准达到 10% 时保留。没有现成的分配计数器，不另建分配测量；时间门是唯一去留条件。
- **A4** 一个变更、四轮。用户先同意先做批量接口，再做 JOIN、Top-N 和上限；平衡审计把后三项拆开。2026-10-23 用户以「批准」接受这个划分。内容范围没有增减。

不做：

- 溢写、外部排序、分区 JOIN。
- Value 表示层（I020）、分配器（I066）、结果流式消费（I065）、查询取消（I090）。
- Arrow 或整套向量化；不删除 `dyn Executor` 上的 `async-trait`。
- 嵌套循环 JOIN 为谓词拼接左右行的复制。
- CLI 新增 flag。只增加环境变量，通道对齐 `RTSQL_WAL_CHECKPOINT_BYTES`。
- 为未超限的 `EXPLAIN` 文本增加新字段。

### 场景草图

1. 批量默认方法。前置：一个会产出 5 行的执行器。动作：`next_batch(2)` 调用三次，再 `next_batch(0)`。结果：依次 2、2、1 行，然后空批且不消费。失败边界：`max = 0` 不得推进内部游标。
2. 批量去留。前置：1 万行表，DataScan 经默认 `next_batch(256)` 已有 criterion 基线。动作：改为 DataScan 自己攒批后再测同一基准。结果：平均时间至少低 10% 则保留生产接线；否则恢复逐行实现，默认方法仍在。失败边界：不靠分配次数推断通过。
3. 哈希 JOIN。前置：右表多行共享连接键，左表多行。动作：`SELECT` 等值 JOIN。结果：匹配行与现有测试一致；左表在首行输出前不必全部进入执行器缓冲。失败边界：左表中途错误时 `execute_sql` 仍返回错误且无部分行。
4. Top-N。前置：唯一排序键的表。动作：`ORDER BY ... LIMIT n OFFSET m`。结果：与先全排序再截取的前缀一致。`DISTINCT`、聚合后再 `LIMIT` 仍走全排序。失败边界：相等键的次序变化若使既有测试失败，停止并回到计划，不另加决胜规则。
5. 上限关闭。前置：未设置上限。动作：执行现有大查询。结果：与现在一样成功。
6. 上限拒绝。前置：上限小于该语句驻留字节。动作：`execute_sql`、显式事务、预编译执行。结果：`Err`，消息含 `query memory limit exceeded` 和上限，`sqlstate` 为 `53200`，无结果行。`INSERT SELECT` 源表过大时目标表零写入。
7. 非法环境变量。前置：`RTSQL_QUERY_MEMORY_BYTES` 不是非负整数。动作：启动会打开库的 CLI。结果：exit 2，stderr 点名该变量，不创建库文件。

## Capabilities

### New Capabilities

- `executor-batch-interface`：`next_batch` 的默认语义，以及测量通过后 DataScan 批量抽取与生产路径接线的可观察等价。
- `query-memory-budget`：查询驻留字节上限、53200 拒绝、哈希 JOIN 左表不整表驻留、Top-N 的有界排序。

### Modified Capabilities

- 无。未超限查询的结果、退出码和 `EXPLAIN` 文本保持现有规格。相等排序键的次序本来就不由 `sort_unstable` 保证。

## Impact

- 代码：`src/executor/executor_trait.rs`、`data_scan.rs`、`filter.rs`、`projection.rs`、`limit.rs`、`having.rs`、`join.rs`、`sort.rs`、`scan.rs`、`distinct.rs`、`aggregate.rs`、`set_operation.rs`、`nested_loop_join.rs`、`semi_join.rs`、`anti_join.rs`、`subquery_eval.rs`、`scalar_subquery_filter.rs`、`plan.rs`；`src/parser/planner/query.rs`；`src/pipeline.rs`；`src/database.rs`；`src/cli/mod.rs`；`src/storage/error.rs`；`src/result.rs`。
- 测试与基准：`benches/data_scan_bench.rs`；新增查询内存与 Top-N、JOIN 行为测试。全量回归仍按项目模型 M19 分批，每个 Iteration 末一次。
- API：`Executor::next_batch`；`Database` 增加上限设置器。默认 0，现有调用不改变结果。无文件格式变更。
- 回滚：批量实现若未达到 10%，在同一 Iteration 内撤回，只留默认方法。上限默认关闭，不设置则行为与变更前一致。无数据迁移。
