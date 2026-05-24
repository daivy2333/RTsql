# 学习记忆

> 最后更新：2026-05-24（M18-Phase4 B-Tree Merge 完成）

## 2026-05-24 新增（M18-Phase4 B-Tree Merge）

### B-Tree Merge 架构知识

| 发现 | 详情 | 来源 |
|------|------|------|
| Redistribution-first 策略 | 先借后合优于纯 merge，避免 ping-pong merge/split | M18-Phase4 T4 |
| MergeInfo 传播模式 | 新增 `new_root: Option<PageId>` 字段处理 root shrink | M18-Phase4 T5 |
| InternalNode sibling 查找 | `child_index` 定位后，left = pos-1 (leftmost for pos=1)，right = pos+1 | M18-Phase4 T4 |
| separator_key 匹配 | merge 时 separator_key = 被吸收页的 first key，父节点据此找到 slot 删除 | M18-Phase4 T4 |
| free-list 实现 | `Mutex<Vec<u64>>` + allocate_page 先 pop + free_page 先 push 后 zero | M18-Phase4 T3 |
| PageGuard drop 时机 | `modify_page` 闭包内修改，闭包返回后 guard 自动释放 | M18-Phase4 T4 |

### API 速查

| API | 文件 | 行 | 用途 |
|-----|------|-----|------|
| `LeafNode::merge_right` | `src/storage/btree/node.rs` | ~380 | 吸收右兄弟 entries，返回 LeafMergeResult |
| `LeafNode::redistribute_right` | `src/storage/btree/node.rs` | ~440 | 从右兄弟借 entries 平衡分布 |
| `LeafNode::can_merge_with` | `src/storage/btree/node.rs` | ~374 | 检查合并后是否超 page 容量 |
| `InternalNode::remove_separator` | `src/storage/btree/node.rs` | ~730 | 删除指定索引的 separator |
| `InternalNode::merge_right` | `src/storage/btree/node.rs` | ~760 | 吸收右兄弟 + 降级 parent separator |
| `InternalNode::can_merge_with` | `src/storage/btree/node.rs` | ~810 | 检查合并后是否超 page 容量 |
| `MergeInfo` | `src/storage/btree/btree.rs` | ~21 | `{freed_page_id, separator_key, new_root}` |
| `SyncPageLoader::free_page` | `src/storage/btree/sync_loader.rs` | ~33 | 释放页（同步包装） |
| `FileStorage.free_pages` | `src/storage/file_storage.rs` | ~18 | `Mutex<Vec<u64>>` free-list |
| `BTree::delete` | `src/storage/btree/btree.rs` | ~401 | 返回 `Result<Option<PageId>>`（root shrink 时返回新 root） |

### 踩坑记录

#### delete_by_key 并发 merge 位置偏移
- **症状**：`delete_by_key` 遍历多个子节点时，第一个 merge 改变了父节点结构，后续子节点位置偏移导致 "no siblings" 错误
- **根因**：子节点列表在 merge 前收集，merge 后 separator 被删除，子节点索引失效
- **解决**：改用 `&mut self` 签名 + root_page_id 现场更新，每次 delete 从新 root 遍历

#### merge 容量溢出
- **症状**：merge_leaves 时 `PageFull` 错误
- **根因**：`min_keys=48`，leaf 容量=92，合并 47+48=95 > 92 溢出
- **解决**：redistribution-first 避免了绝大多数溢出场景；极端情况下 `can_merge_with` 检查拦截

### WAL Group Commit Benchmark 技巧

| 发现 | 详情 | 来源 |
|------|------|------|
| **独立 WAL 层 benchmark** | 直接操作 WALBuffer，不经过 SQL 层，精确度量 WAL 性能无噪声 | benches/wal_group_commit_bench.rs |
| **tempdir leak 模式** | `std::mem::forget(dir)` 保证 WAL 文件在 benchmark 期间存活 | benches/wal_group_commit_bench.rs |
| **AtomicU64 tx_id 分配** | 全局 AtomicU64 计数器避免跨 criterion iterations 的 tx_id 冲突 | benches/wal_group_commit_bench.rs |
| **三组 benchmark** | wal_baseline(capacity=1 逐条fsync) / wal_group_commit(并发1-32) / wal_capacity_impact(capacity 1/10/100) | benches/wal_group_commit_bench.rs |
| **benchmark 参数** | sample_size=50, measurement_time=10s, baseline 1000条, 并发每线程 200 条 | benches/wal_group_commit_bench.rs |

