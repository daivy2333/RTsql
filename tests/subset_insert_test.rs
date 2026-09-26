//! MS24 Iteration 000：子集列清单 INSERT 与 DEFAULT 应用（design D1/D2）
//!
//! change: 2026-09-25-ms24-write-surface-completion / tasks 1.4-1.6
//!
//! 缺陷：DEFAULT 声明解析进 ColumnSchema 后被建表路径整体丢弃（不持久化、
//! 不消费、dump 不渲染）；子集列清单被 `map_insert_values`「恰为全列排列」
//! 计划期拒绝；`VALUES` 中的 DEFAULT 关键字被 `UnsupportedValue` 拒绝。
//!
//! 目标语义（spec sql-write-surface R1）：显式列清单 SHALL 接受表列任意子集；
//! 省略列取声明 DEFAULT（无则 NULL）；`DEFAULT` 关键字等价省略；声明 NOT
//! NULL 且无 DEFAULT 的省略列由既有执行器门零副作用拒绝；DEFAULT 经 catalog
//! 持久化跨重启生效（dump/schema 渲染见 cli_test 增量）；全列清单与无清单
//! 行为不变。dump→restore 往返保真由 cli_test 增量见证（S6）。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use std::path::PathBuf;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

fn expect_error(resp: Response, what: &str) -> String {
    match resp {
        Response::Error { message } => message,
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

async fn assert_count(db: &Database, sql: &str, expected: serde_json::Value, what: &str) {
    match db.execute_sql(sql).await {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0], expected, "{what}");
        }
        other => panic!("{what} 应返回 QueryResult，实际 {other:?}"),
    }
}

async fn setup(db: &Database) {
    expect_affected(
        db.execute_sql(
            "CREATE TABLE t (id INT PRIMARY KEY, name STRING DEFAULT 'anon', score INT)",
        )
        .await,
        "建表",
    );
}

/// R1-S1：子集清单省略列取声明 DEFAULT。
///
/// RED（修复前实测）：子集清单被计划期数量校验拒绝
/// （「expects 3 values, got 2」）。
#[tokio::test]
async fn subset_insert_omitted_column_takes_declared_default() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(
        db.execute_sql("INSERT INTO t (id, score) VALUES (1, 90)")
            .await,
        "子集 INSERT（省略 name）",
    );

    let rows = query_rows(
        db.execute_sql("SELECT id, name, score FROM t").await,
        "行集查询",
    );
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!(1),
            serde_json::json!("anon"),
            serde_json::json!(90),
        ]],
        "省略列必须取声明 DEFAULT（name='anon'），实际: {rows:?}"
    );

    db.wal_buffer.shutdown().await;
}

/// R1-S2：子集清单省略无 DEFAULT 的可空列取 NULL。
///
/// RED（修复前实测）：同源计划期拒绝。
#[tokio::test]
async fn subset_insert_omitted_nullable_column_takes_null() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(
        db.execute_sql("INSERT INTO t (id) VALUES (2)").await,
        "子集 INSERT（省略 name/score）",
    );

    let rows = query_rows(
        db.execute_sql("SELECT id, name, score FROM t WHERE id = 2")
            .await,
        "行集查询",
    );
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!(2),
            serde_json::json!("anon"),
            serde_json::json!(null),
        ]],
        "省略列取 DEFAULT（name）/NULL（score），实际: {rows:?}"
    );

    db.wal_buffer.shutdown().await;
}

/// R1-S3：省略 NOT NULL 且无 DEFAULT 的列被既有执行器门零副作用拒绝。
///
/// RED（修复前实测）：同源计划期拒绝（错误文本为数量不符而非点名）。
#[tokio::test]
async fn subset_insert_omitted_not_null_column_rejected_zero_side_effects() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();

    expect_affected(
        db.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, name STRING NOT NULL)")
            .await,
        "建表",
    );

    let message = expect_error(
        db.execute_sql("INSERT INTO t (id) VALUES (1)").await,
        "省略 NOT NULL 列的子集 INSERT",
    );
    assert!(
        message.contains("name"),
        "错误文本必须点名列名 name，实际: {message}"
    );

    assert_count(
        &db,
        "SELECT COUNT(*) FROM t",
        serde_json::json!(0),
        "被拒绝的 INSERT 不得落库",
    )
    .await;

    db.wal_buffer.shutdown().await;
}

