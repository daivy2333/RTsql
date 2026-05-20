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
                let (version_header, tuple_bytes) =
                    read_tuple_from_data_page(&self.buffer_pool, row_id).await?;

                // MVCC visibility check (M7: only check latest version)
                if let Some(ref snapshot) = self.snapshot {
                    let create_tx = version_header.create_tx_id();
                    let commit_tx = version_header.commit_tx_id();
                    let visible = snapshot.is_visible(create_tx, commit_tx)
                        || snapshot.is_visible_self(create_tx, commit_tx);
                    if !visible {
                        continue;
                    }
                }

                let values = deserialize_tuple(&tuple_bytes, &self.schema)?;
                self.results.push(values);
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
