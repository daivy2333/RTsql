//! Delete executor - delete by key

use crate::executor::{ExecResult, Executor};
use crate::storage::btree::IndexManager;
use crate::storage::page_format::{deserialize_tuple, ColumnType};
use crate::storage::{
    read_tuple_from_data_page, write_tuple_to_data_page, BufferPool, PageId, Result, RowId,
    StorageError, TableMeta,
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

    /// MS23 Iteration 001 (2.7/D7): extract the deleted row's UNIQUE column
    /// keys from the row's data-page slot, before the tombstone write.
    /// Best-effort like the PK path: an unreadable slot (SlotNotFound
    /// tolerance, test fixtures) yields no keys and the unique deletions are
    /// skipped. NULL values never have entries (no key).
    async fn unique_keys_of_row(&self, rid: RowId) -> Result<Vec<(Arc<IndexManager>, Vec<u8>)>> {
        if self.table_meta.unique_indexes.is_empty() {
            return Ok(Vec::new());
        }
        let tuple = match read_tuple_from_data_page(&self.buffer_pool, rid, |_, bytes| Ok(bytes.to_vec()))
            .await
        {
            Ok(bytes) => bytes,
            Err(StorageError::SlotNotFound(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let schema: Vec<ColumnType> = self
            .table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect();
        let values = deserialize_tuple(&tuple, &schema)?;
        let mut keys = Vec::new();
        for (col_idx, uindex) in &self.table_meta.unique_indexes {
            if let Some(key) = values[*col_idx].to_key() {
                keys.push((uindex.clone(), key.as_bytes().to_vec()));
            }
        }
        Ok(keys)
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

        // MS23 Iteration 001 (2.7/D7): 唯一列键值提取——写墓碑前从 rid 处
        // slot 数据反序列化行元组（SlotNotFound 容忍路径无元组可读，与 PK
        // 同型跳过唯一删除）。
        let unique_keys = match row_id.as_ref() {
            Some(rid) => self.unique_keys_of_row(*rid).await?,
            None => Vec::new(),
        };

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

        // MS23 Iteration 001 (2.7/D7): 删 PK 条目同区逐唯一列移除条目——
        // 不移除则残留条目使后续同值插入假阳性 DuplicateKey。
        for (uindex, key) in &unique_keys {
            uindex.delete(key).await?;
        }

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
