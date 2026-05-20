use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

use crate::executor::Value;
use crate::storage::btree::IndexManager;
use crate::storage::page_format::ColumnType;
use crate::storage::page_id::PageId;
use crate::storage::{BufferPool, Result, StorageError};

/// Column schema with constraints
#[derive(Debug, Clone)]
pub struct ColumnSchema {
    /// Column name
    pub name: String,
    /// Column data type
    pub data_type: ColumnType,
    /// NOT NULL constraint
    pub not_null: bool,
    /// UNIQUE constraint
    pub unique: bool,
    /// Default value (if any)
    pub default_value: Option<Value>,
}

impl ColumnSchema {
    /// Create a new column schema with just name and type
    pub fn new(name: String, data_type: ColumnType) -> Self {
        Self {
            name,
            data_type,
            not_null: false,
            unique: false,
            default_value: None,
        }
    }

    /// Convert to tuple format for TableManager::create_table
    pub fn to_tuple(&self) -> (String, ColumnType) {
        (self.name.clone(), self.data_type.clone())
    }
}

/// Table metadata: schema, primary key, per-table index, data page chain.
pub struct TableMeta {
    pub name: String,
    pub columns: Vec<(String, ColumnType)>,
    pub pk_column: String,
    pub pk_index: usize,
    pub index_manager: Arc<IndexManager>,
    pub data_page_head: PageId,
    pub data_page_tail: Mutex<PageId>,
}

/// Manages table schemas and per-table metadata.
///
/// All operations are internally synchronized via a `RwLock<HashMap<...>>`.
pub struct TableManager {
    tables: RwLock<HashMap<String, Arc<TableMeta>>>,
    buffer_pool: Arc<BufferPool>,
}

impl TableManager {
    /// Create a new `TableManager` backed by the given buffer pool.
    pub fn new(buffer_pool: Arc<BufferPool>) -> Self {
        Self {
            tables: RwLock::new(HashMap::new()),
            buffer_pool,
        }
    }

    /// Register a new table.
    ///
    /// # Errors
    /// - `DuplicateTable` when a table with `name` already exists.
    /// - `ColumnNotFound` when the primary-key column name is not present in
    ///   `columns`.
    pub async fn create_table(
        &self,
        name: &str,
        columns: Vec<(String, ColumnType)>,
        pk: &str,
    ) -> Result<()> {
        // --- duplicate check (read lock) ---
        {
            let tables = self.tables.read().await;
            if tables.contains_key(name) {
                return Err(StorageError::DuplicateTable(name.to_string()));
            }
        }

        // --- validate PK column ---
        let pk_index = columns
            .iter()
            .position(|(col_name, _)| col_name == pk)
            .ok_or_else(|| StorageError::ColumnNotFound(pk.to_string()))?;

        // --- allocate data page head ---
        let page_id = self.buffer_pool.storage().allocate_page().await?;

        // --- create per-table index ---
        // IndexManager::new is sync but internally calls block_on, so we
        // offload it to spawn_blocking to avoid blocking the async runtime.
        let bp = self.buffer_pool.clone();
        let index_manager =
            Arc::new(tokio::task::spawn_blocking(move || IndexManager::new(bp)).await??);

        // --- build TableMeta ---
        let table_meta = Arc::new(TableMeta {
            name: name.to_string(),
            columns,
            pk_column: pk.to_string(),
            pk_index,
            index_manager,
            data_page_head: page_id,
            data_page_tail: Mutex::new(page_id),
        });

        // --- atomically insert under write lock (TOCTOU-safe double-check) ---
        {
            let mut tables = self.tables.write().await;
            if tables.contains_key(name) {
                return Err(StorageError::DuplicateTable(name.to_string()));
            }
            tables.insert(name.to_string(), table_meta);
        }

        Ok(())
    }

    /// Look up a table by name.
    ///
    /// Returns an `Arc<TableMeta>` so the caller can share ownership cheaply.
    ///
    /// # Errors
    /// - `TableNotFound` when no table with `name` is registered.
    pub async fn get_table(&self, name: &str) -> Result<Arc<TableMeta>> {
        let tables = self.tables.read().await;
        tables
            .get(name)
            .cloned()
            .ok_or_else(|| StorageError::TableNotFound(name.to_string()))
    }

    /// Check whether a table with the given name exists.
    ///
    /// This is a non-async convenience that uses `try_read` internally.
    /// Under extreme contention it may return `false` for a table that is
    /// currently being inserted, which is acceptable for this use case.
    pub fn table_exists(&self, name: &str) -> bool {
        match self.tables.try_read() {
            Ok(tables) => tables.contains_key(name),
            Err(_) => false,
        }
    }
}
