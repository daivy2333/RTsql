//! 写盘失败按页记录。锚点页仍在失败集合内时，它挡住的释放不提升；
//! 该页被成功重写后离开集合，释放可以提升，新建表能拿到被释放的页。

#![cfg(feature = "network")]
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::IoFailPoint;
use std::path::PathBuf;
use tempfile::TempDir;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("write_failure_state.db")
}

async fn ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{sql} must succeed: {resp:?}"
    );
}

async fn table_pages(db: &Database, name: &str) -> (u64, u64) {
    let meta = db.get_table(name).await.unwrap();
    (meta.data_page_head.0, meta.index_manager.root_page_id().0)
}

fn took_released(data: u64, root: u64, released: [u64; 2]) -> bool {
    released.contains(&data) || released.contains(&root)
}

/// 先把已有脏页刷干净，再 `DROP`。随后失败的 `flush_all` 只能命中这次
/// 目录删除写下的页，也就是这次释放的锚点页。
async fn drop_clean(db: &Database, name: &str) -> [u64; 2] {
    db.buffer_pool.flush_all().await.unwrap();
    let pages = table_pages(db, name).await;
    ok(db, &format!("DROP TABLE {name}")).await;
    [pages.0, pages.1]
}

async fn fail_one_write(db: &Database) {
    db.arm_io_failure(IoFailPoint::DataWrite);
    let err = db.buffer_pool.flush_all().await.unwrap_err();
    assert!(
        err.to_string().contains("injected io failure (write_page)"),
        "the flush must fail with the injected write marker: {err}"
    );
    assert!(
        db.buffer_pool.has_dirty_pages(),
        "the failed page must still be dirty"
    );
}

#[tokio::test]
async fn anchor_stays_unpromoted_while_its_page_is_failed() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    let released = drop_clean(&db, "t").await;
    fail_one_write(&db).await;

    let storage = db.buffer_pool.storage();
    storage.promote_free_releases(storage.release_completion_limit());

    ok(&db, "CREATE TABLE u (id INT PRIMARY KEY, n INT)").await;
    let (data, root) = table_pages(&db, "u").await;
    assert!(
        !took_released(data, root, released),
        "table u must not take a released page {released:?} while the anchor write is still failed (got data {data}, root {root})"
    );
    db.close().await.unwrap();
}

#[tokio::test]
async fn anchor_is_promoted_after_its_page_is_rewritten() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    let released = drop_clean(&db, "t").await;
    fail_one_write(&db).await;

    db.checkpoint().await.unwrap();

    ok(&db, "CREATE TABLE u (id INT PRIMARY KEY, n INT)").await;
    let (data, root) = table_pages(&db, "u").await;
    assert!(
        took_released(data, root, released),
        "after the anchor page is rewritten, table u must reuse one of {released:?} (got data {data}, root {root})"
    );
    db.close().await.unwrap();
}

#[tokio::test]
async fn unrelated_page_failure_does_not_block_a_later_release() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;
    ok(&db, "INSERT INTO t VALUES (1, 10)").await;
    fail_one_write(&db).await;
    db.buffer_pool.flush_all().await.unwrap();

    ok(&db, "CREATE TABLE s (id INT PRIMARY KEY, n INT)").await;
    ok(&db, "INSERT INTO s VALUES (2, 20)").await;
    let released = drop_clean(&db, "s").await;
    db.checkpoint().await.unwrap();

    ok(&db, "CREATE TABLE u (id INT PRIMARY KEY, n INT)").await;
    let (data, root) = table_pages(&db, "u").await;
    assert!(
        took_released(data, root, released),
        "a rewritten unrelated failure must not keep table s's pages {released:?} out of reuse (got data {data}, root {root})"
    );
    db.close().await.unwrap();
}
