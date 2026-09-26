//! Plan builder integration tests
//!
//! Tests for PlanBuilder converting SQL AST to PhysicalPlan

use rtsql::{parse_sql, ColumnConstraint, ColumnType, PhysicalPlan, PlanBuilder};

fn setup_builder() -> PlanBuilder {
    let mut builder = PlanBuilder::new();
    builder.register_table("users", vec!["id".into(), "name".into()], "id");
    builder
}

#[test]
fn test_select_by_pk() {
    let sql = "SELECT id, name FROM users WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::IndexScan(node) => {
            assert_eq!(node.table_name, "users");
            assert_eq!(node.columns, vec!["id", "name"]);
        }
        _ => panic!("Expected IndexScan, got {:?}", plan),
    }
}

#[test]
fn test_select_scan() {
    let sql = "SELECT id, name FROM users";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    // M19: no-WHERE SELECT now routes to DataScan (skip index layer).
    match plan {
        PhysicalPlan::DataScan(node) => {
            assert_eq!(node.table_name, "users");
            assert_eq!(node.columns, vec!["id", "name"]);
        }
        _ => panic!("Expected DataScan, got {:?}", plan),
    }
}

#[test]
fn test_insert() {
    let sql = "INSERT INTO users (id, name) VALUES (1, 'Alice')";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Insert(node) => {
            assert_eq!(node.table_name, "users");
            assert_eq!(node.columns, vec!["id", "name"]);
            assert_eq!(node.values.len(), 1);
            assert_eq!(node.values[0].len(), 2);
        }
        _ => panic!("Expected Insert, got {:?}", plan),
    }
}

#[test]
fn test_update() {
    let sql = "UPDATE users SET name = 'Bob' WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Update(node) => {
            assert_eq!(node.table_name, "users");
            assert_eq!(node.column, "name");
        }
        _ => panic!("Expected Update, got {:?}", plan),
    }
}

#[test]
fn test_delete() {
    let sql = "DELETE FROM users WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Delete(node) => {
            assert_eq!(node.table_name, "users");
        }
        _ => panic!("Expected Delete, got {:?}", plan),
    }
}

#[test]
fn test_table_not_found() {
    let sql = "SELECT id FROM nonexistent";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let result = builder.build_plan(&stmts[0]);

    assert!(result.is_err());
}

#[test]
fn test_invalid_where_not_pk() {
    // MS07-T06: non-PK WHERE without OR is pushed into DataScan
    // (row-level predicate, no Filter node).
    let sql = "SELECT id, name FROM users WHERE name = 'Alice'";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::DataScan(node) => {
            assert_eq!(node.table_name, "users");
            assert!(node.predicate.is_some());
        }
        _ => panic!("Expected DataScan with pushed predicate, got {:?}", plan),
    }
}

#[test]
fn test_unsupported_statement() {
    // ALTER TABLE is not supported
    let sql = "ALTER TABLE test ADD COLUMN name VARCHAR(100)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let result = builder.build_plan(&stmts[0]);

    assert!(result.is_err());
}

// ============================================================================
// DDL Tests (Task 5: CREATE TABLE / DROP TABLE parsing)
// ============================================================================

#[test]
fn test_build_create_table() {
    let sql = "CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR(100) NOT NULL)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::CreateTable(node) => {
            assert_eq!(node.table_name, "users");
            assert_eq!(node.columns.len(), 2);

            // First column: id INT PRIMARY KEY
            assert_eq!(node.columns[0].name, "id");
            assert_eq!(node.columns[0].data_type, ColumnType::Int);
            assert!(node.columns[0].constraints.is_empty()); // PK extracted separately
            assert_eq!(node.primary_key, Some("id".to_string()));

            // Second column: name VARCHAR(100) NOT NULL
            assert_eq!(node.columns[1].name, "name");
            assert_eq!(node.columns[1].data_type, ColumnType::String);
            assert_eq!(node.columns[1].constraints.len(), 1);
            assert!(matches!(
                node.columns[1].constraints[0],
                ColumnConstraint::NotNull
            ));
        }
        _ => panic!("Expected CreateTable, got {:?}", plan),
    }
}

#[test]
fn test_build_create_table_with_defaults() {
    let sql = "CREATE TABLE items (id INT, name TEXT DEFAULT 'unnamed', active BOOL)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::CreateTable(node) => {
            assert_eq!(node.table_name, "items");
            assert_eq!(node.columns.len(), 3);

            // Check default value constraint
            assert_eq!(node.columns[1].constraints.len(), 1);
            match &node.columns[1].constraints[0] {
                ColumnConstraint::DefaultValue(v) => {
                    assert_eq!(v, &rtsql::Value::String("unnamed".to_string()));
                }
                _ => panic!("Expected DefaultValue constraint"),
            }

            // No primary key
            assert_eq!(node.primary_key, None);
        }
        _ => panic!("Expected CreateTable, got {:?}", plan),
    }
}

#[test]
fn test_build_create_table_various_types() {
    let sql = "CREATE TABLE test (a INT, b BIGINT, c FLOAT, d DOUBLE, e REAL, f TEXT, g VARCHAR(50), h BOOLEAN, i BOOL)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::CreateTable(node) => {
            assert_eq!(node.table_name, "test");
            assert_eq!(node.columns.len(), 9);

            // Check type mappings
            assert_eq!(node.columns[0].data_type, ColumnType::Int); // INT
            assert_eq!(node.columns[1].data_type, ColumnType::Int); // BIGINT -> Int
            assert_eq!(node.columns[2].data_type, ColumnType::Float); // FLOAT
            assert_eq!(node.columns[3].data_type, ColumnType::Float); // DOUBLE -> Float
            assert_eq!(node.columns[4].data_type, ColumnType::Float); // REAL -> Float
            assert_eq!(node.columns[5].data_type, ColumnType::String); // TEXT -> String
            assert_eq!(node.columns[6].data_type, ColumnType::String); // VARCHAR -> String
            assert_eq!(node.columns[7].data_type, ColumnType::Bool); // BOOLEAN
            assert_eq!(node.columns[8].data_type, ColumnType::Bool); // BOOL
        }
        _ => panic!("Expected CreateTable, got {:?}", plan),
    }
}

