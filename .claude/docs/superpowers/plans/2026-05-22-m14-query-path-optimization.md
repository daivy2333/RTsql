# M14: 查询路径优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** PK 查询 3-5x 提速，通过 BTree 零拷贝迁移 + SQL 文本级 LRU 缓存

**Architecture:** BTree 读路径从 `page()` + `LeafNode::from_page()` 迁移到 `page_data()` + 新增 `LeafNodeRef`/`InternalNodeRef` 零拷贝读取。Database 层添加 `LruCache<String, PhysicalPlan>` 缓存，命中时跳过 parse+plan。

**Tech Stack:** Rust, lru crate, 现有 PageDataGuard/PageGuard 机制

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `src/storage/btree/node.rs` | Modify | 新增 LeafNodeRef + InternalNodeRef |
| `src/storage/btree/btree.rs` | Modify | 读操作改用 page_data() + *Ref |
| `src/storage/btree/index_manager.rs` | Modify | search/scan_all 零拷贝 |
| `src/executor/plan.rs` | Modify | 确保 PhysicalPlan + 各节点 Clone |
| `src/database.rs` | Modify | 添加 plan_cache 字段 |
| `src/pipeline.rs` | Modify | 缓存查询逻辑 |
| `Cargo.toml` | Modify | 添加 lru crate |

---

### Task 1: 添加 lru crate 依赖

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: 添加 lru 依赖到 Cargo.toml**

在 `[dependencies]` 中添加：

```toml
lru = "0.12"
```

- [ ] **Step 2: 验证编译**

Run: `cargo check 2>&1 | tail -5`
Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore(M14): add lru crate dependency"
```

---

### Task 2: 实现 LeafNodeRef 零拷贝只读结构

**Files:**
- Modify: `src/storage/btree/node.rs`

- [ ] **Step 1: 写 LeafNodeRef 失败测试**

在 `node.rs` 底部 `#[cfg(test)] mod tests` 中添加：

```rust
#[test]
fn test_leaf_node_ref_from_page_data() {
    // 使用真实的 Page + LeafNode 构造数据，再通过 LeafNodeRef 读取
    let mut page = Page::new(PageId(0));
    let mut leaf = LeafNode::init(&mut page);
    leaf.insert(&Key::new(b"hello"), &RowId::new(1, 0)).unwrap();
    leaf.insert(&Key::new(b"world"), &RowId::new(2, 1)).unwrap();

    // 通过 LeafNodeRef 零拷贝读取
    let leaf_ref = LeafNodeRef::new(&page.data[..]);
    assert_eq!(leaf_ref.key_count(), 2);
    assert_eq!(leaf_ref.get_key(0).unwrap().as_bytes(), b"hello");
    assert_eq!(leaf_ref.get_row_id(0).unwrap(), RowId::new(1, 0));
    assert_eq!(leaf_ref.get_key(1).unwrap().as_bytes(), b"world");
    assert_eq!(leaf_ref.get_row_id(1).unwrap(), RowId::new(2, 1));
}

#[test]
fn test_leaf_node_ref_find_key_position() {
    let mut page = Page::new(PageId(0));
    let mut leaf = LeafNode::init(&mut page);
    leaf.insert(&Key::new(b"a"), &RowId::new(1, 0)).unwrap();
    leaf.insert(&Key::new(b"c"), &RowId::new(2, 1)).unwrap();

    let leaf_ref = LeafNodeRef::new(&page.data[..]);

    // 查找存在的 key
    let (found, pos) = leaf_ref.find_key_position(&Key::new(b"a"));
    assert!(found);
    assert_eq!(pos, 0);

    // 查找不存在的 key（应返回插入位置）
    let (found, pos) = leaf_ref.find_key_position(&Key::new(b"b"));
    assert!(!found);
    assert_eq!(pos, 1);
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test test_leaf_node_ref -- --nocapture 2>&1 | tail -10`
Expected: 编译失败（LeafNodeRef 未定义）

- [ ] **Step 3: 实现 LeafNodeRef**

LeafNodeRef 基于 `SlottedPageRef`（与 `LeafNode` 基于 `SlottedPage` 完全对称），
每个 slot 存储 Key(32 bytes) + RowId(6 bytes)。

