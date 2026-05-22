use std::collections::HashMap;
use std::sync::Arc;

use crate::storage::{page_format::RowId, BufferPool, Result};
use tokio::sync::RwLock;

use super::{AsyncPageLoader, BTree, SyncPageLoader};

/// IndexManager: Async API wrapper for BTree
/// Uses RwLock<BTree> for concurrent reads, spawn_blocking for writes
pub struct IndexManager {
    btree: Arc<RwLock<BTree>>,
    async_loader: AsyncPageLoader,
    row_to_key: RwLock<HashMap<RowId, Vec<u8>>>,
}

impl IndexManager {
    /// Create a new IndexManager with the given buffer pool
    /// Must be called within a Tokio runtime context (for SyncPageLoader)
    pub fn new(buffer_pool: Arc<BufferPool>) -> Result<Self> {
        let sync_loader = Arc::new(SyncPageLoader::new(buffer_pool.clone()));
        let async_loader = AsyncPageLoader::new(buffer_pool.clone());
        let btree = BTree::new(sync_loader)?;
        Ok(Self {
            btree: Arc::new(RwLock::new(btree)),
            async_loader,
            row_to_key: RwLock::new(HashMap::new()),
        })
    }

    /// Search for a key in the index — async path (no spawn_blocking)
    /// Uses RwLock read lock + BTree::search_async
    pub async fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
        let btree = self.btree.read().await;
        let key_obj = crate::storage::page_format::Key::new(key);
        btree.search_async(&key_obj, &self.async_loader).await
    }

    /// Insert a key-value pair into the index
    /// Uses spawn_blocking to wrap the synchronous BTree operation
    pub async fn insert(&self, key: &[u8], row_id: RowId) -> Result<()> {
        let btree = self.btree.clone();
        let key_for_btree = key.to_vec();

        tokio::task::spawn_blocking(move || {
            btree.blocking_write().insert(&key_for_btree, row_id)
        })
        .await??;

        // Maintain reverse mapping
        self.row_to_key.write().await.insert(row_id, key.to_vec());
        Ok(())
    }

    /// Delete a key from the index
    /// Returns error if key not found
    pub async fn delete(&self, key: &[u8]) -> Result<()> {
        // Clean reverse mapping first
        if let Some(row_id) = self.search(key).await? {
            self.row_to_key.write().await.remove(&row_id);
        }

        let btree = self.btree.clone();
        let key_for_btree = key.to_vec();

        tokio::task::spawn_blocking(move || {
            btree.blocking_write().delete(&key_for_btree)
        })
        .await?
    }

    /// Scan all entries in the index — async path
    /// Returns all (key, RowId) pairs in key order.
    pub async fn scan_all(&self) -> Result<Vec<(Vec<u8>, RowId)>> {
        let btree = self.btree.read().await;
        let results = btree.scan_all()?;
        Ok(results
            .into_iter()
            .map(|(k, r)| (k.as_bytes().to_vec(), r))
            .collect())
    }

    /// Update the RowId for an existing key
    /// Returns error if key not found
    pub async fn update(&self, key: &[u8], new_row_id: RowId) -> Result<()> {
        let btree = self.btree.clone();
        let key_for_btree = key.to_vec();

        tokio::task::spawn_blocking(move || {
            btree.blocking_write().update(&key_for_btree, new_row_id)
        })
        .await??;

        // Maintain reverse mapping
        self.row_to_key.write().await.insert(new_row_id, key.to_vec());
        Ok(())
    }

    /// Find key by RowId (M10 reverse mapping)
    pub async fn find_key_by_row_id(&self, row_id: RowId) -> Option<Vec<u8>> {
        self.row_to_key.read().await.get(&row_id).cloned()
    }
}