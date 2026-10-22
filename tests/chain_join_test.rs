//! T11（change 2026-10-04-external-review-gaps）— 多表链式 JOIN 装配正确性
//! （R11，场景 S42–S46）。
//!
//! 修复前三形态全坏：hash+hash 链与 hash+NLJ 混合链计划期 `Column not
//! found`（第三表列在第一步严格校验点名）；仅前两表 SELECT `join.rs` 键查表
//! panic；链上同名列条件键裸名查表静默串列。两表单步全形态为逐字节零回归
//! 对照（S46，先确认 GREEN）。

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use tempfile::tempdir;

/// Helper: open an in-memory database.
async fn open_db() -> Database {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");
    Database::open(&path).await.unwrap()
}

/// Helper: execute SQL and return the Response.
async fn exec(db: &Database, sql: &str) -> Response {
    to_response(db.execute_sql(sql).await)
}

/// Helper: extract rows from a QueryResult, panicking if not QueryResult.
fn rows(resp: Response) -> Vec<Vec<serde_json::Value>> {
    match resp {
        Response::QueryResult { rows } => rows,
        Response::AffectedRows { count } => {
            panic!("expected QueryResult, got AffectedRows({count})")
        }
        Response::Error { message, .. } => {
            panic!("expected QueryResult, got Error: {message}")
        }
        Response::Pong => panic!("expected QueryResult, got Pong"),
    }
}

/// Helper: 链夹具——三表各有同名列 `id`（S45 串列见证面），载荷值与键值
/// 错位（错位索引读到载荷即失配）。
/// t1(id,a): (10,x1),(20,x2)；t2(id,b): (10,y1),(20,y2)；t3(id,c): (10,z1),(20,z2)
async fn setup_chain(db: &Database) {
    exec(db, "CREATE TABLE t1 (id INT, a TEXT)").await;
    exec(db, "INSERT INTO t1 VALUES (10, 'x1')").await;
    exec(db, "INSERT INTO t1 VALUES (20, 'x2')").await;
    exec(db, "CREATE TABLE t2 (id INT, b TEXT)").await;
    exec(db, "INSERT INTO t2 VALUES (10, 'y1')").await;
    exec(db, "INSERT INTO t2 VALUES (20, 'y2')").await;
    exec(db, "CREATE TABLE t3 (id INT, c TEXT)").await;
    exec(db, "INSERT INTO t3 VALUES (10, 'z1')").await;
    exec(db, "INSERT INTO t3 VALUES (20, 'z2')").await;
}

// === S46: 两表单步零回归对照（修复前即 GREEN） ===

#[tokio::test]
async fn s46_two_table_select_star() {
    let db = open_db().await;
    setup_chain(&db).await;
    let r = rows(
        exec(
            &db,
            "SELECT * FROM t1 JOIN t2 ON t1.id = t2.id ORDER BY t1.id",
        )
        .await,
    );
    assert_eq!(
        r,
        vec![
            vec![
                serde_json::json!(10),
                serde_json::json!("x1"),
                serde_json::json!(10),
                serde_json::json!("y1")
            ],
            vec![
                serde_json::json!(20),
                serde_json::json!("x2"),
                serde_json::json!(20),
                serde_json::json!("y2")
            ],
        ]
    );
}

#[tokio::test]
async fn s46_two_table_where_filter() {
    let db = open_db().await;
    setup_chain(&db).await;
    let r = rows(
        exec(
            &db,
            "SELECT t2.b FROM t1 JOIN t2 ON t1.id = t2.id WHERE t1.id = 10",
        )
        .await,
    );
    assert_eq!(r, vec![vec![serde_json::json!("y1")]]);
}

#[tokio::test]
async fn s46_two_table_aggregate() {
    let db = open_db().await;
    setup_chain(&db).await;
    let r = rows(
        exec(
            &db,
            "SELECT COUNT(*), MAX(t2.b) FROM t1 JOIN t2 ON t1.id = t2.id",
        )
        .await,
    );
    assert_eq!(r, vec![vec![serde_json::json!(2), serde_json::json!("y2")]]);
}