在 `node.rs` 中 `LeafNode` 结构体之后添加：

```rust
/// Zero-copy read-only view of a leaf node.
/// Wraps SlottedPageRef to read Key+RowId entries without page clone.
pub struct LeafNodeRef<'a> {
    slotted: SlottedPageRef<'a>,
}

impl<'a> LeafNodeRef<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        let slotted = SlottedPageRef::new(data);
        Self { slotted }
    }

    pub fn key_count(&self) -> usize {
        self.slotted.slot_count()
    }

    pub fn get_key(&self, index: usize) -> Option<Key> {
        let slot = self.slotted.get_slot(index)?;
        let data = self.slotted.get_slot_data(&slot);
        if data.len() < MAX_KEY_LEN + RowId::SIZE {
            return None;
        }
        Some(Key::deserialize(&data[..MAX_KEY_LEN]))
    }

    pub fn get_row_id(&self, index: usize) -> Option<RowId> {
        let slot = self.slotted.get_slot(index)?;
        let data = self.slotted.get_slot_data(&slot);
        if data.len() < MAX_KEY_LEN + RowId::SIZE {
            return None;
        }
        Some(RowId::deserialize(&data[MAX_KEY_LEN..]))
    }

    /// Find position where key should be inserted (or is located).
    /// Returns (found, position): found=true if key exists at position.
    pub fn find_key_position(&self, key: &Key) -> (bool, usize) {
        let count = self.key_count();
        for i in 0..count {
            if let Some(current_key) = self.get_key(i) {
                if current_key == *key {
                    return (true, i);
                }
                if current_key > *key {
                    return (false, i);
                }
            }
        }
        (false, count)
    }

    pub fn next_leaf_page_id(&self) -> u32 {
        self.slotted.header().next_page_id
    }
}
```

需要在 node.rs 顶部 import 中添加 `SlottedPageRef`：

```rust
use crate::storage::page_format::{Key, RowId, Slot, SlottedPage, SlottedPageRef, SlottedPageHeader, MAX_KEY_LEN};
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test test_leaf_node_ref -- --nocapture 2>&1 | tail -10`
Expected: 2 tests passed

- [ ] **Step 5: Commit**

```bash
git add src/storage/btree/node.rs
git commit -m "feat(M14): add LeafNodeRef zero-copy read-only structure"
```

---

### Task 3: 实现 InternalNodeRef 零拷贝只读结构

**Files:**
- Modify: `src/storage/btree/node.rs`

- [ ] **Step 1: 写 InternalNodeRef 失败测试**

在 `node.rs` 测试模块中添加：

```rust
#[test]
fn test_internal_node_ref_from_page_data() {
    let mut page = Page::new(PageId(0));
    let mut internal = InternalNode::init(&mut page, 1); // leftmost_child = 1
    internal.insert(&Key::new(b"m"), 2).unwrap(); // key "m" -> child page 2

    let internal_ref = InternalNodeRef::new(&page.data[..]);
    assert_eq!(internal_ref.key_count(), 1);
    assert_eq!(internal_ref.leftmost_child(), 1);
    assert_eq!(internal_ref.get_key(0).unwrap().as_bytes(), b"m");
    assert_eq!(internal_ref.get_child_page_id(0), Some(2));
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test test_internal_node_ref -- --nocapture 2>&1 | tail -10`
Expected: 编译失败

- [ ] **Step 3: 实现 InternalNodeRef**

InternalNodeRef 同样基于 SlottedPageRef，每个 slot 存储 Key(32 bytes) + child_page_id(u32)。

