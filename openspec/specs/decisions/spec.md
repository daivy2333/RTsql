## Purpose

记录 RTsql 项目的重要架构决策（ADR），包含决策、原因、代价、替代方案和状态。被替代的决策保留并标记 superseded。条目使用 `Dxx` 编号。

## Requirements

### Requirement: 决策可追溯

重要选择 SHALL 记录决定、原因、替代方案、影响、状态和关联模型。

#### Scenario: 接受长期选择

- **WHEN** 开发者确认跨模块、兼容性或长期设计选择
- **THEN** 使用递增 D 编号记录 accepted 决策

#### Scenario: 替代旧决策

- **WHEN** 新决策替代已有选择
- **THEN** 保留旧条目并标记 superseded 和替代编号

---

## D01: 两层分离索引结构

- **决策**: RTsql 使用两层分离索引结构（索引页 + 数据页），而非 SQLite 聚簇索引
- **日期**: 2026-05-23
- **原因**:
  - 灵活性：支持多索引、非唯一索引模式
  - MVCC 友好：数据页独立管理，版本链实现更简单
  - 实现简洁：避免聚簇索引复杂性
- **代价**:
  - 空间开销：文件大小 ~3x larger
  - 点查询路径：索引页 → 数据页两次访问
- **替代方案**:
  - SQLite 聚簇索引（更紧凑但灵活性受限）
  - PostgreSQL 分离索引（类似架构）
- **关联模型**: M02
- **状态**: accepted
- **Legacy**: A001

## D02: 固定 32 字节 Key 编码

- **决策**: B-Tree Key 使用固定 32 bytes 存储
- **日期**: 2026-05-23
- **原因**: 实现简洁、性能稳定、调试友好
- **代价**: 短 Key 浪费 ~28 bytes；无法支持 >32 bytes Key
- **后续优化**: M23 Varint Key（I024）
- **关联模型**: M03
- **状态**: accepted
- **Legacy**: A002

## D03: SlottedPage + Logical Row ID

- **决策**: 使用标准 SlottedPage 格式，Slot 内含 logical_id 实现稳定行引用
- **日期**: 2026-05-24
- **原因**: 标准格式、MVCC 友好、零拷贝读、引用稳定
- **代价**: Slot overhead 6 bytes/entry，页填充率 50-70%
- **关联模型**: M04
- **状态**: accepted
- **Legacy**: A003

## D04: 自定义二进制序列化格式

- **决策**: 使用自定义二进制格式（Tag + Value），非 JSON/Protobuf
- **日期**: 2026-05-23
- **原因**: 无外部依赖、实现简洁、性能可控
- **格式**: Int(0x01)/String(0x02)/Null(0x03)/Float(0x04)/Bool(0x05)
- **关联模型**: M05
- **状态**: accepted
- **Legacy**: A004

## D05: 务实 Clippy warnings 清理策略

- **决策**: 平衡性能、安全、重构成本的 warnings 清理策略
- **日期**: 2026-05-23
- **策略**:
  - too_many_arguments → JoinConfig struct
  - type_complexity → type alias
  - await_holding_lock → #[allow] + 安全评估
  - module_inception → #[allow]
- **状态**: accepted
- **Legacy**: A005

## D06: IndexScanAllExecutor 非唯一索引扫描

- **决策**: 新增独立 IndexScanAllExecutor 处理非唯一索引扫描，而非扩展 IndexScanExecutor
- **日期**: 2026-05-23
- **原因**: 避免 IndexScan 复杂化；非唯一索引特性独立（惰性初始化 + MVCC 可见性迭代）
- **关联模型**: M01
- **状态**: accepted
- **Legacy**: A006

## D07: WAL + Group Commit 架构

- **决策**: 实现 WAL 机制，结合 Group Commit 优化 INSERT 性能（5-10x）
- **日期**: 2026-05-23
- **架构**: Executor → WALBuffer → WalWriter::write_batch() → WALFile
- **触发条件**: 缓冲区满(100条) / 定时(100ms) / commit 通知
- **核心组件**: WALRecord（BeginTxn/CommitTxn/AbortTxn）、WALBuffer（Notify + 后台 task）、Executor 隐式事务包装、RecoveryManager
- **关联模型**: M06
- **状态**: accepted
- **Legacy**: A007

## D08: B-Tree Merge 机制（redistribution-first）

- **决策**: 实现 B-Tree 页合并机制，采用 redistribution-first 策略 + free-list 页复用
- **日期**: 2026-05-24
- **数据流**: DELETE → delete_from_page → Leaf/Internal → underflow? → redistribute → merge → MergeInfo → root shrink
- **状态**: accepted
- **Legacy**: A008

## D09: 事务 ID AtomicU64 无锁分配

- **决策**: `TransactionId` 全局计数器使用 `AtomicU64` + `fetch_add(1, SeqCst)` 替代 `Mutex<u64>`
- **日期**: 2026-06-03
- **原因**:
  - 事务 ID 分配是每事务 path 的热点，Mutex 锁开销显著（100ns+）
  - 实测：单线程 2.1x、10 线程 4.6x、100 线程 4.5x 加速
  - Rust `AtomicU64` 在 x86-64 上硬件保证正确性
- **代价**: 无（最终代码改动仅 1 行）
- **替代方案**:
  - Relaxed 排序（更弱保证）
  - thread_local 批量分配（更复杂，不必要）
