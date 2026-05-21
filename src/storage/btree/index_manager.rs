// IndexManager 异步 API（Task 6 实现）
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::storage::{page_format::RowId, BufferPool, Result};
use tokio::sync::RwLock;

use super::{BTree, SyncPageLoader};

/// IndexManager: Async API wrapper for BTree
/// Holds an Arc<Mutex<BTree>> and uses spawn_blocking for async operations
pub struct IndexManager {
    btree: Arc<Mutex<BTree>>,
    row_to_key: RwLock<HashMap<RowId, Vec<u8>>>,
}

impl IndexManager {
    /// Create a new IndexManager with the given buffer pool
    pub fn new(buffer_pool: Arc<BufferPool>) -> Result<Self> {
        let loader = Arc::new(SyncPageLoader::new(buffer_pool));
        let btree = BTree::new(loader)?;
        Ok(Self {
            btree: Arc::new(Mutex::new(btree)),
            row_to_key: RwLock::new(HashMap::new()),
        })
    }

    /// Insert a key-value pair into the index
    /// Uses spawn_blocking to wrap the synchronous BTree operation
    pub async fn insert(&self, key: &[u8], row_id: RowId) -> Result<()> {
        let btree = self.btree.clone();
        // Key needs to be copied (to_vec()) to satisfy Send trait
        let key = key.to_vec();
        let key_for_btree = key.clone();

        tokio::task::spawn_blocking(move || btree.lock().unwrap().insert(&key_for_btree, row_id))
            .await??;

        // Maintain reverse mapping
        self.row_to_key.write().await.insert(row_id, key);
        Ok(())
    }

    /// Search for a key in the index
    /// Returns Some(RowId) if found, None if not found
    pub async fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
        let btree = self.btree.clone();
        let key = key.to_vec();

        tokio::task::spawn_blocking(move || btree.lock().unwrap().search(&key)).await?
    }

    /// Delete a key from the index
    /// Returns error if key not found
    pub async fn delete(&self, key: &[u8]) -> Result<()> {
        // Clean reverse mapping first
        if let Some(row_id) = self.search(key).await? {
            self.row_to_key.write().await.remove(&row_id);
        }

        let btree = self.btree.clone();
        let key = key.to_vec();

        tokio::task::spawn_blocking(move || btree.lock().unwrap().delete(&key)).await?
    }

    /// Scan all entries in the index.
    /// Returns all (key, RowId) pairs in key order.
    pub async fn scan_all(&self) -> Result<Vec<(Vec<u8>, RowId)>> {
        let btree = self.btree.clone();
        tokio::task::spawn_blocking(move || {
            btree.lock().unwrap().scan_all().map(|v| {
                v.into_iter()
                    .map(|(k, r)| (k.as_bytes().to_vec(), r))
                    .collect()
            })
        })
        .await?
    }

    /// Update the RowId for an existing key
    /// Returns error if key not found
    pub async fn update(&self, key: &[u8], new_row_id: RowId) -> Result<()> {
        let btree = self.btree.clone();
        let key = key.to_vec();
        let key_for_btree = key.clone();

        tokio::task::spawn_blocking(move || {
            btree.lock().unwrap().update(&key_for_btree, new_row_id)
        })
        .await??;

        // Maintain reverse mapping
        self.row_to_key.write().await.insert(new_row_id, key);
        Ok(())
    }

    /// Find key by RowId (M10 reverse mapping)
    pub async fn find_key_by_row_id(&self, row_id: RowId) -> Option<Vec<u8>> {
        self.row_to_key.read().await.get(&row_id).cloned()
    }
}
