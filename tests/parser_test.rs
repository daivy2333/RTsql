//! SQL parsing integration tests
//!
//! Tests for parse_sql function and AST extraction

use rtsql::parse_sql;
use sqlparser::ast::Statement;

#[test]
fn test_parse_select() {
    let sql = "SELECT id, name FROM users";
    let stmts = parse_sql(sql).unwrap();
    assert_eq!(stmts.len(), 1);
    assert!(matches!(stmts[0], Statement::Query(_)));
}

#[test]
fn test_parse_select_with_where() {
    let sql = "SELECT id, name FROM users WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    assert_eq!(stmts.len(), 1);
    if let Statement::Query(query) = &stmts[0] {
        // Query has a body that contains Select with selection (WHERE clause)
        use sqlparser::ast::SetExpr;
        if let SetExpr::Select(select) = query.body.as_ref() {
            assert!(select.selection.is_some(), "Expected WHERE clause");
        } else {
            panic!("Expected Select body");
        }
    } else {
        panic!("Expected Query statement");
    }
}

#[test]
fn test_parse_insert() {
    let sql = "INSERT INTO users (id, name) VALUES (1, 'Alice')";
    let stmts = parse_sql(sql).unwrap();
    assert_eq!(stmts.len(), 1);
    if let Statement::Insert { table_name, .. } = &stmts[0] {
        assert_eq!(table_name.to_string().to_lowercase(), "users");
    } else {
        panic!("Expected Insert statement");
    }
}

#[test]
fn test_parse_update() {
    let sql = "UPDATE users SET name = 'Bob' WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    assert_eq!(stmts.len(), 1);
    assert!(matches!(stmts[0], Statement::Update { .. }));
}

#[test]
fn test_parse_delete() {
    let sql = "DELETE FROM users WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    assert_eq!(stmts.len(), 1);
    assert!(matches!(stmts[0], Statement::Delete { .. }));
}

#[test]
fn test_parse_error() {
    let sql = "INVALID SQL STATEMENT";
    let result = parse_sql(sql);
    assert!(result.is_err());
}
