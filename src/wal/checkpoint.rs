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
        if !self.checkpoint_path.exists() {
            return Ok(None);
        }

        let mut file = File::open(&self.checkpoint_path)
            .map_err(|e| WalError::IoError(e.to_string()))?;

        let mut buf = [0u8; 16];
        let bytes_read = file.read(&mut buf).map_err(|e| WalError::IoError(e.to_string()))?;

        if bytes_read < 16 {
            return Ok(None); // 部分位点，视为无效
        }

        let lsn = u64::from_le_bytes(buf[..8].try_into().unwrap());
        let timestamp = u64::from_le_bytes(buf[8..].try_into().unwrap());

        Ok(Some((lsn, timestamp)))
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

        file.write_all(&buf).map_err(|e| WalError::IoError(e.to_string()))?;

        file.sync_all().map_err(|e| WalError::IoError(e.to_string()))?;

        Ok(())
    }

    /// 执行 checkpoint（刷脏页 + 写位点 + 截断 WAL）
    pub async fn checkpoint(&self) -> Result<u64, WalError> {
        // 1. 获取当前 WAL LSN
        let lsn = self.wal_writer.get_current_lsn().await?;

        // 2. 刷所有脏页
        self.buffer_pool
            .flush_all()
            .await
            .map_err(|e| WalError::IoError(e.to_string()))?;

        // 3. 获取时间戳
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| WalError::IoError(e.to_string()))?
            .as_secs();

        // 4. 写 checkpoint 位点
        self.write_checkpoint_site(lsn, timestamp)?;

        // 5. 写 checkpoint WAL 记录（可选，记录 checkpoint 事件）
        let record = WalRecord::Checkpoint { lsn, timestamp };
        self.wal_writer.write_record(record).await?;

        // 6. 重置写入计数
        self.wal_writer.reset_write_count();

        Ok(lsn)
    }
}
