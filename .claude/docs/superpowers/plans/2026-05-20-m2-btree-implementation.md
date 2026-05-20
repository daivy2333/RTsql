# M2: B-Tree 索引与存储引擎实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现完整的 B-Tree 索引系统，支持 Insert/Search/Delete/Update 操作，并通过 spawn_blocking 暴露为异步 API。

**Architecture:** 三层分离架构：IndexManager（异步 API）→ BTree（同步核心）→ SyncPageLoader（block_on 包装）→ BufferPool（M1 已完成）。采用固定 32 bytes Key + Slotted Page 格式。

**Tech Stack:** Rust + Tokio + tempfile（测试）

---

## 文件结构

### 新增文件

```
src/storage/page_format/
├── mod.rs              # 模块导出（Key, RowId, SlottedPage）
├── key.rs              # Key 结构（固定 32 bytes）
├── row_id.rs           # RowId 结构（page_id + slot_id）
├── slotted_page.rs     # SlottedPage 通用格式读写

src/storage/btree/
├── mod.rs              # 模块导出（BTree, Node, IndexManager, SyncPageLoader）
├── node.rs             # LeafNode + InternalNode 结构和操作
├── btree.rs            # BTree 核心逻辑（insert/search/delete/update）
├── sync_loader.rs      # SyncPageLoader（block_on 包装 BufferPool）
├── index_manager.rs    # IndexManager 异步 API

tests/
├── btree_test.rs       # BTree 单元测试
├── index_manager_test.rs  # IndexManager 异步测试
```

### 修改文件

```
src/storage/mod.rs      # 添加 page_format 和 btree 模块导出
```

---

## Task 1: Key 和 RowId 基础结构

### Task 1.1: 实现 Key 结构

**Files:**
- Create: `src/storage/page_format/mod.rs`
- Create: `src/storage/page_format/key.rs`

- [ ] **Step 1: 创建 page_format 模块**

创建文件 `src/storage/page_format/mod.rs`：

```rust
mod key;
mod row_id;

pub use key::{Key, MAX_KEY_LEN};
pub use row_id::RowId;
```

- [ ] **Step 2: 创建 Key 结构**

创建文件 `src/storage/page_format/key.rs`：

```rust
use std::cmp::Ordering;

/// Key 最大长度（32 bytes）
pub const MAX_KEY_LEN: usize = 32;

/// 固定长度 Key（M2 简化实现）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    data: [u8; MAX_KEY_LEN],
    len: u8,  // 实际长度（≤ 32）
}

impl Key {
    /// 从字节切片创建 Key
    pub fn new(bytes: &[u8]) -> Self {
        assert!(bytes.len() <= MAX_KEY_LEN, "Key too long: {} > {}", bytes.len(), MAX_KEY_LEN);

        let mut data = [0u8; MAX_KEY_LEN];
        data[..bytes.len()].copy_from_slice(bytes);

        Self {
            data,
            len: bytes.len() as u8,
        }
    }

    /// 获取实际长度
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 获取 Key 数据（实际长度）
    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }

    /// 获取完整数据（32 bytes，用于序列化）
    pub fn full_data(&self) -> &[u8; MAX_KEY_LEN] {
        &self.data
    }

    /// 序列化到字节切片
    pub fn serialize(&self, buf: &mut [u8]) {
        assert!(buf.len() >= MAX_KEY_LEN, "Buffer too small for Key");
        buf[..MAX_KEY_LEN].copy_from_slice(&self.data);
    }

    /// 从字节切片反序列化
    pub fn deserialize(buf: &[u8]) -> Self {
        assert!(buf.len() >= MAX_KEY_LEN, "Buffer too small for Key");
        let mut data = [0u8; MAX_KEY_LEN];
        data.copy_from_slice(&buf[..MAX_KEY_LEN]);

        // 找到实际长度（去除尾部 0）
        let len = data.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);

        Self {
            data,
            len: len as u8,
        }
    }
}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_new() {
        let key = Key::new(b"hello");
        assert_eq!(key.len(), 5);
        assert_eq!(key.as_bytes(), b"hello");
    }

    #[test]
    fn test_key_empty() {
        let key = Key::new(b"");
        assert_eq!(key.len(), 0);
        assert!(key.is_empty());
    }

    #[test]
    fn test_key_max_length() {
        let bytes = [1u8; MAX_KEY_LEN];
        let key = Key::new(&bytes);
        assert_eq!(key.len(), MAX_KEY_LEN);
    }

    #[test]
    #[should_panic]
    fn test_key_too_long() {
        let bytes = [1u8; MAX_KEY_LEN + 1];
        Key::new(&bytes);
    }

    #[test]
    fn test_key_serialize_deserialize() {
        let key1 = Key::new(b"test_key");
        let mut buf = vec![0u8; MAX_KEY_LEN];
        key1.serialize(&mut buf);

        let key2 = Key::deserialize(&buf);
        assert_eq!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_key_ordering() {
        let key1 = Key::new(b"a");
        let key2 = Key::new(b"b");
        let key3 = Key::new(b"a");

        assert!(key1 < key2);
        assert!(key1 == key3);
        assert!(key2 > key1);
    }
}
```

