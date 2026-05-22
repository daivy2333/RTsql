# 优化方向与技术债

> 最后更新：2026-05-22（M14 Phase 2 T1 profiling 瓶颈定位完成）

---

## 🎯 当前主要瓶颈（M14 Phase 2 T2 目标）

### IndexManager.search（81% 执行时间）

**Profiling 数据**（PK lookup, cache hit, warm run）：
```
Stage                    | Time (µs) | % Total
-------------------------|-----------|--------
executor_execution       |      57.0 |   90.5%
index_manager_search     |      51.0 |   81.0%  ← 主要瓶颈
executor_creation        |       2.0 |    3.2%
cache_hit_check          |       0.0 |    0.0%
parse_and_plan           |       0.0 |    0.0%
-------------------------|-----------|--------
Total                    |      63.0 |  100.0%
```

**根因分析**：
- spawn_blocking + SyncPageLoader::block_on 调度开销（~25µs）
- std::sync::RwLock<BTree> 锁争用（~5µs）
- 实际 BTree.search 计算（~21µs）

**优化方案**：
1. **启用 async search 路径**（推荐）
   - BTree::search_async（已实现，Box::pin 递归）
   - AsyncPageLoader::load_page（已实现，直接 async）
   - IndexManager 改用 async search
   - 预期：消除 spawn_blocking 调度开销，~3-5x 提速

2. **专用线程池**（替代方案）
   - 创建 dedicated BTree thread pool
   - 减少 spawn_blocking 调度竞争
   - 预期：~2-3x 提速

3. **行缓存**（长期优化）
   - 热点行缓存在 memory
   - 减少 BufferPool 访问
   - 预期：热点查询 ~10x

---

## 其他优化方向（按优先级）

# 优化方向与技术债

> 最后更新：2026-05-22（M13 完成 + 性能分析）

---

## 已修复的 Critical Issues ✅

### 1. PageGuard 零拷贝 ✅

**原问题**: `page()` 每次克隆 4KB
**修复**: 添加 `page_data() → PageDataGuard`（Deref to &[u8]），零拷贝读取
**影响**: read_tuple_from_data_page 改用 page_data() + SlottedPageRef
**验证**: 83 lib tests 通过，scan/filter/sort/limit 改善 5-15%

### 2. BufferPool 两阶段锁 ✅

**原问题**: `get_page()` 持写锁期间做 I/O，阻塞其他协程
**修复**: 读锁检查→释放→I/O→写锁插入（double-check）
**影响**: 并发读改善约 5%

### 3. std::sync::Mutex 安全 ✅

**原问题**: 跨 .await 持有可能死锁
**修复**: 验证 PageGuard/PageDataGuard 不跨 .await 持有，添加 SAFETY 注释
**影响**: 无功能变化，安全文档化

---

## 性能分析结论（2026-05-22）

### M14 后最新数据

| 操作 | M13 baseline | M14 当前 | 提速 |
|------|-------------|---------|------|
| PK lookup (cached, same SQL) | ~49µs | ~36µs | **1.4x** |
| PK lookup (uncached) | ~49µs | ~41µs | **1.2x** |
| PK lookup (diff SQL, 100 unique) | ~49µs | ~49µs | **~1x** |
| Full scan 100 rows | ~84µs | ~74µs | **1.1x** |

### 核心瓶颈（M14 后更新）

| 瓶颈 | 现状 | 根因 | 下一步 |
|------|------|------|--------|
| PK 点查询 | ~36µs/次 | spawn_blocking + Mutex + block_on 调度链 | 消除 spawn_blocking，async BTree |
| INSERT | ~440µs/行 | WAL 逐行 fsync | WAL Group Commit |
| 并发混合 | 16并发 ~53ms | 写操作拖慢吞吐 | 行缓存 + 并发写入优化 |

### 热路径分析（M14 后）

