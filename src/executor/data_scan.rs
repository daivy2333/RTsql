//! M19: DataScan executor - direct data page chain traversal
//!
//! Bypasses IndexManager.scan_all() by iterating the data page linked list
//! (`SlottedPageHeader.next_page_id`). Each row triggers only one page access
//! (the data page), halving page accesses compared to `ScanExecutor` (which
//! walks BTree index then fetches the data page).
//!
//! Streaming `next()`: returns one row per call without pre-allocating a
//! `Vec<Vec<Value>>` like the existing `ScanExecutor` does.
//!
//! MVCC visibility: when a `Snapshot` is provided, each slot's `VersionHeader`
//! is parsed and checked. Invisible current versions follow `next_version`
//! pointers (potentially across pages) to find a visible commit.

use crate::executor::apply_projection;
use crate::executor::{ExecResult, Executor, PredicateRef, Value};
use crate::storage::page_format::{deserialize_value_refs, ColumnType, RowId, SlottedPageRef};
use crate::storage::PageId;
use crate::storage::{read_tuple_from_data_page, BufferPool, Result, TableMeta};
use crate::transaction::{Snapshot, VersionHeader};
use std::sync::Arc;

/// Action returned from a per-page closure describing what the outer loop
/// should do next.
enum PageAction {
    /// Yield this row and continue from the next slot on the next call.
    YieldValue(Vec<Value>),
    /// Current version is invisible to the snapshot. `Some(rid)` = start of
    /// version chain to follow; `None` = no chain, just skip the slot.
    NeedVersionChain(Option<RowId>),
    /// Page is exhausted; jump to the page with the given id and reset slot index.
    JumpToPage(u64),
    /// End of scan (next_page_id == 0).
    Done,
}

pub struct DataScanExecutor {
    buffer_pool: Arc<BufferPool>,
    schema: Vec<ColumnType>,
    snapshot: Option<Snapshot>,
    /// MS07-T06: row-level predicate pushed down from the planner. Evaluated
    /// against the same full-schema-order row that a wrapping `FilterExecutor`
    /// would see; `None` means no inline filtering.
    predicate: Option<PredicateRef>,
    /// MS07-T06: maximum number of rows to yield (pushed down from LIMIT as
    /// `offset + limit`; `Some(0)` yields nothing). Counted after visibility
    /// and inline-predicate passes. `None` = unbounded.
    scan_cap: Option<usize>,
    /// Rows yielded so far (only meaningful when `scan_cap` is set).
    produced: usize,
    /// Current data page being scanned; `None` means scan is complete or table is empty.
    current_page_id: Option<PageId>,
    /// Next slot index to read on the current page.
    current_slot_index: usize,
    /// MS08-T02: whether successor-page prefetching is enabled (default off
    /// since the 2026-09-05 replan — the default path measured a 17-47%
    /// regression from per-page task spawn in warm-cache environments;
    /// `with_prefetch(true)` opts in, `with_prefetch(false)` stays available
    /// as the explicit control path).
    prefetch_enabled: bool,
    /// MS08-T02: in-flight prefetch task handle. At most one tracked
    /// (untaken) handle exists at any time; dropping it never aborts the
    /// task — a running prefetch simply finishes loading its page.
    prefetch_handle: Option<tokio::task::JoinHandle<()>>,
    /// MS08-T02: page id already prefetched (dedups successor captures
    /// across the slots of one page).
    prefetched_page: Option<PageId>,
    /// MS10-T01 Iter001: output projection (empty = identity), applied to a
    /// row after visibility and the inline predicate have been evaluated.
    projection: Vec<usize>,
}

impl DataScanExecutor {
    pub fn new(
        table_meta: Arc<TableMeta>,
        buffer_pool: Arc<BufferPool>,
        snapshot: Option<Snapshot>,
        predicate: Option<PredicateRef>,
        scan_cap: Option<usize>,
    ) -> Self {
        let schema: Vec<ColumnType> = table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect();
        // data_page_head is always a valid page id (allocated in create_table).
        // An empty table still has a page; we detect emptiness via slot_count
        // inside the scan loop.
        let current_page_id = Some(table_meta.data_page_head);
        Self {
            buffer_pool,
            schema,
            snapshot,
            predicate,
            scan_cap,
            produced: 0,
            current_page_id,
            current_slot_index: 0,
            prefetch_enabled: false,
            prefetch_handle: None,
            prefetched_page: None,
            projection: Vec::new(),
        }
    }

