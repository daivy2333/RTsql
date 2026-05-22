# 任务跟踪

> 最后更新：2026-05-22（性能分析后路线更新）

## 已完成

### M13: 性能基准测试与关键优化 ✅

**目标**: 性能基准测试框架 + Critical 优化

- [x] criterion.rs 基准测试框架（4 套 benchmark: micro/concurrent/scale/sqlite_compare）
- [x] Baseline 数据收集
- [x] PageGuard 零拷贝（page_data() + PageDataGuard + SlottedPageRef）
- [x] BufferPool 两阶段锁（释放写锁后再做 I/O，避免阻塞其他协程）
- [x] Mutex 安全验证 + SAFETY 注释
- [x] Post-fix benchmark 对比（scan/filter/sort/limit 改善 5-15%）
- [x] 性能瓶颈分析（PK 查询慢 10x、INSERT 慢 20x 根因定位）

**完成日期**: 2026-05-22

---

### M12: JOIN 支持 ✅
**完成日期**: 2026-05-21

### M11: ORDER BY / LIMIT / OFFSET ✅
**完成日期**: 2026-05-21

### M10: WHERE 过滤 ✅
**完成日期**: 2026-05-21

### M9: B-Tree 索引 ✅
**完成日期**: 2026-05-21

### M8: 网络层 ✅
**完成日期**: 2026-05-20

### M7: MVCC 事务 ✅
**完成日期**: 2026-05-20

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

### Phase 1: 查询路径优化（P0）

- **M14**: 查询路径优化 — PK 查询 3-5x 提速
  - BTree 零拷贝迁移（`page()` → `page_data()` + `SlottedPageRef`）
  - Prepared Statement 缓存（跳过重复 parse/plan）

### Phase 2: 功能完善

- **M15**: 聚合函数与 GROUP BY（COUNT/SUM/AVG/MIN/MAX + GROUP BY + HAVING）
- **M16**: 子查询支持（标量子查询 / IN 子查询 / 派生表）
- **M17**: 索引优化（B-Tree split/merge + 非唯一索引）

### Phase 3: 写入路径优化（P1）

- **M18**: WAL 集成 + 写入优化 — INSERT 5-10x 提速
  - Executor 层 WAL 写入集成
  - WAL Group Commit（批量 fsync）
  - spawn_blocking 调度优化

### Phase 4: 进阶优化

- **M19**: 并发与缓存优化（行缓存 + 并发写入优化）
- **M20**: 多类型支持（FLOAT/BOOL/NULL 语义完善）
- **M21**: 持久化与恢复增强（增量 checkpoint + 并行恢复）

## 阻塞

无
