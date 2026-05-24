# M18 Phase3: WAL 集成 + Group Commit + 崩溃恢复 设计文档

> 日期：2026-05-23
> 状态：Draft → Pending Review
> 范围：WAL 与 Executor/TransactionManager 集成、WALBuffer + Group Commit、Redo-only 崩溃恢复

---

## 1. 背景与目标

### 1.1 现有基础（M11 已完成）

| 组件 | 状态 | 文件位置 |
|------|------|----------|
| WalRecord | 已实现（6 种类型） | `src/wal/record.rs` |
| WalWriter | 已实现（追加写入 + fsync） | `src/wal/writer.rs` |
| WalReader | 已实现（读取 + 迭代） | `src/wal/reader.rs` |
| CheckpointManager | 已实现（位点读写 + 刷脏 + 截断） | `src/wal/checkpoint.rs` |
| RecoveryManager | 部分实现（仅返回 committed/aborted 集合） | `src/wal/recovery.rs` |
| Database 集成 | 已集成（open 时调用 recovery，但忽略结果） | `src/database.rs` |

### 1.2 缺失部分（本次 Phase3 目标）

1. **WALBuffer**：内存缓冲区，缓存 WAL 记录，批量刷盘（核心 Group Commit 机制）
2. **Group Commit 策略**：缓冲区满 + 定时 + commit 信号触发刷盘
3. **Executor 集成**：Insert/Update/Delete Executor 写 WAL 记录
4. **TransactionManager 集成**：commit 写 WAL Commit + fsync，abort 写 WAL Abort
5. **RecoveryManager 实际数据重放**：Redo committed 事务的 Insert/Update/Delete
6. **未提交事务清理**：崩溃恢复时清理 uncommitted 事务的脏数据页

### 1.3 性能目标

- INSERT 性能：5-10x faster（相比当前无 WAL 的同步写入）
- 机制：Group Commit 将 N 次 fsync 合并为 1 次

---

## 2. 架构设计（方案 A：分层集成）

### 2.1 整体架构

```
Executor (Insert/Update/Delete)
    ↓ 写操作
TransactionManager::commit()
    ↓ 1. 写 WAL 记录到 WALBuffer
    ↓ 2. 写 Commit 记录到 WALBuffer
    ↓ 3. 等待 WALBuffer flush 通知
WALBuffer (内存缓冲)
    ↓ Group Commit 触发
    ↓ 缓冲区满(100条) / 定时(100ms) / commit 通知
WALWriter::write_batch()
    ↓ 批量写入 WAL 文件
    ↓ fsync
WALFile (持久化)

恢复流程：
Database::open()
    ↓
RecoveryManager::recover()
    ↓ 读取 WAL 文件
    ↓ 识别 committed / aborted / uncommitted 事务
    ↓ Redo committed 事务的数据操作
    ↓ 清理 uncommitted 事务的脏数据页
    ↓ 返回恢复结果
Database 继续运行
```

### 2.2 数据流：INSERT 示例

```
1. InsertExecutor::next() → 写入数据页
2. InsertExecutor → WALBuffer::append(WalRecord::Insert { ... })
3. 所有行写入完成 → TransactionManager::commit()
4. TM → WALBuffer::append(WalRecord::Commit { txn_id, timestamp })
5. TM → WALBuffer::flush_and_wait(txn_id)  // 等待 Commit 记录持久化
6. WALBuffer 收到 flush 请求
   → 将缓冲区中所有记录批量写入 WalWriter
   → WalWriter::write_batch() + fsync()
   → 通知等待的 txn_id
7. TM 收到确认 → 标记事务 committed → 返回成功
```

### 2.3 数据流：崩溃恢复

```
1. Database::open(path)
2. RecoveryManager::recover(wal_path)
   → WalReader 读取所有 WAL 记录
   → 扫描得到三个集合：committed_txns, aborted_txns, uncommitted_txns
3. Redo committed_txns 的 Insert/Update/Delete 操作
   → 重新写入数据页（使用 BufferPool）
4. 清理 uncommitted_txns 的脏数据页
   → 读取数据页，删除 create_tx_id 属于 uncommitted 集合的 tuple
5. CheckpointManager::truncate_wal()  // 恢复完成后截断旧 WAL
6. 返回恢复结果
```

---

## 3. WAL 记录扩展

