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

use crate::storage::data_page::page_write_error_to_storage_error;
use crate::storage::page_format::{
    ColumnType, PageWriteError, Slot, SlottedPage, SlottedPageRef, MAX_PAGE_SLOTS, TAG_BOOL,
    TAG_DATE, TAG_FLOAT, TAG_INT, TAG_LONG_STRING, TAG_NULL, TAG_STRING, TAG_TIMESTAMP,
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

/// One secondary-index registration in a `__tables` row's trailing
/// section (MS21 Iteration 002, design D4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSecondaryIndex {
    /// Index name (library-level unique).
    pub name: String,
    /// Indexed column ordinal within the table's column list.
    pub column_index: u32,
    /// B-Tree root page of this index tree.
    pub root: u32,
}

/// Kind tag byte for a base-table row in the `__tables` trailing kind
/// section (MS21 Iteration 005, design D12).
const ROW_KIND_TABLE: u8 = 0x01;
/// Kind tag byte for a view-definition row (MS21 Iteration 005, D12).
const ROW_KIND_VIEW: u8 = 0x02;

/// MS21 Iteration 005 (T5-a/D12): what a `__tables` row defines. Legacy
/// rows (written before the trailing kind section existed) deserialize as
/// [`CatalogKind::Table`] — base-table semantics are unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogKind {
    /// A base table (data pages, index trees, rows).
    Table,
    /// A persisted read-only view definition (no data pages, no index
    /// trees — the row carries the definition SQL text tail instead).
    View,
}

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
    /// MS21 Iteration 002 (D4): registered secondary (user CREATE INDEX)
    /// indexes, in creation order. Appended after the unique-roots section;
    /// a row without that trailing section deserializes as empty (compat
    /// read — no secondary indexes).
    pub secondary_indexes: Vec<CatalogSecondaryIndex>,
    /// MS21 Iteration 005 (T5-a/D12): row kind, carried in the trailing
    /// kind section. A legacy row (no kind byte) reads as `Table`.
    pub kind: CatalogKind,
    /// MS21 Iteration 005 (T5-a/D12): view definition SQL text (kind=View
    /// rows only, `u16 len | text` tail). `None` for base tables.
    pub view_sql: Option<String>,
    /// True only when the creating statement wrote `PRIMARY KEY`.
    /// A missing trailing byte, including every legacy row, reads as false.
    pub explicit_primary_key: bool,
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
    ///
    /// change 2026-10-13 T3 (D4/D5): column rows are written FIRST and the
    /// `__tables` row LAST — the table row is the create-statement commit
    /// point: a reopened catalog that can see the table row is guaranteed
    /// to also see every column row it references. On success the ordered
    /// statement-boundary flush runs (referenced pages first, commit page
    /// last) so "DDL succeeded" implies "definition durable" even if the
    /// process dies before any checkpoint.
    pub async fn insert_table(
        &self,
        meta: &CatalogRow,
        columns: &[CatalogColumnRow],
    ) -> Result<()> {
        let _guard = self.lock.lock().await;

        // Column rows first, in declaration order.
        for col in columns {
            let payload = serialize_catalog_column_row(col);
            let columns_root = *self.columns_root.lock().await;
            append_to_chain(&self.buffer_pool, &self.storage, columns_root, &payload).await?;
            *self.columns_root.lock().await = chain_head(&self.buffer_pool, columns_root).await?;
        }

        // The table row last: its existence on disk is the commit point.
        let payload = serialize_catalog_row(meta);
        let tables_root = *self.tables_root.lock().await;
        let prev_tail = chain_head(&self.buffer_pool, tables_root).await?;
        let (table_page, _lid) =
            append_to_chain(&self.buffer_pool, &self.storage, tables_root, &payload).await?;
        // Note: the new head page is recorded by `append_to_chain` in the
        // root if a fresh page had to be allocated. Re-read in that case.
        *self.tables_root.lock().await = chain_head(&self.buffer_pool, tables_root).await?;

        // Ordered flush: everything else first, then the commit page(s). If
        // the table row landed on a freshly-linked page, the previous tail
        // page's link bytes are part of the commit point too — without them
        // a reopened chain walk would stop one page short of the new row.
        let mut commit_pages = vec![table_page];
        if table_page != prev_tail {
            commit_pages.push(prev_tail);
        }
        self.flush_commit_point(&commit_pages).await?;

        Ok(())
    }

    /// change 2026-10-13 T3 (D4): DDL statement-boundary ordered flush. Every
    /// dirty page EXCEPT the commit page(s) is written first, then the commit
    /// page(s) in order. A crash mid-sequence then never leaves a durable
    /// catalog row pointing at pages that never reached disk: the commit row
    /// is either absent (statement视同未发生) or fully consistent.
    async fn flush_commit_point(&self, commit_pages: &[PageId]) -> Result<()> {
        let exclude: std::collections::HashSet<PageId> = commit_pages.iter().copied().collect();
        self.buffer_pool.flush_all_except(&exclude).await?;
        self.buffer_pool.flush_pages(commit_pages).await?;
        Ok(())
    }

    /// Delete all rows for `table_name` from both chains. Returns the pages
    /// that were actually modified (empty when the table does not exist).
    /// Idempotent; physical page free is out of scope.
    pub async fn delete_table(&self, name: &str) -> Result<Vec<PageId>> {
        let _guard = self.lock.lock().await;
        let mut changed = delete_from_chain(
            &self.buffer_pool,
            &self.storage,
            PageId(TABLES_PAGE_ID),
            |data| matches_catalog_row_name(data, name),
        )
        .await?;
        changed.extend(
            delete_from_chain(
                &self.buffer_pool,
                &self.storage,
                PageId(COLUMNS_PAGE_ID),
                |data| matches_catalog_column_row_table_name(data, name),
            )
            .await?,
        );
        Ok(changed)
    }

    /// Scan all `__tables` rows.
    ///
    /// Same-name copies collapse to the newest one (an append always allocates
    /// a greater logical_id), which closes the crash window of the
    /// variable-length catalog update — appending the new copy and removing the
    /// stale one are two steps. Chain order is otherwise preserved.
    pub async fn scan_tables(&self) -> Result<Vec<CatalogRow>> {
        let _guard = self.lock.lock().await;
        let rows = scan_chain(&self.buffer_pool, PageId(TABLES_PAGE_ID), |data| {
            deserialize_catalog_row(data)
        })
        .await?;
        Ok(collapse_newest_same_name(rows))
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
            .map(|(_, col)| col)
            .filter(|c| c.table_name == table_name)
            .collect())
    }

    /// MS35 (`free-page-lifecycle` R2): collect every page the chain
    /// starting at `root` occupies (walk `next_page_id` until 0).
    ///
    /// Read-only. A page read failure is returned as an error — the free-page
    /// rebuild treats any traversal error as "abandon the whole rebuild"
    /// (R3), so this helper must not degrade to a partial result.
    pub async fn collect_chain_pages(&self, root: PageId) -> Result<Vec<PageId>> {
        let _guard = self.lock.lock().await;
        let mut pages = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut current = Some(root);
        while let Some(page_id) = current {
            if !visited.insert(page_id.0) {
                // Defensive: `next_page_id` cycle — stop instead of looping.
                break;
            }
            pages.push(page_id);
            let next = self
                .buffer_pool
                .with_page_data(page_id, |data| {
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
        Ok(pages)
    }

    /// MS21 Iter003 (T3-b/D9): append one column row to `__columns` for the
    /// ALTER TABLE ADD COLUMN write. The row carries its own table name and
    /// trailing `column_index` (chain append preserves the chain-order
    /// invariant: a tail-added column is the last in the chain).
    pub async fn insert_column_row(&self, row: &CatalogColumnRow) -> Result<()> {
        let _guard = self.lock.lock().await;
        let payload = serialize_catalog_column_row(row);
        let columns_root = *self.columns_root.lock().await;
        append_to_chain(&self.buffer_pool, &self.storage, columns_root, &payload).await?;
        *self.columns_root.lock().await = chain_head(&self.buffer_pool, columns_root).await?;
        Ok(())
    }

    /// MS21 Iter005 (T5-a/D12): insert a view-definition row (a `__tables`
    /// row with kind=View carrying the SQL text tail) plus its column-name
    /// rows. The row follows the view-row field conventions: zero page
    /// pointers and empty PK (views are non-materialized — no data pages,
    /// no index trees); `column_count` = declared column count.
    pub async fn insert_view_row(
        &self,
        name: &str,
        sql: &str,
        columns: &[CatalogColumnRow],
    ) -> Result<()> {
        let meta = CatalogRow {
            table_name: name.to_string(),
            data_page_head: 0,
            index_root_page_id: 0,
            pk_index: 0,
            pk_column: String::new(),
            column_count: columns.len() as u32,
            data_page_tail: 0,
            unique_roots: Vec::new(),
            secondary_indexes: Vec::new(),
            kind: CatalogKind::View,
            view_sql: Some(sql.to_string()),
            explicit_primary_key: false,
        };
        self.insert_table(&meta, columns).await
    }

    /// MS21 Iter005 (T5-a/D12): scan all view-definition rows.
    pub async fn scan_views(&self) -> Result<Vec<CatalogRow>> {
        Ok(self
            .scan_tables()
            .await?
            .into_iter()
            .filter(|r| r.kind == CatalogKind::View)
            .collect())
    }

    /// change 2026-10-13 T3 (D4/D5): the ALTER TABLE column-count rewrite on
    /// the `__tables` row is the statement commit point: the column rows must
    /// be durable BEFORE the count that advertises them, so the updated row's
    /// page is flushed last (referenced pages first).
    ///
    /// change 2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog /
    /// Iteration 001 (D5): ALTER TABLE DROP COLUMN uses this method too — its
    /// catalog narrowing moved to the statement's commit point (ISS21), so the
    /// drop applies the column rows first and the count last, the same order
    /// as ADD COLUMN. The intermediate state of a crash inside the drop's
    /// commit point is the narrowed column rows with the not-yet-lowered
    /// count: the reopen path derives the live layout from the column rows
    /// (`open_or_init` truncates to the count), so the narrowed layout stays
    /// readable.
    pub async fn update_table_column_count_ddl(&self, name: &str, new_count: u32) -> Result<()> {
        let _guard = self.lock.lock().await;
        let page = update_field_in_chain(
            &self.buffer_pool,
            &self.storage,
            PageId(TABLES_PAGE_ID),
            name,
            |data| {
                let mut row = deserialize_catalog_row(data)?;
                if row.table_name != name {
                    return Ok(None);
                }
                row.column_count = new_count;
                Ok(Some(serialize_catalog_row(&row)))
            },
        )
        .await?;
        if let Some(page) = page {
            self.flush_commit_point(&[page]).await?;
        }
        Ok(())
    }

    /// change 2026-10-25-ms37-defect-ledger-closure
    /// Iteration 006 / Cycle 000（T7 / ISS28）：**非刷盘**的列数恢复入口——
    /// 把 `__tables` 行的 `column_count` 恢复为语句执行前的值。
    ///
    /// 回退**不做存储 IO**（只改缓冲池页）⇒ 在存储正在失败时也能完成 ⇒ 补偿
    /// 本身不成为新的失败源。与 `update_table_column_count_ddl` 的差别**只有**
    /// 「不刷盘」——既有方法的刷盘语义零改动。
    pub async fn restore_table_column_count_no_flush(&self, name: &str, count: u32) -> Result<()> {
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
                row.column_count = count;
                Ok(Some(serialize_catalog_row(&row)))
            },
        )
        .await
        .map(|_| ())
    }

    /// change 2026-10-25-ms37-defect-ledger-closure
    /// Iteration 006 / Cycle 000（T7 / ISS28）：移除 `__columns` 链上**指定
    /// 序号**的列行（非刷盘）——回退「本次新增的列」用。
    ///
    /// 语义 = `rewrite_column_rows_drop` 的「删除指定序号列行」那一步，**不含**
    /// 其后的序号平移（新增的列是末列，其后没有列行）。`rewrite_column_rows_drop`
    /// 本身**零改动**。
    pub async fn remove_column_row_at(&self, table: &str, column_index: u32) -> Result<()> {
        let _guard = self.lock.lock().await;
        delete_from_chain(
            &self.buffer_pool,
            &self.storage,
            PageId(COLUMNS_PAGE_ID),
            |data| {
                matches_column_row_of_table(data, table)
                    && matches_column_row_index(data, column_index)
            },
        )
        .await
        .map(|_| ())
    }

    /// 放弃一次已经刷盘成功的 `ADD COLUMN`：在同一把目录锁里先恢复列数、再移除
    /// 本次列行，然后只刷列数页，再刷列行改过的页。
    ///
    /// 列数页是 `update_field_in_chain` 返回的那一页，列行页是 `delete_from_chain`
    /// 返回的列表。两次 `flush_pages` 都返回后才释放锁。失败补偿仍走各自不刷盘的入口。
    pub(crate) async fn rollback_abandoned_add_column(
        &self,
        name: &str,
        column_index: u32,
        count_before: u32,
    ) -> Result<()> {
        let _guard = self.lock.lock().await;
        let count_page = update_field_in_chain(
            &self.buffer_pool,
            &self.storage,
            PageId(TABLES_PAGE_ID),
            name,
            |data| {
                let mut row = deserialize_catalog_row(data)?;
                if row.table_name != name {
                    return Ok(None);
                }
                row.column_count = count_before;
                Ok(Some(serialize_catalog_row(&row)))
            },
        )
        .await?;
        let column_pages = delete_from_chain(
            &self.buffer_pool,
            &self.storage,
            PageId(COLUMNS_PAGE_ID),
            |data| {
                matches_column_row_of_table(data, name)
                    && matches_column_row_index(data, column_index)
            },
        )
        .await?;
        let count_pages: Vec<PageId> = count_page.into_iter().collect();
        self.buffer_pool.flush_pages(&count_pages).await?;
        self.buffer_pool.flush_pages(&column_pages).await?;
        Ok(())
    }

    /// Update the `data_page_tail` field of an existing row in `__tables`.
    ///
    /// No-op if the table does not exist.
    pub async fn update_table_tail(&self, name: &str, new_tail: u32) -> Result<Option<PageId>> {
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
    pub async fn update_table_root(&self, name: &str, new_root: u32) -> Result<Option<PageId>> {
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
    ) -> Result<Option<PageId>> {
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

    /// MS21 Iteration 002 (D4): update the Nth secondary index root of an
    /// existing row in `__tables` (`ordinal` = position in the row's
    /// `secondary_indexes` section, the same order the roots are serialized
    /// in). Same deserialize→mutate→serialize shape as
    /// [`Self::update_unique_index_root`].
    ///
    /// No-op if the table does not exist.
    pub async fn update_secondary_index_root(
        &self,
        name: &str,
        ordinal: usize,
        new_root: u32,
    ) -> Result<Option<PageId>> {
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
                if ordinal >= row.secondary_indexes.len() {
                    return Err(StorageError::Internal(format!(
                        "update_secondary_index_root: ordinal {} out of range for table '{}' ({})",
                        ordinal,
                        name,
                        row.secondary_indexes.len()
                    )));
                }
                row.secondary_indexes[ordinal].root = new_root;
                Ok(Some(serialize_catalog_row(&row)))
            },
        )
        .await
    }

    /// MS21 Iteration 002 (D4/D5): append one secondary-index registration to
    /// an existing row in `__tables` (the CREATE INDEX catalog write, performed
    /// only after the tree was built and backfilled — CTAS
    /// 「建表→物化→登记」 ordering precedent).
    ///
    /// change 2026-10-13 T3 (D4): the updated `__tables` copy is the CREATE
    /// INDEX commit point — its page is flushed last (the built tree's pages
    /// first), so a reopen either sees the registration with a durable tree
    /// or no registration at all.
    ///
    /// No-op if the table does not exist.
    pub async fn append_secondary_index(
        &self,
        name: &str,
        entry: &CatalogSecondaryIndex,
    ) -> Result<()> {
        let _guard = self.lock.lock().await;
        let page = update_field_in_chain(
            &self.buffer_pool,
            &self.storage,
            PageId(TABLES_PAGE_ID),
            name,
            |data| {
                let mut row = deserialize_catalog_row(data)?;
                if row.table_name != name {
                    return Ok(None);
                }
                row.secondary_indexes.push(entry.clone());
                Ok(Some(serialize_catalog_row(&row)))
            },
        )
        .await?;
        if let Some(page) = page {
            self.flush_commit_point(&[page]).await?;
        }
        Ok(())
    }

    /// MS21 Iteration 002 (D4): remove the named secondary-index registration
    /// from an existing row in `__tables` (DROP INDEX). The remaining entries
    /// keep their relative order; positions after the removed one shift down
    /// by one (callers re-attach `CatalogRootSlot::Secondary` ordinals).
    ///
    /// No-op if the table or index does not exist.
    pub async fn remove_secondary_index(
        &self,
        name: &str,
        index_name: &str,
    ) -> Result<Option<PageId>> {
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
                row.secondary_indexes.retain(|e| e.name != index_name);
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

/// Raw rows of one page as `(logical_id, row bytes)`, paired with the next
/// page-id in the chain.
type PageRows = (Vec<(u16, Vec<u8>)>, u32);

/// A located catalog row to update: `(logical_id, current length, new bytes)`.
type LocatedUpdate = Option<(u16, usize, Vec<u8>)>;

/// Append `payload` as a new slot in the chain starting at `root`. If the
/// last page is full, allocate a new page, init it, link it from the
/// previous page, and write into the new page. Returns the `(page, logical_id)`
/// the payload landed on.
async fn append_to_chain(
    bp: &BufferPool,
    storage: &Arc<dyn AsyncStorage>,
    root: PageId,
    payload: &[u8],
) -> Result<(PageId, u16)> {
    let last = chain_head(bp, root).await?;

    // Try to append to the last page first.
    let add_result: std::result::Result<(u16, usize), PageWriteError> = {
        let guard = bp.get_page(last).await?;
        guard.modify_page(|page| {
            let mut slotted = SlottedPage::new(page);
            slotted.add_slot(payload)
        })
    };

    match add_result {
        Ok((lid, _)) => Ok((last, lid)),
        // 页满 —— allocate a fresh page, init, link, write.
        Err(PageWriteError::PageFull) => {
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
            let write_result: std::result::Result<(u16, usize), PageWriteError> = new_guard
                .modify_page(|page| {
                    let mut slotted = SlottedPage::new(page);
                    slotted.add_slot(payload)
                });
            let (lid, _) = match write_result {
                Ok(v) => v,
                Err(e) => return Err(page_write_error_to_storage_error(new_id.0, e)),
            };
            // The freshly written page stays cached and dirty: evicting it
            // here would drop the row before it reaches disk (and push the
            // page-id onto the storage free list, where a later allocation
            // could hand the same id to an unrelated chain). The normal flush
            // paths (checkpoint full flush / eviction) persist it.
            Ok((new_id, lid))
        }
        // 页损坏 —— **不换页继续**：以点名该页的错误结束写入（损坏页被
        // 跳过继续写会让目录行静默丢失）。
        Err(e) => Err(page_write_error_to_storage_error(last.0, e)),
    }
}

/// Walk a chain starting at `root`, calling `parse` on each row's bytes and
/// collecting `(logical_id, parsed)` in chain order.
async fn scan_chain<P, T>(bp: &BufferPool, root: PageId, parse: P) -> Result<Vec<(u16, T)>>
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
            .with_page_data(page_id, |data| -> Result<PageRows> {
                let slotted = SlottedPageRef::new(data);
                check_catalog_page_slot_count(&slotted, page_id.0)?;
                let mut rows = Vec::with_capacity(slotted.slot_count());
                for i in 0..slotted.slot_count() {
                    if let Some(slot) = slotted.get_slot(i) {
                        let bytes = catalog_slot_payload(&slotted, page_id.0, i, &slot)?;
                        rows.push((slot.logical_id, bytes.to_vec()));
                    }
                }
                Ok((rows, slotted.header().next_page_id))
            })
            .await?;
        for (lid, row_bytes) in rows {
            out.push((lid, parse(&row_bytes)?));
        }
        current = if next == 0 {
            None
        } else {
            Some(PageId(next as u64))
        };
    }
    Ok(out)
}

