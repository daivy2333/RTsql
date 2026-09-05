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
    let captured_lsn = manager.checkpoint().await.unwrap();
    assert!(captured_lsn > 0, "checkpoint 返回本次捕获的 LSN");

    // 读位点验证
    // MS07-T05：重写截断后位点置 0（语义 = 对已缩短文件重放全部），
    // 既有 `checkpoint_lsn > 0` 断言与 Critical Path 冲突，按新语义同步
    let (checkpoint_lsn, _) = manager.read_checkpoint_site().unwrap().unwrap();
    assert_eq!(checkpoint_lsn, 0, "截断后位点必须失效为重放全部");
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
    // MS07-T05：重写截断后位点置 0（语义 = 对已缩短文件重放全部）
    let (lsn, _) = manager.read_checkpoint_site().unwrap().unwrap();
    assert_eq!(lsn, 0, "截断后位点必须失效为重放全部");
}
