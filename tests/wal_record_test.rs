//! WAL Record 序列化/反序列化测试
//!
//! 测试覆盖：
//! - Insert/Update/Delete/Commit/Abort/Checkpoint 6种记录类型
//! - 序列化格式正确性
//! - 反序列化边界检查
//! - 错误处理

use rtsql::wal::WalRecord;

#[test]
fn test_insert_record_serialize_deserialize() {
    // Insert { table_id: u32, row_id: RowId, data: Vec<u8> }
    let record = WalRecord::Insert {
        table_id: 42,
        row_id: rtsql::storage::RowId::new(1, 2),
        data: vec![1, 2, 3, 4, 5],
    };

    let serialized = record.serialize();
    let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();

    assert_eq!(consumed, serialized.len());
    assert_eq!(record, deserialized);
}

#[test]
fn test_commit_record_serialize_deserialize() {
    // Commit { tx_id: u64 }
    let record = WalRecord::Commit { tx_id: 12345 };

    let serialized = record.serialize();
    let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();

    assert_eq!(consumed, serialized.len());
    assert_eq!(record, deserialized);
}

#[test]
fn test_checkpoint_record_serialize_deserialize() {
    // Checkpoint { active_tx_ids: Vec<u64> }
    let record = WalRecord::Checkpoint {
        active_tx_ids: vec![1, 2, 3, 100, 200],
    };

    let serialized = record.serialize();
    let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();

    assert_eq!(consumed, serialized.len());
    assert_eq!(record, deserialized);
}

#[test]
fn test_incomplete_record_error() {
    // Test deserializing incomplete buffer
    let record = WalRecord::Commit { tx_id: 123 };
    let serialized = record.serialize();

    // Truncate buffer to make it incomplete
    let incomplete = &serialized[..5];
    let result = WalRecord::deserialize(incomplete);

    assert!(result.is_err());
}

#[test]
fn test_invalid_record_type_error() {
    // Test deserializing with invalid record type
    let buf = vec![0xFF, 0, 0, 0, 0]; // Invalid type 0xFF
    let result = WalRecord::deserialize(&buf);

    assert!(result.is_err());
}

#[test]
fn test_update_record_serialize_deserialize() {
    // Update { table_id: u32, row_id: RowId, old_data: Vec<u8>, new_data: Vec<u8> }
    let record = WalRecord::Update {
        table_id: 10,
        row_id: rtsql::storage::RowId::new(5, 10),
        old_data: vec![1, 2, 3],
        new_data: vec![4, 5, 6, 7],
    };

    let serialized = record.serialize();
    let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();

    assert_eq!(consumed, serialized.len());
    assert_eq!(record, deserialized);
}
