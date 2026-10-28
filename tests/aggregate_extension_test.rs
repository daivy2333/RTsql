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

#![cfg(all(feature = "network", feature = "cli"))]
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use serde_json::json;
use std::process::Command;
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("test")
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
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

    let rows = query_rows(to_response(
        db.execute_sql("SELECT count(DISTINCT v) FROM t").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(2)]],
        "COUNT(DISTINCT v) 必须按去重非 NULL 计数（1,1,2,NULL → 2）"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S2：全 NULL 组去重计数为 0。
#[tokio::test]
async fn count_distinct_all_null_zero() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, NULL)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, NULL)").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT count(DISTINCT v) FROM t").await,
    ));
    assert_eq!(rows, vec![vec![json!(0)]], "全 NULL 去重计数必须为 0");

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S3：多参数 `COUNT(DISTINCT a, b)` 计划期点名拒绝。
#[tokio::test]
async fn count_distinct_multi_arg_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, a INT, b INT)").await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT count(DISTINCT a, b) FROM t").await,
    ));
    assert!(
        msg.contains("COUNT"),
        "多参数 COUNT(DISTINCT a, b) 必须点名拒绝并提及 COUNT，实际: {msg}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
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

    let rows = query_rows(to_response(
        db.execute_sql("SELECT sum(DISTINCT v) FROM t").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(3)]],
        "SUM(DISTINCT v) 必须按去重值求和"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
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

    let rows = query_rows(to_response(
        db.execute_sql("SELECT avg(DISTINCT v) FROM t").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(2.0)]],
        "AVG(DISTINCT v) 必须按去重值求平均（去重后 1,3 → 2.0）"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
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

    let rows = query_rows(to_response(
        db.execute_sql("SELECT min(DISTINCT v) FROM t").await,
    ));
    let plain = query_rows(to_response(db.execute_sql("SELECT min(v) FROM t").await));
    assert_eq!(rows, vec![vec![json!(1)]], "MIN(DISTINCT v) 必须输出 1");
    assert_eq!(
        rows, plain,
        "MIN(DISTINCT v) 必须与普通 MIN(v) 逐值一致（SQLite 对齐）"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
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

    let rows = query_rows(to_response(
        db.execute_sql("SELECT g, count(DISTINCT v) FROM t GROUP BY g ORDER BY g")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!("a"), json!(2)], vec![json!("b"), json!(1)]],
        "count(DISTINCT v) 必须按分组去重计数（a 组 1,1,2 → 2）"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
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

    let rows = query_rows(to_response(
        db.execute_sql("SELECT group_concat(v) FROM t").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!("a,b,c")]],
        "group_concat 必须按输入行序以逗号连接组内非 NULL 值"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S2：`group_concat(v, '|')` 自定义字面量分隔符。
#[tokio::test]
async fn group_concat_custom_sep() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v VARCHAR)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'a')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'b')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT group_concat(v, '|') FROM t").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!("a|b")]],
        "group_concat 第二参数字面量必须作为分隔符"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
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

    let rows = query_rows(to_response(
        db.execute_sql("SELECT g, group_concat(v) FROM t GROUP BY g ORDER BY g")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!("a"), json!("x")], vec![json!("b"), json!(null)]],
        "group_concat 必须跳过 NULL 值且全 NULL 组返回 NULL"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S4：非字面量分隔符计划期点名拒绝。
