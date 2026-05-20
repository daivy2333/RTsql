//! PostgreSQL wire protocol message types
//!
//! M7: PostgreSQL 3.0 Simple Query Protocol message definitions

use crate::executor::Value;

/// PostgreSQL message serialization functions

/// AuthenticationOk message: 'R' + length(8) + code(0)
pub fn authentication_ok() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(9);
    bytes.push(b'R'); // Message type

    // Length (Int32 BE): 8 (4 bytes for length + 4 bytes for code)
    bytes.extend_from_slice(&8i32.to_be_bytes());

    // Auth code (Int32 BE): 0
    bytes.extend_from_slice(&0i32.to_be_bytes());

    bytes
}

/// ParameterStatus message: 'S' + length + name(NUL) + value(NUL)
pub fn parameter_status(name: &str, value: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(b'S');

    // Calculate length: 4 (length field) + name.len() + 1 (NUL) + value.len() + 1 (NUL)
    let length = 4 + name.len() + 1 + value.len() + 1;
    bytes.extend_from_slice(&(length as i32).to_be_bytes());

    // Name (null-terminated)
    bytes.extend_from_slice(name.as_bytes());
    bytes.push(0);

    // Value (null-terminated)
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);

    bytes
}

/// BackendKeyData message: 'K' + length(12) + process_id + secret_key
pub fn backend_key_data(process_id: u32, secret_key: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(13);
    bytes.push(b'K');

    // Length (Int32 BE): 12
    bytes.extend_from_slice(&12i32.to_be_bytes());

    // Process ID (Int32 BE)
    bytes.extend_from_slice(&process_id.to_be_bytes());

    // Secret Key (Int32 BE)
    bytes.extend_from_slice(&secret_key.to_be_bytes());

    bytes
}

/// ReadyForQuery message: 'Z' + length(5) + status('I'/'T'/'E')
pub fn ready_for_query(status: char) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(5);
    bytes.push(b'Z');

    // Length (Int32 BE): 5
    bytes.extend_from_slice(&5i32.to_be_bytes());

    // Status: 'I' (Idle), 'T' (In transaction), 'E' (Error)
    bytes.push(status as u8);

    bytes
}

/// RowDescription message: 'T' + length + field_count + fields
pub fn row_description(columns: &[(/* name */ &str, /* sample value */ Value)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(b'T');

    // Calculate fields data first
    let mut fields_data = Vec::new();

    // Field count (Int16 BE)
    fields_data.extend_from_slice(&(columns.len() as i16).to_be_bytes());

    for (name, sample_value) in columns {
        // Field name (null-terminated)
        fields_data.extend_from_slice(name.as_bytes());
        fields_data.push(0);

        // Table OID (Int32 BE): 0
        fields_data.extend_from_slice(&0i32.to_be_bytes());

        // Column attr (Int16 BE): 0
        fields_data.extend_from_slice(&0i16.to_be_bytes());

        // Type OID (Int32 BE): Int=23, Text=25, Null=0
        let type_oid = match sample_value {
            Value::Int(_) => 23i32,
            Value::String(_) => 25i32,
            Value::Null => 0i32,
        };
        fields_data.extend_from_slice(&type_oid.to_be_bytes());

        // Type size (Int16 BE): Int=4, Text=-1(varlena), Null=0
        let type_size = match sample_value {
            Value::Int(_) => 4i16,
            Value::String(_) => -1i16,
            Value::Null => 0i16,
        };
        fields_data.extend_from_slice(&type_size.to_be_bytes());

        // Type modifier (Int32 BE): -1
        fields_data.extend_from_slice(&(-1i32).to_be_bytes());

        // Format code (Int16 BE): 0 (text)
        fields_data.extend_from_slice(&0i16.to_be_bytes());
    }

    // Length = 4 (length field) + fields_data.len()
    let length = 4 + fields_data.len();
    bytes.extend_from_slice(&(length as i32).to_be_bytes());

    bytes.extend(fields_data);

    bytes
}
