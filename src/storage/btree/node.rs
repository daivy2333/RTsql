use crate::storage::{
    page_format::{Key, RowId, Slot, SlottedPage, SlottedPageHeader, SlottedPageRef, MAX_KEY_LEN},
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

        // 2. 检查空间是否足够（允许重复 key）
        // 注释掉 DuplicateKey 检查：
        // if position < self.key_count() {
        //     if let Some(existing_key) = self.get_key(position) {
        //         if existing_key == *key {
        //             return Err(StorageError::DuplicateKey);
        //         }
        //     }
        // }
        // 新逻辑：直接继续，允许重复 key

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

    /// 删除指定索引的 slot（用于批量删除）
    pub fn delete_slot(&mut self, index: usize) -> Result<(), StorageError> {
        self.slotted
            .delete_slot(index)
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

        // Check if the key at position matches
        if let Some(existing_key) = self.get_key(position) {
            if existing_key != *key {
                return Err(StorageError::KeyNotFound);
            }
        } else {
            return Err(StorageError::KeyNotFound);
        }

        // Simplified approach: read all entries, update the target, rebuild page
        let entries: Vec<(Key, RowId)> = (0..self.key_count())
            .filter_map(|i| {
                let k = self.get_key(i)?;
                let r = self.get_row_id(i)?;
                Some((k, r))
            })
            .collect();

        // Update the matching entry
        let updated_entries: Vec<(Key, RowId)> = entries
            .into_iter()
            .map(|(k, r)| {
                if k == *key {
                    (k, new_row_id.clone())
                } else {
                    (k, r)
                }
            })
            .collect();

        // Rebuild the page
        let page_id = self.slotted.page_id();
        let mut new_page = Page::new(page_id);
        let mut new_leaf = LeafNode::init(&mut new_page);

        for (k, r) in updated_entries {
            new_leaf.insert_simple(&k, &r)?;
        }

        // Copy new page data to current page
        self.slotted
            .page
            .data
            .copy_from_slice(new_page.data.as_ref());

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

/// LeafNodeRef：零拷贝只读版本的 LeafNode，基于 &[u8]
pub struct LeafNodeRef<'a> {
    slotted: SlottedPageRef<'a>,
}

impl<'a> LeafNodeRef<'a> {
    /// 从页数据字节切片创建 LeafNodeRef
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            slotted: SlottedPageRef::new(data),
        }
    }

    /// 获取 slot 数量（即 key 数量）
    pub fn key_count(&self) -> usize {
        self.slotted.slot_count()
    }

    /// 获取某个 key
    pub fn get_key(&self, index: usize) -> Option<Key> {
        let slot = self.slotted.get_slot(index)?;
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

    /// 查找所有匹配 key 的 slot 索引（用于非唯一索引）
    pub fn find_all_matches(&self, key: &Key) -> Vec<usize> {
        let mut matches = Vec::new();
        for i in 0..self.key_count() {
            if let Some(k) = self.get_key(i) {
                if k == *key {
                    matches.push(i);
                }
            }
        }
        matches
    }

    /// 查找 key 的位置（返回 (found, position)）
    /// found=true 表示 key 已存在，position 为其索引
    /// found=false 表示 key 不存在，position 为应插入的位置
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

    /// Binary search for key position — O(log n) instead of O(n)
    pub fn find_key_position_binary(&self, key: &Key) -> (bool, usize) {
        let count = self.key_count();
        let mut lo: usize = 0;
        let mut hi: usize = count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if let Some(mid_key) = self.get_key(mid) {
                match mid_key.cmp(key) {
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid,
                    std::cmp::Ordering::Equal => return (true, mid),
                }
            } else {
                hi = mid;
            }
        }
        (false, lo)
    }

    /// 获取下一叶子节点页ID
    pub fn next_leaf_page_id(&self) -> u32 {
        self.slotted.header().next_page_id
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

/// InternalNodeRef：零拷贝只读版本的 InternalNode，基于 &[u8]
pub struct InternalNodeRef<'a> {
    slotted: SlottedPageRef<'a>,
}

