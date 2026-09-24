//! MS13 Iteration 000：DATE/TIMESTAMP 类型底座全链（change
//! 2026-09-23-ms13-analytics-functions）
//!
//! 验收域：datetime-type-system delta spec（7 Requirement）。分组：
//! DDL 映射（T3）→ 类型字面量与写入强制解析（T4）→ 比较/排序/PK 路由/
//! CAST/渲染导入导出收口（T5）。
//!
//! 通过真二进制（`CARGO_BIN_EXE_rtsql`）e2e 验证：每个 spawn 以独立
//! TempDir 为 CWD 并把 `RTSQL_HOME` 指向该 TempDir（并行安全）；stdout/
//! stderr 恒为管道（非 TTY，默认 JSON 渲染）。

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
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

/// 启动 rtsql 二进制且 stdin 为管道（`restore <db> -` 用例向其写入输入）。
fn spawn_cli_stdin(dir: &Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary with piped stdin")
}

/// 向 stdin 管道写入输入后等待退出。子进程若在读 stdin 前退出，写入以
/// EPIPE 失败属预期（退出码断言承载结果），不视为夹具错误。
fn run_cli_stdin(dir: &Path, args: &[&str], input: &str) -> CliOutput {
    let mut child = spawn_cli_stdin(dir, args);
    let mut stdin = child.stdin.take().unwrap();
    let _ = stdin.write_all(input.as_bytes());
    drop(stdin); // 关闭写端，子进程 read_to_string 见 EOF
    wait_cli(child)
}

/// 等待子进程退出并收集输出；60s 未退出则 kill 并 panic。
fn wait_cli(mut child: Child) -> CliOutput {
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

// ---------------------------------------------------------------------------
// T3：DDL 显式类型映射（datetime-type-system R2）
// ---------------------------------------------------------------------------

/// R2-S1 建表落列：DATE/TIMESTAMP 落真类型，schema 输出 DATE/TIMESTAMP。
///
/// RED（修复前实测）：`convert_data_type` 未知类型回退 String，
/// schema 输出 `"d" STRING`。
#[test]
fn ddl_maps_date_and_timestamp_columns() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "ev");

    let out = run_cli(
        dir.path(),
        &[
            "ev",
            "CREATE TABLE ev (id INT PRIMARY KEY, d DATE, ts TIMESTAMP)",
        ],
    );
    assert_eq!(out.code, Some(0), "建表应成功: {out:?}");

    let schema = run_cli(dir.path(), &["schema", "ev"]);
    assert_eq!(schema.code, Some(0), "schema 应成功: {schema:?}");
    assert!(
        schema.stdout.contains("\"d\" DATE"),
        "DATE 列应落真类型: {schema:?}"
    );
    assert!(
        schema.stdout.contains("\"ts\" TIMESTAMP"),
        "TIMESTAMP 列应落真类型: {schema:?}"
    );
    assert!(
        !schema.stdout.contains("\"d\" STRING"),
        "DATE 列不应再回退 STRING: {schema:?}"
    );
}

/// R2-S2 INTERVAL 列类型拒绝：显式拒绝（点名文案，exit 3）。
///
/// RED（修复前实测）：INTERVAL 落 String 兜底，建表成功 exit 0。
#[test]
fn interval_column_type_rejected() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "ev");

    let out = run_cli(dir.path(), &["ev", "CREATE TABLE t (i INTERVAL)"]);
    assert_eq!(out.code, Some(3), "INTERVAL 列类型应显式拒绝: {out:?}");
    assert!(
        out.stderr.to_lowercase().contains("interval"),
        "错误文案应点名 INTERVAL: {out:?}"
    );
}

// ---------------------------------------------------------------------------
// T4：类型字面量与写入边界强制解析（datetime-type-system R3）
// ---------------------------------------------------------------------------

