//! LimitExecutor unit tests

use rtsql::executor::{ExecResult, Executor, LimitExecutor, Value};

/// Mock executor that returns predefined rows
struct MockExecutor {
    rows: Vec<Vec<Value>>,
    index: usize,
}

impl MockExecutor {
    fn new(rows: Vec<Vec<Value>>) -> Self {
        Self { rows, index: 0 }
    }
}

#[async_trait::async_trait]
impl Executor for MockExecutor {
    async fn next(&mut self) -> rtsql::storage::Result<Option<ExecResult>> {
        if self.index >= self.rows.len() {
            Ok(None)
        } else {
            let row = self.rows[self.index].clone();
            self.index += 1;
            Ok(Some(ExecResult::Row(row)))
        }
    }
}

#[tokio::test]
async fn test_limit_only() {
    // 输入：[1, 2, 3, 4, 5] LIMIT 3 → 输出：[1, 2, 3]
    let rows = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
        vec![Value::Int(4)],
        vec![Value::Int(5)],
    ];

    let mut executor = LimitExecutor::new(Box::new(MockExecutor::new(rows)), 3, 0);

    let mut results = vec![];
    while let Some(ExecResult::Row(row)) = executor.next().await.unwrap() {
        results.push(row);
    }

    assert_eq!(results.len(), 3);
    assert_eq!(results[0], vec![Value::Int(1)]);
    assert_eq!(results[1], vec![Value::Int(2)]);
    assert_eq!(results[2], vec![Value::Int(3)]);
}

#[tokio::test]
async fn test_offset_only() {
    // 输入：[1, 2, 3, 4, 5] OFFSET 2 → 输出：[3, 4, 5]
    let rows = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
        vec![Value::Int(4)],
        vec![Value::Int(5)],
    ];

    // LIMIT 设为 usize::MAX 表示无限制
    let mut executor = LimitExecutor::new(Box::new(MockExecutor::new(rows)), usize::MAX, 2);

    let mut results = vec![];
    while let Some(ExecResult::Row(row)) = executor.next().await.unwrap() {
        results.push(row);
    }

    assert_eq!(results.len(), 3);
    assert_eq!(results[0], vec![Value::Int(3)]);
    assert_eq!(results[1], vec![Value::Int(4)]);
    assert_eq!(results[2], vec![Value::Int(5)]);
}

#[tokio::test]
async fn test_limit_with_offset() {
    // 输入：[1, 2, 3, 4, 5] LIMIT 2 OFFSET 2 → 输出：[3, 4]
    let rows = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
        vec![Value::Int(4)],
        vec![Value::Int(5)],
    ];

    let mut executor = LimitExecutor::new(Box::new(MockExecutor::new(rows)), 2, 2);

    let mut results = vec![];
    while let Some(ExecResult::Row(row)) = executor.next().await.unwrap() {
        results.push(row);
    }

    assert_eq!(results.len(), 2);
    assert_eq!(results[0], vec![Value::Int(3)]);
    assert_eq!(results[1], vec![Value::Int(4)]);
}

#[tokio::test]
async fn test_offset_exceeds_total() {
    // 输入：[1, 2, 3] OFFSET 10 → 输出：[]
    let rows = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
    ];

    let mut executor = LimitExecutor::new(Box::new(MockExecutor::new(rows)), usize::MAX, 10);

    let result = executor.next().await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_limit_zero() {
    // 输入：[1, 2, 3] LIMIT 0 → 输出：[]
    let rows = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
    ];

    let mut executor = LimitExecutor::new(Box::new(MockExecutor::new(rows)), 0, 0);

    let result = executor.next().await.unwrap();
    assert!(result.is_none());
}