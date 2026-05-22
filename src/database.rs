//! Database coordinator - wires together all RTsql components
//!
//! M7: Single entry point for opening databases, creating tables, and executing SQL.
//! M11: WAL integration for crash recovery.

use crate::network::protocol::Response;
use crate::plan_cache::PlanCache;
use crate::storage::{BufferPool, ColumnType, FileStorage, Result, TableManager, TableMeta};
use crate::transaction::TransactionManager;
use crate::wal::{RecoveryManager, WalWriter};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Database is the central coordinator that owns all major RTsql subsystems.
#[derive(Clone)]
pub struct Database {
    pub buffer_pool: Arc<BufferPool>,
    pub table_manager: Arc<TableManager>,
    pub transaction_manager: Arc<TransactionManager>,
    pub wal_writer: Arc<WalWriter>,
    pub plan_cache: Arc<Mutex<PlanCache>>,
}

impl Database {
    pub async fn open(path: &Path) -> Result<Self> {
        // 1. Recover WAL (get committed and aborted transaction IDs)
        let (committed_tx_ids, aborted_tx_ids) = RecoveryManager::recover(path)
            .map_err(|e| crate::storage::StorageError::WalError(e.to_string()))?;

        // TODO: In future milestones, replay uncommitted transactions
        // For now, just track the transaction states
        let _ = (committed_tx_ids, aborted_tx_ids);

        // 2. Initialize storage
        let storage: Arc<dyn crate::storage::AsyncStorage> = Arc::new(FileStorage::open(path)?);
        let buffer_pool = Arc::new(BufferPool::new(100, storage)?);
        let table_manager = Arc::new(TableManager::new(buffer_pool.clone()));
        let transaction_manager = Arc::new(TransactionManager::new());

        // 3. Initialize WAL
        let wal_writer = Arc::new(
            WalWriter::open(path)
                .map_err(|e| crate::storage::StorageError::WalError(e.to_string()))?,
        );

        // 4. Initialize plan cache
        let plan_cache = Arc::new(Mutex::new(PlanCache::new()));

        Ok(Self {
            buffer_pool,
            table_manager,
            transaction_manager,
            wal_writer,
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
}