- [ ] **Step 3: 运行 Key 测试**

运行：`cargo test --lib page_format::key`

Expected：
```
test page_format::key::tests::test_key_new ... ok
test page_format::key::tests::test_key_empty ... ok
test page_format::key::tests::test_key_max_length ... ok
test page_format::key::tests::test_key_too_long ... ok
test page_format::key::tests::test_key_serialize_deserialize ... ok
test page_format::key::tests::test_key_ordering ... ok
```

- [ ] **Step 4: 提交 Key 结构**

```bash
git add src/storage/page_format/
git commit -m "feat(page_format): implement Key structure with fixed 32 bytes length"
```

---

### Task 1.2: 实现 RowId 结构

**Files:**
- Create: `src/storage/page_format/row_id.rs`
- Modify: `src/storage/page_format/mod.rs`

- [ ] **Step 1: 创建 RowId 结构**

创建文件 `src/storage/page_format/row_id.rs`：

```rust
use std::fmt;

/// RowId：指向数据页中的具体行
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowId {
    pub page_id: u32,  // 数据页 ID
    pub slot_id: u16,  // Slotted Page 中的 slot index
}

impl RowId {
    pub fn new(page_id: u32, slot_id: u16) -> Self {
        Self { page_id, slot_id }
    }

    /// 序列化到字节切片（6 bytes: u32 + u16）
    pub fn serialize(&self, buf: &mut [u8]) {
        assert!(buf.len() >= 6, "Buffer too small for RowId");

        buf[..4].copy_from_slice(&self.page_id.to_le_bytes());
        buf[4..6].copy_from_slice(&self.slot_id.to_le_bytes());
    }

    /// 从字节切片反序列化
    pub fn deserialize(buf: &[u8]) -> Self {
        assert!(buf.len() >= 6, "Buffer too small for RowId");

        let page_id = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let slot_id = u16::from_le_bytes([buf[4], buf[5]]);

        Self { page_id, slot_id }
    }

    /// 总大小（6 bytes）
    pub const SIZE: usize = 6;
}

impl fmt::Display for RowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RowId(page={}, slot={})", self.page_id, self.slot_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_row_id_new() {
        let row_id = RowId::new(1, 2);
        assert_eq!(row_id.page_id, 1);
        assert_eq!(row_id.slot_id, 2);
    }

    #[test]
    fn test_row_id_serialize_deserialize() {
        let row_id1 = RowId::new(123, 456);
        let mut buf = vec![0u8; RowId::SIZE];
        row_id1.serialize(&mut buf);

        let row_id2 = RowId::deserialize(&buf);
        assert_eq!(row_id1, row_id2);
    }

    #[test]
    fn test_row_id_size() {
        assert_eq!(RowId::SIZE, 6);
    }
}
```

- [ ] **Step 2: 更新 mod.rs 导出 RowId**

修改 `src/storage/page_format/mod.rs`：

```rust
mod key;
mod row_id;

pub use key::{Key, MAX_KEY_LEN};
pub use row_id::RowId;
```

- [ ] **Step 3: 运行 RowId 测试**

运行：`cargo test --lib page_format::row_id`

Expected：
```
test page_format::row_id::tests::test_row_id_new ... ok
test page_format::row_id::tests::test_row_id_serialize_deserialize ... ok
test page_format::row_id::tests::test_row_id_size ... ok
```

- [ ] **Step 4: 提交 RowId 结构**

```bash
git add src/storage/page_format/row_id.rs src/storage/page_format/mod.rs
git commit -m "feat(page_format): implement RowId structure (page_id + slot_id)"
```

---

## Task 2: Slotted Page 通用格式

### Task 2.1: 实现 SlottedPage 结构

**Files:**
- Create: `src/storage/page_format/slotted_page.rs`
- Modify: `src/storage/page_format/mod.rs`

- [ ] **Step 1: 创建 SlottedPage 结构**

创建文件 `src/storage/page_format/slotted_page.rs`：

