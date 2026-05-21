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

### 2026-05-20 - M3 架构决策：MVCC 事务系统五层设计

- **决策**: 采用 TransactionId → Snapshot → VersionHeader → RowLockTable → TransactionManager 五层架构
- **原因**:
  - TransactionId（AtomicU64）无锁分配全局唯一 ID
  - Snapshot 实现 Repeatable Read 可见性规则
  - VersionHeader 管理版本链（多版本数据）
  - RowLockTable 异步行级锁（写写冲突等待）
  - TransactionManager 协调事务生命周期（begin/commit/abort）
- **影响**:
  - 快照读无锁（读写不阻塞）
  - 写写冲突通过 tokio::sync::Mutex 等待（不阻塞物理线程）
  - 版本链存储在 Row 数据前（22 bytes header）
- **文件结构**:
```
src/transaction/
├── mod.rs           # 模块导出
├── tx_id.rs         # TransactionId（AtomicU64）
├── error.rs         # TransactionError
├── snapshot.rs      # Snapshot（可见性判断）
├── version_chain.rs # VersionHeader（版本链头部）
├── row_lock.rs      # RowLockTable（异步行锁）
└── manager.rs       # TransactionManager（事务管理）
```
- **替代方案**:
  - 全局大锁（rejected：性能差，阻塞所有并发）
  - 两阶段锁（rejected：读写阻塞，不符合 MVCC 设计）

### 2026-05-20 - M3 架构决策：Repeatable Read 隔离级别

- **决策**: M3 实现 Repeatable Read（可重复读）隔离级别
- **原因**:
  - MVCC 标准模式，真正的快照读
  - 读不阻塞写，写不阻塞读
  - 比 Serializable 实现简单（无需谓词锁）
- **影响**:
  - 事务开始时创建 Snapshot（记录活跃事务列表）
  - 读操作按 Snapshot 可见性规则过滤版本
  - 写操作创建新版本，链接到 VersionChain
- **可见性规则**:
```
版本可见条件：
  1. 创建事务已提交（commit_tx_id 存在）
  2. 创建事务 ID < Snapshot ID（早于快照）
  3. 创建事务不在 Snapshot.active_tx_ids 中
```

### 2026-05-20 - M3 架构决策：版本链同页存储优先

- **决策**: 版本链优先在同页存储，页满时溢出到新页
- **原因**:
  - 紧凑存储，减少 I/O
  - 同页访问无需额外页加载
  - 溢出页处理边界情况
- **影响**:
  - VersionHeader 固定 22 bytes（create_tx_id + commit_tx_id + next_version）
  - next_version 指向上一版本 RowId（6 bytes）
  - 版本过多时需手动清理（推迟到 M7）
- **VersionHeader 布局**:
```
┌──────────────┬──────────────┬──────────────┐
│ create_tx_id │ commit_tx_id │ next_version │
│  (8 bytes)   │  (8 bytes)   │  (6 bytes)   │
└──────────────┴──────────────┴──────────────┘
Total: 22 bytes
```

### 2026-05-20 - M4 架构决策：sqlparser-rs + 直接物理计划

- **决策**: 采用 sqlparser-rs 解析 + 直接映射到 PhysicalPlan（跳过逻辑计划层）
- **原因**:
  - M4 目标是建立解析 → 计划流程，单表主键查询不需要逻辑计划优化
  - sqlparser-rs 提供完整 AST，可直接提取所需信息
  - 避免中间层复杂性，降低实现难度
- **影响**:
  - 解析层纯同步（符合架构决策：CPU 密集型）
  - PlanBuilder 负责语义验证（表名/列名/主键）
  - PhysicalPlan 是静态结构，不含 async 语义
- **文件结构**:
```
src/parser/
├── mod.rs           # 模块导出
├── error.rs         # PlanError（7 种错误类型）
├── value.rs         # Value 转换函数（sqlparser → executor Value）
├── ast.rs           # AST 辅助函数（parse_sql/extract_select_body/extract_table_name）
└── planner.rs       # PlanBuilder（AST → PhysicalPlan）

src/executor/
├── mod.rs           # 模块导出
├── value.rs         # Value enum（Int/String/Null + to_key()）
└── plan.rs          # PhysicalPlan + 5 节点结构
```
- **替代方案**:
  - 三层架构（AST → 逻辑计划 → 物理计划）（rejected：M4 单表主键查询无需优化层）
  - 手写解析器（rejected：sqlparser-rs 已成熟，无需重复造轮）

### 2026-05-20 - M4 架构决策：PhysicalPlan 五节点设计

