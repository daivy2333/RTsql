//! WAL 写入器
//!
//! 负责将 WAL 记录持久化到磁盘

use super::record::{WalError, WalRecord};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::task::spawn_blocking;

/// WAL 写入器（追加写入 + fsync）
///
/// 持有单一持久文件句柄，全部 IO 操作经 `Arc<Mutex<File>>` 串行完成
pub struct WalWriter {
    // 路径仅用于重写截断时打开非 append 覆写句柄
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

    /// 重写截断：保留 `[lsn..end)` 后缀并移到文件偏移 0，WAL 物理缩短
    ///
    /// 用于 checkpoint：位点之前的记录已由刷脏页覆盖，可物理丢弃；
    /// 后缀按原字节保留（embedded LSN 语义不变）。
    /// 全程持有 writer 文件互斥（单次临界区），防止与并发追加交错。
    /// 覆写头部必须用独立非 append 句柄——持有句柄以 O_APPEND 打开，
    /// write 恒定落在文件末尾，无法覆写头部；也不得 temp+rename（rename 后
    /// 持有 FD 指向旧 inode，后续 WAL 写全部丢失）。
    pub async fn rewrite_truncate(&self, lsn: u64) -> Result<u64, WalError> {
        let file = Arc::clone(&self.file);
        let wal_path = self.wal_path.clone();

        spawn_blocking(move || {
            let mut file = file.lock().unwrap();

            let end = file
                .seek(SeekFrom::End(0))
                .map_err(|e| WalError::IoError(e.to_string()))?;
            let suffix_len = end.saturating_sub(lsn);

            let mut suffix = vec![0u8; suffix_len as usize];
            file.seek(SeekFrom::Start(lsn))
                .map_err(|e| WalError::IoError(e.to_string()))?;
            file.read_exact(&mut suffix)
                .map_err(|e| WalError::IoError(e.to_string()))?;

            let mut rewrite = OpenOptions::new()
                .write(true)
                .open(&wal_path)
                .map_err(|e| WalError::IoError(e.to_string()))?;
            rewrite
                .seek(SeekFrom::Start(0))
                .map_err(|e| WalError::IoError(e.to_string()))?;
            rewrite
                .write_all(&suffix)
                .map_err(|e| WalError::IoError(e.to_string()))?;
            rewrite
                .set_len(suffix_len)
                .map_err(|e| WalError::IoError(e.to_string()))?;
            rewrite
                .sync_all()
                .map_err(|e| WalError::IoError(e.to_string()))?;

            Ok(suffix_len)
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
