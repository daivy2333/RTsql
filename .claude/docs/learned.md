# 学习记忆

> 最后更新：2026-05-22 (M12 完成：INNER JOIN 哈希连接)
> 记录探索发现、API路径、技巧、踩坑经验

---

## 记录触发条件

以下情况应主动更新此文档：
- 发现新的 API 调用方式或路径
- 找到某个功能的关键文件位置
- 解决一个棘手问题后的经验
- 发现某个陷阱或坑
- 学到一个有用的技巧或模式
- 理清了模块间的依赖关系

---

## API & 接口路径

| 名称 | 路径/调用方式 | 用途 | 发现时间 |
|------|--------------|------|----------|
| Tokio 运行时 | `#[tokio::main]` 或 `tokio::runtime::Runtime::new()` | 启动异步运行时 | 2026-05-20 |
| spawn_blocking | `tokio::task::spawn_blocking(|| {...})` | CPU密集型操作隔离 | 2026-05-20 |
| Handle::current() | `tokio::runtime::Handle::current()` | 在异步上下文中获取 runtime handle | 2026-05-20 |
| Handle::block_on | `runtime.block_on(async_future)` | 在同步代码中执行异步操作（spawn_blocking 内安全） | 2026-05-20 |
| spawn_blocking + Mutex | `Arc<Mutex<T>>` + spawn_blocking | 在阻塞线程池中使用同步 Mutex（不阻塞异步运行时） | 2026-05-20 |
| AtomicU64 事务ID | `std::sync::atomic::AtomicU64` + `fetch_add` | 无锁分配全局唯一事务 ID | 2026-05-20 |
| tokio::sync::Mutex | `Arc<Mutex<()>>` + `lock().await` | 异步行锁（不阻塞物理线程） | 2026-05-20 |
| sqlparser::parse_sql | `Parser::parse_sql(&GenericDialect, sql)` | 解析 SQL 字符串为 AST | 2026-05-20 |
| sqlparser AST | `sqlparser::ast::{Statement, Query, Select, Expr}` | SQL AST 类型 | 2026-05-20 |
| Value.to_key() | `Value::Int(n).to_key()` | Int 值转为 Key（用于索引查找） | 2026-05-20 |
| Executor trait | `#[async_trait] trait Executor { async fn next(&mut self) -> Result<Option<ExecResult>>; }` | 异步迭代器接口 | 2026-05-20 |
| ExecResult enum | `ExecResult::RowId(RowId) / AffectedRows(u64) / NotImplemented` | 执行结果统一类型 | 2026-05-20 |
| async_trait macro | `#[async_trait::async_trait] impl Executor for X` | 为 trait 提供 Send bounds | 2026-05-20 |
| tokio::net::TcpListener | `TcpListener::bind(addr).await` | TCP 监听 | 2026-05-20 |
| tokio::spawn | `tokio::spawn(async move { handler.handle(stream) })` | 每连接一协程 | 2026-05-20 |
| CancellationToken | `tokio_util::sync::CancellationToken` | Graceful shutdown | 2026-05-20 |
| tokio::select! | `tokio::select! { accept => ..., shutdown => ... }` | 多事件监听 | 2026-05-20 |
| Protocol trait | `#[async_trait] trait Protocol { async fn parse_request/write_response }` | 协议抽象 | 2026-05-20 |
| JSON 帧协议 | 消息以 `\n` 结尾，serde_json 序列化 | 简单帧协议 | 2026-05-20 |
| tokio io-util feature | `tokio = { features = ["io-util"] }` | AsyncReadExt/AsyncWriteExt | 2026-05-20 |
| Database::open | `Database::open(&path).await?` | 打开数据库，初始化所有子系统 | 2026-05-20 |
| Database::execute_sql | `database.execute_sql(sql).await` → `Response` | SQL 执行管道入口 | 2026-05-20 |
| Database::create_table | `database.create_table(name, columns, pk).await` | 创建表（委托 TableManager） | 2026-05-20 |
| Pipeline::execute | `pipeline::execute(database, sql).await` → `Response` | parse→plan→execute→collect | 2026-05-20 |
| write_tuple_to_data_page | `write_tuple_to_data_page(bp, meta, vh, bytes).await?` → `RowId` | 写 Tuple 到数据页（页满自动分配） | 2026-05-20 |
| read_tuple_from_data_page | `read_tuple_from_data_page(bp, row_id).await?` → `(VersionHeader, Vec<u8>)` | 从数据页读 Tuple | 2026-05-20 |
| serialize_tuple | `serialize_tuple(values, schema, &mut buf)?` → `usize` | Int/String/Float/Bool/Null 二进制序列化 | 2026-05-20 |
| deserialize_tuple | `deserialize_tuple(bytes, schema)?` → `Vec<Value>` | 按 schema 反序列化 Tuple | 2026-05-20 |
| compute_tuple_size | `compute_tuple_size(values, schema)` → `usize` | 预计算 Tuple 序列化大小 | 2026-05-20 |
| BTree::scan_all | `btree.scan_all()?` → `Vec<(Key, RowId)>` | 全表扫描（遍历所有 LeafNode） | 2026-05-20 |
| IndexManager::scan_all | `index_manager.scan_all().await?` → `Vec<(Vec<u8>, RowId)>` | async 全扫描包装 | 2026-05-20 |
| TableManager::create_table | `table_mgr.create_table(name, cols, pk).await?` | 注册表元数据 + 分配数据页 + 创建 IndexManager | 2026-05-20 |
| TableManager::get_table | `table_mgr.get_table(name).await?` → `Arc<TableMeta>` | 获取表元数据 | 2026-05-20 |
| TableManager::drop_table | `table_mgr.drop_table(name).await?` | 删除表元数据（物理页删除推迟） | 2026-05-20 |
| VersionHeader::with_next_version | `vh.with_next_version(old_row_id)` → `Self` | 版本链链接到前一版本 | 2026-05-20 |
| Snapshot::is_visible | `snapshot.is_visible(create_id, commit_id)` → `bool` | MVCC 可见性判断（3 规则） | 2026-05-20 |
| Snapshot::is_visible_self | `snapshot.is_visible_self(create_id, commit_id)` → `bool` | 自身未提交写可见 | 2026-05-20 |
| value_to_json | `value_to_json(value)` → `serde_json::Value` | executor::Value → JSON 转换 | 2026-05-20 |
| Predicate trait | `trait Predicate: Send + Sync { fn evaluate(&self, row: &[Value]) -> Result<bool>; }` | WHERE 条件求值 | 2026-05-20 |
| Expression trait | `trait Expression: Send + Sync { fn evaluate(&self, row: &[Value]) -> Result<Value>; }` | 表达式求值 | 2026-05-20 |
| ComparisonPredicate | `ComparisonPredicate { left: ExpressionRef, op: ComparisonOp, right: ExpressionRef }` | 比较操作（Eq/Ne/Gt/Lt/Ge/Le） | 2026-05-20 |
| LogicalPredicate | `LogicalPredicate { left: PredicateRef, op: LogicalOp, right: PredicateRef }` | 逻辑操作（And/Or） | 2026-05-20 |
| ColumnExpression | `ColumnExpression { column_name: String, column_index: usize }` | 列引用表达式 | 2026-05-20 |
| ConstantExpression | `ConstantExpression { value: Value }` | 常量表达式 | 2026-05-20 |
| Value::equals | `value.equals(&other)` → `bool` | 跨类型比较（Int vs Float） | 2026-05-20 |
| Value::gt/lt/ge/le | `value.gt(&other)` → `Result<bool, ValueError>` | 比较操作（支持跨类型） | 2026-05-20 |
| Value::as_float | `value.as_float()` → `Result<f64, ValueError>` | 类型转换（Int→Float） | 2026-05-20 |
| Value::as_bool | `value.as_bool()` → `Result<bool, ValueError>` | 类型转换（Int→Bool） | 2026-05-20 |
| PlanBuilder::build_where | `builder.build_where(expr, schema)` → `PredicateRef` | WHERE 解析 | 2026-05-20 |
| PlanBuilder::build_expression | `builder.build_expression(expr, schema)` → `ExpressionRef` | 表达式解析 | 2026-05-20 |
| PlanBuilder::build_create_table | `builder.build_create_table(name, columns, constraints)` → `PhysicalPlan` | CREATE TABLE 解析 | 2026-05-20 |
| PlanBuilder::build_drop_table | `builder.build_drop_table(names, if_exists)` → `PhysicalPlan` | DROP TABLE 解析 | 2026-05-20 |
| FilterExecutor | `FilterExecutor::new(input_executor, predicate)` | WHERE 过滤执行 | 2026-05-20 |
| CreateTableExecutor | `CreateTableExecutor::new(plan, database)` | CREATE TABLE 执行 | 2026-05-20 |
| DropTableExecutor | `DropTableExecutor::new(plan, database)` | DROP TABLE 执行 | 2026-05-20 |
| ColumnSchema | `ColumnSchema { name, data_type, not_null, unique, default_value }` | 列定义（存储层） | 2026-05-20 |
| ColumnDef::to_schema_column | `column_def.to_schema_column()` → `ColumnSchema` | executor → storage 类型转换 | 2026-05-20 |
| SortExecutor | `SortExecutor::new(input, order_by, columns)` | ORDER BY 排序执行（内存排序） | 2026-05-21 |
| LimitExecutor | `LimitExecutor::new(input, limit, offset)` | LIMIT/OFFSET 分页执行 | 2026-05-21 |
| OrderByColumn | `OrderByColumn { column: String, asc: bool }` | 排序列定义（ASC/DESC） | 2026-05-21 |
| compare_values | `compare_values(&a, &b)` → `Ordering` | Value 比较（支持跨类型 Int/Float） | 2026-05-21 |
| Vec::sort_unstable_by | `rows.sort_unstable_by(|a, b| compare_rows(a, b))` | 高效内存排序（比 sort 快） | 2026-05-21 |
| extract_column_name | `extract_column_name(&expr)` → `String` | ORDER BY 列名提取 | 2026-05-21 |
| parse_limit_value | `parse_limit_value(&expr)` → `usize` | LIMIT 常量解析 | 2026-05-21 |
| parse_offset_value | `parse_offset_value(&expr)` → `usize` | OFFSET 常量解析 | 2026-05-21 |
| TransactionManager.record_version | `tx_manager.record_version(tx_id, row_id).await` | 记录事务创建的版本 | 2026-05-21 |
| TransactionManager.get_tx_versions | `tx_manager.get_tx_versions(tx_id).await` → `HashSet<RowId>` | 获取事务的所有版本（测试） | 2026-05-21 |
| TransactionManager.commit_mark_versions | `tx_manager.commit_mark_versions(tx_id, bp).await` | Commit 时标记所有版本 | 2026-05-21 |
| TransactionManager.abort_cleanup_versions | `tx_manager.abort_cleanup_versions(tx_id, bp, meta).await` | Abort 时清理未提交版本 | 2026-05-21 |
| BufferPool.find_visible_version | `bp.find_visible_version(row_id, snapshot).await` → `Option<Vec<u8>>` | 版本链遍历，找第一个可见版本 | 2026-05-21 |
| BufferPool.read_version_header | `bp.read_version_header(row_id).await` → `VersionHeader` | 仅读取版本头 | 2026-05-21 |
| BufferPool.write_commit_tx_id | `bp.write_commit_tx_id(row_id, tx_id).await` | 设置版本的 commit_tx_id | 2026-05-21 |
| IndexManager.find_key_by_row_id | `index_mgr.find_key_by_row_id(row_id).await` → `Option<Vec<u8>>` | 反向查询：row_id → key | 2026-05-21 |
| update_version_header_in_data_page | `update_version_header_in_data_page(bp, row_id, header, bytes).await` | 更新版本头（不改变 tuple） | 2026-05-21 |
| delete_tuple_from_data_page | `delete_tuple_from_data_page(bp, row_id).await` | 删除数据页条目（GC） | 2026-05-21 |
| TableMeta.gc_table | `table_meta.gc_table(bp).await` → `usize` | 可选 GC，清理旧版本 | 2026-05-21 |
| WalRecord::serialize | `record.serialize()` → `Vec<u8>` | WAL 记录序列化（type+len+data） | 2026-05-21 |
| WalRecord::deserialize | `WalRecord::deserialize(&buf)` → `Result<(Self, usize), WalError>` | WAL 记录反序列化 | 2026-05-21 |
| WalWriter::open | `WalWriter::open(db_path)` → `Result<Self, WalError>` | 打开/创建 WAL 文件 | 2026-05-21 |
| WalWriter::write_record | `writer.write_record(record).await` → `Result<u64, WalError>` | 异步写入 WAL 记录，返回 LSN | 2026-05-21 |
| WalWriter::fsync | `writer.fsync().await` → `Result<(), WalError>` | 异步 fsync WAL 文件 | 2026-05-21 |
| WalWriter::truncate_to | `writer.truncate_to(lsn).await` → `Result<(), WalError>` | 截断 WAL 到指定 LSN | 2026-05-21 |
| WalWriter::get_current_lsn | `writer.get_current_lsn().await` → `Result<u64, WalError>` | 获取 WAL 文件当前大小 | 2026-05-21 |
| WalWriter::should_checkpoint | `writer.should_checkpoint()` → `bool` | 检查是否应触发 checkpoint | 2026-05-21 |
| WalReader::open | `WalReader::open(&wal_path)` → `Result<Self, WalError>` | 打开 WAL 文件读取 | 2026-05-21 |
| WalReader::read_next | `reader.read_next()` → `Result<Option<WalRecord>, WalError>` | 读取下一条记录（迭代） | 2026-05-21 |
| WalReader::read_all | `reader.read_all()` → `Result<Vec<WalRecord>, WalError>` | 读取所有 WAL 记录 | 2026-05-21 |
| WalReader::seek_to | `reader.seek_to(lsn)` → `Result<(), WalError>` | 定位到指定 LSN | 2026-05-21 |
| CheckpointManager::new | `CheckpointManager::new(db_path, wal_writer, buffer_pool)` | 创建 Checkpoint 管理器 | 2026-05-21 |
| CheckpointManager::checkpoint | `manager.checkpoint().await` → `Result<u64, WalError>` | 执行 checkpoint（刷脏页+写位点） | 2026-05-21 |
| CheckpointManager::read_checkpoint_site | `manager.read_checkpoint_site()` → `Result<Option<(u64, u64)>, WalError>` | 读取 checkpoint 位点（lsn+timestamp） | 2026-05-21 |
| CheckpointManager::write_checkpoint_site | `manager.write_checkpoint_site(lsn, ts)` → `Result<(), WalError>` | 写入 checkpoint 位点 | 2026-05-21 |
| RecoveryManager::recover | `RecoveryManager::recover(db_path)` → `Result<(HashSet<u64>, HashSet<u64>), WalError>` | 恢复 commit/abort 标记 | 2026-05-21 |
| RecoveryManager::needs_recovery | `RecoveryManager::needs_recovery(db_path)` → `Result<bool, WalError>` | 检查是否需要恢复 | 2026-05-21 |
| RecoveryManager::read_wal | `RecoveryManager::read_wal(db_path)` → `Result<Vec<WalRecord>, WalError>` | 读取所有 WAL 记录 | 2026-05-21 |
| JoinExecutor::new | `JoinExecutor::new(left, right, conditions, output_columns, left_idx, right_idx, left_name, right_name)` | 创建哈希连接执行器 | 2026-05-22 |
| JoinExecutor::build_hash_key | `executor.build_hash_key_left/right(row)` → `Option<Vec<Value>>` | 计算哈希键（NULL 返回 None） | 2026-05-22 |
| JoinExecutor::build_output_row | `executor.build_output_row(left_row, right_row)` → `Vec<Value>` | 构建输出行（按 output_columns） | 2026-05-22 |
| JoinExecutor::next | `executor.next().await` → `Result<Option<ExecResult>>` | 哈希连接迭代（BuildRight→ScanLeft→Output） | 2026-05-22 |
| PlanBuilder::build_from_clause | `builder.build_from_clause(from)` → `PhysicalPlan` | 解析 FROM + JOIN 链 | 2026-05-22 |
| PlanBuilder::resolve_column_ref | `builder.resolve_column_ref(expr, tables)` → `ColumnRef` | 列引用解析（t.col 或纯列名） | 2026-05-22 |
| PlanBuilder::extract_join_conditions | `builder.extract_join_conditions(left_tables, right_table, on_expr)` → `Vec<JoinCondition>` | ON 条件解析（AND 组合） | 2026-05-22 |
| ColumnRef | `ColumnRef { table: Option<String>, column: String }` | 列引用（支持 t.col） | 2026-05-22 |
| JoinCondition | `JoinCondition { left_column: ColumnRef, right_column: ColumnRef }` | JOIN 等值条件 | 2026-05-22 |
| OutputColumn | `OutputColumn { table, column, table_alias, column_index }` | 输出列映射 | 2026-05-22 |
| JoinNode | `JoinNode { left, right, conditions, output_columns }` | JOIN 物理计划节点 | 2026-05-22 |
| PhysicalPlan::Join | `PhysicalPlan::Join(JoinNode)` | JOIN plan 变体 | 2026-05-22 |
| extract_join_table_name | `extract_join_table_name(relation)` → `String` | JOIN 表名提取 | 2026-05-22 |

