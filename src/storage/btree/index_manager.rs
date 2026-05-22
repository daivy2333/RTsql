use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::future::Future;
use std::pin::Pin;

use crate::storage::{
    btree::node::{InternalNodeRef, LeafNodeRef, LEAF_NODE},
    page_format::{Key, RowId},
    PageId, BufferPool, Result,
};
use tokio::sync::RwLock;

use super::{AsyncPageLoader, BTree, SyncPageLoader};

/// IndexManager: Async API wrapper for BTree
/// Uses AtomicPageId for lock-free root page access (read operations)
/// Write operations use spawn_blocking + temporary BTree instance
pub struct IndexManager {
    root_page_id: AtomicU64,              // 无锁访问根页
    sync_loader: Arc<SyncPageLoader>,     // 写操作仍用 sync
    async_loader: AsyncPageLoader,        // 读操作用 async
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

            self.search_from_page_async(PageId(child_page_id as u64), key).await
        })
    }

    /// Insert a key-value pair — spawn_blocking with temporary BTree instance
    pub async fn insert(&self, key: &[u8], row_id: RowId) -> Result<()> {
        let root_page_id = self.root_page_id.load(Ordering::Acquire);
        let sync_loader = self.sync_loader.clone();
        let key_vec = key.to_vec();

        tokio::task::spawn_blocking(move || {
            let btree = BTree::from_root(PageId(root_page_id), sync_loader);
            btree.insert(&key_vec, row_id)
        }).await??;

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

        tokio::task::spawn_blocking(move || {
            let btree = BTree::from_root(PageId(root_page_id), sync_loader);
            btree.delete(&key_vec)
        }).await?
    }

    /// Async scan all entries — direct async path
    pub async fn scan_all(&self) -> Result<Vec<(Vec<u8>, RowId)>> {
        let root_page_id = PageId(self.root_page_id.load(Ordering::Acquire));
        self.scan_all_async_from_root(root_page_id).await
    }

    async fn scan_all_async_from_root(&self, root_page_id: PageId) -> Result<Vec<(Vec<u8>, RowId)>> {
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
        }).await??;

        self.row_to_key.write().await.insert(new_row_id, key.to_vec());
        Ok(())
    }

    /// Find key by RowId (M10 reverse mapping)
    pub async fn find_key_by_row_id(&self, row_id: RowId) -> Option<Vec<u8>> {
        self.row_to_key.read().await.get(&row_id).cloned()
    }
}