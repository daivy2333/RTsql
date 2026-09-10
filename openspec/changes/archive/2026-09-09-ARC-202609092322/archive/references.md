# 归档载体：references 条目（ARC-202609092322 批次）

> 本文件是 openspec-archivist 清理批次的 Archive 载体，保存被归档 R 条目的完整原文。
> 条目原位置：`openspec/specs/references/spec.md`；归档日期：2026-09-09。

## 项目测试统计

## R05: 项目测试统计（2026-06-04）

- **类型**: schema
- **用途**: 当前测试覆盖与基准测试清单
- **内容**:
  - 总测试数: 475 tests pass, 0 failures（2026-06-04 统计；M31 完成后 481 tests pass）
  - Executor 测试: executor_test.rs（29 tests，含 M19 DataScan 8 tests）
  - 聚合测试: aggregate_test.rs（19 tests）
  - B-Tree 测试: btree_test.rs + btree_split_test.rs + btree_merge_test.rs（22 tests）
  - Visibility 测试: visibility_test.rs（5 tests，含 M21 页面级 MVCC）
  - 基准测试: 8 套（micro/concurrent/scale/sqlite_compare/single/precise_compare/data_scan/visibility）
- **状态**: active（数值会随实施更新）
- **Legacy**: R004

