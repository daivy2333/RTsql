//! Index scan executor - primary key lookup

use crate::executor::{ExecResult, Executor};
use crate::storage::{btree::IndexManager, Result};
use std::sync::Arc;

/// IndexScanExecutor - 主键索引扫描执行器
pub struct IndexScanExecutor {
    index_manager: Arc<IndexManager>,
    key: Vec<u8>,
    executed: bool,
}

impl IndexScanExecutor {
    pub fn new(index_manager: Arc<IndexManager>, key: Vec<u8>) -> Self {
        Self {
            index_manager,
            key,
            executed: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for IndexScanExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        let row_id = self.index_manager.search(&self.key).await?;

        match row_id {
            Some(id) => Ok(Some(ExecResult::RowId(id))),
            None => Ok(None),
        }
    }
}
