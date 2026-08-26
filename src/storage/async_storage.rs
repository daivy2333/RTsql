use crate::storage::{Page, PageId, Result};
use async_trait::async_trait;

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
    /// 0 for a freshly-opened empty file. MS07-T01: used by
    /// `TableManager::new` to decide between catalog bootstrap (empty
    /// file → allocate page 0,1) and catalog open (non-empty → bind to
    /// existing page 0,1).
    fn page_count(&self) -> u64;
}
