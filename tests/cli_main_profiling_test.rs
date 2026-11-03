//! 主命令在 `RTSQL_PROFILING` 打开时返回查询结果或既有 SQL 错误，并在进入
//! 执行阶段后向 stderr 输出旧三段计时表。
//!
//! 每个子进程使用独立 TempDir 作为工作目录和 `RTSQL_HOME`。准备步骤移除
//! `RTSQL_PROFILING`；观察步骤显式设置或移除该变量。

#![cfg(feature = "cli")]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use tempfile::TempDir;

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn fixture() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("db")).expect("create db dir");
    dir
}

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
                    let _ = child.wait();
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

fn run_cli(dir: &Path, args: &[&str], profiling: Option<&str>) -> CliOutput {
    wait_cli(spawn_cli(dir, args, profiling))
}

fn assert_ok(out: &CliOutput, what: &str) {
    assert_eq!(
        out.code,
        Some(0),
        "{what} failed: stdout={} stderr={}",
        out.stdout,
        out.stderr
    );
}

fn prepare_table(dir: &Path, secondary_index: bool) {
    assert_ok(&run_cli(dir, &["new", "app"], None), "rtsql new app");
    assert_ok(
        &run_cli(
            dir,
            &["app", "CREATE TABLE t (id INT PRIMARY KEY, b INT)"],
            None,
        ),
        "create table",
    );
    assert_ok(
        &run_cli(dir, &["app", "INSERT INTO t VALUES (1, 10)"], None),
        "insert seed row",
    );
    if secondary_index {
        assert_ok(
            &run_cli(dir, &["app", "CREATE INDEX idx_b ON t(b)"], None),
            "create secondary index",
        );
    }
}

fn observe(dir: &Path, sql: &str, profiling: Option<&str>) -> CliOutput {
    run_cli(dir, &["--format", "json", "app", sql], profiling)
}

fn assert_rows_contain(stdout: &str, values: &[&str]) {
    let rows = stdout
        .split("\"rows\"")
        .nth(1)
        .unwrap_or_else(|| panic!("stdout has no rows field: {stdout}"));
    for value in values {
        assert!(
            rows.contains(value),
            "rows missing {value}: stdout={stdout}"
        );
    }
}

fn assert_contains_all(stderr: &str, needles: &[&str]) {
    for needle in needles {
        assert!(stderr.contains(needle), "stderr missing {needle}: {stderr}");
    }
}

fn assert_omits(text: &str, needle: &str) {
    assert!(!text.contains(needle), "unexpected {needle}: {text}");
}

fn count_marker(stderr: &str) -> usize {
    stderr.matches("Time (µs)").count()
}

fn tables_in_order(stderr: &str) -> Vec<&str> {
    let marker = "Time (µs)";
    let mut starts = Vec::new();
    let mut offset = 0;
    let mut rest = stderr;
    while let Some(found) = rest.find(marker) {
        starts.push(offset + found);
        let next = found + marker.len();
        offset += next;
        rest = &rest[next..];
    }
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(stderr.len());
            &stderr[*start..end]
        })
        .collect()
}

#[test]
fn main_command_pk_scan_returns_row_and_legacy_table() {
    let dir = fixture();
    prepare_table(dir.path(), false);
    let out = observe(dir.path(), "SELECT * FROM t WHERE id = 1", Some("1"));
    assert_ok(&out, "primary key scan");
    assert_rows_contain(&out.stdout, &["1", "10"]);
    assert_contains_all(
        &out.stderr,
        &[
            "Time (µs)",
            "parse",
            "plan",
            "execute",
            "Total",
            "index_manager_search",
        ],
    );
    assert_omits(&out.stderr, "index_manager_search_all");
    assert_omits(&out.stderr, "diagnostics ");
    assert_omits(&out.stderr, "panicked");
}

#[test]
fn main_command_secondary_scan_returns_row_and_legacy_table() {
    let dir = fixture();
    prepare_table(dir.path(), true);
    let out = observe(dir.path(), "SELECT * FROM t WHERE b = 10", Some("1"));
    assert_ok(&out, "secondary index scan");
    assert_rows_contain(&out.stdout, &["1", "10"]);
    assert_contains_all(
        &out.stderr,
        &[
            "Time (µs)",
            "parse",
            "plan",
            "execute",
            "index_manager_search_all",
        ],
    );
}

