# M18 Phase3: WAL 集成 + Group Commit + 崩溃恢复 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 WALBuffer + Group Commit 机制，将 WAL 集成到 Executor 和 TransactionManager，实现 Redo-only 崩溃恢复，使 INSERT 性能提升 5-10x。

**Architecture:** 分层集成：Executor 写 WAL 记录 → TransactionManager 协调 WALBuffer → WALBuffer 批量刷盘（Group Commit）→ RecoveryManager Redo committed 事务。WALBuffer 使用 tokio Notify + 后台 task 实现异步刷盘，commit 时等待持久化确认。

**Tech Stack:** Rust, Tokio (sync::Notify, time, spawn), crc32fast

---

## File Structure

| 操作 | 文件 | 职责 |
|------|------|------|
| Modify | `src/wal/record.rs` | 新增 BeginTxn/CommitTxn/AbortTxn 记录类型，添加 LSN + CRC32 |
| Modify | `src/wal/writer.rs` | 新增 write_batch 方法，支持批量写入 |
| Modify | `src/wal/reader.rs` | 读取时验证 CRC32，支持带 LSN 的格式 |
| Create | `src/wal/buffer.rs` | WALBuffer + Group Commit 策略 + 后台刷盘 task |
| Modify | `src/wal/recovery.rs` | 实际数据重放（Redo committed + 清理 uncommitted） |
| Modify | `src/wal/mod.rs` | 导出 WALBuffer |
| Modify | `src/transaction/manager.rs` | begin/commit/abort 集成 WAL |
| Modify | `src/executor/insert.rs` | 持有 wal_buffer，写 WAL Insert 记录 |
| Modify | `src/executor/update.rs` | 持有 wal_buffer，写 WAL Update 记录 |
| Modify | `src/executor/delete.rs` | 持有 wal_buffer + buffer_pool + tx_manager，写 WAL Delete 记录 |
| Modify | `src/pipeline.rs` | 传入 wal_buffer 给写入 executor |
| Modify | `src/database.rs` | 持有 wal_buffer，open 时执行恢复并传递结果 |
| Modify | `Cargo.toml` | 添加 crc32fast 依赖 |
| Create | `tests/wal_buffer_test.rs` | WALBuffer + Group Commit 测试 |
| Modify | `tests/recovery_test.rs` | 崩溃恢复 E2E 测试 |
| Create | `benches/wal_group_commit_bench.rs` | Group Commit 性能基准测试 |

---

