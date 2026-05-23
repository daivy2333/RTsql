//! Comprehensive SQLite comparison benchmarks
//!
//! M17.5-T3: Full comparison of RTsql vs SQLite performance
//!
//! Includes:
//! - RTsql Server framework with tokio-postgres client
//! - Basic SQL operation comparison (INSERT, PK lookup, scan, JOIN)
//! - M17 B-Tree split performance tests
//! - M17 non-unique index tests
//! - Resource consumption tests (file size, binary size)

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rusqlite::Connection;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Arc;
use tempfile::{tempdir, NamedTempFile, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;

// ============================================================
// SQLite Helpers
// ============================================================

fn setup_sqlite() -> (TempDir, PathBuf, Connection) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("sqlite_bench.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE bench (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)",
        [],
    )
    .unwrap();
    (dir, db_path, conn)
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

// ============================================================
// RTsql Server Framework
// ============================================================

/// RTsql server process wrapper (kept for future network benchmarks)
#[allow(dead_code)]
struct RTsqlServer {
    process: Option<Child>,
    addr: std::net::SocketAddr,
    temp_file: NamedTempFile,
}

#[allow(dead_code)]
impl RTsqlServer {
    /// Start RTsql server on a dynamic port
    fn start() -> Self {
        let _temp_file = NamedTempFile::new().unwrap();
        let _port = find_available_port();
        let _addr = std::net::SocketAddr::from(([127, 0, 0, 1], _port));

        // Build the server binary path
        let binary_path = std::env::current_dir()
            .unwrap()
            .join("target/release/rtsql");

        // If the binary doesn't exist, try to build it
        if !binary_path.exists() {
            let status = Command::new("cargo")
                .args(["build", "--release", "--bin", "rtsql"])
                .status()
                .expect("Failed to build RTsql server");
            assert!(status.success(), "Failed to build RTsql server");
        }

        // For benchmarks, use the Database API directly to avoid process overhead
        panic!("Use RTsqlDirect instead for benchmarks");
    }

    fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }

    fn shutdown(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

impl Drop for RTsqlServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Find an available port for testing (kept for future network benchmarks)
#[allow(dead_code)]
fn find_available_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// Direct RTsql database access for benchmarks (no network overhead)
/// Uses a shared tokio runtime for efficient async execution
struct RTsqlDirect {
    database: Arc<rtsql::database::Database>,
    runtime: Runtime,
    _temp_file: NamedTempFile,
}

impl RTsqlDirect {
    fn new() -> Self {
        let temp_file = NamedTempFile::new().unwrap();
        let runtime = Runtime::new().unwrap();
        let database = runtime.block_on(async {
            rtsql::database::Database::open(temp_file.path())
                .await
                .unwrap()
        });
        Self {
            database: Arc::new(database),
            runtime,
            _temp_file: temp_file,
        }
    }

    fn setup_bench_table(&self) {
        let db = self.database.clone();
        self.runtime.block_on(async {
            db.create_table(
                "bench",
                vec![
                    ("id".to_string(), rtsql::storage::ColumnType::Int),
                    ("name".to_string(), rtsql::storage::ColumnType::String(100)),
                    ("value".to_string(), rtsql::storage::ColumnType::Int),
                ],
                "id",
            )
            .await
            .unwrap();
        });
    }

    fn execute(&self, sql: &str) -> rtsql::network::protocol::Response {
        let db = self.database.clone();
        self.runtime.block_on(async { db.execute_sql(sql).await })
    }

    #[allow(dead_code)]
    fn file_path(&self) -> &std::path::Path {
        self._temp_file.path()
    }
}

// ============================================================
// PostgreSQL Protocol Client (lightweight)
// ============================================================

/// A minimal PostgreSQL protocol client for RTsql (kept for future network benchmarks)
#[allow(dead_code)]
struct PgClient {
    stream: TcpStream,
}

#[allow(dead_code)]
impl PgClient {
    async fn connect(addr: std::net::SocketAddr) -> std::io::Result<Self> {
        let mut stream = TcpStream::connect(addr).await?;

        // Send startup message
        let params = b"user\0test\0database\0test\0";
        let length = 4 + 4 + params.len() + 1;
        let mut startup_msg = Vec::new();
        startup_msg.extend_from_slice(&(length as i32).to_be_bytes());
        startup_msg.extend_from_slice(&196608i32.to_be_bytes()); // Protocol 3.0
        startup_msg.extend_from_slice(params);
        startup_msg.push(0);

        stream.write_all(&startup_msg).await?;

        // Read startup response (skip validation for benchmark)
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf).await?;

        Ok(Self { stream })
    }

    async fn execute(&mut self, sql: &str) -> std::io::Result<usize> {
        // Send Query message
        let mut query_msg = Vec::new();
        query_msg.push(b'Q');
        let sql_bytes = sql.as_bytes();
        let length = 4 + sql_bytes.len() + 1;
        query_msg.extend_from_slice(&(length as i32).to_be_bytes());
        query_msg.extend_from_slice(sql_bytes);
        query_msg.push(0);

        self.stream.write_all(&query_msg).await?;

        // Read response
        let mut buf = [0u8; 8192];
        let n = self.stream.read(&mut buf).await?;

        Ok(n)
    }

    async fn close(mut self) -> std::io::Result<()> {
        // Send Terminate message
        let terminate_msg = [b'X', 0, 0, 0, 4];
        self.stream.write_all(&terminate_msg).await?;
        Ok(())
    }
}

// ============================================================
// SQLite Benchmarks
// ============================================================

fn bench_sqlite_insert(c: &mut Criterion) {
    let (_dir, path, conn) = setup_sqlite();
    let mut group = c.benchmark_group("sqlite_insert");
    group.throughput(Throughput::Elements(100));

    group.bench_function("insert_100_rows", |b| {
        b.iter(|| {
            insert_sqlite_rows(&conn, 0, 100);
            conn.execute("DELETE FROM bench", []).unwrap();
        });
    });

    group.finish();
    drop(conn);
    let _ = std::fs::remove_file(&path);
}

fn bench_sqlite_select(c: &mut Criterion) {
    let (_dir, path, conn) = setup_sqlite();
    insert_sqlite_rows(&conn, 0, 1000);

    let mut group = c.benchmark_group("sqlite_select");
    group.throughput(Throughput::Elements(1));

    group.bench_function("pk_lookup_1k", |b| {
        b.iter(|| {
            let mut stmt = conn.prepare("SELECT * FROM bench WHERE id = 42").unwrap();
            let _ = stmt.query_row([], |row| {
                let id: i64 = row.get(0).unwrap();
                Ok(id)
            });
        });
    });

    group.finish();
    drop(conn);
    let _ = std::fs::remove_file(&path);
}

fn bench_sqlite_scan(c: &mut Criterion) {
    let (_dir, path, conn) = setup_sqlite();
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
    drop(conn);
    let _ = std::fs::remove_file(&path);
}

fn bench_sqlite_join(c: &mut Criterion) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("sqlite_join.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT)",
        [],
    )
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
            let mut stmt = conn
                .prepare(
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
    drop(conn);
    std::mem::forget(dir);
}

// ============================================================
// RTsql Direct Benchmarks (in-process, no network)
// ============================================================

fn bench_rtsql_insert(c: &mut Criterion) {
    let rtsql = RTsqlDirect::new();
    rtsql.setup_bench_table();

    let mut group = c.benchmark_group("rtsql_insert");
    group.throughput(Throughput::Elements(100));
    group.sample_size(20); // Reduce sample size for slower operations

    group.bench_function("insert_100_rows", |b| {
        b.iter(|| {
            for i in 0..100i64 {
                rtsql.execute(&format!(
                    "INSERT INTO bench VALUES ({}, 'user_{}', {})",
                    i,
                    i,
                    i * 10
                ));
            }
            rtsql.execute("DELETE FROM bench");
        });
    });

    group.finish();
}

fn bench_rtsql_select(c: &mut Criterion) {
    let rtsql = RTsqlDirect::new();
    rtsql.setup_bench_table();

    // Insert test data
    for i in 0..1000i64 {
        rtsql.execute(&format!(
            "INSERT INTO bench VALUES ({}, 'user_{}', {})",
            i,
            i,
            i * 10
        ));
    }

    let mut group = c.benchmark_group("rtsql_select");
    group.throughput(Throughput::Elements(1));

    group.bench_function("pk_lookup_1k", |b| {
        b.iter(|| {
            rtsql.execute("SELECT * FROM bench WHERE id = 42");
        });
    });

    group.finish();
}

fn bench_rtsql_scan(c: &mut Criterion) {
    let rtsql = RTsqlDirect::new();
    rtsql.setup_bench_table();

    // Insert test data
    for i in 0..1000i64 {
        rtsql.execute(&format!(
            "INSERT INTO bench VALUES ({}, 'user_{}', {})",
            i,
            i,
            i * 10
        ));
    }

    let mut group = c.benchmark_group("rtsql_scan");
    group.throughput(Throughput::Elements(1000));

    group.bench_function("full_scan_1k", |b| {
        b.iter(|| {
            rtsql.execute("SELECT * FROM bench");
        });
    });

    group.finish();
}

// ============================================================
// Comparison Benchmarks: RTsql vs SQLite
// ============================================================

fn bench_compare_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("compare_insert");
    group.throughput(Throughput::Elements(100));
    group.sample_size(20);

    // SQLite
    let (_dir, path, sqlite_conn) = setup_sqlite();
    group.bench_function("sqlite_insert_100", |b| {
        b.iter(|| {
            insert_sqlite_rows(&sqlite_conn, 0, 100);
            sqlite_conn.execute("DELETE FROM bench", []).unwrap();
        });
    });
    drop(sqlite_conn);
    let _ = std::fs::remove_file(&path);

    // RTsql
    let rtsql = RTsqlDirect::new();
    rtsql.setup_bench_table();
    group.bench_function("rtsql_insert_100", |b| {
        b.iter(|| {
            for i in 0..100i64 {
                rtsql.execute(&format!(
                    "INSERT INTO bench VALUES ({}, 'user_{}', {})",
                    i,
                    i,
                    i * 10
                ));
            }
            rtsql.execute("DELETE FROM bench");
        });
    });

    group.finish();
}

