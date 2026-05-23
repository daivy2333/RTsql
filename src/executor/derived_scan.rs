//! Derived scan executor - materializes subquery results as in-memory table

use crate::executor::{ExecResult, Executor, Value};
use crate::storage::Result;

pub struct DerivedScanExecutor {
    rows: Vec<Vec<Value>>,
    current_row: usize,
}

impl DerivedScanExecutor {
    pub fn new(rows: Vec<Vec<Value>>) -> Self {
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