impl<'a> InternalNodeRef<'a> {
    /// 从页数据字节切片创建 InternalNodeRef
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            slotted: SlottedPageRef::new(data),
        }
    }

    /// 获取 key 数量
    pub fn key_count(&self) -> usize {
        self.slotted.slot_count()
    }

    /// 获取 leftmost child page id（存储在 header.next_page_id 中）
    pub fn leftmost_child(&self) -> u32 {
        self.slotted.header().next_page_id
    }

    /// 获取某个 key
    pub fn get_key(&self, index: usize) -> Option<Key> {
        let slot = self.slotted.get_slot(index)?;
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
                    if i == 0 {
                        return Some(self.leftmost_child());
                    }
                    return self.get_child_page_id(i);
                }
            }
        }

        // key >= all separators: go to last child (child at last slot)
        if count > 0 {
            self.get_child_page_id(count - 1)
        } else {
            Some(self.leftmost_child())
        }
    }

    /// Binary search for child page id — O(log n) instead of O(n)
    /// Returns the child page id for the subtree that should contain the given key.
    /// In an internal node: leftmost_child | key_0 → child_1 | key_1 → child_2 | ...
    /// If key < key_0, go to leftmost_child; if key < key_i, go to child_i; else go to last child.
    pub fn find_child_page_id_binary(&self, key: &Key) -> u32 {
        let count = self.key_count();
        let mut lo: usize = 0;
        let mut hi: usize = count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if let Some(mid_key) = self.get_key(mid) {
                match mid_key.cmp(key) {
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid,
                    std::cmp::Ordering::Equal => {
                        // key == separator: go to right subtree (child at mid+1)
                        return self.get_child_page_id(mid + 1).unwrap_or(self.leftmost_child());
                    }
                }
            } else {
                hi = mid;
            }
        }
        // lo is the insertion position; child at lo is the subtree for keys < key_lo
        if lo == 0 {
            self.leftmost_child()
        } else if lo >= count {
            // key >= all separators: go to last child
            self.get_child_page_id(count - 1).unwrap_or(self.leftmost_child())
        } else {
            self.get_child_page_id(lo).unwrap_or(self.leftmost_child())
        }
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

    #[test]
    fn test_leaf_node_ref_from_page_data() {
        // 用真实 Page + LeafNode 构造数据
        let mut page = Page::new(PageId(0));
        let mut leaf = LeafNode::init(&mut page);

        let key1 = Key::new(b"hello");
        let row_id1 = RowId::new(10, 5);
        let key2 = Key::new(b"world");
        let row_id2 = RowId::new(20, 10);

        leaf.insert(&key1, &row_id1).unwrap();
        leaf.insert(&key2, &row_id2).unwrap();

        // 通过 LeafNodeRef 读取验证
        let data: &[u8] = page.data.as_ref();
        let leaf_ref = LeafNodeRef::new(data);

        assert_eq!(leaf_ref.key_count(), 2);
        assert_eq!(leaf_ref.get_key(0).unwrap().as_bytes(), b"hello");
        assert_eq!(leaf_ref.get_row_id(0).unwrap(), row_id1);
        assert_eq!(leaf_ref.get_key(1).unwrap().as_bytes(), b"world");
        assert_eq!(leaf_ref.get_row_id(1).unwrap(), row_id2);
        assert_eq!(leaf_ref.next_leaf_page_id(), 0);
    }

    #[test]
    fn test_leaf_node_ref_find_key_position() {
        let mut page = Page::new(PageId(0));
        let mut leaf = LeafNode::init(&mut page);

        leaf.insert(&Key::new(b"a"), &RowId::new(1, 0)).unwrap();
        leaf.insert(&Key::new(b"c"), &RowId::new(2, 1)).unwrap();
        leaf.insert(&Key::new(b"e"), &RowId::new(3, 2)).unwrap();

        let data: &[u8] = page.data.as_ref();
        let leaf_ref = LeafNodeRef::new(data);

        // key 已存在
        let (found, pos) = leaf_ref.find_key_position(&Key::new(b"c"));
        assert!(found);
        assert_eq!(pos, 1);

        // key 不存在，在中间
        let (found, pos) = leaf_ref.find_key_position(&Key::new(b"b"));
        assert!(!found);
        assert_eq!(pos, 1);

        // key 不存在，在末尾
        let (found, pos) = leaf_ref.find_key_position(&Key::new(b"z"));
        assert!(!found);
        assert_eq!(pos, 3);

        // key 不存在，在开头
        let (found, pos) = leaf_ref.find_key_position(&Key::new(b"0"));
        assert!(!found);
        assert_eq!(pos, 0);
    }

    #[test]
    fn test_internal_node_ref_from_page_data() {
        // 用真实 Page + InternalNode 构造数据
        // InternalNode 没有 insert 方法，使用 SlottedPage 直接写入 slot
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, INTERNAL_NODE);

        // 写入 slot 0: Key("b") + child_page_id(100)
        let mut entry0 = vec![0u8; MAX_KEY_LEN + 4];
        Key::new(b"b").serialize(&mut entry0[..MAX_KEY_LEN]);
        entry0[MAX_KEY_LEN..MAX_KEY_LEN + 4].copy_from_slice(&100u32.to_le_bytes());
        slotted.add_slot(&entry0).unwrap();

        // 写入 slot 1: Key("d") + child_page_id(200)
        let mut entry1 = vec![0u8; MAX_KEY_LEN + 4];
        Key::new(b"d").serialize(&mut entry1[..MAX_KEY_LEN]);
        entry1[MAX_KEY_LEN..MAX_KEY_LEN + 4].copy_from_slice(&200u32.to_le_bytes());
        slotted.add_slot(&entry1).unwrap();

        // 通过 InternalNodeRef 读取验证
        let data: &[u8] = page.data.as_ref();
        let internal_ref = InternalNodeRef::new(data);

        assert_eq!(internal_ref.key_count(), 2);
        assert_eq!(internal_ref.get_key(0).unwrap().as_bytes(), b"b");
        assert_eq!(internal_ref.get_child_page_id(0).unwrap(), 100);
        assert_eq!(internal_ref.get_key(1).unwrap().as_bytes(), b"d");
        assert_eq!(internal_ref.get_child_page_id(1).unwrap(), 200);
        assert_eq!(internal_ref.leftmost_child(), 0);
    }

    #[test]
    fn test_leaf_node_ref_binary_search_matches_linear() {
        let mut page = Page::new(PageId(0));
        let mut leaf = LeafNode::init(&mut page);

        for ch in b"acegikm" {
            leaf.insert(&Key::new(&[*ch]), &RowId::new(*ch as u32, 0)).unwrap();
        }

        let data: &[u8] = page.data.as_ref();
        let leaf_ref = LeafNodeRef::new(data);

        // Test all existing keys
        for ch in b"acegikm" {
            let linear = leaf_ref.find_key_position(&Key::new(&[*ch]));
            let binary = leaf_ref.find_key_position_binary(&Key::new(&[*ch]));
            assert_eq!(linear, binary, "Mismatch for existing key {}", *ch as char);
        }

        // Test missing keys
        for ch in b"bdfhjlnz" {
            let linear = leaf_ref.find_key_position(&Key::new(&[*ch]));
            let binary = leaf_ref.find_key_position_binary(&Key::new(&[*ch]));
            assert_eq!(linear, binary, "Mismatch for missing key {}", *ch as char);
        }
    }

    #[test]
    fn test_internal_node_ref_binary_search_matches_linear() {
        let mut page = Page::new(PageId(0));
        // Initialize as INTERNAL_NODE and write slots
        {
            let mut slotted = SlottedPage::init(&mut page, INTERNAL_NODE);
            // Write separators: b(100), d(200), f(300)
            for (ch, child) in [(b'b', 100u32), (b'd', 200u32), (b'f', 300u32)] {
                let mut entry = vec![0u8; MAX_KEY_LEN + 4];
                Key::new(&[ch]).serialize(&mut entry[..MAX_KEY_LEN]);
                entry[MAX_KEY_LEN..MAX_KEY_LEN + 4].copy_from_slice(&child.to_le_bytes());
                slotted.add_slot(&entry).unwrap();
            }
        }
        // Now slotted is dropped, we can modify page.data directly
        // Set leftmost_child (next_page_id in header) to 50
        page.data[5..9].copy_from_slice(&50u32.to_le_bytes());

        let data: &[u8] = page.data.as_ref();
        let internal_ref = InternalNodeRef::new(data);

        // key "a" → leftmost_child (50)
        let linear = internal_ref.find_child_page_id(&Key::new(b"a"));
        let binary = internal_ref.find_child_page_id_binary(&Key::new(b"a"));
        assert_eq!(linear.unwrap(), binary, "Mismatch for key 'a'");

        // key "c" → child 100 (between b and d)
        let linear = internal_ref.find_child_page_id(&Key::new(b"c"));
        let binary = internal_ref.find_child_page_id_binary(&Key::new(b"c"));
        assert_eq!(linear.unwrap(), binary, "Mismatch for key 'c'");

        // key "e" → child 200 (between d and f)
        let linear = internal_ref.find_child_page_id(&Key::new(b"e"));
        let binary = internal_ref.find_child_page_id_binary(&Key::new(b"e"));
        assert_eq!(linear.unwrap(), binary, "Mismatch for key 'e'");

        // key "g" → last child (300)
        let linear = internal_ref.find_child_page_id(&Key::new(b"g"));
        let binary = internal_ref.find_child_page_id_binary(&Key::new(b"g"));
        assert_eq!(linear.unwrap(), binary, "Mismatch for key 'g'");

        // key "b" → child 100 (equal to separator b)
        let linear = internal_ref.find_child_page_id(&Key::new(b"b"));
        let binary = internal_ref.find_child_page_id_binary(&Key::new(b"b"));
        assert_eq!(linear.unwrap(), binary, "Mismatch for key 'b'");
    }
}
