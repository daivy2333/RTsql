## Why

统一诊断门面已经能记录解析、规划、执行和提交等待。各模块仍有五段耗时只进 `RTSQL_PROFILING` 的旧表，比较性能时也还没有首行等待、吞吐、较慢请求和内存峰值。MS49 补上这两块。诊断继续只观察。

## What Changes

- 五段旧子指标用现有的一次测量，经 `record_stage` 挂进当前语句的阶段树：`table_metadata_lookup`、`executor_creation`、`executor_execution`、`index_manager_search`、`index_manager_search_all`。
- 已有文件页读仍只走 `record_page_read`。不新增缓冲池计数，也不新增行数资源计数器。
- 比较指标从会话里已经保留的语句摘要读出：首行等待、吞吐、语句总耗时的 P95，以及进程内存峰值。
- 这些指标不进入 SQL 结果列，不改错误文本，不改 SQLSTATE。

不替换诊断门面，不另建计时系统，不把诊断输出当作 SQL 是否正确的判据。

## 已锁定口径

批准原文：「很好，按你推荐的来就行，默认全局最优」。下面六项不再改口。

1. 不上报入口 `diagnostics::report`。五段旧计时用 `record_stage` 挂到当前语句阶段树。页读继续走 `record_page_read`。不用 `record_pre_measured` 另起一条语句摘要。
2. 解析、规划、执行、提交等待保持原样。只补这五个旧子指标，加上代码里已经有的页读。不新造缓冲池计数或行数计数器。既有 stderr 告警保持原文。`RTSQL_PROFILING` 旧三段表的既有行为保持；这五个计时同时进门面。
3. 首行等待从语句开始到第一行交出。插入、空结果、出错没有这一项。语句总耗时照旧保留。
4. 吞吐是这一段里交出的行数除以这段墙钟，单位是行每秒。这一段就是会话摘要里保留的语句，不新建场景对象。墙钟从这些语句里第一条的开始到最后一条的结束。
5. 较慢请求是这一段里语句总耗时的 P95。样本不足时按最近秩排序取上界。公式见规格，不再征求另一次选择。
6. 内存峰值是进程已有的峰值。Linux 上用 `ru_maxrss`（KiB）乘以 1024 得到字节，不读 `VmHWM` 文件，不读查询预算高水位，不绑定分配器。查询内存上限为 0 时，预算入账仍不计字节。

## 默认假设

这些选择按「复用现有语句摘要、不改 SQL、错误和 SQLSTATE、不加第二套计时」定下，随本提案一起批准。

- 五个调用点各取一个 `Instant`。`RTSQL_PROFILING` 打开时，这个时长仍写入旧表；语句阶段绑定存在时，同一个时长交给 `record_stage`。两个都关时不取测量。
- 不为这五段调用 `open_stage`。提交等待和文件页读仍归当时最内层已经开着的阶段。五段是已完成节点，挂在当时开着的阶段下面。
- `table_metadata_lookup`、`executor_creation`、`executor_execution` 的类别是 `Pipeline`。两个索引查找的类别是 `Index`。
- 索引查找的时间落在 `executor_execution` 里面，但 `record_stage` 把它们记成 `execute` 的直接子阶段，区间允许重叠。独占耗时仍是包含耗时减去直接子阶段包含耗时之和，差为负时饱和为 0。因此「执行独占 + 提交包含 = 执行包含」不再当作一般等式。一次不走索引扫描执行器的插入，执行的直接子阶段是 `executor_creation`、`executor_execution` 和 `commit`，三者不重叠，这三者加独占等于执行的包含耗时。
- 首行等待只在成功交回至少一行 `SqlOutcome::Rows` 时保留，起点是该语句诊断作用域的开始时刻，终点是 `execute_executor` 放进结果的第一行。插入、空结果、出错、以及语句 future 被放弃，都没有这一项，交回行数为 0。`materialize_insert_select_source`、CTAS 源物化和派生表物化里的行不算交出的行。
- 交回行数记在摘要上，只为从摘要算出吞吐。它不是存储层或缓冲池的资源计数器。
- 「第一条」是保留摘要里开始最早的那条，「最后一条」是结束最晚的那条。结束时刻 = 开始时刻 + 语句总耗时。墙钟 = 最晚结束 − 最早开始，用饱和减法。没有保留摘要，或墙钟为 0 时，不给出吞吐。行数为 0 且墙钟大于 0 时，吞吐为 0。
- P95：总耗时升序为 `t_1 ≤ … ≤ t_N`（下标从 1）。`N = 0` 时没有 P95。否则秩 `r = ceil(95 × N / 100)`，P95 = `t_r`。不插值，不另设最小样本数。
- 比较指标的读出在 `diagnostics` 模块：分段指标由摘要切片计算，内存峰值单独读进程。不新增 SQL 列，渲染文本不追加这四项。`record_pre_measured` 形成的摘要没有首行等待，交回行数为 0，开始时刻按记录时刻回推总耗时。

