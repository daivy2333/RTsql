# WAL Group Commit 性能基准测试 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新建 WAL 层 benchmark，验证 Group Commit 相比逐条 fsync 的 5-10x throughput 提升

**Architecture:** 独立 WAL 层 benchmark（benches/wal_group_commit_bench.rs），直接操作 WALBuffer，3 个 benchmark group：baseline（逐条 fsync）、group_commit（并发吞吐）、capacity_impact（capacity 参数影响）

**Tech Stack:** criterion 0.5 + async_tokio, tokio runtime, tempfile, rtsql::wal::{WALBuffer, WalWriter, WalRecord}

---

### Task 1: 添加 Cargo.toml bench 入口

**Files:**
- Modify: `Cargo.toml:44-46`

- [ ] **Step 1: 添加 [[bench]] 入口**

在 `Cargo.toml` 最后一个 `[[bench]]` 后追加：

```toml
[[bench]]
name = "wal_group_commit_bench"
harness = false
```

- [ ] **Step 2: 验证编译**

Run: `cargo check --benches 2>&1 | tail -5`
Expected: 编译错误（bench 文件尚不存在），但 Cargo.toml 语法正确

---

### Task 2: 创建 benchmark 文件

**Files:**
- Create: `benches/wal_group_commit_bench.rs`

- [ ] **Step 1: 写入 benchmark 代码**

```rust
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
        tuple_data: vec![1u8, 2, 3, 4, 5, 6, 7, 8], // 8 bytes dummy data
    }
}

// ── Group 1: wal_baseline — 逐条 fsync 基线 ────────────────────────────────

fn bench_wal_baseline(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("wal_baseline");
    group.sample_size(50).measurement_time(std::time::Duration::from_secs(10));
    group.throughput(Throughput::Elements(1000));

    group.bench_function("per_fsync_capacity1", |b| {
        b.to_async(&rt).iter(|| async {
            let buffer = create_wal_buffer(1, 0);
            let base_tx = TX_ID_COUNTER.fetch_add(1, Ordering::SeqCst);

            for i in 0..1000u64 {
                let record = make_insert_record(base_tx, i);
                buffer.append(record).await;
            }
            // Commit the transaction
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
    group.sample_size(50).measurement_time(std::time::Duration::from_secs(10));

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
    group.sample_size(50).measurement_time(std::time::Duration::from_secs(10));

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
```

- [ ] **Step 2: 验证编译**

Run: `cargo check --bench wal_group_commit_bench 2>&1 | tail -10`
Expected: 编译成功（无 error）

- [ ] **Step 3: 运行 benchmark（短时间验证）**

Run: `cargo bench --bench wal_group_commit_bench -- --quick 2>&1 | tail -30`
Expected: 3 个 group 都有输出，无 panic

---

### Task 3: 验证 + Clippy + 提交

**Files:**
- 无新文件

- [ ] **Step 1: 运行全量测试**

Run: `cargo test 2>&1 | tail -5`
Expected: "test result: ok. 417 passed, 0 failed"

- [ ] **Step 2: 运行 Clippy**

Run: `cargo clippy --benches 2>&1 | tail -5`
Expected: 0 warnings

- [ ] **Step 3: 运行完整 benchmark**

Run: `cargo bench --bench wal_group_commit_bench 2>&1 | tail -40`
Expected: 所有 benchmark 完成无错误，Group Commit throughput 明显高于 baseline

- [ ] **Step 4: 提交**

```bash
git add benches/wal_group_commit_bench.rs Cargo.toml
git commit -m "feat(M18-T7): add WAL Group Commit performance benchmark"
```