    /// MS10-T01 Iter001: narrow produced rows to the given full-schema column
    /// indices (empty = identity, the `new()` default). Applied after the
    /// inline predicate so predicate `column_index` keeps its full-schema
    /// semantics.
    pub fn with_projection(mut self, projection: Vec<usize>) -> Self {
        self.projection = projection;
        self
    }

    /// MS08-T02: override the prefetch switch (default `false` via `new`;
    /// `true` opts in).
    pub fn with_prefetch(mut self, enabled: bool) -> Self {
        self.prefetch_enabled = enabled;
        self
    }

    /// MS08-T02: fire a background prefetch for `next` so its cache miss
    /// overlaps with processing the remaining rows of the current page.
    /// Results and errors are discarded — the real read path via
    /// `with_page_data` is authoritative and joins the same per-page load
    /// through the BufferPool's per-page loading locks. Dedup by page id;
    /// the chain-end sentinel `PageId(0)` is never prefetched (checked by
    /// the caller).
    fn trigger_prefetch(&mut self, next: PageId) {
        if !self.prefetch_enabled || self.prefetched_page == Some(next) {
            return;
        }
        let _ = self.prefetch_handle.take();
        let buffer_pool = Arc::clone(&self.buffer_pool);
        self.prefetch_handle = Some(tokio::spawn(async move {
            let _ = buffer_pool.get_page(next).await;
        }));
        self.prefetched_page = Some(next);
    }

    /// Apply the pushed-down predicate to a candidate row with the exact
    /// `FilterExecutor` semantics (same evaluation row and same error text).
    /// `Ok(None)` = row filtered out.
    fn filter_row(
        predicate: Option<&PredicateRef>,
        values: Vec<Value>,
    ) -> Result<Option<Vec<Value>>> {
        match predicate {
            None => Ok(Some(values)),
            Some(p) => match p.evaluate(&values) {
                Ok(true) => Ok(Some(values)),
                Ok(false) => Ok(None),
                Err(e) => Err(crate::storage::StorageError::ExecutionError(format!(
                    "Predicate evaluation error: {}",
                    e
                ))),
            },
        }
    }

    /// Yield a row that already passed visibility and the inline predicate,
    /// enforcing the pushed-down scan cap. Reaching the cap ends the scan the
    /// way `LimitExecutor` ends when its input is exhausted.
    fn yield_capped(&mut self, values: Vec<Value>) -> Result<Option<ExecResult>> {
        match self.scan_cap {
            None => Ok(Some(ExecResult::Row(values))),
            Some(cap) => {
                if self.produced >= cap {
                    self.current_page_id = None;
                    return Ok(None);
                }
                self.produced += 1;
                Ok(Some(ExecResult::Row(values)))
            }
        }
    }

    /// Walk the version chain from `start_rid`, returning the first version
    /// visible to `snapshot`. Returns `Ok(None)` if no visible version exists
    /// in the chain (e.g., the row was created after the snapshot, or every
    /// older version is also invisible).
    ///
    /// Note: this helper does not hold a reference to `&mut self` so it can
    /// be invoked from `next()` without splitting borrows.
    async fn find_visible_in_chain(
        buffer_pool: &BufferPool,
        start_rid: RowId,
        snapshot: &Snapshot,
        schema: &[ColumnType],
    ) -> Result<Option<Vec<Value>>> {
        let mut current = Some(start_rid);
        let mut depth = 0usize;
        const MAX_CHAIN_DEPTH: usize = 64; // safety bound
        while let Some(rid) = current {
            if depth >= MAX_CHAIN_DEPTH {
                return Err(crate::storage::StorageError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "version chain too deep",
                )));
            }
            depth += 1;
            let result: Result<(VersionHeader, Vec<u8>)> =
                read_tuple_from_data_page(buffer_pool, rid, |vh, bytes| Ok((vh, bytes.to_vec())))
                    .await;
            let (vh, bytes) = result?;
            if snapshot.is_visible(vh.create_tx_id(), vh.commit_tx_id()) {
                let vrs = deserialize_value_refs(&bytes, schema)?;
                return Ok(Some(vrs.iter().map(|vr| vr.to_value()).collect()));
            }
            current = vh.next_version();
        }
        Ok(None)
    }
}

