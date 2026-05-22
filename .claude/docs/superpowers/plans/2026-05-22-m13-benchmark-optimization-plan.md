# M13: Performance Benchmark & Critical Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立 criterion.rs 基准测试框架量化性能，修复 Critical 性能问题（PageGuard 零拷贝、BufferPool 两阶段锁），对比修复前后数据。

**Architecture:** Phase 1 搭建 benchmark 框架并记录基线 → Phase 2 修复 Critical 问题 → Phase 3 重新 benchmark 对比。Benchmark 通过 `database.execute_sql()` 执行 SQL，无需直接操作底层 executor。

**Tech Stack:** criterion 0.5, rusqlite 0.31, tempfile 3.0

---

## File Structure

```
benches/
├── common/
│   └── mod.rs          # 共享辅助：数据库初始化/清理/建表/数据生成
├── micro_bench.rs      # 单操作延迟（INSERT/SELECT/UPDATE/DELETE/SCAN/FILTER/SORT/LIMIT/JOIN）
├── concurrent_bench.rs # 并发压力（多连接读写、混合、事务冲突）
├── scale_bench.rs      # 规模扩展（1K/10K/100K/1M 行）
└── sqlite_compare.rs   # SQLite 对比（相同操作 vs rusqlite）

src/storage/page_frame.rs  # 新增 page_data() 零拷贝方法
src/storage/buffer_pool.rs # 重构 get_page() 两阶段锁
src/storage/data_page.rs   # 迁移 read_tuple_from_data_page 到 page_data()
```

---

### Task 1: 添加 criterion + rusqlite 依赖

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: 添加 dev-dependencies 和 bench 配置**

在 `Cargo.toml` 末尾追加：

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
rusqlite = "0.31"

[[bench]]
name = "micro_bench"
harness = false

[[bench]]
name = "concurrent_bench"
harness = false

[[bench]]
name = "scale_bench"
harness = false

[[bench]]
name = "sqlite_compare"
harness = false
```

- [ ] **Step 2: 验证依赖可编译**

Run: `cargo check`
Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore(M13): add criterion + rusqlite dev-dependencies"
```

---

### Task 2: 创建 benchmark 共享辅助模块

**Files:**
- Create: `benches/common/mod.rs`

- [ ] **Step 1: 编写共享辅助代码**

```rust
use rtsql::database::Database;
use std::path::{Path, PathBuf};

/// 创建临时目录并打开数据库
pub async fn setup_db() -> (PathBuf, Database) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("bench.db");
    let db = Database::open(&db_path).await.unwrap();
    // Leak TempDir so it stays alive for the benchmark duration
    std::mem::forget(dir);
    (db_path, db)
}

/// 清理数据库文件
pub fn cleanup_db(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db.wal"));
    let _ = std::fs::remove_file(path.with_extension("db.checkpoint"));
}

/// 创建标准测试表 (id INT PK, name STRING, value INT)
pub async fn create_test_table(db: &Database) {
    db.execute_sql("CREATE TABLE bench (id INT PRIMARY KEY, name STRING, value INT)")
        .await;
}

/// 插入 n 行数据到 bench 表
pub async fn insert_rows(db: &Database, start: i64, n: i64) {
    for i in start..start + n {
        db.execute_sql(&format!(
            "INSERT INTO bench VALUES ({}, 'user_{}', {})",
            i, i, i * 10
        ))
        .await;
    }
}

/// 创建 JOIN 测试表 (orders + customers)
pub async fn create_join_tables(db: &Database) {
    db.execute_sql("CREATE TABLE customers (id INT PRIMARY KEY, name STRING)")
        .await;
    db.execute_sql("CREATE TABLE orders (id INT PRIMARY KEY, customer_id INT, amount INT)")
        .await;
}

/// 插入 JOIN 测试数据
pub async fn insert_join_data(db: &Database, n: i64) {
    for i in 0..n {
        db.execute_sql(&format!(
            "INSERT INTO customers VALUES ({}, 'customer_{}')",
            i, i
        ))
        .await;
        db.execute_sql(&format!(
            "INSERT INTO orders VALUES ({}, {}, {})",
            i, i, i * 100
        ))
        .await;
    }
}
```

- [ ] **Step 2: 验证编译**

