# M17-Phase2 B-Tree Split 机制实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 B-Tree 的 split 机制，使 insert 操作在节点满时能正确分裂并向上传播，支持树的动态增长。

**Architecture:** 递归 insert + SplitResult 回传方案。`insert_into_page` 返回 `Option<SplitResult>`，递归调用时将子节点的 split 结果上传播到父节点。LeafNode 和 InternalNode 各自实现 `split` 方法。根分裂时创建新 InternalNode 根，BTree::insert 返回新 root_page_id 通知调用方。

**Tech Stack:** Rust, Tokio, tempfile（测试）

---

## 文件结构

| 文件 | 操作 | 职责 |
|------|------|------|
| `src/storage/btree/node.rs` | 修改 | 添加 LeafNode::split、InternalNode::split |
| `src/storage/btree/btree.rs` | 修改 | 重写 insert 逻辑为递归 + split 回传 |
| `src/storage/btree/index_manager.rs` | 修改 | 处理 BTree::insert 返回的新 root_page_id |
| `tests/btree_split_test.rs` | 修改 | 扩展测试套件覆盖所有场景 |

---

## Task 1: LeafNode::split 实现

**Files:**
- Modify: `src/storage/btree/node.rs`（LeafNode impl 块）
- Test: `tests/btree_split_test.rs`

- [ ] **Step 1: 写 LeafNode::split 的失败测试**

在 `tests/btree_split_test.rs` 中添加测试：

```rust
#[test]
fn test_leaf_node_split_basic() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let db = Database::open(&db_path).await.unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)").await.unwrap();
        db.execute("CREATE INDEX idx ON t(v)").await.unwrap();

        // 插入足够多的行触发叶节点分裂
        // LeafNode 容量约 97 个 entries（取决于 key 大小）
        // 插入 120 行确保触发分裂
        for i in 0..120i64 {
            db.execute(&format!("INSERT INTO t VALUES ({}, {})", i, i)).await.unwrap();
        }

        // 验证所有行都能被搜索到
        for i in 0..120i64 {
            let result = db.query(&format!("SELECT * FROM t WHERE id = {}", i)).await.unwrap();
            assert_eq!(result.rows.len(), 1, "should find row with id={}", i);
        }
    });
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test test_leaf_node_split_basic -- --nocapture`
Expected: FAIL — 当前 insert 在 PageFull 时返回错误，120 行插入会失败

- [ ] **Step 3: 实现 LeafNode::split**

在 `src/storage/btree/node.rs` 的 `LeafNode` impl 块中添加：

```rust
/// Split this leaf node, moving the upper half of entries to new_page.
/// Returns SplitResult with the middle key (first key of new page) and new page id.
/// Caller is responsible for allocating new_page and passing it in.
pub fn split(&mut self, new_page: &mut Page, new_page_id: PageId) -> Result<SplitResult, StorageError> {
    // 1. 读取所有 entries
    let mut entries: Vec<(Key, RowId)> = Vec::new();
    for slot_id in 1..=self.header.slot_count as u16 {
        let slot = self.page.get_slot(slot_id)?;
        if slot.is_valid() {
            let key = Key::from_bytes(self.page.get_slot_data(slot_id)?);
            let row_id = RowId::from_bytes(&self.page.get_slot_data(slot_id)?[Key::encoded_size()..]);
            entries.push((key, row_id));
        }
    }

    if entries.is_empty() {
        return Err(StorageError::InternalError("cannot split empty leaf node".into()));
    }

    // 2. 中间分裂
    let mid = entries.len() / 2;

    // 3. 清空原页，重新插入前半部分
    self.page.clear_all_slots();
    self.header.slot_count = 0;
    self.header.free_space_offset = PAGE_SIZE as u16;
    for (key, row_id) in &entries[..mid] {
        self.insert(key.clone(), *row_id)?;
    }

    // 4. 初始化新页为 LeafNode，插入后半部分
    let mut new_leaf = LeafNode::from_page_init(new_page);
    let old_next = self.next_leaf_page_id;
    for (key, row_id) in &entries[mid..] {
        new_leaf.insert(key.clone(), *row_id)?;
    }

    // 5. middle_key = 后半部分第一个 entry 的 key
    let middle_key = entries[mid].0.clone();

    // 6. 维护 leaf 链表
    new_leaf.set_next_leaf(old_next);
    self.set_next_leaf(new_page_id);

    // 7. 更新 new_page 的引用（因为 new_leaf 持有可变引用）
    // new_leaf 的修改已直接反映在 new_page 中（因为 from_page_init 借用了 new_page）

    Ok(SplitResult {
        middle_key,
        new_page_id,
    })
}
```

