//! Executor trait - async iterator interface

use async_trait::async_trait;

use crate::executor::ExecResult;
use crate::storage::Result;

/// Executor trait - 异步迭代器接口
#[async_trait]
pub trait Executor {
    /// 执行一次迭代，返回结果
    /// None 表示迭代结束（无更多结果）
    async fn next(&mut self) -> Result<Option<ExecResult>>;
}
