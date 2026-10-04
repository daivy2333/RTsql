use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

use crate::executor::Value;
use crate::storage::btree::{CatalogRootSlot, IndexManager};
use crate::storage::catalog::{
    Catalog, CatalogColumnRow, CatalogKind, CatalogRow, CatalogSecondaryIndex, COLUMNS_SYSTEM_NAME,
    TABLES_SYSTEM_NAME,
};
use crate::storage::data_page::write_tuple_to_data_page;
use crate::storage::page_format::{overflow, ColumnType, SlottedPage, SlottedPageRef};
use crate::storage::page_id::PageId;
use crate::storage::{
    read_tuple_from_data_page, AsyncStorage, BufferPool, Result, RowId, StorageError,
};
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

/// MS21 Iteration 002 (D4): one user-created secondary index bound to its
/// column ordinal and live B-Tree. `name` is library-level unique; the
/// registration is persisted in the `__tables` row's secondary section
/// (creation order) and rebuilt on open via `from_root`.
#[derive(Clone)]
pub struct SecondaryIndexMeta {
    pub name: String,
    pub column_index: usize,
    pub index: Arc<IndexManager>,
}

/// Iter 002 T9 (change 2026-09-30-catalog-durability-and-cli-surface):
/// read-only data-page chain integrity status for the dump precheck.
/// `terminal` is the page whose `next_page_id` is 0 (`None` when a cyclic
/// `next` was detected — the walk stops there instead of hanging); `tail`
/// is the catalog-persisted chain tail. A healthy chain ends exactly at
/// `tail`: `write_tuple` advances it on every new page, `truncate` resets
/// it in place, empty tables keep `head == tail`.
pub struct DataChainStatus {
    /// Chain terminal page (`next_page_id == 0`); `None` if a cycle was hit.
    pub terminal: Option<PageId>,
    /// `next_page_id` loop detected (chain does not terminate at 0).
    pub cyclic: bool,
    /// Catalog-persisted data page tail.
    pub tail: PageId,
}

