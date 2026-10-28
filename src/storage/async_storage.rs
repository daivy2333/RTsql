use crate::storage::{Page, PageId, Result};
use async_trait::async_trait;

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait AsyncStorage: Send + Sync {
    async fn read_page(&self, page_id: PageId) -> Result<Page>;
    async fn write_page(&self, page_id: PageId, page: &Page) -> Result<()>;
    async fn allocate_page(&self) -> Result<PageId>;
    async fn free_page(&self, page_id: PageId) -> Result<()>;
    async fn sync(&self) -> Result<()>;

    fn page_size(&self) -> usize {
        Page::PAGE_SIZE
    }

    /// Total number of pages currently allocated on this storage. Returns
    /// 0 for a freshly-created file containing only the format header
    /// (MS10-T03). MS07-T01: used by
    /// `TableManager::new` to decide between catalog bootstrap (header-only
    /// file → allocate page 0,1) and catalog open (non-empty → bind to
    /// existing page 0,1).
    fn page_count(&self) -> u64;

    /// MS35 (`free-page-lifecycle` R2): hand the rebuilt allocatable page
    /// set to this storage.
    ///
    /// Default implementation: no effect — storage shapes without a
    /// free-page set (`:memory:`, test doubles) ignore the hand-off, so the
    /// required-method set of `AsyncStorage` stays unchanged.
    ///
    /// Semantics: the handed-over set **replaces** the storage's current
    /// allocatable set (rebuild semantics — `free-page-lifecycle` R2
    /// "rebuild this session's allocatable page set"). The session's
    /// allocatable set is derived from the persisted directory alone; it is
    /// not stacked on top of the releases made while opening (a recovery
    /// that re-rooted an index tree releases the old tree's pages, but the
    /// persisted directory still registers them → they must not become
    /// allocatable).
    fn inject_free_pages(&self, _pages: Vec<u64>) {}

    /// MS35 (`free-page-lifecycle` R3): sample the release completion number
    /// — the count of releases whose anchor write has already been issued.
    ///
    /// The checkpoint samples this number **before** it flushes the dirty
    /// pages; a release whose completion number is below the sample is
    /// covered by that flush (its anchor write was issued before the sample,
    /// hence before the flush). Default implementation: `0` — storage shapes
    /// without a release lifecycle (`:memory:`, test doubles) have no pending
    /// release set, so the sample covers nothing.
    fn release_completion_limit(&self) -> u64 {
        0
    }

    /// MS35 (`free-page-lifecycle` R3): a release operation completed — its
    /// anchor write (the write that stops referencing the released pages) has
    /// been issued.
    ///
    /// `pages` are the pages this release operation released. Only those
    /// pages become "completed": a release that has not completed yet is
    /// never promoted, no matter what else completes meanwhile.
    ///
    /// `anchors` are the pages of the anchor write that stopped referencing
    /// `pages`. An empty list means this release has no anchor write, and
    /// promotion does not consult the per-page write-failure set.
    ///
    /// Default implementation: no effect — storage shapes without a release
    /// lifecycle keep their existing "released pages are allocatable right
    /// away" behavior (`:memory:` has no crash boundary).
    fn complete_free_release(&self, _pages: &[PageId], _anchors: &[PageId]) {}

    /// MS35 (`free-page-lifecycle` R3): the flush that covered the releases
    /// below `limit` succeeded — those releases are established on the crash
    /// boundary and their pages become allocatable.
    ///
    /// Default implementation: no effect — nothing is pending.
    fn promote_free_releases(&self, _limit: u64) {}

    /// MS35 (`free-page-lifecycle` R3): the flush that covered the releases
    /// below `limit` failed — those releases are dropped (not promoted). Their
    /// pages stay owned by their old owner on disk; the next open rebuilds
    /// them from the persisted directory.
    ///
    /// Default implementation: no effect — nothing is pending.
    fn discard_free_releases(&self, _limit: u64) {}
}
