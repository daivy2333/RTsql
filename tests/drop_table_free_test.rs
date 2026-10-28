//! MS07-T02 drop_table 物理页释放集成测试。
//!
//! 验证 `TableManager::drop_table` 在抹除 schema + 移除 in-memory 之后，
//! 将该表的 data page 链与 BTree 索引页释放到 `FileStorage::free_pages`，
//! 使同进程内后续 `create_table` 复用 free-list（`file_len` 不再单调递增）。

#![cfg(feature = "network")]
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::page_format::ColumnType;
use rtsql::storage::{Page, HEADER_SIZE};
use std::path::Path;
use tempfile::tempdir;

/// Physical page count from the on-disk file length. The file lock is
/// exclusive, so the old "second read-only handle" probe is replaced by a
/// plain metadata read; the length is the high-water mark because freed
/// pages go to the free-list without truncation. MS10-T03: the 64-byte
/// format header lives outside the page space and is subtracted first.
fn page_count(path: &Path) -> u64 {
    (std::fs::metadata(path).unwrap().len() - HEADER_SIZE as u64) / Page::PAGE_SIZE as u64
}

async fn create_users(db: &Database) {
    db.create_table(
        "users",
        vec![
            ("id".to_string(), ColumnType::Int),
            ("name".to_string(), ColumnType::String(255)),
        ],
        "id",
    )
    .await
    .unwrap();
}

async fn insert_one(db: &Database, id: i64, name: &str) {
    let sql = format!("INSERT INTO users VALUES ({}, '{}')", id, name);
    if let Response::Error { message, .. } = to_response(db.execute_sql(&sql).await) {
        panic!("INSERT failed: {}", message);
    }
}

#[tokio::test]
async fn test_simple_drop_releases_data_and_btree() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    let db = Database::open(&path).await.unwrap();
    create_users(&db).await;
    let count_after_create = page_count(&path);
    // 被释放的页（数据页头 + 索引树根）——释放列表按后进先出弹出，数据页头
    // 最先被发放，故它足以判定「被释放的页是否被复用」。
    let meta = db.get_table("users").await.unwrap();
    let released = [meta.data_page_head.0, meta.index_manager.root_page_id().0];

    db.table_manager.drop_table("users").await.unwrap();

    // After drop the high-water mark is unchanged (pages moved to the
    // free-list, not truncated).
    let count_after_drop = page_count(&path);
    assert_eq!(count_after_drop, count_after_create);

    // (a) 未 checkpoint：释放未持久化 ⇒ 被释放的页不得可分配——新建表拿到
    //     的是新页（文件页数上界增长），且不占用被释放的页。
    db.create_table("probe", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();
    let count_after_probe = page_count(&path);
    assert!(
        count_after_probe > count_after_create,
        "before the release is persisted the recreated table must not reuse the freed pages (file grew {} pages)",
        count_after_probe - count_after_create
    );
    let probe_meta = db.get_table("probe").await.unwrap();
    assert!(
        !released.contains(&probe_meta.data_page_head.0),
        "the probe table must not take a released page {released:?}"
    );

    // (b) checkpoint 之后：释放已持久化 ⇒ 被释放的页可分配——重建 `users`
    //     复用被释放的页，文件不再增长。
    db.checkpoint().await.unwrap();
    let count_before_recreate = page_count(&path);
    create_users(&db).await;
    let count_after_recreate = page_count(&path);
    assert_eq!(
        count_after_recreate, count_before_recreate,
        "recreating users must reuse freed pages (free-list) without growing the file"
    );
    let users_meta = db.get_table("users").await.unwrap();
    assert!(
        released.contains(&users_meta.data_page_head.0),
        "the recreated table's data page {} must come from the released pages {released:?}",
        users_meta.data_page_head.0
    );
    assert!(
        released.contains(&users_meta.index_manager.root_page_id().0),
        "the recreated table's index root {} must come from the released pages {released:?}",
        users_meta.index_manager.root_page_id().0
    );
}

