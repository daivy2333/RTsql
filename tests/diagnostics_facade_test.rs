//! MS44 Iteration 000（T1）诊断门面集成测试。
//!
//! 门面口径：三项配置（Detail / ModuleSet / Collect）分离，事件与摘要两个
//! 上限，父子耗时按「总耗时 = 根包含耗时、独占 = 包含 − 直接子阶段包含」
//! 计算。本轮只测门面本身，不接语句管道。

use std::sync::Arc;
use std::time::Duration;

use rtsql::database::Database;
use rtsql::diagnostics::{
    render, Category, Collect, Detail, Diagnostics, DiagnosticsConfig, Module, ModuleSet, Record,
    ScopeEnd, Severity, StageSample,
};

/// 只开统计的配置（events / trace 均关）。
fn stats_only_config() -> DiagnosticsConfig {
    DiagnosticsConfig {
        collect: Collect {
            events: false,
            stats: true,
            trace: false,
        },
        ..DiagnosticsConfig::default()
    }
}

/// 一棵根包含 100ms、子阶段 parse 10ms / plan 5ms / execute 70ms（其下
/// commit 40ms）的固定耗时树（Requirement「语句阶段与父子耗时」的固定输入）。
fn fixed_tree() -> StageSample {
    StageSample {
        module: Module::Pipeline,
        name: "statement".to_string(),
        inclusive: Duration::from_millis(100),
        children: vec![
            StageSample {
                module: Module::Pipeline,
                name: "parse".to_string(),
                inclusive: Duration::from_millis(10),
                children: vec![],
            },
            StageSample {
                module: Module::Pipeline,
                name: "plan".to_string(),
                inclusive: Duration::from_millis(5),
                children: vec![],
            },
            StageSample {
                module: Module::Pipeline,
                name: "execute".to_string(),
                inclusive: Duration::from_millis(70),
                children: vec![StageSample {
                    module: Module::Wal,
                    name: "commit".to_string(),
                    inclusive: Duration::from_millis(40),
                    children: vec![],
                }],
            },
        ],
    }
}

/// 只开 stats 时摘要有阶段汇总数，且没有逐条追踪样本。
#[test]
fn stats_without_trace_records_totals() {
    let session = Arc::new(Diagnostics::new());
    session.configure(stats_only_config());

    session.record_pre_measured("SELECT 1", ScopeEnd::Success, fixed_tree(), vec![]);

    let summaries = session.summaries();
    assert_eq!(summaries.len(), 1, "one statement recorded");
    let summary = &summaries[0];
    assert_eq!(
        summary.total,
        Duration::from_millis(100),
        "total is the root inclusive duration"
    );

    let parse = summary
        .stages
        .iter()
        .find(|s| s.name == "parse")
        .expect("parse stage total present");
    assert_eq!(parse.count, 1);
    assert_eq!(parse.inclusive_total, Duration::from_millis(10));
    assert_eq!(parse.exclusive_total, Duration::from_millis(10));

    assert!(
        summary.samples.is_empty(),
        "no per-item trace samples when trace collection is off: {:?}",
        summary.samples
    );
    assert!(
        summary.events.is_empty(),
        "no events collected when events collection is off"
    );
}

/// Detail::Off 仍可从会话读回摘要，渲染为空字符串。
#[test]
fn detail_off_keeps_summary_and_renders_empty() {
    let session = Arc::new(Diagnostics::new());
    session.configure(DiagnosticsConfig {
        detail: Detail::Off,
        ..stats_only_config()
    });
    session.record_pre_measured("SELECT 1", ScopeEnd::Success, fixed_tree(), vec![]);

    let summaries = session.summaries();
    assert_eq!(
        summaries.len(),
        1,
        "summary is still recorded under Detail::Off"
    );
    assert!(!summaries[0].stages.is_empty());
    assert_eq!(
        render(&summaries[0], Detail::Off),
        "",
        "Off renders an empty string"
    );
    // 对照：同一条摘要以 Summary 档渲染非空且以 diagnostics 开头。
    let text = render(&summaries[0], Detail::Summary);
    assert!(text.starts_with("diagnostics "), "summary render: {text}");
}

/// 观察类别不含 pipeline 时不记录 parse / plan / execute；wal 类别不受影响。
#[test]
fn module_filter_skips_pipeline_stages() {
    let session = Arc::new(Diagnostics::new());
    session.configure(DiagnosticsConfig {
        modules: ModuleSet::Only(vec![Module::Wal]),
        ..stats_only_config()
    });
    session.record_pre_measured(
        "INSERT INTO t VALUES (1)",
        ScopeEnd::Success,
        fixed_tree(),
        vec![],
    );

    let summary = &session.summaries()[0];
    for stage in ["parse", "plan", "execute"] {
        assert!(
            summary.stages.iter().all(|s| s.name != stage),
            "{stage} must not be recorded when pipeline is not observed: {:?}",
            summary.stages
        );
    }
    assert!(
        summary
            .stages
            .iter()
            .any(|s| s.name == "commit" && s.module == Module::Wal),
        "wal stages are independent of the pipeline filter: {:?}",
        summary.stages
    );
}

