//! MS25 Iteration 001 / Cycle 000：集合操作（Task T3，design D4）
//!
//! change: 2026-09-26-ms25-sql-read-surface
//!
//! 病灶：`extract_select_body` 只放行 `SetExpr::Select`，复合语句整体计划期
//! 拒绝（`UnsupportedStatement("SET OPERATION")`）——UNION / INTERSECT /
//! EXCEPT 全不可达。
//!
//! 目标语义（delta spec `sql-set-operations`，SQLite 对齐基准）：
//! - R1 `UNION` 去重并集（NULL 相等）、`UNION ALL` 保留重复与序。
//! - R2 `INTERSECT` 去重交集、`EXCEPT` 去重差集（NULL 参与匹配）。
//! - R3 同级左结合、`INTERSECT` 优先级更高；`INTERSECT/EXCEPT ALL` 与括号化
//!   侧臂点名拒绝。
//! - R4 复合语句尾 ORDER BY / LIMIT 作用于复合输出；列数不匹配点名拒绝；
//!   既有单查询面零回归。
//!
//! 计划形状：侧臂经合成 `Query` 复用 `build_query`（可含 JOIN / 外连接 /
//! 聚合 / 表达式项 / WHERE——D2 组合布局全量复用），复合节点列名取左侧列名；
//! 去重与匹配共用 `HashSet<Vec<Value>>`（与 `DistinctExecutor` 同一内核）。

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

