//! WAL Record 序列化/反序列化测试
//!
//! 测试覆盖：
//! - Insert/Update/Delete/Commit/Abort/Checkpoint 6种记录类型
//! - 序列化格式正确性
//! - 反序列化边界检查
//! - 错误处理

use rtsql::wal::{WalError, WalRecord};

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

#[test]
fn test_begin_txn_roundtrip() {
    let record = WalRecord::BeginTxn { tx_id: 42 };
    let serialized = record.serialize();
    let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();
    assert_eq!(consumed, serialized.len());
    assert_eq!(record, deserialized);
}

#[test]
fn test_commit_txn_roundtrip() {
    let record = WalRecord::CommitTxn {
        tx_id: 100,
        timestamp: 99999,
    };
    let serialized = record.serialize();
    let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();
    assert_eq!(consumed, serialized.len());
    assert_eq!(record, deserialized);
}

#[test]
fn test_abort_txn_roundtrip() {
    let record = WalRecord::AbortTxn { tx_id: 77 };
    let serialized = record.serialize();
    let (deserialized, consumed) = WalRecord::deserialize(&serialized).unwrap();
    assert_eq!(consumed, serialized.len());
    assert_eq!(record, deserialized);
}

#[test]
fn test_lsn_crc_roundtrip() {
    let record = WalRecord::Insert {
        tx_id: 55,
        table_name: "test_table".to_string(),
        row_id: rtsql::storage::RowId::new(3, 7),
        tuple_data: vec![10, 20, 30],
    };
    let lsn: u64 = 12345;
    let serialized = record.serialize_with_lsn(lsn);
    let (deserialized_lsn, deserialized_record, consumed) =
        WalRecord::deserialize_with_lsn(&serialized).unwrap();
    assert_eq!(lsn, deserialized_lsn);
    assert_eq!(record, deserialized_record);
    assert_eq!(consumed, serialized.len());
}

#[test]
fn test_crc_mismatch_detected() {
    let record = WalRecord::CommitTxn {
        tx_id: 1,
        timestamp: 2,
    };
    let lsn: u64 = 100;
    let mut serialized = record.serialize_with_lsn(lsn);

    // Tamper with a byte in the body area (after header, before CRC)
    // Format: [lsn:8B][type:1B][len:4B][body:variable][crc:4B]
    // Tamper the first body byte
    let body_start = 8 + 1 + 4; // lsn + type + len
    if serialized.len() > body_start + 4 {
        serialized[body_start] ^= 0xFF;
    }

    let result = WalRecord::deserialize_with_lsn(&serialized);
    assert!(matches!(result, Err(WalError::ChecksumMismatch)));
}

#[test]
fn test_tx_id_method() {
    let insert = WalRecord::Insert {
        tx_id: 10,
        table_name: "t".to_string(),
        row_id: rtsql::storage::RowId::new(1, 1),
        tuple_data: vec![],
    };
    assert_eq!(insert.tx_id(), 10);

    let begin = WalRecord::BeginTxn { tx_id: 20 };
    assert_eq!(begin.tx_id(), 20);

    let commit_txn = WalRecord::CommitTxn {
        tx_id: 30,
        timestamp: 0,
    };
    assert_eq!(commit_txn.tx_id(), 30);

    let abort_txn = WalRecord::AbortTxn { tx_id: 40 };
    assert_eq!(abort_txn.tx_id(), 40);

    let checkpoint = WalRecord::Checkpoint {
        lsn: 0,
        timestamp: 0,
    };
    assert_eq!(checkpoint.tx_id(), 0);
}
