//! Database coordinator - wires together all RTsql components
//!
//! M7: Single entry point for opening databases, creating tables, and executing SQL.

use crate::network::protocol::Response;
use crate::storage::{BufferPool, ColumnType, FileStorage, Result, TableManager, TableMeta};
use crate::transaction::TransactionManager;
use std::path::Path;
use std::sync::Arc;

/// Database is the central coordinator that owns all major RTsql subsystems.
#[derive(Clone)]
pub struct Database {
    pub buffer_pool: Arc<BufferPool>,
    pub table_manager: Arc<TableManager>,
    pub transaction_manager: Arc<TransactionManager>,
}

impl Database {
    pub async fn open(path: &Path) -> Result<Self> {
        let storage: Arc<dyn crate::storage::AsyncStorage> = Arc::new(FileStorage::open(path)?);
        let buffer_pool = Arc::new(BufferPool::new(100, storage)?);
        let table_manager = Arc::new(TableManager::new(buffer_pool.clone()));
        let transaction_manager = Arc::new(TransactionManager::new());
        Ok(Self {
            buffer_pool,
            table_manager,
            transaction_manager,
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
