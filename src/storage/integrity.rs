//! MS27 Iteration 002 (T9): read-only structural integrity check.
//!
//! `check_integrity` walks every page of a database through the buffer pool
//! without modifying anything (no page writes, no dirty-bit clears, no
//! checkpoint/WAL traffic) and reports structural damage as a list of
//! findings instead of failing fast:
//!
//! - page census: every page in `[0, page_count)` classified by its first
//!   byte (`0x00` free/zero, `0x01` b-tree leaf, `0x02` b-tree internal,
//!   `0x03` slotted data/catalog, `0x04` overflow);
//! - catalog parse: `scan_tables`/`scan_columns` failures are findings;
//! - per table (base tables only — views have no pages): data-chain walk
//!   (cycle-safe, terminal page checked against the catalog tail), per-slot
//!   bounds validation, per-locator overflow-chain validation, and the PK /
//!   unique / secondary index tree traversals;
//! - overflow pages referenced by no locator are reported as `info` (known
//!   boundary — they are only leaked space, not lost data).
//!
//! Corrupt input must never panic: every header/slot field is range-checked
//! before use (the production accessors index unsafely into well-formed
//! pages and are only called after validation), and every violation becomes
//! a finding.

use std::collections::{BTreeMap, HashSet};

use crate::storage::buffer_pool::BufferPool;
use crate::storage::catalog::CatalogKind;
use crate::storage::data::{TableManager, TableMeta};
use crate::storage::page_format::overflow::{
    decode_locator, parse_overflow_page, OVERFLOW_PAGE_TYPE,
};
use crate::storage::page_format::MAX_KEY_LEN;
use crate::storage::page_format::{Slot, SlottedPageHeader, SlottedPageRef};
use crate::storage::{btree::INTERNAL_NODE, btree::LEAF_NODE, Page, PageId, Result};
use crate::transaction::VersionHeader;

/// Page type of data and catalog pages (see `catalog.rs` bootstrap).
const SLOTTED_PAGE_TYPE: u8 = 0x03;

/// Severity of a [`IntegrityFinding`]. Only `Error` findings make
/// `rtsql check` exit non-zero; `Info` marks known non-damage boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingSeverity {
    /// Structural damage (unreadable page, broken chain, invalid slot, …).
    Error,
    /// Reported irregularity that is not damage (unreferenced overflow
    /// pages — leaked space only).
    Info,
}

/// One reported problem: where (page, when page-scoped) and what.
#[derive(Debug, Clone)]
pub struct IntegrityFinding {
    pub severity: FindingSeverity,
    /// Page the finding is scoped to (`None` for catalog/file-level findings).
    pub page: Option<u32>,
    pub message: String,
}

/// Result of a full structural check.
#[derive(Debug, Clone, Default)]
pub struct IntegrityReport {
    /// Total pages the file claims (`page_count()` at check time).
    pub total_pages: u64,
    /// First-byte page type → page count, known and unknown types alike.
    pub census: BTreeMap<u8, u64>,
    pub findings: Vec<IntegrityFinding>,
}

impl IntegrityReport {
    pub fn error_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == FindingSeverity::Error)
            .count()
    }

    pub fn info_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == FindingSeverity::Info)
            .count()
    }
}

