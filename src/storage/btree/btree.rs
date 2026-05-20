// BTree 核心逻辑（Task 5 实现）
use std::sync::Arc;

use crate::storage::{
    btree::node::{LeafNode, LEAF_NODE},
    page_format::{Key, RowId},
    PageId, Result, StorageError,
};

use super::SyncPageLoader;

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
        let page = guard.page();

        // Check page type from first byte
        if page.data[0] == LEAF_NODE {
            // Leaf node: search for key in this leaf
            // We need mutable access for LeafNode operations, so we re-load with mutable guard
            let guard2 = self.loader.load_page(page_id)?;
            let result = guard2.modify_page(|page_mut| {
                let leaf = LeafNode::from_page(page_mut)?;
                let pos = leaf.find_key_position(key);

                if pos < leaf.key_count() {
                    if let Some(existing_key) = leaf.get_key(pos) {
                        if existing_key == *key {
                            return Ok(leaf.get_row_id(pos));
                        }
                    }
                }

                Ok(None)
            });

            result
        } else {
            // Internal node: find the child page and recurse
            // Simplified: assume only leaf nodes for now (no internal nodes yet)
            // When we implement splits, we'll handle internal nodes here
            Err(StorageError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Internal node traversal not implemented yet",
            )))
        }
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