fn bench_compare_pk_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("compare_pk_lookup");
    group.throughput(Throughput::Elements(1));

    // SQLite
    let (_dir, path, sqlite_conn) = setup_sqlite();
    insert_sqlite_rows(&sqlite_conn, 0, 1000);
    group.bench_function("sqlite_pk_lookup_1k", |b| {
        b.iter(|| {
            let mut stmt = sqlite_conn
                .prepare("SELECT * FROM bench WHERE id = 42")
                .unwrap();
            let _ = stmt.query_row([], |row| {
                let id: i64 = row.get(0).unwrap();
                Ok(id)
            });
        });
    });
    drop(sqlite_conn);
    let _ = std::fs::remove_file(&path);

    // RTsql
    let rtsql = RTsqlDirect::new();
    rtsql.setup_bench_table();
    for i in 0..1000i64 {
        rtsql.execute(&format!(
            "INSERT INTO bench VALUES ({}, 'user_{}', {})",
            i,
            i,
            i * 10
        ));
    }
    group.bench_function("rtsql_pk_lookup_1k", |b| {
        b.iter(|| {
            rtsql.execute("SELECT * FROM bench WHERE id = 42");
        });
    });

    group.finish();
}

fn bench_compare_full_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("compare_full_scan");
    group.throughput(Throughput::Elements(1000));

    // SQLite
    let (_dir, path, sqlite_conn) = setup_sqlite();
    insert_sqlite_rows(&sqlite_conn, 0, 1000);
    group.bench_function("sqlite_full_scan_1k", |b| {
        b.iter(|| {
            let mut stmt = sqlite_conn.prepare("SELECT * FROM bench").unwrap();
            let count = stmt
                .query_map([], |row| row.get::<_, i64>(0))
                .unwrap()
                .count();
            count
        });
    });
    drop(sqlite_conn);
    let _ = std::fs::remove_file(&path);

    // RTsql
    let rtsql = RTsqlDirect::new();
    rtsql.setup_bench_table();
    for i in 0..1000i64 {
        rtsql.execute(&format!(
            "INSERT INTO bench VALUES ({}, 'user_{}', {})",
            i,
            i,
            i * 10
        ));
    }
    group.bench_function("rtsql_full_scan_1k", |b| {
        b.iter(|| {
            rtsql.execute("SELECT * FROM bench");
        });
    });

    group.finish();
}