### 3.1 新增记录类型

```rust
pub enum WalRecord {
    // M11 已有
    Insert { txn_id: TxnId, table_name: String, row_id: RowId, tuple_data: Vec<u8> },
    Update { txn_id: TxnId, table_name: String, row_id: RowId, old_tuple: Vec<u8>, new_tuple: Vec<u8> },
    Delete { txn_id: TxnId, table_name: String, row_id: RowId },
    Checkpoint { lsn: u64, timestamp: u64 },

    // 新增
    BeginTxn { txn_id: TxnId },
    CommitTxn { txn_id: TxnId, timestamp: u64 },
    AbortTxn { txn_id: TxnId },
}
```

**变更说明**：
- `Commit` → `CommitTxn`：命名更明确
- `Abort` → `AbortTxn`：命名更明确
- 新增 `BeginTxn`：标记事务开始，Recovery 识别未完成事务

### 3.2 LSN（Log Sequence Number）

- 每条 WAL 记录携带全局单调递增的 LSN
- `WALBuffer` 维护 `current_lsn: AtomicU64`
- `Checkpoint` 记录的 LSN 表示"此 LSN 之前的记录已刷盘"，加速恢复

### 3.3 CRC32 校验

- 每条记录末尾添加 4 字节 CRC32
- Recovery 读取时验证 CRC，损坏则停止读取
- 格式：`[LSN: 8B][Len: 4B][Type: 1B][Body: variable][CRC32: 4B]`

---

## 4. WALBuffer + Group Commit

### 4.1 WALBuffer 结构

```rust
pub struct WALBuffer {
    buffer: Mutex<Vec<(u64, WalRecord)>>,  // (lsn, record)
    current_lsn: AtomicU64,
    wal_writer: Arc<WalWriter>,
    flush_notify: Notify,          // 通知后台 task 刷盘
    committed_txns: Mutex<Vec<TxnId>>,  // 等待 flush 的事务
    flush_results: Mutex<HashMap<TxnId, Arc<Notify>>>,  // 事务等待结果
    shutdown: AtomicBool,
    capacity: usize,               // 缓冲区容量（默认 100 条）
    flush_interval_ms: u64,         // 定时刷盘间隔（默认 100ms）
}
```

### 4.2 核心 API

```rust
impl WALBuffer {
    /// 追加 WAL 记录，返回 LSN
    pub async fn append(&self, record: WalRecord) -> u64;

    /// 追加 Commit 记录并等待持久化确认
    /// 这是 Group Commit 的核心：多个并发事务的 Commit 记录
    /// 会被批量刷盘，从而合并 fsync
    pub async fn append_commit_and_wait(&self, txn_id: TxnId) -> Result<u64>;

    /// 通知后台 task 有 commit 需要刷盘
    fn notify_flush(&self);

    /// 后台刷盘 task（tokio::spawn）
    async fn flush_loop(self: Arc<Self>);

    /// 执行一次刷盘
    async fn do_flush(&self);
}
```

### 4.3 Group Commit 触发策略

| 触发条件 | 行为 | 性能影响 |
|----------|------|----------|
| 缓冲区满（100条） | 立即刷盘 | 高并发时自动批量化 |
| commit 请求 | 唤醒后台 task，下一次循环立即刷盘 | 低并发时也能及时提交 |
| 定时（100ms） | 后台 task 定期检查并刷盘 | 无并发时不丢记录 |
| shutdown | 刷盘所有缓冲记录 | 安全关闭 |

### 4.4 flush_and_wait 机制

```
1. TM 调用 append_commit_and_wait(txn_id)
2. WALBuffer 记录 txn_id 到 committed_txns
3. WALBuffer 创建 Notify 并存入 flush_results
4. WALBuffer 发送 flush_notify 信号
5. 后台 task 被唤醒 → do_flush()
6. do_flush() 将所有缓冲记录写入 WalWriter + fsync
7. do_flush() 通知所有 committed_txns 对应的 Notify
8. TM 的 await 收到通知 → commit 完成
```

**关键点**：多个并发事务的 Commit 记录会在同一次 fsync 中持久化，实现 Group Commit。

**注意**：`append_commit_and_wait` 只负责"注册等待 + 通知刷盘"，不写入 Commit 记录。Commit 记录由 TM 在调用前先通过 `append()` 写入。