```rust
/// Zero-copy read-only view of an internal node.
pub struct InternalNodeRef<'a> {
    slotted: SlottedPageRef<'a>,
}

impl<'a> InternalNodeRef<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        let slotted = SlottedPageRef::new(data);
        Self { slotted }
    }

    pub fn key_count(&self) -> usize {
        self.slotted.slot_count()
    }

    pub fn leftmost_child(&self) -> u32 {
        self.slotted.header().next_page_id
    }

    pub fn get_key(&self, index: usize) -> Option<Key> {
        let slot = self.slotted.get_slot(index)?;
        let data = self.slotted.get_slot_data(&slot);
        if data.len() < MAX_KEY_LEN {
            return None;
        }
        Some(Key::deserialize(&data[..MAX_KEY_LEN]))
    }

    pub fn get_child_page_id(&self, index: usize) -> Option<u32> {
        let slot = self.slotted.get_slot(index)?;
        let data = self.slotted.get_slot_data(&slot);
        if data.len() < MAX_KEY_LEN + 4 {
            return None;
        }
        Some(u32::from_be_bytes([
            data[MAX_KEY_LEN],
            data[MAX_KEY_LEN + 1],
            data[MAX_KEY_LEN + 2],
            data[MAX_KEY_LEN + 3],
        ]))
    }

    /// Find the child page to descend into for the given key.
    pub fn find_child_page_id(&self, key: &Key) -> Option<u32> {
        let count = self.key_count();
        for i in 0..count {
            if let Some(current_key) = self.get_key(i) {
                if *key < current_key {
                    if i == 0 {
                        return Some(self.leftmost_child());
                    }
                    return self.get_child_page_id(i - 1);
                }
            }
        }
        if count == 0 {
            return Some(self.leftmost_child());
        }
        self.get_child_page_id(count - 1)
    }
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test test_internal_node_ref -- --nocapture 2>&1 | tail -10`
Expected: 1 test passed

- [ ] **Step 5: Commit**

```bash
git add src/storage/btree/node.rs
git commit -m "feat(M14): add InternalNodeRef zero-copy read-only structure"
```

---

### Task 4: BTree 读操作迁移到零拷贝

**Files:**
- Modify: `src/storage/btree/btree.rs`

- [ ] **Step 1: 写零拷贝 search 失败测试**

在 `btree.rs` 测试模块中添加（或在 tests/ 目录下添加集成测试）：

```rust
#[tokio::test]
async fn test_btree_search_zerocopy() {
    let dir = tempfile::tempdir().unwrap();
    let pool = BufferPool::open(dir.path().join("test.db"), 100).unwrap();
    let btree = BTree::create(&pool).await.unwrap();

    // Insert a key
    let key = Key::from_bytes(b"test_key");
    let row_id = RowId::new(1);
    btree.insert(&key, row_id, &pool).await.unwrap();

    // Search using zero-copy path
    let found = btree.search(&key, &pool).await.unwrap();
    assert_eq!(found, Some(row_id));
}
```

- [ ] **Step 2: 运行现有测试确认 baseline**

Run: `cargo test btree -- --nocapture 2>&1 | tail -10`
Expected: 所有现有测试通过

- [ ] **Step 3: 改造 BTree::search 使用零拷贝**

将 `btree.rs` 中 `search()` 方法的读路径从：

```rust
// 旧: guard.page() + LeafNode::from_page()
let page = guard.page();
let node = LeafNode::from_page(page);
```

改为：

```rust
// 新: guard.page_data() + LeafNodeRef::from_bytes()
let data = guard.page_data();
let node = LeafNodeRef::from_bytes(data);
```

同理改造 search 中遍历 internal node 的部分，使用 `InternalNodeRef`。

具体改动需对照 btree.rs 中 search 方法的实际代码逐行修改。核心原则：
- 所有 `guard.page()` + `LeafNode::from_page()` → `guard.page_data()` + `LeafNodeRef::from_bytes()`
- 所有 `guard.page()` + `InternalNode::from_page()` → `guard.page_data()` + `InternalNodeRef::from_bytes()`
- 写操作（insert/delete/update）保持不变

- [ ] **Step 4: 改造 BTree::scan_all 使用零拷贝**

同 search，将 scan_all 中的读路径迁移到零拷贝。

- [ ] **Step 5: 运行所有 BTree 测试**

Run: `cargo test btree -- --nocapture 2>&1 | tail -15`
Expected: 所有测试通过

- [ ] **Step 6: Commit**

```bash
git add src/storage/btree/btree.rs
git commit -m "perf(M14): migrate BTree read path to zero-copy LeafNodeRef/InternalNodeRef"
```

---