---

## 文件路径速查

| 类型 | 路径 | 说明 |
|------|------|------|
| 入口文件 | src/main.rs | 应用启动点 |
| 配置文件 | Cargo.toml | Rust 项目配置 |
| 存储模块 | src/storage/ | 存储引擎核心 |
| 页格式模块 | src/storage/page_format/ | M2: Key/RowId/SlottedPage + M9: Float/Bool 序列化 |
| B-Tree 模块 | src/storage/btree/ | M2: BTree 索引核心 |
| 事务模块 | src/transaction/ | M3: TransactionId/Snapshot/VersionHeader/RowLockTable/Manager |
| 执行模块 | src/executor/ | M4-M5: PhysicalPlan/Value/ExecResult/Executor trait/5 Executors + M9: Predicate/Filter/DDL Executors |
| Predicate 模块 | src/executor/predicate.rs | M9: Predicate trait + Expression trait + ComparisonPredicate/LogicalPredicate |
| Filter 模块 | src/executor/filter.rs | M9: FilterExecutor（WHERE 过滤） |
| DDL 模块 | src/executor/create_table.rs + src/executor/drop_table.rs | M9: DDL Executors |
| Sort 模块 | src/executor/sort.rs | M9 Phase 2: SortExecutor（ORDER BY 排序） |
| Limit 模块 | src/executor/limit.rs | M9 Phase 2: LimitExecutor（LIMIT/OFFSET 分页） |
| 解析模块 | src/parser/ | M4: PlanBuilder/PlanError/AST helpers + M9: DDL/WHERE/ORDER BY/LIMIT 解析 |
| 网络模块 | src/network/ | M6: Protocol trait/JsonProtocol/Server/ConnectionHandler/SqlHandler |
| 数据库入口 | src/database.rs | M7: Database 协调器（BufferPool+TableManager+TxManager） |
| 执行管道 | src/pipeline.rs | M7: SQL→parse→plan→execute→Response + M11: DDL + WHERE 集成 |
| 数据存储 | src/storage/data/ | M7: TableManager + TableMeta + M9: drop_table + ColumnSchema |
| 数据页读写 | src/storage/data_page.rs | M7: write/read_tuple_to_data_page |
| Tuple 序列化 | src/storage/page_format/tuple.rs | M7: ColumnType + serialize/deserialize_tuple + M9: Float/Bool |
| WAL 模块 | src/wal/ | M11: WalRecord/WalWriter/WalReader/CheckpointManager/RecoveryManager |
| WAL 记录 | src/wal/record.rs | M11: WalRecord enum + serialize/deserialize + WalError |
| WAL 写入器 | src/wal/writer.rs | M11: async write_record + fsync + truncate |
| WAL 读取器 | src/wal/reader.rs | M11: read_next + seek_to + read_all |
| Checkpoint | src/wal/checkpoint.rs | M11: checkpoint flow + 位点读写 |
| Recovery | src/wal/recovery.rs | M11: recover + needs_recovery + read_wal |
| JOIN 模块 | src/executor/join.rs | M12: JoinExecutor 哈希连接 |
| JOIN 解析 | src/parser/planner.rs | M12: build_from_clause + resolve_column_ref + extract_join_conditions |
| JOIN 错误 | src/parser/error.rs | M12: AmbiguousColumn/TableNotFound/MissingOnClause/UnsupportedJoinType |