/// 目录页的**页级**损坏错误——点名该页、页头声称的槽数与槽目录容量上限。
///
/// 页头声称的槽数超出槽目录容量时，槽目录定位
/// `PAGE_SIZE - (index + 1) * Slot::SIZE` 会整数下溢 ⇒ 取槽即越界切片 panic
/// ⇒ 进程 abort。故读点必须在**取槽之前**问该判据。
fn corrupted_catalog_page_slot_count_error(page_id: u64, slot_count: usize) -> StorageError {
    StorageError::CorruptedCatalogPage {
        page_id,
        detail: format!(
            "slot count {slot_count} exceeds the page slot directory capacity of {MAX_PAGE_SLOTS} slots"
        ),
    }
}

/// 目录页的**槽级**损坏错误——点名该页与该槽（含越界区间）。
///
/// 损坏的目录页**绝不**被跳过：跳过会让确实存在的表得到「表不存在」的答案
/// （静默错读）⇒ 目录损坏 ⇒ 该库 SHALL NOT 打开。负载读不出来 ⇒ 只点名该页
/// 与该槽，**不臆造表名**。
fn corrupted_catalog_page_slot_error(page_id: u64, slot_index: usize, slot: &Slot) -> StorageError {
    let start = slot.offset as usize;
    let end = start.saturating_add(slot.length as usize);
    StorageError::CorruptedCatalogPage {
        page_id,
        detail: format!("slot {slot_index} range [{start}, {end}) lies outside the page data area"),
    }
}

