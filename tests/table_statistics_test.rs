//! 会话内基表统计：`ANALYZE TABLE` 的计数、拒绝和版本戳。

use rtsql::database::Database;
use rtsql::pipeline::{plan_stage_routed, AttachRoute, RoutedTables};
use rtsql::{parse_sql, SqlError, SqlOutcome, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

fn count_rows() -> Vec<Vec<Value>> {
    vec![
        vec![
            Value::String("id".into()),
            Value::Int(2),
            Value::Int(0),
            Value::Int(2),
        ],
        vec![
            Value::String("name".into()),
            Value::Int(2),
            Value::Int(1),
            Value::Int(1),
        ],
    ]
}

fn zero_rows() -> Vec<Vec<Value>> {
    vec![
        vec![
            Value::String("id".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
        ],
        vec![
            Value::String("name".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
        ],
    ]
}

fn assert_table(outcome: SqlOutcome, expected: Vec<Vec<Value>>) {
    let SqlOutcome::Rows { columns, rows } = outcome else {
        panic!("expected rows, got {outcome:?}");
    };
    assert_eq!(
        columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        vec!["column", "row_count", "null_count", "ndv"]
    );
    assert_eq!(rows, expected);
}

async fn open_db() -> (Database, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    (db, dir)
}

async fn exec_ok(db: &Database, sql: &str) {
    db.execute_sql(sql)
        .await
        .unwrap_or_else(|error| panic!("{sql} failed: {error}"));
}

async fn seed(db: &Database) {
    exec_ok(db, "CREATE TABLE t (id INT, name STRING)").await;
    exec_ok(db, "INSERT INTO t VALUES (1, 'a')").await;
    exec_ok(db, "INSERT INTO t VALUES (2, NULL)").await;
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn dir_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

async fn assert_rejected(db: &Database, sql: &str, message: &str, sqlstate: &str) {
    let version = db.schema_version();
    let cache = db.plan_cache_len();
    let error = db
        .execute_sql(sql)
        .await
        .expect_err("statement should fail");
    assert_eq!(error.message, message, "{sql}");
    assert_eq!(error.sqlstate.as_deref(), Some(sqlstate), "{sql}");
    assert_eq!(
        db.schema_version(),
        version,
        "{sql} must not bump schema_version"
    );
    assert_eq!(
        db.plan_cache_len(),
        cache,
        "{sql} must not clear the plan cache"
    );
}

#[tokio::test]
async fn analyze_counts_visible_rows() {
    let (db, _dir) = open_db().await;
    seed(&db).await;
    let outcome = db.execute_sql("ANALYZE TABLE t").await;
    let Ok(outcome) = outcome else {
        panic!("expected the two count rows, got {outcome:?}");
    };
    assert_table(outcome, count_rows());
}

#[tokio::test]
async fn analyze_compute_statistics_matches_visible_rows() {
    let (db, _dir) = open_db().await;
    seed(&db).await;
    let outcome = db
        .execute_sql("ANALYZE TABLE t COMPUTE STATISTICS")
        .await
        .unwrap();
    assert_table(outcome, count_rows());
}

#[tokio::test]
async fn analyze_empty_table_reports_zeros() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT, name STRING)").await;
    let outcome = db.execute_sql("ANALYZE TABLE t").await.unwrap();
    assert_table(outcome, zero_rows());
}

#[tokio::test]
async fn analyze_leaves_main_file_and_wal_lengths_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    {
        let db = Database::open(&path).await.unwrap();
        seed(&db).await;
        db.close().await.unwrap();
    }
    let db = Database::open(&path).await.unwrap();
    let main_before = file_len(&path);
    let wal_before = file_len(&path.with_extension("wal"));
    let outcome = db.execute_sql("ANALYZE TABLE t").await.unwrap();
    assert_table(outcome, count_rows());
    assert_eq!(file_len(&path), main_before);
    assert_eq!(file_len(&path.with_extension("wal")), wal_before);
}

#[tokio::test]
async fn analyze_readonly_does_not_create_or_grow_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    {
        let db = Database::open(&path).await.unwrap();
        seed(&db).await;
        db.close().await.unwrap();
    }
    let files_before = dir_files(dir.path());
    let main_before = file_len(&path);
    let wal_before = file_len(&path.with_extension("wal"));
    assert!(
        path.exists(),
        "readonly open must not be the creator of the database file"
    );
    let db = Database::open_readonly(&path).await.unwrap();
    let outcome = db.execute_sql("ANALYZE TABLE t").await.unwrap();
    assert_table(outcome, count_rows());
    assert_eq!(file_len(&path), main_before);
    assert_eq!(file_len(&path.with_extension("wal")), wal_before);
    assert_eq!(dir_files(dir.path()), files_before);
}

#[tokio::test]
async fn analyze_memory_database_counts_visible_rows() {
    let db = Database::open_in_memory().await.unwrap();
    seed(&db).await;
    let outcome = db.execute_sql("ANALYZE TABLE t").await.unwrap();
    assert_table(outcome, count_rows());
    assert_eq!(db.path(), Path::new(":memory:"));
}