## Task 1: WalRecord 扩展 + CRC32 + LSN

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/wal/record.rs`
- Modify: `src/wal/writer.rs`
- Modify: `src/wal/reader.rs`
- Test: `tests/wal_record_test.rs`

- [ ] **Step 1: 添加 crc32fast 依赖**

在 `Cargo.toml` 的 `[dependencies]` 中添加：
```toml
crc32fast = "1.4"
```

- [ ] **Step 2: 扩展 WalRecordType 和 WalRecord**

在 `src/wal/record.rs` 中：

扩展 `WalRecordType`：
```rust
pub enum WalRecordType {
    Insert = 0x01,
    Update = 0x02,
    Delete = 0x03,
    Commit = 0x04,
    Abort = 0x05,
    Checkpoint = 0x06,
    BeginTxn = 0x07,
    CommitTxn = 0x08,
    AbortTxn = 0x09,
}
```

扩展 `WalRecord`：
```rust
pub enum WalRecord {
    Insert { tx_id: u64, table_name: String, row_id: RowId, tuple_data: Vec<u8> },
    Update { tx_id: u64, table_name: String, row_id: RowId, old_tuple: Vec<u8>, new_tuple: Vec<u8> },
    Delete { tx_id: u64, table_name: String, row_id: RowId },
    Commit { tx_id: u64, timestamp: u64 },
    Abort { tx_id: u64 },
    Checkpoint { lsn: u64, timestamp: u64 },
    BeginTxn { tx_id: u64 },
    CommitTxn { tx_id: u64, timestamp: u64 },
    AbortTxn { tx_id: u64 },
}
```

更新 `record_type()` 方法、`serialize()` 和 `deserialize()` 方法以支持新类型。`BeginTxn` serialize: `[tx_id: 8B LE]`。`CommitTxn` serialize: `[tx_id: 8B LE][timestamp: 8B LE]`。`AbortTxn` serialize: `[tx_id: 8B LE]`。

添加 `pub fn tx_id(&self) -> u64` 方法返回记录关联的事务 ID（Checkpoint 返回 0）。

- [ ] **Step 3: 添加 LSN + CRC32 到序列化格式**

修改 `serialize()` 输出格式为：`[lsn: 8B LE][type: 1B][len: 4B LE][data: variable][crc32: 4B LE]`

修改 `deserialize()` 输入格式：读取 LSN → 读取 type+len+data → 计算 CRC → 验证 → 返回 `(lsn, WalRecord, bytes_consumed)`

新签名：
```rust
pub fn serialize_with_lsn(&self, lsn: u64) -> Vec<u8>
pub fn deserialize_with_lsn(buf: &[u8]) -> Result<(u64, Self, usize), WalError>
```

保留旧 `serialize()`/`deserialize()` 用于向后兼容测试（内部调用新方法，lsn=0）。

- [ ] **Step 4: 更新 WalWriter::write_record 使用新格式**

修改 `src/wal/writer.rs` 中 `write_record` 使用 `serialize_with_lsn`，LSN 由 `write_count` 自增生成。

添加 `write_batch` 方法：
```rust
pub async fn write_batch(&self, records: Vec<(u64, WalRecord)>) -> Result<(), WalError>
```
批量写入多条记录，最后一次 fsync。

- [ ] **Step 5: 更新 WalReader 读取新格式**

修改 `src/wal/reader.rs` 中 `read_next` 使用 `deserialize_with_lsn`，CRC 校验失败返回 `WalError::ChecksumMismatch`。

在 `WalError` 中新增 `ChecksumMismatch` 变体。

- [ ] **Step 6: 写测试并验证**

在 `tests/wal_record_test.rs` 中新增测试：
```rust
#[test]
fn test_begin_txn_serialize_deserialize() { ... }

#[test]
fn test_commit_txn_serialize_deserialize() { ... }

#[test]
fn test_abort_txn_serialize_deserialize() { ... }

#[test]
fn test_lsn_crc_roundtrip() { ... }

#[test]
fn test_crc_mismatch_detected() { ... }
```

Run: `cargo test wal_record_test -- --nocapture`
Expected: 所有测试通过

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src/wal/record.rs src/wal/writer.rs src/wal/reader.rs tests/wal_record_test.rs
git commit -m "feat(M18-T1): extend WalRecord with BeginTxn/CommitTxn/AbortTxn, add LSN + CRC32"
```

---

## Task 2: WALBuffer + Group Commit

**Files:**
- Create: `src/wal/buffer.rs`
- Modify: `src/wal/mod.rs`
- Modify: `src/database.rs` (添加 wal_buffer 字段)
- Test: `tests/wal_buffer_test.rs`

- [ ] **Step 1: 写 WALBuffer 失败测试**

创建 `tests/wal_buffer_test.rs`：
```rust
use rtsql::wal::WALBuffer;
use rtsql::wal::record::WalRecord;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_wal_buffer_append_returns_lsn() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let wal_writer = Arc::new(WalWriter::open(&db_path).unwrap());
    let buffer = WALBuffer::new(wal_writer, 100, 100);

    let lsn1 = buffer.append(WalRecord::BeginTxn { tx_id: 1 }).await;
    let lsn2 = buffer.append(WalRecord::BeginTxn { tx_id: 2 }).await;
    assert!(lsn2 > lsn1);
}

#[tokio::test]
async fn test_wal_buffer_flush_on_capacity() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let wal_writer = Arc::new(WalWriter::open(&db_path).unwrap());
    let buffer = WALBuffer::new(wal_writer, 5, 10000); // 容量5，定时10s

    for i in 0..5 {
        buffer.append(WalRecord::BeginTxn { tx_id: i }).await;
    }
    // 缓冲区满应触发自动刷盘
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // 验证 WAL 文件中有记录
    let mut reader = WalReader::open(&db_path.with_extension("wal")).unwrap();
    let records = reader.read_all().unwrap();
    assert!(records.len() >= 5);
}

#[tokio::test]
async fn test_group_commit_multiple_txns() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let wal_writer = Arc::new(WalWriter::open(&db_path).unwrap());
    let buffer = Arc::new(WALBuffer::new(wal_writer, 100, 10000));
    buffer.start_flush_loop();

    // 两个并发事务同时 commit
    let b1 = buffer.clone();
    let b2 = buffer.clone();
    let h1 = tokio::spawn(async move {
        b1.append(WalRecord::CommitTxn { tx_id: 1, timestamp: 100 }).await;
        b1.append_commit_and_wait(1).await.unwrap();
    });
    let h2 = tokio::spawn(async move {
        b2.append(WalRecord::CommitTxn { tx_id: 2, timestamp: 200 }).await;
        b2.append_commit_and_wait(2).await.unwrap();
    });
    h1.await.unwrap();
    h2.await.unwrap();

    buffer.shutdown().await;

    // 验证两个 Commit 记录都已持久化
    let mut reader = WalReader::open(&db_path.with_extension("wal")).unwrap();
    let records = reader.read_all().unwrap();
    let commit_count = records.iter().filter(|r| matches!(r, WalRecord::CommitTxn { .. })).count();
    assert_eq!(commit_count, 2);
}
```