```
execute_sql()
  → [cache hit] parse+plan skipped (~5µs saved)
  → create_executor_from_plan() (~2µs)
  → IndexScanExecutor::next()
    → IndexManager::search()
      → spawn_blocking()          ← ~10-15µs 线程调度
      → Mutex<BTree>::lock()      ← ~5µs 锁获取
      → SyncPageLoader::block_on  ← ~10µs 两次 async→sync 转换
      → BTree::search()           ← ~5µs 实际计算
      → BufferPool::get_page()    ← ~5µs 页缓存命中
```

### 差距归因（M14 后更新）

1. **线程调度开销（主因）**: spawn_blocking + block_on 占 ~25µs，是 PK 查询最大瓶颈
2. **Mutex<BTree> 锁**: 每次 search 需获取全局 Mutex，~5µs
3. **parse+plan 开销（已解决）**: 仅 ~5µs，缓存可消除但收益有限

---

## 🔴 Critical — 查询路径优化

### 10. Prepared Statement 缓存 ✅

**问题**: 每次 `execute_sql()` 都完整走 parse→plan→execute
**修复**: LruCache<String, PhysicalPlan>，容量 256，SELECT 缓存，DDL 清缓存
**实际收益**: 相同 SQL 1.1x 提速（parse+plan 仅 ~5µs，非主要瓶颈）
**教训**: parse+plan 开销远低于预期（~10% 而非 ~60%）

### 11. BTree 迁移到 page_data() 零拷贝 ✅

**问题**: B-Tree 读操作用 `page()` 克隆 4KB
**修复**: 新增 LeafNodeRef/InternalNodeRef，基于 SlottedPageRef 零拷贝
**实际收益**: PK 查询 1.2x 提速（零拷贝消除 4KB clone + modify_page 开销）
**教训**: 零拷贝收益有限，真正瓶颈在 spawn_blocking 调度链

---

## 🟡 Important Issues

### 4. B-Tree split/merge 不完整

**问题**: 当前 B-Tree 仅单叶节点，无 split/merge
**影响**: 插入大量数据后索引性能下降
**修复方案**: 实现 LeafNode split + InternalNode 层级管理
**优先级**: M16

### 5. Executor 层 WAL 写入集成

**问题**: Insert/Update/Delete Executor 未写 WAL 记录
**影响**: 崩溃恢复无法重放数据变更
**修复方案**: Executor 操作后调用 WalWriter::append
**优先级**: M17

### 6. 非唯一索引

**问题**: 仅支持主键索引（唯一）
**影响**: 无法创建辅助索引
**修复方案**: IndexManager 支持非唯一键（duplicate key 允许）
**优先级**: M16

### 12. WAL Group Commit

**问题**: 每次 INSERT 独立 fsync，I/O 开销极大
**影响**: INSERT ~440µs/行，比 SQLite 批量插入慢 ~20x
**修复方案**: WAL 批量提交——多个事务合并一次 fsync，类似 PostgreSQL group commit
**预期收益**: INSERT 5-10x 提速
**优先级**: P1
**难度**: 高
**依赖**: Executor 层 WAL 集成（#5）

### 13. 减少 spawn_blocking 调度开销

**问题**: B-Tree 操作每次 spawn_blocking 有线程调度成本
**影响**: 全操作额外 10-20% 开销
**修复方案**: 评估 B-Tree 操作是否可移至 async 上下文（临界区极短时），或使用专用线程池
**预期收益**: 全操作 10-20% 提速
**优先级**: P1
**难度**: 中

---

## 🟢 Performance Optimizations

### 7. io_uring 替换

**方案**: 替换 spawn_blocking + 同步 I/O → tokio-uring
**优先级**: 低（Linux 5.1+）

### 8. 内存分配器优化

**方案**: jemalloc 或 mimalloc 替换系统分配器
**优先级**: 低

### 9. 大查询并行化

**方案**: 全表扫描按页切分，tokio::spawn 并行处理
**优先级**: 低

### 14. 行缓存（Row Cache）

