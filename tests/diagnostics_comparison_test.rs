//! 比较指标从保留摘要和进程读出。
//!
//! 语句场景走 `Database::execute_sql`。P95 与吞吐用构造的摘要，不执行 SQL。

use std::sync::Arc;
use std::time::{Duration, Instant};

use rtsql::database::Database;
use rtsql::diagnostics::{
    process_memory_peak_bytes, render, segment_metrics, Collect, Detail, Diagnostics,
    DiagnosticsConfig, Module, ScopeEnd, Severity, StageSample, Summary,
};
use rtsql::executor::Value;
use rtsql::result::SqlOutcome;

fn stats_config() -> DiagnosticsConfig {
    DiagnosticsConfig {
        collect: Collect {
            events: false,
            stats: true,
            trace: false,
        },
        ..DiagnosticsConfig::default()
    }
}

fn summary_at(started_at: Instant, total: Duration, rows_delivered: u64) -> Summary {
    Summary {
        sql: String::new(),
        severity: Severity::Info,
        total,
        stages: Vec::new(),
        samples: Vec::new(),
        events: Vec::new(),
        dropped: 0,
        started_at,
        first_row: None,
        rows_delivered,
    }
}

async fn open_db(dir: &std::path::Path) -> Database {
    Database::open(&dir.join("comparison.db")).await.unwrap()
}

async fn rows_of(db: &Database, sql: &str) -> Vec<Vec<Value>> {
    match db.execute_sql(sql).await.unwrap() {
        SqlOutcome::Rows { rows, .. } => rows,
        other => panic!("{sql} must return rows, got {other:?}"),
    }
}

async fn affected_of(db: &Database, sql: &str) -> u64 {
    match db.execute_sql(sql).await.unwrap() {
        SqlOutcome::AffectedRows(count) => count,
        other => panic!("{sql} must report affected rows, got {other:?}"),
    }
}

/// 用现有格式串从 sql、severity、total 和 stages 拼出渲染文本。
fn render_from_existing_fields(summary: &Summary) -> String {
    let mut out = format!(
        "diagnostics sql={} severity={:?} total={:?}",
        summary.sql, summary.severity, summary.total
    );
    for stat in &summary.stages {
        out.push_str(&format!(
            "\n  stage {} module={:?} count={} inclusive={:?} exclusive={:?}",
            stat.name, stat.module, stat.count, stat.inclusive_total, stat.exclusive_total
        ));
    }
    out
}

/// 成功插入、空结果和无法解析的语句都没有首行等待，交回行数为 0。
#[tokio::test]
async fn insert_empty_and_error_omit_first_row() {
    let dir = tempfile::tempdir().unwrap();
    let db = open_db(dir.path()).await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();

    let off_affected = affected_of(&db, "INSERT INTO t VALUES (1)").await;
    db.set_diagnostics(stats_config());
    let on_affected = affected_of(&db, "INSERT INTO t VALUES (2)").await;
    assert_eq!(
        on_affected, off_affected,
        "insert affected-row count is unchanged when stats are on"
    );

    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 1, "only the stats insert is summarized");
    assert_eq!(summaries[0].first_row, None);
    assert_eq!(summaries[0].rows_delivered, 0);

    let empty = rows_of(&db, "SELECT id FROM t WHERE id = 99").await;
    assert!(empty.is_empty(), "missing key returns no rows: {empty:?}");
    let select = db.diagnostic_summaries().pop().unwrap();
    assert_eq!(select.first_row, None);
    assert_eq!(select.rows_delivered, 0);

    db.set_diagnostics(DiagnosticsConfig::default());
    let baseline = db.execute_sql("SELECT FROM WHERE").await.unwrap_err();
    db.set_diagnostics(stats_config());
    let err = db.execute_sql("SELECT FROM WHERE").await.unwrap_err();
    assert_eq!(err.message, baseline.message);
    assert_eq!(err.sqlstate, baseline.sqlstate);
    let failed = db.diagnostic_summaries().pop().unwrap();
    assert_eq!(failed.first_row, None);
    assert_eq!(failed.rows_delivered, 0);
}

/// 非空查询记下首行等待和交回行数。结果行与诊断关闭时相同。
#[tokio::test]
async fn nonempty_query_records_first_row() {
    let dir = tempfile::tempdir().unwrap();
    let db = open_db(dir.path()).await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap();
    db.execute_sql("INSERT INTO t VALUES (2)").await.unwrap();

    let baseline = rows_of(&db, "SELECT id FROM t").await;
    assert!(!baseline.is_empty(), "the table has at least one row");

    db.set_diagnostics(stats_config());
    let rows = rows_of(&db, "SELECT id FROM t").await;
    assert_eq!(rows, baseline);

    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 1);
    let summary = &summaries[0];
    let first_row = summary
        .first_row
        .expect("a nonempty query records a first-row wait");
    assert!(
        first_row <= summary.total,
        "first_row {:?} is not greater than total {:?}",
        first_row,
        summary.total
    );
    assert_eq!(summary.rows_delivered, rows.len() as u64);
}