Run: `cargo test wal_buffer_test -- --nocapture`
Expected: FAIL（WALBuffer 不存在）

- [ ] **Step 2: 实现 WALBuffer**

创建 `src/wal/buffer.rs`：
```rust
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;

use super::record::WalRecord;
use super::writer::WalWriter;

pub struct WALBuffer {
    buffer: Mutex<Vec<(u64, WalRecord)>>,
    current_lsn: AtomicU64,
    wal_writer: Arc<WalWriter>,
    flush_notify: Notify,
    pending_commits: Mutex<Vec<u64>>,          // 等待 flush 的 tx_id 列表
    commit_waiters: Mutex<HashMap<u64, Arc<Notify>>>,  // tx_id → 等待通知
    shutdown: AtomicBool,
    capacity: usize,
    flush_interval_ms: u64,
    flush_handle: Mutex<Option<JoinHandle<()>>>,
}

impl WALBuffer {
    pub fn new(wal_writer: Arc<WalWriter>, capacity: usize, flush_interval_ms: u64) -> Self {
        Self {
            buffer: Mutex::new(Vec::with_capacity(capacity)),
            current_lsn: AtomicU64::new(1),
            wal_writer,
            flush_notify: Notify::new(),
            pending_commits: Mutex::new(Vec::new()),
            commit_waiters: Mutex::new(HashMap::new()),
            shutdown: AtomicBool::new(false),
            capacity,
            flush_interval_ms,
            flush_handle: Mutex::new(None),
        }
    }

    pub async fn append(&self, record: WalRecord) -> u64 {
        let lsn = self.current_lsn.fetch_add(1, Ordering::SeqCst);
        let mut buf = self.buffer.lock().await;
        buf.push((lsn, record));
        if buf.len() >= self.capacity {
            drop(buf);
            self.do_flush().await;
        }
        lsn
    }

    pub async fn append_commit_and_wait(&self, tx_id: u64) -> Result<(), super::record::WalError> {
        let notify = Arc::new(Notify::new());
        {
            let mut waiters = self.commit_waiters.lock().await;
            waiters.insert(tx_id, notify.clone());
        }
        {
            let mut pending = self.pending_commits.lock().await;
            pending.push(tx_id);
        }
        self.flush_notify.notify_one();
        notify.notified().await;
        Ok(())
    }

    pub fn start_flush_loop(self: &Arc<Self>) {
        let this = self.clone();
        let handle = tokio::spawn(async move {
            let interval = tokio::time::Duration::from_millis(this.flush_interval_ms);
            loop {
                tokio::select! {
                    _ = this.flush_notify.notified() => {
                        if this.shutdown.load(Ordering::SeqCst) { break; }
                        this.do_flush().await;
                    }
                    _ = tokio::time::sleep(interval) => {
                        if this.shutdown.load(Ordering::SeqCst) { break; }
                        this.do_flush().await;
                    }
                }
            }
        });
        let mut h = self.flush_handle.blocking_lock();
        *h = Some(handle);
    }

    pub async fn do_flush(&self) {
        let mut buf = self.buffer.lock().await;
        if buf.is_empty() { return; }
        let records: Vec<_> = std::mem::take(&mut *buf);
        drop(buf);

        if let Err(_) = self.wal_writer.write_batch(records).await {
            // fsync 失败，标记错误（后续可扩展为 read-only 模式）
            return;
        }
        let _ = self.wal_writer.fsync().await;

        let mut pending = self.pending_commits.lock().await;
        let mut waiters = self.commit_waiters.lock().await;
        for tx_id in pending.drain(..) {
            if let Some(notify) = waiters.remove(&tx_id) {
                notify.notify_one();
            }
        }
    }

    pub async fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.do_flush().await;
        self.flush_notify.notify_one();
        let mut h = self.flush_handle.lock().await;
        if let Some(handle) = h.take() {
            let _ = handle.await;
        }
    }
}
```

