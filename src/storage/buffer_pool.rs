use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};

use dashmap::DashMap;

use crate::storage::{
    corrupted_data_page_slot_count_error, corrupted_data_page_slot_error,
    page_format::{overflow, SlottedPageRef},
    page_frame::{PageFrame, PageGuard},
    short_data_page_slot_error, AsyncStorage, Page, PageId, PageVisibilityInfo, Result, RowId,
    StorageError, MIN_CREATE_UNKNOWN,
};
use crate::transaction::{Snapshot, VersionHeader};

/// Maximum number of in-flight cache miss loads.
/// Covers typical NVMe SSD QD (16-32) without wasting permit memory.
/// See `design.md` 决策 2 for the selection rationale.
const MISS_SEMAPHORE_PERMITS: usize = 16;

pub struct BufferPool {
    /// Page cache: PageId → Arc<Mutex<PageFrame>>
    ///
    /// M31: Migrated from `RwLock<HashMap<...>>` to `DashMap<...>` for lock-free
    /// concurrent reads. Each shard is independently RwLocked, so cache hit
    /// path (`pages.get`) is fully lock-free. Writes (`pages.insert`/`remove`)
    /// lock only the affected shard.
    ///
    /// SAFETY: Each PageFrame uses std::sync::Mutex. This is safe because:
    /// 1. PageGuard/PageDataGuard are never held across .await points
    /// 2. All Mutex lock/unlock operations are in sync code (no .await between lock and unlock)
    pages: DashMap<PageId, Arc<std::sync::Mutex<PageFrame>>>,
    /// Per-page MVCC visibility map for fast-path skipping.
    /// DashMap provides lock-free concurrent access — no deadlock risk
    /// with existing page cache locks.
    vis_map: DashMap<PageId, PageVisibilityInfo>,
    /// M31: Bounded concurrency for in-flight cache miss loads.
    /// Held in `Arc` so we can use `Semaphore::acquire_owned()` (the permit
    /// returned by `acquire()` is not `Send` across .await points).
    miss_sem: Arc<Semaphore>,
    /// M31: Per-page load locks to serialize concurrent loads of the same
    /// page. Ensures the double-check pattern in `get_page` is correct
    /// (R1: "Double-checked single loading per page"): only one thread
    /// can be in the load path for a given page at a time.
    loading_locks: DashMap<PageId, Arc<tokio::sync::Mutex<()>>>,
    clock_hand: RwLock<Vec<PageId>>,
    capacity: usize,
    storage: Arc<dyn AsyncStorage>,
}

/// 持帧锁复制脏页并标记写入在途。已经在途，或当前不脏，则本轮不发写。
fn claim_dirty_for_write(frame: &std::sync::Mutex<PageFrame>) -> Option<Page> {
    let mut guard = frame.lock().unwrap();
    if guard.write_in_flight || !guard.dirty {
        return None;
    }
    let page = guard.page.clone();
    guard.write_in_flight = true;
    guard.dirty = false;
    Some(page)
}

