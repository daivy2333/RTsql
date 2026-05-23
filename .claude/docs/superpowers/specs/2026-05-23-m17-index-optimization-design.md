# M17 索引优化设计规格

> 创建日期：2026-05-23
> 状态：待审查

## 概述

M17 实现两个核心功能：
1. **非唯一索引**：允许同一 key 对应多个 RowId
2. **B-Tree Split 机制**：页满时自动分裂，解除索引容量限制

**范围说明**：Merge 机制（删除后 underflow 处理）延后到后续里程碑。

---

## 1. 非唯一索引（同页多条目方案）

### 1.1 核心改动

**LeafNode::insert 逻辑调整**：
```rust
// 原逻辑：检查 DuplicateKey 并拒绝
if existing_key == *key {
    return Err(StorageError::DuplicateKey);
}

// 新逻辑：允许重复 key，直接插入
// 去掉 DuplicateKey 检查，相同 key 的 entries 在同页多个 slot
```

**原因**：
- 利用现有 SlottedPage 结构，无需新增溢出页类型
- 零拷贝读路径不变（LeafNodeRef）
- 实现简洁，符合项目轻量理念

### 1.2 新增接口

**LeafNodeRef**：
```rust
/// 查找所有匹配 key 的 slot 索引
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
```

**BTree**：
```rust
/// 返回所有匹配 key 的 RowId
pub fn search_all(&self, key: &[u8]) -> Result<Vec<RowId>>;

/// 删除所有匹配 key 的 entries，返回删除数量
pub fn delete_by_key(&self, key: &[u8]) -> Result<usize>;

/// 精确删除（key + RowId 匹配）
pub fn delete_exact(&self, key: &[u8], row_id: RowId) -> Result<()>;
```

### 1.3 删除逻辑

**delete_by_key**：
- 调用 `LeafNodeRef::find_all_matches` 获取所有匹配 slot
- 逐个调用 `LeafNode::delete_slot`
- 返回删除数量

**delete_exact**：
- 查找所有匹配 key 的 slot
- 检查每个 slot 的 RowId 是否匹配
- 只删除精确匹配的那一个

---

## 2. Split 机制

### 2.1 SplitResult 结构

```rust
pub struct SplitResult {
    /// 上推到父节点的分割 key
    middle_key: Key,
    /// 新分裂出的右页 PageId
    new_page_id: PageId,
}
```

### 2.2 叶子节点 Split

**触发条件**：`insert` 返回 `StorageError::PageFull`

**分裂策略**：中间点分裂（50/50）

**流程**：
```
1. 取 key_count / 2 作为 split_point
2. 分配新页 new_leaf_page_id
3. entries[split_point..key_count] 写入新页
4. entries[0..split_point] 留在原页（清理原页尾部）
5. 设置原页.next_leaf_page_id = new_leaf_page_id（叶子链表）
6. 返回 SplitResult {
     middle_key: entries[split_point].key.clone(),
     new_page_id: new_leaf_page_id,
   }
```

**新 key 的处理**：
- 如果 `new_key < middle_key` → 插入左页（原页）
- 如果 `new_key >= middle_key` → 插入右页（新页）

### 2.3 内部节点 Split

**触发条件**：`insert_separator` 返回 `StorageError::PageFull`

**分裂策略**：中间点分裂，middle key 上推

**流程**：
```
1. 取 key_count / 2 作为 split_point
2. middle_key = separators[split_point].key.clone()
3. 分配新页 new_internal_page_id
4. separators[split_point+1..] + 对应 children 写入新页
5. separators[0..split_point] + 对应 children 留在原页
6. 返回 SplitResult {
     middle_key,  // 上推到父节点
     new_page_id: new_internal_page_id,
   }
```

**child 页的分配**：
- 左页保留 children `[0..split_point+1]`
- 右页保留 children `[split_point+1..key_count+1]`
- leftmost_child 保留在左页

### 2.4 根节点 Split

**触发条件**：根节点 split 返回 SplitResult，且无父节点

**流程**：
```
1. 分配新页作为新根（InternalNode）
2. 设置新根.leftmost_child = old_root_page_id
3. 在新根插入 separator(middle_key) → SplitResult.new_page_id
4. 更新 BTree.root_page_id = new_root_page_id
5. 需要通知 IndexManager 更新引用
```

---

## 3. InternalNode 写入实现

### 3.1 新增方法

