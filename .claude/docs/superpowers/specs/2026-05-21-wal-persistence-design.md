# M11 WAL 持久化设计文档

> 创建日期：2026-05-21
> 里程碑：M11（WAL 持久化 + 崩溃恢复 + Checkpoint）

---

## 一、需求概述

### 1.1 核心目标

实现完整的 WAL 持久化机制，保障嵌入式数据库崩溃恢复能力：
- 已提交事务崩溃后不丢失（原子性保障）
- 崩溃恢复时重放 WAL 记录恢复数据状态
- Checkpoint 机制减少恢复时间

### 1.2 关键决策

| 决策项 | 选择 | 理由 |
|--------|------|------|
| WAL 写入时机 | 操作级 WAL | 每个操作立即写记录，崩溃恢复时重放所有已提交事务 |
| Checkpoint 策略 | 定期自动 Checkpoint | 每 N 次写操作自动触发，适合嵌入式数据库自动化管理 |
| WAL 文件管理 | 单文件追加 | 简单实现，符合嵌入式数据库单文件管理理念 |
| 重放策略 | Checkpoint 后重放 | 仅重放最后 checkpoint 后的 WAL，快速恢复 |
| 提交刷盘策略 | 每次提交 fsync | 严格持久化保障，已提交事务不丢失 |

### 1.3 场景假设（BDD 缺口补充）

| 缺口 | 默认假设 |
|------|----------|
| WAL 文件损坏 | 检测到损坏时返回错误，不尝试恢复 |
| 磁盘空间不足 | write() 返回错误，拒绝新操作 |
| fsync 失败 | 事务失败，清理 WAL 记录 |
| Checkpoint 刷脏页失败 | 继续提供服务，checkpoint 位点不更新 |
| WAL 文件超过阈值 | 不限制大小，checkpoint 时截断 |
| 部分 WAL 记录损坏 | 跳过损坏记录，重放下一个完整记录 |

---

## 二、系统架构

### 2.1 新增模块

```
src/wal/
├── mod.rs           # 模块导出
├── record.rs        # WalRecord enum（Insert/Update/Delete/Commit/Abort/Checkpoint）
├── writer.rs        # WalWriter（追加写入 + fsync）
├── reader.rs        # WalReader（重放 WAL 记录）
├── checkpoint.rs    # CheckpointManager（刷脏页 + 写位点 + 截断 WAL）
└── recovery.rs      # RecoveryManager（启动时重放 WAL）
```

### 2.2 新增文件

| 文件 | 用途 | 格式 |
|------|------|------|
| `<db_path>.wal` | WAL 日志文件 | 追加写入，checkpoint 时截断 |
| `<db_path>.checkpoint` | Checkpoint 位点文件 | `[lsn: 8B LE][timestamp: 8B LE]` |

### 2.3 现有架构集成点

| 集成点 | 改动内容 |
|--------|----------|
| `BufferPool` | 新增 `flush_all_dirty_pages()` 方法 |
| `TransactionManager` | commit/abort 时写 WAL 记录 + fsync |
| `Executor`（Insert/Update/Delete） | 执行后写操作级 WAL 记录 |
| `Database` | 启动时调用 `RecoveryManager::recover()`，持有 `WalWriter` |
| `pipeline.rs` | Executor 执行后调用 `WalWriter::write_record()` |

---

## 三、WAL 记录格式

### 3.1 WalRecord enum

```rust
pub enum WalRecord {
    Insert {
        tx_id: u64,
        table_name: String,
        row_id: RowId,
        tuple_data: Vec<u8>,
    },
    Update {
        tx_id: u64,
        table_name: String,
        row_id: RowId,
        old_tuple: Vec<u8>,
        new_tuple: Vec<u8>,
    },
    Delete {
        tx_id: u64,
        table_name: String,
        row_id: RowId,
    },
    Commit {
        tx_id: u64,
        timestamp: u64,
    },
    Abort {
        tx_id: u64,
    },
    Checkpoint {
        lsn: u64,
        timestamp: u64,
    },
}
```

