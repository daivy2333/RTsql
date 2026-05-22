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

### M14 Phase 2 T2 待办

- [ ] **T2: 消除 spawn_blocking 调度瓶颈**
  - 方案选择：async BTree search / 专用线程池 / 直接 async search
  - 实现并验证 PK 查询 ~3x 提速（34µs → 10-15µs）
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
