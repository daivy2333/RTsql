//! Integration tests for MS11-T02 Iteration 000 T3: non-session rejection of
//! transaction statements (R4/S1-S3).
//!
//! `Database::execute_sql` (network JSON/PG same-source) and `execute_in_tx`
//! SHALL reject BEGIN/COMMIT/ROLLBACK with the design D3 session-only message
//! before plan-cache insertion; a rejection inside an explicit transaction
//! SHALL NOT terminate it. The CLI-session scenarios (R1/R2/R3) extend this
//! file in Iteration 001.

use rtsql::database::Database;
use rtsql::network::handler::SqlHandler;
use rtsql::network::protocol::{Request, Response};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

async fn open_db() -> (Arc<Database>, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("tx_statement.db"))
        .await
        .unwrap();
    (Arc::new(db), dir)
}

fn expect_ok(resp: &Response, what: &str) {
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{what} failed: {:?}",
        resp
    );
}

fn expect_error_contains(resp: Response, needle: &str, what: &str) -> String {
    match resp {
        Response::Error { message } => {
            assert!(
                message.contains(needle),
                "{what}: expected message containing {needle:?}, got {message:?}"
            );
            message
        }
        other => panic!("{what}: expected Response::Error, got {:?}", other),
    }
}

/// R4/S1: `execute_sql("BEGIN")` is rejected with the design D3 session-only
/// message and the plan cache is untouched (no insertion, no eviction).
#[tokio::test]
async fn execute_sql_rejects_begin_without_touching_plan_cache() {
    let (db, _dir) = open_db().await;
    expect_ok(&db.execute_sql("CREATE TABLE t (id INT)").await, "create t");
    expect_ok(&db.execute_sql("SELECT id FROM t").await, "prime cache");
    let len_before = db.plan_cache_len();
    assert!(len_before > 0, "SELECT should populate the plan cache");

    let resp = db.execute_sql("BEGIN").await;
    expect_error_contains(
        resp,
        "only supported in an rtsql CLI session",
        "execute_sql(BEGIN)",
    );
    assert_eq!(
        db.plan_cache_len(),
        len_before,
        "rejected BEGIN must not change the plan cache"
    );
}

/// R4/S2: `execute_in_tx("COMMIT", &tx)` is rejected, and the rejection does
/// not terminate the transaction — in-tx DML still succeeds and the explicit
/// commit afterwards persists the write.
#[tokio::test]
async fn execute_in_tx_rejects_commit_and_transaction_stays_usable() {
    let (db, _dir) = open_db().await;
    expect_ok(&db.execute_sql("CREATE TABLE t (id INT)").await, "create t");
    let tx = db.begin().await.unwrap();

    let resp = db.execute_in_tx("COMMIT", &tx).await;
    expect_error_contains(
        resp,
        "only supported in an rtsql CLI session",
        "execute_in_tx(COMMIT)",
    );

    expect_ok(
        &db.execute_in_tx("INSERT INTO t VALUES (1)", &tx).await,
        "in-tx insert after rejection",
    );
    db.commit(tx).await.unwrap();

    match db.execute_sql("SELECT id FROM t").await {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1, "committed write must be visible");
            assert_eq!(rows[0][0], serde_json::json!(1));
        }
        other => panic!("expected QueryResult, got {:?}", other),
    }
}

/// R4/S3: the network JSON/PG path is same-source with `execute_sql` —
/// `SqlHandler` rejects COMMIT with the identical message.
#[tokio::test]
async fn sql_handler_rejects_commit_same_source_as_execute_sql() {
    let (db, _dir) = open_db().await;
    let handler = SqlHandler::new(db.clone());

    let via_handler = expect_error_contains(
        handler
            .execute(Request::Query {
                sql: "COMMIT".to_string(),
            })
            .await,
        "only supported in an rtsql CLI session",
        "SqlHandler(COMMIT)",
    );
    let via_db = expect_error_contains(
        db.execute_sql("COMMIT").await,
        "only supported in an rtsql CLI session",
        "execute_sql(COMMIT)",
    );
    assert_eq!(
        via_handler, via_db,
        "network handler and execute_sql must produce the same rejection message"
    );
}

