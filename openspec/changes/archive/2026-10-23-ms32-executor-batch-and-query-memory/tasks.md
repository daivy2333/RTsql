# Tasks: 执行器批量接口与查询内存上限

来源：proposal What Changes；设计 D1–D5。里程碑 MS32（I081、I082）。

> Status: 计划已批准（Gate 1 为 2026-10-23 的范围选择；Gate 2 为同日原话「批准」）。Iteration 000 已 accepted（生产攒批撤回，默认 `next_batch` 保留，全量 2125/0/2）。Iteration 001 经 001-replan Cycle 于 2026-10-23 accepted（全量 2126/0/2）。Iteration 002 于 2026-10-23 accepted（有界排序，全量 2128/0/2）。Iteration 003 于 2026-10-23 accepted（查询内存上限，全量 2144/0/2）。**四轮全部 accepted，12 task 全部完成，change 无剩余 Iteration——待收尾（docs-maintainer）。**

## 1. 批量接口（Iteration 000）

- [x] 1.1 为 `Executor` 增加默认 `next_batch`，并用 `StubRowExecutor` 验证 5 行按 2 分批、`max = 0` 不消费。依赖：无。验收：`cargo test --lib next_batch_matches_rows -- --test-threads=1` 通过。
- [x] 1.2 在默认方法落地后，为 10000 行 DataScan 的 `next_batch(256)` 保存 criterion 基线 `before-ms32-batch`。依赖：1.1。验收：基准命令退出码 0，Act Response 记下平均时间。
- [x] 1.3 按 D1 实现 DataScan 攒批和生产路径接线，再对比基线；平均时间至少低 10% 才保留接线，否则撤回接线、保留默认方法。依赖：1.2。验收：同序测试仍通过，Act Response 同时有基线和对比的平均时间。
- [x] 1.4 按项目模型 M19 做全量分批回归。依赖：1.3。验收：`--lib --bins` 一批，`tests/*.rs` 按文件名字典序每 10 个一批，每批 `--test-threads=1` 且 0 failed。

## 2. 哈希 JOIN 左表逐行（Iteration 001）

- [x] 2.1 去掉左表整表缓存和匹配桶整桶克隆，既有 JOIN 结果保持，公共入口在语句中途失败时不返回部分行。依赖：Iteration 000。验收：`cargo test --test join_test --test chain_join_test --test self_join_test -- --test-threads=1` 通过。见证 SQL 改为 JOIN 的 WHERE 谓词溢出（投影表达式在 JOIN 上计划期拒绝，见 `001-replan.md`）。
- [x] 2.2 按 1.4 的分批方式做全量回归。依赖：2.1。验收：各批 0 failed。

## 3. 有界排序（Iteration 002）

- [x] 3.1 仅当 `LIMIT` 直接包住 `Sort`，或中间只隔一层 `Projection` 时，排序只保留 `offset + limit` 行；`EXPLAIN` 文本不变；`DISTINCT` 仍全量排序。依赖：Iteration 001。验收：`cargo test --test sort_test --test order_by_alias_sort_test --test limit_test -- --test-threads=1` 通过，另有唯一键前缀测试。
- [x] 3.2 按 1.4 的分批方式做全量回归。依赖：3.1。验收：各批 0 failed。

## 4. 查询内存上限（Iteration 003）

- [x] 4.1 增加每语句预算、`Database::set_query_memory_limit` 和 `RTSQL_QUERY_MEMORY_BYTES`。默认 0。非法环境变量在打开前 exit 2。依赖：Iteration 002。验收：未设置时既有查询成功；非法值的 CLI 测试退出码 2 且不创建库文件。
- [x] 4.2 按 D5 在驻留点计费和释放。16 字节加字符串长度。最终结果行不计。依赖：4.1。验收：直接 `SELECT *` 在 16 字节上限下成功；派生表、右表 JOIN、无界排序、`DISTINCT … LIMIT 1`、`INSERT SELECT` 按 spec 失败或成功。
- [x] 4.3 三条公共入口都返回 `Execution error: query memory limit exceeded (limit N bytes)` 和 SQLSTATE `53200`，下一条语句不继承计数。依赖：4.2。验收：`cargo test --test query_memory_test -- --test-threads=1` 通过。
- [x] 4.4 按 1.4 的分批方式做全量回归。依赖：4.3。验收：各批 0 failed。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| executor-batch-interface 批量取出与逐行取出同序 | 1.1 | `next_batch_matches_rows` | Covered |
| executor-batch-interface 批量生产路径不改变查询结果 | 1.3、1.4 | DataScan 同序测试 + 1.3 的时间门 | Covered |
| query-memory-budget 默认不限制 | 4.1 | `query_memory_test` 未配置成功 | Covered |
| query-memory-budget 非法配置在打开前拒绝 | 4.1 | CLI 非法环境变量 | Covered |
| query-memory-budget 驻留字节按固定口径计费 | 4.2、4.3 | 直接扫描成功、派生表失败、第二条语句成功 | Covered |
| query-memory-budget 超限整句失败 | 4.3 | 排序超限与 `INSERT SELECT` 零写入 | Covered |
| query-memory-budget 哈希 JOIN 不驻留整表左输入 | 2.1、4.2 | JOIN 结果测试；小右表加大左表在上限下成功 | Covered |
| query-memory-budget LIMIT 直接约束排序时只驻留有界行 | 3.1、4.2 | 唯一键前缀；`LIMIT 1` 在小上限下成功；`DISTINCT` 失败 | Covered |

