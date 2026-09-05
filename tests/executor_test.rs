//! Executor unit tests

use rtsql::executor::{
    ColumnDef, CreateTableExecutor, DataScanExecutor, DataScanNode, DeleteExecutor, ExecResult,
    Executor, IndexScanAllExecutor, IndexScanExecutor, InsertExecutor, PhysicalPlan, ScanExecutor,
    UpdateExecutor, Value, ValueRef,
};
use rtsql::storage::{
    data::TableManager,
    page_format::{deserialize_value_refs, ColumnType},
    read_tuple_from_data_page, BufferPool, FileStorage, Result, StorageError,
};
use rtsql::transaction::TransactionManager;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_scan_executor_full_table() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
    ];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
        None,
    );
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
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![vec![Value::Int(1)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
        None,
    );
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
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
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
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![vec![Value::Int(1)]];
    let mut executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
        None,
    );

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
    let tuple_bytes =
        read_tuple_from_data_page(&buffer_pool, row_id, |_vh, bytes| Ok(bytes.to_vec())).await?;
    assert!(!tuple_bytes.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_insert_executor_batch() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
    ];
    let mut executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
        None,
    );

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
        let tuple_bytes =
            read_tuple_from_data_page(&buffer_pool, row_id, |_vh, bytes| Ok(bytes.to_vec()))
                .await?;
        assert!(!tuple_bytes.is_empty());
    }

    Ok(())
}

#[tokio::test]
async fn test_update_executor() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![vec![Value::Int(1)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        0,
        None,
    );
    insert_executor.next().await?;

    let key = 1i64.to_be_bytes();
    let new_value = Value::Int(100);
    let mut executor = UpdateExecutor::new(
        table_meta.clone(),
        buffer_pool,
        tx_manager,
        key.to_vec(),
        "id".to_string(),
        new_value,
        0,
        None,
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
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let buffer_pool_clone = buffer_pool.clone();
    let index_manager = tokio::task::spawn_blocking(move || {
        Arc::new(IndexManager::new(buffer_pool_clone).unwrap())
    })
    .await
    .unwrap();

    let key = 1i64.to_be_bytes();
    let row_id = RowId::new(0, 1);
    index_manager.insert(&key, row_id).await.unwrap();

    let tx_manager = Arc::new(TransactionManager::new());
    let mut executor = DeleteExecutor::new(
        index_manager.clone(),
        buffer_pool.clone(),
        tx_manager,
        "test".to_string(),
        key.to_vec(),
        0,
        None,
    );

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
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![vec![Value::Int(1)]];
    let mut executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        0,
        None,
    );
    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let values2 = vec![vec![Value::Int(1)]];
    let mut executor2 = InsertExecutor::new(
        table_meta,
        buffer_pool.clone(),
        tx_manager,
        values2,
        0,
        None,
    );
    let err = executor2.next().await.unwrap_err();
    assert!(matches!(err, rtsql::storage::StorageError::DuplicateKey));

    Ok(())
}