/// Run the full read-only structural check. See the module docs for scope.
pub async fn check_integrity(
    buffer_pool: &BufferPool,
    table_manager: &TableManager,
) -> Result<IntegrityReport> {
    let total_pages = buffer_pool.storage().page_count();
    let mut checker = Checker {
        buffer_pool,
        total_pages,
        findings: Vec::new(),
        referenced_overflow: HashSet::new(),
        overflow_pages: HashSet::new(),
    };

    // ① Page census — every page classified by its first byte.
    let census = checker.census_pages().await;

    // ② Catalog parse — directory and column rows must deserialize.
    let mut tables = None;
    match table_manager.catalog().scan_tables().await {
        Err(e) => checker.error(None, format!("catalog table directory is unreadable: {e}")),
        Ok(rows) => {
            for row in &rows {
                if let Err(e) = table_manager.catalog().scan_columns(&row.table_name).await {
                    checker.error(
                        None,
                        format!(
                            "catalog column rows of table '{}' are unreadable: {e}",
                            row.table_name
                        ),
                    );
                }
            }
            tables = Some(rows);
        }
    }

    // ③⑤ Per base table: data chain, overflow chains, index trees. Views
    // own no pages (open_or_init skips them) — catalog parse covers them.
    if let Some(rows) = tables {
        for row in rows.into_iter().filter(|r| r.kind == CatalogKind::Table) {
            match table_manager.get_table(&row.table_name).await {
                Err(e) => checker.error(
                    None,
                    format!(
                        "table '{}' is registered in the catalog but cannot be loaded: {e}",
                        row.table_name
                    ),
                ),
                Ok(meta) => checker.check_table(&meta).await,
            }
        }
    }

    // ④ Overflow pages no locator reaches — leaked space, not lost data.
    let mut unreferenced: Vec<u32> = checker
        .overflow_pages
        .difference(&checker.referenced_overflow)
        .copied()
        .collect();
    unreferenced.sort_unstable();
    for raw in unreferenced {
        checker.info(
            Some(raw),
            "overflow page is not referenced by any row locator (leaked space)".to_string(),
        );
    }

    Ok(IntegrityReport {
        total_pages,
        census,
        findings: checker.findings,
    })
}

struct Checker<'a> {
    buffer_pool: &'a BufferPool,
    total_pages: u64,
    findings: Vec<IntegrityFinding>,
    /// Overflow pages reached through some row's locator.
    referenced_overflow: HashSet<u32>,
    /// Overflow pages seen in the census.
    overflow_pages: HashSet<u32>,
}

impl<'a> Checker<'a> {
    fn error(&mut self, page: Option<u32>, message: impl Into<String>) {
        self.findings.push(IntegrityFinding {
            severity: FindingSeverity::Error,
            page,
            message: message.into(),
        });
    }

    fn info(&mut self, page: Option<u32>, message: impl Into<String>) {
        self.findings.push(IntegrityFinding {
            severity: FindingSeverity::Info,
            page,
            message: message.into(),
        });
    }

    fn in_bounds(&self, raw: u32) -> bool {
        (raw as u64) < self.total_pages
    }

    async fn census_pages(&mut self) -> BTreeMap<u8, u64> {
        let mut census = BTreeMap::new();
        for raw in 0..self.total_pages {
            let page_type = self
                .buffer_pool
                .with_page_data(PageId(raw), |data| Ok(data.first().copied().unwrap_or(0)))
                .await;
            match page_type {
                Ok(t) => {
                    *census.entry(t).or_insert(0) += 1;
                    if t == OVERFLOW_PAGE_TYPE {
                        self.overflow_pages.insert(raw as u32);
                    }
                    if !matches!(
                        t,
                        0x00 | LEAF_NODE | INTERNAL_NODE | SLOTTED_PAGE_TYPE | OVERFLOW_PAGE_TYPE
                    ) {
                        self.error(Some(raw as u32), format!("unknown page type {t:#04x}"));
                    }
                }
                Err(e) => self.error(Some(raw as u32), format!("page is unreadable: {e}")),
            }
        }
        census
    }

    async fn check_table(&mut self, meta: &TableMeta) {
        let tail = *meta.data_page_tail.lock().unwrap();
        self.check_data_chain(&meta.name, meta.data_page_head, tail)
            .await;
        self.check_index_tree(&meta.name, "primary key", meta.index_manager.root_page_id())
            .await;
        for (ordinal, index) in &meta.unique_indexes {
            self.check_index_tree(
                &meta.name,
                &format!("unique index on column ordinal {ordinal}"),
                index.root_page_id(),
            )
            .await;
        }
        for secondary in &meta.secondary_indexes {
            self.check_index_tree(
                &meta.name,
                &format!("secondary index '{}'", secondary.name),
                secondary.index.root_page_id(),
            )
            .await;
        }
    }