// ---------------------------------------------------------------------------
// CLI e2e section (MS11-T02 Iteration 001, T5) — real binary, one-shot calls.
// Scenario mapping (requirement → scenario → test):
//   R1/S1 tx_commit_roundtrip_visible_after_reopen
//   R1/S2 tx_rollback_leaves_no_residue
//   R1/S3 in_tx_select_sees_own_uncommitted_insert
//   R1/S4 in_tx_select_after_update_yields_both_versions
//   R1/S5 in_tx_ddl_survives_rollback
//   R2/S1 reject_set_transaction          R2/S2 reject_savepoint_and_release
//   R2/S3 reject_begin_with_modes         R2/S4 reject_chain_forms
//   R2/S5 and_no_chain_behaves_as_bare
//   R3/S1 commit_without_tx_errors        R3/S2 nested_begin_errors_and_rolls_back_at_exit
//   R3/S3 open_tx_implicitly_rolled_back_with_notice
//   R3/S4 in_tx_fail_fast_notes_uncommitted
//   MODIFIED S7 multi_statement_in_tx_fail_fast (same observable as R3/S4)
//   MODIFIED S8 multi_statement_open_tx_rolled_back_at_exit (same observable as R3/S3)
// Message assertions use design D3 substrings only.
// ---------------------------------------------------------------------------

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 运行 rtsql 二进制并等待退出；60s 未退出则 kill 并 panic
/// （夹具模式与 tests/cli_test.rs 一致：独立 TempDir CWD + RTSQL_HOME，
/// stdout/stderr 均为管道 → 默认 json 格式）。
fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
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

/// 每个测试独立的仓库现场：TempDir + 预建 `db/` 集中存储目录
fn cli_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    dir
}

/// 以裸名 `app` 执行一条 auto-commit 语句，非零退出即 panic（建表/种子用）。
fn run_cli_ok(dir: &Path, sql: &str, what: &str) {
    let out = run_cli(dir, &["app", sql]);
    assert_eq!(out.code, Some(0), "{what} failed: {}", out.stderr);
}

/// stdout 按“每条语句一个独立 JSON 文档（每行一个）”解析。
fn json_docs(stdout: &str) -> Vec<serde_json::Value> {
    stdout
        .trim()
        .lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("invalid JSON document {line:?}: {e}"))
        })
        .collect()
}

/// 重开（新 CLI 调用）执行 SELECT 并返回 rows 数组。
fn select_rows(dir: &Path, sql: &str, what: &str) -> Vec<serde_json::Value> {
    let out = run_cli(dir, &["app", sql]);
    assert_eq!(out.code, Some(0), "{what} failed: {}", out.stderr);
    json_docs(&out.stdout).remove(0)["rows"]
        .as_array()
        .cloned()
        .expect("rows array in select output")
}

