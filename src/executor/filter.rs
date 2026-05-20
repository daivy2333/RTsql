//! Filter executor - WHERE clause filtering

use crate::executor::{ExecResult, Executor, PredicateRef};
use crate::storage::Result;

/// Filter executor - filters rows based on a WHERE predicate
pub struct FilterExecutor {
    input: Box<dyn Executor + Send>,
    predicate: PredicateRef,
}

impl FilterExecutor {
    /// Create a new filter executor
    pub fn new(input: Box<dyn Executor + Send>, predicate: PredicateRef) -> Self {
        Self { input, predicate }
    }
}

#[async_trait::async_trait]
impl Executor for FilterExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        // Loop until we find a row that satisfies the predicate or exhaust input
        loop {
            match self.input.next().await? {
                None => return Ok(None), // Input exhausted

                Some(ExecResult::Row(values)) => {
                    // Evaluate predicate against the row
                    match self.predicate.evaluate(&values) {
                        Ok(true) => return Ok(Some(ExecResult::Row(values))),
                        Ok(false) => continue, // Skip this row
                        Err(e) => {
                            return Err(crate::storage::StorageError::ExecutionError(format!(
                                "Predicate evaluation error: {}",
                                e
                            )))
                        }
                    }
                }

                Some(ExecResult::AffectedRows(n)) => {
                    // Pass through affected rows count (for UPDATE/DELETE with WHERE)
                    return Ok(Some(ExecResult::AffectedRows(n)));
                }

                Some(ExecResult::RowId(row_id)) => {
                    // RowId is not expected in filter context (scan returns Row values)
                    // Pass through as-is for now
                    return Ok(Some(ExecResult::RowId(row_id)));
                }
            }
        }
    }
}