#[tokio::test]
async fn test_insert_stores_tuple_data() -> Result<()> {
    use rtsql::storage::page_format::deserialize_tuple;

    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_poul = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_poul.clone(), storage)
        .await
        .unwrap();
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
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![vec![Value::Int(42), Value::String("hello".to_string())]];
    let mut executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_poul.clone(),
        tx_manager,
        values,
        0,
        None,
    );

    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let key = 42i64.to_be_bytes();
    let row_id = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("row should be indexed");
    let (_vh, tuple_bytes) =
        read_tuple_from_data_page(&buffer_poul, row_id, |vh, bytes| Ok((vh, bytes.to_vec())))
            .await?;

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
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
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
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![vec![Value::Int(42), Value::String("hello".to_string())]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
        None,
    );
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
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![vec![Value::Int(1)]];
    let mut executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        5,
        None,
    );

    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(1)));

    let key = 1i64.to_be_bytes();
    let row_id = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("row should be indexed");
    let version_header =
        read_tuple_from_data_page(&buffer_pool, row_id, |vh, _bytes| Ok(vh)).await?;
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
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![vec![Value::Int(1)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        1,
        None,
    );
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
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![vec![Value::Int(42)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        1,
        None,
    );
    insert_executor.next().await?;

    let key = 42i64.to_be_bytes();
    let row_id = table_meta
        .index_manager
        .search(&key)
        .await?
        .expect("row should exist");

    let (_vh, tuple_bytes) =
        read_tuple_from_data_page(&buffer_pool, row_id, |vh, bytes| Ok((vh, bytes.to_vec())))
            .await?;
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
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
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
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![vec![Value::Int(1), Value::String("alice".to_string())]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager.clone(),
        values,
        1,
        None,
    );
    insert_executor.next().await?;

    let key = 1i64.to_be_bytes();
    let mut update_executor = UpdateExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        key.to_vec(),
        "name".to_string(),
        Value::String("bob".to_string()),
        2,
        None,
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

// =============================================================================
// CreateTableExecutor Tests (M9)
// =============================================================================

#[tokio::test]
async fn test_create_table_executor_success() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
    let table_manager = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();

    use rtsql::database::Database;
    use rtsql::executor::ColumnType;
    let wal_writer =
        Arc::new(rtsql::wal::WalWriter::open(std::path::Path::new(":memory:")).unwrap());
    let database = Arc::new(Database {
        buffer_pool: buffer_pool.clone(),
        table_manager: table_manager.clone(),
        transaction_manager: Arc::new(rtsql::transaction::TransactionManager::new()),
        wal_writer: wal_writer.clone(),
        checkpoint_manager: Arc::new(rtsql::wal::CheckpointManager::new(
            std::path::Path::new(":memory:"),
            wal_writer.clone(),
            buffer_pool.clone(),
        )),
        wal_buffer: Arc::new(rtsql::wal::WALBuffer::new(wal_writer, 100, 100)),
        plan_cache: Arc::new(rtsql::plan_cache::PlanCache::new()),
    });

    let plan = PhysicalPlan::CreateTable(rtsql::executor::CreateTableNode {
        table_name: "users".to_string(),
        columns: vec![
            ColumnDef::new("id".to_string(), ColumnType::Int),
            ColumnDef::new("name".to_string(), ColumnType::String),
        ],
        primary_key: Some("id".to_string()),
    });

    let mut executor = CreateTableExecutor::new(plan, database);
    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(0)));

    // Verify table was created
    assert!(table_manager.table_exists("users"));

    Ok(())
}

#[tokio::test]
async fn test_create_table_executor_already_exists() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
    let table_manager = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();

    use rtsql::database::Database;
    use rtsql::executor::ColumnType as ExecColumnType;
    use rtsql::storage::ColumnType as StorageColumnType;
    let wal_writer =
        Arc::new(rtsql::wal::WalWriter::open(std::path::Path::new(":memory:")).unwrap());
    let database = Arc::new(Database {
        buffer_pool: buffer_pool.clone(),
        table_manager: table_manager.clone(),
        transaction_manager: Arc::new(rtsql::transaction::TransactionManager::new()),
        wal_writer: wal_writer.clone(),
        checkpoint_manager: Arc::new(rtsql::wal::CheckpointManager::new(
            std::path::Path::new(":memory:"),
            wal_writer.clone(),
            buffer_pool.clone(),
        )),
        wal_buffer: Arc::new(rtsql::wal::WALBuffer::new(wal_writer, 100, 100)),
        plan_cache: Arc::new(rtsql::plan_cache::PlanCache::new()),
    });

    // Create table first time (using storage::ColumnType)
    table_manager
        .create_table(
            "users",
            vec![("id".to_string(), StorageColumnType::Int)],
            "id",
        )
        .await?;

    // Try to create again with same name (using executor::ColumnType)
    let plan = PhysicalPlan::CreateTable(rtsql::executor::CreateTableNode {
        table_name: "users".to_string(),
        columns: vec![ColumnDef::new("id".to_string(), ExecColumnType::Int)],
        primary_key: Some("id".to_string()),
    });

    let mut executor = CreateTableExecutor::new(plan, database);
    let result = executor.next().await;
    assert!(result.is_err());
    match result.unwrap_err() {
        StorageError::TableAlreadyExists(name) => assert_eq!(name, "users"),
        e => panic!("Expected TableAlreadyExists error, got {:?}", e),
    }

    Ok(())
}

