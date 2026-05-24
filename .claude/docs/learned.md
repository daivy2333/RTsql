# 学习记忆

> 最后更新：2026-05-24（M19-M23 规划，历史知识精简）

## 项目完成总结（M1-M18）

| 维度 | 数据 |
|------|------|
| 总里程 | M1-M18 全部完成 |
| 核心源码 | 16+ 文件，~8000 行 |
| PhysicalPlan 节点 | 19 种 |
| 架构决策 (ADR) | 8 个 |
| 测试总数 | ~430 tests |
| 性能亮点 | INSERT 332x faster, PK lookup 8x faster than SQLite |
| 技术栈 | Rust + Tokio + sqlparser-rs + criterion |

---

## 架构知识

| 发现 | 详情 | 来源 |
|------|------|------|
| Redistribution-first 策略 | 先借后合优于纯 merge，避免 ping-pong merge/split | M18-Phase4 |
| MergeInfo 传播模式 | 新增 `new_root: Option<PageId>` 处理 root shrink | M18-Phase4 |
| InternalNode sibling 查找 | `child_index` 定位后，left = pos-1, right = pos+1 | M18-Phase4 |
| separator_key 匹配 | merge 时 separator_key = 被吸收页的 first key | M18-Phase4 |
| free-list 实现 | `Mutex<Vec<u64>>` + allocate_page 先 pop + free_page 先 push 后 zero | M18-Phase4 |
| 隐式事务模式 | 每个 DML 在 executor next() 中自动写 BeginTxn→数据→CommitTxn | M18-Phase3 |
| append_commit_and_wait | CommitTxn 后必须调用确保 WAL 持久化确认 | M18-Phase3 |
| WALBuffer 核心 | Mutex<Vec<(u64,WalRecord)>> + AtomicU64 LSN + Notify + 后台 tokio task | M18-Phase3 |
| Group Commit 机制 | append_commit_and_wait → flush_notify → do_flush 批量写入+fsync → 通知等待者 | M18-Phase3 |
| LSN + CRC32 序列化 | `[lsn:8B][type:1B][len:4B][body:var][crc32:4B]` | M18-Phase3 |
| Slot 扩展为 6B | `Slot { logical_id: u16, offset: u16, length: u16 }` | M18-Phase3 gc_test fix |
| next_logical_id 分配 | SlottedPageHeader 新增，每次 add_slot 递增，永不回收 | slotted_page.rs:36 |
| RowId.slot_id 语义变更 | slot_id 现在是 logical_id（稳定跨 compact） | row_id.rs:8 |

---

## 关键踩坑（精简）

| 问题 | 根因 | 解决 |
|------|------|------|
| delete_by_key 并发 merge 位置偏移 | merge 改变父节点结构，后续子节点索引失效 | `&mut self` + root_page_id 现场更新 |
| merge 容量溢出 | min_keys=48，leaf 容量=92，47+48=95>92 | redistribution-first + can_merge_with 拦截 |
| gc_test SlotID 失效 | compacting 改变物理 SlotID，版本链引用旧值 | logical_id 解耦（Slot 4B→6B） |
| delete_slot 不序列化 header | slot_count 修改只在内存，未 serialize 回 page.data | header 修改后必须 serialize |
| gc_test panic 连锁 | 第一个 panic poison BufferPool Mutex → PoisonError 连锁 | 引入 logical_id 根本修复 |
| RecoveryManager 需要表才能 redo | get_table 失败时 redo 静默跳过 | 表定义持久化是完整恢复前提 |
| RecoveryManager::recover 返回元组 | 基础版返回 HashSet 元组，full_recover 返回 RecoveryResult struct | 按需选择版本 |
| HashSet difference 链式调用 | `-` 不能连续用 | `.difference().cloned().collect()` 两次 |

---

## WAL/Recovery 测试策略

| 发现 | 详情 | 来源 |
|------|------|------|
| WAL 记录验证优于重启验证 | TableManager 纯内存，重启后表丢失，改为直接读 WAL 验证 | recovery_e2e_test.rs |
| 独立 WAL 层 benchmark | 直接操作 WALBuffer，不经过 SQL 层 | wal_group_commit_bench.rs |
| tempdir leak 模式 | `std::mem::forget(dir)` 保证 WAL 文件存活 | wal_group_commit_bench.rs |
| AtomicU64 tx_id 分配 | 全局计数器避免跨 iterations 冲突 | wal_group_commit_bench.rs |

