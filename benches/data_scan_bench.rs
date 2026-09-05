//! M19: DataScan vs Scan benchmark
//!
//! Compares the new `DataScanExecutor` (direct data page chain traversal)
//! against the existing `ScanExecutor` (IndexManager.scan_all() based).
//!
//! Setup pattern: build the dataset **once** outside the `iter` closure
//! (using batch `INSERT` via `db.execute_sql` is slow; the bench measures
//! only the scan phase). Inside `iter`, we just construct the executor
//! and drain it — this is what the M19 design changes.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rtsql::database::Database;
use rtsql::executor::{DataScanExecutor, Executor, ScanExecutor};
use tempfile::tempdir;
use tokio::runtime::Runtime;

/// Build a pre-populated bench DB (one-time setup per benchmark case).
/// Uses batch INSERTs (`INSERT INTO t VALUES (...), (...), ...`) for speed.
async fn setup_table_with_rows(n: i64) -> Database {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("bench.db");
    let db = Database::open(&db_path).await.unwrap();
    db.execute_sql("CREATE TABLE bench (id INT PRIMARY KEY, name STRING, value INT)")
        .await;

    // Batch INSERT in chunks of 1000 to keep statement size bounded.
    const CHUNK: i64 = 1000;
    for chunk_start in (0..n).step_by(CHUNK as usize) {
        let chunk_end = (chunk_start + CHUNK).min(n);
        let mut sql = String::from("INSERT INTO bench VALUES ");
        for i in chunk_start..chunk_end {
            if i > chunk_start {
                sql.push(',');
            }
            sql.push_str(&format!("({}, 'user_{}', {})", i, i, i * 10));
        }
        db.execute_sql(&sql).await;
    }
    // Leak TempDir to keep the directory alive for the benchmark lifetime.
    std::mem::forget(dir);
    db
}

fn bench_data_scan_vs_scan(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // One-time setup per size — build DB outside the `iter` closure so the
    // benchmark only measures the scan phase, not the insert phase.
    let mut group = c.benchmark_group("data_scan_vs_scan");

    for &n in &[1_000i64, 10_000] {
        let db = rt.block_on(async { setup_table_with_rows(n).await });
        let table_meta = rt.block_on(async { db.table_manager.get_table("bench").await.unwrap() });
        let bp = db.buffer_pool.clone();
        let tm = table_meta.clone();

        // ScanExecutor (IndexManager.scan_all) — the old path
        group.bench_with_input(BenchmarkId::new("scan_via_index", n), &n, |b, &_n| {
            b.to_async(&rt).iter(|| async {
                let mut executor = ScanExecutor::new(tm.clone(), bp.clone(), None);
                let mut count = 0i64;
                while let Some(_row) = executor.next().await.unwrap() {
                    count += 1;
                }
                assert_eq!(count, _n);
            });
        });

        // DataScanExecutor (direct data page chain) — the M19 path
        group.bench_with_input(BenchmarkId::new("data_scan", n), &n, |b, &_n| {
            b.to_async(&rt).iter(|| async {
                let mut executor = DataScanExecutor::new(tm.clone(), bp.clone(), None, None, None);
                let mut count = 0i64;
                while let Some(_row) = executor.next().await.unwrap() {
                    count += 1;
                }
                assert_eq!(count, _n);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_data_scan_vs_scan);
criterion_main!(benches);
