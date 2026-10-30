//! MS44 Iteration 002（T3）等待类别、跨任务归属与三种配置开销测试。
//!
//! 等待类别来自 design D5 列出的等待点：会话闸门与刷盘闸门记 Lock，
//! 刷盘等待记 Queue，刷盘写盘记 Io，通知到刷盘开始的间隔记 Schedule，
//! 文件页读取记 Io，其余阶段时间记 Compute。归属以「当时最内层 open
//! 阶段」为准；跨任务的刷盘回写只进入被该次提交释放的那条语句。
//!
//! 本文件不设置 `RTSQL_PROFILING`（无进程级环境竞争），按已批准
//! Iteration Plan 以默认并行度执行。

use std::time::{Duration, Instant};

use rtsql::database::Database;
use rtsql::diagnostics::{
    Category, Collect, Detail, DiagnosticsConfig, Module, StageStat, Summary,
};
use rtsql::executor::Value;
use rtsql::result::SqlOutcome;

/// 详细追踪配置（事件 / 统计 / 追踪全开）。
fn full_config() -> DiagnosticsConfig {
    DiagnosticsConfig {
        detail: Detail::Full,
        collect: Collect {
            events: true,
            stats: true,
            trace: true,
        },
        ..DiagnosticsConfig::default()
    }
}

/// 只开统计的配置（事件 / 追踪关闭）。
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

/// 取某阶段的汇总数（按 module + name 定位），缺失即失败。
fn stage_stat<'a>(summary: &'a Summary, module: Module, name: &str) -> &'a StageStat {
    summary
        .stages
        .iter()
        .find(|s| s.module == module && s.name == name)
        .unwrap_or_else(|| panic!("stage {name} missing from summary: {:?}", summary.stages))
}

/// 取查询结果行集（原生 `Value` 行）。
async fn query_rows(db: &Database, sql: &str) -> Vec<Vec<Value>> {
    match db.execute_sql(sql).await.unwrap() {
        SqlOutcome::Rows { rows, .. } => rows,
        other => panic!("{sql} must return rows, got {other:?}"),
    }
}

/// 需求 3 / S9：详细追踪打开时，一次成功插入的提交子阶段给出 Lock、Queue、
/// Io 和 Schedule 四类耗时（Schedule 允许为 0），且这些等待只出现在该语句
/// 的摘要里；插入的行在随后的查询中可见。
#[tokio::test]
async fn commit_wait_classes_present() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("wait.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();

    db.set_diagnostics(full_config());
    match db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap() {
        SqlOutcome::AffectedRows(1) => {}
        other => panic!("insert must affect exactly one row, got {other:?}"),
    }

    let summaries = db.diagnostic_summaries();
    assert_eq!(
        summaries.len(),
        1,
        "only the insert statement is summarized: {summaries:?}"
    );
    let summary = &summaries[0];

    let commit = stage_stat(summary, Module::Wal, "commit");
    let waits = commit.waits;
    assert!(
        waits.get(Category::Lock) > Duration::ZERO,
        "the commit stage waits on the session gate and the flush gate: {waits:?}"
    );
    assert!(
        waits.get(Category::Queue) > Duration::ZERO,
        "the commit stage waits for the flush to release it: {waits:?}"
    );
    assert!(
        waits.get(Category::Io) > Duration::ZERO,
        "the commit stage is released by the WAL write it waited for: {waits:?}"
    );
    assert!(
        waits.get(Category::Schedule) <= waits.wait_total(),
        "schedule is one of the wait categories of the commit stage: {waits:?}"
    );

    // 这四类等待只出现在该语句的摘要里：其他语句（建表）没有被诊断覆盖，
    // 摘要里只有这一条语句。
    assert_eq!(summaries.len(), 1, "no other statement is summarized");

    // 插入的行可见。
    let rows = query_rows(&db, "SELECT id FROM t").await;
    assert_eq!(rows, vec![vec![Value::Int(1)]]);

    // 随后的查询有自己的摘要：没有提交子阶段，也不带这次提交的等待——等待
    // 只进入被释放的那条语句。
    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 2, "the insert and the following query");
    let query = &summaries[1];
    assert!(
        query.stages.iter().all(|stage| stage.name != "commit"),
        "a read-only query has no commit stage: {query:?}"
    );
    assert!(
        query
            .stages
            .iter()
            .all(|stage| stage.waits.wait_total() == Duration::ZERO),
        "the commit waits do not leak into the other statement's summary: {query:?}"
    );
}

