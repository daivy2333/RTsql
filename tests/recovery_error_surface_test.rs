//! change 2026-10-03-practical-core-gaps T11（R6）：恢复重放目标表缺失的
//! 定向错误面。
//!
//! S22：lib 打开 → CREATE TABLE（DDL 不写 WAL，仅 catalog 脏页）→ INSERT
//! 提交（WAL 追加）→ 句柄 **drop 不 close()**（脏页丢失）→ 重新 open：
//! 错误须点名表名、陈述 catalog 缺失事实、陈述最可能原因（上次关闭未经
//! checkpoint，如 lib 句柄未 close() 即 drop、未持久化 DDL 随脏页丢失）并
//! 明示打开拒绝（修复前为归并文案 `WAL redo failed: table 't' lookup
//! failed during redo: table not found: t`——把「DDL 从未持久化」误诊为
//! 「redo 目标表缺失」）。打开仍失败：不跳过、不丢弃不可重放记录（K05）。
//!
//! S23 对照：同序列走 `close()` → open 成功、行可见——定向错误面只覆盖
//! 无 checkpoint 关闭形态，不误伤正常恢复。
//!
//! 注意：S22 的「drop 不 close」是故意的负向夹具（缺陷现场形态），与既有
//! 测试夹具的 checkpoint + shutdown 纪律相反，勿「纠正」。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use serde_json::json;
use tempfile::tempdir;

async fn exec_ok(db: &Database, sql: &str) {
    let resp = db.execute_sql(sql).await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "setup statement failed: {sql:?} -> {resp:?}"
    );
}

/// S22：drop 不 close → 重开打开失败且错误定向指因（修复前：归并文案，
/// 断言 catalog/checkpoint 要素必 RED）。
#[tokio::test]
async fn drop_without_close_reports_directive_error() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test");
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
        exec_ok(&db, "INSERT INTO t VALUES (1, 10)").await;
        // 故意不 close()：句柄 drop，catalog 脏页丢失（缺陷现场形态）。
    }

    let err = match Database::open(&path).await {
        Err(e) => e,
        Ok(_) => panic!("open must be refused without a checkpointed close"),
    };
    let msg = err.to_string();
    // 要素 1：表名。
    assert!(msg.contains("table 't'"), "错误必须点名表名，实际: {msg}");
    // 要素 2：catalog 缺失陈述。
    assert!(
        msg.contains("catalog"),
        "错误必须陈述 catalog 缺失，实际: {msg}"
    );
    // 要素 3：上次关闭未经 checkpoint 的可能原因（含 lib 句柄未 close）。
    assert!(
        msg.contains("checkpoint"),
        "错误必须指因未 checkpoint 关闭，实际: {msg}"
    );
    assert!(
        msg.contains("close()"),
        "错误必须给出未 close() 示例，实际: {msg}"
    );
    // 要素 4：打开拒绝明示。
    assert!(
        msg.contains("cannot be opened") || msg.contains("open is refused"),
        "错误必须明示打开拒绝，实际: {msg}"
    );
}

/// S22 补充：打开结果不变——仍失败、不跳过不丢弃（K05）。
#[tokio::test]
async fn open_still_refused_without_close() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test");
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY)").await;
        exec_ok(&db, "INSERT INTO t VALUES (1)").await;
    }
    assert!(Database::open(&path).await.is_err(), "打开必须保持拒绝");
}

/// S23：同序列走 close() → 重开成功、行可见（定向错误面不误伤正常恢复）。
#[tokio::test]
async fn close_then_reopen_succeeds() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test");
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
        exec_ok(&db, "INSERT INTO t VALUES (1, 10)").await;
        db.close().await.unwrap();
    }
    let db = Database::open(&path).await.unwrap();
    let rows = match db.execute_sql("SELECT id, v FROM t").await {
        Response::QueryResult { rows } => rows,
        other => panic!("Expected QueryResult, got {other:?}"),
    };
    assert_eq!(rows, vec![vec![json!(1), json!(10)]], "正常关闭后行可见");
    db.close().await.unwrap();
}
