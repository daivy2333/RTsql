use rtsql::storage::page_format::ColumnType;
use rtsql::storage::{BufferPool, FileStorage, TableManager};
use std::sync::Arc;
use tempfile::tempdir;

async fn setup() -> (Arc<TableManager>, Arc<BufferPool>, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    (table_mgr, buffer_pool, dir)
}

#[tokio::test]
async fn create_and_get_table() {
    let (table_mgr, _bp, _dir) = setup().await;

    table_mgr
        .create_table(
            "users",
            vec![
                ("id".to_string(), ColumnType::Int),
                ("name".to_string(), ColumnType::String(255)),
            ],
            "id",
        )
        .await
        .unwrap();

    let table = table_mgr.get_table("users").await.unwrap();
    assert_eq!(table.name, "users");
    assert_eq!(table.columns.len(), 2);
    assert_eq!(table.columns[0].0, "id");
    assert_eq!(table.columns[1].0, "name");
    assert_eq!(table.pk_column, "id");
}

#[tokio::test]
async fn duplicate_table_error() {
    let (table_mgr, _bp, _dir) = setup().await;

    table_mgr
        .create_table("users", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();

    let result = table_mgr
        .create_table("users", vec![("id".to_string(), ColumnType::Int)], "id")
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn table_not_found() {
    let (table_mgr, _bp, _dir) = setup().await;

    let result = table_mgr.get_table("nonexistent").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn create_table_allocates_data_page() {
    let (table_mgr, bp, _dir) = setup().await;

    table_mgr
        .create_table("users", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();

    let table = table_mgr.get_table("users").await.unwrap();
    // head and tail must point to the same newly-allocated page
    assert_eq!(table.data_page_head, *table.data_page_tail.lock().unwrap());
    // verify the page is accessible in the buffer pool
    bp.get_page(table.data_page_head).await.unwrap();
}

#[tokio::test]
async fn pk_column_validation() {
    let (table_mgr, _bp, _dir) = setup().await;

    table_mgr
        .create_table(
            "users",
            vec![
                ("id".to_string(), ColumnType::Int),
                ("name".to_string(), ColumnType::String(255)),
            ],
            "id",
        )
        .await
        .unwrap();

    let table = table_mgr.get_table("users").await.unwrap();
    assert_eq!(table.pk_index, 0);
    assert_eq!(table.columns[table.pk_index].0, "id");

    // Verify invalid PK
    let result = table_mgr
        .create_table(
            "t2",
            vec![("a".to_string(), ColumnType::Int)],
            "nonexistent_col",
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn table_exists_check() {
    let (table_mgr, _bp, _dir) = setup().await;

    assert!(!table_mgr.table_exists("users"));

    table_mgr
        .create_table("users", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();

    assert!(table_mgr.table_exists("users"));
    assert!(!table_mgr.table_exists("nonexistent"));
}
