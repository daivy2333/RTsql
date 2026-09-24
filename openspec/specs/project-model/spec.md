## Purpose

记录 RTsql 数据库的当前跨模块模型、组件边界和不变量。条目使用 `Mxx` 编号，按 domain/architecture/runtime 分类。

## Requirements

### Requirement: 项目模型可验证

跨模块约束 SHALL 记录范围、不变量、证据和状态。

#### Scenario: 稳定约束登记

- **WHEN** 已验证某项约束影响多个模块或后续变更
- **THEN** 使用递增 M 编号记录分类、范围、不变量、证据和状态

---

## M01: 整体系统架构与数据流

- **分类**: architecture
- **范围**: 整个数据库系统
- **不变量**:
  - SQL Text → Parser (sqlparser) → PlanCache (LRU, SELECT only) → PlanBuilder → PhysicalPlan (20 节点) → Pipeline → Volcano Executor Tree → Storage (BufferPool → PageGuard / BTree → AtomicPageId / SlottedPage)
  - 20 种 PhysicalPlan 节点：Scan / DataScan / IndexScan / IndexScanAll / Filter / Join / Aggregate / Having / Sort / Limit / SemiJoin / AntiJoin / SubqueryEval / DerivedScan / Projection / Insert / Update / Delete / CreateTable / DropTable（Projection 为 MS11-T01 新增——SELECT 派生列逐行求值节点）
- **证据**: `src/database.rs`, `src/pipeline.rs`, `src/parser/planner.rs`, `src/executor/mod.rs`, `src/storage/buffer_pool.rs`
- **状态**: active
- **Legacy**: A001-A012 系统架构图与节点表（来自 `openspec/specs/architecture/spec.md`）

## M02: 两层分离索引结构

- **分类**: architecture
- **范围**: B-Tree 索引层 + 数据页层
- **不变量**:
  - 索引页存 (key → row_id_pointer)，数据页存实际 row（独立管理）
  - 不采用 SQLite 聚簇索引模式
- **证据**: `src/storage/btree/`, `src/storage/data/table_manager.rs:51`（data_page_head）
- **状态**: active
- **影响**: PK lookup 5.6x faster than SQLite（M17.5 实测），但文件大小 ~3x larger
- **Legacy**: A001

## M03: 固定 32 字节 Key

- **分类**: architecture
- **范围**: B-Tree Key 编码
- **不变量**: B-Tree Key 固定 32 字节存储
- **代价**: 短 Key 浪费 ~28 bytes，无法支持 >32 bytes Key
- **后续优化**: M23 Varint Key（计划中，见 I024）
- **证据**: `src/storage/btree/`
- **状态**: active
- **Legacy**: A002

## M04: SlottedPage 页格式 + Logical Row ID

- **分类**: architecture
- **范围**: 数据页存储格式
- **不变量**:
  - Slot 6B = `{ logical_id: u16, offset: u16, length: u16 }`
  - Header `next_logical_id: u16`（递增分配，永不回收）
  - RowId.slot_id = logical_id（稳定跨 compact）
- **证据**: `src/storage/page_format/slotted_page.rs:21`（SlottedPageHeader）
- **代价**: Slot overhead 6 bytes/entry，页填充率 50-70%
- **状态**: active
- **Legacy**: A003

## M05: 自定义二进制序列化（Tag + Value）

- **分类**: architecture
- **范围**: tuple 字节布局
- **不变量**:
  - Int = [Tag 0x01][8 bytes i64 LE]
  - String = [Tag 0x02][2 bytes len][N bytes UTF-8]
  - Null = [Tag 0x03]
  - Float = [Tag 0x04][8 bytes f64]
  - Bool = [Tag 0x05][1 byte]
- **证据**: `src/executor/value.rs`, `src/storage/data_page.rs`
- **状态**: active
- **Legacy**: A004

## M06: 依赖关系与调用链