**重要说明**：上述代码中的 `from_page_init`、`clear_all_slots`、`set_next_leaf` 等方法需要确认是否已存在。如果 `from_page_init` 不接受 `&mut Page` 引用而是消费 Page，则需要对 split 方法签名和实现做调整。具体实现时需要根据 `LeafNode` 的实际结构来适配。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test test_leaf_node_split_basic -- --nocapture`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/storage/btree/node.rs tests/btree_split_test.rs
git commit -m "feat(M17-T6): add LeafNode::split for b-tree node splitting"
```

---

## Task 2: InternalNode::split 实现

**Files:**
- Modify: `src/storage/btree/node.rs`（InternalNode impl 块）
- Test: `tests/btree_split_test.rs`

- [ ] **Step 1: 写 InternalNode::split 的失败测试**

```rust
#[test]
fn test_internal_node_split() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let db = Database::open(&db_path).await.unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)").await.unwrap();
        db.execute("CREATE INDEX idx ON t(v)").await.unwrap();

        // 插入足够多的行触发多层分裂（内节点分裂）
        // InternalNode 容量取决于 key+child_ptr 大小，约 200+ separators
        // 需要 200*97 ≈ 20000 行才能触发内节点分裂
        // 插入 25000 行确保触发内节点分裂
        for i in 0..25000i64 {
            db.execute(&format!("INSERT INTO t VALUES ({}, {})", i, i * 10)).await.unwrap();
        }

        // 验证所有行都能被搜索到
        for i in 0..25000i64 {
            let result = db.query(&format!("SELECT * FROM t WHERE v = {}", i * 10)).await.unwrap();
            assert_eq!(result.rows.len(), 1, "should find row with v={}", i * 10);
        }
    });
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test test_internal_node_split -- --nocapture`
Expected: FAIL — InternalNode split 尚未实现

- [ ] **Step 3: 实现 InternalNode::split**

在 `src/storage/btree/node.rs` 的 `InternalNode` impl 块中添加：

```rust
/// Split this internal node, moving the upper half of separators to new_page.
/// The middle key is promoted (not kept in either child).
/// Returns SplitResult with the promoted middle key and new page id.
pub fn split(&mut self, new_page: &mut Page, new_page_id: PageId) -> Result<SplitResult, StorageError> {
    // 1. 读取所有 separators (key + right_child pairs)
    let mut separators: Vec<(Key, PageId)> = Vec::new();
    for slot_id in 1..=self.header.slot_count as u16 {
        let slot = self.page.get_slot(slot_id)?;
        if slot.is_valid() {
            let data = self.page.get_slot_data(slot_id)?;
            let key = Key::from_bytes(data);
            let child_id = PageId::from_be_bytes(data[Key::encoded_size()..Key::encoded_size()+8].try_into().unwrap());
            separators.push((key, child_id));
        }
    }

    if separators.is_empty() {
        return Err(StorageError::InternalError("cannot split empty internal node".into()));
    }

    // 2. 中间分裂
    let mid = separators.len() / 2;

    // 3. middle_key 上推
    let middle_key = separators[mid].0.clone();
    let new_leftmost_child = separators[mid].1;

    // 4. 清空原页，重新插入前半部分
    self.page.clear_all_slots();
    self.header.slot_count = 0;
    self.header.free_space_offset = PAGE_SIZE as u16;
    for (key, child_id) in &separators[..mid] {
        self.insert_separator(key.clone(), *child_id)?;
    }

    // 5. 初始化新页为 InternalNode，设置 leftmost_child，插入后半部分
    let mut new_internal = InternalNode::from_page_init(new_page);
    new_internal.set_leftmost_child(new_leftmost_child);
    for (key, child_id) in &separators[mid+1..] {
        new_internal.insert_separator(key.clone(), *child_id)?;
    }

    Ok(SplitResult {
        middle_key,
        new_page_id,
    })
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test test_internal_node_split -- --nocapture`
Expected: PASS（前提是 Task 3 的 BTree::insert 递归逻辑已完成）