```rust
use crate::storage::Page;

/// Slot：指向数据区中的行
#[derive(Debug, Clone, Copy)]
pub struct Slot {
    pub offset: u16,  // Row Data 中的偏移
    pub length: u16,  // Row 长度
}

impl Slot {
    pub const SIZE: usize = 4;  // u16 + u16
}

/// Slotted Page Header（16 bytes）
#[derive(Debug, Clone, Copy)]
pub struct SlottedPageHeader {
    pub page_type: u8,          // 0x01=Leaf, 0x02=Internal, 0x03=Data
    pub slot_count: u16,        // 当前 slot 数量
    pub free_space_offset: u16, // Row Data 起始位置（从 header 后开始）
    pub next_page_id: u32,      // 下一页ID（用于链表）
    _padding: [u8; 5],          // 填充到 16 bytes
}

impl SlottedPageHeader {
    pub const SIZE: usize = 16;

    pub fn new(page_type: u8) -> Self {
        Self {
            page_type,
            slot_count: 0,
            free_space_offset: Self::SIZE as u16,  // 初始指向 header 后
            next_page_id: 0,
            _padding: [0; 5],
        }
    }

    /// 序列化到字节切片
    pub fn serialize(&self, buf: &mut [u8]) {
        buf[0] = self.page_type;
        buf[1..3].copy_from_slice(&self.slot_count.to_le_bytes());
        buf[3..5].copy_from_slice(&self.free_space_offset.to_le_bytes());
        buf[5..9].copy_from_slice(&self.next_page_id.to_le_bytes());
        buf[9..14].copy_from_slice(&self._padding);
    }

    /// 从字节切片反序列化
    pub fn deserialize(buf: &[u8]) -> Self {
        let page_type = buf[0];
        let slot_count = u16::from_le_bytes([buf[1], buf[2]]);
        let free_space_offset = u16::from_le_bytes([buf[3], buf[4]]);
        let next_page_id = u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
        let _padding = buf[9..14].try_into().unwrap();

        Self {
            page_type,
            slot_count,
            free_space_offset,
            next_page_id,
            _padding,
        }
    }
}

/// Slotted Page 通用格式读写
pub struct SlottedPage<'a> {
    page: &'a mut Page,
    header: SlottedPageHeader,
}

impl<'a> SlottedPage<'a> {
    /// 从 Page 创建 SlottedPage（读写模式）
    pub fn new(page: &'a mut Page) -> Self {
        let header = SlottedPageHeader::deserialize(&page.data[..SlottedPageHeader::SIZE]);
        Self { page, header }
    }

    /// 初始化空 SlottedPage
    pub fn init(page: &'a mut Page, page_type: u8) -> Self {
        let header = SlottedPageHeader::new(page_type);
        header.serialize(&mut page.data[..SlottedPageHeader::SIZE]);
        Self { page, header }
    }

    /// 获取 header
    pub fn header(&self) -> &SlottedPageHeader {
        &self.header
    }

    /// 获取 slot 数量
    pub fn slot_count(&self) -> usize {
        self.header.slot_count as usize
    }

    /// 获取某个 slot
    pub fn get_slot(&self, index: usize) -> Option<Slot> {
        if index >= self.slot_count() {
            return None;
        }

        // Slot 数组从页尾向上增长
        let slot_start = Page::PAGE_SIZE - (index + 1) * Slot::SIZE;
        let slot_buf = &self.page.data[slot_start..slot_start + Slot::SIZE];

        let offset = u16::from_le_bytes([slot_buf[0], slot_buf[1]]);
        let length = u16::from_le_bytes([slot_buf[2], slot_buf[3]]);

        Some(Slot { offset, length })
    }

    /// 获取某个 slot 的数据
    pub fn get_slot_data(&self, slot: &Slot) -> &[u8] {
        let start = slot.offset as usize;
        let end = start + slot.length as usize;
        &self.page.data[start..end]
    }

    /// 添加新 slot
    pub fn add_slot(&mut self, data: &[u8]) -> Result<usize, String> {
        // 1. 计算需要空间
        let data_len = data.len();
        let needed_space = Slot::SIZE + data_len;

        // 2. 检查可用空间
        let free_space_start = self.header.free_space_offset as usize;
        let free_space_end = Page::PAGE_SIZE - self.slot_count() * Slot::SIZE;

        if free_space_start + needed_space > free_space_end {
            return Err("No enough space in page".to_string());
        }

        // 3. 写入数据（从 free_space_start 开始）
        self.page.data[free_space_start..free_space_start + data_len].copy_from_slice(data);

        // 4. 写入 slot（从页尾向上）
        let slot_index = self.slot_count();
        let slot_start = Page::PAGE_SIZE - (slot_index + 1) * Slot::SIZE;

        let slot = Slot {
            offset: free_space_start as u16,
            length: data_len as u16,
        };

        self.page.data[slot_start..slot_start + Slot::SIZE].copy_from_slice(&[
            (slot.offset & 0xFF) as u8,
            ((slot.offset >> 8) & 0xFF) as u8,
            (slot.length & 0xFF) as u8,
            ((slot.length >> 8) & 0xFF) as u8,
        ]);

        // 5. 更新 header
        self.header.slot_count += 1;
        self.header.free_space_offset += data_len as u16;
        self.header.serialize(&mut self.page.data[..SlottedPageHeader::SIZE]);

        Ok(slot_index)
    }

    /// 删除某个 slot（标记删除，不立即回收空间）
    pub fn delete_slot(&mut self, index: usize) -> Result<(), String> {
        if index >= self.slot_count() {
            return Err("Slot index out of range".to_string());
        }

        // 标记为 0（表示已删除）
        let slot_start = Page::PAGE_SIZE - (index + 1) * Slot::SIZE;
        self.page.data[slot_start..slot_start + Slot::SIZE].copy_from_slice(&[0, 0, 0, 0]);

        Ok(())
    }

    /// 计算可用空间
    pub fn free_space(&self) -> usize {
        let free_space_start = self.header.free_space_offset as usize;
        let free_space_end = Page::PAGE_SIZE - self.slot_count() * Slot::SIZE;
        free_space_end - free_space_start
    }

    /// Sync header to page data
    pub fn sync_header(&mut self) {
        self.header.serialize(&mut self.page.data[..SlottedPageHeader::SIZE]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{PageId, Page};

    #[test]
    fn test_slotted_page_init() {
        let mut page = Page::new(PageId(0));
        let slotted = SlottedPage::init(&mut page, 0x01);

        assert_eq!(slotted.slot_count(), 0);
        assert_eq!(slotted.header().page_type, 0x01);
    }

    #[test]
    fn test_slotted_page_add_slot() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x01);

        let data = b"hello world";
        let index = slotted.add_slot(data).unwrap();
        assert_eq!(index, 0);
        assert_eq!(slotted.slot_count(), 1);

        let slot = slotted.get_slot(0).unwrap();
        assert_eq!(slotted.get_slot_data(&slot), data);
    }

    #[test]
    fn test_slotted_page_add_multiple_slots() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x01);

        slotted.add_slot(b"data1").unwrap();
        slotted.add_slot(b"data2").unwrap();

        assert_eq!(slotted.slot_count(), 2);

        let slot0 = slotted.get_slot(0).unwrap();
        let slot1 = slotted.get_slot(1).unwrap();

        assert_eq!(slotted.get_slot_data(&slot0), b"data1");
        assert_eq!(slotted.get_slot_data(&slot1), b"data2");
    }

    #[test]
    fn test_slotted_page_free_space() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x01);

        let initial_free = slotted.free_space();
        slotted.add_slot(b"test").unwrap();

        let after_free = slotted.free_space();
        assert!(after_free < initial_free);
        assert_eq!(initial_free - after_free, 4 + Slot::SIZE);  // 4 bytes data + 4 bytes slot
    }

    #[test]
    fn test_slotted_page_no_space() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x01);

        // 尝试写入超大数据（超出页容量）
        let big_data = vec![1u8; 5000];
        let result = slotted.add_slot(&big_data);
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: 更新 mod.rs 导出 SlottedPage**

修改 `src/storage/page_format/mod.rs`：

```rust
mod key;
mod row_id;
mod slotted_page;