// =============================================================================
// DropTableExecutor Tests (M9)
// =============================================================================

#[tokio::test]
async fn test_drop_table_executor_success() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
    let table_manager = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();

    use rtsql::database::Database;
    use rtsql::executor::DropTableExecutor;
    let wal_writer =
        Arc::new(rtsql::wal::WalWriter::open(std::path::Path::new(":memory:")).unwrap());
    let database = Arc::new(Database {
        buffer_pool: buffer_pool.clone(),
        table_manager: table_manager.clone(),
        transaction_manager: Arc::new(rtsql::transaction::TransactionManager::new()),
        wal_writer: wal_writer.clone(),
        checkpoint_manager: Arc::new(rtsql::wal::CheckpointManager::new(
            std::path::Path::new(":memory:"),
            wal_writer.clone(),
            buffer_pool.clone(),
        )),
        wal_buffer: Arc::new(rtsql::wal::WALBuffer::new(wal_writer, 100, 100)),
        plan_cache: Arc::new(rtsql::plan_cache::PlanCache::new()),
    });

    // Create table first
    table_manager
        .create_table(
            "test_table",
            vec![("id".to_string(), ColumnType::Int)],
            "id",
        )
        .await?;
    assert!(table_manager.table_exists("test_table"));

    // Drop the table
    let plan = PhysicalPlan::DropTable(rtsql::executor::DropTableNode {
        table_name: "test_table".to_string(),
        if_exists: false,
    });

    let mut executor = DropTableExecutor::new(plan, database);
    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(0)));

    // Verify table no longer exists
    assert!(!table_manager.table_exists("test_table"));

    Ok(())
}

#[tokio::test]
async fn test_drop_table_executor_not_found() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
    let table_manager = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();

    use rtsql::database::Database;
    use rtsql::executor::DropTableExecutor;
    let wal_writer =
        Arc::new(rtsql::wal::WalWriter::open(std::path::Path::new(":memory:")).unwrap());
    let database = Arc::new(Database {
        buffer_pool: buffer_pool.clone(),
        table_manager: table_manager.clone(),
        transaction_manager: Arc::new(rtsql::transaction::TransactionManager::new()),
        wal_writer: wal_writer.clone(),
        checkpoint_manager: Arc::new(rtsql::wal::CheckpointManager::new(
            std::path::Path::new(":memory:"),
            wal_writer.clone(),
            buffer_pool.clone(),
        )),
        wal_buffer: Arc::new(rtsql::wal::WALBuffer::new(wal_writer, 100, 100)),
        plan_cache: Arc::new(rtsql::plan_cache::PlanCache::new()),
    });

    // Try to drop a non-existent table without IF EXISTS
    let plan = PhysicalPlan::DropTable(rtsql::executor::DropTableNode {
        table_name: "nonexistent".to_string(),
        if_exists: false,
    });

    let mut executor = DropTableExecutor::new(plan, database);
    let result = executor.next().await;
    assert!(result.is_err());
    match result.unwrap_err() {
        StorageError::TableNotFound(name) => assert_eq!(name, "nonexistent"),
        e => panic!("Expected TableNotFound error, got {:?}", e),
    }

    Ok(())
}