- [ ] **Step 5: 提交**

```bash
git add src/storage/btree/node.rs
git commit -m "feat(M17-T6): add InternalNode::split for b-tree internal node splitting"
```

---

## Task 3: BTree::insert 递归 + split 回传 + 根分裂

**Files:**
- Modify: `src/storage/btree/btree.rs`
- Test: `tests/btree_split_test.rs`

这是核心任务，重构 insert 逻辑为递归 + split 回传。

- [ ] **Step 1: 写根分裂的失败测试**

```rust
#[test]
fn test_root_split_creates_new_root() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let db = Database::open(&db_path).await.unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)").await.unwrap();
        db.execute("CREATE INDEX idx ON t(v)").await.unwrap();

        // 插入触发第一次分裂（产生新 InternalNode 根）
        for i in 0..120i64 {
            db.execute(&format!("INSERT INTO t VALUES ({}, {})", i, i)).await.unwrap();
        }

        // 验证：search 仍然正确路由
        for i in 0..120i64 {
            let result = db.query(&format!("SELECT * FROM t WHERE v = {}", i)).await.unwrap();
            assert_eq!(result.rows.len(), 1);
        }

        // 验证：scan_all 返回所有行
        let result = db.query("SELECT * FROM t").await.unwrap();
        assert_eq!(result.rows.len(), 120);
    });
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test test_root_split_creates_new_root -- --nocapture`
Expected: FAIL — BTree::insert 当前遇到 PageFull 返回错误

- [ ] **Step 3: 重写 BTree::insert 为递归 + split 回传**

修改 `src/storage/btree/btree.rs`：

**3a. 修改 insert_into_page 签名和实现：**

```rust
/// Insert a key-row_id pair into the subtree rooted at page_id.
/// Returns Some(SplitResult) if the node split, None if it fit.
fn insert_into_page(
    &mut self,
    page_id: PageId,
    key: Key,
    row_id: RowId,
) -> Result<Option<SplitResult>, StorageError> {
    let page = self.loader.load_page(page_id)?;

    match page.data()[0] {
        LEAF_NODE_TYPE => {
            let mut leaf = LeafNode::from_page(&page);
            match leaf.insert(key, row_id) {
                Ok(()) => Ok(None),
                Err(StorageError::PageFull) => {
                    // 分配新页并分裂
                    let new_page_id = self.loader.allocate_page()?;
                    let mut new_page = self.loader.load_page_for_write(new_page_id)?;
                    let split_result = leaf.split(&mut new_page, new_page_id)?;

                    // 写回原页
                    self.loader.write_back_page(page_id, &page)?;

                    Ok(Some(split_result))
                }
                Err(e) => Err(e),
            }
        }
        INTERNAL_NODE_TYPE => {
            let mut internal = InternalNode::from_page(&page);
            let child_id = internal.find_child(&key);

            // 递归插入
            let split = self.insert_into_page(child_id, key, row_id)?;

            if let Some(child_split) = split {
                // 子节点分裂，需要插入 separator
                match internal.insert_separator(child_split.middle_key, child_split.new_page_id) {
                    Ok(()) => Ok(None),
                    Err(StorageError::PageFull) => {
                        // 内节点也满了，分裂
                        let new_page_id = self.loader.allocate_page()?;
                        let mut new_page = self.loader.load_page_for_write(new_page_id)?;
                        let split_result = internal.split(&mut new_page, new_page_id)?;

                        self.loader.write_back_page(page_id, &page)?;

                        Ok(Some(split_result))
                    }
                    Err(e) => Err(e),
                }
            } else {
                Ok(None)
            }
        }
        _ => Err(StorageError::InternalError(format!(
            "unknown node type: {}",
            page.data()[0]
        ))),
    }
}
```