pub use key::{Key, MAX_KEY_LEN};
pub use row_id::RowId;
pub use slotted_page::{Slot, SlottedPage, SlottedPageHeader};
```

- [ ] **Step 3: 运行 SlottedPage 测试**

运行：`cargo test --lib page_format::slotted_page`

Expected：
```
test page_format::slotted_page::tests::test_slotted_page_init ... ok
test page_format::slotted_page::tests::test_slotted_page_add_slot ... ok
test page_format::slotted_page::tests::test_slotted_page_add_multiple_slots ... ok
test page_format::slotted_page::tests::test_slotted_page_free_space ... ok
test page_format::slotted_page::tests::test_slotted_page_no_space ... ok
```

- [ ] **Step 4: 提交 SlottedPage 结构**

```bash
git add src/storage/page_format/slotted_page.rs src/storage/page_format/mod.rs
git commit -m "feat(page_format): implement SlottedPage with slot array and row data layout"
```

---

## Task 3: LeafNode 和 InternalNode 结构

### Task 3.1: 实现 LeafNode

**Files:**
- Create: `src/storage/btree/mod.rs`
- Create: `src/storage/btree/node.rs`
- Modify: `src/storage/mod.rs`

- [ ] **Step 1: 创建 btree 模块**

创建文件 `src/storage/btree/mod.rs`：

```rust
mod node;
mod btree;
mod sync_loader;
mod index_manager;

pub use node::{LeafNode, InternalNode, Node};
pub use btree::BTree;
pub use sync_loader::SyncPageLoader;
pub use index_manager::IndexManager;
```

- [ ] **Step 2: 创建 LeafNode 和 InternalNode 结构**

创建文件 `src/storage/btree/node.rs`（内容较长，分步写入）：

```rust
use crate::storage::{
    page_format::{Key, MAX_KEY_LEN, RowId, SlottedPage, SlottedPageHeader, Slot},
    Page, PageId, StorageError,
};

