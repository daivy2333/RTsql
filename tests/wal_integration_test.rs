use rtsql::database::Database;
use rtsql::network::protocol::Response;
use tempfile::TempDir;

#[tokio::test]
async fn test_wal_multiple_inserts_group_commit() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test_gc.db");

    let db = Database::open(&db_path).await.unwrap();
    db.execute_sql("CREATE TABLE t2 (id INT, val TEXT)").await;

    for i in 0..10 {
        db.execute_sql(&format!("INSERT INTO t2 VALUES ({}, 'row_{}')", i, i))
            .await;
    }

    let result = db.execute_sql("SELECT * FROM t2").await;
    match &result {
        Response::QueryResult { rows } => assert_eq!(rows.len(), 10),
        other => panic!("Expected QueryResult, got {:?}", other),
    }
}

#[tokio::test]
async fn test_wal_txn_begin_commit_records() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test_txn.db");

    let db = Database::open(&db_path).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await;

    // Single insert
    db.execute_sql("INSERT INTO t VALUES (1, 100)").await;

    let result = db.execute_sql("SELECT * FROM t").await;
    match &result {
        Response::QueryResult { rows } => assert_eq!(rows.len(), 1),
        other => panic!("Expected QueryResult, got {:?}", other),
    }

    // Multiple inserts
    db.execute_sql("INSERT INTO t VALUES (2, 200)").await;
    db.execute_sql("INSERT INTO t VALUES (3, 300)").await;

    let result2 = db.execute_sql("SELECT * FROM t").await;
    match &result2 {
        Response::QueryResult { rows } => assert_eq!(rows.len(), 3),
        other => panic!("Expected QueryResult, got {:?}", other),
    }
}
