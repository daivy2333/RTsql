//! Integration tests for MS07-T06: predicate & LIMIT pushdown into DataScan
//! (R3/S3.1–S3.5, Iteration 002)
//!
//! T1 witness: non-PK WHERE predicates are planned as inline `DataScan`
//! filtering (no `Filter` node), OR predicates keep the `Filter` wrapper,
//! PK paths are unchanged, and query results are identical to the
//! pre-pushdown behavior (numeric / string / AND-chain / NULL boundaries).

use rtsql::database::Database;
use rtsql::executor::PhysicalPlan;
use rtsql::network::protocol::Response;
use rtsql::pipeline::{parse_stage, plan_stage};
use serde_json::json;
use tempfile::tempdir;

const SETUP_ROWS: i32 = 5;

async fn open_db() -> (Database, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("pushdown.db"))
        .await
        .unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, a INT, b VARCHAR)").await;
    // id: a, b — covers numeric / string boundaries and one NULL in `a`.
    for (id, a, b) in [
        (1, "10", "'x'"),
        (2, "20", "'y'"),
        (3, "30", "'x'"),
        (4, "5", "'x'"),
        (5, "NULL", "'z'"),
    ] {
        exec_ok(
            &db,
            &format!("INSERT INTO t (id, a, b) VALUES ({id}, {a}, {b})"),
        )
        .await;
    }
    assert_eq!(SETUP_ROWS, 5);
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

async fn plan_of(db: &Database, sql: &str) -> PhysicalPlan {
    let stmts = parse_stage(sql).await.expect("parse should succeed");
    let stmt = stmts.first().expect("one statement");
    plan_stage(db, sql, stmt, false)
        .await
        .expect("plan should succeed")
}

// ---------------------------------------------------------------------------
// T1 (a): pushed-down predicates return exactly the pre-pushdown row sets
// (queries use SELECT *, whose identity projection leaves the full-schema
// row shape unchanged by MS10-T01 Iter001 true projection)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_pk_where_numeric_rows_identical() {
    let (db, _dir) = open_db().await;
    // a > 15 → ids 2, 3; the NULL-a row (id 5) must be excluded (NULL > 15
    // is false under SQL semantics, evaluated by the same ComparisonPredicate
    // that Filter used pre-pushdown).
    let rows = query_rows(db.execute_sql("SELECT * FROM t WHERE a > 15").await);
    assert_eq!(
        rows,
        vec![
            vec![json!(2), json!(20), json!("y")],
            vec![json!(3), json!(30), json!("x")],
        ]
    );
}

#[tokio::test]
async fn non_pk_where_string_rows_identical() {
    let (db, _dir) = open_db().await;
    let rows = query_rows(db.execute_sql("SELECT * FROM t WHERE b = 'x'").await);
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!(10), json!("x")],
            vec![json!(3), json!(30), json!("x")],
            vec![json!(4), json!(5), json!("x")],
        ]
    );
}

#[tokio::test]
async fn non_pk_where_and_chain_rows_identical() {
    let (db, _dir) = open_db().await;
    let rows = query_rows(
        db.execute_sql("SELECT * FROM t WHERE a >= 10 AND a <= 30")
            .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!(10), json!("x")],
            vec![json!(2), json!(20), json!("y")],
            vec![json!(3), json!(30), json!("x")],
        ]
    );

    let rows = query_rows(
        db.execute_sql("SELECT * FROM t WHERE b = 'x' AND a > 6")
            .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!(10), json!("x")],
            vec![json!(3), json!(30), json!("x")],
        ]
    );
}

#[tokio::test]
async fn null_comparison_excludes_row_under_pushdown() {
    let (db, _dir) = open_db().await;
    // NULL != 10 is false (NULL-aware ComparisonPredicate) — the pushed-down
    // path must keep this semantics, so id 5 is absent.
    let rows = query_rows(db.execute_sql("SELECT * FROM t WHERE a != 10").await);
    assert_eq!(
        rows,
        vec![
            vec![json!(2), json!(20), json!("y")],
            vec![json!(3), json!(30), json!("x")],
            vec![json!(4), json!(5), json!("x")],
        ]
    );
}

// ---------------------------------------------------------------------------
// T1 (b): plan shape — predicate lives inside DataScan, no Filter node
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_pk_where_plan_pushes_predicate_into_datascand() {
    let (db, _dir) = open_db().await;
    let plan = plan_of(&db, "SELECT id FROM t WHERE a > 15").await;
    match plan {
        PhysicalPlan::DataScan(node) => {
            assert_eq!(node.table_name, "t");
            assert!(
                node.predicate.is_some(),
                "pushdown-eligible WHERE must carry the predicate inside DataScan"
            );
            assert_eq!(node.scan_cap, None);
        }
        other => panic!("Expected DataScan with pushed predicate, got {other:?}"),
    }
}

