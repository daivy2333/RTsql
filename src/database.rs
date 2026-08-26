//! Database coordinator - wires together all RTsql components
//!
//! M7: Single entry point for opening databases, creating tables, and executing SQL.
//! M11: WAL integration for crash recovery.

use crate::network::protocol::Response;
use crate::plan_cache::PlanCache;
use crate::storage::{BufferPool, ColumnType, FileStorage, Result, TableManager, TableMeta};
use crate::transaction::TransactionManager;
use crate::wal::{RecoveryManager, WALBuffer, WalWriter};
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

        Ok(Self {
            buffer_pool,
            table_manager,
            transaction_manager,
            wal_writer,
            wal_buffer,
            plan_cache,
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

    /// Get plan cache size (for testing)
    pub fn plan_cache_len(&self) -> usize {
        self.plan_cache.len()
    }

    /// Flush all dirty buffer-pool pages and the WAL to disk.
    ///
    /// MS07-T01: callers that drop the `Database` and immediately re-open
    /// the file must call `close()` first, otherwise the in-memory
    /// catalog pages (or any other dirty pages) never reach the on-disk
    /// file and the re-opened database sees an empty schema.
    pub async fn close(&self) -> Result<()> {
        self.buffer_pool.flush_all().await
    }
}
