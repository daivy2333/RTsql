//! Catalog: persistent system tables `__tables` and `__columns`.
//!
//! MS07-T01: `TableManager` previously kept table definitions in an
//! in-memory `RwLock<HashMap<...>>` that disappeared on restart. This
//! module persists those definitions as two `SlottedPage` chains:
//!
//! - `__tables` — one row per user table (head pointer, index root,
//!   primary key, column count, tail pointer).
//! - `__columns` — one row per column of each user table (name, type,
//!   not-null / unique flags).
//!
//! Both chains live in user-page-id 0 (`__tables`) and 1 (`__columns`).
//! The convention is established by `Catalog::bootstrap` (new file) and
//! preserved by `Catalog::open` (existing file). `Database::open` invokes
//! `open` after `FileStorage::open` so the first `allocate_page` returns
//! the next free page beyond the catalog pages.
//!
//! Concurrency: every public mutating method takes `self.lock` (a Tokio
//! async mutex) so catalog reads see a consistent snapshot of a write.
//! Read-only `scan_*` callers can use the returned `Vec` after the lock
//! is released, since the data is owned.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::storage::page_format::{
    ColumnType, SlottedPage, SlottedPageRef, TAG_BOOL, TAG_DATE, TAG_FLOAT, TAG_INT, TAG_NULL,
    TAG_STRING, TAG_TIMESTAMP,
};
use crate::storage::{AsyncStorage, BufferPool, Page, PageId, Result, StorageError};

/// System table name for the table-of-tables.
pub const TABLES_SYSTEM_NAME: &str = "__tables";
/// System table name for the table-of-columns.
pub const COLUMNS_SYSTEM_NAME: &str = "__columns";
/// Page-id reserved for the `__tables` SlottedPage.
pub const TABLES_PAGE_ID: u64 = 0;
/// Page-id reserved for the `__columns` SlottedPage.
pub const COLUMNS_PAGE_ID: u64 = 1;

/// Tag byte for `ColumnType::Int` in the catalog serialized form.
const COL_TAG_INT: u8 = 0x01;
/// Tag byte for `ColumnType::String(u16)` in the catalog serialized form.
const COL_TAG_STRING: u8 = 0x02;
/// Tag byte for `ColumnType::Float` in the catalog serialized form.
const COL_TAG_FLOAT: u8 = 0x03;
/// Tag byte for `ColumnType::Bool` in the catalog serialized form.
const COL_TAG_BOOL: u8 = 0x04;
/// Tag byte for `ColumnType::Date` in the catalog serialized form (MS13).
const COL_TAG_DATE: u8 = 0x05;
/// Tag byte for `ColumnType::Timestamp` in the catalog serialized form (MS13).
const COL_TAG_TIMESTAMP: u8 = 0x06;

/// One row in the `__tables` SlottedPage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogRow {
    pub table_name: String,
    pub data_page_head: u32,
    pub index_root_page_id: u32,
    pub pk_index: u32,
    pub pk_column: String,
    pub column_count: u32,
    pub data_page_tail: u32,
    /// MS23 Iteration 001 (D4): roots of the per-column UNIQUE indexes, in
    /// ascending qualifying-column order (INT ∧ unique ∧ non-PK). Appended
    /// to the row tail; a legacy row without the trailing section
    /// deserializes as empty (compat read — no unique indexes).
    pub unique_roots: Vec<u32>,
}

/// One row in the `__columns` SlottedPage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogColumnRow {
    pub table_name: String,
    pub column_index: u32,
    pub column_name: String,
    pub column_type: ColumnType,
    pub not_null: bool,
    pub unique: bool,
    /// MS24 Iteration 000 (D1): declared DEFAULT literal, persisted in the
    /// row tail (`u8 has_default | tag | payload` when present). A legacy row
    /// without the trailing block (or a `has_default = 0` block) reads as
    /// `None` — flag-only semantics, no default application.
    pub default_value: Option<crate::executor::Value>,
}

/// Persistent catalog for user tables. Holds the two system-table
/// `SlottedPage` chains (one per system table) and a write lock so
/// `insert_table` / `delete_table` / `update_table_tail` serialize
/// against each other.
pub struct Catalog {
    buffer_pool: Arc<BufferPool>,
    storage: Arc<dyn AsyncStorage>,
    /// Head page-id of the `__tables` SlottedPage chain.
    tables_root: Mutex<PageId>,
    /// Head page-id of the `__columns` SlottedPage chain.
    columns_root: Mutex<PageId>,
    /// Serializes all mutating operations on either chain.
    lock: Mutex<()>,
}

impl Catalog {
    /// Bootstrap a brand-new catalog for a freshly-opened empty file.
    ///
    /// Allocates page 0 and page 1 as empty `SlottedPage`s (page_type
    /// `0x03`, slot_count 0). The next `BufferPool::storage.allocate_page`
    /// returns `PageId(2)`, the first user-data page.
    pub async fn bootstrap(
        buffer_pool: Arc<BufferPool>,
        storage: Arc<dyn AsyncStorage>,
    ) -> Result<Arc<Self>> {
        // Allocate the two reserved pages.
        let tables_id = storage.allocate_page().await?;
        if tables_id.0 != TABLES_PAGE_ID {
            return Err(StorageError::Internal(format!(
                "Catalog::bootstrap: expected page 0 for __tables, got {}",
                tables_id.0
            )));
        }
        let columns_id = storage.allocate_page().await?;
        if columns_id.0 != COLUMNS_PAGE_ID {
            return Err(StorageError::Internal(format!(
                "Catalog::bootstrap: expected page 1 for __columns, got {}",
                columns_id.0
            )));
        }

        // Init each page as an empty SlottedPage (page_type 0x03).
        init_empty_slotted_page(&buffer_pool, tables_id).await?;
        init_empty_slotted_page(&buffer_pool, columns_id).await?;

        // Flush dirty pages so the on-disk file contains the initialized
        // headers. (Do NOT use `BufferPool::free_page` here — that would
        // push the page to the storage free list and zero it.)
        buffer_pool.flush_all().await?;

        Ok(Arc::new(Self {
            buffer_pool,
            storage,
            tables_root: Mutex::new(tables_id),
            columns_root: Mutex::new(columns_id),
            lock: Mutex::new(()),
        }))
    }