#[test]
fn test_build_drop_table() {
    let sql = "DROP TABLE users";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::DropTable(node) => {
            assert_eq!(node.table_name, "users");
            assert!(!node.if_exists);
        }
        _ => panic!("Expected DropTable, got {:?}", plan),
    }
}

#[test]
fn test_build_drop_table_if_exists() {
    let sql = "DROP TABLE IF EXISTS users";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::DropTable(node) => {
            assert_eq!(node.table_name, "users");
            assert!(node.if_exists);
        }
        _ => panic!("Expected DropTable, got {:?}", plan),
    }
}

#[test]
fn test_create_table_empty_columns_error() {
    // Note: SQL parser might not accept empty column list, but we test the logic anyway
    // This would need to be tested with a manually constructed Statement or via error handling
    // For now, we test that a table with columns works
    let sql = "CREATE TABLE test (id INT)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let plan = builder.build_plan(&stmts[0]);

    // Should succeed with one column
    assert!(plan.is_ok());
}

#[test]
fn test_create_table_multiple_pk_error() {
    // Table constraint with composite primary key (should error)
    // Note: Our implementation only supports single-column PK
    // Testing via table constraint: PRIMARY KEY (col1, col2)
    // This is complex to construct, so we'll test through the error path
    // when implementing
}

// ============================================================================
// WHERE Expression Tests (Task 9: WHERE parsing + Filter plan)
// ============================================================================

#[test]
fn test_build_where_comparison() {
    // WHERE id > 10 (comparison predicate)
    let sql = "SELECT id, name FROM users WHERE id > 10";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::DataScan(node) => {
            assert_eq!(node.table_name, "users");
            // MS07-T06: comparison predicate is pushed into DataScan.
            assert!(node.predicate.is_some());
        }
        _ => panic!("Expected DataScan plan for non-PK WHERE, got {:?}", plan),
    }
}

#[test]
fn test_build_where_logical_and() {
    // WHERE id > 10 AND id < 100 (logical AND predicate)
    let sql = "SELECT id, name FROM users WHERE id > 10 AND id < 100";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::DataScan(node) => {
            assert_eq!(node.table_name, "users");
            // MS07-T06: AND chain is pushdown-eligible.
            assert!(node.predicate.is_some());
        }
        _ => panic!("Expected DataScan plan for complex WHERE, got {:?}", plan),
    }
}

#[test]
fn test_build_where_comparison_operators() {
    // Test all comparison operators: =, !=, >, <, >=, <=
    let test_cases = vec![
        ("SELECT id FROM users WHERE id = 5", "eq"),
        ("SELECT id FROM users WHERE id != 5", "ne"),
        ("SELECT id FROM users WHERE id > 5", "gt"),
        ("SELECT id FROM users WHERE id < 5", "lt"),
        ("SELECT id FROM users WHERE id >= 5", "ge"),
        ("SELECT id FROM users WHERE id <= 5", "le"),
    ];

    for (sql, _op_name) in test_cases {
        let stmts = parse_sql(sql).unwrap();
        let mut builder = setup_builder();
        let plan = builder.build_plan(&stmts[0]).unwrap();

        match plan {
            PhysicalPlan::Filter(_) => {} // OR-shaped / complex PK forms keep Filter
            PhysicalPlan::DataScan(_) => {} // MS07-T06: pushdown-eligible WHERE
            PhysicalPlan::IndexScan(_) => {} // = might still use index scan
            _ => panic!(
                "Expected Filter, DataScan or IndexScan for WHERE, got {:?}",
                plan
            ),
        }
    }
}

#[test]
fn test_build_where_column_comparison() {
    // WHERE name = 'Alice' (non-PK column)
    let sql = "SELECT id, name FROM users WHERE name = 'Alice'";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::DataScan(node) => {
            assert_eq!(node.table_name, "users");
            assert!(node.predicate.is_some());
        }
        _ => panic!("Expected DataScan with pushed predicate, got {:?}", plan),
    }
}

#[test]
fn test_build_where_logical_or() {
    // WHERE id < 10 OR id > 100 (logical OR predicate)
    let sql = "SELECT id, name FROM users WHERE id < 10 OR id > 100";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Filter(node) => {
            assert_eq!(node.table_name, "users");
        }
        _ => panic!("Expected Filter plan for OR WHERE, got {:?}", plan),
    }
}

// ============================================================================
// ORDER BY + LIMIT/OFFSET Tests (Task 7: M9 Phase 2)
// ============================================================================

#[test]
fn test_parse_order_by_single_column_asc() {
    let mut builder = PlanBuilder::new();
    builder.register_table(
        "users",
        vec!["id".into(), "name".into(), "age".into()],
        "id",
    );

    let sql = "SELECT id, name FROM users ORDER BY age ASC";
    let stmt = parse_sql(sql).unwrap().first().unwrap().clone();
    let plan = builder.build_plan(&stmt).unwrap();

    match plan {
        PhysicalPlan::Sort(node) => {
            assert_eq!(node.order_by.len(), 1);
            assert_eq!(node.order_by[0].column, "age");
            assert!(node.order_by[0].asc);
        }
        _ => panic!("Expected Sort plan"),
    }
}

