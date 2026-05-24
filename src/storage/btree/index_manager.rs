use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::storage::{
    btree::node::{InternalNodeRef, LeafNodeRef, LEAF_NODE},
    page_format::{Key, RowId},
    BufferPool, PageId, Result,
};
use tokio::sync::RwLock;

use super::{AsyncPageLoader, BTree, SyncPageLoader};

/// IndexManager: Async API wrapper for BTree
/// Uses AtomicPageId for lock-free root page access (read operations)
/// Write operations use spawn_blocking + temporary BTree instance
pub struct IndexManager {
    root_page_id: AtomicU64,          // 无锁访问根页
    sync_loader: Arc<SyncPageLoader>, // 写操作仍用 sync
    async_loader: AsyncPageLoader,    // 读操作用 async
    row_to_key: RwLock<HashMap<RowId, Vec<u8>>>,
}

impl IndexManager {
    pub fn new(buffer_pool: Arc<BufferPool>) -> Result<Self> {
        let sync_loader = Arc::new(SyncPageLoader::new(buffer_pool.clone()));
        let async_loader = AsyncPageLoader::new(buffer_pool.clone());

        // 创建 BTree 并获取 root_page_id
        let btree = BTree::new(sync_loader.clone())?;
        let root_page_id = btree.root_page_id().0;

        Ok(Self {
            root_page_id: AtomicU64::new(root_page_id),
            sync_loader,
            async_loader,
            row_to_key: RwLock::new(HashMap::new()),
        })
    }

    /// Async search — direct async path without spawn_blocking
    pub async fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
        let root_page_id = PageId(self.root_page_id.load(Ordering::Acquire));
        let key_obj = Key::new(key);

