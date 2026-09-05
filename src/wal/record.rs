//! WAL Record 定义与序列化/反序列化
//!
//! 序列化格式：
//! - [record_type: 1B][record_len: 4B LE][record_data: variable]
//!
//! 记录类型：
//! - Insert: tx_id + table_name + row_id + tuple_data
//! - Update: tx_id + table_name + row_id + old_tuple + new_tuple
//! - Delete: tx_id + table_name + row_id
//! - Commit: tx_id + timestamp
//! - Abort: tx_id
//! - Checkpoint: lsn + timestamp

use crate::storage::RowId;
use std::fmt;

/// WAL 记录类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WalRecordType {
    Insert = 0x01,
    Update = 0x02,
    Delete = 0x03,
    Commit = 0x04,
    Abort = 0x05,
    Checkpoint = 0x06,
    BeginTxn = 0x07,
    CommitTxn = 0x08,
    AbortTxn = 0x09,
}

impl TryFrom<u8> for WalRecordType {
    type Error = WalError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(WalRecordType::Insert),
            0x02 => Ok(WalRecordType::Update),
            0x03 => Ok(WalRecordType::Delete),
            0x04 => Ok(WalRecordType::Commit),
            0x05 => Ok(WalRecordType::Abort),
            0x06 => Ok(WalRecordType::Checkpoint),
            0x07 => Ok(WalRecordType::BeginTxn),
            0x08 => Ok(WalRecordType::CommitTxn),
            0x09 => Ok(WalRecordType::AbortTxn),
            _ => Err(WalError::InvalidRecordType(value)),
        }
    }
}

/// WAL 记录
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalRecord {
    /// 插入记录
    Insert {
        tx_id: u64,
        table_name: String,
        row_id: RowId,
        tuple_data: Vec<u8>,
    },
    /// 更新记录
    Update {
        tx_id: u64,
        table_name: String,
        row_id: RowId,
        old_tuple: Vec<u8>,
        new_tuple: Vec<u8>,
    },
    /// 删除记录
    Delete {
        tx_id: u64,
        table_name: String,
        row_id: RowId,
    },
    /// 事务提交
    Commit { tx_id: u64, timestamp: u64 },
    /// 事务回滚
    Abort { tx_id: u64 },
    /// 检查点
    Checkpoint { lsn: u64, timestamp: u64 },
    /// 事务开始（新格式）
    BeginTxn { tx_id: u64 },
    /// 事务提交（新格式）
    CommitTxn { tx_id: u64, timestamp: u64 },
    /// 事务回滚（新格式）
    AbortTxn { tx_id: u64 },
}

impl WalRecord {
    /// 获取记录类型
    pub fn record_type(&self) -> WalRecordType {
        match self {
            WalRecord::Insert { .. } => WalRecordType::Insert,
            WalRecord::Update { .. } => WalRecordType::Update,
            WalRecord::Delete { .. } => WalRecordType::Delete,
            WalRecord::Commit { .. } => WalRecordType::Commit,
            WalRecord::Abort { .. } => WalRecordType::Abort,
            WalRecord::Checkpoint { .. } => WalRecordType::Checkpoint,
            WalRecord::BeginTxn { .. } => WalRecordType::BeginTxn,
            WalRecord::CommitTxn { .. } => WalRecordType::CommitTxn,
            WalRecord::AbortTxn { .. } => WalRecordType::AbortTxn,
        }
    }

    /// 获取记录关联的事务 ID（Checkpoint 返回 0）
    pub fn tx_id(&self) -> u64 {
        match self {
            WalRecord::Insert { tx_id, .. } => *tx_id,
            WalRecord::Update { tx_id, .. } => *tx_id,
            WalRecord::Delete { tx_id, .. } => *tx_id,
            WalRecord::Commit { tx_id, .. } => *tx_id,
            WalRecord::Abort { tx_id } => *tx_id,
            WalRecord::Checkpoint { .. } => 0,
            WalRecord::BeginTxn { tx_id } => *tx_id,
            WalRecord::CommitTxn { tx_id, .. } => *tx_id,
            WalRecord::AbortTxn { tx_id } => *tx_id,
        }
    }

