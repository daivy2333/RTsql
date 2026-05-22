mod common;

use common::*;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rtsql::database::Database;
use std::sync::atomic::{AtomicI64, Ordering};

static CONCURRENT_COUNTER: AtomicI64 = AtomicI64::new(0);

fn bench_concurrent_read(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 1000).await;
        (p, d)
    });

    let mut group = c.benchmark_group("concurrent_read");
    for concurrency in [1usize, 4, 8, 16, 32] {
        group.throughput(Throughput::Elements(concurrency as u64 * 50));
        group.bench_function(BenchmarkId::new("select", concurrency), |b| {
            b.to_async(&rt).iter(|| {
                let db = db.clone();
                async move {
                    let mut handles = vec![];
                    for _ in 0..concurrency {
                        let db = db.clone();
                        handles.push(tokio::spawn(async move {
                            for i in 0..50i64 {
                                db.execute_sql(&format!(
                                    "SELECT * FROM bench WHERE id = {}",
                                    i % 1000
                                ))
                                .await;
                            }
                        }));
                    }
                    for h in handles {
                        let _ = h.await;
                    }
                }
            });
        });
    }
    group.finish();
    cleanup_db(&path);
}

fn bench_concurrent_write(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        (p, d)
    });

    let mut group = c.benchmark_group("concurrent_write");
    for concurrency in [1usize, 4, 8, 16] {
        group.throughput(Throughput::Elements(concurrency as u64 * 50));
        group.bench_function(BenchmarkId::new("insert", concurrency), |b| {
            b.to_async(&rt).iter(|| {
                let db = db.clone();
                let base =
                    CONCURRENT_COUNTER.fetch_add(concurrency as i64 * 50, Ordering::SeqCst);
                async move {
                    let mut handles = vec![];
                    for t in 0..concurrency {
                        let db = db.clone();
                        let start = base + (t as i64) * 50;
                        handles.push(tokio::spawn(async move {
                            for i in 0..50i64 {
                                db.execute_sql(&format!(
                                    "INSERT INTO bench VALUES ({}, 'user_{}', {})",
                                    start + i,
                                    start + i,
                                    (start + i) * 10
                                ))
                                .await;
                            }
                        }));
                    }
                    for h in handles {
                        let _ = h.await;
                    }
                }
            });
        });
    }
    group.finish();
    cleanup_db(&path);
}

fn bench_concurrent_mixed(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 500).await;
        (p, d)
    });

    let mut group = c.benchmark_group("concurrent_mixed");
    for concurrency in [1usize, 4, 8, 16] {
        group.throughput(Throughput::Elements(concurrency as u64 * 100));
        group.bench_function(BenchmarkId::new("80r_20w", concurrency), |b| {
            b.to_async(&rt).iter(|| {
                let db = db.clone();
                let base =
                    CONCURRENT_COUNTER.fetch_add(concurrency as i64 * 20, Ordering::SeqCst);
                async move {
                    let mut handles = vec![];
                    for t in 0..concurrency {
                        let db = db.clone();
                        let write_start = base + (t as i64) * 20;
                        handles.push(tokio::spawn(async move {
                            for i in 0..80i64 {
                                db.execute_sql(&format!(
                                    "SELECT * FROM bench WHERE id = {}",
                                    i % 500
                                ))
                                .await;
                            }
                            for i in 0..20i64 {
                                db.execute_sql(&format!(
                                    "INSERT INTO bench VALUES ({}, 'user_{}', {})",
                                    write_start + i,
                                    write_start + i,
                                    (write_start + i) * 10
                                ))
                                .await;
                            }
                        }));
                    }
                    for h in handles {
                        let _ = h.await;
                    }
                }
            });
        });
    }
    group.finish();
    cleanup_db(&path);
}

fn bench_concurrent_conflict(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 10).await;
        (p, d)
    });

    let mut group = c.benchmark_group("concurrent_conflict");
    for concurrency in [4usize, 8, 16] {
        group.throughput(Throughput::Elements(concurrency as u64 * 50));
        group.bench_function(BenchmarkId::new("update_same_rows", concurrency), |b| {
            b.to_async(&rt).iter(|| {
                let db = db.clone();
                async move {
                    let mut handles = vec![];
                    for _ in 0..concurrency {
                        let db = db.clone();
                        handles.push(tokio::spawn(async move {
                            for i in 0..50i64 {
                                let row_id = i % 10;
                                db.execute_sql(&format!(
                                    "UPDATE bench SET value = value + 1 WHERE id = {}",
                                    row_id
                                ))
                                .await;
                            }
                        }));
                    }
                    for h in handles {
                        let _ = h.await;
                    }
                }
            });
        });
    }
    group.finish();
    cleanup_db(&path);
}

criterion_group!(
    benches,
    bench_concurrent_read,
    bench_concurrent_write,
    bench_concurrent_mixed,
    bench_concurrent_conflict,
);
criterion_main!(benches);
