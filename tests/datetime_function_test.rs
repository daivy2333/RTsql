//! MS13 Iteration 001：日期函数族 + INTERVAL 算术（change
//! 2026-09-23-ms13-analytics-functions）
//!
//! 验收域：datetime-functions delta spec（R1 函数族 / R2 INTERVAL / R3
//! datediff / R4 零回归）。本文件 T6 段承载函数族与 datediff；T7 段承载
//! INTERVAL；T8 的分桶 e2e 在 tests/group_by_expr_test.rs。
//!
//! 通过真二进制（`CARGO_BIN_EXE_rtsql`）e2e 验证：每个 spawn 以独立
//! TempDir 为 CWD 并把 `RTSQL_HOME` 指向该 TempDir（并行安全）；stdout/
//! stderr 恒为管道（非 TTY，默认 JSON 渲染）。

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[derive(Debug)]
struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 运行 rtsql 二进制并等待退出；60s 未退出则 kill 并 panic
/// （one-shot CLI 必须自行退出，挂起即为行为错误）。
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

/// 创建新库（断言成功），供各用例起步。
fn new_db(dir: &Path, name: &str) {
    let out = run_cli(dir, &["new", name]);
    assert_eq!(
        out.code,
        Some(0),
        "rtsql new {name} should succeed: {out:?}"
    );
}

/// 建库 + 事件表（id/d/ts）+ 单行锚点数据（闰日 + 微秒时刻）。
fn setup_event_db(dir: &Path) {
    new_db(dir, "ev");
    let out = run_cli(
        dir,
        &[
            "ev",
            "CREATE TABLE ev (id INT PRIMARY KEY, d DATE, ts TIMESTAMP)",
        ],
    );
    assert_eq!(out.code, Some(0), "建表应成功: {out:?}");
    let out = run_cli(
        dir,
        &[
            "ev",
            "INSERT INTO ev VALUES (1, DATE '2024-02-29', TIMESTAMP '2024-01-15 10:30:45.123456')",
        ],
    );
    assert_eq!(out.code, Some(0), "锚点行写入应成功: {out:?}");
}

/// 解析 one-shot JSON 输出为 (columns, rows)。
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

// ---------------------------------------------------------------------------
// T6：日期函数族（datetime-functions R1）
// ---------------------------------------------------------------------------

/// R1/S1 抽取函数正确：year/month/day(Date)、hour/minute/second(Timestamp)、
/// date(Timestamp) 截断。
///
/// RED（实施前实测）：函数名未注册，SELECT 位置报既有
/// `Unsupported statement type` exit 3。
#[test]
fn extract_functions_on_date_and_timestamp() {
    let dir = TempDir::new().unwrap();
    setup_event_db(dir.path());

    let out = run_cli(
        dir.path(),
        &["ev", "SELECT year(d), month(d), day(d) FROM ev"],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!(2024),
            serde_json::json!(2),
            serde_json::json!(29)
        ]]
    );

    let out = run_cli(
        dir.path(),
        &["ev", "SELECT hour(ts), minute(ts), second(ts) FROM ev"],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!(10),
            serde_json::json!(30),
            serde_json::json!(45)
        ]]
    );

    let out = run_cli(dir.path(), &["ev", "SELECT date(ts) FROM ev"]);
    let (_, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!("2024-01-15")]]);
}

/// R1/S1 date() 双语义：Date 恒等；Timestamp 截断到日。
#[test]
fn date_function_identity_and_truncate() {
    let dir = TempDir::new().unwrap();
    setup_event_db(dir.path());

    let out = run_cli(dir.path(), &["ev", "SELECT date(d) FROM ev"]);
    let (_, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!("2024-02-29")]]);

    let out = run_cli(dir.path(), &["ev", "SELECT date(ts) FROM ev"]);
    let (_, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!("2024-01-15")]]);
}

/// R1/S2 date_trunc 截断正确：day/hour/month/year（Timestamp 全单位）。
#[test]
fn date_trunc_units_on_timestamp() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "ev");
    let out = run_cli(
        dir.path(),
        &["ev", "CREATE TABLE ev (id INT PRIMARY KEY, ts TIMESTAMP)"],
    );
    assert_eq!(out.code, Some(0), "{out:?}");
    let out = run_cli(
        dir.path(),
        &[
            "ev",
            "INSERT INTO ev VALUES (1, TIMESTAMP '2024-01-15 10:30:45')",
        ],
    );
    assert_eq!(out.code, Some(0), "{out:?}");

    for (unit, expected) in [
        ("day", "2024-01-15 00:00:00"),
        ("hour", "2024-01-15 10:00:00"),
        ("month", "2024-01-01 00:00:00"),
        ("year", "2024-01-01 00:00:00"),
    ] {
        let sql = format!("SELECT date_trunc('{unit}', ts) FROM ev");
        let out = run_cli(dir.path(), &["ev", &sql]);
        let (_, rows) = parse_json(&out);
        assert_eq!(
            rows,
            vec![vec![serde_json::json!(expected)]],
            "date_trunc('{unit}', ts)"
        );
    }
}