## Iteration Plan

### Iteration 000: 批量接口

- Status: accepted（2026-10-23 Plan Review。时间门未通过，生产攒批已撤回，默认 `next_batch` 保留。全量 2125/0/2，来源 Act Response，criterion 均值与 `target/criterion` 中的 estimates 一致）
- Tasks: 1.1、1.2、1.3、1.4
- Depends on: None
- Stable baseline: `next_batch` 与 `next` 同序。生产路径要么仍逐行，要么 DataScan 攒批且 10000 行基准平均时间至少低 10%。查询结果不变。
- Verification boundary: 1.1 单测；1.2 和 1.3 用 `cargo bench --bench data_scan_bench`。全量分批在 1.4 预算一次。
- Diagnostic boundary: `src/executor/executor_trait.rs`、`data_scan.rs`、`filter.rs`、`projection.rs`、`limit.rs`、`having.rs`、`pipeline.rs` 的抽取循环、`benches/data_scan_bench.rs`
- Non-goals: JOIN、Top-N、内存上限

### Iteration 001: 哈希 JOIN 左表逐行

- Status: accepted（2026-10-23 Plan Review，001-replan Cycle。join.rs 两阶段重构：左表逐行探测、匹配桶不整桶克隆；见证为 WHERE 谓词溢出整句 Err。全量 2126/0/2，来源 Act Response）
- Tasks: 2.1、2.2
- Depends on: Iteration 000
- Stable baseline: 等值 JOIN 结果与变更前一致；左表不再整表进入 `left_rows`；匹配桶不整桶克隆。
- Verification boundary: 2.1 的 JOIN 测试与中途错误测试。全量分批在 2.2 预算一次。
- Diagnostic boundary: `src/executor/join.rs`
- Non-goals: 选择较小构建侧、嵌套循环 JOIN 的组合行克隆、内存上限

### Iteration 002: 有界排序

- Status: accepted（2026-10-23 Plan Review。`SortNode.bounded` 两 Limit 包装点形状赋值、sort.rs 驻留 ≤ offset+limit、EXPLAIN 文本不变、Distinct 形态不越界。全量 2128/0/2，来源 Act Response）
- Tasks: 3.1、3.2
- Depends on: Iteration 001
- Stable baseline: 直接受 LIMIT 约束的排序只保留 `offset + limit` 行；唯一键前缀与全量排序再截取相同；EXPLAIN 文本不变。
- Verification boundary: 3.1 的排序、别名和 LIMIT 测试。全量分批在 3.2 预算一次。
- Diagnostic boundary: `src/executor/plan.rs`、`src/executor/sort.rs`、`src/parser/planner/query.rs`、`src/executor/plan_format.rs`
- Non-goals: 相等键决胜、DISTINCT 下推、溢写

### Iteration 003: 查询内存上限

- Status: accepted（2026-10-23 Plan Review。语句级预算 + 双通道配置 + D5 驻留点计费 + 三入口 53200 错误面；`ScanExecutor.results` 经 spec 场景裁定不计费〔design D5 已加注〕。全量 2144/0/2 = 2128 + query_memory_test 12 + query_budget 单测 4，来源 Act Response）
- Tasks: 4.1、4.2、4.3、4.4
- Depends on: Iteration 002
- Stable baseline: 默认 0；超限整句 53200；D5 的驻留点按 16 字节加字符串长度计费；写语句零副作用。
- Verification boundary: 4.1–4.3 的 `tests/query_memory_test.rs` 和 CLI 非法值。全量分批在 4.4 预算一次。
- Diagnostic boundary: `src/executor/query_budget.rs`、D5 列出的执行器、`src/database.rs`、`src/pipeline.rs`、`src/cli/mod.rs`、`src/storage/error.rs`
- Non-goals: 最终结果集计费、CLI flag、取消和超时

## 平衡审计

四轮各自只有一个可单独验收的结果。批量去留、JOIN 行配对、排序前缀、超限错误互不替代，所以不并成用户原先说的两轮。1.2 只产生基线数字，必须和 1.1、1.3 放在同一轮，否则没有可保留的接口结果。全量回归留在每轮最后，不单独成轮。