---

### Logical Row ID 修复要点

| 发现 | 详情 | 来源 |
|------|------|------|
| **Slot 扩展为 6B** | `Slot { logical_id: u16, offset: u16, length: u16 }`，从 4B 扩展为 6B | slotted_page.rs |
| **next_logical_id 分配** | SlottedPageHeader 新增 next_logical_id: u16，每次 add_slot 递增，永不回收 | slotted_page.rs:36 |
| **header padding 调整** | _padding 从 5B 减为 3B，总 header 仍 16B（next_logical_id 占 2B） | slotted_page.rs:24 |
| **add_slot 返回 (u16, usize)** | 返回 (logical_id, slot_index)，调用方需适配 | slotted_page.rs:202 |
| **delete_slot 必须序列化 header** | `slot_count -= 1` 后必须 `serialize` 回 page.data，否则 BufferPool 缓存提供过期 slot_count | slotted_page.rs:272-273 |
| **get_slot_by_logical_id** | 线性扫描 slot 数组匹配 logical_id，返回 (Slot, slot_index) | slotted_page.rs:113-122, 183-192 |
| **data_page.rs 全改 logical_id** | read/write/update/delete 全部使用 logical_id 查找，不再用物理 slot_index | data_page.rs |
| **B-Tree 只需适配返回类型** | B-Tree 不需要 logical_id 映射（用物理 slot_index 排序），只需适配 add_slot 返回值 | btree/node.rs |
| **RowId.slot_id 语义变更** | slot_id 现在是 logical_id（稳定跨 compact），不再是物理 slot_index | row_id.rs:8 |

### 关键踩坑：delete_slot 不序列化 header

| 问题 | 原因 | 解决 | 预防 |
|------|------|------|------|
| delete_slot 后 BufferPool 读到过期 slot_count | `self.header.slot_count -= 1` 只修改了内存 header，未 serialize 回 page.data | 在 slot_count 修改后立即调用 `self.header.serialize(&mut self.page.data[..SIZE])` | **SlottedPage 任何 header 修改后必须 serialize 回 page.data** |

---

## 2026-05-24 新增（M18 Phase3 WAL 集成 — T4/T5/T6/T8）

### Executor 隐式事务包装

| 发现 | 详情 | 来源 |
|------|------|------|
| **隐式事务模式** | 每个 Insert/Update/Delete 语句在 executor 的 next() 中自动写 BeginTxn → 数据记录 → CommitTxn WAL 记录，无需 pipeline 层显式事务管理 | executor/insert.rs, update.rs, delete.rs |
| **append_commit_and_wait 调用** | CommitTxn 写入后必须调用 wal_buffer.append_commit_and_wait(tx_id) 确保 WAL 持久化确认（Group Commit） | executor/insert.rs:105-111 |
| **Pipeline tx_id 仍为 0** | pipeline.rs 传入 tx_id=0 给 executor，但 executor WAL 写入使用 TransactionManager.begin() 分配的真实 tx_id 不可用——隐式事务包装在 executor 内部完成，不需要 pipeline 传 tx_id | pipeline.rs:326 |
| **TableManager 纯内存限制** | 表定义不持久化到磁盘，重启后丢失。恢复时 redo 需要表存在才能写入数据页，但 get_table 失败时 redo 静默跳过 | storage/table_manager.rs |
| **BufferPool::mark_tx_aborted 是 stub** | 当前实现为空函数 `Ok(())`，未遍历 SlottedPage 标记 uncommitted tuple。MVCC 可见性依赖 VersionHeader 的 create_tx_id 在 active_tx_ids 中 | storage/buffer_pool.rs:212 |

