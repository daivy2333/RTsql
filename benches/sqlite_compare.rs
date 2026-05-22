use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rusqlite::Connection;
use std::path::PathBuf;

fn setup_sqlite() -> (PathBuf, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("sqlite_bench.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE bench (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)",
        [],
    )
    .unwrap();
    std::mem::forget(dir);
    (db_path, conn)
}

fn insert_sqlite_rows(conn: &Connection, start: i64, n: i64) {
    for i in start..start + n {
        conn.execute(
            "INSERT INTO bench VALUES (?1, ?2, ?3)",
            rusqlite::params![i, format!("user_{}", i), i * 10],
        )
        .unwrap();
    }
}

fn bench_sqlite_insert(c: &mut Criterion) {
    let (path, conn) = setup_sqlite();
    let mut group = c.benchmark_group("sqlite_insert");
    group.throughput(Throughput::Elements(100));

    group.bench_function("insert_100_rows", |b| {
        b.iter(|| {
            insert_sqlite_rows(&conn, 0, 100);
            conn.execute("DELETE FROM bench", []).unwrap();
        });
    });

    group.finish();
    let _ = std::fs::remove_file(&path);
}

fn bench_sqlite_select(c: &mut Criterion) {
    let (path, conn) = setup_sqlite();
    insert_sqlite_rows(&conn, 0, 1000);

    let mut group = c.benchmark_group("sqlite_select");
    group.throughput(Throughput::Elements(1));

    group.bench_function("pk_lookup", |b| {
        b.iter(|| {
            let mut stmt = conn.prepare("SELECT * FROM bench WHERE id = 42").unwrap();
            let _ = stmt.query_row([], |row| {
                let id: i64 = row.get(0).unwrap();
                Ok(id)
            });
        });
    });

    group.finish();
    let _ = std::fs::remove_file(&path);
}

fn bench_sqlite_scan(c: &mut Criterion) {
    let (path, conn) = setup_sqlite();
    insert_sqlite_rows(&conn, 0, 1000);

    let mut group = c.benchmark_group("sqlite_scan");
    group.throughput(Throughput::Elements(1000));

    group.bench_function("full_scan_1k", |b| {
        b.iter(|| {
            let mut stmt = conn.prepare("SELECT * FROM bench").unwrap();
            let count = stmt
                .query_map([], |row| row.get::<_, i64>(0))
                .unwrap()
                .count();
            count
        });
    });

    group.finish();
    let _ = std::fs::remove_file(&path);
}

fn bench_sqlite_join(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("sqlite_join.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    conn.execute(
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER, amount INTEGER)",
        [],
    )
    .unwrap();
    for i in 0..1000i64 {
        conn.execute(
            "INSERT INTO customers VALUES (?1, ?2)",
            rusqlite::params![i, format!("customer_{}", i)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO orders VALUES (?1, ?2, ?3)",
            rusqlite::params![i, i, i * 100],
        )
        .unwrap();
    }

    let mut group = c.benchmark_group("sqlite_join");
    group.throughput(Throughput::Elements(1000));

    group.bench_function("inner_join_1k", |b| {
        b.iter(|| {
            let mut stmt = conn.prepare(
                "SELECT c.id, c.name, o.amount FROM customers c \
                 INNER JOIN orders o ON c.id = o.customer_id",
            )
            .unwrap();
            let count = stmt
                .query_map([], |row| row.get::<_, i64>(0))
                .unwrap()
                .count();
            count
        });
    });

    group.finish();
    std::mem::forget(dir);
}

criterion_group!(
    benches,
    bench_sqlite_insert,
    bench_sqlite_select,
    bench_sqlite_scan,
    bench_sqlite_join,
);
criterion_main!(benches);
