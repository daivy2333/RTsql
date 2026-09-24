//! MS13 Iteration 001 T8：GROUP BY 表达式/别名/位置 + 混合投影（change
//! 2026-09-23-ms13-analytics-functions）
//!
//! 验收域：group-by-expression delta spec（R1 三形态分桶等价 / R2 匹配
//! 失败与既有约束保持）。CLI e2e（真二进制）承载表头 + 行形状断言——
//! 包装路径的表头/行对齐是核心见证面。
//!
//! 多组行序经 HashMap 分桶非确定：凡多组断言均带 ORDER BY（或测试内
//! 排序后比较集合）。

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

/// 建库 + 事件表：dept 'a'×2（同日）、'b'×1（次日）。
fn setup_group_db(dir: &Path) {
    new_db(dir, "g");
    let out = run_cli(
        dir,
        &[
            "g",
            "CREATE TABLE ev (id INT PRIMARY KEY, dept STRING, ts TIMESTAMP)",
        ],
    );
    assert_eq!(out.code, Some(0), "{out:?}");
    for sql in [
        "INSERT INTO ev VALUES (1, 'a', TIMESTAMP '2024-01-15 10:00:00')",
        "INSERT INTO ev VALUES (2, 'a', TIMESTAMP '2024-01-15 11:00:00')",
        "INSERT INTO ev VALUES (3, 'b', TIMESTAMP '2024-01-16 09:00:00')",
    ] {
        let out = run_cli(dir, &["g", sql]);
        assert_eq!(out.code, Some(0), "{sql}: {out:?}");
    }
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

// ---------------------------------------------------------------------------
// R1：GROUP BY 别名 / 表达式 / 位置三形态分桶等价
// ---------------------------------------------------------------------------

/// R1/S1-S3：别名 / 表达式文本 / 位置三形态结果（表头 + 行）逐字节一致，
/// `GROUP BY date_trunc('day', ts)` 分桶计数正确。
///
/// RED（实施前实测）：混合投影在 query.rs 既有混合拒绝面报
/// `Invalid aggregate argument: Expected column name`。
#[test]
fn group_by_alias_expression_position_equivalence() {
    let dir = TempDir::new().unwrap();
    setup_group_db(dir.path());

    let mut results = Vec::new();
    for gb in [
        "GROUP BY day",
        "GROUP BY date_trunc('day', ts)",
        "GROUP BY 1",
    ] {
        let sql =
            format!("SELECT date_trunc('day', ts) AS day, COUNT(*) FROM ev {gb} ORDER BY day");
        let out = run_cli(dir.path(), &["g", &sql]);
        results.push(parse_json(&out));
    }
    let (cols0, rows0) = &results[0];
    assert_eq!(
        cols0,
        &["day".to_string(), "count_star".to_string()],
        "表头应为 SELECT 名: {cols0:?}"
    );
    assert_eq!(
        rows0,
        &vec![
            vec![
                serde_json::json!("2024-01-15 00:00:00"),
                serde_json::json!(2)
            ],
            vec![
                serde_json::json!("2024-01-16 00:00:00"),
                serde_json::json!(1)
            ],
        ],
        "按截断日分桶计数（TIMESTAMP 截断到日）: {rows0:?}"
    );
    assert_eq!(&results[1], &results[0], "表达式文本形态应与别名逐字节一致");
    assert_eq!(&results[2], &results[0], "位置形态应与别名逐字节一致");
}

/// R1 交错序（聚合项在前）：`SELECT COUNT(*), dept ... GROUP BY dept` 经
/// 包装后表头与行形状对齐（修复既有「group 序直出 vs SELECT 序表头」错位）。
///
/// RED（实施前实测）：直出路径行序 [dept, count]，表头 [count_star, dept]。
#[test]
fn mixed_projection_interleaved_count_first_aligns_shape() {
    let dir = TempDir::new().unwrap();
    setup_group_db(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "g",
            "SELECT COUNT(*), dept FROM ev GROUP BY dept ORDER BY dept",
        ],
    );
    let (cols, rows) = parse_json(&out);
    assert_eq!(cols, ["count_star", "dept"], "表头按 SELECT 序: {cols:?}");
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!(2), serde_json::json!("a")],
            vec![serde_json::json!(1), serde_json::json!("b")],
        ],
        "行序与表头对齐: {rows:?}"
    );
}

