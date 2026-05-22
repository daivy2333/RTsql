# 任务清单

> 最后更新：2026-05-22（M14 重新规划）

## 当前任务：M14 查询路径优化（第二阶段）

**状态**: 进行中
**目标**: PK 查询达到 3-5x 提速（当前 1.4x，baseline 49µs → 目标 ~10-15µs）

### M14 第一阶段已完成

- [x] BTree 零拷贝迁移（LeafNodeRef + InternalNodeRef）
- [x] SQL Plan 缓存（LruCache<String, PhysicalPlan>，256 容量）
- [x] DDL 清缓存机制
- [x] 缓存命中/未命中测试
- [x] 性能分析：发现 spawn_blocking 是真正瓶颈

### M14 第二阶段待办

- [ ] **T1: 精确性能参数测试**
  - 分阶段计时：parse / plan / create_executor / BTree search 各环节
  - 量化 spawn_blocking / Mutex / block_on / 实际计算的开销比例
  - 对比 SQLite 各阶段耗时
  - 产出精确的性能参数表

- [ ] **T2: 消除 spawn_blocking 调度瓶颈**
  - 基于 T1 数据选择方案：async BTree / 专用线程池 / 行缓存
  - 实现并验证 PK 查询 3-5x 提速
  - 回归测试全量通过

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