```rust
impl<'a> InternalNode<'a> {
    /// 插入分隔符（key + right_child_page_id）
    pub fn insert_separator(
        &mut self,
        key: &Key,
        right_child: PageId,
    ) -> Result<usize, StorageError> {
        // 1. 查找插入位置
        let position = self.find_insert_position(key);
        
        // 2. 检查 PageFull
        let entry_size = MAX_KEY_LEN + 4;
        if self.slotted.free_space() < Slot::SIZE + entry_size {
            return Err(StorageError::PageFull);
        }
        
        // 3. 写入 slot
        let mut data = vec![0u8; entry_size];
        key.serialize(&mut data[..MAX_KEY_LEN]);
        data[MAX_KEY_LEN..].copy_from_slice(&right_child.to_le_bytes());
        
        let slot_index = self.slotted.add_slot(&data)?;
        
        // 4. 调整顺序（复用 shift_slots_right 模式）
        if slot_index != position {
            self.shift_slots_right(position, slot_index)?;
        }
        
        Ok(position)
    }
    
    /// 删除分隔符
    pub fn delete_separator(&mut self, index: usize) -> Result<(), StorageError> {
        self.slotted.delete_slot(index)?;
        self.slotted.sync_header();
        Ok(())
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
}
```

### 3.2 child 页查找逻辑

保持现有 `find_child_page_id_binary` 不变，只需确保写入时正确维护：
- leftmost_child（存储在 header.next_page_id）
- separator → right_child 映射

---

## 4. BTree 改造

### 4.1 递归 Insert 模式

**核心改动**：从单页直接 insert 改为递归处理 split

```rust
pub fn insert(&self, key: &[u8], row_id: RowId) -> Result<()> {
    let key_obj = Key::new(key);
    
    // 递归插入，处理可能的 split
    let split_result = self.insert_recursive(self.root_page_id, &key_obj, &row_id)?;
    
    // 如果根分裂，创建新根
    if let Some(split) = split_result {
        self.create_new_root(split)?;
    }
    
    Ok(())
}

fn insert_recursive(
    &self,
    page_id: PageId,
    key: &Key,
    row_id: &RowId,
) -> Result<Option<SplitResult>, StorageError> {
    let guard = self.loader.load_page(page_id)?;
    let data_guard = guard.page_data();
    
    if data_guard[0] == LEAF_NODE {
        // 叶子节点：尝试插入
        drop(data_guard);
        drop(guard);
        
        let guard2 = self.loader.load_page(page_id)?;
        let result = guard2.modify_page(|page_mut| {
            let mut leaf = LeafNode::from_page(page_mut)?;
            leaf.insert(key, row_id)  // 可能返回 PageFull
        });
        
        match result {
            Ok(_) => Ok(None),  // 成功，无需 split
            Err(StorageError::PageFull) => {
                // 触发 split
                self.split_leaf(page_id, key, row_id)
            }
            Err(e) => Err(e),
        }
    } else {
        // 内部节点：找到 child，递归插入
        let internal = InternalNodeRef::new(&data_guard);
        let child_page_id = internal.find_child_page_id_binary(key);
        drop(data_guard);
        drop(guard);
        
        // 递归插入到 child
        let child_split = self.insert_recursive(PageId(child_page_id as u64), key, row_id)?;
        
        // 处理 child 的 split
        if let Some(split) = child_split {
            self.handle_child_split(page_id, split)?
        }
        
        Ok(None)
    }
}
```

### 4.2 Split 处理方法

**split_leaf**：
```rust
fn split_leaf(
    &self,
    page_id: PageId,
    new_key: &Key,
    new_row_id: &RowId,
) -> Result<Option<SplitResult>, StorageError> {
    // 1. 读取原页 entries
    // 2. 计算分裂点
    // 3. 分配新页，写入右半
    // 4. 原页保留左半
    // 5. 根据新 key 大小决定插入位置
    // 6. 返回 SplitResult
}
```

**handle_child_split**：
```rust
fn handle_child_split(
    &self,
    parent_page_id: PageId,
    child_split: SplitResult,
) -> Result<Option<SplitResult>, StorageError> {
    // 在父节点插入 separator(middle_key → new_page_id)
    // 如果父节点满 → 触发 internal split → 返回新的 SplitResult
}
```

**create_new_root**：
```rust
fn create_new_root(&self, split: SplitResult) -> Result<()> {
    // 1. 分配新页（InternalNode）
    // 2. 设置 leftmost_child = old_root
    // 3. 插入 separator(split.middle_key → split.new_page_id)
    // 4. 更新 self.root_page_id
    // 5. 通知 IndexManager 更新引用（如果需要）
}
```

### 4.3 root_page_id 更新问题

**当前架构限制**：
- `BTree` 持有 `root_page_id: PageId`
- `IndexManager` 和 `TableMeta` 也引用 root_page_id

**解决方案**：
- 方案 A（推荐）：`BTree::insert` 返回 `Option<PageId>`（新根 ID），调用方负责更新
- 方案 B：`BTree` 内部持有 `root_page_id` 的 Arc<Mutex>，允许更新

