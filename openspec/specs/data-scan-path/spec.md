# data-scan-path Specification

## Purpose
TBD - created by archiving change m19-datascan-path. Update Purpose after archive.
## Requirements
### Requirement: DataScanExecutor 全表扫描

系统 SHALL 提供 `DataScanExecutor`，直接遍历数据页链表（通过 `SlottedPageHeader.next_page_id`），执行全表扫描时每行仅访问 1 次数据页（不经过索引层）。

#### Scenario: 单数据页扫描
- **WHEN** 表数据全部位于单个数据页
- **THEN** DataScanExecutor 从 `TableMeta.data_page_head` 起始，遍历该页所有 slot，返回全部行
- **AND** 仅触发 1 次 BufferPool 页访问

#### Scenario: 多数据页链表扫描
- **WHEN** 表数据跨多个数据页
- **THEN** DataScanExecutor 沿 `next_page_id` 链表依次访问所有数据页，返回全部行
- **AND** 链表末尾（`next_page_id == 0`）返回 `Ok(None)` 终止

#### Scenario: 流式 next() 输出
- **WHEN** 消费者调用 `next()` 一次
- **THEN** DataScanExecutor 返回单行 `ExecResult::Row(values)`，不预加载整张表
- **AND** 当前 slot 耗尽时自动跳转下一页

### Requirement: MVCC 可见性检查

DataScanExecutor SHALL 在遍历每个 slot 时检查 MVCC 可见性：对当前事务 snapshot 不可见的版本 SHALL 沿 `VersionHeader.next_version` 链查找可见版本。

#### Scenario: 当前提交版本可见
- **WHEN** slot 数据 `VersionHeader.commit_tx_id <= snapshot.max_tx_id` 且对当前事务可见
- **THEN** DataScanExecutor 反序列化该 slot 并返回行

#### Scenario: 当前版本不可见，链上存在可见旧版本
- **WHEN** slot 数据 `commit_tx_id > snapshot.max_tx_id`（未提交或晚于 snapshot）
- **AND** 沿 `next_version` 链找到对 snapshot 可见的旧版本
- **THEN** DataScanExecutor 反序列化旧版本并返回行

#### Scenario: 版本链无可见版本
- **WHEN** slot 数据及其整个版本链均对 snapshot 不可见
- **THEN** DataScanExecutor 跳过该 slot，继续下一个 slot

### Requirement: Planner 路由无 WHERE 全表扫描

`build_query` 在 `SELECT` 无 `WHERE` 条件时 SHALL 返回 `PhysicalPlan::DataScan(DataScanNode)`，不再走 `PhysicalPlan::Scan`。

#### Scenario: SELECT * FROM table 无 WHERE
- **WHEN** SQL 为 `SELECT * FROM t`（无 WHERE）
- **THEN** Planner 生成 `PhysicalPlan::DataScan(DataScanNode { table_name: "t", columns: [...] })`
- **AND** 执行器调度为 `DataScanExecutor`

#### Scenario: 兜底路径仍可用
- **WHEN** 显式构造 `PhysicalPlan::Scan(ScanNode)`（测试或历史调用方）
- **THEN** 调度层仍能生成 `ScanExecutor`，不破坏现有调用

### Requirement: Planner 路由非 PK 等值 WHERE

`build_query` 在 WHERE 条件**不含** PK 等值（含 AND 组合中无 PK 等值）时 SHALL 返回 `PhysicalPlan::Filter(FilterNode { input: DataScan })`。

#### Scenario: 非 PK 列过滤
- **WHEN** SQL 为 `SELECT * FROM t WHERE name = 'foo'`（name 非主键）
- **THEN** Planner 生成 `Filter(DataScan(DataScanNode))`，复用 `FilterExecutor`

#### Scenario: AND 组合含 PK 等值
- **WHEN** SQL 为 `SELECT * FROM t WHERE id = 1 AND name = 'foo'`（id 为主键）
- **THEN** Planner 仍生成 `PhysicalPlan::IndexScan`（点查最优不可替代）

#### Scenario: AND 组合无 PK 等值
- **WHEN** SQL 为 `SELECT * FROM t WHERE name = 'foo' AND age > 18`（无 PK 等值）
- **THEN** Planner 生成 `Filter(DataScan)`

### Requirement: 性能基线

全表扫描在 `benches/data_scan_bench.rs` 中的 `criterion` 基准测试 SHALL 显示出 DataScanExecutor 相比 ScanExecutor 至少 **1.5x 提速**（理想目标 ~2x）。

#### Scenario: 1K 行表全表扫描
- **WHEN** 表有 1K 行（100B/行），执行无 WHERE 全表扫描
- **THEN** DataScanExecutor 报告的中位耗时 ≤ ScanExecutor 的 67%（即至少 1.5x 提速）

#### Scenario: 100K 行表全表扫描
- **WHEN** 表有 100K 行（100B/行），执行无 WHERE 全表扫描
- **THEN** DataScanExecutor 报告的中位耗时 ≤ ScanExecutor 的 67%

### Requirement: 测试覆盖

DataScanExecutor SHALL 有完整的单元测试与集成测试覆盖：单页扫描、跨页链表扫描、MVCC 可见、MVCC 不可见、Filter 路由、Planner 路由。

#### Scenario: 单元测试
- **WHEN** 运行 `cargo test --lib executor::data_scan`
- **THEN** 至少 5 个测试用例全部通过：单页、空表、跨页、MVCC 可见、MVCC 不可见

#### Scenario: 集成测试
- **WHEN** 运行 `cargo test --test executor_test`
- **THEN** 现有全表扫描测试全部通过（无回归），新增 2 个 DataScan 路径测试通过

#### Scenario: 回归测试
- **WHEN** 运行 `cargo test --lib --tests`
- **THEN** 0 失败，所有现有测试无回归