### RecoveryManager 数据重放踩坑

| 发现 | 详情 | 来源 |
|------|------|------|
| **RecoveryManager::recover 返回元组** | 基础版 recover 返回 `(HashSet<u64>, HashSet<u64>)`（committed, aborted），full_recover 返回 RecoveryResult struct | wal/recovery.rs |
| **collapsible_if Clippy 规则** | 嵌套 if `committed.contains(&tx_id) { if redo.is_ok() { ... } }` 应合并为 `committed.contains(&tx_id) && redo.is_ok() { ... }` | wal/recovery.rs:115 |
| **RecoveryManager 需要表才能 redo** | redo_record 中 Insert/Update 需要 `table_manager.get_table()` 获取 TableMeta，表不存在则静默跳过。这意味着表定义持久化是实现完整恢复的前提 | wal/recovery.rs redo_record |
| **HashSet difference 需要链式调用** | `&all_tx_ids - &committed_tx_ids - &aborted_tx_ids` 不能直接用 `-`，需要 `all_tx_ids.difference(&committed).cloned().collect()` 两次链式调用 | wal/recovery.rs:102-109 |

### E2E 崩溃恢复测试策略

| 发现 | 详情 | 来源 |
|------|------|------|
| **WAL 记录验证优于重启验证** | 由于 TableManager 纯内存，重启后表丢失，无法验证数据恢复。改为直接读取 WAL 文件验证记录完整性 | tests/recovery_e2e_test.rs |
| **RecoveryManager::recover 分类验证** | 使用基础版 recover（仅分类事务）验证 committed/aborted 事务被正确识别，不需要 BufferPool/TableManager | tests/recovery_e2e_test.rs |
| **Database 重开后重建表** | 重启后重建表可以继续执行新操作，但旧数据的索引丢失（IndexManager 也是纯内存） | tests/recovery_e2e_test.rs test_data_pages_survive_restart |

---

## 2026-05-23 新增（M18 Phase3 T1/T2）

### WAL 记录扩展 + CRC32 + LSN

| 发现 | 详情 | 来源 |
|------|------|------|
| **WalRecord 新变体** | BeginTxn{tx_id}/CommitTxn{tx_id,timestamp}/AbortTxn{tx_id}，类型码 0x07/0x08/0x09 | wal/record.rs |
| **LSN + CRC32 序列化** | `[lsn:8B][type:1B][len:4B][body:var][crc32:4B]`，CRC 对 lsn+type+len+body 计算 | wal/record.rs serialize_with_lsn |
| **WalWriter::write_batch** | 批量写入多条记录，最后一次 fsync | wal/writer.rs |
| **WalReader 格式自动检测** | 检查 byte[8] 是否有效 RecordType + byte[0] 是否无效，区分新旧格式 | wal/reader.rs |

### WALBuffer + Group Commit

| 发现 | 详情 | 来源 |
|------|------|------|
| **WALBuffer 核心** | Mutex<Vec<(u64,WalRecord)>> 缓冲 + AtomicU64 LSN + Notify 信号 + 后台 tokio task | wal/buffer.rs |
| **Group Commit 机制** | append_commit_and_wait 注册 Notify 等待 → flush_notify 唤醒后台 → do_flush 批量写入 + fsync → 通知所有等待者 | wal/buffer.rs |
| **tokio::select! 双监听** | 后台 task 同时监听 flush_notify 和定时器（flush_interval_ms） | wal/buffer.rs flush_loop |
| **std::sync::Mutex for flush_handle** | tokio::sync::Mutex 不能在 runtime 外调用 blocking_lock()，JoinHandle 存储用 std::sync::Mutex | wal/buffer.rs |
| **Database 添加 wal_buffer** | Arc<WALBuffer> 字段，open() 中初始化并 start_flush_loop() | database.rs |

### WAL 集成踩坑（gc_test bug）