---

## 常用命令

| 场景 | 命令 | 输出含义 |
|------|------|----------|
| 初始化项目 | `cargo init` | 创建 Rust 项目骨架 |
| 运行测试 | `cargo test` | 执行全部测试（256 tests） |
| 格式化 | `cargo fmt` | 格式化代码 |
| Lint 检查 | `cargo clippy` | 静态分析检查 |
| 构建 | `cargo build` | 构建项目 |
| 构建（release） | `cargo build --release` | 构建生产版本 |
| 添加依赖 | `cargo add tokio --features full` | 添加 Tokio 依赖 |

---

## 踩坑记录

| 问题 | 原因 | 解决方案 | 发现时间 |
|------|------|----------|----------|
| spawn_blocking JoinError | tokio::task::spawn_blocking 返回 JoinError，StorageError 未处理 | 在 StorageError 中添加 #[from] JoinError | 2026-05-20 |
| Ok(()) 类型推断失败 | spawn_blocking 内部 Ok(()) 缺少类型注解 | 明确指定 Ok::<(), std::io::Error>(()) | 2026-05-20 |
| PageGuard Deref 返回临时引用 | MutexGuard 是临时值，不能返回引用 | 移除 Deref trait，使用 page() 方法返回克隆 | 2026-05-20 |
| Runtime nesting 错误 | 在异步测试中调用 Handle::block_on 导致 "Cannot start a runtime from within a runtime" | 将 SyncPageLoader 创建放入 spawn_blocking 内，避免在异步上下文中调用 block_on | 2026-05-20 |
| PageGuard 写回失效 | BTree 克隆 Page 后操作，但未写回 BufferPool，修改丢失 | 添加 PageGuard::modify_page() 方法，闭包内操作 + 自动 mark_dirty | 2026-05-20 |
| LeafNode insert 有序问题 | SlottedPage::add_slot 总是添加到末尾，无法中间插入 | 实现 shift_slots_right() 方法，调整 slot 数组保持有序 | 2026-05-20 |
| async_trait warning | Executor trait 使用 async fn 导致 clippy warning | 使用 #[async_trait::async_trait] macro 解决 | 2026-05-20 |
| unused field warning | IndexScanExecutor columns 字段未使用 | 移除未使用字段，保持代码简洁 | 2026-05-20 |
| tokio-util sync feature 错误 | tokio-util 没有 sync feature，CancellationToken 需要 rt feature | 修改为 `tokio-util = { features = ["rt"] }` | 2026-05-20 |
| AsyncReadExt/AsyncWriteExt 未找到 | Tokio 缺少 io-util feature，无法使用 read/write_all | 添加 `tokio = { features = ["io-util"] }` | 2026-05-20 |
| handler mut 声明缺失 | ConnectionHandler.handle() 需要 mut self，spawn 内未声明 mut | 修改为 `let mut handler = ConnectionHandler::new(...)` | 2026-05-20 |
| ConnectionHandler unused imports | Protocol trait 导入 Request/Response 但 connection.rs 未使用 | 移除 unused imports，保持代码整洁 | 2026-05-20 |
| MutexGuard 跨 await 非 Send | write_tuple_to_data_page 中 std::sync::MutexGuard 被 async_trait 的 Send bound 拒绝 | 在 await 前将 MutexGuard 局部化 drop，只在需要时短暂加锁 | 2026-05-20 |
| MutexGuard 跨 await 在 buffer_pool | evict_one 中 frame_guard 跨 await 导致 Send trait bound 失败 | 重构为 block 作用域，在 await 前自然 drop MutexGuard | 2026-05-20 |
| PageId(0) 是合法分配 | FileStorage::allocate_page 从 PageId(0) 开始分配 | 断言改为 `data_page_head == data_page_tail` 而非 `> 0` | 2026-05-20 |
| PlanBuilder 表注册依赖 | pipeline 需要表已在 PlanBuilder 中注册，但 TableManager 无 list_tables | 从 sqlparser Statement 提取表名 → TableManager::get_table → 动态注册 | 2026-05-20 |
| UpdateExecutor mock 破坏数据页引用 | M5 UpdateExecutor 使用 fake RowId(0, 999)，M7 IndexScanExecutor 读真实数据页时报 SlotNotFound | M7 重写 UpdateExecutor 为真实版本链创建，修复集成测试 | 2026-05-20 |
| Float 序列化缺少 deserialize 分支 | Task 4 serialize 支持 Float/Bool，但 deserialize 未添加 TAG_FLOAT/TAG_BOOL match arm | Task 4 补全 deserialize_tuple Float/Bool 分支 + ColumnType 扩展 | 2026-05-20 |
| Clippy approx_constant warning | 测试代码使用接近数学常数的浮点值（3.14/3.14159） | 替换为安全值（1.23/4.56）避免近似常数警告 | 2026-05-20 |
| SortExecutor column index mapping bug | Task 9 发现 ORDER BY 排序结果错误，列名未正确映射到行索引 | 在 SortExecutor 中添加 `columns: Vec<String>` 字段，compare_rows 使用列名查找索引 | 2026-05-21 |
| MockExecutor path error | 测试文件使用 `crate::storage::Result` 而非 `rtsql::storage::Result` | 在测试中使用 `rtsql::` 前缀而非 `crate::` | 2026-05-21 |
| WalRecord field mismatch | spec 要求 tx_id + table_name，实现用 table_id | 按 spec 修正字段名，添加 Commit timestamp | 2026-05-21 |
| Worktree isolation | subagent 在 worktree 工作，变更不自动合并到 main | 手动读取 worktree 文件 + Write 到 main 目录 | 2026-05-21 |
| Database missing wal_writer | executor_test 直接构造 Database 缺少 wal_writer 字段 | 添加 wal_writer: Arc::new(WalWriter::open(...)) | 2026-05-21 |
| NULL 键 JOIN 匹配错误 | SQL NULL != NULL，但 Value Eq 使 NULL == NULL | build_hash_key 返回 Option，NULL 键返回 None 被跳过 | 2026-05-22 |
| pipeline extract_column_indices | nested join 的 table name 提取脆弱 | 使用 conditions.first() 或改进为更好的 fallback | 2026-05-22 |

