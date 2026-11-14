## Why

查询结果在回到调用方之前被整段放进 `SqlOutcome::Rows`。执行器内部的 `next_batch` 仍由这条整段收集调用，调用方无法分批拿走结果，也无法用停止拉取来挡住后续生产。MS50 要的是这个公共消费边界。DECIMAL 与 BLOB 用另一组查询验收，不在本变更里。

## What Changes

- 新增公共拉取接口。`Database` 上四个入口打开行游标：`open_rows`、`open_rows_with_params`、`open_rows_in_tx`、`open_rows_in_tx_with_params`。游标一次最多交回调用方请求的行数。
- `max == 0` 交回空批且不推进。调用方不再拉取或丢弃游标后，根执行器不再继续产出。
- 只接受今天会走查询臂、最终交回行的语句。写语句和 DDL 在产生副作用之前拒绝。
- `execute_sql` 仍一次交回完整 `SqlOutcome`。CLI、协议和诊断口径不改。

## 已锁定口径

批准来源是用户原话：「那就开始规划和实施这个ms，要求遵守各种规范，尤其是多批次串行测试规范，本次最多进行一次全量」。本次目标派发把这句话当作 Gate 1 与 Gate 2 的批准，不是跳过。批准不降低场景、规格和追踪检查。

下面几项按公共规则的全局最优解选定，被替代的方案写在 design.md。

1. 公共接口是新的行游标，不是把 `Executor::next_batch` 交给调用方，也不是把 `SqlOutcome::Rows` 改成流。
2. 四个入口覆盖现有的直接执行与显式事务、带参与不带参。预编译语句仍走原来的整段执行。
3. 列元数据在第一次拉取之前就能读到，来自计划上已经解析出的列名和声明类型。游标不对整段结果做值推导。
4. 背压边界在结果交接：一次 `next_batch(max)` 向根执行器至多要 `max` 行，不预取下一批。排序、哈希聚合、哈希连接的构建、DISTINCT 仍按各自语义驻留。本变更不改这些算子。
5. 游标中途失败时，已经交回的批留在调用方。失败批的错误文本和 SQLSTATE 与同一条语句走 `execute_sql` 时相同。`execute_sql` 仍是整句失败、不交回部分行。
6. 游标不调用 `note_first_delivered_row` 和 `finish_delivered_rows`。诊断定义仍只描述 `execute_sql` 那条整段路径。

## 场景草图

失败边界：游标把写语句执行了；`execute_sql` 开始交回部分行或改变错误文本；为了分批而改了诊断定义；一次拉取预取了下一批。

| 场景 | 前置 | 动作 | 可观察结果 |
|---|---|---|---|
| 分批与整段一致 | 表里有多行整数 | `open_rows` 后按 2 行一批拉完 | 拼接顺序和值与 `execute_sql` 相同；列名和声明类型在首拉之前可读 |
| 空结果 | 表在，查询无行 | 打开后拉取 | 列元数据仍在；第一次拉取得到空批 |
| 带参 | 查询含 `?` | `open_rows_with_params` | 行与 `execute_with_params` 相同 |
| 显式事务 | 事务内插入尚未提交 | `open_rows_in_tx` 读这行，然后丢弃游标 | 读到未提交行；丢弃不提交也不回滚；调用方再提交后该行可见 |
| 零批量 | 表里至少一行 | `next_batch(0)` 后再 `next_batch(1)` | 第一次是空批；第二次仍是第一行 |
| 写语句不执行 | 表里有已知行数 | 对 INSERT 或 CREATE TABLE 打开游标 | 错误文本为 `streaming consumption requires a row-producing statement`，SQLSTATE 为空；行数或表集合不变 |
| 只读写语句 | 只读打开 | 对 CREATE TABLE 打开游标 | 仍是既有只读拒绝（SQLSTATE `25006`），不建表 |
| 中途求值失败 | 三行字符串，前两行可转成整数，第三行不能；无 WHERE、无排序、无聚合 | 每次拉 1 行 | 前两批是整数值；第三批失败，文本与同一条语句的 `execute_sql` 错误相同；再拉仍是这个错误 |
| 整段入口不变 | 同上 | `execute_sql` | 整句失败，没有部分行 |
| 提前停止 | 表里多行 | 只拉一批后丢弃游标，再 `execute_sql` | 表内容不变，整段查询仍返回全部行 |
| 诊断不改口径 | 性能统计打开，查询至少一行 | `execute_sql` | 首行等待仍按整段交回记录；游标路径不写这两个计数字段 |

## Capabilities

### New Capabilities

- `result-consumption`：公共行游标的分批拉取、背压和与整段入口的分工。

### Modified Capabilities

无。`typed-library-results`、`executor-batch-interface`、`query-memory-budget`、`unified-diagnostics` 的现有要求保持，本变更不改它们的条文。

## Impact

- 新类型在 `src/result.rs`，四个方法在 `src/database.rs`，打开和拉取在 `src/pipeline.rs` 的查询臂。`src/lib.rs` 重新导出游标。
- 不改 `execute_executor` 的整段收集，不改 CLI 渲染，不改 `network::protocol::to_response`。
- 约束：M19。本里程碑全部变更合计只做一次全量分批，写在 DECIMAL/BLOB 变更的收尾轮，本变更不做。M21 的公共结果面增加游标，不新增跨层环。
- 关联：I065。与 I069 分开，因为验收查询不同。无格式或接口依赖。

## 回滚

删掉四个入口和游标类型，调用方回到 `execute_sql`。没有库文件格式变化，不需要迁移。

## Gate 1

Gate 1：PASS。需求、场景和范围由上述用户原话批准，不是跳过。本变更只覆盖 MS50 里结果消费这一支。
