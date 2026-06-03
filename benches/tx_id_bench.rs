//! 事务 ID 分配微基准（M41）
//!
//! 对比 `Mutex<u64>` 与 `AtomicU64` 在不同并发度下的 `fetch_add` 性能。
//!
//! ## 场景
//! - single：单线程 1M 次分配
//! - threads_10：10 线程 × 100K 次
//! - threads_100：100 线程 × 10K 次
//! - throughput：单线程稳态 ops/sec
//!
//! ## 运行
//! ```bash
//! cargo bench --bench tx_id_bench
//! ```
//!
//! ## 不变量
//! - 不修改 `src/` 任何代码
//! - 不新增 `Cargo.toml` 依赖（criterion 0.5 已在 dev-deps）
//! - 黑盒函数用 `#[inline(never)]` 防止编译器消除真实开销

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

const N_SINGLE: u64 = 1_000_000;
const N_PER_THREAD_10: u64 = 100_000;
const N_PER_THREAD_100: u64 = 10_000;
const N_THREADS_10: usize = 10;
const N_THREADS_100: usize = 100;

// ── Mutex 实现（对照） ────────────────────────────────────────────────────

#[inline(never)]
fn mutex_alloc(counter: &Mutex<u64>) -> u64 {
    let mut guard = counter.lock().unwrap();
    *guard += 1;
    *guard
}

// ── Atomic 实现（生产路径） ──────────────────────────────────────────────

#[inline(never)]
fn atomic_alloc(counter: &AtomicU64) -> u64 {
    counter.fetch_add(1, Ordering::SeqCst) + 1
}

// ── 1. 单线程延迟 ────────────────────────────────────────────────────────

fn bench_single_thread(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_thread");
    group.throughput(Throughput::Elements(N_SINGLE));

    group.bench_function("mutex", |b| {
        let counter = Mutex::new(0u64);
        b.iter(|| {
            for _ in 0..N_SINGLE {
                std::hint::black_box(mutex_alloc(&counter));
            }
        });
    });

    group.bench_function("atomic", |b| {
        let counter = AtomicU64::new(0);
        b.iter(|| {
            for _ in 0..N_SINGLE {
                std::hint::black_box(atomic_alloc(&counter));
            }
        });
    });

    group.finish();
}

// ── 2. 10 线程争用 ───────────────────────────────────────────────────────

fn bench_10_threads(c: &mut Criterion) {
    let mut group = c.benchmark_group("threads_10");
    group.throughput(Throughput::Elements(N_THREADS_10 as u64 * N_PER_THREAD_10));

    group.bench_function("mutex", |b| {
        b.iter(|| {
            let counter = Arc::new(Mutex::new(0u64));
            let mut handles = Vec::with_capacity(N_THREADS_10);
            for _ in 0..N_THREADS_10 {
                let c = counter.clone();
                handles.push(thread::spawn(move || {
                    for _ in 0..N_PER_THREAD_10 {
                        std::hint::black_box(mutex_alloc(&c));
                    }
                }));
            }
            for h in handles {
                h.join().unwrap();
            }
        });
    });

    group.bench_function("atomic", |b| {
        b.iter(|| {
            let counter = Arc::new(AtomicU64::new(0));
            let mut handles = Vec::with_capacity(N_THREADS_10);
            for _ in 0..N_THREADS_10 {
                let c = counter.clone();
                handles.push(thread::spawn(move || {
                    for _ in 0..N_PER_THREAD_10 {
                        std::hint::black_box(atomic_alloc(&c));
                    }
                }));
            }
            for h in handles {
                h.join().unwrap();
            }
        });
    });

    group.finish();
}

// ── 3. 100 线程高争用 ────────────────────────────────────────────────────

fn bench_100_threads(c: &mut Criterion) {
    let mut group = c.benchmark_group("threads_100");
    group.throughput(Throughput::Elements(
        N_THREADS_100 as u64 * N_PER_THREAD_100,
    ));

    group.bench_function("mutex", |b| {
        b.iter(|| {
            let counter = Arc::new(Mutex::new(0u64));
            let mut handles = Vec::with_capacity(N_THREADS_100);
            for _ in 0..N_THREADS_100 {
                let c = counter.clone();
                handles.push(thread::spawn(move || {
                    for _ in 0..N_PER_THREAD_100 {
                        std::hint::black_box(mutex_alloc(&c));
                    }
                }));
            }
            for h in handles {
                h.join().unwrap();
            }
        });
    });

    group.bench_function("atomic", |b| {
        b.iter(|| {
            let counter = Arc::new(AtomicU64::new(0));
            let mut handles = Vec::with_capacity(N_THREADS_100);
            for _ in 0..N_THREADS_100 {
                let c = counter.clone();
                handles.push(thread::spawn(move || {
                    for _ in 0..N_PER_THREAD_100 {
                        std::hint::black_box(atomic_alloc(&c));
                    }
                }));
            }
            for h in handles {
                h.join().unwrap();
            }
        });
    });

    group.finish();
}

// ── 4. 稳态吞吐（参数化：1k/10k/100k/1M） ────────────────────────────────

fn bench_throughput_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");
    for &n in &[1_000u64, 10_000, 100_000, 1_000_000] {
        group.throughput(Throughput::Elements(n));
        group.bench_with_input(BenchmarkId::new("mutex", n), &n, |b, &n| {
            let counter = Mutex::new(0u64);
            b.iter(|| {
                for _ in 0..n {
                    std::hint::black_box(mutex_alloc(&counter));
                }
            });
        });
        group.bench_with_input(BenchmarkId::new("atomic", n), &n, |b, &n| {
            let counter = AtomicU64::new(0);
            b.iter(|| {
                for _ in 0..n {
                    std::hint::black_box(atomic_alloc(&counter));
                }
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_single_thread,
    bench_10_threads,
    bench_100_threads,
    bench_throughput_scaling
);
criterion_main!(benches);
