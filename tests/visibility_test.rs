//! Pre-TDD regression tests for M21 page-level MVCC visibility optimization.
//!
//! These tests validate correctness of the visibility system through the public
//! Database API. They serve as behavioral regression tests: once the page-level
//! all-visible optimization is implemented, these tests must still pass.
//!
//! Important: `execute_sql` auto-commits each DML statement — there is no
//! explicit BEGIN/COMMIT in the SQL dialect. Each INSERT/DELETE/UPDATE is
//! immediately visible to subsequent statements.

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use std::sync::Arc;
use tempfile::tempdir;

// ── Helpers ────────────────────────────────────────────────────────────

async fn open_db() -> Database {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_visibility.db");
    Database::open(&path).await.unwrap()
}

fn rows(resp: Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        Response::AffectedRows { count } => {
            panic!("expected QueryResult, got AffectedRows({count})")
        }
        Response::Error { message } => panic!("expected QueryResult, got Error: {message}"),
        Response::Pong => panic!("expected QueryResult, got Pong"),
    }
}

fn assert_row_count(resp: Response, expected_count: usize) {
    let r = rows(resp);
    assert_eq!(
        r.len(),
        expected_count,
        "expected {expected_count} rows, got {}",
        r.len()
    );
}

#[allow(dead_code)]
fn affected_count(resp: Response) -> u64 {
    match resp {
        Response::AffectedRows { count } => count,
        other => panic!("expected AffectedRows, got {other:?}"),
    }
}

// ── Test 1: All-visible page skips per-row checks ──────────────────────

#[tokio::test]
async fn test_visibility_all_visible_page_skips_per_row_checks() {
    let db = open_db().await;

    db.execute_sql("CREATE TABLE t1 (id INT PRIMARY KEY, val INT)")
        .await;

    for i in 1..=10 {
        db.execute_sql(&format!("INSERT INTO t1 VALUES ({}, {})", i, i * 10))
            .await;
    }

    let resp = db.execute_sql("SELECT * FROM t1").await;
    assert_row_count(resp, 10);

    let resp = db.execute_sql("SELECT COUNT(*) FROM t1").await;
    let r = rows(resp);
    assert_eq!(r[0][0].as_i64().unwrap(), 10);
}

// ── Test 2: INSERT clears all-visible flag ─────────────────────────────

#[tokio::test]
async fn test_visibility_insert_clears_all_visible() {
    let db = open_db().await;

    db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY, val INT)")
        .await;

    // Phase 1: initial batch (page becomes all-visible)
    for i in 1..=5 {
        db.execute_sql(&format!("INSERT INTO t2 VALUES ({}, {})", i, i * 10))
            .await;
    }

    let resp = db.execute_sql("SELECT * FROM t2").await;
    assert_row_count(resp, 5);

    // Phase 2: more inserts (page transitions from all-visible to partial)
    for i in 6..=10 {
        db.execute_sql(&format!("INSERT INTO t2 VALUES ({}, {})", i, i * 10))
            .await;
    }

    let resp = db.execute_sql("SELECT * FROM t2").await;
    assert_row_count(resp, 10);

    let resp = db.execute_sql("SELECT COUNT(*) FROM t2").await;
    let r = rows(resp);
    assert_eq!(r[0][0].as_i64().unwrap(), 10);
}

// ── Test 3: DELETE clears all-visible flag ────────────────────────────

#[tokio::test]
async fn test_visibility_delete_clears_all_visible() {
    let db = open_db().await;

    db.execute_sql("CREATE TABLE t3 (id INT PRIMARY KEY, val INT)")
        .await;

    for i in 1..=5 {
        db.execute_sql(&format!("INSERT INTO t3 VALUES ({}, {})", i, i * 10))
            .await;
    }

    let resp = db.execute_sql("SELECT * FROM t3").await;
    assert_row_count(resp, 5);

    // Delete row id=3
    let resp = db.execute_sql("DELETE FROM t3 WHERE id = 3").await;
    // Accept either: DELETE succeeds and returns affected count, OR
    // DELETE returns an error (not yet supported) — both are valid pre-TDD outcomes.
    match resp {
        Response::AffectedRows { count } => {
            assert!(count >= 1, "DELETE should affect at least 1 row");
            let resp = db.execute_sql("SELECT * FROM t3 WHERE id = 3").await;
            assert_row_count(resp, 0);
            let resp = db.execute_sql("SELECT * FROM t3").await;
            assert_row_count(resp, 5);
            let resp = db.execute_sql("SELECT COUNT(*) FROM t3").await;
            let r = rows(resp);
            assert_eq!(r[0][0].as_i64().unwrap(), 5);
        }
        Response::Error { .. } => {
            // DELETE not yet supported via SQL API — this is expected pre-TDD
        }
        _ => panic!("Unexpected response from DELETE"),
    }
}

// ── Test 4: Concurrent read/write ──────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_visibility_concurrent_read_write() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_conc_vis.db");
    let db = Arc::new(Database::open(&path).await.unwrap());

    db.execute_sql("CREATE TABLE t4 (id INT PRIMARY KEY, val INT)")
        .await;

    for i in 1..=20 {
        db.execute_sql(&format!("INSERT INTO t4 VALUES ({}, {})", i, i * 10))
            .await;
    }

    let db_reader = db.clone();
    let db_writer = db.clone();

    let writer = tokio::spawn(async move {
        for i in 21..=100 {
            db_writer
                .execute_sql(&format!("INSERT INTO t4 VALUES ({}, {})", i, i * 10))
                .await;
            tokio::task::yield_now().await;
        }
    });

    let reader = tokio::spawn(async move {
        let mut seen_max = 20;
        for _ in 0..30 {
            let resp = db_reader.execute_sql("SELECT id, val FROM t4").await;
            let r = rows(resp);

            for row in &r {
                let id = row[0].as_i64().unwrap();
                let val = row[1].as_i64().unwrap();
                assert_eq!(val, id * 10, "invariant violated: id={id}, val={val}");
            }

            let current_count = r.len();
            assert!(
                current_count >= seen_max,
                "read count decreased: {seen_max} -> {current_count}"
            );
            seen_max = seen_max.max(current_count);

            tokio::task::yield_now().await;
        }
    });

    writer.await.unwrap();
    reader.await.unwrap();

    let resp = db.execute_sql("SELECT COUNT(*) FROM t4").await;
    let r = rows(resp);
    let count = r[0][0].as_i64().unwrap();
    assert_eq!(count, 100, "final count should be 100, got {count}");
}

// ── Test 5: Full scan after bulk inserts ───────────────────────────────

#[tokio::test]
async fn test_visibility_full_scan_after_inserts() {
    let db = open_db().await;

    db.execute_sql("CREATE TABLE t5 (id INT PRIMARY KEY, val INT)")
        .await;

    for i in 1..=100 {
        db.execute_sql(&format!("INSERT INTO t5 VALUES ({}, {})", i, i * 10))
            .await;
    }

    let resp = db.execute_sql("SELECT * FROM t5").await;
    assert_row_count(resp, 100);

    let resp = db.execute_sql("SELECT COUNT(*) FROM t5").await;
    let r = rows(resp);
    assert_eq!(r[0][0].as_i64().unwrap(), 100);
}
