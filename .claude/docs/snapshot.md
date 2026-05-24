# 项目快照

> 最后更新：2026-05-24（M18 Phase3 全部完成，WAL Group Commit 基准测试通过）

## 当前状态

- **阶段**: M18 Phase3 WAL集成 + Group Commit + 崩溃恢复 ✅ 全部完成
- **进度**: T1-T8 全部 ✅
- **测试**: 410+ tests pass, 0 failures
- **Clippy**: 0 warnings
- **Benchmark**: wal_group_commit_bench.rs 3 groups pass（baseline/group_commit/capacity_impact）

## 最近提交

- feat(M18-T3): introduce logical Row ID to fix gc_test SlottedPage SlotID invalidation bug
- feat(M18-T2): implement WALBuffer with Group Commit strategy
- feat(M18-T1): extend WalRecord with BeginTxn/CommitTxn/AbortTxn, add LSN + CRC32

**未提交变更**:
- TransactionManager WAL 集成 (begin/commit/abort 写 WAL 记录)
- Executor 隐式事务包装 (Insert/Update/Delete 写 BeginTxn+数据+CommitTxn)
- RecoveryManager 数据重放 (full_recover + redo committed + mark uncommitted)
- recovery_e2e_test.rs 6 个测试
- wal_group_commit_bench.rs 3 个 benchmark groups
- Clippy 修复 (too_many_arguments, collapsible_if)

## 遗留问题清单

### Clippy — 0 warnings ✅

### WAL 集成已知限制

- TableManager 纯内存：表定义不持久化，重启后丢失
- BufferPool::mark_tx_aborted 是 stub：未遍历 SlottedPage 标记 uncommitted tuple
- wal_sync_mode 配置推迟到后续 milestone（当前默认 fsync）

### M15 全面对比待完成项

- [ ] 内存消耗对比（启动 + 工作峰值）
- [ ] 启动时间对比
- [ ] 数据文件大小对比
- [ ] 编译产物大小对比
- [ ] 大规模数据加载性能对比
- [ ] 并发场景资源消耗对比

## Git 状态

- **当前分支**: master
- **ahead of origin**: 14 commits + 未提交变更

## 下一步行动

1. **Phase3 T7**: WAL Group Commit 性能基准测试（验证 5-10x faster）
2. **Phase4**: B-Tree Merge（删除后页合并）

**里程碑路线图**:
- M16: ✅ 子查询支持
- M17-Phase1: ✅ 非唯一索引
- M17-Phase2: ✅ B-Tree Split 机制
- M17.5: ✅ 代码清理 + 全面对比
- M18-Phase1: ✅ 架构Warnings清理
- M18-Phase2: ✅ Executor层非唯一索引
- **M18-Phase3**: ✅ **WAL集成 + Group Commit + 崩溃恢复**（T1-T6/T8 完成）
- M18-Phase4: ⏳ B-Tree Merge（待开始）