/// R2 别名大小写不敏感：`GROUP BY DAY` 与 `GROUP BY day` 等价。
///
/// RED（实施前实测）：混合拒绝面报错。
#[test]
fn group_by_alias_case_insensitive() {
    let dir = TempDir::new().unwrap();
    setup_group_db(dir.path());

    let sql = "SELECT date_trunc('day', ts) AS day, COUNT(*) FROM ev GROUP BY DAY ORDER BY day";
    let out = run_cli(dir.path(), &["g", sql]);
    let (_, rows) = parse_json(&out);
    assert_eq!(
        rows,
        vec![
            vec![
                serde_json::json!("2024-01-15 00:00:00"),
                serde_json::json!(2)
            ],
            vec![
                serde_json::json!("2024-01-16 00:00:00"),
                serde_json::json!(1)
            ],
        ]
    );
}

// ---------------------------------------------------------------------------
// R2：匹配失败与既有约束保持
// ---------------------------------------------------------------------------

/// R2/S1 表达式分组键 NULL 归并：date_trunc 对 NULL ts 求值 NULL，
/// 归并为单一分组。
///
/// RED（实施前实测）：混合拒绝面报错。
#[test]
fn null_expression_key_merges_into_single_group() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "g");
    let out = run_cli(
        dir.path(),
        &["g", "CREATE TABLE n (id INT PRIMARY KEY, ts TIMESTAMP)"],
    );
    assert_eq!(out.code, Some(0), "{out:?}");
    for sql in [
        "INSERT INTO n VALUES (1, NULL)",
        "INSERT INTO n VALUES (2, TIMESTAMP '2024-01-15 10:00:00')",
        "INSERT INTO n VALUES (3, TIMESTAMP '2024-01-15 11:00:00')",
    ] {
        let out = run_cli(dir.path(), &["g", sql]);
        assert_eq!(out.code, Some(0), "{sql}: {out:?}");
    }

    let out = run_cli(
        dir.path(),
        &[
            "g",
            "SELECT date_trunc('day', ts) AS day, COUNT(*) FROM n GROUP BY day ORDER BY day",
        ],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(
        rows,
        vec![
            vec![
                serde_json::json!("2024-01-15 00:00:00"),
                serde_json::json!(2)
            ],
            vec![serde_json::json!(null), serde_json::json!(1)],
        ],
        "NULL 键归并单组（NULL 排序置尾）: {rows:?}"
    );
}

/// R2/S2 不匹配表达式显式错误（点名项文本，不静默回退）。
///
/// RED（实施前实测）：既有 `Invalid aggregate argument: Expected column
/// name`（不点名项文本）。
#[test]
fn unmatched_group_expression_named_error() {
    let dir = TempDir::new().unwrap();
    setup_group_db(dir.path());

    let out = run_cli(
        dir.path(),
        &["g", "SELECT COUNT(*) FROM ev GROUP BY upper(nonexistent)"],
    );
    assert_eq!(out.code, Some(3), "不匹配表达式应显式拒绝: {out:?}");
    // 回显包含语句文本，故断言「错误通道 + 点名项」组合串（回显不含）
    assert!(
        out.stderr
            .contains("Non-aggregated column 'upper(nonexistent)'"),
        "错误应经 NonAggregatedColumn 通道并点名项文本: {out:?}"
    );
}

/// R2/S3 既有列名分组零回归：纯列名「键在前聚合在后」形态表头/行/排序
/// 逐字节保持（直出路径零触碰）。
#[test]
fn pure_column_group_by_unchanged() {
    let dir = TempDir::new().unwrap();
    setup_group_db(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "g",
            "SELECT dept, COUNT(*) FROM ev GROUP BY dept ORDER BY dept",
        ],
    );
    let (cols, rows) = parse_json(&out);
    assert_eq!(cols, ["dept", "count_star"], "{cols:?}");
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!("a"), serde_json::json!(2)],
            vec![serde_json::json!("b"), serde_json::json!(1)],
        ]
    );
}