    /// Open an existing catalog from a file that already has page 0 and
    /// page 1 allocated. Does not allocate; just binds to those pages.
    pub async fn open(
        buffer_pool: Arc<BufferPool>,
        storage: Arc<dyn AsyncStorage>,
    ) -> Result<Arc<Self>> {
        let tables_id = PageId(TABLES_PAGE_ID);
        let columns_id = PageId(COLUMNS_PAGE_ID);

        // Sanity: touch each page so the BufferPool caches a freshly-read
        // copy. We don't validate page_type here — caller decides what to
        // do if the file is not a catalog.
        let _ = buffer_pool.get_page(tables_id).await?;
        let _ = buffer_pool.get_page(columns_id).await?;

        Ok(Arc::new(Self {
            buffer_pool,
            storage,
            tables_root: Mutex::new(tables_id),
            columns_root: Mutex::new(columns_id),
            lock: Mutex::new(()),
        }))
    }

    /// Insert a new table row + its column rows.
    ///
    /// `meta` carries the fields needed for `__tables`; `columns` carries
    /// one `CatalogColumnRow` per column. The `data_page_head` /
    /// `data_page_tail` are equal at create time (the new head page is
    /// the tail until first auto-allocation).
    pub async fn insert_table(
        &self,
        meta: &CatalogRow,
        columns: &[CatalogColumnRow],
    ) -> Result<()> {
        let _guard = self.lock.lock().await;
        let payload = serialize_catalog_row(meta);
        let tables_root = *self.tables_root.lock().await;
        append_to_chain(&self.buffer_pool, &self.storage, tables_root, &payload).await?;
        // Note: the new head page is recorded by `append_to_chain` in the
        // root if a fresh page had to be allocated. Re-read in that case.
        *self.tables_root.lock().await = chain_head(&self.buffer_pool, tables_root).await?;

        for col in columns {
            let payload = serialize_catalog_column_row(col);
            let columns_root = *self.columns_root.lock().await;
            append_to_chain(&self.buffer_pool, &self.storage, columns_root, &payload).await?;
            *self.columns_root.lock().await = chain_head(&self.buffer_pool, columns_root).await?;
        }
        Ok(())
    }

    /// Delete all rows for `table_name` from both chains. Returns `Ok(())`
    /// even if the table does not exist (idempotent; physical page free
    /// is out of scope and handled by MS07-T02).
    pub async fn delete_table(&self, name: &str) -> Result<()> {
        let _guard = self.lock.lock().await;
        delete_from_chain(
            &self.buffer_pool,
            &self.storage,
            PageId(TABLES_PAGE_ID),
            |data| matches_catalog_row_name(data, name),
        )
        .await?;
        delete_from_chain(
            &self.buffer_pool,
            &self.storage,
            PageId(COLUMNS_PAGE_ID),
            |data| matches_catalog_column_row_table_name(data, name),
        )
        .await?;
        Ok(())
    }

    /// Scan all `__tables` rows.
    pub async fn scan_tables(&self) -> Result<Vec<CatalogRow>> {
        let _guard = self.lock.lock().await;
        scan_chain(&self.buffer_pool, PageId(TABLES_PAGE_ID), |data| {
            deserialize_catalog_row(data)
        })
        .await
    }

    /// Scan all `__columns` rows for `table_name`.
    pub async fn scan_columns(&self, table_name: &str) -> Result<Vec<CatalogColumnRow>> {
        let _guard = self.lock.lock().await;
        let all = scan_chain(&self.buffer_pool, PageId(COLUMNS_PAGE_ID), |data| {
            deserialize_catalog_column_row(data)
        })
        .await?;
        Ok(all
            .into_iter()
            .filter(|c| c.table_name == table_name)
            .collect())
    }

    /// Update the `data_page_tail` field of an existing row in `__tables`.
    ///
    /// No-op if the table does not exist.
    pub async fn update_table_tail(&self, name: &str, new_tail: u32) -> Result<()> {
        let _guard = self.lock.lock().await;
        update_field_in_chain(
            &self.buffer_pool,
            &self.storage,
            PageId(TABLES_PAGE_ID),
            name,
            |data| {
                let mut row = deserialize_catalog_row(data)?;
                if row.table_name != name {
                    return Ok(None);
                }
                row.data_page_tail = new_tail;
                Ok(Some(serialize_catalog_row(&row)))
            },
        )
        .await
    }

    /// Update the `index_root_page_id` field of an existing row in `__tables`.
    ///
    /// MS10-T02 Iter000 003-rework (R-T0b-R5): the in-memory B-Tree root
    /// moves on root splits (insert) and root shrink (delete merge); the
    /// catalog row must follow so a restart loads a root that reaches every
    /// entry persisted up to the last checkpoint site. Persistence of the
    /// updated row is guaranteed by the existing flush paths (checkpoint
    /// full flush / page eviction).
    ///
    /// No-op if the table does not exist.
    pub async fn update_table_root(&self, name: &str, new_root: u32) -> Result<()> {
        let _guard = self.lock.lock().await;
        update_field_in_chain(
            &self.buffer_pool,
            &self.storage,
            PageId(TABLES_PAGE_ID),
            name,
            |data| {
                let mut row = deserialize_catalog_row(data)?;
                if row.table_name != name {
                    return Ok(None);
                }
                row.index_root_page_id = new_root;
                Ok(Some(serialize_catalog_row(&row)))
            },
        )
        .await
    }