- **分类**: architecture
- **范围**: 模块依赖图
- **不变量**:
  ```
  Database → Pipeline → Executor Tree
                ↓
         BufferPool → PageGuard
                ↓
           BTree → LeafNode/InternalNode
                ↓
        IndexManager → AtomicPageId (async)
                ↓
         WalWriter → WALBuffer → WALFile
                ↓
      RecoveryManager → TransactionManager
  ```
  - 事务 ID：`TransactionId` 全局 AtomicU64 单调递增分配，无锁（原 D09 决策记录与 K16 实测数据随 2026-09-24 K/D 退役移入清理 carrier，经 arc 指引可解析）
- **证据**: `src/database.rs`, `src/storage/buffer_pool.rs`, `src/storage/btree/`, `src/wal/`
- **状态**: active
- **Legacy**: L013

## M08: AtomicPageId 无锁访问

- **分类**: architecture
- **范围**: B-Tree 根节点访问
- **不变量**: `AtomicU64::load(Acquire)` 异步无锁读取根页 ID，避免 async 路径中 std::sync::RwLock 死锁
- **证据**: `src/storage/btree/index_manager.rs`
- **状态**: active
- **Legacy**: L012, L002

## M09: 异步协程驱动调度核心

- **分类**: runtime
- **范围**: 整个 I/O 与执行模型
- **不变量**:
  - Tokio 多线程 scheduler 作为无栈协程运行时
  - 阻塞 I/O 通过 `spawn_blocking` 隔离（如 BTree 写操作）
  - async 路径不持 std::sync::Mutex 跨 .await
- **证据**: `Cargo.toml` (tokio features), `src/storage/btree/btree.rs` (BTree::from_root + spawn_blocking)
- **状态**: active
- **Legacy**: L012（技巧模式 "临时 BTree 实例"）

## M10: MVCC 可见性语义

- **分类**: domain
- **范围**: 事务隔离
- **不变量**:
  - 唯一支持的隔离级别：Repeatable Read（M24 计划新增 Read Committed + Serializable）
  - 读路径先查页级 `PageVisibilityInfo` 摘要（min_create_tx_id + all_visible），再回退到逐行 VersionHeader 检查
  - 不可见版本沿 `VersionHeader.next_version` 链查找
  - 纯内存优化，崩溃后自动降级为逐行检查（正确性不受影响）
- **证据**: `src/storage/page_visibility.rs`, `src/transaction/snapshot.rs`
- **状态**: active
- **Legacy**: A011, R007, M21 spec

## M11: API 路径索引（API 速查）

- **分类**: architecture
- **范围**: 公共 API 速查
- **不变量**:
  | 名称 | 路径 | 用途 |
  |---|---|---|
  | Database::open | src/database.rs | 打开/创建数据库 |
  | Database::execute_sql | src/database.rs | 执行 SQL 语句 |
  | BufferPool::get_page | src/storage/buffer_pool.rs | 获取页（两阶段锁） |
  | BufferPool::with_page_data | src/storage/buffer_pool.rs | 零拷贝页访问闭包 |
  | PageGuard::page_data | src/storage/page_frame.rs | 零拷贝读取页数据 |
  | PageGuard::modify_page | src/storage/page_frame.rs | 修改页数据（自动 dirty） |
  | IndexManager::search | src/storage/btree/index_manager.rs | Async search |
  | BTree::from_root | src/storage/btree/btree.rs | 临时实例（写操作） |
  | PlanBuilder::build | src/parser/planner.rs | SQL → PhysicalPlan |
  | Pipeline::execute | src/pipeline.rs | 执行管道入口 |
  | inject_correlated_values | src/executor/correlated.rs | 向谓词树注入外层列值 |
  | BTree::search_all | src/storage/btree/btree.rs | 返回所有匹配 RowId |
  | BTree::delete_by_key | src/storage/btree/btree.rs | 删除所有匹配 entries |
  | BTree::delete_exact | src/storage/btree/btree.rs | 精确删除 |
  | LeafNode::merge_right | src/storage/btree/node.rs | 吸收右兄弟 entries |
  | InternalNode::merge_right | src/storage/btree/node.rs | 吸收右兄弟 + 降级 separator |
  | FileStorage.free_pages | src/storage/file_storage.rs | Mutex<Vec<u64>> free-list |
  | Server::new | src/network/server.rs | 创建服务器（addr, db, max_connections） |
  | Server::shutdown_token | src/network/server.rs | 获取 CancellationToken 用于优雅关闭 |
  | TableMeta.data_page_head | src/storage/data/table_manager.rs:51 | 数据页链表头 |
  | SlottedPageHeader.next_page_id | src/storage/page_format/slotted_page.rs:21 | 数据页链表指针 |
  | IndexManager.scan_all | src/storage/btree/index_manager.rs:204 | BTree 全遍历 |
  | PageVisibilityInfo | src/storage/page_visibility.rs | 页面级可见性摘要 |
  | BufferPool::get_visibility | src/storage/buffer_pool.rs | 查询 visibility map |
  | BufferPool::update_visibility_on_insert | src/storage/buffer_pool.rs | INSERT 后更新可见性 |
  | BufferPool::clear_all_visible | src/storage/buffer_pool.rs | 写路径清标志 |
  | BufferPool::set_all_visible | src/storage/buffer_pool.rs | 惰性设置 |
  | DataScanExecutor 快速路径 | src/executor/data_scan.rs | 闭包外查 visibility_map |
