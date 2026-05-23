use crate::storage::{
    page_format::{Key, RowId, Slot, SlottedPage, SlottedPageHeader, SlottedPageRef, MAX_KEY_LEN},
    Page, PageId, StorageError,
};

// B-Tree 常量
pub const LEAF_NODE: u8 = 0x01;
pub const INTERNAL_NODE: u8 = 0x02;

/// Leaf split 操作的中间数据（由 BTree 层消费）
pub struct LeafSplitData {
    /// 上推到父节点的分割 key（右半部分第一个 entry 的 key）
    pub middle_key: Key,
    /// 右半部分 entries（需写入新页）
    pub right_entries: Vec<(Key, RowId)>,
    /// 原页的旧 next_leaf_page_id（用于维护链表）
    pub old_next_page_id: u32,
    /// 新页的 PageId
    pub new_page_id: PageId,
}

/// Internal split 操作的中间数据（由 BTree 层消费）
pub struct InternalSplitData {
    /// 上推到父节点的中间 key（不保留在任一子节点）
    pub middle_key: Key,
    /// separators[mid] 的 right_child → 新页的 leftmost_child
    pub new_leftmost_child: u32,
    /// 右半部分 separators（mid+1..end）
    pub right_separators: Vec<(Key, u32)>,
    /// 新页的 PageId
    pub new_page_id: PageId,
}

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
            .map_err(|e| StorageError::Io(std::io::Error::other(e)))?;

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
    pub fn insert_simple(&mut self, key: &Key, row_id: &RowId) -> Result<(), StorageError> {
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
            .map_err(|e| StorageError::Io(std::io::Error::other(e)))?;

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
            .map_err(|e| StorageError::Io(std::io::Error::other(e)))?;
        self.slotted.sync_header();

        Ok(())
    }

    /// 删除指定索引的 slot（用于批量删除）
    pub fn delete_slot(&mut self, index: usize) -> Result<(), StorageError> {
        self.slotted
            .delete_slot(index)
            .map_err(|e| StorageError::Io(std::io::Error::other(e)))?;
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
            .map(|(k, r)| if k == *key { (k, *new_row_id) } else { (k, r) })
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
        let mut header = *self.slotted.header();
        header.next_page_id = page_id;
        header.serialize(&mut self.slotted.page.data[..SlottedPageHeader::SIZE]);
        self.slotted.reload_header();
    }

    /// 计算可用空间
    pub fn free_space(&self) -> usize {
        self.slotted.free_space()
    }

    /// 最小 key 数量（用于 merge 判断）
    pub fn min_keys(&self) -> usize {
        48 // 见 spec 中的计算
    }

    /// 分裂叶节点：将后半部分 entries 移出，重建原页只保留前半部分。
    /// 返回 LeafSplitData，由调用方（BTree）负责分配新页、写入 right_entries、维护链表指针。
    pub fn split(&mut self, new_page_id: PageId) -> Result<LeafSplitData, StorageError> {
        // 1. 读取所有 entries
        let entries: Vec<(Key, RowId)> = (0..self.key_count())
            .filter_map(|i| {
                let key = self.get_key(i)?;
                let row_id = self.get_row_id(i)?;
                Some((key, row_id))
            })
            .collect();

        if entries.is_empty() {
            return Err(StorageError::Io(std::io::Error::other(
                "cannot split empty leaf node",
            )));
        }

        // 2. 按 key 排序
        let mut sorted_entries = entries;
        sorted_entries.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));

        // 3. 计算中间分裂点
        let mid = sorted_entries.len() / 2;

        // 4. 记录 middle_key 和右半部分 entries
        let middle_key = sorted_entries[mid].0.clone();
        let right_entries = sorted_entries[mid..].to_vec();

        // 5. 记录原页的旧 next_leaf_page_id
        let old_next_page_id = self.next_leaf_page_id();

        // 6. 重建原页：只保留前半部分 entries
        let page_id = self.slotted.page_id();
        let mut new_page = Page::new(page_id);
        let mut new_leaf = LeafNode::init(&mut new_page);
        for (key, row_id) in &sorted_entries[..mid] {
            new_leaf.insert_simple(key, row_id)?;
        }
        // 复制回原页
        self.slotted
            .page
            .data
            .copy_from_slice(new_page.data.as_ref());
        // 刷新缓存 header（copy_from_slice 修改了原始字节，但 slotted 的 header 字段是缓存的）
        self.slotted.reload_header();

        Ok(LeafSplitData {
            middle_key,
            right_entries,
            old_next_page_id,
            new_page_id,
        })
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

    /// 设置 leftmost_child（存储在 header.next_page_id 中）
    pub fn set_leftmost_child(&mut self, child_page_id: u32) {
        self.slotted.page.data[5..9].copy_from_slice(&child_page_id.to_le_bytes());
        self.slotted.reload_header();
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
    /// Layout: leftmost_child | key_0 → child_0 | key_1 → child_1 | ...
    /// For key < key_0, go to leftmost_child
    /// For key_i <= key < key_{i+1}, go to child_i = get_child_page_id(i)
    /// For key >= key_{n-1}, go to child_{n-1} = get_child_page_id(n-1)
    pub fn find_child_page_id(&self, key: &Key) -> u32 {
        let count = self.key_count();
        let leftmost = self.slotted.header().next_page_id;

        for i in 0..count {
            if let Some(sep_key) = self.get_key(i) {
                if *key < sep_key {
                    // key < separator[i]: go to left subtree of separator[i]
                    if i == 0 {
                        return leftmost;
                    } else {
                        return self.get_child_page_id(i - 1).unwrap_or(leftmost);
                    }
                }
                if *key == sep_key {
                    // key == separator[i]: go to right subtree = child_i
                    return self.get_child_page_id(i).unwrap_or(leftmost);
                }
            }
        }

        // key >= all separators: go to last child
        if count > 0 {
            self.get_child_page_id(count - 1).unwrap_or(leftmost)
        } else {
            leftmost
        }
    }

    /// 插入分隔符（key + right_child_page_id）
    pub fn insert_separator(
        &mut self,
        key: &Key,
        right_child: PageId,
    ) -> Result<usize, StorageError> {
        // 1. 查找插入位置
        let position = self.find_insert_position(key);

        // 2. 检查 PageFull
        let entry_size = MAX_KEY_LEN + 4; // Key + PageId (u32)
        if self.slotted.free_space() < Slot::SIZE + entry_size {
            return Err(StorageError::PageFull);
        }

        // 3. 构造数据
        let mut data = vec![0u8; entry_size];
        key.serialize(&mut data[..MAX_KEY_LEN]);
        data[MAX_KEY_LEN..].copy_from_slice(&(right_child.0 as u32).to_le_bytes());

        // 4. 添加 slot
        let slot_index = self
            .slotted
            .add_slot(&data)
            .map_err(|e| StorageError::Io(std::io::Error::other(e)))?;

        // 5. 调整顺序（如果不是插入到末尾）
        if slot_index != position {
            self.shift_slots_right_internal(position, slot_index)?;
        }

        Ok(position)
    }

    /// 查找插入位置（用于有序插入）
    fn find_insert_position(&self, key: &Key) -> usize {
        for i in 0..self.key_count() {
            if let Some(k) = self.get_key(i) {
                if k >= *key {
                    return i;
                }
            }
        }
        self.key_count()
    }

    /// 向右移动 slots（为插入腾出位置）- InternalNode 版本
    fn shift_slots_right_internal(&mut self, _from: usize, _to: usize) -> Result<(), StorageError> {
        // 简化实现：重建页
        let entries: Vec<(Key, u32)> = (0..self.key_count())
            .filter_map(|i| {
                let key = self.get_key(i)?;
                let child = self.get_child_page_id(i)?;
                Some((key, child))
            })
            .collect();

        let mut sorted = entries;
        sorted.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));

        let page_id = self.slotted.page_id();
        let leftmost = self.slotted.header().next_page_id;
        let mut new_page = Page::new(page_id);
        let mut new_internal = InternalNode::init(&mut new_page);
        // Set leftmost_child: init resets header, so set after init
        new_internal.slotted.page.data[5..9].copy_from_slice(&leftmost.to_le_bytes());
        new_internal.slotted.reload_header();

        for (key, child) in sorted {
            new_internal.insert_separator_simple(&key, PageId(child as u64))?;
        }

        self.slotted
            .page
            .data
            .copy_from_slice(new_page.data.as_ref());
        Ok(())
    }

    /// 简单插入（不检查顺序，用于重建）
    pub fn insert_separator_simple(
        &mut self,
        key: &Key,
        right_child: PageId,
    ) -> Result<(), StorageError> {
        let entry_size = MAX_KEY_LEN + 4;
        if self.slotted.free_space() < Slot::SIZE + entry_size {
            return Err(StorageError::PageFull);
        }

        let mut data = vec![0u8; entry_size];
        key.serialize(&mut data[..MAX_KEY_LEN]);
        data[MAX_KEY_LEN..].copy_from_slice(&(right_child.0 as u32).to_le_bytes());

        self.slotted
            .add_slot(&data)
            .map_err(|e| StorageError::Io(std::io::Error::other(e)))?;

        Ok(())
    }

    /// 分裂内节点：将后半部分 separators 移出，重建原页只保留前半部分。
    /// middle_key 上推到父节点（不保留在任一子节点）。
    /// 返回 InternalSplitData，由调用方（BTree）负责分配新页、写入 right_separators。
    pub fn split(&mut self, new_page_id: PageId) -> Result<InternalSplitData, StorageError> {
        // 1. 读取所有 separators（key + right_child 对）
        let separators: Vec<(Key, u32)> = (0..self.key_count())
            .filter_map(|i| {
                let key = self.get_key(i)?;
                let child = self.get_child_page_id(i)?;
                Some((key, child))
            })
            .collect();

        if separators.is_empty() {
            return Err(StorageError::Io(std::io::Error::other(
                "cannot split empty internal node",
            )));
        }

        // 2. 计算中间分裂点
        let mid = separators.len() / 2;

        // 3. middle_key 上推（不保留在任一子节点）
        let middle_key = separators[mid].0.clone();
        let new_leftmost_child = separators[mid].1; // separators[mid] 的 right_child → 新页的 leftmost_child

        // 4. 收集右半部分 separators（mid+1..end）
        let right_separators = if mid + 1 < separators.len() {
            separators[mid + 1..].to_vec()
        } else {
            vec![]
        };

        // 5. 重建原页：只保留 leftmost_child + separators[0..mid]
        let page_id = self.slotted.page_id();
        let old_leftmost = self.slotted.header().next_page_id; // leftmost_child 存储在 header.next_page_id
        let mut new_page = Page::new(page_id);
        let mut new_internal = InternalNode::init(&mut new_page);
        // 保留原 leftmost_child：init 会重置 header，所以需要在 init 之后设置
        new_internal.slotted.page.data[5..9].copy_from_slice(&old_leftmost.to_le_bytes());
        new_internal.slotted.reload_header();
        for (key, child_id) in &separators[..mid] {
            new_internal.insert_separator_simple(key, PageId(*child_id as u64))?;
        }
        // 复制回原页
        self.slotted
            .page
            .data
            .copy_from_slice(new_page.data.as_ref());
        // 刷新缓存 header（copy_from_slice 修改了原始字节，但 slotted 的 header 字段是缓存的）
        self.slotted.reload_header();

        Ok(InternalSplitData {
            middle_key,
            new_leftmost_child,
            right_separators,
            new_page_id,
        })
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
                    // key < key_i: go to left subtree of separator i
                    if i == 0 {
                        return Some(self.leftmost_child());
                    }
                    // key_{i-1} <= key < key_i: go to child_{i-1}
                    return self.get_child_page_id(i - 1);
                }
                // key == key_i: go to right subtree = child_i
                if *key == current_key {
                    return self.get_child_page_id(i);
                }
            }
        }

        // key >= all separators: go to last child
        if count > 0 {
            self.get_child_page_id(count - 1)
        } else {
            Some(self.leftmost_child())
        }
    }

    /// Binary search for child page id — O(log n) instead of O(n)
    /// Returns the child page id for the subtree that should contain the given key.
    /// In an internal node: leftmost_child | key_0 → child_0 | key_1 → child_1 | ...
    /// where child_i = get_child_page_id(i) is the right child of separator i.
    /// If key < key_0, go to leftmost_child; if key_i <= key < key_{i+1}, go to child_i; else go to last child.
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
                        // key == separator[mid]: go to right subtree (child at mid)
                        return self.get_child_page_id(mid).unwrap_or(self.leftmost_child());
                    }
                }
            } else {
                hi = mid;
            }
        }
        // lo is the insertion position. The subtree for keys in [key_{lo-1}, key_{lo})
        // is child_{lo-1} = get_child_page_id(lo - 1).
        if lo == 0 {
            self.leftmost_child()
        } else {
            // key belongs to the subtree rooted at child_{lo-1}
            self.get_child_page_id(lo - 1)
                .unwrap_or(self.leftmost_child())
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
            leaf.insert(&Key::new(&[*ch]), &RowId::new(*ch as u32, 0))
                .unwrap();
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

    #[test]
    fn test_leaf_node_split() {
        // Fill a LeafNode with entries
        let mut page = Page::new(PageId(0));
        let mut leaf = LeafNode::init(&mut page);

        // Insert entries until we have enough for a meaningful split
        let total = 90;
        for i in 0..total {
            let key = Key::new(format!("key_{:03}", i).as_bytes());
            let row_id = RowId::new(i as u32, 0);
            leaf.insert(&key, &row_id).unwrap();
        }

        assert_eq!(leaf.key_count(), total);

        // Set a non-zero next_leaf_page_id to verify it's preserved
        leaf.set_next_leaf_page_id(42);

        // Execute split
        let new_page_id = PageId(1);
        let split_data = leaf.split(new_page_id).unwrap();

        let mid = total / 2; // 45

        // Verify: original page now has only the first half
        assert_eq!(leaf.key_count(), mid);

        // Verify: first half entries are correct
        for i in 0..mid {
            let key = leaf.get_key(i).unwrap();
            assert_eq!(
                key.as_bytes(),
                format!("key_{:03}", i).as_bytes(),
                "Left entry {} mismatch",
                i
            );
        }

        // Verify: middle_key is the first key of the right half
        assert_eq!(
            split_data.middle_key.as_bytes(),
            format!("key_{:03}", mid).as_bytes()
        );

        // Verify: right_entries has the second half
        assert_eq!(split_data.right_entries.len(), mid);
        for (i, (key, row_id)) in split_data.right_entries.iter().enumerate() {
            assert_eq!(
                key.as_bytes(),
                format!("key_{:03}", mid + i).as_bytes(),
                "Right entry {} mismatch",
                i
            );
            assert_eq!(*row_id, RowId::new((mid + i) as u32, 0));
        }

        // Verify: old_next_page_id preserved the original next pointer
        assert_eq!(split_data.old_next_page_id, 42);

        // Verify: new_page_id is passed through correctly
        assert_eq!(split_data.new_page_id, PageId(1));

        // Verify: after split, the leaf's next_leaf_page_id was reset by page rebuild
        // (init resets header, so next_page_id goes back to 0; BTree layer sets it)
        assert_eq!(leaf.next_leaf_page_id(), 0);
    }

    #[test]
    fn test_leaf_node_split_empty_fails() {
        let mut page = Page::new(PageId(0));
        let mut leaf = LeafNode::init(&mut page);

        let result = leaf.split(PageId(1));
        assert!(result.is_err());
    }

    #[test]
    fn test_leaf_node_split_single_entry() {
        // Edge case: split with just 1 entry (mid = 0)
        let mut page = Page::new(PageId(0));
        let mut leaf = LeafNode::init(&mut page);

        leaf.insert(&Key::new(b"only"), &RowId::new(1, 0)).unwrap();
        assert_eq!(leaf.key_count(), 1);

        let split_data = leaf.split(PageId(2)).unwrap();

        // mid = 1 / 2 = 0, so left half has 0 entries, right half has 1 entry
        assert_eq!(leaf.key_count(), 0);
        assert_eq!(split_data.middle_key.as_bytes(), b"only");
        assert_eq!(split_data.right_entries.len(), 1);
        assert_eq!(split_data.new_page_id, PageId(2));
    }

    #[test]
    fn test_leaf_node_split_odd_count() {
        // Odd number of entries: mid = 5 / 2 = 2
        let mut page = Page::new(PageId(0));
        let mut leaf = LeafNode::init(&mut page);

        for ch in b"abcde" {
            leaf.insert(&Key::new(&[*ch]), &RowId::new(*ch as u32, 0))
                .unwrap();
        }

        let split_data = leaf.split(PageId(10)).unwrap();

        // Left: 2 entries (a, b), Right: 3 entries (c, d, e)
        assert_eq!(leaf.key_count(), 2);
        assert_eq!(split_data.right_entries.len(), 3);
        assert_eq!(split_data.middle_key.as_bytes(), b"c");
        assert_eq!(split_data.new_page_id, PageId(10));
    }

    #[test]
    fn test_internal_node_split() {
        let mut page = Page::new(PageId(0));
        let mut internal = InternalNode::init(&mut page);
        // Set leftmost_child after init (init resets header, so set after)
        internal.slotted.page.data[5..9].copy_from_slice(&100u32.to_le_bytes());
        internal.slotted.reload_header();

        // Insert enough separators for a meaningful split
        let total = 50;
        for i in 0..total {
            let key = Key::new(format!("key_{:03}", i).as_bytes());
            internal
                .insert_separator(&key, PageId(101 + i as u64))
                .unwrap();
        }

        assert_eq!(internal.key_count(), total);

        // Execute split
        let new_page_id = PageId(1);
        let split_data = internal.split(new_page_id).unwrap();

        let mid = total / 2; // 25

        // Verify: original page now has only the first half (25 separators)
        assert_eq!(internal.key_count(), mid);

        // Verify: leftmost_child is preserved
        assert_eq!(internal.slotted.header().next_page_id, 100);

        // Verify: first half separators are correct
        for i in 0..mid {
            let key = internal.get_key(i).unwrap();
            assert_eq!(
                key.as_bytes(),
                format!("key_{:03}", i).as_bytes(),
                "Left separator {} mismatch",
                i
            );
        }

        // Verify: middle_key is separators[25].key (pushed up, not in either child)
        assert_eq!(
            split_data.middle_key.as_bytes(),
            format!("key_{:03}", mid).as_bytes()
        );

        // Verify: new_leftmost_child is separators[25].right_child
        assert_eq!(split_data.new_leftmost_child, 101 + mid as u32);

        // Verify: right_separators has 24 entries (mid+1..50 = 26..50)
        assert_eq!(split_data.right_separators.len(), total - mid - 1);
        for (i, (key, child_id)) in split_data.right_separators.iter().enumerate() {
            let expected_idx = mid + 1 + i;
            assert_eq!(
                key.as_bytes(),
                format!("key_{:03}", expected_idx).as_bytes(),
                "Right separator {} mismatch",
                i
            );
            assert_eq!(*child_id, 101 + expected_idx as u32);
        }

        // Verify: new_page_id is passed through correctly
        assert_eq!(split_data.new_page_id, PageId(1));
    }

    #[test]
    fn test_internal_node_split_empty_fails() {
        let mut page = Page::new(PageId(0));
        let _ = InternalNode::init(&mut page);
        // Re-init to get a mutable reference we can call split on
        let mut page2 = Page::new(PageId(0));
        let mut internal = InternalNode::init(&mut page2);

        let result = internal.split(PageId(1));
        assert!(result.is_err());
    }

    #[test]
    fn test_internal_node_split_single_separator() {
        // Edge case: split with just 1 separator (mid = 0)
        let mut page = Page::new(PageId(0));
        let mut internal = InternalNode::init(&mut page);
        // Set leftmost_child after init
        internal.slotted.page.data[5..9].copy_from_slice(&50u32.to_le_bytes());
        internal.slotted.reload_header();

        internal
            .insert_separator(&Key::new(b"only"), PageId(99))
            .unwrap();
        assert_eq!(internal.key_count(), 1);

        let split_data = internal.split(PageId(2)).unwrap();

        // mid = 1 / 2 = 0, so left half has 0 separators, right half has 0 separators
        // middle_key = "only", new_leftmost_child = 99
        assert_eq!(internal.key_count(), 0);
        assert_eq!(split_data.middle_key.as_bytes(), b"only");
        assert_eq!(split_data.new_leftmost_child, 99);
        assert_eq!(split_data.right_separators.len(), 0);
        assert_eq!(split_data.new_page_id, PageId(2));
    }

    #[test]
    fn test_internal_node_split_odd_count() {
        // Odd number of separators: mid = 5 / 2 = 2
        let mut page = Page::new(PageId(0));
        let mut internal = InternalNode::init(&mut page);
        // Set leftmost_child after init
        internal.slotted.page.data[5..9].copy_from_slice(&10u32.to_le_bytes());
        internal.slotted.reload_header();

        for (ch, child) in [
            (b'a', 100u64),
            (b'b', 200u64),
            (b'c', 300u64),
            (b'd', 400u64),
            (b'e', 500u64),
        ] {
            internal
                .insert_separator(&Key::new(&[ch]), PageId(child))
                .unwrap();
        }

        let split_data = internal.split(PageId(10)).unwrap();

        // mid = 2, so left: 2 separators (a, b), middle_key = "c", right: 2 separators (d, e)
        assert_eq!(internal.key_count(), 2);
        assert_eq!(split_data.middle_key.as_bytes(), b"c");
        assert_eq!(split_data.new_leftmost_child, 300); // separators[2].right_child
        assert_eq!(split_data.right_separators.len(), 2);
        assert_eq!(split_data.right_separators[0].0.as_bytes(), b"d");
        assert_eq!(split_data.right_separators[0].1, 400);
        assert_eq!(split_data.right_separators[1].0.as_bytes(), b"e");
        assert_eq!(split_data.right_separators[1].1, 500);
        assert_eq!(split_data.new_page_id, PageId(10));
    }
}