/// R3-S1 类型字面量写入回读：`DATE '...'` 落库为 Date，SELECT 等值回读。
///
/// RED（修复前实测）：TypedString 落 `Unsupported statement type` exit 3。
#[test]
fn typed_date_literal_insert_and_readback() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "ev");

    run_cli(
        dir.path(),
        &["ev", "CREATE TABLE ev (id INT PRIMARY KEY, d DATE)"],
    );
    let out = run_cli(
        dir.path(),
        &["ev", "INSERT INTO ev VALUES (1, DATE '2024-01-15')"],
    );
    assert_eq!(out.code, Some(0), "类型字面量 INSERT 应成功: {out:?}");

    let sel = run_cli(dir.path(), &["ev", "SELECT d FROM ev"]);
    assert_eq!(sel.code, Some(0), "SELECT 应成功: {sel:?}");
    assert!(sel.stdout.contains("2024-01-15"), "应回读日期值: {sel:?}");
}

/// R3-S1b TIMESTAMP 类型字面量写入回读（微秒精度无损）。
///
/// RED（修复前实测）：同上，plan 期拒绝。
#[test]
fn typed_timestamp_literal_insert_and_readback() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "ev");

    run_cli(
        dir.path(),
        &["ev", "CREATE TABLE ev (id INT PRIMARY KEY, ts TIMESTAMP)"],
    );
    let out = run_cli(
        dir.path(),
        &[
            "ev",
            "INSERT INTO ev VALUES (1, TIMESTAMP '2024-01-15 10:30:00.123456')",
        ],
    );
    assert_eq!(out.code, Some(0), "TIMESTAMP 字面量 INSERT 应成功: {out:?}");

    let sel = run_cli(dir.path(), &["ev", "SELECT ts FROM ev"]);
    assert_eq!(sel.code, Some(0), "SELECT 应成功: {sel:?}");
    assert!(
        sel.stdout.contains("2024-01-15 10:30:00.123456"),
        "应回读微秒精度时间戳: {sel:?}"
    );
}

/// R3-S2 裸字符串强制解析：String 字面量写入 DATE 列强制解析为 Date
/// （与类型字面量等价）——经 `WHERE d = DATE '...'` Date=Date 等值过滤
/// 见证类型真落库（String 落库时该过滤为跨族空集）。
///
/// RED（修复前实测）：`DATE '...'` 在 WHERE 侧 plan 期拒绝，exit 3。
#[test]
fn bare_string_insert_coerced_to_date() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "ev");

    run_cli(
        dir.path(),
        &["ev", "CREATE TABLE ev (id INT PRIMARY KEY, d DATE)"],
    );
    let out = run_cli(
        dir.path(),
        &["ev", "INSERT INTO ev VALUES (1, '2024-01-15')"],
    );
    assert_eq!(out.code, Some(0), "裸字符串 INSERT 应成功: {out:?}");

    let sel = run_cli(
        dir.path(),
        &["ev", "SELECT id FROM ev WHERE d = DATE '2024-01-15'"],
    );
    assert_eq!(sel.code, Some(0), "Date=Date 等值过滤应成功: {sel:?}");
    assert!(
        sel.stdout.contains("\"rows\":[[1]]"),
        "裸字符串应已强制解析为 Date（等值过滤命中）: {sel:?}"
    );
}

/// R3-S3 非法日期字符串拒绝：显式解析错误且零行落库（零副作用）。
///
/// RED（修复前实测）：String 值直接落库 exit 0。
#[test]
fn invalid_date_string_rejected_with_zero_side_effect() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "ev");

    run_cli(
        dir.path(),
        &["ev", "CREATE TABLE ev (id INT PRIMARY KEY, d DATE)"],
    );

    for bad in ["'not-a-date'", "'2023-02-29'"] {
        let out = run_cli(
            dir.path(),
            &["ev", &format!("INSERT INTO ev VALUES (1, {bad})")],
        );
        assert_ne!(out.code, Some(0), "非法日期 {bad} 应拒绝: {out:?}");
    }

    let sel = run_cli(dir.path(), &["ev", "SELECT id FROM ev"]);
    assert_eq!(sel.code, Some(0));
    assert!(
        sel.stdout.contains("\"rows\":[]"),
        "拒绝必须零副作用（无行落库）: {sel:?}"
    );
}

