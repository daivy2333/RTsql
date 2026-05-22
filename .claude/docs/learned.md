# 学习记忆

> 最后更新：2026-05-22（M15 聚合函数与 GROUP BY 完成）

## API 路径速查

| API | 用途 | 位置 |
|-----|------|------|
| Database::open(path) | 打开/创建数据库 | database.rs |
| Database::execute_sql(sql) | 执行 SQL 语句 | database.rs |
| Database::close() | 关闭数据库 | database.rs |
| Database::plan_cache_len() | Plan cache 大小（测试） | database.rs |
| BufferPool::get_page(page_id) | 获取页（两阶段锁） | storage/buffer_pool.rs |
| BufferPool::flush_page(page_id) | 刷脏页 | storage/buffer_pool.rs |
| PageGuard::page() | 克隆页数据（4KB） | storage/page_frame.rs |
| PageGuard::page_data() | 零拷贝读取页数据 | storage/page_frame.rs |
| PageGuard::modify_page(f) | 修改页数据（自动标记 dirty） | storage/page_frame.rs |
| PageGuard::mark_dirty() | 标记脏页 | storage/page_frame.rs |
| SlottedPage::new(&mut page) | 可读写 slot 访问 | storage/page_format/slotted_page.rs |
| SlottedPageRef::new(&[u8]) | 只读零拷贝 slot 访问 | storage/page_format/slotted_page.rs |
| SlottedPage::delete_slot(i) | 删除 slot（compact slots） | storage/page_format/slotted_page.rs |
| TableManager::create_table() | 创建表 | storage/data/table_manager.rs |
| TableManager::get_table() | 获取表元数据 | storage/data/table_manager.rs |
| IndexManager::new(buffer_pool) | 创建 IndexManager | storage/btree/index_manager.rs |
| IndexManager::search(key) | Async search（无 spawn_blocking） | storage/btree/index_manager.rs |
| IndexManager::scan_all() | Async scan all entries | storage/btree/index_manager.rs |
| BTree::search_async(key, loader) | Async BTree search | storage/btree/btree.rs |
| BTree::from_root(page_id, loader) | 临时 BTree 实例（写操作） | storage/btree/btree.rs |
| AsyncPageLoader::load_page(page_id) | Async 加载页 | storage/btree/async_loader.rs |
| WalWriter::append() | 追加 WAL 记录 | wal/writer.rs |
| TransactionManager::begin/commit/rollback | 事务操作 | transaction/manager.rs |
| PlanBuilder::build(sql) | SQL → PhysicalPlan | parser/planner.rs |
| Pipeline::execute(plan) | 执行物理计划 | pipeline.rs |
| Profiling::init_profiling() | 初始化 task-local profiling 数据 | profiling.rs |
| Profiling::record_time(stage, duration) | 记录计时数据 | profiling.rs |
| Profiling::print_timings(total) | 输出计时表格到 stderr | profiling.rs |
| Profiling::is_profiling_enabled() | 检查 RTSQL_PROFILING 环境变量 | profiling.rs |

## 文件速查

| 文件 | 作用 |
|------|------|
| src/database.rs | Database 协调器（所有子系统入口） |
| src/pipeline.rs | SQL 执行管道（parse→plan→execute） |
| src/profiling.rs | Task-local profiling 数据 |
| src/plan_cache.rs | LRU plan cache |
| src/storage/buffer_pool.rs | BufferPool（Clock 淘汰 + 两阶段锁） |
| src/storage/page_frame.rs | PageGuard + PageDataGuard |
| src/storage/data_page.rs | 数据页读写（零拷贝读取） |
| src/storage/page_format/slotted_page.rs | SlottedPage + SlottedPageRef + slot compacting |
| src/storage/btree/btree.rs | B-Tree 核心（async search + from_root） |
| src/storage/btree/async_loader.rs | AsyncPageLoader（直接 async） |
| src/storage/btree/index_manager.rs | IndexManager（AtomicPageId + async） |
| src/executor/join.rs | JoinExecutor（哈希连接） |
| src/executor/aggregate.rs | AggregateFunc + AggregateState + AggregateExecutor |
| src/executor/having.rs | HavingExecutor（HAVING 过滤） |
| src/executor/sort.rs | SortExecutor（ORDER BY） |
| src/executor/limit.rs | LimitExecutor（LIMIT/OFFSET） |
| src/parser/planner.rs | PlanBuilder（含 JOIN 解析） |
| src/parser/ast.rs | AST 辅助函数 |
| src/transaction/manager.rs | TransactionManager |
| src/wal/writer.rs | WalWriter |
| src/wal/recovery.rs | RecoveryManager |
| benches/common/mod.rs | 基准测试共享 helper |
| benches/rtsql_vs_sqlite_single.rs | 精确 RTsql vs SQLite 对比 |

## 踩坑记录

| 问题 | 原因 | 解决 | 预防 |
|------|------|------|------|
| SlottedPage 需要 &mut Page | 写操作需要修改页 | 读操作用 SlottedPageRef（零拷贝） | 区分读写场景 |
| BufferPool 写锁持有期间做 I/O | 阻塞其他协程 | 两阶段锁：释放写锁后再做 I/O | I/O 操作不持锁 |
| std::sync::Mutex 跨 .await | 死锁风险 | PageGuard 不跨 .await 持有 | SAFETY 注释标记 |
| Cargo.toml bench 需要文件存在 | 编译时检查 | 先创建 skeleton 文件 | 添加 bench target 前先建文件 |
| criterion async bench | 需要 tokio runtime | b.to_async(&rt) | 用 Runtime::new() 创建 |
| 并发写 key 冲突 | 多线程写相同主键 | AtomicI64 分配不重叠 key 范围 | 并发 benchmark 用原子计数器 |
| TempDir 在 bench 期间被清理 | TempDir drop 删除目录 | std::mem::forget(dir) | benchmark 中 leak TempDir |
| **SlottedPage.delete_slot 不减少 slot_count** | 只标记删除，留下空洞 | **实现 slot compacting** | 删除操作必须更新 header |
| **RwLock<BTree> 跨 async context** | 无法在 async 中持锁 | **改用 AtomicPageId** | Async 路径避免 std::sync::RwLock |