/// R1 date_trunc Date 仅支持 year/month/day（DA9）。
#[test]
fn date_trunc_on_date_supports_calendar_units() {
    let dir = TempDir::new().unwrap();
    setup_event_db(dir.path());

    for (unit, expected) in [
        ("year", "2024-01-01"),
        ("month", "2024-02-01"),
        ("day", "2024-02-29"),
    ] {
        let sql = format!("SELECT date_trunc('{unit}', d) FROM ev");
        let out = run_cli(dir.path(), &["ev", &sql]);
        let (_, rows) = parse_json(&out);
        assert_eq!(
            rows,
            vec![vec![serde_json::json!(expected)]],
            "date_trunc('{unit}', d)"
        );
    }
}

/// R1/S4 Date × 时间单位显式拒绝（Date 仅 year/month/day，DA9）。
///
/// RED（实施前实测）：plan 期 `Unsupported statement type`——断言「非
/// Unsupported 的运行时错误」在 RED 下失败。
#[test]
fn date_trunc_date_with_time_unit_rejected() {
    let dir = TempDir::new().unwrap();
    setup_event_db(dir.path());

    let out = run_cli(dir.path(), &["ev", "SELECT date_trunc('hour', d) FROM ev"]);
    assert_eq!(out.code, Some(3), "Date × hour 应运行时类型错误: {out:?}");
    assert!(
        !out.stderr.contains("Unsupported"),
        "应为运行时错误而非计划期拒绝: {out:?}"
    );
}

/// R1/S4 单位错误点名：date_trunc('week', ts) 显式拒绝且文案点名单位。
///
/// RED（实施前实测）：plan 期 `Unsupported statement type`（不含 week）。
#[test]
fn date_trunc_unknown_unit_named_error() {
    let dir = TempDir::new().unwrap();
    setup_event_db(dir.path());

    let out = run_cli(dir.path(), &["ev", "SELECT date_trunc('week', ts) FROM ev"]);
    assert_eq!(out.code, Some(3), "未知单位应显式拒绝: {out:?}");
    assert!(
        !out.stderr.contains("Unsupported"),
        "应为运行时单位错误而非计划期拒绝: {out:?}"
    );
    assert!(
        out.stderr.to_lowercase().contains("week"),
        "错误文案应点名未知单位: {out:?}"
    );
}

/// R1/S4 严格类型：String/Int 参数运行期类型错误（无隐式转换）。
///
/// RED（实施前实测）：plan 期 `Unsupported statement type`。
#[test]
fn extract_functions_strict_type_errors() {
    let dir = TempDir::new().unwrap();
    setup_event_db(dir.path());

    for sql in ["SELECT year('abc') FROM ev", "SELECT hour(123) FROM ev"] {
        let out = run_cli(dir.path(), &["ev", sql]);
        assert_eq!(out.code, Some(3), "{sql} 应运行时类型错误: {out:?}");
        assert!(
            !out.stderr.contains("Unsupported"),
            "{sql} 应为运行时类型错误而非计划期拒绝: {out:?}"
        );
    }
}

/// R1/S5 NULL 短路：任一参数 NULL → NULL（D3 求值序沿用）。
///
/// RED（实施前实测）：plan 期 `Unsupported statement type`。
#[test]
fn null_argument_yields_null() {
    let dir = TempDir::new().unwrap();
    setup_event_db(dir.path());

    let out = run_cli(dir.path(), &["ev", "SELECT year(NULL) FROM ev"]);
    let (_, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!(null)]]);

    let out = run_cli(
        dir.path(),
        &["ev", "SELECT date_trunc('day', NULL) FROM ev"],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!(null)]]);
}

/// R1/S3 now() 墙钟：TIMESTAMP 形态、两次执行非递减（运行期求值）。
///
/// RED（实施前实测）：plan 期 `Unsupported statement type`。
#[test]
fn now_returns_current_timestamp_nondecreasing() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "ev");
    let out = run_cli(dir.path(), &["ev", "CREATE TABLE ev (id INT PRIMARY KEY)"]);
    assert_eq!(out.code, Some(0), "{out:?}");
    let out = run_cli(dir.path(), &["ev", "INSERT INTO ev VALUES (1)"]);
    assert_eq!(out.code, Some(0), "{out:?}");

    let out = run_cli(dir.path(), &["ev", "SELECT now() FROM ev"]);
    let (columns, rows) = parse_json(&out);
    let first = rows[0][0].as_str().expect("now() 应为 TIMESTAMP 字符串");
    // DA5 形态：`YYYY-MM-DD HH:MM:SS`，微秒≠0 追加 6 位小数（墙钟通常非零）
    assert!(
        first.len() == 19 || (first.len() == 26 && first.as_bytes()[19] == b'.'),
        "now() 应为 DA5 TIMESTAMP 文本形态: {first}"
    );
    assert!(columns.len() == 1);

    // 两次执行非递减（秒级精度比较）
    let out = run_cli(dir.path(), &["ev", "SELECT now() FROM ev"]);
    let (_, rows2) = parse_json(&out);
    let second = rows2[0][0].as_str().expect("now() 应为 TIMESTAMP 字符串");
    assert!(
        second[..19] >= first[..19],
        "now() 应非递减: {first} then {second}"
    );
}