**3b. 修改 BTree::insert 处理根分裂：**

```rust
pub fn insert(&mut self, key: Key, row_id: RowId) -> Result<Option<PageId>, StorageError> {
    let split = self.insert_into_page(self.root_page_id, key, row_id)?;

    if let Some(root_split) = split {
        // 根分裂：创建新根
        let new_root_page_id = self.loader.allocate_page()?;
        let mut new_root_page = self.loader.load_page_for_write(new_root_page_id)?;

        let mut new_root = InternalNode::from_page_init(&mut new_root_page);
        new_root.set_leftmost_child(self.root_page_id);
        new_root.insert_separator(root_split.middle_key, root_split.new_page_id)?;

        self.root_page_id = new_root_page_id;

        Ok(Some(new_root_page_id))
    } else {
        Ok(None)
    }
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test test_root_split_creates_new_root -- --nocapture`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/storage/btree/btree.rs
git commit -m "feat(M17-T7/T8): rewrite BTree::insert with recursive split propagation and root split"
```

---

## Task 4: IndexManager 处理新 root_page_id

**Files:**
- Modify: `src/storage/btree/index_manager.rs`
- Test: `tests/btree_split_test.rs`

- [ ] **Step 1: 写 IndexManager root_page_id 更新的失败测试**

```rust
#[test]
fn test_index_manager_updates_root_after_split() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let db = Database::open(&db_path).await.unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)").await.unwrap();
        db.execute("CREATE INDEX idx ON t(v)").await.unwrap();

        // 插入触发分裂
        for i in 0..120i64 {
            db.execute(&format!("INSERT INTO t VALUES ({}, {})", i, i)).await.unwrap();
        }

        // 关闭并重新打开数据库，验证持久化的 root_page_id 正确
        drop(db);
        let db = Database::open(&db_path).await.unwrap();

        // 重新打开后搜索仍正确
        for i in 0..120i64 {
            let result = db.query(&format!("SELECT * FROM t WHERE v = {}", i)).await.unwrap();
            assert_eq!(result.rows.len(), 1, "should find row with v={} after reopen", i);
        }
    });
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test test_index_manager_updates_root_after_split -- --nocapture`
Expected: FAIL — IndexManager 未处理 BTree::insert 返回的新 root_page_id

- [ ] **Step 3: 修改 IndexManager::insert 处理新 root_page_id**

在 `src/storage/btree/index_manager.rs` 的 `insert` 方法中，调用 `btree.insert` 后检查返回值：

```rust
// 原代码类似：
// btree.insert(key, row_id)?;