---

## 实现技巧

| 技巧 | 详情 | 来源 |
|------|------|------|
| 惰性初始化 | search_all 在首次 next() 调用时执行 | index_scan_all.rs:51-61 |
| MVCC 可见性迭代 | while 循环跳过不可见版本 | index_scan_all.rs:65-86 |
| PhysicalPlan 扩展模式 | enum variant + Node struct + Pipeline + planner match | plan.rs + pipeline.rs |
| JoinConfig 模式 | 8-9个参数组织为单一 struct | join_config.rs |
| Type alias 简化 | CreateExecutorFuture 简化复杂 async 返回类型 | pipeline.rs |
| Mutex 参数注入 | ParameterExpression + Mutex<Value>，clone→inject→rebuild | correlated.rs |
| 双路径执行器 | correlated_params 非空走按行重建，空走快速路径 | semi_join/anti_join |
| 非唯一索引同页多条目 | LeafNode 去掉 DuplicateKey 检查 | btree/node.rs |
| 批量删除从后向前 | delete_by_key matches 从后向前删除 slot | btree/node.rs |
| 两次加载页模式 | 先 page_data() 读找匹配，再 modify_page() 删除 | 页面读写分离 |

---

## 基准测试技巧

| 技巧 | 用途 | 代码位置 |
|------|------|----------|
| 共享 tokio::runtime | 避免 per-iteration 创建 runtime | benches/sqlite_compare.rs |
| RTsqlDirect in-process | 直接调用 API，避免 network overhead | benches/sqlite_compare.rs |
| criterion Throughput | 设置 throughput 更准确测量 | benches/sqlite_compare.rs |
| 减少 sample_size | 慢操作用小 sample_size 加速 | benches/sqlite_compare.rs |

---

## API 路径速查

| API | 用途 | 位置 |
|-----|------|------|
| Database::open(path) | 打开/创建数据库 | database.rs |
| Database::execute_sql(sql) | 执行 SQL 语句 | database.rs |
| Database::close() | 关闭数据库 | database.rs |
| BufferPool::get_page(page_id) | 获取页（两阶段锁） | storage/buffer_pool.rs |
| PageGuard::page_data() | 零拷贝读取页数据 | storage/page_frame.rs |
| PageGuard::modify_page(f) | 修改页数据（自动 dirty） | storage/page_frame.rs |
| SlottedPageRef::new(&[u8]) | 只读零拷贝 slot 访问 | storage/page_format/slotted_page.rs |
| IndexManager::search(key) | Async search | storage/btree/index_manager.rs |
| IndexManager::scan_all() | Async scan all | storage/btree/index_manager.rs |
| BTree::from_root(page_id, loader) | 临时实例（写操作） | storage/btree/btree.rs |
| PlanBuilder::build(stmt) | SQL → PhysicalPlan | parser/planner.rs |
| Pipeline::execute(database, sql) | 执行管道入口 | pipeline.rs |
| inject_correlated_values(plan, values) | 向谓词树注入外层列值 | executor/correlated.rs |
| LeafNodeRef::find_all_matches(key) | 查找所有匹配 key 的 slot | storage/btree/node.rs |
| BTree::search_all(key) | 返回所有匹配 RowId | storage/btree/btree.rs |
| BTree::delete_by_key(key) | 删除所有匹配 entries | storage/btree/btree.rs |
| BTree::delete_exact(key, row_id) | 精确删除 | storage/btree/btree.rs |
| LeafNode::merge_right | 吸收右兄弟 entries | storage/btree/node.rs |
| InternalNode::merge_right | 吸收右兄弟 + 降级 separator | storage/btree/node.rs |
| MergeInfo | {freed_page_id, separator_key, new_root} | storage/btree/btree.rs |
| FileStorage.free_pages | Mutex<Vec<u64>> free-list | storage/file_storage.rs |

## 文件速查

| 文件 | 作用 |
|------|------|
| src/database.rs | Database 协调器 |
| src/pipeline.rs | SQL 执行管道 |
| src/storage/buffer_pool.rs | BufferPool（两阶段锁） |
| src/storage/page_format/slotted_page.rs | SlottedPage + SlottedPageRef + compacting |
| src/storage/btree/index_manager.rs | IndexManager（AtomicPageId + async） |
| src/executor/aggregate.rs | AggregateFunc + AggregateState + AggregateExecutor |
| src/executor/join.rs | JoinExecutor（哈希连接） |
| src/executor/semi_join.rs | SemiJoinExecutorV2 |
| src/executor/anti_join.rs | AntiJoinExecutor |
| src/executor/subquery_eval.rs | SubqueryEvalExecutor |
| src/executor/correlated.rs | inject_correlated_values |
| src/executor/predicate.rs | Predicate/Expression + ParameterExpression |
| src/parser/planner.rs | PlanBuilder（含子查询/关联检测） |

