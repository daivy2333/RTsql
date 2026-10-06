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

    let manager = CheckpointManager::new(db_path, wal_writer, buffer_pool, false);

    // 无位点文件
    let result = manager.read_checkpoint_site().unwrap();
    assert!(result.is_none());

    // 写位点
    manager.write_checkpoint_site(1024, 1234567890, 0).unwrap();

    // 读位点
    let site = manager.read_checkpoint_site().unwrap().unwrap();
    assert_eq!(site.lsn, 1024);
    assert_eq!(site.timestamp, 1234567890);
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

    let manager = CheckpointManager::new(db_path, wal_writer.clone(), buffer_pool, false);

    // 执行 checkpoint
    let captured_lsn = manager.checkpoint(|| 0).await.unwrap();
    assert!(captured_lsn > 0, "checkpoint 返回本次捕获的 LSN");

    // 读位点验证
    // MS07-T05：重写截断后位点置 0（语义 = 对已缩短文件重放全部），
    // 既有 `checkpoint_lsn > 0` 断言与 Critical Path 冲突，按新语义同步
    let site = manager.read_checkpoint_site().unwrap().unwrap();
    assert_eq!(site.lsn, 0, "截断后位点必须失效为重放全部");
}

// MS26 T5：本文件原有的 `test_checkpoint_threshold_trigger`（手写
// `should_checkpoint` 判定循环演示记录数阈值触发）随该接口一并删除——
// `wal-auto-checkpoint` R5 Requirement 强制移除记录数阈值接口
// （`should_checkpoint` / `set_checkpoint_threshold` / `get_checkpoint_threshold`
// / `checkpoint_threshold` 字段），用例的被测对象整体消失。它顺带覆盖的
// `CheckpointManager::checkpoint` + `reset_write_count` + 位点截断语义由上方
// `test_checkpoint_flow` 保持覆盖；字节水位触发通道的见证改由
// `tests/wal_auto_checkpoint_test.rs` 承担。
