//! Tests for PostgreSQL Simple Query Protocol state machine
//!
//! Task 9: PgProtocol state machine structure
//! Task 10: PgProtocol parse_request implementation

use rtsql::network::{PgProtocol, Protocol, Request};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Helper: Create a pair of connected streams for testing
async fn create_stream_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let client = TcpStream::connect(addr).await.unwrap();
    let (server, _) = listener.accept().await.unwrap();

    (client, server)
}

/// Build a StartupMessage for PostgreSQL 3.0
fn build_startup_message(params: &[(&str, &str)]) -> Vec<u8> {
    let mut msg = Vec::new();

    // Protocol version 3.0 = (3 << 16) | 0 = 196608
    let version = 196608u32;

    // Build parameters
    let mut params_data = Vec::new();
    for (key, value) in params {
        params_data.extend_from_slice(key.as_bytes());
        params_data.push(0);
        params_data.extend_from_slice(value.as_bytes());
        params_data.push(0);
    }

    // Total length: 4 (length) + 4 (version) + params_data
    let length = 4 + 4 + params_data.len() as u32;
    msg.extend_from_slice(&length.to_be_bytes());
    msg.extend_from_slice(&version.to_be_bytes());
    msg.extend(params_data);

    msg
}

/// Build a Query message
fn build_query_message(sql: &str) -> Vec<u8> {
    let mut msg = Vec::new();
    msg.push(b'Q');

    // Length: 4 (length) + sql.len() + 1 (NUL)
    let length = 4 + sql.len() + 1;
    msg.extend_from_slice(&(length as i32).to_be_bytes());
    msg.extend_from_slice(sql.as_bytes());
    msg.push(0);

    msg
}

#[test]
fn test_pg_protocol_initial_state() {
    let protocol = PgProtocol::new();
    assert_eq!(protocol.state(), "Startup");
    // process_id and secret_key should be random (at least one > 0)
    assert!(protocol.process_id() > 0 || protocol.secret_key() > 0);
}

#[tokio::test]
async fn test_startup_message_handling() {
    let (mut client, mut server) = create_stream_pair().await;

    // Client sends StartupMessage
    let startup_msg = build_startup_message(&[("user", "test"), ("database", "testdb")]);
    client.write_all(&startup_msg).await.unwrap();

    // Server parses request
    let mut protocol = PgProtocol::new();
    let result = protocol.parse_request(&mut server).await.unwrap();

    // Should return None (no query yet)
    assert!(result.is_none());
    // State should transition to Ready
    assert_eq!(protocol.state(), "Ready");

    // Client should receive startup response
    // Response: AuthenticationOk + ParameterStatus* + BackendKeyData + ReadyForQuery
    let mut buf = [0u8; 256];
    let n = client.read(&mut buf).await.unwrap();

    // Verify response starts with 'R' (AuthenticationOk)
    assert!(n > 0, "Should receive startup response");
    assert_eq!(buf[0], b'R', "First message should be AuthenticationOk");

    // Parse response messages
    let mut pos = 0;
    let mut has_auth_ok = false;
    let mut has_backend_key = false;
    let mut has_ready = false;
    let mut param_count = 0;

    while pos < n {
        let msg_type = buf[pos];
        let length =
            i32::from_be_bytes([buf[pos + 1], buf[pos + 2], buf[pos + 3], buf[pos + 4]]) as usize;
        match msg_type {
            b'R' => {
                has_auth_ok = true;
                // Verify auth code is 0
                let code =
                    i32::from_be_bytes([buf[pos + 5], buf[pos + 6], buf[pos + 7], buf[pos + 8]]);
                assert_eq!(code, 0, "AuthenticationOk code should be 0");
            }
            b'S' => param_count += 1,
            b'K' => has_backend_key = true,
            b'Z' => has_ready = true,
            _ => {}
        }
        pos += 1 + length; // msg_type (1) + length field + payload
    }

    assert!(has_auth_ok, "Should have AuthenticationOk");
    assert!(has_backend_key, "Should have BackendKeyData");
    assert!(has_ready, "Should have ReadyForQuery");
    assert!(
        param_count >= 2,
        "Should have at least 2 ParameterStatus messages"
    );
}

#[tokio::test]
async fn test_query_message_handling() {
    let (mut client, mut server) = create_stream_pair().await;

    // First complete startup
    let startup_msg = build_startup_message(&[("user", "test"), ("database", "testdb")]);
    client.write_all(&startup_msg).await.unwrap();

    let mut protocol = PgProtocol::new();
    let _ = protocol.parse_request(&mut server).await.unwrap();
    assert_eq!(protocol.state(), "Ready");

    // Drain startup response
    let mut buf = [0u8; 512];
    let _ = client.read(&mut buf).await.unwrap();

    // Client sends Query message
    let query_msg = build_query_message("SELECT 1");
    client.write_all(&query_msg).await.unwrap();

    // Server parses Query
    let result = protocol.parse_request(&mut server).await.unwrap();
    assert!(result.is_some(), "Should return a query request");
    let request = result.unwrap();
    match request {
        Request::Query { sql } => assert_eq!(sql, "SELECT 1"),
        _ => panic!("Expected Query request"),
    }

    // State should transition to Querying
    assert_eq!(protocol.state(), "Querying");
}

#[tokio::test]
async fn test_terminate_message() {
    let (mut client, mut server) = create_stream_pair().await;

    // Complete startup
    let startup_msg = build_startup_message(&[("user", "test")]);
    client.write_all(&startup_msg).await.unwrap();

    let mut protocol = PgProtocol::new();
    let _ = protocol.parse_request(&mut server).await.unwrap();

    // Drain startup response
    let mut buf = [0u8; 512];
    let _ = client.read(&mut buf).await.unwrap();

    // Client sends Terminate message
    let mut terminate_msg = Vec::new();
    terminate_msg.push(b'X');
    terminate_msg.extend_from_slice(&4i32.to_be_bytes()); // length = 4
    client.write_all(&terminate_msg).await.unwrap();

    // Server parses Terminate - should return None
    let result = protocol.parse_request(&mut server).await.unwrap();
    assert!(result.is_none(), "Terminate should return None");
}
