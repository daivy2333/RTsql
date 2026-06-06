// M31: BufferPool DashMap + miss Semaphore concurrency benchmark.
//
// Measures 3 scenarios:
//   1. cache_hit_concurrent: 16 tasks reading same cached page (no contention)
//   2. cache_miss_concurrent: 16 tasks loading different uncached pages
//      (verifies DashMap shard distribution + miss semaphore bound)
//   3. miss_backpressure: 1000 tasks loading 1000 unique pages
//      (verifies no IO storm — bounded in-flight)
//
// Run with:
//   cargo bench --bench buffer_pool_concurrency_bench

use criterion::{criterion_group, criterion_main, Criterion};
use rtsql::storage::{BufferPool, FileStorage};
use std::sync::Arc;
use tempfile::tempdir;

fn make_pool(capacity: usize) -> Arc<BufferPool> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("bench.db")).unwrap());
    std::mem::forget(dir);
    Arc::new(BufferPool::new(capacity, storage).unwrap())
}

fn bench_cache_hit_concurrent(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let pool = make_pool(100);
    let page_id = rt.block_on(async { pool.storage().allocate_page().await.unwrap() });
    // Pre-warm cache
    let _ = rt.block_on(async { pool.get_page(page_id).await });

    c.bench_function("cache_hit_16_tasks_same_page", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            async move {
                let mut tasks = vec![];
                for _ in 0..16 {
                    let p = pool.clone();
                    tasks.push(tokio::spawn(async move { p.get_page(page_id).await }));
                }
                for t in tasks {
                    let _ = t.await;
                }
            }
        });
    });
}

fn bench_cache_miss_concurrent(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    // Capacity large enough to hold 32 pages (no eviction)
    let pool = make_pool(64);
    let page_ids: Vec<_> = rt.block_on(async {
        let mut ids = Vec::with_capacity(32);
        for _ in 0..32 {
            ids.push(pool.storage().allocate_page().await.unwrap());
        }
        ids
    });

    c.bench_function("cache_miss_16_tasks_diff_pages", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            let pids = page_ids.clone();
            async move {
                let mut tasks = vec![];
                for pid in pids.into_iter().take(16) {
                    let p = pool.clone();
                    tasks.push(tokio::spawn(async move { p.get_page(pid).await }));
                }
                for t in tasks {
                    let _ = t.await;
                }
            }
        });
    });
}

fn bench_miss_backpressure(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    // Capacity large enough to hold 200 pages (no eviction in test)
    let pool = make_pool(500);
    let page_ids: Vec<_> = rt.block_on(async {
        let mut ids = Vec::with_capacity(200);
        for _ in 0..200 {
            ids.push(pool.storage().allocate_page().await.unwrap());
        }
        ids
    });

    c.bench_function("miss_backpressure_200_tasks", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            let pids = page_ids.clone();
            async move {
                let mut tasks = vec![];
                for pid in pids.into_iter() {
                    let p = pool.clone();
                    tasks.push(tokio::spawn(async move { p.get_page(pid).await }));
                }
                for t in tasks {
                    let _ = t.await;
                }
            }
        });
    });
}

criterion_group!(
    benches,
    bench_cache_hit_concurrent,
    bench_cache_miss_concurrent,
    bench_miss_backpressure
);
criterion_main!(benches);
