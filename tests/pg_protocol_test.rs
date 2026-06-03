//! Tests for PostgreSQL Simple Query Protocol state machine
//!
//! Task 9: PgProtocol state machine structure
//! Task 10: PgProtocol parse_request implementation
//! Task 11: PgProtocol write_response implementation

use rtsql::network::{PgProtocol, Protocol, Request, Response};
use serde_json::json;
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

#[tokio::test]
async fn test_response_mapping_empty_query_result() {
    let (mut client, mut server) = create_stream_pair().await;

    // Complete startup
    let startup_msg = build_startup_message(&[("user", "test")]);
    client.write_all(&startup_msg).await.unwrap();

    let mut protocol = PgProtocol::new();
    let _ = protocol.parse_request(&mut server).await.unwrap();

    // Drain startup response
    let mut buf = [0u8; 512];
    let _ = client.read(&mut buf).await.unwrap();

    // Send a query to transition to Querying state
    let query_msg = build_query_message("SELECT 1");
    client.write_all(&query_msg).await.unwrap();
    let _ = protocol.parse_request(&mut server).await.unwrap();
    assert_eq!(protocol.state(), "Querying");

    // Write empty QueryResult response
    let response = Response::QueryResult { rows: vec![] };
    protocol
        .write_response(&mut server, &response)
        .await
        .unwrap();

    // State should transition back to Ready
    assert_eq!(protocol.state(), "Ready");

    // Client reads response (may need multiple reads for TCP stream)
    let mut total_read = 0;
    let mut all_data = Vec::new();

    // Read until we have at least some data (with timeout)
    for _ in 0..10 {
        let n = client.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        all_data.extend_from_slice(&buf[..n]);
        total_read += n;

        // Check if we have ReadyForQuery (end of response)
        // Look for 'Z' message type in the data
        if all_data
            .windows(5)
            .any(|w| w[0] == b'Z' && w[1] == 0 && w[2] == 0 && w[3] == 0 && w[4] == 5)
        {
            break;
        }

        // Give a small delay for more data
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    let response_data = &all_data[..total_read];

    eprintln!("Response length: {}", total_read);
    eprintln!("Response bytes: {:?}", response_data);

    // Should have CommandComplete("SELECT 0") + ReadyForQuery
    let mut pos = 0;
    let mut has_command_complete = false;
    let mut has_ready = false;

    while pos < response_data.len() {
        let msg_type = response_data[pos];
        let length = i32::from_be_bytes([
            response_data[pos + 1],
            response_data[pos + 2],
            response_data[pos + 3],
            response_data[pos + 4],
        ]) as usize;

        match msg_type {
            b'C' => {
                has_command_complete = true;
                // Verify tag is "SELECT 0"
                let tag_start = pos + 5;
                let tag_end = pos + 1 + length - 1; // Exclude NUL
                let tag = String::from_utf8_lossy(&response_data[tag_start..tag_end]);
                assert!(tag.contains("SELECT 0"), "Tag should be 'SELECT 0'");
            }
            b'Z' => {
                has_ready = true;
                // Verify status is 'I' (Idle)
                assert_eq!(response_data[pos + 5], b'I');
            }
            _ => {}
        }
        pos += 1 + length;
    }

    assert!(has_command_complete, "Should have CommandComplete");
    assert!(has_ready, "Should have ReadyForQuery");
}

#[tokio::test]
async fn test_response_mapping_query_result_with_rows() {
    let (mut client, mut server) = create_stream_pair().await;

    // Complete startup
    let startup_msg = build_startup_message(&[("user", "test")]);
    client.write_all(&startup_msg).await.unwrap();

    let mut protocol = PgProtocol::new();
    let _ = protocol.parse_request(&mut server).await.unwrap();

    // Drain startup response
    let mut buf = [0u8; 1024];
    let _ = client.read(&mut buf).await.unwrap();

    // Send a query to transition to Querying state
    let query_msg = build_query_message("SELECT 1, 2");
    client.write_all(&query_msg).await.unwrap();
    let _ = protocol.parse_request(&mut server).await.unwrap();

    // Write QueryResult with rows
    let response = Response::QueryResult {
        rows: vec![vec![json!(1), json!(2)], vec![json!(3), json!(4)]],
    };
    protocol
        .write_response(&mut server, &response)
        .await
        .unwrap();

    // State should transition back to Ready
    assert_eq!(protocol.state(), "Ready");

    // Client reads response (may need multiple reads for TCP stream)
    let mut total_read = 0;
    let mut all_data = Vec::new();

    for _ in 0..10 {
        let n = client.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        all_data.extend_from_slice(&buf[..n]);
        total_read += n;

        if all_data
            .windows(5)
            .any(|w| w[0] == b'Z' && w[1] == 0 && w[2] == 0 && w[3] == 0 && w[4] == 5)
        {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    let response_data = &all_data[..total_read];

    // Should have RowDescription + 2 DataRow + CommandComplete + ReadyForQuery
    let mut pos = 0;
    let mut has_row_description = false;
    let mut data_row_count = 0;
    let mut has_command_complete = false;
    let mut has_ready = false;

    while pos < response_data.len() {
        let msg_type = response_data[pos];
        let length = i32::from_be_bytes([
            response_data[pos + 1],
            response_data[pos + 2],
            response_data[pos + 3],
            response_data[pos + 4],
        ]) as usize;

        match msg_type {
            b'T' => {
                has_row_description = true;
            }
            b'D' => {
                data_row_count += 1;
            }
            b'C' => {
                has_command_complete = true;
            }
            b'Z' => {
                has_ready = true;
            }
            _ => {}
        }
        pos += 1 + length;
    }

    assert!(has_row_description, "Should have RowDescription");
    assert_eq!(data_row_count, 2, "Should have 2 DataRow messages");
    assert!(has_command_complete, "Should have CommandComplete");
    assert!(has_ready, "Should have ReadyForQuery");
}

#[tokio::test]
async fn test_response_mapping_affected_rows() {
    let (mut client, mut server) = create_stream_pair().await;

    // Complete startup
    let startup_msg = build_startup_message(&[("user", "test")]);
    client.write_all(&startup_msg).await.unwrap();

    let mut protocol = PgProtocol::new();
    let _ = protocol.parse_request(&mut server).await.unwrap();

    // Drain startup response
    let mut buf = [0u8; 512];
    let _ = client.read(&mut buf).await.unwrap();

    // Send an INSERT query
    let query_msg = build_query_message("INSERT INTO test VALUES (1)");
    client.write_all(&query_msg).await.unwrap();
    let _ = protocol.parse_request(&mut server).await.unwrap();

    // Write AffectedRows response
    let response = Response::AffectedRows { count: 5 };
    protocol
        .write_response(&mut server, &response)
        .await
        .unwrap();

    // State should transition back to Ready
    assert_eq!(protocol.state(), "Ready");

    // Client reads response (may need multiple reads for TCP stream)
    let mut total_read = 0;
    let mut all_data = Vec::new();

    for _ in 0..10 {
        let n = client.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        all_data.extend_from_slice(&buf[..n]);
        total_read += n;

        if all_data
            .windows(5)
            .any(|w| w[0] == b'Z' && w[1] == 0 && w[2] == 0 && w[3] == 0 && w[4] == 5)
        {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    let response_data = &all_data[..total_read];

    // Should have CommandComplete("INSERT 5") + ReadyForQuery
    let mut pos = 0;
    let mut has_command_complete = false;
    let mut has_ready = false;

    while pos < response_data.len() {
        let msg_type = response_data[pos];
        let length = i32::from_be_bytes([
            response_data[pos + 1],
            response_data[pos + 2],
            response_data[pos + 3],
            response_data[pos + 4],
        ]) as usize;

        match msg_type {
            b'C' => {
                has_command_complete = true;
                let tag_start = pos + 5;
                let tag_end = pos + 1 + length - 1;
                let tag = String::from_utf8_lossy(&response_data[tag_start..tag_end]);
                assert!(tag.contains("INSERT 5"), "Tag should contain 'INSERT 5'");
            }
            b'Z' => {
                has_ready = true;
            }
            _ => {}
        }
        pos += 1 + length;
    }

    assert!(has_command_complete, "Should have CommandComplete");
    assert!(has_ready, "Should have ReadyForQuery");
}

#[tokio::test]
async fn test_response_mapping_error() {
    let (mut client, mut server) = create_stream_pair().await;

    // Complete startup
    let startup_msg = build_startup_message(&[("user", "test")]);
    client.write_all(&startup_msg).await.unwrap();

    let mut protocol = PgProtocol::new();
    let _ = protocol.parse_request(&mut server).await.unwrap();

    // Drain startup response
    let mut buf = [0u8; 512];
    let _ = client.read(&mut buf).await.unwrap();

    // Send a query
    let query_msg = build_query_message("SELECT * FROM nonexistent");
    client.write_all(&query_msg).await.unwrap();
    let _ = protocol.parse_request(&mut server).await.unwrap();

    // Write Error response
    let response = Response::Error {
        message: "Table not found".to_string(),
    };
    protocol
        .write_response(&mut server, &response)
        .await
        .unwrap();

    // State should transition back to Ready
    assert_eq!(protocol.state(), "Ready");

    // Client reads response (may need multiple reads for TCP stream)
    let mut total_read = 0;
    let mut all_data = Vec::new();

    for _ in 0..10 {
        let n = client.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        all_data.extend_from_slice(&buf[..n]);
        total_read += n;

        if all_data
            .windows(5)
            .any(|w| w[0] == b'Z' && w[1] == 0 && w[2] == 0 && w[3] == 0 && w[4] == 5)
        {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    let response_data = &all_data[..total_read];

    // Should have ErrorResponse + ReadyForQuery
    let mut pos = 0;
    let mut has_error_response = false;
    let mut has_ready = false;

    while pos < response_data.len() {
        let msg_type = response_data[pos];
        let length = i32::from_be_bytes([
            response_data[pos + 1],
            response_data[pos + 2],
            response_data[pos + 3],
            response_data[pos + 4],
        ]) as usize;

        match msg_type {
            b'E' => {
                has_error_response = true;
            }
            b'Z' => {
                has_ready = true;
            }
            _ => {}
        }
        pos += 1 + length;
    }

    assert!(has_error_response, "Should have ErrorResponse");
    assert!(has_ready, "Should have ReadyForQuery");
}

#[tokio::test]
async fn test_response_mapping_pong() {
    let (mut client, mut server) = create_stream_pair().await;

    // Complete startup
    let startup_msg = build_startup_message(&[("user", "test")]);
    client.write_all(&startup_msg).await.unwrap();

    let mut protocol = PgProtocol::new();
    let _ = protocol.parse_request(&mut server).await.unwrap();

    // Drain startup response
    let mut buf = [0u8; 512];
    let _ = client.read(&mut buf).await.unwrap();

    // Send a PING query (custom)
    let query_msg = build_query_message("PING");
    client.write_all(&query_msg).await.unwrap();
    let _ = protocol.parse_request(&mut server).await.unwrap();

    // Write Pong response
    let response = Response::Pong;
    protocol
        .write_response(&mut server, &response)
        .await
        .unwrap();

    // State should transition back to Ready
    assert_eq!(protocol.state(), "Ready");

    // Client reads response (may need multiple reads for TCP stream)
    let mut total_read = 0;
    let mut all_data = Vec::new();

    for _ in 0..10 {
        let n = client.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        all_data.extend_from_slice(&buf[..n]);
        total_read += n;

        if all_data
            .windows(5)
            .any(|w| w[0] == b'Z' && w[1] == 0 && w[2] == 0 && w[3] == 0 && w[4] == 5)
        {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    let response_data = &all_data[..total_read];

    // Should have CommandComplete("PING") + ReadyForQuery
    let mut pos = 0;
    let mut has_command_complete = false;
    let mut has_ready = false;

    while pos < response_data.len() {
        let msg_type = response_data[pos];
        let length = i32::from_be_bytes([
            response_data[pos + 1],
            response_data[pos + 2],
            response_data[pos + 3],
            response_data[pos + 4],
        ]) as usize;

        match msg_type {
            b'C' => {
                has_command_complete = true;
                let tag_start = pos + 5;
                let tag_end = pos + 1 + length - 1;
                let tag = String::from_utf8_lossy(&response_data[tag_start..tag_end]);
                assert!(tag.contains("PING"), "Tag should be 'PING'");
            }
            b'Z' => {
                has_ready = true;
            }
            _ => {}
        }
        pos += 1 + length;
    }

    assert!(has_command_complete, "Should have CommandComplete");
    assert!(has_ready, "Should have ReadyForQuery");


    // Drain: send a query to force server state to Querying, then write Pong
    let query_msg = build_query_message("PING_2");
    client.write_all(&query_msg).await.unwrap();
    let _ = protocol.parse_request(&mut server).await.unwrap();
}

#[tokio::test]
async fn test_batched_write_large_result() {
    let (mut client, mut server) = create_stream_pair().await;

    let startup_msg = build_startup_message(&[("user", "test")]);
    client.write_all(&startup_msg).await.unwrap();

    let mut protocol = PgProtocol::new();
    let _ = protocol.parse_request(&mut server).await.unwrap();

    let mut buf = [0u8; 2048];
    let _ = client.read(&mut buf).await.unwrap();

    let query_msg = build_query_message("SELECT * FROM big_table");
    client.write_all(&query_msg).await.unwrap();
    let _ = protocol.parse_request(&mut server).await.unwrap();

    // 100 rows x 3 columns - exceeds 8KB buffer, tests auto-expansion
    let rows: Vec<Vec<serde_json::Value>> = (0..100)
        .map(|i| vec![json!(i), json!(format!("row_{}", i)), json!(i as f64 * 1.5)])
        .collect();
    let response = Response::QueryResult { rows };
    protocol
        .write_response(&mut server, &response)
        .await
        .unwrap();

    assert_eq!(protocol.state(), "Ready");

    let mut all_data = Vec::new();
    for _ in 0..20 {
        let n = client.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        all_data.extend_from_slice(&buf[..n]);
        if all_data
            .windows(5)
            .any(|w| w[0] == b'Z' && w[1] == 0 && w[2] == 0 && w[3] == 0 && w[4] == 5)
        {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    let mut pos = 0;
    let mut has_row_description = false;
    let mut data_row_count = 0;
    let mut has_command_complete = false;
    let mut has_ready = false;

    while pos < all_data.len() {
        let msg_type = all_data[pos];
        let length = i32::from_be_bytes([
            all_data[pos + 1],
            all_data[pos + 2],
            all_data[pos + 3],
            all_data[pos + 4],
        ]) as usize;

        match msg_type {
            b'T' => has_row_description = true,
            b'D' => data_row_count += 1,
            b'C' => has_command_complete = true,
            b'Z' => has_ready = true,
            _ => {}
        }
        pos += 1 + length;
    }

    assert!(has_row_description, "Should have RowDescription");
    assert_eq!(data_row_count, 100, "Should have 100 DataRow messages");
    assert!(has_command_complete, "Should have CommandComplete");
    assert!(has_ready, "Should have ReadyForQuery");
}

#[tokio::test]
async fn test_multiple_queries_buffer_reuse() {
    let (mut client, mut server) = create_stream_pair().await;

    let startup_msg = build_startup_message(&[("user", "test")]);
    client.write_all(&startup_msg).await.unwrap();

    let mut protocol = PgProtocol::new();
    let _ = protocol.parse_request(&mut server).await.unwrap();

    let mut buf = [0u8; 2048];
    let _ = client.read(&mut buf).await.unwrap();

    // Run multiple queries to verify write_buf is properly cleared and reused
    for batch in 0..4 {
        let query_msg = build_query_message(&format!("SELECT * FROM t{}", batch));
        client.write_all(&query_msg).await.unwrap();
        let _ = protocol.parse_request(&mut server).await.unwrap();

        let rows: Vec<Vec<serde_json::Value>> = (0..5)
            .map(|i| vec![json!(batch * 5 + i), json!("val")])
            .collect();
        let response = Response::QueryResult { rows };
        protocol
            .write_response(&mut server, &response)
            .await
            .unwrap();

        assert_eq!(protocol.state(), "Ready");

        let mut all_data = Vec::new();
        for _ in 0..10 {
            let n = client.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            all_data.extend_from_slice(&buf[..n]);
            if all_data
                .windows(5)
                .any(|w| w[0] == b'Z' && w[1] == 0 && w[2] == 0 && w[3] == 0 && w[4] == 5)
            {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }

        let mut data_count = 0;
        let mut pos = 0;
        while pos < all_data.len() {
            if all_data[pos] == b'D' {
                data_count += 1;
            }
            let length = i32::from_be_bytes([
                all_data[pos + 1],
                all_data[pos + 2],
                all_data[pos + 3],
                all_data[pos + 4],
            ]) as usize;
            pos += 1 + length;
        }
        assert_eq!(data_count, 5, "Batch {}: should have 5 DataRows", batch);
    }
}