/// S46：真未知列错误面不变（JOIN 链上不存在的列计划期点名）。
#[tokio::test]
async fn s46_unknown_column_still_rejected() {
    let db = open_db().await;
    setup_chain(&db).await;
    let resp = exec(&db, "SELECT t1.nosuch FROM t1 JOIN t2 ON t1.id = t2.id").await;
    match resp {
        Response::Error { message, .. } => {
            assert!(message.contains("nosuch"), "unexpected error: {message}");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

// === S42: hash+hash 三表链 ===

/// S42：三表等值链端到端可达且每步配对正确（修复前：计划期 `Column not
/// found`）；链上同名列 `id` 条件键不串列（S45 同面——三表键同名，载荷
/// 与键错位，任何串列/缺列都失配）。
#[tokio::test]
async fn s42_hash_hash_three_table_chain() {
    let db = open_db().await;
    setup_chain(&db).await;
    let r = rows(
        exec(
            &db,
            "SELECT t1.a, t2.b, t3.c FROM t1 JOIN t2 ON t1.id = t2.id \
             JOIN t3 ON t2.id = t3.id ORDER BY t1.a",
        )
        .await,
    );
    assert_eq!(
        r,
        vec![
            vec![
                serde_json::json!("x1"),
                serde_json::json!("y1"),
                serde_json::json!("z1")
            ],
            vec![
                serde_json::json!("x2"),
                serde_json::json!("y2"),
                serde_json::json!("z2")
            ],
        ]
    );
}

/// S42：链中间列仅作键、SELECT 只取首尾载荷（键位重定基 witness——左链
/// 第二表列在组合行内的绝对偏移读取）。
#[tokio::test]
async fn s42_chain_middle_key_absolute_offset() {
    let db = open_db().await;
    setup_chain(&db).await;
    let r = rows(
        exec(
            &db,
            "SELECT t1.a, t3.c FROM t1 JOIN t2 ON t1.id = t2.id \
             JOIN t3 ON t2.id = t3.id ORDER BY t3.c",
        )
        .await,
    );
    assert_eq!(
        r,
        vec![
            vec![serde_json::json!("x1"), serde_json::json!("z1")],
            vec![serde_json::json!("x2"), serde_json::json!("z2")],
        ]
    );
}

// === S43: hash+NLJ 混合链 ===

/// S43：第三步非等值形态（`t3.id + 0` 非结构列引用 → NLJ 路由），谓词作用
/// 全组合行（修复前：计划期 `Column not found`）。
#[tokio::test]
async fn s43_hash_nlj_mixed_chain() {
    let db = open_db().await;
    setup_chain(&db).await;
    let r = rows(
        exec(
            &db,
            "SELECT t1.a, t2.b, t3.c FROM t1 JOIN t2 ON t1.id = t2.id \
             JOIN t3 ON t2.id = t3.id + 0 ORDER BY t1.a",
        )
        .await,
    );
    assert_eq!(
        r,
        vec![
            vec![
                serde_json::json!("x1"),
                serde_json::json!("y1"),
                serde_json::json!("z1")
            ],
            vec![
                serde_json::json!("x2"),
                serde_json::json!("y2"),
                serde_json::json!("z2")
            ],
        ]
    );
}

// === S44: 仅前两表 SELECT（修复前 join.rs panic exit 101） ===

#[tokio::test]
async fn s44_partial_select_no_panic() {
    let db = open_db().await;
    setup_chain(&db).await;
    let r = rows(
        exec(
            &db,
            "SELECT t1.a, t2.b FROM t1 JOIN t2 ON t1.id = t2.id \
             JOIN t3 ON t2.id = t3.id ORDER BY t1.a",
        )
        .await,
    );
    assert_eq!(
        r,
        vec![
            vec![serde_json::json!("x1"), serde_json::json!("y1")],
            vec![serde_json::json!("x2"), serde_json::json!("y2")],
        ]
    );
}

// === EXPLAIN 计划形状见证 ===

/// 链计划可渲染：EXPLAIN 不 panic 且按步呈现 JOIN 节点。
#[tokio::test]
async fn chain_explain_renders() {
    let db = open_db().await;
    setup_chain(&db).await;
    let r = rows(
        exec(
            &db,
            "EXPLAIN SELECT t1.a FROM t1 JOIN t2 ON t1.id = t2.id \
             JOIN t3 ON t2.id = t3.id",
        )
        .await,
    );
    let plan: Vec<&str> = r.iter().map(|row| row[0].as_str().unwrap()).collect();
    let text = plan.join("\n");
    let join_count = text.matches("Join").count();
    assert!(join_count >= 2, "expected two join nodes in plan:\n{text}");
}