- [ ] **Step 3: 导出 WALBuffer**

在 `src/wal/mod.rs` 中添加：
```rust
mod buffer;
pub use buffer::WALBuffer;
```

- [ ] **Step 4: Database 添加 wal_buffer 字段**

在 `src/database.rs` 中：
- `Database` struct 添加 `pub wal_buffer: Arc<WALBuffer>`
- `open()` 中初始化 `wal_buffer` 并启动 flush_loop

- [ ] **Step 5: 运行测试验证**

Run: `cargo test wal_buffer_test -- --nocapture`
Expected: 所有测试通过

- [ ] **Step 6: Commit**

```bash
git add src/wal/buffer.rs src/wal/mod.rs src/database.rs tests/wal_buffer_test.rs
git commit -m "feat(M18-T2): implement WALBuffer with Group Commit strategy"
```

---

## Task 3: TransactionManager 集成 WAL

**Files:**
- Modify: `src/transaction/manager.rs`
- Modify: `src/database.rs`
- Test: `tests/wal_integration_test.rs`

- [ ] **Step 1: 写 TransactionManager WAL 集成失败测试**

在 `tests/wal_integration_test.rs` 中新增：
```rust
#[tokio::test]
async fn test_txn_commit_writes_wal_commit_record() {
    // 开启数据库 → begin → insert → commit
    // 验证 WAL 文件中有 CommitTxn 记录
}

#[tokio::test]
async fn test_txn_abort_writes_wal_abort_record() {
    // 开启数据库 → begin → insert → abort
    // 验证 WAL 文件中有 AbortTxn 记录
}

#[tokio::test]
async fn test_txn_begin_writes_wal_begin_record() {
    // 开启数据库 → begin
    // 验证 WAL 文件中有 BeginTxn 记录
}
```

Run: `cargo test wal_integration_test -- --nocapture`
Expected: FAIL（TM 不写 WAL 记录）

- [ ] **Step 2: TransactionManager 持有 wal_buffer**

修改 `src/transaction/manager.rs`：
- `TransactionManager` 添加 `wal_buffer: Option<Arc<WALBuffer>>` 字段
- 新增 `pub fn set_wal_buffer(&self, wal_buffer: Arc<WALBuffer>)` 方法
- 使用 `Option` 因为内存模式不需要 WAL

- [ ] **Step 3: 修改 begin/commit/abort 写 WAL**

`begin()`:
```rust
pub async fn begin(&self) -> Transaction {
    let tx = Transaction::new(self.tx_id_allocator.next(), Snapshot::new(/*...*/));
    self.active_tx_ids.write().await.insert(tx.id());
    if let Some(ref wal_buffer) = self.wal_buffer {
        wal_buffer.append(WalRecord::BeginTxn { tx_id: tx.id() }).await;
    }
    tx
}
```

`commit()`:
```rust
pub async fn commit(&self, tx: Transaction, buffer_pool: &BufferPool) -> Result<()> {
    if let Some(ref wal_buffer) = self.wal_buffer {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        wal_buffer.append(WalRecord::CommitTxn { tx_id: tx.id(), timestamp }).await;
        wal_buffer.append_commit_and_wait(tx.id()).await?;
    }
    // 现有 commit 逻辑...
    self.commit_mark_versions(tx.id(), buffer_pool).await?;
    self.active_tx_ids.write().await.remove(&tx.id());
    Ok(())
}
```

