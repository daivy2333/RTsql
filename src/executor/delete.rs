//! Delete executor - delete by key

use crate::executor::{ExecResult, Executor};
use crate::storage::{btree::IndexManager, Result};
use std::sync::Arc;

pub struct DeleteExecutor {
    index_manager: Arc<IndexManager>,
    key: Vec<u8>,
    /// Transaction ID for MVCC visibility checks (future use)
    #[allow(dead_code)]
    tx_id: u64,
    executed: bool,
}

impl DeleteExecutor {
    pub fn new(index_manager: Arc<IndexManager>, key: Vec<u8>, tx_id: u64) -> Self {
        Self {
            index_manager,
            key,
            tx_id,
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