    /// Walk one table's data page chain: cycle-safe, type-checked,
    /// slot-validated, locator-following; the terminal page is checked
    /// against the catalog-persisted tail — terminal ahead of the tail is
    /// the known benign post-recovery shape (info), a terminal at or before
    /// it means recorded pages became unreachable (error).
    async fn check_data_chain(&mut self, table: &str, head: PageId, tail: PageId) {
        let mut visited: HashSet<u32> = HashSet::new();
        let mut next = Some(head.0 as u32);
        let mut terminal: Option<u32> = None;
        while let Some(raw) = next {
            if !self.in_bounds(raw) {
                self.error(
                    Some(raw),
                    format!(
                        "data chain of table '{table}' leaves the file at page {raw} (file has {} pages)",
                        self.total_pages
                    ),
                );
                return;
            }
            if !visited.insert(raw) {
                self.error(
                    Some(raw),
                    format!("data chain of table '{table}' is cyclic at page {raw}"),
                );
                return;
            }
            let step = match self
                .buffer_pool
                .with_page_data(PageId(raw as u64), |data| Ok(check_data_page(data)))
                .await
            {
                Ok(step) => step,
                Err(e) => {
                    self.error(
                        Some(raw),
                        format!("data page of table '{table}' is unreadable: {e}"),
                    );
                    return;
                }
            };
            for violation in step.violations {
                self.error(Some(raw), violation);
            }
            if step.page_type != SLOTTED_PAGE_TYPE {
                self.error(
                    Some(raw),
                    format!(
                        "data page of table '{table}' has page type {:#04x}, expected {:#04x}",
                        step.page_type, SLOTTED_PAGE_TYPE
                    ),
                );
                // A non-slotted page's next pointer carries no chain semantics.
                return;
            }
            for (total_len, first_page) in step.locators {
                self.check_overflow_chain(table, total_len, first_page)
                    .await;
            }
            next = if step.next_page_id == 0 {
                terminal = Some(raw);
                None
            } else {
                Some(step.next_page_id)
            };
        }
        if terminal != Some(tail.0 as u32) {
            match terminal {
                // Terminal ahead of the persisted tail is the known benign
                // shape after crash recovery: `redo_tuple_at_row_id` advances
                // only the in-memory tail (recovery.rs step 4) and never
                // writes it back to the catalog row, so the persisted tail
                // stays at its last checkpointed value. Not damage — info.
                Some(raw) if raw > tail.0 as u32 => self.info(
                    Some(raw),
                    format!(
                        "data chain of table '{table}' terminates at page {raw}, ahead of the persisted catalog tail page {} (benign after crash recovery)",
                        tail.0
                    ),
                ),
                // Terminal at or before the persisted tail means pages the
                // catalog recorded as written are no longer reachable.
                Some(raw) => self.error(
                    Some(raw),
                    format!(
                        "data chain of table '{table}' terminates at page {raw} but the catalog tail is page {}",
                        tail.0
                    ),
                ),
                None => self.error(
                    None,
                    format!(
                        "data chain of table '{table}' ends in a cycle; the catalog tail is page {}",
                        tail.0
                    ),
                ),
            }
        }
    }

    /// Follow one row's overflow chain: bounds, cycles, per-page validity,
    /// and the assembled byte total against the locator's declaration.
    async fn check_overflow_chain(&mut self, table: &str, total_len: u32, first_page: u32) {
        if first_page == 0 {
            self.error(
                None,
                format!(
                    "a row of table '{table}' declares {total_len} overflow bytes but references no chain"
                ),
            );
            return;
        }
        let mut visited: HashSet<u32> = HashSet::new();
        let mut next = Some(first_page);
        let mut assembled = 0usize;
        while let Some(raw) = next {
            if !self.in_bounds(raw) {
                self.error(
                    Some(raw),
                    format!(
                        "overflow chain of table '{table}' leaves the file at page {raw} (file has {} pages)",
                        self.total_pages
                    ),
                );
                return;
            }
            if !visited.insert(raw) {
                self.error(
                    Some(raw),
                    format!("overflow chain of table '{table}' is cyclic at page {raw}"),
                );
                return;
            }
            self.referenced_overflow.insert(raw);
            let parsed = self
                .buffer_pool
                .with_page_data(PageId(raw as u64), |data| {
                    parse_overflow_page(PageId(raw as u64), data)
                        .map(|(next, payload)| (next, payload.len()))
                })
                .await;
            match parsed {
                Err(e) => {
                    self.error(
                        Some(raw),
                        format!("overflow chain of table '{table}' hits a corrupt page: {e}"),
                    );
                    return;
                }
                Ok((next_raw, len)) => {
                    assembled += len;
                    next = if next_raw == 0 { None } else { Some(next_raw) };
                }
            }
        }
        if assembled != total_len as usize {
            self.error(
                Some(first_page),
                format!(
                    "overflow chain of table '{table}' assembles {assembled} bytes but the locator declares {total_len}"
                ),
            );
        }
    }