Run: `cargo check --tests`
Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git add benches/
git commit -m "feat(M13): add benchmark common helper module"
```

---

### Task 3: 创建微基准测试（micro_bench.rs）

**Files:**
- Create: `benches/micro_bench.rs`

- [ ] **Step 1: 编写微基准测试**

```rust
mod common;

use common::*;
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use rtsql::database::Database;
use std::path::PathBuf;

fn bench_insert(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        (p, d)
    });

    let mut group = c.benchmark_group("insert");
    for i in 0..100i64 {
        group.bench_function(BenchmarkId::new("single_row", i), |b| {
            b.to_async(&rt).iter(|| {
                let db = db.clone();
                async move {
                    db.execute_sql(&format!(
                        "INSERT INTO bench VALUES ({}, 'user_{}', {})",
                        i * 1000, i, i * 10
                    ))
                    .await;
                }
            });
        });
    }
    group.finish();
    cleanup_db(&path);
}

fn bench_select(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    let mut group = c.benchmark_group("select");
    for i in 0..100i64 {
        group.bench_function(BenchmarkId::new("pk_lookup", i), |b| {
            b.to_async(&rt).iter(|| {
                let db = db.clone();
                async move {
                    db.execute_sql(&format!("SELECT * FROM bench WHERE id = {}", i)).await;
                }
            });
        });
    }
    group.finish();
    cleanup_db(&path);
}

fn bench_update(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    let mut group = c.benchmark_group("update");
    for i in 0..100i64 {
        group.bench_function(BenchmarkId::new("single_col", i), |b| {
            b.to_async(&rt).iter(|| {
                let db = db.clone();
                async move {
                    db.execute_sql(&format!("UPDATE bench SET value = {} WHERE id = {}", i * 99, i))
                        .await;
                }
            });
        });
    }
    group.finish();
    cleanup_db(&path);
}

fn bench_delete(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 200).await;
        (p, d)
    });

    let mut group = c.benchmark_group("delete");
    for i in 100..200i64 {
        group.bench_function(BenchmarkId::new("by_pk", i), |b| {
            b.to_async(&rt).iter(|| {
                let db = db.clone();
                async move {
                    db.execute_sql(&format!("DELETE FROM bench WHERE id = {}", i)).await;
                }
            });
        });
    }
    group.finish();
    cleanup_db(&path);
}

fn bench_scan(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    c.bench_function("scan_full_table_100_rows", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move { db.execute_sql("SELECT * FROM bench").await }
        });
    });
    cleanup_db(&path);
}

fn bench_filter(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    c.bench_function("filter_where_value_gt_500", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move { db.execute_sql("SELECT * FROM bench WHERE value > 500").await }
        });
    });
    cleanup_db(&path);
}

fn bench_sort(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    c.bench_function("order_by_value_desc", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move { db.execute_sql("SELECT * FROM bench ORDER BY value DESC").await }
        });
    });
    cleanup_db(&path);
}

fn bench_limit(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    c.bench_function("limit_10_offset_5", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                db.execute_sql("SELECT * FROM bench ORDER BY id LIMIT 10 OFFSET 5")
                    .await
            }
        });
    });
    cleanup_db(&path);
}

fn bench_join(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_join_tables(&d).await;
        insert_join_data(&d, 50).await;
        (p, d)
    });

    c.bench_function("inner_join_50_rows", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                db.execute_sql(
                    "SELECT customers.id, customers.name, orders.amount \
                     FROM customers INNER JOIN orders ON customers.id = orders.customer_id",
                )
                .await
            }
        });
    });
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
```

- [ ] **Step 2: 验证编译**

Run: `cargo bench --bench micro_bench -- --list`
Expected: 列出所有 benchmark 函数名

- [ ] **Step 3: 运行微基准测试（快速模式，少量迭代）**

Run: `cargo bench --bench micro_bench -- --quick`
Expected: 所有 benchmark 运行完成，输出统计信息

- [ ] **Step 4: Commit**

```bash
git add benches/micro_bench.rs
git commit -m "feat(M13): add micro benchmark (INSERT/SELECT/UPDATE/DELETE/SCAN/FILTER/SORT/LIMIT/JOIN)"
```

---

### Task 4: 创建并发压力测试（concurrent_bench.rs）

**Files:**
- Create: `benches/concurrent_bench.rs`

- [ ] **Step 1: 编写并发压力测试**

```rust
mod common;

