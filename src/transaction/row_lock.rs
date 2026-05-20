use crate::storage::RowId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// RowLockTable manages row-level write locks for MVCC
///
/// Each row has its own Mutex, allowing concurrent writes to different rows
/// while blocking writes to the same row
pub struct RowLockTable {
    locks: RwLock<HashMap<RowId, Arc<Mutex<()>>>>,
}

impl RowLockTable {
    pub fn new() -> Self {
        Self {
            locks: RwLock::new(HashMap::new()),
        }
    }

    /// Get or create the lock for a row
    ///
    /// Returns the Mutex Arc; caller must call `lock().await` to acquire
    pub async fn get_lock(&self, row_id: RowId) -> Arc<Mutex<()>> {
        // Check if lock already exists
        {
            let locks = self.locks.read().await;
            if let Some(lock) = locks.get(&row_id) {
                return lock.clone();
            }
        }

        // Create new lock entry
        let mut locks = self.locks.write().await;

        // Double check after acquiring write lock
        if let Some(lock) = locks.get(&row_id) {
            return lock.clone();
        }

        let lock = Arc::new(Mutex::new(()));
        locks.insert(row_id, lock.clone());
        lock
    }
}

impl Default for RowLockTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_row_lock_acquire_release() {
        let lock_table = RowLockTable::new();
        let row_id = RowId::new(1, 2);

        let lock = lock_table.get_lock(row_id).await;
        let _guard = lock.lock().await;

        // Lock held, can verify it blocks
        // Lock released when _guard drops
    }

    #[tokio::test]
    async fn test_row_lock_concurrent_same_row() {
        let lock_table = Arc::new(RowLockTable::new());
        let row_id = RowId::new(1, 2);

        // First task acquires lock
        let lock_table_clone1 = lock_table.clone();
        let task1 = tokio::spawn(async move {
            let lock = lock_table_clone1.get_lock(row_id).await;
            let _guard = lock.lock().await;

            // Hold lock for 50ms
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

            // Lock released when _guard drops
        });

        // Wait a bit to ensure task1 has acquired the lock
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Second task should wait for lock
        let lock_table_clone2 = lock_table.clone();
        let task2 = tokio::spawn(async move {
            let start = std::time::Instant::now();
            let lock = lock_table_clone2.get_lock(row_id).await;
            let _guard = lock.lock().await;

            // Should have waited at least 40ms (task1 holds for 50ms, we waited 10ms)
            let elapsed = start.elapsed();
            assert!(elapsed >= tokio::time::Duration::from_millis(40));
        });

        task1.await.unwrap();
        task2.await.unwrap();
    }

    #[tokio::test]
    async fn test_row_lock_different_rows() {
        let lock_table = Arc::new(RowLockTable::new());

        let row_id1 = RowId::new(1, 1);
        let row_id2 = RowId::new(1, 2);

        // Two tasks can acquire locks on different rows simultaneously
        let lock_table_clone1 = lock_table.clone();
        let lock_table_clone2 = lock_table.clone();

        let task1 = tokio::spawn(async move {
            let lock = lock_table_clone1.get_lock(row_id1).await;
            let _guard = lock.lock().await;
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        });

        let task2 = tokio::spawn(async move {
            let start = std::time::Instant::now();
            let lock = lock_table_clone2.get_lock(row_id2).await;
            let _guard = lock.lock().await;

            // Should acquire immediately (different row)
            let elapsed = start.elapsed();
            assert!(elapsed < tokio::time::Duration::from_millis(20));
        });

        task1.await.unwrap();
        task2.await.unwrap();
    }
}
