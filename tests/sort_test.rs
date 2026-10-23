//! SortExecutor unit tests

use rtsql::database::Database;
use rtsql::executor::{ExecResult, Executor, OrderByColumn, SortExecutor, Value};
use rtsql::SqlOutcome;

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
        expr: None,
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
        expr: None,
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
            expr: None,
        },
        OrderByColumn {
            column: "name".to_string(),
            asc: true,
            expr: None,
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
        expr: None,
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
        expr: None,
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
        expr: None,
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

// =============================================================================
// MS32 Iter002 有界排序：输出等价与 Distinct 形态边界（Task 3.1）
// =============================================================================

/// 取查询结果的行集合。
async fn query_rows(db: &Database, sql: &str) -> Vec<Vec<Value>> {
    match db.execute_sql(sql).await.unwrap() {
        SqlOutcome::Rows { rows, .. } => rows,
        other => panic!("expected Rows, got {other:?}"),
    }
}

/// 唯一键前缀等价：乱序插入 N 行（N > k + m），`ORDER BY id LIMIT k OFFSET m`
/// 的输出 = 全量升序的中段切片。该等价在有界驻留改造前后都必须成立。
#[tokio::test]
async fn order_by_unique_key_limit_offset_matches_full_sort_prefix() {
    let db = Database::open_in_memory().await.unwrap();

    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();
    // N = 10，k = 3，m = 2：插入序打乱，唯一键保证输出无相等键歧义。
    db.execute_sql("INSERT INTO t VALUES (7), (2), (9), (4), (1), (8), (3), (10), (5), (6)")
        .await
        .unwrap();

    let rows = query_rows(&db, "SELECT id FROM t ORDER BY id LIMIT 3 OFFSET 2").await;
    assert_eq!(
        rows,
        vec![
            vec![Value::Int(3)],
            vec![Value::Int(4)],
            vec![Value::Int(5)],
        ]
    );
}

/// DISTINCT 形态不启用有界：重复值横跨截断界——若误对 Distinct 之下的
/// Sort 设界（驻留 2 行 [1, 1]），Distinct 去重后输出会缺值（只剩 [1]）。
/// 正确行为是全量排序再去重再截取：[1, 2]。
#[tokio::test]
async fn select_distinct_order_by_limit_returns_distinct_prefix() {
    let db = Database::open_in_memory().await.unwrap();

    // v 需可重复：显式声明 id 主键，避免首列隐式主键语义锁死 v 唯一。
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1, 1), (2, 1), (3, 2), (4, 2), (5, 3), (6, 3), (7, 4)")
        .await
        .unwrap();

    let rows = query_rows(&db, "SELECT DISTINCT v FROM t ORDER BY v LIMIT 2").await;
    assert_eq!(rows, vec![vec![Value::Int(1)], vec![Value::Int(2)]]);
}
