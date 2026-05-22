# 架构决策记录

> 最后更新：2026-05-22

## 架构概览

```
┌─────────────────────────────────────────────────────┐
│                    Database                          │
│  (BufferPool + TableManager + TxManager + WalWriter) │
├─────────────────────────────────────────────────────┤
│  Pipeline: SQL → Parser → PlanBuilder → Executor    │
├──────────┬──────────┬──────────┬───────────────────┤
│  Storage │ Executor │   WAL    │   Transaction      │
│          │          │          │                     │
│ BufferPool│ Scan    │ Writer   │  Manager            │
│ PageGuard │ Filter  │ Reader   │  Snapshot           │
│ BTree     │ Sort    │ Checkpoint│ VersionChain       │
│ DataPage  │ Limit   │ Recovery │  RowLock            │
│ FileStore │ Join    │          │                     │
│           │ Insert  │          │                     │
│           │ Update  │          │                     │
│           │ Delete  │          │                     │
├──────────┴──────────┴──────────┴───────────────────┤
│                  Network Layer                       │
│  PgProtocol → ConnectionHandler → Server             │
└─────────────────────────────────────────────────────┘
```

## 设计原则

1. **异步协程驱动**: 所有 I/O 操作基于 Tokio 异步运行时
2. **页式存储**: 4KB 固定页大小，SlottedPage 格式
3. **MVCC**: 快照隔离（Repeatable Read），版本链 + 行锁
4. **WAL**: Write-Ahead Logging，崩溃恢复保证
5. **零拷贝读取**: page_data() + SlottedPageRef，避免 4KB 克隆
6. **两阶段锁**: BufferPool 释放写锁后再做 I/O

## 决策记录

### ADR-001: Tokio 异步运行时

- **日期**: 2026-05-15
- **决策**: 使用 Tokio 多线程调度器
- **原因**: Rust 生态最成熟的异步运行时，支持 I/O 密集型工作负载
- **影响**: 所有 I/O 操作使用 async/await，CPU 密集操作用 spawn_blocking

### ADR-002: SlottedPage 格式

- **日期**: 2026-05-16
- **决策**: 使用 SlottedPage 格式存储数据
- **原因**: 支持变长记录，删除操作只需标记 slot，不需要移动数据
- **影响**: 页内数据从后向前增长，slot 从前向后增长

### ADR-003: Clock 淘汰算法

- **日期**: 2026-05-16
- **决策**: BufferPool 使用 Clock（近似 LRU）淘汰
- **原因**: 实现简单，近似 LRU 效果，无需精确 LRU 的开销
- **影响**: 淘汰精度略低于 LRU，但实现和维护成本低

### ADR-004: MVCC 快照隔离

- **日期**: 2026-05-18
- **决策**: 使用版本链实现 MVCC，Repeatable Read 隔离级别
- **原因**: 读不阻塞写，写不阻塞读，适合高并发场景
- **影响**: 每行 22B VersionHeader 开销，需要定期清理旧版本

### ADR-005: 哈希连接

- **日期**: 2026-05-21
- **决策**: INNER JOIN 使用哈希连接（Hash Join）
- **原因**: 等值连接场景下哈希连接性能最优，O(M+N) 复杂度
- **影响**: 需要内存构建哈希表，不适合非等值连接

### ADR-006: PageGuard 零拷贝

- **日期**: 2026-05-22
- **决策**: 添加 page_data() + SlottedPageRef 零拷贝读取
- **原因**: page() 每次克隆 4KB，读操作不需要修改页数据
- **影响**: 读操作避免 4KB 分配，scan/filter/sort/limit 改善 5-15%
- **替代方案**: 直接返回 &Page 引用（生命周期管理复杂）

### ADR-007: BufferPool 两阶段锁

- **日期**: 2026-05-22
- **决策**: get_page() 释放写锁后再做 I/O
- **原因**: 持写锁期间做 I/O 阻塞其他协程，降低并发性能
- **影响**: I/O 期间其他协程可正常访问缓存页，并发读改善约 5%
- **风险**: I/O 完成后需 double-check 页是否已被其他协程加载

### ADR-008: std::sync::Mutex for PageFrame

- **日期**: 2026-05-22
- **决策**: PageFrame 使用 std::sync::Mutex 而非 tokio::sync::Mutex
- **原因**: 页操作是 CPU 密集型（非 I/O），临界区极短，std::sync::Mutex 开销更低
- **约束**: **绝不跨 .await 持有**，SAFETY 注释标记所有锁获取点
- **影响**: 页操作性能更优，但需要严格审查 .await 使用

## 数据流

### SQL 执行流程

```
SQL String
  → sqlparser::parse_sql()
  → PlanBuilder::build()
  → PhysicalPlan (tree of 13 node types)
  → Pipeline::execute()
  → Executor tree (volcano model: open/next/close)
  → Response (rows affected / result set)
```

### 页读取流程（零拷贝）

```
BufferPool::get_page(page_id)
  → [read-lock] check cache → hit → PageGuard
  → [write-lock] double-check → miss → release lock
  → [no lock] read_page_from_disk()
  → [write-lock] insert into cache → PageGuard
  → PageGuard::page_data() → PageDataGuard (Deref to &[u8])
  → SlottedPageRef::new(&data) → read slots (zero-copy)
```

### 事务流程

```
BEGIN
  → TxManager::begin() → TxId + Snapshot
  → [all subsequent ops use this TxId]

COMMIT
  → TxManager::commit(tx_id) → validate + cleanup

ROLLBACK
  → TxManager::rollback(tx_id) → undo all changes
```

## 关键约束

| 约束 | 原因 | 影响 |
|------|------|------|
| 4KB 固定页大小 | 对齐磁盘 I/O | 单行数据不能超过 ~4000 字节 |
| std::sync::Mutex 不跨 .await | 死锁风险 | PageGuard 生命周期 < .await 边界 |
| spawn_blocking for BTree | CPU 密集 | B-Tree 操作不在 async 上下文 |
| VersionHeader 22B | MVCC 开销 | 每行额外 22 字节 |
| 单文件持久化 | 嵌入式场景 | 并发写入受限于单文件锁 |