**详细踩坑档案**（复杂问题）：

### Runtime Nesting 错误（M2 SyncPageLoader）

- **症状**: 在 #[tokio::test] 异步测试中调用 SyncPageLoader::new() → Handle::block_on → "Cannot start a runtime from within a runtime"
- **根因**: Tokio 不允许在异步上下文（已进入 runtime）中再次调用 block_on（会尝试嵌套启动 runtime）
- **解决**: 将 IndexManager 创建放入 spawn_blocking 内部，在阻塞线程池中调用（不处于异步上下文）
- **预防**: spawn_blocking 内调用 block_on 安全，异步测试中不直接调用 block_on
- **时间**: 2026-05-20

### PageGuard 写回失效（M2 BTree）

- **症状**: BTree insert/delete 操作后，再次 search 返回旧数据（修改未持久化）
- **根因**: BTree 使用 `guard.page().clone()` 克隆 Page 后操作，克隆的数据未写回 BufferPool
- **解决**: 添加 PageGuard::modify_page() 方法，闭包内直接操作 Page + 自动 mark_dirty
- **预防**: 所有页修改使用 modify_page() 而非 clone + 操作
- **时间**: 2026-05-20

### LeafNode 有序插入问题（M2）

- **症状**: LeafNode::insert 后，key 未按序排列（SlottedPage::add_slot 总是添加到末尾）
- **根因**: SlottedPage 不支持中间插入，slot 数组只能从末尾增长
- **解决**: 实现 shift_slots_right() 方法，insert 后调整 slot 数组保持有序
- **预防**: 有序数据结构插入时，需考虑 slot 数组调整逻辑
- **时间**: 2026-05-20

