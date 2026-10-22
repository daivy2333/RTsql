//! change 2026-10-03-practical-core-gaps T11（R6）：恢复重放目标表缺失的
//! 定向错误面。
//!
//! S22：catalog 缺表 + WAL 引用该表 → 重新 open：错误须点名表名、陈述
//! catalog 缺失事实、陈述最可能原因（上次关闭未经 checkpoint，如 lib
//! 句柄未 close() 即 drop、未持久化 DDL 随脏页丢失）并明示打开拒绝
//! （修复前为归并文案 `WAL redo failed: table 't' lookup failed during
//! redo: table not found: t`——把「DDL 从未持久化」误诊为「redo 目标表
//! 缺失」）。打开仍失败：不跳过、不丢弃不可重放记录（K05）。
//!
//! 夹具修订（change 2026-10-13 T3）：「CREATE 后 drop 不 close」曾是该
//! 形态的自然产生法，T3/D4 语句边界有序落盘已将其关闭——本文件经
//! `craft_legacy_brick` 手工构造「存量砖库」形态承载同一错误面（D-2：
//! 存量已坏库不救、保持拒绝；delta R3：K05 SHALL 保持不变）。
//!
//! S23 对照：同序列走 `close()` → open 成功、行可见——定向错误面只覆盖
//! catalog 缺表形态，不误伤正常恢复。

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use serde_json::json;
use tempfile::tempdir;

async fn exec_ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "setup statement failed: {sql:?} -> {resp:?}"
    );
}

/// 构造「存量砖库」形态（change 2026-10-13 T3 夹具修订——原夹具
/// 「CREATE 后 drop 不 close」曾是该形态的自然产生法，T3/D4 起 DDL
/// 语句边界有序落盘已将该缺陷形态关闭，见本 change delta R3 与
/// tests/ddl_crash_recovery_test.rs 崩溃矩阵）。K05 定向错误面仍须
/// 保持（delta R3：K05 SHALL 保持不变；用户决策 D-2：存量已坏库不救、
/// 保持拒绝），故手工构造：WAL 持有表 t 的已提交记录（shutdown +
/// drop 不 close），`__tables` 页重置为空页（= 旧版「DDL 仅脏页」
/// 撕裂残余/冲刷中段崩溃 remnant 的形状）——catalog 缺表 + WAL 引用
/// 该表同时成立。
async fn craft_legacy_brick(path: &std::path::Path) {
    {
        let db = Database::open(path).await.unwrap();
        exec_ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)").await;
        exec_ok(&db, "INSERT INTO t VALUES (1, 10)").await;
        db.wal_buffer.as_ref().unwrap().shutdown().await;
        drop(db);
    }
    // 单表新库的 page 0 仅含 t 行：重置为空 0x03 页即「表定义从未
    // 持久化」的旧版砖形。手工页手术经独立 FileStorage 句柄完成
    //（Database 已 drop，flock 已释放）。
    use rtsql::storage::{AsyncStorage, FileStorage, PageId};
    let storage = FileStorage::open(path).unwrap();
    let mut page = storage.read_page(PageId(0)).await.unwrap();
    // SlottedPageHeader: type=0x03, slot_count=0, free_space_offset=16,
    // next_page_id=0, next_logical_id=0（其余数据区清零）。
    page.data[..16].copy_from_slice(&[0x03, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    for b in page.data[16..].iter_mut() {
        *b = 0;
    }
    storage.write_page(PageId(0), &page).await.unwrap();
    storage.sync().await.unwrap();
    drop(storage);
}

/// S22：catalog 缺表 + WAL 引用该表（存量砖形）→ 重开打开失败且错误
/// 定向指因（修复前：归并文案，断言 catalog/checkpoint 要素必 RED）。
#[tokio::test]
async fn drop_without_close_reports_directive_error() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test");
    craft_legacy_brick(&path).await;

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
/// 夹具同 S22（存量砖形，见 `craft_legacy_brick` 修订说明）。
#[tokio::test]
async fn open_still_refused_without_close() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test");
    craft_legacy_brick(&path).await;
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
    let rows = match to_response(db.execute_sql("SELECT id, v FROM t").await) {
        Response::QueryResult { rows } => rows,
        other => panic!("Expected QueryResult, got {other:?}"),
    };
    assert_eq!(rows, vec![vec![json!(1), json!(10)]], "正常关闭后行可见");
    db.close().await.unwrap();
}
