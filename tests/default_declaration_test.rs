//! MS29 Iteration 001：DEFAULT 声明期一致（T4 CREATE TABLE 校验/归一、
//! T5 TypedString 臂与 dump→restore 往返修复、T6 ALTER ADD COLUMN 同款规则）
//!
//! change: 2026-10-01-ms29-import-input-and-default-declaration / tasks T4-T6
//!
//! 缺陷：DEFAULT 声明解析进 catalog 后零校验——Bool 列 `DEFAULT 1` 建表被
//! 接受，应用默认值的 INSERT 由写入类型门拒绝（「建表成功但应用默认值必然
//! 失败」分裂）；`extract_default_value` 无 TypedString 臂——dump 渲染的
//! `DEFAULT DATE '…'` 在 restore 侧报 `Unsupported value type`，日期族
//! DEFAULT 的 dump→restore 往返断裂。
//!
//! 目标语义（spec sql-write-surface ADDED）：声明期与列类型校验/归一——
//! NULL 豁免、FLOAT 列整数升格归一持久化、日期族 String 强制解析、其余跨
//! 类型点名拒绝零副作用；TypedString 在日期族列上解析接受（目录形态与
//! String 声明同态）、非日期族维持拒绝；ALTER ADD COLUMN 同款。

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

// ---- lib 夹具（subset_insert_test 先例） ----

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

fn expect_error(resp: Response, what: &str) -> String {
    match resp {
        Response::Error { message, .. } => message,
        other => panic!("{what} 应被拒绝为 Error，实际 {other:?}"),
    }
}

fn expect_affected(resp: Response, what: &str) {
    match resp {
        Response::AffectedRows { .. } => {}
        other => panic!("{what} 应成功，实际 {other:?}"),
    }
}

fn query_rows(resp: Response, what: &str) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("{what} 应返回 QueryResult，实际 {other:?}"),
    }
}

async fn expect_table_absent(db: &Database, table: &str, what: &str) {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    match to_response(db.execute_sql(&sql).await) {
        Response::Error { .. } => {}
        other => panic!("{what} 拒绝后表必须不存在，实际 {other:?}"),
    }
}

async fn shutdown(db: &Database) {
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// ---- CLI 夹具（cli_test 先例：独立 TempDir CWD + RTSQL_HOME，管道非 TTY） ----

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    let out = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary")
        .wait_with_output()
        .expect("wait rtsql binary");
    CliOutput {
        code: out.status.code(),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

// ---- T4：CREATE TABLE 声明期 DEFAULT 校验/归一 ----

/// R-声明期-S1：BOOL 列 DEFAULT 整数字面量建表拒绝。
///
/// RED（实现前直证）：任意字面量 DEFAULT 被接受并持久化，分裂形态可达。
#[tokio::test]
async fn create_table_bool_default_int_rejected() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    let message = expect_error(
        to_response(db.execute_sql("CREATE TABLE t (b BOOL DEFAULT 1)").await),
        "BOOL 列 DEFAULT 1 建表",
    );
    assert!(
        message.contains('b') && message.contains("BOOL") && message.contains("INT"),
        "错误必须点名列名与期望/实际类型，实际: {message}"
    );

    expect_table_absent(&db, "t", "BOOL DEFAULT 1 建表拒绝").await;
    shutdown(&db).await;
}

/// R-声明期-S2：跨类型声明拒绝矩阵（INT×String / STRING×Int / INT×Float）。
#[tokio::test]
async fn create_table_cross_type_default_matrix_rejected() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    for (sql, col, expected, actual) in [
        ("CREATE TABLE t1 (a INT DEFAULT 'x')", "a", "INT", "STRING"),
        ("CREATE TABLE t2 (s STRING DEFAULT 1)", "s", "STRING", "INT"),
        ("CREATE TABLE t3 (a INT DEFAULT 1.5)", "a", "INT", "FLOAT"),
    ] {
        let message = expect_error(to_response(db.execute_sql(sql).await), sql);
        assert!(
            message.contains(col) && message.contains(expected) && message.contains(actual),
            "{sql} 错误必须点名列名与类型，实际: {message}"
        );
    }

    expect_table_absent(&db, "t1", "矩阵拒绝").await;
    expect_table_absent(&db, "t2", "矩阵拒绝").await;
    expect_table_absent(&db, "t3", "矩阵拒绝").await;
    shutdown(&db).await;
}