    /// 序列化记录
    ///
    /// 格式: [type: 1B][len: 4B LE][data: variable]
    pub fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::new();

        // 写入类型
        data.push(self.record_type() as u8);

        // 序列化记录数据（先不写长度，后面计算）
        let record_data = self.serialize_data();

        // 写入长度（4字节 LE）
        data.extend_from_slice(&(record_data.len() as u32).to_le_bytes());

        // 写入记录数据
        data.extend(record_data);

        data
    }

    /// 反序列化记录
    ///
    /// 返回 (记录, 消耗的字节数)
    pub fn deserialize(buf: &[u8]) -> Result<(Self, usize), WalError> {
        // 至少需要 5 字节 (type + len)
        if buf.len() < 5 {
            return Err(WalError::IncompleteRecord);
        }

        // 读取类型
        let record_type = WalRecordType::try_from(buf[0])?;

        // 读取长度
        let len = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;

        // 检查缓冲区是否足够
        if buf.len() < 5 + len {
            return Err(WalError::IncompleteRecord);
        }

        // 反序列化记录数据
        let data_buf = &buf[5..5 + len];
        let record = Self::deserialize_data(record_type, data_buf)?;

        Ok((record, 5 + len))
    }

    /// 带 LSN + CRC32 的序列化
    ///
    /// 格式: [lsn: 8B LE][type: 1B][len: 4B LE][body: variable][crc32: 4B LE]
    /// len = body 的长度
    /// crc32 = 对 [lsn + type + len + body] 计算的 CRC32
    pub fn serialize_with_lsn(&self, lsn: u64) -> Vec<u8> {
        let record_data = self.serialize_data();

        // Build the content: [lsn:8B][type:1B][len:4B][body:variable]
        let mut buf = Vec::with_capacity(8 + 1 + 4 + record_data.len() + 4);
        buf.extend_from_slice(&lsn.to_le_bytes());
        buf.push(self.record_type() as u8);
        buf.extend_from_slice(&(record_data.len() as u32).to_le_bytes());
        buf.extend(&record_data);

        // Compute CRC32 over [lsn + type + len + body]
        let crc = crc32fast::hash(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());

        buf
    }

    /// 带 LSN + CRC32 的反序列化
    ///
    /// 返回 (lsn, 记录, 消耗的字节数)
    pub fn deserialize_with_lsn(buf: &[u8]) -> Result<(u64, Self, usize), WalError> {
        // 最少需要 8 (lsn) + 1 (type) + 4 (len) + 4 (crc) = 17 字节
        if buf.len() < 17 {
            return Err(WalError::IncompleteRecord);
        }

        // 读取 LSN
        let lsn = u64::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
        ]);

        // 读取类型
        let record_type = WalRecordType::try_from(buf[8])?;

        // 读取长度
        let len = u32::from_le_bytes([buf[9], buf[10], buf[11], buf[12]]) as usize;

        // 检查缓冲区是否足够（header 13B + body + crc 4B）
        let total_len = 8 + 1 + 4 + len + 4;
        if buf.len() < total_len {
            return Err(WalError::IncompleteRecord);
        }

        // 验证 CRC32：对 [lsn + type + len + body] 计算
        let content_end = 8 + 1 + 4 + len;
        let expected_crc = crc32fast::hash(&buf[..content_end]);
        let stored_crc = u32::from_le_bytes([
            buf[content_end],
            buf[content_end + 1],
            buf[content_end + 2],
            buf[content_end + 3],
        ]);

        if expected_crc != stored_crc {
            return Err(WalError::ChecksumMismatch);
        }

        // 反序列化记录数据
        let data_buf = &buf[13..13 + len];
        let record = Self::deserialize_data(record_type, data_buf)?;

        Ok((lsn, record, total_len))
    }

    /// 序列化记录数据部分
    fn serialize_data(&self) -> Vec<u8> {
        let mut data = Vec::new();

        match self {
            WalRecord::Insert {
                tx_id,
                table_name,
                row_id,
                tuple_data,
            } => {
                data.extend_from_slice(&tx_id.to_le_bytes());
                serialize_string(&mut data, table_name);
                serialize_row_id(&mut data, row_id);
                data.extend(serialize_bytes(tuple_data));
            }
            WalRecord::Update {
                tx_id,
                table_name,
                row_id,
                old_tuple,
                new_tuple,
            } => {
                data.extend_from_slice(&tx_id.to_le_bytes());
                serialize_string(&mut data, table_name);
                serialize_row_id(&mut data, row_id);
                data.extend(serialize_bytes(old_tuple));
                data.extend(serialize_bytes(new_tuple));
            }
            WalRecord::Delete {
                tx_id,
                table_name,
                row_id,
            } => {
                data.extend_from_slice(&tx_id.to_le_bytes());
                serialize_string(&mut data, table_name);
                serialize_row_id(&mut data, row_id);
            }
            WalRecord::Commit { tx_id, timestamp } => {
                data.extend_from_slice(&tx_id.to_le_bytes());
                data.extend_from_slice(&timestamp.to_le_bytes());
            }
            WalRecord::Abort { tx_id } => {
                data.extend_from_slice(&tx_id.to_le_bytes());
            }
            WalRecord::Checkpoint { lsn, timestamp } => {
                data.extend_from_slice(&lsn.to_le_bytes());
                data.extend_from_slice(&timestamp.to_le_bytes());
            }
            WalRecord::BeginTxn { tx_id } => {
                data.extend_from_slice(&tx_id.to_le_bytes());
            }
            WalRecord::CommitTxn { tx_id, timestamp } => {
                data.extend_from_slice(&tx_id.to_le_bytes());
                data.extend_from_slice(&timestamp.to_le_bytes());
            }
            WalRecord::AbortTxn { tx_id } => {
                data.extend_from_slice(&tx_id.to_le_bytes());
            }
        }

        data
    }

    /// 反序列化记录数据部分
    fn deserialize_data(record_type: WalRecordType, buf: &[u8]) -> Result<Self, WalError> {
        match record_type {
            WalRecordType::Insert => {
                if buf.len() < 8 {
                    return Err(WalError::IncompleteRecord);
                }
                let tx_id = u64::from_le_bytes([
                    buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
                ]);
                let (table_name, consumed1) = read_string(&buf[8..])?;
                let offset = 8 + consumed1;
                let row_id = read_row_id(&buf[offset..])?;
                let tuple_data = deserialize_bytes(&buf[offset + 6..])?;
                Ok(WalRecord::Insert {
                    tx_id,
                    table_name,
                    row_id,
                    tuple_data,
                })
            }
            WalRecordType::Update => {
                if buf.len() < 8 {
                    return Err(WalError::IncompleteRecord);
                }
                let tx_id = u64::from_le_bytes([
                    buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
                ]);
                let (table_name, consumed1) = read_string(&buf[8..])?;
                let offset = 8 + consumed1;
                let row_id = read_row_id(&buf[offset..])?;
                let (old_tuple, consumed2) = deserialize_bytes_with_len(&buf[offset + 6..])?;
                let new_tuple = deserialize_bytes(&buf[offset + 6 + consumed2..])?;
                Ok(WalRecord::Update {
                    tx_id,
                    table_name,
                    row_id,
                    old_tuple,
                    new_tuple,
                })
            }
            WalRecordType::Delete => {
                if buf.len() < 8 {
                    return Err(WalError::IncompleteRecord);
                }
                let tx_id = u64::from_le_bytes([
                    buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
                ]);
                let (table_name, consumed1) = read_string(&buf[8..])?;
                let offset = 8 + consumed1;
                let row_id = read_row_id(&buf[offset..])?;
                Ok(WalRecord::Delete {
                    tx_id,
                    table_name,
                    row_id,
                })
            }
            WalRecordType::Commit => {
                if buf.len() < 16 {
                    return Err(WalError::IncompleteRecord);
                }
                let tx_id = u64::from_le_bytes([
                    buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
                ]);
                let timestamp = u64::from_le_bytes([
                    buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
                ]);
                Ok(WalRecord::Commit { tx_id, timestamp })
            }
            WalRecordType::Abort => {
                if buf.len() < 8 {
                    return Err(WalError::IncompleteRecord);
                }
                let tx_id = u64::from_le_bytes([
                    buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
                ]);
                Ok(WalRecord::Abort { tx_id })
            }
            WalRecordType::Checkpoint => {
                if buf.len() < 16 {
                    return Err(WalError::IncompleteRecord);
                }
                let lsn = u64::from_le_bytes([
                    buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
                ]);
                let timestamp = u64::from_le_bytes([
                    buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
                ]);
                Ok(WalRecord::Checkpoint { lsn, timestamp })
            }
            WalRecordType::BeginTxn => {
                if buf.len() < 8 {
                    return Err(WalError::IncompleteRecord);
                }
                let tx_id = u64::from_le_bytes([
                    buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
                ]);
                Ok(WalRecord::BeginTxn { tx_id })
            }
            WalRecordType::CommitTxn => {
                if buf.len() < 16 {
                    return Err(WalError::IncompleteRecord);
                }
                let tx_id = u64::from_le_bytes([
                    buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
                ]);
                let timestamp = u64::from_le_bytes([
                    buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
                ]);
                Ok(WalRecord::CommitTxn { tx_id, timestamp })
            }
            WalRecordType::AbortTxn => {
                if buf.len() < 8 {
                    return Err(WalError::IncompleteRecord);
                }
                let tx_id = u64::from_le_bytes([
                    buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
                ]);
                Ok(WalRecord::AbortTxn { tx_id })
            }
        }
    }
}

