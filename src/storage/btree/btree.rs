// BTree 核心逻辑（Task 5 实现）
use std::sync::Arc;

use crate::storage::{
    btree::node::{InternalNodeRef, LeafNode, LeafNodeRef, LEAF_NODE},
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

    /// Insert a key and RowId into the BTree
    /// Returns error if key already exists (DuplicateKey)
    /// Simplified version: does not handle splits
    pub fn insert(&self, key: &[u8], row_id: RowId) -> Result<()> {
        let key_obj = Key::new(key);
        self.insert_into_page(self.root_page_id, &key_obj, &row_id)
    }

    /// Insert into a page (leaf or internal)
    fn insert_into_page(&self, page_id: PageId, key: &Key, row_id: &RowId) -> Result<()> {
        let guard = self.loader.load_page(page_id)?;
        let page = guard.page();

        if page.data[0] == LEAF_NODE {
            // Leaf node: insert directly
            let guard2 = self.loader.load_page(page_id)?;
            guard2.modify_page(|page_mut| {
                let mut leaf = LeafNode::from_page(page_mut)?;
                leaf.insert(key, row_id)?;
                Ok(())
            })
        } else {
            // Internal node: find child and recurse
            // Simplified: not implemented yet
            Err(StorageError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Internal node insertion not implemented yet",
            )))
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
            // Internal node: find child, recurse
            let internal = InternalNodeRef::new(&data_guard);
            let child_page_id = internal.find_child_page_id_binary(key);
            drop(data_guard);
            drop(guard);
            self.search_all_from_page(PageId(child_page_id as u64), key)
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
            // Internal node: find child, recurse
            let internal = InternalNodeRef::new(&data_guard);
            let child_page_id = internal.find_child_page_id_binary(key);
            drop(data_guard);
            drop(guard);
            self.delete_all_from_page(PageId(child_page_id as u64), key)
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
            let target_idx = matches.into_iter().find(|idx| {
                leaf_ref.get_row_id(*idx) == Some(row_id.clone())
            });

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
            } else {
                Err(StorageError::KeyNotFound)
            }
        } else {
            // Internal node: find child, recurse
            let internal = InternalNodeRef::new(&data_guard);
            let child_page_id = internal.find_child_page_id_binary(key);
            drop(data_guard);
            drop(guard);
            self.delete_exact_from_page(PageId(child_page_id as u64), key, row_id)
        }
    }
}
