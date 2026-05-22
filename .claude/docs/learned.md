# 学习记忆

> 最后更新：2026-05-22

## API 路径速查

| API | 用途 | 位置 |
|-----|------|------|
| Database::open(path) | 打开/创建数据库 | database.rs |
| Database::execute_sql(sql) | 执行 SQL 语句 | database.rs |
| Database::close() | 关闭数据库 | database.rs |
| BufferPool::get_page(page_id) | 获取页（两阶段锁） | storage/buffer_pool.rs |
| BufferPool::flush_page(page_id) | 刷脏页 | storage/buffer_pool.rs |
| PageGuard::page() | 克隆页数据（4KB） | storage/page_frame.rs |
| PageGuard::page_data() | 零拷贝读取页数据 | storage/page_frame.rs |
| PageGuard::modify_page(f) | 修改页数据（自动标记 dirty） | storage/page_frame.rs |
| PageGuard::mark_dirty() | 标记脏页 | storage/page_frame.rs |
| SlottedPage::new(&mut page) | 可读写 slot 访问 | storage/page_format/slotted_page.rs |
| SlottedPageRef::new(&[u8]) | 只读零拷贝 slot 访问 | storage/page_format/slotted_page.rs |
| TableManager::create_table() | 创建表 | storage/data/table_manager.rs |
| TableManager::get_table() | 获取表元数据 | storage/data/table_manager.rs |
| BTree::insert/search/delete | B-Tree 操作 | storage/btree/btree.rs |
| WalWriter::append() | 追加 WAL 记录 | wal/writer.rs |
| TransactionManager::begin/commit/rollback | 事务操作 | transaction/manager.rs |
| PlanBuilder::build(sql) | SQL → PhysicalPlan | parser/planner.rs |
| Pipeline::execute(plan) | 执行物理计划 | pipeline.rs |

## 文件速查

| 文件 | 作用 |
|------|------|
| src/database.rs | Database 协调器（所有子系统入口） |
| src/pipeline.rs | SQL 执行管道（parse→plan→execute） |
| src/storage/buffer_pool.rs | BufferPool（Clock 淘汰 + 两阶段锁） |
| src/storage/page_frame.rs | PageGuard + PageDataGuard |
| src/storage/data_page.rs | 数据页读写（零拷贝读取） |
| src/storage/page_format/slotted_page.rs | SlottedPage + SlottedPageRef |
| src/storage/btree/btree.rs | B-Tree 核心 |
| src/executor/join.rs | JoinExecutor（哈希连接） |
| src/executor/filter.rs | FilterExecutor（WHERE） |
| src/executor/sort.rs | SortExecutor（ORDER BY） |
| src/executor/limit.rs | LimitExecutor（LIMIT/OFFSET） |
| src/parser/planner.rs | PlanBuilder（含 JOIN 解析） |
| src/parser/ast.rs | AST 辅助函数 |
| src/transaction/manager.rs | TransactionManager |
| src/wal/writer.rs | WalWriter |
| src/wal/recovery.rs | RecoveryManager |
| benches/common/mod.rs | 基准测试共享 helper |

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

## 技巧模式

| 模式 | 描述 | 适用场景 |
|------|------|----------|
| 零拷贝页读取 | page_data() + SlottedPageRef 代替 page() + SlottedPage | 只读场景，避免 4KB clone |
| 两阶段锁 | 读锁检查→释放→I/O→写锁插入（double-check） | 缓存加载，避免 I/O 期间持锁 |
| PageGuard::modify_page | 闭包修改页数据，自动标记 dirty | 修改页数据 |
| spawn_blocking + sync BTree | B-Tree 操作在 spawn_blocking 中执行 | 避免阻塞 Tokio 运行时 |
| 哈希连接 | 构建侧哈希表 + 探测侧逐行匹配 | INNER JOIN |
| 链式 JOIN | Join(Join(A, B), C) 递归结构 | 多表连接 |
| criterion Throughput | Throughput::Elements(n) 标记吞吐量 | 基准测试 |
| AtomicI64 key 分配 | fetch_add 分配不重叠的 key 范围 | 并发写 benchmark |

## 待探索

| 主题 | 优先级 | 备注 |
|------|--------|------|
| io_uring 替换 | 低 | Linux 5.1+，需 tokio-uring |
| jemalloc/mimalloc | 低 | 内存分配器优化 |
| B-Tree split/merge | 高 | M16 索引优化 |
| 聚合函数 | 高 | M14 |
| 子查询 | 中 | M15 |