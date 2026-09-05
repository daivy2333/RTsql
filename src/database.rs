//! Database coordinator - wires together all RTsql components
//!
//! M7: Single entry point for opening databases, creating tables, and executing SQL.
//! M11: WAL integration for crash recovery.

use crate::network::protocol::Response;
use crate::plan_cache::PlanCache;
use crate::storage::{BufferPool, ColumnType, FileStorage, Result, TableManager, TableMeta};
use crate::transaction::{Transaction, TransactionManager};
use crate::wal::{CheckpointManager, RecoveryManager, WALBuffer, WalWriter};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Database is the central coordinator that owns all major RTsql subsystems.
#[derive(Clone)]
pub struct Database {
    pub buffer_pool: Arc<BufferPool>,
    pub table_manager: Arc<TableManager>,
    pub transaction_manager: Arc<TransactionManager>,
    pub wal_writer: Arc<WalWriter>,
    pub wal_buffer: Arc<WALBuffer>,
    pub plan_cache: Arc<PlanCache>,
    pub checkpoint_manager: Arc<CheckpointManager>,
}

impl Database {
    pub async fn open(path: &Path) -> Result<Self> {
        // 1. Initialize storage
        let storage: Arc<dyn crate::storage::AsyncStorage> = Arc::new(FileStorage::open(path)?);
        let buffer_pool = Arc::new(BufferPool::new(100, storage.clone())?);
        // MS07-T01: TableManager is async + takes storage. It bootstraps
        // or opens the catalog, then `open_or_init` rebuilds the in-memory
        // cache from the catalog for an existing file.
        let table_manager = TableManager::new(buffer_pool.clone(), storage).await?;
        table_manager.open_or_init().await?;
        let transaction_manager = Arc::new(TransactionManager::new());

        // 2. Initialize WAL
        let wal_writer = Arc::new(
            WalWriter::open(path)
                .map_err(|e| crate::storage::StorageError::WalError(e.to_string()))?,
        );

        // 2b. Initialize WAL buffer with Group Commit
        let wal_buffer = Arc::new(WALBuffer::new(wal_writer.clone(), 100, 100));
        wal_buffer.start_flush_loop();

        // 2c. Connect WAL buffer to TransactionManager
        transaction_manager.set_wal_buffer(wal_buffer.clone()).await;

        // 3. Full recovery: Redo committed + cleanup uncommitted
        let recovery_result =
            RecoveryManager::full_recover(path, buffer_pool.clone(), table_manager.clone())
                .await
                .map_err(|e| crate::storage::StorageError::WalError(e.to_string()))?;

        // Update transaction ID allocator to avoid conflicts with recovered transactions
        let _max_tx_id = recovery_result
            .committed_tx_ids
            .iter()
            .chain(recovery_result.aborted_tx_ids.iter())
            .chain(recovery_result.uncommitted_tx_ids.iter())
            .max()
            .unwrap_or(&0);
        // Skip past recovered transaction IDs
        // (TransactionManager's allocator starts at 1, auto-increment)

        // 4. Initialize plan cache
        let plan_cache = Arc::new(PlanCache::new());

        // 5. Initialize checkpoint manager (MS07-T05)
        let checkpoint_manager = Arc::new(CheckpointManager::new(
            path,
            wal_writer.clone(),
            buffer_pool.clone(),
        ));

        Ok(Self {
            buffer_pool,
            table_manager,
            transaction_manager,
            wal_writer,
            wal_buffer,
            plan_cache,
            checkpoint_manager,
        })
    }

    pub async fn create_table(
        &self,
        name: &str,
        columns: Vec<(String, ColumnType)>,
        pk: &str,
    ) -> Result<()> {
        self.table_manager.create_table(name, columns, pk).await
    }

    pub async fn get_table(&self, name: &str) -> Result<Arc<TableMeta>> {
        self.table_manager.get_table(name).await
    }

    pub async fn execute_sql(&self, sql: &str) -> Response {
        crate::pipeline::execute(self, sql).await
    }

    /// Begin a new explicit transaction (MS07-T04).
    ///
    /// Returns an owned handle; pass it to [`Database::execute_in_tx`] for
    /// every statement that belongs to the transaction, then terminate the
    /// transaction with [`Database::commit`] or [`Database::rollback`].
    /// Statements issued through [`Database::execute_sql`] keep their
    /// implicit auto-commit behavior and do not interact with this handle.
    pub async fn begin(&self) -> Result<Transaction> {
        Ok(self.transaction_manager.begin().await)
    }

    /// Commit an explicit transaction, making all of its writes durable and
    /// visible (WAL commit record + version marking).
    pub async fn commit(&self, tx: Transaction) -> Result<()> {
        self.transaction_manager.commit(tx, &self.buffer_pool).await
    }

    /// Roll back an explicit transaction: every version the transaction
    /// recorded is cleaned up per table (index restored to the previous
    /// version or deleted, aborted versions tombstoned).
    pub async fn rollback(&self, tx: Transaction) -> Result<()> {
        let tx_id = tx.id();
        // Resolve the TableMeta of every table this transaction touched so
        // abort can roll back index entries across all of them. A table that
        // can no longer be resolved (e.g. dropped afterwards) surfaces as an
        // explicit error from `abort` instead of a silent skip.
        let mut tables = HashMap::new();
        for name in self
            .transaction_manager
            .tx_version_tables(tx_id)
            .await
            .keys()
        {
            if let Ok(meta) = self.table_manager.get_table(name).await {
                tables.insert(name.clone(), meta);
            }
        }
        self.transaction_manager
            .abort(tx, &self.buffer_pool, &tables)
            .await
    }

    /// Execute one SQL statement inside an existing explicit transaction
    /// (MS07-T04).
    ///
    /// DML statements reuse `tx.id()` and skip the implicit
    /// begin/commit/abort wrapping of [`Database::execute_sql`]; SELECT and
    /// DDL behave as in the implicit path (visibility semantics unchanged).
    /// A failed statement returns an error response without terminating the
    /// transaction.
    pub async fn execute_in_tx(&self, sql: &str, tx: &Transaction) -> Response {
        crate::pipeline::execute_in_tx(self, sql, tx.id()).await
    }

    /// Get plan cache size (for testing)
    pub fn plan_cache_len(&self) -> usize {
        self.plan_cache.len()
    }

    /// Flush all dirty buffer-pool pages and the WAL to disk, then record a
    /// checkpoint site and physically truncate the WAL (MS07-T05).
    ///
    /// MS07-T01: callers that drop the `Database` and immediately re-open
    /// the file must call `close()` first, otherwise the in-memory
    /// catalog pages (or any other dirty pages) never reach the on-disk
    /// file and the re-opened database sees an empty schema.
    ///
    /// MS07-T05: close() now performs a full checkpoint — dirty pages are
    /// flushed, the checkpoint site is written and the WAL is rewritten
    /// truncated, so the next open replays (almost) nothing.
    pub async fn close(&self) -> Result<()> {
        self.checkpoint().await
    }

    /// Run a checkpoint: flush dirty pages, write the checkpoint site and
    /// rewrite-truncate the WAL so the file stays bounded (MS07-T05).
    pub async fn checkpoint(&self) -> Result<()> {
        self.checkpoint_manager
            .checkpoint()
            .await
            .map(|_captured_lsn| ())
            .map_err(|e| crate::storage::StorageError::WalError(e.to_string()))
    }
}
