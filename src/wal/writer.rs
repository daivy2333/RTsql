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

/// WAL 字节水位的默认阈值（MS26 T5，`wal-auto-checkpoint` R2）——16 MiB。
///
/// 提交边界按「自上次 checkpoint 以来的累计追加字节」判定是否触发，故该值
/// 远大于单批写入量即可覆盖常规短会话；长会话按需下调（见
/// [`WalWriter::set_checkpoint_bytes_threshold`]）。`0` 表示禁用自动触发。
pub const DEFAULT_CHECKPOINT_BYTES: u64 = 16 * 1024 * 1024;

/// WAL 写入器（追加写入 + fsync）
///
/// 持有单一持久文件句柄，全部 IO 操作经 `Arc<Mutex<File>>` 串行完成
pub struct WalWriter {
    // 路径仅用于重写截断时打开非 append 覆写句柄
    wal_path: PathBuf,
    file: Arc<Mutex<File>>,
    write_count: AtomicU64,
    /// MS26 T5：自上次 checkpoint 以来的累计追加字节（内存计数器，追加时
    /// 累加、checkpoint 步骤 9 归零）——`wal-auto-checkpoint` R1 要求不得
    /// 为水位判定引入每语句一次文件 `metadata()` 调用。
    bytes_appended: AtomicU64,
    /// MS26 T5：字节水位阈值；`0` 禁用自动 checkpoint。
    checkpoint_bytes_threshold: AtomicU64,
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
            bytes_appended: AtomicU64::new(0),
            checkpoint_bytes_threshold: AtomicU64::new(DEFAULT_CHECKPOINT_BYTES),
        })
    }

    /// 写入 WAL 记录（异步包装）
    pub async fn write_record(&self, record: WalRecord) -> Result<u64, WalError> {
        let buf = record.serialize();
        let frame_len = buf.len() as u64;
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
        self.bytes_appended.fetch_add(frame_len, Ordering::SeqCst);
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

    /// MS26 T5（`wal-auto-checkpoint` R2）：字节水位阈值；`0` 禁用自动触发。
    pub fn checkpoint_bytes_threshold(&self) -> u64 {
        self.checkpoint_bytes_threshold.load(Ordering::SeqCst)
    }

    /// 设置字节水位阈值（`0` 禁用自动 checkpoint）。`&self` 即可调用——
    /// 阈值经 `Arc<WalWriter>` 共享，库内与 CLI 均在打开后下发。
    pub fn set_checkpoint_bytes_threshold(&self, bytes: u64) {
        self.checkpoint_bytes_threshold
            .store(bytes, Ordering::SeqCst);
    }

    /// 自上次 checkpoint 以来的累计追加字节（MS26 T5）。
    pub fn bytes_appended(&self) -> u64 {
        self.bytes_appended.load(Ordering::SeqCst)
    }

    /// 重置字节水位计数（与 `reset_write_count` 同位由 checkpoint 步骤 9 调用）。
    pub fn reset_bytes_appended(&self) {
        self.bytes_appended.store(0, Ordering::SeqCst);
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

        let appended = spawn_blocking(move || {
            let mut file = file.lock().unwrap();
            let mut appended = 0u64;

            for (lsn, record) in &records {
                let buf = record.serialize_with_lsn(*lsn);
                file.write_all(&buf)
                    .map_err(|e| WalError::IoError(e.to_string()))?;
                appended += buf.len() as u64;
            }

            file.sync_all()
                .map_err(|e| WalError::IoError(e.to_string()))?;

            Ok(appended)
        })
        .await
        .map_err(|e| WalError::IoError(e.to_string()))??;

        self.write_count.fetch_add(count, Ordering::SeqCst);
        // 批内求和后一次原子累加（避免逐帧原子操作）；口径与 `write_record`
        // 同源——该批各帧序列化长度之和。
        self.bytes_appended.fetch_add(appended, Ordering::SeqCst);
        Ok(())
    }
}
