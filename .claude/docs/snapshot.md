# 项目快照

> 最后更新：2026-05-24（gc_test bug 已修复，M18 Phase3 T3 解除阻塞）

## 当前状态

- **阶段**: M18 Phase3 WAL集成 + Group Commit + 崩溃恢复
- **进度**: T1 ✅ T2 ✅ T3 ✅（logical Row ID 修复完成），T4-T8 待开发
- **测试**: 101+ tests pass, 0 failures（含 gc_test 3 个测试）
- **Clippy**: 0 warnings（代码层面）
- **解除阻塞**: gc_test SlottedPage SlotID 失效 bug 已通过 logical Row ID 修复

## 最近提交

- feat(M18-T3): introduce logical Row ID to fix gc_test SlottedPage SlotID invalidation bug
- feat(M18-T2): implement WALBuffer with Group Commit strategy
- feat(M18-T1): extend WalRecord with BeginTxn/CommitTxn/AbortTxn, add LSN + CRC32

<!-- tombstone: snapshot #01 --> Archived to archive.md §snapshot #01 2026-05-24 — M17-Phase2历史快照

## 遗留问题清单

### Clippy — 0 warnings ✅（Phase1 已全部清理）

### M15 全面对比待完成项

- [ ] 内存消耗对比（启动 + 工作峰值）
- [ ] 启动时间对比
- [ ] 数据文件大小对比
- [ ] 编译产物大小对比
- [ ] 大规模数据加载性能对比
- [ ] 并发场景资源消耗对比

## Git 状态

- **当前分支**: master
- **ahead of origin**: 14 commits

## 下一步行动

1. **Phase3: WAL集成 T4-T8**（TransactionManager集成 + Executor集成 + RecoveryManager + 基准测试 + 崩溃恢复E2E）
2. **Phase4: B-Tree Merge**（删除后页合并）

**里程碑路线图**:
- M16: ✅ 子查询支持
- M17-Phase1: ✅ 非唯一索引
- M17-Phase2: ✅ B-Tree Split 机制
- M17.5: ✅ **代码清理 + 全面对比**
- **M18**: ⏳ **优化项目与技术债清理**（Phase1-3 进行中，Phase4 待开始）