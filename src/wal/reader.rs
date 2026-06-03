//! WAL 读取器
//!
//! 负责从磁盘读取 WAL 记录
//! 支持旧格式 (type+len+data) 和新格式 (lsn+type+len+body+crc32)

use super::record::{WalError, WalRecord, WalRecordType};
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
    /// 自动检测旧格式和新格式（带 LSN + CRC32）
    /// 返回 Ok(None) 表示已到达文件末尾
    pub fn read_next(&mut self) -> Result<Option<WalRecord>, WalError> {
        // 先尝试读取足够多的字节来判断格式
        let mut peek_buf = [0u8; 13]; // 至少 13 字节: lsn(8) + type(1) + len(4)
        let bytes_read = self
            .file
            .read(&mut peek_buf)
            .map_err(|e| WalError::IoError(e.to_string()))?;

        if bytes_read == 0 {
            return Ok(None); // 正常结束
        }

        if bytes_read < 5 {
            return Err(WalError::IncompleteRecord);
        }

        // 判断格式：检查是否为新格式
        // 新格式: byte[8] 是有效的 WalRecordType
        // 旧格式: byte[0] 是有效的 WalRecordType
        let is_new_format = bytes_read >= 9
            && WalRecordType::try_from(peek_buf[8]).is_ok()
            && WalRecordType::try_from(peek_buf[0]).is_err();

        if is_new_format {
            // 新格式: [lsn:8B][type:1B][len:4B][body:variable][crc:4B]
            if bytes_read < 13 {
                return Err(WalError::IncompleteRecord);
            }

            let len = u32::from_le_bytes([peek_buf[9], peek_buf[10], peek_buf[11], peek_buf[12]])
                as usize;

            let total_len = 8 + 1 + 4 + len + 4;
            let mut record_buf = vec![0u8; total_len];
            record_buf[..bytes_read].copy_from_slice(&peek_buf[..bytes_read]);

            self.file
                .read_exact(&mut record_buf[bytes_read..])
                .map_err(|e| WalError::IoError(e.to_string()))?;

            let (_lsn, record, _consumed) = WalRecord::deserialize_with_lsn(&record_buf)?;
            Ok(Some(record))
        } else {
            // 旧格式: [type:1B][len:4B][data:variable]
            let len =
                u32::from_le_bytes([peek_buf[1], peek_buf[2], peek_buf[3], peek_buf[4]]) as usize;

            let total_len = 5 + len;
            let mut record_buf = vec![0u8; total_len];
            record_buf[..bytes_read.min(total_len)]
                .copy_from_slice(&peek_buf[..bytes_read.min(total_len)]);

            if total_len > bytes_read {
                self.file
                    .read_exact(&mut record_buf[bytes_read..])
                    .map_err(|e| WalError::IoError(e.to_string()))?;
            }

            let (record, _) = WalRecord::deserialize(&record_buf)?;
            Ok(Some(record))
        }
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
