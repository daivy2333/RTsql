//! MS25 Iteration 004 / Cycle 001：JOIN 输出形状与名字解析收口（Task T8，design D9）
//!
//! change: 2026-09-26-ms25-sql-read-surface
//!
//! 病灶（Plan 实测）：JOIN 基座上 `output_columns` 只按顶层投影项的列引用过滤，
//! 于是
//! - 聚合参数列不在顶层 SELECT 里 → 该列不进组合行 → `extract_value` 静默
//!   返回 NULL → `count(v) = 0`（非零静默错值）；
//! - 名字解析用按列名折叠的 `column_indices` HashMap（裸名末位胜出）→ 同名列
//!   静默取错来源；
//! - 裸列名逐项「过滤式」解析 → 未知列得到零宽行（`columns: []`，exit 0），
//!   纯列引用别名项也落零宽行。
//!
//! 目标语义（delta spec `join-output-resolution`）：
//! - R1 聚合×JOIN 全列承载：聚合查询的组合行承载全部候选列，参数列、限定名
//!   分组键在全形状内精确解析；既有断言零校准。
//! - R2 解析错误计划期点名：JOIN 基座纯列清单未知列 `ColumnNotFound`、裸名多命中
//!   `AmbiguousColumn`（含列名）。
//! - R3 别名列可达：纯列引用别名项经既有 Projection 别名面可达（表头 = 别名）；
//!   表达式项 × JOIN 维持既有拒绝；既有面零回归。
//! - R4 ORDER BY 别名键与 Sort 输入行同基座（无 WHERE 按过滤后布局、WHERE 按全
//!   组合布局）→ JOIN 上别名排序真实生效。

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use serde_json::json;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

async fn exec_ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "setup statement failed: {sql:?} -> {resp:?}"
    );
}

/// 两表 JOIN 基座：两表都有列 `k`（同名面）、`t1.v` / `t2.w` 各自独有列。
async fn join_base(dir: &TempDir) -> Database {
    let db = Database::open(&dir.path().join("test")).await.unwrap();
    exec_ok(&db, "CREATE TABLE t1 (k INT, v STRING)").await;
    exec_ok(&db, "CREATE TABLE t2 (k INT, w STRING)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (1, 'b')").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (2, 'a')").await;
    exec_ok(&db, "INSERT INTO t2 VALUES (1, 'x')").await;
    exec_ok(&db, "INSERT INTO t2 VALUES (2, 'y')").await;
    db
}

fn query_rows(resp: Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("Expected QueryResult, got {other:?}"),
    }
}

fn error_msg(resp: Response) -> String {
    match resp {
        Response::Error { message, .. } => message,
        other => panic!("Expected Error, got {other:?}"),
    }
}

// ==== R1：聚合×JOIN 全列承载 ====

/// R1-S1：聚合参数列不在顶层 SELECT 里仍可达，且按被引用列取值。
///
/// 断言用 `max(v)` 而非 `count(v)`：`v` 是 t1 段的第 2 列，取值错误落到段首
/// （`k`）时结果为数字 2 而非字符串 "b"——见证对索引错误具备判别力。
///
/// RED（实施前实测）：`SELECT max(v) FROM t1 JOIN t2 ON t1.k = t2.k` 返回
/// `[[null]]`——`v` 不进组合行 → `extract_value` 静默 NULL。
#[tokio::test]
async fn agg_arg_outside_select_reachable() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT max(v) FROM t1 JOIN t2 ON t1.k = t2.k")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!("b")]],
        "aggregate param outside the top-level SELECT must stay reachable"
    );
}

/// R1-S2：限定名分组键在全组合形状内精确解析（既有断言零校准见证）。
///
/// GREEN 前后均通过——承载面从「过滤后形状」变为「全形状」后仍按 (t1.k) 精确解析。
/// `ORDER BY` 只接受列名键（既有面），故此处按行序无关比较。
#[tokio::test]
async fn qualified_group_key_precise() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;

    let mut rows = query_rows(to_response(
        db.execute_sql("SELECT t1.k, count(*) FROM t1 JOIN t2 ON t1.k = t2.k GROUP BY t1.k")
            .await,
    ));
    // 行序无关比较（Hash Join 的分组序不承诺稳定）
    rows.sort_by_key(|r| r[0].to_string());
    assert_eq!(
        rows,
        vec![vec![json!(1), json!(1)], vec![json!(2), json!(1)]],
        "qualified GROUP BY key must resolve against the full combination layout"
    );
}

