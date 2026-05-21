//! Tests for MVCC abort cleanup functionality (M10 Phase 5)
//!
//! These tests verify that uncommitted versions are properly cleaned up
//! when a transaction aborts.

use rtsql::storage::{write_tuple_to_data_page, BufferPool, FileStorage, TableManager, TableMeta};
use rtsql::transaction::{TransactionManager, VersionHeader};
use std::sync::Arc;
use tempfile::tempdir;

/// Helper to create test infrastructure
async fn setup() -> (Arc<TransactionManager>, Arc<BufferPool>, Arc<TableMeta>) {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(10, storage).unwrap());

    let table_manager = TableManager::new(buffer_pool.clone());
    table_manager
        .create_table(
            "test_table",
            vec![("id".to_string(), rtsql::storage::ColumnType::Int)],
            "id",
        )
        .await
        .unwrap();

    let table_meta = table_manager.get_table("test_table").await.unwrap();
    let tx_manager = Arc::new(TransactionManager::new());

    (tx_manager, buffer_pool, table_meta)
}

/// Test 1: Transaction inserts a row and aborts - row should not exist
#[tokio::test]
async fn test_abort_insert_row_not_visible() {
    let (tx_manager, buffer_pool, table_meta) = setup().await;

    // Begin transaction
    let tx = tx_manager.begin().await;
    let tx_id = tx.id();

    // Insert a row (manually create version for test)
    let version_header = VersionHeader::new(tx_id, None);
    let tuple_bytes = vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]; // Int value 1

    let row_id = write_tuple_to_data_page(&buffer_pool, &table_meta, &version_header, &tuple_bytes)
        .await
        .unwrap();

    // Record the version
    tx_manager.record_version(tx_id, row_id).await;

    // Insert into index
    let key = b"1";
    table_meta.index_manager.insert(key, row_id).await.unwrap();

    // Verify row is visible via index
    let found = table_meta.index_manager.search(key).await.unwrap();
    assert!(found.is_some(), "Row should be visible before abort");

    // Abort the transaction
    tx_manager
        .abort(tx, &buffer_pool, &table_meta)
        .await
        .unwrap();

    // Verify row is NOT visible after abort (index entry should be deleted)
    let found = table_meta.index_manager.search(key).await.unwrap();
    assert!(
        found.is_none(),
        "Row should not be visible after abort - index entry should be deleted"
    );
}

/// Test 2: Tx1 inserts v1 (value=10), commits. Tx2 updates to v2 (value=20), aborts.
/// After abort, should still see v1.
#[tokio::test]
async fn test_abort_update_reverts_to_previous_version() {
    let (tx_manager, buffer_pool, table_meta) = setup().await;

    // Tx1: Insert v1 (value=10)
    let tx1 = tx_manager.begin().await;
    let tx1_id = tx1.id();

    let version_header_v1 = VersionHeader::new(tx1_id, None);
    let tuple_bytes_v1 = vec![0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]; // Int value 10

    let row_id_v1 = write_tuple_to_data_page(
        &buffer_pool,
        &table_meta,
        &version_header_v1,
        &tuple_bytes_v1,
    )
    .await
    .unwrap();

    tx_manager.record_version(tx1_id, row_id_v1).await;

    // Insert into index
    let key = b"test_key";
    table_meta
        .index_manager
        .insert(key, row_id_v1)
        .await
        .unwrap();

    // Commit tx1
    tx_manager.commit(tx1, &buffer_pool).await.unwrap();

    // Verify v1 is visible
    let found = table_meta.index_manager.search(key).await.unwrap();
    assert_eq!(found, Some(row_id_v1), "v1 should be visible after tx1 commit");

    // Tx2: Update to v2 (value=20)
    let tx2 = tx_manager.begin().await;
    let tx2_id = tx2.id();

    let version_header_v2 = VersionHeader::new(tx2_id, None).with_next_version(row_id_v1);
    let tuple_bytes_v2 = vec![0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]; // Int value 20

    let row_id_v2 = write_tuple_to_data_page(
        &buffer_pool,
        &table_meta,
        &version_header_v2,
        &tuple_bytes_v2,
    )
    .await
    .unwrap();

    tx_manager.record_version(tx2_id, row_id_v2).await;

    // Update index to point to v2
    table_meta
        .index_manager
        .update(key, row_id_v2)
        .await
        .unwrap();

    // Verify v2 is visible
    let found = table_meta.index_manager.search(key).await.unwrap();
    assert_eq!(found, Some(row_id_v2), "v2 should be visible before tx2 abort");

    // Abort tx2
    tx_manager
        .abort(tx2, &buffer_pool, &table_meta)
        .await
        .unwrap();

    // After abort, should still see v1 (index should be updated to point back to v1)
    let found = table_meta.index_manager.search(key).await.unwrap();
    assert_eq!(
        found, Some(row_id_v1),
        "v1 should still be visible after tx2 abort - index should point back to v1"
    );
}

/// Test 3: Multiple aborts in sequence
#[tokio::test]
async fn test_multiple_aborts() {
    let (tx_manager, buffer_pool, table_meta) = setup().await;

    // Tx1: Insert and commit
    let tx1 = tx_manager.begin().await;
    let tx1_id = tx1.id();

    let version_header = VersionHeader::new(tx1_id, None);
    let tuple_bytes = vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

    let row_id = write_tuple_to_data_page(&buffer_pool, &table_meta, &version_header, &tuple_bytes)
        .await
        .unwrap();

    tx_manager.record_version(tx1_id, row_id).await;
    table_meta.index_manager.insert(b"key1", row_id).await.unwrap();
    tx_manager.commit(tx1, &buffer_pool).await.unwrap();

    // Tx2: Insert and abort
    let tx2 = tx_manager.begin().await;
    let tx2_id = tx2.id();

    let version_header2 = VersionHeader::new(tx2_id, None);
    let row_id2 = write_tuple_to_data_page(
        &buffer_pool,
        &table_meta,
        &version_header2,
        &tuple_bytes,
    )
    .await
    .unwrap();

    tx_manager.record_version(tx2_id, row_id2).await;
    table_meta.index_manager.insert(b"key2", row_id2).await.unwrap();

    // Tx3: Insert and abort
    let tx3 = tx_manager.begin().await;
    let tx3_id = tx3.id();

    let version_header3 = VersionHeader::new(tx3_id, None);
    let row_id3 = write_tuple_to_data_page(
        &buffer_pool,
        &table_meta,
        &version_header3,
        &tuple_bytes,
    )
    .await
    .unwrap();

    tx_manager.record_version(tx3_id, row_id3).await;
    table_meta.index_manager.insert(b"key3", row_id3).await.unwrap();

    // Abort both tx2 and tx3
    tx_manager
        .abort(tx2, &buffer_pool, &table_meta)
        .await
        .unwrap();
    tx_manager
        .abort(tx3, &buffer_pool, &table_meta)
        .await
        .unwrap();

    // key1 should still exist
    assert!(
        table_meta.index_manager.search(b"key1").await.unwrap().is_some(),
        "Committed key1 should still exist"
    );

    // key2 and key3 should not exist
    assert!(
        table_meta.index_manager.search(b"key2").await.unwrap().is_none(),
        "Aborted key2 should not exist"
    );
    assert!(
        table_meta.index_manager.search(b"key3").await.unwrap().is_none(),
        "Aborted key3 should not exist"
    );
}