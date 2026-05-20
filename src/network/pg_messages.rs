//! PostgreSQL wire protocol message types
//!
//! M7: PostgreSQL 3.0 Simple Query Protocol message definitions

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