/// R1-S1/R1-S3 种子：`t1(v INT)` 含 1,2；`t2(v INT)` 含 2,3。
async fn seed_two_int_tables(db: &Database) {
    exec_ok(db, "CREATE TABLE t1 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(db, "INSERT INTO t1 VALUES (1, 1)").await;
    exec_ok(db, "INSERT INTO t1 VALUES (2, 2)").await;
    exec_ok(db, "CREATE TABLE t2 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(db, "INSERT INTO t2 VALUES (1, 2)").await;
    exec_ok(db, "INSERT INTO t2 VALUES (2, 3)").await;
}

/// R1-S1：UNION 去重并集（{1,2} ∪ {2,3} → 1,2,3）。
#[tokio::test]
async fn union_dedup() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_two_int_tables(&db).await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT v FROM t1 UNION SELECT v FROM t2")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1)], vec![json!(2)], vec![json!(3)]],
        "UNION 必须输出去重并集并保留首现序"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S2：UNION ALL 保留重复与序（4 行：1,2,2,3）。
#[tokio::test]
async fn union_all_preserves() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_two_int_tables(&db).await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT v FROM t1 UNION ALL SELECT v FROM t2")
            .await,
    ));
    assert_eq!(
        rows,
        vec![
            vec![json!(1)],
            vec![json!(2)],
            vec![json!(2)],
            vec![json!(3)]
        ],
        "UNION ALL 必须保留重复行与输入序"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R1-S3：UNION 中 NULL 相等折叠（两支各一行 NULL → 一行）。
#[tokio::test]
async fn union_null_folds() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE n1 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO n1 VALUES (1, NULL)").await;
    exec_ok(&db, "CREATE TABLE n2 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO n2 VALUES (1, NULL)").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT v FROM n1 UNION SELECT v FROM n2")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(null)]],
        "UNION 必须把两支的 NULL 折叠为一行"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R2-S1：INTERSECT 去重交集。
#[tokio::test]
async fn intersect_basic() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t1 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (1, 1)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (2, 2)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (3, 2)").await;
    exec_ok(&db, "CREATE TABLE t2 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t2 VALUES (1, 2)").await;
    exec_ok(&db, "INSERT INTO t2 VALUES (2, 3)").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT v FROM t1 INTERSECT SELECT v FROM t2")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(2)]],
        "INTERSECT 必须输出去重交集（左侧 1,2,2 与右侧 2,3 → 仅 2）"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R2-S2：EXCEPT 去重差集。
#[tokio::test]
async fn except_basic() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t1 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (1, 1)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (2, 2)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (3, 2)").await;
    exec_ok(&db, "CREATE TABLE t2 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t2 VALUES (1, 2)").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT v FROM t1 EXCEPT SELECT v FROM t2")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1)]],
        "EXCEPT 必须输出去重差集（左侧 1,2,2 减去右侧 2 → 仅 1）"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R2-S3：EXCEPT 中 NULL 参与匹配（NULL 行被差集移除）。
#[tokio::test]
async fn except_null_matching() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t1 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (1, NULL)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (2, 1)").await;
    exec_ok(&db, "CREATE TABLE t2 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t2 VALUES (1, NULL)").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT v FROM t1 EXCEPT SELECT v FROM t2")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1)]],
        "EXCEPT 必须按 NULL 相等语义移除 NULL 行"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S1：链式三分支左结合去重。
#[tokio::test]
async fn chained_three_way() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t1 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (1, 1)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (2, 2)").await;
    exec_ok(&db, "CREATE TABLE t2 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t2 VALUES (1, 2)").await;
    exec_ok(&db, "INSERT INTO t2 VALUES (2, 3)").await;
    exec_ok(&db, "CREATE TABLE t3 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t3 VALUES (1, 3)").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT v FROM t1 UNION SELECT v FROM t2 UNION SELECT v FROM t3")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1)], vec![json!(2)], vec![json!(3)]],
        "三分支链式复合必须左结合去重"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S2：INTERSECT 优先级高于 UNION（`a UNION b INTERSECT c` = `a UNION (b ∩ c)`）。
#[tokio::test]
async fn intersect_precedence() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE a (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO a VALUES (1, 1)").await;
    exec_ok(&db, "INSERT INTO a VALUES (2, 2)").await;
    exec_ok(&db, "CREATE TABLE b (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO b VALUES (1, 2)").await;
    exec_ok(&db, "INSERT INTO b VALUES (2, 3)").await;
    exec_ok(&db, "CREATE TABLE c (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO c VALUES (1, 3)").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT v FROM a UNION SELECT v FROM b INTERSECT SELECT v FROM c")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1)], vec![json!(2)], vec![json!(3)]],
        "INTERSECT 必须先于 UNION 结合（b 与 c 先得 3，再与 a 求并）"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S3：`INTERSECT ALL` 与括号化侧臂计划期点名拒绝（断言点名文案，避免
/// 「因其他原因报错」的假通过）。
#[tokio::test]
async fn intersect_all_and_parens_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_two_int_tables(&db).await;

    let all_msg = error_msg(to_response(
        db.execute_sql("SELECT v FROM t1 INTERSECT ALL SELECT v FROM t2")
            .await,
    ));
    assert!(
        all_msg.contains("INTERSECT ALL"),
        "INTERSECT ALL 必须点名拒绝，实际: {all_msg}"
    );

    let paren_msg = error_msg(to_response(
        db.execute_sql("(SELECT v FROM t1) UNION (SELECT v FROM t2)")
            .await,
    ));
    assert!(
        paren_msg.to_lowercase().contains("parenthesized"),
        "括号化侧臂必须点名拒绝，实际: {paren_msg}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R3-S4：其余拒绝面——`BY NAME` 量化词、顶层 DISTINCT × 复合、子查询内复合。
#[tokio::test]
async fn compound_other_forms_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_two_int_tables(&db).await;

    let by_name = error_msg(to_response(
        db.execute_sql("SELECT v FROM t1 UNION BY NAME SELECT v FROM t2")
            .await,
    ));
    assert!(
        by_name.contains("BY NAME"),
        "BY NAME 量化词必须点名拒绝，实际: {by_name}"
    );

    let with_distinct = error_msg(to_response(
        db.execute_sql("SELECT DISTINCT v FROM t1 UNION SELECT v FROM t2")
            .await,
    ));
    assert!(
        with_distinct.contains("DISTINCT"),
        "DISTINCT × 复合必须点名拒绝，实际: {with_distinct}"
    );

    // 嵌套分支上的 DISTINCT 同样拒绝（否则该分支被静默去重）
    let nested_distinct = error_msg(to_response(
        db.execute_sql("SELECT v FROM t1 UNION SELECT DISTINCT v FROM t2 UNION SELECT v FROM t1")
            .await,
    ));
    assert!(
        nested_distinct.contains("DISTINCT"),
        "嵌套分支的 DISTINCT 必须点名拒绝，实际: {nested_distinct}"
    );

    let in_subquery = error_msg(to_response(
        db.execute_sql("SELECT v FROM t1 WHERE v IN (SELECT v FROM t2 UNION SELECT v FROM t1)")
            .await,
    ));
    assert!(
        in_subquery.to_lowercase().contains("subquer"),
        "子查询内复合必须点名拒绝，实际: {in_subquery}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R4-S1：复合结果 ORDER BY（作用于复合输出）。
#[tokio::test]
async fn compound_order_by() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t1 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (1, 2)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (2, 1)").await;
    exec_ok(&db, "CREATE TABLE t2 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t2 VALUES (1, 3)").await;

    let rows = query_rows(to_response(
        db.execute_sql("SELECT v FROM t1 UNION SELECT v FROM t2 ORDER BY v DESC")
            .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(3)], vec![json!(2)], vec![json!(1)]],
        "复合语句尾 ORDER BY 必须作用于复合输出"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R4-S2：分支列数不匹配计划期点名拒绝。
#[tokio::test]
async fn column_count_mismatch_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t1 (k INT PRIMARY KEY, a INT, b INT)").await;
    exec_ok(&db, "INSERT INTO t1 VALUES (1, 1, 2)").await;
    exec_ok(&db, "CREATE TABLE t2 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(&db, "INSERT INTO t2 VALUES (1, 9)").await;

    let msg = error_msg(to_response(
        db.execute_sql("SELECT a, b FROM t1 UNION SELECT v FROM t2")
            .await,
    ));
    assert!(
        msg.to_lowercase().contains("column count"),
        "列数不匹配的复合必须点名拒绝并说明列数，实际: {msg}"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// R4-S3：既有单查询形态零回归。
#[tokio::test]
async fn single_query_unchanged() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_two_int_tables(&db).await;

    let rows = query_rows(to_response(db.execute_sql("SELECT v FROM t1").await));
    assert_eq!(
        rows,
        vec![vec![json!(1)], vec![json!(2)]],
        "单查询形态必须逐值不变"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// 侧臂复用既有装配：JOIN + WHERE（D2 组合形状）× 外连接的复合查询。
#[tokio::test]
async fn compound_arms_with_join_and_where() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE l (id INT PRIMARY KEY, tag VARCHAR)").await;
    exec_ok(&db, "INSERT INTO l VALUES (1, 'x')").await;
    exec_ok(&db, "INSERT INTO l VALUES (2, 'y')").await;
    exec_ok(
        &db,
        "CREATE TABLE r (k INT PRIMARY KEY, rid INT, tag VARCHAR)",
    )
    .await;
    exec_ok(&db, "INSERT INTO r VALUES (1, 1, 'p')").await;
    exec_ok(&db, "INSERT INTO r VALUES (2, 9, 'q')").await;

    let rows = query_rows(to_response(
        db.execute_sql(
            "SELECT l.id FROM l JOIN r ON l.id = r.rid WHERE r.tag = 'p' \
             UNION SELECT rid FROM r WHERE tag = 'q'",
        )
        .await,
    ));
    assert_eq!(
        rows,
        vec![vec![json!(1)], vec![json!(9)]],
        "复合侧臂必须能承载 JOIN/WHERE 组合形状的完整 SELECT"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// === T5（change 2026-10-04-external-review-gaps）：FROM 派生表内集合操作 ===
//
// R5：`FROM (<SELECT> UNION [ALL]/INTERSECT/EXCEPT <SELECT>)` 作派生表源
// （列名取左臂，外层叠加照常）；IN/EXISTS/标量子查询内的集合操作维持点名
// 拒绝（S21）。

/// T5 派生表种子：`t1(v INT)` 含 1,2；`t2(v INT)` 含 2,3。
async fn seed_derived_set_tables(db: &Database) {
    exec_ok(db, "CREATE TABLE s1 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(db, "INSERT INTO s1 VALUES (1, 1)").await;
    exec_ok(db, "INSERT INTO s1 VALUES (2, 2)").await;
    exec_ok(db, "CREATE TABLE s2 (k INT PRIMARY KEY, v INT)").await;
    exec_ok(db, "INSERT INTO s2 VALUES (1, 2)").await;
    exec_ok(db, "INSERT INTO s2 VALUES (2, 3)").await;
}

/// S19：UNION 去重并集作派生表源。
#[tokio::test]
async fn derived_union_dedup_source() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_derived_set_tables(&db).await;
    let r = query_rows(to_response(
        db.execute_sql("SELECT COUNT(*) FROM (SELECT v FROM s1 UNION SELECT v FROM s2)")
            .await,
    ));
    assert_eq!(r, vec![vec![json!(3)]]);
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S19：UNION ALL 保留重复。
#[tokio::test]
async fn derived_union_all_source() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_derived_set_tables(&db).await;
    let r = query_rows(to_response(
        db.execute_sql("SELECT COUNT(*) FROM (SELECT v FROM s1 UNION ALL SELECT v FROM s2)")
            .await,
    ));
    assert_eq!(r, vec![vec![json!(4)]]);
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S19：INTERSECT / EXCEPT 作派生表源。
#[tokio::test]
async fn derived_intersect_except_source() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_derived_set_tables(&db).await;
    let r = query_rows(to_response(
        db.execute_sql("SELECT v FROM (SELECT v FROM s1 INTERSECT SELECT v FROM s2)")
            .await,
    ));
    assert_eq!(r, vec![vec![json!(2)]]);
    let r = query_rows(to_response(
        db.execute_sql("SELECT v FROM (SELECT v FROM s1 EXCEPT SELECT v FROM s2)")
            .await,
    ));
    assert_eq!(r, vec![vec![json!(1)]]);
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S19/S20：列名取左臂；外层 WHERE / ORDER BY 叠加照常。
#[tokio::test]
async fn derived_set_source_outer_composition() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_derived_set_tables(&db).await;
    let r = query_rows(to_response(
        db.execute_sql(
            "SELECT v FROM (SELECT v FROM s1 UNION SELECT v FROM s2) \
             WHERE v > 1 ORDER BY v DESC",
        )
        .await,
    ));
    assert_eq!(r, vec![vec![json!(3)], vec![json!(2)]]);
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S21：IN 子查询内集合操作维持点名拒绝（文案逐字节不变）。
#[tokio::test]
async fn set_op_inside_in_subquery_still_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_derived_set_tables(&db).await;
    let msg = error_msg(to_response(
        db.execute_sql("SELECT k FROM s1 WHERE v IN (SELECT v FROM s1 UNION SELECT v FROM s2)")
            .await,
    ));
    assert!(
        msg.contains("Set operations are not supported inside subqueries"),
        "unexpected error: {msg}"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S21：EXISTS 子查询内集合操作维持点名拒绝。
#[tokio::test]
async fn set_op_inside_exists_subquery_still_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_derived_set_tables(&db).await;
    let msg = error_msg(to_response(
        db.execute_sql("SELECT k FROM s1 WHERE EXISTS (SELECT v FROM s1 UNION SELECT v FROM s2)")
            .await,
    ));
    assert!(
        msg.contains("Set operations are not supported inside subqueries"),
        "unexpected error: {msg}"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// S21：WHERE 比较腿标量子查询内集合操作维持点名拒绝（T4 通道同面）。
#[tokio::test]
async fn set_op_inside_scalar_comparison_still_rejected() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    seed_derived_set_tables(&db).await;
    let msg = error_msg(to_response(
        db.execute_sql("SELECT k FROM s1 WHERE v = (SELECT v FROM s1 UNION SELECT v FROM s2)")
            .await,
    ));
    assert!(
        msg.contains("Set operations are not supported inside subqueries"),
        "unexpected error: {msg}"
    );
    db.wal_buffer.as_ref().unwrap().shutdown().await;
}
