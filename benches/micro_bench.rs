mod common;

use common::*;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use tokio::runtime::Runtime;

// ── INSERT ──────────────────────────────────────────────────────────────────
fn bench_insert(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("insert");

    // Only 3 representative cases instead of 100
    for i in [0, 50, 99] {
        group.bench_with_input(BenchmarkId::new("single_row", i), &i, |b, &i| {
            b.to_async(&rt).iter(|| async {
                let (path, db) = setup_db().await;
                create_test_table(&db).await;
                // Use i * 1000 + j to avoid primary-key collisions across
                // criterion iterations (each iter runs the closure multiple times).
                for j in 0..1 {
                    let id = i * 1000 + j;
                    db.execute_sql(&format!(
                        "INSERT INTO bench VALUES ({}, 'user_{}', {})",
                        id, id, id * 10
                    ))
                    .await;
                }
                cleanup_db(&path);
            });
        });
    }
    group.finish();
}

// ── SELECT ──────────────────────────────────────────────────────────────────
fn bench_select(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    let mut group = c.benchmark_group("select");
    group.bench_function("pk_lookup", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                for i in 0..30 {
                    db.execute_sql(&format!("SELECT * FROM bench WHERE id = {}", i))
                        .await;
                }
            }
        });
    });
    group.finish();
    cleanup_db(&path);
}

// ── UPDATE ──────────────────────────────────────────────────────────────────
fn bench_update(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    let mut group = c.benchmark_group("update");
    group.bench_function("single_column", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                for i in 0..30 {
                    db.execute_sql(&format!(
                        "UPDATE bench SET value = {} WHERE id = {}",
                        i * 20, i
                    ))
                    .await;
                }
            }
        });
    });
    group.finish();
    cleanup_db(&path);
}

// ── DELETE ──────────────────────────────────────────────────────────────────
fn bench_delete(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("delete");
    group.bench_function("by_pk", |b| {
        b.to_async(&rt).iter(|| async {
            let (path, db) = setup_db().await;
            create_test_table(&db).await;
            // Insert 60 rows so we can delete 30 and still have data left.
            insert_rows(&db, 0, 60).await;

            for i in 0..30 {
                db.execute_sql(&format!("DELETE FROM bench WHERE id = {}", i))
                    .await;
            }
            cleanup_db(&path);
        });
    });
    group.finish();
}

// ── SCAN ────────────────────────────────────────────────────────────────────
fn bench_scan(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    let mut group = c.benchmark_group("scan");
    group.bench_function("full_table", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                db.execute_sql("SELECT * FROM bench").await;
            }
        });
    });
    group.finish();
    cleanup_db(&path);
}

// ── FILTER ──────────────────────────────────────────────────────────────────
fn bench_filter(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    let mut group = c.benchmark_group("filter");
    group.bench_function("where_value_gt_500", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                db.execute_sql("SELECT * FROM bench WHERE value > 500")
                    .await;
            }
        });
    });
    group.finish();
    cleanup_db(&path);
}

// ── SORT ────────────────────────────────────────────────────────────────────
fn bench_sort(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    let mut group = c.benchmark_group("sort");
    group.bench_function("order_by_value_desc", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                db.execute_sql("SELECT * FROM bench ORDER BY value DESC")
                    .await;
            }
        });
    });
    group.finish();
    cleanup_db(&path);
}

// ── LIMIT ───────────────────────────────────────────────────────────────────
fn bench_limit(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    let mut group = c.benchmark_group("limit");
    group.bench_function("limit_10_offset_5", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                db.execute_sql("SELECT * FROM bench LIMIT 10 OFFSET 5")
                    .await;
            }
        });
    });
    group.finish();
    cleanup_db(&path);
}

// ── JOIN ────────────────────────────────────────────────────────────────────
fn bench_join(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_join_tables(&d).await;
        insert_join_data(&d, 50).await;
        (p, d)
    });

    let mut group = c.benchmark_group("join");
    group.bench_function("inner_join", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                db.execute_sql(
                    "SELECT customers.id, customers.name, orders.amount \
                     FROM customers INNER JOIN orders ON customers.id = orders.customer_id",
                )
                .await;
            }
        });
    });
    group.finish();
    cleanup_db(&path);
}

criterion_group!(
    benches,
    bench_insert,
    bench_select,
    bench_update,
    bench_delete,
    bench_scan,
    bench_filter,
    bench_sort,
    bench_limit,
    bench_join,
);
criterion_main!(benches);
