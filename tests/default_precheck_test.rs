//! change 2026-10-13-e2e-verification-and-defect-closure / Iteration 002 T3
//! （R3 / S9，design D6）：DEFAULT 序列化尺寸计划期预检矩阵。
//!
//! DEFAULT 字面量经 catalog 列行序列化后放不进单页列行时，计划期点名
//! 拒绝（错误正文含 DEFAULT 与列名、exit 3）、零副作用（schema 无表）、
//! 同名可重试；边界值（页容量内的大字面量）与全类型小字面量放行；
//! ALTER TABLE ADD COLUMN 同款预检。夹具模式对齐 tests/e2e_test.rs。

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

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
    let mut stdout_pipe = child.stdout.take().unwrap();
    let mut stderr_pipe = child.stderr.take().unwrap();
    let stdout_reader = std::thread::spawn(move || {
        let mut s = String::new();
        stdout_pipe.read_to_string(&mut s).unwrap();
        s
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut s = String::new();
        stderr_pipe.read_to_string(&mut s).unwrap();
        s
    });
    loop {
        match child.try_wait().expect("poll rtsql status") {
            Some(status) => {
                let stdout = stdout_reader.join().unwrap();
                let stderr = stderr_reader.join().unwrap();
                return CliOutput {
                    code: status.code(),
                    stdout,
                    stderr,
                };
            }
            None => {
                assert!(
                    Instant::now() < deadline,
                    "rtsql one-shot did not exit within 60s"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// 错误语义段（`; statement:` 回声之前）——防「语句回显含关键字」假阳性。
fn error_message(out: &CliOutput) -> String {
    let parsed: serde_json::Value =
        serde_json::from_str(&out.stderr).expect("stderr must be JSON error object");
    let error = parsed["error"].as_str().unwrap_or("").to_string();
    error
        .split("; statement:")
        .next()
        .unwrap_or(&error)
        .to_string()
}

/// 超限 STRING DEFAULT：计划期点名拒绝（DEFAULT + 列名）、exit 3、零副作用。
#[test]
fn oversized_default_rejected_with_named_message() {
    let dir = TempDir::new().unwrap();
    let out = run_cli(dir.path(), &["new", "p1"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    let big = "d".repeat(5000);
    let sql = format!("CREATE TABLE t (id INT, pad STRING DEFAULT '{big}')");
    let out = run_cli(dir.path(), &["p1", &sql]);
    assert_eq!(
        out.code,
        Some(3),
        "plan-stage rejection exit 3: {:?}",
        out.stderr
    );
    let message = error_message(&out);
    assert!(
        message.to_uppercase().contains("DEFAULT"),
        "message must name DEFAULT: {message}"
    );
    assert!(
        message.contains("pad"),
        "message must name the column: {message}"
    );

    // 零副作用：schema 无表、同名可重试。
    let out = run_cli(dir.path(), &["schema", "p1"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert!(!out.stdout.contains("\"t\""), "no table: {}", out.stdout);
    let out = run_cli(dir.path(), &["p1", "CREATE TABLE t (id INT)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["p1", "INSERT INTO t VALUES (1)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
}

/// 边界值：页容量内的大字面量（3900B）放行且 DEFAULT 生效。
#[test]
fn near_limit_default_accepted_and_applied() {
    let dir = TempDir::new().unwrap();
    let out = run_cli(dir.path(), &["new", "p2"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    let big = "a".repeat(3900);
    let sql = format!("CREATE TABLE t (id INT, pad STRING DEFAULT '{big}')");
    let out = run_cli(dir.path(), &["p2", &sql]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["p2", "INSERT INTO t VALUES (1, DEFAULT)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["p2", "SELECT pad FROM t WHERE id = 1"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(parsed["rows"][0][0].as_str().unwrap(), big);
}

/// 全类型小字面量 DEFAULT 放行（Int / Float / Bool / Date / Timestamp）。
#[test]
fn small_defaults_all_types_accepted() {
    let dir = TempDir::new().unwrap();
    let out = run_cli(dir.path(), &["new", "p3"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    let sql = "CREATE TABLE t (\
        id INT PRIMARY KEY, \
        f FLOAT DEFAULT 1.5, \
        b BOOL DEFAULT true, \
        d DATE DEFAULT '2024-01-01', \
        ts TIMESTAMP DEFAULT '2024-01-01 08:00:00')";
    let out = run_cli(dir.path(), &["p3", sql]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &[
            "p3",
            "INSERT INTO t VALUES (1, DEFAULT, DEFAULT, DEFAULT, DEFAULT)",
        ],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &["p3", "SELECT f, b, d, ts FROM t WHERE id = 1"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(parsed["rows"][0][0].as_f64().unwrap(), 1.5);
    assert!(parsed["rows"][0][1].as_bool().unwrap());
    assert_eq!(parsed["rows"][0][2].as_str().unwrap(), "2024-01-01");
}

/// ALTER TABLE ADD COLUMN 同款预检：超限 DEFAULT 计划期点名拒绝、表不变。
#[test]
fn alter_add_oversized_default_rejected() {
    let dir = TempDir::new().unwrap();
    let out = run_cli(dir.path(), &["new", "p4"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["p4", "CREATE TABLE t (id INT PRIMARY KEY)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    let big = "d".repeat(5000);
    let sql = format!("ALTER TABLE t ADD COLUMN pad STRING DEFAULT '{big}'");
    let out = run_cli(dir.path(), &["p4", &sql]);
    assert_eq!(
        out.code,
        Some(3),
        "plan-stage rejection exit 3: {:?}",
        out.stderr
    );
    let message = error_message(&out);
    assert!(
        message.to_uppercase().contains("DEFAULT"),
        "message must name DEFAULT: {message}"
    );
    assert!(
        message.contains("pad"),
        "message must name the column: {message}"
    );

    // 表结构不变：schema 仍只有原列，可正常读写。
    let out = run_cli(dir.path(), &["schema", "p4"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("\"id\""),
        "original column stays: {}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("\"pad\""),
        "no phantom column: {}",
        out.stdout
    );
    let out = run_cli(dir.path(), &["p4", "INSERT INTO t VALUES (1)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
}

/// ALTER TABLE ADD COLUMN 边界值：页容量内 DEFAULT 放行并回填存量行。
#[test]
fn alter_add_near_limit_default_accepted() {
    let dir = TempDir::new().unwrap();
    let out = run_cli(dir.path(), &["new", "p5"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["p5", "CREATE TABLE t (id INT PRIMARY KEY)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["p5", "INSERT INTO t VALUES (1)"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);

    let big = "a".repeat(3900);
    let sql = format!("ALTER TABLE t ADD COLUMN pad STRING DEFAULT '{big}'");
    let out = run_cli(dir.path(), &["p5", &sql]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let out = run_cli(dir.path(), &["p5", "SELECT pad FROM t WHERE id = 1"]);
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(parsed["rows"][0][0].as_str().unwrap(), big);
}
