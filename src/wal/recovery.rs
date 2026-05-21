//! 恢复管理器
//!
//! 负责启动时重放 WAL，恢复未完成事务

use crate::wal::WalError;

/// 恢复管理器
pub struct RecoveryManager {
    /// WAL 目录
    wal_dir: std::path::PathBuf,
}

impl RecoveryManager {
    /// 创建新的恢复管理器
    pub fn new(wal_dir: std::path::PathBuf) -> Result<Self, WalError> {
        Ok(Self { wal_dir })
    }
}