---

## 5. Executor 集成

### 5.1 InsertExecutor

```rust
pub struct InsertExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    index_manager: Arc<IndexManager>,
    wal_buffer: Arc<WALBuffer>,     // 新增
    values: Vec<Vec<Value>>,
    rows_inserted: usize,
    txn_id: TxnId,                  // 新增
}
```

**变更**：`next()` 写入数据页后，同时调用 `wal_buffer.append(WalRecord::Insert { ... })`

**注意**：WAL 记录在 commit 前写入，但 commit 标记保证只有 committed 事务的记录会被 redo。这符合 WAL 原则：日志先于数据持久化。

### 5.2 UpdateExecutor / DeleteExecutor

类似模式：
- 持有 `wal_buffer: Arc<WALBuffer>` 和 `txn_id: TxnId`
- `next()` 执行操作后写对应 WAL 记录

### 5.3 Pipeline 创建 Executor

`pipeline.rs` 中 `create_executor_from_plan` 需要将 `database.wal_buffer` 传入 Executor 构造函数。

---

## 6. TransactionManager 集成

### 6.1 commit 流程变更

```rust
pub async fn commit(&self, txn_id: TxnId) -> Result<()> {
    // 1. 写 WAL Commit 记录
    let timestamp = current_timestamp();
    self.wal_buffer.append(WalRecord::CommitTxn { txn_id, timestamp }).await;

    // 2. 等待 Commit 记录持久化（Group Commit）
    self.wal_buffer.append_commit_and_wait(txn_id).await?;

    // 3. 标记事务 committed（现有逻辑）
    self.mark_committed(txn_id);

    Ok(())
}
```

### 6.2 abort 流程变更

```rust
pub async fn abort(&self, txn_id: TxnId) -> Result<()> {
    // 1. 写 WAL Abort 记录
    self.wal_buffer.append(WalRecord::AbortTxn { txn_id }).await;

    // 2. Abort 不需要等待 fsync（崩溃后可重新 abort）
    // 3. 清理 tx_versions（现有逻辑）
    self.cleanup_tx_versions(txn_id);

    Ok(())
}
```

### 6.3 begin 流程变更

```rust
pub async fn begin(&self) -> Result<TxnId> {
    let txn_id = self.next_txn_id();

    // 新增：写 WAL BeginTxn 记录
    self.wal_buffer.append(WalRecord::BeginTxn { txn_id }).await;

    Ok(txn_id)
}
```

---

## 7. RecoveryManager 实际数据重放

### 7.1 恢复流程

```rust
pub async fn recover(wal_path: &str, database: &Database) -> Result<RecoveryResult> {
    // 1. 读取所有 WAL 记录
    let records = WalReader::read_all(wal_path)?;

    // 2. 扫描得到事务分类
    let mut committed_txns = HashSet::new();
    let mut aborted_txns = HashSet::new();
    let mut all_txns = HashSet::new();
    let mut data_records = Vec::new();

    for record in records {
        match record {
            WalRecord::BeginTxn { txn_id } => { all_txns.insert(txn_id); }
            WalRecord::CommitTxn { txn_id, .. } => { committed_txns.insert(txn_id); }
            WalRecord::AbortTxn { txn_id } => { aborted_txns.insert(txn_id); }
            WalRecord::Insert { .. } | WalRecord::Update { .. } | WalRecord::Delete { .. } => {
                data_records.push(record);
            }
            _ => {}
        }
    }

    let uncommitted_txns = all_txns - &committed_txns - &aborted_txns;

    // 3. Redo committed 事务的数据操作
    for record in &data_records {
        let txn_id = record.txn_id();
        if committed_txns.contains(&txn_id) {
            redo_record(record, database).await?;
        }
    }

    // 4. 清理 uncommitted 事务的脏数据页
    cleanup_uncommitted(&uncommitted_txns, database).await?;

    // 5. 截断旧 WAL
    // CheckpointManager::truncate_wal(...)

    Ok(RecoveryResult { committed: committed_txns, aborted: aborted_txns, recovered: uncommitted_txns })
}
```

### 7.2 Redo 操作

