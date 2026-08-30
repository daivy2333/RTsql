use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

use crate::executor::Value;
use crate::storage::btree::IndexManager;
use crate::storage::catalog::{
    Catalog, CatalogColumnRow, CatalogRow, COLUMNS_SYSTEM_NAME, TABLES_SYSTEM_NAME,
};
use crate::storage::data_page::write_tuple_to_data_page;
use crate::storage::page_format::{ColumnType, SlottedPageRef};
use crate::storage::page_id::PageId;
use crate::storage::{AsyncStorage, BufferPool, Result, StorageError};
use crate::transaction::VersionHeader;

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
                if header.commit_tx_id().is_some() && current_id != row_id {
                    old_versions.push(current_id);
                }

                current = header.next_version();
            }

            // Delete old versions
            for old_id in old_versions {
                crate::storage::delete_tuple_from_data_page(buffer_pool, old_id).await?;
                cleaned_count += 1;
            }
        }

        Ok(cleaned_count)
    }
}

/// Manages table schemas and per-table metadata.
///
/// MS07-T01: schemas are now persisted to disk via `Catalog`; an
/// in-memory `RwLock<HashMap<...>>` cache shadows the catalog for
/// read-heavy paths. On `Database::open`, the cache is rebuilt by
/// `open_or_init`.
pub struct TableManager {
    tables: RwLock<HashMap<String, Arc<TableMeta>>>,
    buffer_pool: Arc<BufferPool>,
    catalog: Arc<Catalog>,
}

impl TableManager {
    /// Create a new `TableManager` backed by the given buffer pool and
    /// storage. Bootstraps a fresh `Catalog` if the storage file is
    /// empty; otherwise opens the existing catalog.
    pub async fn new(
        buffer_pool: Arc<BufferPool>,
        storage: Arc<dyn AsyncStorage>,
    ) -> Result<Arc<Self>> {
        // If the file is empty (no pages), bootstrap the catalog pages.
        // Otherwise open the existing catalog.
        let catalog = if storage.page_count() == 0 {
            Catalog::bootstrap(buffer_pool.clone(), storage.clone()).await?
        } else {
            Catalog::open(buffer_pool.clone(), storage.clone()).await?
        };

        Ok(Arc::new(Self {
            tables: RwLock::new(HashMap::new()),
            buffer_pool,
            catalog,
        }))
    }