/// R3-S4 恢复两态一致：写入进程退出（close→checkpoint）后重开等值回读。
///
/// one-shot CLI 每次调用都是独立 open/close；本用例显式跨进程验证
/// close→reopen 后 Date/Timestamp 值两态一致。
#[test]
fn values_survive_close_reopen() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "ev");

    run_cli(
        dir.path(),
        &[
            "ev",
            "CREATE TABLE ev (id INT PRIMARY KEY, d DATE, ts TIMESTAMP)",
        ],
    );
    let out = run_cli(
        dir.path(),
        &[
            "ev",
            "INSERT INTO ev VALUES (1, DATE '2024-02-29', TIMESTAMP '2024-01-15 10:30:00.123456')",
        ],
    );
    assert_eq!(out.code, Some(0), "写入应成功: {out:?}");

    let sel = run_cli(dir.path(), &["ev", "SELECT d, ts FROM ev"]);
    assert_eq!(sel.code, Some(0), "重开后 SELECT 应成功: {sel:?}");
    assert!(
        sel.stdout.contains("2024-02-29") && sel.stdout.contains("2024-01-15 10:30:00.123456"),
        "重开后应等值回读两类型值: {sel:?}"
    );
}

/// R3-S5 UPDATE SET 三形态：类型字面量 / 裸字符串强制 / 非法拒绝零副作用。
///
/// RED（修复前实测）：TypedString 落 UnsupportedValue exit 3。
#[test]
fn update_set_datetime_forms() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "ev");

    run_cli(
        dir.path(),
        &["ev", "CREATE TABLE ev (id INT PRIMARY KEY, d DATE)"],
    );
    run_cli(
        dir.path(),
        &["ev", "INSERT INTO ev VALUES (1, DATE '2024-01-15')"],
    );

    let up1 = run_cli(
        dir.path(),
        &["ev", "UPDATE ev SET d = DATE '2024-01-20' WHERE id = 1"],
    );
    assert_eq!(up1.code, Some(0), "SET 类型字面量应成功: {up1:?}");
    let sel = run_cli(dir.path(), &["ev", "SELECT d FROM ev"]);
    assert!(
        sel.stdout.contains("2024-01-20"),
        "应更新为类型字面量值: {sel:?}"
    );

    let up2 = run_cli(
        dir.path(),
        &["ev", "UPDATE ev SET d = '2024-01-21' WHERE id = 1"],
    );
    assert_eq!(up2.code, Some(0), "SET 裸字符串应强制解析成功: {up2:?}");
    let sel = run_cli(dir.path(), &["ev", "SELECT d FROM ev"]);
    assert!(
        sel.stdout.contains("2024-01-21"),
        "应更新为强制解析值: {sel:?}"
    );

    let up3 = run_cli(
        dir.path(),
        &["ev", "UPDATE ev SET d = 'not-a-date' WHERE id = 1"],
    );
    assert_ne!(up3.code, Some(0), "SET 非法日期应拒绝: {up3:?}");
    let sel = run_cli(dir.path(), &["ev", "SELECT d FROM ev"]);
    assert!(
        sel.stdout.contains("2024-01-21"),
        "拒绝必须零副作用（原值保持）: {sel:?}"
    );
}

