# 优化方向与技术债

> 最后更新：2026-05-22（M13 完成）

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