### Task 5: IndexManager 读操作迁移到零拷贝

**Files:**
- Modify: `src/storage/btree/index_manager.rs`

- [ ] **Step 1: 运行现有测试确认 baseline**

Run: `cargo test index_manager -- --nocapture 2>&1 | tail -10`
Expected: 所有测试通过

- [ ] **Step 2: 改造 IndexManager::search 零拷贝**

IndexManager 的 search 和 scan_all 内部调用 BTree 的对应方法。如果 BTree 已改造完成，IndexManager 可能无需改动（零拷贝已在 BTree 层完成）。

检查 IndexManager 是否有直接操作 page 的代码，如有则同样迁移。

- [ ] **Step 3: 运行所有测试**

Run: `cargo test -- --nocapture 2>&1 | tail -15`
Expected: 所有测试通过

- [ ] **Step 4: Commit（如有改动）**

```bash
git add src/storage/btree/index_manager.rs
git commit -m "perf(M14): migrate IndexManager read path to zero-copy"
```

---

### Task 6: PhysicalPlan Clone 验证与补全

**Files:**
- Modify: `src/executor/plan.rs`

- [ ] **Step 1: 验证 PhysicalPlan Clone 状态**

Run: `cargo check 2>&1 | grep -i "clone" | head -10`

检查 PhysicalPlan 及其所有变体是否已 derive Clone。如果已全部 derive，此 Task 仅需验证。

- [ ] **Step 2: 补全缺失的 Clone 实现**

如果编译报错某些字段不可 Clone：
- `Arc<BTree>` → Arc 本身 Clone，无需改 BTree
- `Box<PhysicalPlan>` → PhysicalPlan Clone 后自动可用
- 其他 → 逐个处理

- [ ] **Step 3: 写 Clone 正确性测试**

```rust
#[test]
fn test_physical_plan_clone() {
    let plan = PhysicalPlan::Scan(ScanNode {
        table_name: "test".to_string(),
        columns: vec!["id".to_string()],
    });
    let cloned = plan.clone();
    // PhysicalPlan derives Clone, verify it works
    assert!(matches!(cloned, PhysicalPlan::Scan(_)));
}
```

- [ ] **Step 4: 运行测试**

