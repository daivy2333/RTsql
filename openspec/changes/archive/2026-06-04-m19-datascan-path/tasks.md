# M19 DataScan 路径 — 实施任务清单

> 关联: M19（依赖 M20 ✅、M36 ✅）
> 关联 spec: `specs/data-scan-path/spec.md`
> 关联 design: `design.md`
> 关联分析: `.claude/analysis/m19-datascan-path.md`

## 1. DataScanExecutor 实现

- [ ] 1.1 新建 `src/executor/data_scan.rs`：`DataScanExecutor` 结构体（含 `current_page_id: Option<PageId>` + `current_slot_index: usize`）
- [ ] 1.2 实现 `Executor::next()` 纯顺序扫描逻辑（无 MVCC）：沿 `data_page_head` → `next_page_id` 链表遍历每个 slot
- [ ] 1.3 单元测试 `tests::single_page_scan`：单数据页全部 slot 正确返回
- [ ] 1.4 单元测试 `tests::multi_page_linked_scan`：跨多页链表顺序返回，末尾返回 `Ok(None)`
- [ ] 1.5 单元测试 `tests::empty_table_scan`：空表（`data_page_head == 0`）返回 `Ok(None)`
- [ ] 1.6 单元测试 `tests::deleted_slots_scan`：GC 后部分 slot 被删除，扫描数量与 schema 一致
- [ ] 1.7 `cargo test --lib executor::data_scan` 全绿（5+ 测试用例）

**验收标准**：`cargo test --lib executor::data_scan` 0 失败，无 MVCC 场景下数据正确

## 2. MVCC 可见性检查

- [ ] 2.1 扩展 `DataScanExecutor`：在 `next()` 内对每个 slot 解析 `VersionHeader`（`begin_tx_id` + `commit_tx_id` + `next_version`）
- [ ] 2.2 实现可见性判断：`commit_tx_id <= snapshot.max_tx_id` 且 `commit_tx_id` 不在活跃事务集中
- [ ] 2.3 不可见时沿 `next_version: Option<RowId>` 跨页访问旧版本（复用 `BufferPool::get_page` + `with_page_data`）
- [ ] 2.4 单元测试 `tests::mvcc_current_visible`：已提交当前版本对 snapshot 可见
- [ ] 2.5 单元测试 `tests::mvcc_old_version_chain`：当前版本不可见，沿链找到旧可见版本
- [ ] 2.6 单元测试 `tests::mvcc_no_visible_version`：版本链无可见版本，跳过 slot
- [ ] 2.7 `cargo test --lib executor::data_scan` 全绿（5+ → 8+ 测试用例）

**验收标准**：MVCC 场景下可见性判断正确，跨页版本链遍历功能正常

## 3. Planner 路由：PhysicalPlan 枚举扩展

- [ ] 3.1 修改 `src/executor/plan.rs`：`PhysicalPlan` 新增 `DataScan(DataScanNode)` 变体
- [ ] 3.2 定义 `DataScanNode { table_name: String, columns: Vec<String> }`
- [ ] 3.3 修改 `src/executor/mod.rs::build_executor`：新增 `PhysicalPlan::DataScan(_) => DataScanExecutor::new(...)` 分支
- [ ] 3.4 单元测试 `tests::executor_dispatch_data_scan`：直接构造 `DataScanNode` 调度为 `DataScanExecutor`
- [ ] 3.5 `cargo build` + `cargo clippy` 0 警告

**验收标准**：`PhysicalPlan::DataScan` 节点能被正确调度为 `DataScanExecutor`，无编译警告

## 4. Planner 改造：无 WHERE 选 DataScan