- **状态**: active
- **Legacy**: L001

## M12: 文件速查索引

- **分类**: architecture
- **范围**: 关键文件路径索引
- **不变量**:
  | 名称 | 路径 | 用途 |
  |---|---|---|
  | database.rs | src/database.rs | Database 协调器 |
  | pipeline.rs | src/pipeline.rs | SQL 执行管道 |
  | buffer_pool.rs | src/storage/buffer_pool.rs | BufferPool（DashMap + miss Sem） |
  | slotted_page.rs | src/storage/page_format/slotted_page.rs | SlottedPage + compacting |
  | index_manager.rs | src/storage/btree/index_manager.rs | IndexManager（AtomicPageId） |
  | aggregate.rs | src/executor/aggregate.rs | AggregateFunc + AggregateState |
  | join.rs | src/executor/join.rs | JoinExecutor（哈希连接） |
  | semi_join.rs | src/executor/semi_join.rs | SemiJoinExecutorV2 |
  | anti_join.rs | src/executor/anti_join.rs | AntiJoinExecutor |
  | subquery_eval.rs | src/executor/subquery_eval.rs | SubqueryEvalExecutor |
  | correlated.rs | src/executor/correlated.rs | inject_correlated_values |
  | predicate.rs | src/executor/predicate.rs | Predicate/Expression + ParameterExpression |
  | planner.rs | src/parser/planner.rs | PlanBuilder（含子查询/关联检测） |
  | data_page.rs | src/storage/data_page.rs | 数据页读写 + VersionHeader |
  | table_manager.rs | src/storage/data/table_manager.rs | TableMeta（data_page_head/tail） |
  | page_visibility.rs | src/storage/page_visibility.rs | PageVisibilityInfo（页面级 MVCC 摘要） |
- **状态**: active
- **Legacy**: L002

## M13: 异步执行原则

- **分类**: runtime
- **范围**: async 代码风格
- **不变量**:
  - async 路径不持 std::sync::Mutex 跨 .await
  - 闭包内禁止 .await（FnOnce 非 async 编译期强制）
  - BufferPool 闭包内禁止递归调用其他 BufferPool 方法
  - BTree 写操作通过 `BTree::from_root()` + `spawn_blocking` 隔离
- **证据**: `src/storage/buffer_pool.rs`, `src/storage/btree/btree.rs`
- **状态**: active
- **Legacy**: L012（技巧模式）, L022

## M14: 数据库表与目录约定

- **分类**: domain
- **范围**: 代码组织
- **不变量**:
  - 源码目录：src/
  - 存储层：src/storage/（buffer_pool、btree、page_format、file_storage、data）
  - 执行器：src/executor/（每个执行器独立文件）
  - 解析器：src/parser/（planner、ast）
  - 测试目录：tests/（集成测试）+ 文件内 #[cfg(test)]（单元测试）
  - 基准测试：benches/（criterion）