### Tokio io-util Feature 缺失（M6）

- **症状**: 编译报错 `unresolved imports: tokio::io::AsyncReadExt, tokio::io::AsyncWriteExt`
- **根因**: Tokio 默认不包含 io-util feature，AsyncReadExt/AsyncWriteExt trait 需单独启用
- **解决**: 在 Cargo.toml 添加 `tokio = { features = ["io-util"] }`
- **预防**: 使用 tokio::io 异步读写 trait 时，必须启用 io-util feature
- **时间**: 2026-05-20

### tokio-util Feature 错误（M6）

- **症状**: 编译报错 `tokio-util does not have feature 'sync'`
- **根因**: CancellationToken 需要 rt feature 而非 sync feature（文档误导）
- **解决**: 修改为 `tokio-util = { version = "0.7", features = ["rt"] }`
- **预防**: 查看 crate 实际可用 feature，不要依赖假设或文档错误
- **时间**: 2026-05-20

### ConnectionHandler mut 声明缺失（M6）

- **症状**: 编译报错 `cannot borrow handler as mutable, as it is not declared as mutable`
- **根因**: ConnectionHandler::handle() 需要 &mut self，但 spawn 内 handler 未声明 mut
- **解决**: 修改为 `let mut handler = ConnectionHandler::new(...)`
- **预防**: async 方法需要 mut self 时，变量声明必须 mut
- **时间**: 2026-05-20

