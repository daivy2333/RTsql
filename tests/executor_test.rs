//! Executor unit tests

use rtsql::executor::{
    DeleteExecutor, ExecResult, Executor, IndexScanExecutor, InsertExecutor, ScanExecutor,
    UpdateExecutor, Value,
};
use rtsql::storage::{
    data::TableManager, page_format::ColumnType, read_tuple_from_data_page, BufferPool,
    FileStorage, Result,
};
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_scan_executor_full_table() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;

    let values = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
    ];
    let mut insert_executor =
        InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), values, 0);
    let result = insert_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(3)));

    let mut executor = ScanExecutor::new(table_meta, buffer_pool, None);

    let mut row_count = 0;
    while let Some(result) = executor.next().await? {
        match result {
            ExecResult::Row(_) => {
                row_count += 1;
            }
            _ => panic!("Expected ExecResult::Row"),
        }
    }
    assert_eq!(row_count, 3);

    Ok(())
}

#[tokio::test]
async fn test_index_scan_executor_found() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;

    let values = vec![vec![Value::Int(1)]];
    let mut insert_executor =
        InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), values, 0);
    insert_executor.next().await?;

    let key = 1i64.to_be_bytes();
    let mut executor = IndexScanExecutor::new(table_meta, buffer_pool, key.to_vec(), None);

    let result = executor.next().await?;
    match result {
        Some(ExecResult::Row(values)) => {
            assert_eq!(values.len(), 1);
            assert_eq!(values[0], Value::Int(1));
        }
        _ => panic!("Expected ExecResult::Row with Int(1)"),
    }

    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}

#[tokio::test]
async fn test_index_scan_executor_not_found() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;

    let key = 999i64.to_be_bytes();
    let mut executor = IndexScanExecutor::new(table_meta, buffer_pool, key.to_vec(), None);

    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}

#[tokio::test]
async fn test_insert_executor_single_row() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let values = vec![vec![Value::Int(1)]];
    let mut executor = InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), values, 0);

    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));
    let result = executor.next().await?;
    assert_eq!(result, None);

    let key = 1i64.to_be_bytes();
    let row_id = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("row should be indexed");
    let (_vh, tuple_bytes) = read_tuple_from_data_page(&buffer_pool, row_id).await?;
    assert!(!tuple_bytes.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_insert_executor_batch() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let values = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
    ];
    let mut executor = InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), values, 0);

    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(3)));
    let result = executor.next().await?;
    assert_eq!(result, None);

    for i in 1i64..=3 {
        let key = i.to_be_bytes();
        let row_id = table_meta
            .index_manager
            .search(&key)
            .await?
            .expect("row should be indexed");
        let (_vh, tuple_bytes) = read_tuple_from_data_page(&buffer_pool, row_id).await?;
        assert!(!tuple_bytes.is_empty());
    }

    Ok(())
}

#[tokio::test]
async fn test_update_executor() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let values = vec![vec![Value::Int(1)]];
    let mut insert_executor =
        InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), values, 0);
    insert_executor.next().await?;

    let key = 1i64.to_be_bytes();
    let new_value = Value::Int(100);
    let mut executor = UpdateExecutor::new(
        table_meta.clone(),
        buffer_pool,
        key.to_vec(),
        "id".to_string(),
        new_value,
        0,
    );

    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}

#[tokio::test]
async fn test_delete_executor() -> Result<()> {
    use rtsql::storage::{btree::IndexManager, page_format::RowId};

    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let buffer_pool_clone = buffer_pool.clone();
    let index_manager = tokio::task::spawn_blocking(move || {
        Arc::new(IndexManager::new(buffer_pool_clone).unwrap())
    })
    .await
    .unwrap();

    let key = 1i64.to_be_bytes();
    let row_id = RowId::new(0, 1);
    index_manager.insert(&key, row_id).await.unwrap();

    let mut executor = DeleteExecutor::new(index_manager.clone(), key.to_vec(), 0);

    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let result = executor.next().await?;
    assert_eq!(result, None);

    let found = index_manager.search(&key).await?;
    assert_eq!(found, None);

    Ok(())
}