impl fmt::Display for WalRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WalRecord::Insert {
                tx_id,
                table_name,
                row_id,
                tuple_data,
            } => {
                write!(
                    f,
                    "Insert(tx={}, table={}, row={}, data_len={})",
                    tx_id,
                    table_name,
                    row_id,
                    tuple_data.len()
                )
            }
            WalRecord::Update {
                tx_id,
                table_name,
                row_id,
                old_tuple,
                new_tuple,
            } => {
                write!(
                    f,
                    "Update(tx={}, table={}, row={}, old_len={}, new_len={})",
                    tx_id,
                    table_name,
                    row_id,
                    old_tuple.len(),
                    new_tuple.len()
                )
            }
            WalRecord::Delete {
                tx_id,
                table_name,
                row_id,
            } => {
                write!(
                    f,
                    "Delete(tx={}, table={}, row={})",
                    tx_id, table_name, row_id
                )
            }
            WalRecord::Commit { tx_id, timestamp } => {
                write!(f, "Commit(tx={}, ts={})", tx_id, timestamp)
            }
            WalRecord::Abort { tx_id } => {
                write!(f, "Abort(tx={})", tx_id)
            }
            WalRecord::Checkpoint { lsn, timestamp } => {
                write!(f, "Checkpoint(lsn={}, ts={})", lsn, timestamp)
            }
            WalRecord::BeginTxn { tx_id } => {
                write!(f, "BeginTxn(tx={})", tx_id)
            }
            WalRecord::CommitTxn { tx_id, timestamp } => {
                write!(f, "CommitTxn(tx={}, ts={})", tx_id, timestamp)
            }
            WalRecord::AbortTxn { tx_id } => {
                write!(f, "AbortTxn(tx={})", tx_id)
            }
        }
    }
}

