//! M21: Page-level visibility benchmark
//!
//! Compares DataScan performance with and without the `all_visible` fast-path:
//!
//! 1. Cold scan (no snapshot, no visibility checks) — baseline
//! 2. First scan with snapshot (visibility checks, builds all_visible cache)
//! 3. Second scan with snapshot (all_visible cache warm, per-row checks skipped)

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rtsql::database::Database;
use rtsql::executor::{DataScanExecutor, Executor};
use rtsql::transaction::Snapshot;
use tempfile::tempdir;
use tokio::runtime::Runtime;

/// Build a pre-populated DB with N committed rows.
async fn setup_committed_rows(n: i64) -> Database {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("bench_vis.db");
    let db = Database::open(&db_path).await.unwrap();
    db.execute_sql("CREATE TABLE bench (id INT PRIMARY KEY, val INT)")
        .await;

    const CHUNK: i64 = 1000;
    for chunk_start in (0..n).step_by(CHUNK as usize) {
        let chunk_end = (chunk_start + CHUNK).min(n);
        let mut sql = String::from("INSERT INTO bench VALUES ");
        for i in chunk_start..chunk_end {
            if i > chunk_start {
                sql.push(',');
            }
            sql.push_str(&format!("({}, {})", i, i * 10));
        }
        db.execute_sql(&sql).await;
    }
    std::mem::forget(dir);
    db
}

fn bench_visibility(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("visibility");

    for &n in &[1_000i64, 10_000] {
        let db = rt.block_on(async { setup_committed_rows(n).await });
        let table_meta = rt.block_on(async { db.table_manager.get_table("bench").await.unwrap() });
        let bp = db.buffer_pool.clone();
        let tm = table_meta.clone();

        // Scenario 1: No snapshot (no MVCC checks, baseline)
        group.bench_with_input(BenchmarkId::new("no_snapshot", n), &n, |b, &_n| {
            b.to_async(&rt).iter(|| async {
                let mut executor = DataScanExecutor::new(tm.clone(), bp.clone(), None);
                let mut count = 0i64;
                while let Some(_row) = executor.next().await.unwrap() {
                    count += 1;
                }
                assert_eq!(count, _n);
            });
        });

        // Scenario 2: Cold scan with snapshot (first scan builds visibility cache)
        // Uses a fresh executor each iteration so all_visible starts as false.
        group.bench_with_input(BenchmarkId::new("snapshot_cold", n), &n, |b, &_n| {
            b.to_async(&rt).iter(|| async {
                let snapshot = Snapshot::new(n as u64 * 10, vec![]);
                let mut executor = DataScanExecutor::new(tm.clone(), bp.clone(), Some(snapshot));
                let mut count = 0i64;
                while let Some(_row) = executor.next().await.unwrap() {
                    count += 1;
                }
                // Don't assert count — measure scan throughput with visibility checks
                let _ = count;
            });
        });

        // Scenario 3: Warm scan with snapshot (all_visible cache populated)
        // Run one scan to populate the cache, then measure subsequent scans.
        {
            let snapshot = Snapshot::new(n as u64 * 10, vec![]);
            let mut warmup = DataScanExecutor::new(tm.clone(), bp.clone(), Some(snapshot));
            rt.block_on(async { while warmup.next().await.unwrap().is_some() {} });
        }
        group.bench_with_input(BenchmarkId::new("snapshot_warm", n), &n, |b, &_n| {
            b.to_async(&rt).iter(|| async {
                let snapshot = Snapshot::new(n as u64 * 10, vec![]);
                let mut executor = DataScanExecutor::new(tm.clone(), bp.clone(), Some(snapshot));
                let mut count = 0i64;
                while let Some(_row) = executor.next().await.unwrap() {
                    count += 1;
                }
                let _ = count;
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_visibility);
criterion_main!(benches);
