# 任务跟踪

> 最后更新：2026-05-22

## 已完成

### M13: 性能基准测试与关键优化 ✅

**目标**: 性能基准测试框架 + Critical 优化

- [x] criterion.rs 基准测试框架（4 套 benchmark: micro/concurrent/scale/sqlite_compare）
- [x] Baseline 数据收集
- [x] PageGuard 零拷贝（page_data() + PageDataGuard + SlottedPageRef）
- [x] BufferPool 两阶段锁（释放写锁后再做 I/O，避免阻塞其他协程）
- [x] Mutex 安全验证 + SAFETY 注释
- [x] Post-fix benchmark 对比（scan/filter/sort/limit 改善 5-15%）

**实现内容**:
- Phase 1: 基准测试框架搭建（criterion + rusqlite + tempfile + 4 套 benchmark）
- Phase 2: Critical 优化（PageGuard 零拷贝 + BufferPool 两阶段锁 + SAFETY 注释）

**完成日期**: 2026-05-22
**验证结果**: cargo test (83 lib tests passed) ✅, cargo bench ✅
**新增文件**: benches/micro_bench.rs, benches/concurrent_bench.rs, benches/scale_bench.rs, benches/sqlite_compare.rs, benches/common/mod.rs
**关键改动**: PageGuard::page_data() 零拷贝, SlottedPageRef 只读访问, BufferPool::get_page() 两阶段锁

---

### M12: JOIN 支持 ✅

**目标**: INNER JOIN 支持

- [x] JoinExecutor 哈希连接实现
- [x] PlanBuilder JOIN 语法解析
- [x] Pipeline 集成
- [x] E2E 测试

**完成日期**: 2026-05-21

---

### M11: ORDER BY / LIMIT / OFFSET ✅

**目标**: 排序与分页

- [x] SortExecutor（内存排序 + 归并）
- [x] LimitExecutor（LIMIT/OFFSET）
- [x] Pipeline 集成 + E2E 测试

**完成日期**: 2026-05-21

---

### M10: WHERE 过滤 ✅

**目标**: 条件过滤

- [x] FilterExecutor + Predicate 系统
- [x] 表达式求值（比较/逻辑/算术）
- [x] Pipeline 集成 + E2E 测试

**完成日期**: 2026-05-21

---

### M9: B-Tree 索引 ✅

**目标**: 主键索引

- [x] BTree 插入/搜索/删除
- [x] IndexManager + IndexScanExecutor
- [x] Pipeline 集成 + E2E 测试

**完成日期**: 2026-05-21

---

### M8: 网络层 ✅

**目标**: PostgreSQL 协议兼容

- [x] PgProtocol 状态机
- [x] ConnectionHandler + Server
- [x] Graceful shutdown

**完成日期**: 2026-05-20

---

### M7: MVCC 事务 ✅

**目标**: 快照隔离

- [x] TransactionManager + Snapshot
- [x] VersionChain + RowLock
- [x] BEGIN/COMMIT/ROLLBACK

**完成日期**: 2026-05-20

---

### M1-M6: 基础设施 ✅

- M1: 项目骨架 + Tokio 异步运行时
- M2: 页式存储（4KB Page + SlottedPage）
- M3: BufferPool（Clock 淘汰）
- M4: WAL（write-ahead logging + recovery）
- M5: SQL 解析（sqlparser-rs + PlanBuilder）
- M6: 执行器框架（Scan/Insert/Update/Delete + Pipeline）

---

## 进行中

无

## 待办

- **M14**: 聚合函数与 GROUP BY（COUNT/SUM/AVG/MIN/MAX + GROUP BY + HAVING）
- **M15**: 子查询支持（标量子查询 / IN 子查询 / 派生表）
- **M16**: 索引优化（B-Tree split/merge + 非唯一索引）
- **M17**: 多类型支持（FLOAT/BOOL/NULL 语义完善）
- **M18**: 持久化与恢复增强（增量 checkpoint + 并行恢复）

## 阻塞

无