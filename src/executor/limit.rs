//! Limit executor - LIMIT and OFFSET clause handling

use crate::executor::{ExecResult, Executor};
use crate::storage::Result;

/// Limit executor - handles LIMIT and OFFSET clauses
pub struct LimitExecutor {
    input: Box<dyn Executor + Send>,
    limit: usize,
    offset: usize,
    skipped: usize,
    taken: usize,
}

impl LimitExecutor {
    /// Create a new limit executor
    pub fn new(input: Box<dyn Executor + Send>, limit: usize, offset: usize) -> Self {
        Self {
            input,
            limit,
            offset,
            skipped: 0,
            taken: 0,
        }
    }
}

#[async_trait::async_trait]
impl Executor for LimitExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        // LIMIT 0 returns no rows immediately
        if self.limit == 0 {
            return Ok(None);
        }

        // Skip OFFSET rows first
        while self.skipped < self.offset {
            match self.input.next().await? {
                None => return Ok(None), // OFFSET exceeds total rows
                Some(ExecResult::Row(_)) => {
                    self.skipped += 1;
                }
                Some(ExecResult::AffectedRows(n)) => {
                    // Pass through affected rows, but don't count as skipped
                    return Ok(Some(ExecResult::AffectedRows(n)));
                }
                Some(ExecResult::RowId(row_id)) => {
                    // Pass through RowId, but don't count as skipped
                    return Ok(Some(ExecResult::RowId(row_id)));
                }
            }
        }

        // Check if we've already taken LIMIT rows
        if self.taken >= self.limit {
            return Ok(None);
        }

        // Get next row from input
        match self.input.next().await? {
            None => Ok(None),
            Some(result) => {
                // Only count Row results toward the limit
                if matches!(result, ExecResult::Row(_)) {
                    self.taken += 1;
                }
                Ok(Some(result))
            }
        }
    }
}
