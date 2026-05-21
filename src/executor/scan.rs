//! Scan executor - full table scan

use crate::executor::{ExecResult, Executor, Value};
use crate::storage::page_format::{deserialize_tuple, ColumnType};
use crate::storage::{read_tuple_from_data_page, BufferPool, Result, TableMeta};
use crate::transaction::Snapshot;
use std::sync::Arc;

pub struct ScanExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    schema: Vec<ColumnType>,
    snapshot: Option<Snapshot>,
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
            results: Vec::new(),
            index: 0,
            executed: false,
        }
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
                    // M10: Use find_visible_version for version chain traversal
                    let tuple_bytes = self
                        .buffer_pool
                        .find_visible_version(row_id, snapshot)
                        .await?;

                    match tuple_bytes {
                        Some(data) => {
                            let values = deserialize_tuple(&data, &self.schema)?;
                            self.results.push(values);
                        }
                        None => {
                            // All versions invisible, skip this row
                            continue;
                        }
                    }
                } else {
                    // No snapshot: read latest version (backward compat)
                    let (_, tuple_bytes) =
                        read_tuple_from_data_page(&self.buffer_pool, row_id).await?;
                    let values = deserialize_tuple(&tuple_bytes, &self.schema)?;
                    self.results.push(values);
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