/// 根 100、执行 70、提交 40：总耗时 100，执行独占 30。
#[test]
fn root_total_is_not_parent_plus_child() {
    let session = Arc::new(Diagnostics::new());
    session.configure(stats_only_config());
    session.record_pre_measured(
        "INSERT INTO t VALUES (1)",
        ScopeEnd::Success,
        fixed_tree(),
        vec![],
    );

    let summary = &session.summaries()[0];
    assert_eq!(
        summary.total,
        Duration::from_millis(100),
        "total is the root inclusive duration, not parent + child"
    );
    let execute = summary
        .stages
        .iter()
        .find(|s| s.name == "execute")
        .expect("execute stat");
    assert_eq!(
        execute.exclusive_total,
        Duration::from_millis(30),
        "execute exclusive = 70 - 40"
    );
    let commit = summary
        .stages
        .iter()
        .find(|s| s.name == "commit")
        .expect("commit stat");
    assert_eq!(commit.exclusive_total, Duration::from_millis(40));
}

/// 事件超过默认上限 64：dropped 计数，只留最新，汇总数仍在。
#[test]
fn event_limit_increments_dropped() {
    let session = Arc::new(Diagnostics::new());
    session.configure(DiagnosticsConfig {
        collect: Collect {
            events: true,
            stats: true,
            trace: false,
        },
        ..DiagnosticsConfig::default()
    });
    let events: Vec<Record> = (0..65)
        .map(|i| Record {
            severity: Severity::Info,
            category: Category::Compute,
            message: format!("e{i}"),
            data: None,
        })
        .collect();
    session.record_pre_measured("SELECT 1", ScopeEnd::Success, fixed_tree(), events);

    let summary = &session.summaries()[0];
    assert_eq!(summary.dropped, 1, "one event beyond the default limit of 64");
    assert_eq!(summary.events.len(), 64);
    assert_eq!(
        summary.events.first().map(|e| e.message.as_str()),
        Some("e1"),
        "oldest dropped, newest kept"
    );
    assert_eq!(
        summary.events.last().map(|e| e.message.as_str()),
        Some("e64")
    );
    assert!(
        !summary.stages.is_empty(),
        "stage totals survive the event limit"
    );
}

/// 摘要超过默认上限 32 时只留最新的。
#[test]
fn summary_limit_drops_oldest() {
    let session = Arc::new(Diagnostics::new());
    session.configure(stats_only_config());
    for i in 0..33 {
        let sql = format!("SELECT {i}");
        session.record_pre_measured(&sql, ScopeEnd::Success, fixed_tree(), vec![]);
    }
    let summaries = session.summaries();
    assert_eq!(summaries.len(), 32, "default summary limit is 32");
    assert_eq!(summaries[0].sql, "SELECT 1", "oldest (SELECT 0) dropped");
    assert_eq!(summaries[31].sql, "SELECT 32", "newest kept");
}

/// SQL 文本在摘要中截断到 80 个字符，第 81 个字符被截掉。
#[test]
fn sql_text_truncates_at_80() {
    let session = Arc::new(Diagnostics::new());
    session.configure(stats_only_config());
    let digits = "0123456789".repeat(10); // 100 chars, 81st char is '0'
    session.record_pre_measured(&digits, ScopeEnd::Success, fixed_tree(), vec![]);

    let summary = &session.summaries()[0];
    assert_eq!(summary.sql.len(), 80);
    assert_eq!(summary.sql, digits.chars().take(80).collect::<String>());
    assert!(
        summary.sql.ends_with('9'),
        "the 81st char ('0') is cut; the kept text ends with the 80th char"
    );
}

/// 公开记录用结构体字面量只填严重级别、类别、消息、数据即可构造——
/// 编译通过即表示没有操作标识、运行标识或内容哈希字段。
#[test]
fn public_record_has_no_identity_field() {
    let record = Record {
        severity: Severity::Info,
        category: Category::Compute,
        message: "stage ended".to_string(),
        data: Some("parse".to_string()),
    };
    let session = Arc::new(Diagnostics::new());
    session.configure(DiagnosticsConfig {
        collect: Collect {
            events: true,
            stats: true,
            trace: false,
        },
        ..DiagnosticsConfig::default()
    });
    session.record_pre_measured("SELECT 1", ScopeEnd::Success, fixed_tree(), vec![record]);

    let summary = &session.summaries()[0];
    assert_eq!(summary.events.len(), 1);
    assert_eq!(summary.events[0].message, "stage ended");
    assert_eq!(summary.events[0].data.as_deref(), Some("parse"));
}

/// 三种结束状态的严重级别：成功 Info、失败 Error、未结束被丢弃 Warn。
#[test]
fn ended_scope_uses_info_error_warn() {
    let session = Arc::new(Diagnostics::new());
    session.configure(stats_only_config());

    session.statement("SELECT 1").end_ok();
    session.statement("SELECT bad").end_err();
    {
        let _abandoned = session.statement("SELECT abandoned");
    } // 析构时未结束 → Dropped

    let summaries = session.summaries();
    assert_eq!(summaries.len(), 3);
    assert_eq!(summaries[0].severity, Severity::Info);
    assert_eq!(summaries[1].severity, Severity::Error);
    assert_eq!(
        summaries[2].severity,
        Severity::Warn,
        "dropped without end is Warn"
    );
}

/// 默认配置（采集全关）下 SELECT 1 成功，摘要为空。
#[tokio::test]
async fn default_session_execute_has_empty_summaries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let db = Database::open(&path).await.unwrap();
    let outcome = db.execute_sql("SELECT 1").await;
    assert!(outcome.is_ok(), "SELECT 1 succeeds on the default session");
    assert!(
        db.diagnostic_summaries().is_empty(),
        "default session records no summaries"
    );
}
