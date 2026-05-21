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
    // Insert { tx_id: u64, table_name: String, row_id: RowId, tuple_data: Vec<u8> }
    let record = WalRecord::Insert {
        tx_id: 123,
        table_name: "users".to_string(),
        row_id: rtsql::storage::RowId::new(1, 2),
        tuple_data: vec![1, 2, 3, 4, 5],
    };

    let serialized = record.serialize();
    let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();

    assert_eq!(consumed, serialized.len());
    assert_eq!(record, deserialized);
}

#[test]
fn test_update_record_serialize_deserialize() {
    // Update { tx_id: u64, table_name: String, row_id: RowId, old_tuple: Vec<u8>, new_tuple: Vec<u8> }
    let record = WalRecord::Update {
        tx_id: 456,
        table_name: "products".to_string(),
        row_id: rtsql::storage::RowId::new(5, 10),
        old_tuple: vec![1, 2, 3],
        new_tuple: vec![4, 5, 6, 7],
    };

    let serialized = record.serialize();
    let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();

    assert_eq!(consumed, serialized.len());
    assert_eq!(record, deserialized);
}

#[test]
fn test_delete_record_serialize_deserialize() {
    // Delete { tx_id: u64, table_name: String, row_id: RowId }
    let record = WalRecord::Delete {
        tx_id: 789,
        table_name: "orders".to_string(),
        row_id: rtsql::storage::RowId::new(100, 200),
    };

    let serialized = record.serialize();
    let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();

    assert_eq!(consumed, serialized.len());
    assert_eq!(record, deserialized);
}

#[test]
fn test_commit_record_serialize_deserialize() {
    // Commit { tx_id: u64, timestamp: u64 }
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
fn test_abort_record_serialize_deserialize() {
    // Abort { tx_id: u64 }
    let record = WalRecord::Abort { tx_id: 999 };

    let serialized = record.serialize();
    let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();

    assert_eq!(consumed, serialized.len());
    assert_eq!(record, deserialized);
}

#[test]
fn test_checkpoint_record_serialize_deserialize() {
    // Checkpoint { lsn: u64, timestamp: u64 }
    let record = WalRecord::Checkpoint {
        lsn: 100,
        timestamp: 200,
    };

    let serialized = record.serialize();
    let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();

    assert_eq!(consumed, serialized.len());
    assert_eq!(record, deserialized);
}

#[test]
fn test_incomplete_record_error() {
    // Test deserializing incomplete buffer
    let record = WalRecord::Commit {
        tx_id: 123,
        timestamp: 456,
    };
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
