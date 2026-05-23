//! End-to-end tests for aggregate functions, GROUP BY, and HAVING.

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use tempfile::tempdir;

/// Helper: open an in-memory database.
async fn open_db() -> Database {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");
    Database::open(&path).await.unwrap()
}

/// Helper: execute SQL and return the Response.
async fn exec(db: &Database, sql: &str) -> Response {
    db.execute_sql(sql).await
}

/// Helper: extract rows from a QueryResult, panicking if not QueryResult.
fn rows(resp: Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        Response::AffectedRows { count } => {
            panic!("expected QueryResult, got AffectedRows({count})")
        }
        Response::Error { message } => {
            panic!("expected QueryResult, got Error: {message}")
        }
        Response::Pong => {
            panic!("expected QueryResult, got Pong")
        }
    }
}

/// Helper: extract error message from an Error response.
fn error_msg(resp: Response) -> String {
    match resp {
        Response::Error { message } => message,
        Response::Pong => "Pong".to_string(),
        _ => panic!("expected Error, got non-error response"),
    }
}

/// Helper: set up a students table with sample data.
async fn setup_students(db: &Database) {
    exec(
        db,
        "CREATE TABLE students (id INT, name TEXT, class TEXT, score INT)",
    )
    .await;
    exec(db, "INSERT INTO students VALUES (1, 'Alice', 'A', 90)").await;
    exec(db, "INSERT INTO students VALUES (2, 'Bob', 'B', 85)").await;
    exec(db, "INSERT INTO students VALUES (3, 'Carol', 'A', 95)").await;
    exec(db, "INSERT INTO students VALUES (4, 'Dave', 'B', 70)").await;
    exec(db, "INSERT INTO students VALUES (5, 'Eve', 'A', 80)").await;
}

/// Helper: set up a table with NULL values.
async fn setup_nulls(db: &Database) {
    exec(db, "CREATE TABLE nulltest (id INT, val INT)").await;
    exec(db, "INSERT INTO nulltest VALUES (1, 10)").await;
    exec(db, "INSERT INTO nulltest VALUES (2, NULL)").await;
    exec(db, "INSERT INTO nulltest VALUES (3, 30)").await;
    exec(db, "INSERT INTO nulltest VALUES (4, NULL)").await;
    exec(db, "INSERT INTO nulltest VALUES (5, 50)").await;
}

/// Helper: set up an empty table.
async fn setup_empty(db: &Database) {
    exec(db, "CREATE TABLE empty_tbl (id INT, score INT)").await;
}

// =============================================================================
// 1. Basic aggregates (no GROUP BY)
// =============================================================================

#[tokio::test]
async fn test_count_star() {
    let db = open_db().await;
    setup_students(&db).await;

    let r = rows(exec(&db, "SELECT COUNT(*) FROM students").await);
    assert_eq!(r.len(), 1, "should have 1 row");
    assert_eq!(r[0].len(), 1, "should have 1 column");
    // COUNT(*) should be 5
    assert_eq!(r[0][0], serde_json::json!(5));
}

#[tokio::test]
async fn test_count_col() {
    let db = open_db().await;
    setup_students(&db).await;

    let r = rows(exec(&db, "SELECT COUNT(score) FROM students").await);
    assert_eq!(r.len(), 1, "should have 1 row");
    assert_eq!(r[0][0], serde_json::json!(5));
}

#[tokio::test]
async fn test_count_col_with_nulls() {
    let db = open_db().await;
    setup_nulls(&db).await;

    let r = rows(exec(&db, "SELECT COUNT(val) FROM nulltest").await);
    // 3 non-NULL values
    assert_eq!(r[0][0], serde_json::json!(3));
}

#[tokio::test]
async fn test_sum() {
    let db = open_db().await;
    setup_students(&db).await;

    let r = rows(exec(&db, "SELECT SUM(score) FROM students").await);
    // 90 + 85 + 95 + 70 + 80 = 420
    assert_eq!(r[0][0], serde_json::json!(420));
}

#[tokio::test]
async fn test_avg() {
    let db = open_db().await;
    setup_students(&db).await;

    let r = rows(exec(&db, "SELECT AVG(score) FROM students").await);
    // 420 / 5 = 84
    let val = r[0][0].as_f64().expect("AVG should return a number");
    assert!((val - 84.0).abs() < 0.01, "AVG should be 84.0, got {val}");
}

