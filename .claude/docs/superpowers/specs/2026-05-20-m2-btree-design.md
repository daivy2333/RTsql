# M2: B-Tree 索引与存储引擎设计规范

> 创建时间：2026-05-20
> 里程碑：M2 - B-Tree 索引与存储引擎

---

## 1. 目标与范围

### 目标

实现完整的 B-Tree 索引系统，支持基本 CRUD 操作（Insert、Search、Delete、Update），并通过 `spawn_blocking` 暴露为异步 API。

### 范围

- ✅ 同步 B-Tree 索引内核（纯 CPU 操作）
- ✅ 异步 API 包装（IndexManager）
- ✅ Slotted Page 行存储格式（4KB 页）
- ✅ Node Split/Merge 操作
- ✅ 索引操作正确性测试
- ⏸️ 范围扫描（推迟到 M5 执行引擎）
- ⏸️ 批量操作与统计（推迟到 M7 优化）

---

## 2. 架构设计

### 2.1 三层分层架构

```
┌─────────────────────────────────────────────┐
│  Async Index API (IndexManager)             │ ← spawn_blocking 包装
│  - async insert(key, row_id)                 │
│  - async search(key) -> Option<RowId>        │
│  - async delete(key)                         │
│  - async update(key, new_row_id)             │
├─────────────────────────────────────────────┤
│  Sync B-Tree Core (BTree)                   │ ← CPU密集型，纯同步
│  - Node operations (split/merge)             │
│  - Tree traversal                           │
│  - 持有 SyncPageLoader                       │
├─────────────────────────────────────────────┤
│  SyncPageLoader                             │ ← block_on 包装 BufferPool
│  - load_page(page_id) -> PageGuard          │
├─────────────────────────────────────────────┤
│  BufferPool + AsyncStorage                  │ ← M1 已完成
└─────────────────────────────────────────────┘
```

### 2.2 职责边界

| 层级 | 职责 | 异步策略 |
|------|------|----------|
| **IndexManager** | 暴露 async API，spawn_blocking 包装 | 全异步 |
| **BTree** | B-Tree 操作逻辑（查找/插入/删除/Split/Merge） | 纯同步，不直接 await |
| **SyncPageLoader** | 在同步代码中加载页（block_on 包装） | 内部 block_on，外部同步 |
| **BufferPool** | 页缓存管理（M1 已完成） | 全异步 |

---

## 3. 页节点设计

### 3.1 分离设计（Leaf vs Internal）

采用分离设计，职责清晰：

#### LeafNode（叶子节点）

```rust
struct LeafHeader {
    page_type: u8,        // 0x01 = Leaf
    slot_count: u16,
    free_space_offset: u16,
    next_leaf_page_id: u32, // 下一叶子节点（用于顺序扫描）
    _padding: [u8; 5],
}

struct LeafNode {
    header: LeafHeader,
    slots: Vec<Slot>,             // Slot 数组（offset + length）
    key_data: Vec<u8>,            // Key 数据区
    row_id_data: Vec<u8>,         // RowId 数据区
}

struct Slot {
    offset: u16,
    length: u16,
}
```

**LeafNode 存储内容**：
- Key（固定长度或变长，本 M2 采用固定 32 bytes）
- RowId（u64，指向实际数据页中的行）

#### InternalNode（内部节点）

```rust
struct InternalHeader {
    page_type: u8,        // 0x02 = Internal
    slot_count: u16,
    free_space_offset: u16,
    _padding: [u8; 9],
}

struct InternalNode {
    header: InternalHeader,
    slots: Vec<Slot>,
    key_data: Vec<u8>,            // Key 数据区
    child_page_ids: Vec<PageId>,  // 子节点页ID（u32）
}
```

**InternalNode 存储内容**：
- Key（分隔键）
- ChildPageId（指向子节点的页ID）

---

### 3.2 Slotted Page 通用格式

所有页（Leaf/Internal/Data）统一采用 Slotted Page 格式：

```
┌────────────┬──────────────────┬─────────────┬─────────────┐
│ Header     │ Free Space       │ Slot Array  │ Row Data    │
│ (16 bytes) │ (grows downward) │ (grows up)  │ (grows down)│
└────────────┴──────────────────┴─────────────┴─────────────┘
```