| 发现 | 详情 | 来源 |
|------|------|------|
| **gc_test panic** | GC 删除 tuple 后 SlottedPage compacting 改变 SlotID，但版本链/索引仍持有旧 row_id → read_tuple_from_data_page 访问空 slot → slice 越界 panic | tests/gc_test.rs + data_page.rs:82 |
| **PoisonError 连锁** | 第一个 panic poison BufferPool Mutex → 第二个测试 unwrap() 触发 PoisonError panic → 析构中再 panic → abort | page_frame.rs:95 |

---

## 2026-05-23 新增（M18 Phase2）

### IndexScanAllExecutor 实现技巧

| 发现 | 详情 | 来源 |
|------|------|------|
| **惰性初始化模式** | search_all 在首次 next() 调用时执行，避免不必要的查询开销 | executor/index_scan_all.rs:51-61 |
| **MVCC 可见性迭代** | while 循环跳过不可见版本，继续下一个 row_id，符合 Executor 逐行返回约定 | executor/index_scan_all.rs:65-86 |
| **非唯一索引测试方法** | 使用 write_tuple_to_data_page + IndexManager.insert 直接创建重复键数据，绕过 InsertExecutor 的 DuplicateKey 检查 | tests/executor_test.rs:1081-1103 |
| **PhysicalPlan 扩展模式** | 新增 enum variant + Node struct + Pipeline match 分支 + correlated.rs/planner.rs match 分支，完整集成链路 | executor/plan.rs + pipeline.rs + correlated.rs + planner.rs |

---

## 2026-05-23 新增（M18 Phase1）

### ClippyWarnings清理技巧

| 发现 | 详情 | 来源 |
|------|------|------|
| **JoinConfig 模式** | 8-9个参数组织为单一 struct，解决 too_many_arguments | executor/join_config.rs |
| **Type alias 简化** | CreateExecutorFuture<'a> 简化复杂 async 返回类型 | pipeline.rs |
| **#[allow] 合理设计** | await_holding_lock（两阶段锁）+ module_inception（标准命名） | buffer_pool.rs, btree/mod.rs |
| **务实策略** | 不追求零 warnings，保留合理设计 + 明确注释 | architecture.md ADR-005 |

---

## 2026-05-23 新增（M17.5）

### 存储架构发现

| 发现 | 详情 | 来源 |
|------|------|------|
| **固定 Key 空间开销** | Key 固定 32 bytes，短 Key 浪费 ~28B，长 Key 限制 32B | storage/page_format/key.rs |
| **两层分离索引** | 索引页 + 数据页（vs SQLite 聚簇索引），灵活性高但空间开销大 | storage/btree/ + storage/data_page.rs |
| **SlottedPage overhead** | 每个 entry 多 4B Slot（offset + length），页填充率 50-70% | storage/page_format/slotted_page.rs |
| **二进制序列化格式** | Tag byte + 固定长度（Int=9B, Float=9B），比 varint 多 ~6-7B | storage/page_format/tuple.rs |

### 基准测试发现

| 发现 | 数据 | 结论 |
|------|------|------|
| **INSERT 性能惊人** | RTsql 693µs vs SQLite 232ms (332x faster) ⚡ | 异步 I/O + MVCC 无锁写优势巨大 |
| **PK lookup 性能** | RTsql 1.05µs vs SQLite 5.88µs (5.6x faster) ⚡ | 零拷贝 + AtomicPageId 有效 |
| **Full Scan 较慢** | RTsql 327µs vs SQLite 80µs (4x slower) | SQLite 扫描优化成熟，后续可并行化 |
| **文件大小差异** | RTsql 1.4MB vs SQLite 217KB (6.5x larger) | 合理权衡：空间换灵活性 |

### Rust 基准测试技巧

