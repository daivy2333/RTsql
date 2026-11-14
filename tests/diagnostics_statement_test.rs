//! MS44 Iteration 001（T2）语句阶段集成测试。
//!
//! 经 `Database::execute_sql` 的真实语句在会话采集打开时产出带 parse /
//! plan / execute 阶段的诊断摘要；隐式 DML 的 `commit` 是 `execute` 的子
//! 阶段；失败与被放弃的语句各自结束记录；`RTSQL_PROFILING` 旧表行为不变；
//! SQL 结果、SQLSTATE 与取消后行可见性不变。
//!
//! 文件内用例经 CLI 子进程操纵进程级环境（`RTSQL_PROFILING`），按已批准
//! Iteration Plan 以 `--test-threads=1` 串行执行。

#![cfg(feature = "cli")]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use rtsql::database::Database;
use rtsql::diagnostics::{
    Collect, Detail, DiagnosticsConfig, Module, ModuleSet, Severity, StageStat, Summary,
};
use rtsql::executor::Value;
use rtsql::result::SqlOutcome;

/// 只开 stats 的会话配置（本文件的阶段断言全部消费阶段汇总数）。
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

/// 需求 2 / S3b：未命中缓存的 INSERT 摘要含 parse / plan / execute / commit
/// 阶段汇总；commit（Module::Wal）是 execute 的子阶段——单次插入下
/// commit.inclusive_total <= execute.inclusive_total，且 execute 的独占耗时
/// 加上 executor_creation、executor_execution、commit 的包含耗时等于
/// execute 的包含耗时。
#[tokio::test]
async fn insert_commit_is_child_of_execute() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("statement.db"))
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();

    db.set_diagnostics(stats_config());
    match db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap() {
        SqlOutcome::AffectedRows(1) => {}
        other => panic!("insert must affect exactly one row, got {other:?}"),
    }

    let summaries = db.diagnostic_summaries();
    assert_eq!(
        summaries.len(),
        1,
        "exactly the insert statement is summarized: {summaries:?}"
    );
    let summary = &summaries[0];
    assert_eq!(summary.severity, Severity::Info);
    assert!(
        summary.total > Duration::ZERO,
        "statement total is a real elapsed duration"
    );

    for (module, name) in [
        (Module::Pipeline, "parse"),
        (Module::Pipeline, "plan"),
        (Module::Pipeline, "execute"),
        (Module::Wal, "commit"),
    ] {
        let stat = stage_stat(summary, module, name);
        assert_eq!(stat.count, 1, "{name} appears exactly once");
    }

    let execute = stage_stat(summary, Module::Pipeline, "execute");
    let creation = stage_stat(summary, Module::Pipeline, "executor_creation");
    let execution = stage_stat(summary, Module::Pipeline, "executor_execution");
    let commit = stage_stat(summary, Module::Wal, "commit");
    assert!(
        commit.inclusive_total <= execute.inclusive_total,
        "commit is nested inside execute: commit={:?} execute={:?}",
        commit.inclusive_total,
        execute.inclusive_total
    );
    assert_eq!(
        execute.exclusive_total
            + creation.inclusive_total
            + execution.inclusive_total
            + commit.inclusive_total,
        execute.inclusive_total,
        "execute exclusive + creation + execution + commit == execute inclusive"
    );
}