use common::*;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rtsql::database::Database;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

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
        group.throughput(Throughput::Elements(concurrency as u64 * 100));
        group.bench_function(BenchmarkId::new("select", concurrency), |b| {
            b.to_async(&rt).iter(|| async {
                let db = db.clone();
                let mut handles = vec![];
                for _ in 0..concurrency {
                    let db = db.clone();
                    handles.push(tokio::spawn(async move {
                        for i in 0..100i64 {
                            db.execute_sql(&format!("SELECT * FROM bench WHERE id = {}", i))
                                .await;
                        }
                    }));
                }
                for h in handles {
                    h.await.unwrap();
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
            b.to_async(&rt).iter(|| async {
                let db = db.clone();
                let base = CONCURRENT_COUNTER.fetch_add(concurrency as i64 * 50, Ordering::SeqCst);
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
                    h.await.unwrap();
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
            b.to_async(&rt).iter(|| async {
                let db = db.clone();
                let base = CONCURRENT_COUNTER.fetch_add(concurrency as i64 * 20, Ordering::SeqCst);
                let mut handles = vec![];
                for t in 0..concurrency {
                    let db = db.clone();
                    let write_start = base + (t as i64) * 20;
                    handles.push(tokio::spawn(async move {
                        // 80 reads
                        for i in 0..80i64 {
                            db.execute_sql(&format!("SELECT * FROM bench WHERE id = {}", i % 500))
                                .await;
                        }
                        // 20 writes
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
                    h.await.unwrap();
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
            b.to_async(&rt).iter(|| async {
                let db = db.clone();
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
                    h.await.unwrap();
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
```

- [ ] **Step 2: 验证编译**

Run: `cargo bench --bench concurrent_bench -- --list`
Expected: 列出所有 benchmark 函数名

- [ ] **Step 3: Commit**

```bash
git add benches/concurrent_bench.rs
git commit -m "feat(M13): add concurrent benchmark (read/write/mixed/conflict)"
```

---

### Task 5: 创建规模扩展测试（scale_bench.rs）

**Files:**
- Create: `benches/scale_bench.rs`

- [ ] **Step 1: 编写规模扩展测试**

```rust
mod common;

use common::*;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rtsql::database::Database;

fn bench_scale_insert(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("scale_insert");

    for &n in &[1_000i64, 10_000, 100_000] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(BenchmarkId::new("rows", n), |b| {
            b.iter(|| {
                rt.block_on(async {
                    let (path, db) = setup_db().await;
                    create_test_table(&db).await;
                    for i in 0..n {
                        db.execute_sql(&format!(
                            "INSERT INTO bench VALUES ({}, 'user_{}', {})",
                            i, i, i * 10
                        ))
                        .await;
                    }
                    cleanup_db(&path);
                });
            });
        });
    }
    group.finish();
}

fn bench_scale_select(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("scale_select");

    for &n in &[1_000i64, 10_000, 100_000] {
        let (path, db) = rt.block_on(async {
            let (p, d) = setup_db().await;
            create_test_table(&d).await;
            insert_rows(&d, 0, n).await;
            (p, d)
        });

        group.throughput(Throughput::Elements(1));
        group.bench_function(BenchmarkId::new("pk_lookup", n), |b| {
            b.to_async(&rt).iter(|| {
                let db = db.clone();
                async move { db.execute_sql("SELECT * FROM bench WHERE id = 42").await }
            });
        });
        cleanup_db(&path);
    }
    group.finish();
}

fn bench_scale_scan(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("scale_scan");

    for &n in &[1_000i64, 10_000, 100_000] {
        let (path, db) = rt.block_on(async {
            let (p, d) = setup_db().await;
            create_test_table(&d).await;
            insert_rows(&d, 0, n).await;
            (p, d)
        });

        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(BenchmarkId::new("full_scan", n), |b| {
            b.to_async(&rt).iter(|| {
                let db = db.clone();
                async move { db.execute_sql("SELECT * FROM bench").await }
            });
        });
        cleanup_db(&path);
    }
    group.finish();
}

fn bench_scale_join(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("scale_join");

    for &n in &[100i64, 1_000, 10_000] {
        let (path, db) = rt.block_on(async {
            let (p, d) = setup_db().await;
            create_join_tables(&d).await;
            insert_join_data(&d, n).await;
            (p, d)
        });

        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(BenchmarkId::new("inner_join", n), |b| {
            b.to_async(&rt).iter(|| {
                let db = db.clone();
                async move {
                    db.execute_sql(
                        "SELECT customers.id, customers.name, orders.amount \
                         FROM customers INNER JOIN orders ON customers.id = orders.customer_id",
                    )
                    .await
                }
            });
        });
        cleanup_db(&path);
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_scale_insert,
    bench_scale_select,
    bench_scale_scan,
    bench_scale_join,
);
criterion_main!(benches);
```

- [ ] **Step 2: 验证编译**

Run: `cargo bench --bench scale_bench -- --list`
Expected: 列出所有 benchmark 函数名

- [ ] **Step 3: Commit**

```bash
git add benches/scale_bench.rs
git commit -m "feat(M13): add scale benchmark (1K/10K/100K rows)"
```

---

### Task 6: 创建 SQLite 对比测试（sqlite_compare.rs）

**Files:**
- Create: `benches/sqlite_compare.rs`

- [ ] **Step 1: 编写 SQLite 对比测试**

```rust
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rusqlite::Connection;
use std::path::PathBuf;

fn setup_sqlite() -> (PathBuf, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("sqlite_bench.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE bench (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)",
        [],
    )
    .unwrap();
    std::mem::forget(dir);
    (db_path, conn)
}

fn insert_sqlite_rows(conn: &Connection, start: i64, n: i64) {
    for i in start..start + n {
        conn.execute(
            "INSERT INTO bench VALUES (?1, ?2, ?3)",
            rusqlite::params![i, format!("user_{}", i), i * 10],
        )
        .unwrap();
    }
}

fn bench_sqlite_insert(c: &mut Criterion) {
    let (path, conn) = setup_sqlite();
    let mut group = c.benchmark_group("sqlite_vs_rtsql_insert");
    group.throughput(Throughput::Elements(100));

    group.bench_function("sqlite_insert_100", |b| {
        b.iter(|| {
            insert_sqlite_rows(&conn, 0, 100);
            conn.execute("DELETE FROM bench", []).unwrap();
        });
    });

    group.finish();
    let _ = std::fs::remove_file(&path);
}

fn bench_sqlite_select(c: &mut Criterion) {
    let (path, conn) = setup_sqlite();
    insert_sqlite_rows(&conn, 0, 1000);

    let mut group = c.benchmark_group("sqlite_vs_rtsql_select");
    group.throughput(Throughput::Elements(1));

    group.bench_function("sqlite_pk_lookup", |b| {
        b.iter(|| {
            let mut stmt = conn.prepare("SELECT * FROM bench WHERE id = 42").unwrap();
            let _ = stmt.query_row([], |row| {
                let id: i64 = row.get(0).unwrap();
                id
            });
        });
    });

    group.finish();
    let _ = std::fs::remove_file(&path);
}

fn bench_sqlite_scan(c: &mut Criterion) {
    let (path, conn) = setup_sqlite();
    insert_sqlite_rows(&conn, 0, 1000);

    let mut group = c.benchmark_group("sqlite_vs_rtsql_scan");
    group.throughput(Throughput::Elements(1000));

    group.bench_function("sqlite_full_scan_1k", |b| {
        b.iter(|| {
            let mut stmt = conn.prepare("SELECT * FROM bench").unwrap();
            let rows: Vec<i64> = stmt.query_map([], |row| row.get(0)).unwrap().map(|r| r.unwrap()).collect();
            rows.len()
        });
    });

    group.finish();
    let _ = std::fs::remove_file(&path);
}

fn bench_sqlite_join(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("sqlite_join.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT)", [])
        .unwrap();
    conn.execute(
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER, amount INTEGER)",
        [],
    )
    .unwrap();
    for i in 0..1000i64 {
        conn.execute(
            "INSERT INTO customers VALUES (?1, ?2)",
            rusqlite::params![i, format!("customer_{}", i)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO orders VALUES (?1, ?2, ?3)",
            rusqlite::params![i, i, i * 100],
        )
        .unwrap();
    }

    let mut group = c.benchmark_group("sqlite_vs_rtsql_join");
    group.throughput(Throughput::Elements(1000));

    group.bench_function("sqlite_inner_join_1k", |b| {
        b.iter(|| {
            let mut stmt = conn.prepare(
                "SELECT c.id, c.name, o.amount FROM customers c INNER JOIN orders o ON c.id = o.customer_id"
            ).unwrap();
            let rows: Vec<i64> = stmt.query_map([], |row| row.get(0)).unwrap().map(|r| r.unwrap()).collect();
            rows.len()
        });
    });

    group.finish();
    std::mem::forget(dir);
}

criterion_group!(
    benches,
    bench_sqlite_insert,
    bench_sqlite_select,
    bench_sqlite_scan,
    bench_sqlite_join,
);
criterion_main!(benches);
```

- [ ] **Step 2: 验证编译**

Run: `cargo bench --bench sqlite_compare -- --list`
Expected: 列出所有 benchmark 函数名

- [ ] **Step 3: Commit**

```bash
git add benches/sqlite_compare.rs
git commit -m "feat(M13): add SQLite comparison benchmark"
```

---

### Task 7: 运行基线 benchmark 并记录数据

**Files:** 无新文件

- [ ] **Step 1: 运行全部 benchmark 记录基线**

Run: `cargo bench -- --quick 2>&1 | tee /tmp/m13_baseline.txt`
Expected: 所有 benchmark 运行完成

- [ ] **Step 2: 验证现有测试仍然通过**

Run: `cargo test`
Expected: 所有测试通过

---

## Phase 2: Critical 修复

### Task 8: PageGuard 零拷贝 — 新增 page_data() 方法

**Files:**
- Modify: `src/storage/page_frame.rs`
- Test: `tests/page_frame_test.rs`（新增）

- [ ] **Step 1: 编写 page_data() 测试**

创建 `tests/page_frame_test.rs`：

```rust
use rtsql::storage::{BufferPool, FileStorage, PageId};
use std::sync::Arc;

#[tokio::test]
async fn test_page_guard_page_data_zero_copy() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let pool = BufferPool::new(10, storage).unwrap();

    let page_id = pool.storage().allocate_page().await.unwrap();
    let guard = pool.get_page(page_id).await.unwrap();

    // Write some data via modify_page
    guard.modify_page(|page| {
        page.data[0] = 0x42;
        page.data[1] = 0x43;
    });

    // Read via page_data() — zero copy
    let data = guard.page_data();
    assert_eq!(data[0], 0x42);
    assert_eq!(data[1], 0x43);
    assert_eq!(data.len(), 4096);
}

#[tokio::test]
async fn test_page_guard_page_data_matches_page() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let pool = BufferPool::new(10, storage).unwrap();

    let page_id = pool.storage().allocate_page().await.unwrap();
    let guard = pool.get_page(page_id).await.unwrap();

    guard.modify_page(|page| {
        page.data[100..104].copy_from_slice(&[1, 2, 3, 4]);
    });

    // page_data() and page() must return same content
    let data = guard.page_data();
    let page = guard.page();
    assert_eq!(&data[100..104], &[1, 2, 3, 4]);
    assert_eq!(&page.data[100..104], &[1, 2, 3, 4]);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test page_guard_page_data`
Expected: FAIL — `page_data` method does not exist

- [ ] **Step 3: 实现 page_data() 方法**

在 `src/storage/page_frame.rs` 的 `impl PageGuard` 中添加：

```rust
/// Get a reference to page data (zero-copy, no allocation)
/// SAFETY: Mutex not held across .await — the lock is acquired and released synchronously.
pub fn page_data(&self) -> &[u8] {
    &self.frame.lock().unwrap().page.data[..]
}
```

同时在 `src/storage/mod.rs` 中确保 `PageGuard` 已导出（已有）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test page_guard_page_data`
Expected: 2 passed

- [ ] **Step 5: 运行全部测试确认无回归**

Run: `cargo test`
Expected: 所有测试通过

- [ ] **Step 6: Commit**

```bash
git add src/storage/page_frame.rs tests/page_frame_test.rs
git commit -m "feat(M13): add PageGuard::page_data() zero-copy method"
```

---

### Task 9: 迁移 read_tuple_from_data_page 到 page_data()

**Files:**
- Modify: `src/storage/data_page.rs`
- Modify: `src/storage/btree/btree.rs`

- [ ] **Step 1: 修改 read_tuple_from_data_page 使用 page_data()**

在 `src/storage/data_page.rs` 中，将 `read_tuple_from_data_page` 函数体替换：

```rust
pub async fn read_tuple_from_data_page(
    buffer_pool: &BufferPool,
    row_id: RowId,
) -> Result<(VersionHeader, Vec<u8>)> {
    let page_id = PageId(row_id.page_id as u64);
    let guard = buffer_pool.get_page(page_id).await?;

    // Zero-copy: read page data directly without cloning 4KB
    let page_data = guard.page_data();
    let header = crate::storage::page_format::SlottedPageHeader::deserialize(&page_data[..16]);
    let slot_count = header.slot_count as usize;

    // Find slot at index
    let slot_idx = row_id.slot_id as usize;
    if slot_idx >= slot_count {
        return Err(StorageError::SlotNotFound(row_id));
    }

    let slot_start = 4096 - (slot_idx + 1) * 4;
    let offset = u16::from_le_bytes([page_data[slot_start], page_data[slot_start + 1]]) as usize;
    let length = u16::from_le_bytes([page_data[slot_start + 2], page_data[slot_start + 3]]) as usize;

    if offset == 0 && length == 0 {
        return Err(StorageError::SlotNotFound(row_id));
    }

    let slot_data = &page_data[offset..offset + length];

    let version_header =
        VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE]).ok_or_else(|| {
            StorageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "malformed version header",
            ))
        })?;

    let tuple_bytes = slot_data[VersionHeader::SIZE..].to_vec();

    Ok((version_header, tuple_bytes))
}
```

- [ ] **Step 2: 运行全部测试确认无回归**

Run: `cargo test`
Expected: 所有测试通过

- [ ] **Step 3: Commit**

```bash
git add src/storage/data_page.rs
git commit -m "perf(M13): migrate read_tuple_from_data_page to zero-copy page_data()"
```

---

### Task 10: BufferPool 两阶段锁异步化

**Files:**
- Modify: `src/storage/buffer_pool.rs`

- [ ] **Step 1: 编写两阶段锁测试**

在 `tests/` 中创建 `buffer_pool_async_test.rs`：

```rust
use rtsql::storage::{BufferPool, FileStorage, PageId};
use std::sync::Arc;

#[tokio::test]
async fn test_two_phase_lock_get_page() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let page_id = pool.storage().allocate_page().await.unwrap();

    // First get — loads from disk
    let guard1 = pool.get_page(page_id).await.unwrap();
    guard1.modify_page(|page| {
        page.data[0] = 0xAA;
    });
    drop(guard1);

    // Second get — should hit cache (no I/O during lock hold)
    let guard2 = pool.get_page(page_id).await.unwrap();
    let data = guard2.page_data();
    assert_eq!(data[0], 0xAA);
}

#[tokio::test]
async fn test_concurrent_get_page_no_deadlock() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let pool = Arc::new(BufferPool::new(100, storage).unwrap());

    // Allocate 10 pages
    let mut page_ids = vec![];
    for _ in 0..10 {
        page_ids.push(pool.storage().allocate_page().await.unwrap());
    }

    // Concurrent access — should not deadlock
    let mut handles = vec![];
    for i in 0..10 {
        let pool = pool.clone();
        let pid = page_ids[i];
        handles.push(tokio::spawn(async move {
            let guard = pool.get_page(pid).await.unwrap();
            guard.modify_page(|page| {
                page.data[0] = i as u8;
            });
        }));
    }

    for h in handles {
        h.await.unwrap();
    }
}
```

- [ ] **Step 2: 运行测试确认通过（当前实现）**

Run: `cargo test buffer_pool_async`
Expected: PASS（当前实现虽然持锁做 I/O，但功能正确）

- [ ] **Step 3: 重构 get_page() 为两阶段锁**

替换 `src/storage/buffer_pool.rs` 的 `get_page` 方法：

```rust
pub async fn get_page(&self, page_id: PageId) -> Result<PageGuard> {
    // Phase 1: Read lock — check cache
    {
        let pages = self.pages.read().await;
        if let Some(frame) = pages.get(&page_id) {
            return Ok(PageGuard::new(frame.clone()));
        }
    } // Read lock released here

    // Phase 2: Load page from storage WITHOUT holding any lock
    // This allows other coroutines to access the cache concurrently
    let page = self.storage.read_page(page_id).await?;

    // Phase 3: Write lock — insert with double-check
    let mut pages = self.pages.write().await;

    // Double-check: another coroutine may have loaded this page
    if let Some(frame) = pages.get(&page_id) {
        return Ok(PageGuard::new(frame.clone()));
    }

    // Evict if cache is full
    if pages.len() >= self.capacity {
        self.evict_one(&mut pages).await?;
    }

    // Insert the loaded page
    let frame = Arc::new(std::sync::Mutex::new(PageFrame::new(page)));
    pages.insert(page_id, frame.clone());
    self.clock_hand.write().await.push(page_id);

    Ok(PageGuard::new(frame))
}
```

- [ ] **Step 4: 运行全部测试确认无回归**

Run: `cargo test`
Expected: 所有测试通过

- [ ] **Step 5: Commit**

```bash
git add src/storage/buffer_pool.rs tests/buffer_pool_async_test.rs
git commit -m "perf(M13): two-phase lock in BufferPool::get_page() — no I/O during lock hold"
```

---

### Task 11: PageGuard Mutex 安全验证 + SAFETY 注释

**Files:**
- Modify: `src/storage/page_frame.rs`
- Modify: `src/storage/buffer_pool.rs`

- [ ] **Step 1: 审查所有 std::sync::Mutex 使用点**

Run: `grep -rn "std::sync::Mutex\|frame.lock()" src/storage/`
确认所有使用点不跨 `.await`。

- [ ] **Step 2: 添加 SAFETY 注释**

在 `src/storage/page_frame.rs` 的 `PageGuard` 结构体上方添加：

```rust
/// 页访问守卫
///
/// SAFETY: The internal `std::sync::Mutex<PageFrame>` is never held across `.await` points.
/// All methods (`page()`, `page_data()`, `modify_page()`, `mark_dirty()`) acquire the lock
/// synchronously and release it before any `.await`. This is safe because:
/// 1. PageGuard itself has no async methods
/// 2. The lock is held only for the duration of a synchronous closure
/// 3. Drop::drop() is synchronous and releases the lock synchronously
pub struct PageGuard {
    frame: Arc<Mutex<PageFrame>>,
}
```

- [ ] **Step 3: 运行全部测试确认无回归**

Run: `cargo test`
Expected: 所有测试通过

- [ ] **Step 4: Commit**

```bash
git add src/storage/page_frame.rs src/storage/buffer_pool.rs
git commit -m "docs(M13): add SAFETY comments for PageGuard Mutex usage"
```

---

## Phase 3: Benchmark 对比 + 文档更新

### Task 12: 修复后重新运行 benchmark 对比

**Files:** 无新文件

- [ ] **Step 1: 运行全部 benchmark（修复后）**

Run: `cargo bench -- --quick 2>&1 | tee /tmp/m13_optimized.txt`
Expected: 所有 benchmark 运行完成

- [ ] **Step 2: 对比基线和优化后数据**

Run: `diff /tmp/m13_baseline.txt /tmp/m13_optimized.txt | head -50`
Expected: 显示性能差异

- [ ] **Step 3: 运行 clippy 检查**

Run: `cargo clippy`
Expected: 0 warnings

- [ ] **Step 4: 运行 fmt 检查**

Run: `cargo fmt --check`
Expected: 无格式问题

---

### Task 13: 更新项目文档

**Files:**
- Modify: `.claude/docs/snapshot.md`
- Modify: `.claude/docs/tasks.md`
- Modify: `.claude/docs/learned.md`
- Modify: `.claude/docs/optimization.md`

- [ ] **Step 1: 更新 snapshot.md**

更新当前状态为 M13 完成，记录新增文件和测试数量。

- [ ] **Step 2: 更新 tasks.md**

标记 M13 所有任务完成，记录 benchmark 框架和 Critical 修复。

- [ ] **Step 3: 更新 learned.md**

记录 criterion 用法、page_data() 零拷贝模式、两阶段锁模式。

- [ ] **Step 4: 更新 optimization.md**

标记 Critical 问题为已修复，记录修复效果。

- [ ] **Step 5: Commit**

```bash
git add .claude/docs/
git commit -m "docs(M13): update project docs for benchmark + critical optimization completion"
```
