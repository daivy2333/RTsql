//! Checkpoint 管理器
//!
//! 负责定期创建检查点，截断 WAL

use super::{WalError, WalRecord, WalWriter};
use crate::storage::BufferPool;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Checkpoint 管理器（位点读写 + 刷脏页 + 截断 WAL）
pub struct CheckpointManager {
    checkpoint_path: PathBuf,
    wal_writer: Arc<WalWriter>,
    buffer_pool: Arc<BufferPool>,
}

/// 读取位点文件（16B 语义：缺失 / 不足 16B → None）
///
/// 自由函数供恢复端按 db_path 消费位点，与 `CheckpointManager` 共享同一语义
pub(crate) fn read_site_file(path: &std::path::Path) -> Result<Option<(u64, u64)>, WalError> {
    if !path.exists() {
        return Ok(None);
    }

    let mut file = File::open(path).map_err(|e| WalError::IoError(e.to_string()))?;

    let mut buf = [0u8; 16];
    let bytes_read = file
        .read(&mut buf)
        .map_err(|e| WalError::IoError(e.to_string()))?;

    if bytes_read < 16 {
        return Ok(None); // 部分位点，视为无效
    }

    let lsn = u64::from_le_bytes(buf[..8].try_into().unwrap());
    let timestamp = u64::from_le_bytes(buf[8..].try_into().unwrap());

    Ok(Some((lsn, timestamp)))
}

impl CheckpointManager {
    /// 创建 CheckpointManager
    pub fn new(
        db_path: &std::path::Path,
        wal_writer: Arc<WalWriter>,
        buffer_pool: Arc<BufferPool>,
    ) -> Self {
        let checkpoint_path = db_path.with_extension("checkpoint");
        Self {
            checkpoint_path,
            wal_writer,
            buffer_pool,
        }
    }

    /// 读取 checkpoint 位点
    /// 返回 (lsn, timestamp) 或 None（无位点文件）
    pub fn read_checkpoint_site(&self) -> Result<Option<(u64, u64)>, WalError> {
        read_site_file(&self.checkpoint_path)
    }

    /// 写入 checkpoint 位点
    pub fn write_checkpoint_site(&self, lsn: u64, timestamp: u64) -> Result<(), WalError> {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.checkpoint_path)
            .map_err(|e| WalError::IoError(e.to_string()))?;

        let mut buf = [0u8; 16];
        buf[..8].copy_from_slice(&lsn.to_le_bytes());
        buf[8..].copy_from_slice(&timestamp.to_le_bytes());

        file.write_all(&buf)
            .map_err(|e| WalError::IoError(e.to_string()))?;

        file.sync_all()
            .map_err(|e| WalError::IoError(e.to_string()))?;

        Ok(())
    }

    /// 执行 checkpoint（刷脏页 → 写位点 → 重写截断 WAL）
    ///
    /// 崩溃窗口次序保证：
    /// - 位点先于截断写入：截断前崩溃按 `≥ lsn` 过滤重放，无丢无重；
    /// - 截断后位点置 0：此后恢复对已缩短文件全量重放；
    /// - 截断中崩溃留下部分/旧尾部字节 → 解析错误 → 恢复显式失败（不静默丢）。
    pub async fn checkpoint(&self) -> Result<u64, WalError> {
        // 1. 获取当前 WAL LSN（文件字节偏移）
        let lsn = self.wal_writer.get_current_lsn().await?;

        // 2. 刷所有脏页：位点前缀的页效果全部落盘
        self.buffer_pool
            .flush_all()
            .await
            .map_err(|e| WalError::IoError(e.to_string()))?;

        // 3. fsync WAL：位点之前的记录持久
        self.wal_writer.fsync().await?;

        // 4. 获取时间戳
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| WalError::IoError(e.to_string()))?
            .as_secs();

        // 5. 先写有效位点（语义 = 重放 ≥ lsn）
        self.write_checkpoint_site(lsn, timestamp)?;

        // 6. 追加 checkpoint WAL 记录
        let record = WalRecord::Checkpoint { lsn, timestamp };
        self.wal_writer.write_record(record).await?;

        // 7. 重写截断：保留 [lsn..end)（含本 Checkpoint 记录），WAL 物理缩短
        self.wal_writer.rewrite_truncate(lsn).await?;

        // 8. 截断后位点置 0（语义 = 重放全部；新写入从文件头重新按偏移编 LSN）
        let timestamp2 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| WalError::IoError(e.to_string()))?
            .as_secs();
        self.write_checkpoint_site(0, timestamp2)?;

        // 9. 重置写入计数
        self.wal_writer.reset_write_count();

        Ok(lsn)
    }
}
