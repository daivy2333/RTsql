//! Distinct executor — MS25 T2: `SELECT DISTINCT` output-row de-duplication
//!
//! Materializes the input row stream on the first `next()` call, keeps the first
//! occurrence of each output row and emits the survivors in first-occurrence
//! order. De-duplication uses `Value`'s `Eq`/`Hash` (discriminant-tagged, Float
//! by bit pattern, `Null == Null` with a matching hash), so NULL groups fold to
//! one row without any extra NULL-matching rule — the same kernel the set
//! operations reuse.

use crate::executor::executor_trait::Executor;
use crate::executor::result::ExecResult;
use crate::executor::Value;
use crate::storage;
use async_trait::async_trait;
use std::collections::HashSet;

pub struct DistinctExecutor {
    input: Box<dyn Executor + Send>,
    /// De-duplicated rows in first-occurrence order (empty until consumed)
    rows: Vec<Vec<Value>>,
    /// Whether the input has been consumed
    has_consumed_input: bool,
    /// Current position in `rows`
    index: usize,
}

impl DistinctExecutor {
    pub fn new(input: Box<dyn Executor + Send>) -> Self {
        Self {
            input,
            rows: Vec::new(),
            has_consumed_input: false,
            index: 0,
        }
    }

    /// Consume the whole input, keeping the first occurrence of each row
    async fn consume_input(&mut self) -> storage::Result<()> {
        let mut seen: HashSet<Vec<Value>> = HashSet::new();
        loop {
            match self.input.next().await? {
                Some(ExecResult::Row(row)) => {
                    if seen.insert(row.clone()) {
                        self.rows.push(row);
                    }
                }
                Some(_) => {} // skip non-row results
                None => break,
            }
        }
        self.has_consumed_input = true;
        Ok(())
    }
}

#[async_trait]
impl Executor for DistinctExecutor {
    async fn next(&mut self) -> storage::Result<Option<ExecResult>> {
        if !self.has_consumed_input {
            self.consume_input().await?;
        }
        if self.index < self.rows.len() {
            let row = self.rows[self.index].clone();
            self.index += 1;
            Ok(Some(ExecResult::Row(row)))
        } else {
            Ok(None)
        }
    }
}
