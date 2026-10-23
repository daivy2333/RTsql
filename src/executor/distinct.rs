//! Distinct executor — MS25 T2: `SELECT DISTINCT` output-row de-duplication
//!
//! Materializes the input row stream on the first `next()` call, keeps the first
//! occurrence of each output row and emits the survivors in first-occurrence
//! order. De-duplication uses `Value`'s `Eq`/`Hash` (discriminant-tagged, Float
//! by bit pattern, `Null == Null` with a matching hash), so NULL groups fold to
//! one row without any extra NULL-matching rule — the same kernel the set
//! operations reuse.

use crate::executor::executor_trait::Executor;
use crate::executor::query_budget::{row_bytes, unlimited_budget, SharedBudget};
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
    /// MS32 Iter003（design D5）：语句级查询内存预算——去重输出行与 seen
    /// 集合副本分别入账。
    budget: SharedBudget,
}

impl DistinctExecutor {
    pub fn new(input: Box<dyn Executor + Send>) -> Self {
        Self {
            input,
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

    /// Consume the whole input, keeping the first occurrence of each row
    async fn consume_input(&mut self) -> storage::Result<()> {
        let mut seen: HashSet<Vec<Value>> = HashSet::new();
        loop {
            match self.input.next().await? {
                Some(ExecResult::Row(row)) => {
                    // MS32 Iter003（design D5）：幸存行两份驻留（输出行 + seen
                    // 副本）入账；重复行的瞬态克隆属在途数据不计。入账先于
                    // 插入——超限整句失败，无需回滚集合。
                    let bytes = row_bytes(&row);
                    if !seen.contains(&row) {
                        self.budget.charge(bytes.saturating_mul(2))?;
                        seen.insert(row.clone());
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
