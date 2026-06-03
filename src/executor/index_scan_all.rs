//! Index scan executor for non-unique indexes - returns all matching rows

use crate::executor::{ExecResult, Executor};
use crate::profiling::{is_profiling_enabled, record_time};
use crate::storage::page_format::{deserialize_value_refs, ColumnType, RowId};
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
                // M36: closure-based zero-copy (deserialize_value_refs + to_value)
                let values_opt = self
                    .buffer_pool
                    .find_visible_version(row_id, snapshot, |bytes| {
                        deserialize_value_refs(bytes, &self.schema)
                            .map(|vrs| vrs.iter().map(|vr| vr.to_value()).collect::<Vec<_>>())
                    })
                    .await?;

                match values_opt {
                    Some(values) => return Ok(Some(ExecResult::Row(values))),
                    None => continue, // all versions invisible: skip to next RowId
                }
            } else {
                // M36: closure-based zero-copy
                let values = read_tuple_from_data_page(&self.buffer_pool, row_id, |_vh, bytes| {
                    deserialize_value_refs(bytes, &self.schema)
                        .map(|vrs| vrs.iter().map(|vr| vr.to_value()).collect::<Vec<_>>())
                })
                .await?;
                return Ok(Some(ExecResult::Row(values)));
            }
        }

        Ok(None) // All RowIds processed
    }
}