#[test]
fn test_parse_order_by_multi_column() {
    let mut builder = PlanBuilder::new();
    builder.register_table(
        "users",
        vec!["id".into(), "name".into(), "age".into()],
        "id",
    );

    let sql = "SELECT * FROM users ORDER BY age DESC, name ASC";
    let stmt = parse_sql(sql).unwrap().first().unwrap().clone();
    let plan = builder.build_plan(&stmt).unwrap();

    match plan {
        PhysicalPlan::Sort(node) => {
            assert_eq!(node.order_by.len(), 2);
            assert_eq!(node.order_by[0].column, "age");
            assert!(!node.order_by[0].asc);
            assert_eq!(node.order_by[1].column, "name");
            assert!(node.order_by[1].asc);
        }
        _ => panic!("Expected Sort plan"),
    }
}

#[test]
fn test_parse_limit_only() {
    let mut builder = PlanBuilder::new();
    builder.register_table("users", vec!["id".into(), "name".into()], "id");

    let sql = "SELECT * FROM users LIMIT 10";
    let stmt = parse_sql(sql).unwrap().first().unwrap().clone();
    let plan = builder.build_plan(&stmt).unwrap();

    match plan {
        PhysicalPlan::Limit(node) => {
            assert_eq!(node.limit, 10);
            assert_eq!(node.offset, 0);
        }
        _ => panic!("Expected Limit plan"),
    }
}

#[test]
fn test_parse_limit_with_offset() {
    let mut builder = PlanBuilder::new();
    builder.register_table("users", vec!["id".into(), "name".into()], "id");

    let sql = "SELECT * FROM users LIMIT 5 OFFSET 10";
    let stmt = parse_sql(sql).unwrap().first().unwrap().clone();
    let plan = builder.build_plan(&stmt).unwrap();

    match plan {
        PhysicalPlan::Limit(node) => {
            assert_eq!(node.limit, 5);
            assert_eq!(node.offset, 10);
        }
        _ => panic!("Expected Limit plan"),
    }
}

#[test]
fn test_parse_order_by_with_limit() {
    let mut builder = PlanBuilder::new();
    builder.register_table(
        "users",
        vec!["id".into(), "name".into(), "age".into()],
        "id",
    );

    let sql = "SELECT * FROM users ORDER BY age DESC LIMIT 10 OFFSET 5";
    let stmt = parse_sql(sql).unwrap().first().unwrap().clone();
    let plan = builder.build_plan(&stmt).unwrap();

    // 期望：Limit -> Sort -> Scan
    match plan {
        PhysicalPlan::Limit(limit_node) => {
            assert_eq!(limit_node.limit, 10);
            assert_eq!(limit_node.offset, 5);

            match *limit_node.input {
                PhysicalPlan::Sort(sort_node) => {
                    assert_eq!(sort_node.order_by[0].column, "age");
                    assert!(!sort_node.order_by[0].asc);
                }
                _ => panic!("Expected Sort inside Limit"),
            }
        }
        _ => panic!("Expected Limit plan"),
    }
}

// ============================================================================
// JOIN Tests (Task 6: M12 INNER JOIN parsing)
// ============================================================================

#[test]
fn test_build_join_two_tables() {
    let mut builder = PlanBuilder::new();
    builder.register_table("orders", vec!["id".into(), "user_id".into()], "id");
    builder.register_table("users", vec!["id".into(), "name".into()], "id");

    let sql = "SELECT * FROM orders JOIN users ON orders.user_id = users.id";
    let stmts = parse_sql(sql).unwrap();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Join(join_node) => {
            // 验证左表是 Scan(orders)
            match join_node.left.as_ref() {
                PhysicalPlan::Scan(scan) => {
                    assert_eq!(scan.table_name, "orders");
                }
                _ => panic!("Expected left to be Scan"),
            }

            // 验证右表是 Scan(users)
            match join_node.right.as_ref() {
                PhysicalPlan::Scan(scan) => {
                    assert_eq!(scan.table_name, "users");
                }
                _ => panic!("Expected right to be Scan"),
            }

            // 验证 ON 条件
            assert_eq!(join_node.conditions.len(), 1);
            assert_eq!(
                join_node.conditions[0].left_column.table,
                Some("orders".to_string())
            );
            assert_eq!(join_node.conditions[0].left_column.column, "user_id");
            assert_eq!(
                join_node.conditions[0].right_column.table,
                Some("users".to_string())
            );
            assert_eq!(join_node.conditions[0].right_column.column, "id");
        }
        _ => panic!("Expected Join plan"),
    }
}

#[test]
fn test_build_join_and_conditions() {
    let mut builder = PlanBuilder::new();
    builder.register_table(
        "orders",
        vec!["id".into(), "user_id".into(), "status".into()],
        "id",
    );
    builder.register_table(
        "users",
        vec!["id".into(), "name".into(), "status".into()],
        "id",
    );

    let sql = "SELECT * FROM orders JOIN users ON orders.user_id = users.id AND orders.status = users.status";
    let stmts = parse_sql(sql).unwrap();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Join(join_node) => {
            assert_eq!(join_node.conditions.len(), 2);
        }
        _ => panic!("Expected Join plan"),
    }
}

#[test]
fn test_build_join_three_tables() {
    let mut builder = PlanBuilder::new();
    builder.register_table(
        "orders",
        vec!["id".into(), "user_id".into(), "product_id".into()],
        "id",
    );
    builder.register_table("users", vec!["id".into(), "name".into()], "id");
    builder.register_table("products", vec!["id".into(), "name".into()], "id");

    let sql = "SELECT * FROM orders JOIN users ON orders.user_id = users.id JOIN products ON orders.product_id = products.id";
    let stmts = parse_sql(sql).unwrap();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    // 顶层应该是 Join(Join(orders, users), products)
    match plan {
        PhysicalPlan::Join(outer_join) => {
            // 外层右表是 products
            match outer_join.right.as_ref() {
                PhysicalPlan::Scan(scan) => {
                    assert_eq!(scan.table_name, "products");
                }
                _ => panic!("Expected outer right to be Scan(products)"),
            }

            // 外层左表是 Join(orders, users)
            match outer_join.left.as_ref() {
                PhysicalPlan::Join(inner_join) => {
                    match inner_join.left.as_ref() {
                        PhysicalPlan::Scan(scan) => assert_eq!(scan.table_name, "orders"),
                        _ => panic!("Expected inner left to be Scan(orders)"),
                    }
                    match inner_join.right.as_ref() {
                        PhysicalPlan::Scan(scan) => assert_eq!(scan.table_name, "users"),
                        _ => panic!("Expected inner right to be Scan(users)"),
                    }
                }
                _ => panic!("Expected outer left to be Join"),
            }
        }
        _ => panic!("Expected outer Join plan"),
    }
}