#[tokio::test]
async fn group_concat_non_literal_sep_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v VARCHAR)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'a')").await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT group_concat(v, upper('|')) FROM t")
            .await,
    ));
    assert!(
        msg.to_lowercase().contains("separator"),
        "非字面量分隔符必须点名拒绝并提及 separator，实际: {msg}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
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

    let rows = query_rows(to_response(
        db.execute_sql(
            "SELECT g, count(*), count(v), sum(v), min(v), max(v) FROM t GROUP BY g ORDER BY g",
        )
        .await,
    ));
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

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1/R2/R3-④：新聚合形态的输出列名（契约 Required behavior ④）——
/// `count_distinct_{col}` / `sum_distinct_{col}` / `avg_distinct_{col}` /
/// `group_concat_{col}`。CLI JSON 渲染携带 `columns`，是真表头的见证面。
#[test]
fn new_aggregate_output_names() {
    let dir = tempdir().unwrap();
    // 含 `/` 的路径直开（裸名会解析到 $RTSQL_HOME/db/，与本夹具位置不同）
    let db_str = db_path(&dir).to_str().unwrap().to_string();

    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建（断言语义不变）。
    let new_out = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(["new", &db_str])
        .current_dir(&dir)
        .env("RTSQL_HOME", dir.path())
        .env_remove("RTSQL_KEY")
        .output()
        .expect("spawn rtsql new");
    assert!(
        new_out.status.success(),
        "rtsql new failed: {}",
        String::from_utf8_lossy(&new_out.stderr)
    );

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

// ===========================================================================
// change 2026-10-03-practical-core-gaps Iteration 001（T4）：聚合表达式嵌入
//
// delta `sql-aggregate-extensions` R2：聚合调用可嵌套于 SELECT 清单值表达式；
// 不含聚合也不含列引用的纯常量项与聚合混排合法（SELECT 与 INSERT SELECT 源
// 同语义）；DISTINCT 量化词在表达式中保持。失败边界（修复前）：嵌入聚合报
// `Unsupported expression type: MAX`、常量项报 Non-aggregated column。
// ===========================================================================

async fn seed_expr_table(dir: &TempDir) -> Database {
    let db = Database::open(&db_path(dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 20)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 30)").await;
    db
}

/// S6：`SELECT MAX(id)+1` 单行 next-id 形态（修复前：Unsupported expression type: MAX）。
#[tokio::test]
async fn nested_aggregate_max_plus_one() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let rows = query_rows(to_response(db.execute_sql("SELECT MAX(id)+1 FROM t").await));
    assert_eq!(rows, vec![vec![json!(4)]], "MAX(id)+1 必须单行返回 next-id");
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S6：`COALESCE(MAX(id),0)+1` 嵌套 + 空表退化（全 NULL 组 → 1）。
#[tokio::test]
async fn nested_aggregate_coalesce_empty_and_data() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT COALESCE(MAX(id),0)+1 FROM t").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(4)]],
        "COALESCE(MAX(id),0)+1 数据表 → 4"
    );
    exec_ok(&db, "CREATE TABLE e (id INT)").await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT COALESCE(MAX(id),0)+1 FROM e").await,
    ));
    assert_eq!(rows, vec![vec![json!(1)]], "空表 COALESCE(MAX(id),0)+1 → 1");
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S9：纯常量项与聚合混排合法（修复前：Non-aggregated column '1'）。
#[tokio::test]
async fn constant_mixing_with_aggregate() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT MAX(id), 1 FROM t").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(3), json!(1)]],
        "常量项必须与聚合混排合法"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S9（Risks 注记）：TypedString 日期字面量同样属纯常量项。
#[tokio::test]
async fn constant_mixing_typed_string_date() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT MAX(id), DATE '2024-01-01' FROM t")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(3), json!("2024-01-01")]],
        "TypedString 日期字面量按常量项混排"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S9：INSERT SELECT 源同语义——常量混排两列目标写入可达。
#[tokio::test]
async fn constant_mixing_insert_select_source() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    exec_ok(&db, "CREATE TABLE t2 (a INT, b INT)").await;
    exec_ok(&db, "INSERT INTO t2 SELECT MAX(id), 1 FROM t").await;
    let rows = query_rows(to_response(db.execute_sql("SELECT a, b FROM t2").await));
    assert_eq!(
        rows,
        vec![vec![json!(3), json!(1)]],
        "INSERT SELECT 源常量混排必须写入"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S11：DISTINCT 量化词在表达式内保持去重。
#[tokio::test]
async fn sum_distinct_inside_expression() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    exec_ok(&db, "INSERT INTO t VALUES (4, 10)").await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT SUM(DISTINCT v)+1 FROM t").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(61)]],
        "SUM(DISTINCT v)+1 = 60+1（去重保持）"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S6：GROUP BY 查询的嵌入聚合表达式项按分组语义求值。
#[tokio::test]
async fn grouped_nested_aggregate_expression() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT v, MAX(id)+1 FROM t GROUP BY v ORDER BY v")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(10), json!(2)],
            vec![json!(20), json!(3)],
            vec![json!(30), json!(4)]
        ],
        "分组嵌入聚合表达式必须按组求值"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// ===========================================================================