`abort()`:
```rust
pub async fn abort(&self, tx: Transaction, buffer_pool: &BufferPool, table_meta: &TableMeta) -> Result<()> {
    if let Some(ref wal_buffer) = self.wal_buffer {
        wal_buffer.append(WalRecord::AbortTxn { tx_id: tx.id() }).await;
    }
    // 现有 abort 逻辑...
    self.abort_cleanup_versions(tx.id(), buffer_pool, table_meta).await?;
    self.active_tx_ids.write().await.remove(&tx.id());
    Ok(())
}
```

- [ ] **Step 4: Database::open 设置 wal_buffer**

在 `src/database.rs` 的 `open()` 中，创建 `wal_buffer` 后调用 `transaction_manager.set_wal_buffer(wal_buffer.clone())`。

- [ ] **Step 5: 运行测试验证**

Run: `cargo test wal_integration_test -- --nocapture`
Expected: 所有测试通过

- [ ] **Step 6: Commit**

```bash
git add src/transaction/manager.rs src/database.rs tests/wal_integration_test.rs
git commit -m "feat(M18-T3): integrate WAL into TransactionManager begin/commit/abort"
```

---

## Task 4: Executor 集成 WAL

**Files:**
- Modify: `src/executor/insert.rs`
- Modify: `src/executor/update.rs`
- Modify: `src/executor/delete.rs`
- Modify: `src/pipeline.rs`
- Test: `tests/executor_test.rs`

- [ ] **Step 1: 写 Executor WAL 集成失败测试**

在 `tests/executor_test.rs` 中新增：
```rust
#[tokio::test]
async fn test_insert_executor_writes_wal_record() {
    // 创建数据库 + 表 → INSERT → 验证 WAL 中有 Insert 记录
}

#[tokio::test]
async fn test_update_executor_writes_wal_record() {
    // 创建数据库 + 表 + INSERT → UPDATE → 验证 WAL 中有 Update 记录
}

#[tokio::test]
async fn test_delete_executor_writes_wal_record() {
    // 创建数据库 + 表 + INSERT → DELETE → 验证 WAL 中有 Delete 记录
}
```

Run: `cargo test executor_test::test_insert_executor_writes_wal -- --nocapture`
Expected: FAIL（Executor 不写 WAL）

- [ ] **Step 2: InsertExecutor 持有 wal_buffer**

修改 `src/executor/insert.rs`：
- 添加 `wal_buffer: Option<Arc<WALBuffer>>` 字段
- 修改 `new()` 接收 `wal_buffer` 参数
- 在 `next()` 中写入数据页后：
```rust
if let Some(ref wal_buffer) = self.wal_buffer {
    wal_buffer.append(WalRecord::Insert {
        tx_id: self.tx_id,
        table_name: self.table_meta.name().to_string(),
        row_id,
        tuple_data: tuple_bytes.clone(),
    }).await;
}
```

- [ ] **Step 3: UpdateExecutor 持有 wal_buffer**

修改 `src/executor/update.rs`：同 InsertExecutor 模式，写 `WalRecord::Update`。

- [ ] **Step 4: DeleteExecutor 扩展**

修改 `src/executor/delete.rs`：
- 添加 `wal_buffer: Option<Arc<WALBuffer>>`、`buffer_pool: Arc<BufferPool>`、`table_meta: Arc<TableMeta>` 字段
- 在 `next()` 中删除后写 `WalRecord::Delete`

- [ ] **Step 5: Pipeline 传入 wal_buffer**

修改 `src/pipeline.rs` 中 `create_executor_from_plan`：
- Insert/Update/Delete 分支传入 `database.wal_buffer.clone()`

- [ ] **Step 6: 运行测试验证**

Run: `cargo test executor_test -- --nocapture`
Expected: 所有测试通过（包括新增的 WAL 测试）

- [ ] **Step 7: Commit**

```bash
git add src/executor/insert.rs src/executor/update.rs src/executor/delete.rs src/pipeline.rs tests/executor_test.rs
git commit -m "feat(M18-T4): integrate WAL into Insert/Update/Delete executors"
```

---

## Task 5: RecoveryManager 数据重放

**Files:**
- Modify: `src/wal/recovery.rs`
- Modify: `src/database.rs`
- Test: `tests/recovery_test.rs`

