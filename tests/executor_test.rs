//! Executor unit tests

use rtsql::executor::{ExecResult, Executor, ScanExecutor};
use rtsql::storage::Result;

#[tokio::test]
async fn test_scan_executor_returns_not_implemented() -> Result<()> {
    let mut executor = ScanExecutor::new();

    // 第一次 next 返回 NotImplemented
    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::NotImplemented));

    // 第二次 next 返回 None（迭代结束）
    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}