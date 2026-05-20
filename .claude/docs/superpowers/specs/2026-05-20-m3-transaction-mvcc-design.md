# M3: 事务与 MVCC 设计规范

> 创建日期：2026-05-20
> 里程碑：M3 - 事务与 MVCC

---

## 一、目标

实现 MVCC（多版本并发控制）事务系统，支持：
- Repeatable Read 隔离级别（可重复读）
- 快照读无锁（读不阻塞写，写不阻塞读）
- 行级写锁（写写冲突通过异步锁等待）
- 并发事务正确性验证

---

## 二、架构设计

### 2.1 核心组件

| 组件 | 职责 | 文件位置 |
|------|------|----------|
| `TransactionId` | 全局事务 ID 分配器 | `src/transaction/tx_id.rs` |
| `VersionChain` | 数据版本链管理 | `src/transaction/version_chain.rs` |
| `TransactionManager` | 事务生命周期管理 | `src/transaction/manager.rs` |
| `RowLock` | 行级写锁（异步） | `src/transaction/row_lock.rs` |
| `Snapshot` | 快照版本管理 | `src/transaction/snapshot.rs` |

### 2.2 模块结构

```
src/transaction/
├── mod.rs           # 模块导出
├── tx_id.rs         # TransactionId（AtomicU64）
├── version_chain.rs # VersionChain（版本指针）
├── snapshot.rs      # Snapshot（事务开始时的版本视图）
├── row_lock.rs      # RowLock（tokio::sync::RwLock 按行）
├── manager.rs       # TransactionManager（begin/commit/abort）
└── error.rs         # TransactionError
```

### 2.3 数据流

```
开始事务 → 分配 TxId → 创建 Snapshot
    ↓
读操作 → 快照读（无锁，按 Snapshot 读取版本）
    ↓
写操作 → 获取 RowLock → 写新版本 → 链接到 VersionChain
    ↓
提交 → 写提交标记 → 释放锁
    ↓
回滚 → 清理未提交版本 → 释放锁
```

---

## 三、核心设计

### 3.1 TransactionId（全局事务 ID）

```rust
pub struct TransactionId {
    counter: AtomicU64,
}

impl TransactionId {
    pub fn new() -> Self;
    pub fn allocate() -> u64;  // 返回新的 TxId
    pub fn current() -> u64;   // 当前最大 TxId
}
```

- **分配策略**：全局 AtomicU64 递增，无锁分配
- **TxId 格式**：纯数字，64 位，足够大（2^64）
- **用途**：
  - 每个事务开始时分配唯一 TxId
  - 作为 Snapshot 的创建时间戳
  - 标记数据版本的创建/删除事务

### 3.2 Snapshot（快照）

```rust
pub struct Snapshot {
    tx_id: u64,             // 创建快照的事务 ID
    active_tx_ids: Vec<u64>, // 创建快照时活跃的事务列表
}

impl Snapshot {
    pub fn new(tx_id: u64, active_tx_ids: Vec<u64>) -> Self;
    pub fn is_visible(&self, version_tx_id: u64, commit_tx_id: Option<u64>) -> bool;
}
```

- **可见性规则**（Repeatable Read）：
  ```
  版本可见条件：
    1. 创建事务已提交（commit_tx_id 存在）
    2. 创建事务的 TxId < Snapshot.TxId（早于快照）
    3. 创建事务不在 Snapshot.active_tx_ids 中（不是快照时的活跃事务）
  ```
- **实现**：`is_visible()` 方法判断版本是否对当前快照可见

### 3.3 VersionChain（版本链）

```rust
pub struct VersionHeader {
    create_tx_id: u64,      // 创建此版本的事务 ID
    commit_tx_id: Option<u64>, // 提交事务 ID（None = 未提交）
    next_version: Option<RowId>, // 指向上一版本的指针
}

pub struct VersionChain {
    // 版本链存储在数据页的 Row 数据中
}
```

- **存储方式**：
  - 每个 Row 数据前添加 VersionHeader（16 bytes）
  - `next_version` 指向该 Row 的上一个版本（类似链表）
  - 最新版本在前，历史版本通过指针追溯
- **溢出策略**：
  - 版本链尽量在同页存储
  - 页满时溢出到新页（overflow page）

### 3.4 RowLock（行级写锁）

```rust
pub struct RowLockTable {
    locks: RwLock<HashMap<RowId, Arc<Mutex<()>>>>,
}

impl RowLockTable {
    pub async fn acquire_write(&self, row_id: RowId) -> RowLockGuard;
    pub fn release(&self, row_id: RowId);
}
```

- **设计**：
  - 使用 `tokio::sync::RwLock<HashMap>` 管理锁表
  - 每个行一个 `Mutex<()>` 作为写锁
  - 异步等待不阻塞物理线程
- **用途**：
  - 写操作前获取 RowLock
  - 防止多个事务同时写同一行
  - 读操作不需要锁（MVCC 无锁读）

### 3.5 TransactionManager（事务管理）

```rust
pub struct TransactionManager {
    tx_id_allocator: TransactionId,
    active_tx_ids: RwLock<HashSet<u64>>,
    row_locks: RowLockTable,
}

impl TransactionManager {
    pub async fn begin(&self) -> Transaction;
    pub async fn commit(&self, tx: Transaction) -> Result<()>;
    pub async fn abort(&self, tx: Transaction) -> Result<()>;
}
```

