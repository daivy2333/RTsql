use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

use crate::executor::Value;
use crate::storage::btree::{CatalogRootSlot, IndexManager};
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
    /// MS23: per-column NOT NULL flags, index-aligned with `columns`.
    /// Catalog-persisted; enforced by the INSERT/UPDATE write paths.
    pub not_null: Vec<bool>,
    pub pk_column: String,
    pub pk_index: usize,
    pub index_manager: Arc<IndexManager>,
    /// MS23 Iteration 001 (2.3/D5): per-column UNIQUE indexes, one per
    /// qualifying column (INT ∧ unique ∧ non-PK), bound to its column
    /// ordinal, ascending by column order — the same order the catalog row
    /// persists `unique_roots` in. PK-declared UNIQUE consumes the PK
    /// index's existing uniqueness and never appears here. Empty for
    /// tables without qualifying columns and for legacy rows (no trailing
    /// catalog section) — flag-only semantics, no enforcement.
    pub unique_indexes: Vec<(usize, Arc<IndexManager>)>,
    /// MS24 Iteration 000 (D1): per-column declared DEFAULT literals,
    /// index-aligned with `columns` (`None` = no default). Catalog-persisted
    /// via the column row's DEFAULT tail section; consumed by the planner's
    /// subset-INSERT fill channel. Empty-`Option` for legacy rows.
    pub defaults: Vec<Option<Value>>,
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

    /// MS10-T02 Iter000 003-rework (R-T0b-R5): attach the catalog root-sync
    /// context to every restored table's index manager. Called by
    /// `Database::open` AFTER `full_recover` — replay runs context-free so
    /// recovery-time root changes are never persisted (load-point
    /// invariance across re-recoveries), while runtime DML after open
    /// persists root changes for the next restart.
    pub async fn attach_index_catalog_contexts(&self) {
        let tables = self.tables.read().await;
        for (name, meta) in tables.iter() {
            meta.index_manager.set_catalog_context(
                self.catalog.clone(),
                CatalogRootSlot::PrimaryKey {
                    table: name.clone(),
                },
            );
            // MS23 Iteration 001 (2.2/D5): restored UNIQUE trees get their
            // slot contexts too, so root splits persist to
            // `unique_roots[ordinal]`. `ordinal` is the tree's position in
            // `unique_indexes` — ascending qualifying-column order, the same
            // order the catalog row persists roots in.
            for (ordinal, (_, uindex)) in meta.unique_indexes.iter().enumerate() {
                uindex.set_catalog_context(
                    self.catalog.clone(),
                    CatalogRootSlot::Unique {
                        table: name.clone(),
                        ordinal,
                    },
                );
            }
        }
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
            // R-T0b-R5: no catalog context here — restored tables get it
            // attached AFTER crash recovery (`attach_index_catalog_contexts`).
            // Replay-time root changes must not be persisted: the recovery
            // load point stays fixed across re-recoveries (a rebuilt tree's
            // pages are not durably coordinated with the catalog row, and a
            // partial tree base would break the next replay).
            let index_manager = Arc::new(IndexManager::from_root(
                self.buffer_pool.clone(),
                root_index_page,
            )?);

            // MS23 Iteration 001 (2.3/D4): rebuild the per-column UNIQUE
            // indexes from the catalog row's `unique_roots` (order: ascending
            // qualifying column). A legacy row (no trailing section → empty
            // roots) keeps flag-only semantics: no unique indexes, no
            // enforcement — directly opening an old file works (R5-S4). A
            // non-empty roots section must match the qualifying column count
            // (rows written by the create path always do); a mismatch is a
            // data anomaly surfaced as an internal error.
            let unique_indexes = if row.unique_roots.is_empty() {
                Vec::new()
            } else {
                let qualifying: Vec<usize> = cols
                    .iter()
                    .enumerate()
                    .filter(|(idx, c)| {
                        c.unique
                            && matches!(c.column_type, ColumnType::Int)
                            && *idx != pk_index
                    })
                    .map(|(idx, _)| idx)
                    .collect();
                if qualifying.len() != row.unique_roots.len() {
                    return Err(StorageError::Internal(format!(
                        "table '{}': catalog row has {} unique roots but {} qualifying UNIQUE columns",
                        row.table_name,
                        row.unique_roots.len(),
                        qualifying.len()
                    )));
                }
                let mut rebuilt = Vec::with_capacity(qualifying.len());
                for (ordinal, col_idx) in qualifying.into_iter().enumerate() {
                    let root = PageId(row.unique_roots[ordinal] as u64);
                    let uindex = Arc::new(IndexManager::from_root(
                        self.buffer_pool.clone(),
                        root,
                    )?);
                    rebuilt.push((col_idx, uindex));
                }
                rebuilt
            };

            let table_meta = Arc::new(TableMeta {
                name: row.table_name.clone(),
                not_null: cols.iter().map(|c| c.not_null).collect(),
                // MS24 Iteration 000 (D1): declared DEFAULT literals read back
                // from the column rows (legacy rows → None).
                defaults: cols.iter().map(|c| c.default_value.clone()).collect(),
                columns,
                pk_column: pk,
                pk_index,
                index_manager,
                unique_indexes,
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
        let columns = columns
            .into_iter()
            .map(|(col_name, col_type)| (col_name, col_type, false, false, None))
            .collect();
        self.create_table_with_constraints(name, columns, pk).await
    }

    /// Register a new table, persisting per-column NOT NULL / UNIQUE flags to
    /// the catalog (MS10-T05 Iter000 001-rework, T5-R1). MS23: the NOT NULL
    /// flag is carried into `TableMeta` and enforced by the INSERT/UPDATE
    /// write paths; UNIQUE is enforced by dedicated per-column B-Trees
    /// (Iteration 001) that are allocated here for qualifying columns
    /// (INT ∧ unique ∧ non-PK), persisted via the catalog row's
    /// `unique_roots`, and rebuilt on open. `create_table` delegates here
    /// with both flags false, keeping legacy call sites' observable behavior
    /// identical.
    ///
    /// MS24 Iteration 000 (D1): the tuple gains a fifth element — the column's
    /// declared DEFAULT literal (`None` = no default), persisted in the
    /// catalog column row's DEFAULT tail section and carried into
    /// `TableMeta.defaults`.
    ///
    /// # Errors
    /// Same as [`TableManager::create_table`].
    pub async fn create_table_with_constraints(
        &self,
        name: &str,
        columns: Vec<(String, ColumnType, bool, bool, Option<Value>)>,
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
            .position(|(col_name, _, _, _, _)| col_name == pk)
            .ok_or_else(|| StorageError::ColumnNotFound(pk.to_string()))?;

        // --- allocate data page head ---
        let page_id = self.buffer_pool.storage().allocate_page().await?;

        // --- create per-table index ---
        // IndexManager::new is sync but internally calls block_on, so we
        // offload it to spawn_blocking to avoid blocking the async runtime.
        // R-T0b-R5: attach the catalog context so root splits during later
        // inserts persist to the catalog row for crash recovery.
        let bp = self.buffer_pool.clone();
        let catalog = self.catalog.clone();
        let table_name = name.to_string();
        let index_manager = Arc::new(
            tokio::task::spawn_blocking(move || IndexManager::new(bp))
                .await??
                .with_catalog_context(
                    catalog,
                    CatalogRootSlot::PrimaryKey {
                        table: table_name,
                    },
                ),
        );
        let index_root_page_id = index_manager.root_page_id().0 as u32;

        // --- MS23 Iteration 001 (2.3/D6): per-column UNIQUE indexes ---
        // One dedicated B-Tree per qualifying column (INT ∧ unique ∧ non-PK),
        // ascending column order. PK-declared UNIQUE consumes the PK index's
        // existing uniqueness (no second tree); non-INT unique flags never
        // reach a NEW table through the DDL path (2.4 rejects them) — a flag
        // here via direct `create_table_with_constraints` calls simply
        // doesn't qualify.
        let mut unique_indexes = Vec::new();
        let mut unique_roots = Vec::new();
        for (idx, (_, col_type, _, unique, _)) in columns.iter().enumerate() {
            if *unique && idx != pk_index && matches!(col_type, ColumnType::Int) {
                let bp = self.buffer_pool.clone();
                let catalog = self.catalog.clone();
                let table_name = name.to_string();
                let ordinal = unique_indexes.len();
                let uindex = Arc::new(
                    tokio::task::spawn_blocking(move || IndexManager::new(bp))
                        .await??
                        .with_catalog_context(
                            catalog,
                            CatalogRootSlot::Unique {
                                table: table_name,
                                ordinal,
                            },
                        ),
                );
                unique_roots.push(uindex.root_page_id().0 as u32);
                unique_indexes.push((idx, uindex));
            }
        }

        // --- build TableMeta ---
        // MS23: TableMeta 携带 per-column NOT NULL 标志（与 columns 列序对
        // 齐），执行器写路径据此强制；UNIQUE 经 unique_indexes 在写路径强制。
        let schema_cols: Vec<(String, ColumnType)> = columns
            .iter()
            .map(|(col_name, col_type, _, _, _)| (col_name.clone(), col_type.clone()))
            .collect();
        let not_null_flags: Vec<bool> = columns
            .iter()
            .map(|(_, _, not_null, _, _)| *not_null)
            .collect();
        // MS24 Iteration 000 (D1): per-column declared DEFAULT literals.
        let defaults: Vec<Option<Value>> = columns
            .iter()
            .map(|(_, _, _, _, default)| default.clone())
            .collect();
        let table_meta = Arc::new(TableMeta {
            name: name.to_string(),
            columns: schema_cols,
            not_null: not_null_flags,
            defaults,
            pk_column: pk.to_string(),
            pk_index,
            index_manager,
            unique_indexes,
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
            // MS23 Iteration 001 (2.1/2.3): roots of the per-column UNIQUE
            // indexes created above, in ascending qualifying-column order.
            unique_roots,
        };
        let catalog_cols: Vec<CatalogColumnRow> = columns
            .iter()
            .enumerate()
            .map(
                |(idx, (col_name, col_type, not_null, unique, default))| CatalogColumnRow {
                    table_name: name.to_string(),
                    column_index: idx as u32,
                    column_name: col_name.clone(),
                    column_type: col_type.clone(),
                    not_null: *not_null,
                    unique: *unique,
                    // MS24 Iteration 000 (D1): declared DEFAULT literal.
                    default_value: default.clone(),
                },
            )
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

    /// MS10-T02 Iter000 004-rework (R-T0b-R8, D10): swap in the
    /// recovery-rebuilt index manager for a restored table, returning the
    /// pre-rebuild instance so the caller can release its (possibly torn)
    /// pages. The rebuilt `TableMeta` inherits every other field, including
    /// the current in-memory `data_page_tail`. Must run BEFORE
    /// `attach_index_catalog_contexts` (called from `Database::open` after
    /// `full_recover` returns), so the rebuilt instance receives the catalog
    /// root-sync context.
    pub async fn replace_index_manager(
        &self,
        name: &str,
        new_index: Arc<IndexManager>,
    ) -> Result<Arc<IndexManager>> {
        let mut tables = self.tables.write().await;
        let old = tables
            .get(name)
            .ok_or_else(|| StorageError::TableNotFound(name.to_string()))?;
        let old_index = old.index_manager.clone();
        let data_page_tail = *old.data_page_tail.lock().unwrap();
        let meta = Arc::new(TableMeta {
            name: old.name.clone(),
            columns: old.columns.clone(),
            not_null: old.not_null.clone(),
            // MS24 Iteration 000 (D1): DEFAULT literals ride the TableMeta
            // rebuild channel (inherited; opened tables read them from the
            // catalog rows).
            defaults: old.defaults.clone(),
            pk_column: old.pk_column.clone(),
            pk_index: old.pk_index,
            index_manager: new_index,
            // MS23 Iteration 001 (2.3): UNIQUE trees are untouched by a PK
            // swap (recovery swaps them through `replace_recovery_indexes`).
            unique_indexes: old.unique_indexes.clone(),
            data_page_head: old.data_page_head,
            data_page_tail: Mutex::new(data_page_tail),
        });
        tables.insert(name.to_string(), meta);
        Ok(old_index)
    }

    /// MS23 Iteration 001 (2.8/D9): recovery-time whole-table index swap —
    /// replaces the PK index AND every UNIQUE index in one step, returning
    /// the previous instances so the caller can hole-tolerantly release
    /// their (possibly torn) pages. The rebuilt `TableMeta` inherits every
    /// other field, including the current in-memory `data_page_tail`. Must
    /// run BEFORE `attach_index_catalog_contexts` (same precondition as
    /// `replace_index_manager`), so the rebuilt instances receive the
    /// catalog root-sync contexts.
    pub async fn replace_recovery_indexes(
        &self,
        name: &str,
        new_index: Arc<IndexManager>,
        new_uniques: Vec<(usize, Arc<IndexManager>)>,
    ) -> Result<(
        Arc<IndexManager>,
        Vec<(usize, Arc<IndexManager>)>,
    )> {
        let mut tables = self.tables.write().await;
        let old = tables
            .get(name)
            .ok_or_else(|| StorageError::TableNotFound(name.to_string()))?;
        let old_index = old.index_manager.clone();
        let old_uniques = old.unique_indexes.clone();
        let data_page_tail = *old.data_page_tail.lock().unwrap();
        let meta = Arc::new(TableMeta {
            name: old.name.clone(),
            columns: old.columns.clone(),
            not_null: old.not_null.clone(),
            // MS24 Iteration 000 (D1): inherited (same channel as above).
            defaults: old.defaults.clone(),
            pk_column: old.pk_column.clone(),
            pk_index: old.pk_index,
            index_manager: new_index,
            unique_indexes: new_uniques,
            data_page_head: old.data_page_head,
            data_page_tail: Mutex::new(data_page_tail),
        });
        tables.insert(name.to_string(), meta);
        Ok((old_index, old_uniques))
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
        let mut index_pages = match table_meta.index_manager.collect_all_pages().await {
            Ok(pages) => pages,
            Err(e) => {
                eprintln!("[drop_table] collect_all_pages({}) failed: {}", name, e);
                Vec::new()
            }
        };
        // MS23 Iteration 001 (D10): release the per-column UNIQUE index
        // trees too — same best-effort policy as the PK tree (collect
        // failure warns and gives up, free failures warn per page).
        for (_, uindex) in &table_meta.unique_indexes {
            match uindex.collect_all_pages().await {
                Ok(pages) => index_pages.extend(pages),
                Err(e) => {
                    eprintln!(
                        "[drop_table] unique index collect_all_pages({}) failed: {}",
                        name, e
                    );
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::FileStorage;
    use tempfile::tempdir;

    /// MS10-T05 Iter000 001-rework (T5-R1): 约束经 SQL 建库链写入 catalog 既有
    /// not_null/unique 字段并经 scan_columns 读回；旧签名委托路径显式 false。
    #[tokio::test]
    async fn create_table_with_constraints_persists_flags() {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
        let pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let tm = TableManager::new(pool, storage).await.unwrap();

        tm.create_table_with_constraints(
            "t",
            vec![
                ("id".to_string(), ColumnType::Int, false, false, None),
                ("name".to_string(), ColumnType::String(255), true, true, None),
            ],
            "id",
        )
        .await
        .unwrap();

        let mut cols = tm.catalog().scan_columns("t").await.unwrap();
        cols.sort_by_key(|c| c.column_index);
        assert_eq!(cols.len(), 2, "both columns persisted: {cols:?}");
        assert!(
            !cols[0].not_null && !cols[0].unique,
            "flagless column must stay false: {cols:?}"
        );
        assert!(cols[1].not_null, "NOT NULL must persist: {cols:?}");
        assert!(cols[1].unique, "UNIQUE must persist: {cols:?}");

        // 旧签名委托：行为与改造前逐字一致（flags 恒 false）
        tm.create_table("plain", vec![("a".to_string(), ColumnType::Int)], "a")
            .await
            .unwrap();
        let cols = tm.catalog().scan_columns("plain").await.unwrap();
        assert!(
            !cols[0].not_null && !cols[0].unique,
            "legacy create_table must keep flags false: {cols:?}"
        );
    }

    /// MS23-T02 (1.3): NOT NULL 标志经 TableMeta 抵达运行时——create 面直接
    /// 携带，open_or_init 面自 catalog 读回，两构造路径标志一致。
    #[tokio::test]
    async fn table_meta_carries_not_null_flags_across_reopen() {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
        let pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let tm = TableManager::new(pool.clone(), storage.clone()).await.unwrap();

        tm.create_table_with_constraints(
            "t",
            vec![
                ("id".to_string(), ColumnType::Int, false, false, None),
                ("name".to_string(), ColumnType::String(255), true, false, None),
            ],
            "id",
        )
        .await
        .unwrap();

        // 创建面：TableMeta 按列序直接携带标志
        let meta = tm.get_table("t").await.unwrap();
        assert_eq!(meta.not_null, vec![false, true], "create path must carry flags");

        // 恢复面：新 TableManager 经 open_or_init 自 catalog 读回
        let tm2 = TableManager::new(pool, storage).await.unwrap();
        tm2.open_or_init().await.unwrap();
        let meta2 = tm2.get_table("t").await.unwrap();
        assert_eq!(meta2.not_null, vec![false, true], "reopen path must read back flags");
    }

    // ===========================================================================
    // MS23 Iteration 001 (2.3): TableMeta 唯一索引承载与生命周期
    // ===========================================================================

    /// (a) create 面：INT UNIQUE 非 PK 列获得专属唯一索引，绑定列序号正确。
    #[tokio::test]
    async fn create_table_builds_unique_index_for_int_unique_column() {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
        let pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let tm = TableManager::new(pool, storage).await.unwrap();

        tm.create_table_with_constraints(
            "t",
            vec![
                ("id".to_string(), ColumnType::Int, false, false, None),
                ("code".to_string(), ColumnType::Int, false, true, None),
                ("name".to_string(), ColumnType::String(255), false, false, None),
            ],
            "id",
        )
        .await
        .unwrap();

        let meta = tm.get_table("t").await.unwrap();
        assert_eq!(
            meta.unique_indexes.len(),
            1,
            "exactly one UNIQUE index for column 'code': {:?}",
            meta.unique_indexes.iter().map(|(i, _)| i).collect::<Vec<_>>()
        );
        assert_eq!(
            meta.unique_indexes[0].0, 1,
            "unique index must bind to column ordinal 1 ('code')"
        );
        // PK 列即便声明 unique 也不建第二索引（D6 消费裁定）
        tm.create_table_with_constraints(
            "pk_unique",
            vec![
                ("id".to_string(), ColumnType::Int, false, true, None),
                ("v".to_string(), ColumnType::Int, false, false, None),
            ],
            "id",
        )
        .await
        .unwrap();
        let meta = tm.get_table("pk_unique").await.unwrap();
        assert!(
            meta.unique_indexes.is_empty(),
            "PK column UNIQUE must consume PK uniqueness, no second index"
        );
    }

    /// (b) 重开面：新 TableManager 经 open_or_init 自 catalog 根 from_root
    /// 重建唯一索引，根页与创建面一致。
    #[tokio::test]
    async fn reopen_rebuilds_unique_indexes_from_catalog_roots() {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
        let pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let tm = TableManager::new(pool.clone(), storage.clone()).await.unwrap();

        tm.create_table_with_constraints(
            "t",
            vec![
                ("id".to_string(), ColumnType::Int, false, false, None),
                ("code".to_string(), ColumnType::Int, false, true, None),
            ],
            "id",
        )
        .await
        .unwrap();
        let meta1 = tm.get_table("t").await.unwrap();
        assert_eq!(meta1.unique_indexes.len(), 1);
        let created_root = meta1.unique_indexes[0].1.root_page_id();

        let tm2 = TableManager::new(pool, storage).await.unwrap();
        tm2.open_or_init().await.unwrap();
        let meta2 = tm2.get_table("t").await.unwrap();
        assert_eq!(meta2.unique_indexes.len(), 1);
        assert_eq!(
            meta2.unique_indexes[0].0, 1,
            "rebuild must bind to the same column ordinal"
        );
        assert_eq!(
            meta2.unique_indexes[0].1.root_page_id(),
            created_root,
            "rebuild must load the catalog-persisted root"
        );
    }

    /// (c) 非 INT unique 标志列（经 create_table_with_constraints 直呼构造）
    /// 不建索引；重开路径同样静默跳过（R5-S4 兼容边界）。
    #[tokio::test]
    async fn non_int_unique_flag_column_gets_no_index() {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
        let pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let tm = TableManager::new(pool.clone(), storage.clone()).await.unwrap();

        tm.create_table_with_constraints(
            "t",
            vec![
                ("id".to_string(), ColumnType::Int, false, false, None),
                ("name".to_string(), ColumnType::String(255), false, true, None),
            ],
            "id",
        )
        .await
        .unwrap();

        let meta = tm.get_table("t").await.unwrap();
        assert!(
            meta.unique_indexes.is_empty(),
            "non-INT unique flag column must not get an index on create"
        );

        let tm2 = TableManager::new(pool, storage).await.unwrap();
        tm2.open_or_init().await.unwrap();
        let meta2 = tm2.get_table("t").await.unwrap();
        assert!(
            meta2.unique_indexes.is_empty(),
            "non-INT unique flag column must stay unindexed on reopen"
        );
    }

    /// (d) 旧格式 catalog 行（空 unique_roots）+ INT unique 标志：打开成功、
    /// 行为＝无唯一索引（R5-S4：直接打开旧文件可用）。
    #[tokio::test]
    async fn legacy_row_with_int_unique_flag_opens_without_unique_index() {
        use crate::storage::catalog::CatalogColumnRow;

        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
        let pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let tm = TableManager::new(pool.clone(), storage.clone()).await.unwrap();

        // 手工插入旧版形态的 catalog 行：INT unique 标志列、无尾随唯一根段
        let head = pool.storage().allocate_page().await.unwrap();
        let row = CatalogRow {
            table_name: "legacy".to_string(),
            data_page_head: head.0 as u32,
            index_root_page_id: 0,
            pk_index: 0,
            pk_column: "id".to_string(),
            column_count: 2,
            data_page_tail: head.0 as u32,
            unique_roots: Vec::new(),
        };
        let cols = vec![
            CatalogColumnRow {
                table_name: "legacy".to_string(),
                column_index: 0,
                column_name: "id".to_string(),
                column_type: ColumnType::Int,
                not_null: false,
                unique: false,
                default_value: None,
            },
            CatalogColumnRow {
                table_name: "legacy".to_string(),
                column_index: 1,
                column_name: "code".to_string(),
                column_type: ColumnType::Int,
                not_null: false,
                unique: true,
                default_value: None,
            },
        ];
        tm.catalog().insert_table(&row, &cols).await.unwrap();

        tm.open_or_init().await.unwrap();
        let meta = tm.get_table("legacy").await.unwrap();
        assert_eq!(meta.columns.len(), 2);
        assert!(
            meta.unique_indexes.is_empty(),
            "legacy row (empty roots) must open without unique indexes, got {:?}",
            meta.unique_indexes.iter().map(|(i, _)| i).collect::<Vec<_>>()
        );
    }

    // ===========================================================================
    // MS24 Iteration 000 (1.4/D1): DEFAULT 持久化承载与重开读回
    // ===========================================================================

    /// create 面：TableMeta.defaults 按列序携带声明 DEFAULT（含 DEFAULT NULL
    /// 保真与无 DEFAULT 的 None）；重开面：新 TableManager 经 open_or_init 自
    /// catalog 列行 DEFAULT 尾段读回，两构造路径一致。
    #[tokio::test]
    async fn table_meta_carries_defaults_across_reopen() {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
        let pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let tm = TableManager::new(pool.clone(), storage.clone()).await.unwrap();

        tm.create_table_with_constraints(
            "t",
            vec![
                ("id".to_string(), ColumnType::Int, false, false, None),
                ("score".to_string(), ColumnType::Int, false, false, Some(Value::Int(90))),
                ("note".to_string(), ColumnType::String(255), false, false, Some(Value::Null)),
            ],
            "id",
        )
        .await
        .unwrap();

        // 创建面：defaults 与列序对齐
        let meta = tm.get_table("t").await.unwrap();
        assert_eq!(
            meta.defaults,
            vec![None, Some(Value::Int(90)), Some(Value::Null)],
            "create path must carry declared defaults"
        );

        // 恢复面：重开读回（DEFAULT NULL 保真为 Some(Null)，非 None）
        let tm2 = TableManager::new(pool, storage).await.unwrap();
        tm2.open_or_init().await.unwrap();
        let meta2 = tm2.get_table("t").await.unwrap();
        assert_eq!(
            meta2.defaults,
            vec![None, Some(Value::Int(90)), Some(Value::Null)],
            "reopen path must read defaults back from catalog rows"
        );
    }
}