**布局规则**：
- Header 固定 16 bytes（页头部）
- Slot Array 从页尾向上增长
- Row/Key Data 从 Header 后向下增长
- Free Space 在中间，动态调整

---

### 3.3 Key 设计

**M2 采用固定长度 Key**：

```rust
const MAX_KEY_LEN: usize = 32;  // 32 bytes 固定

struct Key {
    data: [u8; MAX_KEY_LEN],
    len: u8,  // 实际长度（≤ 32）
}
```

**优势**：
- 简化实现，避免变长 Key 的复杂性
- Slot 结构统一，便于序列化
- 未来可扩展为变长 Key（M7 优化）

---

### 3.4 RowId 设计

```rust
struct RowId {
    page_id: u32,  // 数据页 ID
    slot_id: u16,  // Slotted Page 中的 slot index
}
```

**总大小**：6 bytes（u32 + u16）

---

## 4. B-Tree 操作流程

### 4.1 Insert(key, row_id)

**流程**：

```
1. 从 root 页开始，递归查找插入位置
   - InternalNode：比较 key，选择 child_page_id
   - LeafNode：找到插入位置（按 key 排序）

2. 到达 LeafNode，检查 free_space 是否足够

3. 有空间 → 直接插入
   - 在 key_data 中添加 key
   - 在 row_id_data 中添加 row_id
   - 更新 slots 数组
   - 更新 header.slot_count 和 free_space_offset
   - 标记 PageGuard dirty

4. 无空间 → Split
   a. 分配新页（buffer_pool.allocate_page()）
   b. 复制一半数据到新页
   c. 设置新页的 header.next_leaf_page_id
   d. 更新 parent（插入新的分隔键 + child_page_id）
   e. 递归检查 parent 是否需要 split
   f. 标记所有修改页 dirty

5. 更新 root（如果 root split，分配新 root 页）
```

**Split 逻辑**：
- Leaf Split：将 50% 数据移到新页
- Internal Split：将中间 key 上推到 parent，50% child 分给新节点

---

### 4.2 Search(key)

**流程**：

```
1. 从 root 页开始
2. InternalNode → 比较 key，选择 child_page_id
   - keys[i] <= key < keys[i+1] → child_page_ids[i]
   - key < keys[0] → child_page_ids[0]
   - key >= keys[last] → child_page_ids[last]
3. LeafNode → 遍历 slots，找到匹配 key
4. 返回 RowId 或 None
```

---

### 4.3 Delete(key)

**流程**：

```
1. 从 root 开始查找 LeafNode
2. 找到 key → 删除 slot + key_data + row_id_data
3. 更新 header.slot_count 和 free_space_offset
4. 检查是否需要 Merge（节点太空）
   - Merge 条件：slot_count < MIN_KEYS
   - MIN_KEYS = MAX_KEYS / 2（B-Tree 平衡要求）
**Merge 逻辑**：
- Merge 条件：`slot_count < MIN_KEYS`
- MIN_KEYS = `(MAX_KEYS / 2)`（B-Tree 平衡要求）
- MAX_KEYS：单个 LeafNode 可存储的最大 key 数量（约 100-200，取决于页大小和 Key/RowId 大小）
- 精确值需根据实际页容量计算（4KB - 16 bytes header）：

```
可用空间 ≈ 4096 - 16 = 4080 bytes
每个 slot ≈ 4 bytes (offset + length)
每个 entry ≈ 32 bytes (key) + 6 bytes (row_id) = 38 bytes
slot_count ≈ 4080 / (4 + 38) ≈ 97 entries

MIN_KEYS = 97 / 2 ≈ 48
MAX_KEYS = 97
```

---

### 4.4 Update(key, new_row_id)

**流程**：

```
1. Search(key) → 找到 LeafNode
2. 直接更新 row_id_data 中的 RowId
3. 标记 dirty
```

---

## 5. 异步 API 设计

### 5.1 SyncPageLoader

**问题**：BTree 纯同步代码需要在内部加载页，但 BufferPool 是 async。

**方案**：SyncPageLoader 使用 `tokio::runtime::Runtime::block_on` 包装。

```rust
pub struct SyncPageLoader {
    buffer_pool: Arc<BufferPool>,
    runtime: Handle,  // Tokio runtime handle
}