#[tokio::test]
async fn test_drop_table_if_exists_success() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
    let table_manager = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();

    use rtsql::database::Database;
    use rtsql::executor::DropTableExecutor;
    let wal_writer =
        Arc::new(rtsql::wal::WalWriter::open(std::path::Path::new(":memory:")).unwrap());
    let database = Arc::new(Database {
        buffer_pool: buffer_pool.clone(),
        table_manager: table_manager.clone(),
        transaction_manager: Arc::new(rtsql::transaction::TransactionManager::new()),
        wal_writer: wal_writer.clone(),
        checkpoint_manager: Arc::new(rtsql::wal::CheckpointManager::new(
            std::path::Path::new(":memory:"),
            wal_writer.clone(),
            buffer_pool.clone(),
        )),
        wal_buffer: Arc::new(rtsql::wal::WALBuffer::new(wal_writer, 100, 100)),
        plan_cache: Arc::new(rtsql::plan_cache::PlanCache::new()),
    });

    // Drop a non-existent table with IF EXISTS - should succeed without error
    let plan = PhysicalPlan::DropTable(rtsql::executor::DropTableNode {
        table_name: "nonexistent".to_string(),
        if_exists: true,
    });

    let mut executor = DropTableExecutor::new(plan, database);
    let result = executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(0)));

    Ok(())
}

// =============================================================================
// FilterExecutor Tests (M9)
// =============================================================================

#[tokio::test]
async fn test_filter_executor_gt() -> Result<()> {
    use rtsql::executor::{ColumnExpression, ConstantExpression, ExpressionRef};
    use rtsql::executor::{ComparisonOp, ComparisonPredicate, FilterExecutor, PredicateRef};
    use std::sync::Arc;

    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    // Insert rows: 1, 2, 3, 4, 5
    let values = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
        vec![Value::Int(4)],
        vec![Value::Int(5)],
    ];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
        None,
    );
    insert_executor.next().await?;

    // Create predicate: id > 3
    let pred: PredicateRef = Arc::new(ComparisonPredicate {
        left: Arc::new(ColumnExpression {
            column_name: "id".to_string(),
            column_index: 0,
        }) as ExpressionRef,
        op: ComparisonOp::Gt,
        right: Arc::new(ConstantExpression {
            value: Value::Int(3),
        }) as ExpressionRef,
    });

    // Create scan executor as input
    let scan_executor = ScanExecutor::new(table_meta, buffer_pool, None);

    // Create filter executor
    let mut filter_executor = FilterExecutor::new(Box::new(scan_executor), pred);

    // Collect filtered results
    let mut results = Vec::new();
    while let Some(result) = filter_executor.next().await? {
        if let ExecResult::Row(values) = result {
            results.push(values);
        }
    }

    // Should get rows with id > 3: [4], [5]
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], vec![Value::Int(4)]);
    assert_eq!(results[1], vec![Value::Int(5)]);

    Ok(())
}

#[tokio::test]
async fn test_filter_executor_and() -> Result<()> {
    use rtsql::executor::{ColumnExpression, ConstantExpression, ExpressionRef};
    use rtsql::executor::{
        ComparisonOp, ComparisonPredicate, FilterExecutor, LogicalOp, LogicalPredicate,
        PredicateRef,
    };
    use std::sync::Arc;

    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    // Insert rows: 1, 2, 3, 4, 5
    let values = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
        vec![Value::Int(4)],
        vec![Value::Int(5)],
    ];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
        None,
    );
    insert_executor.next().await?;

    // Create predicate: id >= 2 AND id < 5
    let pred1: PredicateRef = Arc::new(ComparisonPredicate {
        left: Arc::new(ColumnExpression {
            column_name: "id".to_string(),
            column_index: 0,
        }) as ExpressionRef,
        op: ComparisonOp::Ge,
        right: Arc::new(ConstantExpression {
            value: Value::Int(2),
        }) as ExpressionRef,
    });

    let pred2: PredicateRef = Arc::new(ComparisonPredicate {
        left: Arc::new(ColumnExpression {
            column_name: "id".to_string(),
            column_index: 0,
        }) as ExpressionRef,
        op: ComparisonOp::Lt,
        right: Arc::new(ConstantExpression {
            value: Value::Int(5),
        }) as ExpressionRef,
    });

    let pred: PredicateRef = Arc::new(LogicalPredicate {
        left: pred1,
        op: LogicalOp::And,
        right: pred2,
    });

    // Create scan executor as input
    let scan_executor = ScanExecutor::new(table_meta, buffer_pool, None);

    // Create filter executor
    let mut filter_executor = FilterExecutor::new(Box::new(scan_executor), pred);

    // Collect filtered results
    let mut results = Vec::new();
    while let Some(result) = filter_executor.next().await? {
        if let ExecResult::Row(values) = result {
            results.push(values);
        }
    }

    // Should get rows with 2 <= id < 5: [2], [3], [4]
    assert_eq!(results.len(), 3);
    assert_eq!(results[0], vec![Value::Int(2)]);
    assert_eq!(results[1], vec![Value::Int(3)]);
    assert_eq!(results[2], vec![Value::Int(4)]);

    Ok(())
}

