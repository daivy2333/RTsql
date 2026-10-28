//! MS29 Iteration 000（change 2026-10-01-ms29-import-input-and-default-declaration）：
//! import 输入面 e2e——stdin 通道（T1）、免建表自动导入（T2）、`:memory:` 豁免（T3）。
//!
//! 子进程 e2e，沿 `tests/cli_test.rs` 的 `run_cli`/`run_cli_stdin` 夹具先例
//! （RTSQL_HOME 注入 + RTSQL_KEY/RTSQL_BUSY_TIMEOUT 隔离，MS28 D8 校准先例）。

#![cfg(feature = "cli")]
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn spawn_cli(dir: &Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .env_remove("RTSQL_BUSY_TIMEOUT")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary")
}

fn spawn_cli_stdin(dir: &Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .env_remove("RTSQL_BUSY_TIMEOUT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary with piped stdin")
}

/// 向 stdin 管道写入输入后等待退出（写端关闭即 EOF，对齐 restore 先例）。
fn run_cli_stdin(dir: &Path, args: &[&str], input: &str) -> CliOutput {
    let mut child = spawn_cli_stdin(dir, args);
    let mut stdin = child.stdin.take().unwrap();
    let _ = stdin.write_all(input.as_bytes());
    drop(stdin);
    wait_cli(child)
}

fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    wait_cli(spawn_cli(dir, args))
}

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
                    let _ = child.wait();
                    panic!("rtsql did not exit within 60s (one-shot CLI must terminate)");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// 每个测试独立的仓库现场：TempDir + 预建 `db/` 集中存储目录
fn fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    dir
}

fn assert_success(out: &CliOutput, context: &str) {
    assert_eq!(
        out.code,
        Some(0),
        "{context} failed: stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
}

fn select_rows(dir: &Path, sql: &str) -> serde_json::Value {
    let out = run_cli(dir, &["app", sql]);
    assert_success(&out, sql);
    serde_json::from_str(out.stdout.trim()).unwrap()
}

// ---- T1：import stdin 通道（`-` 文件位） ----

/// import MODIFIED「stdin 导入既有表」：`printf 'a,b\n…' | rtsql import app t - --csv`
/// 与文件位等价——exit 0 + affected 渲染 + SELECT 比对。
#[test]
fn stdin_import_existing_table() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_success(&out, "new app");
    let out = run_cli(dir.path(), &["app", "CREATE TABLE t (a INT, b STRING)"]);
    assert_success(&out, "create table");

    let out = run_cli_stdin(
        dir.path(),
        &["import", "app", "t", "-", "--csv"],
        "a,b\n1,x\n2,y\n",
    );
    assert_success(&out, "stdin import");
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["affected_rows"],
        serde_json::json!(2),
        "stdin import must report affected rows: {:?}",
        out.stdout
    );

    let rows = select_rows(dir.path(), "SELECT a, b FROM t");
    assert_eq!(rows["rows"], serde_json::json!([[1, "x"], [2, "y"]]));
}

/// stdin 通道的空字段语义与既有 `csv_value` 规则一致：非 String 列空字段 →
/// NULL、String 列空字段 → 空字符串。
#[test]
fn stdin_import_empty_field_semantics() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_success(&out, "new app");
    let out = run_cli(dir.path(), &["app", "CREATE TABLE t (a INT, b STRING)"]);
    assert_success(&out, "create table");

    let out = run_cli_stdin(
        dir.path(),
        &["import", "app", "t", "-", "--csv"],
        "a,b\n1,\n,2\n",
    );
    assert_success(&out, "stdin import with empty fields");
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["affected_rows"],
        serde_json::json!(2),
        "{:?}",
        out.stdout
    );

    let rows = select_rows(dir.path(), "SELECT a, b FROM t");
    assert_eq!(rows["rows"], serde_json::json!([[1, ""], [null, "2"]]));
}

// ---- T2：免建表导入（自动建表） ----

/// import MODIFIED「目标表不存在自动建表（类型推导）」：INT∪FLOAT → FLOAT
/// 提升、schema 可见、全部行导入且 SELECT 比对。
#[test]
fn auto_create_type_inference_and_promotion() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_success(&out, "new app");

    std::fs::write(
        dir.path().join("data.csv"),
        "id,score,label\n1,2.5,alice\n2,3,bob\n",
    )
    .unwrap();
    let out = run_cli(dir.path(), &["import", "app", "t", "data.csv", "--csv"]);
    assert_success(&out, "auto-create import");
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["affected_rows"],
        serde_json::json!(2),
        "{:?}",
        out.stdout
    );

    // schema 可见自动建表：id INT / score FLOAT（INT∪FLOAT 提升）/ label STRING
    let out = run_cli(dir.path(), &["schema", "app"]);
    assert_success(&out, "schema");
    assert!(
        out.stdout.contains("\"id\" INTEGER"),
        "schema: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("\"score\" FLOAT"),
        "INT∪FLOAT must promote to FLOAT: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("\"label\" TEXT"),
        "schema: {:?}",
        out.stdout
    );

    let rows = select_rows(dir.path(), "SELECT id, score, label FROM t");
    assert_eq!(
        rows["rows"],
        serde_json::json!([[1, 2.5, "alice"], [2, 3.0, "bob"]])
    );
}