/// R1-S3：WHERE 组合 + 聚合参数列不在 SELECT 里（carry_all_columns 既有路径）。
///
/// `max(v)` 判别：过滤后仅 (2, 'a') 存活 → "a"；错取段首 `k` 得 2。
#[tokio::test]
async fn agg_arg_reachable_with_where() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT max(v) FROM t1 JOIN t2 ON t1.k = t2.k WHERE t1.k > 1")
            .await,
    ));
    assert_eq!(rows, vec![vec![json!("a")]]);
}

// ==== R2：解析错误计划期点名 ====

/// R2-S1：JOIN 基座裸名分组键多命中 → `AmbiguousColumn`（含列名）。
///
/// RED（实施前实测）：静默按 `column_indices` 末位胜出按 t2.k 分组返回行。
#[tokio::test]
async fn bare_group_by_ambiguous_rejected() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT k, count(*) FROM t1 JOIN t2 ON t1.k = t2.k GROUP BY k")
            .await,
    ));
    assert!(
        msg.contains("Ambiguous column") && msg.contains('k'),
        "bare GROUP BY key over two matching columns must name the ambiguity: {msg}"
    );
}

/// R2-S2：JOIN 基座裸名聚合参数多命中 → `AmbiguousColumn`。
///
/// RED（实施前实测）：静默取末位列来源统计（恰逢同值时结果看似正确）。
#[tokio::test]
async fn bare_agg_arg_ambiguous_rejected() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT count(k) FROM t1 JOIN t2 ON t1.k = t2.k")
            .await,
    ));
    assert!(
        msg.contains("Ambiguous column") && msg.contains('k'),
        "bare aggregate arg over two matching columns must name the ambiguity: {msg}"
    );
}

/// R2-S3：JOIN 基座裸列投影项多命中 → `AmbiguousColumn`（零宽行面消除）。
///
/// RED（实施前实测）：返回重复列（两列同名 k），不报错。
#[tokio::test]
async fn bare_projection_ambiguous_rejected() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT k FROM t1 JOIN t2 ON t1.k = t2.k")
            .await,
    ));
    assert!(
        msg.contains("Ambiguous column") && msg.contains('k'),
        "bare projection item over two matching columns must name the ambiguity: {msg}"
    );
}

/// R2-S4：JOIN 基座未知裸列 → `ColumnNotFound`（零宽行面消除）。
///
/// RED（实施前实测）：`SELECT nosuch FROM t1 JOIN t2 …` 返回零宽行
/// `{"columns":[],"rows":[[],[]]}`，exit 0。
#[tokio::test]
async fn unknown_column_named_rejected() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT nosuch FROM t1 JOIN t2 ON t1.k = t2.k")
            .await,
    ));
    assert!(
        msg.contains("Column not found") && msg.contains("nosuch"),
        "unknown column over JOIN base must be named: {msg}"
    );
}

/// R2-S5：未知限定列 → `ColumnNotFound`（表在、列不在）。
#[tokio::test]
async fn unknown_qualified_column_named_rejected() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT t1.nosuch FROM t1 JOIN t2 ON t1.k = t2.k")
            .await,
    ));
    assert!(
        msg.contains("Column not found") && msg.contains("nosuch"),
        "unknown qualified column over JOIN base must be named: {msg}"
    );
}

// ==== R3：别名列可达 + 既有拒绝面保持 ====

/// R3-S1：纯列引用别名项可达，表头 = 别名、行值 = 所指列。
///
/// RED（实施前实测）：`SELECT t1.v AS val FROM t1 JOIN t2 …` 零宽行、无表头列。
#[test]
fn aliased_plain_column_reachable() {
    let dir = fixture();
    let out = run_cli(
        dir.path(),
        &[
            "app",
            "--format",
            "table",
            "SELECT t1.v AS val FROM t1 JOIN t2 ON t1.k = t2.k ORDER BY val",
        ],
    );
    assert_eq!(out.code, Some(0), "query failed: {}", out.stderr);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert!(
        lines[0].contains("val"),
        "header must be the projection alias: {:?}",
        lines[0]
    );
    let rows: Vec<String> = lines.iter().skip(2).map(|l| l.trim().to_string()).collect();
    assert_eq!(
        rows,
        vec!["a".to_string(), "b".to_string()],
        "rows must carry the referenced column values: {lines:?}"
    );
}