#[tokio::test]
async fn test_long_data_page_chain_all_released() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    let db = Database::open(&path).await.unwrap();
    create_users(&db).await;
    let count_empty = page_count(&path);

    // 1000 rows force a multi-page data chain.
    for i in 0..1000i64 {
        insert_one(&db, i, &format!("user_{:06}", i)).await;
    }
    let count_filled = page_count(&path);
    assert!(
        count_filled - count_empty >= 5,
        "1000 rows must span a 5+ page data chain, grew {} pages",
        count_filled - count_empty
    );

    // 释放前的页数上界：被释放的页号都小于它，其后新分配的页号都大于等于它。
    let high_water = page_count(&path);

    db.table_manager.drop_table("users").await.unwrap();

    // (a) 未 checkpoint：释放未持久化 ⇒ 被释放的页不得可分配——新建表拿到
    //     的是新页（文件页数上界增长），且不占用被释放的页。
    db.create_table("probe", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();
    let count_after_probe = page_count(&path);
    assert!(
        count_after_probe > count_filled,
        "before the release is persisted the new table must not reuse the freed pages (file grew {} pages)",
        count_after_probe - count_filled
    );
    let probe_meta = db.get_table("probe").await.unwrap();
    assert!(
        probe_meta.data_page_head.0 >= high_water,
        "the probe table must not take a released page (page {} < high water {})",
        probe_meta.data_page_head.0,
        high_water
    );

    // (b) checkpoint 之后：释放已持久化 ⇒ 被释放的页可分配——新建表 `t` 复用
    //     被释放的页，文件不再增长。
    db.checkpoint().await.unwrap();
    let count_before_recreate = page_count(&path);
    db.create_table("t", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();
    let count_after = page_count(&path);
    assert_eq!(
        count_after, count_before_recreate,
        "creating a new table after dropping a multi-page table must not grow the file"
    );
    let t_meta = db.get_table("t").await.unwrap();
    assert!(
        t_meta.data_page_head.0 < high_water,
        "the new table's data page {} must be one of the released pages (high water {})",
        t_meta.data_page_head.0,
        high_water
    );
}

#[tokio::test]
async fn test_btree_height_gt_1_all_pages_released() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    let db = Database::open(&path).await.unwrap();
    create_users(&db).await;

    // Enough rows to force a BTree height >= 2.
    for i in 0..500i64 {
        insert_one(&db, i, &format!("user_{:06}", i)).await;
    }
    let count_filled = page_count(&path);

    // Sanity: the index must actually have height > 1 (root + internal + leaves).
    let meta = db.get_table("users").await.unwrap();
    let index_pages = meta.index_manager.collect_all_pages().await.unwrap();
    assert!(
        index_pages.len() >= 3,
        "expected a height>=2 BTree (>=3 pages), got {} pages",
        index_pages.len()
    );
    // 释放前的页数上界：被释放的页号都小于它，其后新分配的页号都大于等于它。
    let high_water = page_count(&path);

    db.table_manager.drop_table("users").await.unwrap();

    // (a) 未 checkpoint：释放未持久化 ⇒ 被释放的页不得可分配——新建表拿到
    //     的是新页（文件页数上界增长），且不占用被释放的页。
    db.create_table("probe", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();
    let count_after_probe = page_count(&path);
    assert!(
        count_after_probe > count_filled,
        "before the release is persisted the new table must not reuse the freed pages (file grew {} pages)",
        count_after_probe - count_filled
    );
    let probe_meta = db.get_table("probe").await.unwrap();
    assert!(
        probe_meta.data_page_head.0 >= high_water,
        "the probe table must not take a released page (page {} < high water {})",
        probe_meta.data_page_head.0,
        high_water
    );

    // (b) checkpoint 之后：释放已持久化 ⇒ 被释放的页可分配——新建表 `t` 复用
    //     被释放的页，文件不再增长。
    db.checkpoint().await.unwrap();
    let count_before_recreate = page_count(&path);
    db.create_table("t", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();
    let count_after = page_count(&path);
    assert_eq!(
        count_after, count_before_recreate,
        "dropping a height>=2 BTree must release all index pages to the free-list"
    );
    let t_meta = db.get_table("t").await.unwrap();
    assert!(
        t_meta.data_page_head.0 < high_water,
        "the new table's data page {} must be one of the released pages (high water {})",
        t_meta.data_page_head.0,
        high_water
    );
}

#[tokio::test]
async fn test_same_process_free_list_reuse() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    let db = Database::open(&path).await.unwrap();
    create_users(&db).await;
    for i in 0..100i64 {
        insert_one(&db, i, &format!("user_{:06}", i)).await;
    }
    let count_before_drop = page_count(&path);
    let meta = db.get_table("users").await.unwrap();
    let released = [meta.data_page_head.0, meta.index_manager.root_page_id().0];

    db.table_manager.drop_table("users").await.unwrap();

    // (a) 未 checkpoint：被释放的页不可分配 ⇒ 旧上限（增长 ≤ 1 页）不再成立。
    // Recreate the SAME name; the recreated table must not take a released page.
    create_users(&db).await;
    for i in 0..100i64 {
        insert_one(&db, i, &format!("user_{:06}", i)).await;
    }
    let count_after_unpersisted = page_count(&path);
    assert!(
        count_after_unpersisted > count_before_drop + 1,
        "before the release is persisted the file must grow by more than 1 page (grew {} pages)",
        count_after_unpersisted - count_before_drop
    );
    let meta = db.get_table("users").await.unwrap();
    assert!(
        !released.contains(&meta.data_page_head.0),
        "the recreated table must not take a released page {released:?}"
    );

    // (b) checkpoint 之后：释放已持久化 ⇒ 被释放的页可分配 ⇒ 旧上限恢复。
    // 释放 (a) 段的表并再次 checkpoint，使 (b) 段的重建同样复用被释放的页。
    db.checkpoint().await.unwrap();
    db.table_manager.drop_table("users").await.unwrap();
    db.checkpoint().await.unwrap();
    let count_before_recreate = page_count(&path);
    create_users(&db).await;
    for i in 0..100i64 {
        insert_one(&db, i, &format!("user_{:06}", i)).await;
    }
    let count_after = page_count(&path);
    assert!(
        count_after <= count_before_recreate + 1,
        "recreate+reinsert after a persisted release must stay within drop-before + 1 page (grew {} pages)",
        count_after - count_before_recreate
    );

    // And the recreated table is fully usable.
    match to_response(db.execute_sql("SELECT COUNT(*) FROM users").await) {
        Response::QueryResult { rows } => assert!(
            !rows.is_empty(),
            "recreated users table should be queryable"
        ),
        _ => panic!("SELECT COUNT(*) FROM users failed after recreate"),
    }
}

#[tokio::test]
async fn test_cross_restart_after_drop_safe() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    // First session: create + insert + drop inside one transaction scope.
    {
        let db = Database::open(&path).await.unwrap();
        create_users(&db).await;
        insert_one(&db, 1, "alice").await;
        db.table_manager.drop_table("users").await.unwrap();
        db.close().await.unwrap();
    }

    // Second session: table must be gone (catalog erased first), no panic.
    let db2 = Database::open(&path).await.unwrap();
    assert!(
        db2.get_table("users").await.is_err(),
        "users must not exist after drop + restart"
    );

    // Recreate works normally (same 2-column shape used by insert_one).
    db2.create_table(
        "users",
        vec![
            ("id".to_string(), ColumnType::Int),
            ("name".to_string(), ColumnType::String(255)),
        ],
        "id",
    )
    .await
    .unwrap();
    insert_one(&db2, 7, "bob").await;
    match to_response(db2.execute_sql("SELECT COUNT(*) FROM users").await) {
        Response::QueryResult { rows } => assert!(
            !rows.is_empty(),
            "recreated users must be usable after restart"
        ),
        _ => panic!("SELECT failed after recreate"),
    }
}

