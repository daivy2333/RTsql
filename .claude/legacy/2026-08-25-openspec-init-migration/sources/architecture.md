# Architecture — 架构决策记录

> 版本：v1.3 | 最后更新：2026-06-04（A011 M21 延后项完成更新）
> 由 openspec-init 从 `.claude/docs/architecture.md` 迁移。
> 条目格式: <!-- A{编号} --> ### {DATE} - {决策标题}

---

## Purpose

记录 RTsql 数据库的所有架构决策（ADR），确保设计决策有据可查，便于后续维护和新人理解。每个决策包含：决策内容、原因、代价、替代方案。

---

## Requirements

### Requirement: 架构决策可追溯

所有重大架构决策 SHALL 以 ADR 格式记录，包含决策、原因、代价、替代方案。

#### Scenario: 新增架构决策
- **WHEN** 做出影响系统架构的技术决策
- **THEN** 在本文件新增 ADR 条目，编号递增，包含完整四要素

#### Scenario: 查询已有决策
- **WHEN** 需要了解某个设计选择的原因
- **THEN** 通过 `grep "关键词" openspec/specs/architecture/spec.md` 定位对应 ADR

### Requirement: 系统架构可理解

系统整体架构 SHALL 以架构图和数据流图形式记录。

#### Scenario: 新成员了解架构
- **WHEN** 新开发者需要理解系统结构
- **THEN** 通过系统架构图和 PhysicalPlan 节点表快速建立全局认知

---

## 系统架构

```
┌──────────────┐
│   SQL Text   │
└──────┬───────┘
       ▼
┌──────────────┐     ┌──────────┐
│   Parser     │────▶│ PlanCache│ (LRU, SELECT only)
│ (sqlparser)  │     └──────────┘
└──────┬───────┘
       ▼
┌──────────────┐
│  PlanBuilder │───▶ PhysicalPlan
│ (register +  │     (19 种节点)
│  build_plan) │
└──────┬───────┘
       ▼
┌──────────────┐
│   Pipeline   │───▶ create_executor_from_plan (递归)
│              │
└──────┬───────┘
       ▼
┌──────────────────────────────────────┐
│         Volcano Executor Tree        │
│                                      │
│  Scan → Filter → Join → Aggregate   │
│       → Having → Sort → Limit       │
│  IndexScan → Insert/Update/Delete   │
│  SemiJoin → AntiJoin                │
│  SubqueryEval → DerivedScan         │
└──────────────────────────────────────┘
       │
       ▼
┌──────────────┐
│  Storage     │
│  BufferPool  │───▶ PageGuard (零拷贝/修改)
│  BTree       │───▶ AtomicPageId (async) + from_root (sync)
│  SlottedPage │───▶ 读: SlottedPageRef / 写: SlottedPage + compacting
└──────────────┘
```

---

## 决策列表

<!-- A001 --> ### ADR-001: 两层分离索引结构 (2026-05-23)

**决策**：RTsql 使用两层分离的索引结构（索引页 + 数据页），而非 SQLite 的聚簇索引。

**原因**：
- ✅ **灵活性**：支持多索引、非唯一索引模式（M17 已验证）
- ✅ **MVCC 友好**：数据页独立管理，版本链实现更简单
- ✅ **实现简洁**：避免聚簇索引的复杂性

**代价**：
- ❌ **空间开销**：文件大小 ~3x larger（索引页额外开销）
- ❌ **点查询路径**：索引页 → 数据页（两次页访问）

**验证结果**（M17.5）：PK lookup 5.6x faster than SQLite

**替代方案**：SQLite 聚簇索引（更紧凑但灵活性受限）、PostgreSQL 分离索引（类似架构）

---

<!-- A002 --> ### ADR-002: 固定长度 Key（32 bytes）(2026-05-23)

**决策**：B-Tree Key 使用固定 32 bytes 存储，而非变长编码。

**原因**：实现简洁、性能稳定、调试友好。
**代价**：短 Key 浪费 ~28 bytes，无法支持 >32 bytes 的 Key。
**后续优化**（M23）：Varint Key 编码减少 ~70% Key 开销。

---

<!-- A003 --> ### ADR-003: SlottedPage 页格式 + Logical Row ID (2026-05-24)