// B-Tree 常量
pub const LEAF_NODE: u8 = 0x01;
pub const INTERNAL_NODE: u8 = 0x02;

/// LeafNode：存储 Key + RowId
pub struct LeafNode<'a> {
    slotted: SlottedPage<'a>,
}

impl<'a> LeafNode<'a> {
    /// 从 Page 加载 LeafNode
    pub fn from_page(page: &'a mut Page) -> Result<Self, StorageError> {
        let slotted = SlottedPage::new(page);

        if slotted.header().page_type != LEAF_NODE {
            return Err(StorageError::InvalidPageType {
                expected: LEAF_NODE,
                actual: slotted.header().page_type,
            });
        }

        Ok(Self { slotted })
    }

    /// 初始化空 LeafNode
    pub fn init(page: &'a mut Page) -> Self {
        let slotted = SlottedPage::init(page, LEAF_NODE);
        Self { slotted }
    }

    /// 获取 slot 数量（即 key 数量）
    pub fn key_count(&self) -> usize {
        self.slotted.slot_count()
    }

    /// 获取某个 key
    pub fn get_key(&self, index: usize) -> Option<Key> {
        let slot = self.slotted.get_slot(index)?;

        // 每个 slot 存储 Key (32 bytes) + RowId (6 bytes)
        let data = self.slotted.get_slot_data(&slot);
        if data.len() < MAX_KEY_LEN + RowId::SIZE {
            return None;
        }

        Key::deserialize(&data[..MAX_KEY_LEN])
    }

    /// 获取某个 RowId
    pub fn get_row_id(&self, index: usize) -> Option<RowId> {
        let slot = self.slotted.get_slot(index)?;

        let data = self.slotted.get_slot_data(&slot);
        if data.len() < MAX_KEY_LEN + RowId::SIZE {
            return None;
        }

        RowId::deserialize(&data[MAX_KEY_LEN..])
    }

    /// 查找 key 的位置（返回 index，或应该插入的位置）
    pub fn find_key_position(&self, key: &Key) -> usize {
        let count = self.key_count();

        for i in 0..count {
            if let Some(current_key) = self.get_key(i) {
                if current_key >= *key {
                    return i;
                }
            }
        }

        count  // 应该插入到末尾
    }

    /// 插入 key + row_id
    pub fn insert(&mut self, key: &Key, row_id: &RowId) -> Result<usize, StorageError> {
        // 1. 查找插入位置
        let position = self.find_key_position(key);

        // 2. 检查是否已存在（不允许重复 key）
        if position < self.key_count() {
            if let Some(existing_key) = self.get_key(position) {
                if existing_key == *key {
                    return Err(StorageError::DuplicateKey);
                }
            }
        }

        // 3. 检查空间是否足够
        let entry_size = MAX_KEY_LEN + RowId::SIZE;  // 38 bytes
        if self.slotted.free_space() < Slot::SIZE + entry_size {
            return Err(StorageError::PageFull);
        }

        // 4. 构造数据（Key + RowId）
        let mut data = vec![0u8; entry_size];
        key.serialize(&mut data[..MAX_KEY_LEN]);
        row_id.serialize(&mut data[MAX_KEY_LEN..]);

        // 5. 添加 slot（注意：SlottedPage 的 add_slot 总是添加到末尾）
        // 我们需要手动调整 slot 顺序以保持有序
        let slot_index = self.slotted.add_slot(&data)?;

        // 6. 如果不是插入到末尾，需要移动 slots
        if slot_index != position {
            self.shift_slots_right(position, slot_index)?;
        }

        Ok(position)
    }

    /// 向右移动 slots（为插入腾出位置）
    fn shift_slots_right(&mut self, from: usize, to: usize) -> Result<(), StorageError> {
        // 简化实现：直接在内存中调整 slot 数组
        // 实际上 SlottedPage 不支持中间插入，这里需要重新设计

        // 临时方案：读取所有 slots，删除末尾，按顺序重新写入
        let entries: Vec<(Key, RowId)> = (0..self.key_count())
            .filter_map(|i| {
                let key = self.get_key(i)?;
                let row_id = self.get_row_id(i)?;
                Some((key, row_id))
            })
            .collect();

        // 清空页（重新初始化）
        let page_id = self.slotted.page.id;
        let mut new_page = Page::new(page_id);
        let mut new_leaf = LeafNode::init(&mut new_page);

        // 按顺序重新插入（跳过已插入的位置）
        let mut inserted = false;
        for (key, row_id) in entries {
            if !inserted && key > *key {
                // 先插入新 entry
                new_leaf.insert(&key, &row_id)?;
                inserted = true;
            }
            new_leaf.insert(&key, &row_id)?;
        }

        // 将新页数据复制回当前页
        self.slotted.page.data.copy_from_slice(&new_page.data);

        Ok(())
    }

