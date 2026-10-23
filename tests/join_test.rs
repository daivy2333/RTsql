//! Join executor unit tests (M12)

use rtsql::database::Database;
use rtsql::executor::{
    ColumnRef, ExecResult, Executor, InsertExecutor, JoinCondition, JoinConfig, JoinExecutor,
    JoinSide, OutputColumn, ScanExecutor, Value,
};
use rtsql::storage::{
    data::TableManager, page_format::ColumnType, BufferPool, FileStorage, Result,
};
use rtsql::transaction::TransactionManager;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::tempdir;

/// Helper to create a test table and insert data
async fn create_and_populate_table(
    table_name: &str,
    columns: Vec<(&str, ColumnType)>,
    pk_column: &str,
    rows: Vec<Vec<Value>>,
    buffer_pool: Arc<BufferPool>,
) -> Result<Arc<rtsql::storage::data::TableMeta>> {
    let table_mgr = TableManager::new(buffer_pool.clone(), buffer_pool.storage().clone())
        .await
        .unwrap();
    let cols: Vec<(String, ColumnType)> = columns
        .into_iter()
        .map(|(name, t)| (name.to_string(), t))
        .collect();
    table_mgr.create_table(table_name, cols, pk_column).await?;

    let table_meta = table_mgr.get_table(table_name).await?;
    let tx_manager = Arc::new(TransactionManager::new());

    if !rows.is_empty() {
        let mut insert_executor =
            InsertExecutor::new(table_meta.clone(), buffer_pool, tx_manager, rows, 0, None);
        insert_executor.next().await?;
    }

    Ok(table_meta)
}

/// Collect all rows from an executor
async fn collect_rows(executor: &mut dyn Executor) -> Result<Vec<Vec<Value>>> {
    let mut rows = Vec::new();
    while let Some(result) = executor.next().await? {
        if let ExecResult::Row(values) = result {
            rows.push(values);
        }
    }
    Ok(rows)
}

// =============================================================================
// Basic Hash Join Test
// =============================================================================

#[tokio::test]
async fn test_join_basic_hash_join() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    // Create left table: users(id, name)
    let left_table = create_and_populate_table(
        "users",
        vec![("id", ColumnType::Int), ("name", ColumnType::String(100))],
        "id",
        vec![
            vec![Value::Int(1), Value::String("Alice".to_string())],
            vec![Value::Int(2), Value::String("Bob".to_string())],
            vec![Value::Int(3), Value::String("Carol".to_string())],
        ],
        buffer_pool.clone(),
    )
    .await?;

    // Create right table: orders(id, user_id, amount)
    let right_table = create_and_populate_table(
        "orders",
        vec![
            ("id", ColumnType::Int),
            ("user_id", ColumnType::Int),
            ("amount", ColumnType::Int),
        ],
        "id",
        vec![
            vec![Value::Int(101), Value::Int(1), Value::Int(100)],
            vec![Value::Int(102), Value::Int(2), Value::Int(200)],
            vec![Value::Int(103), Value::Int(1), Value::Int(150)],
        ],
        buffer_pool.clone(),
    )
    .await?;

    // Create scan executors
    let left_scan = ScanExecutor::new(left_table, buffer_pool.clone(), None);
    let right_scan = ScanExecutor::new(right_table, buffer_pool, None);

    // Create column index maps
    let mut left_indices = HashMap::new();
    left_indices.insert("id".to_string(), 0);
    left_indices.insert("name".to_string(), 1);

    let mut right_indices = HashMap::new();
    right_indices.insert("id".to_string(), 0);
    right_indices.insert("user_id".to_string(), 1);
    right_indices.insert("amount".to_string(), 2);

    // Join condition: users.id = orders.user_id
    let conditions = vec![JoinCondition {
        left_column: ColumnRef {
            table: Some("users".to_string()),
            column: "id".to_string(),
        },
        right_column: ColumnRef {
            table: Some("orders".to_string()),
            column: "user_id".to_string(),
        },
    }];

    // Output columns: users.name, orders.amount
    let output_columns = vec![
        OutputColumn {
            table: Some("users".to_string()),
            column: "name".to_string(),
            table_alias: "users".to_string(),
            column_index: 1,
        },
        OutputColumn {
            table: Some("orders".to_string()),
            column: "amount".to_string(),
            table_alias: "orders".to_string(),
            column_index: 2,
        },
    ];

    let mut join_executor = JoinExecutor::new(JoinConfig {
        left_executor: Box::new(left_scan),
        right_executor: Box::new(right_scan),
        conditions,
        output_columns,
        left_column_indices: left_indices,
        right_column_indices: right_indices,
        output_sides: vec![JoinSide::Left, JoinSide::Right],
        right_table_name: "orders".to_string(),
    });

    let results = collect_rows(&mut join_executor).await?;

    // Expected results:
    // Alice (id=1) matches orders 101 and 103
    // Bob (id=2) matches order 102
    // Carol (id=3) matches no orders
    assert_eq!(results.len(), 3);

    // Results should be in order: left table order, then right table matches
    // Row 1: Alice, 100
    assert_eq!(results[0].len(), 2);
    assert_eq!(results[0][0], Value::String("Alice".to_string()));
    assert_eq!(results[0][1], Value::Int(100));

    // Row 2: Alice, 150
    assert_eq!(results[1][0], Value::String("Alice".to_string()));
    assert_eq!(results[1][1], Value::Int(150));

    // Row 3: Bob, 200
    assert_eq!(results[2][0], Value::String("Bob".to_string()));
    assert_eq!(results[2][1], Value::Int(200));

    Ok(())
}