    /// MS23 Iteration 001 (2.1/D4): update the Nth UNIQUE index root of an
    /// existing row in `__tables` (`ordinal` = position among the table's
    /// qualifying UNIQUE columns in ascending column order — the same order
    /// `unique_roots` is serialized in). The deserialize→mutate→serialize
    /// round-trip preserves every other field, including sibling unique
    /// roots. Errors if the ordinal is out of range (internal inconsistency:
    /// a unique index only exists when the row carries its root).
    ///
    /// No-op if the table does not exist.
    pub async fn update_unique_index_root(
        &self,
        name: &str,
        ordinal: usize,
        new_root: u32,
    ) -> Result<()> {
        let _guard = self.lock.lock().await;
        update_field_in_chain(
            &self.buffer_pool,
            &self.storage,
            PageId(TABLES_PAGE_ID),
            name,
            |data| {
                let mut row = deserialize_catalog_row(data)?;
                if row.table_name != name {
                    return Ok(None);
                }
                if ordinal >= row.unique_roots.len() {
                    return Err(StorageError::Internal(format!(
                        "update_unique_index_root: ordinal {} out of range for table '{}' ({})",
                        ordinal,
                        name,
                        row.unique_roots.len()
                    )));
                }
                row.unique_roots[ordinal] = new_root;
                Ok(Some(serialize_catalog_row(&row)))
            },
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Initialization helpers
// ---------------------------------------------------------------------------

async fn init_empty_slotted_page(bp: &BufferPool, page_id: PageId) -> Result<()> {
    let guard = bp.get_page(page_id).await?;
    guard.modify_page(|page: &mut Page| {
        let _ = SlottedPage::init(page, 0x03);
    });
    Ok(())
}

/// After `append_to_chain` may have allocated a new page, walk forward
/// from the original root until `next_page_id == 0` and return the last
/// page-id in the chain. This becomes the new "head" for subsequent
/// appends so we always grow the tail.
async fn chain_head(bp: &BufferPool, start: PageId) -> Result<PageId> {
    let mut current = start;
    loop {
        let next = bp
            .with_page_data(current, |data| -> Result<u32> {
                let slotted = SlottedPageRef::new(data);
                Ok(slotted.header().next_page_id)
            })
            .await?;
        if next == 0 {
            return Ok(current);
        }
        current = PageId(next as u64);
    }
}

// ---------------------------------------------------------------------------
// Chain operations
// ---------------------------------------------------------------------------

/// Append `payload` as a new slot in the chain starting at `root`. If the
/// last page is full, allocate a new page, init it, link it from the
/// previous page, and write into the new page.
async fn append_to_chain(
    bp: &BufferPool,
    storage: &Arc<dyn AsyncStorage>,
    root: PageId,
    payload: &[u8],
) -> Result<()> {
    let last = chain_head(bp, root).await?;

    // Try to append to the last page first.
    let add_result: std::result::Result<(u16, usize), String> = {
        let guard = bp.get_page(last).await?;
        guard.modify_page(|page| {
            let mut slotted = SlottedPage::new(page);
            slotted.add_slot(payload)
        })
    };

    match add_result {
        Ok(_) => Ok(()),
        Err(_) => {
            // Page full — allocate a fresh page, init, link, write.
            let new_id = storage.allocate_page().await?;
            let new_guard = bp.get_page(new_id).await?;
            new_guard.modify_page(|page| {
                let _ = SlottedPage::init(page, 0x03);
            });
            // Link previous page's next_page_id (header offset 5..9) to new_id.
            let prev_guard = bp.get_page(last).await?;
            prev_guard.modify_page(|page: &mut Page| {
                let new_id_u32 = new_id.0 as u32;
                page.data[5..9].copy_from_slice(&new_id_u32.to_le_bytes());
            });

            // Write into the new page.
            let new_guard = bp.get_page(new_id).await?;
            let write_result: std::result::Result<(u16, usize), String> =
                new_guard.modify_page(|page| {
                    let mut slotted = SlottedPage::new(page);
                    slotted.add_slot(payload)
                });
            write_result.map_err(|_| StorageError::PageFull)?;
            // Drop cache for the newly-allocated page so subsequent
            // `chain_head` walk sees the on-disk link.
            bp.free_page(new_id).await?;
            Ok(())
        }
    }
}

/// Walk a chain starting at `root`, calling `parse` on each row's bytes
/// and collecting the results.
async fn scan_chain<P, T>(bp: &BufferPool, root: PageId, parse: P) -> Result<Vec<T>>
where
    P: Fn(&[u8]) -> Result<T>,
{
    let mut out = Vec::new();
    let mut current = Some(root);
    let mut visited = std::collections::HashSet::new();
    while let Some(page_id) = current {
        if !visited.insert(page_id.0) {
            // Defensive: cycle in next_page_id. Break to avoid infinite loop.
            break;
        }
        let (rows, next) = bp
            .with_page_data(page_id, |data| -> Result<(Vec<Vec<u8>>, u32)> {
                let slotted = SlottedPageRef::new(data);
                let mut rows = Vec::with_capacity(slotted.slot_count());
                for i in 0..slotted.slot_count() {
                    if let Some(slot) = slotted.get_slot(i) {
                        let bytes = slotted.get_slot_data(&slot).to_vec();
                        rows.push(bytes);
                    }
                }
                Ok((rows, slotted.header().next_page_id))
            })
            .await?;
        for row_bytes in rows {
            out.push(parse(&row_bytes)?);
        }
        current = if next == 0 {
            None
        } else {
            Some(PageId(next as u64))
        };
    }
    Ok(out)
}

/// Delete every row in the chain whose bytes satisfy `matcher`. Rewrites
/// each affected page in place (without compaction, since SlottedPage
/// `delete_slot` already compacts within the page).
async fn delete_from_chain<F>(
    bp: &BufferPool,
    storage: &Arc<dyn AsyncStorage>,
    root: PageId,
    matcher: F,
) -> Result<()>
where
    F: Fn(&[u8]) -> bool,
{
    let mut current = Some(root);
    let mut visited = std::collections::HashSet::new();
    while let Some(page_id) = current {
        if !visited.insert(page_id.0) {
            break;
        }
        // Walk the page, collect logical_ids to delete.
        let to_delete: Vec<u16> = bp
            .with_page_data(page_id, |data| -> Result<Vec<u16>> {
                let slotted = SlottedPageRef::new(data);
                let mut del = Vec::new();
                for i in 0..slotted.slot_count() {
                    if let Some(slot) = slotted.get_slot(i) {
                        let bytes = slotted.get_slot_data(&slot);
                        if matcher(bytes) {
                            del.push(slot.logical_id);
                        }
                    }
                }
                Ok(del)
            })
            .await?;

        if !to_delete.is_empty() {
            let guard = bp.get_page(page_id).await?;
            guard.modify_page(|page| {
                let mut slotted = SlottedPage::new(page);
                for lid in &to_delete {
                    let _ = slotted.delete_slot_by_logical_id(*lid);
                }
            });
        }

        let next = bp
            .with_page_data(page_id, |data| -> Result<u32> {
                let slotted = SlottedPageRef::new(data);
                Ok(slotted.header().next_page_id)
            })
            .await?;
        current = if next == 0 {
            None
        } else {
            Some(PageId(next as u64))
        };
        // `storage` is currently unused; kept for future physical free
        // by MS07-T02.
        let _ = storage;
    }
    Ok(())
}

/// Update the bytes of a row whose name matches `target_name`, using
/// `updater` to produce the new payload from the current bytes.
async fn update_field_in_chain<F>(
    bp: &BufferPool,
    storage: &Arc<dyn AsyncStorage>,
    root: PageId,
    target_name: &str,
    updater: F,
) -> Result<()>
where
    F: Fn(&[u8]) -> Result<Option<Vec<u8>>>,
{
    let mut current = Some(root);
    let mut visited = std::collections::HashSet::new();
    while let Some(page_id) = current {
        if !visited.insert(page_id.0) {
            break;
        }
        // Find first matching logical_id on this page.
        let found: Option<(u16, Vec<u8>)> = bp
            .with_page_data(page_id, |data| -> Result<Option<(u16, Vec<u8>)>> {
                let slotted = SlottedPageRef::new(data);
                for i in 0..slotted.slot_count() {
                    if let Some(slot) = slotted.get_slot(i) {
                        let bytes = slotted.get_slot_data(&slot);
                        if matches_catalog_row_name(bytes, target_name) {
                            if let Some(new_bytes) = updater(bytes)? {
                                return Ok(Some((slot.logical_id, new_bytes)));
                            }
                        }
                    }
                }
                Ok(None)
            })
            .await?;

        if let Some((lid, new_bytes)) = found {
            // SlottedPage has no update-in-place API. Strategy: append the
            // new row, then delete the old one. If the page is full, the
            // new row lands on a fresh page (via `add_slot` Err path
            // handled by the caller returning Ok early) — but here we are
            // inside `update_field_in_chain`, not the append path. The
            // payload is small (CatalogRow ≈ 31 bytes + 6 slot = 37 B),
            // and a 4 KB page fits ~100 rows, so the page-full case is
            // extremely unlikely in practice. We still guard explicitly.
            let guard = bp.get_page(page_id).await?;
            let add_result: std::result::Result<(u16, usize), String> = guard.modify_page(|page| {
                let mut slotted = SlottedPage::new(page);
                slotted.add_slot(&new_bytes)
            });
            match add_result {
                Ok(_) => {
                    // New row appended. Delete the old row by logical_id.
                    let guard = bp.get_page(page_id).await?;
                    guard.modify_page(|page| {
                        let mut slotted = SlottedPage::new(page);
                        let _ = slotted.delete_slot_by_logical_id(lid);
                    });
                }
                Err(_) => {
                    let _ = storage;
                    return Ok(());
                }
            }
        }

        let next = bp
            .with_page_data(page_id, |data| -> Result<u32> {
                let slotted = SlottedPageRef::new(data);
                Ok(slotted.header().next_page_id)
            })
            .await?;
        current = if next == 0 {
            None
        } else {
            Some(PageId(next as u64))
        };
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Row serialization
// ---------------------------------------------------------------------------

/// Layout: u16 name_len | name_bytes | u32 head | u32 idx_root |
///         u32 pk_index | u16 pk_col_len | pk_col_bytes |
///         u32 column_count | u32 tail
///         [ MS23 Iter001 appended: u32 unique_count | unique_count × u32 root ]
pub(crate) fn serialize_catalog_row(row: &CatalogRow) -> Vec<u8> {
    let name_bytes = row.table_name.as_bytes();
    let pk_bytes = row.pk_column.as_bytes();
    let total = 2
        + name_bytes.len()
        + 4
        + 4
        + 4
        + 2
        + pk_bytes.len()
        + 4
        + 4
        + 4
        + 4 * row.unique_roots.len();
    let mut buf = Vec::with_capacity(total);
    buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(name_bytes);
    buf.extend_from_slice(&row.data_page_head.to_le_bytes());
    buf.extend_from_slice(&row.index_root_page_id.to_le_bytes());
    buf.extend_from_slice(&row.pk_index.to_le_bytes());
    buf.extend_from_slice(&(pk_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(pk_bytes);
    buf.extend_from_slice(&row.column_count.to_le_bytes());
    buf.extend_from_slice(&row.data_page_tail.to_le_bytes());
    buf.extend_from_slice(&(row.unique_roots.len() as u32).to_le_bytes());
    for root in &row.unique_roots {
        buf.extend_from_slice(&root.to_le_bytes());
    }
    debug_assert_eq!(buf.len(), total);
    buf
}

pub(crate) fn deserialize_catalog_row(data: &[u8]) -> Result<CatalogRow> {
    let mut p = 0usize;
    if data.len() < 2 {
        return Err(StorageError::Internal(
            "catalog row too short (name_len)".into(),
        ));
    }
    let name_len = u16::from_le_bytes([data[p], data[p + 1]]) as usize;
    p += 2;
    if data.len() < p + name_len {
        return Err(StorageError::Internal(
            "catalog row truncated (table_name)".into(),
        ));
    }
    let table_name = std::str::from_utf8(&data[p..p + name_len])
        .map_err(|e| StorageError::Internal(format!("invalid utf8 table_name: {e}")))?
        .to_string();
    p += name_len;
    if data.len() < p + 4 * 4 {
        return Err(StorageError::Internal(
            "catalog row truncated (u32s)".into(),
        ));
    }
    let data_page_head = u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]);
    p += 4;
    let index_root_page_id = u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]);
    p += 4;
    let pk_index = u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]);
    p += 4;
    if data.len() < p + 2 {
        return Err(StorageError::Internal(
            "catalog row truncated (pk_col_len)".into(),
        ));
    }
    let pk_len = u16::from_le_bytes([data[p], data[p + 1]]) as usize;
    p += 2;
    if data.len() < p + pk_len {
        return Err(StorageError::Internal(
            "catalog row truncated (pk_col)".into(),
        ));
    }
    let pk_column = std::str::from_utf8(&data[p..p + pk_len])
        .map_err(|e| StorageError::Internal(format!("invalid utf8 pk_column: {e}")))?
        .to_string();
    p += pk_len;
    if data.len() < p + 8 {
        return Err(StorageError::Internal(
            "catalog row truncated (count/tail)".into(),
        ));
    }
    let column_count = u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]);
    p += 4;
    let data_page_tail = u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]);
    // MS23 Iter001 (D4) compat read: the trailing unique-roots section
    // (`u32 count | count × u32 roots`) is optional. A legacy row ends at
    // `tail` — an absent or incomplete trailing section deserializes as an
    // empty `unique_roots` (behavior = no unique indexes). Same shape as the
    // 24B checkpoint-site three-branch compat read (MS17-T02).
    let mut unique_roots = Vec::new();
    let rest = &data[p + 4..];
    if rest.len() >= 4 {
        let count = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        if rest.len() >= 4 + count * 4 {
            for i in 0..count {
                let off = 4 + i * 4;
                unique_roots.push(u32::from_le_bytes([
                    rest[off],
                    rest[off + 1],
                    rest[off + 2],
                    rest[off + 3],
                ]));
            }
        }
    }
    Ok(CatalogRow {
        table_name,
        data_page_head,
        index_root_page_id,
        pk_index,
        pk_column,
        column_count,
        data_page_tail,
        unique_roots,
    })
}

