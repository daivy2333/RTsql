//! MS13 Iteration 002 T9：no-FORM SELECT 虚拟单行（change
//! 2026-09-23-ms13-analytics-functions）
//!
//! 验收域：no-from-select delta spec——R1 单行可达（常量算术 / 标量函数 /
//! CASE / 类型字面量）、R2 拒绝面点名（通配 / WHERE / 聚合 / ORDER BY /
//! LIMIT）与列引用错误语义、R3 lib 直连与 CLI 面一致性抽检。CLI e2e
//! （真二进制）承载表头 + 行形状 + 拒绝面文案断言。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

#[derive(Debug)]
struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    let child = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary");
    wait_cli(child)
}

fn wait_cli(mut child: std::process::Child) -> CliOutput {
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
                    panic!("rtsql did not exit within 60s");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn new_db(dir: &Path, name: &str) {
    let out = run_cli(dir, &["new", name]);
    assert_eq!(
        out.code,
        Some(0),
        "rtsql new {name} should succeed: {out:?}"
    );
}

fn parse_json(out: &CliOutput) -> (Vec<String>, Vec<Vec<serde_json::Value>>) {
    assert_eq!(out.code, Some(0), "查询应成功: {out:?}");
    let v: serde_json::Value = serde_json::from_str(out.stdout.trim())
        .unwrap_or_else(|e| panic!("stdout 应为 JSON: {e}; out={out:?}"));
    let columns = v["columns"]
        .as_array()
        .expect("columns array")
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    let rows = v["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .map(|r| r.as_array().expect("row array").clone())
        .collect();
    (columns, rows)
}

/// 拒绝面断言：exit 3 + 点名文案（非既有 MissingField 兜底文案）。
fn expect_rejected(dir: &Path, sql: &str) {
    let out = run_cli(dir, &["probe", sql]);
    assert_eq!(
        out.code,
        Some(3),
        "{sql} must be rejected with exit 3: {out:?}"
    );
    let combined = format!("{}{}", out.stderr, out.stdout);
    assert!(
        combined.contains("not supported without FROM"),
        "{sql}: expected a named no-FROM rejection: {out:?}"
    );
    assert!(
        !combined.contains("Missing required field"),
        "{sql}: must not fall through to the legacy MissingField text: {out:?}"
    );
}

// ---- R1 单行可达 ----

/// R1/S1: `SELECT 1 + 1` 无 FROM 单行 [[2]]，表头 = 表达式文本（既有语义）。
#[test]
fn nofrom_arithmetic_single_row() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "probe");
    let out = run_cli(dir.path(), &["probe", "SELECT 1 + 1"]);
    let (columns, rows) = parse_json(&out);
    assert_eq!(columns, vec!["1 + 1".to_string()]);
    assert_eq!(rows, vec![vec![serde_json::json!(2)]]);
}

/// R1/S2: 标量函数探测——别名表头 `u`；`now()` 单行可达（字符串形态）。
#[test]
fn nofrom_scalar_function_with_alias() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "probe");
    let out = run_cli(dir.path(), &["probe", "SELECT upper('a') AS u"]);
    let (columns, rows) = parse_json(&out);
    assert_eq!(columns, vec!["u".to_string()]);
    assert_eq!(rows, vec![vec![serde_json::json!("A")]]);

    let out = run_cli(dir.path(), &["probe", "SELECT now()"]);
    let (columns, rows) = parse_json(&out);
    assert_eq!(columns.len(), 1, "single projection item: {out:?}");
    assert_eq!(rows.len(), 1, "now() must yield exactly one row");
    assert!(
        rows[0][0].as_str().is_some_and(|s| !s.is_empty()),
        "now() must render a non-empty string form: {:?}",
        rows[0][0]
    );
}

/// R1/S3: CASE 探针组合。
#[test]
fn nofrom_case_probe() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "probe");
    let out = run_cli(
        dir.path(),
        &[
            "probe",
            "SELECT CASE WHEN 1 = 1 THEN 'yes' ELSE 'no' END AS probe",
        ],
    );
    let (columns, rows) = parse_json(&out);
    assert_eq!(columns, vec!["probe".to_string()]);
    assert_eq!(rows, vec![vec![serde_json::json!("yes")]]);
}

/// R1: 类型字面量组合（Iteration 000 TypedString 通路在 no-FORM 下复用）。
#[test]
fn nofrom_typed_literal() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "probe");
    let out = run_cli(dir.path(), &["probe", "SELECT DATE '2024-01-01'"]);
    let (_columns, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!("2024-01-01")]]);
}

// ---- R2 拒绝面 ----

/// R2/S1: 通配符显式拒绝。
#[test]
fn nofrom_wildcard_rejected() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "probe");
    expect_rejected(dir.path(), "SELECT *");
}

/// R2/S2: 谓词与聚合显式拒绝。
#[test]
fn nofrom_where_and_aggregate_rejected() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "probe");
    expect_rejected(dir.path(), "SELECT 1 WHERE 1 = 0");
    expect_rejected(dir.path(), "SELECT COUNT(*)");
}

/// R2: ORDER BY 与 LIMIT（分页子句）显式拒绝。
#[test]
fn nofrom_orderby_and_limit_rejected() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "probe");
    expect_rejected(dir.path(), "SELECT 1 ORDER BY 1");
    expect_rejected(dir.path(), "SELECT 1 LIMIT 1");
}

/// R2/S3: 列引用维持列不存在类错误语义（非拒绝面文案）。
#[test]
fn nofrom_column_reference_error() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "probe");
    let out = run_cli(dir.path(), &["probe", "SELECT nonexistent_col"]);
    assert_eq!(out.code, Some(3), "{out:?}");
    let combined = format!("{}{}", out.stderr, out.stdout);
    assert!(
        combined.contains("Column not found"),
        "column reference without FROM must yield ColumnNotFound: {out:?}"
    );
    assert!(
        !combined.contains("not supported without FROM"),
        "column reference is a resolution error, not the rejection face: {out:?}"
    );
}

// ---- R3 lib 直连一致性 ----

/// R3: lib 直连（execute_sql）与 CLI 面行为一致——`SELECT 1 + 1` 单行 [[2]]。
#[tokio::test]
async fn lib_direct_matches_cli_shape() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("nofrom.db")).await.unwrap();
    let resp = db.execute_sql("SELECT 1 + 1").await;
    match resp {
        Response::QueryResult { rows } => {
            assert_eq!(rows, vec![vec![serde_json::json!(2)]]);
        }
        other => panic!("expected QueryResult, got {:?}", other),
    }
}