/// R1/S1: BEGIN→INSERT→COMMIT commits durably; each statement renders one
/// affected-rows document (BEGIN/COMMIT report 0, INSERT reports 1).
#[test]
fn tx_commit_roundtrip_visible_after_reopen() {
    let dir = cli_fixture();
    run_cli_ok(dir.path(), "CREATE TABLE t (id INT)", "create t");

    let out = run_cli(
        dir.path(),
        &["app", "BEGIN; INSERT INTO t VALUES (1); COMMIT"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let docs = json_docs(&out.stdout);
    assert_eq!(
        docs.len(),
        3,
        "three statements, three documents: {:?}",
        out.stdout
    );
    assert_eq!(docs[0]["affected_rows"], serde_json::json!(0));
    assert_eq!(docs[1]["affected_rows"], serde_json::json!(1));
    assert_eq!(docs[2]["affected_rows"], serde_json::json!(0));

    let rows = select_rows(dir.path(), "SELECT id FROM t", "reopen select");
    assert_eq!(rows, vec![serde_json::json!([1])]);
}

/// R1/S2: BEGIN→INSERT→ROLLBACK leaves no residue.
#[test]
fn tx_rollback_leaves_no_residue() {
    let dir = cli_fixture();
    run_cli_ok(dir.path(), "CREATE TABLE t (id INT)", "create t");

    let out = run_cli(
        dir.path(),
        &["app", "BEGIN; INSERT INTO t VALUES (1); ROLLBACK"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    let rows = select_rows(dir.path(), "SELECT id FROM t", "reopen select");
    assert!(rows.is_empty(), "rolled-back write must be invisible");
}

/// R1/S3: in-transaction SELECT sees the transaction's own uncommitted insert.
#[test]
fn in_tx_select_sees_own_uncommitted_insert() {
    let dir = cli_fixture();
    run_cli_ok(dir.path(), "CREATE TABLE t (id INT)", "create t");

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "BEGIN; INSERT INTO t VALUES (1); SELECT id FROM t; COMMIT",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let docs = json_docs(&out.stdout);
    assert_eq!(docs.len(), 4, "four statements: {:?}", out.stdout);
    assert_eq!(
        docs[2]["rows"],
        serde_json::json!([[1]]),
        "uncommitted in-tx insert must be visible to in-tx select"
    );
}

/// R1/S4: in-transaction SELECT after UPDATE yields both versions
/// (snapshot-free scan: uncommitted new version does not suppress the old
/// one — documented engine semantics). ROLLBACK keeps 'a' on reopen.
#[test]
fn in_tx_select_after_update_yields_both_versions() {
    let dir = cli_fixture();
    run_cli_ok(
        dir.path(),
        "CREATE TABLE t (id INT PRIMARY KEY, name STRING)",
        "create t",
    );
    run_cli_ok(dir.path(), "INSERT INTO t VALUES (1, 'a')", "seed row");

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "BEGIN; UPDATE t SET name='b' WHERE id=1; SELECT name FROM t; ROLLBACK",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let docs = json_docs(&out.stdout);
    assert_eq!(docs.len(), 4, "four statements: {:?}", out.stdout);
    let mut names: Vec<String> = docs[2]["rows"]
        .as_array()
        .expect("select rows")
        .iter()
        .map(|r| r[0].as_str().unwrap().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["a".to_string(), "b".to_string()],
        "snapshot-free in-tx scan must yield both versions"
    );

    let rows = select_rows(dir.path(), "SELECT name FROM t", "reopen select");
    assert_eq!(rows, vec![serde_json::json!(["a"])]);
}

/// R1/S5: in-transaction DDL takes effect immediately and is not undone by
/// ROLLBACK (documented engine semantics).
#[test]
fn in_tx_ddl_survives_rollback() {
    let dir = cli_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "BEGIN; CREATE TABLE t2 (id INT PRIMARY KEY); ROLLBACK",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    let rows = select_rows(dir.path(), "SELECT * FROM t2", "reopen select on t2");
    assert!(
        rows.is_empty(),
        "t2 must exist after rollback (DDL not undone), empty rowset"
    );
}

/// R2/S1: SET TRANSACTION is rejected with its named clause message.
#[test]
fn reject_set_transaction() {
    let dir = cli_fixture();
    let out = run_cli(
        dir.path(),
        &["app", "SET TRANSACTION ISOLATION LEVEL READ COMMITTED"],
    );
    assert_eq!(out.code, Some(3), "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("SET TRANSACTION") && out.stderr.contains("not supported"),
        "stderr must name SET TRANSACTION as unsupported: {:?}",
        out.stderr
    );
}

/// R2/S2: SAVEPOINT and RELEASE SAVEPOINT are rejected, each naming its clause.
#[test]
fn reject_savepoint_and_release() {
    let dir = cli_fixture();

    let out = run_cli(dir.path(), &["app", "SAVEPOINT sp1"]);
    assert_eq!(out.code, Some(3), "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("SAVEPOINT is not supported") && !out.stderr.contains("RELEASE"),
        "stderr must name SAVEPOINT (not RELEASE): {:?}",
        out.stderr
    );

    let out = run_cli(dir.path(), &["app", "RELEASE SAVEPOINT sp1"]);
    assert_eq!(out.code, Some(3), "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("RELEASE SAVEPOINT is not supported"),
        "stderr must name RELEASE SAVEPOINT: {:?}",
        out.stderr
    );
}

/// R2/S3: BEGIN/START TRANSACTION carrying transaction modes are rejected.
#[test]
fn reject_begin_with_modes() {
    let dir = cli_fixture();

    for sql in [
        "BEGIN ISOLATION LEVEL SERIALIZABLE",
        "START TRANSACTION READ ONLY",
    ] {
        let out = run_cli(dir.path(), &["app", sql]);
        assert_eq!(out.code, Some(3), "{sql}: stderr: {}", out.stderr);
        assert!(
            out.stderr
                .contains("transaction modes in BEGIN/START TRANSACTION are not supported"),
            "{sql}: stderr must name unsupported transaction modes: {:?}",
            out.stderr
        );
    }
}

/// R2/S4: CHAIN forms and ROLLBACK TO SAVEPOINT are rejected, each naming
/// its clause.
#[test]
fn reject_chain_forms() {
    let dir = cli_fixture();

    let out = run_cli(dir.path(), &["app", "COMMIT AND CHAIN"]);
    assert_eq!(out.code, Some(3), "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("COMMIT AND CHAIN is not supported"),
        "stderr: {:?}",
        out.stderr
    );

    let out = run_cli(dir.path(), &["app", "ROLLBACK AND CHAIN"]);
    assert_eq!(out.code, Some(3), "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("ROLLBACK AND CHAIN is not supported"),
        "stderr: {:?}",
        out.stderr
    );

    let out = run_cli(dir.path(), &["app", "ROLLBACK TO SAVEPOINT sp1"]);
    assert_eq!(out.code, Some(3), "stderr: {}", out.stderr);
    assert!(
        out.stderr
            .contains("ROLLBACK TO SAVEPOINT is not supported"),
        "stderr: {:?}",
        out.stderr
    );
}

/// R2/S5: `COMMIT AND NO CHAIN` parses as bare COMMIT (sqlparser 0.44 AST
/// equivalence) and takes bare-statement session semantics — the idle-session
/// error, never the AND CHAIN rejection.
#[test]
fn and_no_chain_behaves_as_bare() {
    let dir = cli_fixture();
    let out = run_cli(dir.path(), &["app", "COMMIT AND NO CHAIN"]);
    assert_eq!(out.code, Some(3), "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("no active transaction"),
        "stderr must carry the bare-statement session error: {:?}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("AND CHAIN"),
        "NO CHAIN must not be rejected as a chain form: {:?}",
        out.stderr
    );
}

/// R3/S1: COMMIT/ROLLBACK without an active transaction error with the
/// session message; table data is unchanged.
#[test]
fn commit_without_tx_errors() {
    let dir = cli_fixture();
    run_cli_ok(dir.path(), "CREATE TABLE t (id INT)", "create t");
    run_cli_ok(dir.path(), "INSERT INTO t VALUES (7)", "seed row");

    for sql in ["COMMIT", "ROLLBACK"] {
        let out = run_cli(dir.path(), &["app", sql]);
        assert_eq!(out.code, Some(3), "{sql}: stderr: {}", out.stderr);
        assert!(
            out.stderr.contains("no active transaction"),
            "{sql}: stderr: {:?}",
            out.stderr
        );
    }

    let rows = select_rows(dir.path(), "SELECT id FROM t", "reopen select");
    assert_eq!(rows, vec![serde_json::json!([7])], "data unchanged");
}

/// R3/S2: a second BEGIN inside an open transaction errors with
/// `transaction already active`; the call exits 3 and the open transaction
/// is rolled back (no residue).
#[test]
fn nested_begin_errors_and_rolls_back_at_exit() {
    let dir = cli_fixture();
    run_cli_ok(dir.path(), "CREATE TABLE t (id INT)", "create t");

    let out = run_cli(
        dir.path(),
        &["app", "BEGIN; INSERT INTO t VALUES (1); BEGIN; COMMIT"],
    );
    assert_eq!(out.code, Some(3), "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("transaction already active"),
        "stderr: {:?}",
        out.stderr
    );

    let rows = select_rows(dir.path(), "SELECT id FROM t", "reopen select");
    assert!(rows.is_empty(), "open transaction must be rolled back");
}

/// R3/S3: a call ending with the transaction still open rolls it back
/// explicitly, prints the stderr notice, and still exits 0.
#[test]
fn open_tx_implicitly_rolled_back_with_notice() {
    let dir = cli_fixture();
    run_cli_ok(dir.path(), "CREATE TABLE t (id INT)", "create t");

    let out = run_cli(dir.path(), &["app", "BEGIN; INSERT INTO t VALUES (1)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert!(
        out.stderr
            .contains("uncommitted transaction was rolled back at exit"),
        "stderr must carry the rollback notice: {:?}",
        out.stderr
    );

    let rows = select_rows(dir.path(), "SELECT id FROM t", "reopen select");
    assert!(rows.is_empty(), "uncommitted write must not persist");
}

/// R3/S4: a failure inside a transaction fails fast (exit 3), notes the
/// statement index AND that previous statements were not committed (rolled
/// back with the transaction), and leaves no partial effects.
#[test]
fn in_tx_fail_fast_notes_uncommitted() {
    let dir = cli_fixture();
    run_cli_ok(dir.path(), "CREATE TABLE t (id INT)", "create t");

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "BEGIN; INSERT INTO t VALUES (1); INSERT INTO missing_table VALUES (2)",
        ],
    );
    assert_eq!(out.code, Some(3), "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("statement 3 of 3"),
        "stderr must carry the failing index: {:?}",
        out.stderr
    );
    assert!(
        out.stderr
            .contains("previous statement(s) were not committed"),
        "stderr must note the uncommitted rollback: {:?}",
        out.stderr
    );

    let rows = select_rows(dir.path(), "SELECT id FROM t", "reopen select");
    assert!(rows.is_empty(), "no partial effects may survive");
}

/// MODIFIED S7 (same observable as R3/S4): multi-statement call with an open
/// session transaction fails fast and notes the uncommitted rollback.
#[test]
fn multi_statement_in_tx_fail_fast() {
    let dir = cli_fixture();
    run_cli_ok(dir.path(), "CREATE TABLE t (id INT)", "create t");

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "BEGIN; INSERT INTO t VALUES (1); INSERT INTO missing_table VALUES (2)",
        ],
    );
    assert_eq!(out.code, Some(3), "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("statement 3 of 3")
            && out
                .stderr
                .contains("previous statement(s) were not committed"),
        "stderr: {:?}",
        out.stderr
    );

    let rows = select_rows(dir.path(), "SELECT id FROM t", "reopen select");
    assert!(rows.is_empty(), "no partial effects may survive");
}

/// MODIFIED S8 (same observable as R3/S3): multi-statement call ending with
/// an open transaction rolls it back at exit with the stderr notice, exit 0.
#[test]
fn multi_statement_open_tx_rolled_back_at_exit() {
    let dir = cli_fixture();
    run_cli_ok(dir.path(), "CREATE TABLE t (id INT)", "create t");

    let out = run_cli(dir.path(), &["app", "BEGIN; INSERT INTO t VALUES (1)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert!(
        out.stderr
            .contains("uncommitted transaction was rolled back at exit"),
        "stderr: {:?}",
        out.stderr
    );

    let rows = select_rows(dir.path(), "SELECT id FROM t", "reopen select");
    assert!(rows.is_empty(), "uncommitted write must not persist");
}