---

## 技巧模式

| 模式 | 描述 | 适用场景 |
|------|------|----------|
| 零拷贝页读取 | page_data() + SlottedPageRef | 只读场景 |
| 零拷贝 BTree | page_data() + LeafNodeRef | BTree 读路径 |
| 两阶段锁 | 读锁→释放→I/O→写锁(double-check) | 缓存加载 |
| AtomicPageId | AtomicU64::load(Acquire) | async 无锁访问 |
| Hash Aggregation | HashMap<Vec<Value>, Vec<AggregateState>> | GROUP BY |
| HAVING 复用 | HavingExecutor 结构同 FilterExecutor | 聚合结果过滤 |
| 临时 BTree 实例 | BTree::from_root() + spawn_blocking | 写操作保持 sync |
| 哈希连接 | 构建侧哈希表 + 探测侧匹配 | INNER JOIN |
| Mutex 参数注入 | ParameterExpression + Mutex<Value> | 相关子查询 |
| 双路径执行器 | correlated_params 非空走按行重建 | SemiJoin/AntiJoin |
| 非唯一索引同页多条目 | LeafNode 去掉 DuplicateKey 检查 | 索引允许重复 key |
| 批量删除从后向前 | delete_by_key 从后向前删除 slot | 批量删除同页多个 slot |
| 两次加载页模式 | 先 page_data() 读找，再 modify_page() 删除 | 页面读写分离 |

## 待探索

| 主题 | 优先级 | 备注 |
|------|--------|------|
| io_uring | 低 | Linux 5.1+，需 tokio-uring |
| jemalloc/mimalloc | 低 | 内存分配器优化 |

<!-- tombstone: learned #01 --> Archived to archive.md §learned #01 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #02 --> Archived to archive.md §learned #02 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #03 --> Archived to archive.md §learned #03 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #04 --> Archived to archive.md §learned #04 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #05 --> Archived to archive.md §learned #05 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #06 --> Archived to archive.md §learned #06 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #07 --> Archived to archive.md §learned #07 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #08 --> Archived to archive.md §learned #08 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #14 --> Archived to archive.md §learned #14 2026-05-24 — 已完成探索项 (WAL Group Commit)
<!-- tombstone: learned #15 --> Archived to archive.md §learned #15 2026-05-24 — 已完成探索项 (B-Tree split/merge)

## 详细踩坑档案

<!-- tombstone: learned #09 --> Archived to archive.md §learned #09 2026-05-24 — 已修复踩坑详细档案，表格行已归档
<!-- tombstone: learned #10 --> Archived to archive.md §learned #10 2026-05-24 — 已修复踩坑详细档案，表格行已归档
<!-- tombstone: learned #11 --> Archived to archive.md §learned #11 2026-05-24 — 已修复踩坑详细档案，表格行已归档
<!-- tombstone: learned #12 --> Archived to archive.md §learned #12 2026-05-24 — 已修复踩坑详细档案，表格行已归档
<!-- tombstone: learned #13 --> Archived to archive.md §learned #13 2026-05-24 — 已修复踩坑详细档案，表格行已归档

### get_subquery_first_column 不支持 SemiJoin/AntiJoin — Simplified

**症状→根因→解决**: 嵌套 IN 子查询 SemiJoin 节点未被处理 → 添加 SemiJoin/AntiJoin 分支 + output_columns → Plan 递归提取需覆盖所有带 output_columns 的节点

### inner_column_index 设计失误 — Simplified

**症状→根因→解决**: CorrelatedParam 用 usize 索引匹配 → 改为 param_name: String 按列名匹配 ParameterExpression → 相关子查询注入首选名称匹配

### gc_test SlottedPage SlotID 失效 — Simplified

**症状→根因→解决**: GC delete_slot + compacting 后物理 SlotID 变化 → 引入 logical_id 解耦（Slot 4B→6B） → 数据页引用用 stable ID，header 修改后必须 serialize
