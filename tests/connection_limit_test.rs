//! Connection limit concurrency stress tests
//!
//! Task M30: Test that `max_connections` limits concurrent handlers
//! and excess connections are queued (not rejected at TCP level).
//!
//! Uses real `Server` — validates exclusively through TCP-level observation
//! (response timing), with no test-only hooks or counters in production code.

use std::sync::Arc;
use std::time::Duration;

use rtsql::database::Database;
use rtsql::network::Server;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// ── helpers ────────────────────────────────────────────────────────────

async fn start_limited_server(
    port: u16,
    max_connections: usize,
) -> (Server, std::net::SocketAddr, NamedTempFile) {
    let temp_file = NamedTempFile::new().unwrap();
    let database = Arc::new(Database::open(temp_file.path()).await.unwrap());
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let server = Server::new(addr, database, max_connections);
    (server, addr, temp_file)
}

/// PostgreSQL 3.0 StartupMessage bytes.
/// Parameters: `user=<user>`, `database=<database>`.
fn build_startup(user: &str, database: &str) -> Vec<u8> {
    let params_str = format!("user\0{}\0database\0{}\0", user, database);
    let params = params_str.as_bytes();
    let length = 4 + 4 + params.len() + 1; // len:4 + proto:4 + params + final NUL
    let mut msg = Vec::new();
    msg.extend_from_slice(&(length as i32).to_be_bytes());
    msg.extend_from_slice(&196608i32.to_be_bytes()); // PG 3.0
    msg.extend_from_slice(params);
    msg.push(0); // final NUL terminator
    msg
}

/// Returns true if `buf` contains an AuthenticationOk message ('R' type byte).
fn has_auth_ok(buf: &[u8]) -> bool {
    buf.windows(1).any(|w| w[0] == b'R')
}

/// Send StartupMessage and read response with a 2 s timeout.
/// Returns `Ok(n)` on success or `Err(())` on timeout.
async fn try_handshake(client: &mut TcpStream) -> Result<Vec<u8>, ()> {
    let startup = build_startup("test", "test");
    client.write_all(&startup).await.map_err(|_| ())?;

    let mut buf = vec![0u8; 512];
    let result = tokio::time::timeout(Duration::from_secs(2), client.read(&mut buf)).await;
    match result {
        Ok(Ok(n)) => {
            buf.truncate(n);
            Ok(buf)
        }
        Ok(Err(_)) => Err(()),
        Err(_elapsed) => Err(()),
    }
}

/// Convenience: assert handshake succeeds within timeout.
async fn assert_handshake_ok(client: &mut TcpStream, label: &str) {
    let resp = try_handshake(client).await;
    assert!(
        resp.is_ok(),
        "{}: handshake timed out (handler not started?)",
        label
    );
    assert!(
        has_auth_ok(&resp.unwrap()),
        "{}: response missing AuthenticationOk",
        label
    );
}

// ── test cases ─────────────────────────────────────────────────────────

/// Within the limit: `max=2`, both connections complete the handshake.
#[tokio::test]
async fn test_within_limit_works() {
    let (server, addr, _temp_file) = start_limited_server(15440, 2).await;
    let shutdown = server.shutdown_token();

    tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut conn1 = TcpStream::connect(addr).await.unwrap();
    assert_handshake_ok(&mut conn1, "conn1").await;

    let mut conn2 = TcpStream::connect(addr).await.unwrap();
    assert_handshake_ok(&mut conn2, "conn2").await;

    shutdown.cancel();
}

/// Over the limit: `max=2` with 3 connections — the third is queued (TCP
/// connects but no handler starts). After dropping one active connection
/// the queued handler wakes up and completes the handshake.
#[tokio::test]
async fn test_over_limit_queued() {
    let (server, addr, _temp_file) = start_limited_server(15441, 2).await;
    let shutdown = server.shutdown_token();

    tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    // --- saturate the 2 slots ---
    let mut conn1 = TcpStream::connect(addr).await.unwrap();
    assert_handshake_ok(&mut conn1, "conn1 (slot 1)").await;

    let mut conn2 = TcpStream::connect(addr).await.unwrap();
    assert_handshake_ok(&mut conn2, "conn2 (slot 2)").await;

    // --- 3rd connection: TCP-level connects but handler is queued ---
    let mut conn3 = TcpStream::connect(addr).await.unwrap();
    conn3
        .write_all(&build_startup("test", "test"))
        .await
        .unwrap();

    let mut buf3 = vec![0u8; 512];
    let read3 = tokio::time::timeout(Duration::from_secs(2), conn3.read(&mut buf3)).await;
    assert!(
        read3.is_err(),
        "conn3: expected timeout — semaphore should be full, handler NOT started"
    );

    // --- release one slot: drop conn1 ---
    drop(conn1);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // --- now conn3's handler should wake up and complete handshake ---
    let mut buf3b = vec![0u8; 512];
    let read3b = tokio::time::timeout(Duration::from_secs(2), conn3.read(&mut buf3b)).await;
    let n = read3b
        .expect("conn3: handler should have started after conn1 dropped")
        .expect("conn3: read error");
    assert!(
        has_auth_ok(&buf3b[..n]),
        "conn3: response missing AuthenticationOk after permit released"
    );

    drop(conn2);

    shutdown.cancel();
}

/// Permit is released when a connection closes (`max=1`).
/// Open A (handshake works), B is queued, drop A → B completes.
#[tokio::test]
async fn test_permit_released_on_close() {
    let (server, addr, _temp_file) = start_limited_server(15442, 1).await;
    let shutdown = server.shutdown_token();

    tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    // --- grab the only slot ---
    let mut conn_a = TcpStream::connect(addr).await.unwrap();
    assert_handshake_ok(&mut conn_a, "connA (only slot)").await;

    // --- B: connects at TCP level, handler queued ---
    let mut conn_b = TcpStream::connect(addr).await.unwrap();
    conn_b
        .write_all(&build_startup("test", "test"))
        .await
        .unwrap();

    let mut buf_b = vec![0u8; 512];
    let read_b = tokio::time::timeout(Duration::from_secs(2), conn_b.read(&mut buf_b)).await;
    assert!(
        read_b.is_err(),
        "connB: expected timeout — semaphore full, handler queued"
    );

    // --- release the slot ---
    drop(conn_a);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // --- B should now complete ---
    let mut buf_b2 = vec![0u8; 512];
    let read_b2 = tokio::time::timeout(Duration::from_secs(2), conn_b.read(&mut buf_b2)).await;
    let n = read_b2
        .expect("connB: handler should have started after connA dropped")
        .expect("connB: read error");
    assert!(
        has_auth_ok(&buf_b2[..n]),
        "connB: response missing AuthenticationOk after permit released"
    );

    shutdown.cancel();
}
