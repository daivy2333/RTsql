use crate::storage::{BufferPool, PageId, Result, RowId, TableMeta};
use crate::transaction::{Snapshot, TransactionError, TransactionId};
use crate::wal::{WALBuffer, WalRecord};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Transaction state
#[derive(Debug, Clone, PartialEq)]
pub enum TransactionState {
    Active,
    Committed,
    Aborted,
}

/// Transaction represents an active database transaction
pub struct Transaction {
    id: u64,
    snapshot: Snapshot,
    state: TransactionState,
}

impl Transaction {
    pub fn new(id: u64, snapshot: Snapshot) -> Self {
        Self {
            id,
            snapshot,
            state: TransactionState::Active,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    pub fn state(&self) -> TransactionState {
        self.state.clone()
    }
}

/// TransactionManager manages transaction lifecycle
///
/// - begin: allocate TxId, create snapshot, record in active list
/// - commit: remove from active list, mark committed
/// - abort: remove from active list, mark aborted
pub struct TransactionManager {
    tx_id_allocator: TransactionId,
    active_tx_ids: RwLock<HashSet<u64>>,
    // M10: 跟踪每个事务的未提交版本
    tx_versions: RwLock<HashMap<u64, HashSet<RowId>>>,
    // WAL buffer for writing transaction lifecycle records
    wal_buffer: RwLock<Option<Arc<WALBuffer>>>,
}

impl TransactionManager {
    pub fn new() -> Self {
        Self {
            tx_id_allocator: TransactionId::new(),
            active_tx_ids: RwLock::new(HashSet::new()),
            tx_versions: RwLock::new(HashMap::new()),
            wal_buffer: RwLock::new(None),
        }
    }

    pub async fn set_wal_buffer(&self, wal_buffer: Arc<WALBuffer>) {
        *self.wal_buffer.write().await = Some(wal_buffer);
    }

    /// Begin a new transaction
    ///
    /// - Allocates unique TxId
    /// - Records active transactions for snapshot
    /// - Creates snapshot
    pub async fn begin(&self) -> Transaction {
        let tx_id = self.tx_id_allocator.allocate();

        // WAL: write BeginTxn record
        if let Some(wal) = self.wal_buffer.read().await.as_ref() {
            wal.append(WalRecord::BeginTxn { tx_id }).await;
        }

        // Get current active transactions for snapshot
        let active_ids: Vec<u64> = self.active_tx_ids.read().await.iter().copied().collect();

        // Add this transaction to active list
        self.active_tx_ids.write().await.insert(tx_id);

        let snapshot = Snapshot::new(tx_id, active_ids);
        Transaction::new(tx_id, snapshot)
    }

    /// Commit a transaction
    ///
    /// - Marks all versions as committed
    /// - Removes from active list
    /// - Clears tx_versions
    pub async fn commit(&self, tx: Transaction, buffer_pool: &BufferPool) -> Result<()> {
        let tx_id = tx.id();

        // WAL: write CommitTxn record and wait for persistence (Group Commit)
        if let Some(wal) = self.wal_buffer.read().await.as_ref() {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            wal.append(WalRecord::CommitTxn { tx_id, timestamp }).await;
            wal.append_commit_and_wait(tx_id).await?;
        }

        // M10: Mark all versions as committed
        self.commit_mark_versions(tx_id, buffer_pool).await?;

        // Remove from active list
        let mut active = self.active_tx_ids.write().await;
        if !active.remove(&tx_id) {
            return Err(TransactionError::AlreadyCommitted(tx_id).into());
        }

        // Clear tx_versions
        self.tx_versions.write().await.remove(&tx_id);

        Ok(())
    }

    /// Abort a transaction
    ///
    /// - Cleans up uncommitted versions (M10)
    /// - Removes from active list
    /// - Clears tx_versions
    pub async fn abort(
        &self,
        tx: Transaction,
        buffer_pool: &BufferPool,
        table_meta: &TableMeta,
    ) -> Result<()> {
        let tx_id = tx.id();

        // WAL: write AbortTxn record (no need to wait for persistence)
        if let Some(wal) = self.wal_buffer.read().await.as_ref() {
            wal.append(WalRecord::AbortTxn { tx_id }).await;
        }

        // M10: Cleanup uncommitted versions
        self.abort_cleanup_versions(tx_id, buffer_pool, table_meta)
            .await?;

        // Remove from active list
        let mut active = self.active_tx_ids.write().await;
        if !active.remove(&tx_id) {
            return Err(TransactionError::AlreadyAborted(tx_id).into());
        }

        // Clear tx_versions
        self.tx_versions.write().await.remove(&tx_id);

        Ok(())
    }

    /// Get current active transactions
    pub async fn active_transactions(&self) -> Vec<u64> {
        self.active_tx_ids.read().await.iter().copied().collect()
    }

    /// Record a version created by this transaction (M10)
    ///
    /// Called by InsertExecutor/UpdateExecutor when creating new versions
    pub async fn record_version(&self, tx_id: u64, row_id: RowId) {
        let mut versions = self.tx_versions.write().await;
        versions
            .entry(tx_id)
            .or_insert_with(HashSet::new)
            .insert(row_id);
    }

    /// Get all versions recorded for a transaction (for testing)
    pub async fn get_tx_versions(&self, tx_id: u64) -> HashSet<RowId> {
        self.tx_versions
            .read()
            .await
            .get(&tx_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Get tx_versions (for testing)
    pub async fn tx_versions(&self) -> HashMap<u64, HashSet<RowId>> {
        self.tx_versions.read().await.clone()
    }

    /// Get current max TxId (for testing)
    pub fn current_tx_id(&self) -> u64 {
        self.tx_id_allocator.current()
    }

    /// Commit by ID (for testing error cases)
    pub async fn commit_by_id(&self, tx_id: u64) -> Result<()> {
        let mut active = self.active_tx_ids.write().await;

        if !active.remove(&tx_id) {
            return Err(TransactionError::NotFound(tx_id).into());
        }

        Ok(())
    }

    /// Mark all versions as committed (M10)
    pub async fn commit_mark_versions(&self, tx_id: u64, buffer_pool: &BufferPool) -> Result<()> {
        let versions = self.tx_versions.read().await;
        let tx_versions = versions.get(&tx_id).cloned().unwrap_or_default();

        for row_id in tx_versions {
            buffer_pool.write_commit_tx_id(row_id, tx_id).await?;
            // M21: Clear page visibility summary after COMMIT mark
            let page_id = PageId(row_id.page_id as u64);
            buffer_pool.clear_all_visible(page_id);
        }

        Ok(())
    }

    /// Cleanup uncommitted versions on abort (M10)
    ///
    /// For each uncommitted version created by this transaction:
    /// - If it has a previous version, update index to point to previous
    /// - If it has no previous version, delete from index
    pub async fn abort_cleanup_versions(
        &self,
        tx_id: u64,
        buffer_pool: &BufferPool,
        table_meta: &TableMeta,
    ) -> Result<()> {
        let versions = self.tx_versions.read().await;
        let tx_versions = versions.get(&tx_id).cloned().unwrap_or_default();

        for row_id in tx_versions {
            let header = buffer_pool.read_version_header(row_id).await?;

            let key = table_meta.index_manager.find_key_by_row_id(row_id).await;

            if let Some(key) = key {
                if let Some(prev_row_id) = header.next_version() {
                    table_meta.index_manager.update(&key, prev_row_id).await?;
                } else {
                    table_meta.index_manager.delete(&key).await?;
                }
            }
        }

        Ok(())
    }
}

impl Default for TransactionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{BufferPool, FileStorage, TableManager};
    use std::sync::Arc;
    use tempfile::tempdir;

    /// Create a test buffer pool for tests that need it
    fn create_test_buffer_pool() -> Arc<BufferPool> {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
        Arc::new(BufferPool::new(10, storage).unwrap())
    }

    /// Create a test table for abort tests
    async fn create_test_table(buffer_pool: Arc<BufferPool>) -> Arc<TableMeta> {
        let table_manager = TableManager::new(buffer_pool.clone());
        table_manager
            .create_table(
                "test_table",
                vec![("id".to_string(), crate::storage::ColumnType::Int)],
                "id",
            )
            .await
            .unwrap();
        table_manager.get_table("test_table").await.unwrap()
    }

    #[tokio::test]
    async fn test_transaction_begin() {
        let manager = TransactionManager::new();

        let tx = manager.begin().await;

        assert!(tx.id() > 0);
        assert_eq!(tx.state(), TransactionState::Active);
        assert!(tx.snapshot().tx_id() > 0);
    }

    #[tokio::test]
    async fn test_transaction_commit() {
        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();

        let tx = manager.begin().await;
        let tx_id = tx.id();

        manager.commit(tx, &buffer_pool).await.unwrap();

        // Verify transaction not in active list
        let active = manager.active_transactions().await;
        assert!(!active.contains(&tx_id));
    }

    #[tokio::test]
    async fn test_transaction_abort() {
        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();
        let table_meta = create_test_table(buffer_pool.clone()).await;

        let tx = manager.begin().await;
        let tx_id = tx.id();

        manager.abort(tx, &buffer_pool, &table_meta).await.unwrap();

        // Verify transaction not in active list
        let active = manager.active_transactions().await;
        assert!(!active.contains(&tx_id));
    }

    #[tokio::test]
    async fn test_transaction_multiple() {
        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();
        let table_meta = create_test_table(buffer_pool.clone()).await;

        let tx1 = manager.begin().await;
        let tx2 = manager.begin().await;
        let tx3 = manager.begin().await;

        // IDs should be unique and increasing
        assert!(tx1.id() < tx2.id());
        assert!(tx2.id() < tx3.id());

        // All three should be in active list
        let active = manager.active_transactions().await;
        assert!(active.contains(&tx1.id()));
        assert!(active.contains(&tx2.id()));
        assert!(active.contains(&tx3.id()));
        assert_eq!(active.len(), 3);

        // Commit tx1 and tx3, abort tx2
        manager.commit(tx1, &buffer_pool).await.unwrap();
        manager.abort(tx2, &buffer_pool, &table_meta).await.unwrap();
        manager.commit(tx3, &buffer_pool).await.unwrap();

        // Active list should be empty
        let active = manager.active_transactions().await;
        assert_eq!(active.len(), 0);
    }

    #[tokio::test]
    async fn test_transaction_snapshot_active_list() {
        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();
        let table_meta = create_test_table(buffer_pool.clone()).await;

        // Start two transactions
        let tx1 = manager.begin().await;
        let tx1_id = tx1.id();

        // tx1 should see empty active list (no prior active transactions)
        let snap1_active = tx1.snapshot().tx_id();
        assert_eq!(snap1_active, tx1_id);

        // Start tx2 after tx1
        let tx2 = manager.begin().await;
        let _tx2_id = tx2.id();

        // tx2's snapshot should include tx1 in active list
        // But our current snapshot implementation doesn't track active_ids separately from tx_id
        // The key property: tx2.is_visible(tx1_id, None) should be false (tx1 not committed)

        // Start tx3
        let tx3 = manager.begin().await;

        // Commit tx1
        manager.commit(tx1, &buffer_pool).await.unwrap();

        // tx2 and tx3 snapshots were taken before tx1 committed
        // They should still see tx1 as not visible (based on snapshot rules)

        manager.abort(tx2, &buffer_pool, &table_meta).await.unwrap();
        manager.commit(tx3, &buffer_pool).await.unwrap();
    }

    #[tokio::test]
    async fn test_double_commit_error() {
        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();

        let tx = manager.begin().await;
        manager.commit(tx, &buffer_pool).await.unwrap();

        // Second commit on same tx_id should fail
        // But we can't reuse the same tx object after commit (it was moved)
        // Test with a non-existent tx_id
        let result = manager.commit_by_id(999).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tx_versions_initialization() {
        let manager = TransactionManager::new();
        // tx_versions should be empty initially
        assert!(manager.tx_versions().await.is_empty());
    }

    #[tokio::test]
    async fn test_record_version_single() {
        let manager = TransactionManager::new();
        let row_id = RowId::new(1, 0);
        manager.record_version(1, row_id).await;
        let versions = manager.get_tx_versions(1).await;
        assert!(versions.contains(&row_id));
        assert_eq!(versions.len(), 1);
    }

    #[tokio::test]
    async fn test_record_version_multiple() {
        let manager = TransactionManager::new();
        let row_id1 = RowId::new(1, 0);
        let row_id2 = RowId::new(2, 0);
        manager.record_version(1, row_id1).await;
        manager.record_version(1, row_id2).await;
        let versions = manager.get_tx_versions(1).await;
        assert!(versions.contains(&row_id1));
        assert!(versions.contains(&row_id2));
        assert_eq!(versions.len(), 2);
    }

    #[tokio::test]
    async fn test_get_tx_versions_empty() {
        let manager = TransactionManager::new();
        let versions = manager.get_tx_versions(999).await;
        assert!(versions.is_empty());
    }
}
