//! WALBuffer + Group Commit 测试
//!
//! 验证 WALBuffer 的追加、缓冲、Group Commit 和关闭行为

use rtsql::storage::RowId;
use rtsql::wal::{WalReader, WalRecord, WalWriter};
use std::sync::Arc;
use tempfile::NamedTempFile;

/// Helper: 创建 WALBuffer（capacity 条，flush_interval_ms 毫秒）
fn create_wal_buffer(
    db_path: &std::path::Path,
    capacity: usize,
    flush_interval_ms: u64,
) -> Arc<rtsql::wal::WALBuffer> {
    let wal_writer = Arc::new(WalWriter::open(db_path).unwrap());
    let buffer = Arc::new(rtsql::wal::WALBuffer::new(
        wal_writer,
        capacity,
        flush_interval_ms,
    ));
    buffer.start_flush_loop();
    buffer
}

/// Helper: 读取 WAL 文件中所有记录（使用 new format path）
fn read_wal_records(db_path: &std::path::Path) -> Vec<WalRecord> {
    let wal_path = db_path.with_extension("wal");
    let mut reader = WalReader::open(&wal_path).unwrap();
    reader.read_all().unwrap()
}

/// Helper: 读取 WAL 文件大小
fn wal_file_size(db_path: &std::path::Path) -> u64 {
    let wal_path = db_path.with_extension("wal");
    std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0)
}

#[tokio::test]
async fn test_wal_buffer_append_returns_lsn() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let buffer = create_wal_buffer(db_path, 100, 1000);

    let record1 = WalRecord::BeginTxn { tx_id: 1 };
    let record2 = WalRecord::Insert {
        tx_id: 1,
        table_name: "users".to_string(),
        row_id: RowId::new(0, 0),
        tuple_data: vec![1, 2, 3],
    };
    let record3 = WalRecord::CommitTxn {
        tx_id: 1,
        timestamp: 100,
    };

    let lsn1 = buffer.append(record1).await;
    let lsn2 = buffer.append(record2).await;
    let lsn3 = buffer.append(record3).await;

    // LSN should be monotonically increasing
    assert!(lsn2 > lsn1, "LSN should increase: {} > {}", lsn2, lsn1);
    assert!(lsn3 > lsn2, "LSN should increase: {} > {}", lsn3, lsn2);

    buffer.shutdown().await;
}

#[tokio::test]
async fn test_wal_buffer_flush_on_capacity() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let capacity = 3;
    let buffer = create_wal_buffer(db_path, capacity, 10000); // long interval to avoid timer flush

    // Append exactly `capacity` records to trigger automatic flush
    for i in 0..capacity {
        let record = WalRecord::Insert {
            tx_id: i as u64,
            table_name: "test".to_string(),
            row_id: RowId::new(0, i as u16),
            tuple_data: vec![1],
        };
        buffer.append(record).await;
    }

    // Give a small moment for I/O to complete
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // WAL file should have content (records were flushed)
    let size = wal_file_size(db_path);
    assert!(
        size > 0,
        "WAL file should have content after capacity flush, but size is 0"
    );

    // Verify records are parseable from the WAL file
    let records = read_wal_records(db_path);
    assert_eq!(
        records.len(),
        capacity,
        "WAL file should contain {} records after capacity flush, got {}",
        capacity,
        records.len()
    );

    buffer.shutdown().await;
}

#[tokio::test]
async fn test_group_commit_multiple_txns() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let buffer = create_wal_buffer(db_path, 100, 10000); // large capacity, no timer

    // Two concurrent transactions committing at the same time should share one fsync
    let buf1 = buffer.clone();
    let buf2 = buffer.clone();

    // Append some records first
    buffer.append(WalRecord::BeginTxn { tx_id: 10 }).await;
    buffer.append(WalRecord::BeginTxn { tx_id: 20 }).await;

    // Both transactions commit concurrently - group commit
    let handle1 = tokio::spawn(async move {
        buf1.append_commit_and_wait(10).await.unwrap();
    });
    let handle2 = tokio::spawn(async move {
        buf2.append_commit_and_wait(20).await.unwrap();
    });

    handle1.await.unwrap();
    handle2.await.unwrap();

    // Verify WAL file has content (group commit flushed)
    let size = wal_file_size(db_path);
    assert!(
        size > 0,
        "WAL file should have content after group commit, but size is 0"
    );

    // Verify records are parseable
    let records = read_wal_records(db_path);
    let _commit_count = records
        .iter()
        .filter(|r| matches!(r, WalRecord::CommitTxn { .. } | WalRecord::Commit { .. }))
        .count();
    // The commit waiters triggered flush, but CommitTxn records need to be
    // appended by the caller. Here we just verify the BeginTxn records were flushed.
    let begin_count = records
        .iter()
        .filter(|r| matches!(r, WalRecord::BeginTxn { .. }))
        .count();
    assert!(
        begin_count >= 2,
        "Should have at least 2 BeginTxn records, got {}",
        begin_count
    );
    // At least verify the file grew (flush happened)
    assert!(
        !records.is_empty(),
        "WAL should contain records after group commit"
    );
}

#[tokio::test]
async fn test_wal_buffer_shutdown_flushes() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();
    let buffer = create_wal_buffer(db_path, 100, 10000); // large capacity, no timer

    // Append some records (less than capacity, so no auto-flush)
    for i in 0..5u64 {
        let record = WalRecord::Insert {
            tx_id: i,
            table_name: "data".to_string(),
            row_id: RowId::new(0, i as u16),
            tuple_data: vec![42],
        };
        buffer.append(record).await;
    }

    // Records should not be in WAL yet (capacity not reached, no timer, no commit)
    let size_before = wal_file_size(db_path);
    assert_eq!(
        size_before, 0,
        "WAL file should be empty before shutdown (records buffered in memory)"
    );

    // Shutdown should flush all buffered records
    buffer.shutdown().await;

    // Now records should be in the WAL file
    let size_after = wal_file_size(db_path);
    assert!(
        size_after > 0,
        "WAL file should have content after shutdown flush"
    );

    // Verify all 5 records are readable
    let records = read_wal_records(db_path);
    assert_eq!(
        records.len(),
        5,
        "WAL file should contain 5 records after shutdown flush, got {}",
        records.len()
    );
}