/// R2 直出边界锁定（design D12「SELECT 序键前聚合后且全纯列名 → 直出零
/// 变化」）：GROUP BY 序与 SELECT 键序交错的既有形态保持直出——行按
/// GROUP BY 序产出、表头按 SELECT 序（既有错位行为，非本 change 引入，
/// design R2 记载）。行集合（测试内排序）锁定。
#[test]
fn groupby_order_interleave_keeps_direct_path() {
    let dir = TempDir::new().unwrap();
    setup_group_db(dir.path());

    let out = run_cli(
        dir.path(),
        &["g", "SELECT id, dept, COUNT(*) FROM ev GROUP BY dept, id"],
    );
    let (cols, rows) = parse_json(&out);
    assert_eq!(
        cols,
        ["id", "dept", "count_star"],
        "表头按 SELECT 序: {cols:?}"
    );
    // 直出路径行 = group_key（GROUP BY 序）++ 聚合：[dept, id, count]；
    // HashMap 迭代序非确定 → 测试内按键排序后比较集合。
    let mut sorted: Vec<Vec<i64>> = rows
        .iter()
        .map(|r| {
            vec![
                r[0].as_str()
                    .map(|s| s.bytes().map(|b| b as i64).sum())
                    .unwrap_or(-1),
                r[1].as_i64().unwrap(),
                r[2].as_i64().unwrap(),
            ]
        })
        .collect();
    sorted.sort();
    assert_eq!(
        sorted,
        vec![
            vec![97, 1, 1], // 'a', id=1, count=1
            vec![97, 2, 1], // 'a', id=2, count=1
            vec![98, 3, 1], // 'b', id=3, count=1
        ],
        "直出行形状 [dept, id, count] 保持: {sorted:?}"
    );
}

/// R2 位置越界显式错误（既有错误面保持）。
#[test]
fn group_by_position_out_of_range_error() {
    let dir = TempDir::new().unwrap();
    setup_group_db(dir.path());

    let out = run_cli(
        dir.path(),
        &["g", "SELECT dept, COUNT(*) FROM ev GROUP BY 5"],
    );
    assert_eq!(out.code, Some(3), "越界位置应显式拒绝: {out:?}");
    assert!(
        out.stderr.contains("Non-aggregated"),
        "应经 NonAggregatedColumn 通道: {out:?}"
    );
}

/// R2 位置引用指向聚合项显式拒绝（键不得为聚合）。
#[test]
fn group_by_position_on_aggregate_rejected() {
    let dir = TempDir::new().unwrap();
    setup_group_db(dir.path());

    let out = run_cli(
        dir.path(),
        &["g", "SELECT dept, COUNT(*) FROM ev GROUP BY 2"],
    );
    assert_eq!(out.code, Some(3), "位置指向聚合项应显式拒绝: {out:?}");
    assert!(
        out.stderr.contains("Non-aggregated"),
        "应经 NonAggregatedColumn 通道: {out:?}"
    );
}

/// R2 混合投影 + HAVING 既有机制保持：HAVING 在包装之下按聚合行求值。
///
/// RED（实施前实测）：混合拒绝面报错。
#[test]
fn having_with_mixed_projection() {
    let dir = TempDir::new().unwrap();
    setup_group_db(dir.path());

    let out = run_cli(
        dir.path(),
        &["g", "SELECT date_trunc('day', ts) AS day, COUNT(*) FROM ev GROUP BY day HAVING COUNT(*) > 1 ORDER BY day"],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!("2024-01-15 00:00:00"),
            serde_json::json!(2)
        ]]
    );
}

/// R2 混合投影 + ORDER BY 别名：Sort 叠加于包装之上，按别名列排序生效。
///
/// RED（实施前实测）：混合拒绝面报错。
#[test]
fn order_by_alias_with_wrapped_projection() {
    let dir = TempDir::new().unwrap();
    setup_group_db(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "g",
            "SELECT date_trunc('day', ts) AS day, COUNT(*) FROM ev GROUP BY day ORDER BY day DESC",
        ],
    );
    let (_, rows) = parse_json(&out);
    assert_eq!(
        rows,
        vec![
            vec![
                serde_json::json!("2024-01-16 00:00:00"),
                serde_json::json!(1)
            ],
            vec![
                serde_json::json!("2024-01-15 00:00:00"),
                serde_json::json!(2)
            ],
        ],
        "ORDER BY day DESC: {rows:?}"
    );
}

/// R2 GROUP BY ALL 既有语义保持（非聚合列清单分组）。
#[test]
fn group_by_all_unchanged() {
    let dir = TempDir::new().unwrap();
    setup_group_db(dir.path());

    let out = run_cli(
        dir.path(),
        &[
            "g",
            "SELECT dept, COUNT(*) FROM ev GROUP BY ALL ORDER BY dept",
        ],
    );
    let (cols, rows) = parse_json(&out);
    assert_eq!(cols, ["dept", "count_star"], "{cols:?}");
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!("a"), serde_json::json!(2)],
            vec![serde_json::json!("b"), serde_json::json!(1)],
        ]
    );
}