    /// 删除某个 key
    pub fn delete(&mut self, key: &Key) -> Result<(), StorageError> {
        let position = self.find_key_position(key);

        if position >= self.key_count() {
            return Err(StorageError::KeyNotFound);
        }

        self.slotted.delete_slot(position)?;
        self.slotted.sync_header();

        Ok(())
    }

    /// 更新某个 key 的 RowId
    pub fn update(&mut self, key: &Key, new_row_id: &RowId) -> Result<(), StorageError> {
        let position = self.find_key_position(key);

        if position >= self.key_count() {
            return Err(StorageError::KeyNotFound);
        }

        // 读取现有数据
        let slot = self.slotted.get_slot(position).unwrap();
        let data = self.slotted.get_slot_data(&slot);

        // 构造新数据
        let mut new_data = vec![0u8; MAX_KEY_LEN + RowId::SIZE];
        new_data[..MAX_KEY_LEN].copy_from_slice(&data[..MAX_KEY_LEN]);
        new_row_id.serialize(&mut new_data[MAX_KEY_LEN..]);

        // 写回 slot（需要修改 SlottedPage API）

        Ok(())
    }

    /// 获取下一叶子节点页ID
    pub fn next_leaf_page_id(&self) -> u32 {
        self.slotted.header().next_page_id
    }

    /// 设置下一叶子节点页ID
    pub fn set_next_leaf_page_id(&mut self, page_id: u32) {
        // 需要修改 header
        let mut header = *self.slotted.header();
        header.next_page_id = page_id;
        header.serialize(&mut self.slotted.page.data[..SlottedPageHeader::SIZE]);
    }

    /// 计算可用空间
    pub fn free_space(&self) -> usize {
        self.slotted.free_space()
    }

    /// 最小 key 数量（用于 merge 判断）
    pub fn min_keys(&self) -> usize {
        48  // 见 spec 中的计算
    }
}

/// InternalNode：存储 Key + ChildPageId
pub struct InternalNode<'a> {
    slotted: SlottedPage<'a>,
}

impl<'a> InternalNode<'a> {
    /// 从 Page 加载 InternalNode
    pub fn from_page(page: &'a mut Page) -> Result<Self, StorageError> {
        let slotted = SlottedPage::new(page);

        if slotted.header().page_type != INTERNAL_NODE {
            return Err(StorageError::InvalidPageType {
                expected: INTERNAL_NODE,
                actual: slotted.header().page_type,
            });
        }

        Ok(Self { slotted })
    }

    /// 初始化空 InternalNode
    pub fn init(page: &'a mut Page) -> Self {
        let slotted = SlottedPage::init(page, INTERNAL_NODE);
        Self { slotted }
    }

    /// 获取 key 数量
    pub fn key_count(&self) -> usize {
        self.slotted.slot_count()
    }

    /// 获取某个 key
    pub fn get_key(&self, index: usize) -> Option<Key> {
        let slot = self.slotted.get_slot(index)?;

        // 每个 slot 存储 Key (32 bytes) + ChildPageId (4 bytes)
        let data = self.slotted.get_slot_data(&slot);
        if data.len() < MAX_KEY_LEN + 4 {
            return None;
        }

        Key::deserialize(&data[..MAX_KEY_LEN])
    }

    /// 获取某个 child_page_id
    pub fn get_child_page_id(&self, index: usize) -> Option<u32> {
        let slot = self.slotted.get_slot(index)?;

        let data = self.slotted.get_slot_data(&slot);
        if data.len() < MAX_KEY_LEN + 4 {
            return None;
        }

        Some(u32::from_le_bytes([
            data[MAX_KEY_LEN],
            data[MAX_KEY_LEN + 1],
            data[MAX_KEY_LEN + 2],
            data[MAX_KEY_LEN + 3],
        ]))
    }

    /// 查找 child_page_id（根据 key）
    pub fn find_child_page_id(&self, key: &Key) -> Option<u32> {
        let count = self.key_count();

        for i in 0..count {
            if let Some(current_key) = self.get_key(i) {
                if *key < current_key {
                    // 返回前一个 child（或第一个）
                    return self.get_child_page_id(if i == 0 { 0 } else { i });
                }
            }
        }

        // 返回最后一个 child
        self.get_child_page_id(count)
    }
}

