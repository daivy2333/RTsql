mod common;

use common::*;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

fn scale_insert(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("scale_insert");

    for &n in &[1_000i64, 10_000, 100_000] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::new("rows", n), &n, |b, &n| {
            b.iter(|| {
                rt.block_on(async {
                    let (path, db) = setup_db().await;
                    create_test_table(&db).await;
                    insert_rows(&db, 0, n).await;
                    cleanup_db(&path);
                });
            });
        });
    }
    group.finish();
}

fn scale_select(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("scale_select");

    for &n in &[1_000i64, 10_000, 100_000] {
        // Pre-insert data
        let (path, db) = rt.block_on(async {
            let (p, d) = setup_db().await;
            create_test_table(&d).await;
            insert_rows(&d, 0, n).await;
            (p, d)
        });

        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::new("pk_lookup", n), &n, |b, &n| {
            b.iter(|| {
                rt.block_on(async {
                    for i in 0..n {
                        db.execute_sql(&format!("SELECT * FROM bench WHERE id = {}", i))
                            .await;
                    }
                });
            });
        });
        cleanup_db(&path);
    }
    group.finish();
}

fn scale_scan(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("scale_scan");

    for &n in &[1_000i64, 10_000, 100_000] {
        // Pre-insert data
        let (path, db) = rt.block_on(async {
            let (p, d) = setup_db().await;
            create_test_table(&d).await;
            insert_rows(&d, 0, n).await;
            (p, d)
        });

        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::new("full_scan", n), &n, |b, _| {
            b.iter(|| {
                rt.block_on(async {
                    db.execute_sql("SELECT * FROM bench").await;
                });
            });
        });
        cleanup_db(&path);
    }
    group.finish();
}

fn scale_join(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("scale_join");

    for &n in &[100i64, 1_000, 10_000] {
        // Pre-insert data
        let (path, db) = rt.block_on(async {
            let (p, d) = setup_db().await;
            create_join_tables(&d).await;
            insert_join_data(&d, n).await;
            (p, d)
        });

        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::new("inner_join", n), &n, |b, _| {
            b.iter(|| {
                rt.block_on(async {
                    db.execute_sql(
                        "SELECT * FROM orders INNER JOIN customers ON orders.customer_id = customers.id",
                    )
                    .await;
                });
            });
        });
        cleanup_db(&path);
    }
    group.finish();
}

criterion_group!(benches, scale_insert, scale_select, scale_scan, scale_join);
criterion_main!(benches);