impl SyncPageLoader {
    pub fn new(buffer_pool: Arc<BufferPool>) -> Self {
        // 从当前 Tokio runtime 获取 handle
        // 注意：必须在 Tokio runtime context 内调用（如 #[tokio::test] 或 tokio::spawn 内）
        let runtime = tokio::runtime::Handle::current();
        Self {
            buffer_pool,
            runtime,
        }
    }

    pub fn load_page(&self, page_id: PageId) -> Result<PageGuard> {
        self.runtime.block_on(self.buffer_pool.get_page(page_id))
    }

    pub fn allocate_page(&self) -> Result<PageId> {
        self.runtime.block_on(self.buffer_pool.storage().allocate_page())
    }
}
```

**注意**：
- 在 `spawn_blocking` 内使用 `block_on` 是安全的（不会阻塞异步运行时）
- `Handle::current()` 获取当前 runtime handle（spawn_blocking 内可用）

---

### 5.2 BTree 同步核心

```rust
pub struct BTree {
    loader: SyncPageLoader,
    root_page_id: PageId,
}

impl BTree {
    pub fn new(loader: SyncPageLoader) -> Result<Self> {
        let root_page_id = loader.allocate_page()?;
        // 初始化 root 为空 LeafNode
        let root_page = loader.load_page(root_page_id)?;
        let mut leaf = LeafNode::new_empty();
        leaf.write_to_page(&root_page)?;
        root_page.mark_dirty();

        Ok(Self { loader, root_page_id })
    }

    pub fn insert(&self, key: &[u8], row_id: RowId) -> Result<()> {
        // 递归插入逻辑
        // 需要多次 load_page
    }

    pub fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
        // 递归查找逻辑
    }

    pub fn delete(&self, key: &[u8]) -> Result<()> {
        // 递归删除逻辑
    }

    pub fn update(&self, key: &[u8], new_row_id: RowId) -> Result<()> {
        // 查找 + 更新
    }
}
```

---

### 5.3 IndexManager 异步包装

```rust
pub struct IndexManager {
    btree: Arc<std::sync::Mutex<BTree>>,
}

impl IndexManager {
    pub fn new(buffer_pool: Arc<BufferPool>) -> Result<Self> {
        let loader = SyncPageLoader::new(buffer_pool);
        let btree = BTree::new(loader)?;
        Ok(Self {
            btree: Arc::new(std::sync::Mutex::new(btree)),
        })
    }

    pub async fn insert(&self, key: &[u8], row_id: RowId) -> Result<()> {
        let btree = self.btree.clone();
        let key = key.to_vec();
        tokio::task::spawn_blocking(move || {
            btree.lock().unwrap().insert(&key, row_id)
        }).await?
    }

    pub async fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
        let btree = self.btree.clone();
        let key = key.to_vec();
        tokio::task::spawn_blocking(move || {
            btree.lock().unwrap().search(&key)
        }).await?
    }

    pub async fn delete(&self, key: &[u8]) -> Result<()> {
        let btree = self.btree.clone();
        let key = key.to_vec();
        tokio::task::spawn_blocking(move || {
            btree.lock().unwrap().delete(&key)
        }).await?
    }

    pub async fn update(&self, key: &[u8], new_row_id: RowId) -> Result<()> {
        let btree = self.btree.clone();
        let key = key.to_vec();
        tokio::task::spawn_blocking(move || {
            btree.lock().unwrap().update(&key, new_row_id)
        }).await?
    }
}
```

**关键点**：
- BTree 用 `std::sync::Mutex`（在 spawn_blocking 内同步锁）
- IndexManager API 全异步
- Key 需要复制（spawn_blocking 要求 Send）

---

## 6. 文件结构

```
src/storage/
├── btree/
│   ├── mod.rs           # 模块导出
│   ├── node.rs          # LeafNode + InternalNode 结构
│   ├── btree.rs         # BTree 同步核心（insert/search/delete/update）
│   ├── sync_loader.rs   # SyncPageLoader（block_on 包装）
│   └── index_manager.rs # IndexManager 异步 API
├── page_format/
│   ├── mod.rs           # 模块导出
│   ├── slotted_page.rs  # SlottedPage 通用格式读写
│   ├── row.rs           # Row 序列化/反序列化
│   ├── row_id.rs        # RowId 结构
│   └── key.rs           # Key 结构（固定 32 bytes）
└── (现有文件保持不变)
    ├── mod.rs
    ├── error.rs
    ├── page_id.rs
    ├── page.rs
    ├── async_storage.rs
    ├── file_storage.rs
    ├── buffer_pool.rs
    ├── page_frame.rs