#[tokio::test]
async fn test_min_max() {
    let db = open_db().await;
    setup_students(&db).await;

    let r_min = rows(exec(&db, "SELECT MIN(score) FROM students").await);
    assert_eq!(r_min[0][0], serde_json::json!(70));

    let r_max = rows(exec(&db, "SELECT MAX(score) FROM students").await);
    assert_eq!(r_max[0][0], serde_json::json!(95));
}

#[tokio::test]
async fn test_count_star_empty_table() {
    let db = open_db().await;
    setup_empty(&db).await;

    let r = rows(exec(&db, "SELECT COUNT(*) FROM empty_tbl").await);
    assert_eq!(r.len(), 1, "should have 1 row even for empty table");
    assert_eq!(r[0][0], serde_json::json!(0));
}

#[tokio::test]
async fn test_sum_empty_table() {
    let db = open_db().await;
    setup_empty(&db).await;

    let r = rows(exec(&db, "SELECT SUM(score) FROM empty_tbl").await);
    assert_eq!(r.len(), 1, "should have 1 row even for empty table");
    assert!(r[0][0].is_null(), "SUM on empty table should be NULL");
}

#[tokio::test]
async fn test_multiple_aggregates() {
    let db = open_db().await;
    setup_students(&db).await;

    let r = rows(
        exec(
            &db,
            "SELECT COUNT(*), SUM(score), AVG(score), MIN(score), MAX(score) FROM students",
        )
        .await,
    );
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].len(), 5);
    assert_eq!(r[0][0], serde_json::json!(5)); // COUNT(*)
    assert_eq!(r[0][1], serde_json::json!(420)); // SUM
    let avg = r[0][2].as_f64().expect("AVG should be a number");
    assert!((avg - 84.0).abs() < 0.01, "AVG should be 84.0, got {avg}");
    assert_eq!(r[0][3], serde_json::json!(70)); // MIN
    assert_eq!(r[0][4], serde_json::json!(95)); // MAX
}

// =============================================================================
// 2. GROUP BY
// =============================================================================

#[tokio::test]
async fn test_group_by_count() {
    let db = open_db().await;
    setup_students(&db).await;

    let r = rows(exec(&db, "SELECT class, COUNT(*) FROM students GROUP BY class").await);
    assert_eq!(r.len(), 2, "should have 2 groups (A, B)");

    // Find the group for class A
    let group_a = r
        .iter()
        .find(|row| row[0] == serde_json::json!("A"))
        .expect("should have class A");
    assert_eq!(group_a[1], serde_json::json!(3)); // Alice, Carol, Eve

    let group_b = r
        .iter()
        .find(|row| row[0] == serde_json::json!("B"))
        .expect("should have class B");
    assert_eq!(group_b[1], serde_json::json!(2)); // Bob, Dave
}

#[tokio::test]
async fn test_group_by_sum() {
    let db = open_db().await;
    setup_students(&db).await;

    let r = rows(exec(&db, "SELECT class, SUM(score) FROM students GROUP BY class").await);

    let group_a = r
        .iter()
        .find(|row| row[0] == serde_json::json!("A"))
        .expect("should have class A");
    // Alice(90) + Carol(95) + Eve(80) = 265
    assert_eq!(group_a[1], serde_json::json!(265));

    let group_b = r
        .iter()
        .find(|row| row[0] == serde_json::json!("B"))
        .expect("should have class B");
    // Bob(85) + Dave(70) = 155
    assert_eq!(group_b[1], serde_json::json!(155));
}

#[tokio::test]
async fn test_group_by_avg() {
    let db = open_db().await;
    setup_students(&db).await;

    let r = rows(exec(&db, "SELECT class, AVG(score) FROM students GROUP BY class").await);

    let group_a = r
        .iter()
        .find(|row| row[0] == serde_json::json!("A"))
        .expect("should have class A");
    let avg_a = group_a[1].as_f64().expect("AVG should be a number");
    // 265 / 3 = 88.333...
    assert!(
        (avg_a - 88.333).abs() < 0.1,
        "AVG for A should be ~88.33, got {avg_a}"
    );

    let group_b = r
        .iter()
        .find(|row| row[0] == serde_json::json!("B"))
        .expect("should have class B");
    let avg_b = group_b[1].as_f64().expect("AVG should be a number");
    // 155 / 2 = 77.5
    assert!(
        (avg_b - 77.5).abs() < 0.01,
        "AVG for B should be 77.5, got {avg_b}"
    );
}

