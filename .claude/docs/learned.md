# 学习记忆

> 最后更新：2026-05-20 (M6 网络层完成)
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

---

## 文件路径速查

| 类型 | 路径 | 说明 |
|------|------|------|
| 入口文件 | src/main.rs | 应用启动点 |
| 配置文件 | Cargo.toml | Rust 项目配置 |
| 存储模块 | src/storage/ | 存储引擎核心 |
| 页格式模块 | src/storage/page_format/ | M2: Key/RowId/SlottedPage |
| B-Tree 模块 | src/storage/btree/ | M2: BTree 索引核心 |
| 事务模块 | src/transaction/ | M3: TransactionId/Snapshot/VersionHeader/RowLockTable/Manager |
| 执行模块 | src/executor/ | M4-M5: PhysicalPlan/Value/ExecResult/Executor trait/5 Executors |
| 解析模块 | src/parser/ | M4: PlanBuilder/PlanError/AST helpers |
| 网络模块 | src/network/ | M6: Protocol trait/JsonProtocol/Server/ConnectionHandler/SqlHandler |

---

## 常用命令

| 场景 | 命令 | 输出含义 |
|------|------|----------|
| 初始化项目 | `cargo init` | 创建 Rust 项目骨架 |
| 运行测试 | `cargo test` | 执行全部测试 |
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
| NotImplemented variant | 未实现标记 | `ExecResult::NotImplemented` 用于 Scan（M6 补数据层） |

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

- [ ] io_uring 集成方式（M7 阶段）
- [ ] PostgreSQL 有线协议细节（M6 阶段）
- [x] MVCC 实现细节（M3 阶段）→ Repeatable Read 隔离级别已实现
- [x] B-Tree 索引优化策略（M2 阶段）→ 简化实现（Split/Merge 未完整）
- [x] PageGuard::modify_page() 方法（M2 添加）
- [x] SQL 解析与计划生成（M4 阶段）→ sqlparser-rs + PhysicalPlan 已实现
- [x] 异步执行引擎（M5 阶段）→ Executor trait + 5 Executors 已实现
- [ ] WAL（Write-Ahead Logging）实现（M7 阶段）
- [ ] 版本链 GC（清理旧版本）（M7 阶段）
- [ ] Serializable 隔离级别（需谓词锁，推迟）
- [ ] 复杂 WHERE 表达式计算（M5/M6 阶段）
- [ ] JOIN 多表计划与执行（M5/M6 阶段）
- [ ] 数据存储层（TableManager、Row 数据）（M6 阶段）
- [ ] DDL 元数据管理（后续里程碑）

---

## 已验证的知识

| 知识点 | 验证方式 | 结论 |
|--------|----------|------|
| Tokio 多线程调度器适合数据库 | 架构分析 | 多线程调度器可充分利用多核，适合高并发场景 |
| spawn_blocking 不阻塞异步运行时 | 文档确认 | CPU密集型操作隔离，不影响协程调度 |
| AtomicU64 无锁分配正确性 | 多线程测试（10 线程并发） | fetch_add 保证原子递增，ID 唯一且有序 |
| Snapshot 可见性规则正确 | 5 个单元测试 | 已提交/未提交/活跃事务场景正确判断 |
| 异步行锁不阻塞物理线程 | 3 个并发测试 | tokio::sync::Mutex 在 await 时挂起协程，线程继续执行其他任务 |