/// R3-S6 WITH-FORM 算术解锁：BinaryOp 放行后 `SELECT id + 1` 可达。
///
/// RED（修复前实测）：BinaryOp 不在放行清单，`Unsupported statement type`
/// exit 3。
#[test]
fn select_arithmetic_expression_unlocked() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "ev");

    run_cli(dir.path(), &["ev", "CREATE TABLE t2 (id INT PRIMARY KEY)"]);
    run_cli(dir.path(), &["ev", "INSERT INTO t2 VALUES (1)"]);
    run_cli(dir.path(), &["ev", "INSERT INTO t2 VALUES (2)"]);

    let sel = run_cli(dir.path(), &["ev", "SELECT id + 1 FROM t2 ORDER BY id"]);
    assert_eq!(sel.code, Some(0), "算术表达式 SELECT 应可达: {sel:?}");
    assert!(
        sel.stdout.contains("[[2],[3]]"),
        "id + 1 应逐行求值输出 2/3: {sel:?}"
    );
}

// ---------------------------------------------------------------------------
// T5：比较/排序/PK 路由/CAST/渲染/导入导出收口（datetime-type-system R4/R5/R6）
// ---------------------------------------------------------------------------

/// 日期域公共夹具：表 ev(id INT PK, d DATE, ts TIMESTAMP) 含三行日期数据。
fn fixture_datetime_table(dir: &TempDir) {
    new_db(dir.path(), "ev");
    run_cli(
        dir.path(),
        &[
            "ev",
            "CREATE TABLE ev (id INT PRIMARY KEY, d DATE, ts TIMESTAMP)",
        ],
    );
    for sql in [
        "INSERT INTO ev VALUES (1, DATE '2024-01-15', TIMESTAMP '2024-01-15 08:00:00')",
        "INSERT INTO ev VALUES (2, DATE '2024-02-29', TIMESTAMP '2024-02-29 10:30:00.123456')",
        "INSERT INTO ev VALUES (3, DATE '2023-12-31', TIMESTAMP '2023-12-31 23:59:59')",
    ] {
        let out = run_cli(dir.path(), &["ev", sql]);
        assert_eq!(out.code, Some(0), "fixture insert failed: {sql} -> {out:?}");
    }
}

/// R4-S1 时间序过滤与排序：`WHERE d > DATE '2024-01-01'` 命中两行，
/// `ORDER BY d` 按时间序（非字典序）正确排列。
///
/// RED（修复前实测，T2 前）：compare_values 无日期臂，`_ => Equal` 兜底
/// 吞掉排序；过滤谓词跨族不可达。
#[test]
fn date_comparison_filter_and_ordering() {
    let dir = TempDir::new().unwrap();
    fixture_datetime_table(&dir);

    let sel = run_cli(
        dir.path(),
        &[
            "ev",
            "SELECT id FROM ev WHERE d > DATE '2024-01-01' ORDER BY d",
        ],
    );
    assert_eq!(sel.code, Some(0), "时间序过滤应成功: {sel:?}");
    assert!(
        sel.stdout.contains("\"rows\":[[1],[2]]"),
        "应按时间序命中 2024-01-15 与 2024-02-29: {sel:?}"
    );
}

/// R4-S2 跨类型比较拒绝：`WHERE d = '2024-01-15'`（String 字面量侧）显式
/// 类型错误——比较严格（决策 2），类型字面量或 CAST 才可达。
///
/// RED（修复前实测）：equals 跨族返回 false，静默空集 exit 0。
#[test]
fn cross_type_comparison_rejected() {
    let dir = TempDir::new().unwrap();
    fixture_datetime_table(&dir);

    let sel = run_cli(
        dir.path(),
        &["ev", "SELECT id FROM ev WHERE d = '2024-01-15'"],
    );
    assert_ne!(
        sel.code,
        Some(0),
        "Date 与 String 等值比较应显式类型错误: {sel:?}"
    );
    assert!(
        sel.stderr.to_lowercase().contains("type mismatch"),
        "错误文案应为类型不匹配: {sel:?}"
    );
}