/// import MODIFIED「同型列推导与全 NULL 列」：全空字段列落 STRING（空字段
/// 插入空字符串），同型整数列落 INT。
#[test]
fn auto_create_all_null_column() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_success(&out, "new app");

    std::fs::write(dir.path().join("data.csv"), "a,b\n1,\n2,\n").unwrap();
    let out = run_cli(dir.path(), &["import", "app", "t", "data.csv", "--csv"]);
    assert_success(&out, "auto-create import with all-null column");

    let out = run_cli(dir.path(), &["schema", "app"]);
    assert_success(&out, "schema");
    assert!(
        out.stdout.contains("\"a\" INTEGER"),
        "schema: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("\"b\" TEXT"),
        "all-NULL column must fall to STRING: {:?}",
        out.stdout
    );

    let rows = select_rows(dir.path(), "SELECT a, b FROM t");
    assert_eq!(rows["rows"], serde_json::json!([[1, ""], [2, ""]]));
}

/// import MODIFIED「零数据行（仅表头）自动建表」：仅表头 → 全列 STRING、
/// 导入 0 行、exit 0。
#[test]
fn auto_create_zero_data_rows() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_success(&out, "new app");

    std::fs::write(dir.path().join("data.csv"), "a,b,c\n").unwrap();
    let out = run_cli(dir.path(), &["import", "app", "t", "data.csv", "--csv"]);
    assert_success(&out, "zero-data auto-create import");
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["affected_rows"],
        serde_json::json!(0),
        "{:?}",
        out.stdout
    );

    let out = run_cli(dir.path(), &["schema", "app"]);
    assert_success(&out, "schema");
    assert!(
        out.stdout.contains("\"a\" TEXT")
            && out.stdout.contains("\"b\" TEXT")
            && out.stdout.contains("\"c\" TEXT"),
        "header-only CSV must create all-STRING columns: {:?}",
        out.stdout
    );
}

/// import MODIFIED「类型冲突点名拒绝零副作用」：列内 INT∪STRING 冲突 →
/// exit 1 点名列名与冲突类型；目标表不存在（schema 零输出）。
#[test]
fn auto_create_conflict_rejected_zero_side_effects() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_success(&out, "new app");

    std::fs::write(dir.path().join("data.csv"), "a\n1\n2\nabc\n").unwrap();
    let out = run_cli(dir.path(), &["import", "app", "t", "data.csv", "--csv"]);
    assert_eq!(
        out.code,
        Some(1),
        "conflict must be General exit 1: stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("a (INT, STRING)"),
        "error must name the column and both conflict types: {:?}",
        out.stderr
    );

    let out = run_cli(dir.path(), &["schema", "app"]);
    assert_success(&out, "schema");
    assert!(
        out.stdout.trim().is_empty(),
        "failed inference must leave no table behind: {:?}",
        out.stdout
    );
}

/// import MODIFIED「自动建表首列隐式主键（引擎语义沿用）」（replan 修订）：
/// INT 首列 = 隐式主键，于首个重复值处 DuplicateKey fail-fast（exit 3，
/// 失败前已提交行重开可见，schema 渲染首列 PRIMARY KEY）；STRING 首列
/// 重复值全部导入（非键行语义）。
#[test]
fn auto_create_first_column_implicit_pk_semantics() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_success(&out, "new app");

    // INT 首列：引擎「未声明 PK → 首列隐式主键」（与 CTAS 同款）。
    std::fs::write(dir.path().join("int.csv"), "id,v\n1,a\n1,b\n").unwrap();
    let out = run_cli(dir.path(), &["import", "app", "t", "int.csv", "--csv"]);
    assert_eq!(
        out.code,
        Some(3),
        "INT first column is the implicit PK: duplicates must fail fast: \
         stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("Duplicate"),
        "error must carry DuplicateKey semantics: {:?}",
        out.stderr
    );

    let rows = select_rows(dir.path(), "SELECT id, v FROM t");
    assert_eq!(
        rows["rows"],
        serde_json::json!([[1, "a"]]),
        "row committed before the failure stays visible: {:?}",
        rows
    );

    let out = run_cli(dir.path(), &["schema", "app"]);
    assert_success(&out, "schema");
    assert!(
        out.stdout.contains("\"id\" INTEGER PRIMARY KEY"),
        "auto-created schema must render first column as PRIMARY KEY: {:?}",
        out.stdout
    );

    // STRING 首列：非键行 → 重复值全部导入 exit 0。
    std::fs::write(dir.path().join("str.csv"), "name,v\na,1\na,2\n").unwrap();
    let out = run_cli(dir.path(), &["import", "app", "s", "str.csv", "--csv"]);
    assert_success(&out, "STRING first column duplicates are non-key rows");
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["affected_rows"],
        serde_json::json!(2),
        "{:?}",
        out.stdout
    );

    let rows = select_rows(dir.path(), "SELECT name, v FROM s");
    assert_eq!(rows["rows"], serde_json::json!([["a", 1], ["a", 2]]));
}