| 技巧 | 用途 | 代码位置 |
|------|------|----------|
| **共享 tokio::runtime** | 避免 per-iteration 创建 runtime（开销大） | benches/sqlite_compare.rs |
| **RTsqlDirect in-process** | 直接调用 API，避免 network overhead | benches/sqlite_compare.rs |
| **criterion Throughput** | 设置 throughput 更准确的测量 | benches/sqlite_compare.rs |
| **减少 sample_size** | 慢操作用小 sample_size 加速完成 | benches/sqlite_compare.rs |

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
| SlottedPage::delete_slot(i) | 删除 slot（含 compacting） | storage/page_format/slotted_page.rs |
| IndexManager::search(key) | Async search（无 spawn_blocking） | storage/btree/index_manager.rs |
| IndexManager::scan_all() | Async scan all | storage/btree/index_manager.rs |
| BTree::from_root(page_id, loader) | 临时实例（写操作） | storage/btree/btree.rs |
| PlanBuilder::build(stmt) | SQL → PhysicalPlan | parser/planner.rs |
| Pipeline::execute(database, sql) | 执行管道入口 | pipeline.rs |
| inject_correlated_values(plan, values) | 向谓词树注入外层列值 | executor/correlated.rs |
| ParameterExpression::new(name) | 创建可注入参数占位符 | executor/predicate.rs |
| predicate.inject_parameters(values) | 递归注入参数值到谓词树 | executor/predicate.rs |
| **LeafNodeRef::find_all_matches(key)** | 查找所有匹配 key 的 slot 索引 | storage/btree/node.rs |
| **BTree::search_all(key)** | 返回所有匹配 RowId（非唯一索引） | storage/btree/btree.rs |
| **BTree::delete_by_key(key)** | 删除所有匹配 entries（返回数量） | storage/btree/btree.rs |
| **BTree::delete_exact(key, row_id)** | 精确删除（key + RowId 匹配） | storage/btree/btree.rs |
| **InternalNode::insert_separator(key, right_child)** | 插入分隔符（用于 split） | storage/btree/node.rs |
| **LeafNode::delete_slot(index)** | 按索引删除 slot（公开方法） | storage/btree/node.rs |

## 文件速查

| 文件 | 作用 |
|------|------|
| src/database.rs | Database 协调器 |
| src/pipeline.rs | SQL 执行管道 |
| src/storage/buffer_pool.rs | BufferPool（两阶段锁） |
| src/storage/page_format/slotted_page.rs | SlottedPage + SlottedPageRef + compacting |
| src/storage/btree/index_manager.rs | IndexManager（AtomicPageId + async） |
| src/executor/aggregate.rs | AggregateFunc + AggregateState + AggregateExecutor |
| src/executor/having.rs | HavingExecutor |
| src/executor/join.rs | JoinExecutor（哈希连接） |
| src/executor/semi_join.rs | SemiJoinExecutorV2（独立+关联双路径） |
| src/executor/anti_join.rs | AntiJoinExecutor（独立+关联双路径） |
| src/executor/subquery_eval.rs | SubqueryEvalExecutor（独立+关联双路径） |
| src/executor/correlated.rs | inject_correlated_values 注入函数 |
| src/executor/predicate.rs | Predicate/Expression trait + ParameterExpression |
| src/parser/planner.rs | PlanBuilder（含子查询/关联检测） |
| src/parser/ast.rs | extract_columns/extract_qualified_columns |

## 踩坑记录

| 问题 | 原因 | 解决 | 预防 |
|------|------|------|------|
| AggregateExecutor::new partial move | Pipeline 解构 AggregateNode | 改为传单个字段 | 避免结构体解构导致 partial move |
| **get_subquery_first_column 不支持 SemiJoin** | 嵌套 IN 子查询 Plan 是 SemiJoin 节点 | 添加 SemiJoin/AntiJoin 分支，使用 output_columns | Plan 递归提取函数需覆盖所有带 output_columns 的节点 |
| **多层检测永远不触发** | get_subquery_first_column 在 extract_correlated_params 之前调用，且未处理 SemiJoin | 修复 get_subquery_first_column 让流程走到 extract_correlated_params | 检查顺序敏感的函数需确保上游验证不影响下游逻辑 |
| **inner_column_index 设计错误** | 设为 usize 表示"替换位置索引"，但 ColumnExpression 在谓词树非输出列 | 改为 param_name: String，按列名匹配 ParameterExpression | 设计相关子查询注入时用名称匹配而非索引匹配 |
| **gc_test SlottedPage SlotID 失效** | ✅ 已修复 | 引入 logical_id 解耦 RowId.slot_id 与物理 slot_index，Slot 从 4B 扩展为 6B | GC 删除操作不影响版本链/索引中的 row_id 引用 |

