//! Single row executor - MS13 T9 no-FROM SELECT 虚拟单行输入
//!
//! 恰产出一行空行（`vec![]`）后结束；供 `ProjectionExecutor` 在该行上逐项
//! 求值 SELECT 表达式项。空行上 ColumnExpression 不可达——no-FORM 分支中
//! 列引用在 plan 期即 `ColumnNotFound`（design D13 安全性质）。

use crate::executor::{ExecResult, Executor};
use crate::storage::Result;

/// 无 FROM 虚拟单行输入执行器
#[derive(Debug, Default, Clone, Copy)]
pub struct SingleRowExecutor {
    yielded: bool,
}

impl SingleRowExecutor {
    pub fn new() -> Self {
        Self { yielded: false }
    }
}

#[async_trait::async_trait]
impl Executor for SingleRowExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.yielded {
            Ok(None)
        } else {
            self.yielded = true;
            Ok(Some(ExecResult::Row(Vec::new())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn yields_exactly_one_empty_row() {
        let mut exec = SingleRowExecutor::new();
        match exec.next().await.unwrap() {
            Some(ExecResult::Row(row)) => assert!(row.is_empty()),
            other => panic!("expected empty row, got {:?}", other),
        }
        assert!(exec.next().await.unwrap().is_none());
    }
}
