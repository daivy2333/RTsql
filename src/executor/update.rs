//! Update executor - updates RowId in index

use crate::executor::{ExecResult, Executor, Value};
use crate::storage::{btree::IndexManager, page_format::RowId, Result};
use std::sync::Arc;

/// UpdateExecutor - updates the RowId for an existing key
pub struct UpdateExecutor {
    index_manager: Arc<IndexManager>,
    key: Vec<u8>,
    // M5: new_value will be used in future to compute new RowId
    #[allow(dead_code)]
    new_value: Value,
    executed: bool,
}

impl UpdateExecutor {
    pub fn new(index_manager: Arc<IndexManager>, key: Vec<u8>, new_value: Value) -> Self {
        Self {
            index_manager,
            key,
            new_value,
            executed: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for UpdateExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        // M5: 使用测试占位 RowId（page_id=0, slot_id=999）
        let new_row_id = RowId::new(0, 999);
        self.index_manager.update(&self.key, new_row_id).await?;

        Ok(Some(ExecResult::AffectedRows(1)))
    }
}
