//! PostgreSQL integration tests
//!
//! Task 13: Integration tests for PostgreSQL 3.0 Simple Query Protocol

use std::sync::Arc;

use rtsql::database::Database;
use rtsql::network::Server;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn start_test_server(port: u16) -> (Server, std::net::SocketAddr, NamedTempFile) {
    let temp_file = NamedTempFile::new().unwrap();
    let database = Arc::new(Database::open(temp_file.path()).await.unwrap());
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let server = Server::new(addr, database);
    (server, addr, temp_file)
}

#[tokio::test]
async fn test_pg_connection_startup() {
    let (server, addr, _temp_file) = start_test_server(15433).await;
    let shutdown = server.shutdown_token();

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    // Wait for server to start
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let mut client = TcpStream::connect(addr).await.unwrap();

    // Build StartupMessage:
    // - length (i32 BE): total message length including itself
    // - protocol version (i32 BE): 196608 for PostgreSQL 3.0
    // - parameters: key\0value\0 pairs, terminated by \0
    let params = b"user\0test_user\0database\0test_db\0";
    let length = 4 + 4 + params.len() + 1; // length field + version + params + final NUL

    let mut startup_msg = Vec::new();
    startup_msg.extend_from_slice(&(length as i32).to_be_bytes());
    startup_msg.extend_from_slice(&196608i32.to_be_bytes()); // Protocol 3.0
    startup_msg.extend_from_slice(params);
    startup_msg.push(0); // Final NUL terminator

    client.write_all(&startup_msg).await.unwrap();

    // Read response sequence
    let mut buf = [0u8; 1024];
    let n = client.read(&mut buf).await.unwrap();
    let response = &buf[..n];

    // Verify response contains:
    // - 'R' (AuthenticationOk)
    // - 'S' (ParameterStatus)
    // - 'K' (BackendKeyData)
    // - 'Z' (ReadyForQuery)
    let has_auth_ok = response.windows(1).any(|w| w[0] == b'R');
    let has_param_status = response.windows(1).any(|w| w[0] == b'S');
    let has_backend_key = response.windows(1).any(|w| w[0] == b'K');
    let has_ready_for_query = response.windows(1).any(|w| w[0] == b'Z');

    assert!(has_auth_ok, "Expected AuthenticationOk message 'R'");
    assert!(has_param_status, "Expected ParameterStatus message 'S'");
    assert!(has_backend_key, "Expected BackendKeyData message 'K'");
    assert!(has_ready_for_query, "Expected ReadyForQuery message 'Z'");

    shutdown.cancel();
}
