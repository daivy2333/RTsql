//! Set operation executor — MS25 T3: `UNION [ALL]` / `INTERSECT` / `EXCEPT`
//!
//! Materializes both branch row streams on the first `next()` call and combines
//! them per the operation:
//!
//! - `UNION` — left rows followed by right rows, de-duplicated unless `all`.
//! - `UNION ALL` — left rows followed by right rows, duplicates and input order
//!   preserved.
//! - `INTERSECT` — left rows present in the right set, de-duplicated.
//! - `EXCEPT` — left rows absent from the right set, de-duplicated.
//!
//! Matching and de-duplication share the `HashSet<Vec<Value>>` kernel of
//! `DistinctExecutor` (`Value`'s `Eq`/`Hash`, NULL equal to NULL), so all row
//! algebra in this change has a single source of NULL semantics. Row order is
//! the left branch's first-occurrence order (documented; compatible with
//! SQLite's "order unspecified").

use crate::executor::executor_trait::Executor;
use crate::executor::plan::SetOpKind;
use crate::executor::result::ExecResult;
use crate::executor::Value;
use crate::storage;
use async_trait::async_trait;
use std::collections::HashSet;

pub struct SetOperationExecutor {
    left: Box<dyn Executor + Send>,
    right: Box<dyn Executor + Send>,
    op: SetOpKind,
    all: bool,
    /// Combined rows (empty until consumed)
    rows: Vec<Vec<Value>>,
    /// Whether both inputs have been consumed
    has_consumed_input: bool,
    /// Current position in `rows`
    index: usize,
}

impl SetOperationExecutor {
    pub fn new(
        left: Box<dyn Executor + Send>,
        right: Box<dyn Executor + Send>,
        op: SetOpKind,
        all: bool,
    ) -> Self {
        Self {
            left,
            right,
            op,
            all,
            rows: Vec::new(),
            has_consumed_input: false,
            index: 0,
        }
    }

    /// Drain one input stream into a row vector
    async fn drain(input: &mut Box<dyn Executor + Send>) -> storage::Result<Vec<Vec<Value>>> {
        let mut rows = Vec::new();
        loop {
            match input.next().await? {
                Some(ExecResult::Row(row)) => rows.push(row),
                Some(_) => {} // skip non-row results
                None => break,
            }
        }
        Ok(rows)
    }

    /// Consume both branches and compute the combined row stream
    async fn consume_input(&mut self) -> storage::Result<()> {
        let left_rows = Self::drain(&mut self.left).await?;
        let right_rows = Self::drain(&mut self.right).await?;

        let mut rows: Vec<Vec<Value>> = Vec::new();
        let mut seen: HashSet<Vec<Value>> = HashSet::new();
        match self.op {
            SetOpKind::Union => {
                if self.all {
                    rows = left_rows;
                    rows.extend(right_rows);
                } else {
                    for row in left_rows.into_iter().chain(right_rows) {
                        if seen.insert(row.clone()) {
                            rows.push(row);
                        }
                    }
                }
            }
            SetOpKind::Intersect => {
                let right: HashSet<Vec<Value>> = right_rows.into_iter().collect();
                for row in left_rows {
                    if right.contains(&row) && seen.insert(row.clone()) {
                        rows.push(row);
                    }
                }
            }
            SetOpKind::Except => {
                let right: HashSet<Vec<Value>> = right_rows.into_iter().collect();
                for row in left_rows {
                    if !right.contains(&row) && seen.insert(row.clone()) {
                        rows.push(row);
                    }
                }
            }
        }

        self.rows = rows;
        self.has_consumed_input = true;
        Ok(())
    }
}

#[async_trait]
impl Executor for SetOperationExecutor {
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