// =============================================================================
// NULL Handling Test
// =============================================================================

#[tokio::test]
async fn test_join_null_keys_no_match() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    // Create left table with NULL values
    let left_table = create_and_populate_table(
        "left_table",
        vec![("id", ColumnType::Int), ("value", ColumnType::String(100))],
        "id",
        vec![
            vec![Value::Int(1), Value::String("row1".to_string())],
            vec![Value::Null, Value::String("row_null".to_string())], // NULL key
            vec![Value::Int(3), Value::String("row3".to_string())],
        ],
        buffer_pool.clone(),
    )
    .await?;

    // Create right table with NULL values
    let right_table = create_and_populate_table(
        "right_table",
        vec![("id", ColumnType::Int), ("data", ColumnType::String(100))],
        "id",
        vec![
            vec![Value::Int(1), Value::String("data1".to_string())],
            vec![Value::Null, Value::String("data_null".to_string())], // NULL key
            vec![Value::Int(3), Value::String("data3".to_string())],
        ],
        buffer_pool.clone(),
    )
    .await?;

    let left_scan = ScanExecutor::new(left_table, buffer_pool.clone(), None);
    let right_scan = ScanExecutor::new(right_table, buffer_pool, None);

    let mut left_indices = HashMap::new();
    left_indices.insert("id".to_string(), 0);
    left_indices.insert("value".to_string(), 1);

    let mut right_indices = HashMap::new();
    right_indices.insert("id".to_string(), 0);
    right_indices.insert("data".to_string(), 1);

    // Join on id (NULL should not match NULL)
    let conditions = vec![JoinCondition {
        left_column: ColumnRef {
            table: Some("left_table".to_string()),
            column: "id".to_string(),
        },
        right_column: ColumnRef {
            table: Some("right_table".to_string()),
            column: "id".to_string(),
        },
    }];

    let output_columns = vec![
        OutputColumn {
            table: Some("left_table".to_string()),
            column: "value".to_string(),
            table_alias: "left_table".to_string(),
            column_index: 1,
        },
        OutputColumn {
            table: Some("right_table".to_string()),
            column: "data".to_string(),
            table_alias: "right_table".to_string(),
            column_index: 1,
        },
    ];

    let mut join_executor = JoinExecutor::new(JoinConfig {
        left_executor: Box::new(left_scan),
        right_executor: Box::new(right_scan),
        conditions,
        output_columns,
        left_column_indices: left_indices,
        right_column_indices: right_indices,
        output_sides: vec![JoinSide::Left, JoinSide::Right],
        right_table_name: "right_table".to_string(),
    });

    let results = collect_rows(&mut join_executor).await?;

    // NULL should not match NULL (SQL semantics)
    // Only id=1 and id=3 should match
    assert_eq!(results.len(), 2);

    assert_eq!(results[0][0], Value::String("row1".to_string()));
    assert_eq!(results[0][1], Value::String("data1".to_string()));

    assert_eq!(results[1][0], Value::String("row3".to_string()));
    assert_eq!(results[1][1], Value::String("data3".to_string()));

    Ok(())
}

