use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use rusqlite::{Connection, params};
use tempfile::TempDir;
use rtsql::Database;

/// 精确单次查询对比测试，验证 8x SQLite speedup 的可信性
///
/// 测试方法：
/// - 直接测量单次 execute_sql() / query_row()，消除循环开销
/// - criterion 直接测量，无 profiling overhead
/// - warmup 50 次预热，确保缓存命中
fn single_pk_lookup_comparison(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // SQLite setup
    let sqlite_dir = TempDir::new().unwrap();
    let sqlite_path = sqlite_dir.path().join("sqlite.db");
    let sqlite_conn = Connection::open(&sqlite_path).unwrap();
    sqlite_conn.execute(
        "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)",
        [],
    ).unwrap();
    for i in 0..1000 {
        sqlite_conn.execute(
            "INSERT INTO test (id, value) VALUES (?1, ?2)",
            params![i, format!("value_{}", i)],
        ).unwrap();
    }

    // RTsql setup
    let rtsql_dir = TempDir::new().unwrap();
    let rtsql_path = rtsql_dir.path().join("rtsql.db");
    let rtsql_db = rt.block_on(Database::open(&rtsql_path)).unwrap();
    rt.block_on(rtsql_db.execute_sql(
        "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)"
    )).unwrap();
    for i in 0..1000 {
        rt.block_on(rtsql_db.execute_sql(
            &format!("INSERT INTO test (id, value) VALUES ({}, 'value_{}')", i, i)
        )).unwrap();
    }

    // Warmup (50 iterations)
    for _ in 0..50 {
        sqlite_conn.query_row(
            "SELECT value FROM test WHERE id = 500",
            [],
            |row| row.get::<_, String>(0),
        ).unwrap();
        rt.block_on(rtsql_db.execute_sql(
            "SELECT value FROM test WHERE id = 500"
        )).unwrap();
    }

    // SQLite single PK lookup
    let mut group = c.benchmark_group("single_pk_lookup");
    group.throughput(Throughput::Elements(1));

    group.bench_function("sqlite", |b| {
        b.iter(|| {
            sqlite_conn.query_row(
                "SELECT value FROM test WHERE id = 500",
                [],
                |row| row.get::<_, String>(0),
            ).unwrap()
        })
    });

    // RTsql single PK lookup
    group.bench_function("rtsql", |b| {
        b.to_async(&rt).iter(|| async {
            rtsql_db.execute_sql("SELECT value FROM test WHERE id = 500").await.unwrap()
        })
    });

    group.finish();
}

criterion_group!(benches, single_pk_lookup_comparison);
criterion_main!(benches);