### 3.2 序列化格式

**记录结构**：
```
[record_type: 1B][record_len: 4B LE][record_data: variable]
```

**类型编码**：
- `0x01`: Insert
- `0x02`: Update
- `0x03`: Delete
- `0x04`: Commit
- `0x05`: Abort
- `0x06`: Checkpoint

**数据编码**：
- String: `[len: 2B LE][UTF-8 bytes]`
- Vec<u8>: `[len: 4B LE][bytes]`
- RowId: `[page_id: 4B LE][slot_id: 2B LE]`（6 bytes）

---

## 四、WAL 写入流程

### 4.1 写入时机

| 操作 | WAL 记录 | 写入时机 |
|------|----------|----------|
| InsertExecutor | WalRecord::Insert | 执行后立即写 |
| UpdateExecutor | WalRecord::Update | 执行后立即写 |
| DeleteExecutor | WalRecord::Delete | 执行后立即写 |
| TransactionManager::commit() | WalRecord::Commit + fsync | commit 时写并刷盘 |
| TransactionManager::abort() | WalRecord::Abort | abort 时写 |
| CheckpointManager | WalRecord::Checkpoint | checkpoint 时写 |

### 4.2 写入流程

```
Executor 执行操作
  → Database.wal_writer.write_record(WalRecord::Insert{...})
  → spawn_blocking 包装同步 write()
  → WAL 文件追加写入
  → 记录写入位置（用于 checkpoint）

事务提交
  → Database.wal_writer.write_record(WalRecord::Commit{tx_id})
  → spawn_blocking 包装同步 write() + fsync()
  → fsync 完成后返回成功
  → 事务持久化保障
```

### 4.3 WalWriter 设计

```rust
pub struct WalWriter {
    wal_path: PathBuf,
    file: Mutex<File>,           // WAL 文件（追加写入）
    write_count: AtomicU64,      // 写入计数（用于触发 checkpoint）
    checkpoint_threshold: u64,   // Checkpoint 阈值（默认 1000）
}

impl WalWriter {
    pub async fn write_record(&self, record: WalRecord) -> Result<u64>;
    pub async fn fsync(&self) -> Result<()>;
    pub async fn truncate_to(&self, lsn: u64) -> Result<()>;
    pub fn get_write_count(&self) -> u64;
}
```

---

## 五、Checkpoint 机制

### 5.1 触发策略

**定期自动 Checkpoint**：
- 计数器：`write_count: AtomicU64`
- 阈值：`checkpoint_threshold: u64`（默认 1000）
- 触发条件：`write_count % checkpoint_threshold == 0`

### 5.2 Checkpoint 流程

```
触发条件满足
  → 获取当前 WAL 文件大小作为 lsn
  → BufferPool::flush_all_dirty_pages()
     → 遍历所有 PageFrame
     → 刷脏页到磁盘（spawn_blocking）
  → CheckpointManager::write_checkpoint_site(lsn, timestamp)
     → 写入位点文件（fsync）
  → WalWriter::truncate_to(lsn)
     → 截断 WAL 文件（删除 checkpoint 前的记录）
  → 重置 write_count
```

### 5.3 Checkpoint 位点文件

**文件路径**：`<db_path>.checkpoint`

**文件格式**：
```
[lsn: 8B LE][timestamp: 8B LE]
```

**更新时机**：每次 checkpoint 成功后更新

### 5.4 CheckpointManager 设计

```rust
pub struct CheckpointManager {
    checkpoint_path: PathBuf,
    wal_writer: Arc<WalWriter>,
    buffer_pool: Arc<BufferPool>,
}

impl CheckpointManager {
    pub async fn checkpoint(&self) -> Result<u64>;
    pub fn read_checkpoint_site(&self) -> Result<Option<(u64, u64)>>;
    pub fn write_checkpoint_site(&self, lsn: u64, timestamp: u64) -> Result<()>;
}
```