### Float/Bool Deserialization Missing（M9 Task 4）

- **症状**: deserialize_tuple 遇到 TAG_FLOAT/TAG_BOOL 报错 "unknown tag byte"
- **根因**: serialize_tuple 支持 Float/Bool，但 deserialize_tuple 缺少对应 match arm
- **解决**: 补全 deserialize_tuple 的 TAG_FLOAT/TAG_BOOL 分支 + ColumnType 扩展
- **预防**: 序列化扩展时，必须同步扩展反序列化分支
- **时间**: 2026-05-20

### Column Index Mapping Bug（M9 Phase 2 Task 9）

- **症状**: ORDER BY age ASC 排序结果错误，返回未排序数据
- **根因**: SortExecutor.compare_rows 假设列索引等于 order_by 数组位置，而非实际列在结果中的位置
- **解决**: 添加 `columns: Vec<String>` 字段，使用列名查找实际索引：`self.columns.iter().position(|c| c == order_col.column)`
- **预防**: Executor 需要列名→索引映射才能正确处理列引用，PlanBuilder 必须传递 SELECT projection 列名
- **时间**: 2026-05-21

---

## 技巧 & 模式

| 技巧 | 适用场景 | 示例代码/用法 |
|------|----------|--------------|
| 异步迭代器 | 执行引擎流式返回 | `async fn next() -> Result<Option<Row>>` |
| spawn_blocking 包装 | CPU密集型索引操作 | `tokio::task::spawn_blocking(|| btree_op())` |
| 异步页加载 | Buffer Pool 管理 | `get_page(page_id) -> impl Future<Output = Page>` |
| SyncPageLoader 模式 | 同步代码访问异步 BufferPool | `loader.load_page(page_id)` (内部 block_on) |
| PageGuard::modify_page | 页修改自动写回 | `guard.modify_page(|page| { leaf.insert(...) })` |
| AtomicU64 无锁分配 | 全局事务 ID | `counter.fetch_add(1, Ordering::SeqCst) + 1` |
| Snapshot 可见性判断 | MVCC 快照读 | `snapshot.is_visible(create_tx_id, commit_tx_id)` |
| 异步行锁 | 写写冲突等待 | `lock_table.get_lock(row_id).await; let guard = lock.lock().await` |
| sqlparser-rs 解析 | SQL 字符串解析 | `Parser::parse_sql(&GenericDialect{}, sql)` |
| AST 直接映射 | 简单查询计划生成 | `Statement::Query → build_query(query)` |
| PlanBuilder 表注册 | 元数据管理 | `builder.register_table("users", columns, "id")` |
| Value.to_key() | Int 转 Key | `Value::Int(n).to_key()` → Key::new(&n.to_be_bytes()) |
| Executor 模式 | 异步迭代器 | `async fn next(&mut self) -> Result<Option<ExecResult>>` |
| executed flag | 单次执行状态 | `if self.executed { return Ok(None); } self.executed = true; ...` |
| RowId placeholder | M5 测试占位 | `RowId::new(0, slot_id)` 或 `RowId::new(0, 999)` |
| NotImplemented variant | 未实现标记 | `ExecResult::NotImplemented` 用于 Scan（M7 补数据层） |
| Protocol trait 抽象 | 协议层可替换设计 | `#[async_trait] trait Protocol { async fn parse_request/write_response }` |
| newline-delimited framing | JSON 帧协议 | 消息以 `\n` 结尾，`stream.read_until(b'\n')` |
| CancellationToken shutdown | Graceful 关闭 | `tokio::select! { accept => ..., _ = shutdown.cancelled() => break }` |
| 每连接一协程 | 高并发连接模型 | `tokio::spawn(async move { handler.handle(stream).await })` |
| mock executor 模式 | 分阶段实现 | M6 SqlHandler 返回固定值，M7 整合真实 executor |
| VersionHeader::with_next_version | 版本链创建 | `VersionHeader::new(tx_id, None).with_next_version(old_row_id)` |
| Snapshot 可见性过滤 | 执行器读路径 | `snapshot.is_visible(vh.create_tx_id(), vh.commit_tx_id())` 过滤不可见版本 |
| Database 协调器模式 | 组件生命周期管理 | `Arc<Database>` 集中持有 BufferPool+TableManager+TxManager |
| Pipeline 管道模式 | SQL 全流程 | parse→extract table→register table→plan→executor→collect→Response |
| Tuple 序列化格式 | 紧凑二进制存储 | Int(9B) / String(3+N B) / Float(9B) / Bool(2B) / Null(1B) 的 type-tag 格式 |
| 数据页自动扩展 | Page 满时处理 | 写路径 detect PageFull → allocate new page → link via next_page_id → update tail |
| Predicate trait 模式 | WHERE 条件求值 | `trait Predicate: Send + Sync { fn evaluate(&self, row: &[Value]) -> Result<bool>; }` |
| Expression trait 模式 | 表达式求值 | `trait Expression: Send + Sync { fn evaluate(&self, row: &[Value]) -> Result<Value>; }` |
| ComparisonPredicate | 比较操作实现 | `ComparisonPredicate { left, op: Eq/Ne/Gt/Lt/Ge/Le, right }` |
| LogicalPredicate | 逻辑操作实现 | `LogicalPredicate { left, op: And/Or, right }`（短路求值） |
| ColumnExpression | 列引用 | `ColumnExpression { column_name, column_index }` (index 在构建时解析) |
| ConstantExpression | 常量值 | `ConstantExpression { value }` |
| Value 跨类型比较 | Int vs Float | `value.equals(&other)` 支持 Int(42) == Float(42.0) |
| FilterExecutor | WHERE 过滤 | 循环读取行 → MVCC 检查 → Predicate.evaluate → 返回满足条件的行 |
| DDL Executor 模式 | DDL 执行 | CreateTableExecutor/DropTableExecutor 检查表存在性 → 调用 TableManager |
| PlanBuilder DDL 解析 | DDL 解析 | build_create_table/build_drop_table → PhysicalPlan::CreateTable/DropTable |
| PlanBuilder WHERE 解析 | WHERE 解析 | build_where/build_expression → PredicateRef（递归处理 AND/OR） |
| create_executor_from_plan | Executor 创建 | PhysicalPlan::Filter → FilterExecutor(input_executor, predicate) |
| SortExecutor 内存排序 | ORDER BY 执行 | 收集所有行 → Vec::sort_unstable_by → 逐行输出（流水线中断） |
| LimitExecutor 增量计数 | LIMIT/OFFSET 执行 | skipped < offset → skip；taken >= limit → stop |
| NULL 排末尾 | SQL NULL 排序 | (Value::Null, non-Null) → Ordering::Greater（排在后面） |
| Vec::sort_unstable_by | 高效排序 | 比 sort 快，但可能改变相等元素顺序（可接受） |
| 列名→索引映射 | Executor 列引用 | SortNode.columns → compare_rows 使用列名查找实际位置 |
| Parser ORDER BY 解析 | ORDER BY 解析 | query.order_by → Vec<OrderByColumn> → PhysicalPlan::Sort |
| Parser LIMIT/OFFSET 解析 | 分页解析 | query.limit + query.offset → PhysicalPlan::Limit |
| Pipeline 递归 Executor 创建 | 嵌套 Plan 处理 | Limit(Sort(Filter(Scan))) → 递归 create_executor_from_plan |
| WalRecord 序列化格式 | WAL 紧凑格式 | [type:1B][len:4B LE][data:var]（手动序列化比 serde 紧凑） | 2026-05-21 |
| WalWriter spawn_blocking | 异步文件写入 | spawn_blocking(|| File::open().append().write()) 避免阻塞协程 | 2026-05-21 |
| Checkpoint 位点文件 | 位点持久化 | `<db_path>.checkpoint` 存储 (lsn:8B + timestamp:8B) | 2026-05-21 |
| Checkpoint flow | checkpoint 执行顺序 | 获取 LSN → flush_all → 写位点 → 写 WAL Checkpoint 记录 → 重置计数 | 2026-05-21 |
| RecoveryManager commit/abort | 崩溃恢复标记 | 遍历 WAL → 收集 Commit/Abort tx_id → 返回 HashSet | 2026-05-21 |
| Database WAL 集成 | 启动恢复 | RecoveryManager::recover → WalWriter::open → 添加到 Database 结构 | 2026-05-21 |
| 哈希连接算法 | INNER JOIN 执行 | BuildRight（建哈希表）→ ScanLeft（缓存）→ Output（逐行匹配） | 2026-05-22 |
| NULL 键跳过 | SQL NULL != NULL | build_hash_key 返回 Option<Vec<Value>>，NULL 返回 None → 跳过不匹配 | 2026-05-22 |
| Value Hash trait | HashMap 键支持 | Float 使用 to_bits() 哈希，支持 Vec<Value> 作为哈希键 | 2026-05-22 |
| ON 条件解析 | AND 组合递归 | extract_join_conditions 递归处理 AND → 返回 Vec<JoinCondition> | 2026-05-22 |
| 列名歧义检测 | 多表同名列 | resolve_column_ref 检查 sources.len()，>1 返回 AmbiguousColumn 错误 | 2026-05-22 |
| 链式 JOIN | 多表连接 | Join(Join(A, B), C) 递归结构，每个 Join 节点独立哈希连接 | 2026-05-22 |
| PageGuard::page_data() | 零拷贝读取页数据 | 返回 PageDataGuard（Deref to &[u8]），避免 4KB clone | 2026-05-22 |
| SlottedPageRef | 从 &[u8] 只读访问 slot | 配合 page_data() 使用，不需要 &mut Page | 2026-05-22 |
| BufferPool 两阶段锁 | 释放写锁后再做 I/O | get_page() 读锁检查→释放→I/O→写锁插入（double-check） | 2026-05-22 |
| criterion.rs async bench | 异步 benchmark | 用 b.to_async(&rt) 包装，rt = tokio::runtime::Runtime | 2026-05-22 |
| AtomicI64 避免并发写冲突 | 并发 benchmark key 分配 | static AtomicI64::fetch_add 分配不重叠的 key 范围 | 2026-05-22 |
| std::mem::forget(TempDir) | benchmark 临时目录 | leak TempDir 防止临时目录在 bench 期间被清理 | 2026-05-22 |

