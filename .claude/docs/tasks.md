# 任务清单

> 最后更新：2026-05-23（M17-Phase1 非唯一索引 完成）

## 当前任务：M17-Phase2 B-Tree Split 机制

**状态**: 待继续
**优先级**: 高
**范围**: Split 机制实现（非唯一索引已完成）

### M17 任务分解

| Task | 内容 | 依赖 | 状态 |
|------|------|------|------|
| T1 | LeafNode 去掉 DuplicateKey 检查 | - | ✅ 完成 |
| T2 | LeafNodeRef::find_all_matches | T1 | ✅ 完成 |
| T3 | BTree::search_all / delete_by_key / delete_exact | T2 | ✅ 完成 |
| T4 | SplitResult 结构定义 | - | ✅ 完成 |
| T5 | InternalNode::insert_separator | T4 | ✅ 完成 |
| T6 | LeafNode split 逻辑（split_leaf） | T4 | ⏳ 待实现 |
| T7 | 递归 insert + split 传播 | T5, T6 | ⏳ 待实现 |
| T8 | 根分裂处理（create_new_root） | T7 | ⏳ 待实现 |
| T9 | 测试：容量 + 非唯一 + split（完整测试套） | T1-T8 | ✅ 部分完成（5 tests pass） |

## M17-Phase1 已完成（非唯一索引）

**测试覆盖**: 5 tests（non_unique_insert + find_all_matches + search_all_matches + delete_by_key + delete_exact）

| 功能 | 实现方式 | 测试 |
|------|----------|------|
| LeafNode 允许重复 key | 去掉 DuplicateKey 检查 | ✅ |
| LeafNodeRef::find_all_matches | 遍历所有 slot 查找匹配 | ✅ |
| BTree::search_all | 返回所有匹配 RowId | ✅ |
| BTree::delete_by_key | 删除所有匹配 entries | ✅ |
| BTree::delete_exact | 精确删除（key + RowId） | ✅ |
| SplitResult 结构 | middle_key + new_page_id | ✅（待使用）|
| InternalNode::insert_separator | separator 插入方法 | ✅（待使用）|

## 里程碑路线

| 里程碑 | 内容 | 状态 |
|--------|------|------|
| M1-M12 | 核心功能实现 | ✅ 完成 |
| M13 | PageGuard 零拷贝 + BufferPool 两阶段锁 | ✅ 完成 |
| M14 | 查询路径优化（17x 内部提速，8x vs SQLite） | ✅ 完成 |
| M15 | 聚合/GROUP BY/HAVING | ✅ 完成 |
| **M16-Phase1** | **子查询支持（独立子查询）** | ✅ 完成 |
| **M16-Phase2** | **相关子查询注入逻辑** | ✅ 完成 |
| **M17-Phase1** | **非唯一索引** | ✅ 完成 |
| M17-Phase2 | B-Tree Split 机制 | ⏳ 待实现 |
| M17.5 | B-Tree Merge 机制（删除后 underflow） | ⏳ 待规划 |
| M18 | WAL 集成 + 写入优化（INSERT 5-10x 提速） | ⏳ 待开始 |

## M16 全部完成

**测试覆盖**: 90 lib + 20 subquery + 19 aggregate = 129 tests pass

### 独立子查询（Phase 1）

| 功能 | 实现方式 | 测试 |
|------|----------|------|
| WHERE IN 子查询 | SemiJoin 反嵌套 | ✅ |
| WHERE NOT IN 子查询 | AntiJoin 反嵌套 | ✅ |
| WHERE EXISTS / NOT EXISTS | SemiJoin/AntiJoin (空条件) | ✅ |
| SELECT 标量子查询 | SubqueryEval Volcano 节点 | ✅ |
| FROM 派生表子查询 | DerivedScan 物化节点 | ✅ |

### 相关子查询（Phase 2）

| 功能 | 实现方式 | 测试 |
|------|----------|------|
| WHERE IN 关联 | ParameterExpression + 按行 clone+inject+rebuild | ✅ |
| WHERE NOT IN 关联 | 同上（AntiJoinExecutor） | ✅ |
| WHERE EXISTS 关联 | SemiJoin 关联注入 | ✅ |
| WHERE NOT EXISTS 关联 | AntiJoin 关联注入 | ✅ |
| SELECT 标量关联 | SubqueryEval 按行注入 | ✅ |
| 多层嵌套检测 | has_outer_refs_outside 编译期检测 | ✅ |

## M16-Phase2 已完成的 Tasks

| Task | 内容 | 状态 |
|------|------|------|
| T1 | CorrelatedParam 重构（inner_column_index → param_name）+ ParameterExpression | ✅ |
| T2 | Planner 外部引用检测 + 多层嵌套错误 | ✅ |
| T3 | inject_correlated_values 函数（新模块 correlated.rs） | ✅ |
| T4 | SemiJoinExecutor 相关子查询执行逻辑 | ✅ |
| T5 | AntiJoinExecutor 相关子查询执行逻辑 | ✅ |
| T6 | SubqueryEvalExecutor 相关子查询执行逻辑 | ✅ |
| T7 | Pipeline 接线 | ✅ |
| T8 | 集成测试（20 tests） | ✅ |

## 阻塞项

- 无

## 已知限制

1. **关联 IN + 空右侧**: 返回所有行而非 0 行（已标记 KNOWN BUG）
2. **EXISTS + SELECT 1 表达式**: 已修复（extract_columns / expr_to_column_name 支持 Expr::Value）
3. **多层嵌套 SemiJoin/AntiJoin**: 已修复（get_subquery_first_column 支持 SemiJoin/AntiJoin）

## M15 补充任务（SQLite 全面对比）

- [ ] 内存消耗对比（启动 + 工作峰值）
- [ ] 启动时间对比
- [ ] 数据文件大小对比
- [ ] 编译产物大小对比

## 最近完成

- **M16-Phase2**: 相关子查询全功能完成，ParameterExpression + Mutex 注入架构，20 tests pass
- **M16-Phase1**: 独立子查询全功能完成，14 tests pass
- **M15**: 聚合函数(COUNT/SUM/AVG/MIN/MAX) + GROUP BY + HAVING，19 tests
- **M14**: 查询路径优化，17x 内部提速，8x vs SQLite
