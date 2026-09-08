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
        Ok(self.read_next_with_lsn()?.map(|(_lsn, record)| record))
    }

    /// 读取下一条 WAL 记录及其记录起始字节偏移
    ///
    /// 新格式返回记录内嵌的 LSN（即写入时的文件偏移）；
    /// 旧格式无内嵌 LSN，退化为读取时的文件偏移。
    ///
    /// 格式判别（逐帧无歧义）：新格式帧首字节是内嵌 LSN 的最低有效字节，
    /// 当文件偏移低字节落在合法 type 值域 (0x01-0x09) 时，byte[0] 无法单独
    /// 判别格式——先按新格式解析并以 CRC32 为接受判据，失败则 seek 回帧首
    /// 按旧格式重试；两路皆败显式报错。非歧义偏移（byte[0] 非合法 type）
    /// 必为新格式。
    pub fn read_next_with_lsn(&mut self) -> Result<Option<(u64, WalRecord)>, WalError> {
        let start = self
            .file
            .stream_position()
            .map_err(|e| WalError::IoError(e.to_string()))?;

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

        let byte0_is_type = WalRecordType::try_from(peek_buf[0]).is_ok();

        if !byte0_is_type {
            // 非歧义偏移：必为新格式
            let (lsn, record) = self.read_new_format_frame(&peek_buf, bytes_read)?;
            return Ok(Some((lsn, record)));
        }

        // 歧义偏移：先按新格式解析（CRC 为接受判据）。
        // 新格式尝试的任何失败（EOF / CRC / 结构）都是回退信号，不向调用者传播
        if bytes_read >= 13 {
            if let Ok(pair) = self.read_new_format_frame(&peek_buf, bytes_read) {
                return Ok(Some(pair));
            }
            // seek 回 peek 之后的位点（peek_buf 仍持有帧首前缀），按旧格式重试
            self.file
                .seek(SeekFrom::Start(start + bytes_read as u64))
                .map_err(|e| WalError::IoError(e.to_string()))?;
        }

        // 旧格式: [type:1B][len:4B][data:variable]
        let len = u32::from_le_bytes([peek_buf[1], peek_buf[2], peek_buf[3], peek_buf[4]]) as usize;

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
        Ok(Some((start, record)))
    }

    /// 按新格式 [lsn:8B][type:1B][len:4B][body][crc:4B] 读取一帧（头部已 peek）
    ///
    /// 失败（头部不足 13B / EOF / type 或 CRC 验证不通过）返回 Err；
    /// 由调用者决定传播（非歧义偏移）还是回退旧格式（歧义偏移）。
    fn read_new_format_frame(
        &mut self,
        peek_buf: &[u8; 13],
        bytes_read: usize,
    ) -> Result<(u64, WalRecord), WalError> {
        if bytes_read < 13 {
            return Err(WalError::IncompleteRecord);
        }

        let len =
            u32::from_le_bytes([peek_buf[9], peek_buf[10], peek_buf[11], peek_buf[12]]) as usize;

        let total_len = 8 + 1 + 4 + len + 4;
        let mut record_buf = vec![0u8; total_len];
        record_buf[..bytes_read].copy_from_slice(&peek_buf[..bytes_read]);

        self.file
            .read_exact(&mut record_buf[bytes_read..])
            .map_err(|e| WalError::IoError(e.to_string()))?;

        let (lsn, record, _consumed) = WalRecord::deserialize_with_lsn(&record_buf)?;
        Ok((lsn, record))
    }

    /// 读取所有 WAL 记录（带记录起始字节偏移）
    pub fn read_all_with_lsn(&mut self) -> Result<Vec<(u64, WalRecord)>, WalError> {
        let mut records = Vec::new();
        while let Some((lsn, record)) = self.read_next_with_lsn()? {
            records.push((lsn, record));
        }
        Ok(records)
    }

    /// 读取所有 WAL 记录
    pub fn read_all(&mut self) -> Result<Vec<WalRecord>, WalError> {
        Ok(self
            .read_all_with_lsn()?
            .into_iter()
            .map(|(_lsn, record)| record)
            .collect())
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