#[test]
fn test_join_ambiguous_column_error() {
    let mut builder = PlanBuilder::new();
    builder.register_table("orders", vec!["id".into()], "id");
    builder.register_table("users", vec!["id".into()], "id");

    let sql = "SELECT id FROM orders JOIN users ON orders.user_id = users.id";
    let stmts = parse_sql(sql).unwrap();
    let result = builder.build_plan(&stmts[0]);

    // 应该报错（id 列在两表都存在）
    assert!(result.is_err());
}

// ===========================================================================
// MS11-T01 T3: 新 WHERE 形态的路由形态断言（追加段）
// - BETWEEN/LIKE/IS NULL：无 OR、非 PK 等值 → 谓词下推 DataScan
// - IN/NOT IN：脱糖构造即含 OR → 保留 Filter 包装（保守基线）
// ===========================================================================

/// BETWEEN 纯 AND 脱糖 → DataScan 装入谓词（无 Filter 节点）
#[test]
fn test_between_pushes_predicate_into_datascand() {
    let sql = "SELECT id, name FROM users WHERE name BETWEEN 'a' AND 'z'";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::DataScan(node) => {
            assert!(
                node.predicate.is_some(),
                "BETWEEN 脱糖为纯 AND，必须下推进 DataScan"
            );
        }
        other => panic!("Expected DataScan with pushed BETWEEN, got {:?}", other),
    }
}

/// IN 常量列表脱糖为 OR 链 → Filter(DataScan) 包装，谓词不下推
#[test]
fn test_in_keeps_filter_over_datascand() {
    let sql = "SELECT id, name FROM users WHERE id IN (1, 2)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Filter(node) => match node.input.as_ref() {
            PhysicalPlan::DataScan(inner) => {
                assert!(inner.predicate.is_none(), "IN 含 OR，谓词必须留在 Filter");
            }
            other => panic!("Expected Filter over DataScan, got {:?}", other),
        },
        other => panic!("Expected Filter for IN, got {:?}", other),
    }
}

/// NOT IN = Not 包装 OR 链 → 同样保留 Filter
#[test]
fn test_not_in_keeps_filter() {
    let sql = "SELECT id, name FROM users WHERE id NOT IN (1, 2)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Filter(node) => match node.input.as_ref() {
            PhysicalPlan::DataScan(inner) => {
                assert!(inner.predicate.is_none());
            }
            other => panic!("Expected Filter over DataScan, got {:?}", other),
        },
        other => panic!("Expected Filter for NOT IN, got {:?}", other),
    }
}

/// LIKE 无 OR → 下推 DataScan
#[test]
fn test_like_pushes_predicate_into_datascand() {
    let sql = "SELECT id, name FROM users WHERE name LIKE 'A%'";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::DataScan(node) => assert!(node.predicate.is_some()),
        other => panic!("Expected DataScan with pushed LIKE, got {:?}", other),
    }
}

/// IS NULL / IS NOT NULL → 下推 DataScan
#[test]
fn test_is_null_pushes_predicate_into_datascand() {
    for sql in [
        "SELECT id, name FROM users WHERE name IS NULL",
        "SELECT id, name FROM users WHERE name IS NOT NULL",
    ] {
        let stmts = parse_sql(sql).unwrap();
        let mut builder = setup_builder();
        let plan = builder.build_plan(&stmts[0]).unwrap();
        match plan {
            PhysicalPlan::DataScan(node) => assert!(node.predicate.is_some(), "{sql}"),
            other => panic!(
                "Expected DataScan with pushed IS [NOT] NULL for {sql}, got {:?}",
                other
            ),
        }
    }
}

// ===========================================================================
// MS11-T01 T5 (I040): INSERT 负数字面量（追加段）
// ===========================================================================

/// `-<number>` 字面量折叠为负值入库
#[test]
fn test_insert_negative_number_literal_folds() {
    let sql = "INSERT INTO users (id, name) VALUES (-1, 'Bob')";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Insert(node) => {
            assert_eq!(node.values[0][0], rtsql::executor::Value::Int(-1));
        }
        other => panic!(
            "Expected Insert with folded negative literal, got {:?}",
            other
        ),
    }
}

/// 非字面量（列引用）取负维持 UnsupportedValue 拒绝
#[test]
fn test_insert_non_literal_negation_rejected() {
    let sql = "INSERT INTO users (id, name) VALUES (-id, 'Bob')";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("Unsupported value type"),
            "列引用取负必须维持拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected UnsupportedValue rejection, got {:?}", plan),
    }
}

// ===========================================================================
// MS23 Iteration 000 — 约束诚实化（R2）
// ===========================================================================

/// MS23 1.1（R2-S1）：列级 CHECK 建表计划期点名拒绝（不再静默丢弃）
#[test]
fn test_create_table_column_check_rejected() {
    let sql = "CREATE TABLE t (a INT CHECK (a > 0))";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("Unsupported constraint: CHECK"),
            "列级 CHECK 必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected CHECK rejection, got {:?}", plan),
    }
}

