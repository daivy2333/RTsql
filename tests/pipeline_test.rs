//! Pipeline integration tests for DDL + WHERE execution

use rtsql::database::Database;
use tempfile::tempdir;

#[tokio::test]
async fn test_pipeline_create_table() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db"))
        .await
        .expect("Failed to open database");

    // Create table via SQL
    let response = db
        .execute_sql("CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR)")
        .await;

    match response {
        rtsql::network::protocol::Response::AffectedRows { count } => {
            assert_eq!(count, 0);
        }
        rtsql::network::protocol::Response::Error { message } => {
            panic!("CREATE TABLE failed: {}", message);
        }
        _ => panic!("Expected AffectedRows response"),
    }

    // Verify table exists by querying it
    let response = db.execute_sql("SELECT * FROM users").await;
    match response {
        rtsql::network::protocol::Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 0); // Empty table
        }
        rtsql::network::protocol::Response::Error { message } => {
            panic!("SELECT failed: {}", message);
        }
        _ => panic!("Expected QueryResult response"),
    }
}

#[tokio::test]
async fn test_pipeline_create_table_already_exists() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db"))
        .await
        .expect("Failed to open database");

    // Create table first time
    let response = db
        .execute_sql("CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR)")
        .await;
    assert!(matches!(
        response,
        rtsql::network::protocol::Response::AffectedRows { count: 0 }
    ));

    // Try to create same table again - should fail
    let response = db
        .execute_sql("CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR)")
        .await;

    match response {
        rtsql::network::protocol::Response::Error { message } => {
            assert!(message.contains("already exists"));
        }
        _ => panic!("Expected Error response for duplicate table"),
    }
}

#[tokio::test]
async fn test_pipeline_drop_table() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db"))
        .await
        .expect("Failed to open database");

    // Create table
    let response = db
        .execute_sql("CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR)")
        .await;
    assert!(matches!(
        response,
        rtsql::network::protocol::Response::AffectedRows { count: 0 }
    ));

    // Drop table
    let response = db.execute_sql("DROP TABLE users").await;
    match response {
        rtsql::network::protocol::Response::AffectedRows { count } => {
            assert_eq!(count, 0);
        }
        rtsql::network::protocol::Response::Error { message } => {
            panic!("DROP TABLE failed: {}", message);
        }
        _ => panic!("Expected AffectedRows response"),
    }

    // Verify table is dropped - should fail to query
    let response = db.execute_sql("SELECT * FROM users").await;
    match response {
        rtsql::network::protocol::Response::Error { message } => {
            assert!(message.contains("not found") || message.contains("does not exist"));
        }
        _ => panic!("Expected Error response for non-existent table"),
    }
}

#[tokio::test]
async fn test_pipeline_drop_table_if_exists() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db"))
        .await
        .expect("Failed to open database");

    // DROP TABLE IF EXISTS on non-existent table should succeed
    let response = db.execute_sql("DROP TABLE IF EXISTS nonexistent").await;
    match response {
        rtsql::network::protocol::Response::AffectedRows { count } => {
            assert_eq!(count, 0);
        }
        _ => panic!("Expected AffectedRows response for IF EXISTS"),
    }

    // DROP TABLE without IF EXISTS on non-existent table should fail
    let response = db.execute_sql("DROP TABLE nonexistent").await;
    match response {
        rtsql::network::protocol::Response::Error { message } => {
            assert!(message.contains("not found"));
        }
        _ => panic!("Expected Error response for non-existent table"),
    }
}

#[tokio::test]
async fn test_pipeline_where_select() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db"))
        .await
        .expect("Failed to open database");

    // Create table
    db.execute_sql("CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR, age INT)")
        .await;

    // Insert rows
    db.execute_sql("INSERT INTO users (id, name, age) VALUES (1, 'Alice', 30)")
        .await;
    db.execute_sql("INSERT INTO users (id, name, age) VALUES (2, 'Bob', 25)")
        .await;
    db.execute_sql("INSERT INTO users (id, name, age) VALUES (3, 'Charlie', 35)")
        .await;

    // Query with WHERE clause (non-PK column)
    let response = db
        .execute_sql("SELECT id, name, age FROM users WHERE age > 28")
        .await;

    match response {
        rtsql::network::protocol::Response::QueryResult { rows } => {
            // Should return Alice (30) and Charlie (35)
            assert_eq!(rows.len(), 2, "Expected 2 rows with age > 28");
        }
        rtsql::network::protocol::Response::Error { message } => {
            panic!("SELECT with WHERE failed: {}", message);
        }
        _ => panic!("Expected QueryResult response"),
    }
}

