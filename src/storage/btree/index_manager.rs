// IndexManager 异步 API（Task 6 实现）
use std::sync::Arc;

use crate::storage::{page_format::RowId, BufferPool, Result};

pub struct IndexManager {
    _buffer_pool: Arc<BufferPool>,
}

impl IndexManager {
    pub fn new(buffer_pool: Arc<BufferPool>) -> Result<Self> {
        Ok(Self {
            _buffer_pool: buffer_pool,
        })
    }

    pub async fn insert(&self, _key: &[u8], _row_id: RowId) -> Result<()> {
        // Task 6 实现
        Ok(())
    }

    pub async fn search(&self, _key: &[u8]) -> Result<Option<RowId>> {
        // Task 6 实现
        Ok(None)
    }
}