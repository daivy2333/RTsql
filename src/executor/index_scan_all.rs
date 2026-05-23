//! Index scan executor for non-unique indexes - returns all matching rows

use crate::executor::{ExecResult, Executor};
use crate::profiling::{is_profiling_enabled, record_time};
use crate::storage::page_format::{deserialize_tuple, ColumnType, RowId};
use crate::storage::{read_tuple_from_data_page, BufferPool, Result, TableMeta};
use crate::transaction::Snapshot;
use std::sync::Arc;
use std::time::Instant;

pub struct IndexScanAllExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    key: Vec<u8>,
    schema: Vec<ColumnType>,
    snapshot: Option<Snapshot>,
    row_ids: Vec<RowId>,
    current_idx: usize,
    initialized: bool,
}

impl IndexScanAllExecutor {
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
            row_ids: Vec::new(),
            current_idx: 0,
            initialized: false,
        }
    }
}

#[async_trait::async_trait]
impl Executor for IndexScanAllExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        // Lazy initialization: execute search-all on first call
        if !self.initialized {
            let profiling = is_profiling_enabled();
            if profiling {
                let t0 = Instant::now();
                self.row_ids = self.table_meta.index_manager.search_all(&self.key).await?;
                record_time("index_manager_search_all", t0.elapsed());
            } else {
                self.row_ids = self.table_meta.index_manager.search_all(&self.key).await?;
            }
            self.initialized = true;
        }

        // Iterate through all RowIds, returning visible versions
        while self.current_idx < self.row_ids.len() {
            let row_id = self.row_ids[self.current_idx];
            self.current_idx += 1;

            // MVCC visibility check (reuse IndexScanExecutor logic)
            if let Some(ref snapshot) = self.snapshot {
                let tuple_bytes = self
                    .buffer_pool
                    .find_visible_version(row_id, snapshot)
                    .await?;

                match tuple_bytes {
                    Some(data) => {
                        let values = deserialize_tuple(&data, &self.schema)?;
                        return Ok(Some(ExecResult::Row(values)));
                    }
                    None => {
                        // All versions invisible: skip to next RowId
                        continue;
                    }
                }
            } else {
                // No snapshot: read latest version (backward compat)
                let (_, tuple_bytes) = read_tuple_from_data_page(&self.buffer_pool, row_id).await?;
                let values = deserialize_tuple(&tuple_bytes, &self.schema)?;
                return Ok(Some(ExecResult::Row(values)));
            }
        }

        Ok(None) // All RowIds processed
    }
}
