use std::collections::HashMap;
use std::sync::Arc;

use crate::storage::{page_format::RowId, BufferPool, Result};
use tokio::sync::RwLock;

use super::{AsyncPageLoader, BTree, SyncPageLoader};

/// IndexManager: Async API wrapper for BTree
/// Uses std::sync::RwLock<BTree> for concurrent reads with spawn_blocking
/// Read ops use read lock (concurrent), write ops use write lock (exclusive)
pub struct IndexManager {
    btree: Arc<std::sync::RwLock<BTree>>,
    async_loader: AsyncPageLoader,
    row_to_key: RwLock<HashMap<RowId, Vec<u8>>>,
}

impl IndexManager {
    pub fn new(buffer_pool: Arc<BufferPool>) -> Result<Self> {
        let sync_loader = Arc::new(SyncPageLoader::new(buffer_pool.clone()));
        let async_loader = AsyncPageLoader::new(buffer_pool.clone());
        let btree = BTree::new(sync_loader)?;
        Ok(Self {
            btree: Arc::new(std::sync::RwLock::new(btree)),
            async_loader,
            row_to_key: RwLock::new(HashMap::new()),
        })
    }

    /// Search for a key — spawn_blocking with read lock (concurrent reads OK)
    /// Uses binary search via BTree::search, which now calls find_key_position_binary
    pub async fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
        let btree = self.btree.clone();
        let key_vec = key.to_vec();
        tokio::task::spawn_blocking(move || {
            let btree_guard = btree.read().unwrap();
            btree_guard.search(&key_vec)
        })
        .await?
    }

    /// Insert a key-value pair — spawn_blocking with write lock
    pub async fn insert(&self, key: &[u8], row_id: RowId) -> Result<()> {
        let btree = self.btree.clone();
        let key_vec = key.to_vec();
        tokio::task::spawn_blocking(move || {
            btree.write().unwrap().insert(&key_vec, row_id)
        }).await??;

        self.row_to_key.write().await.insert(row_id, key.to_vec());
        Ok(())
    }

    /// Delete a key — spawn_blocking with write lock
    pub async fn delete(&self, key: &[u8]) -> Result<()> {
        if let Some(row_id) = self.search(key).await? {
            self.row_to_key.write().await.remove(&row_id);
        }

        let btree = self.btree.clone();
        let key_vec = key.to_vec();
        tokio::task::spawn_blocking(move || {
            btree.write().unwrap().delete(&key_vec)
        })
        .await?
    }

    /// Scan all entries — spawn_blocking with read lock
    pub async fn scan_all(&self) -> Result<Vec<(Vec<u8>, RowId)>> {
        let btree = self.btree.clone();
        let results = tokio::task::spawn_blocking(move || {
            let btree_guard = btree.read().unwrap();
            btree_guard.scan_all()
        })
        .await??;
        Ok(results
            .into_iter()
            .map(|(k, r)| (k.as_bytes().to_vec(), r))
            .collect())
    }

    /// Update the RowId for an existing key — spawn_blocking with write lock
    pub async fn update(&self, key: &[u8], new_row_id: RowId) -> Result<()> {
        let btree = self.btree.clone();
        let key_vec = key.to_vec();
        tokio::task::spawn_blocking(move || {
            btree.write().unwrap().update(&key_vec, new_row_id)
        }).await??;

        self.row_to_key.write().await.insert(new_row_id, key.to_vec());
        Ok(())
    }

    /// Find key by RowId (M10 reverse mapping)
    pub async fn find_key_by_row_id(&self, row_id: RowId) -> Option<Vec<u8>> {
        self.row_to_key.read().await.get(&row_id).cloned()
    }
}