/// 只开 stats、未设置 `RTSQL_PROFILING` 时，成功的 `INSERT INTO t VALUES (1)`
/// 把三个流水线子指标记入当前语句摘要。随后查询能看见这一行。
#[tokio::test]
async fn insert_records_pipeline_submetrics() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("statement.db"))
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();

    db.set_diagnostics(stats_config());
    match db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap() {
        SqlOutcome::AffectedRows(1) => {}
        other => panic!("insert must affect exactly one row, got {other:?}"),
    }

    let summaries = db.diagnostic_summaries();
    assert_eq!(
        summaries.len(),
        1,
        "exactly the insert statement is summarized: {summaries:?}"
    );
    let summary = &summaries[0];
    assert!(
        summary.total > Duration::ZERO,
        "statement total is the root inclusive duration"
    );

    for name in [
        "table_metadata_lookup",
        "executor_creation",
        "executor_execution",
    ] {
        let stat = stage_stat(summary, Module::Pipeline, name);
        assert_eq!(stat.count, 1, "{name} appears exactly once");
    }

    let plan = stage_stat(summary, Module::Pipeline, "plan");
    let lookup = stage_stat(summary, Module::Pipeline, "table_metadata_lookup");
    assert_eq!(
        plan.exclusive_total + lookup.inclusive_total,
        plan.inclusive_total,
        "plan exclusive + table_metadata_lookup inclusive == plan inclusive"
    );

    let execute = stage_stat(summary, Module::Pipeline, "execute");
    let creation = stage_stat(summary, Module::Pipeline, "executor_creation");
    let execution = stage_stat(summary, Module::Pipeline, "executor_execution");
    let commit = stage_stat(summary, Module::Wal, "commit");
    assert!(
        commit.inclusive_total <= execute.inclusive_total,
        "commit is nested inside execute: commit={:?} execute={:?}",
        commit.inclusive_total,
        execute.inclusive_total
    );
    assert_eq!(
        execute.exclusive_total
            + creation.inclusive_total
            + execution.inclusive_total
            + commit.inclusive_total,
        execute.inclusive_total,
        "execute exclusive + creation + execution + commit == execute inclusive"
    );

    assert_eq!(
        query_rows(&db, "SELECT id FROM t").await,
        vec![vec![Value::Int(1)]]
    );
}

/// 取查询结果行集（原生 `Value` 行）。
async fn query_rows(db: &Database, sql: &str) -> Vec<Vec<Value>> {
    match db.execute_sql(sql).await.unwrap() {
        SqlOutcome::Rows { rows, .. } => rows,
        other => panic!("{sql} must return rows, got {other:?}"),
    }
}

/// 需求 2 / S11：不写数据的查询（SELECT）摘要没有 `commit` 阶段。
#[tokio::test]
async fn query_has_no_commit_stage() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("statement.db"))
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap();

    db.set_diagnostics(stats_config());
    let outcome = db.execute_sql("SELECT id FROM t").await.unwrap();
    assert!(matches!(outcome, SqlOutcome::Rows { .. }));

    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 1, "only the SELECT is summarized");
    let summary = &summaries[0];
    for name in ["parse", "plan", "execute"] {
        stage_stat(summary, Module::Pipeline, name);
    }
    assert!(
        summary.stages.iter().all(|s| s.name != "commit"),
        "a read-only query has no commit stage: {:?}",
        summary.stages
    );
}

/// 需求 2 / S1：同一 SELECT 文本第二次执行（缓存命中）的摘要没有 parse /
/// plan，有 cache_hit_check 与 execute；对照首次（未命中）有 parse / plan、
/// cache_hit_check 不作为诊断阶段。
#[tokio::test]
async fn cache_hit_skips_parse_and_plan() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("statement.db"))
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1)").await.unwrap();

    db.set_diagnostics(stats_config());
    db.execute_sql("SELECT id FROM t").await.unwrap(); // 未命中
    db.execute_sql("SELECT id FROM t").await.unwrap(); // 缓存命中

    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 2);
    let miss = &summaries[0];
    stage_stat(miss, Module::Pipeline, "parse");
    stage_stat(miss, Module::Pipeline, "plan");
    assert!(
        miss.stages.iter().all(|s| s.name != "cache_hit_check"),
        "cache_hit_check is not a diagnostics stage on the miss path: {:?}",
        miss.stages
    );

    let hit = &summaries[1];
    assert!(
        hit.stages
            .iter()
            .all(|s| s.name != "parse" && s.name != "plan"),
        "cache hit skips parse and plan: {:?}",
        hit.stages
    );
    stage_stat(hit, Module::Pipeline, "cache_hit_check");
    stage_stat(hit, Module::Pipeline, "execute");
}

