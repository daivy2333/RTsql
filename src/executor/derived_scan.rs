//! Derived scan executor - materializes subquery results as in-memory table

use crate::executor::{apply_projection, ExecResult, Executor, Value};
use crate::storage::Result;

pub struct DerivedScanExecutor {
    rows: Vec<Vec<Value>>,
    current_row: usize,
}

impl DerivedScanExecutor {
    /// MS26 T8: `projection` mirrors `ScanNode`/`DataScanNode.projection` — it
    /// trims each materialized derived row to the outer Select list; empty
    /// keeps the full derived row (identity). Applied through the same
    /// `apply_projection` helper the scan and filter executors use.
    pub fn new(rows: Vec<Vec<Value>>, projection: Vec<usize>) -> Self {
        let rows = if projection.is_empty() {
            rows
        } else {
            rows.into_iter()
                .map(|row| apply_projection(&projection, row))
                .collect()
        };
        Self {
            rows,
            current_row: 0,
        }
    }
}

#[async_trait::async_trait]
impl Executor for DerivedScanExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.current_row < self.rows.len() {
            let row = self.rows[self.current_row].clone();
            self.current_row += 1;
            Ok(Some(ExecResult::Row(row)))
        } else {
            Ok(None)
        }
    }
}
