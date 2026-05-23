use crate::storage::buffer_pool::BufferPool;
use crate::storage::page_frame::PageGuard;
use crate::storage::page_id::PageId;
use crate::storage::Result;
use std::sync::Arc;

/// AsyncPageLoader: loads pages directly in async context (no block_on)
/// Used by BTree async read path to eliminate spawn_blocking + block_on overhead
pub struct AsyncPageLoader {
    buffer_pool: Arc<BufferPool>,
}

impl AsyncPageLoader {
    pub fn new(buffer_pool: Arc<BufferPool>) -> Self {
        Self { buffer_pool }
    }

    pub async fn load_page(&self, page_id: PageId) -> Result<PageGuard> {
        self.buffer_pool.get_page(page_id).await
    }
}
