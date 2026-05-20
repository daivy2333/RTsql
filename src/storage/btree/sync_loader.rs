// SyncPageLoader（Task 4 实现）
use std::sync::Arc;

use crate::storage::{BufferPool, PageGuard, PageId, Result, StorageError};

/// SyncPageLoader：在同步代码中加载页（使用 block_on 包装 BufferPool）
pub struct SyncPageLoader {
    buffer_pool: Arc<BufferPool>,
}

impl SyncPageLoader {
    pub fn new(buffer_pool: Arc<BufferPool>) -> Self {
        Self {
            buffer_pool,
        }
    }

    pub fn load_page(&self, _page_id: PageId) -> Result<PageGuard> {
        // Task 4 实现：使用 tokio::runtime::Handle::current().block_on()
        Err(StorageError::Io(std::io::Error::new(std::io::ErrorKind::Other, "SyncPageLoader not implemented yet")))
    }

    pub fn allocate_page(&self) -> Result<PageId> {
        // Task 4 实现：使用 tokio::runtime::Handle::current().block_on()
        Err(StorageError::Io(std::io::Error::new(std::io::ErrorKind::Other, "SyncPageLoader not implemented yet")))
    }
}