/// 需求 5 / S5：无法解析的语句摘要 severity 为 Error；其错误文本与
/// SQLSTATE 与关闭诊断执行同一语句逐字节相同。
#[tokio::test]
async fn parse_error_keeps_error_text_and_summary_fails() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("statement.db"))
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .unwrap();

    // 关闭诊断（默认配置）执行同一坏语句：取错误基线。
    let baseline = db.execute_sql("SELECT FROM WHERE").await.unwrap_err();

    db.set_diagnostics(stats_config());
    let err = db.execute_sql("SELECT FROM WHERE").await.unwrap_err();
    assert_eq!(
        err.message, baseline.message,
        "error text is byte-identical"
    );
    assert_eq!(err.sqlstate, baseline.sqlstate, "SQLSTATE is identical");

    let summaries = db.diagnostic_summaries();
    assert_eq!(
        summaries.len(),
        1,
        "only the failing statement is summarized"
    );
    assert_eq!(
        summaries[0].severity,
        Severity::Error,
        "a failed statement ends its record as Error"
    );
    // 权威规格场景「解析失败留下失败摘要且错误不变」的「含有解析阶段」子句：
    // 解析失败语句的摘要含 parse 阶段（`stage_stat` 在阶段缺失时 panic）。
    stage_stat(&summaries[0], Module::Pipeline, "parse");
}

/// 锚点：活跃事务集合中出现「已记录版本数 ≥ 2」的事务（先例
/// `tests/statement_cancellation_test.rs`；全部公开 API——执行器逐行推进，
/// ≥2 个版本保证第一个键的索引条目已在位）。
async fn anchor_tx(db: &Database) -> Option<u64> {
    for tx_id in db.transaction_manager.active_transactions().await {
        if db.transaction_manager.get_tx_versions(tx_id).await.len() >= 2 {
            return Some(tx_id);
        }
    }
    None
}

/// 需求 5 / S6：语句 future 在提交前被放弃——摘要 severity 为 Warn
///（Dropped）；下一条写语句触发回收后，被放弃语句的行不存在（同一键可
/// 再次插入成功，目标表行数只含已完成语句的行）。
#[tokio::test]
async fn abandoned_future_summary_is_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("statement.db"))
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY)")
        .await
        .unwrap();
    for id in 1..=2000i64 {
        db.execute_sql(&format!("INSERT INTO t VALUES ({id}, {id})"))
            .await
            .unwrap();
    }

    // 夹具在默认配置下建好（2000 行 setup 不进摘要），之后才打开采集。
    db.set_diagnostics(stats_config());

    // 放弃一条正在执行的隐式语句（一条语句、一个隐式事务、2000 行）：
    // 锚点命中即丢弃语句 future。
    let mut statement = Box::pin(db.execute_sql("INSERT INTO t2 SELECT id FROM t"));
    let mut anchor = Box::pin(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(tx_id) = anchor_tx(&db).await {
                    return tx_id;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect(
            "the abandon anchor must be reachable: the fixture statement did not reach it in time",
        )
    });
    let tx_id = tokio::select! {
        biased;
        tx_id = &mut anchor => tx_id,
        outcome = &mut statement => panic!(
            "statement finished before the abandon anchor was reached: {outcome:?}"
        ),
    };
    drop(anchor);
    // 放弃语句 future——根作用域随 future 析构按 Dropped 记录；隐式事务留在
    // 活跃集里，由下一条写语句回收。
    drop(statement);
    assert!(
        db.transaction_manager
            .active_transactions()
            .await
            .contains(&tx_id),
        "the abandoned statement's transaction must still be active right after the abandon"
    );

    // 行可见性 oracle：回收后被放弃语句的首个键可再次插入，目标表只含已完成
    // 语句的行。
    match db.execute_sql("INSERT INTO t2 VALUES (1)").await.unwrap() {
        SqlOutcome::AffectedRows(1) => {}
        other => panic!("re-insert of the abandoned key must succeed, got {other:?}"),
    }

    // 摘要：被放弃语句 Warn（Dropped），重插语句 Info。（先于下面的可见性
    // SELECT 断言——SELECT 自身也会形成一条摘要。）
    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 2, "abandoned + re-insert: {summaries:?}");
    assert_eq!(
        summaries[0].severity,
        Severity::Warn,
        "an abandoned statement ends its record as Warn (Dropped)"
    );
    assert_eq!(
        summaries[0].first_row, None,
        "an abandoned statement has no first-row wait"
    );
    assert_eq!(summaries[0].rows_delivered, 0);
    assert!(summaries[0].sql.contains("INSERT INTO t2 SELECT"));
    assert_eq!(summaries[1].severity, Severity::Info);

    let rows = query_rows(&db, "SELECT id FROM t2").await;
    assert_eq!(
        rows,
        vec![vec![Value::Int(1)]],
        "t2 must only contain rows of completed statements: {rows:?}"
    );
}

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 启动 rtsql 二进制（先例 `tests/recovery_bounded_read_test.rs` 的
/// `spawn_cli` 形态：`RTSQL_HOME` 指向临时目录隔离集中存储，显式移除键环境
/// 防宿主污染；`RTSQL_PROFILING` 只在子进程环境设置/移除）。
fn spawn_cli(dir: &Path, args: &[&str], profiling: Option<&str>) -> Child {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtsql"));
    cmd.args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .env_remove("RTSQL_BUSY_TIMEOUT")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match profiling {
        Some(value) => {
            cmd.env("RTSQL_PROFILING", value);
        }
        None => {
            cmd.env_remove("RTSQL_PROFILING");
        }
    }
    cmd.spawn().expect("spawn rtsql binary")
}