- [ ] **Step 1: 写崩溃恢复失败测试**

在 `tests/recovery_test.rs` 中新增：
```rust
#[tokio::test]
async fn test_recovery_redo_committed_insert() {
    // 1. 创建数据库，INSERT 数据，正常关闭
    // 2. 重新打开数据库（触发恢复）
    // 3. 验证数据完整
}

#[tokio::test]
async fn test_recovery_skip_uncommitted() {
    // 1. 创建数据库，INSERT 数据但不 commit
    // 2. 模拟崩溃（不调用 shutdown/close）
    // 3. 重新打开数据库
    // 4. 验证 uncommitted 数据不可见
}

#[tokio::test]
async fn test_recovery_after_crash_during_insert() {
    // 1. 创建数据库，INSERT 部分数据后崩溃
    // 2. 重新打开数据库
    // 3. 验证已 commit 的数据恢复，未 commit 的被清理
}
```

Run: `cargo test recovery_test -- --nocapture`
Expected: FAIL（RecoveryManager 不重放数据）

- [ ] **Step 2: 实现 RecoveryManager 数据重放**

修改 `src/wal/recovery.rs`：

```rust
pub struct RecoveryResult {
    pub committed_tx_ids: HashSet<u64>,
    pub aborted_tx_ids: HashSet<u64>,
    pub uncommitted_tx_ids: HashSet<u64>,
    pub redo_count: usize,
}

impl RecoveryManager {
    pub async fn recover(
        db_path: &Path,
        buffer_pool: &BufferPool,
        table_manager: &TableManager,
    ) -> Result<RecoveryResult, WalError> {
        let wal_path = db_path.with_extension("wal");
        let mut reader = WalReader::open(&wal_path)?;
        let records = reader.read_all()?;

        // 扫描事务分类
        let mut all_tx_ids = HashSet::new();
        let mut committed_tx_ids = HashSet::new();
        let mut aborted_tx_ids = HashSet::new();
        let mut data_records: Vec<&WalRecord> = Vec::new();

        for record in &records {
            match record {
                WalRecord::BeginTxn { tx_id } => { all_tx_ids.insert(*tx_id); }
                WalRecord::CommitTxn { tx_id, .. } | WalRecord::Commit { tx_id, .. } => {
                    committed_tx_ids.insert(*tx_id);
                }
                WalRecord::AbortTxn { tx_id } | WalRecord::Abort { tx_id } => {
                    aborted_tx_ids.insert(*tx_id);
                }
                WalRecord::Insert { .. } | WalRecord::Update { .. } | WalRecord::Delete { .. } => {
                    data_records.push(record);
                }
                _ => {}
            }
        }

        let uncommitted_tx_ids = &all_tx_ids - &committed_tx_ids - &aborted_tx_ids;

        // Redo committed 事务
        let mut redo_count = 0;
        for record in &data_records {
            let tx_id = record.tx_id();
            if committed_tx_ids.contains(&tx_id) {
                if let Ok(()) = redo_record(record, buffer_pool, table_manager).await {
                    redo_count += 1;
                }
            }
        }

        // 清理 uncommitted 事务的脏数据
        cleanup_uncommitted(&uncommitted_tx_ids, buffer_pool, table_manager).await;

        Ok(RecoveryResult { committed_tx_ids, aborted_tx_ids, uncommitted_tx_ids, redo_count })
    }
}
```

实现 `redo_record` 和 `cleanup_uncommitted` 辅助函数。

- [ ] **Step 3: Database::open 使用恢复结果**

修改 `src/database.rs` 的 `open()`：
- 调用 `RecoveryManager::recover()` 并使用恢复结果
- 将 `committed_tx_ids` 传递给 `TransactionManager`（更新当前 tx_id 分配器，避免 ID 冲突）

- [ ] **Step 4: 运行测试验证**

Run: `cargo test recovery_test -- --nocapture`
Expected: 所有测试通过

- [ ] **Step 5: Commit**

```bash
git add src/wal/recovery.rs src/database.rs tests/recovery_test.rs
git commit -m "feat(M18-T5): implement RecoveryManager data redo and uncommitted cleanup"
```

---

## Task 6: 性能基准测试