---

## 六、崩溃恢复流程

### 6.1 恢复时机

**Database 启动时**：
```rust
pub async fn open(path: &Path) -> Result<Arc<Database>> {
    // 1. 恢复 WAL
    RecoveryManager::recover(path)?;

    // 2. 正常初始化 Database
    let buffer_pool = Arc::new(BufferPool::new(...));
    let wal_writer = Arc::new(WalWriter::open(path)?);
    ...
}
```

### 6.2 恢复流程

```
1. 读取 checkpoint 位点
   → CheckpointManager::read_checkpoint_site()
   → 获取 last_checkpoint_lsn
   → 如无位点文件，从头重放（lsn = 0）

2. 打开 WAL 文件
   → WalReader::open(wal_path)
   → 定位到 lsn 位置（seek）

3. 重放 WAL 记录
   → WalReader::read_records() 迭代器
   → 对每条记录：
     - Insert: 写入数据页 + 创建索引（调用 TableManager）
     - Update: 更新数据页（调用 BufferPool）
     - Delete: 删除数据页记录 + 索引
     - Commit: 记录 tx_id 到 committed_tx_ids HashSet
     - Abort: 记录 tx_id 到 aborted_tx_ids HashSet
     - Checkpoint: 跳过

4. 过滤未提交事务
   → 遍历重放的操作记录
   → 跳过 tx_id 在 aborted_tx_ids 中的操作
   → 仅保留 tx_id 在 committed_tx_ids 中的操作

5. 恢复完成
   → 数据库进入正常运行状态
   → WAL 文件继续追加写入
```

### 6.3 WalReader 设计

```rust
pub struct WalReader {
    file: File,
    current_lsn: u64,
}

impl WalReader {
    pub fn open(path: &Path) -> Result<Self>;
    pub fn seek_to(&mut self, lsn: u64) -> Result<()>;
    pub fn read_records(&mut self) -> impl Iterator<Item = Result<(u64, WalRecord)>>;
}
```

### 6.4 RecoveryManager 设计

```rust
pub struct RecoveryManager;

impl RecoveryManager {
    pub fn recover(db_path: &Path) -> Result<()> {
        // 1. 读取 checkpoint 位点
        // 2. 打开 WAL 文件并定位
        // 3. 重放 WAL 记录
        // 4. 过滤未提交事务
    }
}
```

---

## 七、与现有系统集成

### 7.1 Database 结构修改

```rust
pub struct Database {
    buffer_pool: Arc<BufferPool>,
    table_manager: Arc<TableManager>,
    tx_manager: Arc<TransactionManager>,
    wal_writer: Arc<WalWriter>,       // 新增
    checkpoint_manager: Arc<CheckpointManager>, // 新增
}

impl Database {
    pub async fn open(path: &Path) -> Result<Arc<Database>> {
        // 1. 恢复 WAL
        RecoveryManager::recover(path)?;

        // 2. 初始化组件
        let buffer_pool = Arc::new(BufferPool::new(...));
        let wal_writer = Arc::new(WalWriter::open(path)?);
        let checkpoint_manager = Arc::new(CheckpointManager::new(...));

        // 3. 返回 Database
        Ok(Arc::new(Database { ... }))
    }
}
```

### 7.2 BufferPool 修改

新增方法：
```rust
impl BufferPool {
    pub async fn flush_all_dirty_pages(&self) -> Result<()> {
        // 遍历所有 PageFrame
        // 刷 dirty 页到磁盘
    }
}
```

### 7.3 TransactionManager 修改