// change 2026-10-03-practical-core-gaps Iteration 001（T5）：HAVING 完整
// 表达式编译——算术/AND/OR/CAST/CASE/COALESCE/标量函数腿在聚合输出行求值；
// 聚合引用既有臂（输出列引用）保持。
// ===========================================================================

/// S7：`HAVING MAX(id)+1 > 2` 组过滤（修复前：Unsupported expression type: EXPRESSION）。
#[tokio::test]
async fn having_arithmetic_filter() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT v, MAX(id) FROM t GROUP BY v HAVING MAX(id)+1 > 2 ORDER BY v")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(20), json!(2)], vec![json!(30), json!(3)]],
        "HAVING 算术腿必须按 max+1>2 过滤"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S7：AND/OR 组合腿。
#[tokio::test]
async fn having_and_or_legs() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    // MIN(v) 须在 SELECT 清单内注册（HAVING 不解锁新聚合——既有
    // HavingNonAggregatedReference 通道保持，仅表达式编译解锁）。
    let rows = query_rows(
        to_response(db.execute_sql(
            "SELECT v, MAX(id), MIN(v) FROM t GROUP BY v HAVING MAX(id) > 1 AND MIN(v) < 30 ORDER BY v",
        )
        .await),
    );
    assert_eq!(
        rows,
        vec![vec![json!(20), json!(2), json!(20)]],
        "AND 组合腿过滤"
    );
    let rows = query_rows(to_response(
        db.execute_sql(
            "SELECT v, MAX(id) FROM t GROUP BY v HAVING MAX(id) = 1 OR MAX(id) = 3 ORDER BY v",
        )
        .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(10), json!(1)], vec![json!(30), json!(3)]],
        "OR 组合腿过滤"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S7：双侧复合腿（聚合算术 vs 聚合算术）。
#[tokio::test]
async fn having_composite_legs_both_sides() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql(
            "SELECT v, MAX(id), MIN(v) FROM t GROUP BY v HAVING MAX(id) + MIN(v) >= 31 ORDER BY v",
        )
        .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(30), json!(3), json!(30)]],
        "复合左右腿比较"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S7：COALESCE 腿（空输入组退化）。
#[tokio::test]
async fn having_coalesce_leg() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    exec_ok(&db, "CREATE TABLE e (id INT)").await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT COUNT(*), MAX(id) FROM e HAVING COALESCE(MAX(id), 0) = 0")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(0), json!(null)]],
        "COALESCE 腿在空输入组成立"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S7：CASE 腿。
#[tokio::test]
async fn having_case_leg() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let rows = query_rows(
        to_response(db.execute_sql(
            "SELECT v, MAX(id) FROM t GROUP BY v HAVING CASE WHEN MAX(id) > 2 THEN 1 ELSE 0 END = 1 ORDER BY v",
        )
        .await),
    );
    assert_eq!(rows, vec![vec![json!(30), json!(3)]], "CASE 腿按组求值过滤");
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S7：标量函数腿（upper 作用于分组键）。
#[tokio::test]
async fn having_scalar_function_leg() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE s (k INT PRIMARY KEY, name STRING)").await;
    exec_ok(&db, "INSERT INTO s VALUES (1, 'a')").await;
    exec_ok(&db, "INSERT INTO s VALUES (2, 'B')").await;
    let rows = query_rows(to_response(
        db.execute_sql(
            "SELECT name, COUNT(*) FROM s GROUP BY name HAVING upper(name) = 'A' ORDER BY name",
        )
        .await,
    ));
    assert_eq!(rows, vec![vec![json!("a"), json!(1)]], "标量函数腿可达");
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// ===========================================================================
// change 2026-10-03-practical-core-gaps Iteration 001（T6）：ORDER BY 聚合
// 表达式键——非裸列键经编译器在聚合输出行上求值；键中聚合调用（含 SELECT
// 未出现的新调用）在输出行可达（Sort 落于聚合行、条件包装在 Sort 之上装配）。
// ===========================================================================

/// S8：`ORDER BY MAX(id)+1` 升序（修复前：ORDER BY only supports column names）。
#[tokio::test]
async fn order_by_aggregate_expression_asc() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT v, MAX(id) FROM t GROUP BY v ORDER BY MAX(id)+1")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(10), json!(1)],
            vec![json!(20), json!(2)],
            vec![json!(30), json!(3)]
        ],
        "ORDER BY MAX(id)+1 升序"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S8：降序 + 与裸列键混排。
