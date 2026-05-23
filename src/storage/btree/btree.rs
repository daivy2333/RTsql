// BTree 核心逻辑（Task 5 实现）
use std::sync::Arc;

use crate::storage::{
    btree::node::{InternalNode, InternalNodeRef, LeafNode, LeafNodeRef, LEAF_NODE},
    page_format::{Key, RowId},
    PageId, Result, StorageError,
};

use super::{AsyncPageLoader, SyncPageLoader};

/// Split 操作的结果（用于 split 传播）
pub struct SplitResult {
    /// 上推到父节点的分割 key
    pub middle_key: Key,
    /// 新分裂出的右页 PageId
    pub new_page_id: PageId,
}

pub struct BTree {
    loader: Arc<SyncPageLoader>,
    root_page_id: PageId,
}

impl BTree {
    /// Create a new BTree with an empty LeafNode as root
    pub fn new(loader: Arc<SyncPageLoader>) -> Result<Self> {
        // Allocate a page for the root
        let root_page_id = loader.allocate_page()?;

        // Initialize it as an empty LeafNode
        {
            let guard = loader.load_page(root_page_id)?;
            guard.modify_page(|page| {
                LeafNode::init(page);
            });
        }

        Ok(Self {
            loader,
            root_page_id,
        })
    }

    /// Get the root page ID
    pub fn root_page_id(&self) -> PageId {
        self.root_page_id
    }

    /// Create BTree from existing root page (for write operations)
    pub fn from_root(root_page_id: PageId, loader: Arc<SyncPageLoader>) -> Self {
        Self {
            loader,
            root_page_id,
        }
    }

