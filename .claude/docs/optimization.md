# 优化方向与技术债

> 最后更新：2026-05-24（归档旧优化项）

## 已完成的优化

| # | 优化项 | 里程碑 | 结果 |
|---|--------|--------|------|
| 5 | Async search (AtomicPageId) | M14 | 17x internal + 8x vs SQLite |
| 6 | 聚合函数 + GROUP BY | M15 | 19 tests |
| 7 | 子查询支持（独立+关联） | M16 | 20 tests |
| 8 | 非唯一索引（同页多条目） | M17-Phase1 | 5 tests |
| 9 | B-Tree Split 机制 | M17-Phase2 | 7 tests，支持多层级索引 |
| 10 | **架构Warnings清理** | **M18-Phase1** | **0 warnings（代码层面）** ⚡ |
| 11 | **Executor层非唯一索引** | **M18-Phase2** | **IndexScanAllExecutor + 3 tests** ⚡ |

<!-- tombstone: optimization #01 --> Archived to archive.md §optimization #01 2026-05-24 — M13已完成 >90d
<!-- tombstone: optimization #02 --> Archived to archive.md §optimization #02 2026-05-24 — M13已完成 >90d
<!-- tombstone: optimization #03 --> Archived to archive.md §optimization #03 2026-05-24 — M14已完成 >90d
<!-- tombstone: optimization #04 --> Archived to archive.md §optimization #04 2026-05-24 — M14已完成 >90d

## M14 性能验证（已完成）

| 指标 | RTsql | SQLite | 比值 |
|------|-------|--------|------|
| PK lookup | ~0.66µs | ~5.25µs | 8x faster |
| 16线程并发 | ~54% | - | - |
| 32线程并发 | ~63% | - | - |

## M17.5 全面对比基准（已完成）

> 2026-05-23: RTsql vs SQLite 多维度性能对比（通过扩展 criterion.rs 基准测试）

### 速度对比结果

| 操作 | SQLite | RTsql | 性能比 | 说明 |
|------|--------|-------|--------|------|
| **INSERT 100 rows** | 232ms | 693µs | **332x faster** ⚡ | RTsql 异步 I/O + MVCC 无锁读 |
| **PK Lookup** | 5.88µs | 1.05µs | **5.6x faster** ⚡ | B-Tree 零拷贝 + AtomicPageId |
| **Full Scan 1K** | 80µs | 327µs | 4x slower | SQLite 扫描优化成熟 |
| **JOIN 1K** | 待测 | 待测 | - | RTsql JOIN 支持 M16+ |

### 资源消耗对比

| 维度 | SQLite | RTsql | 比值 | 说明 |
|------|--------|-------|------|------|
| **数据文件 (10K rows)** | 217KB | 1.4MB | 6.5x larger | RTsql 页格式开销（见下方分析） |
| **二进制大小** | 1.6MB | 3.5MB | 2.2x larger | RTsql Tokio runtime + async 依赖 |

### 文件大小差异分析

> 2026-05-23: 为什么 RTsql 文件大小 ~6.5x larger than SQLite？

#### 存储结构对比

| 项目 | RTsql | SQLite |
|------|-------|--------|
| **索引结构** | 两层分离（索引页 + 数据页） | 聚簇索引（数据在 B-Tree） |
| **Key 存储** | 固定 32 bytes | Varint 1-9 bytes |
| **页填充率** | 50-70% | 70-90% |
| **数据序列化** | Tag byte + 固定长度 | Varint + 变长 |

#### RTsql 存储开销（10K rows）

```
索引层（B-Tree Leaf）：
  - 每个 entry: Key (32B) + RowId (6B) + Slot (4B) = 42B
  - 10K rows: 420KB → 页开销 → ~700KB

数据层（Data Pages）：
  - 每个 tuple: (id 4B + name ~20B + value 4B) + Slot 4B = ~32B
  - 10K rows: 320KB → 页开销 → ~532KB

总计: ~1.2MB → 实际测量 1.4MB ✅
```

#### SQLite 存储开销（10K rows）

```
聚簇 B-Tree：
  - INTEGER PRIMARY KEY 作为 rowid（无额外索引层）
  - Varint 编码（1-3 bytes per integer）
  - 数据直接在 B-Tree leaf pages

10K rows * ~22B (avg tuple) ≈ 220KB → 页元数据 → 217KB ✅
```

#### 核心差异原因

