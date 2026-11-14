//! 公共行游标：分批交回、拒绝非行语句、未索取行不提前失败、带参与事务。

use rtsql::database::Database;
use rtsql::executor::Value;
use rtsql::storage::ColumnType as StorageColumnType;
use rtsql::{ColumnMeta, RowCursor, SqlOutcome};
use tempfile::TempDir;

async fn memory_db() -> Database {
    Database::open_in_memory().await.unwrap()
}

fn open_err(result: Result<RowCursor, rtsql::SqlError>) -> rtsql::SqlError {
    match result {
        Ok(_) => panic!("expected open to fail"),
        Err(error) => error,
    }
}

fn rows_of(outcome: SqlOutcome) -> (Vec<ColumnMeta>, Vec<Vec<Value>>) {
    match outcome {
        SqlOutcome::Rows { columns, rows } => (columns, rows),
        other => panic!("expected rows, got {other:?}"),
    }
}

async fn collect_batches(cursor: &mut RowCursor, max: usize) -> Vec<Vec<Value>> {
    let mut rows = Vec::new();
    loop {
        let batch = cursor.next_batch(max).await.unwrap();
        if batch.is_empty() {
            break;
        }
        assert!(batch.len() <= max);
        rows.extend(batch);
    }
    rows
}

#[tokio::test]
async fn batches_match_execute_sql() {
    let db = memory_db().await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
        .await
        .unwrap();
    for n in [10, 20, 30, 40, 50] {
        db.execute_sql(&format!("INSERT INTO t VALUES ({n}, {n})"))
            .await
            .unwrap();
    }

    let sql = "SELECT id, n FROM t";
    let (columns, rows) = rows_of(db.execute_sql(sql).await.unwrap());
    assert_eq!(
        columns
            .iter()
            .map(|column| column.column_type.clone())
            .collect::<Vec<_>>(),
        vec![Some(StorageColumnType::Int), Some(StorageColumnType::Int)]
    );

    let mut cursor = db.open_rows(sql).await.unwrap();
    assert_eq!(cursor.columns(), columns.as_slice());
    assert_eq!(collect_batches(&mut cursor, 2).await, rows);
}

#[tokio::test]
async fn empty_result_keeps_column_metadata() {
    let db = memory_db().await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
        .await
        .unwrap();

    let sql = "SELECT id, n FROM t";
    let (columns, rows) = rows_of(db.execute_sql(sql).await.unwrap());
    assert!(rows.is_empty());

    let mut cursor = db.open_rows(sql).await.unwrap();
    assert_eq!(cursor.columns(), columns.as_slice());
    assert!(cursor.next_batch(2).await.unwrap().is_empty());
}

#[tokio::test]
async fn expression_column_stays_unknown_before_fetch() {
    let db = memory_db().await;
    db.execute_sql("CREATE TABLE t (a INT PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap();

    let sql = "SELECT a + 1 FROM t";
    let (columns, _) = rows_of(db.execute_sql(sql).await.unwrap());
    assert_eq!(columns[0].column_type, Some(StorageColumnType::Int));

    let cursor = db.open_rows(sql).await.unwrap();
    assert_eq!(cursor.columns().len(), 1);
    assert_eq!(cursor.columns()[0].name, columns[0].name);
    assert_eq!(cursor.columns()[0].column_type, None);
}

#[tokio::test]
async fn zero_batch_does_not_advance() {
    let db = memory_db().await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap();

    let mut cursor = db.open_rows("SELECT id FROM t").await.unwrap();
    assert!(cursor.next_batch(0).await.unwrap().is_empty());
    assert_eq!(
        cursor.next_batch(1).await.unwrap(),
        vec![vec![Value::Int(1)]]
    );
}

#[tokio::test]
async fn insert_is_rejected_without_writing() {
    let db = memory_db().await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap();

    let error = open_err(db.open_rows("INSERT INTO t VALUES (2)").await);
    assert_eq!(
        error.message,
        "streaming consumption requires a row-producing statement"
    );
    assert_eq!(error.sqlstate, None);

    let (_, rows) = rows_of(db.execute_sql("SELECT id FROM t").await.unwrap());
    assert_eq!(rows, vec![vec![Value::Int(1)]]);
}

#[tokio::test]
async fn readonly_create_table_keeps_readonly_rejection() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("ro.db");
    {
        let db = Database::open(&path).await.unwrap();
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
            .await
            .unwrap();
        db.close().await.unwrap();
    }
    let db = Database::open_readonly(&path).await.unwrap();
    let error = open_err(db.open_rows("CREATE TABLE u (id INT PRIMARY KEY)").await);
    assert_eq!(error.sqlstate.as_deref(), Some("25006"));

    let missing = db.execute_sql("SELECT * FROM u").await.unwrap_err();
    assert_eq!(missing.sqlstate.as_deref(), Some("42P01"));
}