    /// Search for a key in the BTree
    /// Returns the RowId if found, None if not found
    pub fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
        let key_obj = Key::new(key);
        self.search_from_page(self.root_page_id, &key_obj)
    }

    /// Recursive search from a page
    fn search_from_page(&self, page_id: PageId, key: &Key) -> Result<Option<RowId>> {
        let guard = self.loader.load_page(page_id)?;
        let data_guard = guard.page_data();

        if data_guard[0] == LEAF_NODE {
            let leaf = LeafNodeRef::new(&data_guard);
            let (found, pos) = leaf.find_key_position_binary(key);
            if found {
                Ok(leaf.get_row_id(pos))
            } else {
                Ok(None)
            }
        } else {
            let internal = InternalNodeRef::new(&data_guard);
            let child_page_id = internal.find_child_page_id_binary(key);
            drop(data_guard);
            drop(guard);
            self.search_from_page(PageId(child_page_id as u64), key)
        }
    }

    /// Async search — direct async path without spawn_blocking/block_on
    pub async fn search_async(&self, key: &Key, loader: &AsyncPageLoader) -> Result<Option<RowId>> {
        self.search_from_page_async(self.root_page_id, key, loader).await
    }

    fn search_from_page_async<'a>(
        &'a self,
        page_id: PageId,
        key: &'a Key,
        loader: &'a AsyncPageLoader,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<RowId>>> + Send + 'a>> {
        Box::pin(async move {
            let child_page_id = {
                let guard = loader.load_page(page_id).await?;
                let data_guard = guard.page_data();

                if data_guard[0] == LEAF_NODE {
                    let leaf = LeafNodeRef::new(&data_guard);
                    let (found, pos) = leaf.find_key_position_binary(key);
                    if found {
                        return Ok(leaf.get_row_id(pos));
                    } else {
                        return Ok(None);
                    }
                } else {
                    let internal = InternalNodeRef::new(&data_guard);
                    internal.find_child_page_id_binary(key)
                }
            }; // guard and data_guard dropped here, before recursive await

            self.search_from_page_async(PageId(child_page_id as u64), key, loader).await
        })
    }

    /// Insert a key and RowId into the BTree.
    /// Returns Ok(Some(new_root_page_id)) if a root split occurred (caller should update root),
    /// Returns Ok(None) if insertion succeeded without root split.
    pub fn insert(&mut self, key: &[u8], row_id: RowId) -> Result<Option<PageId>> {
        let key_obj = Key::new(key);

        let split = self.insert_into_page(self.root_page_id, &key_obj, &row_id)?;

        if let Some(split_result) = split {
            // Root split: create a new root InternalNode
            let new_root_page_id = self.loader.allocate_page()?;
            let guard = self.loader.load_page(new_root_page_id)?;
            guard.modify_page(|page| {
                let mut new_root = InternalNode::init(page);
                // Set leftmost_child to the old root page id
                new_root.set_leftmost_child(self.root_page_id.0 as u32);
                // Insert separator for the split
                new_root.insert_separator(&split_result.middle_key, split_result.new_page_id)?;
                Ok::<(), StorageError>(())
            })?;

            self.root_page_id = new_root_page_id;
            Ok(Some(new_root_page_id))
        } else {
            Ok(None)
        }
    }

    /// Recursive insert into a page. Returns Some(SplitResult) if the page split,
    /// None if insertion completed without split.
    fn insert_into_page(
        &self,
        page_id: PageId,
        key: &Key,
        row_id: &RowId,
    ) -> Result<Option<SplitResult>> {
        let guard = self.loader.load_page(page_id)?;

        // Determine node type using zero-copy read
        let is_leaf = {
            let data_guard = guard.page_data();
            data_guard[0] == LEAF_NODE
        };

        if is_leaf {
            // Try direct insert first
            let insert_result: std::result::Result<usize, StorageError> = guard.modify_page(|page| {
                let mut leaf = LeafNode::from_page(page)?;
                leaf.insert(key, row_id)
            });

            match insert_result {
                Ok(_) => Ok(None), // Insert succeeded, no split needed
                Err(StorageError::PageFull) => {
                    // Need to split the leaf node
                    let new_page_id = self.loader.allocate_page()?;

                    // 1. Split the original leaf (rebuilds it with left half)
                    let leaf_split = guard.modify_page(|page| {
                        let mut leaf = LeafNode::from_page(page)?;
                        leaf.split(new_page_id)
                    })?;

                    // 2. Re-insert the new entry into the appropriate half after split
                    let key_in_right = key >= &leaf_split.middle_key;

                    if !key_in_right {
                        // Key belongs to the left (original) half
                        // Use insert (not insert_simple) to maintain sorted order
                        guard.modify_page(|page| {
                            let mut leaf = LeafNode::from_page(page)?;
                            leaf.insert(key, row_id)?;
                            Ok::<(), StorageError>(())
                        })?;
                    }

                    // 3. Initialize the new leaf page with right entries
                    //    (and include the new entry if it belongs to the right half)
                    let new_guard = self.loader.load_page(new_page_id)?;
                    new_guard.modify_page(|page| {
                        let mut new_leaf = LeafNode::init(page);

                        // Collect all entries for the right page (sorted + new entry if applicable)
                        let mut right_with_new = leaf_split.right_entries.clone();
                        if key_in_right {
                            right_with_new.push((key.clone(), row_id.clone()));
                            right_with_new.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));
                        }

                        for (k, r) in &right_with_new {
                            new_leaf.insert_simple(k, r)?;
                        }
                        // Maintain linked list: new page's next = old page's old next
                        new_leaf.set_next_leaf_page_id(leaf_split.old_next_page_id);
                        Ok::<(), StorageError>(())
                    })?;

                    // 4. Update original page's next pointer to the new page
                    guard.modify_page(|page| {
                        let mut leaf = LeafNode::from_page(page)?;
                        leaf.set_next_leaf_page_id(new_page_id.0 as u32);
                        Ok::<(), StorageError>(())
                    })?;

                    Ok(Some(SplitResult {
                        middle_key: leaf_split.middle_key,
                        new_page_id,
                    }))
                }
                Err(e) => Err(e), // Other error, propagate
            }
        } else {
            // Internal node: find the child to recurse into
            let child_page_id_u32 = {
                let data_guard = guard.page_data();
                let internal_ref = InternalNodeRef::new(&data_guard);
                internal_ref.find_child_page_id_binary(key)
            };
            let child_page_id = PageId(child_page_id_u32 as u64);

            // Recursively insert into child
            let child_split = self.insert_into_page(child_page_id, key, row_id)?;

            if let Some(child_split_result) = child_split {
                // Child split: need to insert a separator into this internal node
                let separator_insert_result: std::result::Result<usize, StorageError> = guard.modify_page(|page| {
                    let mut internal = InternalNode::from_page(page)?;
                    internal.insert_separator(&child_split_result.middle_key, child_split_result.new_page_id)
                });

                match separator_insert_result {
                    Ok(_) => Ok(None), // Separator inserted successfully
                    Err(StorageError::PageFull) => {
                        // Internal node is also full, need to split it
                        let new_page_id = self.loader.allocate_page()?;

                        // 1. Split the original internal node
                        let internal_split = guard.modify_page(|page| {
                            let mut internal = InternalNode::from_page(page)?;
                            internal.split(new_page_id)
                        })?;

                        // 2. Re-insert the pending separator into the appropriate half
                        //    The separator that caused PageFull was child_split_result.middle_key.
                        let sep_in_right =
                            child_split_result.middle_key >= internal_split.middle_key;

                        if !sep_in_right {
                            // Separator belongs to the left (original) half
                            guard.modify_page(|page| {
                                let mut internal = InternalNode::from_page(page)?;
                                internal.insert_separator(
                                    &child_split_result.middle_key,
                                    child_split_result.new_page_id,
                                )?;
                                Ok::<(), StorageError>(())
                            })?;
                        }

                        // 3. Initialize the new internal node page
                        //    (and include the pending separator if it belongs to the right half)
                        let new_guard = self.loader.load_page(new_page_id)?;
                        new_guard.modify_page(|page| {
                            let mut new_internal = InternalNode::init(page);
                            // Set leftmost_child
                            new_internal.set_leftmost_child(internal_split.new_leftmost_child);
                            // Insert right separators
                            for (k, child_id) in &internal_split.right_separators {
                                new_internal.insert_separator_simple(k, PageId(*child_id as u64))?;
                            }
                            if sep_in_right {
                                new_internal.insert_separator(
                                    &child_split_result.middle_key,
                                    child_split_result.new_page_id,
                                )?;
                            }
                            Ok::<(), StorageError>(())
                        })?;

                        Ok(Some(SplitResult {
                            middle_key: internal_split.middle_key,
                            new_page_id,
                        }))
                    }
                    Err(e) => Err(e), // Other error, propagate
                }
            } else {
                // Child did not split, insertion complete
                Ok(None)
            }
        }
    }

    /// Delete a key from the BTree
    /// Returns error if key not found (KeyNotFound)
    /// Simplified version: does not handle merges
    pub fn delete(&self, key: &[u8]) -> Result<()> {
        let key_obj = Key::new(key);
        self.delete_from_page(self.root_page_id, &key_obj)
    }

    /// Delete from a page
    fn delete_from_page(&self, page_id: PageId, key: &Key) -> Result<()> {
        let guard = self.loader.load_page(page_id)?;
        let page = guard.page();

        if page.data[0] == LEAF_NODE {
            // Leaf node: delete directly
            let guard2 = self.loader.load_page(page_id)?;
            guard2.modify_page(|page_mut| {
                let mut leaf = LeafNode::from_page(page_mut)?;
                leaf.delete(key)?;
                Ok(())
            })
        } else {
            // Internal node: find child and recurse
            // Simplified: not implemented yet
            Err(StorageError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Internal node deletion not implemented yet",
            )))
        }
    }

    /// Scan all entries in the BTree by iterating through all leaf nodes.
    /// Returns all (Key, RowId) pairs in key order.
    pub fn scan_all(&self) -> Result<Vec<(Key, RowId)>> {
        let mut results = Vec::new();
        let mut page_id = self.root_page_id;
        let mut first_iteration = true;

        loop {
            let guard = self.loader.load_page(page_id)?;
            let data_guard = guard.page_data();

            // Check if it's a leaf node
            if data_guard[0] != LEAF_NODE {
                // Internal node: follow leftmost child
                let internal = InternalNodeRef::new(&data_guard);
                let child_id = internal.leftmost_child();
                drop(data_guard);
                drop(guard);
                page_id = PageId(child_id as u64);
                continue;
            }

            let leaf = LeafNodeRef::new(&data_guard);
            let count = leaf.key_count();
            let mut entries = Vec::with_capacity(count);
            for i in 0..count {
                if let (Some(key), Some(row_id)) = (leaf.get_key(i), leaf.get_row_id(i)) {
                    entries.push((key, row_id));
                }
            }

            let next_page_u32 = leaf.next_leaf_page_id();
            drop(data_guard);
            drop(guard);

            results.extend(entries);

            // Stop after first leaf if no next_leaf_page_id (single leaf tree)
            if next_page_u32 == 0 && !first_iteration {
                break;
            }
            first_iteration = false;

            // Move to next leaf, or stop if next_page_id is invalid (0 after first iteration)
            if next_page_u32 == 0 {
                break;
            }
            page_id = PageId(next_page_u32 as u64);
        }

        Ok(results)
    }

    /// Update the RowId for an existing key
    /// Returns error if key not found (KeyNotFound)
    pub fn update(&self, key: &[u8], new_row_id: RowId) -> Result<()> {
        let key_obj = Key::new(key);
        self.update_in_page(self.root_page_id, &key_obj, &new_row_id)
    }

    /// Update in a page
    fn update_in_page(&self, page_id: PageId, key: &Key, new_row_id: &RowId) -> Result<()> {
        let guard = self.loader.load_page(page_id)?;
        let page = guard.page();

        if page.data[0] == LEAF_NODE {
            // Leaf node: find and update
            let guard2 = self.loader.load_page(page_id)?;
            guard2.modify_page(|page_mut| {
                let mut leaf = LeafNode::from_page(page_mut)?;
                leaf.update(key, new_row_id)?;
                Ok(())
            })
        } else {
            // Internal node: find child and recurse
            // Simplified: not implemented yet
            Err(StorageError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Internal node update not implemented yet",
            )))
        }
    }

    /// 返回所有匹配 key 的 RowId（用于非唯一索引）
    pub fn search_all(&self, key: &[u8]) -> Result<Vec<RowId>> {
        let key_obj = Key::new(key);
        self.search_all_from_page(self.root_page_id, &key_obj)
    }

    fn search_all_from_page(&self, page_id: PageId, key: &Key) -> Result<Vec<RowId>> {
        let guard = self.loader.load_page(page_id)?;
        let data_guard = guard.page_data();

        if data_guard[0] == LEAF_NODE {
            let leaf = LeafNodeRef::new(&data_guard);
            let matches = leaf.find_all_matches(key);
            let mut row_ids = Vec::new();
            for idx in matches {
                if let Some(rid) = leaf.get_row_id(idx) {
                    row_ids.push(rid);
                }
            }

            Ok(row_ids)
        } else {
            // Internal node: for non-unique keys that equal a separator, we need to
            // search both the left and right subtrees.
            let internal = InternalNodeRef::new(&data_guard);
            let count = internal.key_count();

            // Find if the key matches any separator
            let mut separator_match_idx: Option<usize> = None;
            for i in 0..count {
                if let Some(sep_key) = internal.get_key(i) {
                    if sep_key == *key {
                        separator_match_idx = Some(i);
                        break;
                    }
                }
            }

            if let Some(idx) = separator_match_idx {
                // Key matches a separator — search both left subtree (get_child_page_id(idx-1) or leftmost)
                // and right subtree (get_child_page_id(idx))
                let left_child = if idx == 0 {
                    internal.leftmost_child()
                } else {
                    internal.get_child_page_id(idx - 1).unwrap_or(internal.leftmost_child())
                };
                let right_child = internal.get_child_page_id(idx).unwrap_or(internal.leftmost_child());

                drop(data_guard);
                drop(guard);

                let mut results = self.search_all_from_page(PageId(left_child as u64), key)?;
                let right_results = self.search_all_from_page(PageId(right_child as u64), key)?;
                results.extend(right_results);
                Ok(results)
            } else {
                // Key doesn't match any separator — route normally
                let child_page_id = internal.find_child_page_id_binary(key);
                drop(data_guard);
                drop(guard);
                self.search_all_from_page(PageId(child_page_id as u64), key)
            }
        }
    }

    /// 删除所有匹配 key 的 entries，返回删除数量（用于非唯一索引）
    pub fn delete_by_key(&self, key: &[u8]) -> Result<usize> {
        let key_obj = Key::new(key);
        self.delete_all_from_page(self.root_page_id, &key_obj)
    }

    fn delete_all_from_page(&self, page_id: PageId, key: &Key) -> Result<usize> {
        // First, read the page to find matches
        let guard = self.loader.load_page(page_id)?;
        let data_guard = guard.page_data();

        if data_guard[0] == LEAF_NODE {
            let leaf_ref = LeafNodeRef::new(&data_guard);
            let matches = leaf_ref.find_all_matches(key);
            let count = matches.len();
            drop(data_guard);
            drop(guard);

            // Then, modify the page to delete
            if count > 0 {
                let guard2 = self.loader.load_page(page_id)?;
                guard2.modify_page(|page_mut| {
                    let mut leaf = LeafNode::from_page(page_mut)?;
                    // Delete from back to front (avoid index shifting)
                    for idx in matches.into_iter().rev() {
                        leaf.delete_slot(idx)?;
                    }
                    Ok::<(), StorageError>(())
                })?;
            }

            Ok(count)
        } else {
            // Internal node: for non-unique keys that equal a separator, search both subtrees
            let internal = InternalNodeRef::new(&data_guard);
            let count = internal.key_count();

            // Find if the key matches any separator
            let mut separator_match_idx: Option<usize> = None;
            for i in 0..count {
                if let Some(sep_key) = internal.get_key(i) {
                    if sep_key == *key {
                        separator_match_idx = Some(i);
                        break;
                    }
                }
            }

            if let Some(idx) = separator_match_idx {
                let left_child = if idx == 0 {
                    internal.leftmost_child()
                } else {
                    internal.get_child_page_id(idx - 1).unwrap_or(internal.leftmost_child())
                };
                let right_child = internal.get_child_page_id(idx).unwrap_or(internal.leftmost_child());

                drop(data_guard);
                drop(guard);

                let left_count = self.delete_all_from_page(PageId(left_child as u64), key)?;
                let right_count = self.delete_all_from_page(PageId(right_child as u64), key)?;
                Ok(left_count + right_count)
            } else {
                let child_page_id = internal.find_child_page_id_binary(key);
                drop(data_guard);
                drop(guard);
                self.delete_all_from_page(PageId(child_page_id as u64), key)
            }
        }
    }

    /// 精确删除（key + RowId 匹配）
    pub fn delete_exact(&self, key: &[u8], row_id: RowId) -> Result<()> {
        let key_obj = Key::new(key);
        self.delete_exact_from_page(self.root_page_id, &key_obj, &row_id)
    }

    fn delete_exact_from_page(&self, page_id: PageId, key: &Key, row_id: &RowId) -> Result<()> {
        // First, read the page to find exact match
        let guard = self.loader.load_page(page_id)?;
        let data_guard = guard.page_data();

        if data_guard[0] == LEAF_NODE {
            let leaf_ref = LeafNodeRef::new(&data_guard);
            let matches = leaf_ref.find_all_matches(key);

            // Find slot with matching RowId
            let target_idx = matches.iter().find(|idx| {
                leaf_ref.get_row_id(**idx) == Some(row_id.clone())
            }).copied();

            let key_has_matches_in_leaf = !matches.is_empty();
            let next_page_u32 = if key_has_matches_in_leaf {
                leaf_ref.next_leaf_page_id()
            } else {
                0
            };

            drop(data_guard);
            drop(guard);

            // Then, modify the page to delete
            if let Some(idx) = target_idx {
                let guard2 = self.loader.load_page(page_id)?;
                guard2.modify_page(|page_mut| {
                    let mut leaf = LeafNode::from_page(page_mut)?;
                    leaf.delete_slot(idx)?;
                    Ok::<(), StorageError>(())
                })?;
                Ok(())
            } else if key_has_matches_in_leaf && next_page_u32 != 0 {
                // Key was found in this leaf but RowId wasn't; continue to next leaf
                self.delete_exact_from_page(PageId(next_page_u32 as u64), key, row_id)
            } else {
                Err(StorageError::KeyNotFound)
            }
        } else {
            // Internal node: for non-unique keys that equal a separator, search both subtrees
            let internal = InternalNodeRef::new(&data_guard);
            let count = internal.key_count();

            // Find if the key matches any separator
            let mut separator_match_idx: Option<usize> = None;
            for i in 0..count {
                if let Some(sep_key) = internal.get_key(i) {
                    if sep_key == *key {
                        separator_match_idx = Some(i);
                        break;
                    }
                }
            }

            if let Some(idx) = separator_match_idx {
                let left_child = if idx == 0 {
                    internal.leftmost_child()
                } else {
                    internal.get_child_page_id(idx - 1).unwrap_or(internal.leftmost_child())
                };
                let right_child = internal.get_child_page_id(idx).unwrap_or(internal.leftmost_child());

                drop(data_guard);
                drop(guard);

                // Try left subtree first, then right subtree
                match self.delete_exact_from_page(PageId(left_child as u64), key, row_id) {
                    Ok(()) => Ok(()),
                    Err(StorageError::KeyNotFound) => {
                        self.delete_exact_from_page(PageId(right_child as u64), key, row_id)
                    }
                    Err(e) => Err(e),
                }
            } else {
                let child_page_id = internal.find_child_page_id_binary(key);
                drop(data_guard);
                drop(guard);
                self.delete_exact_from_page(PageId(child_page_id as u64), key, row_id)
            }
        }
    }
}
