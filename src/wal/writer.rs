//! WAL 写入器
//!
//! 负责将 WAL 记录持久化到磁盘

use crate::wal::WalError;
use std::path::PathBuf;

/// WAL 写入器
pub struct WalWriter {
    /// WAL 文件路径
    path: PathBuf,
}

impl WalWriter {
    /// 创建新的 WAL 写入器
    pub fn new(path: PathBuf) -> Result<Self, WalError> {
        Ok(Self { path })
    }
}