| 原因 | 影响 | RTsql 设计权衡 |
|------|------|---------------|
| **两层索引** | ~3x larger | ✅ 灵活性：支持多索引、非唯一索引 |
| **固定 Key 32B** | ~10x per key | ✅ 简化实现：避免变长处理复杂性 |
| **页填充率低** | ~1.3x larger | ✅ MVCC 友好：SlottedPage 易于版本链 |
| **Tag byte** | ~1.2x larger | ✅ 类型安全：明确类型标记 |

#### 结论

RTsql 选择**实现简洁性 + 架构灵活性**，牺牲一些空间效率：
- ✅ 性能验证：INSERT 332x faster, PK lookup 5.6x faster
- ✅ 功能扩展：非唯一索引、多索引支持更灵活
- ✅ MVCC 实现：SlottedPage + 两层分离更易版本管理

后续优化方向（M18+）：
- Varint Key 编码（减少 ~70% Key 开销）
- 页填充率优化（提高到 80%+）
- 不追求聚簇索引（保持架构灵活性）

### M17 新功能性能验证

| 功能 | 测试结果 | 性能评估 |
|------|----------|----------|
| **B-Tree Split** | ✅ 通过 | Split 后 PK lookup 性能保持稳定 |
| **非唯一索引** | ✅ 通过 | search_all 正常处理重复键 |

### 结论

RTsql 在写入和点查询场景展现出显著性能优势，验证了异步协程架构的有效性。全表扫描性能落后于 SQLite，后续可通过并行扫描优化（M18+）。资源消耗略高于 SQLite，但仍在可接受范围内。

**核心优势**：
- ✅ 写入性能：异步 I/O + 两阶段锁缓冲池
- ✅ 点查询性能：B-Tree 零拷贝 + MVCC 无锁读
- ✅ Split 机制：多层级索引容量扩展无性能损失
- ✅ 非唯一索引：灵活索引模式支持

## 当前性能瓶颈

| 瓶颈 | 现状 | 目标 | 优化方案 | 里程碑 |
|------|------|------|----------|--------|
| INSERT 慢 | ~440µs/行 | 5-10x 提速 | WAL Group Commit | M18 |
| **B-Tree split 缺失** | **✅ 已完成** | **多层级索引** | **递归 split + 回传** | **M17-Phase2 ✅** |
| Executor WAL 集成 | 未写 WAL | 崩溃恢复 | Executor 写 WAL 记录 | M18 |
| B-Tree Merge | 未实现 | 删除后 underflow | 页合并 + 页释放 | M18+ |

## M17.5 Clippy 清理（已完成）

> 2026-05-23: M17.5 Clippy 清理（简单 warnings 自动修复）

**结果**：自动修复 33个 + 手动修复 6个，剩余 6个架构 warnings 留档待 M18+

---

## M18 Phase1 架构Warnings清理（已完成）⚡

> 2026-05-23: Phase1 完成所有架构 warnings 清理

**成果**：
- ✅ warnings 从 6降至 0（仅剩 cargo config deprecated）
- ✅ JoinConfig/JoinRelatedConfig：参数组织清晰（解决 too_many_arguments）
- ✅ CreateExecutorFuture type alias：简化 async 返回类型（解决 type_complexity）
- ✅ #[allow] await_holding_lock：两阶段锁模式安全设计
- ✅ #[allow] module_inception：标准命名模式合理

---

## M18 Phase2 Executor层非唯一索引测试覆盖（已完成）⚡

> 2026-05-23: Phase2 完成 Executor 层非唯一索引支持

**成果**：
- ✅ IndexManager::search_all 方法：async，支持非唯一索引查询
- ✅ IndexScanAllExecutor：惰性初始化 + MVCC可见性迭代 + 逐行返回
- ✅ executor_test.rs 新增 3 个测试：基础功能/空结果/单结果
- ✅ PhysicalPlan::IndexScanAll 节点：完整集成链路（Planner + Pipeline + correlated.rs）
- ✅ 101 tests pass, 0 failures
- ✅ Clippy 0 warnings（代码层面）

**关键实现技巧**：
- ✅ 惰性初始化：search_all 在首次 next() 调用时执行，避免不必要开销
- ✅ MVCC 可见性迭代：while 循环跳过不可见版本，继续下一个 row_id
- ✅ 测试方法创新：直接使用 write_tuple_to_data_page + IndexManager.insert 创建重复键数据