// =============================================================================
// Empty Table Test
// =============================================================================

#[tokio::test]
async fn test_join_empty_right_table() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    // Create left table with data
    let left_table = create_and_populate_table(
        "left_table",
        vec![("id", ColumnType::Int), ("value", ColumnType::String(100))],
        "id",
        vec![
            vec![Value::Int(1), Value::String("row1".to_string())],
            vec![Value::Int(2), Value::String("row2".to_string())],
        ],
        buffer_pool.clone(),
    )
    .await?;

    // Create empty right table
    let right_table = create_and_populate_table(
        "right_table",
        vec![("id", ColumnType::Int), ("data", ColumnType::String(100))],
        "id",
        vec![], // Empty
        buffer_pool.clone(),
    )
    .await?;

    let left_scan = ScanExecutor::new(left_table, buffer_pool.clone(), None);
    let right_scan = ScanExecutor::new(right_table, buffer_pool, None);

    let mut left_indices = HashMap::new();
    left_indices.insert("id".to_string(), 0);

    let mut right_indices = HashMap::new();
    right_indices.insert("id".to_string(), 0);

    let conditions = vec![JoinCondition {
        left_column: ColumnRef {
            table: Some("left_table".to_string()),
            column: "id".to_string(),
        },
        right_column: ColumnRef {
            table: Some("right_table".to_string()),
            column: "id".to_string(),
        },
    }];

    let output_columns = vec![OutputColumn {
        table: Some("left_table".to_string()),
        column: "value".to_string(),
        table_alias: "left_table".to_string(),
        column_index: 1,
    }];

    let mut join_executor = JoinExecutor::new(JoinConfig {
        left_executor: Box::new(left_scan),
        right_executor: Box::new(right_scan),
        conditions,
        output_columns,
        left_column_indices: left_indices,
        right_column_indices: right_indices,
        output_sides: vec![JoinSide::Left],
        right_table_name: "right_table".to_string(),
    });

    let results = collect_rows(&mut join_executor).await?;

    // Empty right table means no matches for inner join
    assert_eq!(results.len(), 0);

    Ok(())
}

#[tokio::test]
async fn test_join_empty_left_table() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    // Create empty left table
    let left_table = create_and_populate_table(
        "left_table",
        vec![("id", ColumnType::Int), ("value", ColumnType::String(100))],
        "id",
        vec![], // Empty
        buffer_pool.clone(),
    )
    .await?;

    // Create right table with data
    let right_table = create_and_populate_table(
        "right_table",
        vec![("id", ColumnType::Int), ("data", ColumnType::String(100))],
        "id",
        vec![
            vec![Value::Int(1), Value::String("data1".to_string())],
            vec![Value::Int(2), Value::String("data2".to_string())],
        ],
        buffer_pool.clone(),
    )
    .await?;

    let left_scan = ScanExecutor::new(left_table, buffer_pool.clone(), None);
    let right_scan = ScanExecutor::new(right_table, buffer_pool, None);

    let mut left_indices = HashMap::new();
    left_indices.insert("id".to_string(), 0);

    let mut right_indices = HashMap::new();
    right_indices.insert("id".to_string(), 0);

    let conditions = vec![JoinCondition {
        left_column: ColumnRef {
            table: Some("left_table".to_string()),
            column: "id".to_string(),
        },
        right_column: ColumnRef {
            table: Some("right_table".to_string()),
            column: "id".to_string(),
        },
    }];

    let output_columns = vec![OutputColumn {
        table: Some("right_table".to_string()),
        column: "data".to_string(),
        table_alias: "right_table".to_string(),
        column_index: 1,
    }];

    let mut join_executor = JoinExecutor::new(JoinConfig {
        left_executor: Box::new(left_scan),
        right_executor: Box::new(right_scan),
        conditions,
        output_columns,
        left_column_indices: left_indices,
        right_column_indices: right_indices,
        output_sides: vec![JoinSide::Right],
        right_table_name: "right_table".to_string(),
    });

    let results = collect_rows(&mut join_executor).await?;

    // Empty left table means no rows to output
    assert_eq!(results.len(), 0);

    Ok(())
}