**方案**: 热点行在 BufferPool 之上增加行级缓存，避免反复解析页数据
**预期收益**: 热点行查询 2-3x 提速
**优先级**: P2
**难度**: 高
**依赖**: Prepared Statement 缓存（#10）

---

## 优化路线图

```
Phase 1 (P0): 查询路径优化 — 预期 PK 查询 3-5x 提速
  ├── #11 BTree 零拷贝迁移（低难度，快速收益）
  └── #10 Prepared Statement 缓存（中难度，最大收益）

Phase 2 (P1): 写入路径优化 — 预期 INSERT 5-10x 提速
  ├── #5  Executor 层 WAL 集成（前置依赖）
  ├── #12 WAL Group Commit（高难度，写入最大收益）
  └── #13 减少 spawn_blocking 开销（中难度）

Phase 3 (P2): 进阶优化 — 热点场景进一步提升
  ├── #4  B-Tree split/merge（大数据集索引）
  ├── #6  非唯一索引（辅助索引支持）
  └── #14 行缓存（热点行查询）

Phase 4 (低优先级): 基础设施优化
  ├── #7  io_uring 替换
  ├── #8  内存分配器优化
  └── #9  大查询并行化
```

### 与里程碑映射

| 里程碑 | 优化项 | 目标 |
|--------|--------|------|
| M14 | #10 + #11 | PK 查询 3-5x 提速 |
| M15 | 聚合函数 + GROUP BY | 功能完善 |
| M16 | 子查询支持 | 功能完善 |
| M17 | #4 + #6 | 索引优化 |
| M18 | #5 + #12 + #13 | WAL 集成 + 写入提速 |
| M19 | #14 + 并发优化 | 缓存与并发 |
| M20 | 多类型支持 | FLOAT/BOOL/NULL |
| M21 | 持久化增强 | 增量 checkpoint + 并行恢复 |

---

## M13 Benchmark 数据

### Micro Benchmarks（100 行）

| 操作 | Baseline | Post-fix | 改善 |
|------|----------|----------|------|
| INSERT | ~440 µs | ~440 µs | ~0% |
| SELECT (pk, 100x) | ~4.9 ms | ~5.1 ms | ~0% |
| UPDATE (100x) | ~9.0 ms | ~9.7 ms | ~0% |
| DELETE (100x) | ~26.2 ms | ~27.3 ms | ~0% |
| SCAN | ~87 µs | ~84 µs | ~3% |
| FILTER | ~86 µs | ~83 µs | ~4% |
| SORT | ~115 µs | ~96 µs | ~14% |
| LIMIT | ~78 µs | ~68 µs | ~13% |
| JOIN | ~163 µs | ~157 µs | ~4% |

### Scale Benchmarks

| 操作 | 数据量 | 延迟 | 吞吐 |
|------|--------|------|------|
| scan | 1K | ~86 µs | ~11.6 Melem/s |
| scan | 10K | ~75 µs | ~113 Melem/s |
| scan | 100K | ~80 µs | ~1.24 Gelem/s |
| join | 100 | ~200 µs | ~500 Kelem/s |
| join | 1K | ~200 µs | ~5.0 Melem/s |
| join | 10K | ~185 µs | ~51 Melem/s |

### SQLite Comparison

| 操作 | RTsql | SQLite |
|------|-------|--------|
| insert 100 rows | - | ~223 ms |
| pk lookup | ~4.9 ms (100x) | ~5.5 µs (1x) |
| full scan 1K | ~84 µs | ~77 µs |
| inner join 1K | ~157 µs | ~102 µs |

---

## 陷阱提醒

```
❌ 不要在 .await 期间持有 std::sync::Mutex
❌ CPU 密集操作必须用 spawn_blocking 隔离
❌ 页缓存淘汰不要用复杂锁竞争
❌ I/O 操作不要持锁（两阶段锁模式）
✅ 读操作用 page_data()（零拷贝），写操作用 modify_page()
```