// ============================================================
// M17 B-Tree Split Performance Tests
// ============================================================

fn bench_split_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("m17_split");
    group.sample_size(10);

    // Test insert performance with large datasets that trigger splits
    for size in [100, 500].iter() {
        // SQLite baseline
        let (_dir, path, sqlite_conn) = setup_sqlite();
        group.bench_function(BenchmarkId::new("sqlite_insert", size), |b| {
            b.iter(|| {
                for i in 0..*size {
                    sqlite_conn
                        .execute(
                            "INSERT INTO bench VALUES (?1, ?2, ?3)",
                            rusqlite::params![i, format!("user_{}", i), i * 10],
                        )
                        .unwrap();
                }
                sqlite_conn.execute("DELETE FROM bench", []).unwrap();
            });
        });
        drop(sqlite_conn);
        let _ = std::fs::remove_file(&path);

        // RTsql
        let rtsql = RTsqlDirect::new();
        rtsql.setup_bench_table();
        group.bench_function(BenchmarkId::new("rtsql_insert", size), |b| {
            b.iter(|| {
                for i in 0..*size {
                    rtsql.execute(&format!(
                        "INSERT INTO bench VALUES ({}, 'user_{}', {})",
                        i,
                        i,
                        i * 10
                    ));
                }
                rtsql.execute("DELETE FROM bench");
            });
        });
    }

    group.finish();
}

