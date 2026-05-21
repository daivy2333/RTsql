use crate::executor::{ExecResult, Executor, Value};
use crate::storage::page_format::{compute_tuple_size, serialize_tuple, ColumnType};
use crate::storage::{write_tuple_to_data_page, BufferPool, Result, StorageError, TableMeta};
use crate::transaction::{TransactionManager, VersionHeader};
use std::sync::Arc;

pub struct InsertExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    tx_manager: Arc<TransactionManager>,
    values: Vec<Vec<Value>>,
    schema: Vec<ColumnType>,
    pk_index: usize,
    tx_id: u64,
    executed: bool,
}

impl InsertExecutor {
    pub fn new(
        table_meta: Arc<TableMeta>,
        buffer_pool: Arc<BufferPool>,
        tx_manager: Arc<TransactionManager>,
        values: Vec<Vec<Value>>,
        tx_id: u64,
    ) -> Self {
        let schema: Vec<ColumnType> = table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect();
        let pk_index = table_meta.pk_index;
        Self {
            table_meta,
            buffer_pool,
            tx_manager,
            values,
            schema,
            pk_index,
            tx_id,
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
        for row_values in &self.values {
            let pk_value = &row_values[self.pk_index];

            let key = match pk_value.to_key() {
                Some(k) => k,
                None => continue,
            };

            if self
                .table_meta
                .index_manager
                .search(key.as_bytes())
                .await?
                .is_some()
            {
                return Err(StorageError::DuplicateKey);
            }

            let size = compute_tuple_size(row_values, &self.schema);
            let mut buf = vec![0u8; size];
            serialize_tuple(row_values, &self.schema, &mut buf)?;

            let version_header = VersionHeader::new(self.tx_id, None);

            let row_id = write_tuple_to_data_page(
                &self.buffer_pool,
                &self.table_meta,
                &version_header,
                &buf,
            )
            .await?;

            // Record version in tx_versions (M10)
            self.tx_manager.record_version(self.tx_id, row_id).await;

            self.table_meta
                .index_manager
                .insert(key.as_bytes(), row_id)
                .await?;

            count += 1;
        }

        Ok(Some(ExecResult::AffectedRows(count)))
    }
}
