//! MS25 Iteration 004 / Cycle 001：聚合输出形状一致性收口（Task T9，design D9）
//!
//! change: 2026-09-26-ms25-sql-read-surface
//!
//! 病灶（Plan 实测）：`direct` 判定只检查 `keys_all_column` 与「SELECT 序为键项
//! 在前」，不检查**键是否为分组键的子集**与**键序**——于是
//! - `SELECT count(*) FROM t GROUP BY v`：表头 1 列而行 2 值；
//! - `SELECT v, k, count(*) FROM t GROUP BY k, v`：行按 GROUP BY 序、表头按
//!   SELECT 序，行列错位。
//!
//! 目标语义（delta spec `aggregate-output-shape`）：
//! - R1 形状守恒：表头列数恒等于行值数（投影项集合 = 分组键集合）。
//! - R2 键序无关：SELECT 项为分组键超集时按 SELECT 序出值。
//! - R3 既有形状零回归（含两处白名单校准用例）。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use serde_json::json;
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("test")
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = db.execute_sql(sql).await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "setup statement failed: {sql:?} -> {resp:?}"
    );
}

async fn seed(dir: &TempDir) -> Database {
    let db = Database::open(&db_path(dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, k INT, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 1, 10)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 1, 20)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 2, 30)").await;
    db
}

fn rows_of(resp: Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("Expected QueryResult, got {other:?}"),
    }
}

/// 表头列数（真二进制 JSON 渲染）与行宽必须一致——形状守恒的端到端见证。
fn header_and_rows(sql: &str) -> (Vec<String>, Vec<Vec<serde_json::Value>>) {
    let out = query_cli(sql);
    assert_eq!(out.code, Some(0), "query failed: {}", out.stderr);
    let parsed: serde_json::Value =
        serde_json::from_str(out.stdout.trim()).unwrap_or_else(|e| panic!("{e}: {:?}", out.stdout));
    let columns: Vec<String> = parsed["columns"]
        .as_array()
        .expect("columns")
        .iter()
        .map(|c| c.as_str().expect("column name").to_string())
        .collect();
    let rows: Vec<Vec<serde_json::Value>> = parsed["rows"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| r.as_array().expect("row array").clone())
                .collect()
        })
        .unwrap_or_default();
    for row in &rows {
        assert_eq!(
            row.len(),
            columns.len(),
            "shape conservation: row width must equal header width for {sql:?}"
        );
    }
    (columns, rows)
}

// ==== R1：形状守恒 ====

/// R1-S1：未入选分组键不进输出——表头恰 1 列、每行恰 1 值。
///
/// RED（实施前实测）：`SELECT count(*) FROM t GROUP BY v` 表头 1 列而行 2 值
/// （行首携带未入选的分组键）。
#[test]
fn unselected_group_key_dropped_from_output() {
    let (columns, rows) = header_and_rows("SELECT count(*) FROM t GROUP BY v");
    assert_eq!(
        columns,
        vec!["count_star".to_string()],
        "unselected group key must not appear in the header"
    );
    assert_eq!(
        rows.len(),
        3,
        "one row per v group (10/20/30), each carrying only the aggregate value"
    );
    let mut sums: Vec<i64> = rows
        .iter()
        .map(|r| r[0].as_i64().expect("aggregate value"))
        .collect();
    sums.sort();
    assert_eq!(sums, vec![1, 1, 1], "each v value has exactly one row");
}

/// R1-S2：未入选分组键仍可作 HAVING 绑定依据（Plan 行为扩展）。
#[test]
fn unselected_key_bindable_in_having() {
    let (columns, rows) = header_and_rows("SELECT count(*) FROM t GROUP BY v HAVING v = 10");
    assert_eq!(columns, vec!["count_star".to_string()]);
    assert_eq!(rows, vec![vec![json!(1)]]);
}

/// R1-S3：反向（投影多出非键列项）维持既有点名面。
#[test]
fn non_key_projection_item_named() {
    let out = query_cli("SELECT v, count(*) FROM t GROUP BY k");
    assert_ne!(out.code, Some(0), "non-key projection item must be named");
    assert!(
        out.stderr.contains('v'),
        "error must name the offending projection item: {}",
        out.stderr
    );
}