**决策**：使用标准 SlottedPage 格式，Slot 内含 logical_id 实现稳定行引用。

**Logical Row ID 设计**：
- Slot 6B：`{ logical_id: u16, offset: u16, length: u16 }`
- Header `next_logical_id: u16`（递增分配，永不回收）
- RowId.slot_id = logical_id（稳定跨 compact）

**原因**：标准格式、MVCC 友好、零拷贝读、引用稳定。
**代价**：Slot overhead 6 bytes/entry，页填充率 50-70%。

---

<!-- A004 --> ### ADR-004: 自定义二进制序列化 (2026-05-23)

**决策**：使用自定义二进制格式（Tag + Value），而非 JSON 或 Protobuf。

```
Int    = [Tag 0x01][8 bytes i64 LE]
String = [Tag 0x02][2 bytes len][N bytes UTF-8]
Null   = [Tag 0x03]
Float  = [Tag 0x04][8 bytes f64]
Bool   = [Tag 0x05][1 byte]
```

---

<!-- A005 --> ### ADR-005: 务实 Clippy warnings 清理策略 (2026-05-23)

**决策**：采用务实策略清理架构 warnings，平衡性能、安全、重构成本。

| Warning | 修复方案 |
|---------|----------|
| too_many_arguments | JoinConfig struct |
| type_complexity | type alias |
| await_holding_lock | #[allow] + 安全评估 |
| module_inception | #[allow] |

---

<!-- A006 --> ### ADR-006: IndexScanAllExecutor 非唯一索引 (2026-05-23)

**决策**：新增 IndexScanAllExecutor 处理非唯一索引扫描，而非扩展 IndexScanExecutor。
**关键特性**：惰性初始化 + MVCC 可见性迭代 + 逐行返回。

---

<!-- A007 --> ### ADR-007: WAL + Group Commit 架构 (2026-05-23)

**决策**：实现 WAL 机制，结合 Group Commit 优化 INSERT 性能（5-10x）。

**架构**：
```
Executor → WALBuffer → WalWriter::write_batch() → WALFile
触发：缓冲区满(100条) / 定时(100ms) / commit 通知
```

**核心组件**：WALRecord（BeginTxn/CommitTxn/AbortTxn）、WALBuffer（Notify + 后台 task）、Executor 隐式事务包装、RecoveryManager。

---

<!-- A008 --> ### ADR-008: B-Tree Merge 机制 (2026-05-24)

**决策**：实现 B-Tree 页合并机制，采用 redistribution-first 策略，配合 free-list 页复用。

**核心数据流**：
```
DELETE → delete_from_page
  ├─ Leaf: underflow? → redistribute → merge → MergeInfo ↑
  └─ Internal: recurse → remove_separator → underflow? → redistribute/merge → MergeInfo ↑
      → root shrink
```

---

<!-- A009 --> ### ADR-009: 事务 ID AtomicU64 无锁分配 (2026-06-03)

- **决策**：`TransactionId` 全局计数器使用 `AtomicU64` + `fetch_add(1, SeqCst)` 替代 `Mutex<u64>`。
- **原因**：
  - 事务 ID 分配是每事务 path 的热点，`Mutex` 锁开销显著（100ns+ 级）
  - 单线程 2.1x、10 线程 4.6x、100 线程 4.5x 加速
  - Rust `AtomicU64` 在 x86-64 上硬件保证正确性，`SeqCst` 比 `Relaxed` 更安全且几乎无开销
- **代价**：无 — 最终代码改动仅 1 行（`counter.fetch_add(1, SeqCst)`），因 main 已有 2yo 前缀的 AtomicU64 计数器
- **替代方案**：`Relaxed` 排序（更弱保证）+ `thread_local` 批量分配（更复杂，不必要）
- **实测验证**（4 场景 ns/op，详见 `specs/learned/spec.md` L017）：
| 场景 | Mutex | Atomic | 加速比 |
|------|-------|--------|--------|
| 单线程 1M | 10.7 | 5.1 | 2.1x |
| 10 线程争用 | 84.7 | 18.6 | 4.6x |
| 100 线程高争用 | 100.8 | 22.5 | 4.5x |
| 吞吐@1M (单线程) | 90.8 Melem/s | 138.1 Melem/s | 1.52x |
- **影响**：
  - 0 行为变化（仍是单调递增）
  - 0 API 变化（公开方法签名不变）
  - 微基准 `benches/tx_id_bench.rs` 建立 4 场景持续监控
  - 单线程下 Atomic 也快 2.1x（推翻 spec "单线程差异 < 20%" 假设 — 锁自身开销不可忽略）

