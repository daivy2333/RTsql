use rtsql::storage::{BufferPool, FileStorage};
use rtsql::wal::{CheckpointManager, WalRecord, WalWriter};
use std::sync::Arc;
use tempfile::NamedTempFile;

#[test]
fn test_read_write_checkpoint_site() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    let wal_writer = Arc::new(WalWriter::open(db_path).unwrap());
    let storage = FileStorage::open(db_path).unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, Arc::new(storage)).unwrap());

    let manager = CheckpointManager::new(db_path, wal_writer, buffer_pool);

    // 无位点文件
    let result = manager.read_checkpoint_site().unwrap();
    assert!(result.is_none());

    // 写位点
    manager.write_checkpoint_site(1024, 1234567890).unwrap();

    // 读位点
    let (lsn, timestamp) = manager.read_checkpoint_site().unwrap().unwrap();
    assert_eq!(lsn, 1024);
    assert_eq!(timestamp, 1234567890);
}

#[tokio::test]
async fn test_checkpoint_flow() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    let wal_writer = Arc::new(WalWriter::open(db_path).unwrap());
    let storage = FileStorage::open(db_path).unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, Arc::new(storage)).unwrap());

    // 写一些 WAL 记录
    for i in 0..5 {
        wal_writer
            .write_record(WalRecord::Commit {
                tx_id: i,
                timestamp: i,
            })
            .await
            .unwrap();
    }

    let manager = CheckpointManager::new(db_path, wal_writer.clone(), buffer_pool);

    // 执行 checkpoint
    let lsn = manager.checkpoint().await.unwrap();

    // 读位点验证
    let (checkpoint_lsn, _) = manager.read_checkpoint_site().unwrap().unwrap();
    assert!(checkpoint_lsn > 0);
}

#[tokio::test]
async fn test_checkpoint_threshold_trigger() {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path();

    let mut writer = WalWriter::open(db_path).unwrap();
    writer.set_checkpoint_threshold(3);
    assert_eq!(writer.get_checkpoint_threshold(), 3);

    let wal_writer = Arc::new(writer);
    let storage = FileStorage::open(db_path).unwrap();
    let buffer_pool = Arc::new(BufferPool::new(10, Arc::new(storage)).unwrap());
    let manager = CheckpointManager::new(db_path, wal_writer.clone(), buffer_pool);

    // 写 3 条记录后应触发 checkpoint
    for i in 0..3 {
        wal_writer
            .write_record(WalRecord::Commit {
                tx_id: i,
                timestamp: i,
            })
            .await
            .unwrap();

        if wal_writer.should_checkpoint() {
            manager.checkpoint().await.unwrap();
            wal_writer.reset_write_count();
        }
    }

    // 验证 checkpoint 位点已写入
    let (lsn, _) = manager.read_checkpoint_site().unwrap().unwrap();
    assert!(lsn > 0);
}