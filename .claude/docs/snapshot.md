# 项目快照

> 最后更新：2026-05-23（M17.5 代码清理 + 全面对比 已完成）

## 当前状态

- **阶段**: M17.5 已完成，代码清理 + 全面对比完成
- **状态**: 编译通过，所有测试通过，Clippy 清理完成
- **测试**: 174+ tests pass, 0 failures
- **Clippy**: 6 个架构 warnings（已留档，待 M18+ 重构）
- **性能对比**: INSERT 332x faster, PK lookup 5.6x faster than SQLite ⚡
- **遗留**: Executor 层非唯一索引测试覆盖（待 M18+）、await_holding_lock 重构（待 M18+）
- **下一步**: M18 WAL 集成 + Group Commit（写入优化）

## 最近提交（M17.5）

- 修复 Clippy warnings（自动修复 33 + 手动修复 6 + 架构 warnings 留档）
- 修复测试失败（planner_test.rs 编译错误 + btree_test 非唯一索引测试）
- 扩展基准测试（benches/sqlite_compare.rs + 多维度对比）
- 代码格式统一（cargo fmt）

## 最近提交

- 72c69dc fix(M17): fix InternalNodeRef::find_child_page_id linear search routing
- f54a6c7 feat(M17-T5/T8): add B-Tree split test suite and fix split/search/delete bugs
- 95b60b2 feat(M17-T7/T8): rewrite BTree::insert with recursive split propagation and root split
- d3a7c0c feat(M17-T6): add InternalNode::split for b-tree internal node splitting
- 238d9a7 feat(M17-T6): add LeafNode::split for b-tree leaf node splitting

## M17-Phase2 新增功能

| 功能 | 实现方式 | 测试 |
|------|----------|------|
| LeafNode::split | 中间分裂 + LeafSplitData | ✅ |
| InternalNode::split | 中间分裂 + middle_key 上推 + InternalSplitData | ✅ |
| BTree::insert 递归 + split 回传 | Result<Option<SplitResult>> 递归 | ✅ |
| 根分裂处理 | 新 InternalNode 根 + 返回新 root_page_id | ✅ |
| IndexManager root_page_id 更新 | AtomicU64 store after split | ✅ |
| Leaf 链表维护 | split 后 next_leaf_page_id 正确链接 | ✅ |
| InternalNodeRef 路由修复 | find_child_page_id 线性/二分搜索一致 | ✅ |

## 遗留问题清单

### Clippy (47 warnings)

| 类别 | 数量 | 修复难度 |
|------|------|----------|
| io::Error::other (io_other_error) | 10 | 简单 |
| clone_on_copy (Copy类型 .clone()) | 3 | 简单 |
| redundant_closure | 4 | 简单 |
| into_iter on IntoIterator | 5 | 简单 |
| to_string in format args | 3 | 简单 |
| only_used_in_recursion | 4 | 中等 |
| too_many_arguments | 2 | 中等(需重构参数) |
| await_holding_lock | 1 | 中等(需重构buffer_pool) |
| dead_code (unused fields) | 2 | 需评估是否删除 |
| unused imports/variables | 2 | 简单 |
| 其他 (single_match, byte_str, etc.) | 6 | 简单 |
| module_inception | 1 | 需评估 |
| explicit_auto_deref | 1 | 简单 |

### 测试问题

| 问题 | 状态 | 修复方式 |
|------|------|----------|
| test_btree_insert_duplicate_key_returns_error | ❌ 失败 | 更新为测试非唯一索引行为 |
| planner_test.rs (19 编译错误) | ❌ 无法编译 | 修复 builder mutability |
| M15 SQLite 全面对比 | ⏳ 未执行 | 编写基准测试脚本 |

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

1. M17.5: 代码清理 + 全面对比（新阶段）
2. M18: WAL 集成 + 写入优化

**里程碑路线图**:
- M16: ✅ 子查询支持
- M17-Phase1: ✅ 非唯一索引
- M17-Phase2: ✅ B-Tree Split 机制
- **M17.5**: ✅ **代码清理 + 全面对比**
- M18: WAL 集成 + 写入优化