// =============================================================================
// Multi-Condition Test (AND combined ON conditions)
// =============================================================================

#[tokio::test]
async fn test_join_multiple_conditions_and() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    // Create left table: (id, dept_id, year, project_name)
    // Use 'id' as primary key to allow duplicate dept_id values
    let left_table = create_and_populate_table(
        "projects",
        vec![
            ("id", ColumnType::Int),
            ("dept_id", ColumnType::Int),
            ("year", ColumnType::Int),
            ("project_name", ColumnType::String(100)),
        ],
        "id",
        vec![
            vec![
                Value::Int(1),
                Value::Int(1),
                Value::Int(2024),
                Value::String("Project A".to_string()),
            ],
            vec![
                Value::Int(2),
                Value::Int(1),
                Value::Int(2023),
                Value::String("Project B".to_string()),
            ],
            vec![
                Value::Int(3),
                Value::Int(2),
                Value::Int(2024),
                Value::String("Project C".to_string()),
            ],
            vec![
                Value::Int(4),
                Value::Int(2),
                Value::Int(2023),
                Value::String("Project D".to_string()),
            ],
        ],
        buffer_pool.clone(),
    )
    .await?;

    // Create right table: (id, dept_id, year, budget)
    // Use 'id' as primary key to allow duplicate dept_id values
    let right_table = create_and_populate_table(
        "budgets",
        vec![
            ("id", ColumnType::Int),
            ("dept_id", ColumnType::Int),
            ("year", ColumnType::Int),
            ("budget", ColumnType::Int),
        ],
        "id",
        vec![
            vec![
                Value::Int(101),
                Value::Int(1),
                Value::Int(2024),
                Value::Int(100000),
            ],
            vec![
                Value::Int(102),
                Value::Int(1),
                Value::Int(2023),
                Value::Int(90000),
            ],
            vec![
                Value::Int(103),
                Value::Int(2),
                Value::Int(2024),
                Value::Int(120000),
            ],
            // Note: no budget for dept_id=2, year=2023
        ],
        buffer_pool.clone(),
    )
    .await?;

    let left_scan = ScanExecutor::new(left_table, buffer_pool.clone(), None);
    let right_scan = ScanExecutor::new(right_table, buffer_pool, None);

    let mut left_indices = HashMap::new();
    left_indices.insert("id".to_string(), 0);
    left_indices.insert("dept_id".to_string(), 1);
    left_indices.insert("year".to_string(), 2);
    left_indices.insert("project_name".to_string(), 3);

    let mut right_indices = HashMap::new();
    right_indices.insert("id".to_string(), 0);
    right_indices.insert("dept_id".to_string(), 1);
    right_indices.insert("year".to_string(), 2);
    right_indices.insert("budget".to_string(), 3);

    // Join on BOTH dept_id AND year
    let conditions = vec![
        JoinCondition {
            left_column: ColumnRef {
                table: Some("projects".to_string()),
                column: "dept_id".to_string(),
            },
            right_column: ColumnRef {
                table: Some("budgets".to_string()),
                column: "dept_id".to_string(),
            },
        },
        JoinCondition {
            left_column: ColumnRef {
                table: Some("projects".to_string()),
                column: "year".to_string(),
            },
            right_column: ColumnRef {
                table: Some("budgets".to_string()),
                column: "year".to_string(),
            },
        },
    ];

    let output_columns = vec![
        OutputColumn {
            table: Some("projects".to_string()),
            column: "project_name".to_string(),
            table_alias: "projects".to_string(),
            column_index: 3,
        },
        OutputColumn {
            table: Some("budgets".to_string()),
            column: "budget".to_string(),
            table_alias: "budgets".to_string(),
            column_index: 3,
        },
    ];

    let mut join_executor = JoinExecutor::new(JoinConfig {
        left_executor: Box::new(left_scan),
        right_executor: Box::new(right_scan),
        conditions,
        output_columns,
        left_column_indices: left_indices,
        right_column_indices: right_indices,
        output_sides: vec![JoinSide::Left, JoinSide::Right],
        right_table_name: "budgets".to_string(),
    });

    let results = collect_rows(&mut join_executor).await?;

    // Only 3 matches:
    // - (1, 2024) -> Project A, 100000
    // - (1, 2023) -> Project B, 90000
    // - (2, 2024) -> Project C, 120000
    // - (2, 2023) has no match (no budget for dept 2 year 2023)
    assert_eq!(results.len(), 3);

    // Verify the matches
    let project_names: Vec<String> = results
        .iter()
        .filter_map(|row| {
            if let Value::String(name) = &row[0] {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();

    assert!(project_names.contains(&"Project A".to_string()));
    assert!(project_names.contains(&"Project B".to_string()));
    assert!(project_names.contains(&"Project C".to_string()));
    assert!(!project_names.contains(&"Project D".to_string())); // No match

    Ok(())
}

// =============================================================================
// Edge Cases
// =============================================================================

#[tokio::test]
async fn test_join_no_matching_keys() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    // Create left table with keys 1, 2, 3
    let left_table = create_and_populate_table(
        "left_table",
        vec![("id", ColumnType::Int), ("value", ColumnType::String(100))],
        "id",
        vec![
            vec![Value::Int(1), Value::String("row1".to_string())],
            vec![Value::Int(2), Value::String("row2".to_string())],
            vec![Value::Int(3), Value::String("row3".to_string())],
        ],
        buffer_pool.clone(),
    )
    .await?;

    // Create right table with keys 4, 5, 6 (no overlap)
    let right_table = create_and_populate_table(
        "right_table",
        vec![("id", ColumnType::Int), ("data", ColumnType::String(100))],
        "id",
        vec![
            vec![Value::Int(4), Value::String("data4".to_string())],
            vec![Value::Int(5), Value::String("data5".to_string())],
            vec![Value::Int(6), Value::String("data6".to_string())],
        ],
        buffer_pool.clone(),
    )
    .await?;

    let left_scan = ScanExecutor::new(left_table, buffer_pool.clone(), None);
    let right_scan = ScanExecutor::new(right_table, buffer_pool, None);

    let mut left_indices = HashMap::new();
    left_indices.insert("id".to_string(), 0);

    let mut right_indices = HashMap::new();
    right_indices.insert("id".to_string(), 0);

    let conditions = vec![JoinCondition {
        left_column: ColumnRef {
            table: Some("left_table".to_string()),
            column: "id".to_string(),
        },
        right_column: ColumnRef {
            table: Some("right_table".to_string()),
            column: "id".to_string(),
        },
    }];

    let output_columns = vec![OutputColumn {
        table: Some("left_table".to_string()),
        column: "value".to_string(),
        table_alias: "left_table".to_string(),
        column_index: 1,
    }];

    let mut join_executor = JoinExecutor::new(JoinConfig {
        left_executor: Box::new(left_scan),
        right_executor: Box::new(right_scan),
        conditions,
        output_columns,
        left_column_indices: left_indices,
        right_column_indices: right_indices,
        output_sides: vec![JoinSide::Left],
        right_table_name: "right_table".to_string(),
    });

    let results = collect_rows(&mut join_executor).await?;

    // No matching keys means no results
    assert_eq!(results.len(), 0);

    Ok(())
}

#[tokio::test]
async fn test_join_one_to_many() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    // Create left table: one user
    let left_table = create_and_populate_table(
        "users",
        vec![("id", ColumnType::Int), ("name", ColumnType::String(100))],
        "id",
        vec![vec![Value::Int(1), Value::String("Alice".to_string())]],
        buffer_pool.clone(),
    )
    .await?;

    // Create right table: many orders for same user
    let right_table = create_and_populate_table(
        "orders",
        vec![
            ("id", ColumnType::Int),
            ("user_id", ColumnType::Int),
            ("amount", ColumnType::Int),
        ],
        "id",
        vec![
            vec![Value::Int(101), Value::Int(1), Value::Int(100)],
            vec![Value::Int(102), Value::Int(1), Value::Int(200)],
            vec![Value::Int(103), Value::Int(1), Value::Int(300)],
            vec![Value::Int(104), Value::Int(1), Value::Int(400)],
        ],
        buffer_pool.clone(),
    )
    .await?;

    let left_scan = ScanExecutor::new(left_table, buffer_pool.clone(), None);
    let right_scan = ScanExecutor::new(right_table, buffer_pool, None);

    let mut left_indices = HashMap::new();
    left_indices.insert("id".to_string(), 0);
    left_indices.insert("name".to_string(), 1);

    let mut right_indices = HashMap::new();
    right_indices.insert("id".to_string(), 0);
    right_indices.insert("user_id".to_string(), 1);
    right_indices.insert("amount".to_string(), 2);

    let conditions = vec![JoinCondition {
        left_column: ColumnRef {
            table: Some("users".to_string()),
            column: "id".to_string(),
        },
        right_column: ColumnRef {
            table: Some("orders".to_string()),
            column: "user_id".to_string(),
        },
    }];

    let output_columns = vec![
        OutputColumn {
            table: Some("users".to_string()),
            column: "name".to_string(),
            table_alias: "users".to_string(),
            column_index: 1,
        },
        OutputColumn {
            table: Some("orders".to_string()),
            column: "amount".to_string(),
            table_alias: "orders".to_string(),
            column_index: 2,
        },
    ];

    let mut join_executor = JoinExecutor::new(JoinConfig {
        left_executor: Box::new(left_scan),
        right_executor: Box::new(right_scan),
        conditions,
        output_columns,
        left_column_indices: left_indices,
        right_column_indices: right_indices,
        output_sides: vec![JoinSide::Left, JoinSide::Right],
        right_table_name: "orders".to_string(),
    });

    let results = collect_rows(&mut join_executor).await?;

    // One user matches 4 orders
    assert_eq!(results.len(), 4);

    // All rows should have the same user name
    for row in &results {
        assert_eq!(row[0], Value::String("Alice".to_string()));
    }

    // Verify all order amounts
    let amounts: Vec<i64> = results
        .iter()
        .filter_map(|row| {
            if let Value::Int(amount) = &row[1] {
                Some(*amount)
            } else {
                None
            }
        })
        .collect();

    assert_eq!(amounts, vec![100, 200, 300, 400]);

    Ok(())
}