```rust
impl TransactionManager {
    pub async fn commit(&self, tx_id: u64, wal_writer: &WalWriter) -> Result<()> {
        // 1. 写 Commit WAL 记录
        wal_writer.write_record(WalRecord::Commit { tx_id, timestamp }).await?;

        // 2. fsync WAL（严格持久化）
        wal_writer.fsync().await?;

        // 3. 标记事务已提交（现有逻辑）
        self.commit_mark_versions(tx_id);
    }

    pub async fn abort(&self, tx_id: u64, wal_writer: &WalWriter) -> Result<()> {
        // 1. 写 Abort WAL 记录
        wal_writer.write_record(WalRecord::Abort { tx_id }).await?;

        // 2. 清理未提交版本（现有逻辑）
        self.abort_cleanup_versions(tx_id);
    }
}
```

### 7.4 Executor 修改

InsertExecutor：
```rust
impl InsertExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        // 1. 执行插入（现有逻辑）
        let row_id = self.table_manager.insert_tuple(...);

        // 2. 写 WAL 记录
        self.wal_writer.write_record(WalRecord::Insert {
            tx_id, table_name, row_id, tuple_data
        }).await?;

        // 3. 返回结果
        Ok(Some(ExecResult::AffectedRows(1)))
    }
}
```

UpdateExecutor/DeleteExecutor 类似。

### 7.5 pipeline.rs 修改

```rust
pub async fn execute_sql(database: &Database, sql: &str) -> Result<Response> {
    // 现有逻辑：parse → plan → execute

    // Executor 创建时传入 wal_writer
    // Executor 执行后自动写 WAL 记录
}
```

---

## 八、测试策略

### 8.1 单元测试

| 测试文件 | 测试内容 |
|----------|----------|
| `tests/wal_record_test.rs` | WalRecord 序列化/反序列化 |
| `tests/wal_writer_test.rs` | WalWriter 写入/fsync/truncate |
| `tests/wal_reader_test.rs` | WalReader 读取/定位/迭代 |
| `tests/checkpoint_test.rs` | Checkpoint 位点读写 |
| `tests/recovery_test.rs` | WAL 重放逻辑 |

### 8.2 集成测试

| 测试文件 | 测试内容 |
|----------|----------|
| `tests/wal_integration_test.rs` | 完整写入→checkpoint→恢复流程 |
| `tests/crash_recovery_test.rs` | 崩溃恢复 E2E：写入→强制关闭→重启→验证数据 |
| `tests/transaction_durability_test.rs` | 事务持久化：提交→fsync→崩溃→恢复 |
| `tests/checkpoint_trigger_test.rs` | Checkpoint 自动触发：写入 N 次→验证截断 |

### 8.3 失败场景测试

| 测试内容 | 验证行为 |
|----------|----------|
| WAL 文件损坏 | RecoveryManager 返回错误，不尝试恢复 |
| Checkpoint 中断恢复 | 下次启动时从上一个有效 checkpoint 重放 |
| 部分 WAL 记录损坏 | 跳过损坏记录，重放下一个完整记录 |
| 磁盘空间不足 | write() 返回错误，拒绝新操作 |

---

## 九、性能考虑

### 9.1 写入性能

| 优化点 | 设计 |
|--------|------|
| 异步包装 | `spawn_blocking` 包装同步 write/fsync，不阻塞协程 |
| 追加写入 | WAL 文件追加写入，无需定位，减少 I/O |
| fsync 频率 | 仅 commit 时 fsync，减少刷盘开销 |

### 9.2 Checkpoint 性能

| 优化点 | 设计 |
|--------|------|
| 定期触发 | 每 1000 次写操作触发，可配置阈值 |
| 刷脏页 | 刷所有脏页（嵌入式数据库数据量可控） |
| 截断 WAL | 减少 WAL 文件大小，加快恢复 |

### 9.3 恢复性能

| 优化点 | 设计 |
|--------|------|
| Checkpoint 后重放 | 仅重放 checkpoint 后的 WAL，减少重放时间 |
| 跳过未提交事务 | 根据 Commit/Abort 记录过滤，减少工作量 |
| 损坏记录跳过 | 快速跳过损坏记录，不阻塞恢复 |

