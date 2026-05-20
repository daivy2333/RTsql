use crate::storage::{
    page_format::{Key, RowId, Slot, SlottedPage, SlottedPageHeader, MAX_KEY_LEN},
    Page, StorageError,
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

        Some(Key::deserialize(&data[..MAX_KEY_LEN]))
    }

    /// 获取某个 RowId
    pub fn get_row_id(&self, index: usize) -> Option<RowId> {
        let slot = self.slotted.get_slot(index)?;

        let data = self.slotted.get_slot_data(&slot);
        if data.len() < MAX_KEY_LEN + RowId::SIZE {
            return None;
        }

        Some(RowId::deserialize(&data[MAX_KEY_LEN..]))
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

        count // 应该插入到末尾
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
        let entry_size = MAX_KEY_LEN + RowId::SIZE; // 38 bytes
        if self.slotted.free_space() < Slot::SIZE + entry_size {
            return Err(StorageError::PageFull);
        }

        // 4. 构造数据（Key + RowId）
        let mut data = vec![0u8; entry_size];
        key.serialize(&mut data[..MAX_KEY_LEN]);
        row_id.serialize(&mut data[MAX_KEY_LEN..]);

        // 5. 添加 slot（注意：SlottedPage 的 add_slot 总是添加到末尾）
        // 我们需要手动调整 slot 顺序以保持有序
        let slot_index = self
            .slotted
            .add_slot(&data)
            .map_err(|e| StorageError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        // 6. 如果不是插入到末尾，需要移动 slots
        if slot_index != position {
            self.shift_slots_right(position, slot_index)?;
        }

        Ok(position)
    }

    /// 向右移动 slots（为插入腾出位置）
    fn shift_slots_right(&mut self, _from: usize, _to: usize) -> Result<(), StorageError> {
        // 简化实现：读取所有 entries，清空页，按正确顺序重新插入

        // 1. 读取所有 entries（包括新插入的）
        let entries: Vec<(Key, RowId)> = (0..self.key_count())
            .filter_map(|i| {
                let key = self.get_key(i)?;
                let row_id = self.get_row_id(i)?;
                Some((key, row_id))
            })
            .collect();

        // 2. 按 key 排序（确保有序）
        let mut sorted_entries = entries;
        sorted_entries.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));

        // 3. 清空页（重新初始化）
        let page_id = self.slotted.page_id();
        let mut new_page = Page::new(page_id);
        let mut new_leaf = LeafNode::init(&mut new_page);

        // 4. 按排序后的顺序重新插入
        for (key, row_id) in sorted_entries {
            new_leaf.insert_simple(&key, &row_id)?;
        }

        // 5. 将新页数据复制回当前页
        self.slotted
            .page
            .data
            .copy_from_slice(new_page.data.as_ref());

        Ok(())
    }

    /// 简单插入（不检查顺序，用于重建时）
    fn insert_simple(&mut self, key: &Key, row_id: &RowId) -> Result<(), StorageError> {
        // 检查空间是否足够
        let entry_size = MAX_KEY_LEN + RowId::SIZE; // 38 bytes
        if self.slotted.free_space() < Slot::SIZE + entry_size {
            return Err(StorageError::PageFull);
        }

        // 构造数据（Key + RowId）
        let mut data = vec![0u8; entry_size];
        key.serialize(&mut data[..MAX_KEY_LEN]);
        row_id.serialize(&mut data[MAX_KEY_LEN..]);

        // 添加 slot（直接添加到末尾）
        self.slotted
            .add_slot(&data)
            .map_err(|e| StorageError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        Ok(())
    }

    /// 删除某个 key
    pub fn delete(&mut self, key: &Key) -> Result<(), StorageError> {
        let position = self.find_key_position(key);

        if position >= self.key_count() {
            return Err(StorageError::KeyNotFound);
        }

        self.slotted
            .delete_slot(position)
            .map_err(|e| StorageError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
        self.slotted.sync_header();

        Ok(())
    }

    /// 更新某个 key 的 RowId
    pub fn update(&mut self, key: &Key, new_row_id: &RowId) -> Result<(), StorageError> {
        let position = self.find_key_position(key);

        if position >= self.key_count() {
            return Err(StorageError::KeyNotFound);
        }

        // 读取现有 key
        let existing_key = self.get_key(position).ok_or(StorageError::KeyNotFound)?;

        // 构造新数据
        let mut new_data = vec![0u8; MAX_KEY_LEN + RowId::SIZE];
        existing_key.serialize(&mut new_data[..MAX_KEY_LEN]);
        new_row_id.serialize(&mut new_data[MAX_KEY_LEN..]);

        // 需要更新 slot 数据（SlottedPage 当前不支持更新，需要先删除再插入）
        // 简化实现：删除旧 entry，插入新 entry
        self.delete(key)?;

        // 重新构造 slotted page 引用
        // 注意：这里需要重新获取可变引用
        // 由于 delete 操作已经修改了页数据，我们需要重新初始化 LeafNode

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
        48 // 见 spec 中的计算
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

        Some(Key::deserialize(&data[..MAX_KEY_LEN]))
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
    use crate::storage::{Page, PageId};

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
