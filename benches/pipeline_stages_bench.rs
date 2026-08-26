//! MS06-T04: Pipeline 三阶段独立 micro-bench
//!
//! 测量 MS06-T04 引入的三个 pub 阶段函数（parse_stage / plan_stage / execute_stage）
//! 各自的开销，作为后续 perf 调优与回归监测的基线入口。
//!
//! 模式：criterion + tokio Runtime + benches/common（与 micro_bench 保持一致）。
//! 每组 benchmark 一次性 setup，建表与数据预填充在 `iter` 之外完成。

mod common;

use common::*;
use criterion::{criterion_group, criterion_main, Criterion};
use rtsql::executor::PhysicalPlan;
use rtsql::pipeline::{execute_stage, parse_stage, plan_stage};
use tokio::runtime::Runtime;

const SELECT_SQL: &str = "SELECT * FROM bench WHERE id = 42";

fn bench_parse_stage(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("pipeline_stage_parse");
    group.bench_function("parse_stage", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = parse_stage(SELECT_SQL).await;
        });
    });
    group.finish();
}

fn bench_plan_stage(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    let mut group = c.benchmark_group("pipeline_stage_plan");
    group.bench_function("plan_stage_uncached", |b| {
        b.to_async(&rt).iter(|| async {
            // Use a fresh SQL each iter to bypass cache; this measures the
            // miss-path cost (build_plan + register_table + cache put).
            let sql = format!("SELECT * FROM bench WHERE id = {}", 42);
            let stmts = parse_stage(&sql).await.unwrap();
            let stmt = stmts.first().unwrap();
            let _ = plan_stage(&db, &sql, stmt, false).await;
        });
    });
    group.finish();
    cleanup_db(&path);
}

fn bench_execute_stage(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (path, db) = rt.block_on(async {
        let (p, d) = setup_db().await;
        create_test_table(&d).await;
        insert_rows(&d, 0, 100).await;
        (p, d)
    });

    // Pre-build a plan once, outside the iter closure, so the benchmark
    // measures only the executor-creation + execution phase.
    let plan: PhysicalPlan = rt.block_on(async {
        let stmts = parse_stage(SELECT_SQL).await.unwrap();
        let stmt = stmts.first().unwrap();
        plan_stage(&db, SELECT_SQL, stmt, false).await.unwrap()
    });

    let mut group = c.benchmark_group("pipeline_stage_execute");
    group.bench_function("execute_stage_prebuilt_plan", |b| {
        b.to_async(&rt).iter(|| async {
            // Clone the plan so execute_stage can consume it; PhysicalPlan
            // derives Clone, and cloning is cheap for scan-class plans.
            let _ = execute_stage(&db, plan.clone(), false).await;
        });
    });
    group.finish();
    cleanup_db(&path);
}

criterion_group!(
    benches,
    bench_parse_stage,
    bench_plan_stage,
    bench_execute_stage,
);
criterion_main!(benches);
