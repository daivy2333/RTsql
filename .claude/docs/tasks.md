# 任务清单

> 最后更新：2026-05-22（M15 聚合函数与 GROUP BY 完成）

## 当前任务：M16 子查询支持

**状态**: 待开始
**优先级**: 中

### M15 聚合函数与 GROUP BY 已完成 ✅

**功能实现成功**：
- ✅ COUNT(*) / COUNT(col) / SUM / AVG / MIN / MAX 聚合函数
- ✅ GROUP BY 单列分组（HashMap hash aggregation）
- ✅ HAVING 聚合结果过滤
- ✅ SQL 标准 NULL 处理语义（COUNT(*) 计所有行，其他跳过 NULL）
- ✅ 严格模式（非聚合列必须出现在 GROUP BY 中）
- ✅ 空表聚合返回单行（COUNT→0，其他→NULL）
- ✅ 聚合 + WHERE + ORDER BY 组合查询

**新增文件**：
- `src/executor/aggregate.rs` — AggregateFunc, AggregateState, AggregateExecutor
- `src/executor/having.rs` — HavingExecutor
- `tests/aggregate_test.rs` — 19 个端到端测试

**修改文件**：
- `src/executor/plan.rs` — AggregateNode, HavingNode
- `src/executor/value.rs` — add(), lt_agg(), div() 算术方法
- `src/parser/planner.rs` — 聚合检测、GROUP BY/HAVING 解析
- `src/parser/error.rs` — 4 个新错误变体
- `src/parser/ast.rs` — extract_columns 支持 Expr::Function
- `src/pipeline.rs` — Aggregate/Having executor 整合

**测试**：19 aggregate tests + 88 lib tests + 其他集成测试 = 149 tests passing

### M14 Phase 2 T2 已完成 ✅✅

**性能优化成功**（目标 5-6x，实际 **17x 提速**）：
- ✅ 架构重构：移除 RwLock<BTree>，改用 AtomicPageId
- ✅ Async search：消除 spawn_blocking 调度开销
- ✅ Async scan_all：读操作完全 async 路径
- ✅ Write operations：保持 sync 路径（使用临时 BTree 实例）
- ✅ BTree::from_root() helper：辅助方法实现
- ✅ Slot compacting：修复 SlottedPage.delete_slot bug

**性能数据**（Profiling 验证）：
- **优化前**：index_manager_search ~51µs (81%)
- **优化后**：index_manager_search ~2-4µs（平均 3µs）
- **提速**：51µs → 3µs = **17x 提速**（远超预期 5-6x）

**SQLite 对比**（可信性验证）：
- SQLite PK lookup: ~5.25µs
- RTsql PK lookup: ~0.66µs (657ns)
- **提速对比**：**8x faster than SQLite**

**并发性能改进**：
| 并发度 | 优化前 | 优化后 | 提速 |
|--------|--------|--------|------|
| 1 线程 | ~170µs | ~99µs | **41%** |
| 4 线程 | ~290µs | ~182µs | **37%** |
| 8 线程 | ~520µs | ~283µs | **46%** |
| 16 线程 | ~1.2ms | ~559µs | **54%** |
| 32 线程 | ~3.2ms | ~1.2ms | **63%** |

**Bug 修复完成**：
- ✅ 根因：SlottedPage.delete_slot 不减少 slot_count
- ✅ 修复：实现 slot compacting（移动 slots backward）
- ✅ 验证：所有测试通过（88 lib + 74 integration）
- ✅ 性能：修复后性能仍然达标（1-3µs）

**测试参数配置**：
- 所有 benchmark: 50 次迭代
- 并发测试: [1, 4, 8, 16, 32] 线程
- 规模测试: [1K, 10K, 100K] 行

**文档更新完成**：
- ✅ optimization.md: 性能数据 + 可信性验证 + 测试参数
- ✅ tasks.md: M14 T2 完成状态
- ✅ snapshot.md: 项目最新状态
- ✅ learned.md: 新踩坑记录

- [x] Profiling 模块实现（task_local! + 输出表格）
- [x] Pipeline 计时点（cache_hit_check, parse_and_plan, table_metadata_lookup, executor_creation, executor_execution）
- [x] IndexScanExecutor 计时（index_manager_search）
- [x] Bench example（examples/bench_minimal.rs + RTSQL_PROFILING=1）
- [x] **瓶颈定位成功**：IndexManager.search 占 81% 执行时间
- [x] Binary search bug 修复（主分支原有 bug，阻塞 merge）

