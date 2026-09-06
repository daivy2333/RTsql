//! Scan executor - full table scan

use crate::executor::apply_projection;
use crate::executor::{ExecResult, Executor, Value};
use crate::storage::page_format::{deserialize_value_refs, ColumnType};
use crate::storage::{read_tuple_from_data_page, BufferPool, Result, TableMeta};
use crate::transaction::Snapshot;
use std::sync::Arc;

pub struct ScanExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    schema: Vec<ColumnType>,
    snapshot: Option<Snapshot>,
    /// MS10-T01 Iter001: output projection (empty = identity).
    projection: Vec<usize>,
    results: Vec<Vec<Value>>,
    index: usize,
    executed: bool,
}

impl ScanExecutor {
    pub fn new(
        table_meta: Arc<TableMeta>,
        buffer_pool: Arc<BufferPool>,
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
            schema,
            snapshot,
            projection: Vec::new(),
            results: Vec::new(),
            index: 0,
            executed: false,
        }
    }

    /// MS10-T01 Iter001: narrow produced rows to the given full-schema column
    /// indices (empty = identity, the `new()` default).
    pub fn with_projection(mut self, projection: Vec<usize>) -> Self {
        self.projection = projection;
        self
    }
}

#[async_trait::async_trait]
impl Executor for ScanExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if !self.executed {
            self.executed = true;

            let entries = self.table_meta.index_manager.scan_all().await?;
            for (_key, row_id) in entries {
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
                        Some(values) => self
                            .results
                            .push(apply_projection(&self.projection, values)),
                        None => continue, // all versions invisible
                    }
                } else {
                    // M36: closure-based zero-copy
                    let values =
                        read_tuple_from_data_page(&self.buffer_pool, row_id, |_vh, bytes| {
                            deserialize_value_refs(bytes, &self.schema)
                                .map(|vrs| vrs.iter().map(|vr| vr.to_value()).collect::<Vec<_>>())
                        })
                        .await?;
                    self.results
                        .push(apply_projection(&self.projection, values));
                }
            }
        }

        if self.index < self.results.len() {
            let values = self.results[self.index].clone();
            self.index += 1;
            Ok(Some(ExecResult::Row(values)))
        } else {
            Ok(None)
        }
    }
}
