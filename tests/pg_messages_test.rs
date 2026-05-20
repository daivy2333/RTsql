use rtsql::network::pg_messages;
use rtsql::executor::Value;

#[test]
fn test_authentication_ok_serialization() {
    let bytes = pg_messages::authentication_ok();

    // PostgreSQL AuthenticationOk format: 'R' + length(8) + code(0)
    assert_eq!(bytes.len(), 9);
    assert_eq!(bytes[0], b'R'); // Message type

    // Length (Int32 BE): 8
    assert_eq!(bytes[1..5], [0, 0, 0, 8]);

    // Auth code (Int32 BE): 0 (AuthenticationOk)
    assert_eq!(bytes[5..9], [0, 0, 0, 0]);
}

#[test]
fn test_parameter_status_serialization() {
    let bytes = pg_messages::parameter_status("server_version", "14.0");

    // Format: 'S' + length + name(NUL) + value(NUL)
    assert_eq!(bytes[0], b'S');

    // Length = 4 (length field) + 14 (name) + 1 (NUL) + 4 (value) + 1 (NUL) = 24
    let length = i32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
    assert_eq!(length, 24);
}

#[test]
fn test_backend_key_data_serialization() {
    let bytes = pg_messages::backend_key_data(12345, 67890);

    // Format: 'K' + length(12) + process_id + secret_key
    assert_eq!(bytes[0], b'K');
    assert_eq!(bytes.len(), 13);

    let length = i32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
    assert_eq!(length, 12);
}

#[test]
fn test_ready_for_query_serialization() {
    let bytes = pg_messages::ready_for_query('I');

    // Format: 'Z' + length(5) + status('I')
    // Total: 1 (type) + 4 (length) + 1 (status) = 6 bytes
    assert_eq!(bytes[0], b'Z');
    assert_eq!(bytes.len(), 6);
    assert_eq!(bytes[5], b'I');
}

#[test]
fn test_row_description_serialization() {
    let columns = vec![
        ("id", Value::Int(0)),  // OID 23
        ("name", Value::String(String::new())),  // OID 25
    ];

    let bytes = pg_messages::row_description(&columns);

    // Format: 'T' + length + field_count + fields...
    assert_eq!(bytes[0], b'T');

    // Field count (Int16 BE): 2
    let field_count = i16::from_be_bytes([bytes[5], bytes[6]]);
    assert_eq!(field_count, 2);
}

#[test]
fn test_data_row_serialization() {
    let row = vec![
        Value::Int(42),
        Value::String("hello".to_string()),
        Value::Null,
    ];

    let bytes = pg_messages::data_row(&row);

    // Format: 'D' + length + column_count + columns...
    assert_eq!(bytes[0], b'D');

    // Column count (Int16 BE): 3
    let column_count = i16::from_be_bytes([bytes[5], bytes[6]]);
    assert_eq!(column_count, 3);
}

#[test]
fn test_command_complete_serialization() {
    let bytes = pg_messages::command_complete("SELECT 5");

    // Format: 'C' + length + tag(NUL)
    assert_eq!(bytes[0], b'C');
    assert!(bytes.ends_with(&[0]));  // NUL terminated
}