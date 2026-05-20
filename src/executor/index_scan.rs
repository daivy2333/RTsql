//! Index scan executor - primary key lookup

use crate::executor::{ExecResult, Executor};
use crate::storage::page_format::{deserialize_tuple, ColumnType};
use crate::storage::{read_tuple_from_data_page, BufferPool, Result, TableMeta};
use crate::transaction::Snapshot;
use std::sync::Arc;

pub struct IndexScanExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    key: Vec<u8>,
    schema: Vec<ColumnType>,
    snapshot: Option<Snapshot>,
    executed: bool,
}

impl IndexScanExecutor {
    pub fn new(
        table_meta: Arc<TableMeta>,
        buffer_pool: Arc<BufferPool>,
        key: Vec<u8>,
        snapshot: Option<Snapshot>,
    ) -> Self {
        let schema: Vec<ColumnType> = table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect();
        Self {
            table_meta,
            buffer_pool,
            key,
            schema,
            snapshot,
            executed: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for IndexScanExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;

        let row_id = self.table_meta.index_manager.search(&self.key).await?;

        match row_id {
            Some(id) => {
                let (version_header, tuple_bytes) =
                    read_tuple_from_data_page(&self.buffer_pool, id).await?;

                // MVCC visibility check (M7: only check latest version)
                if let Some(ref snapshot) = self.snapshot {
                    let create_tx = version_header.create_tx_id();
                    let commit_tx = version_header.commit_tx_id();
                    let visible = snapshot.is_visible(create_tx, commit_tx)
                        || snapshot.is_visible_self(create_tx, commit_tx);
                    if !visible {
                        return Ok(None);
                    }
                }

                let values = deserialize_tuple(&tuple_bytes, &self.schema)?;
                Ok(Some(ExecResult::Row(values)))
            }
            None => Ok(None),
        }
    }
}
