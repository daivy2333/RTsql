//! MS25 Iteration 001 / Cycle 000：聚合扩展（Task T4，design D7）
//!
//! change: 2026-09-26-ms25-sql-read-surface
//!
//! 病灶：① `extract_aggregate_func` 不读 `func.distinct` 量化词——
//! `COUNT/SUM/AVG(DISTINCT x)` 静默按普通形态计算（静默错误结果，MS15/MS16
//! 同类红线）；② `group_concat` 不在聚合名单，走标量函数路径报未知名。
//!
//! 目标语义（delta spec `sql-aggregate-extensions`）：
//! - R1 `COUNT(DISTINCT X)` 去重非 NULL 计数（全 NULL → 0）；多参数点名拒绝。
//! - R2 `SUM/AVG(DISTINCT X)` 去重计算；`MIN/MAX(DISTINCT X)` 接受并等价普通
//!   形态（SQLite 对齐）；与 GROUP BY 组合按分组语义。
//! - R3 `group_concat(X[, sep])`：逗号/字面量分隔符连接组内非 NULL 值（输入
//!   行序），全 NULL 组 → NULL；非字面量分隔符点名拒绝。
//! - R4 既有五聚合无 DISTINCT 形态逐字节不变（零校准由既有套件见证）。
//!
//! 输出名：`count_distinct_{col}` / `sum_distinct_{col}` / `avg_distinct_{col}` /
//! `group_concat_{col}`。多组行序经 HashMap 分桶非确定：多组断言带 ORDER BY。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use serde_json::json;
use std::process::Command;
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("test")
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = db.execute_sql(sql).await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "setup statement failed: {sql:?} -> {resp:?}"
    );
}

fn query_rows(resp: Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("Expected QueryResult, got {other:?}"),
    }
}

fn error_msg(resp: Response) -> String {
    match resp {
        Response::Error { message, .. } => message,
        other => panic!("Expected Error, got {other:?}"),
    }
}

/// R1-S1：`COUNT(DISTINCT v)` 去重非 NULL 计数（现状 RED：静默输出 3）。
#[tokio::test]
async fn count_distinct_basic() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 2)").await;
    exec_ok(&db, "INSERT INTO t VALUES (4, NULL)").await;

    let rows = query_rows(db.execute_sql("SELECT count(DISTINCT v) FROM t").await);
    assert_eq!(
        rows,
        vec![vec![json!(2)]],
        "COUNT(DISTINCT v) 必须按去重非 NULL 计数（1,1,2,NULL → 2）"
    );

    db.wal_buffer.shutdown().await;
}

/// R1-S2：全 NULL 组去重计数为 0。
#[tokio::test]
async fn count_distinct_all_null_zero() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, NULL)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, NULL)").await;

    let rows = query_rows(db.execute_sql("SELECT count(DISTINCT v) FROM t").await);
    assert_eq!(rows, vec![vec![json!(0)]], "全 NULL 去重计数必须为 0");

    db.wal_buffer.shutdown().await;
}

/// R1-S3：多参数 `COUNT(DISTINCT a, b)` 计划期点名拒绝。
#[tokio::test]
async fn count_distinct_multi_arg_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, a INT, b INT)").await;

    let msg = error_msg(db.execute_sql("SELECT count(DISTINCT a, b) FROM t").await);
    assert!(
        msg.contains("COUNT"),
        "多参数 COUNT(DISTINCT a, b) 必须点名拒绝并提及 COUNT，实际: {msg}"
    );

    db.wal_buffer.shutdown().await;
}

/// R2-S1：`SUM(DISTINCT v)` 去重求和（1,1,2 → 3）。
#[tokio::test]
async fn sum_distinct() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 2)").await;

    let rows = query_rows(db.execute_sql("SELECT sum(DISTINCT v) FROM t").await);
    assert_eq!(
        rows,
        vec![vec![json!(3)]],
        "SUM(DISTINCT v) 必须按去重值求和"
    );

    db.wal_buffer.shutdown().await;
}

/// R2-S2：`AVG(DISTINCT v)` 去重平均（1,1,3 → 2.0）。
#[tokio::test]
async fn avg_distinct() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 3)").await;

    let rows = query_rows(db.execute_sql("SELECT avg(DISTINCT v) FROM t").await);
    assert_eq!(
        rows,
        vec![vec![json!(2.0)]],
        "AVG(DISTINCT v) 必须按去重值求平均（去重后 1,3 → 2.0）"
    );

    db.wal_buffer.shutdown().await;
}

/// R2-S3：`MIN(DISTINCT v)` 等价普通 MIN（3,1,1 → 1）。
#[tokio::test]
async fn min_distinct_eq_plain() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 3)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 1)").await;

    let rows = query_rows(db.execute_sql("SELECT min(DISTINCT v) FROM t").await);
    let plain = query_rows(db.execute_sql("SELECT min(v) FROM t").await);
    assert_eq!(rows, vec![vec![json!(1)]], "MIN(DISTINCT v) 必须输出 1");
    assert_eq!(
        rows, plain,
        "MIN(DISTINCT v) 必须与普通 MIN(v) 逐值一致（SQLite 对齐）"
    );

    db.wal_buffer.shutdown().await;
}

/// R2-S4：DISTINCT 量化词与 GROUP BY 组合按分组语义。
#[tokio::test]
async fn distinct_with_group_by() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, g VARCHAR, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'a', 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'a', 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 'a', 2)").await;
    exec_ok(&db, "INSERT INTO t VALUES (4, 'b', 5)").await;

    let rows = query_rows(
        db.execute_sql("SELECT g, count(DISTINCT v) FROM t GROUP BY g ORDER BY g")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!("a"), json!(2)], vec![json!("b"), json!(1)]],
        "count(DISTINCT v) 必须按分组去重计数（a 组 1,1,2 → 2）"
    );

    db.wal_buffer.shutdown().await;
}

