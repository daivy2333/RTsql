use rtsql::database::Database;
use std::path::{Path, PathBuf};

/// 创建临时目录并打开数据库
pub async fn setup_db() -> (PathBuf, Database) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("bench.db");
    let db = Database::open(&db_path).await.unwrap();
    // Leak TempDir so it stays alive for the benchmark duration
    std::mem::forget(dir);
    (db_path, db)
}

/// 清理数据库文件
pub fn cleanup_db(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db.wal"));
    let _ = std::fs::remove_file(path.with_extension("db.checkpoint"));
}

/// 创建标准测试表 (id INT PK, name STRING, value INT)
pub async fn create_test_table(db: &Database) {
    db.execute_sql("CREATE TABLE bench (id INT PRIMARY KEY, name STRING, value INT)")
        .await;
}

/// 插入 n 行数据到 bench 表
pub async fn insert_rows(db: &Database, start: i64, n: i64) {
    for i in start..start + n {
        db.execute_sql(&format!(
            "INSERT INTO bench VALUES ({}, 'user_{}', {})",
            i,
            i,
            i * 10
        ))
        .await;
    }
}

/// 创建 JOIN 测试表 (orders + customers)
#[allow(dead_code)] // kept as a shared fixture for future join benchmarks
pub async fn create_join_tables(db: &Database) {
    db.execute_sql("CREATE TABLE customers (id INT PRIMARY KEY, name STRING)")
        .await;
    db.execute_sql("CREATE TABLE orders (id INT PRIMARY KEY, customer_id INT, amount INT)")
        .await;
}

/// 插入 JOIN 测试数据
#[allow(dead_code)] // kept as a shared fixture for future join benchmarks
pub async fn insert_join_data(db: &Database, n: i64) {
    for i in 0..n {
        db.execute_sql(&format!(
            "INSERT INTO customers VALUES ({}, 'customer_{}')",
            i, i
        ))
        .await;
        db.execute_sql(&format!(
            "INSERT INTO orders VALUES ({}, {}, {})",
            i,
            i,
            i * 100
        ))
        .await;
    }
}
