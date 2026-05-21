//! 恢复管理器
//!
//! 负责启动时重放 WAL，恢复未完成事务

use super::{WalError, WalReader, WalRecord};
use std::collections::HashSet;
use std::path::Path;

/// 恢复管理器
pub struct RecoveryManager;

impl RecoveryManager {
    /// 执行崩溃恢复
    ///
    /// 返回 (已提交事务ID集合, 已回滚事务ID集合)
    pub fn recover(db_path: &Path) -> Result<(HashSet<u64>, HashSet<u64>), WalError> {
        let wal_path = db_path.with_extension("wal");

        // WAL 文件不存在，无需恢复
        if !wal_path.exists() {
            return Ok((HashSet::new(), HashSet::new()));
        }

        let mut reader = WalReader::open(&wal_path)?;
        let records = reader.read_all()?;

        let mut committed_tx_ids = HashSet::new();
        let mut aborted_tx_ids = HashSet::new();

        for record in records {
            match record {
                WalRecord::Commit { tx_id, .. } => {
                    committed_tx_ids.insert(tx_id);
                }
                WalRecord::Abort { tx_id } => {
                    aborted_tx_ids.insert(tx_id);
                }
                // 其他记录类型在基础恢复中不需要处理
                _ => {}
            }
        }

        Ok((committed_tx_ids, aborted_tx_ids))
    }

    /// 检查是否需要恢复（WAL 文件是否存在且非空）
    pub fn needs_recovery(db_path: &Path) -> Result<bool, WalError> {
        let wal_path = db_path.with_extension("wal");

        if !wal_path.exists() {
            return Ok(false);
        }

        let metadata =
            std::fs::metadata(&wal_path).map_err(|e| WalError::IoError(e.to_string()))?;

        Ok(metadata.len() > 0)
    }

    /// 重放 WAL 记录（用于更完整的恢复）
    ///
    /// 返回所有 WAL 记录
    pub fn read_wal(db_path: &Path) -> Result<Vec<WalRecord>, WalError> {
        let wal_path = db_path.with_extension("wal");

        if !wal_path.exists() {
            return Ok(Vec::new());
        }

        let mut reader = WalReader::open(&wal_path)?;
        reader.read_all()
    }
}