    /// Walk one index tree from its root: cycle-safe, bounds-checked,
    /// type-checked; internal-node child pointers validated before use.
    /// Tree re-visits are silently skipped (a leaf is reachable both from
    /// its parent and via its sibling's chain pointer — the graph is a DAG,
    /// not a tree; same policy as `collect_all_pages_tolerant`), so a corrupt
    /// cycle terminates the walk without hanging but is not itself reported.
    async fn check_index_tree(&mut self, table: &str, role: &str, root: PageId) {
        if root.0 == 0 {
            self.error(
                None,
                format!("the {role} index of table '{table}' has no root page"),
            );
            return;
        }
        let mut visited: HashSet<u32> = HashSet::new();
        let mut stack = vec![root.0 as u32];
        while let Some(raw) = stack.pop() {
            if !self.in_bounds(raw) {
                self.error(
                    Some(raw),
                    format!(
                        "the {role} index tree of table '{table}' references page {raw} outside the file (file has {} pages)",
                        self.total_pages
                    ),
                );
                continue;
            }
            if !visited.insert(raw) {
                continue;
            }
            let step = match self
                .buffer_pool
                .with_page_data(PageId(raw as u64), |data| Ok(check_tree_page(data)))
                .await
            {
                Ok(step) => step,
                Err(e) => {
                    self.error(
                        Some(raw),
                        format!(
                            "index page of the {role} index of table '{table}' is unreadable: {e}"
                        ),
                    );
                    continue;
                }
            };
            for violation in step.violations {
                self.error(Some(raw), violation);
            }
            if let Some(page_type) = step.invalid_type {
                self.error(
                    Some(raw),
                    format!(
                        "index page of the {role} index of table '{table}' has page type {page_type:#04x}, expected {LEAF_NODE:#04x} or {INTERNAL_NODE:#04x}"
                    ),
                );
                continue;
            }
            for child in step.children {
                stack.push(child);
            }
        }
    }
}

/// Outcome of validating one data page image inside the page lock.
struct DataPageCheck {
    page_type: u8,
    next_page_id: u32,
    /// `(total_len, first_overflow_page)` per overflow-referencing slot.
    locators: Vec<(u32, u32)>,
    violations: Vec<String>,
}

/// Validate a slotted page image without panicking: every header/slot field
/// is range-checked before the production accessors see it (they index
/// unsafely into well-formed pages). The overflow marker is probed on the
/// tuple segment (`slot_data[VersionHeader::SIZE..]`), matching the read
/// path — never on the version header itself.
fn check_data_page(data: &[u8]) -> DataPageCheck {
    let mut violations = Vec::new();
    if data.len() < SlottedPageHeader::SIZE {
        violations.push("page is shorter than a page header".to_string());
        return DataPageCheck {
            page_type: 0,
            next_page_id: 0,
            locators: Vec::new(),
            violations,
        };
    }
    let header = SlottedPageHeader::deserialize(&data[..SlottedPageHeader::SIZE]);
    let mut out = DataPageCheck {
        page_type: header.page_type,
        next_page_id: header.next_page_id,
        locators: Vec::new(),
        violations,
    };
    let slots = match validate_slots(data, &header, &mut out.violations) {
        Some(slots) => slots,
        None => return out, // slot iteration would index out of bounds
    };
    for (_i, _slot, start, end) in slots {
        if end - start > VersionHeader::SIZE {
            if let Some(locator) = decode_locator(&data[start + VersionHeader::SIZE..end]) {
                out.locators.push(locator);
            }
        }
    }
    out
}

