//! WAL 读取器
//!
//! 负责从磁盘读取 WAL 记录

use crate::wal::WalError;
use std::path::PathBuf;

/// WAL 读取器
pub struct WalReader {
    /// WAL 文件路径
    path: PathBuf,
}

impl WalReader {
    /// 创建新的 WAL 读取器
    pub fn new(path: PathBuf) -> Result<Self, WalError> {
        Ok(Self { path })
    }
}
