use crate::storage::{Page, PageId, Result};
use async_trait::async_trait;

#[async_trait]
pub trait AsyncStorage: Send + Sync {
    async fn read_page(&self, page_id: PageId) -> Result<Page>;
    async fn write_page(&self, page_id: PageId, page: &Page) -> Result<()>;
    async fn allocate_page(&self) -> Result<PageId>;
    async fn sync(&self) -> Result<()>;

    fn page_size(&self) -> usize {
        Page::PAGE_SIZE
    }
}