/// R3-S1：`group_concat(v)` 默认逗号分隔（'a','b','c' → 'a,b,c'）。
#[tokio::test]
async fn group_concat_default_sep() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v VARCHAR)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'a')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'b')").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 'c')").await;

    let rows = query_rows(db.execute_sql("SELECT group_concat(v) FROM t").await);
    assert_eq!(
        rows,
        vec![vec![json!("a,b,c")]],
        "group_concat 必须按输入行序以逗号连接组内非 NULL 值"
    );

    db.wal_buffer.shutdown().await;
}

/// R3-S2：`group_concat(v, '|')` 自定义字面量分隔符。
#[tokio::test]
async fn group_concat_custom_sep() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v VARCHAR)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'a')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'b')").await;

    let rows = query_rows(db.execute_sql("SELECT group_concat(v, '|') FROM t").await);
    assert_eq!(
        rows,
        vec![vec![json!("a|b")]],
        "group_concat 第二参数字面量必须作为分隔符"
    );

    db.wal_buffer.shutdown().await;
}

/// R3-S3：NULL 跳过；全 NULL 组 → NULL。
#[tokio::test]
async fn group_concat_null_skip_and_all_null_group() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(
        &db,
        "CREATE TABLE t (k INT PRIMARY KEY, g VARCHAR, v VARCHAR)",
    )
    .await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'a', NULL)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'a', 'x')").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 'b', NULL)").await;

    let rows = query_rows(
        db.execute_sql("SELECT g, group_concat(v) FROM t GROUP BY g ORDER BY g")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![json!("a"), json!("x")], vec![json!("b"), json!(null)]],
        "group_concat 必须跳过 NULL 值且全 NULL 组返回 NULL"
    );

    db.wal_buffer.shutdown().await;
}

/// R3-S4：非字面量分隔符计划期点名拒绝。
#[tokio::test]
async fn group_concat_non_literal_sep_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v VARCHAR)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'a')").await;

    let msg = error_msg(
        db.execute_sql("SELECT group_concat(v, upper('|')) FROM t")
            .await,
    );
    assert!(
        msg.to_lowercase().contains("separator"),
        "非字面量分隔符必须点名拒绝并提及 separator，实际: {msg}"
    );

    db.wal_buffer.shutdown().await;
}

/// R4-S1：既有五聚合无 DISTINCT 形态结果与输出名零变化。
#[tokio::test]
async fn existing_aggregates_unchanged() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, g VARCHAR, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'a', 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'a', 3)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 'b', NULL)").await;

    let rows = query_rows(
        db.execute_sql(
            "SELECT g, count(*), count(v), sum(v), min(v), max(v) FROM t GROUP BY g ORDER BY g",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![json!("a"), json!(2), json!(2), json!(4), json!(1), json!(3)],
            vec![
                json!("b"),
                json!(1),
                json!(0),
                json!(null),
                json!(null),
                json!(null)
            ],
        ],
        "既有五聚合形态结果必须逐值不变"
    );

    db.wal_buffer.shutdown().await;
}

/// R1/R2/R3-④：新聚合形态的输出列名（契约 Required behavior ④）——
/// `count_distinct_{col}` / `sum_distinct_{col}` / `avg_distinct_{col}` /
/// `group_concat_{col}`。CLI JSON 渲染携带 `columns`，是真表头的见证面。
#[test]
fn new_aggregate_output_names() {
    let dir = tempdir().unwrap();
    // 含 `/` 的路径直开（裸名会解析到 $RTSQL_HOME/db/，与本夹具位置不同）
    let db_str = db_path(&dir).to_str().unwrap().to_string();

    for sql in [
        "CREATE TABLE t (k INT PRIMARY KEY, v INT, s VARCHAR)",
        "INSERT INTO t VALUES (1, 1, 'a')",
        "INSERT INTO t VALUES (2, 1, 'b')",
        "INSERT INTO t VALUES (3, 2, 'c')",
    ] {
        let out = run_cli(&dir, &db_str, sql);
        assert_eq!(out.0, Some(0), "setup 失败 {sql}: {}", out.2);
    }

    let out = run_cli(
        &dir,
        &db_str,
        "SELECT count(DISTINCT v), sum(DISTINCT v), avg(DISTINCT v), group_concat(s) FROM t",
    );
    assert_eq!(out.0, Some(0), "CLI 查询失败: {}", out.2);
    let parsed: serde_json::Value = serde_json::from_str(out.1.trim())
        .unwrap_or_else(|e| panic!("stdout 非 JSON（{e}）: {}", out.1));
    assert_eq!(
        parsed["columns"],
        json!([
            "count_distinct_v",
            "sum_distinct_v",
            "avg_distinct_v",
            "group_concat_s"
        ]),
        "新聚合形态必须产出契约规定的输出列名"
    );
    assert_eq!(
        parsed["rows"],
        json!([[2, 3, 1.5, "a,b,c"]]),
        "新聚合形态的结果值必须与契约语义一致"
    );
}

/// 运行真二进制（one-shot，JSON 渲染）——(退出码, stdout, stderr)。
fn run_cli(dir: &TempDir, db: &str, sql: &str) -> (Option<i32>, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .arg(db)
        .arg(sql)
        .args(["--format", "json"])
        .current_dir(dir.path())
        .env("RTSQL_HOME", dir.path())
        .env_remove("RTSQL_KEY")
        .output()
        .expect("spawn rtsql binary");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}