- **决策**: PhysicalPlan 采用 5 种节点：Scan、IndexScan、Insert、Update、Delete
- **原因**:
  - M4 范围：DML Only（INSERT/UPDATE/DELETE/SELECT）
  - 节点类型直接对应 SQL 语句类型
  - 与 M2/M3 集成接口明确（IndexScan.key → IndexManager.get()）
- **影响**:
  - Scan：全表扫描（无 WHERE）
  - IndexScan：主键查询（WHERE pk = value）
  - Insert：插入（VALUES 列表）
  - Update：单列更新（WHERE pk = value）
  - Delete：删除（WHERE pk = value）
- **推迟内容**:
  - 复杂 WHERE（表达式计算，推迟到 M5）
  - JOIN（多表，推迟到 M5/M6）
  - 聚合函数（需聚合算子，推迟到 M5）
  - DDL（CREATE/DROP，需元数据管理，后续里程碑）

### 2026-05-20 - M5 架构决策：Executor trait + 多实现模式

- **决策**: 采用 Executor trait + 多 Executor 结构设计（而非单一 Executor switch PhysicalPlan）
- **原因**:
  - 每种 PhysicalPlan 节点职责独立，符合 Rust trait 抽象
  - 便于扩展新节点类型（未来 Filter/Join 等）
  - 测试时可单独测试每种 Executor
  - 符合 "Design for isolation and clarity" 原则
- **影响**:
  - Executor trait 定义统一接口：`async fn next(&mut self) -> Result<Option<ExecResult>>`
  - 5 种 Executor 实现：ScanExecutor、IndexScanExecutor、InsertExecutor、UpdateExecutor、DeleteExecutor
  - 使用 async_trait macro 保证 Send bounds
- **文件结构**:
```
src/executor/
├── mod.rs           # 模块导出
├── value.rs         # Value enum（M4）
├── plan.rs          # PhysicalPlan（M4）
├── result.rs        # ExecResult enum（M5）
├── executor_trait.rs # Executor trait（M5）
├── scan.rs          # ScanExecutor（M5: NotImplemented）
├── index_scan.rs    # IndexScanExecutor（M5）
├── insert.rs        # InsertExecutor（M5）
├── update.rs        # UpdateExecutor（M5）
└── delete.rs        # DeleteExecutor（M5）
```
- **替代方案**:
  - 单一 Executor + switch PhysicalPlan（rejected：扩展性差，职责混乱）

### 2026-05-20 - M5 架构决策：ExecResult 统一返回类型

- **决策**: ExecResult enum 包含三种结果：RowId、AffectedRows、NotImplemented
- **原因**:
  - 统一返回类型，适配不同 PhysicalPlan 节点
  - RowId 用于 IndexScan（M5 仅索引层执行）
  - AffectedRows 符合 SQL 语义（INSERT 返回插入行数）
  - NotImplemented 明确标记未完成功能（Scan 暂不实现）
- **影响**:
  - M5 仅返回 RowId（数据存储层推迟 M6）
  - ScanExecutor 返回 NotImplemented，不阻塞其他测试
  - 写操作返回影响计数，符合 SQL 标准
- **推迟内容**:
  - 完整 Row 数据返回（M6 实现数据存储层）
  - Scan 全表扫描（M7 实现数据存储层）
  - 事务整合（M7 网络层整合）

### 2026-05-20 - M6 架构决策：网络层三层分离设计

- **决策**: 采用 Protocol trait + JsonProtocol + ConnectionHandler + Server 三层分离架构
- **原因**:
  - Protocol trait 抽象：为后续升级 PostgreSQL 协议预留接口
  - JsonProtocol 简单实现：快速验证全流程，降低实现复杂度
  - ConnectionHandler 隔离：每连接一协程，职责清晰
  - Server 统一管理：TcpListener + graceful shutdown
- **影响**:
  - 协议可替换：升级 PG 协议只需新增 `PgProtocol` 实现
  - 测试友好：每层可独立测试（protocol、server）
  - mock executor：M6 仅网络层，真实执行推迟 M7
- **文件结构**:
```
src/network/
├── mod.rs           # 模块导出
├── error.rs         # NetworkError enum
├── protocol.rs      # Protocol trait + JsonProtocol + Request/Response
├── connection.rs    # ConnectionHandler（每连接一协程）
├── handler.rs       # SqlHandler（M6 mock executor）
└── server.rs        # Server + TcpListener + CancellationToken
```
- **替代方案**:
  - 单文件简单架构（rejected：职责混乱，难扩展）
  - Actor 模式（rejected：过度设计，不符合无状态场景）

### 2026-05-20 - M6 架构决策：Protocol trait 抽象

