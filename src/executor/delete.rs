//! Delete executor - delete by key

use crate::executor::{ExecResult, Executor};
use crate::storage::{btree::IndexManager, Result};
use std::sync::Arc;

/// DeleteExecutor - delete a key from the index
pub struct DeleteExecutor {
    index_manager: Arc<IndexManager>,
    key: Vec<u8>,
    executed: bool,
}

impl DeleteExecutor {
    pub fn new(index_manager: Arc<IndexManager>, key: Vec<u8>) -> Self {
        Self {
            index_manager,
            key,
            executed: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for DeleteExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;
        self.index_manager.delete(&self.key).await?;
        Ok(Some(ExecResult::AffectedRows(1)))
    }
}