- **实测数据** (4 场景 ns/op):

  | 场景 | Mutex | Atomic | 加速比 |
  |------|-------|--------|--------|
  | 单线程 1M | 10.7 | 5.1 | 2.1x |
  | 10 线程争用 | 84.7 | 18.6 | 4.6x |
  | 100 线程高争用 | 100.8 | 22.5 | 4.5x |
  | 吞吐@1M (单线程) | 90.8 Melem/s | 138.1 Melem/s | 1.52x |

- **影响**:
  - 0 行为变化（仍单调递增）
  - 0 API 变化（公开方法签名不变）
  - 单线程差异假设（< 20%）被推翻：实际 Atomic 在单线程下也 2x 快（锁自身开销不可忽略）
- **关联知识**: K03（AtomicU64 性能数据）, K16（M41 实施细节）
- **状态**: accepted
- **Legacy**: A009, L017

## D10: 网络响应批写缓冲

- **决策**: PgProtocol 内嵌 `write_buf: Vec<u8>`（8KB），所有响应消息累积后单次 `write_all`+`flush`
- **日期**: 2026-06-03
- **原因**:
  - 原实现每 DataRow 一次 `write_all()` + 尾部 `flush()`（N+2+ syscall）
  - 批写后 N 行查询 = 2 syscall（1 write + 1 flush），syscall -99%+
  - 不引入 BufWriter/BytesMut 外部依赖
- **代价**: PgProtocol 内存占用 +8KB
- **替代方案**:
  - tokio BufWriter 包裹 TcpStream（需改 Protocol trait）
  - bytes::BytesMut（增加外部依赖）
- **关联知识**: K06
- **状态**: accepted
- **Legacy**: A010, L020, L021

## D11: 页面级 MVCC 可见性摘要

- **决策**: 引入 `PageVisibilityInfo`（min_create_tx_id: u64 + all_visible: bool），存储在 `BufferPool` 的 `DashMap<PageId, PageVisibilityInfo>` 中
- **日期**: 2026-06-04
- **原因**:
  - 每行 22B VersionHeader 解析 + is_visible 调用约 50-100ns/行
  - 30-100 行/页 → 1.5-10µs/page 可跳过
  - DashMap 分片无锁读，崩溃安全（丢失后自动降级）
  - 纯内存优化，不持久化、不入 WAL
- **关键子决策**:
  1. 惰性设置 all_visible 延后（避免扫描设置 + 并发 INSERT 竞态）
  2. COMMIT 路径必须清标志（write_commit_tx_id 后可见性变化）
  3. min_create_tx_id 用于 all_invisible_for 判断
  4. DELETE 通过 mark_deleted() 标记 version header（DELETED_TX_ID 哨兵值）
  5. 惰性 set_all_visible：check_page_all_visible() 三条件验证后设置
- **代价**: DashMap entry 开销 ~50B/page，10K 页 ≈ 500KB 内存
- **替代方案**:
  - RwLock<HashMap<>>（10 线程争用瓶颈）
  - 字节存在 SlottedPageHeader 内（影响页格式 + 崩溃后过期）
  - PostgreSQL 风格 VMB fork（额外文件 + 持久化，过于复杂）
- **关联模型**: M10
- **关联知识**: K08, K09
- **状态**: accepted
- **Legacy**: A011, L028, L030, page-visibility-map spec

## D12: BufferPool DashMap + Miss Semaphore + Per-Page Loading Locks

- **决策**: `BufferPool.pages` 字段从 `RwLock<HashMap<...>>` 迁移到 `DashMap<...>>`，新增 miss Semaphore (16) + per-page loading_locks
- **日期**: 2026-06-06
- **原因**:
  - M30 连接并发后，多线程 get_page 争用全表 RwLock 读锁加剧
  - DashMap 分片锁使 cache hit 路径完全 lock-free（~0ns/次 get）
  - miss Semaphore 防止突发 IO 风暴
  - per-page loading lock 保证 R3 double-check 正确性（同 page 8 并发 miss 只 1 次 read）
- **关键子决策**:
  1. clock_hand 保持 RwLock<Vec<PageId>>（串行淘汰已够）
  2. evict_one 签名去掉 &mut HashMap<...> 参数
  3. flush_all 重构为 collect-then-write（避免跨 await 持 DashMap iter 锁）
  4. **per-page loading lock 修正**：原 design 漏掉 — miss Sem 只 bound 总并发，不能保证同 page double-check 正确性
  5. 锁顺序约定：miss_sem → loading_lock → pages → clock_hand → frame
  6. 公开 API 签名 0 变化
- **代价**:
  - DashMap entry 开销 ~50B/page
  - loading_locks 永远不清理（~50B/page Arc 持有）
  - 多 1 个 permit acquire（~30-50ns，未满时立即返回）
- **替代方案**:
  - RwLock<BTreeMap<>>（仍单锁无收益）
  - 纯 miss Semaphore 不加 per-page lock（R3 失败）
  - 全局写锁保护 miss 路径（退化到全表锁）
  - LRU cache crate（引入依赖 + 改变淘汰策略）
- **影响下游**: M22 (预取) 可扩展为 prefetch_sem；M35 (脏页 writev) 配合批量写盘
- **关联知识**: K10, K11
- **状态**: accepted
- **Legacy**: A012, L031, buffer-pool-concurrency spec