impl BufferPool {
    pub fn new(capacity: usize, storage: Arc<dyn AsyncStorage>) -> Result<Self> {
        if capacity == 0 {
            return Err(StorageError::InvalidCapacity(capacity));
        }

        Ok(Self {
            pages: DashMap::new(),
            vis_map: DashMap::new(),
            miss_sem: Arc::new(Semaphore::new(MISS_SEMAPHORE_PERMITS)),
            loading_locks: DashMap::new(),
            clock_hand: RwLock::new(Vec::new()),
            capacity,
            storage,
        })
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get storage reference for page allocation
    pub fn storage(&self) -> &Arc<dyn AsyncStorage> {
        &self.storage
    }

    pub async fn get_page(&self, page_id: PageId) -> Result<PageGuard> {
        // 1. Lock-free cache hit check
        if let Some(frame) = self.pages.get(&page_id) {
            return Ok(PageGuard::new(frame.clone()));
        }

        // 2. Cache miss path: acquire miss permit to bound in-flight IO loads.
        // The permit is held for the entire miss path (including eviction)
        // and released at scope end (success or error).
        let permit = self
            .miss_sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| StorageError::SemaphoreClosed)?;

        // 3. Per-page load serialization. DashMap doesn't provide a global
        // write lock, so multiple threads can pass the double-check below and
        // all call storage.read_page. We need a per-page mutex to ensure
        // exactly one thread loads each page (correctness for R1/E1).
        //
        // Acquire ordering: miss_sem → loading_lock → pages → clock_hand.
        // loading_lock is per-page, so different pages load in parallel.
        let loading_lock = self
            .loading_locks
            .entry(page_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _load_guard = loading_lock.lock().await;

        // 4. Double-check under per-page lock — another thread may have
        // loaded the page in the gap between step 1 and the load lock.
        if let Some(frame) = self.pages.get(&page_id) {
            return Ok(PageGuard::new(frame.clone()));
        }

        // 5. Capacity check + eviction
        if self.pages.len() >= self.capacity {
            self.evict_one().await?;
        }

        // 6. Load from storage (storage errors propagate; permit still released by Drop)
        let page = self.storage.read_page(page_id).await?;
        let frame = Arc::new(std::sync::Mutex::new(PageFrame::new(page)));

        // 7. Insert into cache (DashMap shard-locked; no global lock)
        self.pages.insert(page_id, frame.clone());
        self.clock_hand.write().await.push(page_id);

        drop(permit);

        Ok(PageGuard::new(frame))
    }

    /// M20: Zero-copy page data accessor (closure-based API).
    ///
    /// Returns a `&[u8]` view (4096 bytes) to the page data without copying.
    /// The closure receives the slice, may construct a `SlottedPageRef` to
    /// read slots, and the page lock is released when the closure returns.
    ///
    /// Why a closure (not `Result<PageDataGuard<'_>>`): the `MutexGuard`
    /// inside `PageDataGuard` borrows from the page frame, so the borrow
    /// cannot be extended beyond the function call scope in safe Rust
    /// (E0505). The closure keeps the borrow scope local — a Rust async
    /// + lock standard pattern. See `learned/spec.md` L022 for the
    ///   3 failed attempts that led to this design.
    ///
    /// SAFETY:
    /// - Closure must NOT call `.await` (the page lock is `std::sync::Mutex`,
    ///   not `Send`, cannot cross await boundary).
    /// - Closure must NOT recursively call other `BufferPool` methods
    ///   (deadlock — `BufferPool` acquires other locks).
    pub async fn with_page_data<F, R>(&self, page_id: PageId, f: F) -> Result<R>
    where
        F: FnOnce(&[u8]) -> Result<R>,
    {
        let guard = self.get_page(page_id).await?;
        let data_guard = guard.page_data();
        f(&data_guard)
    }

    async fn evict_one(&self) -> Result<()> {
        let mut clock_hand = self.clock_hand.write().await;
        let mut attempts = 0;
        let max_attempts = clock_hand.len() * 2;

        while attempts < max_attempts {
            if clock_hand.is_empty() {
                return Err(StorageError::BufferPoolFull);
            }

            let candidate_id = clock_hand.remove(0);
            attempts += 1;

            // Lock-free lookup on DashMap (per-shard RwLock).
            let frame = match self.pages.get(&candidate_id) {
                Some(f) => f.clone(),
                None => continue,
            };

            enum EvictChoice {
                Defer,
                RemoveClean,
                Write(Page),
            }

            let choice = {
                let mut frame_guard = frame.lock().unwrap();

                // 引用中与写入在途都不能移出池：在途页的脏标记已被清掉，
                // 但写盘还没返回，按干净页 remove 会丢掉尚未落盘的新内容。
                if frame_guard.ref_count > 0 || frame_guard.write_in_flight {
                    clock_hand.push(candidate_id);
                    EvictChoice::Defer
                } else if frame_guard.clock_bit {
                    frame_guard.clock_bit = false;
                    clock_hand.push(candidate_id);
                    EvictChoice::Defer
                } else if frame_guard.dirty {
                    let page = frame_guard.page.clone();
                    frame_guard.write_in_flight = true;
                    frame_guard.dirty = false;
                    EvictChoice::Write(page)
                } else {
                    EvictChoice::RemoveClean
                }
            };

            match choice {
                EvictChoice::Defer => continue,
                EvictChoice::RemoveClean => {
                    self.pages.remove(&candidate_id);
                    return Ok(());
                }
                EvictChoice::Write(page_copy) => {
                    let write_result = self.storage.write_page(candidate_id, &page_copy).await;
                    let failed = write_result.is_err();
                    let still_dirty = {
                        let mut frame_guard = frame.lock().unwrap();
                        frame_guard.write_in_flight = false;
                        if failed {
                            frame_guard.dirty = true;
                        }
                        frame_guard.dirty
                    };
                    if let Err(e) = write_result {
                        // 页留在池内，也留在 clock 上，否则这一帧占着容量却再也不会被选中。
                        clock_hand.push(candidate_id);
                        return Err(e);
                    }
                    if still_dirty {
                        // 等待写盘期间又被改脏：旧副本已经写出，新内容必须留在池内。
                        clock_hand.push(candidate_id);
                        continue;
                    }
                    self.pages.remove(&candidate_id);
                    return Ok(());
                }
            }
        }

        Err(StorageError::BufferPoolFull)
    }

    /// Flush all dirty pages to storage.
    ///
    /// M31: DashMap iteration holds a per-shard read lock (not a global lock).
    /// We collect dirty pages into a Vec first, drop all DashMap shard locks
    /// (iteration scope ends), then issue async writes without holding any
    /// page lock. This avoids `await_holding_lock` and allows concurrent
    /// `get_page` from other shards.
    pub async fn flush_all(&self) -> Result<()> {
        // Phase 1: collect dirty pages while holding per-shard locks briefly.
        let mut to_write: Vec<(PageId, Page)> = Vec::new();
        for entry in self.pages.iter() {
            if let Some(page) = claim_dirty_for_write(entry.value()) {
                to_write.push((*entry.key(), page));
            }
        }

        // Phase 2: write to storage (no locks held).
        //
        // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
        // Iteration 004 (RDP R1): the dirty bit is cleared in phase 1, so a
        // failure in phase 2 must hand the bit back for **every** page this
        // flush cleared but did not write — the failing page *and* the pages
        // after it that were never attempted. Restoring only the failing page
        // would leave the same hole: those pages would never be rewritten and
        // their content would be lost silently.
        let mut written = 0usize;
        for (page_id, page) in to_write.iter() {
            if let Err(e) = self.storage.write_page(*page_id, page).await {
                self.restore_dirty_from(&to_write, written);
                return Err(e);
            }
            self.finish_write_in_flight(*page_id, true);
            written += 1;
        }

        Ok(())
    }

    /// Give the dirty bit back to every page of `collected` from index
    /// `from` on — the pages this flush cleared but did not successfully
    /// write. A page that a concurrent writer re-dirtied in the meantime
    /// stays dirty (the bit is set again, which is its correct state).
    /// The in-flight flag is cleared for the same pages: the write attempt
    /// has returned.
    fn restore_dirty_from(&self, collected: &[(PageId, Page)], from: usize) {
        for (page_id, _) in collected.iter().skip(from) {
            self.finish_write_in_flight(*page_id, false);
        }
    }

    fn finish_write_in_flight(&self, page_id: PageId, success: bool) {
        if let Some(entry) = self.pages.get(&page_id) {
            let mut frame_guard = entry.value().lock().unwrap();
            frame_guard.write_in_flight = false;
            if !success {
                frame_guard.dirty = true;
            }
        }
    }

    /// change 2026-10-13 T3 (design D4): flush the given pages in the given
    /// order; non-dirty pages are skipped. Same per-page discipline as
    /// `flush_all` — the frame lock is held only to copy the page and clear
    /// the dirty bit, never across the storage write await (M17).
    pub async fn flush_pages(&self, page_ids: &[PageId]) -> Result<()> {
        for &page_id in page_ids {
            let page_copy = self
                .pages
                .get(&page_id)
                .and_then(|entry| claim_dirty_for_write(entry.value()));
            if let Some(page) = page_copy {
                // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
                // Iteration 004 (RDP R1): a failed write must hand the dirty
                // bit back — the page keeps its content pending instead of
                // being treated as already persisted.
                if let Err(e) = self.storage.write_page(page_id, &page).await {
                    self.restore_dirty_from(&[(page_id, page)], 0);
                    return Err(e);
                }
                self.finish_write_in_flight(page_id, true);
            }
        }
        Ok(())
    }

    /// change 2026-10-13 T3 (design D4): flush every dirty page except those
    /// in `exclude`. Same two-phase collect-then-write discipline as
    /// `flush_all`.
    pub async fn flush_all_except(
        &self,
        exclude: &std::collections::HashSet<PageId>,
    ) -> Result<()> {
        // Phase 1: collect dirty pages while holding per-shard locks briefly.
        let mut to_write: Vec<(PageId, Page)> = Vec::new();
        for entry in self.pages.iter() {
            if exclude.contains(entry.key()) {
                continue;
            }
            if let Some(page) = claim_dirty_for_write(entry.value()) {
                to_write.push((*entry.key(), page));
            }
        }

        // Phase 2: write to storage (no locks held).
        //
        // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
        // Iteration 004 (RDP R1): same discipline as `flush_all` — a failure
        // hands the dirty bit back to every page this flush cleared but did
        // not write.
        let mut written = 0usize;
        for (page_id, page) in to_write.iter() {
            if let Err(e) = self.storage.write_page(*page_id, page).await {
                self.restore_dirty_from(&to_write, written);
                return Err(e);
            }
            self.finish_write_in_flight(*page_id, true);
            written += 1;
        }

        Ok(())
    }

    /// MS18-T01: read-only probe — is any cached page dirty (not yet flushed
    /// to storage)?
    ///
    /// Same DashMap iteration discipline as `flush_all` Phase 1 (per-shard
    /// read lock, no global lock), but it neither clears nor sets the dirty
    /// bit and returns as soon as one dirty frame is seen. Used by the
    /// checkpoint zero-work guard, where "no WAL appends and no dirty pages"
    /// means there is nothing to flush, write or truncate.
    pub fn has_dirty_pages(&self) -> bool {
        self.pages
            .iter()
            .any(|entry| entry.value().lock().unwrap().dirty)
    }

    /// Read only the version header from a data page (M10, M20 closure form)
    ///
    /// Zero-copy: only the 8-byte `VersionHeader` is materialized, the
    /// tuple payload slice is dropped.
    pub async fn read_version_header(&self, row_id: RowId) -> Result<VersionHeader> {
        crate::storage::read_tuple_from_data_page(self, row_id, |vh, _bytes| Ok(vh)).await
    }

    /// Write commit transaction ID to version header (M10, M20 closure form)
    ///
    /// Needs owned `Vec<u8>` for the write path (WAL record, version
    /// chain update), so the closure calls `.to_vec()` inside the page lock.
    pub async fn write_commit_tx_id(&self, row_id: RowId, commit_tx_id: u64) -> Result<()> {
        let (version_header, tuple_bytes) =
            crate::storage::read_tuple_from_data_page(self, row_id, |vh, bytes| {
                Ok((vh, bytes.to_vec()))
            })
            .await?;
        let new_header = version_header.commit(commit_tx_id);
        crate::storage::update_version_header_in_data_page(self, row_id, new_header, &tuple_bytes)
            .await?;
        Ok(())
    }

    /// Traverse version chain to find first visible version (M20, closure-based).
    ///
    /// The closure receives the visible tuple's `&[u8]` (zero-copy) and
    /// must return `Result<R>` so deserialize errors propagate naturally.
    /// The page lock is held only inside the `with_page_data` closure;
    /// each version traversal acquires a fresh lock and releases it
    /// before the next iteration. Invisible versions only read the 8-byte
    /// `VersionHeader` (no payload copy).
    pub async fn find_visible_version<F, R>(
        &self,
        row_id: RowId,
        snapshot: &Snapshot,
        f: F,
    ) -> Result<Option<R>>
    where
        F: FnOnce(&[u8]) -> Result<R>,
    {
        // Internal enum: separates "visible (consume closure)" from
        // "invisible (continue traversal)" without leaking the closure
        // type out of the loop.
        enum VisibilityResult<R> {
            Visible(R),
            /// MS27 T4: the visible slot carries an overflow locator — the
            /// payload is assembled outside the page lock, then the closure
            /// consumes the assembled bytes.
            VisibleOverflow {
                total_len: u32,
                first_page: u32,
            },
            NotVisible(Option<RowId>),
            NotFound,
        }

        let mut f_opt = Some(f);
        let mut current_row_id = Some(row_id);

        // M21: Page-level visibility fast-path.
        // Check the visibility map before entering per-row version chain traversal.
        // If the page is all-visible (every row committed), skip per-row checks entirely.
        // If the page is all-invisible for this snapshot, return None immediately.
        if let Some(vis_info) = self.get_visibility(PageId(row_id.page_id as u64)) {
            if vis_info.all_visible {
                // All rows on this page are committed — skip per-row visibility checks.
                // MS27 T4: overflow slots exit the page lock with their locator
                // and are assembled afterwards (same as the main loop below).
                enum FastPath<R> {
                    Done(Option<R>),
                    Overflow { total_len: u32, first_page: u32 },
                }
                let fast = self
                    .with_page_data(PageId(row_id.page_id as u64), |data| {
                        let page_id = PageId(row_id.page_id as u64);
                        let slotted = SlottedPageRef::new(data);
                        // MS37 T17: 页头槽数越界 ⇒ 明确错误（取槽**之前**判；
                        // 否则槽目录定位下溢 ⇒ 越界切片 panic ⇒ 进程 abort）。
                        if slotted.slot_count_exceeds_capacity() {
                            return Err(corrupted_data_page_slot_count_error(
                                page_id.0,
                                slotted.slot_count(),
                            ));
                        }
                        let (slot, slot_index) =
                            match slotted.get_slot_by_logical_id(row_id.slot_id) {
                                Some(s) => s,
                                None => return Ok(FastPath::Done(None)),
                            };
                        // MS37 T3: 越界槽 / 过短负载 ⇒ 明确错误。快慢两支语义
                        // 一致——不得「查不到」而慢路径报错。
                        let slot_data = slotted.get_slot_data_checked(&slot).ok_or_else(|| {
                            corrupted_data_page_slot_error(page_id.0, slot_index, &slot)
                        })?;
                        if slot_data.len() < VersionHeader::SIZE {
                            return Err(short_data_page_slot_error(page_id.0, slot_index, &slot));
                        }
                        let tuple_bytes = &slot_data[VersionHeader::SIZE..];
                        if overflow::is_overflow_ref(tuple_bytes) {
                            let (total_len, first_page) = overflow::decode_locator(tuple_bytes)
                                .ok_or_else(|| {
                                    StorageError::Io(std::io::Error::new(
                                        std::io::ErrorKind::InvalidData,
                                        "malformed overflow locator",
                                    ))
                                })?;
                            return Ok(FastPath::Overflow {
                                total_len,
                                first_page,
                            });
                        }
                        let f = f_opt.take().expect("closure should be available");
                        let result = f(tuple_bytes)?;
                        Ok(FastPath::Done(Some(result)))
                    })
                    .await?;
                return match fast {
                    FastPath::Done(result) => Ok(result),
                    FastPath::Overflow {
                        total_len,
                        first_page,
                    } => {
                        let bytes =
                            overflow::read_overflow_chain(self, first_page, total_len).await?;
                        let f = f_opt.take().expect("closure should be available");
                        Ok(Some(f(&bytes)?))
                    }
                };
            }

            if vis_info.all_invisible_for(snapshot.high_water()) {
                return Ok(None);
            }
        }

        while let Some(current) = current_row_id {
            let visibility_result: VisibilityResult<R> = self
                .with_page_data(PageId(current.page_id as u64), |data| {
                    let current_page_id = PageId(current.page_id as u64);
                    let slotted = SlottedPageRef::new(data);
                    // MS37 T17: 页头槽数越界 ⇒ 明确错误（取槽**之前**判；否则
                    // 槽目录定位下溢 ⇒ 越界切片 panic ⇒ 进程 abort）。
                    if slotted.slot_count_exceeds_capacity() {
                        return Err(corrupted_data_page_slot_count_error(
                            current_page_id.0,
                            slotted.slot_count(),
                        ));
                    }
                    let (slot, slot_index) = match slotted.get_slot_by_logical_id(current.slot_id) {
                        Some(s) => s,
                        None => return Ok(VisibilityResult::<R>::NotFound),
                    };
                    // MS37 T3: 越界槽 / 过短负载 ⇒ 明确错误（今天短负载 panic）。
                    let slot_data = slotted.get_slot_data_checked(&slot).ok_or_else(|| {
                        corrupted_data_page_slot_error(current_page_id.0, slot_index, &slot)
                    })?;
                    if slot_data.len() < VersionHeader::SIZE {
                        return Err(short_data_page_slot_error(
                            current_page_id.0,
                            slot_index,
                            &slot,
                        ));
                    }
                    let version_header =
                        match VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE]) {
                            Some(vh) => vh,
                            None => return Ok(VisibilityResult::<R>::NotFound),
                        };
                    let tuple_bytes = &slot_data[VersionHeader::SIZE..];

                    let create_tx = version_header.create_tx_id();
                    let commit_tx = version_header.commit_tx_id();
                    let visible = snapshot.is_visible(create_tx, commit_tx)
                        || snapshot.is_visible_self(create_tx, commit_tx);

                    if visible {
                        // MS27 T4: overflow slots exit the page lock with
                        // their locator; the closure is consumed after the
                        // payload is assembled (below).
                        if overflow::is_overflow_ref(tuple_bytes) {
                            let (total_len, first_page) = overflow::decode_locator(tuple_bytes)
                                .ok_or_else(|| {
                                    StorageError::Io(std::io::Error::new(
                                        std::io::ErrorKind::InvalidData,
                                        "malformed overflow locator",
                                    ))
                                })?;
                            return Ok(VisibilityResult::VisibleOverflow {
                                total_len,
                                first_page,
                            });
                        }
                        // f_opt.take() consumes f exactly once across all iterations.
                        // f returns Result<R> so deserialize errors propagate via `?`.
                        let f = f_opt.take().expect("closure should be available");
                        let result = f(tuple_bytes)?;
                        Ok(VisibilityResult::Visible(result))
                    } else {
                        Ok(VisibilityResult::NotVisible(version_header.next_version()))
                    }
                })
                .await?;

            match visibility_result {
                VisibilityResult::Visible(result) => return Ok(Some(result)),
                VisibilityResult::VisibleOverflow {
                    total_len,
                    first_page,
                } => {
                    let bytes = overflow::read_overflow_chain(self, first_page, total_len).await?;
                    let f = f_opt.take().expect("closure should be available");
                    return Ok(Some(f(&bytes)?));
                }
                VisibilityResult::NotVisible(next) => current_row_id = next,
                VisibilityResult::NotFound => return Ok(None),
            }
        }