/// 等待子进程退出并收集输出；60s 未退出则 kill 并 panic（先例 `wait_cli`）。
fn wait_cli(mut child: Child) -> CliOutput {
    use std::io::Read;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (tx_out, rx_out) = std::sync::mpsc::channel();
    let (tx_err, rx_err) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx_out.send(buf);
    });
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        let _ = tx_err.send(buf);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    panic!("rtsql did not exit within 60s");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    CliOutput {
        code: status.code(),
        stdout: String::from_utf8_lossy(&rx_out.recv().unwrap()).to_string(),
        stderr: String::from_utf8_lossy(&rx_err.recv().unwrap()).to_string(),
    }
}

/// 需求 8 / S4：`RTSQL_PROFILING` 决定旧计时表——CLI 真实子进程入口执行
/// SQL（`rtsql import` 的自动建表 DDL 与逐行 INSERT 都经
/// `Database::execute_sql`）；变量设置且会话保持默认配置时 stderr 含
/// parse / plan / execute 三段计时表，未设置时同一入口无该表。
#[test]
fn legacy_profiling_table_follows_env_var() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("legacy.db");
    let db_str = db_path.to_str().unwrap().to_string();
    let csv_path = dir.path().join("data.csv");
    std::fs::write(&csv_path, "id\n1\n").unwrap();

    // 建库（`rtsql new` 是唯一创建入口）——全程无进程内 `Database` 句柄，
    // 不存在 advisory 锁竞争。
    let out = wait_cli(spawn_cli(dir.path(), &["new", &db_str], None));
    assert_eq!(out.code, Some(0), "rtsql new must succeed: {}", out.stderr);

    // 设置 RTSQL_PROFILING：import 的建表与插入语句都产生旧计时表。
    let import_args = &[
        "import",
        db_str.as_str(),
        "t",
        csv_path.to_str().unwrap(),
        "--csv",
    ];
    let out = wait_cli(spawn_cli(dir.path(), import_args, Some("1")));
    assert_eq!(
        out.code,
        Some(0),
        "import must succeed: stdout={} stderr={}",
        out.stdout,
        out.stderr
    );
    for stage in ["parse", "plan", "execute"] {
        assert!(
            out.stderr.contains(stage),
            "the profiling table must contain the {stage} stage: {}",
            out.stderr
        );
    }
    assert!(
        out.stderr.contains("Total"),
        "the profiling table has a Total row: {}",
        out.stderr
    );
    // 会话保持默认配置（采集全关）：同一子进程不产生新诊断输出。
    assert!(
        !out.stderr.contains("diagnostics "),
        "the default session writes no diagnostics output: {}",
        out.stderr
    );

    // 同一入口、变量未设置：stderr 无计时表。（新值 CSV——自动建表的键唯一
    // 性使同键二次导入报 Duplicate key，与本断言的判定无关。）
    let csv_path_2 = dir.path().join("data2.csv");
    std::fs::write(&csv_path_2, "id\n2\n").unwrap();
    let import_args_2 = &[
        "import",
        db_str.as_str(),
        "t",
        csv_path_2.to_str().unwrap(),
        "--csv",
    ];
    let out = wait_cli(spawn_cli(dir.path(), import_args_2, None));
    assert_eq!(
        out.code,
        Some(0),
        "import must succeed: stdout={} stderr={}",
        out.stdout,
        out.stderr
    );
    assert!(
        !out.stderr.contains("Time (µs)"),
        "no profiling table without RTSQL_PROFILING: {}",
        out.stderr
    );
}

