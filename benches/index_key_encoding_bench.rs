//! 候选键布局的页级吞吐。输入在计时外生成；冷读每次使用新的空缓冲池。

#[path = "../tests/support/index_key_encoding.rs"]
mod index_key_encoding;

use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use index_key_encoding::{
    decode_fixed32_string, decode_ordered_int, decode_ordered_str, encode_for_layout, pack_records,
    read_prefix_step, read_slot, records_for, values_common_string, values_long_string,
    values_short_int, values_symmetric_int, values_wide_int, write_pages, CountingStorage, Domain,
    Layout, LogicalValue, Record, TailKind, HOT_SET_LEN, WORKLOAD_LEN,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rtsql::storage::{BufferPool, PageId};
use tokio::runtime::Runtime;

const POOL_CAPACITY: usize = 64;

struct Target {
    name: String,
    layout: Layout,
    kind: TailKind,
    values: Arc<Vec<LogicalValue>>,
}

fn configure<M: criterion::measurement::Measurement>(group: &mut criterion::BenchmarkGroup<'_, M>) {
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(1));
}

fn screen_targets() -> Vec<Target> {
    let short = Arc::new(values_short_int());
    let common = Arc::new(values_common_string());
    let wide = Arc::new(values_wide_int());
    let long64 = Arc::new(values_long_string(64));
    let mut targets = Vec::new();
    for (workload, values, kinds, layouts) in [
        (
            "short-int",
            short,
            vec![TailKind::Leaf, TailKind::Internal],
            vec![
                Layout::Fixed32,
                Layout::OrderedVariable,
                Layout::PrefixRestart,
            ],
        ),
        (
            "common-string",
            common,
            vec![TailKind::Leaf, TailKind::Internal],
            vec![
                Layout::Fixed32,
                Layout::OrderedVariable,
                Layout::PrefixRestart,
            ],
        ),
        (
            "wide-int",
            wide,
            vec![TailKind::Leaf],
            vec![
                Layout::Fixed32,
                Layout::OrderedVariable,
                Layout::PrefixRestart,
            ],
        ),
        (
            "long-string-64",
            long64,
            vec![TailKind::Leaf],
            vec![Layout::OrderedVariable, Layout::PrefixRestart],
        ),
    ] {
        for kind in kinds {
            for layout in &layouts {
                targets.push(Target {
                    name: format!("{workload}/{kind:?}/{layout:?}"),
                    layout: *layout,
                    kind,
                    values: Arc::clone(&values),
                });
            }
        }
    }
    targets
}

fn roundtrip(layout: Layout, value: &LogicalValue) -> LogicalValue {
    let encoded = encode_for_layout(layout, value).expect("encode");
    match (layout, value) {
        (Layout::Fixed32, LogicalValue::Int(_)) => {
            let mut be = [0u8; 8];
            be.copy_from_slice(&encoded[..8]);
            LogicalValue::Int(i64::from_be_bytes(be))
        }
        (Layout::Fixed32, LogicalValue::Str(_)) => {
            let mut key = [0u8; 32];
            key.copy_from_slice(&encoded);
            LogicalValue::Str(decode_fixed32_string(&key).expect("fixed32 string"))
        }
        (_, LogicalValue::Int(_)) => LogicalValue::Int(decode_ordered_int(&encoded).expect("int")),
        (_, LogicalValue::Str(_)) => {
            LogicalValue::Str(decode_ordered_str(&encoded).expect("string"))
        }
    }
}

fn bench_encode_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_decode");
    configure(&mut group);
    group.throughput(Throughput::Elements(WORKLOAD_LEN as u64));
    let sets = [
        ("short-int", values_short_int()),
        ("symmetric-int", values_symmetric_int()),
        ("wide-int", values_wide_int()),
        ("common-string", values_common_string()),
        ("long-string-32", values_long_string(32)),
        ("long-string-64", values_long_string(64)),
        ("long-string-256", values_long_string(256)),
    ];
    for (name, values) in &sets {
        let layouts: &[Layout] = if name.starts_with("long-") {
            &[Layout::OrderedVariable, Layout::PrefixRestart]
        } else {
            &[
                Layout::Fixed32,
                Layout::OrderedVariable,
                Layout::PrefixRestart,
            ]
        };
        for layout in layouts {
            group.bench_function(format!("{name}/{layout:?}"), |b| {
                b.iter(|| {
                    for value in values {
                        let decoded = roundtrip(*layout, value);
                        assert_eq!(&decoded, value);
                        black_box(decoded);
                    }
                });
            });
        }
    }
    group.finish();
}

