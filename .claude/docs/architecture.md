# 架构决策记录 (ADR)

> 记录关键架构决策及其背景

## 决策列表

### 2026-05-20 - 项目初始化：异步协程架构

- **决策**: 采用 Tokio 无栈协程为调度核心的嵌入式关系型数据库架构
- **原因**:
  - 利用 Rust 零成本抽象 + Tokio 协程调度实现高性能
  - 消除线程上下文切换开销，最大化 I/O 吞吐
  - 支持海量连接（数万长连接），低内存占用
  - 架构天然支持未来分布式扩展（可添加 Raft 共识层）
- **影响**:
  - 全栈异步化：从网络层到存储层均采用 async/await
  - 模块划分需清晰区分同步核心与异步包装
  - 测试需覆盖异步场景和并发正确性
- **替代方案**:
  - 传统多线程模型（rejected：线程切换开销大，连接数受限）
  - 纯同步阻塞式（rejected：无法利用异步 I/O 性能优势）

### 2026-05-20 - 异步优先架构分层

- **决策**: 采用六层异步架构，从上到下依次为：网络层、解析层、执行层、事务层、存储层、I/O层
- **原因**: 模块职责清晰，异步边界明确，便于逐步实现和测试
- **影响**:
  - 各层需定义明确的 async trait 和接口
  - I/O 操作统一通过 `.await` 挂起，不阻塞物理线程
  - CPU 重操作通过 `spawn_blocking` 隔离
- **架构图**:
```
┌──────────────────────────────────────────┐
│         Async API / Network Layer        │   ← tokio::net (TCP/HTTP)
├──────────────────────────────────────────┤
│        Async SQL Parser & Planner        │   ← 同步解析 + spawn_blocking
├──────────────────────────────────────────┤
│          Async Execution Engine          │   ← async 迭代器，流式返回
├──────────────┬───────────────────────────┤
│  Async Tx    │    Index Manager          │   ← 索引同步，异步 I/O 包装
│  Manager     │  (B-Tree / LSM)           │
├──────────────┴───────────────────────────┤
│          Async Storage Engine            │   ← get_page() -> impl Future
├──────────────────────────────────────────┤
│     Async Buffer Pool / Page Cache       │   ← 异步页加载/淘汰
├──────────────────────────────────────────┤
│  Async File I/O (io_uring / tokio::fs)   │   ← 真正的异步磁盘读写
└──────────────────────────────────────────┘
```

### 2026-05-20 - 关键技术选型

| 模块 | 方案 | 决策理由 |
|------|------|----------|
| 异步运行时 | **Tokio** (多线程 scheduler) | 生态成熟，提供 net/fs/sync/io_uring 支持，多线程调度器适合数据库高并发 |
| SQL 解析 | `sqlparser-rs` | 同步解析，解析结果立即转为内部计划，避免异步复杂性 |
| 存储格式 | 自定义 4KB 页，Slotted Page 行存储 | 紧凑、零拷贝访问，配合 `zerocopy` crate |
| 索引 | 自行实现 B-Tree（同步）+ LSM 可选 | 索引操作在 `spawn_blocking` 中执行，CPU密集型隔离 |
| 并发控制 | MVCC + 异步读写锁 | 读无锁，写冲突通过协程挂起等待，符合协程调度模型 |
| 文件 I/O | 阶段1：`spawn_blocking` + 同步IO；阶段2：`io_uring` | 逐步演进，平衡实现复杂度和性能 |
| 内存分配 | `jemalloc` / `mimalloc` | 减少内存碎片，提升多线程分配效率 |
| 测试 | `sqllogictest` + `proptest` | 保证 SQL 兼容性和正确性，覆盖边界场景 |

### 2026-05-20 - M1 存储层架构决策

- **决策**: 采用 AsyncStorage trait + FileStorage 实现 + BufferPool（Clock 淘汰）三层架构
- **原因**:
  - trait 抽象便于未来扩展（内存模式、io_uring）
  - spawn_blocking 只在 FileStorage 层使用，不污染上层
  - Clock 淘汰比纯 LRU 更好适应扫描访问模式
- **影响**:
  - BufferPool 使用 RwLock<HashMap> + Mutex<PageFrame> 组合
  - PageGuard 通过引用计数防止淘汰正在使用的页
  - Dirty page 在淘汰时自动写回（不主动 flush）
- **替代方案**:
  - 一体化 BufferPool（rejected：职责混乱，难以测试）
  - 纯 LRU 淘汰（rejected：实现复杂，对扫描访问性能差）
- **文件结构**:
```
src/storage/
├── mod.rs           # 模块导出
├── error.rs         # StorageError（含 JoinError）
├── page_id.rs       # PageId 结构
├── page.rs          # Page 结构（4KB Box<[u8]>）
├── async_storage.rs # AsyncStorage trait
├── file_storage.rs  # FileStorage 实现
├── buffer_pool.rs   # BufferPool + Clock 淘汰
└── page_frame.rs    # PageFrame + PageGuard
```

### 2026-05-20 - M2 架构决策：B-Tree 索引三层分离设计

- **决策**: 采用 IndexManager（异步）→ BTree（同步）→ SyncPageLoader（block_on 包装）三层架构
- **原因**:
  - BTree 操作（split/merge）是 CPU 密集型，需纯同步实现避免 await overhead
  - BufferPool 是异步接口，需要 block_on 桥接同步/异步边界
  - IndexManager 作为公开 API，必须异步以不阻塞协程调度
- **影响**:
  - SyncPageLoader 使用 `Handle::block_on` 在 spawn_blocking 内安全执行
  - BTree 核心逻辑不依赖 async，便于后续优化和测试
  - IndexManager 使用 `Arc<Mutex<BTree>>` 提供线程安全访问
