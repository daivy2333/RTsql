# 优化方向与技术债

> 最后更新：2026-05-23（M17-Phase2 B-Tree Split 完成）

## 已完成的优化

| # | 优化项 | 里程碑 | 结果 |
|---|--------|--------|------|
| 1 | PageGuard 零拷贝 | M13 | scan/filter/sort 5-15% |
| 2 | BufferPool 两阶段锁 | M13 | 并发读 ~5% |
| 3 | Plan Cache (LRU) | M14 | 相同 SQL 1.1x |
| 4 | BTree 零拷贝读 | M14 | PK 查询 1.2x |
| 5 | Async search (AtomicPageId) | M14 | 17x internal + 8x vs SQLite |
| 6 | 聚合函数 + GROUP BY | M15 | 19 tests |
| 7 | 子查询支持（独立+关联） | M16 | 20 tests |
| 8 | 非唯一索引（同页多条目） | M17-Phase1 | 5 tests |
| 9 | B-Tree Split 机制 | M17-Phase2 | 7 tests，支持多层级索引 |

## M14 性能验证（已完成）

| 指标 | RTsql | SQLite | 比值 |
|------|-------|--------|------|
| PK lookup | ~0.66µs | ~5.25µs | 8x faster |
| 16线程并发 | ~54% | - | - |
| 32线程并发 | ~63% | - | - |

## 待执行：M15 全面对比基准

> M15 只完成了速度对比，资源成本对比从未执行。这是 M17.5 阶段的核心任务。

### 待测维度

| 维度 | 测量方式 | 目标 |
|------|----------|------|
| 内存消耗 | 启动后 RSS + 10K 行 INSERT 峰值 RSS | 量化内存效率 |
| 启动时间 | 冷启动 + 热启动时间 | 量化启动开销 |
| 数据文件大小 | 10K/100K 行后 .db 文件大小 | 量化存储效率 |
| 编译产物大小 | release binary 大小 | 量化部署成本 |
| 大规模加载 | 批量 INSERT 吞吐 | 量化写入性能 |
| 并发资源 | 不同并发度下 CPU + 内存 | 量化并发成本 |

## 当前性能瓶颈

| 瓶颈 | 现状 | 目标 | 优化方案 | 里程碑 |
|------|------|------|----------|--------|
| INSERT 慢 | ~440µs/行 | 5-10x 提速 | WAL Group Commit | M18 |
| **B-Tree split 缺失** | **✅ 已完成** | **多层级索引** | **递归 split + 回传** | **M17-Phase2 ✅** |
| Executor WAL 集成 | 未写 WAL | 崩溃恢复 | Executor 写 WAL 记录 | M18 |
| B-Tree Merge | 未实现 | 删除后 underflow | 页合并 + 页释放 | M18+ |

## 技术债清单（M17.5 清理目标）

### Clippy 债务 (47 warnings)

| 优先级 | 类别 | 数量 | 修复策略 |
|--------|------|------|----------|
| P0 | io_other_error | 10 | 批量替换为 io::Error::other() |
| P0 | clone_on_copy | 3 | 替换为 *x |
| P0 | redundant_closure | 4 | 替换为函数引用 |
| P0 | into_iter | 5 | 移除显式调用 |
| P1 | to_string_in_format | 3 | 移除多余 to_string() |
| P1 | explicit_auto_deref | 1 | 移除显式解引用 |
| P1 | byte_char_slices | 1 | 替换为 byte string |
| P1 | single_match | 2 | 替换为 if let |
| P1 | unnecessary_map_or | 2 | 简化表达式 |
| P2 | only_used_in_recursion | 4 | 评估是否移除参数 |
| P2 | too_many_arguments | 2 | 引入参数结构体 |
| P2 | await_holding_lock | 1 | buffer_pool 重构为 tokio Mutex |
| P3 | dead_code | 2 | 评估是否删除字段 |
| P3 | module_inception | 1 | 评估是否重命名模块 |
| P3 | unused imports/vars | 2 | 移除 |

### 测试债务

| 优先级 | 问题 | 修复方式 |
|--------|------|----------|
| P0 | test_btree_insert_duplicate_key_returns_error 失败 | 更新为非唯一索引行为测试 |
| P0 | planner_test.rs 19 个编译错误 | 修复 builder mutability |
| P1 | M17 新功能缺少 SQL 集成测试 | 添加非唯一索引 + split 的 SQL 测试 |

## 优化路线图

| 里程碑 | 优化项 | 目标 | 状态 |
|--------|--------|------|------|
| M17-Phase2 | B-Tree Split | 索引容量扩展 | ✅ 完成 |
| **M17.5** | **代码清理 + 全面对比** | **0 clippy、0 test failures、全面基准** | **⏳ 规划中** |
| M18 | WAL 集成 + Group Commit | INSERT 5-10x 提速 | ⏳ 待开始 |

## 低优先级优化

| 方向 | 说明 |
|------|------|
| io_uring | Linux 5.1+ 零拷贝异步磁盘 |
| jemalloc/mimalloc | 内存分配器 |
| 大查询并行化 | 全表扫描按页切分 |

## 陷阱提醒

```
❌ std::sync::Mutex 不跨 .await 持有
❌ I/O 操作不持锁（两阶段锁模式）
❌ CPU 密集操作用 spawn_blocking 隔离
✅ 读操作用 page_data()，写操作用 modify_page()
✅ HAVING 谓词解析用聚合输出列，不是原始表列
✅ AVG 结果必须是 Float 类型
✅ 相关子查询注入: ParameterExpression + Mutex，clone→inject→rebuild per row
✅ 多层 Plan 检测: 确保检测在提取首列之前触发
⚠️ 关联 IN + 空右侧: 已知 bug，返回所有行而非 0 行（待修复）
✅ InternalNodeRef find_child_page_id: key < key_i → left subtree = child_{i-1}; key == key_i → right subtree = child_i
✅ LeafNode split 后链表维护: 原页 next → 新页，新页 next → 原页旧 next
```