#[tokio::test]
async fn or_where_keeps_filter_over_datascand() {
    let (db, _dir) = open_db().await;
    let plan = plan_of(&db, "SELECT id FROM t WHERE a < 8 OR b = 'y'").await;
    match plan {
        PhysicalPlan::Filter(node) => {
            assert_eq!(node.table_name, "t");
            match node.input.as_ref() {
                PhysicalPlan::DataScan(inner) => {
                    assert!(inner.predicate.is_none(), "OR predicate must not be pushed");
                }
                other => panic!("Expected Filter over DataScan, got {other:?}"),
            }
        }
        other => panic!("Expected Filter to remain for OR predicate, got {other:?}"),
    }
    // Result equality for the retained Filter path.
    let rows = query_rows(
        db.execute_sql("SELECT * FROM t WHERE a < 8 OR b = 'y'")
            .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!(2), json!(20), json!("y")], // b = 'y'
            vec![json!(4), json!(5), json!("x")],  // a = 5 < 8
        ]
    );
}

// ---------------------------------------------------------------------------
// T1 (c): PK paths are unchanged (S3.4)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn simple_pk_equality_still_index_scan() {
    let (db, _dir) = open_db().await;
    let plan = plan_of(&db, "SELECT id FROM t WHERE id = 2").await;
    assert!(
        matches!(plan, PhysicalPlan::IndexScan(_)),
        "simple PK equality must stay IndexScan, got {plan:?}"
    );
}

#[tokio::test]
async fn complex_pk_equality_still_filter_over_scan() {
    let (db, _dir) = open_db().await;
    let plan = plan_of(&db, "SELECT id FROM t WHERE id = 2 AND a > 5").await;
    match plan {
        PhysicalPlan::Filter(node) => match node.input.as_ref() {
            PhysicalPlan::Scan(_) => {}
            other => panic!("Expected Filter over Scan for complex PK, got {other:?}"),
        },
        other => panic!("Expected Filter for complex PK WHERE, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// T2 (a): LIMIT pushdown returns exactly the pre-pushdown row sets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn limit_rows_identical() {
    let (db, _dir) = open_db().await;
    let rows = query_rows(db.execute_sql("SELECT * FROM t LIMIT 2").await);
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!(10), json!("x")],
            vec![json!(2), json!(20), json!("y")],
        ]
    );

    let rows = query_rows(db.execute_sql("SELECT * FROM t LIMIT 2 OFFSET 1").await);
    assert_eq!(
        rows,
        vec![
            vec![json!(2), json!(20), json!("y")],
            vec![json!(3), json!(30), json!("x")],
        ]
    );
}

#[tokio::test]
async fn limit_boundaries_rows_identical() {
    let (db, _dir) = open_db().await;
    // LIMIT beyond row count → all rows.
    let rows = query_rows(db.execute_sql("SELECT * FROM t LIMIT 100").await);
    assert_eq!(rows.len(), 5);

    // OFFSET beyond row count → empty.
    let rows = query_rows(db.execute_sql("SELECT * FROM t LIMIT 10 OFFSET 100").await);
    assert_eq!(rows, Vec::<Vec<serde_json::Value>>::new());

    // LIMIT 0 → empty.
    let rows = query_rows(db.execute_sql("SELECT * FROM t LIMIT 0").await);
    assert_eq!(rows, Vec::<Vec<serde_json::Value>>::new());

    // OFFSET lands on the last row.
    let rows = query_rows(db.execute_sql("SELECT * FROM t LIMIT 10 OFFSET 4").await);
    assert_eq!(rows, vec![vec![json!(5), json!(null), json!("z")]]);
}

#[tokio::test]
async fn where_and_limit_cap_counts_passed_rows() {
    let (db, _dir) = open_db().await;
    // Predicate-passing rows are ids 1, 2, 3 — the cap counts only rows that
    // passed the inline predicate, so LIMIT 2 yields ids 1, 2 (not fewer).
    let rows = query_rows(db.execute_sql("SELECT * FROM t WHERE a > 6 LIMIT 2").await);
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!(10), json!("x")],
            vec![json!(2), json!(20), json!("y")],
        ]
    );
}

// ---------------------------------------------------------------------------
// T2 (b): plan shape — cap lives inside DataScan, top-level Limit kept
// ---------------------------------------------------------------------------

