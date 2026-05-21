use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

use crate::executor::Value;
use crate::storage::btree::IndexManager;
use crate::storage::page_format::ColumnType;
use crate::storage::page_id::PageId;
use crate::storage::{delete_tuple_from_data_page, BufferPool, Result, StorageError};

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

impl TableMeta {
    /// Garbage collect old committed versions from the version chain (M10 GC)
    ///
    /// This is an optional maintenance operation that removes old committed
    /// versions that are no longer the latest version for a key.
    ///
    /// Returns the number of versions cleaned up.
    pub async fn gc_table(&self, buffer_pool: &BufferPool) -> Result<usize> {
        let mut cleaned_count = 0;

        let all_entries = self.index_manager.scan_all().await?;

        for (_key, row_id) in all_entries {
            let mut current = Some(row_id);
            let mut old_versions = Vec::new();

            // Traverse version chain, collect old committed versions
            while let Some(current_id) = current {
                let header = buffer_pool.read_version_header(current_id).await?;

                // Collect committed old versions (not the latest)
                // The latest version is the one pointed to by the index (row_id)
                if header.commit_tx_id().is_some() && current_id != row_id {
                    old_versions.push(current_id);
                }

                current = header.next_version();
            }

            // Delete old versions
            for old_id in old_versions {
                delete_tuple_from_data_page(buffer_pool, old_id).await?;
                cleaned_count += 1;
            }
        }

        Ok(cleaned_count)
    }
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

    /// Drop a table by name.
    ///
    /// # Errors
    /// - `TableNotFound` when no table with `name` is registered.
    ///
    /// # Note
    /// This is a simplified implementation that only removes the table metadata.
    /// Physical page deletion is not implemented yet.
    pub async fn drop_table(&self, name: &str) -> Result<()> {
        let mut tables = self.tables.write().await;

        // Remove table from metadata (returns None if not found)
        tables
            .remove(name)
            .ok_or_else(|| StorageError::TableNotFound(name.to_string()))?;

        // TODO: In the future, we should also:
        // 1. Delete all data pages associated with the table
        // 2. Clean up the index manager
        // 3. Deallocate pages from storage

        Ok(())
    }
}