#[tokio::test]
async fn test_filter_executor_empty_result() -> Result<()> {
    use rtsql::executor::{ColumnExpression, ConstantExpression, ExpressionRef};
    use rtsql::executor::{ComparisonOp, ComparisonPredicate, FilterExecutor, PredicateRef};
    use std::sync::Arc;

    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    // Insert rows: 1, 2, 3
    let values = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
    ];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
        None,
    );
    insert_executor.next().await?;

    // Create predicate: id > 100 (no rows satisfy this)
    let pred: PredicateRef = Arc::new(ComparisonPredicate {
        left: Arc::new(ColumnExpression {
            column_name: "id".to_string(),
            column_index: 0,
        }) as ExpressionRef,
        op: ComparisonOp::Gt,
        right: Arc::new(ConstantExpression {
            value: Value::Int(100),
        }) as ExpressionRef,
    });

    // Create scan executor as input
    let scan_executor = ScanExecutor::new(table_meta, buffer_pool, None);

    // Create filter executor
    let mut filter_executor = FilterExecutor::new(Box::new(scan_executor), pred);

    // Collect filtered results (should be empty)
    let mut count = 0;
    while let Some(result) = filter_executor.next().await? {
        if let ExecResult::Row(_) = result {
            count += 1;
        }
    }

    assert_eq!(count, 0);

    Ok(())
}

// =============================================================================
// IndexScanAllExecutor Tests (M18 Phase2)
// =============================================================================

#[tokio::test]
async fn test_index_scan_all_executor_basic() -> Result<()> {
    use rtsql::storage::write_tuple_to_data_page;
    use rtsql::transaction::VersionHeader;

    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;

    // Write data pages and insert duplicate keys directly
    let key = 1i64.to_be_bytes();
    let version_header = VersionHeader::new(0, None);
    let tuple_bytes = vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]; // Int(1) serialized

    let row_id1 =
        write_tuple_to_data_page(&buffer_pool, &table_meta, &version_header, &tuple_bytes).await?;
    let row_id2 =
        write_tuple_to_data_page(&buffer_pool, &table_meta, &version_header, &tuple_bytes).await?;
    let row_id3 =
        write_tuple_to_data_page(&buffer_pool, &table_meta, &version_header, &tuple_bytes).await?;

    table_meta.index_manager.insert(&key, row_id1).await?;
    table_meta.index_manager.insert(&key, row_id2).await?;
    table_meta.index_manager.insert(&key, row_id3).await?;

    // Search for all rows with key = 1
    let mut executor = IndexScanAllExecutor::new(table_meta, buffer_pool, key.to_vec(), None);

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
async fn test_index_scan_all_executor_empty() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;

    // Search for non-existent key
    let key = 999i64.to_be_bytes().to_vec();
    let mut executor = IndexScanAllExecutor::new(table_meta, buffer_pool, key, None);

    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}

#[tokio::test]
async fn test_index_scan_all_executor_single() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    // Insert 1 row (unique key scenario, but search_all still works)
    let key = 42i64.to_be_bytes().to_vec();
    let values = vec![vec![Value::Int(42)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
        None,
    );
    insert_executor.next().await?;

    // Search for all rows with key = 42
    let mut executor = IndexScanAllExecutor::new(table_meta, buffer_pool, key.clone(), None);

    let mut row_count = 0;
    while let Some(result) = executor.next().await? {
        match result {
            ExecResult::Row(values) => {
                assert_eq!(values.len(), 1);
                assert_eq!(values[0], Value::Int(42));
                row_count += 1;
            }
            _ => panic!("Expected ExecResult::Row"),
        }
    }
    assert_eq!(row_count, 1);

    // Verify no more results
    let result = executor.next().await?;
    assert_eq!(result, None);

    Ok(())
}

