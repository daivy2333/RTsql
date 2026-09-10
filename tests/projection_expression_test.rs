//! MS11-T01 Iteration 001: SELECT 派生列（投影表达式机制）E2E 见证
//!
//! change: 2026-09-10-ms11-t01-sql-expressions / spec: sql-expression-evaluation
//!
//! 见证范围：R4/S1-S4 全场景 + R3/S1-S4 的 SELECT 投影形态（Iteration 000
//! Plan Review 裁定归属本 Iteration）+ 排除面守卫（通配/标量子查询/JOIN 与
//! 表达式项混用拒绝、聚合报错保持）。列名（表头）断言在 cli_test 渲染追加
//! 中承载（lib Response 不携带表头）。
//!
//! RED（实施前）：表达式项报 `Plan error: ...`（CASE 走聚合检测、
//! COALESCE/CAST 走 extract_columns 拒绝）；`SELECT 42 FROM t` 恒等回退
//! 返回全 schema 行。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use tempfile::TempDir;

async fn open_db() -> (Database, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("proj.db")).await.unwrap();
    (db, dir)
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

fn error_message(resp: Response) -> String {
    match resp {
        Response::Error { message } => message,
        other => panic!("Expected Error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// R4/S1: 派生列输出与命名
// ---------------------------------------------------------------------------

/// R4/S1：普通列 + CASE 派生列（AS 别名）逐行求值
#[tokio::test]
async fn derived_column_with_alias() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING, score INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('Alice', 95)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('Bob', 40)").await;

    let rows = query_rows(
        db.execute_sql("SELECT name, CASE WHEN score >= 60 THEN 'Y' ELSE 'N' END AS passed FROM t")
            .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!("Alice"), serde_json::json!("Y")],
            vec![serde_json::json!("Bob"), serde_json::json!("N")],
        ]
    );
}

/// R4/S1（后半）：去掉 AS 别名后仍逐行求值（表头 Display 文本由 cli_test 断言）
#[tokio::test]
async fn derived_column_without_alias_values() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (name STRING, score INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('Alice', 95)").await;

    let rows = query_rows(
        db.execute_sql("SELECT name, CASE WHEN score >= 60 THEN 'Y' ELSE 'N' END FROM t")
            .await,
    );
    assert_eq!(
        rows,
        vec![vec![serde_json::json!("Alice"), serde_json::json!("Y")]]
    );
}

// ---------------------------------------------------------------------------
// R4/S2: 派生列与普通列、字面量混合
// ---------------------------------------------------------------------------

/// R4/S2：`SELECT id, COALESCE(NULL, id)` 每行输出 id 与 id 值
#[tokio::test]
async fn mixed_column_and_expression() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2)").await;

    let rows = query_rows(db.execute_sql("SELECT id, COALESCE(NULL, id) FROM t").await);
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!(1), serde_json::json!(1)],
            vec![serde_json::json!(2), serde_json::json!(2)],
        ]
    );
}

/// 常量项与列混合：`SELECT id, 42 FROM t` 输出两列（42 不再恒等回退）
#[tokio::test]
async fn mixed_column_and_constant() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2)").await;

    let rows = query_rows(db.execute_sql("SELECT id, 42 FROM t").await);
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!(1), serde_json::json!(42)],
            vec![serde_json::json!(2), serde_json::json!(42)],
        ]
    );
}

/// 怪癖修正：`SELECT 42 FROM t` 输出单列常量，不再恒等回退返回全 schema 行
#[tokio::test]
async fn constant_item_single_column() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2)").await;

    let rows = query_rows(db.execute_sql("SELECT 42 FROM t").await);
    assert_eq!(rows, vec![vec![serde_json::json!(42); 1]; 2]);
}

// ---------------------------------------------------------------------------
// R3/S1-S4: 值表达式的 SELECT 投影形态（Iteration 000 Review 裁定归属）
// ---------------------------------------------------------------------------

/// R3/S1：searched CASE 缺省 ELSE 产出 NULL
#[tokio::test]
async fn searched_case_default_else_in_select() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (score INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (95)").await;
    exec_ok(&db, "INSERT INTO t VALUES (40)").await;

    let rows = query_rows(
        db.execute_sql("SELECT CASE WHEN score >= 60 THEN 'pass' END FROM t")
            .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!("pass")],
            vec![serde_json::Value::Null],
        ]
    );
}

/// R3/S2：simple CASE operand 为 NULL 不命中任何 WHEN（三值语义）
#[tokio::test]
async fn simple_case_null_operand_in_select() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (NULL)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;

    let rows = query_rows(
        db.execute_sql("SELECT CASE v WHEN 1 THEN 'one' ELSE 'other' END FROM t")
            .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!("other")],
            vec![serde_json::json!("one")],
        ]
    );
}

/// R3/S3：COALESCE 逐参数取首个非 NULL，全 NULL 落兜底
#[tokio::test]
async fn coalesce_in_select() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (a STRING, b STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES (NULL, 'x')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('y', 'z')").await;
    exec_ok(&db, "INSERT INTO t VALUES (NULL, NULL)").await;

    let rows = query_rows(
        db.execute_sql("SELECT COALESCE(a, b, 'fallback') FROM t")
            .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!("x")],
            vec![serde_json::json!("y")],
            vec![serde_json::json!("fallback")],
        ]
    );
}

