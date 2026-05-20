//! Scan executor - full table scan (M5: NotImplemented)

use crate::executor::{ExecResult, Executor};
use crate::storage::Result;

/// ScanExecutor - 全表扫描执行器
/// M5: 暂不实现，返回 NotImplemented
pub struct ScanExecutor {
    executed: bool,
}

impl ScanExecutor {
    pub fn new() -> Self {
        Self { executed: false }
    }
}

impl Default for ScanExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Executor for ScanExecutor {
    async fn next(&mut self) -> Result<Option<ExecResult>> {
        if self.executed {
            return Ok(None);
        }

        self.executed = true;
        Ok(Some(ExecResult::NotImplemented))
    }
}
