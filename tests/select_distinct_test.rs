//! MS25 Iteration 001 / Cycle 000：SELECT DISTINCT（Task T2，design D3）
//!
//! change: 2026-09-26-ms25-sql-read-surface
//!
//! 病灶：`Query.distinct` 量化词在 planner 全无消费——`SELECT DISTINCT` 静默
//! 返回重复行（不报错，MS15/MS16「静默错误结果」红线同类）。
//!
//! 目标语义（delta spec `select-distinct`）：
//! - R1 按输出行去重（表达式项按求值后值；NULL 彼此相等折叠），首现序保留。
//! - R2 ORDER BY 键 SHALL ∈ 输出列集（非输出项点名拒绝）；与聚合输出组合按
//!   输出行去重。
//! - R3 `DISTINCT ON(...)` 点名拒绝；`ALL` 量化词等价既有不去重；既有查询
//!   零回归。
//!
//! 链位（design D3）：`base → [Sort] → [Projection] → Distinct → Limit`——
//! DISTINCT 作用于**已就绪的输出行形状**（纯列清单在 Sort 之下即输出形状；
//! 表达式项须待顶层 Projection 求值后），LIMIT 随之上移到 DISTINCT 之上。
//! 标量子查询项 × DISTINCT 计划期点名拒绝（SubqueryEval 在 Limit 之上插列，
//! 链位冲突）。

#![cfg(feature = "network")]
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

/// R1-S1：单列去重（'a','a','b' → 'a','b'；现状 RED：静默 3 行）。
#[tokio::test]
async fn distinct_single_col() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (v VARCHAR)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('a')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('a')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('b')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT DISTINCT v FROM t").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!("a")], vec![json!("b")]],
        "SELECT DISTINCT 必须按输出行去重并保留首现序"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S2：多列元组去重。
#[tokio::test]
async fn distinct_multi_col() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, a INT, b VARCHAR)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 1, 'x')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 1, 'x')").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 1, 'y')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT DISTINCT a, b FROM t").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1), json!("x")], vec![json!(1), json!("y")]],
        "多列 DISTINCT 必须按整行元组去重"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S3：NULL 相等折叠（NULL,NULL,1 → NULL 与 1 各一行）。
#[tokio::test]
async fn distinct_null_folds() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, NULL)").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, NULL)").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 1)").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT DISTINCT v FROM t").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(null)], vec![json!(1)]],
        "NULL 与 NULL 必须视为相等折叠为一行"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S4：表达式投影按求值后值去重（'ab','AB','cd' → 'AB','CD'）。
#[tokio::test]
async fn distinct_expression_dedup() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, name VARCHAR)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'ab')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'AB')").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 'cd')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT DISTINCT upper(name) FROM t").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!("AB")], vec![json!("CD")]],
        "DISTINCT 必须作用于表达式求值后的输出值"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R2-S1：DISTINCT + ORDER BY 输出项（排序作用于去重结果）。
#[tokio::test]
async fn distinct_order_by_output() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v VARCHAR)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'b')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'a')").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 'a')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT DISTINCT v FROM t ORDER BY v").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!("a")], vec![json!("b")]],
        "DISTINCT + ORDER BY 输出项必须先去重后排序"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R2-S2：DISTINCT + ORDER BY 非输出项计划期点名拒绝。