Run: `cargo test test_physical_plan_clone -- --nocapture 2>&1 | tail -5`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/executor/plan.rs
git commit -m "feat(M14): ensure PhysicalPlan Clone for plan caching"
```

---

### Task 7: Database 添加 plan_cache 字段

**Files:**
- Modify: `src/database.rs`

- [ ] **Step 1: 写缓存命中集成测试**

在 tests/ 目录下添加：

```rust
#[tokio::test]
async fn test_plan_cache_hit() {
    let db = Database::open_in_memory().unwrap();

    // First execution - cache miss
    let result1 = db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").await.unwrap();
    db.execute("INSERT INTO t VALUES (1, 'alice')").await.unwrap();

    let result2 = db.query("SELECT * FROM t WHERE id = 1").await.unwrap();
    assert_eq!(result2.len(), 1);

    // Second execution - should hit cache
    let result3 = db.query("SELECT * FROM t WHERE id = 1").await.unwrap();
    assert_eq!(result3.len(), 1);

    // Verify cache has entry
    let cache_len = db.plan_cache_len();
    assert!(cache_len > 0);
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test test_plan_cache_hit -- --nocapture 2>&1 | tail -10`
Expected: 编译失败（plan_cache_len 方法不存在）

- [ ] **Step 3: 在 Database 中添加 plan_cache**

修改 `database.rs`：

```rust
use lru::LruCache;
use std::num::NonZeroUsize;

pub struct Database {
    // ... existing fields ...
    plan_cache: Arc<Mutex<LruCache<String, PhysicalPlan>>>,
}

impl Database {
    pub fn open_in_memory() -> Result<Self> {
        // ... existing init ...
        let plan_cache = Arc::new(Mutex::new(
            LruCache::new(NonZeroUsize::new(256).unwrap())
        ));
        // ... construct with plan_cache ...
    }

    pub fn plan_cache_len(&self) -> usize {
        self.plan_cache.lock().unwrap().len()
    }
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test test_plan_cache_hit -- --nocapture 2>&1 | tail -10`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/database.rs
git commit -m "feat(M14): add plan_cache LRU field to Database"
```

---

### Task 8: Pipeline 集成缓存查询逻辑

**Files:**
- Modify: `src/pipeline.rs`

- [ ] **Step 1: 写缓存命中/未命中对比测试**

```rust
#[tokio::test]
async fn test_pipeline_cache_miss_then_hit() {
    let db = Database::open_in_memory().unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").await.unwrap();
    db.execute("INSERT INTO t VALUES (1)").await.unwrap();

    let sql = "SELECT * FROM t WHERE id = 1";

    // Miss
    assert_eq!(db.plan_cache_len(), 0);
    let r1 = db.query(sql).await.unwrap();
    assert_eq!(r1.len(), 1);
    let cache_size_after_miss = db.plan_cache_len();
    assert!(cache_size_after_miss > 0);

    // Hit
    let r2 = db.query(sql).await.unwrap();
    assert_eq!(r2.len(), 1);
    assert_eq!(db.plan_cache_len(), cache_size_after_miss);
}

#[tokio::test]
async fn test_ddl_clears_cache() {
    let db = Database::open_in_memory().unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)").await.unwrap();

    let sql = "SELECT * FROM t";
    db.query(sql).await.unwrap();
    assert!(db.plan_cache_len() > 0);

    // DDL should clear cache
    db.execute("CREATE TABLE t2 (id INTEGER PRIMARY KEY)").await.unwrap();
    assert_eq!(db.plan_cache_len(), 0);
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test test_pipeline_cache -- --nocapture 2>&1 | tail -10`
Expected: 缓存未生效

- [ ] **Step 3: 修改 Pipeline::execute_sql 集成缓存**

在 `pipeline.rs` 的 `execute_sql()` 方法中：

```rust
pub async fn execute_sql(&self, sql: &str) -> Result<ExecutionResult> {
    // Check cache first
    {
        let mut cache = self.plan_cache.lock().unwrap();
        if let Some(cached_plan) = cache.get(sql).cloned() {
            // Cache hit — skip parse + plan
            return self.execute_plan(cached_plan).await;
        }
    }

    // Cache miss — normal flow
    let tokens = tokenize(sql)?;
    let stmt = parse(&tokens)?;
    let plan = self.plan(&stmt)?;

    // Store in cache
    {
        let mut cache = self.plan_cache.lock().unwrap();
        cache.put(sql.to_string(), plan.clone());
    }

    self.execute_plan(plan).await
}
```

对于 DDL 操作（CREATE TABLE / DROP TABLE），在执行后清空缓存：

```rust
fn is_ddl(stmt: &Statement) -> bool {
    matches!(stmt, Statement::CreateTable(_) | Statement::Drop(_))
}

// In execute_sql, after DDL execution:
if is_ddl(&stmt) {
    self.plan_cache.lock().unwrap().clear();
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test test_pipeline_cache -- --nocapture 2>&1 | tail -10`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/pipeline.rs
git commit -m "feat(M14): integrate plan cache into Pipeline execute_sql"
```

---

### Task 9: 全量测试 + 性能验证

**Files:**
- None (verification only)

- [ ] **Step 1: 运行全量测试**

Run: `cargo test 2>&1 | tail -20`
Expected: 0 failures

- [ ] **Step 2: 运行 clippy**

Run: `cargo clippy 2>&1 | tail -10`
Expected: 0 warnings

- [ ] **Step 3: 运行 micro benchmark**

Run: `cargo bench --bench micro_bench 2>&1 | grep -E "pk_lookup|time"` 
Expected: PK 查询时间相比 M13 baseline 有明显下降

- [ ] **Step 4: 记录性能数据到 learned.md**

将 benchmark 结果记录到 `.claude/docs/learned.md`

- [ ] **Step 5: 更新 snapshot.md 和 tasks.md**

更新项目状态文档

- [ ] **Step 6: Final commit**

```bash
git add -A
git commit -m "perf(M14): BTree zero-copy + SQL plan cache — PK query 3-5x speedup"
```
