//! Having executor - filters aggregate result rows based on HAVING predicate

use crate::executor::executor_trait::Executor;
use crate::executor::predicate::PredicateRef;
use crate::executor::result::ExecResult;
use crate::storage;

/// Having executor - filters aggregate result rows
/// Structurally identical to FilterExecutor, but semantically filters
/// aggregate output rows (not raw table rows)
pub struct HavingExecutor {
    input: Box<dyn Executor + Send>,
    predicate: PredicateRef,
}

impl HavingExecutor {
    pub fn new(input: Box<dyn Executor + Send>, predicate: PredicateRef) -> Self {
        Self { input, predicate }
    }
}

#[async_trait::async_trait]
impl Executor for HavingExecutor {
    async fn next(&mut self) -> storage::Result<Option<ExecResult>> {
        loop {
            match self.input.next().await? {
                Some(ExecResult::Row(row)) => match self.predicate.evaluate(&row) {
                    Ok(true) => return Ok(Some(ExecResult::Row(row))),
                    Ok(false) => continue,
                    Err(e) => {
                        return Err(crate::storage::StorageError::ExecutionError(format!(
                            "HAVING predicate evaluation error: {}",
                            e
                        )));
                    }
                },
                Some(other) => return Ok(Some(other)),
                None => return Ok(None),
            }
        }
    }
}
