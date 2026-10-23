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
use crate::executor::query_budget::{row_bytes, unlimited_budget, SharedBudget};
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
    /// MS32 Iter003（design D5）：语句级查询内存预算——分支行、去重集合与
    /// 交/差右集副本分别入账，丢弃行出账。
    budget: SharedBudget,
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
            budget: unlimited_budget(),
        }
    }

    /// MS32 Iter003（design D5）：注入语句级预算（pipeline 构造臂）。
    pub(crate) fn with_budget(mut self, budget: SharedBudget) -> Self {
        self.budget = budget;
        self
    }

    /// Drain one input stream into a row vector
    async fn drain(
        input: &mut Box<dyn Executor + Send>,
        budget: &SharedBudget,
    ) -> storage::Result<Vec<Vec<Value>>> {
        let mut rows = Vec::new();
        loop {
            match input.next().await? {
                Some(ExecResult::Row(row)) => {
                    budget.charge(row_bytes(&row))?;
                    rows.push(row);
                }
                Some(_) => {} // skip non-row results
                None => break,
            }
        }
        Ok(rows)
    }

    /// Consume both branches and compute the combined row stream
    async fn consume_input(&mut self) -> storage::Result<()> {
        let budget = self.budget.clone();
        let left_rows = Self::drain(&mut self.left, &budget).await?;
        let right_rows = Self::drain(&mut self.right, &budget).await?;

        let mut rows: Vec<Vec<Value>> = Vec::new();
        let mut seen: HashSet<Vec<Value>> = HashSet::new();
        match self.op {
            SetOpKind::Union => {
                if self.all {
                    // 全部行原样进入输出——分支行入账随 move 持续有效。
                    rows = left_rows;
                    rows.extend(right_rows);
                } else {
                    for row in left_rows.into_iter().chain(right_rows) {
                        let bytes = row_bytes(&row);
                        if !seen.contains(&row) {
                            // 幸存行：输出行已入账，补 seen 副本；重复行出账。
                            budget.charge(bytes)?;
                            seen.insert(row.clone());
                            rows.push(row);
                        } else {
                            budget.release(bytes);
                        }
                    }
                }
            }
            SetOpKind::Intersect | SetOpKind::Except => {
                // 右集副本逐行入账（重复行出账——集合只驻留一份）。
                let mut right: HashSet<Vec<Value>> = HashSet::new();
                for row in right_rows {
                    let bytes = row_bytes(&row);
                    if !right.contains(&row) {
                        budget.charge(bytes)?;
                        right.insert(row);
                    } else {
                        budget.release(bytes);
                    }
                }
                let want = self.op == SetOpKind::Intersect;
                for row in left_rows {
                    let bytes = row_bytes(&row);
                    let survives = right.contains(&row) == want && !seen.contains(&row);
                    if survives {
                        // 输出行已入账，补 seen 副本；未幸存左行出账。
                        budget.charge(bytes)?;
                        seen.insert(row.clone());
                        rows.push(row);
                    } else {
                        budget.release(bytes);
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
