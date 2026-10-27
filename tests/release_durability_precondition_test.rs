//! change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
//! Iteration 004 / Cycle 000 —— `release-durability-precondition` R1 / R2。
//!
//! 本域承载 B-1（DDL 边界刷盘失败后**提前**提升释放 ⇒ 未持久化的释放被提前
//! 变成可分配）。两条用例各自真实构造时序，断言只落在用户可见结果上：
//!
//! - R1（`failed_write_keeps_page_dirty`）：一次受控写盘失败命中脏页 ⇒ 刷盘
//!   返回错误 ⇒ **该页仍为脏** ⇒ 之后一次成功刷盘把该页内容写入存储、该页
//!   此后不再为脏 ⇒ 重开库后读到最新内容。
//! - R2（`release_not_promoted_when_anchor_write_not_durable`）：`DROP TABLE` 的
//!   目录删除（锚点写）已发出但未落盘；承载锚点写的页写盘失败过一次 ⇒ 一次
//!   不覆盖该页的刷盘之后，checkpoint 成功重写锚点页 ⇒ 该页离开失败集合，
//!   释放被提升（新建表拿到被释放的页之一）；重开该库后目录与数据面一致。
//!
//! 观察约定沿用既有先例：真实文件库 + 真实 `Database::open` / `execute_sql` /
//! `checkpoint` / `buffer_pool.flush_all()` / `buffer_pool.flush_pages()` 入口 +
//! 既有一次性注入点 `Database::arm_io_failure(IoFailPoint::DataWrite)`；断言落在
//! 页是否仍脏、内容是否落盘、页是否被发放、重开后目录与数据面是否一致。

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::integrity::check_integrity;
use rtsql::storage::{IoFailPoint, PageId};
use std::path::PathBuf;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("release_durability.db")
}

/// 执行单条 SQL 并断言成功。
async fn ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{sql} must succeed: {resp:?}"
    );
}

/// 断言查询结果行集。
async fn rows(db: &Database, sql: &str) -> Vec<Vec<serde_json::Value>> {
    match to_response(db.execute_sql(sql).await) {
        Response::QueryResult { rows } => rows,
        other => panic!("{sql} must return rows: {other:?}"),
    }
}

/// 断言该 SQL 返回错误。
async fn fails(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        matches!(resp, Response::Error { .. }),
        "{sql} must fail: {resp:?}"
    );
}

// ===========================================================================
// R1：刷盘失败不丢弃脏页写
// ===========================================================================

/// R1 场景「数据页写失败后该页仍为脏」+「失败后的脏页在后续成功刷盘时被重写」：
/// 一次受控写盘失败命中脏页后，刷盘返回错误，但**该页仍为脏**（否则它的内容
/// 此后永不被重写）；之后一次成功刷盘把它写出、该页不再为脏；重开库后读到最新
/// 内容。
#[tokio::test]
async fn failed_write_keeps_page_dirty() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let db = Database::open(&path).await.unwrap();
    ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    assert!(
        db.buffer_pool.has_dirty_pages(),
        "the insert must leave the data page dirty in the pool"
    );

    // 下一次同型写盘被受控失败命中 ⇒ 刷盘返回错误。
    db.arm_io_failure(IoFailPoint::DataWrite);
    let err = db.buffer_pool.flush_all().await.unwrap_err();
    assert!(
        err.to_string().contains("injected io failure (write_page)"),
        "the flush must fail with the injected write marker: {err}"
    );

    // R1 的核心断言：失败的刷盘**不得**把该页标成「不脏」——否则它的内容此后
    // 永不被重写（写丢失且无人重试）。
    assert!(
        db.buffer_pool.has_dirty_pages(),
        "a failed flush must leave the unwritten pages dirty: their content would \
         otherwise be lost forever"
    );

    // 之后一次成功刷盘 ⇒ 该页被写入存储、此后不再为脏。
    db.buffer_pool.flush_all().await.unwrap();
    assert!(
        !db.buffer_pool.has_dirty_pages(),
        "a successful flush must clear the dirty flag of every page it wrote"
    );
    db.close().await.unwrap();
    // 释放句柄（文件锁随 fd 生命周期持有）后再重开同一库。
    drop(db);

    // 重开库 ⇒ 读到最新内容。
    let db = Database::open(&path).await.unwrap();
    assert_eq!(
        rows(&db, "SELECT id, n FROM t").await,
        vec![vec![serde_json::json!(1), serde_json::json!(10)]],
        "the row must survive the reopen: the failed flush must not have lost it"
    );
    db.close().await.unwrap();
}

