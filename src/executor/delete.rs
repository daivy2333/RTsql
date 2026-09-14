//! Delete executor - delete by key

use crate::executor::{ExecResult, Executor};
use crate::storage::{
    write_tuple_to_data_page, BufferPool, PageId, Result, RowId, StorageError, TableMeta,
};
use crate::transaction::{TransactionManager, VersionHeader};
use crate::wal::{WALBuffer, WalRecord};
use std::sync::Arc;

pub struct DeleteExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    tx_manager: Arc<TransactionManager>,
    key: Vec<u8>,
    tx_id: u64,
    executed: bool,
    wal_buffer: Option<Arc<WALBuffer>>,
}

impl DeleteExecutor {
    pub fn new(
        table_meta: Arc<TableMeta>,
        buffer_pool: Arc<BufferPool>,
        tx_manager: Arc<TransactionManager>,
        key: Vec<u8>,
        tx_id: u64,
        wal_buffer: Option<Arc<WALBuffer>>,
    ) -> Self {
        Self {
            table_meta,
            buffer_pool,
            tx_manager,
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
        let row_id = self.table_meta.index_manager.search(&self.key).await?;

        // MS09 Iter000 (D1, I033): express the delete as an independent
        // tombstone version slot (create_tx = deleter, commit = delete
        // sentinel, chain pointer -> deleted row). The deleted version's own
        // header is no longer overwritten: while the deleter is uncommitted,
        // snapshot-less scans fall through to the pre-delete version, and the
        // tombstone stays self-describing for the deleter-commit-state
        // suppression decision. The header read doubles as the existence
        // probe for the pre-existing SlotNotFound tolerance (test fixtures).
        let mut tombstone_rid: Option<RowId> = None;
        if let Some(rid) = &row_id {
            match self.buffer_pool.read_version_header(*rid).await {
                Ok(_) => {
                    let tombstone = VersionHeader::new(self.tx_id, None)
                        .with_next_version(*rid)
                        .mark_deleted();
                    let written = write_tuple_to_data_page(
                        &self.buffer_pool,
                        &self.table_meta,
                        &tombstone,
                        &[],
                    )
                    .await?;
                    self.buffer_pool
                        .clear_all_visible(PageId(written.page_id as u64));
                    self.buffer_pool
                        .clear_all_visible(PageId(rid.page_id as u64));
                    tombstone_rid = Some(written);
                }
                Err(StorageError::SlotNotFound(_)) => {
                    // Data page/slot doesn't exist — skip the tombstone write.
                    // The index entry will still be removed, which is sufficient
                    // for PK lookup correctness.
                }
                Err(e) => return Err(e),
            }
        }

        self.table_meta.index_manager.delete(&self.key).await?;

        // M10: record the version object for commit/abort bookkeeping — the
        // tombstone slot when one was written, the deleted row's rid
        // otherwise (slot-missing tolerance keeps its pre-change shape).
        // commit() keeps the tombstone sentinel (version_chain commit guard);
        // abort neutralizes the tombstone slot (manager abort cleanup).
        if let Some(rid) = row_id {
            let recorded = tombstone_rid.unwrap_or(rid);
            self.tx_manager
                .record_version(self.tx_id, &self.table_meta.name, recorded)
                .await;

            // WAL: Delete record carrying the deleted row's rid (record
            // format and field semantics unchanged; recovery rebuilds the
            // tombstone slot from it). BeginTxn/CommitTxn are written by
            // TransactionManager::begin()/commit() (the single source of
            // truth).
            if let Some(wal) = &self.wal_buffer {
                wal.append(WalRecord::Delete {
                    tx_id: self.tx_id,
                    table_name: self.table_meta.name.clone(),
                    row_id: rid,
                })
                .await;
            }
        }

        Ok(Some(ExecResult::AffectedRows(1)))
    }
}
