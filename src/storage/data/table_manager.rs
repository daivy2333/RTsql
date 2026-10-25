use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

use crate::executor::Value;
use crate::storage::btree::{CatalogRootSlot, IndexManager, LEAF_NODE};
use crate::storage::catalog::{
    Catalog, CatalogColumnRow, CatalogKind, CatalogRow, CatalogSecondaryIndex, COLUMNS_PAGE_ID,
    COLUMNS_SYSTEM_NAME, TABLES_PAGE_ID, TABLES_SYSTEM_NAME,
};
use crate::storage::data_page::update_version_header_in_data_page;
use crate::storage::data_page::write_tuple_to_data_page;
use crate::storage::free_page_rebuild::take_free_page_rebuild_fault;
use crate::storage::integrity::{check_data_page, check_tree_page, violation_summary};
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

/// change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
/// Iteration 003（design D7 / ISS25）：每表共享的「在途窄化」状态单元。
///
/// 状态与**表**绑定，不与某个 `TableMeta` 版本绑定——同一张表的任何活布局
/// 版本都指向**同一**单元（见 `TableMeta::narrowing`）。读者持有的是计划期
/// 快照（旧 `TableMeta`），因此单元必须随活布局一起被读者观察到，否则窗口
/// 内的拒绝对计划期快照失效。
///
/// 纯内存状态：不持久化、不进 WAL / 目录 ⇒ 磁盘格式不变（MS36 的 G1/G2/G3
/// 兼容面不受影响）。单元不携带任何运行身份（无 hash / 指纹 / revision /
/// run-id / 时间戳）。
#[derive(Debug)]
pub struct NarrowingState {
    /// 该单元所属的表名——拒绝信息据此点名表。
    table: String,
    /// 是否正处于「改写已开始、目录窄化尚未生效」的窗口内。
    in_flight: AtomicBool,
}

impl NarrowingState {
    /// 新建一个未发布在途状态的单元。
    pub fn new(table: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            in_flight: AtomicBool::new(false),
        }
    }

    /// 在途窄化中——读者据此拒绝解码。
    fn is_in_flight(&self) -> bool {
        self.in_flight.load(Ordering::SeqCst)
    }

    /// 发布在途窄化（改写开始处）。
    pub fn publish(&self) {
        self.in_flight.store(true, Ordering::SeqCst);
    }

    /// 清除在途窄化（目录窄化生效后 / 语句异常或放弃路径）。
    pub fn clear(&self) {
        self.in_flight.store(false, Ordering::SeqCst);
    }

    /// 读者侧检查：窗口内拒绝解码，点名该表正在被修改。
    ///
    /// 拒绝面按**表**（不是按库 / 全局）：未改写的表不受影响。
    pub fn ensure_readable(&self) -> Result<()> {
        if self.is_in_flight() {
            return Err(StorageError::ExecutionError(format!(
                "table '{}' is being modified by an in-flight ALTER TABLE DROP COLUMN; read refused",
                self.table
            )));
        }
        Ok(())
    }
}

/// change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
/// Iteration 003（design D7 / ISS25）：`ALTER TABLE DROP COLUMN` 的语句作用域
/// 守卫——语句异常或被放弃（语句 future 被丢弃）时清除该表的「在途窄化」状态。
///
/// 守卫在改写开始处（发布点）创建，随 [`PendingColumnDrop`] 一路活到语句结束：
/// 正常路径由 `apply_column_drop` 在目录窄化生效后清除；异常 / 放弃路径由守卫
/// 析构兜底清除 ⇒ 该表不会因失败的 DDL 而永久不可读（那比 ISS25 原缺陷更
/// 严重）。守卫不携带任何运行身份。
pub struct NarrowingGuard {
    state: Arc<NarrowingState>,
}

impl NarrowingGuard {
    fn new(state: Arc<NarrowingState>) -> Self {
        Self { state }
    }
}