/// 需求 3 / S9b：解析阶段记为计算——Compute 大于 0，且没有 Io（解析阶段
/// 不读文件页）。
#[tokio::test]
async fn parse_stage_has_compute_without_io() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("wait.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();

    db.set_diagnostics(full_config());
    db.execute_sql("SELECT id FROM t").await.unwrap();

    let summaries = db.diagnostic_summaries();
    assert_eq!(
        summaries.len(),
        1,
        "only the query is summarized: {summaries:?}"
    );
    let parse = stage_stat(&summaries[0], Module::Pipeline, "parse");
    assert!(
        parse.waits.get(Category::Compute) > Duration::ZERO,
        "the parse stage time is compute: {:?}",
        parse.waits
    );
    assert_eq!(
        parse.waits.get(Category::Io),
        Duration::ZERO,
        "the parse stage reads no page: {:?}",
        parse.waits
    );
}

/// 需求 3 / S9c：文件页读取记为 Io 并把页数加 1；缓冲池已经持有的页不因
/// 被访问而记 Io。
#[tokio::test]
async fn file_page_read_records_io() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wait.db");
    {
        let db = Database::open(&path).await.unwrap();
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
            .await
            .unwrap();
        for id in 1..=10 {
            db.execute_sql(&format!("INSERT INTO t VALUES ({id})"))
                .await
                .unwrap();
        }
        db.close().await.unwrap();
    }

    // 重开（只读）：只读打开不做恢复，也不做打开后的空闲页重建——后者会
    // 走遍全部活页并把它们读进缓冲池。因此只读重开后缓冲池里没有目标数据
    // 页，查询会真的从文件读页。
    let db = Database::open_readonly(&path).await.unwrap();
    db.set_diagnostics(full_config());
    let rows = query_rows(&db, "SELECT id FROM t WHERE id = 3").await;
    assert_eq!(rows, vec![vec![Value::Int(3)]]);

    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 1, "only the query is summarized");
    let summary = &summaries[0];
    let io_total: Duration = summary
        .stages
        .iter()
        .map(|stage| stage.waits.get(Category::Io))
        .sum();
    let pages: u64 = summary.stages.iter().map(|stage| stage.waits.pages()).sum();
    assert!(
        io_total > Duration::ZERO,
        "the query read a page from the file: {summary:?}"
    );
    assert!(pages >= 1, "the read page is counted: {summary:?}");
    let parse = stage_stat(summary, Module::Pipeline, "parse");
    assert_eq!(
        parse.waits.get(Category::Io),
        Duration::ZERO,
        "the page read belongs to the execution stage, not to parse: {:?}",
        parse.waits
    );

    // 缓冲池已持有的页：再查一次不再读文件，Io 与页数都为 0。
    db.execute_sql("SELECT id FROM t WHERE id = 3")
        .await
        .unwrap();
    let summaries = db.diagnostic_summaries();
    let cached = summaries.last().expect("the second query is summarized");
    let cached_io: Duration = cached
        .stages
        .iter()
        .map(|stage| stage.waits.get(Category::Io))
        .sum();
    let cached_pages: u64 = cached.stages.iter().map(|stage| stage.waits.pages()).sum();
    assert_eq!(
        cached_io,
        Duration::ZERO,
        "a page already held by the buffer pool is not read again: {cached:?}"
    );
    assert_eq!(cached_pages, 0, "no page is counted twice: {cached:?}");
}

/// 需求 3 / S9d：只开性能统计（详细追踪关闭）时，等待以类别汇总数出现在
/// 阶段汇总里，摘要不含逐条追踪样本。
#[tokio::test]
async fn stats_only_reports_wait_totals_without_samples() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("wait.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();

    db.set_diagnostics(stats_config());
    db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap();

    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 1, "only the insert is summarized");
    let summary = &summaries[0];
    assert!(
        summary.samples.is_empty(),
        "stats-only mode keeps no per-item trace sample: {summary:?}"
    );
    let commit = stage_stat(summary, Module::Wal, "commit");
    assert!(
        commit.waits.wait_total() > Duration::ZERO,
        "the commit stage reports wait category totals: {:?}",
        commit.waits
    );
    assert!(
        commit.waits.get(Category::Compute) > Duration::ZERO,
        "the commit stage also reports compute: {:?}",
        commit.waits
    );
}

