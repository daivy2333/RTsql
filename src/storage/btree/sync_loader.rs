// SyncPageLoader（Task 4 实现）
use std::sync::Arc;
use tokio::runtime::Handle;

use crate::storage::{BufferPool, PageGuard, PageId, Result};

/// SyncPageLoader：在同步代码中加载页（使用 block_on 包装 BufferPool）
pub struct SyncPageLoader {
    buffer_pool: Arc<BufferPool>,
    runtime: Handle,
}

impl SyncPageLoader {
    /// Create SyncPageLoader (must be called within Tokio runtime context)
    pub fn new(buffer_pool: Arc<BufferPool>) -> Self {
        let runtime = Handle::current();
        Self {
            buffer_pool,
            runtime,
        }
    }

    /// Load page synchronously using block_on
    pub fn load_page(&self, page_id: PageId) -> Result<PageGuard> {
        self.runtime.block_on(self.buffer_pool.get_page(page_id))
    }

    /// Allocate page synchronously using block_on
    pub fn allocate_page(&self) -> Result<PageId> {
        self.runtime
            .block_on(self.buffer_pool.storage().allocate_page())
    }

    /// Free page synchronously using block_on
    pub fn free_page(&self, page_id: PageId) -> Result<()> {
        self.runtime.block_on(self.buffer_pool.free_page(page_id))
    }
}