// ===========================================================================
// R2：释放的可分配判定以锚点写已持久为准
// ===========================================================================

/// R2 修订后：锚点页写失败后，checkpoint 成功重写该页，释放被提升。
/// `DROP TABLE t` 的目录删除已发出；一次不覆盖目录页的刷盘之后走完
/// checkpoint ⇒ 新建表拿到被释放的页之一。锚点仍失败时不提升由
/// `write_failure_state_test` 锁住。重开后目录与数据面一致。
#[tokio::test]
async fn release_not_promoted_when_anchor_write_not_durable() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);

    let db = Database::open(&path).await.unwrap();
    ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    // 只取页号（`TableMeta` 持有索引树引用，会连带持有存储句柄）。
    let (t_data_page, t_tree_root) = {
        let meta = db.get_table("t").await.unwrap();
        (meta.data_page_head.0, meta.index_manager.root_page_id().0)
    };

    // 锚点写（目录删除）发出且未落盘；释放完成号已记。
    ok(&db, "DROP TABLE t").await;

    // 承载锚点写的目录页写盘失败一次 ⇒ 刷盘返回错误。
    db.arm_io_failure(IoFailPoint::DataWrite);
    let err = db.buffer_pool.flush_all().await.unwrap_err();
    assert!(
        err.to_string().contains("injected io failure (write_page)"),
        "the flush must fail with the injected write marker: {err}"
    );
    // 失败的刷盘不得丢弃脏页写（RDP R1）：该页此后仍为脏。
    assert!(
        db.buffer_pool.has_dirty_pages(),
        "a failed flush must leave the unwritten pages dirty"
    );

    // 一次**不覆盖该目录页**的成功刷盘（该页此刻仍未落盘）。
    db.buffer_pool
        .flush_pages(&[PageId(t_data_page)])
        .await
        .unwrap();

    // 走完 checkpoint。
    db.checkpoint().await.unwrap();

    // checkpoint 已成功重写锚点页 ⇒ 释放被提升 ⇒ 新建表拿到被释放的页之一。
    ok(&db, "CREATE TABLE u (id INT PRIMARY KEY, n INT)").await;
    let (u_data, u_root) = {
        let u_meta = db.get_table("u").await.unwrap();
        let released = [t_data_page, t_tree_root];
        let data = u_meta.data_page_head.0;
        let root = u_meta.index_manager.root_page_id().0;
        assert!(
            released.contains(&data) || released.contains(&root),
            "after checkpoint rewrites the anchor page, the new table must reuse \
             one of {released:?} (got data {data}, root {root})"
        );
        (data, root)
    };

    // 数据库仍可正常使用，完整性扫描的 error 级发现数为 0。
    ok(&db, "INSERT INTO u VALUES (7, 70)").await;
    assert_eq!(
        rows(&db, "SELECT id, n FROM u").await,
        vec![vec![serde_json::json!(7), serde_json::json!(70)]]
    );
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(
        report.error_count(),
        0,
        "no error-level findings after the failed anchor write: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.severity == rtsql::storage::integrity::FindingSeverity::Error)
            .collect::<Vec<_>>()
    );
    db.close().await.unwrap();
    drop(db);

    // 重开该库 ⇒ 目录与数据面一致：`t` 不再出现；`u` 仍占着它在本会话拿到的
    // 页，行还在。这些页已经登记给 `u`，不能再发给下一张表。
    let db = Database::open(&path).await.unwrap();
    fails(&db, "SELECT id, n FROM t").await;
    assert_eq!(
        rows(&db, "SELECT id, n FROM u").await,
        vec![vec![serde_json::json!(7), serde_json::json!(70)]]
    );
    {
        let u_meta = db.get_table("u").await.unwrap();
        assert_eq!(u_meta.data_page_head.0, u_data);
        assert_eq!(u_meta.index_manager.root_page_id().0, u_root);
    }
    ok(&db, "CREATE TABLE v (id INT PRIMARY KEY, n INT)").await;
    {
        let v_meta = db.get_table("v").await.unwrap();
        assert_ne!(v_meta.data_page_head.0, u_data);
        assert_ne!(v_meta.index_manager.root_page_id().0, u_root);
    }
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .expect("check_integrity must complete");
    assert_eq!(
        report.error_count(),
        0,
        "no error-level findings after the reopen: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.severity == rtsql::storage::integrity::FindingSeverity::Error)
            .collect::<Vec<_>>()
    );
    db.close().await.unwrap();
}
