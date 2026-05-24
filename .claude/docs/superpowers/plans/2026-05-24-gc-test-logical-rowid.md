# gc_test Logical Row ID 修复实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 gc_test 3 个 panic，通过引入 logical_id 解耦 RowId.slot_id 与物理 slot_index

**Architecture:** 在 SlottedPage 的 Slot 条目中新增 logical_id: u16 字段（4B→6B），add_slot 返回 logical_id 作为 RowId.slot_id，read/delete 通过 logical_id 查找。compact 时只更新 slot 位置，logical_id 不变。

**Tech Stack:** Rust, tokio (async tests), tempfile (test fixtures)

---

## File Structure

| 文件 | 变更类型 | 职责 |
|------|----------|------|
| `src/storage/page_format/slotted_page.rs` | 重构 | Slot 6B + logical_id API + Header next_logical_id |
| `src/storage/page_format/row_id.rs` | 注释更新 | slot_id 语义说明更新 |
| `src/storage/data_page.rs` | 修改 | read/write/update/delete 改用 logical_id |
| `src/storage/btree/node.rs` | 适配 | add_slot 返回值适配 (logical_id, slot_index) |
| `tests/gc_test.rs` | 无变更 | 修复后应全部通过 |

---

### Task 1: 扩展 Slot 结构和 SlottedPageHeader

**Files:**
- Modify: `src/storage/page_format/slotted_page.rs:1-62`

- [ ] **Step 1: 写失败测试 — Slot::SIZE 变为 6**

在 `slotted_page.rs` 的 `mod tests` 中添加测试：

```rust
#[test]
fn test_slot_size_is_6() {
    assert_eq!(Slot::SIZE, 6, "Slot must be 6 bytes: logical_id(2) + offset(2) + length(2)");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test test_slot_size_is_6 -- --nocapture`
Expected: FAIL (Slot::SIZE == 4)

- [ ] **Step 3: 修改 Slot 结构**

```rust
#[derive(Debug, Clone, Copy)]
pub struct Slot {
    pub logical_id: u16, // Logical ID (stable across compact)
    pub offset: u16,     // Offset into Row Data area
    pub length: u16,     // Row length
}

impl Slot {
    pub const SIZE: usize = 6; // u16 + u16 + u16
}
```

- [ ] **Step 4: 修改 SlottedPageHeader — 新增 next_logical_id**