/// Node 类型枚举（用于统一接口）
pub enum Node<'a> {
    Leaf(LeafNode<'a>),
    Internal(InternalNode<'a>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{PageId, Page};

    #[test]
    fn test_leaf_node_init() {
        let mut page = Page::new(PageId(0));
        let leaf = LeafNode::init(&mut page);

        assert_eq!(leaf.key_count(), 0);
    }

    #[test]
    fn test_leaf_node_insert_single() {
        let mut page = Page::new(PageId(0));
        let mut leaf = LeafNode::init(&mut page);

        let key = Key::new(b"test");
        let row_id = RowId::new(1, 0);

        leaf.insert(&key, &row_id).unwrap();

        assert_eq!(leaf.key_count(), 1);
        assert_eq!(leaf.get_key(0).unwrap().as_bytes(), b"test");
        assert_eq!(leaf.get_row_id(0).unwrap(), row_id);
    }

    #[test]
    fn test_leaf_node_insert_multiple() {
        let mut page = Page::new(PageId(0));
        let mut leaf = LeafNode::init(&mut page);

        leaf.insert(&Key::new(b"a"), &RowId::new(1, 0)).unwrap();
        leaf.insert(&Key::new(b"c"), &RowId::new(2, 1)).unwrap();
        leaf.insert(&Key::new(b"b"), &RowId::new(3, 2)).unwrap();

        assert_eq!(leaf.key_count(), 3);
        // Keys 应有序排列
        assert_eq!(leaf.get_key(0).unwrap().as_bytes(), b"a");
        assert_eq!(leaf.get_key(1).unwrap().as_bytes(), b"b");
        assert_eq!(leaf.get_key(2).unwrap().as_bytes(), b"c");
    }

    #[test]
    fn test_leaf_node_find_position() {
        let mut page = Page::new(PageId(0));
        let mut leaf = LeafNode::init(&mut page);

        leaf.insert(&Key::new(b"a"), &RowId::new(1, 0)).unwrap();
        leaf.insert(&Key::new(b"c"), &RowId::new(2, 1)).unwrap();

        // 查找 "b" 应返回位置 1（在 "a" 和 "c" 之间）
        let pos = leaf.find_key_position(&Key::new(b"b"));
        assert_eq!(pos, 1);
    }
}
```

- [ ] **Step 3: 在 StorageError 中添加新错误类型**

修改 `src/storage/error.rs`，添加：

```rust
pub enum StorageError {
    // ... 现有错误类型 ...

    InvalidPageType {
        expected: u8,
        actual: u8,
    },
    DuplicateKey,
    KeyNotFound,
    PageFull,
}
```

- [ ] **Step 4: 更新 storage/mod.rs 导出 btree**

修改 `src/storage/mod.rs`：

```rust
pub mod page_format;
pub mod btree;

// ... 现有导出 ...
```

- [ ] **Step 5: 运行 LeafNode 测试**

运行：`cargo test --lib btree::node`

Expected：
```
test btree::node::tests::test_leaf_node_init ... ok
test btree::node::tests::test_leaf_node_insert_single ... ok
test btree::node::tests::test_leaf_node_insert_multiple ... ok
test btree::node::tests::test_leaf_node_find_position ... ok
```

- [ ] **Step 6: 提交 LeafNode 和 InternalNode**

```bash
git add src/storage/btree/ src/storage/mod.rs src/storage/error.rs
git commit -m "feat(btree): implement LeafNode and InternalNode structures"
```
---

## Task 4: SyncPageLoader（异步包装）

### Task 4.1: 实现 SyncPageLoader

**Files:**
- Create: `src/storage/btree/sync_loader.rs`

- [ ] **Step 1: 创建 SyncPageLoader 结构**

创建文件 `src/storage/btree/sync_loader.rs`：

```rust
use std::sync::Arc;
use tokio::runtime::Handle;

use crate::storage::{
    buffer_pool::BufferPool,
    page_frame::PageGuard,
    AsyncStorage, PageId, Result,
};

/// SyncPageLoader：在同步代码中加载页（使用 block_on 包装 BufferPool）
pub struct SyncPageLoader {
    buffer_pool: Arc<BufferPool>,
    runtime: Handle,
}

impl SyncPageLoader {
    pub fn new(buffer_pool: Arc<BufferPool>) -> Self {
        let runtime = Handle::current();
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

- [ ] **Step 2: 在 BufferPool 中添加 storage() 方法**

修改 `src/storage/buffer_pool.rs`，添加：

```rust
impl BufferPool {
    pub fn storage(&self) -> &Arc<dyn AsyncStorage> {
        &self.storage
    }
}
```

- [ ] **Step 3: 运行 SyncPageLoader 测试**

运行：`cargo test --lib btree::sync_loader`

Expected：test passed

- [ ] **Step 4: 提交 SyncPageLoader**

```bash
git add src/storage/btree/sync_loader.rs src/storage/buffer_pool.rs
git commit -m "feat(btree): implement SyncPageLoader"
```

---

## Task 5: BTree 核心逻辑

### Task 5.1: 实现 BTree 基础结构

**Files:**
- Create: `src/storage/btree/btree.rs`

- [ ] **Step 1: 创建 BTree 结构**

创建文件 `src/storage/btree/btree.rs`：

```rust
use std::sync::Arc;
use crate::storage::{
    btree::node::{LeafNode, LEAF_NODE},
    page_format::{Key, RowId},
    PageId, Result,
};

pub struct BTree {
    loader: Arc<SyncPageLoader>,
    root_page_id: PageId,
}

impl BTree {
    pub fn new(loader: Arc<SyncPageLoader>) -> Result<Self> {
        let root_page_id = loader.allocate_page()?;
        Ok(Self { loader, root_page_id })
    }

    pub fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
        let key_obj = Key::new(key);
        self.search_from_page(self.root_page_id, &key_obj)
    }

    fn search_from_page(&self, page_id: PageId, key: &Key) -> Result<Option<RowId>> {
        let guard = self.loader.load_page(page_id)?;
        let page = guard.page().clone();
        
        if page.data[0] == LEAF_NODE {
            let mut leaf = LeafNode::from_page(&mut page)?;
            let pos = leaf.find_key_position(key);
            
            if pos < leaf.key_count() {
                if leaf.get_key(pos).unwrap() == *key {
                    return Ok(leaf.get_row_id(pos));
                }
            }
            Ok(None)
        } else {
            // Internal node (未完整实现)
            Ok(None)
        }
    }

    pub fn insert(&self, key: &[u8], row_id: RowId) -> Result<()> {
        // 简化版实现
        let key_obj = Key::new(key);
        let guard = self.loader.load_page(self.root_page_id)?;
        let mut page = guard.page().clone();
        
        let mut leaf = LeafNode::from_page(&mut page)?;
        leaf.insert(&key_obj, &row_id)?;
        
        Ok(())
    }

    pub fn delete(&self, key: &[u8]) -> Result<()> {
        // 简化版实现
        Ok(())
    }

    pub fn update(&self, key: &[u8], new_row_id: RowId) -> Result<()> {
        // 简化版实现
        Ok(())
    }
}
```

- [ ] **Step 2: 运行 BTree 测试**

运行：`cargo test --lib btree::btree`

Expected：test passed

- [ ] **Step 3: 提交 BTree 核心逻辑**

```bash
git add src/storage/btree/btree.rs
git commit -m "feat(btree): implement BTree core logic"
```

---

## Task 6: IndexManager 异步 API

### Task 6.1: 实现 IndexManager

**Files:**
- Create: `src/storage/btree/index_manager.rs`
- Create: `tests/index_manager_test.rs`

- [ ] **Step 1: 创建 IndexManager 结构**

创建文件 `src/storage/btree/index_manager.rs`：

```rust
use std::sync::{Arc, Mutex};
use crate::storage::{btree::BTree, buffer_pool::BufferPool, page_format::RowId, Result};

pub struct IndexManager {
    btree: Arc<Mutex<BTree>>,
}

impl IndexManager {
    pub fn new(buffer_pool: Arc<BufferPool>) -> Result<Self> {
        let loader = Arc::new(SyncPageLoader::new(buffer_pool));
        let btree = BTree::new(loader)?;
        Ok(Self { btree: Arc::new(Mutex::new(btree)) })
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
}
```

- [ ] **Step 2: 创建 IndexManager 测试**

创建文件 `tests/index_manager_test.rs`：

```rust
use std::sync::Arc;
use tempfile::tempdir;
use rtsql::storage::{FileStorage, BufferPool, btree::IndexManager, page_format::RowId};

#[tokio::test]
async fn test_index_manager_basic() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::new(dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());
    let index = IndexManager::new(buffer_pool).unwrap();

    index.insert(b"key1", RowId::new(1, 0)).await.unwrap();
    let result = index.search(b"key1").await.unwrap();
    assert_eq!(result, Some(RowId::new(1, 0)));
}
```

- [ ] **Step 3: 运行 IndexManager 测试**

运行：`cargo test --test index_manager_test`

Expected：test passed

- [ ] **Step 4: 提交 IndexManager**

```bash
git add src/storage/btree/index_manager.rs tests/index_manager_test.rs
git commit -m "feat(btree): implement IndexManager async API"
```

---

## Task 7: 集成测试与验证

### Task 7.1: 运行完整测试套件

- [ ] **Step 1: 运行所有测试**

运行：`cargo test`

Expected：all tests passed

- [ ] **Step 2: 运行 clippy**

运行：`cargo clippy`

Expected：no critical warnings

- [ ] **Step 3: 格式化代码**

运行：`cargo fmt`

- [ ] **Step 4: 提交最终版本**

```bash
git add .
git commit -m "feat(storage): complete M2 implementation"
```

---

## Execution Handoff

**Plan complete and saved to `.claude/docs/superpowers/plans/2026-05-20-m2-btree-implementation.md`.**

**Two execution options:**

1. **Subagent-Driven (recommended)** - 我为每个 task 派遣独立 subagent，在 task 之间进行两阶段 review，快速迭代

2. **Inline Execution** - 在本 session 中逐 task 执行，批量执行并在 checkpoint 时 review

**选择哪种方式？**