---

## 十、实现计划（Phase 2 详细规划）

### 10.1 Phase 分解

**Phase 1: WAL 基础结构**
- Task 1: 实现 WalRecord enum + 序列化
- Task 2: 实现 WalWriter（追加写入 + fsync）
- Task 3: 实现 WalReader（读取 + 定位）
- Task 4: 单元测试（record/writer/reader）

**Phase 2: Checkpoint 机制**
- Task 5: BufferPool 新增 flush_all_dirty_pages()
- Task 6: 实现 CheckpointManager（位点读写）
- Task 7: Checkpoint 自动触发逻辑
- Task 8: Checkpoint 单元测试

**Phase 3: Executor 集成**
- Task 9: Database 结构修改（持有 WalWriter）
- Task 10: InsertExecutor 写 WAL 记录
- Task 11: UpdateExecutor 写 WAL 记录
- Task 12: DeleteExecutor 写 WAL 记录
- Task 13: Executor 集成测试

**Phase 4: TransactionManager 集成**
- Task 14: TransactionManager::commit() 写 WAL + fsync
- Task 15: TransactionManager::abort() 写 WAL
- Task 16: TransactionManager 集成测试

**Phase 5: Recovery 实现**
- Task 17: 实现 RecoveryManager::recover()
- Task 18: Database::open() 调用恢复
- Task 19: 崩溃恢复 E2E 测试

**Phase 6: 端到端验证**
- Task 20: 完整测试套件运行
- Task 21: 性能验证
- Task 22: 文档更新

### 10.2 预估工作量

| Phase | 任务数 | 预估时间 |
|-------|--------|----------|
| Phase 1 | 4 | 1 天 |
| Phase 2 | 4 | 1 天 |
| Phase 3 | 5 | 1 天 |
| Phase 4 | 3 | 0.5 天 |
| Phase 5 | 3 | 0.5 天 |
| Phase 6 | 3 | 0.5 天 |
| **总计** | **22** | **4 天** |

---

## 十一、风险与应对

| 风险 | 影响 | 应对策略 |
|------|------|----------|
| WAL 文件损坏恢复失败 | 数据丢失 | 提供备份机制，checkpoint 前备份 WAL |
| Checkpoint 刷脏页阻塞服务 | 性能下降 | 异步 checkpoint，不阻塞协程 |
| fsync 性能瓶颈 | 提交延迟 | 提供批量 fsync 配置选项（推迟） |
| 恢复时间长 | 启动延迟 | 减小 checkpoint 频率，增加位点检查 |

---

## 十二、后续优化方向

| 优化项 | 推迟里程碑 | 说明 |
|--------|-----------|------|
| WAL 文件滚动 | M13 | 固定大小后滚动新文件，避免单文件过大 |
| LSN + page_lsn | M13 | 精准判断哪些页已刷盘，减少 checkpoint 刷页量 |
| 批量 fsync | M13 | 提供配置选项，减少刷盘频率 |
| 并行恢复 | M13 | 多线程重放 WAL，加快恢复速度 |

---

## 附录：需求完整性检查

| 需求项 | 覆盖任务 | 状态 |
|--------|----------|------|
| WAL 写入流程 | Task 1-4, 9-13 | ✅ |
| 崩溃恢复（WAL 重放） | Task 17-19 | ✅ |
| Checkpoint 机制 | Task 5-8 | ✅ |
| 原子性保障（fsync） | Task 14-16 | ✅ |
| 定期自动 Checkpoint | Task 7 | ✅ |
| Checkpoint 后重放 | Task 17 | ✅ |
| 单文件追加 WAL | Task 2 | ✅ |
| 每次提交 fsync | Task 14 | ✅ |

**无 Simplification，无 Missing 需求**。