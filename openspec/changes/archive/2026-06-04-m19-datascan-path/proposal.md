## Why

当前 `ScanExecutor`（`src/executor/scan.rs`）采用 **Index→RowId→Data** 三步访问模式：每次全表扫描需要先遍历 BTree 索引获取 `(key, RowId)`，再对每行访问数据页读取 tuple，导致每行至少 **2 次页访问**（索引页 + 数据页）。实测全表扫描性能落后 SQLite ~4x（详见 `SNAPSHOT.md` 已知限制）。

`SlottedPageHeader.next_page_id`（`src/storage/page_format/slotted_page.rs:21`）与 `TableMeta.data_page_head`（`src/storage/data/table_manager.rs:51`）**已存在**支持数据页链表遍历，但 `ScanExecutor` 未利用。依赖 M20 零拷贝 SlottedPageRef（✅ 2026-06-03）与 M36 零拷贝 ValueRef（✅ 2026-06-03）已落地，M19 可直接复用零拷贝基础设施，从架构上对齐 SQLite Table B-Tree 设计（全表扫描每行 1 次页访问）。

**Why now**：M20/M36 已完成，零拷贝读写路径稳定，DataScan 改造无依赖阻塞；实测分配开销 ~5% 改进已封顶，要再提速需从访问模式层面切入。

## What Changes

- **新增** `PhysicalPlan::DataScan(DataScanNode)` 枚举变体（`src/executor/plan.rs`）
- **新增** `DataScanExecutor`（`src/executor/data_scan.rs`）：直接遍历数据页链表，**每行 1 次页访问**（仅数据页），无需 `results: Vec<Vec<Value>>` 预加载（流式 `next()`）
- **修改** `build_query`（`src/parser/planner.rs:393`）扫描方式选择逻辑：
  - 无 WHERE → `PhysicalPlan::DataScan`
  - 有 WHERE 但**无** PK 等值条件 → `PhysicalPlan::Filter(DataScan)`（复用现有 `FilterExecutor`）
  - 有 WHERE 且**是**简单 PK 等值 → 保持 `PhysicalPlan::IndexScan`（点查最优）
- **新增** `has_pk_equality` 递归检查（`planner.rs`）：检测 AND 组合中是否含 `pk = value`
- **新增** `benches/data_scan_bench.rs`：criterion 基准测试对比 `ScanExecutor` vs `DataScanExecutor`
- **新增** `tests/executor_test.rs` 中 DataScanExecutor 集成测试（无 MVCC → 含 MVCC → Filter 路由）

**不做什么**：
- ❌ 不修改 `IndexManager.scan_all` 行为（保持原 `ScanExecutor` 用于 `IndexScan` 场景的 fallback）
- ❌ 不重构 BufferPool / SlottedPage（M20 闭包 API 已就绪）
- ❌ 不引入新依赖（tokio/criterion 已存在）

**BREAKING**：无。`PhysicalPlan::Scan(ScanNode)` 仍保留为兜底，`IndexScan` 行为不变。

## Capabilities

### New Capabilities
- `data-scan-path`: 全表扫描数据页链表直接遍历能力，配套 Planner 路由选择与 MVCC 可见性检查

### Modified Capabilities
无。`zero-copy-page-access` (M20) 与 `zero-copy-value-ref` (M36) 均为已落地能力，M19 仅**复用**而非修改其需求规格。

## Impact

**新增文件**（2）：
- `src/executor/data_scan.rs` (~120 行)
- `benches/data_scan_bench.rs` (~80 行)

**修改文件**（3）：
- `src/executor/plan.rs` (+12 行) — `DataScan` 枚举变体
- `src/executor/mod.rs` (+3 行) — 导出 `DataScanExecutor`
- `src/parser/planner.rs` (+40 行) — `build_query` 改造 + `has_pk_equality` 检查

**集成测试**（1）：
- `tests/executor_test.rs` (+60 行) — DataScanExecutor 单测 + 集成测试

**性能预期**（基于分析文档第 6.4 节）：
- 全表扫描 **~2x 提速**（页访问次数减半：2→1）
- 流式处理减少内存压力（无需 `results: Vec<Vec<Value>>`）
- MVCC 版本链遍历场景收益打折（仍需跨页访问历史版本）

**回滚方案**：
- `DataScan` 是新增执行器，旧 `ScanExecutor` / `PhysicalPlan::Scan` 保留
- Planner 路由改回 `Scan` 仅需 3 行修改（`build_query` 中 `selection.is_none()` 分支）
- 仓库可平滑回退至 M36 状态

**引用 ADR**：
- A{N}: M20 零拷贝 SlottedPageRef 决策（已归档 learned.md L011-L015）
- A{N}: M36 零拷贝 ValueRef 决策（已归档 learned.md L025）