/// R3-S2：表达式项 × JOIN 维持既有拒绝（语义面不扩大）。
#[tokio::test]
async fn expression_item_join_still_rejected() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT upper(t1.v) FROM t1 JOIN t2 ON t1.k = t2.k")
            .await,
    ));
    assert!(
        msg.contains("expression") || msg.contains("not supported"),
        "expression item over JOIN base must keep the existing rejection: {msg}"
    );
}

// ==== R4：ORDER BY 别名键与 Sort 输入行同基座 ====

/// R4-S1：无 WHERE 的 JOIN 上 ORDER BY 别名键真实排序。
///
/// RED（实施前实测）：`SELECT t1.v AS val FROM t1 JOIN t2 … ORDER BY val` 保持
/// 输入序（别名键静默跳过）。
#[test]
fn alias_sort_over_join() {
    let dir = fixture();
    let out = run_cli(
        dir.path(),
        &[
            "app",
            "--format",
            "table",
            "SELECT t1.v AS val FROM t1 JOIN t2 ON t1.k = t2.k ORDER BY val",
        ],
    );
    assert_eq!(out.code, Some(0), "query failed: {}", out.stderr);
    let rows: Vec<String> = out
        .stdout
        .lines()
        .skip(2)
        .map(|l| l.trim().to_string())
        .collect();
    assert_eq!(rows, vec!["a".to_string(), "b".to_string()]);
}

/// R4-S2：带 WHERE 的 JOIN 上 ORDER BY 别名键真实排序（全组合布局面）。
#[test]
fn alias_sort_over_join_with_where() {
    let dir = fixture();
    let out = run_cli(
        dir.path(),
        &[
            "app",
            "--format",
            "table",
            "SELECT t1.v AS val FROM t1 JOIN t2 ON t1.k = t2.k WHERE t1.k > 0 ORDER BY val",
        ],
    );
    assert_eq!(out.code, Some(0), "query failed: {}", out.stderr);
    let rows: Vec<String> = out
        .stdout
        .lines()
        .skip(2)
        .map(|l| l.trim().to_string())
        .collect();
    assert_eq!(rows, vec!["a".to_string(), "b".to_string()]);
}

// ==== R6（MS25 T12）：JOIN 基座聚合参数列缺失计划期点名 ====

/// T12-S1：JOIN 基座未知裸名聚合参数 → `ColumnNotFound` 点名。
///
/// RED（Cycle 002 前实测）：`SELECT count(nosuch) FROM t1 JOIN t2 ON t1.k = t2.k`
/// 返回 `[[0]]` exit 0——`resolve_layout_index` 零命中返回 `Ok(None)`，参数回落
/// `column_indices` 名字查找同样落空 → `extract_value` 静默 `Value::Null` → COUNT 计 0。
#[tokio::test]
async fn unknown_bare_agg_arg_rejected() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT count(nosuch) FROM t1 JOIN t2 ON t1.k = t2.k")
            .await,
    ));
    assert!(
        msg.contains("Column not found") && msg.contains("nosuch"),
        "unknown bare aggregate arg over JOIN base must be named: {msg}"
    );
}

/// T12-S2：JOIN 基座未知限定名聚合参数 → `ColumnNotFound` 点名（表在、列不在）。
///
/// RED（Cycle 002 前实测）：`SELECT sum(t1.nosuch) FROM t1 JOIN t2 ON t1.k = t2.k`
/// 返回 `[[null]]` exit 0。
#[tokio::test]
async fn unknown_qualified_agg_arg_rejected() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT sum(t1.nosuch) FROM t1 JOIN t2 ON t1.k = t2.k")
            .await,
    ));
    assert!(
        msg.contains("Column not found") && msg.contains("nosuch"),
        "unknown qualified aggregate arg over JOIN base must be named: {msg}"
    );
}

// ==== CLI 夹具（仅表头/排序见证需要真二进制）====