---

## 依赖关系图

```
[Network Layer] → [SQL Parser] → [Execution Engine] → [Transaction Manager] → [Storage Engine] → [Buffer Pool] → [File I/O]

异步边界：
  - Network Layer: tokio::spawn 处理连接
  - Execution Engine: async fn next() 迭代器
  - Transaction Manager: tokio::sync::RwLock 异步锁
  - Buffer Pool: get_page() -> Future
  - File I/O: spawn_blocking 或 io_uring
```

---

## 待探索

以下内容尚未完全理解，需要后续探索：

- [ ] io_uring 集成方式（M13 阶段）
- [ ] PostgreSQL 有线协议细节（M6 阶段）
- [x] MVCC 实现细节（M3 阶段）→ Repeatable Read 隔离级别已实现
- [x] B-Tree 索引优化策略（M2 阶段）→ 简化实现（Split/Merge 未完整）
- [x] PageGuard::modify_page() 方法（M2 添加）
- [x] SQL 解析与计划生成（M4 阶段）→ sqlparser-rs + PhysicalPlan 已实现
- [x] 异步执行引擎（M5 阶段）→ Executor trait + 5 Executors 已实现
- [ ] WAL（Write-Ahead Logging）实现（M11 阶段）
- [x] 版本链 GC（清理旧版本）（M10 阶段）→ gc_table 已实现（可选）
- [ ] Serializable 隔离级别（需谓词锁，推迟）
- [x] 复杂 WHERE 表达式计算（M9 阶段）→ Predicate trait + Expression trait 已实现
- [ ] JOIN 多表计划与执行（M12 阶段）
- [x] 数据存储层（TableManager、Row 数据）（M7 阶段）→ 已完成，157 测试通过
- [x] DDL 元数据管理（M9 阶段）→ CREATE TABLE/DROP TABLE 已实现
- [x] 全流程集成（M7 阶段）→ Database + Pipeline + 真实 SqlHandler
- [x] MVCC 可见性集成（M7 阶段）→ 最新版本可见性过滤，版本链创建
- [x] 完整版本链遍历（follow next_version）（M10 阶段）→ find_visible_version 已实现
- [x] WHERE 表达式求值器（M9 阶段）→ Predicate trait + FilterExecutor 已实现
- [x] ORDER BY 排序（M9 Phase 2 阶段）→ SortExecutor + 列名映射已实现
- [x] LIMIT/OFFSET 分页（M9 Phase 2 阶段）→ LimitExecutor 已实现
- [x] WAL（Write-Ahead Logging）基础实现（M11 阶段）→ WalRecord/WalWriter/WalReader/CheckpointManager/RecoveryManager 已实现
- [ ] Executor 层 WAL 写入集成（M11 推迟）
- [ ] WAL 完整数据重放（M11 推迟）
- [x] JOIN 多表计划与执行（M12 阶段）→ JoinExecutor + ON 子句解析 + NULL 处理已实现