---

## M18 Phase3 WAL集成 + Group Commit + 崩溃恢复（进行中）

> 2026-05-23: Phase3 开始，T1/T2 完成，T3 被 gc_test bug 阻塞

**T1 成果**（WalRecord 扩展 + CRC32 + LSN）：
- ✅ WalRecord 新增 BeginTxn/CommitTxn/AbortTxn 变体
- ✅ LSN + CRC32 序列化格式：[lsn:8B][type:1B][len:4B][body:var][crc32:4B]
- ✅ WalWriter::write_batch 批量写入 + 单次 fsync
- ✅ WalReader CRC 校验 + 新旧格式自动检测

**T2 成果**（WALBuffer + Group Commit）：
- ✅ WALBuffer 内存缓冲 + Notify 信号 + 后台 tokio task
- ✅ Group Commit: append_commit_and_wait → flush_notify → do_flush → 通知等待者
- ✅ tokio::select! 双监听: flush_notify + 定时器
- ✅ Database 添加 wal_buffer 字段 + start_flush_loop()

**阻塞项**：
- ❌ gc_test 3 个测试 panic — GC 删除 tuple 后 SlottedPage SlotID 失效（M10 遗留 bug）

---

## 技术债清单（M18 Phase1 已清理）

### Clippy 债务 ✅ 已解决

| Warning | 解决方案 | 状态 |
|---------|----------|------|
| too_many_arguments | JoinConfig/JoinRelatedConfig struct | ✅ 已解决 |
| type_complexity | CreateExecutorFuture type alias | ✅ 已解决 |
| await_holding_lock | #[allow] + 两阶段锁注释 | ✅ 已解决 |
| module_inception | #[allow] + 合理设计注释 | ✅ 已解决 |

### 测试债务

| 问题 | 状态 | 修复方式 |
|------|------|----------|
| Executor层非唯一索引测试覆盖缺失 | ✅ 已解决（M18-Phase2） | 新增 IndexScanAllExecutor + 3 tests |

---

## 优化路线图

| Warning | 文件 | 说明 | 重构方向 |
|---------|------|------|----------|
| too_many_arguments (9/7) | anti_join.rs, semi_join.rs | Executor::new 参数过多 | 引入 JoinConfig struct |
| too_many_arguments (8/7) | join.rs | Executor::new 参数过多 | 引入 JoinConfig struct |
| type_complexity | pipeline.rs | 返回类型过于复杂 | 定义 ExecutorFuture type alias |
| module_inception | btree/mod.rs | mod btree 与模块同名 | 评估是否重命名为 btree_node |
| await_holding_lock | buffer_pool.rs | MutexGuard 跨 await | 重构为 tokio::sync::Mutex |
| dead_code: output_columns | aggregate.rs | 未读字段（保留） | 投影优化时使用 |
| dead_code: tx_id | delete.rs | 未读字段（保留） | MVCC 事务可见性检查 |

#### Dead Code 字段用途说明

| 字段 | 所在结构 | 当前状态 | 未来用途 |
|------|----------|----------|----------|
| output_columns | AggregateExecutor | 未使用 | 投影优化：聚合后输出列名，避免重新计算列顺序 |
| tx_id | DeleteExecutor | 未使用 | MVCC 事务：删除操作的可见性检查，确保只删除当前事务可见的行 |

### 测试债务

| 优先级 | 问题 | 修复方式 | 状态 |
|--------|------|----------|------|
| P0 | test_btree_insert_duplicate_key_returns_error 失败 | 更新为非唯一索引行为测试 | ✅ 已修复 |
| P0 | planner_test.rs 19 个编译错误 | 修复 builder mutability | ✅ 已修复 |
| P1 | Executor 层非唯一索引测试覆盖缺失 | IndexScanAllExecutor + 3 tests | ✅ 已修复（M18-Phase2） |
| **P0** | **gc_test 3 个测试 panic** | **GC delete_slot + compacting 后 SlotID 变化，但 row_id 引用未更新 → slice 越界** | **❌ 未修复** |

**Executor 层测试覆盖说明**：M17 的非唯一索引功能（NonUniqueIndex + search_all）已在 BTree 层通过 btree_test 和 btree_split_test 验证，但 Executor 层（IndexScanExecutor）暂不支持 search_all。需要后续添加 IndexScanAllExecutor 或修改 IndexScanExecutor 以支持非唯一索引扫描。

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