// ---------------------------------------------------------------------------
// T6：datediff（datetime-functions R3）
// ---------------------------------------------------------------------------

/// 建库 + 单行占位表（常量表达式查询需 FROM；no-FROM 属 Iteration 002）。
fn setup_single_row_db(dir: &Path) {
    new_db(dir, "one");
    let out = run_cli(dir, &["one", "CREATE TABLE one (id INT PRIMARY KEY)"]);
    assert_eq!(out.code, Some(0), "{out:?}");
    let out = run_cli(dir, &["one", "INSERT INTO one VALUES (1)"]);
    assert_eq!(out.code, Some(0), "{out:?}");
}

/// R3/S1 日差与月差：30 与 1（3-14 减 1-15 不足 2 整月，截断）。
///
/// RED（实施前实测）：plan 期 `Unsupported statement type`。
#[test]
fn datediff_day_and_month() {
    let dir = TempDir::new().unwrap();
    setup_single_row_db(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "one",
            "SELECT datediff('day', DATE '2024-01-01', DATE '2024-01-31') FROM one",
        ],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!(30)]]);

    let out = run_cli(
        dir.path(),
        &[
            "one",
            "SELECT datediff('month', DATE '2024-01-15', DATE '2024-03-14') FROM one",
        ],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!(1)]]);
}

/// R3 负向截断（DA8「b−a 整单位数、按单位截断」的对称方向）：
/// 2024-03-14 → 2024-01-15 为 -1 整月（朝零截断，PostgreSQL age 语义方向）。
#[test]
fn datediff_month_negative_truncates_toward_zero() {
    let dir = TempDir::new().unwrap();
    setup_single_row_db(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "one",
            "SELECT datediff('month', DATE '2024-03-14', DATE '2024-01-15') FROM one",
        ],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!(-1)]]);
}

/// R3 同族约束：跨族（Date × Timestamp）运行期类型错误。
///
/// RED（实施前实测）：plan 期 `Unsupported statement type`。
#[test]
fn datediff_cross_family_rejected() {
    let dir = TempDir::new().unwrap();
    setup_single_row_db(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "one",
            "SELECT datediff('day', DATE '2024-01-01', TIMESTAMP '2024-01-31 10:00:00') FROM one",
        ],
    );
    assert_eq!(out.code, Some(3), "跨族 datediff 应显式拒绝: {out:?}");
    assert!(
        !out.stderr.contains("Unsupported"),
        "应为运行时类型错误而非计划期拒绝: {out:?}"
    );
}

// ---------------------------------------------------------------------------
// T7：INTERVAL 表达式算术（datetime-functions R2）
// ---------------------------------------------------------------------------

/// 建库 + 区间算术锚点表（d = 2024-01-15，ts = 2024-01-15 10:00:00）。
fn setup_interval_db(dir: &Path) {
    new_db(dir, "iv");
    let out = run_cli(
        dir,
        &[
            "iv",
            "CREATE TABLE iv (id INT PRIMARY KEY, d DATE, ts TIMESTAMP)",
        ],
    );
    assert_eq!(out.code, Some(0), "{out:?}");
    let out = run_cli(
        dir,
        &[
            "iv",
            "INSERT INTO iv VALUES (1, DATE '2024-01-15', TIMESTAMP '2024-01-15 10:00:00')",
        ],
    );
    assert_eq!(out.code, Some(0), "{out:?}");
}

/// R2/S1 日/时级算术：`d + INTERVAL '1 day'`、`ts - INTERVAL '90 minutes'`。
///
/// RED（实施前实测）：`Expr::Interval` 腿落入 UnsupportedExpression 兜底
///（plan 期 `Unsupported expression type`）。
#[test]
fn date_and_ts_interval_day_minute_arithmetic() {
    let dir = TempDir::new().unwrap();
    setup_interval_db(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "iv",
            "SELECT d + INTERVAL '1 day', d - INTERVAL '1 day' FROM iv",
        ],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!("2024-01-16"),
            serde_json::json!("2024-01-14")
        ]]
    );

    let out = run_cli(
        dir.path(),
        &["iv", "SELECT ts - INTERVAL '90 minutes' FROM iv"],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!("2024-01-15 08:30:00")]]);
}