/// R4-S3 Date 主键路由回退：DATE 列作 PRIMARY KEY，键位等值过滤经 MS16
/// 非 Int 键列 DataScan 回退可达（无静默漏行）。
///
/// 回归锚点（MS16 路由 2026-09-13 已落地，本用例锁定其覆盖日期键列）。
#[test]
fn date_pk_equality_routing_fallback() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "ev");

    run_cli(
        dir.path(),
        &["ev", "CREATE TABLE ev (d DATE PRIMARY KEY, v INT)"],
    );
    run_cli(
        dir.path(),
        &["ev", "INSERT INTO ev VALUES (DATE '2024-01-15', 10)"],
    );
    run_cli(
        dir.path(),
        &["ev", "INSERT INTO ev VALUES (DATE '2024-02-20', 20)"],
    );

    let sel = run_cli(
        dir.path(),
        &["ev", "SELECT v FROM ev WHERE d = DATE '2024-01-15'"],
    );
    assert_eq!(sel.code, Some(0), "Date PK 键位等值过滤应可达: {sel:?}");
    assert!(
        sel.stdout.contains("\"rows\":[[10]]"),
        "键位等值必须命中 Date 主键行（MS16 回退）: {sel:?}"
    );
}

/// R5-S1/S2 CAST 矩阵：String→Date/Timestamp 解析、Date/Timestamp→String
/// 格式化、Timestamp→Date 截断、Date→Timestamp 零点扩展；非法与跨族拒绝。
///
/// RED（修复前实测）：CAST 目标四族，DATE 目标报 Unsupported CAST target。
#[test]
fn cast_matrix_datetime() {
    let dir = TempDir::new().unwrap();
    fixture_datetime_table(&dir);

    let cases: Vec<(&str, &str)> = vec![
        // (SQL, 期望输出包含)
        (
            "SELECT CAST('2024-01-15' AS DATE) AS c FROM ev WHERE id = 1",
            "\"rows\":[[\"2024-01-15\"]]",
        ),
        (
            "SELECT CAST('2024-01-15 10:30:00.5' AS TIMESTAMP) AS c FROM ev WHERE id = 1",
            "\"rows\":[[\"2024-01-15 10:30:00.500000\"]]",
        ),
        (
            "SELECT CAST(d AS STRING) AS c FROM ev WHERE id = 1",
            "\"rows\":[[\"2024-01-15\"]]",
        ),
        (
            "SELECT CAST(ts AS DATE) AS c FROM ev WHERE id = 2",
            "\"rows\":[[\"2024-02-29\"]]",
        ),
        (
            "SELECT CAST(d AS TIMESTAMP) AS c FROM ev WHERE id = 1",
            "\"rows\":[[\"2024-01-15 00:00:00\"]]",
        ),
    ];
    for (sql, expect) in cases {
        let sel = run_cli(dir.path(), &["ev", sql]);
        assert_eq!(sel.code, Some(0), "CAST 应成功: {sql} -> {sel:?}");
        assert!(sel.stdout.contains(expect), "{sql}\n应含 {expect}: {sel:?}");
    }

    // 非法解析与跨族拒绝
    for sql in [
        "SELECT CAST('bad' AS DATE) AS c FROM ev WHERE id = 1",
        "SELECT CAST(42 AS DATE) AS c FROM ev WHERE id = 1",
    ] {
        let sel = run_cli(dir.path(), &["ev", sql]);
        assert_ne!(sel.code, Some(0), "应显式拒绝: {sql} -> {sel:?}");
    }
}