/// Bounds-check a slotted page's header and slots. Returns `None` when the
/// slot count alone makes iteration unsafe (violation pushed); otherwise a
/// list of `(index, slot, data start, data end)` for the in-bounds slots
/// (out-of-bounds slots are skipped with a violation, not dropped silently).
fn validate_slots(
    data: &[u8],
    header: &SlottedPageHeader,
    violations: &mut Vec<String>,
) -> Option<Vec<(usize, Slot, usize, usize)>> {
    let slot_area_start = Page::PAGE_SIZE.saturating_sub(header.slot_count as usize * Slot::SIZE);
    if slot_area_start < SlottedPageHeader::SIZE {
        violations.push(format!(
            "slot count {} exceeds the page capacity",
            header.slot_count
        ));
        return None;
    }
    let fso = header.free_space_offset as usize;
    if fso < SlottedPageHeader::SIZE || fso > slot_area_start {
        violations.push(format!(
            "free space offset {fso} lies outside the data area [{}, {slot_area_start}]",
            SlottedPageHeader::SIZE
        ));
    }
    let page = SlottedPageRef::new(data);
    let mut slots = Vec::with_capacity(header.slot_count as usize);
    for i in 0..header.slot_count as usize {
        let slot = match page.get_slot(i) {
            Some(slot) => slot,
            None => break,
        };
        let start = slot.offset as usize;
        let end = start + slot.length as usize;
        if start < SlottedPageHeader::SIZE || end > slot_area_start {
            violations.push(format!(
                "slot {i} data range [{start}, {end}) overlaps the page header or the slot area"
            ));
            continue;
        }
        slots.push((i, slot, start, end));
    }
    Some(slots)
}

/// Outcome of validating one index-tree page image.
struct TreePageCheck {
    children: Vec<u32>,
    /// Set when the page's type belongs to no tree (subtree not traversed).
    invalid_type: Option<u8>,
    violations: Vec<String>,
}

