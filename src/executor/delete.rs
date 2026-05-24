//! Delete executor - delete by key

use crate::executor::{ExecResult, Executor};
use crate::storage::{btree::IndexManager, Result};
use crate::wal::{WALBuffer, WalRecord};
use std::sync::Arc;

pub struct DeleteExecutor {
    index_manager: Arc<IndexManager>,
    table_name: String,
    key: Vec<u8>,
    tx_id: u64,
    executed: bool,
    wal_buffer: Option<Arc<WALBuffer>>,
}

impl DeleteExecutor {
    pub fn new(
        index_manager: Arc<IndexManager>,
        table_name: String,
        key: Vec<u8>,
        tx_id: u64,
        wal_buffer: Option<Arc<WALBuffer>>,
    ) -> Self {
        Self {
            index_manager,
            table_name,
            key,
            tx_id,
            executed: false,
            wal_buffer,
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

        // WAL: BeginTxn (implicit transaction per statement)
        if let Some(wal) = &self.wal_buffer {
            wal.append(WalRecord::BeginTxn { tx_id: self.tx_id })
                .await;
        }

        // Search for row_id before deleting
        let row_id = self.index_manager.search(&self.key).await?;

        self.index_manager.delete(&self.key).await?;

        // WAL: Delete + CommitTxn
        if let (Some(wal), Some(row_id)) = (&self.wal_buffer, row_id) {
            wal.append(WalRecord::Delete {
                tx_id: self.tx_id,
                table_name: self.table_name.clone(),
                row_id,
            })
            .await;
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            wal.append(WalRecord::CommitTxn { tx_id: self.tx_id, timestamp })
                .await;
            let _ = wal.append_commit_and_wait(self.tx_id).await;
        }

        Ok(Some(ExecResult::AffectedRows(1)))
    }
}