/// 目录页的**页级判据**——取槽**之前**问：页头声称的槽数是否超出该页的槽
/// 目录容量。
///
/// 判据取页格式层冻结的**唯一**实现（`SlottedPageRef::slot_count_exceeds_
/// capacity`，与 `integrity::validate_slots` 同源）⇒ 目录路径不另造阈值。
fn check_catalog_page_slot_count(slotted: &SlottedPageRef<'_>, page_id: u64) -> Result<()> {
    if slotted.slot_count_exceeds_capacity() {
        return Err(corrupted_catalog_page_slot_count_error(
            page_id,
            slotted.slot_count(),
        ));
    }
    Ok(())
}

/// 目录页的**有界槽负载**——经 `get_slot_data_checked` 取得（与索引树 / 数据
/// 页读侧同形）；越界 ⇒ 点名该页与该槽的错误。
///
/// 负载越界时**不得**跳过该目录行（会让该表静默消失），**不得**臆造表名。
fn catalog_slot_payload<'a>(
    slotted: &SlottedPageRef<'a>,
    page_id: u64,
    slot_index: usize,
    slot: &Slot,
) -> Result<&'a [u8]> {
    match slotted.get_slot_data_checked(slot) {
        Some(bytes) => Ok(bytes),
        None => Err(corrupted_catalog_page_slot_error(page_id, slot_index, slot)),
    }
}