#[tokio::test]
async fn distinct_order_by_non_output_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, a INT, b INT)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 1, 1)").await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT DISTINCT a FROM t ORDER BY b").await,
    ));
    assert!(
        msg.to_lowercase().contains("distinct"),
        "DISTINCT 查询的 ORDER BY 非输出项必须点名拒绝，实际: {msg}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R2-S3：DISTINCT × 聚合输出按输出行去重（行形状与值不变）。
///
/// 见证说明：聚合执行器的输出行恒为「分组键 ++ 聚合值」，故不同分组的输出行
/// 天然不同——本用例断言 DISTINCT 形态的行流等于非 DISTINCT 行流的去重结果
/// （去重内核作用于聚合输出行、链位可达），不虚构不可达的折叠场景。
///
/// MS25 T9 校准（design D9，alpha delta `aggregate-output-shape-conformance`
/// R1/R2）：原用例断言「两组各 2 行的聚合输出行互异，去重后仍为 2 行」——该互异
/// 依赖分组键随行携带。MS25 T9 起未入选分组键不进输出行（表头/行形状守恒），
/// 两组的聚合值同为 2，DISTINCT 因此真正折叠为 1 行。
#[tokio::test]
async fn distinct_aggregate_output() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, g VARCHAR)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'a')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'a')").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 'b')").await;
    exec_ok(&db, "INSERT INTO t VALUES (4, 'b')").await;

    let plain = query_rows(to_response(
        db.execute_sql("SELECT count(*) FROM t GROUP BY g").await,
    ));
    let distinct = query_rows(to_response(
        db.execute_sql("SELECT DISTINCT count(*) FROM t GROUP BY g")
            .await,
    ));

    // 聚合分组经 HashMap 分桶，多组行序非确定（每次执行独立随机种子）→ 按
    // 行文本排序后比较集合。
    let sort_rows = |mut rows: Vec<Vec<serde_json::Value>>| {
        rows.sort_by_key(|r| r.iter().map(|v| v.to_string()).collect::<Vec<_>>());
        rows
    };
    let mut expected: Vec<Vec<serde_json::Value>> = Vec::new();
    for row in sort_rows(plain) {
        if !expected.contains(&row) {
            expected.push(row);
        }
    }
    assert_eq!(
        sort_rows(distinct),
        expected,
        "DISTINCT × 聚合必须按聚合输出行去重且行形状不变"
    );
    assert_eq!(
        expected.len(),
        1,
        "MS25 T9 起未入选分组键不进输出行：两组聚合值同为 2，DISTINCT 折叠为 1 行"
    );

    // MS25 T9 校准见证：未入选分组键不进输出行——两组各 1 值、聚合值同为 2。
    let shape = query_rows(to_response(
        db.execute_sql("SELECT count(*) FROM t GROUP BY g").await,
    ));
    assert_eq!(
        shape,
        vec![vec![json!(2)], vec![json!(2)]],
        "未入选分组键不进输出行，每行恰 1 值（形状守恒）"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// 链位：LIMIT 作用于去重**之后**的行数（'a','a','b' LIMIT 2 → a,b 两行）。
#[tokio::test]
async fn distinct_limit_above_dedup() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (v VARCHAR)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('a')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('a')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('b')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT DISTINCT v FROM t LIMIT 2").await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!("a")], vec![json!("b")]],
        "LIMIT 必须作用于去重后的行流（先去重后截断）"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// 链位：DISTINCT 作用于 JOIN 组合形状的输出行（同一 id 出现两次 → 折叠）。
#[tokio::test]
async fn distinct_over_join_result() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE l (id INT PRIMARY KEY)").await;
    exec_ok(&db, "INSERT INTO l VALUES (1)").await;
    exec_ok(&db, "INSERT INTO l VALUES (2)").await;
    exec_ok(
        &db,
        "CREATE TABLE r (k INT PRIMARY KEY, rid INT, val VARCHAR)",
    )
    .await;
    exec_ok(&db, "INSERT INTO r VALUES (1, 1, 'a')").await;
    exec_ok(&db, "INSERT INTO r VALUES (2, 1, 'b')").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT DISTINCT l.id FROM l JOIN r ON l.id = r.rid")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1)]],
        "DISTINCT 必须作用于 JOIN 组合形状的输出行（id=1 出现两次 → 一行）"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S1：`DISTINCT ON(...)` 计划期点名拒绝。
#[tokio::test]
async fn distinct_on_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, a INT, b INT)").await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT DISTINCT ON(a) a, b FROM t").await,
    ));
    assert!(
        msg.to_lowercase().contains("distinct"),
        "DISTINCT ON 必须点名拒绝，实际: {msg}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S2：`ALL` 量化词等价既有不去重行为。
#[tokio::test]
async fn all_quantifier_identity() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (v VARCHAR)").await;
    exec_ok(&db, "INSERT INTO t VALUES ('a')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('a')").await;
    exec_ok(&db, "INSERT INTO t VALUES ('b')").await;

    let all = query_rows(to_response(db.execute_sql("SELECT ALL v FROM t").await));
    let plain = query_rows(to_response(db.execute_sql("SELECT v FROM t").await));
    assert_eq!(
        all,
        vec![vec![json!("a")], vec![json!("a")], vec![json!("b")]],
        "SELECT ALL 必须输出 3 行（与普通 SELECT 一致）"
    );
    assert_eq!(all, plain, "SELECT ALL 必须与无量化词形态逐值一致");

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S3：既有非 DISTINCT 查询零回归（无量化词路径不新增任何节点）。
#[tokio::test]
async fn non_distinct_queries_unchanged() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v VARCHAR)").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'a')").await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'a')").await;
    exec_ok(&db, "INSERT INTO t VALUES (3, 'b')").await;

    let plain = query_rows(to_response(db.execute_sql("SELECT v FROM t").await));
    assert_eq!(
        plain,
        vec![vec![json!("a")], vec![json!("a")], vec![json!("b")]],
        "无 DISTINCT 量化词的查询必须逐值不变"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}