/// 需求「既有计时出口和 stderr 告警保持」/ 场景「执行阶段之前的失败路径不
/// 输出旧的三段计时表」：`RTSQL_PROFILING` 已设置时，执行阶段之前失败的
/// 语句不输出旧的三段计时表。
///
/// 观察面是 CLI 真实子进程的 stderr（库入口的 `eprintln!` 被测试框架捕获，
/// 进程内读不回）。失败形状取 `rtsql import` 的自动建表 DDL 规划期拒绝
/// （重复列名）——失败发生在 execute 阶段之前。用例先用一次成功导入证明
/// stderr 观测面有效（该次导入打出旧表）。
#[test]
fn legacy_table_absent_before_execute_stage() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("legacy_absent.db");
    let db_str = db_path.to_str().unwrap().to_string();

    let out = wait_cli(spawn_cli(dir.path(), &["new", &db_str], None));
    assert_eq!(out.code, Some(0), "rtsql new must succeed: {}", out.stderr);

    // 正例：成功导入 → 旧表出现（本次 stderr 观测面有效）。
    let csv_ok = dir.path().join("ok.csv");
    std::fs::write(&csv_ok, "id\n1\n").unwrap();
    let out = wait_cli(spawn_cli(
        dir.path(),
        &[
            "import",
            db_str.as_str(),
            "t_ok",
            csv_ok.to_str().unwrap(),
            "--csv",
        ],
        Some("1"),
    ));
    assert_eq!(
        out.code,
        Some(0),
        "import must succeed: stdout={} stderr={}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("Time (µs)"),
        "the successful import prints the legacy table: {}",
        out.stderr
    );

    // 反例：重复列名 → 自动建表 DDL 规划期失败（execute 阶段之前）→ 无旧表。
    let csv_bad = dir.path().join("bad.csv");
    std::fs::write(&csv_bad, "id,id\n1,2\n").unwrap();
    let out = wait_cli(spawn_cli(
        dir.path(),
        &[
            "import",
            db_str.as_str(),
            "t_bad",
            csv_bad.to_str().unwrap(),
            "--csv",
        ],
        Some("1"),
    ));
    assert_eq!(
        out.code,
        Some(3),
        "the auto-create DDL failure exits 3: stdout={} stderr={}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("duplicate column name"),
        "the failure text is unchanged: {}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("Time (µs)"),
        "a failure before the execute stage prints no legacy table: {}",
        out.stderr
    );
}