        // All versions invisible
        Ok(None)
    }

    pub async fn free_page(&self, page_id: PageId) -> Result<()> {
        // M31: DashMap atomic remove (per-shard lock, not global).
        self.pages.remove(&page_id);
        let mut hand = self.clock_hand.write().await;
        hand.retain(|id| *id != page_id);
        self.storage.free_page(page_id).await
    }

    /// MS35（`free-page-lifecycle` R3）：释放操作的完成标记——本次释放的
    /// 锚点写已经发出。释放点必须在锚点写发出**之后**调用它：释放的页在
    /// 此之前不得进入可分配集合。
    pub fn complete_free_release(&self, pages: &[PageId], anchors: &[PageId]) {
        self.storage.complete_free_release(pages, anchors);
    }

    /// Get the current visibility info for a page. Returns None if no entry exists
    /// (meaning: no optimization hint available — fall back to per-row checks).
    pub fn get_visibility(&self, page_id: PageId) -> Option<PageVisibilityInfo> {
        self.vis_map.get(&page_id).map(|r| *r)
    }

    /// Mark a page as all-visible (all rows committed).
    /// Called lazily by scan paths when they discover every row is committed.
    pub fn set_all_visible(&self, page_id: PageId) {
        self.vis_map
            .entry(page_id)
            .and_modify(|info| info.all_visible = true)
            .or_insert(PageVisibilityInfo {
                all_visible: true,
                min_create_tx_id: u64::MAX,
            });
    }

    /// Clear the all_visible flag for a page.
    /// Called by INSERT/DELETE/UPDATE/COMMIT paths.
    ///
    /// The first-build records the [`MIN_CREATE_UNKNOWN`]
    /// sentinel instead of `Default`'s `0`: the write path always follows with
    /// `update_visibility_on_insert`, whose `min()` merge must see UNKNOWN —
    /// not 0 — or the real writer id would be pinned to 0 forever and the
    /// all-invisible fast path could never fire (MS17-T02 close-out).
    pub fn clear_all_visible(&self, page_id: PageId) {
        self.vis_map
            .entry(page_id)
            .and_modify(|info| info.all_visible = false)
            .or_insert(PageVisibilityInfo {
                min_create_tx_id: MIN_CREATE_UNKNOWN,
                all_visible: false,
            });
    }

    /// Update visibility info after a new row is inserted on this page.
    /// Update min_create_tx_id and clear all_visible (new row is uncommitted).
    pub fn update_visibility_on_insert(&self, page_id: PageId, create_tx_id: u64) {
        self.vis_map
            .entry(page_id)
            .and_modify(|info| {
                info.min_create_tx_id = info.min_create_tx_id.min(create_tx_id);
                info.all_visible = false;
            })
            .or_insert(PageVisibilityInfo {
                min_create_tx_id: create_tx_id,
                all_visible: false,
            });
    }

    /// Check if all rows on a page are visible to the given snapshot.
    ///
    /// Returns true if every slot satisfies three conditions:
    /// 1. Committed (commit_tx_id is Some and not a delete sentinel)
    /// 2. Created not after the snapshot's visibility high-water mark
    ///    (create_tx_id <= snapshot.high_water())
    /// 3. Not from an active transaction at snapshot time
    ///
    /// Used by DataScan to lazily set `all_visible` after scanning a page.
    pub async fn check_page_all_visible(&self, page_id: PageId, snapshot: &Snapshot) -> bool {
        self.with_page_data(page_id, |data| -> Result<bool> {
            let slotted = SlottedPageRef::new(data);
            // MS37 T17: 页头槽数越界 ⇒ 降级为安全值（非「全可见」）⇒ 走慢
            // 路径，错误由随后的实际取行处抛出（布尔辅助自身不报页损坏）。
            if slotted.slot_count_exceeds_capacity() {
                return Ok(false);
            }
            for i in 0..slotted.slot_count() {
                let slot = match slotted.get_slot(i) {
                    Some(s) => s,
                    None => continue,
                };
                // MS37 T3: 越界槽 ⇒ 安全值（非「全可见」）⇒ 走慢路径，
                // 错误由随后的实际取行处抛出（布尔辅助自身不报页损坏）。
                let slot_data = match slotted.get_slot_data_checked(&slot) {
                    Some(d) => d,
                    None => return Ok(false),
                };
                if slot_data.len() < VersionHeader::SIZE {
                    return Ok(false);
                }
                let vh = match VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE]) {
                    Some(v) => v,
                    None => return Ok(false),
                };
                // Condition 1: must be committed (not uncommitted, not deleted)
                if vh.commit_tx_id().is_none() || vh.is_deleted() {
                    return Ok(false);
                }
                // Condition 2: created not after the snapshot's visibility
                // high-water mark (mirrors `is_visible` rule 2; under
                // `Snapshot::new` the mark equals the reader's own id and
                // self-created rows fail condition 1 first, so RR behavior
                // is unchanged)
                if vh.create_tx_id() > snapshot.high_water() {
                    return Ok(false);
                }
                // Condition 3: not from an active transaction
                if snapshot.contains_active_tx(vh.create_tx_id()) {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .await
        .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::FileStorage;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tempfile::tempdir;

    /// 写盘进入后停住，放行时返回 `Io` 且不改已持久化的字节。
    /// 只存在于本模块的测试里，用来卡住「清脏之后、写盘返回之前」。
    struct GatedWriteStorage {
        persisted: Mutex<HashMap<PageId, Page>>,
        entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    }

    #[async_trait]
    impl AsyncStorage for GatedWriteStorage {
        async fn read_page(&self, page_id: PageId) -> Result<Page> {
            Ok(self
                .persisted
                .lock()
                .unwrap()
                .get(&page_id)
                .cloned()
                .unwrap_or_else(|| Page::new(page_id)))
        }

        async fn write_page(&self, _page_id: PageId, _page: &Page) -> Result<()> {
            if let Some(tx) = self.entered.lock().unwrap().take() {
                let _ = tx.send(());
            }
            let release = self.release.lock().unwrap().take();
            if let Some(rx) = release {
                let _ = rx.await;
            }
            Err(StorageError::Io(std::io::Error::other(
                "injected write failure",
            )))
        }

        async fn allocate_page(&self) -> Result<PageId> {
            Ok(PageId(0))
        }

        async fn free_page(&self, _page_id: PageId) -> Result<()> {
            Ok(())
        }

        async fn sync(&self) -> Result<()> {
            Ok(())
        }

        fn page_count(&self) -> u64 {
            self.persisted.lock().unwrap().len() as u64
        }
    }

    fn setup() -> (BufferPool, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("vis-test.db")).unwrap());
        let pool = BufferPool::new(10, storage).unwrap();
        (pool, dir)
    }

    /// MS17-T02 T5 (MAX poison): `set_all_visible` on an entry-less page
    /// first-builds `{u64::MAX, true}`; a later write clears `all_visible`
    /// without touching `min_create_tx_id`. The stale `{UNKNOWN, false}` entry
    /// must fall back to per-row checks, not report the whole page invisible.
    #[test]
    fn cleared_page_after_set_all_visible_never_reports_all_invisible() {
        let (pool, _dir) = setup();
        let page = PageId(0);
        pool.set_all_visible(page);
        pool.clear_all_visible(page);
        let info = pool.get_visibility(page).expect("summary entry exists");
        assert!(
            !info.all_invisible_for(0),
            "stale UNKNOWN min_create must fall back to per-row checks"
        );
    }

    /// MS17-T02 T5 (0 poison): the write path builds the summary entry via
    /// `clear_all_visible` before `update_visibility_on_insert` — the
    /// first-build must use the UNKNOWN sentinel so the subsequent min() merge
    /// records the real writer id instead of pinning 0 forever.
    #[test]
    fn insert_after_first_build_records_real_min_create() {
        let (pool, _dir) = setup();
        let page = PageId(0);
        pool.clear_all_visible(page);
        pool.update_visibility_on_insert(page, 7);
        let info = pool.get_visibility(page).expect("summary entry exists");
        assert_eq!(info.min_create_tx_id, 7, "real writer id must be recorded");
        assert!(!info.all_visible);
        // Merged real value keeps strict-greater semantics (S1 scenario THEN).
        assert!(info.all_invisible_for(6));
        assert!(!info.all_invisible_for(7));
    }

    /// 刷盘已取走页内容、写盘尚未返回时，再取另一页迫使驱逐。
    /// 写盘失败后原页仍在池内，字节仍是这次要写的新内容，错误上抛。
    #[tokio::test]
    async fn failed_flush_then_evict_keeps_the_new_bytes() {
        const OLD: u8 = 0x11;
        const NEW: u8 = 0xAB;
        let resident = PageId(1);
        let intruder = PageId(2);

        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let mut old = Page::new(resident);
        old.data.fill(OLD);
        let storage = Arc::new(GatedWriteStorage {
            persisted: Mutex::new(HashMap::from([(resident, old)])),
            entered: Mutex::new(Some(entered_tx)),
            release: Mutex::new(Some(release_rx)),
        });
        let pool = Arc::new(BufferPool::new(1, storage.clone() as Arc<dyn AsyncStorage>).unwrap());

        {
            let guard = pool.get_page(resident).await.unwrap();
            guard.modify_page(|page| page.data.fill(NEW));
        }

        let flushing = {
            let pool = Arc::clone(&pool);
            tokio::spawn(async move { pool.flush_all().await })
        };
        entered_rx.await.expect("write_page entered");

        // 迫使驱逐。变更前这一步会把已清脏的页移出池；变更后在途页留在池内，
        // 这一步可能因池满失败。断言不看这一步的结果。
        let _ = pool.get_page(intruder).await;

        let _ = release_tx.send(());
        let flush_result = flushing.await.unwrap();
        assert!(
            flush_result.is_err(),
            "flush_all must surface the write error"
        );

        let guard = pool.get_page(resident).await.unwrap();
        assert!(
            guard.page_data().iter().all(|byte| *byte == NEW),
            "after the failed write the pool must still hold the new bytes"
        );
        let persisted = storage.persisted.lock().unwrap();
        let on_storage = persisted.get(&resident).expect("seeded page");
        assert!(
            on_storage.data.iter().all(|byte| *byte == OLD),
            "the injected write must not update storage"
        );
    }
}
