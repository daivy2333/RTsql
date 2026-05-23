//! PostgreSQL wire protocol message types
//!
//! M7: PostgreSQL 3.0 Simple Query Protocol message definitions

use crate::executor::Value;
use crate::network::NetworkError;

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

        // Type OID (Int32 BE): Int=23, Text=25, Float=701, Bool=16, Null=0
        let type_oid = match sample_value {
            Value::Int(_) => 23i32,
            Value::String(_) => 25i32,
            Value::Float(_) => 701i32, // PostgreSQL float8 OID
            Value::Bool(_) => 16i32,   // PostgreSQL bool OID
            Value::Null => 0i32,
        };
        fields_data.extend_from_slice(&type_oid.to_be_bytes());

        // Type size (Int16 BE): Int=4, Text=-1(varlena), Float=8, Bool=1, Null=0
        let type_size = match sample_value {
            Value::Int(_) => 4i16,
            Value::String(_) => -1i16,
            Value::Float(_) => 8i16,
            Value::Bool(_) => 1i16,
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

/// DataRow message: 'D' + length + column_count + columns
pub fn data_row(row: &[Value]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(b'D');

    // Calculate columns data first
    let mut columns_data = Vec::new();

    // Column count (Int16 BE)
    columns_data.extend_from_slice(&(row.len() as i16).to_be_bytes());

    for value in row {
        // Column length (Int32 BE): -1 (NULL) or N (data bytes)
        match value {
            Value::Int(n) => {
                // Convert to text format: i64 → ASCII
                let text = n.to_string();
                columns_data.extend_from_slice(&(text.len() as i32).to_be_bytes());
                columns_data.extend_from_slice(text.as_bytes());
            }
            Value::String(s) => {
                columns_data.extend_from_slice(&(s.len() as i32).to_be_bytes());
                columns_data.extend_from_slice(s.as_bytes());
            }
            Value::Float(f) => {
                // Convert to text format: f64 → ASCII
                let text = f.to_string();
                columns_data.extend_from_slice(&(text.len() as i32).to_be_bytes());
                columns_data.extend_from_slice(text.as_bytes());
            }
            Value::Bool(b) => {
                // Convert to text format: bool → "t"/"f"
                let text = if *b { "t" } else { "f" };
                columns_data.extend_from_slice(&(text.len() as i32).to_be_bytes());
                columns_data.extend_from_slice(text.as_bytes());
            }
            Value::Null => {
                // Length = -1 (NULL)
                columns_data.extend_from_slice(&(-1i32).to_be_bytes());
            }
        }
    }

    // Length = 4 (length field) + columns_data.len()
    let length = 4 + columns_data.len();
    bytes.extend_from_slice(&(length as i32).to_be_bytes());

    bytes.extend(columns_data);

    bytes
}

/// CommandComplete message: 'C' + length + tag(NUL)
pub fn command_complete(tag: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(b'C');

    // Length = 4 (length field) + tag.len() + 1 (NUL)
    let length = 4 + tag.len() + 1;
    bytes.extend_from_slice(&(length as i32).to_be_bytes());

    // Tag (null-terminated)
    bytes.extend_from_slice(tag.as_bytes());
    bytes.push(0);

    bytes
}

/// ErrorResponse message: 'E' + length + fields + NUL
///
/// Fields format: field_type(byte) + value(NUL)
/// Common fields:
/// - 'S': Severity (e.g., "ERROR", "FATAL", "PANIC")
/// - 'V': Severity (non-localized, same as 'S')
/// - 'C': Code (SQLSTATE, e.g., "42000")
/// - 'M': Message (human-readable error message)
/// - Final NUL terminator
pub fn error_response(severity: &str, code: &str, message: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.push(b'E');

    let mut fields_data = Vec::new();

    // 'S' Severity
    fields_data.push(b'S');
    fields_data.extend_from_slice(severity.as_bytes());
    fields_data.push(0);

    // 'V' Severity (non-localized)
    fields_data.push(b'V');
    fields_data.extend_from_slice(severity.as_bytes());
    fields_data.push(0);

    // 'C' Code (SQLSTATE)
    fields_data.push(b'C');
    fields_data.extend_from_slice(code.as_bytes());
    fields_data.push(0);

    // 'M' Message
    fields_data.push(b'M');
    fields_data.extend_from_slice(message.as_bytes());
    fields_data.push(0);

    // NUL terminator (end of fields)
    fields_data.push(0);

    let length = 4 + fields_data.len();
    bytes.extend_from_slice(&(length as i32).to_be_bytes());
    bytes.extend(fields_data);

    bytes
}

/// Map error to PostgreSQL SQLSTATE code
///
/// Returns (severity, code) where:
/// - severity: "ERROR", "FATAL", or "PANIC"
/// - code: SQLSTATE code (5-character string)
pub fn map_error_to_sqlstate(error: &NetworkError) -> (&'static str, &'static str) {
    match error {
        // Syntax errors: 42000 (syntax error or access rule violation)
        NetworkError::ProtocolParse(_) => ("ERROR", "42000"),

        // System errors: 58000 (system error)
        NetworkError::Io(_) => ("ERROR", "58000"),

        // SQL parse errors: 42601 (syntax error)
        NetworkError::SqlParse(_) => ("ERROR", "42601"),

        // Execution errors: 42000 (syntax error or access rule violation)
        NetworkError::Execution(_) => ("ERROR", "42000"),

        // Protocol write errors: 58000 (system error)
        NetworkError::ProtocolWrite(_) => ("ERROR", "58000"),
    }
}