#[tokio::test]
async fn limit_plan_pushes_cap_into_datascand() {
    let (db, _dir) = open_db().await;
    let plan = plan_of(&db, "SELECT * FROM t LIMIT 2").await;
    match plan {
        PhysicalPlan::Limit(node) => {
            assert_eq!(node.limit, 2);
            assert_eq!(node.offset, 0);
            match node.input.as_ref() {
                PhysicalPlan::DataScan(inner) => {
                    assert_eq!(inner.scan_cap, Some(2), "cap = offset + limit");
                }
                other => panic!("Expected Limit over DataScan, got {other:?}"),
            }
        }
        other => panic!("Expected top-level Limit node, got {other:?}"),
    }

    let plan = plan_of(&db, "SELECT * FROM t LIMIT 2 OFFSET 1").await;
    match plan {
        PhysicalPlan::Limit(node) => {
            assert_eq!(node.limit, 2);
            assert_eq!(node.offset, 1);
            match node.input.as_ref() {
                PhysicalPlan::DataScan(inner) => {
                    assert_eq!(inner.scan_cap, Some(3), "cap = offset + limit");
                }
                other => panic!("Expected Limit over DataScan, got {other:?}"),
            }
        }
        other => panic!("Expected top-level Limit node, got {other:?}"),
    }

    // LIMIT 0: the scan itself must be immediately Done (cap Some(0)).
    let plan = plan_of(&db, "SELECT * FROM t LIMIT 0").await;
    match plan {
        PhysicalPlan::Limit(node) => match node.input.as_ref() {
            PhysicalPlan::DataScan(inner) => assert_eq!(inner.scan_cap, Some(0)),
            other => panic!("Expected Limit over DataScan, got {other:?}"),
        },
        other => panic!("Expected top-level Limit node, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// T2 (c)/(d): eligibility — Sort / Aggregate / Filter chains are not capped
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sort_limit_no_scan_cap() {
    let (db, _dir) = open_db().await;
    // Explicit projection so the sort column resolves (SELECT * + ORDER BY
    // is a known pre-existing no-sort case — see Act Response).
    let sql = "SELECT id, a, b FROM t ORDER BY id DESC LIMIT 2";
    let plan = plan_of(&db, sql).await;
    match plan {
        PhysicalPlan::Limit(node) => match node.input.as_ref() {
            PhysicalPlan::Sort(sort) => match sort.input.as_ref() {
                PhysicalPlan::DataScan(inner) => {
                    assert_eq!(inner.scan_cap, None, "must not cap below a Sort");
                }
                other => panic!("Expected Sort over DataScan, got {other:?}"),
            },
            other => panic!("Expected Sort directly under Limit, got {other:?}"),
        },
        other => panic!("Expected top-level Limit node, got {other:?}"),
    }
    // Rows: sorted DESC by id, take first 2.
    let rows = query_rows(db.execute_sql(sql).await);
    assert_eq!(
        rows,
        vec![
            vec![json!(5), json!(null), json!("z")],
            vec![json!(4), json!(5), json!("x")],
        ]
    );
}

#[tokio::test]
async fn aggregate_limit_no_scan_cap() {
    let (db, _dir) = open_db().await;
    let plan = plan_of(&db, "SELECT COUNT(*) FROM t LIMIT 1").await;
    match plan {
        PhysicalPlan::Limit(node) => match node.input.as_ref() {
            PhysicalPlan::Aggregate(agg) => match agg.input.as_ref() {
                PhysicalPlan::DataScan(inner) => {
                    assert_eq!(inner.scan_cap, None, "must not cap below an Aggregate");
                }
                other => panic!("Expected Aggregate over DataScan, got {other:?}"),
            },
            other => panic!("Expected Aggregate directly under Limit, got {other:?}"),
        },
        other => panic!("Expected top-level Limit node, got {other:?}"),
    }
    // The aggregate must see the full input (COUNT = 5), proving the cap
    // was not pushed through it.
    let rows = query_rows(db.execute_sql("SELECT COUNT(*) FROM t LIMIT 1").await);
    assert_eq!(rows, vec![vec![json!(5)]]);
}

#[tokio::test]
async fn or_filter_limit_no_cap_under_filter() {
    let (db, _dir) = open_db().await;
    // A remaining Filter (OR predicate) is not row-transparent: pushing a cap
    // below it could truncate the Filter's input and change results.
    let plan = plan_of(&db, "SELECT * FROM t WHERE a < 8 OR b = 'y' LIMIT 1").await;
    match plan {
        PhysicalPlan::Limit(node) => match node.input.as_ref() {
            PhysicalPlan::Filter(filter) => match filter.input.as_ref() {
                PhysicalPlan::DataScan(inner) => {
                    assert_eq!(inner.scan_cap, None, "must not cap below a Filter");
                    assert!(inner.predicate.is_none());
                }
                other => panic!("Expected Filter over DataScan, got {other:?}"),
            },
            other => panic!("Expected Filter directly under Limit, got {other:?}"),
        },
        other => panic!("Expected top-level Limit node, got {other:?}"),
    }
    let rows = query_rows(
        db.execute_sql("SELECT * FROM t WHERE a < 8 OR b = 'y' LIMIT 1")
            .await,
    );
    assert_eq!(rows, vec![vec![json!(2), json!(20), json!("y")]]);
}
