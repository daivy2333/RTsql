//! WalWriter 测试
//!
//! 测试 WAL 写入器的追加写入、fsync 和截断功能

use rtsql::storage::RowId;
use rtsql::wal::{WalRecord, WalWriter};
use std::io::Read;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_write_insert_record() {
    // 创建临时文件
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    // 打开 WAL 写入器
    let writer = WalWriter::open(db_path).unwrap();

    // 创建插入记录
    let record = WalRecord::Insert {
        tx_id: 1,
        table_name: "users".to_string(),
        row_id: RowId::new(0, 0),
        tuple_data: vec![1, 2, 3, 4],
    };

    // 写入记录
    let lsn = writer.write_record(record).await.unwrap();
    assert_eq!(lsn, 0, "First record should have LSN 0");

    // 验证写入计数
    assert_eq!(writer.get_write_count(), 1);

    // 读取文件验证内容
    let wal_path = db_path.with_extension("wal");
    let mut file = std::fs::File::open(wal_path).unwrap();
    let mut contents = Vec::new();
    file.read_to_end(&mut contents).unwrap();
    assert!(!contents.is_empty(), "WAL file should not be empty");
}

#[tokio::test]
async fn test_write_multiple_records() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    let writer = WalWriter::open(db_path).unwrap();

    // 写入多条记录
    let record1 = WalRecord::Insert {
        tx_id: 1,
        table_name: "users".to_string(),
        row_id: RowId::new(0, 0),
        tuple_data: vec![1, 2, 3],
    };

    let record2 = WalRecord::Commit {
        tx_id: 1,
        timestamp: 100,
    };

    let record3 = WalRecord::Delete {
        tx_id: 2,
        table_name: "orders".to_string(),
        row_id: RowId::new(1, 5),
    };

    let lsn1 = writer.write_record(record1).await.unwrap();
    let lsn2 = writer.write_record(record2).await.unwrap();
    let lsn3 = writer.write_record(record3).await.unwrap();

    // 验证 LSN 递增
    assert!(lsn2 > lsn1, "LSN should increase");
    assert!(lsn3 > lsn2, "LSN should increase");

    // 验证写入计数
    assert_eq!(writer.get_write_count(), 3);

    // 验证文件大小增长
    let current_lsn = writer.get_current_lsn().await.unwrap();
    assert!(current_lsn > 0, "Current LSN should be positive");
}

#[tokio::test]
async fn test_fsync_after_write() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    let writer = WalWriter::open(db_path).unwrap();

    // 写入记录
    let record = WalRecord::Commit {
        tx_id: 1,
        timestamp: 12345,
    };
    writer.write_record(record).await.unwrap();

    // 执行 fsync
    let result = writer.fsync().await;
    assert!(result.is_ok(), "fsync should succeed");

    // 验证文件已持久化
    let wal_path = db_path.with_extension("wal");
    let metadata = std::fs::metadata(wal_path).unwrap();
    assert!(metadata.len() > 0, "WAL file should have content");
}

#[tokio::test]
async fn test_truncate_wal() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    let writer = WalWriter::open(db_path).unwrap();

    // 写入多条记录
    let record1 = WalRecord::Insert {
        tx_id: 1,
        table_name: "test".to_string(),
        row_id: RowId::new(0, 0),
        tuple_data: vec![1, 2, 3, 4, 5],
    };

    let record2 = WalRecord::Commit {
        tx_id: 1,
        timestamp: 100,
    };

    let _lsn1 = writer.write_record(record1).await.unwrap();
    let lsn2 = writer.write_record(record2).await.unwrap();

    // 验证文件大小
    let size_before = writer.get_current_lsn().await.unwrap();
    assert!(
        size_before > lsn2,
        "File size should be larger than last LSN"
    );

    // 截断到第一个记录的 LSN
    writer.truncate_to(lsn2).await.unwrap();

    // 验证文件被截断
    let size_after = writer.get_current_lsn().await.unwrap();
    assert_eq!(size_after, lsn2, "File should be truncated to LSN");
}

#[tokio::test]
async fn test_checkpoint_threshold() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    let mut writer = WalWriter::open(db_path).unwrap();

    // 验证默认阈值
    assert_eq!(writer.get_checkpoint_threshold(), 1000);

    // 设置新阈值
    writer.set_checkpoint_threshold(500);
    assert_eq!(writer.get_checkpoint_threshold(), 500);

    // 写入记录验证计数器
    let record = WalRecord::Abort { tx_id: 42 };
    writer.write_record(record).await.unwrap();
    assert_eq!(writer.get_write_count(), 1);
}
