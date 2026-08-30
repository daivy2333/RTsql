//! MS07-T02 drop_table 物理页释放集成测试。
//!
//! 验证 `TableManager::drop_table` 在抹除 schema + 移除 in-memory 之后，
//! 将该表的 data page 链与 BTree 索引页释放到 `FileStorage::free_pages`，
//! 使同进程内后续 `create_table` 复用 free-list（`file_len` 不再单调递增）。

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::storage::page_format::ColumnType;
use rtsql::storage::FileStorage;
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;

/// Open a second read-only handle to the same file and return its current
/// page count (file is not resized, so this reflects the high-water mark).
fn page_count(path: &Path) -> u64 {
    Arc::new(FileStorage::open(path).unwrap()).page_count()
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
    if let Response::Error { message } = db.execute_sql(&sql).await {
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

    db.table_manager.drop_table("users").await.unwrap();

    // After drop the high-water mark is unchanged (pages moved to the
    // free-list, not truncated), and a recreate reuses the freed pages so
    // the file does not grow.
    let count_after_drop = page_count(&path);
    assert_eq!(count_after_drop, count_after_create);

    create_users(&db).await;
    let count_after_recreate = page_count(&path);
    assert_eq!(
        count_after_recreate, count_after_create,
        "recreating users must reuse freed pages (free-list) without growing the file"
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

    db.table_manager.drop_table("users").await.unwrap();

    // Recreate table `t`: it must reuse freed pages (no file growth).
    db.create_table("t", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();
    let count_after = page_count(&path);
    assert_eq!(
        count_after, count_filled,
        "creating a new table after dropping a multi-page table must not grow the file"
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

    db.table_manager.drop_table("users").await.unwrap();

    db.create_table("t", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();
    let count_after = page_count(&path);
    assert_eq!(
        count_after, count_filled,
        "dropping a height>=2 BTree must release all index pages to the free-list"
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

    db.table_manager.drop_table("users").await.unwrap();

    // Recreate the SAME name; growth must stay within drop-before + 1 page.
    create_users(&db).await;
    for i in 0..100i64 {
        insert_one(&db, i, &format!("user_{:06}", i)).await;
    }
    let count_after = page_count(&path);
    assert!(
        count_after <= count_before_drop + 1,
        "recreate+reinsert after drop grew file by {} pages (expected <= 1)",
        count_after - count_before_drop
    );

    // And the recreated table is fully usable.
    match db.execute_sql("SELECT COUNT(*) FROM users").await {
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
    match db2.execute_sql("SELECT COUNT(*) FROM users").await {
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
    assert_eq!(
        count_after_recreate, count_after_drop,
        "recreating 10 dropped tables must reuse the free-list"
    );
}
