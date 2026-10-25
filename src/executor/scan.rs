//! Scan executor - full table scan

use crate::executor::apply_projection;
use crate::executor::data_scan::DataScanExecutor;
use crate::executor::{ExecResult, Executor, Value};
use crate::storage::page_format::{deserialize_row, ColumnType};
use crate::storage::{read_tuple_from_data_page, BufferPool, Result, TableMeta};
use crate::transaction::{Snapshot, TransactionManager};
use std::sync::Arc;

pub struct ScanExecutor {
    table_meta: Arc<TableMeta>,
    buffer_pool: Arc<BufferPool>,
    schema: Vec<ColumnType>,
    snapshot: Option<Snapshot>,
    /// MS10-T01 Iter001: output projection (empty = identity).
    projection: Vec<usize>,
    /// T03-D1: transaction manager for the tombstone deleter-commit-state
    /// suppression decision (I033) on the keyless data-page fallback. `None`
    /// is equivalent to an empty active set (direct unit-test constructions).
    tx_manager: Option<Arc<TransactionManager>>,
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
            tx_manager: None,
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

    /// T03-D1: supply the transaction manager used by the keyless data-page
    /// fallback (I033 tombstone suppression). The PK path never consults it.
    pub fn with_tx_manager(mut self, tx_manager: Option<Arc<TransactionManager>>) -> Self {
        self.tx_manager = tx_manager;
        self
    }

    /// T03-D1: keyless criterion — the PK-position column type is not `Int`
    /// (or the position is out of range). Such a table's rows carry no
    /// keyable PK value, so they are stored on the data pages only and never
    /// enter the PK index: the index enumeration channel would yield nothing.
    fn is_keyless(table_meta: &TableMeta) -> bool {
        !matches!(
            table_meta.columns.get(table_meta.pk_index),
            Some((_, ColumnType::Int))
        )
    }

    /// T03-D1: keyless tables fall back to the data page chain (reusing
    /// `DataScanExecutor` with no predicate and no scan cap), so their base
    /// rows are fully visible to the enclosing join. The externally
    /// materialized shape matches the PK path (all rows collected up front).
    async fn collect_keyless_rows(&mut self) -> Result<()> {
        let mut scan = DataScanExecutor::new(
            self.table_meta.clone(),
            self.buffer_pool.clone(),
            self.snapshot.clone(),
            None,
            None,
            self.tx_manager.clone(),
        )
        .with_projection(self.projection.clone());

        while let Some(result) = scan.next().await? {
            if let ExecResult::Row(values) = result {
                self.results.push(values);
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl Executor for ScanExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        // change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
        // Iteration 003（design D7 / ISS25）：在途窄化的表被读 ⇒ **明确拒绝**
        // ——不得返回按位置错位的值，也不得静默返回空结果。检查在 `executed`
        // 置位之前：拒绝后重试仍是同一拒绝。
        self.table_meta.narrowing.ensure_readable()?;
        if !self.executed {
            self.executed = true;

            if Self::is_keyless(&self.table_meta) {
                // T03-D1: 无键表行只落数据页——枚举通道回退数据页走查。
                self.collect_keyless_rows().await?;
            } else {
                let entries = self.table_meta.index_manager.scan_all().await?;
                for (_key, row_id) in entries {
                    if let Some(ref snapshot) = self.snapshot {
                        // M36 closure path; MS21 Iter003 (T3-a/D9): width-aware
                        // read-back + DEFAULT overlay so ADD COLUMN narrow rows
                        // materialize at full schema width.
                        let values_opt = self
                            .buffer_pool
                            .find_visible_version(row_id, snapshot, |bytes| {
                                deserialize_row(bytes, &self.schema, &self.table_meta.defaults)
                            })
                            .await?;

                        match values_opt {
                            Some(values) => self
                                .results
                                .push(apply_projection(&self.projection, values)),
                            None => continue, // all versions invisible
                        }
                    } else {
                        // M36 closure path; MS21 Iter003 (T3-a/D9): width-aware
                        // read-back + DEFAULT overlay.
                        let values =
                            read_tuple_from_data_page(&self.buffer_pool, row_id, |_vh, bytes| {
                                deserialize_row(bytes, &self.schema, &self.table_meta.defaults)
                            })
                            .await?;
                        self.results
                            .push(apply_projection(&self.projection, values));
                    }
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