#[tokio::test]
async fn order_by_aggregate_expression_desc_mixed_keys() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT v, MAX(id) FROM t GROUP BY v ORDER BY MAX(id)+1 DESC")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(30), json!(3)],
            vec![json!(20), json!(2)],
            vec![json!(10), json!(1)]
        ],
        "ORDER BY MAX(id)+1 降序"
    );
    let rows = query_rows(to_response(
        db.execute_sql("SELECT v, MAX(id) FROM t GROUP BY v ORDER BY v, MAX(id)+1")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(10), json!(1)],
            vec![json!(20), json!(2)],
            vec![json!(30), json!(3)]
        ],
        "裸列键与表达式键混排"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S8：`ORDER BY MAX(v)`——SELECT 未出现的聚合调用经预扫描注册后可达。
#[tokio::test]
async fn order_by_aggregate_not_in_select() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT v, COUNT(*) FROM t GROUP BY v ORDER BY MAX(v) DESC")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(30), json!(1)],
            vec![json!(20), json!(1)],
            vec![json!(10), json!(1)]
        ],
        "ORDER BY MAX(v)（SELECT 无此聚合）按 MAX(v) 降序"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// ===========================================================================
// change 2026-10-03-practical-core-gaps Iteration 001（T7）：拒绝面保持——
// WHERE/JOIN ON 内聚合、嵌套聚合、无 GROUP BY 非聚合列引用（含表达式内嵌套
// 非聚合列引用）在聚合嵌入新路由下维持既有计划期点名通道。
// ===========================================================================