#[tokio::test]
async fn test_insert_duplicate_key_error() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let values = vec![vec![Value::Int(1)]];
    let mut executor = InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), values, 0);
    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let values2 = vec![vec![Value::Int(1)]];
    let mut executor2 = InsertExecutor::new(table_meta, buffer_pool.clone(), values2, 0);
    let err = executor2.next().await.unwrap_err();
    assert!(matches!(err, rtsql::storage::StorageError::DuplicateKey));

    Ok(())
}

#[tokio::test]
async fn test_insert_stores_tuple_data() -> Result<()> {
    use rtsql::storage::page_format::deserialize_tuple;

    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_poul = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_poul.clone());
    table_mgr
        .create_table(
            "test",
            vec![
                ("id".to_string(), ColumnType::Int),
                ("name".to_string(), ColumnType::String(100)),
            ],
            "id",
        )
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let values = vec![vec![Value::Int(42), Value::String("hello".to_string())]];
    let mut executor = InsertExecutor::new(table_meta.clone(), buffer_poul.clone(), values, 0);

    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let key = 42i64.to_be_bytes();
    let row_id = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("row should be indexed");
    let (_vh, tuple_bytes) = read_tuple_from_data_page(&buffer_poul, row_id).await?;

    let schema = [ColumnType::Int, ColumnType::String(100)];
    let deserialized = deserialize_tuple(&tuple_bytes, &schema)?;
    assert_eq!(deserialized.len(), 2);
    assert_eq!(deserialized[0], Value::Int(42));
    assert_eq!(deserialized[1], Value::String("hello".to_string()));

    Ok(())
}

#[tokio::test]
async fn test_index_scan_returns_row_data() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table(
            "test",
            vec![
                ("id".to_string(), ColumnType::Int),
                ("name".to_string(), ColumnType::String(100)),
            ],
            "id",
        )
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let values = vec![vec![Value::Int(42), Value::String("hello".to_string())]];
    let mut insert_executor =
        InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), values, 0);
    insert_executor.next().await?;

    let key = 42i64.to_be_bytes();
    let mut executor = IndexScanExecutor::new(table_meta, buffer_pool, key.to_vec(), None);

    let result = executor.next().await?;
    match result {
        Some(ExecResult::Row(values)) => {
            assert_eq!(values.len(), 2);
            assert_eq!(values[0], Value::Int(42));
            assert_eq!(values[1], Value::String("hello".to_string()));
        }
        _ => panic!("Expected ExecResult::Row with row data"),
    }

    Ok(())
}

#[test]
fn test_exec_result_row_variant() {
    let row = ExecResult::Row(vec![Value::Int(42)]);
    match row {
        ExecResult::Row(ref values) => {
            assert_eq!(values.len(), 1);
            assert_eq!(values[0], Value::Int(42));
        }
        _ => panic!("Expected ExecResult::Row"),
    }
}

#[test]
fn test_response_rows_serialization() {
    use rtsql::network::Response;
    let resp = Response::QueryResult {
        rows: vec![vec![serde_json::Value::Number(1.into())]],
    };
    let json = serde_json::to_string(&resp).unwrap();
    let parsed: Response = serde_json::from_str(&json).unwrap();
    match parsed {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].len(), 1);
            assert_eq!(rows[0][0], serde_json::Value::Number(1.into()));
        }
        _ => panic!("Expected QueryResult"),
    }
}

// =============================================================================
// MVCC Tests (M7)
// =============================================================================