#[tokio::test]
async fn parse_and_parameter_errors_match_whole_statement() {
    let db = memory_db().await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();

    let bad_sql = "SELEC 1";
    assert_eq!(
        open_err(db.open_rows(bad_sql).await),
        db.execute_sql(bad_sql).await.unwrap_err()
    );

    let parameterized = "SELECT id FROM t WHERE id = ?";
    assert_eq!(
        open_err(db.open_rows_with_params(parameterized, &[]).await),
        db.execute_with_params(parameterized, &[])
            .await
            .unwrap_err()
    );
}

async fn table_of_short_strings(db: &Database) {
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, s TEXT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1, '1')")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (2, '2')")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (3, 'nope')")
        .await
        .unwrap();
}

#[tokio::test]
async fn later_cast_failure_is_not_prefetched() {
    let db = memory_db().await;
    table_of_short_strings(&db).await;
    let sql = "SELECT CAST(s AS INTEGER) FROM t";
    let mut cursor = db.open_rows(sql).await.unwrap();

    assert_eq!(
        cursor.next_batch(1).await.unwrap(),
        vec![vec![Value::Int(1)]]
    );
    assert_eq!(
        cursor.next_batch(1).await.unwrap(),
        vec![vec![Value::Int(2)]]
    );
    let third = cursor.next_batch(1).await.unwrap_err();
    let whole = db.execute_sql(sql).await.unwrap_err();
    assert_eq!(third.message, whole.message);
    assert_eq!(third.sqlstate, whole.sqlstate);
    let again = cursor.next_batch(1).await.unwrap_err();
    assert_eq!(again, third);
}

#[tokio::test]
async fn execute_sql_cast_returns_no_rows() {
    let db = memory_db().await;
    table_of_short_strings(&db).await;
    let error = db
        .execute_sql("SELECT CAST(s AS INTEGER) FROM t")
        .await
        .unwrap_err();
    assert!(!error.message.is_empty());
}

#[tokio::test]
async fn dropping_cursor_leaves_table_unchanged() {
    let db = memory_db().await;
    table_of_short_strings(&db).await;
    let before = rows_of(db.execute_sql("SELECT id, s FROM t").await.unwrap()).1;

    let mut cursor = db.open_rows("SELECT s FROM t").await.unwrap();
    let _ = cursor.next_batch(1).await.unwrap();
    drop(cursor);

    let after = rows_of(db.execute_sql("SELECT id, s FROM t").await.unwrap()).1;
    assert_eq!(after, before);
}

#[tokio::test]
async fn integer_full_scan_succeeds_under_tiny_limit() {
    let db = memory_db().await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();
    for id in 1..=3 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({id})"))
            .await
            .unwrap();
    }
    db.set_query_memory_limit(16);
    let sql = "SELECT * FROM t";
    let (_, rows) = rows_of(db.execute_sql(sql).await.unwrap());
    assert_eq!(rows.len(), 3);
    let mut cursor = db.open_rows(sql).await.unwrap();
    assert_eq!(collect_batches(&mut cursor, 1).await, rows);
}