fn bench_pack(c: &mut Criterion) {
    let mut group = c.benchmark_group("pack_pages");
    configure(&mut group);
    group.throughput(Throughput::Elements(WORKLOAD_LEN as u64));
    for target in screen_targets() {
        let records = records_for(&target.values, target.kind);
        group.bench_function(&target.name, |b| {
            b.iter(|| {
                let packed = pack_records(target.layout, &records).expect("pack");
                black_box(packed.pages.len() + packed.payload_bytes as usize);
            });
        });
    }
    group.finish();
}

fn bench_write(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    let mut group = c.benchmark_group("write_sync");
    configure(&mut group);
    group.throughput(Throughput::Elements(WORKLOAD_LEN as u64));
    for target in screen_targets() {
        let records = records_for(&target.values, target.kind);
        group.bench_function(&target.name, |b| {
            b.iter_batched(
                || {
                    let dir = tempfile::tempdir().expect("tempdir");
                    let storage =
                        Arc::new(CountingStorage::open(&dir.path().join("db")).expect("open"));
                    let pool = BufferPool::new(POOL_CAPACITY, storage.clone()).expect("pool");
                    (dir, storage, pool)
                },
                |(dir, storage, pool)| {
                    rt.block_on(async {
                        let packed = pack_records(target.layout, &records).expect("pack");
                        let ids = write_pages(&storage, &packed.pages).await.expect("write");
                        black_box((ids.len(), pool.capacity()));
                    });
                    drop(pool);
                    drop(storage);
                    drop(dir);
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

struct Loaded {
    _dir: tempfile::TempDir,
    storage: Arc<CountingStorage>,
    records: Vec<Record>,
    accesses: Vec<(PageId, usize)>,
    layout: Layout,
}

async fn load_target(target: &Target) -> Loaded {
    let records = records_for(&target.values, target.kind);
    let packed = pack_records(target.layout, &records).expect("pack");
    let dir = tempfile::tempdir().expect("tempdir");
    let storage = Arc::new(CountingStorage::open(&dir.path().join("db")).expect("open"));
    let ids = write_pages(&storage, &packed.pages).await.expect("write");
    let accesses = packed
        .locations
        .iter()
        .map(|loc| (ids[loc.page_index], loc.slot_index))
        .collect();
    Loaded {
        _dir: dir,
        storage,
        records,
        accesses,
        layout: target.layout,
    }
}

async fn scan(
    pool: &BufferPool,
    layout: Layout,
    records: &[Record],
    accesses: &[(PageId, usize)],
    repeats: usize,
) -> usize {
    let domain = match records[0].value {
        LogicalValue::Int(_) => Domain::Int,
        LogicalValue::Str(_) => Domain::Str,
    };
    let kind = records[0].tail.kind();
    let mut seen = 0usize;
    for _ in 0..repeats {
        let mut carried: Option<(PageId, usize, Vec<u8>)> = None;
        for (index, (page_id, slot)) in accesses.iter().enumerate() {
            let guard = pool.get_page(*page_id).await.expect("get_page");
            let view = guard.page_data();
            let value = if layout == Layout::PrefixRestart {
                let adjacent = matches!(
                    &carried,
                    Some((prev_page, prev_slot, _))
                        if prev_page == page_id && *prev_slot + 1 == *slot
                );
                if adjacent || *slot % index_key_encoding::RESTART_INTERVAL == 0 {
                    let previous = carried.as_ref().and_then(|(prev_page, prev_slot, key)| {
                        if prev_page == page_id && *prev_slot + 1 == *slot {
                            Some(key.as_slice())
                        } else {
                            None
                        }
                    });
                    let (value, _, key) =
                        read_prefix_step(domain, kind, &view, *slot, previous).expect("decode");
                    carried = Some((*page_id, *slot, key));
                    value
                } else {
                    let (value, _) = read_slot(layout, domain, kind, &view, *slot).expect("decode");
                    carried = None;
                    value
                }
            } else {
                let (value, _) = read_slot(layout, domain, kind, &view, *slot).expect("decode");
                value
            };
            assert_eq!(value, records[index].value);
            seen += 1;
            black_box(seen);
        }
    }
    seen
}

fn bench_cold_read(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    let mut group = c.benchmark_group("cold_read");
    configure(&mut group);
    group.throughput(Throughput::Elements(WORKLOAD_LEN as u64));
    for target in screen_targets() {
        let loaded = rt.block_on(load_target(&target));
        group.bench_function(&target.name, |b| {
            b.iter_batched(
                || BufferPool::new(POOL_CAPACITY, loaded.storage.clone()).expect("pool"),
                |pool| {
                    rt.block_on(async {
                        black_box(
                            scan(&pool, loaded.layout, &loaded.records, &loaded.accesses, 1).await,
                        );
                    });
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

fn bench_hot_read(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    let mut group = c.benchmark_group("hot_read");
    configure(&mut group);
    let hot_elems = (HOT_SET_LEN as u64) * 4;
    group.throughput(Throughput::Elements(hot_elems));
    for target in screen_targets() {
        let loaded = rt.block_on(load_target(&target));
        let hot_len = HOT_SET_LEN.min(loaded.accesses.len());
        group.bench_function(&target.name, |b| {
            b.iter_batched(
                || {
                    let pool =
                        BufferPool::new(POOL_CAPACITY, loaded.storage.clone()).expect("pool");
                    rt.block_on(async {
                        scan(
                            &pool,
                            loaded.layout,
                            &loaded.records,
                            &loaded.accesses[..hot_len],
                            1,
                        )
                        .await;
                    });
                    pool
                },
                |pool| {
                    rt.block_on(async {
                        black_box(
                            scan(
                                &pool,
                                loaded.layout,
                                &loaded.records,
                                &loaded.accesses[..hot_len],
                                4,
                            )
                            .await,
                        );
                    });
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

fn bench_random_read(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    let mut group = c.benchmark_group("random_read");
    configure(&mut group);
    group.throughput(Throughput::Elements(HOT_SET_LEN as u64));
    let mut rng = StdRng::seed_from_u64(38);
    let positions = (0..HOT_SET_LEN)
        .map(|_| rng.gen_range(0..WORKLOAD_LEN))
        .collect::<Vec<_>>();
    for target in screen_targets().into_iter().filter(|target| {
        target.kind == TailKind::Leaf
            && matches!(
                target.layout,
                Layout::OrderedVariable | Layout::PrefixRestart
            )
            && (target.name.starts_with("short-int/") || target.name.starts_with("common-string/"))
    }) {
        let loaded = rt.block_on(load_target(&target));
        let accesses = positions
            .iter()
            .map(|index| loaded.accesses[*index])
            .collect::<Vec<_>>();
        let records = positions
            .iter()
            .map(|index| loaded.records[*index].clone())
            .collect::<Vec<_>>();
        group.bench_function(&target.name, |b| {
            b.iter_batched(
                || {
                    let pool =
                        BufferPool::new(POOL_CAPACITY, loaded.storage.clone()).expect("pool");
                    rt.block_on(async {
                        let mut seen = std::collections::HashSet::new();
                        for (page_id, _) in &accesses {
                            if seen.insert(*page_id) {
                                pool.get_page(*page_id).await.expect("warm");
                            }
                        }
                    });
                    pool
                },
                |pool| {
                    rt.block_on(async {
                        black_box(scan(&pool, loaded.layout, &records, &accesses, 1).await);
                    });
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

fn bench_entry(c: &mut Criterion) {
    bench_encode_decode(c);
    bench_pack(c);
    bench_write(c);
    bench_cold_read(c);
    bench_hot_read(c);
    bench_random_read(c);
}

criterion_group!(benches, bench_entry);
criterion_main!(benches);