fn fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    // 建表与夹具数据走真二进制，保持与断言面同一现场。
    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建（断言语义不变）。
    let new_out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(new_out.code, Some(0), "new app failed: {}", new_out.stderr);
    for sql in [
        "CREATE TABLE t1 (k INT, v STRING)",
        "CREATE TABLE t2 (k INT, w STRING)",
        "INSERT INTO t1 VALUES (1, 'b')",
        "INSERT INTO t1 VALUES (2, 'a')",
        "INSERT INTO t2 VALUES (1, 'x')",
        "INSERT INTO t2 VALUES (2, 'y')",
    ] {
        let out = run_cli(dir.path(), &["app", sql]);
        assert_eq!(out.code, Some(0), "setup {sql:?} failed: {}", out.stderr);
    }
    dir
}

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait().expect("poll rtsql status") {
            Some(status) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                child
                    .stdout
                    .take()
                    .unwrap()
                    .read_to_string(&mut stdout)
                    .unwrap();
                child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                return CliOutput {
                    code: status.code(),
                    stdout,
                    stderr,
                };
            }
            None => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rtsql did not exit within 60s (one-shot CLI must terminate)");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

// ==== T10（change 2026-10-04-external-review-gaps）：JOIN 基座输出列序
// SELECT 化（R10，S38–S41）====
//
// 纯列清单的 JOIN 基座输出列序 SHALL 按 SELECT 清单序（对齐 Scan 基座既有
// 语义）：裸路径 kept 与 WHERE/ORDER BY 裁剪路径同规则；重复项重复产出；
// `SELECT *` 表序不变；消费面（CTAS 列名）随输出表头。

/// S38：裸路径（无 WHERE / 无 ORDER BY）按 SELECT 序。
#[tokio::test]
async fn select_order_bare_path() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT t2.w, t1.v FROM t1 JOIN t2 ON t1.k = t2.k")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!("x"), json!("b")], vec![json!("y"), json!("a")]],
        "rows: {rows:?}"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S39：WHERE 裁剪路径按 SELECT 序（Filter 承载裁剪）。
#[tokio::test]
async fn select_order_where_path() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT t2.w, t1.v FROM t1 JOIN t2 ON t1.k = t2.k WHERE t2.k = 2")
            .await,
    ));
    assert_eq!(rows, vec![vec![json!("y"), json!("a")]], "rows: {rows:?}");
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S40：ORDER BY 裁剪路径按 SELECT 序（Sort 承载裁剪，排序键在裁剪之外
/// 仍可达）。
#[tokio::test]
async fn select_order_order_by_path() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT t2.w, t1.v FROM t1 JOIN t2 ON t1.k = t2.k ORDER BY t1.v")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!("y"), json!("a")], vec![json!("x"), json!("b")]],
        "rows: {rows:?}"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S41：重复项重复产出；`SELECT *` 表序不变。
#[tokio::test]
async fn select_order_duplicates_and_star() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT t1.v, t1.v FROM t1 JOIN t2 ON t1.k = t2.k WHERE t1.k = 1")
            .await,
    ));
    assert_eq!(rows, vec![vec![json!("b"), json!("b")]], "rows: {rows:?}");
    let rows = query_rows(to_response(
        db.execute_sql("SELECT * FROM t1 JOIN t2 ON t1.k = t2.k ORDER BY t1.k")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!("b"), json!(1), json!("x")],
            vec![json!(2), json!("a"), json!(2), json!("y")],
        ],
        "rows: {rows:?}"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S41：消费面表头随输出序——CTAS 列名取输出表头（与 Scan 基座一致）。
#[tokio::test]
async fn select_order_ctas_column_names() {
    let dir = tempdir().unwrap();
    let db = join_base(&dir).await;
    exec_ok(
        &db,
        "CREATE TABLE ct AS SELECT t2.w, t1.v FROM t1 JOIN t2 ON t1.k = t2.k",
    )
    .await;
    // SELECT * 直接暴露物化列序——表头随 SELECT 输出序（修复前表序
    // [v, w]，行随之错序）。
    let rows = query_rows(to_response(db.execute_sql("SELECT * FROM ct").await));
    assert_eq!(
        rows,
        vec![vec![json!("x"), json!("b")], vec![json!("y"), json!("a")]],
        "rows: {rows:?}"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}