    /// Access the underlying `Catalog` (for callers that need to read or
    /// persist schema-level data outside the in-memory cache).
    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog
    }

    /// Rebuild the in-memory `tables` cache by scanning the catalog.
    ///
    /// For a freshly-bootstrapped database this is a no-op (the catalog
    /// is empty). For a database opened from an existing file, this
    /// restores every persisted `TableMeta` so subsequent DML works.
    pub async fn open_or_init(&self) -> Result<()> {
        let rows = self.catalog.scan_tables().await?;
        if rows.is_empty() {
            return Ok(());
        }

        let mut tables = self.tables.write().await;
        for row in rows {
            let cols = self.catalog.scan_columns(&row.table_name).await?;
            let columns: Vec<(String, ColumnType)> = cols
                .iter()
                .map(|c| (c.column_name.clone(), c.column_type.clone()))
                .collect();
            let pk = row.pk_column.clone();
            let pk_index = row.pk_index as usize;
            let data_page_head = PageId(row.data_page_head as u64);
            let data_page_tail = PageId(row.data_page_tail as u64);
            let root_index_page = PageId(row.index_root_page_id as u64);
            let index_manager = Arc::new(IndexManager::from_root(
                self.buffer_pool.clone(),
                root_index_page,
            )?);
            let table_meta = Arc::new(TableMeta {
                name: row.table_name.clone(),
                columns,
                pk_column: pk,
                pk_index,
                index_manager,
                data_page_head,
                data_page_tail: Mutex::new(data_page_tail),
            });
            tables.insert(row.table_name, table_meta);
        }
        Ok(())
    }

    /// Register a new table.
    ///
    /// # Errors
    /// - `DuplicateTable` when a table with `name` already exists.
    /// - `ColumnNotFound` when the primary-key column name is not present in
    ///   `columns`.
    /// - `ReservedTableName` when `name` is a system table name
    ///   (`__tables` / `__columns`).
    pub async fn create_table(
        &self,
        name: &str,
        columns: Vec<(String, ColumnType)>,
        pk: &str,
    ) -> Result<()> {
        // --- reserved name guard (BEFORE duplicate check) ---
        if name == TABLES_SYSTEM_NAME || name == COLUMNS_SYSTEM_NAME {
            return Err(StorageError::ReservedTableName(name.to_string()));
        }

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
        let index_root_page_id = index_manager.root_page_id().0 as u32;

        // --- build TableMeta ---
        let table_meta = Arc::new(TableMeta {
            name: name.to_string(),
            columns: columns.clone(),
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
            tables.insert(name.to_string(), table_meta.clone());
        }

        // --- persist to catalog (after in-memory insert) ---
        let catalog_row = CatalogRow {
            table_name: name.to_string(),
            data_page_head: page_id.0 as u32,
            index_root_page_id,
            pk_index: pk_index as u32,
            pk_column: pk.to_string(),
            column_count: columns.len() as u32,
            data_page_tail: page_id.0 as u32,
        };
        let catalog_cols: Vec<CatalogColumnRow> = columns
            .iter()
            .enumerate()
            .map(|(idx, (col_name, col_type))| CatalogColumnRow {
                table_name: name.to_string(),
                column_index: idx as u32,
                column_name: col_name.clone(),
                column_type: col_type.clone(),
                not_null: false,
                unique: false,
            })
            .collect();
        if let Err(e) = self.catalog.insert_table(&catalog_row, &catalog_cols).await {
            // Roll back the in-memory insert to keep state consistent.
            let mut tables = self.tables.write().await;
            tables.remove(name);
            return Err(e);
        }

        Ok(())
    }

    /// Look up a table by name.
    pub async fn get_table(&self, name: &str) -> Result<Arc<TableMeta>> {
        let tables = self.tables.read().await;
        tables
            .get(name)
            .cloned()
            .ok_or_else(|| StorageError::TableNotFound(name.to_string()))
    }

    /// Check whether a table with the given name exists.
    pub fn table_exists(&self, name: &str) -> bool {
        match self.tables.try_read() {
            Ok(tables) => tables.contains_key(name),
            Err(_) => false,
        }
    }

    /// Drop a table by name.
    ///
    /// Removes the in-memory cache entry and the catalog rows, then frees
    /// the table's data pages and BTree index pages to the storage
    /// free-list (`FileStorage::free_pages`). Same-process `allocate_page`
    /// prefers popping from the free-list, so `file_len` no longer grows
    /// monotonically. The free-list itself is not persisted (MS07-T02):
    /// after a restart the freed pages are scattered on disk but
    /// unreachable — their catalog rows were erased first.
    pub async fn drop_table(&self, name: &str) -> Result<()> {
        // Reserved-name guard: never allow dropping system tables.
        if name == TABLES_SYSTEM_NAME || name == COLUMNS_SYSTEM_NAME {
            return Err(StorageError::ReservedTableName(name.to_string()));
        }

        // Take TableMeta now (clone Arc under read lock); we need its data
        // page head + index manager after the in-memory entry is removed.
        let table_meta = self.get_table(name).await?;

        // First delete from catalog (idempotent — silently succeeds if absent).
        self.catalog.delete_table(name).await?;

        // Then remove from in-memory cache.
        {
            let mut tables = self.tables.write().await;
            tables
                .remove(name)
                .ok_or_else(|| StorageError::TableNotFound(name.to_string()))?;
        }

        // Physical free (best effort): reduce the table's pages to the
        // storage free-list so subsequent allocate_page can reuse them.
        let index_pages = match table_meta.index_manager.collect_all_pages().await {
            Ok(pages) => pages,
            Err(e) => {
                eprintln!("[drop_table] collect_all_pages({}) failed: {}", name, e);
                Vec::new()
            }
        };
        let data_pages = self.collect_data_pages(table_meta.data_page_head).await;

        for page in index_pages.into_iter().chain(data_pages) {
            if let Err(e) = self.buffer_pool.free_page(page).await {
                eprintln!("[drop_table] free_page({}) failed: {}", page.0, e);
            }
        }

        Ok(())
    }

    /// Walk the table's data-page chain starting at `head`, collecting every
    /// `PageId` reachable via the SlottedPage `next_page_id` header.
    ///
    /// Best effort: a read error stops the walk and returns what was
    /// collected so far (callers treat physical free as best effort).
    async fn collect_data_pages(&self, head: PageId) -> Vec<PageId> {
        let mut pages = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut current = head;

        while current.0 != 0 && visited.insert(current.0) {
            pages.push(current);
            current = match self
                .buffer_pool
                .with_page_data(current, |data| {
                    let slotted = SlottedPageRef::new(data);
                    Ok(slotted.header().next_page_id)
                })
                .await
            {
                Ok(next) => PageId(next as u64),
                Err(e) => {
                    eprintln!("[drop_table] read data page {:?} failed: {}", current, e);
                    break;
                }
            };
        }

        pages
    }

    /// Write a tuple to a table's data pages, transparently updating the
    /// persisted `data_page_tail` in the catalog when a new page is
    /// auto-allocated.
    ///
    /// MS07-T01: this is the canonical write path for DML. The existing
    /// `write_tuple_to_data_page` (in `data_page.rs`) updates the
    /// in-memory `data_page_tail`; we additionally persist the new tail
    /// to the catalog so that restart-after-write still sees a correct
    /// tail pointer.
    pub async fn write_tuple(
        &self,
        table_meta: &Arc<TableMeta>,
        version_header: &VersionHeader,
        tuple_bytes: &[u8],
    ) -> Result<crate::storage::page_format::RowId> {
        let old_tail = *table_meta.data_page_tail.lock().unwrap();
        let row_id =
            write_tuple_to_data_page(&self.buffer_pool, table_meta, version_header, tuple_bytes)
                .await?;
        let new_page_id = PageId(row_id.page_id as u64);
        if new_page_id != old_tail {
            // A new page was auto-allocated; persist the new tail.
            self.catalog
                .update_table_tail(&table_meta.name, new_page_id.0 as u32)
                .await?;
        }
        Ok(row_id)
    }
}