#[tokio::test]
async fn test_pipeline_where_select_and() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db"))
        .await
        .expect("Failed to open database");

    // Create table
    db.execute_sql("CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR, age INT)")
        .await;

    // Insert rows
    db.execute_sql("INSERT INTO users (id, name, age) VALUES (1, 'Alice', 30)")
        .await;
    db.execute_sql("INSERT INTO users (id, name, age) VALUES (2, 'Bob', 25)")
        .await;
    db.execute_sql("INSERT INTO users (id, name, age) VALUES (3, 'Charlie', 35)")
        .await;

    // Query with AND in WHERE clause
    let response = db
        .execute_sql("SELECT id, name, age FROM users WHERE age > 25 AND age < 35")
        .await;

    match response {
        rtsql::network::protocol::Response::QueryResult { rows } => {
            // Should return only Alice (30)
            assert_eq!(rows.len(), 1, "Expected 1 row with age > 25 AND age < 35");
        }
        rtsql::network::protocol::Response::Error { message } => {
            panic!("SELECT with WHERE AND failed: {}", message);
        }
        _ => panic!("Expected QueryResult response"),
    }
}

#[tokio::test]
async fn test_pipeline_where_select_or() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db"))
        .await
        .expect("Failed to open database");

    // Create table
    db.execute_sql("CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR, age INT)")
        .await;

    // Insert rows
    db.execute_sql("INSERT INTO users (id, name, age) VALUES (1, 'Alice', 30)")
        .await;
    db.execute_sql("INSERT INTO users (id, name, age) VALUES (2, 'Bob', 25)")
        .await;
    db.execute_sql("INSERT INTO users (id, name, age) VALUES (3, 'Charlie', 35)")
        .await;

    // Query with OR in WHERE clause
    let response = db
        .execute_sql("SELECT id, name, age FROM users WHERE age < 26 OR age > 34")
        .await;

    match response {
        rtsql::network::protocol::Response::QueryResult { rows } => {
            // Should return Bob (25) and Charlie (35)
            assert_eq!(rows.len(), 2, "Expected 2 rows with age < 26 OR age > 34");
        }
        rtsql::network::protocol::Response::Error { message } => {
            panic!("SELECT with WHERE OR failed: {}", message);
        }
        _ => panic!("Expected QueryResult response"),
    }
}

#[tokio::test]
async fn test_pipeline_where_string_comparison() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db"))
        .await
        .expect("Failed to open database");

    // Create table
    db.execute_sql("CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR)")
        .await;

    // Insert rows
    db.execute_sql("INSERT INTO users (id, name) VALUES (1, 'Alice')")
        .await;
    db.execute_sql("INSERT INTO users (id, name) VALUES (2, 'Bob')")
        .await;
    db.execute_sql("INSERT INTO users (id, name) VALUES (3, 'Charlie')")
        .await;

    // Query with string comparison
    let response = db
        .execute_sql("SELECT id, name FROM users WHERE name = 'Bob'")
        .await;

    match response {
        rtsql::network::protocol::Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1, "Expected 1 row with name = 'Bob'");
        }
        rtsql::network::protocol::Response::Error { message } => {
            panic!("SELECT with string WHERE failed: {}", message);
        }
        _ => panic!("Expected QueryResult response"),
    }
}

#[tokio::test]
async fn test_pipeline_full_flow() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db"))
        .await
        .expect("Failed to open database");

    // Full flow: CREATE TABLE -> INSERT -> SELECT with WHERE -> DROP TABLE

    // 1. CREATE TABLE
    let response = db
        .execute_sql("CREATE TABLE products (id INT PRIMARY KEY, name VARCHAR, price FLOAT)")
        .await;
    assert!(matches!(
        response,
        rtsql::network::protocol::Response::AffectedRows { count: 0 }
    ));

    // 2. INSERT
    let response = db
        .execute_sql("INSERT INTO products (id, name, price) VALUES (1, 'Apple', 1.5)")
        .await;

    match response {
        rtsql::network::protocol::Response::AffectedRows { count } => {
            assert_eq!(count, 1, "Expected 1 row affected");
        }
        rtsql::network::protocol::Response::Error { message } => {
            panic!("INSERT failed: {}", message);
        }
        other => panic!("Unexpected response type: {:?}", other),
    }

    db.execute_sql("INSERT INTO products (id, name, price) VALUES (2, 'Banana', 0.5)")
        .await;
    db.execute_sql("INSERT INTO products (id, name, price) VALUES (3, 'Cherry', 2.0)")
        .await;

    // 3. SELECT with WHERE (float comparison)
    let response = db
        .execute_sql("SELECT id, name, price FROM products WHERE price > 1.0")
        .await;

    match response {
        rtsql::network::protocol::Response::QueryResult { rows } => {
            // Apple (1.5) and Cherry (2.0)
            assert_eq!(rows.len(), 2, "Expected 2 products with price > 1.0");
        }
        _ => panic!("Expected QueryResult"),
    }

    // 4. DROP TABLE
    let response = db.execute_sql("DROP TABLE products").await;
    assert!(matches!(
        response,
        rtsql::network::protocol::Response::AffectedRows { count: 0 }
    ));
}
