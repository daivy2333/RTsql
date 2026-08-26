//! Integration tests for MS06-T02 PlanCache DashMap + SQL normalization.

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::plan_cache::normalize_sql_key;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_case_variant_hits_cache() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)")
        .await;
    db.execute_sql("INSERT INTO t (id, name) VALUES (1, 'alice')")
        .await;

    db.execute_sql("SELECT * FROM t WHERE id = 1").await;
    let n1 = db.plan_cache_len();
    assert!(n1 > 0, "first SELECT should populate the cache");

    let r = db.execute_sql("select * from t where id = 1").await;
    assert!(
        matches!(r, Response::QueryResult { .. }),
        "lowercase variant should return a result"
    );
    assert_eq!(
        db.plan_cache_len(),
        n1,
        "lowercase variant should hit the same cache entry"
    );
}

#[tokio::test]
async fn test_whitespace_variant_hits_cache() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    db.execute_sql("INSERT INTO t (id) VALUES (1)").await;

    db.execute_sql("SELECT * FROM t").await;
    let n1 = db.plan_cache_len();
    assert!(n1 > 0);

    let r = db.execute_sql("SELECT\n*\nFROM t").await;
    assert!(matches!(r, Response::QueryResult { .. }));
    assert_eq!(
        db.plan_cache_len(),
        n1,
        "whitespace variant should hit the same cache entry"
    );
}

#[tokio::test]
async fn test_string_literal_case_does_not_hit() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)")
        .await;
    db.execute_sql("INSERT INTO t (id, name) VALUES (1, 'alice')")
        .await;

    db.execute_sql("SELECT * FROM t WHERE name = 'alice'").await;
    let n1 = db.plan_cache_len();
    assert!(n1 > 0);

    let r = db.execute_sql("SELECT * FROM t WHERE name = 'Alice'").await;
    assert!(matches!(r, Response::QueryResult { .. }));
    // 'Alice' 与 'alice' 字符串字面量大小写不同 → 规范化后 key 不同 → cache miss → size +1
    assert_eq!(
        db.plan_cache_len(),
        n1 + 1,
        "string literal case difference should miss the cache"
    );
}

#[tokio::test]
async fn test_concurrent_hits_do_not_block_runtime() {
    use std::time::Instant;
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(&dir.path().join("test.db")).await.unwrap());
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    for i in 0..50 {
        db.execute_sql(&format!("INSERT INTO t (id, v) VALUES ({}, {})", i, i))
            .await;
    }
    // 预热 cache
    db.execute_sql("SELECT * FROM t WHERE id = 1").await;

    let start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..100 {
        let db = db.clone();
        handles.push(tokio::spawn(async move {
            db.execute_sql("SELECT * FROM t WHERE id = 1").await
        }));
    }
    for h in handles {
        let r = h.await.unwrap();
        assert!(matches!(r, Response::QueryResult { .. }));
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 5,
        "100 concurrent SELECTs should finish in <5s, took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_dml_still_not_cached() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    db.execute_sql("INSERT INTO t (id) VALUES (1)").await;
    assert_eq!(
        db.plan_cache_len(),
        0,
        "INSERT should not be cached (DML is excluded by is_cacheable)"
    );
}

#[tokio::test]
async fn test_ddl_still_clears_cache() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("test.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await;
    db.execute_sql("SELECT * FROM t").await;
    assert!(db.plan_cache_len() > 0, "SELECT should populate the cache");
    db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY)").await;
    assert_eq!(
        db.plan_cache_len(),
        0,
        "CREATE TABLE (DDL) should clear the cache"
    );
}

#[test]
fn normalize_module_function_public() {
    assert_eq!(normalize_sql_key("SELECT 1"), "select 1");
    assert_eq!(
        normalize_sql_key("  SELECT   1  "),
        "select 1",
        "normalize_sql_key should fold case + collapse + trim whitespace"
    );
}