#[tokio::test]
async fn params_match_execute_with_params() {
    let db = memory_db().await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1, 10)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (2, 20)")
        .await
        .unwrap();

    let sql = "SELECT n FROM t WHERE id = ?";
    let params = [Value::Int(2)];
    let (_, rows) = rows_of(db.execute_with_params(sql, &params).await.unwrap());
    let mut cursor = db.open_rows_with_params(sql, &params).await.unwrap();
    assert_eq!(collect_batches(&mut cursor, 2).await, rows);
}

#[tokio::test]
async fn transaction_cursor_sees_uncommitted_row() {
    let db = memory_db().await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
        .await
        .unwrap();
    let tx = db.begin().await.unwrap();
    db.execute_in_tx("INSERT INTO t VALUES (1, 7)", &tx)
        .await
        .unwrap();

    let mut cursor = db.open_rows_in_tx("SELECT n FROM t", &tx).await.unwrap();
    assert_eq!(
        collect_batches(&mut cursor, 2).await,
        vec![vec![Value::Int(7)]]
    );
    drop(cursor);

    let active = db.transaction_manager.active_transactions().await;
    assert!(active.contains(&tx.id()));
    db.commit(tx).await.unwrap();
    let (_, committed) = rows_of(db.execute_sql("SELECT n FROM t").await.unwrap());
    assert_eq!(committed, vec![vec![Value::Int(7)]]);
}

#[tokio::test]
async fn dropping_cursor_still_allows_rollback() {
    let db = memory_db().await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
        .await
        .unwrap();
    let tx = db.begin().await.unwrap();
    db.execute_in_tx("INSERT INTO t VALUES (1, 7)", &tx)
        .await
        .unwrap();
    let mut cursor = db.open_rows_in_tx("SELECT n FROM t", &tx).await.unwrap();
    let _ = cursor.next_batch(1).await.unwrap();
    drop(cursor);

    db.rollback(tx).await.unwrap();
    let (_, rows) = rows_of(db.execute_sql("SELECT n FROM t").await.unwrap());
    assert!(rows.is_empty());
}

#[tokio::test]
async fn transaction_insert_is_rejected_without_writing() {
    let db = memory_db().await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
        .await
        .unwrap();
    let tx = db.begin().await.unwrap();
    let error = open_err(db.open_rows_in_tx("INSERT INTO t VALUES (1, 7)", &tx).await);
    assert_eq!(
        error.message,
        "streaming consumption requires a row-producing statement"
    );
    assert_eq!(error.sqlstate, None);

    let mut cursor = db.open_rows_in_tx("SELECT id FROM t", &tx).await.unwrap();
    assert!(cursor.next_batch(2).await.unwrap().is_empty());
    drop(cursor);
    db.rollback(tx).await.unwrap();
}

#[tokio::test]
async fn transaction_params_match_execute_in_tx() {
    let db = memory_db().await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, n INT)")
        .await
        .unwrap();
    let tx = db.begin().await.unwrap();
    db.execute_in_tx("INSERT INTO t VALUES (1, 7)", &tx)
        .await
        .unwrap();

    let sql = "SELECT n FROM t WHERE id = ?";
    let params = [Value::Int(1)];
    let (_, rows) = rows_of(
        db.execute_in_tx_with_params(sql, &params, &tx)
            .await
            .unwrap(),
    );
    let mut cursor = db
        .open_rows_in_tx_with_params(sql, &params, &tx)
        .await
        .unwrap();
    assert_eq!(collect_batches(&mut cursor, 2).await, rows);
    drop(cursor);
    db.rollback(tx).await.unwrap();
    let (_, after) = rows_of(db.execute_sql("SELECT n FROM t").await.unwrap());
    assert!(after.is_empty());
}