```rust
async fn redo_record(record: &WalRecord, database: &Database) -> Result<()> {
    match record {
        WalRecord::Insert { table_name, row_id, tuple_data, .. } => {
            // 重新写入数据页（幂等操作：先检查 row_id 是否已存在）
            write_tuple_to_data_page(database, table_name, *row_id, tuple_data).await
        }
        WalRecord::Update { table_name, row_id, new_tuple, .. } => {
            update_tuple_in_data_page(database, table_name, *row_id, new_tuple).await
        }
        WalRecord::Delete { table_name, row_id, .. } => {
            delete_tuple_from_data_page(database, table_name, *row_id).await
        }
        _ => Ok(())
    }
}
```

### 7.3 清理 Uncommitted 事务

崩溃时可能有 uncommitted 事务的数据已写入数据页但未 commit。恢复时需要：

1. 扫描所有数据页，找到 `create_tx_id` 属于 `uncommitted_txns` 的 tuple
2. 删除这些 tuple（类似 abort 清理）

**简化方案**：由于 MVCC 版本链的存在，uncommitted 事务的 tuple 的 `commit_tx_id` 为 0（未标记 committed）。恢复时将这些 tuple 的 `commit_tx_id` 设为 `TxnId::MAX`（特殊标记值，表示 aborted），后续查询时 MVCC 可见性会自动跳过。

---

## 8. 错误处理

### 8.1 WAL 写入失败

- WALBuffer append 失败 → 返回错误给 Executor → 事务 abort
- fsync 失败 → 标记数据库为 read-only → 返回错误给所有等待事务

### 8.2 Recovery 损坏 WAL

- CRC 校验失败 → 停止读取 → 截断到最近的有效记录
- 损坏记录之后的所有记录视为丢失

### 8.3 Recovery 写入失败

- Redo 操作失败 → 记录错误 → 跳过该记录（可能数据已存在）
- 清理失败 → 记录警告 → 继续（MVCC 可见性会兜底）

---

## 9. 测试策略

### 9.1 单元测试

| 测试 | 覆盖 |
|------|------|
| WALBuffer append | 基本追加 |
| WALBuffer flush | 缓冲区满触发刷盘 |
| WALBuffer commit wait | append_commit_and_wait 确认 |
| Group Commit | 多事务并发 commit 合并 fsync |
| CRC 校验 | 损坏记录检测 |

### 9.2 集成测试

| 测试 | 覆盖 |
|------|------|
| INSERT → WAL | InsertExecutor 写 WAL 记录 |
| UPDATE → WAL | UpdateExecutor 写 WAL 记录 |
| DELETE → WAL | DeleteExecutor 写 WAL 记录 |
| Commit → WAL fsync | TM commit 写 Commit + 等待 fsync |

### 9.3 崩溃恢复 E2E 测试

| 测试 | 覆盖 |
|------|------|
| 正常关闭恢复 | 所有 committed 数据完整 |
| 崩溃恢复（committed） | 已 commit 未刷盘数据恢复 |
| 崩溃恢复（uncommitted） | 未 commit 数据被清理 |
| Checkpoint 加速恢复 | Checkpoint 后只重放增量 WAL |

### 9.4 性能基准测试

| 基准 | 度量 |
|------|------|
| INSERT with WAL + Group Commit | vs 无 WAL（预期 5-10x） |
| fsync 频率 | Group Commit 合并比 |
| 延迟分布 | P50/P95/P99 |

---

## 10. 配置参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `wal_buffer_capacity` | 100 | 缓冲区条目上限 |
| `wal_flush_interval_ms` | 100 | 定时刷盘间隔 |
| `wal_sync_mode` | "full" | "full"(fsync) / "normal"(fdatasync) / "off"(无sync)。**推迟到后续 milestone**，当前默认使用 fsync |
| `wal_checksum` | true | CRC32 校验 |

---

## 11. 实现顺序

按依赖关系从底层到上层：

1. **WalRecord 扩展**：新增 BeginTxn/CommitTxn/AbortTxn，添加 LSN + CRC
2. **WALBuffer 实现**：内存缓冲 + Group Commit 策略
3. **TransactionManager 集成**：begin/commit/abort 写 WAL
4. **Executor 集成**：Insert/Update/Delete 写 WAL
5. **RecoveryManager 数据重放**：Redo + 清理 uncommitted
6. **性能基准测试**：验证 5-10x INSERT 提速
7. **崩溃恢复 E2E 测试**：完整恢复流程验证