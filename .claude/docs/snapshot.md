# 项目快照

> 最后更新：2026-05-24（M18 全部完成，B-Tree Merge + ~430 tests pass）

## 当前状态

- **阶段**: M18 全部完成 ✅
- **进度**: Phase1-Phase4 全部 ✅
- **测试**: ~430 tests pass, 0 failures
- **Clippy**: 0 warnings

## 最近提交

- feat(M18-T3): introduce logical Row ID to fix gc_test SlottedPage SlotID invalidation bug
- feat(M18-T2): implement WALBuffer with Group Commit strategy
- feat(M18-T1): extend WalRecord with BeginTxn/CommitTxn/AbortTxn, add LSN + CRC32

**M18-Phase4 未提交变更**:
- node.rs: +LeafMergeResult/InternalMergeResult, +merge_right/redistribute_right/remove_separator/can_merge_with
- btree.rs: 完整重写 delete 路径（MergeInfo + redistribution-first + root shrink 传播）
- async_storage.rs: +free_page trait 方法
- file_storage.rs: +free-list (Mutex<Vec<u64>>)
- buffer_pool.rs / sync_loader.rs: +free_page
- index_manager.rs: delete 返回 Option<PageId> 处理 root shrink
- tests/btree_merge_test.rs: 新增 10 个 merge 集成测试

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

1. **提交 Phase4 变更**：commits 按 wave 组织
2. **项目收尾**：清理遗留问题，性能优化

**里程碑路线图**:
- M16: ✅ 子查询支持
- M17-Phase1: ✅ 非唯一索引
- M17-Phase2: ✅ B-Tree Split 机制
- M17.5: ✅ 代码清理 + 全面对比
- M18-Phase1: ✅ 架构Warnings清理
- M18-Phase2: ✅ Executor层非唯一索引
- **M18-Phase3**: ✅ **WAL集成 + Group Commit + 崩溃恢复**
- **M18-Phase4**: ✅ **B-Tree Merge**（项目完成）