use rtsql::database::Database;
use std::time::Instant;

#[tokio::test]
async fn cache_perf_measurement() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("perf.db");
    let db = Database::open(&db_path).await.unwrap();

    db.execute_sql("CREATE TABLE bench (id INT PRIMARY KEY, name STRING, value INT)")
        .await;
    for i in 0..100 {
        db.execute_sql(&format!("INSERT INTO bench VALUES ({}, 'user_{}', {})", i, i, i * 10))
            .await;
    }

    let sql = "SELECT * FROM bench WHERE id = 42";

    // Warm up
    for _ in 0..10 {
        db.execute_sql(sql).await;
    }

    // Measure cached (same SQL repeated)
    let n = 1000;
    let start = Instant::now();
    for _ in 0..n {
        db.execute_sql(sql).await;
    }
    let cached_us = start.elapsed().as_micros() / n as u128;

    // Clear cache by DDL, measure uncached
    db.execute_sql("CREATE TABLE _dummy (x INT)").await;
    let start = Instant::now();
    for _ in 0..n {
        db.execute_sql(sql).await;
    }
    let uncached_us = start.elapsed().as_micros() / n as u128;

    println!("PK lookup (same SQL x{}):", n);
    println!("  Cached:   {} µs/call", cached_us);
    println!("  Uncached: {} µs/call", uncached_us);
    if uncached_us > 0 && cached_us > 0 {
        println!("  Cache speedup: {:.1}x", uncached_us as f64 / cached_us as f64);
    }

    // Measure with different SQLs (low cache hit)
    let start = Instant::now();
    for i in 0..n {
        let id = (i % 100) as i64;
        db.execute_sql(&format!("SELECT * FROM bench WHERE id = {}", id)).await;
    }
    let diff_sql_us = start.elapsed().as_micros() / n as u128;
    println!("  Diff SQL: {} µs/call (100 unique SQLs)", diff_sql_us);

    // Measure full scan
    let start = Instant::now();
    for _ in 0..100 {
        db.execute_sql("SELECT * FROM bench").await;
    }
    let scan_us = start.elapsed().as_micros() / 100 as u128;
    println!("  Full scan: {} µs/call", scan_us);

    std::mem::forget(dir);
}