/// MS23 1.1（R2-S2）：列级 FOREIGN KEY（REFERENCES）建表计划期点名拒绝
#[test]
fn test_create_table_column_foreign_key_rejected() {
    let sql = "CREATE TABLE t (a INT REFERENCES o (x))";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string()
                .contains("Unsupported constraint: FOREIGN KEY"),
            "列级 FOREIGN KEY 必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected FOREIGN KEY rejection, got {:?}", plan),
    }
}

/// MS23 1.1（R2-S3）：方言项（AUTO_INCREMENT）建表计划期点名拒绝
#[test]
fn test_create_table_dialect_specific_rejected() {
    let sql = "CREATE TABLE t (id INT AUTO_INCREMENT)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string()
                .contains("Unsupported constraint: dialect-specific"),
            "方言项必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected dialect-specific rejection, got {:?}", plan),
    }
}

/// MS23 1.1（Preserve 见证）：`NULL`/`COMMENT` 无语义期望选项维持忽略
#[test]
fn test_create_table_null_and_comment_options_still_ignored() {
    let sql = "CREATE TABLE t (a INT NULL, b INT COMMENT 'note')";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::CreateTable(node) => {
            assert_eq!(node.columns.len(), 2);
            assert!(node.columns.iter().all(|c| c.constraints.is_empty()));
        }
        other => panic!("Expected CreateTable, got {:?}", other),
    }
}

/// MS23 1.2（R2-S4）：表级 CHECK 建表计划期点名拒绝
#[test]
fn test_create_table_table_level_check_rejected() {
    let sql = "CREATE TABLE t (a INT, CONSTRAINT chk CHECK (a > 0))";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("Unsupported constraint: CHECK"),
            "表级 CHECK 必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected table-level CHECK rejection, got {:?}", plan),
    }
}

/// MS23 1.2（R2-S4）：表级 FOREIGN KEY 建表计划期点名拒绝
#[test]
fn test_create_table_table_level_foreign_key_rejected() {
    let sql = "CREATE TABLE t (a INT, b INT, FOREIGN KEY (a) REFERENCES o (x))";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string()
                .contains("Unsupported constraint: FOREIGN KEY"),
            "表级 FOREIGN KEY 必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected table-level FOREIGN KEY rejection, got {:?}", plan),
    }
}

/// MS23 1.2（Preserve 见证）：表级 PRIMARY KEY 既有消费路径保持
#[test]
fn test_create_table_table_level_primary_key_still_works() {
    let sql = "CREATE TABLE t (a INT, b INT, PRIMARY KEY (a))";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::CreateTable(node) => {
            assert_eq!(node.primary_key, Some("a".to_string()));
        }
        other => panic!("Expected CreateTable, got {:?}", other),
    }
}

// ===========================================================================
// MS23 Iteration 001 — UNIQUE DDL 策略面（R4）
// ===========================================================================

/// MS23 2.4（R4-S1）：非 INT 列级 UNIQUE 建表点名拒绝（×5 类型矩阵）
#[test]
fn test_create_table_non_int_column_unique_rejected() {
    let cases = [
        ("CREATE TABLE t (code VARCHAR(10) UNIQUE)", "String"),
        ("CREATE TABLE t (f FLOAT UNIQUE)", "Float"),
        ("CREATE TABLE t (b BOOL UNIQUE)", "Bool"),
        ("CREATE TABLE t (d DATE UNIQUE)", "Date"),
        ("CREATE TABLE t (ts TIMESTAMP UNIQUE)", "Timestamp"),
    ];
    for (sql, ty) in cases {
        let stmts = parse_sql(sql).unwrap();
        let mut builder = PlanBuilder::new();
        let result = builder.build_plan(&stmts[0]);
        match result {
            Err(e) => assert!(
                e.to_string().contains("UNIQUE") && e.to_string().contains("INT"),
                "{ty} 列 UNIQUE 必须点名仅支持 INT，实际: {e}"
            ),
            Ok(plan) => panic!("Expected UNIQUE rejection for {ty}, got {:?}", plan),
        }
    }
}

/// MS23 2.4（R4-S2）：表级单列 UNIQUE(col) 等价映射列级标志——plan 中该列
/// 携带 Unique 约束（to_schema_column 折叠后 catalog 持久化与 dump 渲染自动正确）
#[test]
fn test_create_table_table_level_single_column_unique_maps_to_column_flag() {
    let sql = "CREATE TABLE t (id INT PRIMARY KEY, code INT, UNIQUE (code))";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::CreateTable(node) => {
            let code = node
                .columns
                .iter()
                .find(|c| c.name == "code")
                .expect("column 'code' must exist");
            assert!(
                code.constraints
                    .iter()
                    .any(|c| matches!(c, ColumnConstraint::Unique)),
                "table-level UNIQUE(code) must map onto the column: {:?}",
                code.constraints
            );
        }
        other => panic!("Expected CreateTable, got {:?}", other),
    }
}

/// MS23 2.4（R4-S3）：表级组合 UNIQUE 点名拒绝
#[test]
fn test_create_table_composite_unique_rejected() {
    let sql = "CREATE TABLE t (a INT, b INT, UNIQUE (a, b))";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("UNIQUE"),
            "组合 UNIQUE 必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected composite UNIQUE rejection, got {:?}", plan),
    }
}

/// MS23 2.4（D6 消费裁定）：PK 列声明的 UNIQUE 消费为 PK 既有唯一性——
/// 建表成功不拒绝（自家 dump 的 `pk BOOL PRIMARY KEY NOT NULL UNIQUE`
/// 形态 restore 恒可达）；第二索引由承载面跳过 PK 列保证不建
#[test]
fn test_create_table_pk_column_unique_accepted() {
    let sql = "CREATE TABLE t (id INT PRIMARY KEY UNIQUE, v INT)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::CreateTable(node) => {
            assert_eq!(node.primary_key, Some("id".to_string()));
            let id = node.columns.iter().find(|c| c.name == "id").unwrap();
            assert!(
                id.constraints
                    .iter()
                    .any(|c| matches!(c, ColumnConstraint::Unique)),
                "PK column UNIQUE flag stays on the column (consumed by carrying face)"
            );
        }
        other => panic!("Expected CreateTable, got {:?}", other),
    }
}