fn bench_pk_lookup_after_split(c: &mut Criterion) {
    let mut group = c.benchmark_group("m17_lookup_after_split");
    group.throughput(Throughput::Elements(1));

    // Insert rows to trigger splits, then measure PK lookup
    for size in [100, 500].iter() {
        // SQLite
        let (_dir, path, sqlite_conn) = setup_sqlite();
        insert_sqlite_rows(&sqlite_conn, 0, *size as i64);
        group.bench_function(BenchmarkId::new("sqlite_pk_lookup", size), |b| {
            b.iter(|| {
                let mut stmt = sqlite_conn
                    .prepare(&format!("SELECT * FROM bench WHERE id = {}", size / 2))
                    .unwrap();
                let _ = stmt.query_row([], |row| {
                    let id: i64 = row.get(0).unwrap();
                    Ok(id)
                });
            });
        });
        drop(sqlite_conn);
        let _ = std::fs::remove_file(&path);

        // RTsql
        let rtsql = RTsqlDirect::new();
        rtsql.setup_bench_table();
        for i in 0..*size as i64 {
            rtsql.execute(&format!(
                "INSERT INTO bench VALUES ({}, 'user_{}', {})",
                i,
                i,
                i * 10
            ));
        }
        group.bench_function(BenchmarkId::new("rtsql_pk_lookup", size), |b| {
            b.iter(|| {
                rtsql.execute(&format!("SELECT * FROM bench WHERE id = {}", size / 2));
            });
        });
    }

    group.finish();
}

// ============================================================
// M17 Non-Unique Index Tests
// ============================================================

fn bench_non_unique_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("m17_non_unique_index");
    group.sample_size(20);

    // SQLite with non-unique values
    let (_dir, path, sqlite_conn) = setup_sqlite();
    // Insert rows with duplicate values (same value for every 10 rows)
    for i in 0..1000i64 {
        let dup_val = i / 10;
        sqlite_conn
            .execute(
                "INSERT INTO bench VALUES (?1, ?2, ?3)",
                rusqlite::params![i, format!("user_{}", i), dup_val],
            )
            .unwrap();
    }
    // Create non-unique index on value column
    sqlite_conn
        .execute("CREATE INDEX idx_value ON bench(value)", [])
        .unwrap();

    group.bench_function("sqlite_search_duplicates", |b| {
        b.iter(|| {
            let mut stmt = sqlite_conn
                .prepare("SELECT * FROM bench WHERE value = 50")
                .unwrap();
            let count = stmt
                .query_map([], |row| row.get::<_, i64>(0))
                .unwrap()
                .count();
            count
        });
    });
    drop(sqlite_conn);
    let _ = std::fs::remove_file(&path);

    // RTsql with duplicate values (B-Tree supports non-unique keys)
    let rtsql = RTsqlDirect::new();
    rtsql.setup_bench_table();
    for i in 0..1000i64 {
        let dup_val = i / 10;
        rtsql.execute(&format!(
            "INSERT INTO bench VALUES ({}, 'user_{}', {})",
            i, i, dup_val
        ));
    }

    group.bench_function("rtsql_search_duplicates", |b| {
        b.iter(|| {
            rtsql.execute("SELECT * FROM bench WHERE value = 50");
        });
    });

    group.finish();
}

