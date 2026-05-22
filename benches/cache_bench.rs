mod common;

use common::*;
use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;

fn bench_pk_lookup_cached(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    // Warm up cache: execute the same SQL once so it gets cached
    rt.block_on(async {
        db.execute_sql("SELECT * FROM bench WHERE id = 42").await;
    });

    let mut group = c.benchmark_group("pk_lookup_cached");
    group.bench_function("same_sql_cached", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                // Same SQL every time — should hit plan cache
                db.execute_sql("SELECT * FROM bench WHERE id = 42").await;
            }
        });
    });
    group.finish();
    cleanup_db(&path);
}

fn bench_pk_lookup_uncached(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    let mut group = c.benchmark_group("pk_lookup_uncached");
    group.bench_function("same_sql_uncached", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                // Same SQL every time but no cache warm-up
                db.execute_sql("SELECT * FROM bench WHERE id = 42").await;
            }
        });
    });
    group.finish();
    cleanup_db(&path);
}

criterion_group!(benches, bench_pk_lookup_cached, bench_pk_lookup_uncached);
criterion_main!(benches);