#[tokio::test]
async fn analyze_rejections_keep_version_and_cache() {
    let (db, _dir) = open_db().await;
    seed(&db).await;
    exec_ok(&db, "CREATE VIEW v AS SELECT * FROM t").await;
    exec_ok(&db, "SELECT * FROM t").await;
    assert!(db.plan_cache_len() > 0);

    assert_rejected(
        &db,
        "ANALYZE TABLE missing",
        "Plan error: Table not found: missing",
        "42P01",
    )
    .await;
    assert_rejected(
        &db,
        "ANALYZE TABLE v",
        "Plan error: ANALYZE requires a base table: v",
        "0A000",
    )
    .await;
    assert_rejected(
        &db,
        "ANALYZE TABLE t NOSCAN",
        "Plan error: ANALYZE does not support NOSCAN",
        "0A000",
    )
    .await;
    assert_rejected(
        &db,
        "ANALYZE TABLE t PARTITION (id)",
        "Plan error: ANALYZE does not support PARTITION",
        "0A000",
    )
    .await;
    assert_rejected(
        &db,
        "ANALYZE TABLE t FOR COLUMNS",
        "Plan error: ANALYZE does not support FOR COLUMNS",
        "0A000",
    )
    .await;
    assert_rejected(
        &db,
        "ANALYZE TABLE t CACHE METADATA",
        "Plan error: ANALYZE does not support CACHE METADATA",
        "0A000",
    )
    .await;
}

#[tokio::test]
async fn analyze_attached_table_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let main = Database::open(&dir.path().join("main.db")).await.unwrap();
    let attached = Database::open(&dir.path().join("other.db")).await.unwrap();
    exec_ok(&attached, "CREATE TABLE t (id INT)").await;
    let version = main.schema_version();
    let route = AttachRoute::new(HashMap::from([("o".to_string(), Arc::new(attached))]));
    let sql = "ANALYZE TABLE o.t";
    let statements = parse_sql(sql).unwrap();
    let error = plan_stage_routed(
        &main,
        sql,
        &statements[0],
        false,
        Some(&route),
        &mut RoutedTables::new(),
    )
    .await
    .expect_err("attached analyze should fail");
    assert_eq!(
        error.to_string(),
        "Plan error: ANALYZE does not support attached tables: o.t"
    );
    assert_eq!(error.sqlstate, "0A000");
    assert_eq!(main.schema_version(), version);
}

#[tokio::test]
async fn analyze_inside_transaction_is_rejected_without_prefix() {
    let (db, _dir) = open_db().await;
    seed(&db).await;
    let version = db.schema_version();
    let tx = db.begin().await.unwrap();
    let error = db
        .execute_in_tx("ANALYZE TABLE t", &tx)
        .await
        .expect_err("analyze inside a transaction should fail");
    assert_eq!(
        error,
        SqlError {
            message: "ANALYZE is not supported inside a transaction".to_string(),
            sqlstate: Some("0A000".to_string()),
        }
    );
    assert_eq!(db.schema_version(), version);
}

#[tokio::test]
async fn analyze_one_byte_budget_does_not_save() {
    let (db, _dir) = open_db().await;
    seed(&db).await;
    let version = db.schema_version();
    db.set_query_memory_limit(1);
    let error = db
        .execute_sql("ANALYZE TABLE t")
        .await
        .expect_err("one-byte budget should fail");
    assert_eq!(
        error.message,
        "Execution error: query memory limit exceeded (limit 1 bytes)"
    );
    assert_eq!(error.sqlstate.as_deref(), Some("53200"));
    assert_eq!(db.schema_version(), version);
    db.set_query_memory_limit(0);
    let outcome = db.execute_sql("ANALYZE TABLE t").await.unwrap();
    assert_table(outcome, count_rows());
}

#[tokio::test]
async fn analyze_clears_plan_cache_and_bumps_schema_version() {
    let (db, _dir) = open_db().await;
    seed(&db).await;
    exec_ok(&db, "SELECT * FROM t").await;
    assert!(db.plan_cache_len() > 0);
    let version = db.schema_version();
    let outcome = db.execute_sql("ANALYZE TABLE t").await.unwrap();
    assert_table(outcome, count_rows());
    assert_eq!(db.plan_cache_len(), 0);
    assert_eq!(db.schema_version(), version + 1);
}

#[tokio::test]
async fn analyze_keeps_written_join_order_in_explain() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE left_table (id INT, name STRING)").await;
    exec_ok(&db, "CREATE TABLE right_table (id INT, name STRING)").await;
    let sql = "EXPLAIN SELECT * FROM left_table JOIN right_table \
               ON left_table.id = right_table.id ORDER BY left_table.id";
    let before = explain_lines(&db, sql).await;
    exec_ok(&db, "ANALYZE TABLE left_table").await;
    let after = explain_lines(&db, sql).await;
    assert_eq!(after, before);
    assert!(after.iter().all(|line| !line.contains("JoinOrder")));
    let scan = after
        .iter()
        .find(|line| line.contains("Scan "))
        .expect("explain has a scan");
    assert!(
        scan.contains("left_table") && !scan.contains("right_table"),
        "first scan should stay the written left table, got {scan}"
    );
}

async fn explain_lines(db: &Database, sql: &str) -> Vec<String> {
    match db.execute_sql(sql).await.unwrap() {
        SqlOutcome::Rows { rows, .. } => rows
            .into_iter()
            .map(|row| match &row[0] {
                Value::String(text) => text.clone(),
                other => panic!("expected text, got {other:?}"),
            })
            .collect(),
        other => panic!("expected explain rows, got {other:?}"),
    }
}
