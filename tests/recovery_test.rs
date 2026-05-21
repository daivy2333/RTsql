use rtsql::storage::{BufferPool, FileStorage};
use rtsql::wal::{CheckpointManager, RecoveryManager, WalRecord, WalWriter};
use std::sync::Arc;
use tempfile::NamedTempFile;

#[test]
fn test_recover_from_empty_wal() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    // 无 WAL 文件
    let result = RecoveryManager::recover(db_path).unwrap();
    let (committed, aborted) = result;

    assert!(committed.is_empty());
    assert!(aborted.is_empty());
}

#[tokio::test]
async fn test_recover_commit_abort_marks() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    let wal_writer = WalWriter::open(db_path).unwrap();

    // 写入事务操作 + commit/abort 标记
    wal_writer
        .write_record(WalRecord::Commit {
            tx_id: 100,
            timestamp: 1000,
        })
        .await
        .unwrap();
    wal_writer
        .write_record(WalRecord::Abort { tx_id: 200 })
        .await
        .unwrap();
    wal_writer
        .write_record(WalRecord::Commit {
            tx_id: 300,
            timestamp: 3000,
        })
        .await
        .unwrap();
    wal_writer.fsync().await.unwrap();

    // 恢复
    let (committed, aborted) = RecoveryManager::recover(db_path).unwrap();

    assert_eq!(committed.len(), 2);
    assert!(committed.contains(&100));
    assert!(committed.contains(&300));

    assert_eq!(aborted.len(), 1);
    assert!(aborted.contains(&200));
}

#[tokio::test]
async fn test_recover_from_checkpoint() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    let wal_writer = Arc::new(WalWriter::open(db_path).unwrap());
    let storage = FileStorage::open(db_path).unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, Arc::new(storage)).unwrap());

    // 写入 checkpoint 前记录
    for i in 0..3 {
        wal_writer
            .write_record(WalRecord::Commit {
                tx_id: i,
                timestamp: i,
            })
            .await
            .unwrap();
    }

    let manager = CheckpointManager::new(db_path, wal_writer.clone(), buffer_pool);
    manager.checkpoint().await.unwrap();

    // 写入 checkpoint 后记录
    wal_writer
        .write_record(WalRecord::Commit {
            tx_id: 100,
            timestamp: 1000,
        })
        .await
        .unwrap();
    wal_writer.fsync().await.unwrap();

    // 恢复（仅重放 checkpoint 后）
    let (committed, _) = RecoveryManager::recover(db_path).unwrap();

    // 应包含 checkpoint 后的 tx_id=100 和 checkpoint 本身
    assert!(committed.contains(&100));
}