pub(crate) fn matches_catalog_row_name(data: &[u8], name: &str) -> bool {
    if data.len() < 2 {
        return false;
    }
    let name_len = u16::from_le_bytes([data[0], data[1]]) as usize;
    if data.len() < 2 + name_len {
        return false;
    }
    &data[2..2 + name_len] == name.as_bytes()
}

/// Layout: u16 table_name_len | table_name_bytes | u32 column_index |
///         u16 column_name_len | column_name_bytes | u8 col_type_tag |
///         (optional u16 string_max_len) | u8 not_null | u8 unique
///         [ MS24 Iter000 appended when a DEFAULT is declared:
///           u8 has_default=1 | u8 value_tag (tuple TAG_* domain) | payload ]
pub(crate) fn serialize_catalog_column_row(row: &CatalogColumnRow) -> Vec<u8> {
    let table_name_bytes = row.table_name.as_bytes();
    let column_name_bytes = row.column_name.as_bytes();
    let (col_tag, extra) = match row.column_type {
        ColumnType::Int => (COL_TAG_INT, None),
        ColumnType::Float => (COL_TAG_FLOAT, None),
        ColumnType::Bool => (COL_TAG_BOOL, None),
        ColumnType::Date => (COL_TAG_DATE, None),
        ColumnType::Timestamp => (COL_TAG_TIMESTAMP, None),
        ColumnType::String(max_len) => (COL_TAG_STRING, Some(max_len)),
    };
    let mut buf = Vec::new();
    buf.extend_from_slice(&(table_name_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(table_name_bytes);
    buf.extend_from_slice(&row.column_index.to_le_bytes());
    buf.extend_from_slice(&(column_name_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(column_name_bytes);
    buf.push(col_tag);
    if let Some(max_len) = extra {
        buf.extend_from_slice(&max_len.to_le_bytes());
    }
    buf.push(if row.not_null { 1 } else { 0 });
    buf.push(if row.unique { 1 } else { 0 });
    // MS24 Iter000 (D1): the DEFAULT tail section is appended only when a
    // default is declared, so rows without one stay byte-identical to the
    // legacy layout. Payload widths mirror the tuple format (Int i64 LE /
    // Float f64 LE / Bool 1B / Date i32 LE / Timestamp i64 LE / String
    // u16-len + bytes / Null none).
    if let Some(value) = &row.default_value {
        buf.push(1);
        match value {
            crate::executor::Value::Int(n) => {
                buf.push(TAG_INT);
                buf.extend_from_slice(&n.to_le_bytes());
            }
            crate::executor::Value::String(s) => {
                buf.push(TAG_STRING);
                buf.extend_from_slice(&(s.len() as u16).to_le_bytes());
                buf.extend_from_slice(s.as_bytes());
            }
            crate::executor::Value::Null => {
                buf.push(TAG_NULL);
            }
            crate::executor::Value::Float(f) => {
                buf.push(TAG_FLOAT);
                buf.extend_from_slice(&f.to_le_bytes());
            }
            crate::executor::Value::Bool(b) => {
                buf.push(TAG_BOOL);
                buf.push(if *b { 1 } else { 0 });
            }
            crate::executor::Value::Date(d) => {
                buf.push(TAG_DATE);
                buf.extend_from_slice(&d.to_le_bytes());
            }
            crate::executor::Value::Timestamp(t) => {
                buf.push(TAG_TIMESTAMP);
                buf.extend_from_slice(&t.to_le_bytes());
            }
        }
    }
    buf
}

pub(crate) fn deserialize_catalog_column_row(data: &[u8]) -> Result<CatalogColumnRow> {
    let mut p = 0usize;
    if data.len() < 2 {
        return Err(StorageError::Internal(
            "catalog column row too short (table_name_len)".into(),
        ));
    }
    let tn_len = u16::from_le_bytes([data[p], data[p + 1]]) as usize;
    p += 2;
    if data.len() < p + tn_len {
        return Err(StorageError::Internal(
            "catalog column row truncated (table_name)".into(),
        ));
    }
    let table_name = std::str::from_utf8(&data[p..p + tn_len])
        .map_err(|e| StorageError::Internal(format!("invalid utf8 table_name: {e}")))?
        .to_string();
    p += tn_len;
    if data.len() < p + 4 {
        return Err(StorageError::Internal(
            "catalog column row truncated (column_index)".into(),
        ));
    }
    let column_index = u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]);
    p += 4;
    if data.len() < p + 2 {
        return Err(StorageError::Internal(
            "catalog column row truncated (column_name_len)".into(),
        ));
    }
    let cn_len = u16::from_le_bytes([data[p], data[p + 1]]) as usize;
    p += 2;
    if data.len() < p + cn_len {
        return Err(StorageError::Internal(
            "catalog column row truncated (column_name)".into(),
        ));
    }
    let column_name = std::str::from_utf8(&data[p..p + cn_len])
        .map_err(|e| StorageError::Internal(format!("invalid utf8 column_name: {e}")))?
        .to_string();
    p += cn_len;
    if data.len() < p + 1 {
        return Err(StorageError::Internal(
            "catalog column row truncated (tag)".into(),
        ));
    }
    let tag = data[p];
    p += 1;
    let column_type = match tag {
        COL_TAG_INT => ColumnType::Int,
        COL_TAG_FLOAT => ColumnType::Float,
        COL_TAG_BOOL => ColumnType::Bool,
        COL_TAG_DATE => ColumnType::Date,
        COL_TAG_TIMESTAMP => ColumnType::Timestamp,
        COL_TAG_STRING => {
            if data.len() < p + 2 {
                return Err(StorageError::Internal(
                    "catalog column row truncated (string max_len)".into(),
                ));
            }
            let max_len = u16::from_le_bytes([data[p], data[p + 1]]);
            p += 2;
            ColumnType::String(max_len)
        }
        other => {
            return Err(StorageError::Internal(format!(
                "unknown column type tag {other:#x}"
            )))
        }
    };
    if data.len() < p + 2 {
        return Err(StorageError::Internal(
            "catalog column row truncated (flags)".into(),
        ));
    }
    let not_null = data[p] != 0;
    let unique = data[p + 1] != 0;
    p += 2;
    // MS24 Iter000 (D1) compat read: the trailing DEFAULT section
    // (`u8 has_default | [tag | payload]`) is optional. A legacy row (or a
    // `has_default = 0` block) reads as `None` — same shape as the unique-
    // roots tail read and the 24B checkpoint-site compat read.
    let mut default_value = None;
    if p < data.len() {
        let has_default = data[p];
        p += 1;
        if has_default == 1 {
            if data.len() < p + 1 {
                return Err(StorageError::Internal(
                    "catalog column row truncated (default tag)".into(),
                ));
            }
            let tag = data[p];
            p += 1;
            let read_i64 = |data: &[u8], p: usize| -> Result<i64> {
                if data.len() < p + 8 {
                    return Err(StorageError::Internal(
                        "catalog column row truncated (default payload)".into(),
                    ));
                }
                Ok(i64::from_le_bytes([
                    data[p],
                    data[p + 1],
                    data[p + 2],
                    data[p + 3],
                    data[p + 4],
                    data[p + 5],
                    data[p + 6],
                    data[p + 7],
                ]))
            };
            default_value = Some(match tag {
                TAG_INT => crate::executor::Value::Int(read_i64(data, p)?),
                TAG_STRING => {
                    if data.len() < p + 2 {
                        return Err(StorageError::Internal(
                            "catalog column row truncated (default payload)".into(),
                        ));
                    }
                    let len = u16::from_le_bytes([data[p], data[p + 1]]) as usize;
                    p += 2;
                    if data.len() < p + len {
                        return Err(StorageError::Internal(
                            "catalog column row truncated (default payload)".into(),
                        ));
                    }
                    let s = std::str::from_utf8(&data[p..p + len])
                        .map_err(|e| {
                            StorageError::Internal(format!("invalid utf8 default value: {e}"))
                        })?
                        .to_string();
                    crate::executor::Value::String(s)
                }
                TAG_NULL => crate::executor::Value::Null,
                TAG_FLOAT => {
                    crate::executor::Value::Float(f64::from_bits(read_i64(data, p)? as u64))
                }
                TAG_BOOL => {
                    if data.len() < p + 1 {
                        return Err(StorageError::Internal(
                            "catalog column row truncated (default payload)".into(),
                        ));
                    }
                    crate::executor::Value::Bool(data[p] != 0)
                }
                TAG_DATE => {
                    if data.len() < p + 4 {
                        return Err(StorageError::Internal(
                            "catalog column row truncated (default payload)".into(),
                        ));
                    }
                    crate::executor::Value::Date(i32::from_le_bytes([
                        data[p],
                        data[p + 1],
                        data[p + 2],
                        data[p + 3],
                    ]))
                }
                TAG_TIMESTAMP => crate::executor::Value::Timestamp(read_i64(data, p)?),
                other => {
                    return Err(StorageError::Internal(format!(
                        "unknown default value tag {other:#x}"
                    )))
                }
            });
        }
    }
    Ok(CatalogColumnRow {
        table_name,
        column_index,
        column_name,
        column_type,
        not_null,
        unique,
        default_value,
    })
}

pub(crate) fn matches_catalog_column_row_table_name(data: &[u8], name: &str) -> bool {
    if data.len() < 2 {
        return false;
    }
    let tn_len = u16::from_le_bytes([data[0], data[1]]) as usize;
    if data.len() < 2 + tn_len {
        return false;
    }
    &data[2..2 + tn_len] == name.as_bytes()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::FileStorage;
    use tempfile::tempdir;

    fn sample_row() -> CatalogRow {
        CatalogRow {
            table_name: "users".to_string(),
            data_page_head: 7,
            index_root_page_id: 9,
            pk_index: 0,
            pk_column: "id".to_string(),
            column_count: 2,
            data_page_tail: 7,
            unique_roots: Vec::new(),
        }
    }

    fn sample_col(name: &str, idx: u32, ty: ColumnType) -> CatalogColumnRow {
        CatalogColumnRow {
            table_name: "users".to_string(),
            column_index: idx,
            column_name: name.to_string(),
            column_type: ty,
            not_null: false,
            unique: false,
            default_value: None,
        }
    }

    #[test]
    fn row_serde_roundtrip() {
        let row = sample_row();
        let bytes = serialize_catalog_row(&row);
        let back = deserialize_catalog_row(&bytes).unwrap();
        assert_eq!(row, back);
    }

    #[test]
    fn column_row_serde_roundtrip_int() {
        let col = sample_col("id", 0, ColumnType::Int);
        let bytes = serialize_catalog_column_row(&col);
        let back = deserialize_catalog_column_row(&bytes).unwrap();
        assert_eq!(col, back);
    }

    #[test]
    fn column_row_serde_roundtrip_datetime() {
        for ty in [ColumnType::Date, ColumnType::Timestamp] {
            let col = sample_col("d", 1, ty);
            let bytes = serialize_catalog_column_row(&col);
            let back = deserialize_catalog_column_row(&bytes).unwrap();
            assert_eq!(col, back);
        }
    }

    #[test]
    fn column_row_serde_roundtrip_string() {
        let col = sample_col("name", 1, ColumnType::String(255));
        let bytes = serialize_catalog_column_row(&col);
        let back = deserialize_catalog_column_row(&bytes).unwrap();
        assert_eq!(col, back);
    }

    #[test]
    fn column_row_serde_roundtrip_float_bool() {
        let c1 = sample_col("score", 0, ColumnType::Float);
        let c2 = sample_col("active", 1, ColumnType::Bool);
        assert_eq!(
            c1,
            deserialize_catalog_column_row(&serialize_catalog_column_row(&c1)).unwrap()
        );
        assert_eq!(
            c2,
            deserialize_catalog_column_row(&serialize_catalog_column_row(&c2)).unwrap()
        );
    }

    #[test]
    fn matches_row_name() {
        let row = sample_row();
        let bytes = serialize_catalog_row(&row);
        assert!(matches_catalog_row_name(&bytes, "users"));
        assert!(!matches_catalog_row_name(&bytes, "other"));
    }

    #[tokio::test]
    async fn bootstrap_and_open_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let storage = Arc::new(FileStorage::open(&path).unwrap());
        let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

        let cat = Catalog::bootstrap(bp.clone(), storage.clone())
            .await
            .unwrap();
        let rows = cat.scan_tables().await.unwrap();
        assert_eq!(rows.len(), 0);
    }

    #[tokio::test]
    async fn insert_and_scan_one_table() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let storage = Arc::new(FileStorage::open(&path).unwrap());
        let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

        let cat = Catalog::bootstrap(bp.clone(), storage.clone())
            .await
            .unwrap();
        let row = sample_row();
        let cols = vec![
            sample_col("id", 0, ColumnType::Int),
            sample_col("name", 1, ColumnType::String(255)),
        ];
        cat.insert_table(&row, &cols).await.unwrap();

        let scanned = cat.scan_tables().await.unwrap();
        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0], row);

        let scanned_cols = cat.scan_columns("users").await.unwrap();
        assert_eq!(scanned_cols, cols);
    }

    #[tokio::test]
    async fn insert_and_delete() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let storage = Arc::new(FileStorage::open(&path).unwrap());
        let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

        let cat = Catalog::bootstrap(bp.clone(), storage.clone())
            .await
            .unwrap();
        let row = sample_row();
        let cols = vec![sample_col("id", 0, ColumnType::Int)];
        cat.insert_table(&row, &cols).await.unwrap();
        assert_eq!(cat.scan_tables().await.unwrap().len(), 1);

        cat.delete_table("users").await.unwrap();
        assert_eq!(cat.scan_tables().await.unwrap().len(), 0);
        assert_eq!(cat.scan_columns("users").await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn update_table_tail() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let storage = Arc::new(FileStorage::open(&path).unwrap());
        let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

        let cat = Catalog::bootstrap(bp.clone(), storage.clone())
            .await
            .unwrap();
        let mut row = sample_row();
        cat.insert_table(&row, &[]).await.unwrap();

        cat.update_table_tail("users", 42).await.unwrap();
        row.data_page_tail = 42;
        let scanned = cat.scan_tables().await.unwrap();
        assert_eq!(scanned[0], row);
    }

    // ===========================================================================
    // MS23 Iteration 001 (2.1): catalog 表行尾随追加唯一索引根页
    // ===========================================================================

    /// 新格式往返：尾随 `u32 count | N × u32 roots` 段随行序列化并读回。
    #[test]
    fn catalog_row_with_unique_roots_roundtrip() {
        let mut row = sample_row();
        row.unique_roots = vec![41, 42];
        let bytes = serialize_catalog_row(&row);
        let back = deserialize_catalog_row(&bytes).unwrap();
        assert_eq!(row, back);
        assert_eq!(back.unique_roots, vec![41, 42]);
    }

    /// 旧格式兼容：无尾随段的行（旧版写入）读为空 unique_roots。
    #[test]
    fn legacy_layout_row_deserializes_with_empty_unique_roots() {
        // 按旧版固定布局手工拼字节：u16 name_len | name | u32 head |
        // u32 idx_root | u32 pk_index | u16 pk_len | pk | u32 count | u32 tail
        let name = b"users";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&7u32.to_le_bytes());
        bytes.extend_from_slice(&9u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(b"id");
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&7u32.to_le_bytes());

        let row = deserialize_catalog_row(&bytes).unwrap();
        assert_eq!(row.table_name, "users");
        assert_eq!(row.index_root_page_id, 9);
        assert!(
            row.unique_roots.is_empty(),
            "legacy row must deserialize with empty unique_roots, got {:?}",
            row.unique_roots
        );
    }

    /// `update_table_root` 的 deserialize→mutate→serialize 往返必须保住
    /// 尾随追加字段（写回根不丢唯一根）。
    #[tokio::test]
    async fn update_table_root_preserves_unique_roots() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let storage = Arc::new(FileStorage::open(&path).unwrap());
        let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

        let cat = Catalog::bootstrap(bp.clone(), storage.clone())
            .await
            .unwrap();
        let mut row = sample_row();
        row.unique_roots = vec![7, 8];
        cat.insert_table(&row, &[]).await.unwrap();

        cat.update_table_root("users", 99).await.unwrap();
        let scanned = cat.scan_tables().await.unwrap();
        assert_eq!(scanned[0].index_root_page_id, 99);
        assert_eq!(
            scanned[0].unique_roots,
            vec![7, 8],
            "root write-back must preserve appended unique roots"
        );
    }

    /// `update_unique_index_root` 只改目标 ordinal 位，其余字段（含其他唯一根）不变。
    #[tokio::test]
    async fn update_unique_index_root_changes_only_target_ordinal() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let storage = Arc::new(FileStorage::open(&path).unwrap());
        let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

        let cat = Catalog::bootstrap(bp.clone(), storage.clone())
            .await
            .unwrap();
        let mut row = sample_row();
        row.unique_roots = vec![7, 8];
        cat.insert_table(&row, &[]).await.unwrap();

        cat.update_unique_index_root("users", 1, 99).await.unwrap();
        let scanned = cat.scan_tables().await.unwrap();
        assert_eq!(scanned[0].unique_roots, vec![7, 99]);
        assert_eq!(scanned[0].index_root_page_id, 9, "PK root untouched");
        assert_eq!(scanned[0].data_page_tail, 7, "other fields untouched");
    }

    #[tokio::test]
    async fn reserved_table_name_check() {
        // Direct check: the names __tables / __columns should match the
        // public constants. (Exposed here so the assertion is in the
        // catalog module's own test suite.)
        assert_eq!(TABLES_SYSTEM_NAME, "__tables");
        assert_eq!(COLUMNS_SYSTEM_NAME, "__columns");
    }

    // ===========================================================================
    // MS24 Iteration 000 (1.4/D1): catalog 列行 DEFAULT 尾段序列化
    // ===========================================================================

    /// 新格式往返：七变体 DEFAULT 值（含 DEFAULT NULL 保真）随行序列化并读回。
    #[test]
    fn column_row_with_default_roundtrip_all_variants() {
        let cases = [
            (ColumnType::Int, crate::executor::Value::Int(-42)),
            (
                ColumnType::String(255),
                crate::executor::Value::String("anon".to_string()),
            ),
            (ColumnType::Int, crate::executor::Value::Null),
            (ColumnType::Float, crate::executor::Value::Float(2.5)),
            (ColumnType::Bool, crate::executor::Value::Bool(true)),
            (ColumnType::Date, crate::executor::Value::Date(739_000)),
            (
                ColumnType::Timestamp,
                crate::executor::Value::Timestamp(1_758_739_200_000_000),
            ),
        ];
        for (ty, value) in cases {
            let mut col = sample_col("c", 1, ty);
            col.default_value = Some(value.clone());
            let bytes = serialize_catalog_column_row(&col);
            let back = deserialize_catalog_column_row(&bytes).unwrap();
            assert_eq!(col, back, "roundtrip failed for {value:?}");
            assert_eq!(back.default_value, Some(value));
        }
    }

    /// 无 DEFAULT 行输出与旧格式逐字节一致（Preserve：兼容读 + 字节面不变）。
    #[test]
    fn no_default_row_output_is_byte_identical_to_legacy() {
        let col = sample_col("c", 1, ColumnType::String(255));
        let bytes = serialize_catalog_column_row(&col);
        // 手工拼旧版固定布局字节流并比较
        let mut legacy = Vec::new();
        legacy.extend_from_slice(&("users".len() as u16).to_le_bytes());
        legacy.extend_from_slice(b"users");
        legacy.extend_from_slice(&1u32.to_le_bytes());
        legacy.extend_from_slice(&("c".len() as u16).to_le_bytes());
        legacy.extend_from_slice(b"c");
        legacy.push(COL_TAG_STRING);
        legacy.extend_from_slice(&255u16.to_le_bytes());
        legacy.push(0); // not_null
        legacy.push(0); // unique
        assert_eq!(
            bytes, legacy,
            "no-default row must not carry a tail section"
        );
    }

    /// 旧格式兼容：无尾随段（及 has_default=0 形态）读为 None。
    #[test]
    fn legacy_column_row_without_default_reads_none() {
        // 旧版固定布局：… u8 not_null | u8 unique（无尾随段）
        let mut legacy = Vec::new();
        legacy.extend_from_slice(&(5u16).to_le_bytes());
        legacy.extend_from_slice(b"users");
        legacy.extend_from_slice(&0u32.to_le_bytes());
        legacy.extend_from_slice(&(2u16).to_le_bytes());
        legacy.extend_from_slice(b"id");
        legacy.push(COL_TAG_INT);
        legacy.push(0);
        legacy.push(0);
        let row = deserialize_catalog_column_row(&legacy).unwrap();
        assert_eq!(row.default_value, None, "legacy row must read default None");

        // 防御形态：has_default=0 → None
        let mut zero_flag = legacy.clone();
        zero_flag.push(0);
        let row = deserialize_catalog_column_row(&zero_flag).unwrap();
        assert_eq!(
            row.default_value, None,
            "has_default=0 must read default None"
        );
    }
}