<!-- A010 --> ### ADR-010: 网络响应批写缓冲 (2026-06-03)

- **决策**：PgProtocol 内嵌 `write_buf: Vec<u8>`（8KB），所有响应消息（DataRow、CommandComplete、ReadyForQuery 等）累积后单次 `write_all`+`flush`，不引入 BufWriter/BytesMut 外部依赖。
- **原因**：
  - 原实现每 DataRow 一次 `write_all()` + 尾部 `flush()`，N 行查询 = N+2+ syscall
  - 批写后 N 行查询 = 2 syscall（1 write + 1 flush），syscall 减少 99%+
  - Vec<u8> 已满足需求（`extend_from_slice`+`clear()`），无需引入 bytes/BufWriter 依赖
  - 不修改 Protocol trait，改动完全内聚于 PgProtocol
- **代价**：PgProtocol 内存占用 +8KB（缓冲区），每个连接独立持有
- **替代方案**：
  - A: tokio BufWriter 包裹 TcpStream — 需改 Protocol trait 签名，影响 JsonProtocol
  - B: bytes::BytesMut — 功能丰富但不必要，增加外部依赖

<!-- A011 --> ### ADR-011: 页面级 MVCC 可见性摘要 (2026-06-04)

- **决策**：引入 `PageVisibilityInfo`（`min_create_tx_id: u64 + all_visible: bool`），存储在 `BufferPool` 的 `DashMap<PageId, PageVisibilityInfo>` 中。读路径在逐行 VersionHeader 检查前先查 summary 快速判断；写路径（INSERT/DELETE/UPDATE/COMMIT）均清 `all_visible` 标志。
- **原因**：
  - 每行 22B VersionHeader 解析 + `is_visible` 调用约 50-100ns/行，30-100 行/页 → 1.5-10µs/page 可跳过
  - `DashMap` 分片无锁读，读路径几乎无额外开销
  - 纯内存优化，崩溃安全（丢失后自动降级为逐行检查）
  - 不持久化、不入 WAL
- **关键子决策**：
  1. 惰性设置 `all_visible` 延后 — 避免扫描设置 + 并发 INSERT 的竞态条件
  2. COMMIT 路径必须清标志 — `write_commit_tx_id` 后 commit_tx_id 从 None→Some，可见性变化
  3. `min_create_tx_id` 用于 `all_invisible_for`：`snapshot.tx_id < min_create_tx_id` → 整页不可见
  4. DELETE 通过 `mark_deleted()` 标记 version header（`DELETED_TX_ID` 哨兵值），DataScan 跳过已删除行
  5. 惰性 `set_all_visible`：`check_page_all_visible()` 三条件验证后设置，页面扫描结束时触发
- **代价**：`DashMap` entry 开销 ~50B/page，10K 页 ≈ 500KB 内存
- **替代方案**：
  - A: `RwLock<HashMap<>>` — 更简单但读互斥，10 线程争用瓶颈
  - B: 字节存在 `SlottedPageHeader` 内 — 改页格式影响所有读写，崩溃后数据可能过期
  - C: PostgreSQL 风格 VMB（visibility map fork）— 额外文件 + 持久化，过于复杂

---

<!-- A012 --> ### ADR-012: BufferPool DashMap + Miss Semaphore (2026-06-06)

- **决策**：`BufferPool.pages` 字段从 `RwLock<HashMap<PageId, Arc<Mutex<PageFrame>>>>` 迁移到 `DashMap<PageId, Arc<Mutex<PageFrame>>>`，并新增两个字段：
  - `miss_sem: Arc<Semaphore>`（固定容量 16）— bound in-flight miss load IO
  - `loading_locks: DashMap<PageId, Arc<tokio::sync::Mutex<()>>>` — per-page load 序列化