/// R-声明期-S4：日期族 DEFAULT String 强制解析——非法日期计划期拒绝。
///
/// RED（实现前直证）：`DEFAULT '2026-13-99'` 建表成功。
#[tokio::test]
async fn create_table_invalid_date_string_default_rejected() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    let message = expect_error(
        to_response(
            db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, d DATE DEFAULT '2026-13-99')")
                .await,
        ),
        "非法日期 DEFAULT 建表",
    );
    assert!(message.contains('d'), "错误必须点名列名，实际: {message}");

    expect_table_absent(&db, "t", "非法日期 DEFAULT 拒绝").await;
    shutdown(&db).await;
}

/// R-声明期-S3：FLOAT 列整数 DEFAULT 升格归一——读回 2.0 且跨重启一致。
///
/// 实现前：声明 Int(2) 原样持久化（应用时写入类型门升格，读回同为 2.0——
/// 归一的区分观察点在 dump 渲染，见 float_default_dump_renders_normalized）。
#[tokio::test]
async fn float_column_int_default_normalized_across_restart() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();

        expect_affected(
            to_response(
                db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, score FLOAT DEFAULT 2)")
                    .await,
            ),
            "建表",
        );
        expect_affected(
            to_response(db.execute_sql("INSERT INTO t (id) VALUES (1)").await),
            "省略 score 的 INSERT",
        );
        let rows = query_rows(
            to_response(db.execute_sql("SELECT score FROM t WHERE id = 1").await),
            "默认值读回",
        );
        assert_eq!(
            rows[0][0],
            serde_json::json!(2.0),
            "FLOAT 列整数 DEFAULT 升格"
        );
        db.close().await.unwrap();
    } // FileStorage 随 Database 释放 advisory 锁（close 不释放锁，subset_insert 先例）

    let db2 = Database::open(&path).await.unwrap();
    expect_affected(
        to_response(db2.execute_sql("INSERT INTO t (id) VALUES (2)").await),
        "重开后省略 score 的 INSERT",
    );
    let rows = query_rows(
        to_response(db2.execute_sql("SELECT score FROM t WHERE id = 2").await),
        "重开后默认值读回",
    );
    assert_eq!(rows[0][0], serde_json::json!(2.0), "归一持久化跨重启一致");
    shutdown(&db2).await;
}

/// R-声明期-S4（接受面）：日期族 DEFAULT String 解析通过——省略列读回。
#[tokio::test]
async fn date_column_string_default_accepted_reads_back() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        to_response(
            db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, d DATE DEFAULT '2026-01-02')")
                .await,
        ),
        "建表",
    );
    expect_affected(
        to_response(db.execute_sql("INSERT INTO t (id) VALUES (1)").await),
        "省略 d 的 INSERT",
    );
    let rows = query_rows(
        to_response(db.execute_sql("SELECT d FROM t WHERE id = 1").await),
        "默认值读回",
    );
    assert_eq!(rows[0][0], serde_json::json!("2026-01-02"));
    shutdown(&db).await;
}

/// R-声明期-S7：DEFAULT NULL 豁免（NULL 性由 NOT NULL 门裁决）。
#[tokio::test]
async fn default_null_exempt_on_bool_column() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        to_response(
            db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, b BOOL DEFAULT NULL)")
                .await,
        ),
        "建表",
    );
    expect_affected(
        to_response(db.execute_sql("INSERT INTO t (id) VALUES (1)").await),
        "省略 b 的 INSERT",
    );
    let rows = query_rows(
        to_response(db.execute_sql("SELECT b FROM t WHERE id = 1").await),
        "默认值读回",
    );
    assert_eq!(
        rows[0][0],
        serde_json::json!(null),
        "DEFAULT NULL 行读回 NULL"
    );
    shutdown(&db).await;
}

/// R-声明期-S8 对照见证：BOOL 列 Bool 字面量为同型接受面（防拒绝矩阵误伤）。
#[tokio::test]
async fn bool_column_bool_default_accepted_control() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        to_response(
            db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, b BOOL DEFAULT TRUE)")
                .await,
        ),
        "建表",
    );
    expect_affected(
        to_response(db.execute_sql("INSERT INTO t (id) VALUES (1)").await),
        "省略 b 的 INSERT",
    );
    let rows = query_rows(
        to_response(db.execute_sql("SELECT b FROM t WHERE id = 1").await),
        "默认值读回",
    );
    assert_eq!(rows[0][0], serde_json::json!(true));
    shutdown(&db).await;
}

