# 项目快照

> 最后更新：2026-05-24（M19-M23 全维度性能优化规划）

## 当前阶段

**全维度性能优化（M19-M23）**

核心短板：Full Scan 4x slower than SQLite，文件大小 6.5x larger。

| 里程碑 | 优化项 | 预期收益 | 状态 |
|--------|--------|---------|------|
| M19 | DataScan 路径 | ~2x 扫描提速 | 待开始 |
| M20 | 零拷贝读取 | ~20-30% I/O 提速 | 待开始 |
| M21 | 页面级 MVCC | ~10-15% 提速 | 待开始 |
| M22 | 预取 Prefetch | 大表 ~15-25% 提速 | 待开始 |
| M23 | Varint Key 编码 | 索引空间 ~70% 缩减 | 待开始 |

## 历史里程碑

M1-M18 核心开发完成（2026-05-24 归档）：
- ~430 tests pass, Clippy 0 warnings
- INSERT 332x faster, PK lookup 5.6x faster than SQLite
- 完整 SQL + WAL + Group Commit + 崩溃恢复 + B-Tree Split & Merge

## 已知限制

- 全表扫描性能落后 SQLite ~4x
- 文件大小 ~6.5x SQLite（固定 Key + 两层索引）
- TableManager 纯内存：表定义不持久化
- BufferPool::mark_tx_aborted 是 stub

## Git 状态

- **当前分支**: master
- **最新 tag**: v0.1.0（M18 完成）