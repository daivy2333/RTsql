use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use dashmap::DashMap;

use crate::storage::{
    page_format::SlottedPageRef,
    page_frame::{PageFrame, PageGuard},
    AsyncStorage, Page, PageId, PageVisibilityInfo, Result, RowId, StorageError,
};
use crate::transaction::{Snapshot, VersionHeader};

pub struct BufferPool {
    /// Page cache: PageId → Arc<Mutex<PageFrame>>
    /// SAFETY: Each PageFrame uses std::sync::Mutex. This is safe because:
    /// 1. PageGuard/PageDataGuard are never held across .await points
    /// 2. All Mutex lock/unlock operations are in sync code (no .await between lock and unlock)
    /// 3. The RwLock on the HashMap itself is tokio::sync::RwLock (async-safe)
    pages: RwLock<HashMap<PageId, Arc<std::sync::Mutex<PageFrame>>>>,
    /// Per-page MVCC visibility map for fast-path skipping.
    /// DashMap provides lock-free concurrent access — no deadlock risk
    /// with existing page cache locks.
    vis_map: DashMap<PageId, PageVisibilityInfo>,
    clock_hand: RwLock<Vec<PageId>>,
    capacity: usize,
    storage: Arc<dyn AsyncStorage>,
}

impl BufferPool {
    pub fn new(capacity: usize, storage: Arc<dyn AsyncStorage>) -> Result<Self> {
        if capacity == 0 {
            return Err(StorageError::InvalidCapacity(capacity));
        }

        Ok(Self {
            pages: RwLock::new(HashMap::new()),
            vis_map: DashMap::new(),
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
        // 1. 读锁检查缓存
        {
            let pages = self.pages.read().await;
            if let Some(frame) = pages.get(&page_id) {
                return Ok(PageGuard::new(frame.clone()));
            }
        }

        // 2. 写锁加载页
        let mut pages = self.pages.write().await;

        // Double check
        if let Some(frame) = pages.get(&page_id) {
            return Ok(PageGuard::new(frame.clone()));
        }

        // 3. 缓存满则淘汰
        if pages.len() >= self.capacity {
            self.evict_one(&mut pages).await?;
        }

        // 4. 从存储加载页
        let page = self.storage.read_page(page_id).await?;
        let frame = Arc::new(std::sync::Mutex::new(PageFrame::new(page)));

        pages.insert(page_id, frame.clone());
        self.clock_hand.write().await.push(page_id);

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
    /// 3 failed attempts that led to this design.
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

    async fn evict_one(
        &self,
        pages: &mut HashMap<PageId, Arc<std::sync::Mutex<PageFrame>>>,
    ) -> Result<()> {
        let mut clock_hand = self.clock_hand.write().await;
        let mut attempts = 0;
        let max_attempts = clock_hand.len() * 2;

        while attempts < max_attempts {
            if clock_hand.is_empty() {
                return Err(StorageError::BufferPoolFull);
            }

            let candidate_id = clock_hand.remove(0);
            attempts += 1;

            let frame = match pages.get(&candidate_id) {
                Some(f) => f.clone(),
                None => continue,
            };

            let (dirty, page_copy): (bool, Option<Page>) = {
                let mut frame_guard = frame.lock().unwrap();

                if frame_guard.ref_count > 0 {
                    clock_hand.push(candidate_id);
                    (false, None)
                } else if frame_guard.clock_bit {
                    frame_guard.clock_bit = false;
                    clock_hand.push(candidate_id);
                    (false, None)
                } else {
                    let is_dirty = frame_guard.dirty;
                    let page = frame_guard.page.clone();
                    (is_dirty, Some(page))
                }
            };

            let Some(page_copy) = page_copy else {
                continue;
            };

            if dirty {
                self.storage.write_page(candidate_id, &page_copy).await?;
            }

            pages.remove(&candidate_id);
            return Ok(());
        }

        Err(StorageError::BufferPoolFull)
    }

    /// Flush all dirty pages to storage
    ///
    /// Note: MutexGuard held during clone, then explicitly dropped before .await
    /// - frame_guard is held only during synchronous clone + dirty flag reset
    /// - Explicit drop(frame_guard) releases lock before async I/O
    /// - No lock contention during storage.write_page().await
    #[allow(clippy::await_holding_lock)]
    pub async fn flush_all(&self) -> Result<()> {
        let pages = self.pages.read().await;

        for (page_id, frame) in pages.iter() {
            let mut frame_guard = frame.lock().unwrap();

            if frame_guard.dirty {
                let page = frame_guard.page.clone();
                frame_guard.dirty = false;
                drop(frame_guard);
                self.storage.write_page(*page_id, &page).await?;
            }
        }

        Ok(())
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
                return self
                    .with_page_data(PageId(row_id.page_id as u64), |data| {
                        let slotted = SlottedPageRef::new(data);
                        let (slot, _) = match slotted.get_slot_by_logical_id(row_id.slot_id) {
                            Some(s) => s,
                            None => return Ok(None),
                        };
                        let slot_data = slotted.get_slot_data(&slot);
                        if slot_data.len() < VersionHeader::SIZE {
                            return Ok(None);
                        }
                        let tuple_bytes = &slot_data[VersionHeader::SIZE..];
                        let f = f_opt.take().expect("closure should be available");
                        let result = f(tuple_bytes)?;
                        Ok(Some(result))
                    })
                    .await;
            }

            if vis_info.all_invisible_for(snapshot.tx_id()) {
                return Ok(None);
            }
        }

        while let Some(current) = current_row_id {
            let visibility_result: VisibilityResult<R> = self
                .with_page_data(PageId(current.page_id as u64), |data| {
                    let slotted = SlottedPageRef::new(data);
                    let (slot, _) = match slotted.get_slot_by_logical_id(current.slot_id) {
                        Some(s) => s,
                        None => return Ok(VisibilityResult::<R>::NotFound),
                    };
                    let slot_data = slotted.get_slot_data(&slot);
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
                VisibilityResult::NotVisible(next) => current_row_id = next,
                VisibilityResult::NotFound => return Ok(None),
            }
        }

        // All versions invisible
        Ok(None)
    }

    /// Mark all tuples created by an aborted transaction
    /// Sets commit_tx_id = u64::MAX so MVCC visibility skips them
    ///
    /// TODO: Implement proper slot iteration using SlottedPage API.
    /// For now, aborted tuples remain invisible to other transactions
    /// because MVCC visibility checks will see create_tx_id in the
    /// active_tx_ids set (which is preserved across restarts via WAL).
    pub async fn mark_tx_aborted(&self, _aborted_tx_id: u64) -> Result<()> {
        Ok(())
    }

    pub async fn free_page(&self, page_id: PageId) -> Result<()> {
        self.pages.write().await.remove(&page_id);
        let mut hand = self.clock_hand.write().await;
        hand.retain(|id| *id != page_id);
        self.storage.free_page(page_id).await
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
    pub fn clear_all_visible(&self, page_id: PageId) {
        self.vis_map
            .entry(page_id)
            .and_modify(|info| info.all_visible = false)
            .or_default();
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
    /// 2. Created before the snapshot (create_tx_id < snapshot.tx_id)
    /// 3. Not from an active transaction at snapshot time
    ///
    /// Used by DataScan to lazily set `all_visible` after scanning a page.
    pub async fn check_page_all_visible(&self, page_id: PageId, snapshot: &Snapshot) -> bool {
        self.with_page_data(page_id, |data| -> Result<bool> {
            let slotted = SlottedPageRef::new(data);
            for i in 0..slotted.slot_count() {
                let slot = match slotted.get_slot(i) {
                    Some(s) => s,
                    None => continue,
                };
                let slot_data = slotted.get_slot_data(&slot);
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
                // Condition 2: created before snapshot
                if vh.create_tx_id() >= snapshot.tx_id() {
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