/// R2/S2 月末锚定：`2024-01-31 + 1 month` → `2024-02-29`（闰年截月末，
/// PostgreSQL 语义）。
#[test]
fn interval_month_end_anchoring() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "iv");
    let out = run_cli(
        dir.path(),
        &["iv", "CREATE TABLE iv (id INT PRIMARY KEY, d DATE)"],
    );
    assert_eq!(out.code, Some(0), "{out:?}");
    let out = run_cli(
        dir.path(),
        &["iv", "INSERT INTO iv VALUES (1, DATE '2024-01-31')"],
    );
    assert_eq!(out.code, Some(0), "{out:?}");

    let out = run_cli(dir.path(), &["iv", "SELECT d + INTERVAL '1 month' FROM iv"]);
    let (_, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!("2024-02-29")]]);
}

/// R2 负区间与年算术锚定：`2024-03-31 - 1 month` → `2024-02-29`（负向同日
/// 锚定截月末）；`2024-02-29 + 1 year` → `2025-02-28`（闰日锚定截月末）。
#[test]
fn interval_negative_and_year_anchoring() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "iv");
    let out = run_cli(
        dir.path(),
        &["iv", "CREATE TABLE iv (id INT PRIMARY KEY, d DATE)"],
    );
    assert_eq!(out.code, Some(0), "{out:?}");
    let out = run_cli(
        dir.path(),
        &[
            "iv",
            "INSERT INTO iv VALUES (1, DATE '2024-03-31'), (2, DATE '2024-02-29')",
        ],
    );
    assert_eq!(out.code, Some(0), "{out:?}");

    let out = run_cli(
        dir.path(),
        &["iv", "SELECT d - INTERVAL '1 month' FROM iv WHERE id = 1"],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!("2024-02-29")]]);

    let out = run_cli(
        dir.path(),
        &["iv", "SELECT d + INTERVAL '1 year' FROM iv WHERE id = 2"],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!("2025-02-28")]]);
}

/// R2 两形态等价：`INTERVAL 1 DAY`（数值+字段）与 `INTERVAL '1 day'`
///（字符串内嵌单位）结果一致。
#[test]
fn interval_field_form_equivalent_to_string_form() {
    let dir = TempDir::new().unwrap();
    setup_interval_db(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "iv",
            "SELECT d + INTERVAL 1 DAY, d + INTERVAL '1 day' FROM iv",
        ],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!("2024-01-16"),
            serde_json::json!("2024-01-16")
        ]]
    );
}

/// R2 WHERE 腿：`d + INTERVAL '1 day'` 参与谓词比较可达。
#[test]
fn interval_arithmetic_in_where_predicate() {
    let dir = TempDir::new().unwrap();
    setup_interval_db(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "iv",
            "SELECT id FROM iv WHERE d + INTERVAL '1 day' = DATE '2024-01-16'",
        ],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(rows, vec![vec![serde_json::json!(1)]]);
}

/// R2/S3 独立投影项拒绝：`SELECT INTERVAL '1 day'` 点名拒绝（不可存储/
/// 不可独立求值）。
///
/// RED（实施前实测）：ast.rs 放行门前为 `Unsupported statement type`。
#[test]
fn standalone_interval_projection_rejected() {
    let dir = TempDir::new().unwrap();
    setup_interval_db(dir.path());

    let out = run_cli(dir.path(), &["iv", "SELECT INTERVAL '1 day' FROM iv"]);
    assert_eq!(out.code, Some(3), "独立 INTERVAL 投影项应显式拒绝: {out:?}");
    assert!(
        !out.stderr.contains("Unsupported"),
        "应点名拒绝而非既有兜底文案: {out:?}"
    );
}

/// R2 拒绝面：INTERVAL 在左（`INTERVAL + d`）、乘除腿（`d * INTERVAL`）、
/// 多段字符串（`'1 day 2 hours'`）、未知字段单位（`INTERVAL 1 WEEK`）。
#[test]
fn interval_rejection_matrix() {
    let dir = TempDir::new().unwrap();
    setup_interval_db(dir.path());

    for sql in [
        "SELECT INTERVAL '1 day' + d FROM iv",
        "SELECT d * INTERVAL '2 day' FROM iv",
        "SELECT d + INTERVAL '1 day 2 hours' FROM iv",
        "SELECT d + INTERVAL 1 WEEK FROM iv",
    ] {
        let out = run_cli(dir.path(), &["iv", sql]);
        assert_eq!(out.code, Some(3), "{sql} 应显式拒绝: {out:?}");
        assert!(
            !out.stderr.contains("Unsupported"),
            "{sql} 应点名拒绝而非既有兜底文案: {out:?}"
        );
    }
}