// =============================================================================
// Keyless Base Table Test (T03-D1)
// =============================================================================

/// T03-D1（change 2026-10-20-ms30-semantic-binding-differential-net）：
/// 无键基表（首列 TEXT → 隐式主键不可键控，行只落数据页不入 PK 索引）作为
/// JOIN 基座时行完整可见——枚举通道回退数据页走查。
///
/// 现状（修复前）：`ScanExecutor` 只枚举 PK 索引，无键表索引为空 → 基座
/// 零行 → JOIN 结果零行。
#[tokio::test]
async fn test_join_keyless_base_tables_visible() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    // 左表首列 TEXT → 隐式主键不可键控 → 无键表
    let left_table = create_and_populate_table(
        "items_l",
        vec![("k", ColumnType::String(100)), ("a", ColumnType::Int)],
        "k",
        vec![
            vec![Value::String("a1".to_string()), Value::Int(1)],
            vec![Value::String("a2".to_string()), Value::Int(2)],
            vec![Value::String("a3".to_string()), Value::Int(3)],
        ],
        buffer_pool.clone(),
    )
    .await?;

    // 右表同为无键表
    let right_table = create_and_populate_table(
        "items_r",
        vec![("k", ColumnType::String(100)), ("b", ColumnType::Int)],
        "k",
        vec![
            vec![Value::String("b1".to_string()), Value::Int(1)],
            vec![Value::String("b2".to_string()), Value::Int(2)],
        ],
        buffer_pool.clone(),
    )
    .await?;

    let left_scan = ScanExecutor::new(left_table, buffer_pool.clone(), None);
    let right_scan = ScanExecutor::new(right_table, buffer_pool, None);

    let mut left_indices = HashMap::new();
    left_indices.insert("k".to_string(), 0);
    left_indices.insert("a".to_string(), 1);

    let mut right_indices = HashMap::new();
    right_indices.insert("k".to_string(), 0);
    right_indices.insert("b".to_string(), 1);

    let conditions = vec![JoinCondition {
        left_column: ColumnRef {
            table: Some("items_l".to_string()),
            column: "a".to_string(),
        },
        right_column: ColumnRef {
            table: Some("items_r".to_string()),
            column: "b".to_string(),
        },
    }];

    let output_columns = vec![
        OutputColumn {
            table: Some("items_l".to_string()),
            column: "k".to_string(),
            table_alias: "items_l".to_string(),
            column_index: 0,
        },
        OutputColumn {
            table: Some("items_r".to_string()),
            column: "b".to_string(),
            table_alias: "items_r".to_string(),
            column_index: 1,
        },
    ];

    let mut join_executor = JoinExecutor::new(JoinConfig {
        left_executor: Box::new(left_scan),
        right_executor: Box::new(right_scan),
        conditions,
        output_columns,
        left_column_indices: left_indices,
        right_column_indices: right_indices,
        output_sides: vec![JoinSide::Left, JoinSide::Right],
        right_table_name: "items_r".to_string(),
    });

    let results = collect_rows(&mut join_executor).await?;

    // 无键基座三行全部可见，与键表基座的 JOIN 语义同形：a=1 与 a=2 各命中一行
    assert_eq!(results.len(), 2);
    assert_eq!(results[0][0], Value::String("a1".to_string()));
    assert_eq!(results[0][1], Value::Int(1));
    assert_eq!(results[1][0], Value::String("a2".to_string()));
    assert_eq!(results[1][1], Value::Int(2));

    Ok(())
}

