# 任务清单

> 最后更新：2026-05-22（M14 Phase 2 T1 完成 + Binary search bug 修复）

## 当前任务：M14 Phase 2 T2（IndexManager.search 优化）

**状态**: 准备开始
**目标**: 消除 spawn_blocking 瓶颈，PK 查询从 ~34µs → ~10-15µs（~3x 提速）

### M14 Phase 2 T1 已完成 ✅

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
| M14 | 查询路径优化 | 🔄 进行中（第二阶段） |
| M15 | 聚合/GROUP BY | ⏳ 待开始 |
| M16 | 子查询 | ⏳ 待开始 |
| M17 | 索引优化 | ⏳ 待开始 |
| M18 | WAL + 写入优化 | ⏳ 待开始 |

## 阻塞项

无

## 最近完成

- M14 第一阶段：BTree 零拷贝 + SQL 缓存（1.4x 提速）
- M13：PageGuard 零拷贝 + BufferPool 两阶段锁
- M12：WAL + Recovery + Checkpoint
- M11：MVCC + Repeatable Read + 行锁