/// Validate an index-tree page: leaf pages get the same slot bounds check as
/// data pages (no locator probing — leaf entries are `[Key][RowId]` and can
/// legitimately start with any byte); internal pages additionally yield their
/// child pointers, each entry validated to carry `[Key][child u32]`.
fn check_tree_page(data: &[u8]) -> TreePageCheck {
    let mut out = TreePageCheck {
        children: Vec::new(),
        invalid_type: None,
        violations: Vec::new(),
    };
    if data.len() < SlottedPageHeader::SIZE {
        out.violations
            .push("page is shorter than a page header".to_string());
        return out;
    }
    let header = SlottedPageHeader::deserialize(&data[..SlottedPageHeader::SIZE]);
    match header.page_type {
        LEAF_NODE => {
            let mut violations = Vec::new();
            // Slot bounds only — the child is the sibling chain pointer.
            let _ = validate_slots(data, &header, &mut violations);
            out.violations = violations;
            if header.next_page_id > 0 {
                out.children.push(header.next_page_id);
            }
        }
        INTERNAL_NODE => {
            // The leftmost child lives in the header's `next_page_id` field
            // (InternalNodeRef::leftmost_child) — same slot bounds check as
            // data pages, plus per-entry child extraction.
            let mut violations = Vec::new();
            if let Some(slots) = validate_slots(data, &header, &mut violations) {
                for (i, _slot, start, end) in slots {
                    if end - start < MAX_KEY_LEN + 4 {
                        violations.push(format!(
                            "slot {i} is too short for an internal-node entry ({} bytes, need {})",
                            end - start,
                            MAX_KEY_LEN + 4
                        ));
                        continue;
                    }
                    out.children.push(u32::from_le_bytes([
                        data[start + MAX_KEY_LEN],
                        data[start + MAX_KEY_LEN + 1],
                        data[start + MAX_KEY_LEN + 2],
                        data[start + MAX_KEY_LEN + 3],
                    ]));
                }
            }
            if header.next_page_id > 0 {
                out.children.push(header.next_page_id);
            }
            out.violations = violations;
        }
        other => out.invalid_type = Some(other),
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::network::protocol::Response;
    use crate::storage::file_header::HEADER_SIZE;
    use crate::storage::page_format::overflow::{chunk_plan, collect_overflow_pages};
    use crate::storage::page_format::SlottedPageRef;
    use std::path::Path;
    use tempfile::tempdir;

    async fn exec_ok(db: &Database, sql: &str) {
        let resp = db.execute_sql(sql).await;
        assert!(
            !matches!(resp, Response::Error { .. }),
            "statement failed: {sql:?} -> {resp:?}"
        );
    }

    /// Build a database with small rows plus one ~10KB overflow row, then
    /// checkpoint so the file is complete and the WAL is empty — injected
    /// byte edits survive a reopen (no recovery rewrite).
    async fn build_db(path: &Path, big_len: usize) -> Database {
        let db = Database::open(path).await.unwrap();
        exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;
        for i in 0..5 {
            exec_ok(&db, &format!("INSERT INTO t VALUES ({i}, 'v{i}')")).await;
        }
        if big_len > 0 {
            exec_ok(
                &db,
                &format!("INSERT INTO t VALUES (100, '{}')", "x".repeat(big_len)),
            )
            .await;
        }
        db.checkpoint().await.unwrap();
        db
    }

    fn poke(path: &Path, offset: u64, value: u8) {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        f.seek(SeekFrom::Start(offset)).unwrap();
        f.write_all(&[value]).unwrap();
    }

    fn poke_u32(path: &Path, offset: u64, value: u32) {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        f.seek(SeekFrom::Start(offset)).unwrap();
        f.write_all(&value.to_le_bytes()).unwrap();
    }

    fn page_file_offset(page: u32, in_page: usize) -> u64 {
        (HEADER_SIZE + page as usize * Page::PAGE_SIZE + in_page) as u64
    }

    /// On-disk locations of the overflow row: the chain page ids in order
    /// and the absolute file offset of the locator tag byte (the locator
    /// occupies the tuple segment right after the version header).
    struct Locations {
        chain: Vec<u32>,
        locator_offset: u64,
    }

    async fn locate(path: &Path) -> Locations {
        let db = Database::open(path).await.unwrap();
        let meta = db.table_manager.get_table("t").await.unwrap();
        // The big row may not fit the head page's remaining space and land on
        // a fresh chain page — walk the whole data chain to find the locator.
        let mut found: Option<(u32, u64)> = None;
        let mut page = Some(meta.data_page_head.0 as u32);
        let mut visited = std::collections::HashSet::new();
        while let Some(raw) = page {
            assert!(visited.insert(raw), "data chain cycle in test fixture");
            let hit = db
                .buffer_pool
                .with_page_data(crate::storage::PageId(raw as u64), |data| {
                    let page = SlottedPageRef::new(data);
                    for i in 0..page.slot_count() {
                        let slot = page.get_slot(i).unwrap();
                        let start = slot.offset as usize;
                        let end = start + slot.length as usize;
                        if let Some((_, first)) =
                            decode_locator(&data[start + VersionHeader::SIZE..end])
                        {
                            let offset = (HEADER_SIZE
                                + raw as usize * Page::PAGE_SIZE
                                + start
                                + VersionHeader::SIZE)
                                as u64;
                            return Ok(Some((first, offset)));
                        }
                    }
                    Ok(None)
                })
                .await
                .unwrap();
            if let Some((first, offset)) = hit {
                found = Some((first, offset));
                break;
            }
            let next = db
                .buffer_pool
                .with_page_data(crate::storage::PageId(raw as u64), |data| {
                    Ok(SlottedPageRef::new(data).header().next_page_id)
                })
                .await
                .unwrap();
            page = if next == 0 { None } else { Some(next) };
        }
        let (first_page, locator_offset) =
            found.expect("test fixture must contain an overflow row");
        let chain = collect_overflow_pages(&db.buffer_pool, first_page)
            .await
            .unwrap();
        drop(db);
        Locations {
            chain,
            locator_offset,
        }
    }

    async fn check(db: &Database) -> IntegrityReport {
        check_integrity(&db.buffer_pool, &db.table_manager)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn clean_db_zero_findings_and_census_counts() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("clean.db");
        let db = build_db(&path, 10_000).await;
        drop(db);

        let db = Database::open(&path).await.unwrap();
        let report = check(&db).await;

        assert_eq!(report.error_count(), 0, "findings: {:?}", report.findings);
        assert_eq!(report.info_count(), 0, "findings: {:?}", report.findings);
        assert!(report.total_pages > 0);
        assert_eq!(
            report.census.values().sum::<u64>(),
            report.total_pages,
            "census must cover every page exactly once"
        );
        // tuple = int(9) + string(3 + 10_000) = 10_012 payload bytes →
        // 4080 + 4080 + 1852 → exactly 3 overflow pages.
        assert_eq!(
            chunk_plan(10_012).len(),
            3,
            "test precondition: the 10KB row spans 3 chunks"
        );
        assert_eq!(
            report.census.get(&OVERFLOW_PAGE_TYPE),
            Some(&3),
            "census: {:?}",
            report.census
        );
        assert!(
            report.census.get(&SLOTTED_PAGE_TYPE).copied().unwrap_or(0) >= 3,
            "census must count the two catalog pages and the data page: {:?}",
            report.census
        );
    }

    #[tokio::test]
    async fn unknown_page_type_detected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("corrupt.db");
        let db = build_db(&path, 0).await;
        drop(db);
        let head = Database::open(&path)
            .await
            .unwrap()
            .table_manager
            .get_table("t")
            .await
            .unwrap()
            .data_page_head
            .0 as u32;
        poke(&path, page_file_offset(head, 0), 0x42);

        let db = Database::open(&path).await.unwrap();
        let report = check(&db).await;

        assert!(
            report.error_count() > 0,
            "corruption must be reported: {:?}",
            report.findings
        );
        assert_eq!(report.census.get(&0x42), Some(&1));
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.page == Some(head) && f.message.contains("unknown page type")),
            "findings: {:?}",
            report.findings
        );
    }

    #[tokio::test]
    async fn slot_area_corruption_detected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("corrupt.db");
        let db = build_db(&path, 0).await;
        drop(db);
        let head = Database::open(&path)
            .await
            .unwrap()
            .table_manager
            .get_table("t")
            .await
            .unwrap()
            .data_page_head
            .0 as u32;
        // slot_count lives in header bytes 1..3 — 0xFFFF exceeds any page.
        poke(&path, page_file_offset(head, 1), 0xFF);
        poke(&path, page_file_offset(head, 2), 0xFF);

        let db = Database::open(&path).await.unwrap();
        let report = check(&db).await;

        assert!(
            report
                .findings
                .iter()
                .any(|f| f.page == Some(head) && f.message.contains("slot count")),
            "findings: {:?}",
            report.findings
        );
    }

    #[tokio::test]
    async fn overflow_chain_cycle_detected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("corrupt.db");
        let db = build_db(&path, 10_000).await;
        drop(db);
        let loc = locate(&path).await;
        assert!(loc.chain.len() >= 2, "test precondition: multi-page chain");
        // Point the second chain page back at the first: P1 → P2 → P1 …
        poke_u32(&path, page_file_offset(loc.chain[1], 5), loc.chain[0]);

        let db = Database::open(&path).await.unwrap();
        let report = check(&db).await;

        assert!(
            report
                .findings
                .iter()
                .any(|f| f.severity == FindingSeverity::Error && f.message.contains("cyclic")),
            "findings: {:?}",
            report.findings
        );
    }

    #[tokio::test]
    async fn index_tree_page_type_destroyed_detected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("corrupt.db");
        let db = build_db(&path, 0).await;
        drop(db);
        let pk_root = Database::open(&path)
            .await
            .unwrap()
            .table_manager
            .get_table("t")
            .await
            .unwrap()
            .index_manager
            .root_page_id()
            .0 as u32;
        poke(&path, page_file_offset(pk_root, 0), 0x42);

        let db = Database::open(&path).await.unwrap();
        let report = check(&db).await;

        assert!(
            report.findings.iter().any(|f| f.page == Some(pk_root)
                && f.severity == FindingSeverity::Error
                && f.message.contains("primary key")),
            "findings: {:?}",
            report.findings
        );
    }

    #[tokio::test]
    async fn unreferenced_overflow_pages_reported_as_info() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("leak.db");
        let db = build_db(&path, 10_000).await;
        drop(db);
        let loc = locate(&path).await;
        // Overwrite the locator tag byte: the row stops referencing its
        // chain, which becomes unreferenced leaked space (no structural
        // damage anywhere).
        poke(&path, loc.locator_offset, 0x01);

        let db = Database::open(&path).await.unwrap();
        let report = check(&db).await;

        assert_eq!(report.error_count(), 0, "findings: {:?}", report.findings);
        let mut infos: Vec<u32> = report
            .findings
            .iter()
            .filter(|f| f.severity == FindingSeverity::Info)
            .filter_map(|f| f.page)
            .collect();
        infos.sort_unstable();
        let mut expected = loc.chain.clone();
        expected.sort_unstable();
        assert_eq!(infos, expected, "findings: {:?}", report.findings);
        assert!(
            report.findings.iter().all(
                |f| f.severity != FindingSeverity::Info || f.message.contains("not referenced")
            ),
            "findings: {:?}",
            report.findings
        );
    }
}
