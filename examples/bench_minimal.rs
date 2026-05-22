use rtsql::database::Database;

#[tokio::main]
async fn main() {
    // Enable profiling
    std::env::set_var("RTSQL_PROFILING", "1");

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("bench.db");
    let db = Database::open(&db_path).await.unwrap();
    std::mem::forget(dir);

    db.execute_sql("CREATE TABLE bench (id INTEGER PRIMARY KEY, val TEXT)").await;
    for i in 0..50i64 {
        db.execute_sql(&format!("INSERT INTO bench VALUES ({}, 'hello')", i)).await;
    }

    // Warm up (trigger plan cache)
    for _ in 0..50 {
        db.execute_sql("SELECT * FROM bench WHERE id = 42").await;
    }

    // Measure only 10 iterations to avoid excessive output
    let iterations = 10;
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        db.execute_sql("SELECT * FROM bench WHERE id = 42").await;
    }
    let elapsed = start.elapsed();
    let per_query = elapsed / iterations;
    println!("\n=== Summary ===");
    println!("PK lookup (avg): {:?}", per_query);
    println!("Total: {:?} for {} iterations", elapsed, iterations);
}