impl Drop for NarrowingGuard {
    fn drop(&mut self) {
        // 语句异常 / 被放弃时的兜底清除——正常路径已由 `apply_column_drop`
        // 清除（幂等）。
        self.state.clear();
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
    /// change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
    /// Iteration 003（design D7 / ISS25）：该表的「在途窄化」状态单元。
    ///
    /// 同一张表的任何活布局版本共享**同一**单元——包括 `ALTER TABLE DROP
    /// COLUMN` 产出的窄化副本（`new_meta`）与目录窄化生效后接管活布局的新
    /// 版本 ⇒ 计划期快照（旧 `TableMeta`）也能观察到窗口内状态。
    pub narrowing: Arc<NarrowingState>,
}

/// change 2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog /
/// Iteration 001 (D5, ISS21): a completed `ALTER TABLE DROP COLUMN`'s catalog
/// change, produced by [`TableManager::alter_drop_column`] and applied at the
/// statement's commit point by [`TableManager::apply_column_drop`].
///
/// The narrowing is deliberately NOT part of the statement's transactional
/// body: an abandoned or failed DDL must leave no catalog-side change. The
/// pending change carries everything the application step needs — the table,
/// the dropped column's full catalog definition (with its original ordinal)
/// and the column count the table must advertise afterwards.
pub struct PendingColumnDrop {
    /// The table the drop applies to.
    pub table: String,
    /// The dropped column's catalog definition; `column_index` holds its
    /// original ordinal.
    pub removed_column: CatalogColumnRow,
    /// The column count the table must advertise once the drop is applied.
    pub new_column_count: u32,
    /// The narrowed in-memory layout the data-plane rewrite wrote against.
    pub new_meta: Arc<TableMeta>,
    /// change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
    /// Iteration 003（design D7 / ISS25）：语句作用域守卫——本 pending 值被
    /// 丢弃（语句异常 / 被放弃）时清除在途窄化状态。
    _narrowing_guard: NarrowingGuard,
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
    /// MS35 (`free-page-lifecycle` R2): the persisted directory as read at
    /// open time (before crash recovery). Empty until `open_or_init` runs.
    persisted_catalog_rows: RwLock<Vec<CatalogRow>>,
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
            persisted_catalog_rows: RwLock::new(Vec::new()),
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
        // MS35 (`free-page-lifecycle` R2): keep the persisted directory as
        // read at open time (before crash recovery runs). Recovery may
        // re-root an index tree and persist the new root into the in-memory
        // catalog row — the persisted directory still registers the old
        // tree, whose pages must stay non-allocatable (they are the load
        // point of the next recovery).
        *self.persisted_catalog_rows.write().await = rows.clone();
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
            // change 2026-10-13 T3 (D5): bound by the table row's declared
            // count. An orphan column row (ALTER ADD COLUMN crash window
            // between its flush and the table-row commit flush) must not
            // materialize as a phantom column on reopen.
            cols.truncate(row.column_count as usize);
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
                // Iteration 003（D7）：新建表 ⇒ 全新状态单元。
                narrowing: Arc::new(NarrowingState::new(row.table_name.clone())),
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
            // Iteration 003（D7）：新建表 ⇒ 全新状态单元。
            narrowing: Arc::new(NarrowingState::new(name.to_string())),
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
            // change 2026-10-13 T3 (D5): failure cleanup — the CREATE leaves
            // zero usable residue and the same name is retryable:
            // 1. roll back the in-memory registration;
            // 2. remove the column rows already appended (the `__tables`
            //    row is written last inside `insert_table`, so on a column
            //    failure none exists — `delete_table`'s TABLES arm is a
            //    no-op and its COLUMNS arm removes exactly our rows);
            // 3. release the pages allocated for this attempt (data head +
            //    fresh index roots — create_index.rs backfill-failure
            //    precedent, best-effort).
            {
                let mut tables = self.tables.write().await;
                tables.remove(name);
            }
            let _ = self.catalog.delete_table(name).await;
            let mut released: Vec<PageId> = Vec::new();
            let root = table_meta.index_manager.root_page_id();
            for page in [page_id, root]
                .into_iter()
                .chain(table_meta.unique_indexes.iter().map(|(_, u)| u.root_page_id()))
            {
                if self.buffer_pool.free_page(page).await.is_ok() {
                    released.push(page);
                }
            }
            // MS35（`free-page-lifecycle` R3）：释放完成标记——本次释放没有
            // 锚点写（该表从未登记进目录），按保守方向等下一次成功刷盘。
            self.buffer_pool.complete_free_release(&released);
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
        // change 2026-10-13 T3 (D4): the column-count rewrite is the ALTER
        // ADD COLUMN commit point — flushed last so a reopened catalog never
        // advertises a column whose row never reached disk (orphan rows left
        // by an earlier crash stay invisible: open bounds columns by the
        // table row's `column_count`, D5).
        self.catalog
            .update_table_column_count_ddl(table, (old.columns.len() + 1) as u32)
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
            // Iteration 003（D7）：同一张表的活布局版本共享同一状态单元。
            narrowing: old.narrowing.clone(),
        });
        tables.insert(table.to_string(), meta);
        Ok(())
    }

    /// change 2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog /
    /// Iteration 001 (D5, ISS21): ALTER TABLE DROP COLUMN — remove a
    /// non-key column.
    ///
    /// The catalog narrowing is NOT part of this statement's transactional
    /// body. The body does the guard checks, computes the narrowed layout and
    /// rewrites every live row DELETE+INSERT inside the caller's transaction;
    /// the catalog change is handed back as a [`PendingColumnDrop`] and
    /// applied at the statement's commit point by
    /// [`TableManager::apply_column_drop`].
    ///
    /// Rationale (ISS21): narrowing the catalog *before* the data-plane
    /// rewrite left an abandoned or failed DDL with a narrowed catalog and
    /// unwritten rows — the directory advertised 2 columns while the rows
    /// still carried 3. Deferring the narrowing to the commit point makes the
    /// directory change and the data-plane rewrite live and die together: an
    /// abandoned or failed statement leaves neither.
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
    ) -> Result<PendingColumnDrop> {
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

        // 1. The dropped column's catalog definition, with its original
        // ordinal. Read while the catalog still describes the old layout —
        // the narrowing itself is deferred to the commit point.
        let removed_column = self
            .catalog
            .scan_columns(&old.name)
            .await?
            .into_iter()
            .find(|c| c.column_index == removed as u32)
            .ok_or_else(|| {
                StorageError::Internal(format!(
                    "alter_drop_column: no catalog column row at ordinal {} for table '{}'",
                    removed, old.name
                ))
            })?;
        let new_column_count = new_columns.len() as u32;

        // 2. The narrowed in-memory layout — the rewrite writes the narrowed
        // tuples and their index keys against it. The live table mapping is
        // switched only at the commit point (`apply_column_drop`).
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
            // Iteration 003（D7）：同一张表的活布局版本共享同一状态单元。
            narrowing: old.narrowing.clone(),
        });

        // 3. Implicit-transaction row rewrite: every live (chain-tail,
        // non-tombstone) row is deleted (tombstone + index cleanup + WAL)
        // and re-inserted as the narrowed row (fresh tuple + index rebuild
        // + WAL). All through the same WAL records / version chain /
        // index paths the DML executors use.
        //
        // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
        // Iteration 003（design D7 / ISS25）：**改写开始处发布在途窄化状态**
        // ——发布后到目录窄化生效前，读路径对该表给出明确拒绝，绝不返回按位置
        // 错位的值。清除点：`apply_column_drop`（目录窄化生效后）与语句作用域
        // 守卫（异常 / 放弃路径）。
        old.narrowing.publish();
        // 守卫随 `PendingColumnDrop` 活到语句结束——异常 / 放弃路径由它兜底
        // 清除，避免该表永久不可读。
        let narrowing_guard = NarrowingGuard::new(old.narrowing.clone());
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
                        // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
                        // Iteration 001（MTV R1 / ISS24 的 M2）：抑制集合只承认
                        // 「指向者未被回滚」的链接——已回滚（`create_tx_id == 0`）
                        // 的指向者不抑制其前驱版本，与
                        // `DataScanExecutor::superseder_suppresses` 同源。
                        if vh.create_tx_id() != 0 {
                            if let Some(target) = vh.next_version() {
                                pointed.insert(target);
                            }
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
            //
            // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
            // Iteration 001 (ISS22): both legs append through the LIVE table
            // meta (`old`), so the rewrite owns exactly one tail pointer and
            // one chain-advance path. Two independent tails (`old` for the
            // DELETE leg, `new_meta` for the INSERT leg) each allocate a new
            // page when the tail page fills and each overwrite the chain's
            // `next` pointer — the page allocated first falls out of the
            // chain and the rows written there become unreachable.
            //
            // Iteration 001 (ISS24): the tombstone is written **without** its
            // chain pointer, registered with the transaction manager, and only
            // then linked to the old row. The abandon window (the statement
            // future is dropped) may land between "the version slot is
            // written" and "the version is registered" — a tombstone that
            // already carries its chain pointer suppresses its predecessor by
            // the deleter's transaction state, and the reclaim cannot see the
            // unregistered version ⇒ the abandoned rewrite hides the row it
            // started rewriting. A tombstone without a chain pointer
            // suppresses nothing, so that window is harmless. The slot's final
            // content (create_tx / delete sentinel / chain pointer) is
            // byte-identical to the DeleteExecutor shape.
            let tombstone = VersionHeader::new(tx_id, None).mark_deleted();
            let tombstone_rid = self.write_tuple(&old, &tombstone, &[]).await?;
            self.buffer_pool
                .clear_all_visible(PageId(tombstone_rid.page_id as u64));
            self.buffer_pool
                .clear_all_visible(PageId(rid.page_id as u64));

            // 登记在链指针之前——登记之后回收才看得见这个版本（放弃窗口内的
            // 墓碑必须能被中和）。
            tx_manager
                .record_version(tx_id, &old.name, tombstone_rid)
                .await;

            // 补链指针：墓碑指向被删的旧行。
            update_version_header_in_data_page(
                &self.buffer_pool,
                tombstone_rid,
                VersionHeader::new(tx_id, None)
                    .with_next_version(*rid)
                    .mark_deleted(),
                &[],
            )
            .await?;

            // 索引条目清理在链指针之后——放弃窗口落在链指针之前时，条目仍在位。
            if let Some(key) = &old_pk_key {
                old.index_manager.delete(key.as_bytes()).await?;
            }
            for (uindex, key) in &old_unique_keys {
                uindex.delete(key).await?;
            }
            for (sindex, key) in &old_secondary_keys {
                sindex.delete_exact(key, *rid).await?;
            }

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
            // ISS22: same live-table append path as the DELETE leg — one
            // tail pointer, one chain-advance path for the whole rewrite.
            let new_rid = self.write_tuple(&old, &vh, &buf).await?;
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

        // ISS22: the rewrite appended through the live table meta, so the
        // narrowed meta's own tail pointer never advanced. `apply_column_drop`
        // installs `new_meta` as the live mapping at the commit point — hand
        // it the real chain terminal, otherwise the next append would start
        // from a stale page and fork the chain again.
        *new_meta.data_page_tail.lock().unwrap() = *old.data_page_tail.lock().unwrap();

        Ok(PendingColumnDrop {
            table: old.name.clone(),
            removed_column,
            new_column_count,
            new_meta,
            _narrowing_guard: narrowing_guard,
        })
    }

    /// Apply a completed `ALTER TABLE DROP COLUMN`'s catalog change and switch
    /// the live table mapping to the narrowed layout.
    ///
    /// This is the statement's commit point (ISS21 / design D5): it runs
    /// **after** the statement's transaction has committed, never inside the
    /// transactional body. An abandoned or failed DDL therefore applies
    /// nothing — the catalog keeps the pre-statement column definition and
    /// count, and the live layout stays the old one.
    ///
    /// The catalog change is applied in the ADD COLUMN commit-point order:
    /// the column rows first, the advertising column count last.
    /// `update_table_column_count_ddl` flushes every dirty page except the
    /// count page first and the count page last, so a crash can never leave a
    /// durable count that the column rows do not back.
    pub async fn apply_column_drop(&self, pending: &PendingColumnDrop) -> Result<()> {
        self.catalog
            .rewrite_column_rows_drop(&pending.table, pending.removed_column.column_index)
            .await?;
        self.catalog
            .update_table_column_count_ddl(&pending.table, pending.new_column_count)
            .await?;
        // The live mapping switches only once the catalog advertises the
        // narrowed layout.
        self.tables
            .write()
            .await
            .insert(pending.table.clone(), pending.new_meta.clone());
        // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
        // Iteration 003（design D7 / ISS25）：目录窄化已生效 ⇒ 清除在途窄化
        // 状态，读路径恢复正常。单元与旧活布局共享 ⇒ 持有计划期快照的读者
        // 也一并恢复。
        pending.new_meta.narrowing.clear();
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
                        // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
                        // Iteration 001（MTV R1 / ISS24 的 M2）：抑制集合只承认
                        // 「指向者未被回滚」的链接——已回滚（`create_tx_id == 0`）
                        // 的指向者不抑制其前驱版本，与
                        // `DataScanExecutor::superseder_suppresses` 同源。
                        if vh.create_tx_id() != 0 {
                            if let Some(target) = vh.next_version() {
                                pointed.insert(target);
                            }
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
            // Iteration 003（D7）：同一张表的活布局版本共享同一状态单元。
            narrowing: old.narrowing.clone(),
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
            // Iteration 003（D7）：同一张表的活布局版本共享同一状态单元。
            narrowing: old.narrowing.clone(),
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
            // Iteration 003（D7）：同一张表的活布局版本共享同一状态单元。
            narrowing: old.narrowing.clone(),
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
            // Iteration 003（D7）：同一张表的活布局版本共享同一状态单元。
            narrowing: old.narrowing.clone(),
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

        let mut released: Vec<PageId> = Vec::new();
        for page in index_pages
            .into_iter()
            .chain(overflow_pages)
            .chain(data_pages)
        {
            match self.buffer_pool.free_page(page).await {
                Ok(()) => released.push(page),
                Err(e) => eprintln!("[drop_table] free_page({}) failed: {}", page.0, e),
            }
        }

        // MS35（`free-page-lifecycle` R3）：释放完成标记——本次释放的锚点写
        // （目录删除，`:1414`）已经发出。标记之前，这些页不得可分配。
        self.buffer_pool.complete_free_release(&released);

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
    /// locators and collect every referenced chain page id (chains are
    /// independent pages, never in `next_page_id` links).
    ///
    /// change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
    /// Iteration 002 (B-4): the slot payloads are read through the integrity
    /// module's bounded reader (`check_data_page`) — the production accessor
    /// (`SlottedPageRef::get_slot_data`) slices the page unchecked and panics
    /// on a damaged slot offset, and this scan runs on the release path.
    ///
    /// Best-effort, same policy as `collect_data_pages`: a data page read or
    /// a chain walk error warns and skips that page/chain, returning what was
    /// collected so far. A damaged page is additionally counted and reported
    /// (its locators are not used, so its chains stay unreleased).
    async fn collect_overflow_chain_pages(&self, data_pages: &[PageId]) -> Vec<PageId> {
        let mut chain_heads: Vec<u32> = Vec::new();
        for &pid in data_pages {
            let heads: Vec<u32> = match self
                .buffer_pool
                .with_page_data(pid, |data| {
                    // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
                    // Iteration 002 (B-4): bounded read — the integrity
                    // module's data-page check yields the slot locators only
                    // for in-bounds slots. The previous unchecked
                    // `get_slot_data` slice panicked on an out-of-bounds slot
                    // offset (release path ⇒ process abort). A damaged page
                    // is counted, reported and skipped: its locators are not
                    // used, so its overflow chains stay unreleased.
                    let step = check_data_page(data);
                    if !step.violations.is_empty() {
                        eprintln!(
                            "[overflow-release] data page {} is damaged: {}",
                            pid.0,
                            violation_summary(&step.violations)
                        );
                        return Ok(Vec::new());
                    }
                    let mut heads = Vec::new();
                    for (_total_len, first) in step.locators {
                        if first != 0 {
                            heads.push(first);
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

    /// MS35 (`free-page-lifecycle` R2): rebuild this session's allocatable
    /// page set from the persisted directory.
    ///
    /// The live-page set covers every page owner (the exhaustive set of
    /// `allocate_page` call sites):
    ///
    /// - both catalog chains (`__tables` page 0, `__columns` page 1, walked
    ///   along `next_page_id` until 0);
    /// - each base table's data-page chain (from the row's `data_page_head`);
    /// - the overflow-chain pages the data pages' slot locators point to;
    /// - each table's PK tree (`index_root_page_id`), UNIQUE trees
    ///   (`unique_roots`) and secondary trees (`secondary_indexes[].root`).
    ///
    /// The registered tree pages and the in-memory index managers' tree pages
    /// are unioned: a recovery that re-rooted an index tree leaves the
    /// persisted directory registering the old tree while memory holds the
    /// new one — both page sets are live.
    ///
    /// The allocatable set is the page-count bound minus the live-page set.
    /// View rows own no pages.
    ///
    /// Failure safety (R3): any traversal error abandons the whole rebuild —
    /// nothing is injected, and the caller keeps the open successful.
    pub async fn rebuild_free_page_set(&self, db_path: &Path) -> Result<()> {
        // The bound is taken BEFORE the traversal: the recovery may have
        // allocated pages while opening, and pages allocated after the
        // snapshot must not be judged free.
        let page_bound = self.buffer_pool.storage().page_count();
        let live = self.collect_live_pages(db_path).await?;
        let free: Vec<u64> = (0..page_bound).filter(|page| !live.contains(page)).collect();
        self.buffer_pool.storage().inject_free_pages(free);
        Ok(())
    }

    /// The live-page set behind [`Self::rebuild_free_page_set`].
    ///
    /// Strict by construction: a read error propagates so the caller can
    /// abandon the whole rebuild (R3) instead of judging a partially walked
    /// ownership graph.
    async fn collect_live_pages(&self, db_path: &Path) -> Result<HashSet<u64>> {
        // MS35 (`free-page-lifecycle` R3): a controlled page-read failure
        // aborts the traversal — the rebuild is abandoned whole, nothing is
        // injected and the open stays successful. The injection stands in
        // for the page read that fails; it fires before the first read of
        // the traversal.
        if take_free_page_rebuild_fault(db_path) {
            return Err(StorageError::Io(std::io::Error::other(
                "injected page read failure (free-page rebuild)",
            )));
        }

        let mut live: HashSet<u64> = HashSet::new();

        // Catalog chains (both). A chain only ever grows (rows are deleted
        // in place, pages are appended), so the current chain covers the
        // persisted one — walking it covers both.
        for root in [PageId(TABLES_PAGE_ID), PageId(COLUMNS_PAGE_ID)] {
            for page in self.catalog.collect_chain_pages(root).await? {
                live.insert(page.0);
            }
        }

        // Catalog rows: the persisted directory (read at open) and the
        // current one (after recovery). Both register pages that must stay
        // non-allocatable.
        let persisted_rows = self.persisted_catalog_rows.read().await.clone();
        let current_rows = self.catalog.scan_tables().await?;
        for row in persisted_rows.iter().chain(current_rows.iter()) {
            if row.kind != CatalogKind::Table {
                // View rows own no pages.
                continue;
            }
            let (data_pages, chain_heads) = self
                .collect_data_chain_strict(PageId(row.data_page_head as u64))
                .await?;
            live.extend(data_pages.iter().map(|page| page.0));
            for page in self.collect_overflow_chain_pages_strict(&chain_heads).await? {
                live.insert(page.0);
            }
            // The trees the directory registers.
            for page in self
                .collect_tree_pages(PageId(row.index_root_page_id as u64))
                .await?
            {
                live.insert(page);
            }
            for root in &row.unique_roots {
                for page in self.collect_tree_pages(PageId(*root as u64)).await? {
                    live.insert(page);
                }
            }
            for entry in &row.secondary_indexes {
                for page in self.collect_tree_pages(PageId(entry.root as u64)).await? {
                    live.insert(page);
                }
            }
        }

        // The trees the in-memory index managers hold (recovery re-rooting
        // ⇒ union with the registered trees).
        {
            let tables = self.tables.read().await;
            for meta in tables.values() {
                for page in self.collect_tree_pages(meta.index_manager.root_page_id()).await? {
                    live.insert(page);
                }
                for (_, uindex) in &meta.unique_indexes {
                    for page in self.collect_tree_pages(uindex.root_page_id()).await? {
                        live.insert(page);
                    }
                }
                for sindex in &meta.secondary_indexes {
                    for page in self.collect_tree_pages(sindex.index.root_page_id()).await? {
                        live.insert(page);
                    }
                }
            }
        }

        // MS35 (`free-page-lifecycle` R3): a controlled page-read failure
        // aborts the traversal — the rebuild is abandoned whole, nothing is
        // injected and the open stays successful.
        Ok(live)
    }

    /// Strict data-page chain walk.
    ///
    /// The page image is parsed through the integrity module's bounds-checked
    /// reader (`check_data_page`): the production accessors index unchecked
    /// into a page and would panic on a damaged page, while the rebuild runs
    /// on every open and must never crash the open. A damaged page is an
    /// error (R3) — the rebuild is abandoned rather than judged on a
    /// half-read ownership graph.
    async fn collect_data_chain_strict(
        &self,
        head: PageId,
    ) -> Result<(Vec<PageId>, Vec<u32>)> {
        let mut pages = Vec::new();
        let mut chain_heads: Vec<u32> = Vec::new();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut current = head;
        while current.0 != 0 && visited.insert(current.0) {
            let step = self
                .buffer_pool
                .with_page_data(current, |data| Ok(check_data_page(data)))
                .await?;
            if !step.violations.is_empty() {
                return Err(StorageError::Internal(format!(
                    "data page {} of the persisted directory is damaged: {}",
                    current.0,
                    step.violations.join("; ")
                )));
            }
            pages.push(current);
            for (_, first) in step.locators {
                if first != 0 {
                    chain_heads.push(first);
                }
            }
            current = PageId(step.next_page_id as u64);
        }
        Ok((pages, chain_heads))
    }

    /// Strict overflow-chain collection: the locators found in the data pages
    /// are followed to their chain pages; a read error or a malformed chain is
    /// an error (R3).
    async fn collect_overflow_chain_pages_strict(&self, chain_heads: &[u32]) -> Result<Vec<PageId>> {
        let mut pages = Vec::new();
        for first in chain_heads {
            for raw in overflow::collect_overflow_pages(&self.buffer_pool, *first).await? {
                pages.push(PageId(raw as u64));
            }
        }
        Ok(pages)
    }

    /// The pages of the tree rooted at `root`.
    ///
    /// The tree is walked from the directory's registered root through the
    /// integrity module's bounds-checked reader (`check_tree_page`) — a
    /// damaged tree page is an error (R3), never a panic.
    async fn collect_tree_pages(&self, root: PageId) -> Result<Vec<u64>> {
        if root.0 == 0 {
            // No tree registered (legacy row / no index) — page 0 is the
            // catalog page and is never a tree root.
            return Ok(Vec::new());
        }
        let mut pages = Vec::new();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut stack = vec![root.0];
        while let Some(raw) = stack.pop() {
            if !visited.insert(raw) {
                continue;
            }
            let step = self
                .buffer_pool
                .with_page_data(PageId(raw), |data| Ok(check_tree_page(data)))
                .await?;
            if let Some(actual) = step.invalid_type {
                return Err(StorageError::InvalidPageType {
                    expected: LEAF_NODE,
                    actual,
                });
            }
            if !step.violations.is_empty() {
                return Err(StorageError::Internal(format!(
                    "index tree page {raw} is damaged: {}",
                    step.violations.join("; ")
                )));
            }
            pages.push(raw);
            for child in step.children {
                if child != 0 {
                    stack.push(child as u64);
                }
            }
        }
        Ok(pages)
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
        let mut released: Vec<PageId> = Vec::new();
        for page in self.collect_overflow_chain_pages(&data_pages).await {
            match self.buffer_pool.free_page(page).await {
                Ok(()) => released.push(page),
                Err(e) => eprintln!(
                    "[truncate_table] free overflow page({}) failed: {}",
                    page.0, e
                ),
            }
        }
        for page in data_pages.into_iter().skip(1) {
            match self.buffer_pool.free_page(page).await {
                Ok(()) => released.push(page),
                Err(e) => eprintln!("[truncate_table] free_page({}) failed: {}", page.0, e),
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

        // MS35（`free-page-lifecycle` R3）：步骤 1 的释放（溢出链页 + 尾数据
        // 页）的锚点写 = 表头原地重置 + 目录尾指针更新，两者此刻都已发出 ⇒
        // 释放完成标记。标记之前这些页不得可分配。
        self.buffer_pool.complete_free_release(&released);

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
            let mut released: Vec<PageId> = Vec::new();
            for tree in std::iter::once(&new_pk)
                .chain(new_uniques.iter().map(|(_, t)| t))
                .chain(new_secondaries.iter().map(|s| &s.index))
            {
                for page in tree.collect_all_pages_tolerant().await {
                    if self.buffer_pool.free_page(page).await.is_ok() {
                        released.push(page);
                    }
                }
            }
            // MS35（`free-page-lifecycle` R3）：释放完成标记——新建的树从未
            // 登记进目录（无锚点写），按保守方向等下一次成功刷盘。
            self.buffer_pool.complete_free_release(&released);
            return Err(e);
        }
        let (old_pk, old_uniques, old_secondaries) = self
            .replace_recovery_indexes(name, new_pk, new_uniques, new_secondaries)
            .await?;

        // 4. Old trees best-effort release (drop_table policy: collect
        //    failure warns and gives up, free failures warn per page).
        let mut released: Vec<PageId> = Vec::new();
        for tree in std::iter::once(old_pk)
            .chain(old_uniques.into_iter().map(|(_, t)| t))
            .chain(old_secondaries.into_iter().map(|s| s.index))
        {
            match tree.collect_all_pages().await {
                Ok(pages) => {
                    for page in pages {
                        match self.buffer_pool.free_page(page).await {
                            Ok(()) => released.push(page),
                            Err(e) => {
                                eprintln!("[truncate_table] free_page({}) failed: {}", page.0, e)
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[truncate_table] collect_all_pages({}) failed: {}", name, e);
                }
            }
        }

        // MS35（`free-page-lifecycle` R3）：旧树的释放完成标记——本次释放的
        // 锚点写 = 步骤 3 的目录 root 更新（PK / 唯一 / 二级），已发出。
        self.buffer_pool.complete_free_release(&released);

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