```rust
pub struct SlottedPageHeader {
    pub page_type: u8,
    pub slot_count: u16,
    pub free_space_offset: u16,
    pub next_page_id: u32,
    pub next_logical_id: u16, // 新增：下一个可分配的 logical_id
    _padding: [u8; 3],        // 缩减：5 → 3
}

impl SlottedPageHeader {
    pub const SIZE: usize = 16;

    pub fn new(page_type: u8) -> Self {
        Self {
            page_type,
            slot_count: 0,
            free_space_offset: Self::SIZE as u16,
            next_page_id: 0,
            next_logical_id: 0,
            _padding: [0; 3],
        }
    }

    pub fn serialize(&self, buf: &mut [u8]) {
        buf[0] = self.page_type;
        buf[1..3].copy_from_slice(&self.slot_count.to_le_bytes());
        buf[3..5].copy_from_slice(&self.free_space_offset.to_le_bytes());
        buf[5..9].copy_from_slice(&self.next_page_id.to_le_bytes());
        buf[9..11].copy_from_slice(&self.next_logical_id.to_le_bytes());
        buf[11..14].copy_from_slice(&self._padding);
    }

    pub fn deserialize(buf: &[u8]) -> Self {
        let page_type = buf[0];
        let slot_count = u16::from_le_bytes([buf[1], buf[2]]);
        let free_space_offset = u16::from_le_bytes([buf[3], buf[4]]);
        let next_page_id = u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
        let next_logical_id = u16::from_le_bytes([buf[9], buf[10]]);
        let _padding = buf[11..14].try_into().unwrap();

        Self {
            page_type,
            slot_count,
            free_space_offset,
            next_page_id,
            next_logical_id,
            _padding,
        }
    }
}
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test test_slot_size_is_6 -- --nocapture`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/storage/page_format/slotted_page.rs
git commit -m "refactor: expand Slot to 6B with logical_id, add next_logical_id to header"
```

---

### Task 2: 重构 SlottedPage 读写方法 — logical_id 支持

**Files:**
- Modify: `src/storage/page_format/slotted_page.rs:64-258`

- [ ] **Step 1: 写失败测试 — add_slot 返回 (logical_id, slot_index)**

```rust
#[test]
fn test_add_slot_returns_logical_id() {
    let mut page = Page::new(PageId(0));
    let mut slotted = SlottedPage::init(&mut page, 0x03);

    let (lid0, idx0) = slotted.add_slot(b"data0").unwrap();
    let (lid1, idx1) = slotted.add_slot(b"data1").unwrap();

    assert_eq!(lid0, 0, "first logical_id should be 0");
    assert_eq!(lid1, 1, "second logical_id should be 1");
    assert_eq!(idx0, 0, "first slot_index should be 0");
    assert_eq!(idx1, 1, "second slot_index should be 1");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test test_add_slot_returns_logical_id -- --nocapture`
Expected: FAIL (add_slot returns usize not tuple)

- [ ] **Step 3: 重构 SlottedPageRef — Slot 读取包含 logical_id**

修改 `SlottedPageRef::get_slot` 读取 6B Slot：

```rust
pub fn get_slot(&self, index: usize) -> Option<Slot> {
    if index >= self.slot_count() {
        return None;
    }
    let slot_start = Page::PAGE_SIZE - (index + 1) * Slot::SIZE;
    let slot_buf = &self.data[slot_start..slot_start + Slot::SIZE];
    let logical_id = u16::from_le_bytes([slot_buf[0], slot_buf[1]]);
    let offset = u16::from_le_bytes([slot_buf[2], slot_buf[3]]);
    let length = u16::from_le_bytes([slot_buf[4], slot_buf[5]]);
    Some(Slot { logical_id, offset, length })
}
```

新增 `get_slot_by_logical_id`：

```rust
pub fn get_slot_by_logical_id(&self, logical_id: u16) -> Option<(Slot, usize)> {
    for i in 0..self.slot_count() {
        if let Some(slot) = self.get_slot(i) {
            if slot.logical_id == logical_id {
                return Some((slot, i));
            }
        }
    }
    None
}
```

- [ ] **Step 4: 重构 SlottedPage — Slot 读写包含 logical_id**

修改 `SlottedPage::get_slot` 同 SlottedPageRef。

修改 `add_slot` 返回 `(u16, usize)`：

```rust
pub fn add_slot(&mut self, data: &[u8]) -> Result<(u16, usize), String> {
    let data_len = data.len();
    let needed_space = Slot::SIZE + data_len;

    let free_space_start = self.header.free_space_offset as usize;
    let free_space_end = Page::PAGE_SIZE - self.slot_count() * Slot::SIZE;

    if free_space_start + needed_space > free_space_end {
        return Err("No enough space in page".to_string());
    }

    self.page.data[free_space_start..free_space_start + data_len].copy_from_slice(data);

    let slot_index = self.slot_count();
    let slot_start = Page::PAGE_SIZE - (slot_index + 1) * Slot::SIZE;

    let logical_id = self.header.next_logical_id;
    let slot = Slot {
        logical_id,
        offset: free_space_start as u16,
        length: data_len as u16,
    };

    self.page.data[slot_start..slot_start + Slot::SIZE].copy_from_slice(&[
        (slot.logical_id & 0xFF) as u8,
        ((slot.logical_id >> 8) & 0xFF) as u8,
        (slot.offset & 0xFF) as u8,
        ((slot.offset >> 8) & 0xFF) as u8,
        (slot.length & 0xFF) as u8,
        ((slot.length >> 8) & 0xFF) as u8,
    ]);

    self.header.slot_count += 1;
    self.header.free_space_offset += data_len as u16;
    self.header.next_logical_id += 1;
    self.header.serialize(&mut self.page.data[..SlottedPageHeader::SIZE]);

    Ok((logical_id, slot_index))
}
```

新增 `get_slot_by_logical_id` 和 `delete_slot_by_logical_id`：

```rust
pub fn get_slot_by_logical_id(&self, logical_id: u16) -> Option<(Slot, usize)> {
    for i in 0..self.slot_count() {
        if let Some(slot) = self.get_slot(i) {
            if slot.logical_id == logical_id {
                return Some((slot, i));
            }
        }
    }
    None
}

pub fn delete_slot_by_logical_id(&mut self, logical_id: u16) -> Result<(), String> {
    let (_, slot_index) = self.get_slot_by_logical_id(logical_id)
        .ok_or_else(|| format!("logical_id {} not found", logical_id))?;
    self.delete_slot(slot_index)
}
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test test_add_slot_returns_logical_id -- --nocapture`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/storage/page_format/slotted_page.rs
git commit -m "feat: add logical_id to SlottedPage, get/delete by logical_id"
```

---

### Task 3: 适配 data_page.rs — 使用 logical_id

**Files:**
- Modify: `src/storage/data_page.rs:24-140`

- [ ] **Step 1: 写失败测试 — 删除后通过 RowId 仍可读取**

在 `data_page.rs` 的 `mod tests` 中添加：

```rust
#[tokio::test]
async fn delete_then_read_surviving_tuple() {
    let (pool, table, _dir) = setup().await;

    let vh1 = VersionHeader::new(1, Some(1));
    let vh2 = VersionHeader::new(2, Some(2));

    let rid1 = write_tuple_to_data_page(&pool, &table, &vh1, b"tuple-a")
        .await
        .unwrap();
    let rid2 = write_tuple_to_data_page(&pool, &table, &vh2, b"tuple-b")
        .await
        .unwrap();

    // Delete rid1, then rid2 should still be readable via its RowId
    delete_tuple_from_data_page(&pool, rid1).await.unwrap();

    let (_, data) = read_tuple_from_data_page(&pool, rid2).await.unwrap();
    assert_eq!(data, b"tuple-b");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test delete_then_read_surviving_tuple -- --nocapture`
Expected: FAIL (read uses slot_index, after delete compact the index shifts)

- [ ] **Step 3: 修改 write_tuple_to_data_page — 使用 logical_id 构造 RowId**

```rust
pub async fn write_tuple_to_data_page(
    buffer_pool: &Arc<BufferPool>,
    table_meta: &Arc<TableMeta>,
    version_header: &VersionHeader,
    tuple_bytes: &[u8],
) -> Result<RowId> {
    let mut slot_data = version_header.to_bytes();
    slot_data.extend_from_slice(tuple_bytes);

    let tail_id = *table_meta.data_page_tail.lock().unwrap();
    let guard = buffer_pool.get_page(tail_id).await?;

    let add_result: std::result::Result<(u16, usize), String> = guard.modify_page(|page| {
        let page_type = page.data[0];
        let mut slotted = if page_type == 0 {
            SlottedPage::init(page, 0x03)
        } else {
            SlottedPage::new(page)
        };
        slotted.add_slot(&slot_data)
    });

    match add_result {
        Ok((logical_id, _slot_index)) => Ok(RowId::new(tail_id.0 as u32, logical_id)),
        Err(_) => {
            let new_page_id = buffer_pool.storage().allocate_page().await?;
            let new_guard = buffer_pool.get_page(new_page_id).await?;
            new_guard.modify_page(|page| {
                SlottedPage::init(page, 0x03);
            });
            guard.modify_page(|page| {
                let next_id = new_page_id.0 as u32;
                page.data[5..9].copy_from_slice(&next_id.to_le_bytes());
            });
            let (logical_id, _slot_index): (u16, usize) = new_guard
                .modify_page(|page| {
                    let mut slotted = SlottedPage::new(page);
                    slotted.add_slot(&slot_data)
                })
                .map_err(|_| StorageError::PageFull)?;
            *table_meta.data_page_tail.lock().unwrap() = new_page_id;
            Ok(RowId::new(new_page_id.0 as u32, logical_id))
        }
    }
}
```

- [ ] **Step 4: 修改 read_tuple_from_data_page — 通过 logical_id 查找**

```rust
pub async fn read_tuple_from_data_page(
    buffer_pool: &BufferPool,
    row_id: RowId,
) -> Result<(VersionHeader, Vec<u8>)> {
    let page_id = PageId(row_id.page_id as u64);
    let guard = buffer_pool.get_page(page_id).await?;

    let data_guard = guard.page_data();
    let slotted = SlottedPageRef::new(&data_guard);

    let (slot, _) = slotted
        .get_slot_by_logical_id(row_id.slot_id)
        .ok_or(StorageError::SlotNotFound(row_id))?;

    let slot_data = slotted.get_slot_data(&slot);

    let version_header =
        VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE]).ok_or_else(|| {
            StorageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "malformed version header",
            ))
        })?;

    let tuple_bytes = slot_data[VersionHeader::SIZE..].to_vec();
    Ok((version_header, tuple_bytes))
}
```

- [ ] **Step 5: 修改 update_version_header_in_data_page — 通过 logical_id 查找**

```rust
pub async fn update_version_header_in_data_page(
    buffer_pool: &BufferPool,
    row_id: RowId,
    new_header: VersionHeader,
    _tuple_bytes: &[u8],
) -> Result<()> {
    let page_id = PageId(row_id.page_id as u64);

    let page_guard = buffer_pool.get_page(page_id).await?;

    let result: std::result::Result<(), String> = page_guard.modify_page(|page| {
        let slotted = SlottedPage::new(page);

        let (slot, _) = slotted
            .get_slot_by_logical_id(row_id.slot_id)
            .ok_or_else(|| format!("logical_id {} not found", row_id.slot_id))?;
        let slot_offset = slot.offset as usize;

        let header_bytes = new_header.to_bytes();
        page.data[slot_offset..slot_offset + VersionHeader::SIZE].copy_from_slice(&header_bytes);
        Ok(())
    });

    result.map_err(|_| StorageError::SlotNotFound(row_id))?;
    Ok(())
}
```

- [ ] **Step 6: 修改 delete_tuple_from_data_page — 通过 logical_id 删除**

```rust
pub async fn delete_tuple_from_data_page(buffer_pool: &BufferPool, row_id: RowId) -> Result<()> {
    let page_id = PageId(row_id.page_id as u64);

    let page_guard = buffer_pool.get_page(page_id).await?;

    let result: std::result::Result<(), String> = page_guard.modify_page(|page| {
        let mut slotted = SlottedPage::new(page);
        slotted.delete_slot_by_logical_id(row_id.slot_id)
    });

    result.map_err(|_| StorageError::SlotNotFound(row_id))?;
    Ok(())
}
```

- [ ] **Step 7: 运行测试确认通过**

Run: `cargo test delete_then_read_surviving_tuple -- --nocapture`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add src/storage/data_page.rs
git commit -m "feat: data_page uses logical_id for read/update/delete"
```

---

### Task 4: 适配 B-Tree 层 — add_slot 返回值变更

**Files:**
- Modify: `src/storage/btree/node.rs:106-200`

- [ ] **Step 1: 修改 LeafNode::insert — add_slot 返回 (logical_id, slot_index)**

Line 134-137 修改：

```rust
// 旧:
let slot_index = self
    .slotted
    .add_slot(&data)
    .map_err(|e| StorageError::Io(std::io::Error::other(e)))?;

// 新:
let (_logical_id, slot_index) = self
    .slotted
    .add_slot(&data)
    .map_err(|e| StorageError::Io(std::io::Error::other(e)))?;
```

- [ ] **Step 2: 修改 LeafNode::insert_simple — add_slot 返回值**

Line 197-199 修改：

```rust
// 旧:
self.slotted
    .add_slot(&data)
    .map_err(|e| StorageError::Io(std::io::Error::other(e)))?;

// 新:
let _: (u16, usize) = self
    .slotted
    .add_slot(&data)
    .map_err(|e| StorageError::Io(std::io::Error::other(e)))?;
```

- [ ] **Step 3: 修改 InternalNode::insert 和 insert_simple — 同样适配**

找到所有 `add_slot` 调用，适配返回值 `(u16, usize)`。

- [ ] **Step 4: 修改 B-Tree 测试中的 add_slot 调用**

Line ~964, ~970, ~1023 等处的 `slotted.add_slot(...)` 需要适配新返回值。

- [ ] **Step 5: 运行 B-Tree 测试确认通过**

Run: `cargo test btree -- --nocapture`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/storage/btree/node.rs
git commit -m "fix: adapt btree node.rs to add_slot returning (logical_id, slot_index)"
```

---

### Task 5: 更新 RowId 注释 + 全量测试验证

**Files:**
- Modify: `src/storage/page_format/row_id.rs:3-8`

- [ ] **Step 1: 更新 RowId 注释**

```rust
/// RowId：指向数据页中的具体行
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowId {
    pub page_id: u32, // 数据页 ID
    pub slot_id: u16, // Logical ID in SlottedPage (stable across compact)
}
```

- [ ] **Step 2: 运行全量测试**

Run: `cargo test 2>&1 | tail -20`
Expected: All tests pass, 0 failures

- [ ] **Step 3: 运行 gc_test 验证**

Run: `cargo test gc_test -- --nocapture`
Expected: 3 tests PASS

- [ ] **Step 4: 运行 Clippy**

Run: `cargo clippy 2>&1 | tail -10`
Expected: 0 warnings

- [ ] **Step 5: Commit**

```bash
git add src/storage/page_format/row_id.rs
git commit -m "docs: update RowId.slot_id comment to reflect logical_id semantics"
```

---

### Task 6: 新增 SlottedPage logical_id 单元测试

**Files:**
- Modify: `src/storage/page_format/slotted_page.rs` (tests section)

- [ ] **Step 1: 添加测试 — logical_id 递增且删除后不变**

```rust
#[test]
fn test_logical_id_increment() {
    let mut page = Page::new(PageId(0));
    let mut slotted = SlottedPage::init(&mut page, 0x03);

    let (lid0, _) = slotted.add_slot(b"a").unwrap();
    let (lid1, _) = slotted.add_slot(b"b").unwrap();
    let (lid2, _) = slotted.add_slot(b"c").unwrap();

    assert_eq!(lid0, 0);
    assert_eq!(lid1, 1);
    assert_eq!(lid2, 2);
}

#[test]
fn test_delete_preserves_logical_id() {
    let mut page = Page::new(PageId(0));
    let mut slotted = SlottedPage::init(&mut page, 0x03);

    let (lid0, _) = slotted.add_slot(b"a").unwrap();
    let (lid1, _) = slotted.add_slot(b"b").unwrap();
    let (lid2, _) = slotted.add_slot(b"c").unwrap();

    // Delete lid1 (logical_id=1)
    slotted.delete_slot_by_logical_id(lid1).unwrap();

    // lid0 and lid2 should still be accessible by logical_id
    let (slot0, _) = slotted.get_slot_by_logical_id(lid0).unwrap();
    assert_eq!(slotted.get_slot_data(&slot0), b"a");

    let (slot2, _) = slotted.get_slot_by_logical_id(lid2).unwrap();
    assert_eq!(slotted.get_slot_data(&slot2), b"c");

    // lid1 should be gone
    assert!(slotted.get_slot_by_logical_id(lid1).is_none());
}

#[test]
fn test_get_by_logical_id_after_compact() {
    let mut page = Page::new(PageId(0));
    let mut slotted = SlottedPage::init(&mut page, 0x03);

    let (lid0, _) = slotted.add_slot(b"data0").unwrap();
    let (lid1, _) = slotted.add_slot(b"data1").unwrap();
    let (lid2, _) = slotted.add_slot(b"data2").unwrap();

    // Delete lid0 → compact shifts lid1 and lid2
    slotted.delete_slot_by_logical_id(lid0).unwrap();

    // Verify lid1 and lid2 still accessible with correct data
    let (slot1, _) = slotted.get_slot_by_logical_id(lid1).unwrap();
    assert_eq!(slotted.get_slot_data(&slot1), b"data1");

    let (slot2, _) = slotted.get_slot_by_logical_id(lid2).unwrap();
    assert_eq!(slotted.get_slot_data(&slot2), b"data2");
}
```

- [ ] **Step 2: 运行新测试**

Run: `cargo test test_logical_id -- --nocapture`
Expected: All PASS

- [ ] **Step 3: Commit**

```bash
git add src/storage/page_format/slotted_page.rs
git commit -m "test: add logical_id unit tests for SlottedPage"
```

---

### Task 7: 最终验证 — 全量测试 + Clippy + gc_test

**Files:** None (verification only)

- [ ] **Step 1: 全量 cargo test**

Run: `cargo test 2>&1 | tail -30`
Expected: All tests pass, 0 failures

- [ ] **Step 2: gc_test 专项验证**

Run: `cargo test gc_test -- --nocapture`
Expected: 3 tests PASS

- [ ] **Step 3: Clippy 验证**

Run: `cargo clippy 2>&1 | grep "warning" | head -5`
Expected: 0 warnings (or only pre-existing cargo config warnings)

- [ ] **Step 4: 更新项目文档**

更新 `.claude/docs/tasks.md` — gc_test bug 标记为已修复，M18 Phase3 T3 解除阻塞。
更新 `.claude/docs/snapshot.md` — 当前状态反映修复完成。
更新 `.claude/docs/learned.md` — 记录 logical_id 设计决策。
