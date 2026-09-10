//! Projection executor - SELECT 派生列（投影表达式机制）
//!
//! MS11-T01 Iter001：对输入的全形状行逐项求值 `ProjectionItem` 表达式并按
//! 项顺序产出。求值必须走 owned `Expression::evaluate` 路径——新值表达式
//! （CASE/COALESCE/CAST）的 `evaluate_ref` 零拷贝路径对 String 结果显式
//! 报错，禁止使用（Iteration 000 Plan Review 硬约束）。

use crate::executor::{ExecResult, Executor, ExpressionRef};
use crate::storage::Result;

/// Projection executor - 逐行求值 SELECT 列表表达式项
pub struct ProjectionExecutor {
    input: Box<dyn Executor + Send>,
    items: Vec<ExpressionRef>,
}

impl ProjectionExecutor {
    /// Create a new projection executor（列名在 plan 节点上，执行器只求值）
    pub fn new(input: Box<dyn Executor + Send>, items: Vec<ExpressionRef>) -> Self {
        Self { input, items }
    }
}

#[async_trait::async_trait]
impl Executor for ProjectionExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        match self.input.next().await? {
            None => Ok(None),
            Some(ExecResult::Row(values)) => {
                let mut out = Vec::with_capacity(self.items.len());
                for expr in &self.items {
                    let value = expr.evaluate(&values).map_err(|e| {
                        crate::storage::StorageError::ExecutionError(format!(
                            "Expression evaluation error: {}",
                            e
                        ))
                    })?;
                    out.push(value);
                }
                Ok(Some(ExecResult::Row(out)))
            }
            // SELECT 侧输入只产出行；非行结果原样透传（与 Limit 同策略）
            Some(other) => Ok(Some(other)),
        }
    }
}