/// S10：WHERE 内聚合计划期点名拒绝（聚合在行过滤前无定义）。
#[tokio::test]
async fn where_aggregate_rejected() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let msg = error_msg(to_response(
        db.execute_sql("SELECT id FROM t WHERE MAX(id) > 1").await,
    ));
    assert!(
        msg.contains("Unsupported expression type: MAX"),
        "WHERE 内聚合必须经既有通道点名，实际: {msg}"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S10：JOIN ON 内聚合计划期点名拒绝。
#[tokio::test]
async fn join_on_aggregate_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE a (id INT PRIMARY KEY)").await;
    exec_ok(&db, "CREATE TABLE b (id INT PRIMARY KEY, a_id INT)").await;
    let msg = error_msg(to_response(
        db.execute_sql("SELECT a.id FROM a JOIN b ON MAX(a.id) = b.id")
            .await,
    ));
    assert!(
        msg.contains("Unsupported expression type: MAX"),
        "JOIN ON 内聚合必须经既有通道点名，实际: {msg}"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S10：嵌套聚合 `MAX(MIN(x))` 经聚合参数校验点名拒绝。
#[tokio::test]
async fn nested_aggregate_rejected() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let msg = error_msg(to_response(
        db.execute_sql("SELECT MAX(MIN(id)) FROM t").await,
    ));
    assert!(
        msg.contains("Invalid aggregate argument"),
        "嵌套聚合必须经既有参数校验通道点名，实际: {msg}"
    );
    let msg = error_msg(to_response(
        db.execute_sql("SELECT MAX(id)+MIN(MIN(v)) FROM t").await,
    ));
    assert!(
        msg.contains("Invalid aggregate argument"),
        "表达式内嵌套聚合同样点名拒绝，实际: {msg}"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S10：无 GROUP BY 的非聚合列引用既有拒绝语义保持。
#[tokio::test]
async fn nonagg_column_without_group_by_rejected() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let msg = error_msg(to_response(
        db.execute_sql("SELECT v, MAX(id) FROM t").await,
    ));
    assert!(
        msg.contains("Non-aggregated column 'v'"),
        "无 GROUP BY 非聚合列引用必须点名，实际: {msg}"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S10：聚合嵌入表达式内的非聚合列引用按同族通道点名（新路由下编译期拒绝）。
#[tokio::test]
async fn nested_nonagg_column_in_expression_rejected() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let msg = error_msg(to_response(db.execute_sql("SELECT MAX(id)+v FROM t").await));
    assert!(
        msg.contains("Non-aggregated column 'v'"),
        "表达式内非聚合列引用必须点名，实际: {msg}"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// ===========================================================================
// change 2026-10-03-practical-core-gaps Iteration 002（T12）：聚合显示名
// 重名解析去歧义（S24）——每处聚合输出引用解析到与该调用对应的注册聚合
// 输出；同一调用多次出现仍共享一个输出列；重名可观察表头保持。
// ===========================================================================

/// S24 夹具：g=1 组 max(a)=20 / max(b)=200；g=2 组 max(a)=30 / max(b)=300。
async fn seed_dup_name_table(dir: &TempDir) -> Database {
    let db = Database::open(&db_path(dir)).await.unwrap();
    exec_ok(
        &db,
        "CREATE TABLE s (id INT PRIMARY KEY, g INT, a INT, b INT)",
    )
    .await;
    exec_ok(&db, "INSERT INTO s VALUES (1, 1, 5, 50)").await;
    exec_ok(&db, "INSERT INTO s VALUES (2, 1, 20, 200)").await;
    exec_ok(&db, "INSERT INTO s VALUES (3, 2, 8, 80)").await;
    exec_ok(&db, "INSERT INTO s VALUES (4, 2, 30, 300)").await;
    db
}

/// S24-①：重复别名——两个 `AS m` 各自解析到自己的注册聚合输出
/// （修复前：wrap 按显示名首匹配，第三列静默取 max(a) 值 20/30）。
#[tokio::test]
async fn duplicate_alias_resolves_to_own_output() {
    let dir = tempdir().unwrap();
    let db = seed_dup_name_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT MAX(a) AS m, g, MAX(b) AS m FROM s GROUP BY g ORDER BY g")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(20), json!(1), json!(200)],
            vec![json!(30), json!(2), json!(300)]
        ],
        "重复别名时每个聚合项必须解析到自己的注册输出（第三列 = MAX(b)）"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S24-②：别名遮蔽 result_column_name——`MAX(b)` 的规范名 max_b 撞
/// `MAX(a) AS max_b` 的显示名，引用仍须解析到 MAX(b) 的注册输出
/// （修复前：第三列静默取 max(a) 值 20/30）。
#[tokio::test]
async fn alias_shadowing_result_name_resolves_correctly() {
    let dir = tempdir().unwrap();
    let db = seed_dup_name_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT MAX(a) AS max_b, g, MAX(b) FROM s GROUP BY g ORDER BY g")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(20), json!(1), json!(200)],
            vec![json!(30), json!(2), json!(300)]
        ],
        "别名遮蔽规范名时 MAX(b) 必须解析到自己的注册输出"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S24-③：同②形态的 HAVING 引用按调用解析（修复前：按 MAX(a) 过滤，
/// 两组均被 >150 滤空）。
#[tokio::test]
async fn having_disambiguates_shadowed_call() {
    let dir = tempdir().unwrap();
    let db = seed_dup_name_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql(
            "SELECT MAX(a) AS max_b, g, MAX(b) FROM s GROUP BY g HAVING MAX(b) > 150 ORDER BY g",
        )
        .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(20), json!(1), json!(200)],
            vec![json!(30), json!(2), json!(300)]
        ],
        "HAVING MAX(b) 必须按 MAX(b) 的注册输出过滤（两组均保留）"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S24-④：同②形态的嵌入引用按调用解析（修复前：MAX(b) 被显示名去重
/// 误判为已注册未进入共享清单，嵌入引用落到 MAX(a) 列得 21/31）。
#[tokio::test]
async fn embedded_reference_disambiguates_shadowed_call() {
    let dir = tempdir().unwrap();
    let db = seed_dup_name_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT MAX(a) AS max_b, g, MAX(b)+1 FROM s GROUP BY g ORDER BY g")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(20), json!(1), json!(201)],
            vec![json!(30), json!(2), json!(301)]
        ],
        "嵌入 MAX(b)+1 必须解析到 MAX(b) 的注册输出"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S24 对照：同一调用多次出现共享一个输出列（`SELECT MAX(id), MAX(id)+1`
/// 第二项 = 第一项 + 1；去重语义在 T12 解析机制下保持）。
#[tokio::test]
async fn shared_output_column_for_same_call() {
    let dir = tempdir().unwrap();
    let db = seed_expr_table(&dir).await;
    let rows = query_rows(to_response(
        db.execute_sql("SELECT MAX(id), MAX(id)+1 FROM t").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(3), json!(4)]],
        "同一调用共享输出列：MAX(id)+1 与 MAX(id) 同源"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}
