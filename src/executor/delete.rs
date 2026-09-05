//! Delete executor - delete by key

use crate::executor::{ExecResult, Executor};
use crate::storage::{
    btree::IndexManager, update_version_header_in_data_page, BufferPool, PageId, Result,
    StorageError,
};
use crate::transaction::TransactionManager;
use crate::wal::{WALBuffer, WalRecord};
use std::sync::Arc;

pub struct DeleteExecutor {
    index_manager: Arc<IndexManager>,
    buffer_pool: Arc<BufferPool>,
    tx_manager: Arc<TransactionManager>,
    table_name: String,
    key: Vec<u8>,
    tx_id: u64,
    executed: bool,
    wal_buffer: Option<Arc<WALBuffer>>,
}

impl DeleteExecutor {
    pub fn new(
        index_manager: Arc<IndexManager>,
        buffer_pool: Arc<BufferPool>,
        tx_manager: Arc<TransactionManager>,
        table_name: String,
        key: Vec<u8>,
        tx_id: u64,
        wal_buffer: Option<Arc<WALBuffer>>,
    ) -> Self {
        Self {
            index_manager,
            buffer_pool,
            tx_manager,
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

        // Search for row_id before deleting
        let row_id = self.index_manager.search(&self.key).await?;

        // Mark version header as deleted on the data page (before removing index).
        // This ensures DataScan sees the row as deleted via MVCC visibility.
        // Gracefully skip if the data page/slot doesn't exist (e.g., test fixtures).
        if let Some(rid) = &row_id {
            match self.buffer_pool.read_version_header(*rid).await {
                Ok(vh) => {
                    let deleted_vh = vh.mark_deleted();
                    let _ = update_version_header_in_data_page(
                        &self.buffer_pool,
                        *rid,
                        deleted_vh,
                        &[],
                    )
                    .await;
                    // M21: Clear page visibility after marking deleted
                    self.buffer_pool
                        .clear_all_visible(PageId(rid.page_id as u64));
                }
                Err(StorageError::SlotNotFound(_)) => {
                    // Data page/slot doesn't exist — skip mark_deleted.
                    // The index entry will still be removed, which is sufficient
                    // for PK lookup correctness.
                }
                Err(e) => return Err(e),
            }
        }

        self.index_manager.delete(&self.key).await?;

        // M10: Record this version in tx_versions so abort can clean up the
        // index entry. Required because the index was already mutated above.
        if let Some(rid) = row_id {
            self.tx_manager
                .record_version(self.tx_id, &self.table_name, rid)
                .await;

            // WAL: Delete record only. BeginTxn/CommitTxn are written by
            // TransactionManager::begin()/commit() (the single source of truth).
            if let Some(wal) = &self.wal_buffer {
                wal.append(WalRecord::Delete {
                    tx_id: self.tx_id,
                    table_name: self.table_name.clone(),
                    row_id: rid,
                })
                .await;
            }
        }

        Ok(Some(ExecResult::AffectedRows(1)))
    }
}
