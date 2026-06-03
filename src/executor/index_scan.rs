//! Index scan executor - primary key lookup

use crate::executor::{ExecResult, Executor};
use crate::profiling::{is_profiling_enabled, record_time};
use crate::storage::page_format::{deserialize_value_refs, ColumnType};
use crate::storage::{read_tuple_from_data_page, BufferPool, Result, TableMeta};
use crate::transaction::Snapshot;
use std::sync::Arc;
use std::time::Instant;

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

        let profiling = is_profiling_enabled();
        let row_id = {
            if profiling {
                let t0 = Instant::now();
                let result = self.table_meta.index_manager.search(&self.key).await?;
                record_time("index_manager_search", t0.elapsed());
                result
            } else {
                self.table_meta.index_manager.search(&self.key).await?
            }
        };

        match row_id {
            Some(id) => {
                if let Some(ref snapshot) = self.snapshot {
                    // M36: closure-based zero-copy (deserialize_value_refs + to_value)
                    let values_opt = self
                        .buffer_pool
                        .find_visible_version(id, snapshot, |bytes| {
                            deserialize_value_refs(bytes, &self.schema).map(|vrs| {
                                vrs.iter().map(|vr| vr.to_value()).collect::<Vec<_>>()
                            })
                        })
                        .await?;

                    match values_opt {
                        Some(values) => Ok(Some(ExecResult::Row(values))),
                        None => Ok(None), // all versions invisible
                    }
                } else {
                    // M36: closure-based zero-copy
                    let values = read_tuple_from_data_page(
                        &self.buffer_pool,
                        id,
                        |_vh, bytes| {
                            deserialize_value_refs(bytes, &self.schema).map(|vrs| {
                                vrs.iter().map(|vr| vr.to_value()).collect::<Vec<_>>()
                            })
                        },
                    )
                    .await?;
                    Ok(Some(ExecResult::Row(values)))
                }
            }
            None => Ok(None),
        }
    }
}