- **原因**：
  - M30 连接并发后，多线程 `get_page` 争用全表 RwLock 读锁加剧（实测 50-100ns/次 await）
  - DashMap 分片锁使 cache hit 路径完全 lock-free（~0ns/次 get）
  - miss Semaphore 防止突发 IO 风暴（100 线程同时 miss 不再触发 100 次并行 read）
  - per-page loading lock 保证 R3 double-check 正确性（同 page 8 并发 miss 只 1 次 read）
- **关键子决策**：
  1. `clock_hand` 保持 `RwLock<Vec<PageId>>` — 串行淘汰已够，引入 DashSet 无收益
  2. `evict_one` 签名去掉 `&mut HashMap<...>` 参数（DashMap 无全局借用）
  3. `flush_all` 重构为 collect-then-write（避免跨 await 持 DashMap iter 锁）
  4. **per-page loading lock 修正**：原 design 漏掉 — miss Semaphore 只 bound 总并发，不能保证同 page double-check 正确性。实施时发现并加 `loading_locks` 字段
  5. 锁顺序约定：`miss_sem → loading_lock → pages → clock_hand → frame`
  6. 公开 API 签名 0 变化（`get_page`, `with_page_data`, `free_page`, `flush_all`, `capacity`, `storage`）
- **代价**：
  - DashMap entry 开销 ~50B/page（已有 vis_map 同开销）
  - loading_locks 永远不清理（同 page 多次 cache miss 复用同一 lock；用 Arc 持有，~50B/page）
  - 多 1 个 permit acquire（~30-50ns，未满时立即返回）
- **替代方案**：
  - A: `RwLock<BTreeMap<>>` — 仍单锁，无收益
  - B: 纯 miss Semaphore 不加 per-page lock — R3 失败（8 并发 → 8 次 read_page）
  - C: 全局写锁保护 miss 路径 — 退化到全表锁，违背初衷
  - D: LRU cache crate — 引入依赖 + 改变淘汰策略
- **影响下游**：M22 (预取) 可扩展为 `prefetch_sem`；M35 (脏页 writev) 配合批量写盘


---

## 架构权衡总结

| 设计决策 | 空间代价 | 性能收益 | 灵活性收益 |
|----------|---------|---------|-----------|
| 两层分离索引 | ~3x larger | PK lookup 5.6x faster ⚡ | 多索引 ✅ |
| 固定 Key 32B | ~10x per key | CPU 开销低 ✅ | 实现简洁 ✅ |
| SlottedPage | ~1.4x larger | MVCC 无锁读 ⚡ | 版本链管理 ✅ |
| 二进制序列化 | ~1.2x larger | 比 JSON 快 ✅ | 无外部依赖 ✅ |

---

## PhysicalPlan 节点（19 种）

| 节点 | 输入 | 用途 |
|------|------|------|
| Scan | - | 全表扫描（索引路径） |
| DataScan | - | 数据页链表遍历（M19，跳过索引层） |
| IndexScan | - | 主键索引扫描 |
| IndexScanAll | - | 非唯一索引扫描（ADR-006） |
| Filter | 1 | WHERE 过滤 |
| Join | 2 | 哈希连接 |
| Aggregate | 1 | 聚合 + GROUP BY |
| Having | 1 | HAVING 过滤 |
| Sort | 1 | ORDER BY |
| Limit | 1 | LIMIT/OFFSET |
| SemiJoin | 2 | IN/EXISTS 子查询 |
| AntiJoin | 2 | NOT IN/NOT EXISTS |
| SubqueryEval | 1 | SELECT 标量子查询 |
| DerivedScan | 1 | FROM 子查询 |
| Insert/Update/Delete | - | DML |
| CreateTable/DropTable | - | DDL |

---

## 架构原则

1. **Volcano 迭代器模型**：算子可自由组合，扩展方便
2. **Tokio async 协程**：无栈协程轻量，适合 I/O 密集
3. **两阶段锁 BufferPool**：I/O 期间不持锁，避免阻塞
4. **AtomicPageId 无锁读**：async 路径避免死锁
5. **空间换简洁**：固定 Key、二进制序列化等选择优先实现简洁性
