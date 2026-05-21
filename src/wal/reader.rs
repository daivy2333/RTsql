//! WAL 读取器
//!
//! 负责从磁盘读取 WAL 记录

use super::record::{WalError, WalRecord};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// WAL 读取器
pub struct WalReader {
    file: File,
    wal_path: PathBuf,
}

impl WalReader {
    /// 打开 WAL 文件进行读取
    pub fn open(wal_path: &Path) -> Result<Self, WalError> {
        let file = File::open(wal_path).map_err(|e| WalError::IoError(e.to_string()))?;
        Ok(Self {
            file,
            wal_path: wal_path.to_path_buf(),
        })
    }

    /// 读取下一条 WAL 记录
    ///
    /// 返回 Ok(None) 表示已到达文件末尾
    pub fn read_next(&mut self) -> Result<Option<WalRecord>, WalError> {
        let mut header_buf = [0u8; 5];
        let bytes_read = self
            .file
            .read(&mut header_buf)
            .map_err(|e| WalError::IoError(e.to_string()))?;

        if bytes_read < 5 {
            // 不足 5 字节，说明到达文件末尾或文件损坏
            if bytes_read == 0 {
                return Ok(None); // 正常结束
            } else {
                return Err(WalError::IncompleteRecord);
            }
        }

        // 解析长度
        let len = u32::from_le_bytes([header_buf[1], header_buf[2], header_buf[3], header_buf[4]])
            as usize;

        // 读取完整记录
        let mut record_buf = vec![0u8; 5 + len];
        record_buf[..5].copy_from_slice(&header_buf);

        self.file
            .read_exact(&mut record_buf[5..])
            .map_err(|e| WalError::IoError(e.to_string()))?;

        let (record, _) = WalRecord::deserialize(&record_buf)?;
        Ok(Some(record))
    }

    /// 读取所有 WAL 记录
    pub fn read_all(&mut self) -> Result<Vec<WalRecord>, WalError> {
        let mut records = Vec::new();
        while let Some(record) = self.read_next()? {
            records.push(record);
        }
        Ok(records)
    }

    /// 定位到指定 LSN（字节偏移）
    pub fn seek_to(&mut self, lsn: u64) -> Result<(), WalError> {
        self.file
            .seek(SeekFrom::Start(lsn))
            .map_err(|e| WalError::IoError(e.to_string()))?;
        Ok(())
    }

    /// 获取当前位置（字节偏移）
    pub fn current_position(&mut self) -> Result<u64, WalError> {
        self.file
            .stream_position()
            .map_err(|e| WalError::IoError(e.to_string()))
    }

    /// 获取 WAL 文件路径
    pub fn path(&self) -> &Path {
        &self.wal_path
    }
}

impl std::io::Read for WalReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl std::io::Seek for WalReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}
