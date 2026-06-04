## Context

**当前状态**：`ScanExecutor`（`src/executor/scan.rs`）通过 `IndexManager.scan_all()` 遍历 BTree 索引获取 `(key, RowId)`，再对每行调用 `find_visible_version` → `BufferPool.get_page` → `deserialize_value_refs`。每行 **2 次页访问**（索引页 + 数据页），且一次性预加载所有结果到 `results: Vec<Vec<Value>>`，内存压力大。

**架构现状**：
- `SlottedPageHeader.next_page_id: u32`（`src/storage/page_format/slotted_page.rs:21`）— 数据页链表指针已就位
- `TableMeta.data_page_head: PageId`（`src/storage/data/table_manager.rs:51`）— 链表头已存
- `TableMeta.data_page_tail: Mutex<PageId>`（`src/storage/data/table_manager.rs:52`）— 链表尾（追加用）
- M20 零拷贝 SlottedPageRef（✅）— `with_page_data` 闭包原语
- M36 零拷贝 ValueRef（✅）— `deserialize_value_refs` 借用 `&'a [u8]`

**Stakeholders**：
- 全表扫描查询（无 WHERE / 非 PK WHERE）
- `criterion` 基准测试套件（micro / sqlite_compare）
- 集成测试 `tests/executor_test.rs`

## Goals / Non-Goals

**Goals**：
- 全表扫描从 **2 次页访问/行** 降至 **1 次页访问/行**（仅数据页）
- 流式 `next()` 输出，无需 `results: Vec<Vec<Value>>` 预加载
- 与现有 Planner / Executor / Filter 架构兼容
- 复用 M20 闭包 API + M36 零拷贝 ValueRef
- 全表扫描性能 **~2x 提速**（criterion 验证）

**Non-Goals**：
- ❌ 不修改 `IndexManager.scan_all`（M20 闭包调用，保留为兜底）
- ❌ 不重构 BufferPool / SlottedPage 基础结构
- ❌ 不引入新 crate 依赖
- ❌ 不修改 `IndexScan` 行为（点查最优路径保持）
- ❌ 不优化 MVCC 版本链遍历（仍需跨页访问历史版本）

## Decisions

### D1: 新增 `PhysicalPlan::DataScan` 变体 vs 扩展 `PhysicalPlan::Scan`

**选择**：新增 `DataScan(DataScanNode)` 变体

**理由**：
- 现有 `Scan(ScanNode)` 已被 `IndexScan` 间接使用（作为 fallback）
- 新增变体使 `PlanNode` 自描述，`Filter(DataScan)` 组合更清晰
- 调度层 `src/executor/mod.rs::build_executor` 仅多一个 match 分支

**备选**：
- A) 给 `ScanNode` 加 `scan_mode` 枚举字段 — 修改面更广，影响所有 `Scan` 使用者
- B) 复用 `IndexScan` 节点加 `use_index: bool` — 语义混淆

### D2: DataScanExecutor 状态管理 — 游标 vs Vec 预加载

**选择**：游标式（`current_page_id: Option<PageId>` + `current_slot_index: usize`）

**理由**：
- 与 `IndexScanAllExecutor` 流式风格对齐
- 避免 `results: Vec<Vec<Value>>` 内存压力（大数据表 OOM 风险）
- 单次 `next()` 调用仅访问当前 slot，延迟友好

**备选**：
- A) 维持 `Vec<Vec<Value>>` 预加载 — 简单但内存压力大
- B) 通道 `mpsc::channel` 跨任务 — 复杂且 tokio 协程调度开销

### D3: MVCC 可见性检查 — slot 内解析 vs 链外跟踪

**选择**：在 `DataScanExecutor::next()` 内对每个 slot 解析 `VersionHeader` 检查可见性

**理由**：
- `Slot` 已含 `logical_id` 和数据区偏移，可直接读 VersionHeader
- 不可见时沿 `next_version: Option<RowId>` 跨页访问（与现有 `find_visible_version` 行为一致）
- 复用 M20 闭包 API + M36 `deserialize_value_refs`

**备选**：
- A) 把可见性检查下沉到 `SlottedPage` 层 — 破坏分层（SlottedPage 不应感知 MVCC）
- B) 整体拷贝 `find_visible_version` 逻辑 — 代码重复

### D4: Planner 路由策略 — 三路分发

