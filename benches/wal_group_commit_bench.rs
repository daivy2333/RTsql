use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rtsql::storage::RowId;
use rtsql::wal::{WalRecord, WalWriter, WALBuffer};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static TX_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// 创建临时 WAL 文件 + WALBuffer
fn create_wal_buffer(capacity: usize, flush_interval_ms: u64) -> Arc<WALBuffer> {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("bench.db");
    let wal_writer = Arc::new(WalWriter::open(&db_path).unwrap());
    let buffer = Arc::new(WALBuffer::new(wal_writer, capacity, flush_interval_ms));
    buffer.start_flush_loop();
    // Leak TempDir so WAL file stays alive
    std::mem::forget(dir);
    buffer
}

/// 生成测试用 Insert 记录
fn make_insert_record(tx_id: u64, i: u64) -> WalRecord {
    WalRecord::Insert {
        tx_id,
        table_name: "bench_table".to_string(),
        row_id: RowId::new((i % 1000) as u32, (i % 65535) as u16),
        tuple_data: vec![1u8, 2, 3, 4, 5, 6, 7, 8],
    }
}

// ── Group 1: wal_baseline — 逐条 fsync 基线 ────────────────────────────────

fn bench_wal_baseline(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("wal_baseline");
    group
        .sample_size(50)
        .measurement_time(std::time::Duration::from_secs(10));
    group.throughput(Throughput::Elements(1000));

    group.bench_function("per_fsync_capacity1", |b| {
        b.to_async(&rt).iter(|| async {
            let buffer = create_wal_buffer(1, 0);
            let base_tx = TX_ID_COUNTER.fetch_add(1, Ordering::SeqCst);

            for i in 0..1000u64 {
                let record = make_insert_record(base_tx, i);
                buffer.append(record).await;
            }
            buffer
                .append(WalRecord::CommitTxn {
                    tx_id: base_tx,
                    timestamp: 0,
                })
                .await;
            buffer.append_commit_and_wait(base_tx).await.unwrap();
            buffer.shutdown().await;
        });
    });

    group.finish();
}

// ── Group 2: wal_group_commit — Group Commit 并发吞吐 ──────────────────────

fn bench_wal_group_commit(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("wal_group_commit");
    group
        .sample_size(50)
        .measurement_time(std::time::Duration::from_secs(10));

    for concurrency in [1usize, 4, 8, 16, 32] {
        let records_per_thread: u64 = 200;
        let total_records = concurrency as u64 * records_per_thread;
        group.throughput(Throughput::Elements(total_records));
        group.bench_function(BenchmarkId::new("concurrent_insert", concurrency), |b| {
            b.to_async(&rt).iter(|| {
                let buffer = create_wal_buffer(100, 100);
                async move {
                    let mut handles = vec![];
                    for _ in 0..concurrency {
                        let buffer = buffer.clone();
                        let tx_id = TX_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
                        handles.push(tokio::spawn(async move {
                            for i in 0..records_per_thread {
                                let record = make_insert_record(tx_id, i);
                                buffer.append(record).await;
                            }
                            buffer
                                .append(WalRecord::CommitTxn {
                                    tx_id,
                                    timestamp: 0,
                                })
                                .await;
                            buffer.append_commit_and_wait(tx_id).await.unwrap();
                        }));
                    }
                    for h in handles {
                        let _ = h.await;
                    }
                    buffer.shutdown().await;
                }
            });
        });
    }

    group.finish();
}

// ── Group 3: wal_capacity_impact — capacity 参数影响 ───────────────────────

fn bench_wal_capacity_impact(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("wal_capacity_impact");
    group
        .sample_size(50)
        .measurement_time(std::time::Duration::from_secs(10));

    let concurrency: usize = 8;
    let records_per_thread: u64 = 200;
    let total_records = concurrency as u64 * records_per_thread;

    for capacity in [1usize, 10, 100] {
        group.throughput(Throughput::Elements(total_records));
        group.bench_function(BenchmarkId::new("capacity", capacity), |b| {
            b.to_async(&rt).iter(|| {
                let buffer = create_wal_buffer(capacity, 100);
                async move {
                    let mut handles = vec![];
                    for _ in 0..concurrency {
                        let buffer = buffer.clone();
                        let tx_id = TX_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
                        handles.push(tokio::spawn(async move {
                            for i in 0..records_per_thread {
                                let record = make_insert_record(tx_id, i);
                                buffer.append(record).await;
                            }
                            buffer
                                .append(WalRecord::CommitTxn {
                                    tx_id,
                                    timestamp: 0,
                                })
                                .await;
                            buffer.append_commit_and_wait(tx_id).await.unwrap();
                        }));
                    }
                    for h in handles {
                        let _ = h.await;
                    }
                    buffer.shutdown().await;
                }
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_wal_baseline,
    bench_wal_group_commit,
    bench_wal_capacity_impact,
);
criterion_main!(benches);