/// Delete every row in the chain whose bytes satisfy `matcher`. Rewrites
/// each affected page in place (without compaction, since SlottedPage
/// `delete_slot` already compacts within the page).
async fn delete_from_chain<F>(
    bp: &BufferPool,
    storage: &Arc<dyn AsyncStorage>,
    root: PageId,
    matcher: F,
) -> Result<Vec<PageId>>
where
    F: Fn(&[u8]) -> bool,
{
    let mut changed = Vec::new();
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
                check_catalog_page_slot_count(&slotted, page_id.0)?;
                let mut del = Vec::new();
                for i in 0..slotted.slot_count() {
                    if let Some(slot) = slotted.get_slot(i) {
                        let bytes = catalog_slot_payload(&slotted, page_id.0, i, &slot)?;
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
            changed.push(page_id);
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
    Ok(changed)
}

/// Update the bytes of a row whose name matches `target_name`, using
/// `updater` to produce the new payload from the current bytes.
///
/// The write strategy follows the payload length:
///
/// - **Equal length** — overwrite the live slot in place
///   ([`SlottedPage::update_slot_in_place`]). Every fixed-width field update
///   takes this path (tail / root / ordinal root / column count): no page
///   space is consumed, no slot is allocated, and the write still lands on a
///   page whose data area is already exhausted.
/// - **Different length** — append the new copy at the chain tail, then delete
///   every same-name copy left on the chain, so exactly one copy (the new one)
///   remains when the call returns. Appending first keeps a failure
///   non-destructive. The two steps are not atomic; a crash between them leaves
///   two copies, which [`Catalog::scan_tables`] collapses deterministically.
///
/// No-op if the row does not exist (`Ok(None)`); on a write, returns the
/// page the updated bytes landed on (`Some`) — the in-place page for an
/// equal-length rewrite, the freshly-appended copy page otherwise. An
/// update that cannot be persisted never reports success.
async fn update_field_in_chain<F>(
    bp: &BufferPool,
    storage: &Arc<dyn AsyncStorage>,
    root: PageId,
    target_name: &str,
    updater: F,
) -> Result<Option<PageId>>
where
    F: Fn(&[u8]) -> Result<Option<Vec<u8>>>,
{
    // Pass 1: locate the first same-name row and compute its new payload.
    let mut current = Some(root);
    let mut visited = std::collections::HashSet::new();
    let mut target: Option<(PageId, u16, usize, Vec<u8>)> = None;
    while let Some(page_id) = current {
        if !visited.insert(page_id.0) {
            break;
        }
        let (next, found) = bp
            .with_page_data(page_id, |data| -> Result<(u32, LocatedUpdate)> {
                let slotted = SlottedPageRef::new(data);
                check_catalog_page_slot_count(&slotted, page_id.0)?;
                let mut found = None;
                for i in 0..slotted.slot_count() {
                    let Some(slot) = slotted.get_slot(i) else {
                        continue;
                    };
                    let bytes = catalog_slot_payload(&slotted, page_id.0, i, &slot)?;
                    if matches_catalog_row_name(bytes, target_name) {
                        if let Some(new_bytes) = updater(bytes)? {
                            found = Some((slot.logical_id, bytes.len(), new_bytes));
                            break;
                        }
                    }
                }
                Ok((slotted.header().next_page_id, found))
            })
            .await?;
        if let Some((lid, old_len, new_bytes)) = found {
            target = Some((page_id, lid, old_len, new_bytes));
            break;
        }
        current = if next == 0 {
            None
        } else {
            Some(PageId(next as u64))
        };
    }

    // Row absent: existing no-op contract.
    let Some((page_id, lid, old_len, new_bytes)) = target else {
        return Ok(None);
    };

    if new_bytes.len() == old_len {
        let guard = bp.get_page(page_id).await?;
        let updated: std::result::Result<(), PageWriteError> = guard.modify_page(|page| {
            let mut slotted = SlottedPage::new(page);
            slotted.update_slot_in_place(lid, &new_bytes)
        });
        match updated {
            Ok(()) => {}
            // 页损坏 ⇒ 点名该页的错误（**不得**被折叠成「表不存在」或无细节
            // 的泛化错误）。
            Err(
                e @ (PageWriteError::SlotCountExceedsCapacity { .. }
                | PageWriteError::SlotPayloadOutOfBounds { .. }),
            ) => {
                return Err(page_write_error_to_storage_error(page_id.0, e));
            }
            Err(e) => {
                return Err(StorageError::Internal(format!(
                    "catalog update of table '{target_name}' failed: {e}"
                )));
            }
        }
        return Ok(Some(page_id));
    }

    let (new_page, new_lid) = append_to_chain(bp, storage, root, &new_bytes)
        .await
        .map_err(|e| {
            StorageError::Internal(format!(
                "catalog update of table '{target_name}' failed: {e}"
            ))
        })?;
    delete_named_copies_except(bp, root, target_name, (new_page.0, new_lid)).await?;
    Ok(Some(new_page))
}

/// Delete every same-name row in the chain except the copy identified by
/// `keep (page_id, logical_id)`. Collect-then-delete per page keeps the delete
/// targets valid while the page is being mutated.
async fn delete_named_copies_except(
    bp: &BufferPool,
    root: PageId,
    target_name: &str,
    keep: (u64, u16),
) -> Result<()> {
    let mut current = Some(root);
    let mut visited = std::collections::HashSet::new();
    while let Some(page_id) = current {
        if !visited.insert(page_id.0) {
            break;
        }
        let (next, to_delete) = bp
            .with_page_data(page_id, |data| -> Result<(u32, Vec<u16>)> {
                let slotted = SlottedPageRef::new(data);
                check_catalog_page_slot_count(&slotted, page_id.0)?;
                let mut del = Vec::new();
                for i in 0..slotted.slot_count() {
                    let Some(slot) = slotted.get_slot(i) else {
                        continue;
                    };
                    // Keep exactly the copy just appended — the stale copies
                    // sit before it in slot order and share its page whenever
                    // the row still had room to grow.
                    if page_id.0 == keep.0 && slot.logical_id == keep.1 {
                        continue;
                    }
                    let bytes = catalog_slot_payload(&slotted, page_id.0, i, &slot)?;
                    if matches_catalog_row_name(bytes, target_name) {
                        del.push(slot.logical_id);
                    }
                }
                Ok((slotted.header().next_page_id, del))
            })
            .await?;

        if !to_delete.is_empty() {
            let guard = bp.get_page(page_id).await?;
            let failed: Option<String> = guard.modify_page(|page| {
                let mut slotted = SlottedPage::new(page);
                for lid in &to_delete {
                    if let Err(e) = slotted.delete_slot_by_logical_id(*lid) {
                        return Some(format!("logical_id {lid}: {e}"));
                    }
                }
                None
            });
            if let Some(e) = failed {
                return Err(StorageError::Internal(format!(
                    "catalog update of table '{target_name}' failed to remove stale copy ({e})"
                )));
            }
        }

        current = if next == 0 {
            None
        } else {
            Some(PageId(next as u64))
        };
    }
    Ok(())
}

/// Collapse same-name `__tables` rows, keeping the copy with the greatest
/// logical_id (an append always allocates a greater id) while preserving the
/// first-appearance order of the surviving names.
fn collapse_newest_same_name(rows: Vec<(u16, CatalogRow)>) -> Vec<CatalogRow> {
    let mut order: Vec<String> = Vec::new();
    let mut newest: std::collections::HashMap<String, (u16, CatalogRow)> =
        std::collections::HashMap::new();
    for (lid, row) in rows {
        let name = row.table_name.clone();
        match newest.get(&name) {
            Some((best_lid, _)) if *best_lid >= lid => {}
            Some(_) => {
                newest.insert(name, (lid, row));
            }
            None => {
                order.push(name.clone());
                newest.insert(name, (lid, row));
            }
        }
    }
    order
        .into_iter()
        .filter_map(|name| newest.remove(&name).map(|(_, row)| row))
        .collect()
}

// ---------------------------------------------------------------------------
// Row serialization
// ---------------------------------------------------------------------------

/// Layout: u16 name_len | name_bytes | u32 head | u32 idx_root |
///         u32 pk_index | u16 pk_col_len | pk_col_bytes |
///         u32 column_count | u32 tail
///         [ MS23 Iter001 appended: u32 unique_count | unique_count × u32 root ]
///         [ MS21 Iter002 appended: u32 sec_count |
///           sec_count × (u16 name_len | name | u32 col_index | u32 root) ]
///         [ MS21 Iter005 appended: u8 kind; kind=View carries
///           u16 sql_len | sql_bytes ]
///         [ trailing u8 explicit_primary_key: 1 written, 0 not; absent reads 0 ]
pub(crate) fn serialize_catalog_row(row: &CatalogRow) -> Vec<u8> {
    let name_bytes = row.table_name.as_bytes();
    let pk_bytes = row.pk_column.as_bytes();
    let sec_bytes: usize = row
        .secondary_indexes
        .iter()
        .map(|e| 2 + e.name.len() + 4 + 4)
        .sum();
    let view_bytes = match (&row.kind, &row.view_sql) {
        (CatalogKind::View, Some(sql)) => 2 + sql.len(),
        _ => 0,
    };
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
        + 4 * row.unique_roots.len()
        + 4
        + sec_bytes
        + 1
        + view_bytes
        + 1;
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
    buf.extend_from_slice(&(row.secondary_indexes.len() as u32).to_le_bytes());
    for entry in &row.secondary_indexes {
        let idx_bytes = entry.name.as_bytes();
        buf.extend_from_slice(&(idx_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(idx_bytes);
        buf.extend_from_slice(&entry.column_index.to_le_bytes());
        buf.extend_from_slice(&entry.root.to_le_bytes());
    }
    buf.push(match row.kind {
        CatalogKind::Table => ROW_KIND_TABLE,
        CatalogKind::View => ROW_KIND_VIEW,
    });
    if row.kind == CatalogKind::View {
        let sql_bytes = row.view_sql.as_deref().unwrap_or("").as_bytes();
        buf.extend_from_slice(&(sql_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(sql_bytes);
    }
    buf.push(if row.explicit_primary_key { 1 } else { 0 });
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
    let mut unique_consumed = 0usize;
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
            unique_consumed = 4 + count * 4;
        }
    }
    // MS21 Iter002 (D4) compat read: the secondary-index section
    // (`u32 sec_count | N × (u16 name_len | name | u32 col_index | u32 root)`)
    // follows the unique-roots section. A row written before MS21 (or an
    // incomplete trailing section) deserializes as empty — behavior = no
    // secondary indexes.
    let mut secondary_indexes = Vec::new();
    let sec_rest = &rest[unique_consumed..];
    let mut sec_consumed = 0usize;
    if sec_rest.len() >= 4 {
        let sec_count =
            u32::from_le_bytes([sec_rest[0], sec_rest[1], sec_rest[2], sec_rest[3]]) as usize;
        let mut off = 4usize;
        let mut ok = true;
        for _ in 0..sec_count {
            if sec_rest.len() < off + 2 {
                ok = false;
                break;
            }
            let idx_len = u16::from_le_bytes([sec_rest[off], sec_rest[off + 1]]) as usize;
            off += 2;
            if sec_rest.len() < off + idx_len + 8 {
                ok = false;
                break;
            }
            let idx_name = match std::str::from_utf8(&sec_rest[off..off + idx_len]) {
                Ok(s) => s.to_string(),
                Err(_) => {
                    ok = false;
                    break;
                }
            };
            off += idx_len;
            let col_index = u32::from_le_bytes([
                sec_rest[off],
                sec_rest[off + 1],
                sec_rest[off + 2],
                sec_rest[off + 3],
            ]);
            off += 4;
            let root = u32::from_le_bytes([
                sec_rest[off],
                sec_rest[off + 1],
                sec_rest[off + 2],
                sec_rest[off + 3],
            ]);
            off += 4;
            secondary_indexes.push(CatalogSecondaryIndex {
                name: idx_name,
                column_index: col_index,
                root,
            });
        }
        if !ok {
            secondary_indexes.clear();
        } else {
            sec_consumed = off;
        }
    }
    // Kind byte, then view SQL when kind is View. After both are consumed,
    // one trailing u8 is explicit_primary_key (1 true, 0 false). A missing
    // byte reads as false and does not reject the row. An incomplete kind
    // section still reads as Table with no SQL.
    let kind_rest = &sec_rest[sec_consumed..];
    let (kind, view_sql, kind_consumed) = match kind_rest.first().copied() {
        Some(ROW_KIND_VIEW) => {
            if kind_rest.len() >= 3 {
                let sql_len = u16::from_le_bytes([kind_rest[1], kind_rest[2]]) as usize;
                if kind_rest.len() >= 3 + sql_len {
                    match std::str::from_utf8(&kind_rest[3..3 + sql_len]) {
                        Ok(s) => (CatalogKind::View, Some(s.to_string()), 3 + sql_len),
                        Err(_) => (CatalogKind::Table, None, 0usize),
                    }
                } else {
                    (CatalogKind::Table, None, 0usize)
                }
            } else {
                (CatalogKind::Table, None, 0usize)
            }
        }
        Some(ROW_KIND_TABLE) => (CatalogKind::Table, None, 1usize),
        _ => (CatalogKind::Table, None, 0usize),
    };
    let explicit_primary_key = if kind_consumed > 0 {
        matches!(kind_rest.get(kind_consumed).copied(), Some(1))
    } else {
        false
    };
    Ok(CatalogRow {
        table_name,
        data_page_head,
        index_root_page_id,
        pk_index,
        pk_column,
        column_count,
        data_page_tail,
        unique_roots,
        secondary_indexes,
        kind,
        view_sql,
        explicit_primary_key,
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
                // MS27 Iter001 (T12/D11): >65535B literals switch to the tuple
                // format's long-string tag (`[0x08][u32 len]`) — the legacy
                // `u16` prefix silently wrapped above 65535. Byte layout at
                // ≤65535 is untouched (existing files open with zero migration).
                if s.len() > u16::MAX as usize {
                    buf.push(TAG_LONG_STRING);
                    buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
                } else {
                    buf.push(TAG_STRING);
                    buf.extend_from_slice(&(s.len() as u16).to_le_bytes());
                }
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

/// change 2026-10-13 T3 (design D6): plan-time capacity probe for a catalog
/// column row. Serializes the row and attempts the same slot placement
/// `append_to_chain` performs on a freshly-initialized empty chain page. A
/// row that cannot fit an empty page can never be appended, so the planner
/// rejects it (named, zero side effects) instead of failing mid-CREATE with
/// "Page full" after a half-built table. The threshold is derived from the
/// real page format — no hardcoded byte constant.
pub(crate) fn catalog_column_row_fits(row: &CatalogColumnRow) -> bool {
    let payload = serialize_catalog_column_row(row);
    let mut page = Page::new(PageId(0));
    let _ = SlottedPage::init(&mut page, 0x03);
    let mut slotted = SlottedPage::new(&mut page);
    slotted.add_slot(&payload).is_ok()
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
                TAG_LONG_STRING => {
                    // MS27 Iter001 (T12/D11): long-string arm mirroring the
                    // tuple format's 0x08 representation — truncated payloads
                    // and invalid UTF-8 are named errors, never silent cuts.
                    if data.len() < p + 4 {
                        return Err(StorageError::Internal(
                            "catalog column row truncated (default payload)".into(),
                        ));
                    }
                    let len = u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]])
                        as usize;
                    p += 4;
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

impl Catalog {
    /// MS21 Iter003 (T3-c/D10): rewrite the `__columns` chain for a DROP
    /// COLUMN — delete the removed column's row and shift every later
    /// column's `column_index` down by one, rewriting each affected row
    /// **in place on its page** (same-page append + delete,
    /// `update_field_in_chain` strategy; the payload is fixed-width in
    /// `column_index`, so the replacement is byte-equal in length). This
    /// preserves the chain-order invariant (`scan_columns` returns rows in
    /// chain order; `open_or_init` additionally sorts by `column_index`).
    ///
    /// Single chain walk for the whole shift. No-op if the table has no
    /// qualifying column rows.
    pub async fn rewrite_column_rows_drop(&self, table: &str, removed_index: u32) -> Result<()> {
        let _guard = self.lock.lock().await;

        // Pass 1: delete the removed column's row (delete_from_chain shape).
        delete_from_chain(
            &self.buffer_pool,
            &self.storage,
            PageId(COLUMNS_PAGE_ID),
            |data| {
                matches_column_row_of_table(data, table)
                    && matches_column_row_index(data, removed_index)
            },
        )
        .await?;

        // Pass 2: one walk — every row of the table with
        // `column_index > removed_index` is rewritten with its ordinal
        // decremented, in ascending ordinal order within the page (later
        // ordinals first would collide; within one page we rewrite rows
        // sorted ascending so each rewritten row is unique against the
        // pre-pass state).
        let mut current = Some(PageId(COLUMNS_PAGE_ID));
        let mut visited = std::collections::HashSet::new();
        while let Some(page_id) = current {
            if !visited.insert(page_id.0) {
                break;
            }
            // Locate all qualifying rows on this page (ascending ordinal).
            let found: Vec<(u16, Vec<u8>)> = self
                .buffer_pool
                .with_page_data(page_id, |data| -> Result<Vec<(u16, Vec<u8>)>> {
                    let slotted = SlottedPageRef::new(data);
                    check_catalog_page_slot_count(&slotted, page_id.0)?;
                    let mut out = Vec::new();
                    for i in 0..slotted.slot_count() {
                        let Some(slot) = slotted.get_slot(i) else {
                            continue;
                        };
                        let bytes = catalog_slot_payload(&slotted, page_id.0, i, &slot)?;
                        if !matches_column_row_of_table(bytes, table) {
                            continue;
                        }
                        let idx = column_row_index(bytes);
                        if idx <= removed_index {
                            continue;
                        }
                        let mut new_bytes = bytes.to_vec();
                        let name_len = u16::from_le_bytes([new_bytes[0], new_bytes[1]]) as usize;
                        let after = 2 + name_len;
                        new_bytes[after..after + 4].copy_from_slice(&(idx - 1).to_le_bytes());
                        out.push((slot.logical_id, new_bytes));
                    }
                    out.sort_by_key(|(lid, bytes)| (column_row_index(bytes), *lid));
                    Ok(out)
                })
                .await?;

            if !found.is_empty() {
                let guard = self.buffer_pool.get_page(page_id).await?;
                let add_error: Option<String> = guard.modify_page(|page| {
                    let mut slotted = SlottedPage::new(page);
                    for (_lid, new_bytes) in &found {
                        if let Err(e) = slotted.add_slot(new_bytes) {
                            return Some(format!("add_slot failed: {e}"));
                        }
                    }
                    for (lid, _) in &found {
                        let _ = slotted.delete_slot_by_logical_id(*lid);
                    }
                    None
                });
                if let Some(e) = add_error {
                    let _ = self.storage;
                    return Err(StorageError::Internal(format!(
                        "rewrite_column_rows_drop: {e}"
                    )));
                }
            }

            let next = self
                .buffer_pool
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
}

/// MS21 Iter003 (T3-c): whether the bytes encode a `__columns` row of the
/// given table (first field is the table name — same prefix layout as
/// `__tables` rows).
fn matches_column_row_of_table(data: &[u8], table: &str) -> bool {
    matches_catalog_row_name(data, table)
}

/// MS21 Iter003 (T3-c): the `column_index` (u32 LE after the table-name
/// field) of a `__columns` row.
fn column_row_index(data: &[u8]) -> u32 {
    let name_len = u16::from_le_bytes([data[0], data[1]]) as usize;
    let after = 2 + name_len;
    if data.len() < after + 4 {
        return u32::MAX; // malformed: never matches a shift target
    }
    u32::from_le_bytes([
        data[after],
        data[after + 1],
        data[after + 2],
        data[after + 3],
    ])
}

/// MS21 Iter003 (T3-c): whether the bytes encode a `__columns` row with the
/// given `column_index`.
fn matches_column_row_index(data: &[u8], column_index: u32) -> bool {
    column_row_index(data) == column_index
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::page_format::Slot;
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
            secondary_indexes: Vec::new(),
            kind: CatalogKind::Table,
            view_sql: None,
            explicit_primary_key: false,
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
    // Catalog durability (T2): equal-length in-place update, variable-length
    // relocation, same-name dedup, honest error propagation
    // ===========================================================================

    /// `(slot_count, free_space_offset, free_space)` of a catalog page.
    async fn page_space(bp: &BufferPool, page: PageId) -> (u16, u16, usize) {
        bp.with_page_data(page, |data| {
            let page_ref = SlottedPageRef::new(data);
            let h = page_ref.header();
            let free =
                Page::PAGE_SIZE - h.slot_count as usize * Slot::SIZE - h.free_space_offset as usize;
            Ok((h.slot_count, h.free_space_offset, free))
        })
        .await
        .unwrap()
    }

    /// Fill `page` with well-formed `__tables` rows of a filler table until no
    /// further slot fits — the same page-space exhaustion a long-lived
    /// database reaches through repeated row updates.
    async fn suffocate_page(bp: &BufferPool, page: PageId) {
        let bytes = serialize_catalog_row(&filler_row());
        let guard = bp.get_page(page).await.unwrap();
        guard.modify_page(|p| {
            let mut slotted = SlottedPage::new(p);
            while slotted.free_space() >= Slot::SIZE + bytes.len() {
                slotted.add_slot(&bytes).unwrap();
            }
        });
    }

    fn filler_row() -> CatalogRow {
        let mut row = sample_row();
        row.table_name = "filler".to_string();
        row
    }

    /// The one live `__tables` row named `name` (asserts it is unique).
    fn live_row<'a>(rows: &'a [CatalogRow], name: &str) -> &'a CatalogRow {
        let mut hits = rows.iter().filter(|r| r.table_name == name);
        let row = hits.next().expect("row must exist");
        assert!(hits.next().is_none(), "exactly one row named {name}");
        row
    }

    /// Re-open the catalog from `path` with a fresh storage + buffer pool. The
    /// first session's advisory lock must already be released.
    async fn reopen_tables(path: &std::path::Path) -> Vec<CatalogRow> {
        let storage = Arc::new(FileStorage::open(path).unwrap());
        let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let cat = Catalog::open(bp.clone(), storage.clone()).await.unwrap();
        let rows = cat.scan_tables().await.unwrap();
        bp.flush_all().await.unwrap();
        drop(cat);
        drop(bp);
        drop(storage);
        rows
    }

    /// 200 fixed-width `update_table_tail` calls must not consume page space and
    /// the last value must survive a flush + re-open.
    #[tokio::test]
    async fn update_table_tail_200_cycles_zero_leak_and_persists() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        {
            let storage = Arc::new(FileStorage::open(&path).unwrap());
            let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
            let cat = Catalog::bootstrap(bp.clone(), storage.clone())
                .await
                .unwrap();
            cat.insert_table(&sample_row(), &[]).await.unwrap();

            let before = page_space(&bp, PageId(TABLES_PAGE_ID)).await;
            for i in 0..200u32 {
                cat.update_table_tail("users", 1_000 + i).await.unwrap();
            }
            let after = page_space(&bp, PageId(TABLES_PAGE_ID)).await;
            assert_eq!(
                (after.0, after.1),
                (before.0, before.1),
                "fixed-width updates must not grow slot_count / free_space_offset"
            );

            bp.flush_all().await.unwrap();
            drop(cat);
            drop(bp);
            drop(storage);
        }
        let rows = reopen_tables(&path).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].data_page_tail, 1_199);
    }

    /// An equal-length update on a page whose data area is exhausted still
    /// lands and survives a re-open (the old append-then-delete path dropped it
    /// silently).
    #[tokio::test]
    async fn update_table_tail_on_suffocated_page_persists() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        {
            let storage = Arc::new(FileStorage::open(&path).unwrap());
            let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
            let cat = Catalog::bootstrap(bp.clone(), storage.clone())
                .await
                .unwrap();
            cat.insert_table(&sample_row(), &[]).await.unwrap();

            suffocate_page(&bp, PageId(TABLES_PAGE_ID)).await;
            let (_, _, free) = page_space(&bp, PageId(TABLES_PAGE_ID)).await;
            assert!(
                free < serialize_catalog_row(&filler_row()).len() + Slot::SIZE,
                "fixture must leave no room for another catalog row (free={free})"
            );

            cat.update_table_tail("users", 777).await.unwrap();
            let rows = cat.scan_tables().await.unwrap();
            assert_eq!(live_row(&rows, "users").data_page_tail, 777);

            bp.flush_all().await.unwrap();
            drop(cat);
            drop(bp);
            drop(storage);
        }
        let rows = reopen_tables(&path).await;
        assert_eq!(live_row(&rows, "users").data_page_tail, 777);
    }

    /// A variable-length update on a full page relocates to a fresh chain page,
    /// leaves exactly one copy behind, and survives a re-open.
    #[tokio::test]
    async fn append_secondary_index_on_full_page_relocates_and_persists() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let entry = CatalogSecondaryIndex {
            name: "idx_payload".to_string(),
            column_index: 1,
            root: 42,
        };
        {
            let storage = Arc::new(FileStorage::open(&path).unwrap());
            let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
            let cat = Catalog::bootstrap(bp.clone(), storage.clone())
                .await
                .unwrap();
            cat.insert_table(&sample_row(), &[]).await.unwrap();
            suffocate_page(&bp, PageId(TABLES_PAGE_ID)).await;

            cat.append_secondary_index("users", &entry).await.unwrap();

            let rows = cat.scan_tables().await.unwrap();
            assert_eq!(
                live_row(&rows, "users").secondary_indexes,
                vec![entry.clone()],
                "relocation must leave one copy carrying the registration"
            );

            bp.flush_all().await.unwrap();
            drop(cat);
            drop(bp);
            drop(storage);
        }
        let rows = reopen_tables(&path).await;
        assert_eq!(live_row(&rows, "users").secondary_indexes, vec![entry]);
    }

    /// `scan_tables` collapses same-name copies and keeps the newest one (the
    /// greatest logical_id, which an append always allocates).
    #[tokio::test]
    async fn scan_tables_keeps_highest_logical_id_copy() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let storage = Arc::new(FileStorage::open(&path).unwrap());
        let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let cat = Catalog::bootstrap(bp.clone(), storage.clone())
            .await
            .unwrap();
        cat.insert_table(&sample_row(), &[]).await.unwrap();

        // Manually add a second copy of the same row carrying a newer payload,
        // as a crashed variable-length update would leave behind.
        let mut newer = sample_row();
        newer.data_page_tail = 999;
        let bytes = serialize_catalog_row(&newer);
        {
            let guard = bp.get_page(PageId(TABLES_PAGE_ID)).await.unwrap();
            guard.modify_page(|page| {
                let mut slotted = SlottedPage::new(page);
                slotted.add_slot(&bytes).unwrap();
            });
        }

        let rows = cat.scan_tables().await.unwrap();
        assert_eq!(rows.len(), 1, "same-name copies must collapse to one row");
        assert_eq!(rows[0].data_page_tail, 999);
    }

    /// A variable-length update that cannot land anywhere fails loudly, names
    /// the table, and leaves the live row untouched.
    #[tokio::test]
    async fn append_secondary_index_failure_names_table_and_keeps_row() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let storage = Arc::new(FileStorage::open(&path).unwrap());
        let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let cat = Catalog::bootstrap(bp.clone(), storage.clone())
            .await
            .unwrap();
        cat.insert_table(&sample_row(), &[]).await.unwrap();
        suffocate_page(&bp, PageId(TABLES_PAGE_ID)).await;

        // A registration whose serialized row exceeds a whole page: neither the
        // exhausted page nor the fresh page `append_to_chain` allocates can
        // hold it.
        let entry = CatalogSecondaryIndex {
            name: "x".repeat(5000),
            column_index: 1,
            root: 7,
        };
        let err = cat
            .append_secondary_index("users", &entry)
            .await
            .expect_err("variable-length update must not report success");
        assert!(
            err.to_string().contains("users"),
            "error must name the table, got: {err}"
        );

        let rows = cat.scan_tables().await.unwrap();
        assert!(
            live_row(&rows, "users").secondary_indexes.is_empty(),
            "failed update must not leave a registration behind"
        );
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

    // ===========================================================================
    // MS21 Iter005 (T5-a/D12): catalog kind 段与视图行
    // ===========================================================================

    /// 视图行编解码往返：kind=View + SQL 文本尾段随行序列化并读回。
    #[test]
    fn catalog_view_row_roundtrip() {
        let mut row = sample_row();
        row.kind = CatalogKind::View;
        row.view_sql = Some("SELECT id, v FROM users WHERE v > 100".to_string());
        let bytes = serialize_catalog_row(&row);
        let back = deserialize_catalog_row(&bytes).unwrap();
        assert_eq!(row, back);
    }

    /// 旧格式兼容：无 kind 尾段的行读为 Table、无 SQL，且没写出 PRIMARY KEY。
    /// 当前布局末尾是 kind 字节再加 explicit_primary_key 字节，两字节都要截掉。
    #[test]
    fn legacy_row_without_kind_section_reads_as_table() {
        let row = sample_row();
        let mut bytes = serialize_catalog_row(&row);
        bytes.pop();
        bytes.pop();
        let back = deserialize_catalog_row(&bytes).unwrap();
        assert_eq!(back.kind, CatalogKind::Table);
        assert_eq!(back.view_sql, None);
        assert!(!back.explicit_primary_key);
    }

    /// 写出 PRIMARY KEY 的表行往返后仍为真。
    #[test]
    fn catalog_row_explicit_primary_key_roundtrip() {
        let mut row = sample_row();
        row.explicit_primary_key = true;
        let bytes = serialize_catalog_row(&row);
        let back = deserialize_catalog_row(&bytes).unwrap();
        assert_eq!(back, row);
        assert!(back.explicit_primary_key);
    }

    /// 当前 kind 布局缺尾字节时读回为没写出 PRIMARY KEY，kind 仍是 Table。
    #[test]
    fn legacy_row_without_explicit_primary_key_byte_reads_false() {
        let row = sample_row();
        let mut bytes = serialize_catalog_row(&row);
        bytes.pop();
        let back = deserialize_catalog_row(&bytes).unwrap();
        assert!(!back.explicit_primary_key);
        assert_eq!(back.kind, CatalogKind::Table);
    }

    /// kind=View 但 SQL 文本段截断：截进 SQL 正文后仍读为 Table。
    /// 只去掉写出主键的尾字节不会进入正文。
    #[test]
    fn view_row_with_truncated_sql_section_tolerates_as_table() {
        let mut row = sample_row();
        row.kind = CatalogKind::View;
        row.view_sql = Some("SELECT 1".to_string());
        let mut bytes = serialize_catalog_row(&row);
        bytes.truncate(bytes.len() - 2);
        let back = deserialize_catalog_row(&bytes).unwrap();
        assert_eq!(back.kind, CatalogKind::Table);
        assert_eq!(back.view_sql, None);
    }

    /// 视图行 + 列行经 catalog 持久往返：`insert_view_row` → scan_views/
    /// scan_tables/scan_columns 读回，与基表行共存互不串扰；`delete_table`
    /// 复用既有删除机制删除视图行与列行。
    #[tokio::test]
    async fn view_row_persist_roundtrip_and_delete() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let storage = Arc::new(FileStorage::open(&path).unwrap());
        let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let cat = Catalog::bootstrap(bp.clone(), storage.clone())
            .await
            .unwrap();

        let sql = "SELECT id, v FROM users WHERE v > 100";
        let cols = vec![
            CatalogColumnRow {
                table_name: "big".to_string(),
                column_index: 0,
                column_name: "id".to_string(),
                column_type: ColumnType::Int,
                not_null: false,
                unique: false,
                default_value: None,
            },
            CatalogColumnRow {
                table_name: "big".to_string(),
                column_index: 1,
                column_name: "v".to_string(),
                column_type: ColumnType::Int,
                not_null: false,
                unique: false,
                default_value: None,
            },
        ];
        cat.insert_view_row("big", sql, &cols).await.unwrap();
        cat.insert_table(&sample_row(), &[]).await.unwrap();

        let views = cat.scan_views().await.unwrap();
        assert_eq!(views.len(), 1, "scan_views must return only view rows");
        assert_eq!(views[0].table_name, "big");
        assert_eq!(views[0].kind, CatalogKind::View);
        assert_eq!(views[0].view_sql.as_deref(), Some(sql));
        assert_eq!(views[0].column_count, 2);
        // 视图行字段约定：零页指针、空 PK（无数据页/索引树）
        assert_eq!(views[0].data_page_head, 0);
        assert_eq!(views[0].data_page_tail, 0);
        assert_eq!(views[0].index_root_page_id, 0);
        assert_eq!(views[0].pk_column, "");

        let all = cat.scan_tables().await.unwrap();
        assert_eq!(all.len(), 2);
        let base: Vec<_> = all
            .iter()
            .filter(|r| r.kind == CatalogKind::Table)
            .collect();
        assert_eq!(base.len(), 1);
        assert_eq!(base[0].table_name, "users");
        assert_eq!(base[0].view_sql, None);

        let scanned_cols = cat.scan_columns("big").await.unwrap();
        assert_eq!(scanned_cols, cols);

        cat.delete_table("big").await.unwrap();
        assert!(cat.scan_views().await.unwrap().is_empty());
        assert!(cat.scan_columns("big").await.unwrap().is_empty());
        assert_eq!(cat.scan_tables().await.unwrap().len(), 1);
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

    // ===========================================================================
    // MS21 Iteration 002 (T2-a/D4): catalog 表行二级索引登记尾段
    // ===========================================================================

    /// 新格式往返：二级索引登记段（名称/列位/根页）随行序列化并读回，唯一根
    /// 段不受影响。
    #[test]
    fn catalog_row_with_secondary_indexes_roundtrip() {
        let mut row = sample_row();
        row.unique_roots = vec![41];
        row.secondary_indexes = vec![
            CatalogSecondaryIndex {
                name: "idx_age".to_string(),
                column_index: 1,
                root: 51,
            },
            CatalogSecondaryIndex {
                name: "idx_city".to_string(),
                column_index: 2,
                root: 52,
            },
        ];
        let bytes = serialize_catalog_row(&row);
        let back = deserialize_catalog_row(&bytes).unwrap();
        assert_eq!(row, back);
        assert_eq!(back.secondary_indexes.len(), 2);
        assert_eq!(back.secondary_indexes[0].name, "idx_age");
        assert_eq!(back.secondary_indexes[1].root, 52);
    }

    /// 旧格式兼容：MS23 时代行（唯一根段之后无二级段）读为空二级登记。
    #[test]
    fn ms23_era_row_without_secondary_section_reads_empty() {
        // MS23 固定布局：基础字段 + 唯一根段（count=1），其后结束。
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
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&41u32.to_le_bytes());

        let row = deserialize_catalog_row(&bytes).unwrap();
        assert_eq!(row.unique_roots, vec![41]);
        assert!(
            row.secondary_indexes.is_empty(),
            "row without secondary section must read empty, got {:?}",
            row.secondary_indexes
        );
    }

    /// `append_secondary_index` / `remove_secondary_index` /
    /// `update_secondary_index_root` 的链上读写往返：其余字段（含唯一根）不丢。
    #[tokio::test]
    async fn secondary_index_registration_lifecycle_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let storage = Arc::new(FileStorage::open(&path).unwrap());
        let bp = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

        let cat = Catalog::bootstrap(bp.clone(), storage.clone())
            .await
            .unwrap();
        let mut row = sample_row();
        row.unique_roots = vec![7];
        cat.insert_table(&row, &[]).await.unwrap();

        cat.append_secondary_index(
            "users",
            &CatalogSecondaryIndex {
                name: "idx_age".to_string(),
                column_index: 1,
                root: 51,
            },
        )
        .await
        .unwrap();
        cat.append_secondary_index(
            "users",
            &CatalogSecondaryIndex {
                name: "idx_city".to_string(),
                column_index: 2,
                root: 52,
            },
        )
        .await
        .unwrap();

        let scanned = cat.scan_tables().await.unwrap();
        assert_eq!(scanned[0].unique_roots, vec![7], "unique roots preserved");
        assert_eq!(scanned[0].secondary_indexes.len(), 2);

        // ordinal 定位根写回只改目标位
        cat.update_secondary_index_root("users", 1, 99)
            .await
            .unwrap();
        let scanned = cat.scan_tables().await.unwrap();
        assert_eq!(scanned[0].secondary_indexes[0].root, 51);
        assert_eq!(scanned[0].secondary_indexes[1].root, 99);

        // 删除首个登记：余者保持相对次序
        cat.remove_secondary_index("users", "idx_age")
            .await
            .unwrap();
        let scanned = cat.scan_tables().await.unwrap();
        assert_eq!(scanned[0].secondary_indexes.len(), 1);
        assert_eq!(scanned[0].secondary_indexes[0].name, "idx_city");
        assert_eq!(scanned[0].secondary_indexes[0].root, 99);
        assert_eq!(scanned[0].unique_roots, vec![7]);
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

    // ===========================================================================
    // MS27 Iteration 001 (T12/D11): DEFAULT 段长字符串表示（0x08）
    // ===========================================================================

    /// DEFAULT 段 String 臂的 value-tag 起始偏移（sample_col 形态：表 users、
    /// 列名 1 字节、String 类型带 u16 max_len、双 flag 关、has_default=1）。
    fn default_string_tag_offset() -> usize {
        2 + 5 + 4 + 2 + 1 + 1 + 2 + 1 + 1 + 1
    }

    /// 切换点内侧 65535：段字节布局与既有完全一致（首 tag 0x02 + u16 长度）。
    #[test]
    fn default_string_65535_keeps_legacy_layout() {
        let mut col = sample_col("c", 1, ColumnType::String(65535));
        col.default_value = Some(crate::executor::Value::String("x".repeat(65535)));
        let bytes = serialize_catalog_column_row(&col);

        let tag_pos = default_string_tag_offset();
        assert_eq!(bytes[tag_pos], TAG_STRING, "65535 stays TAG_STRING (0x02)");
        let len = u16::from_le_bytes([bytes[tag_pos + 1], bytes[tag_pos + 2]]);
        assert_eq!(len, 65535, "u16 length prefix unchanged at the boundary");

        let back = deserialize_catalog_column_row(&bytes).unwrap();
        assert_eq!(back.default_value, col.default_value);
    }

    /// 切换点外侧 65536：段切换为 `[0x08][u32 len]`（既有 u16 布局对
    /// >65535B 值长度前缀回绕——修复面）。
    #[test]
    fn default_string_65536_switches_to_long_tag() {
        let mut col = sample_col("c", 1, ColumnType::String(65535));
        col.default_value = Some(crate::executor::Value::String("y".repeat(65536)));
        let bytes = serialize_catalog_column_row(&col);

        let tag_pos = default_string_tag_offset();
        assert_eq!(
            bytes[tag_pos], TAG_LONG_STRING,
            "65536 must switch to TAG_LONG_STRING (0x08)"
        );
        let len = u32::from_le_bytes([
            bytes[tag_pos + 1],
            bytes[tag_pos + 2],
            bytes[tag_pos + 3],
            bytes[tag_pos + 4],
        ]);
        assert_eq!(len, 65536, "u32 length prefix carries the true value");

        let back = deserialize_catalog_column_row(&bytes).unwrap();
        assert_eq!(
            back.default_value, col.default_value,
            "long default must roundtrip without truncation"
        );
    }

    /// 远超切换点的长 DEFAULT 行级往返等价（仿
    /// `column_row_with_default_roundtrip_all_variants` 形态）。
    #[test]
    fn long_default_row_roundtrip() {
        let mut col = sample_col("c", 2, ColumnType::String(65535));
        col.default_value = Some(crate::executor::Value::String("z".repeat(70_000)));
        let bytes = serialize_catalog_column_row(&col);
        let back = deserialize_catalog_column_row(&bytes).unwrap();
        assert_eq!(col, back, "full row roundtrip must be equivalent");
        assert_eq!(back.default_value, col.default_value);
    }

    /// 0x08 段诚实拒绝：payload 截断与坏 UTF-8 均点名错误（不静默截断）。
    #[test]
    fn long_default_rejects_truncated_and_bad_utf8() {
        // 截断：u32 声明 100 字节 payload，实际只给 4 字节
        let mut truncated = Vec::new();
        truncated.extend_from_slice(&(5u16).to_le_bytes());
        truncated.extend_from_slice(b"users");
        truncated.extend_from_slice(&0u32.to_le_bytes());
        truncated.extend_from_slice(&(1u16).to_le_bytes());
        truncated.extend_from_slice(b"c");
        truncated.push(COL_TAG_INT);
        truncated.push(0);
        truncated.push(0);
        truncated.push(1); // has_default
        truncated.push(TAG_LONG_STRING);
        truncated.extend_from_slice(&100u32.to_le_bytes());
        truncated.extend_from_slice(b"ab12");
        let err = deserialize_catalog_column_row(&truncated).unwrap_err();
        assert!(
            err.to_string().contains("default payload"),
            "truncated long default must name the default payload, got {err}"
        );

        // 坏 UTF-8：长度声明与字节数一致但内容非法
        let mut bad_utf8 = truncated[..truncated.len() - 4 - 4].to_vec();
        bad_utf8.extend_from_slice(&4u32.to_le_bytes());
        bad_utf8.extend_from_slice(&[0xFF, 0xFE, 0xFD, 0xFC]);
        let err = deserialize_catalog_column_row(&bad_utf8).unwrap_err();
        assert!(
            err.to_string().contains("utf8"),
            "bad utf8 long default must be named, got {err}"
        );
    }
}