#[tokio::test]
async fn test_group_by_strict_non_aggregated_column() {
    let db = open_db().await;
    setup_students(&db).await;

    // name is not in GROUP BY and not aggregated — should error
    let resp = exec(&db, "SELECT name, COUNT(*) FROM students GROUP BY class").await;
    let msg = error_msg(resp);
    assert!(
        msg.to_lowercase().contains("group")
            || msg.to_lowercase().contains("aggregate")
            || msg.to_lowercase().contains("not in"),
        "expected error about non-aggregated column, got: {msg}"
    );
}

#[tokio::test]
async fn test_group_by_null_handling() {
    let db = open_db().await;
    exec(
        &db,
        "CREATE TABLE gn (id INT PRIMARY KEY, cat TEXT, val INT)",
    )
    .await;
    exec(&db, "INSERT INTO gn VALUES (1, 'x', 1)").await;
    exec(&db, "INSERT INTO gn VALUES (2, NULL, 2)").await;
    exec(&db, "INSERT INTO gn VALUES (3, 'x', 3)").await;
    exec(&db, "INSERT INTO gn VALUES (4, NULL, 4)").await;

    let r = rows(exec(&db, "SELECT cat, COUNT(*) FROM gn GROUP BY cat").await);
    // Two groups: 'x' and NULL
    assert_eq!(r.len(), 2, "should have 2 groups (x and NULL)");

    let group_x = r
        .iter()
        .find(|row| row[0] == serde_json::json!("x"))
        .expect("should have group x");
    assert_eq!(group_x[1], serde_json::json!(2));
}

// =============================================================================
// 3. HAVING
// =============================================================================

#[tokio::test]
async fn test_having_count() {
    let db = open_db().await;
    setup_students(&db).await;

    // Only class A has COUNT(*) > 2
    let r = rows(
        exec(
            &db,
            "SELECT class, COUNT(*) FROM students GROUP BY class HAVING COUNT(*) > 2",
        )
        .await,
    );
    assert_eq!(r.len(), 1, "only class A should pass HAVING COUNT(*) > 2");
    assert_eq!(r[0][0], serde_json::json!("A"));
    assert_eq!(r[0][1], serde_json::json!(3));
}

#[tokio::test]
async fn test_having_sum() {
    let db = open_db().await;
    setup_students(&db).await;

    // Class A sum=265, Class B sum=155. Only A has SUM > 200.
    let r = rows(
        exec(
            &db,
            "SELECT class, SUM(score) FROM students GROUP BY class HAVING SUM(score) > 200",
        )
        .await,
    );
    assert_eq!(r.len(), 1);
    assert_eq!(r[0][0], serde_json::json!("A"));
    assert_eq!(r[0][1], serde_json::json!(265));
}

#[tokio::test]
async fn test_having_filters_all() {
    let db = open_db().await;
    setup_students(&db).await;

    // No group has COUNT(*) > 10
    let r = rows(
        exec(
            &db,
            "SELECT class, COUNT(*) FROM students GROUP BY class HAVING COUNT(*) > 10",
        )
        .await,
    );
    assert_eq!(r.len(), 0, "HAVING should filter out all groups");
}

// =============================================================================
// 4. Combination queries
// =============================================================================

#[tokio::test]
async fn test_aggregate_with_where() {
    let db = open_db().await;
    setup_students(&db).await;

    // Only scores > 80: Alice(90), Bob(85), Carol(95)
    let r = rows(
        exec(
            &db,
            "SELECT COUNT(*), SUM(score) FROM students WHERE score > 80",
        )
        .await,
    );
    assert_eq!(r[0][0], serde_json::json!(3)); // COUNT
    assert_eq!(r[0][1], serde_json::json!(270)); // 90 + 85 + 95
}

#[tokio::test]
async fn test_group_by_with_where() {
    let db = open_db().await;
    setup_students(&db).await;

    // Only scores > 80: Alice(A,90), Bob(B,85), Carol(A,95)
    let r = rows(
        exec(
            &db,
            "SELECT class, COUNT(*), SUM(score) FROM students WHERE score > 80 GROUP BY class",
        )
        .await,
    );

    let group_a = r
        .iter()
        .find(|row| row[0] == serde_json::json!("A"))
        .expect("should have class A");
    assert_eq!(group_a[1], serde_json::json!(2)); // Alice, Carol
    assert_eq!(group_a[2], serde_json::json!(185)); // 90 + 95

    let group_b = r
        .iter()
        .find(|row| row[0] == serde_json::json!("B"))
        .expect("should have class B");
    assert_eq!(group_b[1], serde_json::json!(1)); // Bob
    assert_eq!(group_b[2], serde_json::json!(85));
}