/// M36: Verify Value::as_value_ref borrows from String
#[tokio::test]
async fn test_m36_value_as_value_ref() {
    let v = Value::String("hello".to_string());
    let vr = v.as_value_ref();
    assert_eq!(vr, ValueRef::Text("hello"));
}

/// M36: Verify deserialize_value_refs borrows from input data
#[tokio::test]
async fn test_m36_deserialize_value_refs_borrow() {
    let data = vec![0x02, 0x05, 0x00, b'h', b'e', b'l', b'l', b'o'];
    let schema = [ColumnType::String(100)];
    let refs = deserialize_value_refs(&data, &schema).unwrap();
    match &refs[0] {
        ValueRef::Text(s) => {
            let s_ptr = s.as_ptr();
            let data_ptr = data.as_ptr();
            assert!(s_ptr >= data_ptr, "Text must borrow from data");
            assert!(
                s_ptr < unsafe { data_ptr.add(data.len()) },
                "Text must point inside data"
            );
        }
        _ => panic!("expected Text"),
    }
}

// =====================================================================
// M19: DataScan 路径 - 全表扫描直接遍历数据页链表
// =====================================================================

/// M19 T1: DataScanExecutor 全表扫描（单数据页，3 行）
#[tokio::test]
async fn test_data_scan_executor_full_table() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let values = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
    ];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
        None,
    );
    let result = insert_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(3)));

    let mut executor = DataScanExecutor::new(table_meta, buffer_pool, None, None, None);

    let mut row_count = 0;
    let mut collected_ids: Vec<i64> = Vec::new();
    while let Some(result) = executor.next().await? {
        match result {
            ExecResult::Row(values) => {
                row_count += 1;
                if let Value::Int(id) = &values[0] {
                    collected_ids.push(*id);
                }
            }
            _ => panic!("Expected ExecResult::Row"),
        }
    }
    assert_eq!(row_count, 3);
    collected_ids.sort();
    assert_eq!(collected_ids, vec![1, 2, 3]);

    Ok(())
}

/// M19 T1: 空表（data_page_head 指向未分配页）应返回 None
#[tokio::test]
async fn test_data_scan_executor_empty_table() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;

    let mut executor = DataScanExecutor::new(table_meta, buffer_pool, None, None, None);

    // 第一次 next() 应返回 Ok(None) — 空表无数据
    let result = executor.next().await?;
    assert_eq!(result, None, "Empty table should return None immediately");

    Ok(())
}

/// M19 T1: 多数据页链表扫描（>1 页 4KB 数据）
#[tokio::test]
async fn test_data_scan_executor_multi_page() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table(
            "test",
            vec![
                ("id".to_string(), ColumnType::Int),
                ("name".to_string(), ColumnType::String(50)),
            ],
            "id",
        )
        .await?;

    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    // 插入 200 行，强制数据页链表
    let values: Vec<Vec<Value>> = (0..200)
        .map(|i| vec![Value::Int(i), Value::String(format!("row_{:03}", i))])
        .collect();
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
        None,
    );
    let result = insert_executor.next().await?;
    assert_eq!(result, Some(ExecResult::AffectedRows(200)));

    let mut executor = DataScanExecutor::new(table_meta, buffer_pool, None, None, None);

    let mut row_count = 0;
    while let Some(result) = executor.next().await? {
        if let ExecResult::Row(_) = result {
            row_count += 1;
        } else {
            panic!("Expected ExecResult::Row");
        }
    }
    assert_eq!(
        row_count, 200,
        "Should scan all 200 rows across multiple pages"
    );

    Ok(())
}