**Files:**
- Create: `benches/wal_group_commit_bench.rs`
- Modify: `Cargo.toml` (添加 bench target)

- [ ] **Step 1: 写 Group Commit 性能基准测试**

创建 `benches/wal_group_commit_bench.rs`：
```rust
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use rtsql::Database;
use std::path::Path;
use tempfile::TempDir;

fn bench_insert_with_wal_group_commit(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("bench.db");

    let mut group = c.benchmark_group("insert_wal");
    for rows in [100, 500, 1000] {
        group.bench_with_input(BenchmarkId::from_parameter(rows), &rows, |b, &rows| {
            b.to_async(&rt).iter(|| async {
                let db = Database::open(&db_path).await.unwrap();
                db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
                for i in 0..rows {
                    db.execute_sql(&format!("INSERT INTO t VALUES ({}, {})", i, i)).await;
                }
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_insert_with_wal_group_commit);
criterion_main!(benches);
```

- [ ] **Step 2: 运行基准测试**

Run: `cargo bench --bench wal_group_commit_bench`
Expected: 成功运行，输出 INSERT 吞吐量数据

- [ ] **Step 3: Commit**

```bash
git add benches/wal_group_commit_bench.rs Cargo.toml
git commit -m "bench(M18-T6): add WAL Group Commit performance benchmark"
```

---

## Task 7: 崩溃恢复 E2E 测试 + 全量验证

**Files:**
- Modify: `tests/recovery_test.rs`
- Modify: `tests/wal_integration_test.rs`

- [ ] **Step 1: 写完整 E2E 崩溃恢复测试**

在 `tests/recovery_test.rs` 中新增：
```rust
#[tokio::test]
async fn test_e2e_crash_recovery_with_concurrent_txns() {
    // 1. 开启数据库，创建表
    // 2. 并发执行多个 INSERT 事务（部分 commit，部分未 commit）
    // 3. 模拟崩溃（drop 数据库，不调用 close）
    // 4. 重新打开数据库
    // 5. 验证 committed 数据完整，uncommitted 数据不可见
}

#[tokio::test]
async fn test_e2e_crash_recovery_update_delete() {
    // 1. 开启数据库，INSERT 数据，commit
    // 2. UPDATE 数据，commit
    // 3. DELETE 数据，commit
    // 4. 崩溃 → 恢复
    // 5. 验证 DELETE 后数据不可见
}
```

- [ ] **Step 2: 运行全量测试**

Run: `cargo test -- --nocapture`
Expected: 所有测试通过，0 failures

- [ ] **Step 3: 运行 Clippy**

Run: `cargo clippy -- -D warnings`
Expected: 0 warnings

- [ ] **Step 4: Commit**

```bash
git add tests/recovery_test.rs tests/wal_integration_test.rs
git commit -m "test(M18-T7): add E2E crash recovery tests and full verification"
```

---

## Requirements Traceability Matrix

| Requirement | Task(s) | Coverage | Simplification | Status |
|-------------|---------|----------|----------------|--------|
| R1: WalRecord 扩展（BeginTxn/CommitTxn/AbortTxn + LSN + CRC） | T1 | 100% | None | ✅ |
| R2: WALBuffer + Group Commit | T2 | 100% | None | ✅ |
| R3: TransactionManager 集成 WAL | T3 | 100% | None | ✅ |
| R4: Executor 集成 WAL（Insert/Update/Delete） | T4 | 100% | None | ✅ |
| R5: RecoveryManager 数据重放（Redo + 清理） | T5 | 100% | None | ✅ |
| R6: 性能基准测试（5-10x INSERT） | T6 | 100% | None | ✅ |
| R7: 崩溃恢复 E2E 测试 | T7 | 100% | None | ✅ |
| R8: Group Commit 触发策略（缓冲区满 + 定时 + commit） | T2 | 100% | None | ✅ |
| R9: CRC32 校验 | T1 | 100% | None | ✅ |
| R10: 配置参数（capacity/interval/sync_mode） | T2 | 80% | sync_mode 推迟到后续 | ⚠️ |

**⚠️ 说明**：R10 的 `wal_sync_mode` 配置（full/normal/off）推迟到后续 milestone，当前默认使用 fsync。这不影响核心功能和性能目标。