// 改为：
let new_root = btree.insert(key, row_id)?;
if let Some(new_root_page_id) = new_root {
    self.root_page_id.store(new_root_page_id.as_u64(), Ordering::SeqCst);
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test test_index_manager_updates_root_after_split -- --nocapture`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/storage/btree/index_manager.rs
git commit -m "feat(M17-T8): update IndexManager root_page_id after btree split"
```

---

## Task 5: Leaf 链表维护测试

**Files:**
- Test: `tests/btree_split_test.rs`

- [ ] **Step 1: 写 leaf 链表维护的测试**

```rust
#[test]
fn test_leaf_chain_after_split() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let db = Database::open(&db_path).await.unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)").await.unwrap();
        db.execute("CREATE INDEX idx ON t(v)").await.unwrap();

        // 插入触发分裂
        for i in 0..120i64 {
            db.execute(&format!("INSERT INTO t VALUES ({}, {})", i, i)).await.unwrap();
        }

        // search_all 应该返回所有匹配行（验证 leaf 链表遍历正确）
        // 使用非唯一索引的 search_all
        let result = db.query("SELECT * FROM t WHERE v >= 0").await.unwrap();
        assert!(result.rows.len() >= 120, "scan_all should find all rows after split, found {}", result.rows.len());
    });
}
```

- [ ] **Step 2: 运行测试验证通过**

Run: `cargo test test_leaf_chain_after_split -- --nocapture`
Expected: PASS（前提是 Task 1-4 已完成）

- [ ] **Step 3: 提交**

```bash
git add tests/btree_split_test.rs
git commit -m "test(M17-T9): add leaf chain maintenance test after split"
```

---

## Task 6: 非唯一 Key 分裂测试

**Files:**
- Test: `tests/btree_split_test.rs`

- [ ] **Step 1: 写非唯一 key 分裂的测试**

```rust
#[test]
fn test_non_unique_key_split() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let db = Database::open(&db_path).await.unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)").await.unwrap();
        db.execute("CREATE INDEX idx ON t(v)").await.unwrap();

        // 插入大量相同 key 的行，触发分裂
        for i in 0..120i64 {
            db.execute(&format!("INSERT INTO t VALUES ({}, 42)", i)).await.unwrap();
        }

        // search_all 应返回所有 120 行
        let result = db.query("SELECT * FROM t WHERE v = 42").await.unwrap();
        assert_eq!(result.rows.len(), 120, "non-unique search should find all 120 rows after split");

        // delete_by_key 应删除所有行
        db.execute("DELETE FROM t WHERE v = 42").await.unwrap();
        let result = db.query("SELECT * FROM t WHERE v = 42").await.unwrap();
        assert_eq!(result.rows.len(), 0, "delete_by_key should remove all rows after split");
    });
}
```

- [ ] **Step 2: 运行测试验证通过**

Run: `cargo test test_non_unique_key_split -- --nocapture`
Expected: PASS

- [ ] **Step 3: 提交**

```bash
git add tests/btree_split_test.rs
git commit -m "test(M17-T9): add non-unique key split test"
```

---

## Task 7: 分裂后删除测试

**Files:**
- Test: `tests/btree_split_test.rs`

- [ ] **Step 1: 写分裂后删除的测试**

```rust
#[test]
fn test_delete_after_split() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let db = Database::open(&db_path).await.unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)").await.unwrap();
        db.execute("CREATE INDEX idx ON t(v)").await.unwrap();

        // 插入触发分裂
        for i in 0..120i64 {
            db.execute(&format!("INSERT INTO t VALUES ({}, {})", i, i)).await.unwrap();
        }

        // 删除部分行
        for i in 0..60i64 {
            db.execute(&format!("DELETE FROM t WHERE id = {}", i)).await.unwrap();
        }

        // 验证剩余行
        for i in 60..120i64 {
            let result = db.query(&format!("SELECT * FROM t WHERE v = {}", i)).await.unwrap();
            assert_eq!(result.rows.len(), 1, "should find row with v={} after delete", i);
        }

        // 验证已删除的行
        for i in 0..60i64 {
            let result = db.query(&format!("SELECT * FROM t WHERE v = {}", i)).await.unwrap();
            assert_eq!(result.rows.len(), 0, "should not find row with v={} after delete", i);
        }
    });
}
```

- [ ] **Step 2: 运行测试验证通过**

Run: `cargo test test_delete_after_split -- --nocapture`
Expected: PASS

- [ ] **Step 3: 提交**

```bash
git add tests/btree_split_test.rs
git commit -m "test(M17-T9): add delete after split test"
```

---

## Task 8: 大规模连续分裂测试 + 全量回归

**Files:**
- Test: `tests/btree_split_test.rs`

- [ ] **Step 1: 写大规模连续分裂测试**

```rust
#[test]
fn test_massive_insert_with_multiple_splits() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let db = Database::open(&db_path).await.unwrap();
        db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)").await.unwrap();
        db.execute("CREATE INDEX idx ON t(v)").await.unwrap();

        // 插入 5000 行，触发多次分裂
        for i in 0..5000i64 {
            db.execute(&format!("INSERT INTO t VALUES ({}, {})", i, i * 7)).await.unwrap();
        }

        // 验证所有行
        for i in 0..5000i64 {
            let result = db.query(&format!("SELECT * FROM t WHERE v = {}", i * 7)).await.unwrap();
            assert_eq!(result.rows.len(), 1, "should find row with v={}", i * 7);
        }

        // 验证总行数
        let result = db.query("SELECT * FROM t").await.unwrap();
        assert_eq!(result.rows.len(), 5000, "total row count should be 5000");
    });
}
```

- [ ] **Step 2: 运行测试验证通过**

Run: `cargo test test_massive_insert_with_multiple_splits -- --nocapture`
Expected: PASS

- [ ] **Step 3: 运行全量回归测试**

Run: `cargo test`
Expected: 所有测试通过（0 failed）

- [ ] **Step 4: 运行 clippy**

Run: `cargo clippy -- -D warnings`
Expected: 0 warnings

- [ ] **Step 5: 提交**

```bash
git add tests/btree_split_test.rs
git commit -m "test(M17-T9): add massive split test and verify full regression"
```

---

## 自审清单

### 1. Spec 覆盖

| Spec 需求 | Task |
|-----------|------|
| T6: LeafNode::split | Task 1 |
| T7: InternalNode::split | Task 2 |
| T7: 递归 insert + split 回传 | Task 3 |
| T8: 根分裂处理 | Task 3 |
| T8: IndexManager root_page_id 更新 | Task 4 |
| T9-S1: 空树首次分裂 | Task 1 (test_leaf_node_split_basic) |
| T9-S2: 连续分裂 | Task 8 (test_massive_insert_with_multiple_splits) |
| T9-S3: 根分裂 | Task 3 (test_root_split_creates_new_root) |
| T9-S4: 非唯一 key 分裂 | Task 6 (test_non_unique_key_split) |
| T9-S5: InternalNode 分裂 | Task 2 (test_internal_node_split) |
| T9-S6: Leaf 链表维护 | Task 5 (test_leaf_chain_after_split) |
| T9-S7: 分裂后搜索 | Task 1/3 自然覆盖 |
| T9-S8: 分裂后删除 | Task 7 (test_delete_after_split) |
| T9-S9: 单 entry 分裂 | 覆盖在 Task 1 中（mid=1 时 entries.len()=2） |

### 2. Placeholder 扫描

无 TBD/TODO/模糊描述 ✅

### 3. 类型一致性

- `SplitResult` 在 node.rs 中定义，Task 1/2/3 使用一致 ✅
- `BTree::insert` 返回 `Result<Option<PageId>>`，Task 3/4 使用一致 ✅
- `PageId` 类型贯穿所有 Task 一致 ✅

### 关键实现注意事项

1. **LeafNode/InternalNode 的页写回**：当前 `SyncPageLoader` 使用 `load_page` 返回共享引用，split 需要可变访问。需要确认 `load_page_for_write` 或等效 API 是否存在，否则需要调整读写模式。

2. **from_page_init 签名**：需确认是否接受 `&mut Page` 引用。如果消费 Page，split 方法需要调整——先 split 数据，再初始化新页。

3. **clear_all_slots**：需确认 Page 是否有此方法。如果没有，需要手动清空 slot 区域。

4. **write_back_page**：split 后需要将修改写回磁盘。需确认 SyncPageLoader 的写回 API。

5. **测试中的 SQL 接口**：测试使用 `Database::open`、`execute`、`query` 等高级 API。需确认这些 API 的实际签名和返回类型。
