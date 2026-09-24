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

/// Checkpoint 位点（MS17-T02 Iter002，design D8：24B = lsn + timestamp + tx watermark）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointSite {
    pub lsn: u64,
    pub timestamp: u64,
    /// 位点写入时（LSN 捕获之后）读取的事务分配器水位；16B 旧格式位点为 `None`
    pub tx_watermark: Option<u64>,
}

/// 读取位点文件（兼容读：≥24B 携带水位 / 16B..24B 旧格式无水位 / <16B → None）
///
/// 自由函数供恢复端按 db_path 消费位点，与 `CheckpointManager` 共享同一语义
pub(crate) fn read_site_file(path: &std::path::Path) -> Result<Option<CheckpointSite>, WalError> {
    if !path.exists() {
        return Ok(None);
    }

    let mut file = File::open(path).map_err(|e| WalError::IoError(e.to_string()))?;

    let mut buf = [0u8; 24];
    let bytes_read = file
        .read(&mut buf)
        .map_err(|e| WalError::IoError(e.to_string()))?;

    if bytes_read < 16 {
        return Ok(None); // 部分位点，视为无效
    }

    let lsn = u64::from_le_bytes(buf[..8].try_into().unwrap());
    let timestamp = u64::from_le_bytes(buf[8..16].try_into().unwrap());
    let tx_watermark = if bytes_read >= 24 {
        Some(u64::from_le_bytes(buf[16..24].try_into().unwrap()))
    } else {
        None // 16B 旧格式位点：无水位字段
    };

    Ok(Some(CheckpointSite {
        lsn,
        timestamp,
        tx_watermark,
    }))
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
    /// 返回位点或 None（无位点文件）
    pub fn read_checkpoint_site(&self) -> Result<Option<CheckpointSite>, WalError> {
        read_site_file(&self.checkpoint_path)
    }

    /// 写入 checkpoint 位点（24B：lsn + timestamp + tx watermark）
    ///
    /// MS17-T02 Iter002（design D8）：水位 MUST 在 LSN 捕获之后读取
    /// （见 [`CheckpointManager::checkpoint`]），保证位点前缀内一切
    /// Begin 的 tx id ≤ 水位，重开侧 max(WAL 观测, 水位) 不漏观测。
    pub fn write_checkpoint_site(
        &self,
        lsn: u64,
        timestamp: u64,
        tx_watermark: u64,
    ) -> Result<(), WalError> {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.checkpoint_path)
            .map_err(|e| WalError::IoError(e.to_string()))?;

        let mut buf = [0u8; 24];
        buf[..8].copy_from_slice(&lsn.to_le_bytes());
        buf[8..16].copy_from_slice(&timestamp.to_le_bytes());
        buf[16..].copy_from_slice(&tx_watermark.to_le_bytes());

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
    ///
    /// MS17-T02 Iter002（design D8 健全性关键）：`tx_watermark` 闭包在本
    /// 方法内、步骤 1 LSN 捕获**之后**调用——位点前缀内（offset < lsn）落盘
    /// Begin 的事务，其 id 分配先于 Begin 写入、Begin 写入先于 LSN 捕获，
    /// 故 id ≤ 此处读取的水位；水位之后分配的事务其 Begin 必落
    /// offset ≥ lsn → 重放尾部观测覆盖。任意崩溃点
    /// max(WAL 观测, 位点水位) ≥ 一切已分配历史 id。禁止水位经调用前
    /// 捕获的参数传入（LSN 之前读取会漏观测「读取与 LSN 捕获之间分配且
    /// Begin 落前缀」的 id）。
    pub async fn checkpoint(&self, tx_watermark: impl Fn() -> u64) -> Result<u64, WalError> {
        // 1. 获取当前 WAL LSN（文件字节偏移）
        let lsn = self.wal_writer.get_current_lsn().await?;

        // 1b. LSN 捕获之后读取分配器水位（D8，健全性关键——见方法 doc）
        let watermark = tx_watermark();

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

        // 5. 先写有效位点（语义 = 重放 ≥ lsn），携带分配器水位
        self.write_checkpoint_site(lsn, timestamp, watermark)?;

        // 6. 追加 checkpoint WAL 记录
        let record = WalRecord::Checkpoint { lsn, timestamp };
        self.wal_writer.write_record(record).await?;

        // 7. 重写截断：保留 [lsn..end)（含本 Checkpoint 记录），WAL 物理缩短
        self.wal_writer.rewrite_truncate(lsn).await?;

        // 8. 截断后位点置 0（语义 = 重放全部；新写入从文件头重新按偏移编 LSN），
        //    同一水位随位点落盘——截断后残余 WAL 无 id 历史，水位即恢复唯一来源
        let timestamp2 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| WalError::IoError(e.to_string()))?
            .as_secs();
        self.write_checkpoint_site(0, timestamp2, watermark)?;

        // 9. 重置写入计数
        self.wal_writer.reset_write_count();

        Ok(lsn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{BufferPool, FileStorage};
    use tempfile::tempdir;

    fn manager_for(db_path: &std::path::Path) -> CheckpointManager {
        let wal_writer = Arc::new(WalWriter::open(db_path).unwrap());
        let storage = FileStorage::open(db_path).unwrap();
        let buffer_pool = Arc::new(BufferPool::new(10, Arc::new(storage)).unwrap());
        CheckpointManager::new(db_path, wal_writer, buffer_pool)
    }

    /// transaction-isolation-levels S2（MS17-T02 Iter002）：24B 位点往返——
    /// write 携带水位，read 返回水位且 lsn/timestamp 不变。
    #[test]
    fn test_site_roundtrip_carries_watermark() {
        let dir = tempdir().unwrap();
        let manager = manager_for(&dir.path().join("site.db"));

        manager.write_checkpoint_site(1024, 1234567890, 42).unwrap();
        let site = manager.read_checkpoint_site().unwrap().unwrap();
        assert_eq!(
            site,
            CheckpointSite {
                lsn: 1024,
                timestamp: 1234567890,
                tx_watermark: Some(42),
            }
        );
    }

    /// transaction-isolation-levels S2：16B 旧格式位点按无水位读取
    /// （兼容读，lsn/timestamp 语义不变）。
    #[test]
    fn test_legacy_16b_site_reads_without_watermark() {
        let dir = tempdir().unwrap();
        let path = dir.path().with_extension("checkpoint");

        let mut buf = [0u8; 16];
        buf[..8].copy_from_slice(&777u64.to_le_bytes());
        buf[8..].copy_from_slice(&1234567890u64.to_le_bytes());
        std::fs::write(&path, buf).unwrap();

        let site = read_site_file(&path).unwrap().unwrap();
        assert_eq!(site.lsn, 777);
        assert_eq!(site.timestamp, 1234567890);
        assert_eq!(site.tx_watermark, None);
    }

    /// transaction-isolation-levels S2：不足 16B 位点无效（既有撕裂/短写
    /// 安全退化语义不变）。
    #[test]
    fn test_short_site_is_invalid() {
        let dir = tempdir().unwrap();
        let path = dir.path().with_extension("checkpoint");
        std::fs::write(&path, [0u8; 8]).unwrap();

        assert!(read_site_file(&path).unwrap().is_none());
    }
}