#[tokio::test]
async fn test_concurrent_drop_different_tables() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");

    let db = Database::open(&path).await.unwrap();
    for k in 0..10 {
        db.create_table(
            &format!("t{}", k),
            vec![("id".to_string(), ColumnType::Int)],
            "id",
        )
        .await
        .unwrap();
    }

    // 释放前的页数上界：被释放的页号都小于它，其后新分配的页号都大于等于它。
    let high_water = page_count(&path);

    // 10 concurrent drops of distinct tables.
    let tm = db.table_manager.clone();
    let mut handles = Vec::new();
    for k in 0..10 {
        let tm = tm.clone();
        handles.push(tokio::spawn(async move {
            tm.drop_table(&format!("t{}", k)).await
        }));
    }
    for h in handles {
        h.await.unwrap().unwrap();
    }

    for k in 0..10 {
        assert!(db.get_table(&format!("t{}", k)).await.is_err());
    }

    // Recreating all 10 must reuse the freed pages (no file growth), which
    // also indirectly verifies the free-list holds 10 distinct page pairs.
    let count_after_drop = page_count(&path);

    // (a) 未 checkpoint：被释放的页不可分配 ⇒ 重建的 10 张表拿到的是新页
    //     （文件页数上界增长），且不占用被释放的页。
    for k in 0..10 {
        db.create_table(
            &format!("t{}", k),
            vec![("id".to_string(), ColumnType::Int)],
            "id",
        )
        .await
        .unwrap();
    }
    let count_after_recreate = page_count(&path);
    assert!(
        count_after_recreate > count_after_drop,
        "before the release is persisted the recreated tables must not reuse the freed pages (file grew {} pages)",
        count_after_recreate - count_after_drop
    );
    for k in 0..10 {
        let meta = db.get_table(&format!("t{}", k)).await.unwrap();
        assert!(
            meta.data_page_head.0 >= high_water,
            "the recreated table t{k} must not take a released page (page {} < high water {})",
            meta.data_page_head.0,
            high_water
        );
    }

    // (b) checkpoint 之后：被释放的页可分配 ⇒ 重建的 10 张表复用被释放的页
    //     （文件页数上界不增长）。
    db.checkpoint().await.unwrap();
    let count_before_recreate = page_count(&path);
    for k in 0..10 {
        db.create_table(
            &format!("u{}", k),
            vec![("id".to_string(), ColumnType::Int)],
            "id",
        )
        .await
        .unwrap();
    }
    let count_after_reuse = page_count(&path);
    assert_eq!(
        count_after_reuse, count_before_recreate,
        "recreating 10 dropped tables must reuse the free-list"
    );
    for k in 0..10 {
        let meta = db.get_table(&format!("u{}", k)).await.unwrap();
        assert!(
            meta.data_page_head.0 < high_water,
            "the recreated table u{k}'s data page {} must be one of the released pages (high water {})",
            meta.data_page_head.0,
            high_water
        );
    }
}
