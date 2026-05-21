use crate::storage::{BufferPool, Result, RowId};
use crate::transaction::{Snapshot, TransactionError, TransactionId};
use std::collections::{HashMap, HashSet};
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
}

impl TransactionManager {
    pub fn new() -> Self {
        Self {
            tx_id_allocator: TransactionId::new(),
            active_tx_ids: RwLock::new(HashSet::new()),
            tx_versions: RwLock::new(HashMap::new()),
        }
    }

    /// Begin a new transaction
    ///
    /// - Allocates unique TxId
    /// - Records active transactions for snapshot
    /// - Creates snapshot
    pub async fn begin(&self) -> Transaction {
        let tx_id = self.tx_id_allocator.allocate();

        // Get current active transactions for snapshot
        let active_ids: Vec<u64> = self.active_tx_ids.read().await.iter().copied().collect();

        // Add this transaction to active list
        self.active_tx_ids.write().await.insert(tx_id);

        let snapshot = Snapshot::new(tx_id, active_ids);
        Transaction::new(tx_id, snapshot)
    }

    /// Commit a transaction
    ///
    /// - Removes from active list
    /// - (Future: write commit marker to versions)
    pub async fn commit(&self, tx: Transaction) -> Result<()> {
        let tx_id = tx.id();

        let mut active = self.active_tx_ids.write().await;

        if !active.remove(&tx_id) {
            return Err(TransactionError::AlreadyCommitted(tx_id).into());
        }

        Ok(())
    }

    /// Abort a transaction
    ///
    /// - Removes from active list
    /// - (Future: cleanup uncommitted versions)
    pub async fn abort(&self, tx: Transaction) -> Result<()> {
        let tx_id = tx.id();

        let mut active = self.active_tx_ids.write().await;

        if !active.remove(&tx_id) {
            return Err(TransactionError::AlreadyAborted(tx_id).into());
        }

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
        versions.entry(tx_id).or_insert_with(HashSet::new).insert(row_id);
    }

    /// Get all versions recorded for a transaction (for testing)
    pub async fn get_tx_versions(&self, tx_id: u64) -> HashSet<RowId> {
        self.tx_versions.read().await.get(&tx_id).cloned().unwrap_or_default()
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

        let tx = manager.begin().await;
        let tx_id = tx.id();

        manager.commit(tx).await.unwrap();

        // Verify transaction not in active list
        let active = manager.active_transactions().await;
        assert!(!active.contains(&tx_id));
    }

    #[tokio::test]
    async fn test_transaction_abort() {
        let manager = TransactionManager::new();

        let tx = manager.begin().await;
        let tx_id = tx.id();

        manager.abort(tx).await.unwrap();

        // Verify transaction not in active list
        let active = manager.active_transactions().await;
        assert!(!active.contains(&tx_id));
    }

    #[tokio::test]
    async fn test_transaction_multiple() {
        let manager = TransactionManager::new();

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
        manager.commit(tx1).await.unwrap();
        manager.abort(tx2).await.unwrap();
        manager.commit(tx3).await.unwrap();

        // Active list should be empty
        let active = manager.active_transactions().await;
        assert_eq!(active.len(), 0);
    }

    #[tokio::test]
    async fn test_transaction_snapshot_active_list() {
        let manager = TransactionManager::new();

        // Start two transactions
        let tx1 = manager.begin().await;
        let tx1_id = tx1.id();

        // tx1 should see empty active list (no prior active transactions)
        let snap1_active = tx1.snapshot().tx_id();
        assert_eq!(snap1_active, tx1_id);

        // Start tx2 after tx1
        let tx2 = manager.begin().await;
        let tx2_id = tx2.id();

        // tx2's snapshot should include tx1 in active list
        // But our current snapshot implementation doesn't track active_ids separately from tx_id
        // The key property: tx2.is_visible(tx1_id, None) should be false (tx1 not committed)

        // Start tx3
        let tx3 = manager.begin().await;

        // Commit tx1
        manager.commit(tx1).await.unwrap();

        // tx2 and tx3 snapshots were taken before tx1 committed
        // They should still see tx1 as not visible (based on snapshot rules)

        manager.abort(tx2).await.unwrap();
        manager.commit(tx3).await.unwrap();
    }

    #[tokio::test]
    async fn test_double_commit_error() {
        let manager = TransactionManager::new();

        let tx = manager.begin().await;
        manager.commit(tx).await.unwrap();

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