## 场景草图

失败边界都是：SQL 结果、错误文本或 SQLSTATE 与诊断关闭时不同；或为了把五段挂进门面而改了提交等待的归属、旧表输出时机、告警原文。

| 场景 | 前置 | 动作 | 可观察结果 |
|---|---|---|---|
| 插入带上三个流水线子指标 | 性能统计打开，未设旧开关 | 插入一行并再查出来 | 摘要有这三个子指标和 commit；commit 的包含耗时不大于 execute；三者加独占等于 execute 的包含耗时；该行可见 |
| 主键等值 | 表里已有目标行，性能统计打开 | `SELECT * FROM t WHERE id = 1` | 摘要有 `index_manager_search`，没有 `index_manager_search_all`；结果行与诊断关闭时相同 |
| 二级索引等值 | 已建 `CREATE INDEX idx_b ON t(b)` | `SELECT * FROM t WHERE b = 10` | 摘要有 `index_manager_search_all`；结果行正确 |
| 页读仍走原入口 | 只读重开，缓冲池尚未持有目标页 | 点查该行 | 摘要页数至少为 1，解析阶段 Io 为 0；同时有索引子指标 |
| 观察类别排除 Index | 性能统计打开，类别只有 Pipeline | 主键等值 | 没有两个索引阶段名，仍有 execute；结果行正确 |
| 观察类别排除 Pipeline | 性能统计打开，类别只有 Index | 成功插入 | 没有三个流水线子指标；插入的行可见 |
| 采集关闭 | 会话配置保持默认，未设旧开关 | 成功查询 | 没有摘要，不写出新的诊断 stderr |
| 旧表保持 | `RTSQL_PROFILING` 已设置 | CLI 主键等值 | stderr 仍有旧表和 `index_manager_search`；结果行仍在 |
| 错误不变 | 同一条非法语句 | 诊断关闭与只开统计各执行一次 | 错误文本与 SQLSTATE 相同；出错的摘要没有首行等待 |
| 非空查询的首行 | 表里至少一行，性能统计打开 | 查询这些行 | 首行等待有值且不大于语句总耗时；交回行数等于结果行数；行值正确 |
| 插入与空结果 | 性能统计打开 | 插入；再查一个不存在的键 | 两条都没有首行等待；插入的交回行数为 0；空结果行数为 0 |
| 语句被放弃 | 沿用已有的放弃锚点 | 丢弃未完成的插入 future，再插入同一键 | 被放弃摘要没有首行等待；该键可以再次插入 |
| 段落只用保留摘要 | 摘要上限为 1，连续两条成功查询 | 读比较指标 | 吞吐和 P95 只根据后一条摘要 |
| P95 与吞吐公式 | 用构造好的摘要，不跑 SQL | 计算分段指标 | 按规格里的秩和墙钟规则得到数值或「没有值」 |
| 进程峰值 | Linux，查询上限为 0 | 读峰值并执行一条已知查询 | 峰值等于 `ru_maxrss × 1024`；查询返回原行，不报预算超限 |
| 指标不进结果 | 性能统计打开 | 查询一行 | 结果行与诊断关闭时相同，行里没有指标名称 |

## Capabilities

### New Capabilities

无。

### Modified Capabilities

- `unified-diagnostics`：五段旧子指标进入当前语句阶段树；比较指标从保留摘要和进程峰值读出。执行阶段不再假定提交是唯一直接子阶段。

## Impact

- 调用点：`plan_stage_impl` 的三处 `table_metadata_lookup`；`execute_stage` 的 DML 臂与查询臂、`execute_ctas_routed` 的 `executor_creation` / `executor_execution`；`IndexScanExecutor::next` 与 `IndexScanAllExecutor::next`。
- 摘要与读出：`src/diagnostics.rs` 的 `Summary`、`execute_executor` 里第一行放进结果的位置、进程 `getrusage`。
- 既有断言要改写的只有「执行独占 + 提交包含 = 执行包含」：`tests/diagnostics_statement_test.rs` 与 `tests/typed_batch_write_test.rs`。
- 不改 SQL 结果形状、错误文本、SQLSTATE、查询预算的 0 上限短路、旧表打印时机、`[recovery]` 与其他既有 stderr 告警。
- 约束：M19 的分批串行只在最后一轮用一次；M22 里名为 `diagnostics::report` 的交付形态本变更不新增，沿用已经落地的 `record_stage` / `record_page_read`；M23 不预建缺陷轮次。

## 回滚

本变更只增加观察。回滚时去掉五处 `record_stage` 接线、摘要上的开始时刻与交回行数、比较指标读出，并恢复上述两条独占等式断言。不需要迁移库文件，不改变已提交的行。

## Gate 1

Gate 1：PASS。需求、场景缺口和范围由本次 goal 批准，不是跳过。批准不降低场景、规格和追踪检查。
