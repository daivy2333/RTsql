//! SortExecutor unit tests

use rtsql::executor::{ExecResult, Executor, OrderByColumn, SortExecutor, Value};

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
async fn test_sort_single_column_asc() {
    // 输入：[3, 1, 2] → 输出：[1, 2, 3]
    let rows = vec![
        vec![Value::Int(3)],
        vec![Value::Int(1)],
        vec![Value::Int(2)],
    ];

    let columns = vec!["id".to_string()];
    let order_by = vec![OrderByColumn {
        column: "id".to_string(),
        asc: true,
    }];
    let mut executor = SortExecutor::new(Box::new(MockExecutor::new(rows)), order_by, columns);

    let mut results = vec![];
    while let Some(ExecResult::Row(row)) = executor.next().await.unwrap() {
        results.push(row);
    }

    assert_eq!(
        results,
        vec![
            vec![Value::Int(1)],
            vec![Value::Int(2)],
            vec![Value::Int(3)],
        ]
    );
}

#[tokio::test]
async fn test_sort_single_column_desc() {
    // 输入：[1, 2, 3] → 输出：[3, 2, 1]
    let rows = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
    ];

    let columns = vec!["id".to_string()];
    let order_by = vec![OrderByColumn {
        column: "id".to_string(),
        asc: false,
    }];
    let mut executor = SortExecutor::new(Box::new(MockExecutor::new(rows)), order_by, columns);

    let mut results = vec![];
    while let Some(ExecResult::Row(row)) = executor.next().await.unwrap() {
        results.push(row);
    }

    assert_eq!(
        results,
        vec![
            vec![Value::Int(3)],
            vec![Value::Int(2)],
            vec![Value::Int(1)],
        ]
    );
}

#[tokio::test]
async fn test_sort_multi_column() {
    // 输入：[(1, 'b'), (1, 'a'), (2, 'c')] → 输出：[(1, 'a'), (1, 'b'), (2, 'c')]
    let rows = vec![
        vec![Value::Int(1), Value::String("b".to_string())],
        vec![Value::Int(1), Value::String("a".to_string())],
        vec![Value::Int(2), Value::String("c".to_string())],
    ];

    let columns = vec!["age".to_string(), "name".to_string()];
    let order_by = vec![
        OrderByColumn {
            column: "age".to_string(),
            asc: true,
        },
        OrderByColumn {
            column: "name".to_string(),
            asc: true,
        },
    ];
    let mut executor = SortExecutor::new(Box::new(MockExecutor::new(rows)), order_by, columns);

    let mut results = vec![];
    while let Some(ExecResult::Row(row)) = executor.next().await.unwrap() {
        results.push(row);
    }

    assert_eq!(
        results,
        vec![
            vec![Value::Int(1), Value::String("a".to_string())],
            vec![Value::Int(1), Value::String("b".to_string())],
            vec![Value::Int(2), Value::String("c".to_string())],
        ]
    );
}

#[tokio::test]
async fn test_sort_null_at_end() {
    // 输入：[NULL, 1, 3, NULL, 2] → 输出：[1, 2, 3, NULL, NULL]
    let rows = vec![
        vec![Value::Null],
        vec![Value::Int(1)],
        vec![Value::Int(3)],
        vec![Value::Null],
        vec![Value::Int(2)],
    ];

    let columns = vec!["val".to_string()];
    let order_by = vec![OrderByColumn {
        column: "val".to_string(),
        asc: true,
    }];
    let mut executor = SortExecutor::new(Box::new(MockExecutor::new(rows)), order_by, columns);

    let mut results = vec![];
    while let Some(ExecResult::Row(row)) = executor.next().await.unwrap() {
        results.push(row);
    }

    // NULL 排在末尾
    assert_eq!(results[0], vec![Value::Int(1)]);
    assert_eq!(results[1], vec![Value::Int(2)]);
    assert_eq!(results[2], vec![Value::Int(3)]);
    assert!(results[3][0].is_null());
    assert!(results[4][0].is_null());
}

#[tokio::test]
async fn test_sort_empty_input() {
    let rows = vec![];

    let columns = vec!["id".to_string()];
    let order_by = vec![OrderByColumn {
        column: "id".to_string(),
        asc: true,
    }];
    let mut executor = SortExecutor::new(Box::new(MockExecutor::new(rows)), order_by, columns);

    let result = executor.next().await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_sort_with_float() {
    // 输入：[Float(2.5), Int(1), Float(3.0)] → 输出：[Int(1), Float(2.5), Float(3.0)]
    // Float 和 Int 比较时自动转换
    let rows = vec![
        vec![Value::Float(2.5)],
        vec![Value::Int(1)],
        vec![Value::Float(3.0)],
    ];

    let columns = vec!["val".to_string()];
    let order_by = vec![OrderByColumn {
        column: "val".to_string(),
        asc: true,
    }];
    let mut executor = SortExecutor::new(Box::new(MockExecutor::new(rows)), order_by, columns);

    let mut results = vec![];
    while let Some(ExecResult::Row(row)) = executor.next().await.unwrap() {
        results.push(row);
    }

    assert_eq!(results[0], vec![Value::Int(1)]);
    assert_eq!(results[1], vec![Value::Float(2.5)]);
    assert_eq!(results[2], vec![Value::Float(3.0)]);
}