- **决策**: 定义 Protocol trait 包含 `parse_request` 和 `write_response` 两个 async 方法
- **原因**:
  - 抽象协议层：支持多种协议实现（JSON、PostgreSQL、自定义）
  - async trait：支持流式读写（大消息分帧）
  - 泛型 ConnectionHandler：`ConnectionHandler<P: Protocol>` 支持协议替换
- **影响**:
  - JsonProtocol 实现：newline-delimited framing（简单帧协议）
  - PostgreSQL 协议预留：后续只需实现 `PgProtocol`
  - 测试隔离：协议测试不依赖 server
- **协议格式**:
```
JSON 帧协议：
  Request:  {"Query":{"sql":"SELECT * FROM users"}}\n
  Response: {"QueryResult":{"row_ids":[[0,1]]}}\n
```

### 2026-05-20 - M6 架构决策：JSON 消息优先（后续升级 PG）

- **决策**: M6 先实现 JSON 协议，后续里程碑升级 PostgreSQL 协议
- **原因**:
  - YAGNI 原则：先验证正确性，后优化性能
  - serde_json 成熟：无需额外依赖管理
  - JSON 可读性强：调试友好
- **影响**:
  - 效率较低：JSON 序列化有开销（标记为优化点）
  - 不兼容 psql：需要专用客户端
  - 快速迭代：先验证全流程，后续升级
- **推迟内容**:
  - PostgreSQL 有线协议（后续里程碑）
  - 二进制协议优化（后续里程碑）

### 2026-05-20 - M7 架构决策：Database 协调器结构

- **决策**: 引入 `Database` struct 集中管理 `BufferPool`、`TableManager`、`TransactionManager`
- **原因**:
  - 消除分散初始化样板，提供单一 `open(Path)` + `execute_sql(SQL)` 入口
  - 类似 sqlite3 的 `sqlite3*` handle 模式，降低使用复杂度
  - 所有组件通过 `Arc` 共享，支持多连接并发访问
- **影响**:
  - `Server` 接受 `Arc<Database>`，每个 `ConnectionHandler` 的 `SqlHandler` 持有 clone
  - Pipeline 通过 Database 访问所有子系统
- **替代方案**: 分散初始化各组件（rejected：样板代码多，生命周期管理复杂）

### 2026-05-20 - M7 架构决策：数据存储模块

- **决策**: 创建 `src/storage/data/` 模块存放 `TableManager`，数据页读写放在 `src/storage/data_page.rs`
- **原因**: 分离"数据存储"（Tuple、表元数据）与"页格式"（Key、RowId、SlottedPage），后者被 BTree 复用
- **影响**:
  - `TableMeta` 持有 `IndexManager`（主键索引）和 data_page 链表（head + tail）
  - `write_tuple_to_data_page` 负责页满自动分配新页 + 链表更新
  - `read_tuple_from_data_page` 通过 RowId 定位 page + slot，解析 VersionHeader + tuple
- **文件结构**:
```
src/storage/data/         # 数据存储
├── mod.rs
└── table_manager.rs      # TableManager + TableMeta
src/storage/data_page.rs  # write/read_tuple_to_data_page
src/storage/page_format/tuple.rs  # ColumnType + serialize/deserialize
```

### 2026-05-20 - M7 架构决策：简单类型化 Tuple 序列化

- **决策**: 采用 type-tag + value 的紧凑二进制格式，不依赖 serde
- **原因**:
  - Schema 在反序列化时已知（来自 TableManager），无需自描述格式
  - 紧凑存储（Int 9B, String 3+N B, Null 1B），适合 4KB 页内密集打包
- **格式**: `[0x01][8B i64 LE]` / `[0x02][2B len LE][N B UTF-8]` / `[0x03]`
- **替代方案**: serde 自描述格式（rejected：冗余大，页利用率低）

### 2026-05-20 - M7 架构决策：BTree 全扫描接口

- **决策**: 添加 `BTree::scan_all()` 遍历所有 LeafNode 的 entries，通过 `IndexManager::scan_all()` 暴露 async API
- **原因**: 为 `ScanExecutor`（全表扫描）提供数据源；跟随 `next_leaf_page_id` 链支持未来多叶子节点
- **影响**: 当前仅单根叶节点（无 split），链在 root 后终止；未来 multi-leaf 自然扩展

### 2026-05-20 - M7 架构决策：MVCC 分阶段实现

- **决策**: M7 仅验证最新版本的 MVCC 可见性（Snapshot.is_visible），完整版本链遍历推迟到 M8
- **原因**:
  - 版本链遍历需要跟随 `next_version` 跨页读取，增加 I/O 复杂度
  - 先验证单版本可见性规则正确性，再扩展多版本遍历