/// 需求「既有计时出口和 stderr 告警保持」/ 场景「失败路径也输出旧的三段
/// 计时表」（执行阶段被触达的那一半）：进入执行阶段后失败的语句仍输出
/// 旧表。
///
/// 失败形状取预置同键行后 `rtsql import` 的逐行 INSERT 执行期拒绝
/// （Duplicate key）——失败发生在 execute 阶段内。该次导入只执行一条语句，
/// 所以子进程 stderr 里的这张表只能是失败的那条 INSERT 打出的。
#[test]
fn legacy_table_present_on_execute_stage_failure() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("legacy_present.db");
    let db_str = db_path.to_str().unwrap().to_string();

    let out = wait_cli(spawn_cli(dir.path(), &["new", &db_str], None));
    assert_eq!(out.code, Some(0), "rtsql new must succeed: {}", out.stderr);

    // 主命令不经语句管道（不在旧表的观察面内）：只用来预置同键行。
    let out = wait_cli(spawn_cli(
        dir.path(),
        &[db_str.as_str(), "CREATE TABLE t (id INT PRIMARY KEY)"],
        None,
    ));
    assert_eq!(
        out.code,
        Some(0),
        "CREATE TABLE must succeed: stdout={} stderr={}",
        out.stdout,
        out.stderr
    );
    let out = wait_cli(spawn_cli(
        dir.path(),
        &[db_str.as_str(), "INSERT INTO t VALUES (1)"],
        None,
    ));
    assert_eq!(
        out.code,
        Some(0),
        "the pre-seed INSERT must succeed: stdout={} stderr={}",
        out.stdout,
        out.stderr
    );

    let csv = dir.path().join("dup.csv");
    std::fs::write(&csv, "id\n1\n").unwrap();
    let out = wait_cli(spawn_cli(
        dir.path(),
        &[
            "import",
            db_str.as_str(),
            "t",
            csv.to_str().unwrap(),
            "--csv",
        ],
        Some("1"),
    ));
    assert_eq!(
        out.code,
        Some(3),
        "the duplicate key import exits 3: stdout={} stderr={}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("Duplicate key"),
        "the failure text is unchanged: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("Time (µs)"),
        "a failure inside the execute stage still prints the legacy table: {}",
        out.stderr
    );
}

/// 需求 6（T2 部分）/ S12：诊断不改变事务结果——关闭诊断插入一行、
/// `Detail::Full` + trace 打开插入另一行后重开库，两行都可见，行集与逐列
/// 值一致。
#[tokio::test]
async fn inserts_visible_with_and_without_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("statement.db");
    let db = Database::open(&path).await.unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, v INT)")
        .await
        .unwrap();

    // 关闭诊断（默认配置）插入一行。
    db.execute_sql("INSERT INTO t VALUES (1, 10)")
        .await
        .unwrap();

    // Detail::Full + 全采集插入另一行。
    db.set_diagnostics(DiagnosticsConfig {
        detail: Detail::Full,
        collect: Collect {
            events: true,
            stats: true,
            trace: true,
        },
        ..DiagnosticsConfig::default()
    });
    db.execute_sql("INSERT INTO t VALUES (2, 20)")
        .await
        .unwrap();

    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 1, "only the traced insert is summarized");
    assert_eq!(summaries[0].severity, Severity::Info);

    let expected = vec![
        vec![Value::Int(1), Value::Int(10)],
        vec![Value::Int(2), Value::Int(20)],
    ];
    let rows = query_rows(&db, "SELECT id, v FROM t ORDER BY id").await;
    assert_eq!(rows, expected, "both rows visible with column-exact values");

    // 重开后两行仍可见、逐列一致。
    db.close().await.unwrap();
    drop(db);
    let db = Database::open(&path).await.unwrap();
    let rows = query_rows(&db, "SELECT id, v FROM t ORDER BY id").await;
    assert_eq!(rows, expected, "both rows survive the reopen, column-exact");
}

fn stats_for(modules: ModuleSet) -> DiagnosticsConfig {
    DiagnosticsConfig {
        collect: Collect {
            events: false,
            stats: true,
            trace: false,
        },
        modules,
        ..DiagnosticsConfig::default()
    }
}

async fn indexed_table(db: &Database) {
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, b INT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1, 10)")
        .await
        .unwrap();
}