// ---- T5：TypedString 臂与 dump→restore 往返修复 ----

/// R-TypedString-S1：日期族 TypedString 声明接受（目录形态与 String 同态）。
///
/// RED（实现前直证）：`extract_default_value` 无 TypedString 臂，报
/// `Unsupported value type`。
#[tokio::test]
async fn date_typedstring_default_accepted() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        to_response(
            db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, d DATE DEFAULT DATE '2026-01-02')")
                .await,
        ),
        "TypedString DATE DEFAULT 建表",
    );
    expect_affected(
        to_response(db.execute_sql("INSERT INTO t (id) VALUES (1)").await),
        "省略 d 的 INSERT",
    );
    let rows = query_rows(
        to_response(db.execute_sql("SELECT d FROM t WHERE id = 1").await),
        "默认值读回",
    );
    assert_eq!(rows[0][0], serde_json::json!("2026-01-02"));
    shutdown(&db).await;
}

/// R-TypedString-S1（Timestamp 面）：无时区 TIMESTAMP TypedString 同款接受。
#[tokio::test]
async fn timestamp_typedstring_default_accepted() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        to_response(db.execute_sql(
            "CREATE TABLE t (id INT PRIMARY KEY, ts TIMESTAMP DEFAULT TIMESTAMP '2026-01-02 10:30:00')",
        )
        .await),
        "TypedString TIMESTAMP DEFAULT 建表",
    );
    expect_affected(
        to_response(db.execute_sql("INSERT INTO t (id) VALUES (1)").await),
        "省略 ts 的 INSERT",
    );
    let rows = query_rows(
        to_response(db.execute_sql("SELECT ts FROM t WHERE id = 1").await),
        "默认值读回",
    );
    assert_eq!(rows[0][0], serde_json::json!("2026-01-02 10:30:00"));
    shutdown(&db).await;
}

/// R-TypedString-S2：非日期族 TypedString 维持既有拒绝（Unsupported value）。
#[tokio::test]
async fn non_datetime_typedstring_still_rejected() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    let message = expect_error(
        to_response(
            db.execute_sql("CREATE TABLE t (a INT DEFAULT INT '5')")
                .await,
        ),
        "非日期族 TypedString 建表",
    );
    assert!(
        message.contains("Unsupported value"),
        "非日期族 TypedString 必须维持 Unsupported value 语义，实际: {message}"
    );
    expect_table_absent(&db, "t", "非日期族 TypedString 拒绝").await;
    shutdown(&db).await;
}