/// M19 T1: 流式 next() — 每次调用返回一行（不预加载）
#[tokio::test]
async fn test_data_scan_executor_streaming() -> Result<()> {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;

    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    let values: Vec<Vec<Value>> = (1..=5).map(|i| vec![Value::Int(i)]).collect();
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        0,
        None,
    );
    insert_executor.next().await?;

    let mut executor = DataScanExecutor::new(table_meta, buffer_pool, None, None, None);

    // 流式验证：连续 5 次 next() 各返回 1 行，第 6 次返回 None
    for i in 1..=5 {
        let result = executor.next().await?;
        match result {
            Some(ExecResult::Row(row)) => {
                assert_eq!(row.len(), 1, "Row should have 1 column");
                if let Value::Int(id) = &row[0] {
                    assert!(*id >= 1 && *id <= 5, "id should be in 1..=5, got {}", id);
                } else {
                    panic!("Expected Value::Int");
                }
            }
            _ => panic!("Expected ExecResult::Row at iteration {}", i),
        }
    }
    // 第 6 次应返回 None
    let result = executor.next().await?;
    assert_eq!(result, None, "6th next() should return None after 5 rows");

    Ok(())
}

/// M19 T2: 未提交 tuple 对 snapshot 不可见 — scan 应跳过
#[tokio::test]
async fn test_data_scan_mvcc_uncommitted_invisible() -> Result<()> {
    use rtsql::transaction::Snapshot;

    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    // Insert with tx_id=1 (uncommitted — InsertExecutor doesn't commit by default)
    let values = vec![vec![Value::Int(1)], vec![Value::Int(2)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        1, // tx_id=1, no commit → commit_tx_id=None
        None,
    );
    insert_executor.next().await?;

    // Snapshot at tx_id=2, tx1 still active → uncommitted rows invisible
    let snapshot = Snapshot::new(2, vec![1]);

    let mut executor = DataScanExecutor::new(table_meta, buffer_pool, Some(snapshot), None, None);

    let mut row_count = 0;
    while let Some(result) = executor.next().await? {
        if let ExecResult::Row(_) = result {
            row_count += 1;
        }
    }
    assert_eq!(
        row_count, 0,
        "Uncommitted rows should be invisible to active snapshot"
    );

    Ok(())
}

/// M19 T2: 已提交 tuple 对 snapshot 可见 — scan 返回所有行
#[tokio::test]
async fn test_data_scan_mvcc_committed_visible() -> Result<()> {
    use rtsql::storage::write_tuple_to_data_page;
    use rtsql::transaction::{Snapshot, VersionHeader};

    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());

    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await?;
    let table_meta = table_mgr.get_table("test").await?;
    let tx_manager = Arc::new(TransactionManager::new());

    // Insert with tx_id=1, uncommitted initially
    let values = vec![vec![Value::Int(10)], vec![Value::Int(20)]];
    let mut insert_executor = InsertExecutor::new(
        table_meta.clone(),
        buffer_pool.clone(),
        tx_manager,
        values,
        1, // tx_id=1
        None,
    );
    insert_executor.next().await?;

    // Manually commit: rewrite each slot's VersionHeader with commit_tx_id=2
    let keys = vec![10i64.to_be_bytes(), 20i64.to_be_bytes()];
    for key in &keys {
        let old_rid = table_meta
            .index_manager
            .search(key)
            .await?
            .expect("row should exist");
        let (_vh, tuple_bytes) =
            read_tuple_from_data_page(&buffer_pool, old_rid, |vh, bytes| Ok((vh, bytes.to_vec())))
                .await?;
        // Rewrite slot with committed header
        let committed_header = VersionHeader::new(1, Some(2));
        let new_rid =
            write_tuple_to_data_page(&buffer_pool, &table_meta, &committed_header, &tuple_bytes)
                .await?;
        // Update index to point to new committed slot
        table_meta.index_manager.update(key, new_rid).await?;
    }

    // Snapshot at tx_id=3, no active tx → committed rows visible
    let snapshot = Snapshot::new(3, vec![]);

    let mut executor = DataScanExecutor::new(table_meta, buffer_pool, Some(snapshot), None, None);

    let mut row_count = 0;
    let mut collected_ids: Vec<i64> = Vec::new();
    while let Some(result) = executor.next().await? {
        if let ExecResult::Row(row) = result {
            row_count += 1;
            if let Value::Int(id) = &row[0] {
                collected_ids.push(*id);
            }
        }
    }
    collected_ids.sort();
    assert_eq!(row_count, 2, "Committed rows should be visible");
    assert_eq!(collected_ids, vec![10, 20]);

    Ok(())
}