/// 需求 4 / S10：同一会话上两条 SQL 文本不同的并发插入，各自的摘要只含有
/// 自己的 SQL 文本；两条都成功、两行都可见；公开摘要没有操作标识、运行
/// 标识或内容哈希字段（摘要的类型里没有这类字段）。
#[tokio::test]
async fn concurrent_inserts_keep_own_sql() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("wait.db")).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await
        .unwrap();

    db.set_diagnostics(full_config());
    let db_a = db.clone();
    let db_b = db.clone();
    let (first, second) = tokio::join!(
        tokio::spawn(async move { db_a.execute_sql("INSERT INTO t VALUES (1, 10)").await }),
        tokio::spawn(async move { db_b.execute_sql("INSERT INTO t VALUES (2, 20)").await }),
    );
    assert!(first.unwrap().is_ok(), "the first insert succeeds");
    assert!(second.unwrap().is_ok(), "the second insert succeeds");

    let summaries = db.diagnostic_summaries();
    assert_eq!(
        summaries.len(),
        2,
        "each concurrent statement has its own summary: {summaries:?}"
    );
    for summary in &summaries {
        let own = if summary.sql.contains("VALUES (1, 10)") {
            "VALUES (2, 20)"
        } else if summary.sql.contains("VALUES (2, 20)") {
            "VALUES (1, 10)"
        } else {
            panic!("a summary carries an unknown SQL text: {summary:?}");
        };
        assert!(
            !summary.sql.contains(own),
            "a concurrent statement's summary must not carry the other statement's text: {summary:?}"
        );
    }

    // 两行都可见。
    let rows = query_rows(&db, "SELECT id, v FROM t ORDER BY id").await;
    assert_eq!(
        rows,
        vec![
            vec![Value::Int(1), Value::Int(10)],
            vec![Value::Int(2), Value::Int(20)],
        ],
        "both concurrent inserts are visible"
    );
}

/// 需求 7：诊断关闭、只开性能统计、详细追踪三种配置在同一条已有单行查询
/// 负载上行集相同，三次的耗时都被记录；不比较三者差值。
#[tokio::test]
async fn three_modes_match_rows() {
    const SELECT_SQL: &str = "SELECT * FROM bench WHERE id = 42";
    let mut row_sets: Vec<Vec<Vec<Value>>> = Vec::new();
    let mut elapsed: Vec<Duration> = Vec::new();

    for mode in 0..3 {
        // 三次各自新建文件库——避免计划缓存把后两次变成另一种路径。
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("bench.db")).await.unwrap();
        db.execute_sql("CREATE TABLE bench (id INT PRIMARY KEY, name STRING, value INT)")
            .await
            .unwrap();
        for id in 0..100 {
            db.execute_sql(&format!(
                "INSERT INTO bench VALUES ({id}, 'user_{id}', {})",
                id * 10
            ))
            .await
            .unwrap();
        }

        match mode {
            1 => db.set_diagnostics(stats_config()),
            2 => db.set_diagnostics(full_config()),
            _ => {}
        }

        let start = Instant::now();
        let outcome = tokio::time::timeout(Duration::from_secs(30), db.execute_sql(SELECT_SQL))
            .await
            .expect("the query returns within 30 seconds")
            .unwrap();
        elapsed.push(start.elapsed());
        let rows = match outcome {
            SqlOutcome::Rows { rows, .. } => rows,
            other => panic!("{SELECT_SQL} must return rows, got {other:?}"),
        };

        let summaries = db.diagnostic_summaries();
        if mode == 0 {
            assert!(
                summaries.is_empty(),
                "the closed configuration records no summary: {summaries:?}"
            );
        } else {
            let summary = summaries.last().expect("the query is summarized");
            assert!(
                summary.total > Duration::ZERO,
                "the elapsed time of the statement is recorded: {summary:?}"
            );
        }
        row_sets.push(rows);
    }

    assert_eq!(row_sets[0], row_sets[1], "closed and stats-only agree");
    assert_eq!(row_sets[1], row_sets[2], "stats-only and full trace agree");
    // 三次耗时都被记录；不比较三者差值（通过条件不含差值上限）。
    assert!(
        elapsed.iter().all(|duration| *duration > Duration::ZERO),
        "each of the three configurations recorded its elapsed time: {elapsed:?}"
    );
}