#[test]
fn main_command_count_without_index_lookup_prints_legacy_table() {
    let dir = fixture();
    prepare_table(dir.path(), false);
    let out = observe(dir.path(), "SELECT COUNT(*) FROM t", Some("1"));
    assert_ok(&out, "count");
    assert_rows_contain(&out.stdout, &["1"]);
    assert_contains_all(
        &out.stderr,
        &["Time (µs)", "parse", "plan", "execute", "Total"],
    );
}

#[test]
fn main_command_without_profiling_omits_legacy_table() {
    let dir = fixture();
    prepare_table(dir.path(), false);
    let out = observe(dir.path(), "SELECT * FROM t WHERE id = 1", None);
    assert_ok(&out, "primary key scan without profiling");
    assert_rows_contain(&out.stdout, &["1", "10"]);
    assert_omits(&out.stderr, "Time (µs)");
}

#[test]
fn main_command_parse_error_omits_legacy_table() {
    let dir = fixture();
    prepare_table(dir.path(), false);
    let out = observe(dir.path(), "SELEC 1", Some("1"));
    assert_eq!(out.code, Some(3), "stderr={}", out.stderr);
    assert!(out.stderr.contains("Parse error:"), "stderr={}", out.stderr);
    assert_omits(&out.stderr, "sqlstate");
    assert_omits(&out.stderr, "Time (µs)");
}

#[test]
fn main_command_missing_table_omits_legacy_table() {
    let dir = fixture();
    prepare_table(dir.path(), false);
    let out = observe(dir.path(), "SELECT * FROM missing WHERE id = 1", Some("1"));
    assert_eq!(out.code, Some(3), "stderr={}", out.stderr);
    assert_contains_all(&out.stderr, &["Table 'missing' not found", "42P01"]);
    assert_omits(&out.stderr, "Time (µs)");
}

#[test]
fn main_command_duplicate_key_keeps_error_and_prints_table() {
    let dir = fixture();
    prepare_table(dir.path(), false);
    let out = observe(dir.path(), "INSERT INTO t VALUES (1, 10)", Some("1"));
    assert_eq!(out.code, Some(3), "stderr={}", out.stderr);
    assert!(out.stdout.trim().is_empty(), "stdout={}", out.stdout);
    assert_contains_all(
        &out.stderr,
        &["Execution error: Duplicate key", "Time (µs)"],
    );
    let error_line = out
        .stderr
        .lines()
        .find(|line| line.contains("\"error\""))
        .unwrap_or_else(|| panic!("stderr has no error line: {}", out.stderr));
    assert_omits(error_line, "sqlstate");
}

#[test]
fn main_command_transaction_pk_scan_prints_one_table() {
    let dir = fixture();
    prepare_table(dir.path(), false);
    let out = observe(
        dir.path(),
        "BEGIN; SELECT * FROM t WHERE id = 1; COMMIT",
        Some("1"),
    );
    assert_ok(&out, "transaction primary key scan");
    assert_rows_contain(&out.stdout, &["1"]);
    assert_eq!(count_marker(&out.stderr), 1, "stderr={}", out.stderr);
    assert_contains_all(
        &out.stderr,
        &["parse", "plan", "execute", "index_manager_search"],
    );
    assert_omits(&out.stderr, "index_manager_search_all");
    assert_omits(
        &out.stderr,
        "uncommitted transaction was rolled back at exit",
    );
}

#[test]
fn main_command_two_statements_keep_separate_tables() {
    let dir = fixture();
    prepare_table(dir.path(), false);
    let out = observe(
        dir.path(),
        "SELECT * FROM t WHERE id = 1; SELECT COUNT(*) FROM t",
        Some("1"),
    );
    assert_ok(&out, "two statements");
    assert_eq!(count_marker(&out.stderr), 2, "stderr={}", out.stderr);
    let tables = tables_in_order(&out.stderr);
    assert_eq!(tables.len(), 2, "stderr={}", out.stderr);
    for table in &tables {
        assert_contains_all(table, &["parse", "plan", "execute"]);
    }
    assert_omits(tables[1], "index_manager_search");
}

#[test]
fn main_command_attach_only_omits_legacy_table() {
    let dir = fixture();
    assert_ok(&run_cli(dir.path(), &["new", "app"], None), "rtsql new app");
    assert_ok(
        &run_cli(dir.path(), &["new", "other"], None),
        "rtsql new other",
    );
    let out = observe(dir.path(), "ATTACH 'other' AS o", Some("1"));
    assert_ok(&out, "attach");
    assert_omits(&out.stderr, "Time (µs)");
}
