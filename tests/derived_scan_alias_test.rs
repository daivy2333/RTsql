//! MS25 Iteration 004 / Cycle 001：派生表 / CTE 基座的输出别名应用收口（Task T10，design D9）
//!
//! change: 2026-09-26-ms25-sql-read-surface
//!
//! 病灶（Plan 实测）：`alias_reroutable` 的基座条件是「仅 `Scan(_)`」，派生表与
//! CTE 引用（两者共用 `DerivedScan` 装配路径）不在其内——纯列引用别名项既不进
//! 顶层 Projection 别名面（表头退化为列名），`ORDER BY <别名>` 键又不在 Sort 的
//! 可见列集内而点名列不存在。
//!
//! 目标语义（delta spec `derived-scan-alias`）：
//! - R1 FROM 子查询基座的纯列别名项：表头 = 别名、行值 = 所指列。
//! - R2 CTE 基座同款形态（同一 DerivedScan 装配路径）。
//! - R3 别名排序可达（DESC）；无别名、限定名引用与既有拒绝面零回归。

use rtsql::database::Database;
use serde_json::json;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("test")
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = db.execute_sql(sql).await;
    assert!(
        !matches!(resp, rtsql::network::protocol::Response::Error { .. }),
        "setup statement failed: {sql:?} -> {resp:?}"
    );
}

async fn seed(dir: &TempDir) -> Database {
    let db = Database::open(&db_path(dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (v STRING, n INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('b', 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('a', 2)").await;
    db
}

fn rows_of(resp: rtsql::network::protocol::Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        rtsql::network::protocol::Response::QueryResult { rows } => rows,
        other => panic!("Expected QueryResult, got {other:?}"),
    }
}

// ==== R1：FROM 子查询基座 ====

/// R1-S1：派生表基座的纯列别名项表头 = 别名、行值 = 所指列。
///
/// RED（实施前实测）：表头为列名 `v`（别名被丢弃）。
#[tokio::test]
async fn derived_plain_column_alias_name() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT v AS val FROM (SELECT v FROM t)")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("b")], vec![json!("a")]]);
}

/// R1-S2：限定名引用的派生表别名项。
#[tokio::test]
async fn derived_qualified_alias_name() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT d.v AS val FROM (SELECT v FROM t) AS d")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("b")], vec![json!("a")]]);
}

// ==== R2：CTE 基座 ====

/// R2-S1：CTE 基座的纯列别名项（与 FROM 子查询共用 DerivedScan 装配路径）。
#[tokio::test]
async fn cte_plain_column_alias_name() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("WITH c AS (SELECT v FROM t) SELECT v AS val FROM c")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("b")], vec![json!("a")]]);
}

/// R2-S2：CTE 基座的别名排序（DESC）真实生效。
///
/// RED（实施前实测）：`ORDER BY val` 计划期 `ColumnNotFound` 点名。
#[tokio::test]
async fn cte_alias_sort_desc() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("WITH c AS (SELECT v FROM t) SELECT v AS val FROM c ORDER BY val DESC")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("b")], vec![json!("a")]]);
}

// ==== R3：既有面零回归 ====

/// R3-S1：无别名的派生表引用表头维持列名。
#[test]
fn derived_no_alias_header_unchanged() {
    let out = query_cli("SELECT v FROM (SELECT v FROM t)");
    assert_eq!(out.code, Some(0), "query failed: {}", out.stderr);
    assert_eq!(
        out.columns(),
        vec!["v".to_string()],
        "no-alias derived reference must keep the column name header"
    );
}

/// R3-S2：别名项表头 = 别名（真二进制 table 渲染）。
#[test]
fn derived_alias_header_is_alias() {
    let out = query_cli("SELECT v AS val FROM (SELECT v FROM t)");
    assert_eq!(out.code, Some(0), "query failed: {}", out.stderr);
    assert_eq!(
        out.columns(),
        vec!["val".to_string()],
        "alias item header must be the alias"
    );
}

/// R3-S3：表达式项 × 派生表维持既有可达语义（Iteration 000 值表达式面），
/// 表头 = 表达式文本——别名门放宽不改变该面。
#[tokio::test]
async fn derived_expression_item_unchanged() {
    let dir = tempdir().unwrap();
    let db = seed(&dir).await;

    let rows = rows_of(
        db.execute_sql("SELECT upper(v) FROM (SELECT v FROM t)")
            .await,
    );
    assert_eq!(rows, vec![vec![json!("B")], vec![json!("A")]]);
    let out = query_cli("SELECT upper(v) FROM (SELECT v FROM t)");
    assert_eq!(out.code, Some(0), "query failed: {}", out.stderr);
    assert_eq!(
        out.columns(),
        vec!["upper(v)".to_string()],
        "expression item header must stay the expression text"
    );
}

// ==== CLI 夹具（表头断言需要真二进制 JSON 列名）====

fn query_cli(sql: &str) -> CliOutput {
    let dir = fixture();
    run_cli(dir.path(), &["app", sql])
}

fn fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    for sql in [
        "CREATE TABLE t (v STRING, n INT)",
        "INSERT INTO t VALUES ('b', 1)",
        "INSERT INTO t VALUES ('a', 2)",
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

impl CliOutput {
    /// JSON 渲染的列名清单（CLI 默认格式）。
    fn columns(&self) -> Vec<String> {
        let parsed: serde_json::Value = serde_json::from_str(self.stdout.trim())
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {:?}", self.stdout));
        parsed["columns"]
            .as_array()
            .expect("columns array")
            .iter()
            .map(|c| c.as_str().expect("column name").to_string())
            .collect()
    }
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
