//! WAL Record 定义与序列化/反序列化
//!
//! 序列化格式：
//! - [record_type: 1B][record_len: 4B LE][record_data: variable]
//!
//! 记录类型：
//! - Insert: table_id + row_id + data
//! - Update: table_id + row_id + old_data + new_data
//! - Delete: table_id + row_id
//! - Commit: tx_id
//! - Abort: tx_id
//! - Checkpoint: active_tx_ids

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
            _ => Err(WalError::InvalidRecordType(value)),
        }
    }
}

/// WAL 记录
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalRecord {
    /// 插入记录
    Insert {
        table_id: u32,
        row_id: RowId,
        data: Vec<u8>,
    },
    /// 更新记录
    Update {
        table_id: u32,
        row_id: RowId,
        old_data: Vec<u8>,
        new_data: Vec<u8>,
    },
    /// 删除记录
    Delete { table_id: u32, row_id: RowId },
    /// 事务提交
    Commit { tx_id: u64 },
    /// 事务回滚
    Abort { tx_id: u64 },
    /// 检查点
    Checkpoint { active_tx_ids: Vec<u64> },
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

    /// 序列化记录数据部分
    fn serialize_data(&self) -> Vec<u8> {
        let mut data = Vec::new();

        match self {
            WalRecord::Insert {
                table_id,
                row_id,
                data: record_data,
            } => {
                data.extend_from_slice(&table_id.to_le_bytes());
                let mut row_id_buf = vec![0u8; RowId::SIZE];
                row_id.serialize(&mut row_id_buf);
                data.extend(row_id_buf);
                data.extend(serialize_bytes(record_data));
            }
            WalRecord::Update {
                table_id,
                row_id,
                old_data,
                new_data,
            } => {
                data.extend_from_slice(&table_id.to_le_bytes());
                let mut row_id_buf = vec![0u8; RowId::SIZE];
                row_id.serialize(&mut row_id_buf);
                data.extend(row_id_buf);
                data.extend(serialize_bytes(old_data));
                data.extend(serialize_bytes(new_data));
            }
            WalRecord::Delete { table_id, row_id } => {
                data.extend_from_slice(&table_id.to_le_bytes());
                let mut row_id_buf = vec![0u8; RowId::SIZE];
                row_id.serialize(&mut row_id_buf);
                data.extend(row_id_buf);
            }
            WalRecord::Commit { tx_id } | WalRecord::Abort { tx_id } => {
                data.extend_from_slice(&tx_id.to_le_bytes());
            }
            WalRecord::Checkpoint { active_tx_ids } => {
                data.extend_from_slice(&(active_tx_ids.len() as u32).to_le_bytes());
                for tx_id in active_tx_ids {
                    data.extend_from_slice(&tx_id.to_le_bytes());
                }
            }
        }

        data
    }

    /// 反序列化记录数据部分
    fn deserialize_data(record_type: WalRecordType, buf: &[u8]) -> Result<Self, WalError> {
        match record_type {
            WalRecordType::Insert => {
                if buf.len() < 10 {
                    return Err(WalError::IncompleteRecord);
                }
                let table_id = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                let row_id = RowId::deserialize(&buf[4..10]);
                let data = deserialize_bytes(&buf[10..])?;
                Ok(WalRecord::Insert {
                    table_id,
                    row_id,
                    data,
                })
            }
            WalRecordType::Update => {
                if buf.len() < 10 {
                    return Err(WalError::IncompleteRecord);
                }
                let table_id = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                let row_id = RowId::deserialize(&buf[4..10]);
                let (old_data, consumed) = deserialize_bytes_with_len(&buf[10..])?;
                let new_data = deserialize_bytes(&buf[10 + consumed..])?;
                Ok(WalRecord::Update {
                    table_id,
                    row_id,
                    old_data,
                    new_data,
                })
            }
            WalRecordType::Delete => {
                if buf.len() < 10 {
                    return Err(WalError::IncompleteRecord);
                }
                let table_id = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                let row_id = RowId::deserialize(&buf[4..10]);
                Ok(WalRecord::Delete { table_id, row_id })
            }
            WalRecordType::Commit => {
                if buf.len() < 8 {
                    return Err(WalError::IncompleteRecord);
                }
                let tx_id = u64::from_le_bytes([
                    buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
                ]);
                Ok(WalRecord::Commit { tx_id })
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
                if buf.len() < 4 {
                    return Err(WalError::IncompleteRecord);
                }
                let count = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
                if buf.len() < 4 + count * 8 {
                    return Err(WalError::IncompleteRecord);
                }
                let mut active_tx_ids = Vec::with_capacity(count);
                for i in 0..count {
                    let offset = 4 + i * 8;
                    let tx_id = u64::from_le_bytes([
                        buf[offset],
                        buf[offset + 1],
                        buf[offset + 2],
                        buf[offset + 3],
                        buf[offset + 4],
                        buf[offset + 5],
                        buf[offset + 6],
                        buf[offset + 7],
                    ]);
                    active_tx_ids.push(tx_id);
                }
                Ok(WalRecord::Checkpoint { active_tx_ids })
            }
        }
    }
}

impl fmt::Display for WalRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WalRecord::Insert {
                table_id,
                row_id,
                data,
            } => {
                write!(
                    f,
                    "Insert(table={}, row={}, data_len={})",
                    table_id,
                    row_id,
                    data.len()
                )
            }
            WalRecord::Update {
                table_id,
                row_id,
                old_data,
                new_data,
            } => {
                write!(
                    f,
                    "Update(table={}, row={}, old_len={}, new_len={})",
                    table_id,
                    row_id,
                    old_data.len(),
                    new_data.len()
                )
            }
            WalRecord::Delete { table_id, row_id } => {
                write!(f, "Delete(table={}, row={})", table_id, row_id)
            }
            WalRecord::Commit { tx_id } => {
                write!(f, "Commit(tx={})", tx_id)
            }
            WalRecord::Abort { tx_id } => {
                write!(f, "Abort(tx={})", tx_id)
            }
            WalRecord::Checkpoint { active_tx_ids } => {
                write!(f, "Checkpoint(active={:?})", active_tx_ids)
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
    /// IO 错误
    IoError(String),
}

impl std::fmt::Display for WalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalError::IncompleteRecord => write!(f, "Incomplete WAL record"),
            WalError::InvalidRecordType(t) => write!(f, "Invalid WAL record type: 0x{:02X}", t),
            WalError::IoError(msg) => write!(f, "WAL IO error: {}", msg),
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
            table_id: 1,
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
}