/// 摘要上限只留最新一条。吞吐和 P95 只用这一条，不用被丢掉的行数。
#[tokio::test]
async fn segment_metrics_uses_only_retained_summaries() {
    let dir = tempfile::tempdir().unwrap();
    let db = open_db(dir.path()).await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();
    for id in 1..=3 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({id})"))
            .await
            .unwrap();
    }

    let one_row = "SELECT id FROM t WHERE id = 1";
    let three_rows = "SELECT id FROM t";
    let base_one = rows_of(&db, one_row).await;
    let base_three = rows_of(&db, three_rows).await;
    assert_ne!(base_one.len(), base_three.len());

    db.set_diagnostics(DiagnosticsConfig {
        summary_limit: 1,
        ..stats_config()
    });
    assert_eq!(rows_of(&db, one_row).await, base_one);
    let retained_rows = rows_of(&db, three_rows).await;
    assert_eq!(retained_rows, base_three);

    let summaries = db.diagnostic_summaries();
    assert_eq!(
        summaries.len(),
        1,
        "summary_limit keeps only the later query"
    );
    let summary = &summaries[0];
    assert_eq!(summary.rows_delivered, retained_rows.len() as u64);
    assert_ne!(
        summary.rows_delivered,
        (base_one.len() + base_three.len()) as u64
    );

    let metrics = segment_metrics(&summaries);
    assert_eq!(metrics.slow_p95, Some(summary.total));
    assert!(
        summary.total > Duration::ZERO,
        "a real query has a positive wall clock"
    );
    let expected = summary.rows_delivered as f64 / summary.total.as_secs_f64();
    assert_eq!(metrics.throughput_rows_per_sec, Some(expected));
}

/// P95 取最近秩上界，不插值。
#[test]
fn p95_uses_nearest_rank_upper_bound() {
    assert_eq!(segment_metrics(&[]).slow_p95, None);

    let one = summary_at(Instant::now(), Duration::from_millis(5), 0);
    assert_eq!(
        segment_metrics(std::slice::from_ref(&one)).slow_p95,
        Some(Duration::from_millis(5))
    );

    let started = Instant::now();
    let two = [
        summary_at(started, Duration::from_millis(9), 0),
        summary_at(started, Duration::from_millis(3), 0),
    ];
    assert_eq!(
        segment_metrics(&two).slow_p95,
        Some(Duration::from_millis(9))
    );

    let twenty: Vec<Summary> = (1..=20)
        .rev()
        .map(|millis| summary_at(started, Duration::from_millis(millis), 0))
        .collect();
    assert_eq!(
        segment_metrics(&twenty).slow_p95,
        Some(Duration::from_millis(19))
    );
}

/// 墙钟为 0 时不给吞吐。行数为 0 且墙钟大于 0 时吞吐为 0。
/// 重叠区间的墙钟是最早开始到最晚结束。
#[test]
fn throughput_handles_zero_wall_and_zero_rows() {
    let started = Instant::now();
    let zero_wall = [summary_at(started, Duration::ZERO, 4)];
    assert_eq!(segment_metrics(&zero_wall).throughput_rows_per_sec, None);

    let zero_rows = [summary_at(started, Duration::from_secs(3), 0)];
    assert_eq!(
        segment_metrics(&zero_rows).throughput_rows_per_sec,
        Some(0.0)
    );

    let overlap = [
        summary_at(started, Duration::from_secs(2), 3),
        summary_at(started + Duration::from_secs(1), Duration::from_secs(1), 1),
    ];
    assert_eq!(segment_metrics(&overlap).throughput_rows_per_sec, Some(2.0));
}

/// `record_pre_measured` 不记交付。开始时刻回推后，结束时刻落在调用窗口内。
#[test]
fn pre_measured_summary_has_no_delivery() {
    let session = Arc::new(Diagnostics::new());
    session.configure(stats_config());
    let before = Instant::now();
    session.record_pre_measured(
        "SELECT 1",
        ScopeEnd::Success,
        StageSample {
            module: Module::Pipeline,
            name: "statement".to_string(),
            inclusive: Duration::from_millis(100),
            children: vec![],
        },
        vec![],
    );
    let after = Instant::now();

    let summary = &session.summaries()[0];
    assert_eq!(summary.first_row, None);
    assert_eq!(summary.rows_delivered, 0);
    let ended = summary.started_at + summary.total;
    assert!(
        ended >= before && ended <= after,
        "started_at + total must fall inside the call window: {ended:?} not in {before:?}..={after:?}"
    );
}

/// Linux 峰值等于同一次 `ru_maxrss * 1024`。上限 0 的已知查询仍返回原行。
#[tokio::test]
async fn linux_memory_peak_matches_ru_maxrss() {
    #[cfg(target_os = "linux")]
    {
        let reported = process_memory_peak_bytes();
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        assert_eq!(rc, 0);
        assert!(usage.ru_maxrss >= 0);
        assert_eq!(reported, Some((usage.ru_maxrss as u64) * 1024));
    }
    #[cfg(not(target_os = "linux"))]
    {
        assert_eq!(process_memory_peak_bytes(), None);
    }

    let dir = tempfile::tempdir().unwrap();
    let db = open_db(dir.path()).await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap();
    let before_diagnostics = rows_of(&db, "SELECT id FROM t").await;

    db.set_query_memory_limit(0);
    db.set_diagnostics(stats_config());
    match db.execute_sql("SELECT id FROM t").await {
        Ok(SqlOutcome::Rows { rows, .. }) => {
            assert_eq!(rows, before_diagnostics);
        }
        Ok(other) => panic!("known query must return its original rows, got {other:?}"),
        Err(err) => panic!("known query must not exceed the query memory budget, got {err:?}"),
    }
}

/// 比较指标不进入 SQL 结果，也不进入现有渲染格式串。
#[tokio::test]
async fn comparison_metrics_absent_from_sql_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db = open_db(dir.path()).await;
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap();

    let baseline = rows_of(&db, "SELECT id FROM t").await;
    db.set_diagnostics(stats_config());
    let rows = rows_of(&db, "SELECT id FROM t").await;
    assert_eq!(rows, baseline);

    let summary = db.diagnostic_summaries().pop().unwrap();
    let text = render(&summary, Detail::Summary);
    assert_eq!(text, render_from_existing_fields(&summary));
}