/// WAL 错误类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalError {
    /// 记录不完整
    IncompleteRecord,
    /// 无效的记录类型
    InvalidRecordType(u8),
    /// 无效的 UTF-8 字符串
    InvalidUtf8,
    /// IO 错误
    IoError(String),
    /// CRC32 校验不匹配
    ChecksumMismatch,
    /// Redo 阶段失败（上下文含表名或 tx_id；K05 显式化，不再静默吞错）
    RedoFailed(String),
}

impl std::fmt::Display for WalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalError::IncompleteRecord => write!(f, "Incomplete WAL record"),
            WalError::InvalidRecordType(t) => write!(f, "Invalid WAL record type: 0x{:02X}", t),
            WalError::InvalidUtf8 => write!(f, "Invalid UTF-8 string in WAL record"),
            WalError::IoError(msg) => write!(f, "WAL IO error: {}", msg),
            WalError::ChecksumMismatch => write!(f, "WAL record CRC32 checksum mismatch"),
            WalError::RedoFailed(msg) => write!(f, "WAL redo failed: {}", msg),
        }
    }
}

impl std::error::Error for WalError {}

// ============================================================================
// Helper functions for serialization
// ============================================================================

/// 序列化字节数组（带长度前缀）
///
/// 格式: [len: 4B LE][data: variable]
fn serialize_bytes(data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + data.len());
    buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
    buf.extend_from_slice(data);
    buf
}