/// 主键等值查找把 `index_manager_search` 记入摘要一次，不记
/// `index_manager_search_all`。返回行与诊断关闭时相同。
#[tokio::test]
async fn primary_key_lookup_records_index_search() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("statement.db"))
        .await
        .unwrap();
    indexed_table(&db).await;

    let baseline = query_rows(&db, "SELECT * FROM t WHERE id = 1").await;
    db.set_diagnostics(stats_config());
    let rows = query_rows(&db, "SELECT * FROM t WHERE id = 1").await;
    assert_eq!(rows, baseline);

    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 1, "only the diagnosed lookup is summarized");
    let summary = &summaries[0];
    let search = stage_stat(summary, Module::Index, "index_manager_search");
    assert_eq!(search.count, 1);
    assert!(
        summary
            .stages
            .iter()
            .all(|stage| stage.name != "index_manager_search_all"),
        "primary-key equality has no index_manager_search_all: {:?}",
        summary.stages
    );
}

/// 二级索引等值查找把 `index_manager_search_all` 记入摘要。返回行与诊断关闭时相同。
#[tokio::test]
async fn secondary_index_lookup_records_search_all() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("statement.db"))
        .await
        .unwrap();
    indexed_table(&db).await;
    db.execute_sql("CREATE INDEX idx_b ON t(b)").await.unwrap();

    let baseline = query_rows(&db, "SELECT * FROM t WHERE b = 10").await;
    db.set_diagnostics(stats_config());
    let rows = query_rows(&db, "SELECT * FROM t WHERE b = 10").await;
    assert_eq!(rows, baseline);

    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 1, "only the diagnosed lookup is summarized");
    let search = stage_stat(&summaries[0], Module::Index, "index_manager_search_all");
    assert_eq!(search.count, 1);
}

/// 观察类别只有 Pipeline 时，主键等值摘要没有两个索引阶段名，仍有 execute，
/// 结果行与诊断关闭时相同。
#[tokio::test]
async fn index_stages_hidden_when_module_excludes_index() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("statement.db"))
        .await
        .unwrap();
    indexed_table(&db).await;

    let baseline = query_rows(&db, "SELECT * FROM t WHERE id = 1").await;
    db.set_diagnostics(stats_for(ModuleSet::Only(vec![Module::Pipeline])));
    let rows = query_rows(&db, "SELECT * FROM t WHERE id = 1").await;
    assert_eq!(rows, baseline);

    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 1, "only the diagnosed lookup is summarized");
    let summary = &summaries[0];
    stage_stat(summary, Module::Pipeline, "execute");
    assert!(
        summary.stages.iter().all(|stage| {
            stage.name != "index_manager_search" && stage.name != "index_manager_search_all"
        }),
        "Pipeline-only summary omits index stages: {:?}",
        summary.stages
    );
}

/// 观察类别只有 Index 时，成功插入的摘要没有三个流水线子指标，插入的行可见。
#[tokio::test]
async fn pipeline_submetrics_hidden_when_module_excludes_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("statement.db"))
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, b INT)")
        .await
        .unwrap();

    db.set_diagnostics(stats_for(ModuleSet::Only(vec![Module::Index])));
    match db.execute_sql("INSERT INTO t VALUES (1, 10)").await.unwrap() {
        SqlOutcome::AffectedRows(1) => {}
        other => panic!("insert must affect exactly one row, got {other:?}"),
    }

    let summaries = db.diagnostic_summaries();
    assert_eq!(summaries.len(), 1, "only the insert is summarized");
    assert!(
        summaries[0].stages.iter().all(|stage| {
            stage.name != "table_metadata_lookup"
                && stage.name != "executor_creation"
                && stage.name != "executor_execution"
        }),
        "Index-only summary omits pipeline submetrics: {:?}",
        summaries[0].stages
    );

    assert_eq!(
        query_rows(&db, "SELECT * FROM t").await,
        vec![vec![Value::Int(1), Value::Int(10)]]
    );
}
