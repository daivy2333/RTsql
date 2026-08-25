//! Update executor - MVCC-aware row update

use crate::executor::{ExecResult, Executor, Value};
use crate::storage::page_format::{
    compute_tuple_size, deserialize_tuple, serialize_tuple, ColumnType,
};
use crate::storage::{
    read_tuple_from_data_page, write_tuple_to_data_page, BufferPool, PageId, Result, StorageError,
    TableMeta,
};
use crate::transaction::{TransactionManager, VersionHeader};
use crate::wal::{WALBuffer, WalRecord};
use std::sync::Arc;

pub struct UpdateExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    tx_manager: Arc<TransactionManager>,
    key: Vec<u8>,
    column_name: String,
    new_value: Value,
    tx_id: u64,
    schema: Vec<ColumnType>,
    executed: bool,
    wal_buffer: Option<Arc<WALBuffer>>,
}

impl UpdateExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        table_meta: Arc<TableMeta>,
        buffer_pool: Arc<BufferPool>,
        tx_manager: Arc<TransactionManager>,
        key: Vec<u8>,
        column_name: String,
        new_value: Value,
        tx_id: u64,
        wal_buffer: Option<Arc<WALBuffer>>,
    ) -> Self {
        let schema: Vec<ColumnType> = table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect();
        Self {
            table_meta,
            buffer_pool,
            tx_manager,
            key,
            column_name,
            new_value,
            tx_id,
            schema,
            executed: false,
            wal_buffer,
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

        // Step 1: Search index for key → get old RowId
        let old_row_id = match self.table_meta.index_manager.search(&self.key).await? {
            Some(id) => id,
            None => return Err(StorageError::KeyNotFound),
        };

        // Step 2: Read old tuple from data page (M20 closure form, .to_vec() for WAL ownership)
        let (_version_header, old_tuple_bytes) =
            read_tuple_from_data_page(&self.buffer_pool, old_row_id, |vh, bytes| {
                Ok((vh, bytes.to_vec()))
            })
            .await?;
        let mut values = deserialize_tuple(&old_tuple_bytes, &self.schema)?;

        // Step 3: Find column index and modify the target column
        let col_idx = self
            .table_meta
            .columns
            .iter()
            .position(|(name, _)| name == &self.column_name)
            .ok_or_else(|| StorageError::ColumnNotFound(self.column_name.clone()))?;
        values[col_idx] = self.new_value.clone();

        // Step 4: Serialize new tuple
        let size = compute_tuple_size(&values, &self.schema);
        let mut buf = vec![0u8; size];
        serialize_tuple(&values, &self.schema, &mut buf)?;

        // Step 5: Create new VersionHeader with next_version → old RowId
        let version_header = VersionHeader::new(self.tx_id, None).with_next_version(old_row_id);

        // Step 6: Write new tuple to data page
        let new_row_id =
            write_tuple_to_data_page(&self.buffer_pool, &self.table_meta, &version_header, &buf)
                .await?;

        // M21: Clear page visibility summary after UPDATE (new version page + old version page)
        let new_page_id = PageId(new_row_id.page_id as u64);
        self.buffer_pool.clear_all_visible(new_page_id);
        let old_page_id = PageId(old_row_id.page_id as u64);
        self.buffer_pool.clear_all_visible(old_page_id);

        // WAL: Update record only. BeginTxn/CommitTxn are written by
        // TransactionManager::begin()/commit() (the single source of truth).
        if let Some(wal) = &self.wal_buffer {
            wal.append(WalRecord::Update {
                tx_id: self.tx_id,
                table_name: self.table_meta.name.clone(),
                row_id: new_row_id,
                old_tuple: old_tuple_bytes.clone(),
                new_tuple: buf.clone(),
            })
            .await;
        }

        // Step 6.1: Record version in tx_versions (M10)
        self.tx_manager.record_version(self.tx_id, new_row_id).await;

        // Step 7: Update index → new RowId
        self.table_meta
            .index_manager
            .update(&self.key, new_row_id)
            .await?;

        Ok(Some(ExecResult::AffectedRows(1)))
    }
}