impl DataChainStatus {
    /// The chain terminates exactly at the catalog-persisted tail.
    pub fn is_complete(&self) -> bool {
        self.terminal == Some(self.tail)
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
    /// MS21 Iteration 002 (D4): user-created secondary indexes in creation
    /// order — the same order the catalog row persists the secondary
    /// section in. Empty for tables without secondary indexes and for
    /// legacy rows (no trailing catalog section).
    pub secondary_indexes: Vec<SecondaryIndexMeta>,
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
            // MS21 Iteration 002 (D4): restored secondary trees get their slot
            // contexts too — `ordinal` is the tree's position in
            // `secondary_indexes` (creation order, the catalog section order).
            for (ordinal, sindex) in meta.secondary_indexes.iter().enumerate() {
                sindex.index.set_catalog_context(
                    self.catalog.clone(),
                    CatalogRootSlot::Secondary {
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
            // MS21 Iter005 (T5-a/D12): view-definition rows have no data
            // pages and no index trees — building a TableMeta from one would
            // wire zero-page heads into the table domain. Views live in the
            // Database-level registry (see `Database::open_with_key`).
            if row.kind == CatalogKind::View {
                continue;
            }
            let mut cols = self.catalog.scan_columns(&row.table_name).await?;
            // MS21 Iter003 (T3-c/D10): chain-order independence hardening —
            // schema order is the persisted `column_index` order, not the
            // physical chain order (lifecycle `schema`/`dump` sort the same
            // way). ALTER rewrites keep the chain ordered, this closes the
            // dependency for good.
            cols.sort_by_key(|c| c.column_index);
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
                        c.unique && matches!(c.column_type, ColumnType::Int) && *idx != pk_index
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
                    let uindex = Arc::new(IndexManager::from_root(self.buffer_pool.clone(), root)?);
                    rebuilt.push((col_idx, uindex));
                }
                rebuilt
            };

            // MS21 Iteration 002 (D4/T2-a): rebuild the user-created secondary
            // indexes from the catalog row's secondary section (order:
            // creation). A legacy row (no trailing section → empty) opens
            // with no secondary indexes.
            let mut secondary_indexes = Vec::with_capacity(row.secondary_indexes.len());
            for entry in &row.secondary_indexes {
                let root = PageId(entry.root as u64);
                let index = Arc::new(IndexManager::from_root(self.buffer_pool.clone(), root)?);
                secondary_indexes.push(SecondaryIndexMeta {
                    name: entry.name.clone(),
                    column_index: entry.column_index as usize,
                    index,
                });
            }

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
                secondary_indexes,
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

        // change 2026-10-03-practical-core-gaps T1 (R1/D1): initialize the
        // head as a legal empty data page. Previously the freshly allocated
        // page was left all-zero (page type 0x00), so a brand-new zero-row
        // table failed `rtsql check` until the first write lazily init'd the
        // page. Mirrors the truncate_table head-reset precedent below
        // (:1512-1517): init dirties the page and persists on close.
        {
            let guard = self.buffer_pool.get_page(page_id).await?;
            guard.modify_page(|page| {
                let _ = SlottedPage::init(page, 0x03);
            });
            drop(guard);
        }

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
                .with_catalog_context(catalog, CatalogRootSlot::PrimaryKey { table: table_name }),
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
            // MS21 Iteration 002 (D4): a brand-new table has no user-created
            // secondary indexes.
            secondary_indexes: Vec::new(),
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
            // MS21 Iteration 002 (D4): a brand-new table carries no secondary
            // registrations.
            secondary_indexes: Vec::new(),
            // MS21 Iteration 005 (T5-a/D12): the create path writes base
            // tables only (views go through `Catalog::insert_view_row`).
            kind: CatalogKind::Table,
            view_sql: None,
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

    /// MS21 Iter003 (T3-b/D9): ALTER TABLE ADD COLUMN metadata evolution —
    /// persist the new column row and the incremented `column_count`, then
    /// swap in a rebuilt `TableMeta` whose index-aligned vectors (`columns`,
    /// `not_null`, `defaults`) gain the trailing entry. Trees, roots and
    /// page pointers are untouched (key semantics are value-based, and the
    /// column is appended, so every existing ordinal stays put).
    ///
    /// Catalog-first ordering: a failed catalog write leaves the in-memory
    /// meta unchanged (zero side effects, retryable — CTAS/CREATE INDEX
    /// precedent); the in-memory swap itself cannot fail.
    pub async fn alter_add_column(
        &self,
        table: &str,
        column_name: &str,
        column_type: ColumnType,
        not_null: bool,
        default_value: Option<Value>,
    ) -> Result<()> {
        let mut tables = self.tables.write().await;
        let old = tables
            .get(table)
            .ok_or_else(|| StorageError::TableNotFound(table.to_string()))?;

        let col_row = CatalogColumnRow {
            table_name: old.name.clone(),
            column_index: old.columns.len() as u32,
            column_name: column_name.to_string(),
            column_type: column_type.clone(),
            not_null,
            unique: false,
            default_value: default_value.clone(),
        };
        self.catalog.insert_column_row(&col_row).await?;
        self.catalog
            .update_table_column_count(table, (old.columns.len() + 1) as u32)
            .await?;

        let mut columns = old.columns.clone();
        columns.push((column_name.to_string(), column_type));
        let mut not_nulls = old.not_null.clone();
        not_nulls.push(not_null);
        let mut defaults = old.defaults.clone();
        defaults.push(default_value);

        let meta = Arc::new(TableMeta {
            name: old.name.clone(),
            columns,
            not_null: not_nulls,
            defaults,
            pk_column: old.pk_column.clone(),
            pk_index: old.pk_index,
            index_manager: old.index_manager.clone(),
            unique_indexes: old.unique_indexes.clone(),
            secondary_indexes: old.secondary_indexes.clone(),
            data_page_head: old.data_page_head,
            data_page_tail: Mutex::new(*old.data_page_tail.lock().unwrap()),
        });
        tables.insert(table.to_string(), meta);
        Ok(())
    }

    /// MS21 Iter003 (T3-c/D10): ALTER TABLE DROP COLUMN — remove a
    /// non-key column. Crash-consistent ordering (CTAS precedent
    /// generalised, superseding D10's draft ordering per the approved
    /// Risks mitigation ③): catalog rewrite + `flush_all()` FIRST, then
    /// one implicit transaction rewrites every live row DELETE+INSERT,
    /// then commit. Every crash window is then a readable state:
    ///
    /// 1. before catalog rewrite → old schema + old wide rows;
    /// 2. after catalog write, before flush → pages lost on crash → all
    ///    old (WAL holds BeginTxn only — no data records yet);
    /// 3. after flush, during rewrite → replayed tombstones + narrow rows
    ///    against the *persisted* new schema (exact width);
    /// 4. after commit → replay is complete.
    ///
    /// The narrow-row × old-schema mid-row shift (the one form the draft
    /// ordering could produce) never occurs: no data WAL record exists
    /// before the new schema is durable.
    ///
    /// Guard matrix (defense arm — the planner rejects these earlier):
    /// last column / PK / UNIQUE-declared / secondary-indexed column.
    pub async fn alter_drop_column(
        &self,
        table: &str,
        column_name: &str,
        tx_id: u64,
        tx_manager: Arc<crate::transaction::TransactionManager>,
        wal: Option<Arc<crate::wal::WALBuffer>>,
    ) -> Result<()> {
        let old = self.get_table(table).await?;

        // Defense guards (planner rejects earlier; direct construction only)
        let removed = old
            .columns
            .iter()
            .position(|(name, _)| name == column_name)
            .ok_or_else(|| {
                StorageError::Internal(format!(
                    "alter_drop_column: column '{}' not found on table '{}'",
                    column_name, table
                ))
            })?;
        if old.columns.len() == 1 {
            return Err(StorageError::Internal(
                "cannot drop the last column of a table".into(),
            ));
        }
        if old.pk_index == removed {
            return Err(StorageError::Internal(format!(
                "column '{}' is the PRIMARY KEY and cannot be dropped",
                column_name
            )));
        }
        if old.unique_indexes.iter().any(|(idx, _)| *idx == removed) {
            return Err(StorageError::Internal(format!(
                "column '{}' is UNIQUE-declared and cannot be dropped",
                column_name
            )));
        }
        if old
            .secondary_indexes
            .iter()
            .any(|s| s.column_index == removed)
        {
            return Err(StorageError::Internal(format!(
                "column '{}' is referenced by a secondary index and cannot be dropped (drop the index first)",
                column_name
            )));
        }

        // New layout: shifted ordinals for every index-aligned vector. Key
        // trees are value-based — their contents stay untouched, only the
        // ordinal bookkeeping moves.
        let new_columns: Vec<(String, ColumnType)> = old
            .columns
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != removed)
            .map(|(_, c)| c.clone())
            .collect();
        let new_not_null: Vec<bool> = old
            .not_null
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != removed)
            .map(|(_, v)| *v)
            .collect();
        let new_defaults: Vec<Option<Value>> = old
            .defaults
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != removed)
            .map(|(_, v)| v.clone())
            .collect();
        let new_pk_index = if old.pk_index > removed {
            old.pk_index - 1
        } else {
            old.pk_index
        };
        let new_unique_indexes: Vec<(usize, Arc<IndexManager>)> = old
            .unique_indexes
            .iter()
            .filter(|(idx, _)| *idx != removed)
            .map(|(idx, im)| (if *idx > removed { idx - 1 } else { *idx }, im.clone()))
            .collect();
        let new_secondary_indexes: Vec<SecondaryIndexMeta> = old
            .secondary_indexes
            .iter()
            .filter(|s| s.column_index != removed)
            .map(|s| SecondaryIndexMeta {
                name: s.name.clone(),
                column_index: if s.column_index > removed {
                    s.column_index - 1
                } else {
                    s.column_index
                },
                index: s.index.clone(),
            })
            .collect();

        // 1. Catalog rewrite (column row delete + ordinal shift in place,
        // preserving the chain-order invariant) + column_count.
        self.catalog
            .rewrite_column_rows_drop(&old.name, removed as u32)
            .await?;
        self.catalog
            .update_table_column_count(&old.name, new_columns.len() as u32)
            .await?;

        // 2. Durable new schema before any data WAL record (crash window 3).
        self.buffer_pool.flush_all().await?;