/// Test that InsertExecutor creates a VersionHeader with the correct tx_id.
#[tokio::test]
async fn test_insert_creates_version_header() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let values = vec![vec![Value::Int(1)]];
    let mut executor = InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), values, 5);

    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let key = 1i64.to_be_bytes();
    let row_id = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("row should be indexed");
    let (version_header, _tuple_bytes) = read_tuple_from_data_page(&buffer_pool, row_id).await?;
    assert_eq!(version_header.create_tx_id(), 5);
    assert_eq!(version_header.commit_tx_id(), None);

    Ok(())
}

/// Test that a Snapshot hides uncommitted tuples.
/// Insert with tx_id=1 (uncommitted), Snapshot(tx_id=2, active={1}) → NOT visible.
#[tokio::test]
async fn test_snapshot_hides_uncommitted() -> Result<()> {
    use rtsql::transaction::Snapshot;

    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let values = vec![vec![Value::Int(1)]];
    let mut insert_executor =
        InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), values, 1);
    insert_executor.next().await?;

    let snapshot = Snapshot::new(2, vec![1]);

    let key = 1i64.to_be_bytes();
    let mut executor =
        IndexScanExecutor::new(table_meta, buffer_pool, key.to_vec(), Some(snapshot));

    let result = executor.next().await?;
    assert_eq!(result, None, "uncommitted tuple should be invisible");

    Ok(())
}

/// Test that a Snapshot shows committed tuples.
/// Insert with tx_id=1, Snapshot(tx_id=2, active=[]) → visible.
#[tokio::test]
async fn test_snapshot_shows_committed() -> Result<()> {
    use rtsql::transaction::{Snapshot, VersionHeader};

    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let values = vec![vec![Value::Int(42)]];
    let mut insert_executor =
        InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), values, 1);
    insert_executor.next().await?;

    let key = 42i64.to_be_bytes();
    let row_id = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("row should exist");

    let (_vh, tuple_bytes) = read_tuple_from_data_page(&buffer_pool, row_id).await?;
    let committed_header = VersionHeader::new(1, Some(2));

    use rtsql::storage::write_tuple_to_data_page;
    let new_row_id =
        write_tuple_to_data_page(&buffer_pool, &table_meta, &committed_header, &tuple_bytes)
            .await?;

    table_meta.index_manager.update(&key, new_row_id).await?;

    let snapshot = Snapshot::new(3, vec![]);

    let mut executor =
        IndexScanExecutor::new(table_meta, buffer_pool, key.to_vec(), Some(snapshot));

    let result = executor.next().await?;
    assert!(
        matches!(result, Some(ExecResult::Row(_))),
        "committed tuple should be visible, got {:?}",
        result
    );

    Ok(())
}

/// Test that UpdateExecutor creates a new version chain entry.
/// Insert row → Update with new value → scan → verify new value visible.
#[tokio::test]
async fn test_update_creates_new_version() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone());
    table_mgr
        .create_table(
            "test",
            vec![
                ("id".to_string(), ColumnType::Int),
                ("name".to_string(), ColumnType::String(100)),
            ],
            "id",
        )
        .await?;
    let table_meta = table_mgr.get_table("test").await?;

    let values = vec![vec![Value::Int(1), Value::String("alice".to_string())]];
    let mut insert_executor =
        InsertExecutor::new(table_meta.clone(), buffer_pool.clone(), values, 1);
    insert_executor.next().await?;

    let key = 1i64.to_be_bytes();
    let mut update_executor = UpdateExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        key.to_vec(),
        "name".to_string(),
        Value::String("bob".to_string()),
        2,
    );
    let result = update_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let mut scan = IndexScanExecutor::new(table_meta, buffer_pool, key.to_vec(), None);
    let result = scan.next().await?;
    match result {
        Some(ExecResult::Row(values)) => {
            assert_eq!(values.len(), 2);
            assert_eq!(values[0], Value::Int(1));
            assert_eq!(values[1], Value::String("bob".to_string()));
        }
        _ => panic!("Expected Row with updated values, got {:?}", result),
    }

    Ok(())
}