// ============================================================
// Resource Consumption Tests
// ============================================================

fn bench_file_size(c: &mut Criterion) {
    let mut group = c.benchmark_group("resource_file_size");

    for size in [10000].iter() {
        // SQLite file size
        let dir = tempdir().unwrap();
        let sqlite_path = dir.path().join(format!("sqlite_{}.db", size));
        let sqlite_conn = Connection::open(&sqlite_path).unwrap();
        sqlite_conn
            .execute(
                "CREATE TABLE bench (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)",
                [],
            )
            .unwrap();
        insert_sqlite_rows(&sqlite_conn, 0, *size as i64);
        drop(sqlite_conn);

        let sqlite_size = std::fs::metadata(&sqlite_path).unwrap().len();
        group.bench_function(BenchmarkId::new("sqlite_size", size), |b| {
            b.iter(|| sqlite_size);
        });

        // RTsql file size
        let rtsql_temp = NamedTempFile::new().unwrap();
        let runtime = Runtime::new().unwrap();
        let rtsql_db = runtime.block_on(async {
            rtsql::database::Database::open(rtsql_temp.path())
                .await
                .unwrap()
        });
        let rtsql_db = Arc::new(rtsql_db);
        runtime.block_on(async {
            rtsql_db
                .create_table(
                    "bench",
                    vec![
                        ("id".to_string(), rtsql::storage::ColumnType::Int),
                        ("name".to_string(), rtsql::storage::ColumnType::String(100)),
                        ("value".to_string(), rtsql::storage::ColumnType::Int),
                    ],
                    "id",
                )
                .await
                .unwrap();
        });
        for i in 0..*size as i64 {
            runtime.block_on(async {
                rtsql_db
                    .execute_sql(&format!(
                        "INSERT INTO bench VALUES ({}, 'user_{}', {})",
                        i,
                        i,
                        i * 10
                    ))
                    .await
            });
        }
        drop(rtsql_db);

        let rtsql_size = std::fs::metadata(rtsql_temp.path()).unwrap().len();
        group.bench_function(BenchmarkId::new("rtsql_size", size), |b| {
            b.iter(|| rtsql_size);
        });

        // Report sizes
        println!(
            "File size for {} rows: SQLite = {} bytes, RTsql = {} bytes",
            size, sqlite_size, rtsql_size
        );
    }

    group.finish();
}

fn bench_binary_size(c: &mut Criterion) {
    let mut group = c.benchmark_group("resource_binary_size");

    // SQLite binary (from system)
    let sqlite_binary = which::which("sqlite3").ok();
    if let Some(ref path) = sqlite_binary {
        if let Ok(metadata) = std::fs::metadata(path) {
            let size = metadata.len();
            group.bench_function("sqlite_binary", |b| {
                b.iter(|| size);
            });
            println!("SQLite binary size: {} bytes", size);
        }
    }

    // RTsql binary
    let rtsql_binary = std::env::current_dir()
        .unwrap()
        .join("target/release/rtsql");
    if rtsql_binary.exists() {
        if let Ok(metadata) = std::fs::metadata(&rtsql_binary) {
            let size = metadata.len();
            group.bench_function("rtsql_binary", |b| {
                b.iter(|| size);
            });
            println!("RTsql binary size: {} bytes", size);
        }
    } else {
        println!("RTsql binary not found at {:?}", rtsql_binary);
    }

    group.finish();
}

// ============================================================
// Criterion Groups
// ============================================================

criterion_group!(
    sqlite_benches,
    bench_sqlite_insert,
    bench_sqlite_select,
    bench_sqlite_scan,
    bench_sqlite_join,
);

criterion_group!(
    rtsql_benches,
    bench_rtsql_insert,
    bench_rtsql_select,
    bench_rtsql_scan,
);

criterion_group!(
    compare_benches,
    bench_compare_insert,
    bench_compare_pk_lookup,
    bench_compare_full_scan,
);

criterion_group!(
    m17_benches,
    bench_split_performance,
    bench_pk_lookup_after_split,
    bench_non_unique_index,
);

criterion_group!(resource_benches, bench_file_size, bench_binary_size,);

criterion_main!(
    sqlite_benches,
    rtsql_benches,
    compare_benches,
    m17_benches,
    resource_benches,
);