**选择方案 A**：
- 与现有模式兼容（`IndexManager::insert_index_entry` 已有协调逻辑）
- 职责清晰：BTree 负责 split，IndexManager 负责引用更新

---

## 5. 测试计划

### 5.1 测试场景

| 场景 | 测试目标 | 验证方法 |
|------|----------|----------|
| **容量测试** | 插入 > 400 条，触发多次 split | scan_all 返回正确数量，结构验证 |
| **非唯一索引** | 同一 key 插入多次 | search_all 返回所有 RowId |
| **精确删除** | delete_exact 按 key + RowId | search_all 返回剩余匹配 |
| **批量删除** | delete_by_key 删除所有 | search_all 返回空 |
| **叶子分裂** | 单页分裂 | 验证两个叶子页的 entries 分布 |
| **内部分裂** | 3 层树结构 | 验证内部节点 separators |
| **根分裂** | 触发根分裂 | 验证 root_page_id 更新，新根结构 |

### 5.2 测试代码结构

```rust
#[cfg(test)]
mod split_tests {
    use super::*;
    
    #[test]
    fn test_leaf_split_basic() {
        // 插入 > 400 条，触发分裂
    }
    
    #[test]
    fn test_non_unique_insert() {
        // 同 key 多次插入
    }
    
    #[test]
    fn test_search_all_matches() {
        // 验证 search_all 返回所有匹配
    }
    
    #[test]
    fn test_delete_exact() {
        // 精确删除验证
    }
    
    #[test]
    fn test_root_split() {
        // 根分裂验证
    }
    
    #[test]
    fn test_three_level_tree() {
        // 3 层树结构验证
    }
}
```

---

## 6. 文件改动清单

| 文件 | 改动类型 | 内容 |
|------|----------|------|
| `src/storage/btree/node.rs` | 修改 | LeafNode 去掉 DuplicateKey 检查 |
| `src/storage/btree/node.rs` | 新增 | LeafNodeRef::find_all_matches |
| `src/storage/btree/node.rs` | 新增 | InternalNode::insert_separator/delete_separator |
| `src/storage/btree/btree.rs` | 新增 | SplitResult 结构 |
| `src/storage/btree/btree.rs` | 重构 | insert → insert_recursive + split 处理 |
| `src/storage/btree/btree.rs` | 新增 | split_leaf / handle_child_split / create_new_root |
| `src/storage/btree/btree.rs` | 新增 | search_all / delete_by_key / delete_exact |
| `src/storage/index_manager.rs` | 修改 | 支持根更新回调（可选） |
| `tests/btree_test.rs` | 新增 | split + 非唯一索引测试（7 个场景） |

---

## 7. 实现步骤建议

| Task | 内容 | 依赖 |
|------|------|------|
| T1 | LeafNode 去掉 DuplicateKey 检查 | - |
| T2 | LeafNodeRef::find_all_matches | T1 |
| T3 | BTree::search_all / delete_by_key / delete_exact | T2 |
| T4 | SplitResult 结构定义 | - |
| T5 | InternalNode::insert_separator | T4 |
| T6 | LeafNode split 逻辑（split_leaf） | T4 |
| T7 | 递归 insert + split 传播 | T5, T6 |
| T8 | 根分裂处理（create_new_root） | T7 |
| T9 | 测试：容量 + 非唯一 + split | T1-T8 |

---

## 8. 后续任务（延后）

**Merge 机制 → M19 或独立优化任务**：
- 页面 underflow 检测（`key_count < min_keys()`）
- sibling 页合并或 entries 重新分配
- 根节点收缩（只剩一个 child 时降级为 LeafNode）

**优先级**：
- Split 解除容量硬阻塞 → M17 必需
- Merge 影响空间效率但不阻塞功能 → 可延后

---

## 9. 性能影响评估

| 方面 | 影响 | 说明 |
|------|------|------|
| **读路径** | 无影响 | LeafNodeRef/InternalNodeRef 零拷贝不变 |
| **非唯一查询** | 略慢 | 需遍历多个 slot，但仍在单页内 |
| **插入性能** | split 时有额外开销 | 分配新页 + entries 移动，但频率低 |
| **空间效率** | split 后 50/50 | 标准利用率，可接受 |

---

## 10. 设计决策总结

| 决策 | 选择 | 原因 |
|------|------|------|
| 非唯一索引 | 同页多条目 | 最小改动，利用现有架构 |
| Split 策略 | 中间点分裂 | B-Tree 标准，页面平衡 |
| 根节点分裂 | 创建新根页 | 清晰易懂，格式转换复杂 |
| Merge 优先级 | 延后到后续里程碑 | 不阻塞功能，降低 M17 复杂度 |
| root_page_id 更新 | 返回给调用方 | 职责清晰，与现有模式兼容 |