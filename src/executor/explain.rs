//! EXPLAIN 计划树执行器（MS25 T6）
//!
//! 静态行迭代：构造期一次性渲染计划树文本，之后每行产出一个单值行。
//! 内层计划**从不执行**——本执行器不持有内层执行器（pipeline 构造臂亦不构造），
//! 因此派生表急切物化与任何执行器副作用都不触发。

use crate::executor::plan::PhysicalPlan;
use crate::executor::plan_format::render_plan_tree;
use crate::executor::{ExecResult, Executor, Value};
use crate::storage::Result;

/// 计划树文本执行器
#[derive(Debug)]
pub struct ExplainExecutor {
    lines: std::vec::IntoIter<String>,
}

impl ExplainExecutor {
    /// 渲染被观测计划（不执行）并固化为文本行。
    pub fn new(input: &PhysicalPlan) -> Self {
        Self {
            lines: render_plan_tree(input).into_iter(),
        }
    }
}

#[async_trait::async_trait]
impl Executor for ExplainExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        match self.lines.next() {
            Some(line) => Ok(Some(ExecResult::Row(vec![Value::String(line)]))),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::plan::LimitNode;

    /// 逐节点产出一行（首行恒 `EXPLAIN` 根标识），内层计划零执行。
    #[tokio::test]
    async fn yields_one_row_per_plan_node() {
        let plan = PhysicalPlan::Limit(LimitNode {
            input: Box::new(PhysicalPlan::SingleRow),
            limit: 3,
            offset: 0,
        });
        let mut exec = ExplainExecutor::new(&plan);

        let mut rows: Vec<Vec<Value>> = Vec::new();
        while let Some(result) = exec.next().await.unwrap() {
            match result {
                ExecResult::Row(row) => rows.push(row),
                other => panic!("expected Row, got {other:?}"),
            }
        }

        assert_eq!(rows.len(), 3, "根行 + Limit + SingleRow");
        assert!(matches!(&rows[0][0], Value::String(s) if s == "EXPLAIN"));
        assert!(matches!(&rows[1][0], Value::String(s) if s.contains("Limit")));
        assert!(rows.iter().all(|r| r.len() == 1), "行宽恒 1");
    }
}