        self.search_from_page_async(root_page_id, &key_obj).await
    }

    /// Async search_all — find all RowIds matching a key (for non-unique indexes)
    pub async fn search_all(&self, key: &[u8]) -> Result<Vec<RowId>> {
        let root_page_id = PageId(self.root_page_id.load(Ordering::Acquire));
        let key_obj = Key::new(key);

        self.search_all_from_page_async(root_page_id, &key_obj).await
    }

    /// Recursive async search from a page
    fn search_from_page_async<'a>(
        &'a self,
        page_id: PageId,
        key: &'a Key,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Option<RowId>>> + Send + 'a>> {
        Box::pin(async move {
            let child_page_id = {
                let guard = self.async_loader.load_page(page_id).await?;
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
            }; // guard and data_guard dropped here

            self.search_from_page_async(PageId(child_page_id as u64), key)
                .await
        })
    }

    /// Recursive async search_all from a page
    #[allow(clippy::only_used_in_recursion)]
    fn search_all_from_page_async<'a>(
        &'a self,
        page_id: PageId,
        key: &'a Key,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Vec<RowId>>> + Send + 'a>> {
        Box::pin(async move {
            let child_page_ids = {
                let guard = self.async_loader.load_page(page_id).await?;
                let data_guard = guard.page_data();

                if data_guard[0] == LEAF_NODE {
                    // Leaf: collect all matching RowIds
                    let leaf = LeafNodeRef::new(&data_guard);
                    let matches = leaf.find_all_matches(key);
                    let mut row_ids = Vec::new();
                    for idx in matches {
                        if let Some(rid) = leaf.get_row_id(idx) {
                            row_ids.push(rid);
                        }
                    }
                    return Ok(row_ids);
                } else {
                    // Internal: check if key matches any separator (need to search both subtrees)
                    let internal = InternalNodeRef::new(&data_guard);
                    let count = internal.key_count();

                    let mut child_page_ids = Vec::new();
                    for i in 0..count {
                        if let Some(sep_key) = internal.get_key(i) {
                            if sep_key == *key {
                                // Key matches separator: search both left and right subtrees
                                let left_child = if i == 0 {
                                    internal.leftmost_child()
                                } else {
                                    internal.get_child_page_id(i - 1).unwrap_or(internal.leftmost_child())
                                };
                                let right_child = internal.get_child_page_id(i).unwrap_or(internal.leftmost_child());
                                child_page_ids.push(PageId(left_child as u64));
                                child_page_ids.push(PageId(right_child as u64));
                            }
                        }
                    }

                    // If no separator match, follow normal routing
                    if child_page_ids.is_empty() {
                        let child_page_id = internal.find_child_page_id_binary(key);
                        child_page_ids.push(PageId(child_page_id as u64));
                    }

                    child_page_ids
                }
            }; // guard and data_guard dropped

            // Recursively search all child subtrees
            let mut results = Vec::new();
            for child_page_id in child_page_ids {
                let child_results = self.search_all_from_page_async(child_page_id, key).await?;
                results.extend(child_results);
            }
            Ok(results)
        })
    }

    /// Insert a key-value pair — spawn_blocking with temporary BTree instance
    /// Updates root_page_id atomically if a root split occurred
    pub async fn insert(&self, key: &[u8], row_id: RowId) -> Result<()> {
        let root_page_id = self.root_page_id.load(Ordering::Acquire);
        let sync_loader = self.sync_loader.clone();
        let key_vec = key.to_vec();

        let new_root = tokio::task::spawn_blocking(move || {
            let mut btree = BTree::from_root(PageId(root_page_id), sync_loader);
            btree.insert(&key_vec, row_id)
        })
        .await??;

        // If root split occurred, update the atomic root page id
        if let Some(new_root_id) = new_root {
            self.root_page_id.store(new_root_id.0, Ordering::Release);
        }

        self.row_to_key.write().await.insert(row_id, key.to_vec());
        Ok(())
    }

    /// Delete a key — spawn_blocking with temporary BTree instance
    pub async fn delete(&self, key: &[u8]) -> Result<()> {
        if let Some(row_id) = self.search(key).await? {
            self.row_to_key.write().await.remove(&row_id);
        }

        let root_page_id = self.root_page_id.load(Ordering::Acquire);
        let sync_loader = self.sync_loader.clone();
        let key_vec = key.to_vec();

        let new_root = tokio::task::spawn_blocking(move || {
            let mut btree = BTree::from_root(PageId(root_page_id), sync_loader);
            btree.delete(&key_vec)
        })
        .await??;

        if let Some(new_root_id) = new_root {
            self.root_page_id.store(new_root_id.0, Ordering::Release);
        }

        Ok(())
    }

    /// Async scan all entries — direct async path
    pub async fn scan_all(&self) -> Result<Vec<(Vec<u8>, RowId)>> {
        let root_page_id = PageId(self.root_page_id.load(Ordering::Acquire));
        self.scan_all_async_from_root(root_page_id).await
    }

    async fn scan_all_async_from_root(
        &self,
        root_page_id: PageId,
    ) -> Result<Vec<(Vec<u8>, RowId)>> {
        let mut results = Vec::new();
        let mut page_id = root_page_id;

        while page_id.0 != 0 {
            let guard = self.async_loader.load_page(page_id).await?;
            let data_guard = guard.page_data();
            let leaf = LeafNodeRef::new(&data_guard);

            let count = leaf.key_count();
            let mut entries = Vec::with_capacity(count);
            for i in 0..count {
                if let (Some(key), Some(row_id)) = (leaf.get_key(i), leaf.get_row_id(i)) {
                    entries.push((key.as_bytes().to_vec(), row_id));
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

    /// Update the RowId for an existing key — spawn_blocking with temporary BTree instance
    pub async fn update(&self, key: &[u8], new_row_id: RowId) -> Result<()> {
        let root_page_id = self.root_page_id.load(Ordering::Acquire);
        let sync_loader = self.sync_loader.clone();
        let key_vec = key.to_vec();

        tokio::task::spawn_blocking(move || {
            let btree = BTree::from_root(PageId(root_page_id), sync_loader);
            btree.update(&key_vec, new_row_id)
        })
        .await??;

        self.row_to_key
            .write()
            .await
            .insert(new_row_id, key.to_vec());
        Ok(())
    }

    /// Find key by RowId (M10 reverse mapping)
    pub async fn find_key_by_row_id(&self, row_id: RowId) -> Option<Vec<u8>> {
        self.row_to_key.read().await.get(&row_id).cloned()
    }
}