- [ ] 4.1 修改 `src/parser/planner.rs:393 build_query`：在 `selection.is_none()` 分支返回 `PhysicalPlan::DataScan`
- [ ] 4.2 集成测试 `tests::planner_no_where_routes_to_data_scan`：`SELECT * FROM t` 生成 `DataScan` 节点
- [ ] 4.3 集成测试 `tests::planner_with_where_keeps_index_scan`：`SELECT * FROM t WHERE id = 1`（id 是 PK）生成 `IndexScan`
- [ ] 4.4 集成测试 `tests::planner_with_where_routes_to_filter_scan`：`SELECT * FROM t WHERE name = 'x'` 生成 `Filter(Scan)`（原行为保留，因 Filter 改 DataScan 是 T5）
- [ ] 4.5 `cargo test --test executor_test` 全绿，无回归

**验收标准**：无 WHERE 场景走 DataScan 路径，有 PK 等值 WHERE 仍走 IndexScan

## 5. Planner 改造：非 PK WHERE → Filter(DataScan)

- [ ] 5.1 新增 `Planner::has_pk_equality(table_name, expr)` 递归方法（planner.rs）
- [ ] 5.2 修改 `build_query`：WHERE 非 PK 等值时返回 `Filter(FilterNode { input: Box::new(DataScan(...)), ... })`
- [ ] 5.3 单元测试 `tests::has_pk_equality_simple`：`id = 1` 返回 `true`
- [ ] 5.4 单元测试 `tests::has_pk_equality_and_combined`：`id = 1 AND name = 'x'` 返回 `true`
- [ ] 5.5 单元测试 `tests::has_pk_equality_none`：`name = 'x' AND age > 18` 返回 `false`
- [ ] 5.6 单元测试 `tests::has_pk_equality_nested`：`(id = 1 OR name = 'x') AND age > 18` 返回 `false`（OR 内 PK 暂不优化）
- [ ] 5.7 集成测试 `tests::planner_non_pk_where_routes_to_filter_data_scan`：`WHERE name = 'x'` 生成 `Filter(DataScan)`
- [ ] 5.8 `cargo test --lib parser::planner` 全绿

**验收标准**：`has_pk_equality` 递归检测正确，非 PK WHERE 全部走 `Filter(DataScan)`

## 6. criterion 基准测试

- [ ] 6.1 新建 `benches/data_scan_bench.rs`：criterion group 包含 `scan_executor_via_index` + `data_scan_executor` 两个 bench
- [ ] 6.2 1K 行表基准（100B/行）— 单测规模验证功能
- [ ] 6.3 10K 行表基准 — 中等规模
- [ ] 6.4 100K 行表基准 — 验证 ~2x 提速目标
- [ ] 6.5 运行 `cargo bench --bench data_scan_bench` 并记录输出
- [ ] 6.6 输出片段粘贴至 `.claude/analysis/m19-datascan-path.md` 性能小节
- [ ] 6.7 更新 `learned.md` 记录 M19 实施后的实测性能（参考 L025 模式）

**验收标准**：`cargo bench` 跑通，100K 行场景 `data_scan_executor` 中位耗时 ≤ `scan_executor_via_index` 的 67%

## 7. 集成验证与文档同步

- [ ] 7.1 `cargo fmt` 全文件无差异
- [ ] 7.2 `cargo clippy --all-targets` 0 警告
- [ ] 7.3 `cargo test --lib --tests` 0 失败（基线 110+ lib + 全部集成测试）
- [ ] 7.4 `cargo bench` 性能数字达预期（≥ 1.5x 提速）
- [ ] 7.5 更新 `.claude/docs/SNAPSHOT.md`：M19 状态 `⏳` → `✅`，记录实测性能数字
- [ ] 7.6 更新 `.claude/docs/tasks.md`：T01-T07 任务状态同步
- [ ] 7.7 更新 `openspec/specs/learned/spec.md`：M19 实施经验记录（L026+，参考 L025 模式）
- [ ] 7.8 提交：commit message 格式 `feat(executor): M19 DataScan 路径 (~2x 全表扫描提速)`，按子任务拆分

**验收标准**：所有验证命令输出 0 失败，文档状态同步，commit history 清晰