/// R3/S4：CAST 出现在 SELECT 列表（数值/字符串转换矩阵 + NULL 短路 + 错误面）
#[tokio::test]
async fn cast_in_select() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (s STRING, f FLOAT)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('42', 1.7)").await;

    let rows = query_rows(db.execute_sql("SELECT CAST(s AS INT) FROM t").await);
    assert_eq!(rows, vec![vec![serde_json::json!(42)]]);

    let rows = query_rows(db.execute_sql("SELECT CAST(f AS INT) FROM t").await);
    assert_eq!(rows, vec![vec![serde_json::json!(1)]]);

    let rows = query_rows(db.execute_sql("SELECT CAST(42 AS STRING) FROM t").await);
    assert_eq!(rows, vec![vec![serde_json::json!("42")]]);

    // 非法转换执行期报错（'abc' 不可解析为 Int）
    exec_ok(&db, "INSERT INTO t VALUES ('abc', 2.5)").await;
    let resp = db.execute_sql("SELECT CAST(s AS INT) FROM t").await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "CAST('abc' AS INT) must fail at execution, got {resp:?}"
    );
}

/// R3（CAST NULL → NULL）：可空列 CAST 产出 NULL
#[tokio::test]
async fn cast_null_in_select() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (NULL)").await;

    let rows = query_rows(db.execute_sql("SELECT CAST(v AS STRING) FROM t").await);
    assert_eq!(rows, vec![vec![serde_json::Value::Null]]);
}

// ---------------------------------------------------------------------------
// R4/S3: SELECT * 不变与聚合查询报错保持
// ---------------------------------------------------------------------------

/// R4/S3（前半）：SELECT * 行为与现状完全一致（全 schema 行）
#[tokio::test]
async fn select_star_unchanged() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'Alice')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'Bob')").await;

    let rows = query_rows(db.execute_sql("SELECT * FROM t").await);
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!(1), serde_json::json!("Alice")],
            vec![serde_json::json!(2), serde_json::json!("Bob")],
        ]
    );
}

/// R4/S3（后半）：聚合查询中的非聚合表达式项保持聚合路径报错
#[tokio::test]
async fn aggregate_expression_error_preserved() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;

    let message = error_message(
        db.execute_sql("SELECT COUNT(*), CASE WHEN 1 = 1 THEN 'x' END FROM t")
            .await,
    );
    assert!(
        message.contains("Invalid aggregate argument"),
        "aggregate + expression must keep the aggregate-path error, got: {message}"
    );
}

// ---------------------------------------------------------------------------
// 排除面守卫（Excluded scope：混用显式拒绝，不静默变形）
// ---------------------------------------------------------------------------

/// `SELECT *, expr` 通配混用拒绝
#[tokio::test]
async fn wildcard_mixed_with_expression_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;

    let message = error_message(
        db.execute_sql("SELECT *, CASE WHEN 1 = 1 THEN 'x' END FROM t")
            .await,
    );
    assert!(
        message.contains("expression"),
        "wildcard + expression must be rejected with an explicit message, got: {message}"
    );
}

/// 表达式项与标量子查询项混用拒绝（子查询列追加会移位输出形状）
#[tokio::test]
async fn subquery_mixed_with_expression_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;

    let message = error_message(
        db.execute_sql("SELECT (SELECT MAX(id) FROM t), CASE WHEN 1 = 1 THEN 'x' END FROM t")
            .await,
    );
    assert!(
        message.to_lowercase().contains("expression"),
        "scalar subquery + expression must be rejected with an explicit message, got: {message}"
    );
}

/// 表达式项 + JOIN 显式拒绝（不得静默变形）
#[tokio::test]
async fn join_with_expression_rejected() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE a (id INT PRIMARY KEY)").await;
    exec_ok(&db, "CREATE TABLE b (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO a VALUES (1)").await;
    exec_ok(&db, "INSERT INTO b VALUES (1)").await;

    let message = error_message(
        db.execute_sql("SELECT a.id, CASE WHEN 1 = 1 THEN 'x' END FROM a JOIN b ON a.id = b.id")
            .await,
    );
    assert!(
        message.to_lowercase().contains("expression") || message.contains("Unsupported"),
        "JOIN + expression must be rejected explicitly, got: {message}"
    );
}

// ---------------------------------------------------------------------------
// ORDER BY 与表达式项组合（排序键为基础列，全行流过 Sort、顶层统一求值）
// ---------------------------------------------------------------------------

/// ORDER BY 基础列 + 表达式项：排序正确且派生列逐行求值
#[tokio::test]
async fn order_by_base_column_with_expression() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3)").await;

    let rows = query_rows(
        db.execute_sql(
            "SELECT id, CASE WHEN id >= 2 THEN 'hi' ELSE 'lo' END AS tag FROM t ORDER BY id DESC",
        )
        .await,
    );
    assert_eq!(
        rows,
        vec![
            vec![serde_json::json!(3), serde_json::json!("hi")],
            vec![serde_json::json!(2), serde_json::json!("hi")],
            vec![serde_json::json!(1), serde_json::json!("lo")],
        ]
    );
}