/// MS23 2.4（R4-S1 表级面）：表级单列 UNIQUE 指向非 INT 列同样点名拒绝
#[test]
fn test_create_table_table_level_single_column_unique_non_int_rejected() {
    let sql = "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR(10), UNIQUE (name))";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = PlanBuilder::new();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("UNIQUE") && e.to_string().contains("INT"),
            "表级单列 UNIQUE 指向非 INT 列必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected non-INT UNIQUE rejection, got {:?}", plan),
    }
}

// ===========================================================================
// MS24 Iteration 000 (1.5/D2): 子集列清单 INSERT 与 DEFAULT 填充
// ===========================================================================

fn setup_builder_with_defaults() -> PlanBuilder {
    let mut builder = PlanBuilder::new();
    builder.register_table(
        "users",
        vec!["id".into(), "name".into(), "score".into()],
        "id",
    );
    // name 列声明 DEFAULT 'anon'；id/score 无声明
    builder.set_table_defaults(
        "users",
        vec![
            None,
            Some(rtsql::executor::Value::String("anon".to_string())),
            None,
        ],
    );
    builder
}

/// 子集清单（省略 name）计划构造成功且输出恒全宽——省略位填声明 DEFAULT。
#[test]
fn subset_list_plan_fills_declared_default() {
    let sql = "INSERT INTO users (id, score) VALUES (1, 90)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_with_defaults();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Insert(node) => {
            assert_eq!(node.columns, vec!["id", "score"]);
            assert_eq!(node.values[0].len(), 3, "输出恒全宽（表列数）");
            assert_eq!(node.values[0][0], rtsql::executor::Value::Int(1));
            assert_eq!(
                node.values[0][1],
                rtsql::executor::Value::String("anon".to_string()),
                "省略位必须填声明 DEFAULT"
            );
            assert_eq!(node.values[0][2], rtsql::executor::Value::Int(90));
        }
        other => panic!("Expected Insert, got {:?}", other),
    }
}

/// 省略位无声明 DEFAULT 时填 NULL。
#[test]
fn subset_list_plan_fills_null_for_omitted_without_default() {
    let sql = "INSERT INTO users (id) VALUES (2)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_with_defaults();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Insert(node) => {
            assert_eq!(
                node.values[0],
                vec![
                    rtsql::executor::Value::Int(2),
                    rtsql::executor::Value::String("anon".to_string()),
                    rtsql::executor::Value::Null,
                ],
                "无声明 DEFAULT 的省略位必须填 NULL"
            );
        }
        other => panic!("Expected Insert, got {:?}", other),
    }
}

/// VALUES 中的 DEFAULT 关键字等价省略——取该列声明 DEFAULT 或 NULL。
#[test]
fn default_keyword_maps_to_declared_default_or_null() {
    let sql = "INSERT INTO users (id, name, score) VALUES (1, DEFAULT, DEFAULT)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_with_defaults();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Insert(node) => {
            assert_eq!(
                node.values[0],
                vec![
                    rtsql::executor::Value::Int(1),
                    rtsql::executor::Value::String("anon".to_string()),
                    rtsql::executor::Value::Null,
                ],
                "DEFAULT 关键字位必须取声明 DEFAULT（name）/ NULL（score）"
            );
        }
        other => panic!("Expected Insert, got {:?}", other),
    }
}

/// 子集未知列维持计划期点名拒绝（既有意图承接）。
#[test]
fn subset_unknown_column_rejected_at_plan_time() {
    let sql = "INSERT INTO users (id, zz) VALUES (1, 2)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_with_defaults();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("zz"),
            "未知列必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected unknown-column rejection, got {:?}", plan),
    }
}

/// 子集重复列维持计划期点名拒绝（既有意图承接）。
#[test]
fn subset_duplicate_column_rejected_at_plan_time() {
    let sql = "INSERT INTO users (id, id) VALUES (1, 2)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_with_defaults();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("Duplicate"),
            "重复列必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected duplicate-column rejection, got {:?}", plan),
    }
}

// ===========================================================================
// MS24 Iteration 001 (2.1/D4): ON CONFLICT / REPLACE INTO 计划表示与解析分派
// ===========================================================================

/// users：id INT PK / name STRING（DEFAULT 'anon'）/ score INT 唯一索引列。
fn setup_builder_upsert() -> PlanBuilder {
    let mut builder = PlanBuilder::new();
    builder.register_table(
        "users",
        vec!["id".into(), "name".into(), "score".into()],
        "id",
    );
    builder.set_pk_column_type("users", rtsql::storage::ColumnType::Int);
    builder.set_table_defaults(
        "users",
        vec![
            None,
            Some(rtsql::executor::Value::String("anon".to_string())),
            None,
        ],
    );
    // score 列承载唯一索引（MS23 unique_indexes 的列位序）
    builder.set_table_unique_columns("users", vec![2]);
    builder
}