/// M19 T3: Planner 路由 — `SELECT *` (无 WHERE) 生成 `PhysicalPlan::DataScan`
#[tokio::test]
async fn test_planner_no_where_routes_to_data_scan() -> Result<()> {
    use rtsql::parser::{parse_sql, PlanBuilder};

    let mut builder = PlanBuilder::new();
    builder.register_table("test", vec!["id".to_string(), "name".to_string()], "id");

    let sql = "SELECT id, name FROM test";
    let stmts = parse_sql(sql).unwrap();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::DataScan(DataScanNode {
            table_name,
            columns,
            predicate,
            scan_cap,
        }) => {
            assert_eq!(table_name, "test");
            assert_eq!(columns, vec!["id", "name"]);
            // MS07-T06: no WHERE / no LIMIT here — no pushed predicate or cap.
            assert!(predicate.is_none());
            assert_eq!(scan_cap, None);
        }
        other => panic!("Expected DataScan, got {:?}", other),
    }
    Ok(())
}

/// M19 T3: Planner 路由 — PK 等值 WHERE 仍走 IndexScan (M19 不改变这条路径)
#[tokio::test]
async fn test_planner_with_pk_equality_keeps_index_scan() -> Result<()> {
    use rtsql::parser::{parse_sql, PlanBuilder};

    let mut builder = PlanBuilder::new();
    builder.register_table("test", vec!["id".to_string(), "name".to_string()], "id");

    let sql = "SELECT * FROM test WHERE id = 1";
    let stmts = parse_sql(sql).unwrap();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::IndexScan(node) => {
            assert_eq!(node.table_name, "test");
        }
        other => panic!("Expected IndexScan for PK equality, got {:?}", other),
    }
    Ok(())
}

/// M19 T4 + MS07-T06: Planner 路由 — 非 PK WHERE → `DataScan`（谓词行内下推，无 Filter 包裹）
#[tokio::test]
async fn test_planner_non_pk_where_routes_to_filter_data_scan() -> Result<()> {
    use rtsql::parser::{parse_sql, PlanBuilder};

    let mut builder = PlanBuilder::new();
    builder.register_table("test", vec!["id".to_string(), "name".to_string()], "id");

    let sql = "SELECT * FROM test WHERE name = 'foo'";
    let stmts = parse_sql(sql).unwrap();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    match plan {
        PhysicalPlan::DataScan(node) => {
            assert_eq!(node.table_name, "test");
            assert!(
                node.predicate.is_some(),
                "MS07-T06: non-PK WHERE must carry its predicate inside DataScan"
            );
        }
        other => panic!("Expected DataScan with pushed predicate, got {:?}", other),
    }
    Ok(())
}

/// M19 T4: Planner 路由 — `WHERE id = 1 AND name = 'x'` 含 PK 等值 → 保持 `Filter(Scan)` (兜底)
#[tokio::test]
async fn test_planner_pk_and_other_keeps_filter_scan() -> Result<()> {
    use rtsql::executor::FilterNode;
    use rtsql::parser::{parse_sql, PlanBuilder};

    let mut builder = PlanBuilder::new();
    builder.register_table("test", vec!["id".to_string(), "name".to_string()], "id");

    let sql = "SELECT * FROM test WHERE id = 1 AND name = 'foo'";
    let stmts = parse_sql(sql).unwrap();
    let plan = builder.build_plan(&stmts[0]).unwrap();

    // has_pk_equality returns true for `id = 1 AND name = 'foo'`, so the
    // planner keeps the original Scan inside Filter. (A more sophisticated
    // optimizer would push down the name filter; not M19's scope.)
    match plan {
        PhysicalPlan::Filter(FilterNode { input, .. }) => match *input {
            PhysicalPlan::Scan(_) => { /* expected */ }
            other => panic!("Expected Filter over Scan, got {:?}", other),
        },
        other => panic!("Expected Filter, got {:?}", other),
    }
    Ok(())
}
