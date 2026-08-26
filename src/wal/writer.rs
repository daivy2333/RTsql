//! WAL 写入器
//!
//! 负责将 WAL 记录持久化到磁盘

use super::record::{WalError, WalRecord};
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::task::spawn_blocking;

/// WAL 写入器（追加写入 + fsync）
///
/// 持有单一持久文件句柄，全部 IO 操作经 `Arc<Mutex<File>>` 串行完成
pub struct WalWriter {
    // 保留路径仅用于诊断；IO 一律经持久句柄完成
    #[allow(dead_code)]
    wal_path: PathBuf,
    file: Arc<Mutex<File>>,
    write_count: AtomicU64,
    checkpoint_threshold: u64,
}

impl WalWriter {
    /// 打开或创建 WAL 文件
    pub fn open(db_path: &std::path::Path) -> Result<Self, WalError> {
        let wal_path = db_path.with_extension("wal");

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&wal_path)
            .map_err(|e| WalError::IoError(e.to_string()))?;

        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            wal_path,
            write_count: AtomicU64::new(0),
            checkpoint_threshold: 1000,
        })
    }

    /// 写入 WAL 记录（异步包装）
    pub async fn write_record(&self, record: WalRecord) -> Result<u64, WalError> {
        let buf = record.serialize();
        let file = Arc::clone(&self.file);

        let lsn = spawn_blocking(move || {
            let mut file = file.lock().unwrap();

            // Seek to end to get correct LSN
            file.seek(SeekFrom::End(0))
                .map_err(|e| WalError::IoError(e.to_string()))?;

            let lsn = file
                .stream_position()
                .map_err(|e| WalError::IoError(e.to_string()))?;

            file.write_all(&buf)
                .map_err(|e| WalError::IoError(e.to_string()))?;

            Ok(lsn)
        })
        .await
        .map_err(|e| WalError::IoError(e.to_string()))??;

        self.write_count.fetch_add(1, Ordering::SeqCst);
        Ok(lsn)
    }

    /// fsync WAL 文件
    pub async fn fsync(&self) -> Result<(), WalError> {
        let file = Arc::clone(&self.file);

        spawn_blocking(move || {
            let file = file.lock().unwrap();

            file.sync_all()
                .map_err(|e| WalError::IoError(e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| WalError::IoError(e.to_string()))?
    }

    /// 截断 WAL 文件到指定 LSN
    pub async fn truncate_to(&self, lsn: u64) -> Result<(), WalError> {
        let file = Arc::clone(&self.file);

        spawn_blocking(move || {
            let file = file.lock().unwrap();

            file.set_len(lsn)
                .map_err(|e| WalError::IoError(e.to_string()))?;

            Ok(())
        })
        .await
        .map_err(|e| WalError::IoError(e.to_string()))?
    }

    pub fn get_write_count(&self) -> u64 {
        self.write_count.load(Ordering::SeqCst)
    }

    pub fn get_checkpoint_threshold(&self) -> u64 {
        self.checkpoint_threshold
    }

    pub fn set_checkpoint_threshold(&mut self, threshold: u64) {
        self.checkpoint_threshold = threshold;
    }

    /// 检查是否应该触发 checkpoint
    pub fn should_checkpoint(&self) -> bool {
        self.write_count.load(Ordering::SeqCst) >= self.checkpoint_threshold
    }

    /// 重置写入计数（checkpoint 后调用）
    pub fn reset_write_count(&self) {
        self.write_count.store(0, Ordering::SeqCst);
    }

    pub async fn get_current_lsn(&self) -> Result<u64, WalError> {
        let file = Arc::clone(&self.file);

        spawn_blocking(move || {
            let file = file.lock().unwrap();

            let len = file
                .metadata()
                .map_err(|e| WalError::IoError(e.to_string()))?
                .len();

            Ok(len)
        })
        .await
        .map_err(|e| WalError::IoError(e.to_string()))?
    }

    /// 批量写入 WAL 记录（带 LSN + CRC32）
    ///
    /// 遍历 records，对每条调用 serialize_with_lsn(lsn) → write_all
    /// 最后一次性 fsync
    pub async fn write_batch(&self, records: Vec<(u64, WalRecord)>) -> Result<(), WalError> {
        let file = Arc::clone(&self.file);
        let count = records.len() as u64;

        spawn_blocking(move || {
            let mut file = file.lock().unwrap();

            for (lsn, record) in &records {
                let buf = record.serialize_with_lsn(*lsn);
                file.write_all(&buf)
                    .map_err(|e| WalError::IoError(e.to_string()))?;
            }

            file.sync_all()
                .map_err(|e| WalError::IoError(e.to_string()))?;

            Ok(())
        })
        .await
        .map_err(|e| WalError::IoError(e.to_string()))??;

        self.write_count.fetch_add(count, Ordering::SeqCst);
        Ok(())
    }
}