<!-- tombstone: learned #01 --> Archived to archive.md §learned #01 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #02 --> Archived to archive.md §learned #02 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #03 --> Archived to archive.md §learned #03 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #04 --> Archived to archive.md §learned #04 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #05 --> Archived to archive.md §learned #05 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #06 --> Archived to archive.md §learned #06 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #07 --> Archived to archive.md §learned #07 2026-05-24 — 已修复踩坑 >30d
<!-- tombstone: learned #08 --> Archived to archive.md §learned #08 2026-05-24 — 已修复踩坑 >30d

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
| **Mutex 参数注入** | ParameterExpression 携带 Mutex<Value>，clone plan → inject → rebuild executor | 相关子查询外层值注入 |
| **双路径执行器** | correlated_params 非空走按行重建路径，空走预构建快速路径 | SemiJoin/AntiJoin/SubqueryEval |
| **inner_table_names 上下文** | PlanBuilder 字段，build_query 前设置子查询表列表，build_expression 据此创建 ParameterExpression | 外部引用检测 |
| **非唯一索引同页多条目** | LeafNode 去掉 DuplicateKey 检查，find_all_matches 遍历所有匹配 slot | 索引允许重复 key |
| **批量删除从后向前** | delete_by_key matches 从后向前删除 slot，避免索引错位 | 批量删除同页多个 slot |
| **两次加载页模式** | 先 page_data() 读取找匹配，再 modify_page() 删除，避免闭包借用冲突 | 页面读写分离操作 |

## 待探索

| 主题 | 优先级 | 备注 |
|------|--------|------|
| io_uring | 低 | Linux 5.1+，需 tokio-uring |
| jemalloc/mimalloc | 低 | 内存分配器优化 |
<!-- tombstone: learned #14 --> Archived to archive.md §learned #14 2026-05-24 — 已完成探索项 (WAL Group Commit)
<!-- tombstone: learned #15 --> Archived to archive.md §learned #15 2026-05-24 — 已完成探索项 (B-Tree split/merge)

## 详细踩坑档案

<!-- tombstone: learned #09 --> Archived to archive.md §learned #09 2026-05-24 — 已修复踩坑详细档案，表格行已归档
<!-- tombstone: learned #10 --> Archived to archive.md §learned #10 2026-05-24 — 已修复踩坑详细档案，表格行已归档
<!-- tombstone: learned #11 --> Archived to archive.md §learned #11 2026-05-24 — 已修复踩坑详细档案，表格行已归档
<!-- tombstone: learned #12 --> Archived to archive.md §learned #12 2026-05-24 — 已修复踩坑详细档案，表格行已归档
<!-- tombstone: learned #13 --> Archived to archive.md §learned #13 2026-05-24 — 已修复踩坑详细档案，表格行已归档

### get_subquery_first_column 不支持 SemiJoin/AntiJoin — Simplified

**症状→根因→解决**: 嵌套 IN 子查询 SemiJoin 节点未被 get_subquery_first_column 处理 → 添加 SemiJoin/AntiJoin 分支 + output_columns → Plan 递归提取需覆盖所有带 output_columns 的节点

### inner_column_index 设计失误 — Simplified

**症状→根因→解决**: CorrelatedParam 用 usize 級引匹配，但 ColumnExpression 不在输出列 → 改为 param_name: String 按列名匹配 ParameterExpression → 相关子查询注入首选名称匹配

### gc_test SlottedPage SlotID 失效 — Simplified

**症状→根因→解决**: GC delete_slot + compacting 后物理 SlotID 变化，版本链引用旧 slot_id → slice 越界 → 引入 logical_id 解耦（Slot 4B→6B，header 新增 next_logical_id） → 数据页引用用 stable ID，header 修改后必须 serialize