- **begin**：
  - 分配 TxId
  - 记录活跃事务列表
  - 创建 Snapshot
- **commit**：
  - 写入提交标记（commit_tx_id）
  - 从活跃列表移除
  - 释放持有的锁
- **abort**：
  - 清理未提交版本（标记为删除）
  - 从活跃列表移除
  - 释放持有的锁

---

## 四、数据格式

### 4.1 VersionHeader（16 bytes）

```
┌──────────────┬──────────────┬──────────────┬──────────────┐
│ create_tx_id │ commit_tx_id │ next_version │   padding    │
│  (8 bytes)   │  (8 bytes)   │  (8 bytes)   │  (optional)  │
└──────────────┴──────────────┴──────────────┴──────────────┘
```

- `create_tx_id`: 创建此版本的事务 ID（必须存在）
- `commit_tx_id`: 提交事务 ID（None = 0xFFFFFFFFFFFFFFFF）
- `next_version`: 上一个版本的 RowId（None = 0xFFFFFFFFFFFFFFFF）

### 4.2 Row 数据布局（带版本）

```
┌──────────────┬────────────────────────────────────────────┐
│VersionHeader│            Row Data                        │
│ (16 bytes)  │        (原有数据格式)                      │
└──────────────┴────────────────────────────────────────────┘
```

---

## 五、操作流程

### 5.1 读操作（快照读）

```
1. 获取 Snapshot
2. 定位 Row（通过 BTree Index）
3. 读取最新版本
4. 检查可见性：
   - 未提交 → 追溯 next_version
   - 不在快照范围内 → 追溯 next_version
   - 可见 → 返回数据
5. 无可见版本 → 返回 None（行不存在或已删除）
```

### 5.2 写操作（插入/更新）

```
插入：
  1. 获取 RowLock（如果已存在行）
  2. 写入新版本（create_tx_id = 当前 TxId, commit_tx_id = None）
  3. 设置 next_version = None（新行无历史）

更新：
  1. 获取 RowLock（必须）
  2. 快照读找到可见版本
  3. 写入新版本（create_tx_id = 当前 TxId, next_version = 原版本 RowId）
  4. 提交时设置 commit_tx_id
```

### 5.3 删除操作

```
1. 获取 RowLock
2. 快照读找到可见版本
3. 写入删除标记版本（特殊标记）
4. 提交时设置 commit_tx_id
```

---

## 六、测试覆盖

### 6.1 单元测试

| 测试 | 文件 | 内容 |
|------|------|------|
| `tx_id_test.rs` | TransactionId 分配 | 单线程分配、多线程并发分配 |
| `snapshot_test.rs` | Snapshot 可见性 | 已提交可见、未提交不可见、活跃事务不可见 |
| `version_chain_test.rs` | 版本链追溯 | 多版本追溯、溢出页追溯 |
| `row_lock_test.rs` | 行锁 | 单行锁、并发锁等待、超时 |

### 6.2 并发测试

| 测试 | 内容 |
|------|------|
| `concurrent_write_write` | 两事务写同一行 → 第二个等待 |
| `concurrent_read_write` | 读不阻塞写，写不阻塞读 |
| `snapshot_consistency` | 事务内多次读返回相同版本 |
| `commit_abort` | 提交后可见，回滚后不可见 |

### 6.3 测试目标

- 单元测试：15+ 测试通过
- 并发测试：5+ 测试通过
- 总测试数：20+ 测试通过

---

## 七、集成点

### 7.1 与 M2（B-Tree）集成

- IndexManager 添加事务参数：`insert(tx: &Transaction, key, row)`
- BTree 返回 RowId，用于 VersionChain 定位
- 读操作传入 Snapshot

### 7.2 与 M1（BufferPool）集成

- 版本链页通过 BufferPool 管理
- 页修改时 mark_dirty

---

## 八、简化与推迟

| 项目 | 状态 | 原因 |
|------|------|------|
| Serializable 隔离级别 | 推迟到 M7 | 实现复杂，需谓词锁 |
| 版本清理（GC） | 推迟到 M7 | 需后台任务，暂手动清理 |
| WAL（预写日志） | 推迟到 M7 | 需持久化事务日志 |

---

## 九、依赖

| 依赖 | 版本 | 用途 |
|------|------|------|
| `tokio::sync::RwLock` | 已有 | 行锁表、活跃事务列表 |
| `std::sync::AtomicU64` | 内置 | TxId 分配 |
| `std::collections::HashSet` | 内置 | 活跃事务集合 |

---

## 十、验收标准

- [ ] TransactionId 分配唯一且递增
- [ ] Snapshot 可见性判断正确
- [ ] 版本链追溯正确（多版本可见）
- [ ] 行级写锁正常工作（写写冲突等待）
- [ ] 快照读无锁（读写并发）
- [ ] 提交后版本可见
- [ ] 回滚后版本不可见
- [ ] 所有测试通过（20+ tests）
- [ ] Clippy 无 Critical 警告

---

## 附录：场景假设

根据 BDD 方法论，以下默认假设适用于未明确覆盖的场景：

| 场景 | 默认假设 |
|------|----------|
| 空事务 | begin 后立即 commit/abort 无副作用 |
| 超大事务 | 不限制事务大小，资源耗尽时返回错误 |
| 长事务 | 不限制事务时长，快照可能过旧 |
| 死锁 | 暂不处理，依赖用户按顺序获取锁 |
| 版本过多 | 暂不清理，页满时溢出 |