//! Insert executor - batch insertion

use crate::executor::{ExecResult, Executor, Value};
use crate::storage::{btree::IndexManager, page_format::RowId, Result};
use std::sync::Arc;

/// InsertExecutor - 批量插入执行器
pub struct InsertExecutor {
    index_manager: Arc<IndexManager>,
    values: Vec<Vec<Value>>,
    executed: bool,
}

impl InsertExecutor {
    pub fn new(index_manager: Arc<IndexManager>, values: Vec<Vec<Value>>) -> Self {
        Self {
            index_manager,
            values,
            executed: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for InsertExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        let mut count = 0u64;
        for (slot_id, row_values) in self.values.iter().enumerate() {
            // 取第一列作为 key（假设主键在第一列）
            if let Some(first_value) = row_values.first() {
                if let Some(key) = first_value.to_key() {
                    // M5: 使用测试占位 RowId（page_id=0, slot_id 递增）
                    let row_id = RowId::new(0, slot_id as u16);
                    self.index_manager.insert(key.as_bytes(), row_id).await?;
                    count += 1;
                }
            }
        }

        Ok(Some(ExecResult::AffectedRows(count)))
    }
}