/// R6-S1 dump/restore 恒等：含 DATE/TIMESTAMP 列与值的库 dump → 空库
/// restore → 再 dump，两代 dump 文本恒等（类型化字面量 + DATE/TIMESTAMP DDL）。
///
/// RED（修复前实测）：dump 输出裸字符串字面量，restore 后日期列类型丢失。
#[test]
fn dump_restore_roundtrip_identity() {
    let dir = TempDir::new().unwrap();
    fixture_datetime_table(&dir);

    let dump1 = run_cli(dir.path(), &["dump", "ev"]);
    assert_eq!(dump1.code, Some(0), "dump 应成功: {dump1:?}");
    assert!(
        dump1.stdout.contains("\"d\" DATE") && dump1.stdout.contains("\"ts\" TIMESTAMP"),
        "dump DDL 应渲染 DATE/TIMESTAMP 类型: {dump1:?}"
    );
    assert!(
        dump1.stdout.contains("DATE '2024-01-15'")
            && dump1
                .stdout
                .contains("TIMESTAMP '2024-02-29 10:30:00.123456'"),
        "dump 应输出类型化字面量: {dump1:?}"
    );

    new_db(dir.path(), "dst");
    let restore = run_cli_stdin(dir.path(), &["restore", "dst", "-"], &dump1.stdout);
    assert_eq!(restore.code, Some(0), "restore 应成功: {restore:?}");

    let dump2 = run_cli(dir.path(), &["dump", "dst"]);
    assert_eq!(dump2.code, Some(0), "二次 dump 应成功: {dump2:?}");
    assert_eq!(
        dump1.stdout, dump2.stdout,
        "两代 dump 文本必须恒等（多代 dump/restore）"
    );
}

/// R6-S2 CSV import 落类型：DATE 列字段经强制解析通路落 Date，空字段落
/// NULL，非法日期字段 fail-fast 报错。
///
/// RED（修复前实测）：Date 列空字段走 String 默认空串、非空字段落 String。
#[test]
fn csv_import_datetime_columns() {
    let dir = TempDir::new().unwrap();
    new_db(dir.path(), "ev");
    run_cli(
        dir.path(),
        &[
            "ev",
            "CREATE TABLE ev (id INT PRIMARY KEY, d DATE, ts TIMESTAMP)",
        ],
    );

    let csv_path = dir.path().join("dates.csv");
    std::fs::write(
        &csv_path,
        "id,d,ts\n1,2024-01-15,2024-01-15 08:00:00\n2,,\n",
    )
    .unwrap();
    let imp = run_cli(
        dir.path(),
        &["import", "--csv", "ev", "ev", csv_path.to_str().unwrap()],
    );
    assert_eq!(imp.code, Some(0), "import 应成功: {imp:?}");

    let sel = run_cli(dir.path(), &["ev", "SELECT id, d, ts FROM ev ORDER BY id"]);
    assert!(
        sel.stdout.contains("2024-01-15")
            && sel
                .stdout
                .contains("\"rows\":[[1,\"2024-01-15\",\"2024-01-15 08:00:00\"],[2,null,null]]"),
        "日期字段应落类型、空字段落 NULL: {sel:?}"
    );

    // 非法日期字段 fail-fast
    let bad_path = dir.path().join("bad.csv");
    std::fs::write(&bad_path, "id,d,ts\n3,not-a-date,\n").unwrap();
    let bad = run_cli(
        dir.path(),
        &["import", "--csv", "ev", "ev", bad_path.to_str().unwrap()],
    );
    assert_ne!(bad.code, Some(0), "非法日期字段应 fail-fast: {bad:?}");
}

/// R6-S3 json 渲染字符串形态：Date/Timestamp 经 value_to_json 输出 DA5
/// 字符串（带引号 JSON string），四格式渲染面契约锁定。
#[test]
fn json_render_string_form() {
    let dir = TempDir::new().unwrap();
    fixture_datetime_table(&dir);

    let sel = run_cli(
        dir.path(),
        &[
            "--format",
            "json",
            "ev",
            "SELECT d, ts FROM ev WHERE id = 2",
        ],
    );
    assert_eq!(sel.code, Some(0));
    assert!(
        sel.stdout
            .contains("\"rows\":[[\"2024-02-29\",\"2024-02-29 10:30:00.123456\"]]"),
        "json 渲染应为 DA5 字符串形态: {sel:?}"
    );
}