- **影响**:
  - InsertExecutor 创建 `VersionHeader(tx_id, None)`（未提交标记）
  - UpdateExecutor 创建新版本 + `with_next_version(old_row_id)` 链
  - Read Executor 通过 `Snapshot` 参数过滤不可见版本
  - M8 补全：follow next_version 找第一个可见版本

### 2026-05-20 - M7 架构决策：SQL 执行管道

- **决策**: 创建 `pipeline.rs` 实现完整的 SQL→Response 转换管道
- **原因**:
  - 统一 parse→plan→execute→collect→Response 流程
  - 从 Statement 提取表名 → 在 TableManager 中查找 → 动态注册到 PlanBuilder
  - `value_to_json()` 将 executor::Value 映射到 serde_json::Value
- **影响**:
  - `SqlHandler` 变得极简：仅持有 `Arc<Database>`，`execute()` 委托给 pipeline
  - 新增 PhysicalPlan 节点类型时只需更新 pipeline 的 match arm

### 2026-05-20 - M6 架构决策：Graceful Shutdown

- **决策**: 使用 tokio_util::sync::CancellationToken 实现 graceful shutdown
- **原因**:
  - 协程安全：不强制中断正在处理的连接
  - 多点监听：Server、ConnectionHandler 都可监听 shutdown signal
  - tokio::select!：优雅处理 accept + shutdown 并发
- **影响**:
  - Ctrl+C 安全：服务器优雅关闭
  - 测试友好：测试用例可通过 shutdown.cancel() 停止服务器
  - 扩展性强：后续可添加超时、等待连接完成等功能

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

### 2026-05-21 - M9 Phase 2 架构决策：两算子分离（SortExecutor + LimitExecutor）

- **决策**: ORDER BY + LIMIT/OFFSET 采用两算子分离方案（SortExecutor + LimitExecutor）
- **原因**:
  - 职责单一，符合现有 FilterExecutor 包装器模式
  - Plan 节点清晰（SortNode + LimitNode）
  - 可独立测试（SortExecutor test, LimitExecutor test）
- **影响**:
  - SortExecutor 必须收集全部数据后排序（流水线中断）
  - LimitExecutor 增量计数处理（无需全收集）
  - Pipeline 递归创建 Executor（Limit(Sort(Filter(Scan)))）
- **替代方案**:
  - SortLimitExecutor 合并算子（rejected：职责不单一，难以测试）
- **Pipeline 示例**:
```
SELECT ... WHERE age > 28 ORDER BY age DESC LIMIT 10 OFFSET 5

Plan:
  Limit(limit=10, offset=5)
    → Sort(order_by=[OrderByColumn("age", false)])
      → Filter(predicate=ComparisonPredicate(age > 28))
        → Scan(table="users", columns=["id", "name", "age"])

Executor 递归创建:
  LimitExecutor(input=SortExecutor, limit=10, offset=5)
    → SortExecutor(input=FilterExecutor, order_by=[...], columns=[...])
      → FilterExecutor(input=ScanExecutor, predicate=...)
        → ScanExecutor(table_meta, buffer_pool)
```

### 2026-05-21 - M9 Phase 2 架构决策：SortExecutor 列名映射

- **决策**: SortExecutor 添加 `columns: Vec<String>` 字段，compare_rows 通过列名查找索引
- **原因**:
  - ORDER BY 列名与 Scan 返回行顺序可能不一致
  - 例如 `SELECT name, age FROM users ORDER BY age`，age 在行索引 1 而非 0
  - 需列名→索引映射才能正确比较
- **影响**:
  - PlanBuilder 需传递 SELECT projection 列名到 SortNode.columns
  - compare_rows 使用 `self.columns.iter().position(|c| c == order_col.column)`
  - 支持任意列顺序的排序
- **实现关键**:
```rust
pub struct SortNode {
    pub input: Box<PhysicalPlan>,
    pub order_by: Vec<OrderByColumn>,
    pub columns: Vec<String>,  // SELECT projection 列名列表
    pub table_name: String,
}

fn compare_rows(&self, a: &[Value], b: &[Value]) -> Ordering {
    for order_col in &self.order_by {
        // 查找列名在 columns 中的索引
        let col_idx = self.columns.iter()
            .position(|c| c.to_lowercase() == order_col.column.to_lowercase());
        
        if let Some(idx) = col_idx {
            // 比较 a[idx] 和 b[idx]
        }
    }
}
```

### 2026-05-21 - M9 Phase 2 架构决策：NULL 排末尾

