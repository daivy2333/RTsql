// BTree 核心逻辑（Task 5 实现）
use std::sync::Arc;

use crate::storage::{
    btree::node::{InternalNodeRef, LeafNode, LeafNodeRef, LEAF_NODE},
    page_format::{Key, RowId},
    PageId, Result, StorageError,
};

use super::{AsyncPageLoader, SyncPageLoader};

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

        while page_id.0 != 0 {
            let guard = self.loader.load_page(page_id)?;
            let data_guard = guard.page_data();
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
}