**选择**：
```
WHERE 条件            →  PhysicalPlan
─────────────────────────────────────────
无                     →  DataScan
非 PK 等值              →  Filter(DataScan)
简单 PK 等值            →  IndexScan（保持原行为）
```

**理由**：
- `IndexScan` 点查最优不可替代
- `DataScan` vs `IndexManager.scan_all` 性能优势明确（页访问减半）
- `Filter(DataScan)` 复用现有 `FilterExecutor`，改造面小

**新增 `has_pk_equality`**（递归 AND 组合）vs 复用 `is_simple_pk_equality`：
- `is_simple_pk_equality` 仅检查**单层** `pk = value`
- 新增 `has_pk_equality` 递归处理 `pk = a AND other_cond` 组合（仍可走 IndexScan，但简化后不优化 PK 与其他条件组合的混合场景）

### D5: PageGuard 生命周期 — 同步 drop vs 跨 await

**选择**：在 `next()` 同步块内获取 `PageGuard`，用 `with_page_data` 闭包消费

**理由**：
- `with_page_data` 闭包返回 `&[u8]` 借用，闭包结束自动释放
- 避免 `PageGuard` 跨 `.await` 持有（M20 闭包 API 核心收益）
- 沿用 M36 `deserialize_value_refs` 借用模式

## Risks / Trade-offs

**R1: MVCC 版本链跨页访问** → 仍需多页访问，收益打折
   - 缓解：仅在 `commit_tx_id` 不可见时触发，常见场景（已提交）仍 1 次页访问
   - 验证：criterion 短事务基准测试（低版本链深度）

**R2: 数据页碎片化** → `next_page_id` 链表不连续，缓存命中率低
   - 缓解：依赖 BufferPool LRU 缓存
   - 验证：`sqlite_compare` 基准测试对比

**R3: `logical_id` 与 `slot_count` 错位** → GC 删除 slot 后遍历需小心
   - 缓解：按 `slot_count` 物理顺序遍历，`logical_id` 仅用于构造 `RowId`
   - 单测覆盖：删除若干 slot 后扫描数量正确

**R4: Planner `has_pk_equality` 误判** → AND 组合中的 `pk = value` 检测遗漏
   - 缓解：递归检查覆盖任意深度 AND 嵌套
   - 单元测试：`pk = a AND b = c` / `pk = a AND pk2 = b` 多个 case

**R5: 现有 `ScanExecutor` 行为兼容性** → 兜底路径仍可能被 `IndexScan` 等触发
   - 缓解：`PhysicalPlan::Scan` 节点保留，仅 `build_query` 主路径改走 DataScan
   - 集成测试：现有全表扫描 SQL 用例全部通过

**R6: 测试覆盖盲点** → 性能数字可能因基准测试规模小而失真
   - 缓解：criterion 多档规模（1K / 10K / 100K 行），与 SQLite 对比

## Migration Plan

**部署步骤**：
1. T1: 实现 `DataScanExecutor` + 单元测试（保持 `ScanExecutor` 不变）
2. T2: 加入 MVCC 可见性检查
3. T3: Planner 无 WHERE 路由改走 `DataScan`
4. T4: Planner 非 PK WHERE 路由改走 `Filter(DataScan)`
5. T5: criterion 基准测试验证提速

**回滚策略**：
- `DataScan` 是新增执行器，旧 `ScanExecutor` / `PhysicalPlan::Scan` 完整保留
- 仅需回退 `src/parser/planner.rs:393` `build_query` 中 3 行修改（`selection.is_none()` 分支）
- 不涉及 schema 变更、数据迁移、API 破坏
- 回归测试：`cargo test --lib --tests` 必须全绿

**数据迁移**：无（纯代码层改造）

## Open Questions

- Q1: `DataScan` 与 `Scan` 并存期间，是否需要加 `#[deprecated]` 标记 `Scan`？
  - 倾向：不加，保留兜底；后续 M-series 完全弃用时再处理
- Q2: `has_pk_equality` 是否要支持 OR 组合中含 PK 等值？
  - 倾向：暂不支持（M21 页面级 MVCC 时再优化 OR 场景）
- Q3: 性能提升如未达 2x（如 1.5x），是否仍合入？
  - 倾向：合入（仍优于现状），但需更新 L0xx 教训记录实际数字