---

## 已验证的知识

| 知识点 | 验证方式 | 结论 |
|--------|----------|------|
| Tokio 多线程调度器适合数据库 | 架构分析 | 多线程调度器可充分利用多核，适合高并发场景 |
| spawn_blocking 不阻塞异步运行时 | 文档确认 | CPU密集型操作隔离，不影响协程调度 |
| AtomicU64 无锁分配正确性 | 多线程测试（10 线程并发） | fetch_add 保证原子递增，ID 唯一且有序 |
| Snapshot 可见性规则正确 | 5 个单元测试 | 已提交/未提交/活跃事务场景正确判断 |
| 异步行锁不阻塞物理线程 | 3 个并发测试 | tokio::sync::Mutex 在 await 时挂起协程，线程继续执行其他任务 |
| Predicate trait 设计正确 | 12 个测试 | ComparisonPredicate/LogicalPredicate 正确求值，NULL 处理符合 SQL 语义 |
| Expression trait 设计正确 | 测试验证 | ColumnExpression/ConstantExpression 正确求值 |
| FilterExecutor 正确过滤 | 3 个测试 | WHERE 条件正确应用，MVCC 可见性检查正确 |
| DDL Executors 正确执行 | 集成测试验证 | CREATE TABLE/DROP TABLE 正确执行，错误处理正确 |
| Value 跨类型比较正确 | 19 个测试 | Int vs Float 比较正确，类型转换正确 |
| Float/Bool 序列化正确 | roundtrip 测试 | serialize/deserialize 正确，字节格式符合规范 |