// =============================================================================
// Statement-level error surface (MS32 Iter001 Task 2.1)
// =============================================================================

/// 等值哈希 JOIN 的 WHERE 谓词在左侧行 id=2 上 `abs(i64::MIN)` 溢出，行 id=1
/// 先通过谓词。语句级结果必须是 `Err`，不得作为只含成功行的 `Ok` 交出部分行。
/// i64::MIN 无法以 SQL 字面量写入，按 scalar_function_test 先例用
/// `0 - 9223372036854775807 - 1` 构造。JOIN 上的表达式投影是计划期点名拒绝
/// 的既有边界（不放宽），见证放在 WHERE 谓词面。
#[tokio::test]
async fn execute_sql_returns_err_not_partial_rows_on_join_where_overflow() -> Result<()> {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("join_overflow.db"))
        .await
        .unwrap();

    db.execute_sql("CREATE TABLE l (id INT PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE r (id INT PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO l VALUES (1), (2)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO r VALUES (1), (2)")
        .await
        .unwrap();

    let error = db
        .execute_sql(
            "SELECT l.id FROM l JOIN r ON l.id = r.id \
             WHERE abs(CASE WHEN l.id = 2 THEN 0 - 9223372036854775807 - 1 ELSE l.id END) > 0",
        )
        .await
        .expect_err("predicate overflow must fail the whole statement");
    assert!(
        error.to_string().contains("Integer overflow in abs"),
        "unexpected error: {error}"
    );

    Ok(())
}
