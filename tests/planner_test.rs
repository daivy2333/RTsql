//! Plan builder integration tests
//!
//! Tests for PlanBuilder converting SQL AST to PhysicalPlan

use rtsql::{parse_sql, PhysicalPlan, PlanBuilder};

fn setup_builder() -> PlanBuilder {
    let mut builder = PlanBuilder::new();
    builder.register_table("users", vec!["id".into(), "name".into()], "id");
    builder
}

#[test]
fn test_select_by_pk() {
    let sql = "SELECT id, name FROM users WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    let builder = setup_builder();
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
    let builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Scan(node) => {
            assert_eq!(node.table_name, "users");
            assert_eq!(node.columns, vec!["id", "name"]);
        }
        _ => panic!("Expected Scan, got {:?}", plan),
    }
}

#[test]
fn test_insert() {
    let sql = "INSERT INTO users (id, name) VALUES (1, 'Alice')";
    let stmts = parse_sql(sql).unwrap();
    let builder = setup_builder();
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
    let builder = setup_builder();
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
    let builder = setup_builder();
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
    let builder = setup_builder();
    let result = builder.build_plan(&stmts[0]);

    assert!(result.is_err());
}

#[test]
fn test_invalid_where_not_pk() {
    // Non-PK WHERE clause should fall back to full table scan
    let sql = "SELECT id, name FROM users WHERE name = 'Alice'";
    let stmts = parse_sql(sql).unwrap();
    let builder = setup_builder();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::Scan(node) => {
            assert_eq!(node.table_name, "users");
        }
        _ => panic!("Expected Scan for non-PK WHERE, got {:?}", plan),
    }
}

#[test]
fn test_unsupported_statement() {
    let sql = "CREATE TABLE test (id INT)";
    let stmts = parse_sql(sql).unwrap();
    let builder = setup_builder();
    let result = builder.build_plan(&stmts[0]);

    assert!(result.is_err());
}