/// R1-S4：VALUES 中的 DEFAULT 关键字等价省略——取声明 DEFAULT 或 NULL。
///
/// RED（修复前实测）：DEFAULT 关键字被 `UnsupportedValue` 拒绝。
#[tokio::test]
async fn default_keyword_in_values_takes_declared_default_or_null() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    expect_affected(
        db.execute_sql("INSERT INTO t (id, name, score) VALUES (1, DEFAULT, DEFAULT)")
            .await,
        "DEFAULT 关键字 INSERT",
    );

    let rows = query_rows(
        db.execute_sql("SELECT id, name, score FROM t").await,
        "行集查询",
    );
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!(1),
            serde_json::json!("anon"),
            serde_json::json!(null),
        ]],
        "DEFAULT 关键字必须取声明 DEFAULT（name）/NULL（score），实际: {rows:?}"
    );

    db.wal_buffer.shutdown().await;
}

/// R1-S5：DEFAULT 跨重启生效——干净关闭重开后再子集 INSERT，省略列取
/// 声明 DEFAULT（catalog 持久化语义与建表会话内一致）。
///
/// RED（修复前实测）：DEFAULT 不持久化，重开后 TableMeta 无 defaults，
/// 省略位填 NULL。
#[tokio::test]
async fn declared_default_survives_restart() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    {
        let db = Database::open(&path).await.unwrap();
        setup(&db).await;
        db.close().await.unwrap();
    }

    let db2 = Database::open(&path).await.unwrap();
    expect_affected(
        db2.execute_sql("INSERT INTO t (id, score) VALUES (3, 60)")
            .await,
        "重开后的子集 INSERT",
    );

    let rows = query_rows(
        db2.execute_sql("SELECT id, name, score FROM t WHERE id = 3")
            .await,
        "重开后行集查询",
    );
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!(3),
            serde_json::json!("anon"),
            serde_json::json!(60),
        ]],
        "重开后省略列必须仍取声明 DEFAULT，实际: {rows:?}"
    );

    db2.wal_buffer.shutdown().await;
}

/// R1-S8 锚点：全列清单与无清单 INSERT 行为不变（含 MS16 乱序映射语义），
/// 子集化放宽不改变既有形态。
#[tokio::test]
async fn full_list_and_listless_inserts_unchanged() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    setup(&db).await;

    // 全列清单（乱序映射既有语义）
    expect_affected(
        db.execute_sql("INSERT INTO t (score, name, id) VALUES (70, 'bob', 4)")
            .await,
        "乱序全列清单 INSERT",
    );
    // 无清单
    expect_affected(
        db.execute_sql("INSERT INTO t VALUES (5, 'cat', 80)").await,
        "无清单 INSERT",
    );

    let rows = query_rows(
        db.execute_sql("SELECT id, name, score FROM t").await,
        "行集查询",
    );
    assert_eq!(
        rows,
        vec![
            vec![
                serde_json::json!(4),
                serde_json::json!("bob"),
                serde_json::json!(70)
            ],
            vec![
                serde_json::json!(5),
                serde_json::json!("cat"),
                serde_json::json!(80)
            ],
        ],
        "全列/无清单形态必须保持既有语义，实际: {rows:?}"
    );

    // 无清单行长度不符仍计划期拒绝（既有意图）
    let message = expect_error(
        db.execute_sql("INSERT INTO t VALUES (6, 'dog')").await,
        "无清单行长度不符 INSERT",
    );
    assert!(
        message.contains("expects 3") && message.contains("got 2"),
        "无清单行长度拒绝文案必须保持，实际: {message}"
    );

    db.wal_buffer.shutdown().await;
}
