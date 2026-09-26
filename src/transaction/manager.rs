use crate::storage::{
    read_tuple_from_data_page, update_version_header_in_data_page, BufferPool, PageId, Result,
    RowId, StorageError, TableMeta,
};
use crate::transaction::{Snapshot, TransactionError, TransactionId};
use crate::wal::{WALBuffer, WalRecord};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 墓碑行索引还原时的版本链回溯上限（MS24 Iter001 replan 2.9/D9）——仅防御
/// 损坏库的无环性；正常链长受版本链深度限制，远小于该值。
const MAX_TOMBSTONE_CHAIN_WALK: usize = 64;

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
    // M10: 跟踪每个事务的未提交版本，按表聚合以支持多表事务回滚（MS07-T04）
    tx_versions: RwLock<HashMap<u64, HashMap<String, HashSet<RowId>>>>,
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
    /// - Cleans up uncommitted versions (M10), rolling back each table's
    ///   index entries via the resolved `TableMeta` map (MS07-T04)
    /// - Removes from active list
    /// - Clears tx_versions
    pub async fn abort(
        &self,
        tx: Transaction,
        buffer_pool: &BufferPool,
        tables: &HashMap<String, Arc<TableMeta>>,
    ) -> Result<()> {
        let tx_id = tx.id();

        // WAL: write AbortTxn record (no need to wait for persistence)
        if let Some(wal) = self.wal_buffer.read().await.as_ref() {
            wal.append(WalRecord::AbortTxn { tx_id }).await;
        }

        // M10: Cleanup uncommitted versions
        self.abort_cleanup_versions(tx_id, buffer_pool, tables)
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
    /// Called by InsertExecutor/UpdateExecutor/DeleteExecutor when creating
    /// new versions. Versions are aggregated per table so an abort can roll
    /// back every table the transaction touched (MS07-T04).
    pub async fn record_version(&self, tx_id: u64, table_name: &str, row_id: RowId) {
        let mut versions = self.tx_versions.write().await;
        versions
            .entry(tx_id)
            .or_default()
            .entry(table_name.to_string())
            .or_default()
            .insert(row_id);
    }

    /// Get all versions recorded for a transaction, across all its tables
    /// (for testing)
    pub async fn get_tx_versions(&self, tx_id: u64) -> HashSet<RowId> {
        self.tx_versions
            .read()
            .await
            .get(&tx_id)
            .map(|tables| tables.values().flatten().copied().collect())
            .unwrap_or_default()
    }

    /// Get the per-table versions recorded for a transaction (for rollback)
    pub async fn tx_version_tables(&self, tx_id: u64) -> HashMap<String, HashSet<RowId>> {
        self.tx_versions
            .read()
            .await
            .get(&tx_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Get tx_versions flattened across tables (for testing)
    pub async fn tx_versions(&self) -> HashMap<u64, HashSet<RowId>> {
        self.tx_versions
            .read()
            .await
            .iter()
            .map(|(tx_id, tables)| {
                (
                    *tx_id,
                    tables.values().flatten().copied().collect::<HashSet<_>>(),
                )
            })
            .collect()
    }

    /// Get current max TxId (for testing)
    pub fn current_tx_id(&self) -> u64 {
        self.tx_id_allocator.current()
    }

    /// Advance the tx id allocator past every id observed by recovery
    /// (MS09 Iter000 001-replan, D10): the next allocated id is strictly
    /// greater than `max_used`, so ids are never reused after a restart and
    /// the Read Committed high-water argument ("every id ≤ the allocator's
    /// current value is committed, aborted, or active") stays sound.
    pub fn advance_past(&self, max_used: u64) {
        self.tx_id_allocator.advance_past(max_used);
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
        let tx_versions: HashSet<RowId> = versions
            .get(&tx_id)
            .map(|tables| tables.values().flatten().copied().collect())
            .unwrap_or_default();

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
    /// For each uncommitted version created by this transaction, grouped by
    /// table (MS07-T04):
    /// - If it has a previous version, update index to point to previous
    /// - If it has no previous version, delete from index
    ///
    /// MS24 Iter001 replan (2.9/D9) 增加 B 趟：DELETE / REPLACE 的墓碑 slot
    /// 从不进入索引（条目在删除时已清理），`find_key_by_row_id` 恒为 `None`，
    /// 整段还原被跳过 → 回滚后被删行的 PK 与唯一条目不恢复（PK 等值点查漏行、
    /// 唯一值被释放致 `UNIQUE` 静默失效）。两趟划分在任何索引写入之前一次性
    /// 完成——这是处理顺序与 `record_version` 集合迭代顺序无关的唯一保证点。
    pub async fn abort_cleanup_versions(
        &self,
        tx_id: u64,
        buffer_pool: &BufferPool,
        tables: &HashMap<String, Arc<TableMeta>>,
    ) -> Result<()> {
        let versions = self.tx_versions.read().await;
        let tx_versions = versions.get(&tx_id).cloned().unwrap_or_default();
        drop(versions);

        for (table_name, row_ids) in tx_versions {
            let table_meta = tables.get(&table_name).ok_or_else(|| {
                StorageError::ExecutionError(format!(
                    "abort cleanup for tx {}: no table meta resolved for table '{}'",
                    tx_id, table_name
                ))
            })?;

            // A 趟 / B 趟划分（MS24 replan 2.9）：按「当前是否持有索引条目」
            // 分桶——A 趟为 INSERT / UPDATE 形态（条目在位，现状回退或移除），
            // B 趟为条目已随删除清理的墓碑（需从前驱存活版本还原）。
            let mut indexed = Vec::new();
            let mut tombstones = Vec::new();
            for row_id in row_ids {
                if table_meta
                    .index_manager
                    .find_key_by_row_id(row_id)
                    .await
                    .is_some()
                {
                    indexed.push(row_id);
                } else {
                    tombstones.push(row_id);
                }
            }

            for row_id in indexed {
                let header = buffer_pool.read_version_header(row_id).await?;

                let key = table_meta.index_manager.find_key_by_row_id(row_id).await;

                if let Some(key) = key {
                    if let Some(prev_row_id) = header.next_version() {
                        table_meta.index_manager.update(&key, prev_row_id).await?;
                    } else {
                        table_meta.index_manager.delete(&key).await?;
                    }
                }

                // MS23 Iteration 001 (2.7/D8): 唯一索引同型修复——不修复则
                // 回滚的 INSERT 残留唯一条目，后续同值插入假阳性
                // DuplicateKey。
                for (_, uindex) in &table_meta.unique_indexes {
                    if let Some(key) = uindex.find_key_by_row_id(row_id).await {
                        if let Some(prev_row_id) = header.next_version() {
                            uindex.update(&key, prev_row_id).await?;
                        } else {
                            uindex.delete(&key).await?;
                        }
                    }
                }

                // MS07-T04: tombstone the aborted version. Index fixup alone
                // leaves the tuple in its data-page slot, and snapshot-less
                // scans (DataScan with `snapshot: None`) yield every slot
                // that is not deleted — the rolled-back row would stay
                // visible to `SELECT *`. Marking it aborted (MS09 Iter000
                // T4/D2: create_tx_id = 0 + delete sentinel) makes scans skip
                // it and — unlike the plain delete sentinel — marks it as
                // belonging to no transaction, so the tombstone never
                // suppresses the surviving predecessor versions.
                update_version_header_in_data_page(buffer_pool, row_id, header.mark_aborted(), &[])
                    .await?;
            }

            for row_id in tombstones {
                restore_tombstone_index_entries(buffer_pool, table_meta, tx_id, row_id).await?;

                // 与 A 趟同型的中性化：墓碑 slot 转为 aborted（create_tx_id = 0
                // + delete 哨兵），扫描跳过且不抑制前驱存活版本——回滚后
                // 「该删除未发生」。
                let header = buffer_pool.read_version_header(row_id).await?;
                update_version_header_in_data_page(buffer_pool, row_id, header.mark_aborted(), &[])
                    .await?;
            }
        }

        Ok(())
    }
}

/// 墓碑行索引条目还原（MS24 Iter001 replan 2.9 / design D9）：从墓碑的
/// `next_version()` 出发回溯版本链，跳过本事务创建的版本，取首个非本事务
/// 版本为还原目标，经 `wal::recovery::extract_index_keys` 派生 PK 键与各
/// 唯一列键后逐项 `insert` 还原（条目已在删除时移除）。
///
/// 终止与跳过条件（均不报错，与既有 `SlotNotFound` 容忍同型）：
/// - 墓碑无前驱（`next_version()` 为 `None`）——本事务插入后同事务删除的
///   新行形态，无存活版本可还原；
/// - 链上全部版本均由本事务创建——同上；
/// - 链回溯超出 `MAX_TOMBSTONE_CHAIN_WALK` 上限（仅防御损坏库的无环性）；
/// - slot 缺失 / 目标版本本身是墓碑 / 键不可键控（无键行、NULL 唯一值）——
///   对应位 `None`，`extract_index_keys` 已按此语义返回。
async fn restore_tombstone_index_entries(
    buffer_pool: &BufferPool,
    table_meta: &Arc<TableMeta>,
    tx_id: u64,
    tombstone_row_id: RowId,
) -> Result<()> {
    let unique_cols: Vec<usize> = table_meta
        .unique_indexes
        .iter()
        .map(|(col_idx, _)| *col_idx)
        .collect();

    let mut cursor = buffer_pool
        .read_version_header(tombstone_row_id)
        .await?
        .next_version();
    let mut steps = 0usize;

    while let Some(row_id) = cursor {
        if steps >= MAX_TOMBSTONE_CHAIN_WALK {
            break;
        }
        steps += 1;

        // 单次页读同时取出版本头与（命中还原目标时的）元组字节。
        let read = read_tuple_from_data_page(buffer_pool, row_id, |header, bytes| {
            let is_target = header.create_tx_id() != tx_id && !header.is_deleted();
            Ok((header, is_target.then(|| bytes.to_vec())))
        })
        .await;

        let Ok((header, tuple)) = read else {
            break;
        };

        if header.create_tx_id() == tx_id {
            // 本事务创建的版本（update→delete 形态）——继续回溯。
            cursor = header.next_version();
            continue;
        }

        if let Some(tuple) = tuple {
            let (pk_key, unique_keys) =
                crate::wal::recovery::extract_index_keys(table_meta, &unique_cols, &tuple);
            if let Some(pk_key) = pk_key {
                table_meta.index_manager.insert(&pk_key, row_id).await?;
            }
            for ((_, uindex), key) in table_meta.unique_indexes.iter().zip(unique_keys) {
                if let Some(key) = key {
                    uindex.insert(&key, row_id).await?;
                }
            }
        }
        break;
    }

    Ok(())
}

impl Default for TransactionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{write_tuple_to_data_page, BufferPool, FileStorage, TableManager};
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
        let storage = buffer_pool.storage().clone();
        let table_manager = TableManager::new(buffer_pool.clone(), storage)
            .await
            .unwrap();
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

        let tables = HashMap::from([("test_table".to_string(), table_meta)]);
        manager.abort(tx, &buffer_pool, &tables).await.unwrap();

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
        let tables = HashMap::from([("test_table".to_string(), table_meta)]);
        manager.abort(tx2, &buffer_pool, &tables).await.unwrap();
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

        let tables = HashMap::from([("test_table".to_string(), table_meta)]);
        manager.abort(tx2, &buffer_pool, &tables).await.unwrap();
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
        manager.record_version(1, "t1", row_id).await;
        let versions = manager.get_tx_versions(1).await;
        assert!(versions.contains(&row_id));
        assert_eq!(versions.len(), 1);
    }

    #[tokio::test]
    async fn test_record_version_multiple() {
        let manager = TransactionManager::new();
        let row_id1 = RowId::new(1, 0);
        let row_id2 = RowId::new(2, 0);
        manager.record_version(1, "t1", row_id1).await;
        manager.record_version(1, "t1", row_id2).await;
        let versions = manager.get_tx_versions(1).await;
        assert!(versions.contains(&row_id1));
        assert!(versions.contains(&row_id2));
        assert_eq!(versions.len(), 2);
    }

    #[tokio::test]
    async fn test_record_version_multiple_tables() {
        let manager = TransactionManager::new();
        manager.record_version(1, "t1", RowId::new(1, 0)).await;
        manager.record_version(1, "t2", RowId::new(2, 0)).await;
        manager.record_version(1, "t1", RowId::new(3, 0)).await;

        // Union view keeps the legacy flat per-tx shape.
        let versions = manager.get_tx_versions(1).await;
        assert_eq!(versions.len(), 3);

        // Per-table view separates the two tables.
        let tables = manager.tx_version_tables(1).await;
        assert_eq!(tables.get("t1").unwrap().len(), 2);
        assert_eq!(tables.get("t2").unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_abort_cleanup_multi_table() {
        use crate::transaction::VersionHeader;

        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();
        let storage = buffer_pool.storage().clone();
        let table_manager = TableManager::new(buffer_pool.clone(), storage)
            .await
            .unwrap();
        for name in ["t1", "t2"] {
            table_manager
                .create_table(
                    name,
                    vec![("id".to_string(), crate::storage::ColumnType::Int)],
                    "id",
                )
                .await
                .unwrap();
        }
        let t1 = table_manager.get_table("t1").await.unwrap();
        let t2 = table_manager.get_table("t2").await.unwrap();

        let tx = manager.begin().await;
        let tx_id = tx.id();

        // One uncommitted insert per table, each registered in its index.
        let tuple = vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let rid1 =
            write_tuple_to_data_page(&buffer_pool, &t1, &VersionHeader::new(tx_id, None), &tuple)
                .await
                .unwrap();
        let rid2 =
            write_tuple_to_data_page(&buffer_pool, &t2, &VersionHeader::new(tx_id, None), &tuple)
                .await
                .unwrap();
        manager.record_version(tx_id, "t1", rid1).await;
        manager.record_version(tx_id, "t2", rid2).await;
        t1.index_manager.insert(b"1", rid1).await.unwrap();
        t2.index_manager.insert(b"2", rid2).await.unwrap();

        let metas = HashMap::from([
            ("t1".to_string(), t1.clone()),
            ("t2".to_string(), t2.clone()),
        ]);
        manager.abort(tx, &buffer_pool, &metas).await.unwrap();

        // Both index entries are rolled back; version bookkeeping is cleared.
        assert_eq!(t1.index_manager.search(b"1").await.unwrap(), None);
        assert_eq!(t2.index_manager.search(b"2").await.unwrap(), None);
        assert!(manager.get_tx_versions(tx_id).await.is_empty());

        // Both aborted versions are tombstoned so snapshot-less scans skip
        // them (no residue after rollback).
        assert!(buffer_pool
            .read_version_header(rid1)
            .await
            .unwrap()
            .is_deleted());
        assert!(buffer_pool
            .read_version_header(rid2)
            .await
            .unwrap()
            .is_deleted());
    }

    #[tokio::test]
    async fn test_abort_cleanup_missing_table_meta_errors() {
        use crate::transaction::VersionHeader;

        let manager = TransactionManager::new();
        let buffer_pool = create_test_buffer_pool();
        let table_meta = create_test_table(buffer_pool.clone()).await;

        let tx = manager.begin().await;
        let tx_id = tx.id();
        let tuple = vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let rid = write_tuple_to_data_page(
            &buffer_pool,
            &table_meta,
            &VersionHeader::new(tx_id, None),
            &tuple,
        )
        .await
        .unwrap();
        manager.record_version(tx_id, "test_table", rid).await;

        // No meta resolved for the recorded table: abort must surface an
        // error instead of silently leaving a dangling index entry.
        let empty = HashMap::new();
        let err = manager.abort(tx, &buffer_pool, &empty).await.unwrap_err();
        assert!(err.to_string().contains("test_table"), "got: {}", err);
    }

    #[tokio::test]
    async fn test_get_tx_versions_empty() {
        let manager = TransactionManager::new();
        let versions = manager.get_tx_versions(999).await;
        assert!(versions.is_empty());
    }

    // MS09 Iter000 001-replan (T6-R3/D10): after recovery hands the max used
    // tx id to the allocator, `current_tx_id` reflects the watermark and the
    // next begun transaction gets an id strictly above every recovered id.

    #[tokio::test]
    async fn test_advance_past_reflected_in_current_and_next_begin() {
        let manager = TransactionManager::new();
        assert_eq!(manager.current_tx_id(), 0);

        manager.advance_past(42);
        assert_eq!(manager.current_tx_id(), 42);

        let tx = manager.begin().await;
        assert_eq!(tx.id(), 43);
    }
}
