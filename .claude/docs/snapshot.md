# 项目快照

> 最后更新：2026-05-24（项目完成，M18 全部里程碑达成）

## 当前状态

- **阶段**: 项目完成 ✅
- **里程碑**: M1-M18 全部完成
- **测试**: ~430 tests pass, 0 failures
- **Clippy**: 0 warnings
- **功能**: 完整 SQL 嵌入式数据库（DML/DDL/子查询/索引/WAL/MVCC）

## 项目成果

| 维度 | 数据 |
|------|------|
| Rust 源码 | 16+ 核心文件，~8000 行 |
| 测试覆盖 | ~430 tests（含 btree/split/merge/WAL/e2e/executor） |
| SQL 支持 | 19 种 PhysicalPlan 节点 |
| 架构决策 | 8 个 ADR |
| 性能亮点 | INSERT 332x faster than SQLite, PK lookup 8x faster |

## 最近提交

- docs: final README rewrite with comprehensive SQLite comparison
- docs(M18-Phase4): update all documents for BTree Merge completion
- feat(M18-Phase4-T6): add BTree merge integration tests
- feat(M18-Phase4-T4-T5-T7): rewrite BTree delete with merge propagation
- feat(M18-Phase4-T1-T2): add LeafNode and InternalNode merge helpers
- feat(M18-Phase4-T3): add free_page to storage stack with free-list reuse

## 已知限制

- TableManager 纯内存：表定义不持久化，重启后丢失（后续优化方向）
- BufferPool::mark_tx_aborted 是 stub
- 全表扫描性能落后 SQLite ~4x
- 文件大小 ~6.5x SQLite（固定 Key + 两层索引）

## Git 状态

- **当前分支**: master
- **最新 tag**: v0.1.0（M18 完成）

## 下一步

项目核心开发完成。后续方向：
- Varint Key 编码（减少 ~70% 索引空间）
- 全表扫描并行化
- 表定义持久化
- io_uring 异步磁盘 I/O

**里程碑路线图**:
- M16: ✅ 子查询支持
- M17-Phase1: ✅ 非唯一索引
- M17-Phase2: ✅ B-Tree Split 机制
- M17.5: ✅ 代码清理 + 全面对比
- M18-Phase1: ✅ 架构Warnings清理
- M18-Phase2: ✅ Executor层非唯一索引
- **M18-Phase3**: ✅ **WAL集成 + Group Commit + 崩溃恢复**
- **M18-Phase4**: ✅ **B-Tree Merge**（项目完成）