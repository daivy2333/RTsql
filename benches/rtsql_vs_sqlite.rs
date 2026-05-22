use criterion::{criterion_group, criterion_main, Criterion};
use rusqlite::Connection;
use rtsql::database::Database;
use std::path::PathBuf;
use tokio::runtime::Runtime;

fn bench_rtsql_vs_sqlite_pk_lookup(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // Setup SQLite
    let sqlite_dir = tempfile::tempdir().unwrap();
    let sqlite_path = sqlite_dir.path().join("sqlite.db");
    let sqlite_conn = Connection::open(&sqlite_path).unwrap();
    sqlite_conn.execute(
        "CREATE TABLE bench (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)",
        [],
    )
    .unwrap();
    for i in 0..1000i64 {
        sqlite_conn.execute(
            "INSERT INTO bench VALUES (?1, ?2, ?3)",
            rusqlite::params![i, format!("user_{}", i), i * 10],
        )
        .unwrap();
    }
    std::mem::forget(sqlite_dir);

    // Setup RTsql
    let rtsql_dir = tempfile::tempdir().unwrap();
    let rtsql_path = rtsql_dir.path().join("rtsql.db");
    let rtsql_db = rt.block_on(async {
        let db = Database::open(&rtsql_path).await.unwrap();
        db.execute_sql("CREATE TABLE bench (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)")
            .await;
        for i in 0..1000i64 {
            db.execute_sql(&format!(
                "INSERT INTO bench VALUES ({}, 'user_{}', {})",
                i, i, i * 10
            ))
            .await;
        }
        db
    });
    std::mem::forget(rtsql_dir);

    // Benchmark SQLite
    let mut group = c.benchmark_group("pk_lookup_comparison");
    group.bench_function("sqlite", |b| {
        b.iter(|| {
            let mut stmt = sqlite_conn.prepare("SELECT * FROM bench WHERE id = 42").unwrap();
            let _ = stmt.query_row([], |row| {
                let id: i64 = row.get(0).unwrap();
                Ok(id)
            });
        });
    });

    // Benchmark RTsql
    group.bench_function("rtsql", |b| {
        b.to_async(&rt).iter(|| {
            let db = rtsql_db.clone();
            async move {
                db.execute_sql("SELECT * FROM bench WHERE id = 42").await;
            }
        });
    });

    group.finish();
}

criterion_group!(benches, bench_rtsql_vs_sqlite_pk_lookup);
criterion_main!(benches);