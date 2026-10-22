//! MS25 Iteration 003 / Cycle 000：ORDER BY 输出别名排序与语料校准（Task T7，design D8）
//!
//! change: 2026-09-26-ms25-sql-read-surface
//!
//! 病灶：顶层投影项的输出别名（`ExprWithAlias`）在 ORDER BY 中计划期可解析
//! （`sort_columns ∪ alias_columns`），但既不解析为排序列也不被 SortExecutor
//! 消费——`compare_rows` 对列集里不存在的键 `position()` 得 `None` 后静默跳过，
//! 结果保持输入序且不报错（MS15/MS16「静默错误结果」红线同类）。
//!
//! 目标语义（delta spec `order-by-alias-sorting`）：
//! - R1 别名键按输出列真实排序：表达式项别名 / 普通列别名 / DESC / 多键混合。
//! - R2 输出名优先遮蔽：键与别名、输入列均可匹配时按输出列排序（别名 = 源列名、
//!   别名遮蔽源列两形态）；仅输入列可匹配的键维持既有列名键逐字节不变。
//! - R3 未知键 `ColumnNotFound` 保持；DISTINCT × 别名排序组合可达；既有排序面
//!   零回归（`scalar_function_test` R5/S5 依 delta 校准，见该文件白名单用例）。
//!
//! 实现载体：planner 在 `build_query` 的 ORDER BY 装配段把别名键解析为顶层
//! Projection 已编译的同款表达式（输出名优先），置入 `OrderByColumn.expr`；
//! SortExecutor 对表达式键逐行 decorate（owned `evaluate`，求值错误经 `Result`
//! 传播）→ 按键序比较 → undecorate 回输入行形态，再施加既有投影裁剪。

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use serde_json::json;
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

fn col(rows: &[Vec<serde_json::Value>], idx: usize) -> Vec<serde_json::Value> {
    rows.iter().map(|r| r[idx].clone()).collect()
}

/// R1-S1：表达式项别名按输出列真实排序。
///
/// RED（实施前实测）：`SELECT upper(name) AS u … ORDER BY u` 返回输入序
/// ['B','A']——别名键被 `compare_rows` 静默跳过。
#[tokio::test]
async fn alias_sorts_expression() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('b')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('a')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT upper(name) AS u FROM t ORDER BY u")
            .await,
    ));
    assert_eq!(
        col(&rows, 0),
        vec![json!("A"), json!("B")],
        "rows: {rows:?}"
    );
}

/// R1-S2：普通列别名的 DESC 键按输出列反序（覆盖 `asc` 翻转臂）。
#[tokio::test]
async fn alias_sorts_plain_desc() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT, name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'x')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'y')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT id, name AS n FROM t ORDER BY n DESC")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(2), json!("y")], vec![json!(1), json!("x")]],
        "rows: {rows:?}"
    );
}

/// R1-S3：多键混合——基础列键与普通列别名键共存，按键序生效。
///
/// a=1 组内输入序为 (1,2),(1,1)——若别名键未生效（退化为按 a 单键），组内保持
/// 插入序，结果不可区分。
#[tokio::test]
async fn mixed_keys() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, a INT, b INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 2, 1)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 1, 2)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 1, 1)").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT a, b AS second FROM t ORDER BY a, second")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(1), json!(1)],
            vec![json!(1), json!(2)],
            vec![json!(2), json!(1)]
        ],
        "rows: {rows:?}"
    );
}

/// R2-S1：输出名优先遮蔽源列——别名与基列同名时按键解析为输出项表达式
/// （`10 - x`），而非同名基列 `x`。
///
/// delta `order-by-alias-sorting` R2 的场景示例为 `x % 2`；本引擎的值表达式
/// 算术只支持 `+ - * /`（`%` 在计划期按 `UnsupportedExpression`
/// "ARITHMETIC OPERATOR %" 点名拒绝），故用同样非单调的 `10 - x` 承载同一
/// 语义断言（delta 措辞与实现的偏差记录在 Act Response）。
#[tokio::test]
async fn alias_shadows_source_column() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (x INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (5)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3)").await;
    exec_ok(&db, "INSERT INTO t VALUES (20)").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT 10 - x AS x FROM t ORDER BY x").await,
    ));
    // 按 10-x 排序（输出列）→ x=20,5,3 → [-10,5,7]；
    // 按基列 x 排序（列名键）→ x=3,5,20 → [7,5,-10]。两序可区分。
    assert_eq!(
        col(&rows, 0),
        vec![json!(-10), json!(5), json!(7)],
        "rows: {rows:?}"
    );
}

/// R2-S2：仅输入列可匹配的键维持既有列名键行为（纯列清单，无别名可解析）。
#[tokio::test]
async fn input_column_key_unchanged() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('b')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('a')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT name FROM t ORDER BY name").await,
    ));
    assert_eq!(
        col(&rows, 0),
        vec![json!("a"), json!("b")],
        "rows: {rows:?}"
    );
}

/// R3-S1：未知排序键仍点名 `ColumnNotFound`（SHALL NOT 静默跳过）。
#[tokio::test]
async fn unknown_key_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (name STRING)").await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT name FROM t ORDER BY nosuch").await,
    ));
    assert!(
        msg.contains("Column not found: nosuch"),
        "unexpected message: {msg}"
    );
}

/// R3-S2：DISTINCT × 别名排序组合可达（链位 `Sort → Projection → Distinct`：
/// 别名键在未投影的输入行上求值，去重后按输出行产出）。
///
/// 覆盖两条链位形态——普通列别名（别名重路由 → 表达式项通路）与表达式项别名。
#[tokio::test]
async fn distinct_alias_sort_combo() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (v STRING)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('b')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('a')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('b')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT DISTINCT v AS val FROM t ORDER BY val")
            .await,
    ));
    assert_eq!(
        col(&rows, 0),
        vec![json!("a"), json!("b")],
        "rows: {rows:?}"
    );

    let rows = query_rows(to_response(
        db.execute_sql("SELECT DISTINCT upper(v) AS ug FROM t ORDER BY ug")
            .await,
    ));
    assert_eq!(
        col(&rows, 0),
        vec![json!("A"), json!("B")],
        "rows: {rows:?}"
    );
}