        // 3. Swap the in-memory TableMeta to the new layout.
        let new_meta = Arc::new(TableMeta {
            name: old.name.clone(),
            columns: new_columns,
            not_null: new_not_null,
            defaults: new_defaults,
            pk_column: old.pk_column.clone(),
            pk_index: new_pk_index,
            index_manager: old.index_manager.clone(),
            unique_indexes: new_unique_indexes,
            secondary_indexes: new_secondary_indexes,
            data_page_head: old.data_page_head,
            data_page_tail: Mutex::new(*old.data_page_tail.lock().unwrap()),
        });
        self.tables
            .write()
            .await
            .insert(old.name.clone(), new_meta.clone());

        // 4. Implicit-transaction row rewrite: every live (chain-tail,
        // non-tombstone) row is deleted (tombstone + index cleanup + WAL)
        // and re-inserted as the narrowed row (fresh tuple + index rebuild
        // + WAL). All through the same WAL records / version chain /
        // index paths the DML executors use.
        let old_schema: Vec<ColumnType> = old.columns.iter().map(|(_, ct)| ct.clone()).collect();
        let new_schema: Vec<ColumnType> =
            new_meta.columns.iter().map(|(_, ct)| ct.clone()).collect();

        let mut live_rows: Vec<RowId> = Vec::new();
        let mut pointed: HashSet<RowId> = HashSet::new();
        let mut page_id = Some(old.data_page_head);
        while let Some(pid) = page_id {
            let (next_page, rids) = self
                .buffer_pool
                .with_page_data(pid, |data| -> Result<(u32, Vec<RowId>)> {
                    let slotted = SlottedPageRef::new(data);
                    let mut rids = Vec::new();
                    for index in 0..slotted.slot_count() {
                        let Some(slot) = slotted.get_slot(index) else {
                            continue;
                        };
                        let slot_data = slotted.get_slot_data(&slot);
                        if slot_data.len() < VersionHeader::SIZE {
                            continue;
                        }
                        let Some(vh) = VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE])
                        else {
                            continue;
                        };
                        if let Some(target) = vh.next_version() {
                            pointed.insert(target);
                        }
                        if !vh.is_deleted() {
                            rids.push(RowId::new(pid.0 as u32, slot.logical_id));
                        }
                    }
                    Ok((slotted.header().next_page_id, rids))
                })
                .await?;
            live_rows.extend(rids);
            page_id = if next_page == 0 {
                None
            } else {
                Some(PageId(next_page as u64))
            };
        }
        live_rows.retain(|rid| !pointed.contains(rid));

        for rid in &live_rows {
            // Read the old row (width-aware: narrow pre-ADD rows are legal).
            let old_bytes =
                read_tuple_from_data_page(&self.buffer_pool, *rid, |_, b| Ok(b.to_vec())).await?;
            let old_values = crate::storage::page_format::deserialize_row(
                &old_bytes,
                &old_schema,
                &old.defaults,
            )?;
            let old_pk_key = old_values[old.pk_index].to_key();
            let old_unique_keys: Vec<(Arc<IndexManager>, Vec<u8>)> = old
                .unique_indexes
                .iter()
                .filter_map(|(idx, im)| {
                    old_values[*idx]
                        .to_key()
                        .map(|k| (im.clone(), k.as_bytes().to_vec()))
                })
                .collect();
            let old_secondary_keys: Vec<(Arc<IndexManager>, Vec<u8>)> = old
                .secondary_indexes
                .iter()
                .filter_map(|s| {
                    old_values[s.column_index]
                        .to_key()
                        .map(|k| (s.index.clone(), k.as_bytes().to_vec()))
                })
                .collect();

            // DELETE leg: independent tombstone slot (DeleteExecutor shape).
            let tombstone = VersionHeader::new(tx_id, None)
                .with_next_version(*rid)
                .mark_deleted();
            let tombstone_rid =
                write_tuple_to_data_page(&self.buffer_pool, &old, &tombstone, &[]).await?;
            self.buffer_pool
                .clear_all_visible(PageId(tombstone_rid.page_id as u64));
            self.buffer_pool
                .clear_all_visible(PageId(rid.page_id as u64));

            if let Some(key) = &old_pk_key {
                old.index_manager.delete(key.as_bytes()).await?;
            }
            for (uindex, key) in &old_unique_keys {
                uindex.delete(key).await?;
            }
            for (sindex, key) in &old_secondary_keys {
                sindex.delete_exact(key, *rid).await?;
            }

            tx_manager
                .record_version(tx_id, &old.name, tombstone_rid)
                .await;
            if let Some(wal) = &wal {
                wal.append(crate::wal::WalRecord::Delete {
                    tx_id,
                    table_name: old.name.clone(),
                    row_id: *rid,
                })
                .await;
            }

            // INSERT leg: narrowed row (shifted column order), fresh tuple.
            let new_values: Vec<Value> = old_values
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != removed)
                .map(|(_, v)| v.clone())
                .collect();
            let size = crate::storage::page_format::compute_tuple_size(&new_values, &new_schema);
            let mut buf = vec![0u8; size];
            let written =
                crate::storage::page_format::serialize_tuple(&new_values, &new_schema, &mut buf)?;
            buf.truncate(written);

            let vh = VersionHeader::new(tx_id, None);
            let new_rid = self.write_tuple(&new_meta, &vh, &buf).await?;
            self.buffer_pool
                .clear_all_visible(PageId(new_rid.page_id as u64));

            let new_pk_key = new_values[new_meta.pk_index].to_key();
            if let Some(key) = &new_pk_key {
                new_meta
                    .index_manager
                    .insert(key.as_bytes(), new_rid)
                    .await?;
            }
            for (idx, uindex) in &new_meta.unique_indexes {
                if let Some(key) = new_values[*idx].to_key() {
                    uindex.insert(key.as_bytes(), new_rid).await?;
                }
            }
            for sindex in &new_meta.secondary_indexes {
                if let Some(key) = new_values[sindex.column_index].to_key() {
                    sindex.index.insert(key.as_bytes(), new_rid).await?;
                }
            }

            tx_manager
                .record_version(tx_id, &new_meta.name, new_rid)
                .await;
            if let Some(wal) = &wal {
                wal.append(crate::wal::WalRecord::Insert {
                    tx_id,
                    table_name: new_meta.name.clone(),
                    row_id: new_rid,
                    tuple_data: buf,
                })
                .await;
            }
        }

        Ok(())
    }

    /// MS21 Iter003 (T3-b/D9): whether the table holds any live row — the
    /// plan-stage input for the `NOT NULL`-without-DEFAULT rejection. Chain
    /// scan (CREATE INDEX backfill shape): a slot is live when it is a chain
    /// tail (not pointed at by any `next_version`) and not a tombstone,
    /// including active uncommitted versions (rollback cleanup makes those
    /// consistent either way).
    pub async fn table_has_live_rows(&self, table: &str) -> Result<bool> {
        let meta = self.get_table(table).await?;
        let mut pointed: HashSet<RowId> = HashSet::new();
        let mut candidates: Vec<RowId> = Vec::new();
        let mut page_id = Some(meta.data_page_head);
        while let Some(pid) = page_id {
            let (next_page, slots) = self
                .buffer_pool
                .with_page_data(pid, |data| -> Result<(u32, Vec<RowId>)> {
                    let slotted = SlottedPageRef::new(data);
                    let mut slots = Vec::new();
                    for index in 0..slotted.slot_count() {
                        let Some(slot) = slotted.get_slot(index) else {
                            continue;
                        };
                        let slot_data = slotted.get_slot_data(&slot);
                        if slot_data.len() < VersionHeader::SIZE {
                            continue;
                        }
                        let Some(vh) = VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE])
                        else {
                            continue;
                        };
                        if let Some(target) = vh.next_version() {
                            pointed.insert(target);
                        }
                        if !vh.is_deleted() {
                            slots.push(RowId::new(pid.0 as u32, slot.logical_id));
                        }
                    }
                    Ok((slotted.header().next_page_id, slots))
                })
                .await?;
            candidates.extend(slots);
            page_id = if next_page == 0 {
                None
            } else {
                Some(PageId(next_page as u64))
            };
        }
        for rid in candidates {
            if !pointed.contains(&rid) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Iter 002 T9 (change 2026-09-30-catalog-durability-and-cli-surface):
    /// walk the table's data page chain from `data_page_head` and report the
    /// terminal page against the catalog-persisted `data_page_tail`. Read-only
    /// (buffer pool page reads only, no locks or WAL); the `visited` guard
    /// bounds the walk so a cyclic `next_page_id` cannot hang the caller — a
    /// cycle is reported via `cyclic` instead.
    pub async fn data_chain_status(&self, table: &str) -> Result<DataChainStatus> {
        let meta = self.get_table(table).await?;
        let tail = *meta.data_page_tail.lock().unwrap();
        let mut visited: HashSet<PageId> = HashSet::new();
        let mut current = Some(meta.data_page_head);
        let mut terminal = None;
        let mut cyclic = false;
        while let Some(pid) = current {
            if !visited.insert(pid) {
                cyclic = true;
                break;
            }
            let next_page = self
                .buffer_pool
                .with_page_data(pid, |data| -> Result<u32> {
                    Ok(SlottedPageRef::new(data).header().next_page_id)
                })
                .await?;
            current = if next_page == 0 {
                terminal = Some(pid);
                None
            } else {
                Some(PageId(next_page as u64))
            };
        }
        Ok(DataChainStatus {
            terminal,
            cyclic,
            tail,
        })
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
            // MS21 Iteration 002 (D4): secondary trees untouched by a PK swap
            // (same swap channel as above).
            secondary_indexes: old.secondary_indexes.clone(),
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
        new_secondaries: Vec<SecondaryIndexMeta>,
    ) -> Result<(
        Arc<IndexManager>,
        Vec<(usize, Arc<IndexManager>)>,
        Vec<SecondaryIndexMeta>,
    )> {
        let mut tables = self.tables.write().await;
        let old = tables
            .get(name)
            .ok_or_else(|| StorageError::TableNotFound(name.to_string()))?;
        let old_index = old.index_manager.clone();
        let old_uniques = old.unique_indexes.clone();
        let old_secondaries = old.secondary_indexes.clone();
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
            secondary_indexes: new_secondaries,
            data_page_head: old.data_page_head,
            data_page_tail: Mutex::new(data_page_tail),
        });
        tables.insert(name.to_string(), meta);
        Ok((old_index, old_uniques, old_secondaries))
    }

    /// MS21 Iteration 002 (D4/D5, T2-c): register a freshly built and
    /// backfilled secondary index on `table`. Persists the catalog section
    /// entry (name / column / root) and swaps the index into the in-memory
    /// `TableMeta` with its `CatalogRootSlot::Secondary` context attached
    /// (ordinal = position in the registration list). Caller ordering
    /// contract (D5): the tree is created and backfilled FIRST, registration
    /// happens once — a failure before this call leaves zero side effects
    /// and the operation is retryable.
    pub async fn register_secondary_index(
        &self,
        table: &str,
        index_name: &str,
        column_index: usize,
        index: Arc<IndexManager>,
    ) -> Result<()> {
        let root = index.root_page_id().0 as u32;
        self.catalog
            .append_secondary_index(
                table,
                &CatalogSecondaryIndex {
                    name: index_name.to_string(),
                    column_index: column_index as u32,
                    root,
                },
            )
            .await?;

        let mut tables = self.tables.write().await;
        let old = tables
            .get(table)
            .ok_or_else(|| StorageError::TableNotFound(table.to_string()))?;
        let ordinal = old.secondary_indexes.len();
        index.set_catalog_context(
            self.catalog.clone(),
            CatalogRootSlot::Secondary {
                table: table.to_string(),
                ordinal,
            },
        );
        let mut secondary_indexes = old.secondary_indexes.clone();
        secondary_indexes.push(SecondaryIndexMeta {
            name: index_name.to_string(),
            column_index,
            index,
        });
        let meta = Arc::new(TableMeta {
            name: old.name.clone(),
            columns: old.columns.clone(),
            not_null: old.not_null.clone(),
            defaults: old.defaults.clone(),
            pk_column: old.pk_column.clone(),
            pk_index: old.pk_index,
            index_manager: old.index_manager.clone(),
            unique_indexes: old.unique_indexes.clone(),
            secondary_indexes,
            data_page_head: old.data_page_head,
            data_page_tail: Mutex::new(*old.data_page_tail.lock().unwrap()),
        });
        tables.insert(table.to_string(), meta);
        Ok(())
    }

    /// MS21 Iteration 002 (D4, T2-f): remove the named secondary index from
    /// its table — catalog section entry and in-memory registration — and
    /// return `(table_name, column_index, previous tree)` so the caller can
    /// release the tree pages. `Ok(None)` when no index carries that name.
    /// Ordinals of the table's remaining secondary indexes shift down and
    /// their catalog contexts are re-attached accordingly.
    pub async fn take_secondary_index(
        &self,
        index_name: &str,
    ) -> Result<Option<(String, usize, Arc<IndexManager>)>> {
        let holder = {
            let tables = self.tables.read().await;
            tables
                .values()
                .find(|meta| meta.secondary_indexes.iter().any(|s| s.name == index_name))
                .map(|meta| meta.name.clone())
        };
        let Some(table) = holder else {
            return Ok(None);
        };

        self.catalog
            .remove_secondary_index(&table, index_name)
            .await?;

        let mut tables = self.tables.write().await;
        let old = tables
            .get(&table)
            .ok_or_else(|| StorageError::TableNotFound(table.clone()))?;
        let pos = old
            .secondary_indexes
            .iter()
            .position(|s| s.name == index_name)
            .ok_or_else(|| {
                StorageError::Internal(format!(
                    "take_secondary_index: '{}' vanished from table '{}'",
                    index_name, table
                ))
            })?;
        let removed = old.secondary_indexes[pos].clone();
        let mut secondary_indexes = old.secondary_indexes.clone();
        secondary_indexes.remove(pos);
        let meta = Arc::new(TableMeta {
            name: old.name.clone(),
            columns: old.columns.clone(),
            not_null: old.not_null.clone(),
            defaults: old.defaults.clone(),
            pk_column: old.pk_column.clone(),
            pk_index: old.pk_index,
            index_manager: old.index_manager.clone(),
            unique_indexes: old.unique_indexes.clone(),
            data_page_head: old.data_page_head,
            data_page_tail: Mutex::new(*old.data_page_tail.lock().unwrap()),
            secondary_indexes,
        });
        tables.insert(table.clone(), meta.clone());

        // Ordinals after the removed entry shifted down — re-attach their
        // catalog contexts so later root splits persist to the right slot.
        for (ordinal, sindex) in meta.secondary_indexes.iter().enumerate() {
            sindex.index.set_catalog_context(
                self.catalog.clone(),
                CatalogRootSlot::Secondary {
                    table: table.clone(),
                    ordinal,
                },
            );
        }

        Ok(Some((table, removed.column_index, removed.index)))
    }

    /// MS21 Iteration 002 (D14 plan-time wiring): every registered secondary
    /// index name in the library (unsorted). Consumed by the planner's
    /// CREATE INDEX duplicate-name and DROP INDEX existence checks.
    pub async fn all_secondary_index_names(&self) -> Vec<String> {
        let tables = self.tables.read().await;
        tables
            .values()
            .flat_map(|meta| meta.secondary_indexes.iter().map(|s| s.name.clone()))
            .collect()
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
        // MS21 Iteration 002 (D4): release the user-created secondary index
        // trees with the same best-effort policy.
        for sindex in &table_meta.secondary_indexes {
            match sindex.index.collect_all_pages().await {
                Ok(pages) => index_pages.extend(pages),
                Err(e) => {
                    eprintln!(
                        "[drop_table] secondary index collect_all_pages({}) failed: {}",
                        name, e
                    );
                }
            }
        }
        let data_pages = self.collect_data_pages(table_meta.data_page_head).await;

        // MS27 T6 (design D5): overflow chain pages referenced by slot
        // locators die with the table — collect them from the data pages
        // (header-only slot reads; chains never join the data chain) and
        // free them with everything else, best-effort.
        let overflow_pages = self.collect_overflow_chain_pages(&data_pages).await;

        for page in index_pages
            .into_iter()
            .chain(overflow_pages)
            .chain(data_pages)
        {
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

    /// MS27 T6 (design D5): scan the given data pages' slots for overflow
    /// locators and collect every referenced chain page id (header-only slot
    /// reads — chains are independent pages, never in `next_page_id` links).
    ///
    /// Best-effort, same policy as `collect_data_pages`: a data page read or
    /// a chain walk error warns and skips that page/chain, returning what was
    /// collected so far.
    async fn collect_overflow_chain_pages(&self, data_pages: &[PageId]) -> Vec<PageId> {
        let mut chain_heads: Vec<u32> = Vec::new();
        for &pid in data_pages {
            let heads: Vec<u32> = match self
                .buffer_pool
                .with_page_data(pid, |data| {
                    let slotted = SlottedPageRef::new(data);
                    let mut heads = Vec::new();
                    for index in 0..slotted.slot_count() {
                        let Some(slot) = slotted.get_slot(index) else {
                            continue;
                        };
                        let slot_data = slotted.get_slot_data(&slot);
                        if slot_data.len() < VersionHeader::SIZE {
                            continue;
                        }
                        if let Some((_, first)) =
                            overflow::decode_locator(&slot_data[VersionHeader::SIZE..])
                        {
                            if first != 0 {
                                heads.push(first);
                            }
                        }
                    }
                    Ok(heads)
                })
                .await
            {
                Ok(heads) => heads,
                Err(e) => {
                    eprintln!(
                        "[overflow-release] scan data page {:?} for locators failed: {}",
                        pid, e
                    );
                    continue;
                }
            };
            chain_heads.extend(heads);
        }

        let mut pages = Vec::new();
        for first in chain_heads {
            match overflow::collect_overflow_pages(&self.buffer_pool, first).await {
                Ok(chain) => {
                    pages.extend(chain.into_iter().map(|raw| PageId(raw as u64)));
                }
                Err(e) => {
                    eprintln!(
                        "[overflow-release] collect chain from page {first} failed: {}",
                        e
                    );
                }
            }
        }
        pages
    }

    /// TRUNCATE a table: physically free all data pages, keep the definition.
    ///
    /// MS21 Iteration 004 (T4-a, design D11): the caller (TruncateExecutor)
    /// runs `Database::checkpoint()` FIRST, so the WAL holds no pre-TRUNCATE
    /// DML replay records when the physical clearing starts — replayed rows
    /// cannot resurrect into the cleared pages.
    ///
    /// Ordering (crash-window analysis, D11):
    /// 1. tail data pages are freed to the storage free-list (zeroed on disk
    ///    immediately by `FileStorage::free_page`), the head page is reset in
    ///    place as an empty SlottedPage;
    /// 2. the in-memory tail and the catalog `data_page_tail` return to the
    ///    head page;
    /// 3. fresh empty trees replace the PK / UNIQUE / secondary indexes and
    ///    their roots are written back to the catalog row;
    /// 4. old index trees are released best-effort (drop_table policy);
    /// 5. `flush_all()` persists everything — the statement returns durable.
    ///
    /// A crash between step 1 and step 5 leaves the documented "partially
    /// cleared" DDL state (some tail rows gone, head rows still visible,
    /// index roots still pointing at entries on zeroed pages) — the same
    /// honest class as drop_table's window; after step 5 the full TRUNCATE
    /// state is durable. No "resurrected cleared rows" form exists (the WAL
    /// was truncated by the checkpoint before any page was touched).
    ///
    /// TRUNCATE never writes row-level traces: no tombstones, no row-level
    /// WAL records, no version-chain touches — DELETE's MVCC semantics do
    /// not apply (physical semantics).
    pub async fn truncate_table(&self, name: &str) -> Result<()> {
        // Reserved-name guard: never allow truncating system tables.
        if name == TABLES_SYSTEM_NAME || name == COLUMNS_SYSTEM_NAME {
            return Err(StorageError::ReservedTableName(name.to_string()));
        }
        let table_meta = self.get_table(name).await?;
        let head = table_meta.data_page_head;

        // 1. Free every data page after the head; reset the head in place.
        //    MS27 T6 (design D5): overflow chain pages referenced by any
        //    slot (head included) are collected first and freed before the
        //    data pages go — best-effort, same policy as the page frees.
        let data_pages = self.collect_data_pages(head).await;
        for page in self.collect_overflow_chain_pages(&data_pages).await {
            if let Err(e) = self.buffer_pool.free_page(page).await {
                eprintln!(
                    "[truncate_table] free overflow page({}) failed: {}",
                    page.0, e
                );
            }
        }
        for page in data_pages.into_iter().skip(1) {
            if let Err(e) = self.buffer_pool.free_page(page).await {
                eprintln!("[truncate_table] free_page({}) failed: {}", page.0, e);
            }
        }
        let guard = self.buffer_pool.get_page(head).await?;
        guard.modify_page(|page| {
            let _ = SlottedPage::init(page, 0x03);
        });
        drop(guard);

        // 2. Tail returns to the head page — in-memory first (the swap below
        //    inherits it into the rebuilt TableMeta), then the catalog row.
        *table_meta.data_page_tail.lock().unwrap() = head;
        self.catalog.update_table_tail(name, head.0 as u32).await?;

        // 3. Empty trees for all three index classes; catalog-first root
        //    write-back, then the whole-table swap (a failed catalog write
        //    leaves the in-memory meta unchanged — CTAS/CREATE INDEX
        //    zero-side-effect discipline). On any failure the freshly built
        //    trees are released best-effort before propagating.
        let new_pk = self
            .new_empty_tree(CatalogRootSlot::PrimaryKey {
                table: name.to_string(),
            })
            .await?;
        let mut new_uniques: Vec<(usize, Arc<IndexManager>)> = Vec::new();
        let mut new_secondaries: Vec<SecondaryIndexMeta> = Vec::new();
        let build_result: Result<()> = async {
            for (ordinal, (col_idx, _)) in table_meta.unique_indexes.iter().enumerate() {
                let tree = self
                    .new_empty_tree(CatalogRootSlot::Unique {
                        table: name.to_string(),
                        ordinal,
                    })
                    .await?;
                new_uniques.push((*col_idx, tree));
            }
            for (ordinal, s) in table_meta.secondary_indexes.iter().enumerate() {
                let tree = self
                    .new_empty_tree(CatalogRootSlot::Secondary {
                        table: name.to_string(),
                        ordinal,
                    })
                    .await?;
                new_secondaries.push(SecondaryIndexMeta {
                    name: s.name.clone(),
                    column_index: s.column_index,
                    index: tree,
                });
            }
            // Roots to the catalog row (PK + per-ordinal UNIQUE/secondary).
            self.catalog
                .update_table_root(name, new_pk.root_page_id().0 as u32)
                .await?;
            for (ordinal, (_, tree)) in new_uniques.iter().enumerate() {
                self.catalog
                    .update_unique_index_root(name, ordinal, tree.root_page_id().0 as u32)
                    .await?;
            }
            for (ordinal, s) in new_secondaries.iter().enumerate() {
                self.catalog
                    .update_secondary_index_root(name, ordinal, s.index.root_page_id().0 as u32)
                    .await?;
            }
            Ok(())
        }
        .await;
        if let Err(e) = build_result {
            for tree in std::iter::once(&new_pk)
                .chain(new_uniques.iter().map(|(_, t)| t))
                .chain(new_secondaries.iter().map(|s| &s.index))
            {
                for page in tree.collect_all_pages_tolerant().await {
                    let _ = self.buffer_pool.free_page(page).await;
                }
            }
            return Err(e);
        }
        let (old_pk, old_uniques, old_secondaries) = self
            .replace_recovery_indexes(name, new_pk, new_uniques, new_secondaries)
            .await?;

        // 4. Old trees best-effort release (drop_table policy: collect
        //    failure warns and gives up, free failures warn per page).
        for tree in std::iter::once(old_pk)
            .chain(old_uniques.into_iter().map(|(_, t)| t))
            .chain(old_secondaries.into_iter().map(|s| s.index))
        {
            match tree.collect_all_pages().await {
                Ok(pages) => {
                    for page in pages {
                        if let Err(e) = self.buffer_pool.free_page(page).await {
                            eprintln!("[truncate_table] free_page({}) failed: {}", page.0, e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[truncate_table] collect_all_pages({}) failed: {}", name, e);
                }
            }
        }

        // 5. Statement-return-is-durable: persist the head reset and every
        //    catalog rewrite (alter_drop_column precedent).
        self.buffer_pool.flush_all().await?;

        Ok(())
    }

    /// Build a fresh empty B-Tree index bound to a catalog root slot
    /// (`spawn_blocking`: `IndexManager::new` blocks internally).
    async fn new_empty_tree(&self, slot: CatalogRootSlot) -> Result<Arc<IndexManager>> {
        let bp = self.buffer_pool.clone();
        let catalog = self.catalog.clone();
        Ok(Arc::new(
            tokio::task::spawn_blocking(move || IndexManager::new(bp))
                .await??
                .with_catalog_context(catalog, slot),
        ))
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
        let tail = *table_meta.data_page_tail.lock().unwrap();
        // change 2026-10-03-recovery-tail-persistence T2：内存 tail 陈旧
        // （落后链 terminal）时先自愈再追加。
        let old_tail = self.heal_stale_tail(table_meta, tail).await?;
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

    /// change 2026-10-03-recovery-tail-persistence (T2/R2)：写路径陈旧 tail
    /// 自愈。tail 页 `next != 0` = tail 落后链 terminal（存量库恢复期形态）：
    /// 沿链前进到 terminal，同步内存与持久 tail 后以 terminal 为追加目标——
    /// 链中既有 `next` 指针不被覆写，已恢复页行保持可达。健康库（`next == 0`）
    /// 单次 header 探测即返回，无链行走查；异常（页越界、环）按既有
    /// `StorageError` 面显式传播，不静默。不得进入恢复面复用的
    /// `write_tuple_to_data_page`（重放期 `next != 0` 是链重建中间态）。
    async fn heal_stale_tail(&self, table_meta: &Arc<TableMeta>, tail: PageId) -> Result<PageId> {
        let page_count = self.buffer_pool.storage().page_count();
        let mut visited: HashSet<PageId> = HashSet::new();
        let mut current = tail;
        let terminal = loop {
            if current.0 >= page_count {
                return Err(StorageError::InvalidPageId(current.0));
            }
            if !visited.insert(current) {
                return Err(StorageError::Internal(format!(
                    "data page chain cycle while healing stale tail of table '{}'",
                    table_meta.name
                )));
            }
            let next = self
                .buffer_pool
                .with_page_data(current, |data| {
                    Ok(SlottedPageRef::new(data).header().next_page_id)
                })
                .await?;
            if next == 0 {
                break current;
            }
            current = PageId(next as u64);
        };
        if terminal == tail {
            return Ok(tail);
        }
        *table_meta.data_page_tail.lock().unwrap() = terminal;
        self.catalog
            .update_table_tail(&table_meta.name, terminal.0 as u32)
            .await?;
        Ok(terminal)
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
                (
                    "name".to_string(),
                    ColumnType::String(255),
                    true,
                    true,
                    None,
                ),
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
        let tm = TableManager::new(pool.clone(), storage.clone())
            .await
            .unwrap();

        tm.create_table_with_constraints(
            "t",
            vec![
                ("id".to_string(), ColumnType::Int, false, false, None),
                (
                    "name".to_string(),
                    ColumnType::String(255),
                    true,
                    false,
                    None,
                ),
            ],
            "id",
        )
        .await
        .unwrap();

        // 创建面：TableMeta 按列序直接携带标志
        let meta = tm.get_table("t").await.unwrap();
        assert_eq!(
            meta.not_null,
            vec![false, true],
            "create path must carry flags"
        );

        // 恢复面：新 TableManager 经 open_or_init 自 catalog 读回
        let tm2 = TableManager::new(pool, storage).await.unwrap();
        tm2.open_or_init().await.unwrap();
        let meta2 = tm2.get_table("t").await.unwrap();
        assert_eq!(
            meta2.not_null,
            vec![false, true],
            "reopen path must read back flags"
        );
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
                (
                    "name".to_string(),
                    ColumnType::String(255),
                    false,
                    false,
                    None,
                ),
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
            meta.unique_indexes
                .iter()
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
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
        let tm = TableManager::new(pool.clone(), storage.clone())
            .await
            .unwrap();

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
        let tm = TableManager::new(pool.clone(), storage.clone())
            .await
            .unwrap();

        tm.create_table_with_constraints(
            "t",
            vec![
                ("id".to_string(), ColumnType::Int, false, false, None),
                (
                    "name".to_string(),
                    ColumnType::String(255),
                    false,
                    true,
                    None,
                ),
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
        let tm = TableManager::new(pool.clone(), storage.clone())
            .await
            .unwrap();

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
            secondary_indexes: Vec::new(),
            kind: CatalogKind::Table,
            view_sql: None,
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
            meta.unique_indexes
                .iter()
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
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
        let tm = TableManager::new(pool.clone(), storage.clone())
            .await
            .unwrap();

        tm.create_table_with_constraints(
            "t",
            vec![
                ("id".to_string(), ColumnType::Int, false, false, None),
                (
                    "score".to_string(),
                    ColumnType::Int,
                    false,
                    false,
                    Some(Value::Int(90)),
                ),
                (
                    "note".to_string(),
                    ColumnType::String(255),
                    false,
                    false,
                    Some(Value::Null),
                ),
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

    // ===========================================================================
    // MS21 Iteration 002 (T2-a/D4): TableMeta 二级索引承载与生命周期
    // ===========================================================================

    /// create 面：新表无二级索引；register 面：登记后 TableMeta 携带（名称/
    /// 列位/树），catalog 段同步；重开面：from_root 重建，根页与登记一致；
    /// take 面：按名移除后余者 ordinal 平移，再移除不存在的名返回 None。
    #[tokio::test]
    async fn secondary_index_registration_survives_reopen_and_take() {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
        let pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let tm = TableManager::new(pool.clone(), storage.clone())
            .await
            .unwrap();

        tm.create_table(
            "t",
            vec![
                ("id".to_string(), ColumnType::Int),
                ("age".to_string(), ColumnType::Int),
            ],
            "id",
        )
        .await
        .unwrap();
        let meta = tm.get_table("t").await.unwrap();
        assert!(
            meta.secondary_indexes.is_empty(),
            "new table: no secondaries"
        );

        // 登记两个二级索引（模拟 CREATE INDEX 执行器的建树+登记序）
        let bp = pool.clone();
        let tree1 = Arc::new(
            tokio::task::spawn_blocking(move || IndexManager::new(bp))
                .await
                .unwrap()
                .unwrap(),
        );
        let root1 = tree1.root_page_id();
        tm.register_secondary_index("t", "idx_age", 1, tree1.clone())
            .await
            .unwrap();

        let bp = pool.clone();
        let tree2 = Arc::new(
            tokio::task::spawn_blocking(move || IndexManager::new(bp))
                .await
                .unwrap()
                .unwrap(),
        );
        let root2 = tree2.root_page_id();
        tm.register_secondary_index("t", "idx_city", 1, tree2.clone())
            .await
            .unwrap();

        let meta = tm.get_table("t").await.unwrap();
        assert_eq!(meta.secondary_indexes.len(), 2);
        assert_eq!(meta.secondary_indexes[0].name, "idx_age");
        assert_eq!(meta.secondary_indexes[0].column_index, 1);
        assert_eq!(meta.secondary_indexes[0].index.root_page_id(), root1);

        // 库名清单
        let mut names = tm.all_secondary_index_names().await;
        names.sort();
        assert_eq!(names, vec!["idx_age".to_string(), "idx_city".to_string()]);

        // 重开面：from_root 重建，根页与创建面一致
        let tm2 = TableManager::new(pool.clone(), storage.clone())
            .await
            .unwrap();
        tm2.open_or_init().await.unwrap();
        let meta2 = tm2.get_table("t").await.unwrap();
        assert_eq!(meta2.secondary_indexes.len(), 2);
        assert_eq!(meta2.secondary_indexes[0].name, "idx_age");
        assert_eq!(
            meta2.secondary_indexes[0].index.root_page_id(),
            root1,
            "rebuild must load the catalog-persisted root"
        );
        assert_eq!(meta2.secondary_indexes[1].index.root_page_id(), root2);

        // take 面：按名移除，返回被移除树；余者保留
        let (table, col, old_tree) = tm.take_secondary_index("idx_age").await.unwrap().unwrap();
        assert_eq!(table, "t");
        assert_eq!(col, 1);
        assert_eq!(old_tree.root_page_id(), root1);
        let meta = tm.get_table("t").await.unwrap();
        assert_eq!(meta.secondary_indexes.len(), 1);
        assert_eq!(meta.secondary_indexes[0].name, "idx_city");

        assert!(
            tm.take_secondary_index("idx_age").await.unwrap().is_none(),
            "removing a taken index again must be Ok(None)"
        );

        // 删除跨重启持久：catalog 段只剩 idx_city
        let tm3 = TableManager::new(pool, storage).await.unwrap();
        tm3.open_or_init().await.unwrap();
        let meta3 = tm3.get_table("t").await.unwrap();
        assert_eq!(meta3.secondary_indexes.len(), 1);
        assert_eq!(meta3.secondary_indexes[0].name, "idx_city");
    }

    // ===========================================================================
    // change 2026-10-03-recovery-tail-persistence (T2/R2): 写路径陈旧 tail 自愈
    // ===========================================================================

    /// 沿链统计存活行（VersionHeader 未携带删除标记），用于全链可达断言。
    async fn count_live_rows(tm: &TableManager, meta: &Arc<TableMeta>) -> usize {
        let mut visited: HashSet<PageId> = HashSet::new();
        let mut current = Some(meta.data_page_head);
        let mut count = 0usize;
        while let Some(pid) = current {
            assert!(visited.insert(pid), "chain cycle during row count");
            let (next, live) = tm
                .buffer_pool
                .with_page_data(pid, |data| {
                    let sp = SlottedPageRef::new(data);
                    let mut live = 0usize;
                    for i in 0..sp.slot_count() {
                        if let Some(slot) = sp.get_slot(i) {
                            if let Some(vh) = VersionHeader::from_bytes(sp.get_slot_data(&slot)) {
                                if !vh.is_deleted() {
                                    live += 1;
                                }
                            }
                        }
                    }
                    Ok((sp.header().next_page_id, live))
                })
                .await
                .unwrap();
            count += live;
            current = if next == 0 {
                None
            } else {
                Some(PageId(next as u64))
            };
        }
        count
    }

    /// (T2/R2)：内存/持久 tail 陈旧（落后链 terminal）时，write_tuple 先沿链
    /// 自愈到 terminal 并持久化修正值再照常追加——存量库首次写入不再覆写链中
    /// next 指针，陈旧 tail 与 terminal 之间已恢复页的行保持可达。
    #[tokio::test]
    async fn stale_tail_heals_on_write() {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
        let pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let tm = TableManager::new(pool, storage).await.unwrap();

        tm.create_table("t", vec![("id".to_string(), ColumnType::Int)], "id")
            .await
            .unwrap();
        let meta = tm.get_table("t").await.unwrap();

        // 跨页写入：tuple = 9B INT + 4033B STRING = 4042B、slot_data = 22 +
        // 4042 = 4064B → 每页恰一行（数据区 4080B），3 行后链跨 3 页。
        let vh = VersionHeader::new(1, None);
        let payload = vec![0x61u8; 4030];
        for _ in 0..3 {
            tm.write_tuple(&meta, &vh, &payload).await.unwrap();
        }
        let head = meta.data_page_head;
        assert!(
            meta.data_page_tail.lock().unwrap().0 >= head.0 + 2,
            "fixture must span >= 3 data pages"
        );

        // 模拟存量陈旧形态：内存与 catalog tail 一并回拨到 head（head.next ≠ 0）。
        *meta.data_page_tail.lock().unwrap() = head;
        tm.catalog()
            .update_table_tail("t", head.0 as u32)
            .await
            .unwrap();

        // 单次 write_tuple：自愈至 terminal 后照常追加，已恢复页行不丢失。
        tm.write_tuple(&meta, &vh, &payload).await.unwrap();

        let status = tm.data_chain_status("t").await.unwrap();
        let mem_tail = *meta.data_page_tail.lock().unwrap();
        assert_eq!(
            mem_tail,
            status.terminal.unwrap(),
            "memory tail must equal the chain terminal after heal"
        );
        let catalog_tail = tm.catalog().scan_tables().await.unwrap()[0].data_page_tail;
        assert_eq!(
            mem_tail.0 as u32, catalog_tail,
            "catalog tail must be healed to the chain terminal"
        );
        assert_eq!(
            count_live_rows(&tm, &meta).await,
            4,
            "rows on healed pages plus the new row must all be reachable"
        );
    }
}