**关键发现**：
- IndexManager.search 通过 spawn_blocking + SyncPageLoader 调用 BTree.search
- 调度开销占 ~81% 执行时间（主要瓶颈）
- Plan cache 工作正常（cache hit 场景 parse/plan = 0µs）

### M14 Phase 2 T2 待办（基于 Profiling 数据）

**瓶颈数据**：IndexManager.search 占 81% 执行时间（51µs）
- spawn_blocking + SyncPageLoader::block_on：~25µs（调度开销）
- std::sync::RwLock<BTree> 锁争用：~5µs
- 实际 BTree.search 计算：~21µs

**优化方案选项**：

#### 方案 1：启用 async search 路径（推荐，预期 3-5x 提速）
- [ ] IndexManager 改用 `search_async` 而非 `spawn_blocking`
- [ ] 移除 SyncPageLoader::block_on 包装
- [ ] 直接调用 AsyncPageLoader::load_page
- [ ] 验证性能：预期消除 ~25µs 调度开销
- **优势**：已有实现（BTree::search_async + AsyncPageLoader），改动小
- **风险**：可能仍有 RwLock 锁争用（~5µs）

#### 方案 2：专用 BTree 线程池（预期 2-3x 提速）
- [ ] 创建 dedicated thread pool（4-8 threads）
- [ ] IndexManager.search 通过专用池执行
- [ ] 减少 spawn_blocking 调度竞争
- **优势**：隔离 BTree 操作，减少调度开销
- **风险**：线程池管理复杂度，可能引入新瓶颈

#### 方案 3：行缓存（长期优化，预期热点查询 10x）
- [ ] 热点行缓存到 memory（LRU + RowId 索引）
- [ ] 减少 BufferPool 访问
- [ ] 需要缓存失效机制（INSERT/UPDATE/DELETE）
- **优势**：彻底消除 BufferPool + BTree 访问
- **风险**：缓存一致性复杂，适合热点查询场景

#### 方案 4：RwLock 替换或优化（预期 1.1-1.2x 提速）
- [ ] 测试 tokio::sync::RwLock（已验证回退 10-20%）
- [ ] 或改用 RwLock + read guard pooling
- [ ] 或无锁设计（MVCC + 乐观读）
- **优势**：减少锁争用
- **风险**：tokio RwLock 已验证有性能回退，需谨慎

#### 方案 5：BTree search 算法优化（预期 1.1-1.2x 提速）
- [ ] LeafNodeRef/InternalNodeRef 二分搜索（已实现）
- [ ] 减少 key 比较（缩短 Key 类型）
- [ ] SIMD 优化（avx2 指令集）
- **优势**：纯算法优化，无架构改动
- **风险**：提速有限（仅优化 ~21µs 计算部分）

**推荐优先级**：
- **高优先级**：方案 1（启用 async search，最小改动，最大效果）
- **中优先级**：方案 5（BTree search 优化，叠加效果）
- **低优先级**：方案 2/3/4（复杂度高，收益不确定）

## 里程碑路线

| 里程碑 | 内容 | 状态 |
|--------|------|------|
| M1-M12 | 核心功能实现 | ✅ 完成 |
| M13 | PageGuard 零拷贝 + BufferPool 两阶段锁 | ✅ 完成 |
| M14 | 查询路径优化 | ✅ 完成（Phase 2 T2） |
| **M15** | **聚合/GROUP BY + SQLite 全面对比** | ⏳ 待开始 |
| M16 | 子查询 | ⏳ 待开始 |
| M17 | 索引优化 | ⏳ 待开始 |
| M18 | WAL + 写入优化 | ⏳ 待开始 |

**M15 补充任务**（SQLite 全面对比）：
- [ ] 创建 `benches/resource_comparison.rs`
- [ ] 内存消耗对比（启动 + 工作峰值）
- [ ] 启动时间对比（`Database::open()` vs `Connection::open()`）
- [ ] 并发资源对比（线程数 + 协程数 + 锁争用）
- [ ] 数据文件大小对比（相同数据量）
- [ ] 编译产物大小对比（Release binary）
- [ ] 更新 README.md 补充资源对比数据

## 阻塞项

无

## 最近完成

- M14 第一阶段：BTree 零拷贝 + SQL 缓存（1.4x 提速）
- M13：PageGuard 零拷贝 + BufferPool 两阶段锁
- M12：WAL + Recovery + Checkpoint
- M11：MVCC + Repeatable Read + 行锁