/// import MODIFIED「自动建表经保留名/重复列名守卫拒绝」：重复列名
/// 经常规建表通道拒绝（SQL 失败通道 exit 3），零副作用。
#[test]
fn auto_create_duplicate_column_names_rejected() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_success(&out, "new app");

    std::fs::write(dir.path().join("data.csv"), "a,a\n1,2\n").unwrap();
    let out = run_cli(dir.path(), &["import", "app", "t", "data.csv", "--csv"]);
    assert_eq!(
        out.code,
        Some(3),
        "duplicate column names must fail via the SQL channel: stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("duplicate column name"),
        "guard must name the duplicated column: {:?}",
        out.stderr
    );

    let out = run_cli(dir.path(), &["schema", "app"]);
    assert_success(&out, "schema");
    assert!(
        out.stdout.trim().is_empty(),
        "failed auto-create must leave no table behind: {:?}",
        out.stdout
    );
}

/// 自动建表经保留名守卫拒绝：目标表名为系统保留名（`__tables`）时
/// 常规建表通道点名拒绝，零副作用。
#[test]
fn auto_create_reserved_name_rejected() {
    let dir = fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_success(&out, "new app");

    std::fs::write(dir.path().join("data.csv"), "a\n1\n").unwrap();
    let out = run_cli(
        dir.path(),
        &["import", "app", "__tables", "data.csv", "--csv"],
    );
    assert_eq!(
        out.code,
        Some(3),
        "reserved table name must fail via the SQL channel: stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        !out.stderr.is_empty(),
        "reserved name rejection must carry a message"
    );
}

/// import MODIFIED「目标表或库不存在拒绝」库缺失臂的 stdin 形态：
/// exit 1 + `does not exist` 文案 + 零文件创建（既有语义保持）。
#[test]
fn import_missing_db_stdin_form_zero_file_creation() {
    let dir = fixture();
    let out = run_cli_stdin(
        dir.path(),
        &["import", "missing", "t", "-", "--csv"],
        "a\n1\n",
    );
    assert_eq!(
        out.code,
        Some(1),
        "missing db must stay rejected: stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("does not exist"),
        "missing db error must follow the shared wording: {:?}",
        out.stderr
    );
    assert!(
        !dir.path().join("db/missing.db").exists(),
        "import must never create the database file"
    );
}

// ---- T3：`:memory:` import 豁免面 ----

/// import MODIFIED「`:memory:` import 免建表可达」：stdin + 自动建表组合
/// exit 0（进程退出即弃，无任何文件产生）；文件位形态同款可达。
///
/// 命令形态注记：`:memory:` 是 import 子命令的库实参（对齐既有拒绝面
/// `rtsql schema :memory:` 先例），delta spec 场景文本的
/// `rtsql :memory: import …` 语序为笔误——登记 Act Response 由 Plan 校准。
#[test]
fn memory_import_stdin_and_file() {
    let dir = fixture();

    let out = run_cli_stdin(
        dir.path(),
        &["import", ":memory:", "t", "-", "--csv"],
        "a,b\n1,x\n",
    );
    assert_success(&out, ":memory: stdin import");
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["affected_rows"],
        serde_json::json!(1),
        "{:?}",
        out.stdout
    );
    assert!(
        std::fs::read_dir(dir.path().join("db")).unwrap().count() == 0,
        ":memory: must not produce any file"
    );

    std::fs::write(dir.path().join("data.csv"), "a,b\n2,y\n").unwrap();
    let out = run_cli(
        dir.path(),
        &["import", ":memory:", "t", "data.csv", "--csv"],
    );
    assert_success(&out, ":memory: file import");
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(
        parsed["affected_rows"],
        serde_json::json!(1),
        "{:?}",
        out.stdout
    );
}

/// import MODIFIED「`:memory:` import `--key` 拒绝」+「其他生命周期子命令
/// 维持拒绝」+ `--readonly` 全局守卫：豁免面仅 import 读写形态。
#[test]
fn memory_import_key_rejected_and_others_still_guarded() {
    let dir = fixture();

    // `--key` × `:memory:` import → Usage exit 2（对齐主命令文案）
    let out = run_cli(
        dir.path(),
        &["import", ":memory:", "t", "-", "--csv", "--key", "secret"],
    );
    assert_eq!(
        out.code,
        Some(2),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stderr.contains("does not support --key"),
        "must align with the main-command wording: {:?}",
        out.stderr
    );

    // import 以外的生命周期子命令维持既有拒绝
    for args in [
        vec!["schema", ":memory:"],
        vec!["dump", ":memory:"],
        vec!["restore", ":memory:", "-"],
    ] {
        let out = run_cli(dir.path(), &args);
        assert_eq!(
            out.code,
            Some(2),
            "{args:?} must stay rejected: stdout: {:?} stderr: {:?}",
            out.stdout,
            out.stderr
        );
    }

    // `--readonly` 全局守卫先行序不变
    let out = run_cli(
        dir.path(),
        &["--readonly", "import", ":memory:", "t", "data.csv", "--csv"],
    );
    assert_eq!(
        out.code,
        Some(2),
        "stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );
}
