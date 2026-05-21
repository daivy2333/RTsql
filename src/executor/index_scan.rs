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
                if let Some(ref snapshot) = self.snapshot {
                    // M10: Use find_visible_version for version chain traversal
                    let tuple_bytes = self.buffer_pool.find_visible_version(id, snapshot).await?;

                    match tuple_bytes {
                        Some(data) => {
                            let values = deserialize_tuple(&data, &self.schema)?;
                            Ok(Some(ExecResult::Row(values)))
                        }
                        None => Ok(None), // All versions invisible
                    }
                } else {
                    // No snapshot: read latest version (backward compat)
                    let (_, tuple_bytes) = read_tuple_from_data_page(&self.buffer_pool, id).await?;
                    let values = deserialize_tuple(&tuple_bytes, &self.schema)?;
                    Ok(Some(ExecResult::Row(values)))
                }
            }
            None => Ok(None),
        }
    }
}
