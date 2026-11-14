//! 一百条逐行插入的语句总耗时与执行阶段独占耗时。
//!
//! 建表发生在打开统计之前，两个整数只累加这 100 条插入摘要。
//! 测试不断言这两个整数的大小关系。

use std::time::Duration;

use rtsql::database::Database;
use rtsql::diagnostics::{
    Collect, DiagnosticsConfig, Module, ModuleSet, Severity, StageStat, Summary,
};
use rtsql::executor::Value;
use rtsql::result::SqlOutcome;

fn stage_stat<'a>(summary: &'a Summary, module: Module, name: &str) -> &'a StageStat {
    summary
        .stages
        .iter()
        .find(|stage| stage.module == module && stage.name == name)
        .unwrap_or_else(|| panic!("stage {name} missing from summary: {:?}", summary.stages))
}

#[tokio::test]
async fn row_insert_share_of_hundred_inserts() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("bench.db")).await.unwrap();
    db.execute_sql("CREATE TABLE bench (id INT PRIMARY KEY, name STRING, value INT)")
        .await
        .unwrap();

    db.set_diagnostics(DiagnosticsConfig {
        collect: Collect {
            events: false,
            stats: true,
            trace: false,
        },
        modules: ModuleSet::All,
        summary_limit: 100,
        ..DiagnosticsConfig::default()
    });

    for i in 0..100 {
        let sql = format!("INSERT INTO bench VALUES ({i}, 'user_{i}', {})", i * 10);
        db.execute_sql(&sql).await.unwrap();
    }

    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 100, "one summary per insert");

    let mut total_ns: u128 = 0;
    let mut execute_exclusive_ns: u128 = 0;
    for summary in &summaries {
        assert_eq!(summary.severity, Severity::Info);
        assert!(
            summary.sql.contains("INSERT INTO bench VALUES"),
            "summary sql is the insert: {}",
            summary.sql
        );
        assert!(
            summary.total > Duration::ZERO,
            "statement total is zero: {}",
            summary.sql
        );

        stage_stat(summary, Module::Pipeline, "parse");
        stage_stat(summary, Module::Pipeline, "plan");
        let execute = stage_stat(summary, Module::Pipeline, "execute");
        let creation = stage_stat(summary, Module::Pipeline, "executor_creation");
        let execution = stage_stat(summary, Module::Pipeline, "executor_execution");
        let commit = stage_stat(summary, Module::Wal, "commit");
        assert_eq!(
            execute.exclusive_total
                + creation.inclusive_total
                + execution.inclusive_total
                + commit.inclusive_total,
            execute.inclusive_total,
            "execute exclusive + creation + execution + commit == execute inclusive"
        );

        total_ns += summary.total.as_nanos();
        execute_exclusive_ns += execute.exclusive_total.as_nanos();
    }

    println!(
        "ms47_insert_share statements=100 total_ns={total_ns} execute_exclusive_ns={execute_exclusive_ns}"
    );

    match db.execute_sql("SELECT COUNT(*) FROM bench").await.unwrap() {
        SqlOutcome::Rows { rows, .. } => {
            assert_eq!(rows, vec![vec![Value::Int(100)]]);
        }
        other => panic!("count must return rows, got {other:?}"),
    }
}