/// R1-S4：仅聚合项 + 分组键（与 delta spec 场景同名形态）形状守恒。
#[test]
fn aggregate_only_with_group_by_shape_conserved() {
    let (columns, rows) = header_and_rows("SELECT count(*) FROM t GROUP BY k");
    assert_eq!(columns, vec!["count_star".to_string()]);
    assert_eq!(rows.len(), 2);
}

// ==== R2：键序无关 ====

/// R2-S1：SELECT 序与 GROUP BY 序不同时按 SELECT 序出值（形状与语义同时对齐）。
///
/// RED（实施前实测）：`SELECT v, k, count(*) FROM t GROUP BY k, v` 的行按分组键
/// 序（k, v）出值而表头按 SELECT 序（v, k）→ 行列错位。
#[test]
fn select_order_key_order_mismatch_aligned() {
    let (columns, mut rows) = header_and_rows("SELECT v, k, count(*) FROM t GROUP BY k, v");
    assert_eq!(
        columns,
        vec!["v".to_string(), "k".to_string(), "count_star".to_string()],
        "header follows the SELECT list"
    );
    rows.sort_by_key(|r| r[0].to_string());
    assert_eq!(
        rows,
        vec![
            vec![json!(10), json!(1), json!(1)],
            vec![json!(20), json!(1), json!(1)],
            vec![json!(30), json!(2), json!(1)],
        ],
        "each row must line up with its SELECT-list position"
    );
}

/// R2-S2：普通单键形态维持直出（既有形状零回归）。
#[test]
fn simple_key_shape_unchanged() {
    let (columns, mut rows) = header_and_rows("SELECT k, count(*) FROM t GROUP BY k");
    assert_eq!(columns, vec!["k".to_string(), "count_star".to_string()]);
    rows.sort_by_key(|r| r[0].to_string());
    assert_eq!(
        rows,
        vec![vec![json!(1), json!(2)], vec![json!(2), json!(1)]]
    );
}

/// R2-S3：键项在聚合项之后的交叠序形态（形状仍守恒，键项取键值）。
#[test]
fn interleaved_column_and_aggregate_shape_conserved() {
    let (columns, rows) = header_and_rows("SELECT k, count(*), k FROM t GROUP BY k");
    assert_eq!(
        columns,
        vec!["k".to_string(), "count_star".to_string(), "k".to_string()],
        "header follows the SELECT list"
    );
    let mut rows = rows;
    rows.sort_by_key(|r| r[0].to_string());
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!(2), json!(1)],
            vec![json!(2), json!(1), json!(2)]
        ],
        "interleaved form keeps the key value in both key positions"
    );
}

/// R2-S4：带 WHERE 的聚合形态形状守恒。
#[tokio::test]
async fn where_aggregate_shape_conserved() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;
    let rows = rows_of(
        db.execute_sql("SELECT k, count(*) FROM t WHERE k = 1 GROUP BY k")
            .await,
    );
    assert_eq!(rows, vec![vec![json!(1), json!(2)]]);
}

// ==== CLI 夹具 ====

fn query_cli(sql: &str) -> CliOutput {
    let dir = fixture();
    run_cli(dir.path(), &["app", sql])
}

fn fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建（断言语义不变）。
    let new_out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(new_out.code, Some(0), "new app failed: {}", new_out.stderr);
    for sql in [
        "CREATE TABLE t (id INT PRIMARY KEY, k INT, v INT)",
        "INSERT INTO t VALUES (1, 1, 10)",
        "INSERT INTO t VALUES (2, 1, 20)",
        "INSERT INTO t VALUES (3, 2, 30)",
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

fn run_cli(dir: &std::path::Path, args: &[&str]) -> CliOutput {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        match child.try_wait().expect("poll rtsql status") {
            Some(status) => {
                use std::io::Read;
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
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rtsql did not exit within 60s (one-shot CLI must terminate)");
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
}
