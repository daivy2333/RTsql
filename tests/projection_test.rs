//! Integration tests for MS10-T01 Iteration 001: true projection in scan
//! executors (IR1).
//!
//! Witness for the three symptom families recorded by Iteration 000's Plan
//! Review and the 2026-09-06 exploration session:
//! 1. subset projection returns projected rows with matching shapes on every
//!    scan path (DataScan bare / pushdown, IndexScan point lookup,
//!    Filter-wrapped OR shape),
//! 2. PK-point aggregates return real values instead of NULL,
//! 3. ORDER BY keys outside the projection still sort.

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use serde_json::json;
use tempfile::tempdir;

async fn open_db() -> (Database, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("projection.db"))
        .await
        .unwrap();
    exec_ok(
        &db,
        "CREATE TABLE s (id INT PRIMARY KEY, name VARCHAR, price INT)",
    )
    .await;
    for (id, name, price) in [(1, "'Alice'", 10), (2, "'Bob'", 20), (3, "'Carol'", 30)] {
        exec_ok(
            &db,
            &format!("INSERT INTO s (id, name, price) VALUES ({id}, {name}, {price})"),
        )
        .await;
    }
    (db, dir)
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = db.execute_sql(sql).await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "setup statement failed: {sql:?} -> {resp:?}"
    );
}

fn query_rows(resp: Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("Expected QueryResult, got {other:?}"),
    }
}

/// (a) Bare DataScan: subset projection returns single-column rows.
#[tokio::test]
async fn data_scan_subset_projection() {
    let (db, _dir) = open_db().await;
    let rows = query_rows(db.execute_sql("SELECT name FROM s").await);
    assert_eq!(
        rows,
        vec![
            vec![json!("Alice")],
            vec![json!("Bob")],
            vec![json!("Carol")]
        ]
    );
}

/// (b) IndexScan point lookup: subset projection returns single-column rows.
#[tokio::test]
async fn index_scan_point_projection() {
    let (db, _dir) = open_db().await;
    let rows = query_rows(db.execute_sql("SELECT name FROM s WHERE id = 1").await);
    assert_eq!(rows, vec![vec![json!("Alice")]]);
}

/// (c) `SELECT *` keeps the full-schema row shape (behavior-unchanged guard).
#[tokio::test]
async fn select_star_unchanged() {
    let (db, _dir) = open_db().await;
    let rows = query_rows(db.execute_sql("SELECT * FROM s").await);
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!("Alice"), json!(10)],
            vec![json!(2), json!("Bob"), json!(20)],
            vec![json!(3), json!("Carol"), json!(30)],
        ]
    );
}

/// (f) Filter-wrapped OR shape: subset projection is applied after the
/// wrapper predicate evaluates on the full-schema row.
#[tokio::test]
async fn filter_shape_subset_projection() {
    let (db, _dir) = open_db().await;
    let rows = query_rows(
        db.execute_sql("SELECT name FROM s WHERE id = 1 OR id = 3")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("Alice")], vec![json!("Carol")]]);
}

/// (d) PK-point aggregate: SUM over an IndexScan input returns the real
/// value instead of a silent NULL (input-schema column mapping aligned).
#[tokio::test]
async fn pk_point_aggregate_real_value() {
    let (db, _dir) = open_db().await;
    let rows = query_rows(
        db.execute_sql("SELECT SUM(price) FROM s WHERE id = 2")
            .await,
    );
    assert_eq!(rows, vec![vec![json!(20)]]);
}

/// (e) ORDER BY key outside the projection still sorts (Sort consumes
/// full-schema rows and owns the projection trim at output).
#[tokio::test]
async fn order_by_key_outside_projection() {
    let (db, _dir) = open_db().await;
    // price > 15 -> id 2 (Bob), id 3 (Carol); ORDER BY name DESC -> Carol, Bob.
    let rows = query_rows(
        db.execute_sql("SELECT id FROM s WHERE price > 15 ORDER BY name DESC")
            .await,
    );
    assert_eq!(rows, vec![vec![json!(3)], vec![json!(2)]]);
}