/// 反序列化字节数组
fn deserialize_bytes(buf: &[u8]) -> Result<Vec<u8>, WalError> {
    if buf.len() < 4 {
        return Err(WalError::IncompleteRecord);
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if buf.len() < 4 + len {
        return Err(WalError::IncompleteRecord);
    }
    Ok(buf[4..4 + len].to_vec())
}

/// 反序列化字节数组，返回数据 + 消耗的字节数
fn deserialize_bytes_with_len(buf: &[u8]) -> Result<(Vec<u8>, usize), WalError> {
    if buf.len() < 4 {
        return Err(WalError::IncompleteRecord);
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if buf.len() < 4 + len {
        return Err(WalError::IncompleteRecord);
    }
    Ok((buf[4..4 + len].to_vec(), 4 + len))
}

/// 序列化字符串
///
/// 格式: [len: 2B LE][UTF-8 bytes]
pub fn serialize_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(bytes);
}

/// 读取字符串，返回 (字符串, 消耗的字节数)
pub fn read_string(buf: &[u8]) -> Result<(String, usize), WalError> {
    if buf.len() < 2 {
        return Err(WalError::IncompleteRecord);
    }
    let len = u16::from_le_bytes([buf[0], buf[1]]) as usize;
    if buf.len() < 2 + len {
        return Err(WalError::IncompleteRecord);
    }
    let s = std::str::from_utf8(&buf[2..2 + len]).map_err(|_| WalError::InvalidUtf8)?;
    Ok((s.to_string(), 2 + len))
}

/// 序列化 RowId
///
/// 格式: [page_id: 4B LE][slot_id: 2B LE]
pub fn serialize_row_id(buf: &mut Vec<u8>, row_id: &RowId) {
    buf.extend_from_slice(&row_id.page_id.to_le_bytes());
    buf.extend_from_slice(&row_id.slot_id.to_le_bytes());
}

/// 读取 RowId
pub fn read_row_id(buf: &[u8]) -> Result<RowId, WalError> {
    if buf.len() < 6 {
        return Err(WalError::IncompleteRecord);
    }
    let page_id = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let slot_id = u16::from_le_bytes([buf[4], buf[5]]);
    Ok(RowId::new(page_id, slot_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_type_conversion() {
        assert_eq!(
            WalRecordType::try_from(0x01).unwrap(),
            WalRecordType::Insert
        );
        assert_eq!(
            WalRecordType::try_from(0x06).unwrap(),
            WalRecordType::Checkpoint
        );
        assert!(WalRecordType::try_from(0xFF).is_err());
    }

    #[test]
    fn test_delete_record() {
        let record = WalRecord::Delete {
            tx_id: 123,
            table_name: "users".to_string(),
            row_id: RowId::new(2, 3),
        };
        let serialized = record.serialize();
        let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();
        assert_eq!(consumed, serialized.len());
        assert_eq!(record, deserialized);
    }

    #[test]
    fn test_abort_record() {
        let record = WalRecord::Abort { tx_id: 999 };
        let serialized = record.serialize();
        let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();
        assert_eq!(consumed, serialized.len());
        assert_eq!(record, deserialized);
    }

    #[test]
    fn test_commit_record() {
        let record = WalRecord::Commit {
            tx_id: 12345,
            timestamp: 67890,
        };
        let serialized = record.serialize();
        let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();
        assert_eq!(consumed, serialized.len());
        assert_eq!(record, deserialized);
    }

    #[test]
    fn test_checkpoint_record() {
        let record = WalRecord::Checkpoint {
            lsn: 100,
            timestamp: 200,
        };
        let serialized = record.serialize();
        let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();
        assert_eq!(consumed, serialized.len());
        assert_eq!(record, deserialized);
    }
}