```

---

## 7. 测试策略

### 7.1 单元测试（同步）

```rust
// node.rs 内部测试
#[cfg(test)]
mod tests {
    #[test]
    fn test_leaf_node_insert_single() { ... }

    #[test]
    fn test_leaf_node_insert_fill() { ... }

    #[test]
    fn test_leaf_node_split() { ... }

    #[test]
    fn test_internal_node_insert_child() { ... }
}
```

### 7.2 BTree 单元测试（内存存储）

```rust
// btree.rs 内部测试
#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use crate::storage::*;

    #[test]
    fn test_btree_insert_search() {
        let dir = tempdir().unwrap();
        let storage = FileStorage::new(dir.path().join("test.db")).unwrap();
        let buffer_pool = BufferPool::new(100, Arc::new(storage)).unwrap();
        let loader = SyncPageLoader::new(Arc::new(buffer_pool));
        let btree = BTree::new(loader).unwrap();

        // 插入测试
        btree.insert(b"key1", RowId { page_id: 1, slot_id: 0 }).unwrap();
        btree.insert(b"key2", RowId { page_id: 2, slot_id: 1 }).unwrap();

        // 查找测试
        let result = btree.search(b"key1").unwrap();
        assert_eq!(result, Some(RowId { page_id: 1, slot_id: 0 }));

        let result = btree.search(b"key3").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_btree_split_merge() {
        // 插入大量数据触发 split
        // 删除数据触发 merge
    }
}
```

### 7.3 IndexManager 异步测试

```rust
// tests/index_test.rs
use tokio::test;

#[tokio::test]
async fn test_index_manager_basic_ops() {
    let dir = tempdir().unwrap();
    let storage = FileStorage::new(dir.path().join("test.db")).unwrap();
    let buffer_pool = Arc::new(BufferPool::new(100, Arc::new(storage)).unwrap());
    let index = IndexManager::new(buffer_pool).unwrap();

    // Insert
    index.insert(b"key1", RowId { page_id: 1, slot_id: 0 }).await.unwrap();

    // Search
    let result = index.search(b"key1").await.unwrap();
    assert_eq!(result, Some(RowId { page_id: 1, slot_id: 0 }));

    // Delete
    index.delete(b"key1").await.unwrap();
    let result = index.search(b"key1").await.unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn test_index_manager_concurrent_ops() {
    // 多协程并发 insert/search
    use tokio::spawn;

    let index = Arc::new(setup_index());

    let mut tasks = vec![];
    for i in 0..100 {
        tasks.push(spawn(async move {
            index.insert(format!("key{}", i).as_bytes(), ...).await
        }));
    }

    for task in tasks {
        task.await.unwrap();
    }
}
```

---

## 8. 技术决策

| 决策点 | 选择 | 原因 |
|--------|------|------|
| Key 长度 | 固定 32 bytes | M2 简化实现，避免变长复杂性 |
| Node 设计 | 分离（Leaf vs Internal） | 职责清晰，便于未来扩展 |
| Page 格式 | Slotted Page | 通用格式，支持变长数据 |
| BTree 异步策略 | SyncPageLoader + block_on | 纯同步核心，spawn_blocking 包装 |
| BTree 锁 | std::sync::Mutex | spawn_blocking 内同步锁足够 |
| Split 策略 | 50% 数据迁移 | B-Tree 平衡标准策略 |
| Merge 策略 | slot_count < MIN_KEYS | B-Tree 平衡标准策略 |

---

## 9. 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| block_on 在 spawn_blocking 内开销 | 性能影响 | 接受（spawn_blocking 本身隔离 CPU 操作） |
| BTree 递归深度过大 | 栈溢出 | 转为循环 + explicit stack |
| Split/Merge 逻辑复杂 | Bug 风险 | 详细单元测试 + proptest |
| 固定 Key 限制灵活性 | 功能限制 | M7 扩展变长 Key |

---

## 10. 下一步

1. 实现 Slotted Page 格式读写
2. 实现 LeafNode + InternalNode
3. 实现 BTree 核心逻辑（Insert/Search/Delete）
4. 实现 SyncPageLoader
5. 实现 IndexManager
6. 编写测试验证正确性