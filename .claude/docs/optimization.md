# 优化方向与技术债

> 最后更新：2026-05-22（M15 完成）

## 已完成的优化

| # | 优化项 | 里程碑 | 结果 |
|---|--------|--------|------|
| 1 | PageGuard 零拷贝 | M13 | scan/filter/sort 5-15% |
| 2 | BufferPool 两阶段锁 | M13 | 并发读 ~5% |
| 3 | Plan Cache (LRU) | M14 | 相同 SQL 1.1x（parse+plan 仅 ~10%） |
| 4 | BTree 零拷贝读 | M14 | PK 查询 1.2x |
| 5 | Async search (AtomicPageId) | M14 | 17x internal + 8x vs SQLite |
| 6 | 聚合函数 + GROUP BY | M15 | 19 tests，功能完善 |

## M14 性能验证（已完成）

| 指标 | 优化前 | 优化后 |
|------|--------|--------|
| index_manager_search | ~51µs (81%) | ~2-4µs |
| RTsql PK lookup | - | ~0.66µs |
| SQLite PK lookup | - | ~5.25µs |
| **RTsql vs SQLite** | - | **8x faster** |

并发改进: 16线程 ~54%，32线程 ~63%

## 当前性能瓶颈

| 瓶颈 | 现状 | 目标 | 优化方案 | 里程碑 |
|------|------|------|----------|--------|
| INSERT 慢 | ~440µs/行 | 5-10x 提速 | WAL Group Commit | M18 |
| B-Tree split/merge 缺失 | 单叶节点 | 多层级索引 | 实现 split + InternalNode | M17 |
| 非唯一索引缺失 | 仅 PK | 辅助索引 | duplicate key 支持 | M17 |
| Executor WAL 集成 | 未写 WAL | 崩溃恢复 | Executor 写 WAL 记录 | M18 |

## 优化路线图

| 里程碑 | 优化项 | 目标 |
|--------|--------|------|
| M16 | 子查询支持 | 功能完善 |
| M17 | B-Tree split/merge + 非唯一索引 | 索引完整性 |
| M18 | WAL 集成 + Group Commit | INSERT 5-10x 提速 |
| M19 | 行缓存 + 并发优化 | 热点行 2-3x |

## 低优先级优化

| 方向 | 说明 |
|------|------|
| io_uring | Linux 5.1+ 零拷贝异步磁盘 |
| jemalloc/mimalloc | 内存分配器 |
| 大查询并行化 | 全表扫描按页切分 |

## M15 补充任务（SQLite 资源对比）

待执行：
- 内存消耗对比（启动 + 工作峰值）
- 启动时间对比
- 数据文件大小对比
- 编译产物大小对比

## 陷阱提醒

```
❌ std::sync::Mutex 不跨 .await 持有
❌ I/O 操作不持锁（两阶段锁模式）
❌ CPU 密集操作用 spawn_blocking 隔离
✅ 读操作用 page_data()，写操作用 modify_page()
✅ HAVING 谓词解析用聚合输出列，不是原始表列
✅ AVG 结果必须是 Float 类型
```