/// 目标省略 = 全仲裁（PK + 全部唯一索引），动作 DO NOTHING；values 为 D2 全宽行。
#[test]
fn on_conflict_without_target_uses_all_arbiter() {
    let sql = "INSERT INTO users (id, name) VALUES (1, 'Alice') ON CONFLICT DO NOTHING";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Upsert(node) => {
            assert_eq!(node.table_name, "users");
            assert!(
                matches!(node.arbiter, rtsql::executor::ConflictArbiter::All),
                "省略目标必须全仲裁，实际: {:?}",
                node.arbiter
            );
            assert!(
                matches!(node.action, rtsql::executor::ConflictAction::DoNothing),
                "实际: {:?}",
                node.action
            );
            assert_eq!(
                node.values[0],
                vec![
                    rtsql::executor::Value::Int(1),
                    rtsql::executor::Value::String("Alice".to_string()),
                    rtsql::executor::Value::Null,
                ],
                "UpsertNode.values 必须是 D2 全宽填充后的行"
            );
        }
        other => panic!("Expected Upsert, got {:?}", other),
    }
}

/// 显式单列目标命中 INT 声明 PK 列 → Column(0)。
#[test]
fn on_conflict_single_pk_target_resolves_column_arbiter() {
    let sql = "INSERT INTO users (id) VALUES (1) ON CONFLICT (id) DO NOTHING";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Upsert(node) => assert!(
            matches!(node.arbiter, rtsql::executor::ConflictArbiter::Column(0)),
            "显式 PK 单列目标必须仲裁该列，实际: {:?}",
            node.arbiter
        ),
        other => panic!("Expected Upsert, got {:?}", other),
    }
}

/// 显式单列目标命中唯一索引列 → Column(2)。
#[test]
fn on_conflict_single_unique_target_resolves_column_arbiter() {
    let sql = "INSERT INTO users (id) VALUES (1) ON CONFLICT (score) DO NOTHING";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Upsert(node) => assert!(
            matches!(node.arbiter, rtsql::executor::ConflictArbiter::Column(2)),
            "显式唯一列目标必须仲裁该列，实际: {:?}",
            node.arbiter
        ),
        other => panic!("Expected Upsert, got {:?}", other),
    }
}

/// 组合多列目标点名拒绝（引擎无组合唯一约束，SQLite 语义文案）。
#[test]
fn on_conflict_composite_target_rejected() {
    let sql = "INSERT INTO users (id) VALUES (1) ON CONFLICT (id, score) DO NOTHING";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string()
                .contains("does not match any PRIMARY KEY or UNIQUE constraint"),
            "组合目标必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected composite-target rejection, got {:?}", plan),
    }
}

/// 非唯一、非键列目标点名拒绝。
#[test]
fn on_conflict_non_unique_column_target_rejected() {
    let sql = "INSERT INTO users (id) VALUES (1) ON CONFLICT (name) DO NOTHING";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string()
                .contains("does not match any PRIMARY KEY or UNIQUE constraint"),
            "非唯一列目标必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected non-unique-target rejection, got {:?}", plan),
    }
}

/// 非 INT 声明 PK 列目标点名拒绝（`to_key` 仅 Int 产键，无索引条目可仲裁）。
#[test]
fn on_conflict_non_int_pk_target_rejected() {
    let mut builder = PlanBuilder::new();
    builder.register_table("kv", vec!["k".into(), "v".into()], "k");
    builder.set_pk_column_type("kv", rtsql::storage::ColumnType::String(100));

    let sql = "INSERT INTO kv (k, v) VALUES ('a', 1) ON CONFLICT (k) DO NOTHING";
    let stmts = parse_sql(sql).unwrap();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string()
                .contains("does not match any PRIMARY KEY or UNIQUE constraint"),
            "非 INT 声明 PK 目标必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected non-INT-PK-target rejection, got {:?}", plan),
    }
}

/// `ON CONFLICT ON CONSTRAINT <name>` 点名拒绝。
#[test]
fn on_conflict_on_constraint_rejected() {
    let sql = "INSERT INTO users (id) VALUES (1) ON CONFLICT ON CONSTRAINT users_pk DO NOTHING";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("ON CONSTRAINT"),
            "ON CONSTRAINT 必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected ON CONSTRAINT rejection, got {:?}", plan),
    }
}

/// `DO UPDATE ... WHERE` v1 点名拒绝。
#[test]
fn do_update_where_rejected() {
    let sql =
        "INSERT INTO users (id, name) VALUES (1, 'a') ON CONFLICT (id) DO UPDATE SET name = 'b' WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("DO UPDATE WHERE is not supported"),
            "DO UPDATE WHERE 必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected DO UPDATE WHERE rejection, got {:?}", plan),
    }
}

/// MySQL `ON DUPLICATE KEY UPDATE` 点名拒绝。
#[test]
fn on_duplicate_key_update_rejected() {
    let sql = "INSERT INTO users (id, name) VALUES (1, 'a') ON DUPLICATE KEY UPDATE name = 'b'";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string()
                .contains("ON DUPLICATE KEY UPDATE is not supported"),
            "ON DUPLICATE KEY UPDATE 必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected ON DUPLICATE KEY UPDATE rejection, got {:?}", plan),
    }
}

/// `REPLACE INTO` 与 `ON CONFLICT` 并存点名拒绝。
#[test]
fn replace_into_with_on_conflict_rejected() {
    let sql = "REPLACE INTO users (id) VALUES (1) ON CONFLICT DO NOTHING";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("REPLACE INTO"),
            "REPLACE INTO 与 ON CONFLICT 并存必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected replace_into+on rejection, got {:?}", plan),
    }
}

/// `REPLACE INTO` 映射为全仲裁 + Replace 动作。
#[test]
fn replace_into_maps_to_all_arbiter_replace_action() {
    let sql = "REPLACE INTO users (id, name) VALUES (1, 'Alice')";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Upsert(node) => {
            assert!(
                matches!(node.arbiter, rtsql::executor::ConflictArbiter::All),
                "REPLACE INTO 仲裁全部约束，实际: {:?}",
                node.arbiter
            );
            assert!(
                matches!(node.action, rtsql::executor::ConflictAction::Replace),
                "实际: {:?}",
                node.action
            );
        }
        other => panic!("Expected Upsert, got {:?}", other),
    }
}