## 技巧模式

| 模式 | 描述 | 适用场景 |
|------|------|----------|
| 零拷贝页读取 | page_data() + SlottedPageRef 代替 page() + SlottedPage | 只读场景，避免 4KB clone |
| 零拷贝 BTree 读取 | page_data() + LeafNodeRef/InternalNodeRef | BTree 读路径，避免 clone + modify_page |
| SQL Plan 缓存 | LruCache<String, PhysicalPlan>，命中跳过 parse+plan | 相同 SQL 重复执行 |
| 两阶段锁 | 读锁检查→释放→I/O→写锁插入（double-check） | 缓存加载，避免 I/O 期间持锁 |
| PageGuard::modify_page | 闭包修改页数据，自动标记 dirty | 修改页数据 |
| **AtomicPageId 无锁访问** | AtomicU64::load(Ordering::Acquire) | **Async search 路径** |
| **Async search 路径** | AsyncPageLoader + search_from_page_async | **消除 spawn_blocking** |
| **临时 BTree 实例** | BTree::from_root() + spawn_blocking | **写操作保持 sync** |
| 哈希连接 | 构建侧哈希表 + 探测侧逐行匹配 | INNER JOIN |
| 链式 JOIN | Join(Join(A, B), C) 递归结构 | 多表连接 |
| criterion Throughput | Throughput::Elements(n) 标记吞吐量 | 基准测试 |
| AtomicI64 key 分配 | fetch_add 分配不重叠的 key 范围 | 并发写 benchmark |
| **Slot compacting** | 移动后续 slots backward + 减少 slot_count | **删除操作** |

## 待探索

| 主题 | 优先级 | 备注 |
|------|--------|------|
| Prepared Statement 缓存 | 高 | M14 已实现，PK 查询 1.1x 提速（parse+plan 开销小） |
| BTree 零拷贝迁移 | 高 | M14 已实现，PK 查询 1.2x 提速 |
| **Async search 路径** | 高 | **M14 已实现，17x internal + 8x vs SQLite** |
| WAL Group Commit | 中 | M18，INSERT 5-10x 提速 |
| io_uring 替换 | 低 | Linux 5.1+，需 tokio-uring |
| jemalloc/mimalloc | 低 | 内存分配器优化 |
| B-Tree split/merge | 中 | M17 索引优化 |
| 聚合函数 | 中 | M15 |
| 子查询 | 中 | M16 |

### M14 Phase 2 T2 踩坑：SlottedPage.delete_slot 不减少 slot_count 导致二分搜索错误

**症状**：delete key=2 后，search key=3 返回 None（应该返回 Some）
**根因**：SlottedPage.delete_slot 只标记 slot 为 deleted（offset=0, length=0），但**不减少 header.slot_count**
**后果**：
- LeafNodeRef.find_key_position_binary 遇到 deleted slot（get_key 返回 None）
- 二分搜索逻辑错误收缩边界：`hi = mid`
- 最终 search key=3 返回 None

**解决**：实现 slot compacting
```rust
pub fn delete_slot(&mut self, index: usize) -> Result<(), String> {
    // Compact slots: move slots after index backward
    for i in index..(count - 1) {
        let slot_bytes = self.page.data[src_start..src_start + Slot::SIZE].to_vec();
        self.page.data[dst_start..dst_start + Slot::SIZE].copy_from_slice(&slot_bytes);
    }
    // Decrease slot_count
    self.header.slot_count -= 1;
}
```

**预防**：删除操作必须更新 header，不能只标记删除

### M14 Phase 2 T2 踩坑：RwLock<BTree> 无法在 async context 持锁

**症状**：无法在 async search 路径中持有 std::sync::RwLock<BTree>
**根因**：std::sync::RwLock 不能跨 .await 持锁（死锁风险）
**解决**：移除 RwLock<BTree>，改用 AtomicPageId
```rust
pub struct IndexManager {
    root_page_id: AtomicU64,  // 替换 RwLock<BTree>
    sync_loader: Arc<SyncPageLoader>,
    async_loader: AsyncPageLoader,
}
```

**预防**：async 路程避免 std::sync::RwLock/Mutex

### M14 Phase 2 T2 踩坑：search_from_page_async 的 lifetime 问题

**症状**：`impl Future<Output = Result<Option<RowId>>> + Send` 无法捕获 `&self` 的 lifetime
**根因**：返回的 Future 捕获了 `&self` 的 anonymous lifetime，但 bounds 中未标注
**解决**：使用 Pin<Box<dyn Future + Send + 'a>>
```rust
fn search_from_page_async<'a>(
    &'a self,
    page_id: PageId,
    key: &'a Key,
) -> Pin<Box<dyn Future<Output = Result<Option<RowId>>> + Send + 'a>> {
    Box::pin(async move { ... })
}
```

**预防**：async 递归方法需要显式标注 lifetime

### M14 Phase 2 T2 踩坑：criterion benchmark iterations 过多导致超时

**症状**：`cargo bench` 运行超过 10 分钟
**根因**：`for i in 0..100` 生成 100 个独立 criterion case，每个多次采样
**解决**：减少 iterations 为 50，用代表性 case 代替大量独立 case
**预防**：benchmark 避免生成大量独立 case