#[async_trait::async_trait]
impl Executor for DataScanExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        loop {
            let page_id = match self.current_page_id {
                Some(p) => p,
                None => return Ok(None), // scan complete
            };

            // Snapshot schema and slot index into locals for the closure.
            // The closure is `FnOnce` (synchronous) so we can move `slot_index`
            // in, mutate it, then write it back to `self` after the await.
            let schema = self.schema.clone();
            let mut slot_index = self.current_slot_index;
            // MS08-T02: the closure captures the current page's successor for
            // the post-await prefetch trigger (header read is in-memory only).
            let mut captured_next: u32 = 0;

            // M21: Page-level visibility fast-path.
            // Query the visibility summary map before entering the page-data closure
            // (the closure is synchronous FnOnce so it cannot access self.buffer_pool).
            let page_vis = self.buffer_pool.get_visibility(page_id);
            let page_all_visible = page_vis.map(|v| v.all_visible).unwrap_or(false);
            let page_all_invisible = self
                .snapshot
                .as_ref()
                .zip(page_vis)
                .map(|(s, v)| v.all_invisible_for(s.tx_id()))
                .unwrap_or(false);

            let snapshot_ref = self.snapshot.as_ref();

            let action: PageAction = self
                .buffer_pool
                .with_page_data(page_id, |data| -> Result<PageAction> {
                    let slotted = SlottedPageRef::new(data);
                    let slot_count = slotted.slot_count();
                    // MS08-T02: capture the successor once per closure entry;
                    // reused by the all-invisible/exhausted branches below.
                    let next = slotted.header().next_page_id;
                    captured_next = next;

                    // M21: All-invisible fast-path — skip the entire page.
                    if page_all_invisible {
                        return Ok(if next == 0 {
                            PageAction::Done
                        } else {
                            PageAction::JumpToPage(next as u64)
                        });
                    }

                    if slot_index >= slot_count {
                        // Page exhausted → follow the linked list.
                        return Ok(if next == 0 {
                            PageAction::Done
                        } else {
                            PageAction::JumpToPage(next as u64)
                        });
                    }

                    let slot = slotted
                        .get_slot(slot_index)
                        .expect("slot_index < slot_count validated above");
                    let slot_data = slotted.get_slot_data(&slot);
                    slot_index += 1; // consume this slot regardless of outcome

                    if slot_data.len() < VersionHeader::SIZE {
                        // Malformed slot — skip to next slot.
                        return Ok(PageAction::NeedVersionChain(None));
                    }
                    let vh = VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE])
                        .ok_or_else(|| {
                            crate::storage::StorageError::Io(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "malformed version header",
                            ))
                        })?;
                    let tuple_bytes = &slot_data[VersionHeader::SIZE..];

                    // Skip deleted rows (commit_tx_id = DELETED_TX_ID sentinel)
                    if vh.is_deleted() {
                        return Ok(PageAction::NeedVersionChain(None));
                    }

                    // M21: Skip per-row MVCC visibility check when the whole page
                    // is known to be all-visible (every slot committed).
                    if !page_all_visible {
                        if let Some(snapshot) = snapshot_ref {
                            if !snapshot.is_visible(vh.create_tx_id(), vh.commit_tx_id()) {
                                return Ok(PageAction::NeedVersionChain(vh.next_version()));
                            }
                        }
                    }

                    // Visible (or no snapshot) — deserialize and yield.
                    let vrs = deserialize_value_refs(tuple_bytes, &schema)?;
                    let values: Vec<Value> = vrs.iter().map(|vr| vr.to_value()).collect();
                    Ok(PageAction::YieldValue(values))
                })
                .await?;

            // Commit slot index back regardless of action.
            self.current_slot_index = slot_index;

            // MS08-T02: the closure revealed the current page's successor —
            // prefetch it so the next page's miss load overlaps with the
            // remaining rows of this page. `Done`/chain-end capture 0,
            // which never triggers.
            if captured_next != 0 {
                self.trigger_prefetch(PageId(captured_next as u64));
            }

            // M21: Lazy set_all_visible — after scanning all slots on a page,
            // check if the entire page is visible to this snapshot and cache the result.
            // Condition: snapshot exists, all_visible not already set, page not all-invisible,
            // and action is JumpToPage/Done (page exhausted, not a per-row yield/chain).
            if self.snapshot.is_some()
                && !page_all_visible
                && !page_all_invisible
                && matches!(action, PageAction::JumpToPage(_) | PageAction::Done)
            {
                if let Some(snapshot) = self.snapshot.as_ref() {
                    if self
                        .buffer_pool
                        .check_page_all_visible(page_id, snapshot)
                        .await
                    {
                        self.buffer_pool.set_all_visible(page_id);
                    }
                }
            }

            match action {
                PageAction::YieldValue(values) => {
                    match Self::filter_row(self.predicate.as_ref(), values)? {
                        Some(values) => {
                            return self.yield_capped(apply_projection(&self.projection, values))
                        }
                        None => continue, // filtered out by the inline predicate
                    }
                }
                PageAction::NeedVersionChain(None) => {
                    // No chain — skip this slot, continue to next.
                    continue;
                }
                PageAction::NeedVersionChain(Some(rid)) => {
                    if let Some(snapshot) = self.snapshot.as_ref() {
                        match Self::find_visible_in_chain(
                            &self.buffer_pool,
                            rid,
                            snapshot,
                            &self.schema,
                        )
                        .await?
                        {
                            Some(values) => {
                                match Self::filter_row(self.predicate.as_ref(), values)? {
                                    Some(values) => {
                                        return self.yield_capped(apply_projection(
                                            &self.projection,
                                            values,
                                        ))
                                    }
                                    None => continue, // filtered out by the inline predicate
                                }
                            }
                            None => continue, // no visible version in chain
                        }
                    } else {
                        // Defensive: no snapshot but chain needed shouldn't happen.
                        continue;
                    }
                }
                PageAction::JumpToPage(next_id) => {
                    self.current_page_id = Some(PageId(next_id));
                    self.current_slot_index = 0;
                    // Continue the outer loop to read the first slot of the new page.
                }
                PageAction::Done => {
                    self.current_page_id = None;
                    return Ok(None);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{data::TableManager, page_format::ColumnType, FileStorage};

    /// MS08-T02 replan (2026-09-05): `new` must default prefetch OFF — the
    /// default path measured a 17-47% regression from per-page task spawn in
    /// warm-cache environments, so prefetch is opt-in via `with_prefetch(true)`.
    /// This guards the default switch value itself (non-vacuous: it
    /// distinguishes the default from explicit opt-in); behavioral equivalence
    /// in both switch states is covered by `tests/prefetch_test.rs`.
    #[tokio::test]
    async fn new_defaults_prefetch_off() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("t.db")).unwrap());
        let buffer_pool = Arc::new(BufferPool::new(8, storage.clone()).unwrap());
        let table_mgr = TableManager::new(buffer_pool.clone(), storage)
            .await
            .unwrap();
        table_mgr
            .create_table("t", vec![("id".to_string(), ColumnType::Int)], "id")
            .await
            .unwrap();
        let table_meta = table_mgr.get_table("t").await.unwrap();

        let default_executor =
            DataScanExecutor::new(table_meta.clone(), buffer_pool.clone(), None, None, None);
        assert!(
            !default_executor.prefetch_enabled,
            "new() must default to prefetch disabled"
        );

        let explicit_on =
            DataScanExecutor::new(table_meta, buffer_pool, None, None, None).with_prefetch(true);
        assert!(
            explicit_on.prefetch_enabled,
            "with_prefetch(true) must explicitly enable prefetch"
        );
    }
}