/// R-TypedString-S3（CLI e2e）：日期族 DEFAULT dump→restore 往返修复。
///
/// RED（实现前直证，proposal 实证序列）：restore 报
/// `Unsupported value type` exit 3，dump 渲染的日期族 DEFAULT 不可重建。
#[tokio::test]
async fn dump_restore_datetime_default_roundtrip() {
    let dir = TempDir::new().unwrap();
    let dir = dir.path();

    let out = run_cli(dir, &["new", "app"]);
    assert_eq!(out.code, Some(0), "new app failed: {}", out.stderr);

    let out = run_cli(
        dir,
        &[
            "app",
            "CREATE TABLE t (id INT PRIMARY KEY, d DATE DEFAULT '2026-01-02')",
        ],
    );
    assert_eq!(out.code, Some(0), "create table failed: {}", out.stderr);
    let out = run_cli(dir, &["app", "INSERT INTO t (id) VALUES (1)"]);
    assert_eq!(out.code, Some(0), "seed failed: {}", out.stderr);

    let dump_out = run_cli(dir, &["dump", "app"]);
    assert_eq!(dump_out.code, Some(0), "dump failed: {}", dump_out.stderr);
    assert!(
        dump_out.stdout.contains("DEFAULT DATE '2026-01-02'"),
        "dump 必须渲染类型化日期 DEFAULT: {:?}",
        dump_out.stdout
    );

    let dump_file = dir.join("dump.sql");
    std::fs::write(&dump_file, &dump_out.stdout).unwrap();

    let out = run_cli(dir, &["new", "b"]);
    assert_eq!(out.code, Some(0), "new b failed: {}", out.stderr);
    let out = run_cli(dir, &["restore", "b", dump_file.to_str().unwrap()]);
    assert_eq!(
        out.code,
        Some(0),
        "restore 必须成功重建日期族 DEFAULT（修复前 Unsupported value type），stdout: {:?} stderr: {:?}",
        out.stdout,
        out.stderr
    );

    // restore 后 schema 含 DEFAULT，省略列 INSERT 行为与原库一致。
    let out = run_cli(dir, &["schema", "b"]);
    assert_eq!(out.code, Some(0), "schema failed: {}", out.stderr);
    assert!(
        out.stdout.contains("DEFAULT DATE '2026-01-02'"),
        "restore 后 schema 必须含日期族 DEFAULT: {:?}",
        out.stdout
    );
    let out = run_cli(dir, &["b", "INSERT INTO t (id) VALUES (2)"]);
    assert_eq!(
        out.code,
        Some(0),
        "restore 后省略列 INSERT 失败: {}",
        out.stderr
    );
    let out = run_cli(
        dir,
        &["b", "SELECT d FROM t WHERE id = 2", "--format", "json"],
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let parsed: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    assert_eq!(parsed["rows"][0][0], serde_json::json!("2026-01-02"));
}

/// R-声明期-S3（CLI e2e）：FLOAT 归一后 dump 渲染 `DEFAULT 2.0`。
///
/// RED（实现前直证）：声明 Int(2) 原样持久化，dump 渲染 `DEFAULT 2`。
#[tokio::test]
async fn float_default_dump_renders_normalized_literal() {
    let dir = TempDir::new().unwrap();
    let dir = dir.path();

    let out = run_cli(dir, &["new", "app"]);
    assert_eq!(out.code, Some(0), "new app failed: {}", out.stderr);
    let out = run_cli(
        dir,
        &[
            "app",
            "CREATE TABLE t (id INT PRIMARY KEY, score FLOAT DEFAULT 2)",
        ],
    );
    assert_eq!(out.code, Some(0), "create table failed: {}", out.stderr);

    let dump_out = run_cli(dir, &["dump", "app"]);
    assert_eq!(dump_out.code, Some(0), "dump failed: {}", dump_out.stderr);
    assert!(
        dump_out.stdout.contains("DEFAULT 2.0"),
        "FLOAT 列整数 DEFAULT 必须归一为 FLOAT 渲染 2.0: {:?}",
        dump_out.stdout
    );
}

// ---- T6：ALTER ADD COLUMN 同款规则 ----

/// R-ALTER-S1：ADD COLUMN 跨类型 DEFAULT 计划期拒绝且零副作用。
///
/// RED（实现前直证）：`ADD COLUMN b BOOL DEFAULT 1` 成功（同款分裂）。
#[tokio::test]
async fn alter_add_column_bool_default_int_rejected() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        to_response(db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await),
        "建表",
    );
    let message = expect_error(
        to_response(
            db.execute_sql("ALTER TABLE t ADD COLUMN b BOOL DEFAULT 1")
                .await,
        ),
        "ADD COLUMN BOOL DEFAULT 1",
    );
    assert!(
        message.contains('b') && message.contains("BOOL") && message.contains("INT"),
        "错误必须点名列名与期望/实际类型，实际: {message}"
    );

    // 零副作用：表结构不变（无 b 列）。
    expect_affected(
        to_response(db.execute_sql("INSERT INTO t (id) VALUES (1)").await),
        "拒绝后原结构 INSERT",
    );
    let rows = query_rows(
        to_response(db.execute_sql("SELECT id FROM t WHERE id = 1").await),
        "原结构查询",
    );
    assert_eq!(rows[0].len(), 1, "ADD COLUMN 拒绝必须零副作用");
    shutdown(&db).await;
}

/// R-ALTER-S2：日期族 ADD COLUMN 建列成功，存量行读回声明默认值。
#[tokio::test]
async fn alter_add_column_date_default_existing_rows_read_back() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        to_response(db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await),
        "建表",
    );
    expect_affected(
        to_response(db.execute_sql("INSERT INTO t (id) VALUES (1)").await),
        "存量行",
    );
    expect_affected(
        to_response(
            db.execute_sql("ALTER TABLE t ADD COLUMN d DATE DEFAULT '2026-01-02'")
                .await,
        ),
        "ADD COLUMN DATE DEFAULT",
    );
    let rows = query_rows(
        to_response(db.execute_sql("SELECT d FROM t WHERE id = 1").await),
        "存量行默认值读回",
    );
    assert_eq!(rows[0][0], serde_json::json!("2026-01-02"));
    shutdown(&db).await;
}