- **文件结构**:
```
src/storage/btree/
├── mod.rs           # 模块导出
├── node.rs          # LeafNode + InternalNode（纯同步）
├── btree.rs         # BTree 核心逻辑（纯同步）
├── sync_loader.rs   # SyncPageLoader（block_on 包装）
└── index_manager.rs # IndexManager（全异步）
```
- **替代方案**:
  - BTree 直接 async（rejected：CPU 操作无需 async，增加 overhead）
  - IndexManager 使用 async Mutex（rejected：spawn_blocking 内用 sync Mutex 更高效）

### 2026-05-20 - M2 架构决策：分离设计 LeafNode vs InternalNode

- **决策**: LeafNode 和 InternalNode 分别定义结构，职责清晰
- **原因**:
  - LeafNode 存储 Key + RowId（指向数据页）
  - InternalNode 存储 Key + ChildPageId（指向子节点）
  - 分离设计便于未来扩展（如变长 Key、压缩）
- **影响**:
  - LeafNode entry size: 32 bytes (Key) + 6 bytes (RowId) = 38 bytes
  - InternalNode entry size: 32 bytes (Key) + 4 bytes (ChildPageId) = 36 bytes
  - 两者使用统一的 SlottedPage 格式存储
- **替代方案**:
  - 统一 Node 结构（rejected：类型检查复杂，访问需检查 is_leaf）

### 2026-05-20 - M2 架构决策：固定长度 Key（32 bytes）

- **决策**: M2 采用固定 32 bytes Key，简化实现
- **原因**:
  - 避免变长 Key 的复杂性（内存管理、序列化、比较）
  - Slot 结构统一，便于序列化和访问
  - 快速定位和比较（无需额外长度字段）
- **影响**:
  - Key 结构包含 data[32] + len（实际长度）
  - 序列化固定写入 32 bytes（尾部填充 0）
  - 限制 Key 最大长度为 32 bytes（超出 panic）
- **替代方案**:
  - 变长 Key（推迟到 M7：需要复杂内存管理和序列化）

### 2026-05-20 - M2 架构决策：Slotted Page 统一格式

- **决策**: 所有页（Leaf/Internal/Data）统一采用 Slotted Page 格式
- **原因**:
  - 通用格式，支持变长数据（为未来扩展做准备）
  - Slot 数组从页尾向上增长，Row Data 从 header 向下增长
  - Free Space 在中间，动态调整
- **影响**:
  - Header 固定 16 bytes（page_type + slot_count + free_space_offset + next_page_id）
  - Slot 固定 4 bytes（offset + length）
  - 统一的读写接口（add_slot、delete_slot、free_space）
- **布局**:
```
┌────────────┬──────────────────┬─────────────┬─────────────┐
│ Header     │ Free Space       │ Slot Array  │ Row Data    │
│ (16 bytes) │ (grows ↓)        │ (grows ↑)   │ (grows ↓)   │
└────────────┴──────────────────┴─────────────┴─────────────┘
```

### 模块划分与职责

| 模块 | 职责 | 异步策略 |
|------|------|----------|
| **网络层** | 处理 TCP 连接，实现 PostgreSQL 有线协议或自定义协议 | `tokio::net::TcpListener`，每个连接 `tokio::spawn` 一个协程 |
| **解析层** | SQL 解析与计划生成 | 同步解析，通过 `spawn_blocking` 包装 |
| **执行层** | 物理计划执行，流式返回结果 | `async fn next() -> Result<Option<Row>>`，异步迭代器 |
| **事务层** | MVCC 并发控制，事务提交 | `AtomicU64` 维护事务ID，异步锁实现提交等待 |
| **存储层** | Buffer Pool 管理，页缓存 | `get_page(page_id) -> PageFuture`，异步页加载/淘汰 |
| **I/O层** | 文件读写，WAL 写入 | 初期 `spawn_blocking` 包装同步IO，未来升级 `io_uring` |

### 协程调度模型

- **核心思想**: 用户态无栈协程，协作式调度
- **连接模型**: 每个数据库连接是一个协程，数千连接复用少量工作线程
- **执行模型**: 每条 SQL 执行是一个协程，I/O 操作通过 `.await` 挂起
- **锁等待**: 通过 `tokio::sync` 实现，不阻塞物理线程
- **CPU隔离**: 重操作（排序、哈希）通过 `spawn_blocking` 移至阻塞线程池

### 性能目标

- **海量连接**: 单台机器支持数万长连接，每个连接仅占用极少量内存
- **I/O 并发**: 磁盘利用率趋近 100%，协程自动调度
- **低延迟**: 请求延迟微秒级，无线程阻塞和唤醒开销
- **可伸缩性**: 工作线程数固定（等于 CPU 核数），避免上下文切换

### 开发路线图

| 里程碑 | 内容 | 异步相关重点 |
|--------|------|-------------|
| M0 | 项目骨架，引入 Tokio | 确定异步运行时配置 |
| M1 | 文件/缓存层 | 实现 `AsyncStorage` trait，使用 `spawn_blocking` 读页 | ✅ 完成 |
| M2 | B-Tree 索引与存储引擎 | 索引同步，通过 `spawn_blocking` 暴露为 async API | ✅ 完成 |
| M3 | 事务与 MVCC | 用异步锁实现提交等待，快照读无锁 |
| M4 | SQL 解析与计划 | 同步解析，生成物理计划（包含 async 节点） |
| M5 | 异步执行引擎 | 实现 `async fn next()` 迭代器，整合存储异步接口 |
| M6 | 全流程集成 + 网络层 | 实现 TCP 服务器，每个连接一个协程 |
| M7 | 性能深度优化 | 替换 `io_uring`，调优协程调度、页缓存策略 |