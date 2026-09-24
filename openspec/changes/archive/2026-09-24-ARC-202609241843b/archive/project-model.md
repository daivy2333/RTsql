# 归档载体：project-model 条目（ARC-202609241843b 批次）

> 本文件是 openspec-archivist 清理批次的 Archive 载体，保存被归档 M 条目的完整原文。
> 条目原位置：`openspec/specs/project-model/spec.md`；归档日期：2026-09-24。


## M07: 两阶段锁 BufferPool（历史基线）

- **分类**: architecture
- **范围**: 缓存加载路径
- **不变量**: 读锁→释放→I/O→写锁(double-check) 模式加载缺失页
- **证据**: `src/storage/buffer_pool.rs:BufferPool::get_page`
- **状态**: active（M31 演进后被 DashMap + per-page loading_locks 增强）
- **Legacy**: L012, ADR-012

## M16: 已知限制（不变量边界）

- **分类**: compatibility
- **范围**: 系统行为边界
- **不变量**:
  - TableManager 纯内存：表定义不持久化（M44 计划解决）
  - 全表扫描性能已通过 M19 DataScan 优化至 1.8-2.4x 提速
  - 文件大小 ~6.5x SQLite（固定 Key + 两层索引）
  - 仅 Repeatable Read 隔离级别（M24 计划解决）
- **证据**: `src/storage/data/table_manager.rs`（纯内存）
- **状态**: active
- **Legacy**: snapshot.md "已知限制"