- **决策**: NULL 值在排序中固定排在末尾（无论 ASC/DESC）
- **原因**:
  - SQL 标准常见行为（PostgreSQL 默认）
  - 嵌入式数据库简单默认足够
  - NULLS FIRST/LAST 语法推迟到后续优化
- **影响**:
  - compare_values 处理 NULL：`(Null, non-Null) → Ordering::Greater`
  - 排序后 NULL 值出现在末尾
  - 测试验证 NULL 排序行为
- **实现**:
```rust
fn compare_values(a: &Value, b: &Value) -> Ordering {
    // NULL handling: NULL sorts to end
    (Value::Null, Value::Null) => Ordering::Equal,
    (Value::Null, _) => Ordering::Greater,  // NULL > non-NULL
    (_, Value::Null) => Ordering::Less,     // non-NULL < NULL
    // ... 其他类型比较
}
```

### 2026-05-21 - M9 Phase 2 架构决策：内存排序策略

- **决策**: SortExecutor 使用 `Vec::sort_unstable_by` 进行内存排序
- **原因**:
  - 嵌入式数据库数据量预期不大
  - 内存排序实现简单，性能足够
  - sort_unstable_by 比 sort 快（不保持相等元素顺序）
- **影响**:
  - 首次 next() 调用收集所有行到 Vec
  - 排序后逐行输出
  - 未来数据量大时可扩展外部排序（推迟到 M13）
- **替代方案**:
  - 外部排序（rejected：嵌入式场景可能不需要）
- **性能考虑**:
  - sort_unstable: O(n log n)，快于 sort
  - 内存分配: Vec clone 整行（可优化为 drain 或 VecDeque）

### 性能目标

- **海量连接**: 单台机器支持数万长连接，每个连接仅占用极少量内存
- **I/O 并发**: 磁盘利用率趋近 100%，协程自动调度
- **低延迟**: 请求延迟微秒级，无线程阻塞和唤醒开销
- **可伸缩性**: 工作线程数固定（等于 CPU 核数），避免上下文切换

### 开发路线图（重新规划）

> 嵌入式数据库核心功能优先级调整（2026-05-20）

| 里程碑 | 内容 | 优先级 | 异步相关重点 |
|--------|------|--------|-------------|
| M0 | 项目骨架，引入 Tokio | ✅ 完成 | 确定异步运行时配置 |
| M1 | 文件/缓存层 | ✅ 完成 | 实现 `AsyncStorage` trait，使用 `spawn_blocking` 读页 |
| M2 | B-Tree 索引与存储引擎 | ✅ 完成 | 索引同步，通过 `spawn_blocking` 暴露为 async API |
| M3 | 事务与 MVCC | ✅ 完成 | 用异步锁实现提交等待，快照读无锁 |
| M4 | SQL 解析与计划 | ✅ 完成 | 同步解析，生成物理计划（包含 async 节点） |
| M5 | 异步执行引擎 | ✅ 完成 | 实现 `async fn next()` 迭代器，整合存储异步接口 |
| M6 | 网络层 | ✅ 完成 | TCP 服务器 + Protocol trait + JSON 协议 + graceful shutdown |
| M7 | 数据存储层 + 全流程集成 | ✅ 完成 | 实现 TableManager、Row 数据存储、整合真实 executor + MVCC |
| M8 | PostgreSQL 协议 | ✅ 完成 | Simple Query Protocol + PgProtocol 状态机 |
| **M9** | **SQL 基础能力完善** | 🔴 **高** | DDL: CREATE TABLE/DROP TABLE + WHERE 表达式 + ORDER BY/LIMIT |
| **M10** | **MVCC 完整性** | 🟡 **中** | 完整版本链遍历 + 版本链 GC + Read Committed 隔离级别 |
| **M11** | **WAL 持久化** | 🔴 **高** | WAL 写入 + 重放恢复 + Checkpoint + 崩溃恢复 |
| **M12** | **JOIN 多表** | 🟢 **低** | INNER JOIN + LEFT/RIGHT JOIN + 多表 WHERE |
| M13 | 性能优化与完善 | 可选 | io_uring + 协程调度优化 + 性能基准测试 |

**优先级说明**:
- 🔴 高优先级: 嵌入式数据库核心必需功能（SQL 基础 + 持久化）
- 🟡 中优先级: 事务完整性保障（MVCC 完善）
- 🟢 低优先级: 扩展功能（JOIN，嵌入式场景可能单表为主）

**PostgreSQL 协议层（M8）**: 嵌入式数据库可能不需要外部连接，考虑分离或删除