- **状态**: active
- **Legacy**: 项目特定规范

## M15: 命名规范

- **分类**: domain
- **范围**: 代码风格
- **不变量**:
  - 模块名：snake_case（buffer_pool、slotted_page）
  - 类型名：PascalCase（PageGuard、WalRecord）
  - 常量：SCREAMING_SNAKE_CASE（MAX_RETRY_COUNT）
  - 布尔值：is_/has_/can_/should_ 前缀
  - 集合：复数形式（pages、slots）
- **状态**: active
- **Legacy**: 项目特定规范（命名规范）

## M17: BufferPool 并发模型（DashMap + Miss Semaphore + Per-Page Loading Locks）

- **分类**: architecture
- **范围**: 缓存并发与页加载路径
- **不变量**:
  - `pages: DashMap<PageId, Arc<Mutex<PageFrame>>>`——cache hit 路径 lock-free（整体替代原 RwLock<HashMap> 两阶段锁加载模式）
  - miss Semaphore（16 permits）约束并发页加载总量，防突发 IO 风暴
  - per-page `loading_locks: DashMap<PageId, Arc<tokio::sync::Mutex<()>>>`——同页并发 miss 仅一次 read_page（miss Sem 只限总量，不保证同页 double-check 正确性；per-page lock 才保证）
  - 锁顺序约定：`miss_sem.acquire() → loading_lock.lock() → pages.get() → clock_hand.read() → frame.lock()`；同序获取无环，异序可死锁
  - `flush_all` 采用 collect-then-write（DashMap iter 持分片读锁，await 不得持锁跨页写）
  - 淘汰保持 clock_hand `RwLock<Vec<PageId>>` 串行淘汰；公开 API 签名不变
- **证据**: `src/storage/buffer_pool.rs`（pages/vis_map/loading_locks 字段与 get_page）；原决策与设计修正全文见 2026-09-24 清理 carrier（D12/K10/K11，经 arc 指引可解析）
- **状态**: active
- **Legacy**: D12, K10, K11（2026-09-24 K/D 退役迁入）；取代 M07 两阶段锁历史模型（M07 已归档同 carrier）

## M18: 恢复路径索引去信任与重建不变量

- **分类**: domain
- **范围**: WAL 恢复 × B-Tree 索引一致性
- **不变量**:
  - BufferPool 页驱逐按 LRU 而非树拓扑刷盘——checkpoint 后的运行期修改使磁盘 B-Tree 同时含洞（已刷盘父页指向未刷盘子页，读为 `InvalidPageType`）与孤儿页（已刷盘但不可达）；任何 catalog root 同步策略都无法修复
  - `redo_count > 0`（不洁关闭）时恢复路径零消费磁盘索引树：Update `old_row_id` 由数据页自建的磁盘版本多映射按 `max{rid < record.row_id}` 派生（同键版本链 rid 序 == LSN 序，行锁串行化保证），重放后从最终数据页重建各表 PK 索引（链尾回溯 + 重复 PK 显式报错）经 `replace_index_manager` 换入
  - `redo_count == 0`（checkpoint-clean 关闭）时磁盘树可信（checkpoint 全量刷盘保证一致）
  - 数据页不受此影响——WAL 位置寻址重放本就以数据页为权威
  - 触碰驱逐/刷盘路径的性能改动（I031 撕裂树运行期根修、脏页批量写回类）实施前必须对照本条
- **证据**: `src/wal/recovery.rs::rebuild_pk_indexes`；MS10-T02 design D10 + 003/004-rework Cycle（归档 change `openspec/changes/archive/2026-09-08-2026-09-06-ms10-t02-file-lock-graceful-shutdown/`）；原全文见 2026-09-24 清理 carrier（K38，经 arc 指引可解析）
- **状态**: active
- **Legacy**: K38（2026-09-24 K/D 退役迁入）

<!-- arc: ARC-202609241843b --> 2 条已归档 (2026-09-24) → openspec/changes/archive/2026-09-24-ARC-202609241843b/proposal.md