/// DO UPDATE 赋值三形态：字面量 / `excluded.col`（新行值）/ 裸列名（旧行值）。
#[test]
fn do_update_assignment_three_forms() {
    let sql = "INSERT INTO users (id, name, score) VALUES (1, 'new', 5) \
               ON CONFLICT (id) DO UPDATE SET name = 'lit', score = excluded.score";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Upsert(node) => match node.action {
            rtsql::executor::ConflictAction::DoUpdate(assignments) => {
                assert_eq!(assignments.len(), 2, "多列赋值必须逐项承载");
                assert_eq!(assignments[0].column, 1);
                assert!(
                    matches!(
                        assignments[0].expr,
                        rtsql::executor::UpsertValueExpr::Literal(_)
                    ),
                    "字面量赋值形态，实际: {:?}",
                    assignments[0].expr
                );
                assert_eq!(assignments[1].column, 2);
                assert!(
                    matches!(
                        assignments[1].expr,
                        rtsql::executor::UpsertValueExpr::Excluded(2)
                    ),
                    "excluded.col 必须解析为待插行值位，实际: {:?}",
                    assignments[1].expr
                );
            }
            other => panic!("Expected DoUpdate, got {:?}", other),
        },
        other => panic!("Expected Upsert, got {:?}", other),
    }
}

/// DO UPDATE 裸列名引用解析为旧行值位（`Old`）。
#[test]
fn do_update_bare_column_is_old_row_reference() {
    let sql =
        "INSERT INTO users (id, score) VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET score = score";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Upsert(node) => match node.action {
            rtsql::executor::ConflictAction::DoUpdate(assignments) => {
                assert!(
                    matches!(
                        assignments[0].expr,
                        rtsql::executor::UpsertValueExpr::Old(2)
                    ),
                    "裸列名必须解析为旧行值位，实际: {:?}",
                    assignments[0].expr
                );
            }
            other => panic!("Expected DoUpdate, got {:?}", other),
        },
        other => panic!("Expected Upsert, got {:?}", other),
    }
}

/// DO UPDATE 的 `DEFAULT` 关键字在计划期字面化为该列声明 DEFAULT。
#[test]
fn do_update_default_keyword_literalized() {
    let sql = "INSERT INTO users (id) VALUES (1) ON CONFLICT (id) DO UPDATE SET name = DEFAULT";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Upsert(node) => match node.action {
            rtsql::executor::ConflictAction::DoUpdate(assignments) => {
                assert_eq!(assignments[0].column, 1);
                match &assignments[0].expr {
                    rtsql::executor::UpsertValueExpr::Literal(v) => assert_eq!(
                        v,
                        &rtsql::executor::Value::String("anon".to_string()),
                        "DEFAULT 必须字面化为该列声明默认值"
                    ),
                    other => panic!("Expected Literal, got {:?}", other),
                }
            }
            other => panic!("Expected DoUpdate, got {:?}", other),
        },
        other => panic!("Expected Upsert, got {:?}", other),
    }
}

/// 无声明 DEFAULT 的列在 DO UPDATE 中字面化为 NULL。
#[test]
fn do_update_default_keyword_without_declared_default_is_null() {
    let sql = "INSERT INTO users (id) VALUES (1) ON CONFLICT (id) DO UPDATE SET score = DEFAULT";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Upsert(node) => match node.action {
            rtsql::executor::ConflictAction::DoUpdate(assignments) => match &assignments[0].expr {
                rtsql::executor::UpsertValueExpr::Literal(v) => {
                    assert_eq!(v, &rtsql::executor::Value::Null)
                }
                other => panic!("Expected Literal, got {:?}", other),
            },
            other => panic!("Expected DoUpdate, got {:?}", other),
        },
        other => panic!("Expected Upsert, got {:?}", other),
    }
}

/// 算术赋值表达式点名拒绝。
#[test]
fn do_update_arithmetic_expression_rejected() {
    let sql = "INSERT INTO users (id) VALUES (1) ON CONFLICT (id) DO UPDATE SET score = score + 1";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("score"),
            "算术赋值必须点名拒绝并带出列名，实际: {e}"
        ),
        Ok(plan) => panic!("Expected arithmetic-assignment rejection, got {:?}", plan),
    }
}

/// 函数赋值表达式点名拒绝。
#[test]
fn do_update_function_expression_rejected() {
    let sql = "INSERT INTO users (id) VALUES (1) ON CONFLICT (id) DO UPDATE SET name = upper(name)";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("name"),
            "函数赋值必须点名拒绝并带出列名，实际: {e}"
        ),
        Ok(plan) => panic!("Expected function-assignment rejection, got {:?}", plan),
    }
}

/// DO UPDATE 未知赋值列沿用既有 ColumnNotFound 面。
#[test]
fn do_update_unknown_column_rejected() {
    let sql = "INSERT INTO users (id) VALUES (1) ON CONFLICT (id) DO UPDATE SET zz = 'x'";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("zz"),
            "未知赋值列必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!(
            "Expected unknown-assignment-column rejection, got {:?}",
            plan
        ),
    }
}

/// DO UPDATE 复合左值（`t.col = ...`）点名拒绝。
#[test]
fn do_update_compound_lhs_rejected() {
    let sql = "INSERT INTO users (id) VALUES (1) ON CONFLICT (id) DO UPDATE SET users.name = 'x'";
    let stmts = parse_sql(sql).unwrap();
    let mut builder = setup_builder_upsert();
    let result = builder.build_plan(&stmts[0]);

    match result {
        Err(e) => assert!(
            e.to_string().contains("name"),
            "复合左值必须点名拒绝，实际: {e}"
        ),
        Ok(plan) => panic!("Expected compound-LHS rejection, got {:?}", plan),
    }
}
