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
    let server = Server::new(addr, database, 64);
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

// ---------------------------------------------------------------------------
// MS19 Iteration 001 — PG 错误码分类（R3）
// ---------------------------------------------------------------------------

/// PG 简单查询客户端：启动握手后逐条发 Query，读到 ReadyForQuery 为止。
/// 返回本次查询 ErrorResponse 的 (SQLSTATE, message)；无错误则 code 为空串。
struct PgClient {
    stream: TcpStream,
}

/// ErrorResponse 中解析出的 SQLSTATE 与消息文本（无错误时两者为空串）。
#[derive(Default)]
struct ErrorFields {
    code: String,
    message: String,
}

impl PgClient {
    async fn connect(addr: std::net::SocketAddr) -> Self {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let params = b"user\0test_user\0database\0test_db\0";
        let length = 4 + 4 + params.len() + 1;
        let mut startup_msg = Vec::new();
        startup_msg.extend_from_slice(&(length as i32).to_be_bytes());
        startup_msg.extend_from_slice(&196608i32.to_be_bytes());
        startup_msg.extend_from_slice(params);
        startup_msg.push(0);

        stream.write_all(&startup_msg).await.unwrap();
        read_until_ready(&mut stream).await;
        Self { stream }
    }

    async fn query(&mut self, sql: &str) -> ErrorFields {
        let mut msg = vec![b'Q'];
        let payload_len = 4 + sql.len() + 1;
        msg.extend_from_slice(&(payload_len as i32).to_be_bytes());
        msg.extend_from_slice(sql.as_bytes());
        msg.push(0);
        self.stream.write_all(&msg).await.unwrap();
        self.stream.flush().await.unwrap();
        read_until_ready(&mut self.stream).await
    }
}

/// 逐条读 PG 消息（1 字节类型 + 4 字节长度 + 载荷），到 'Z' 为止；
/// 顺带把 ErrorResponse 字段解析成结构。
async fn read_until_ready(stream: &mut TcpStream) -> ErrorFields {
    let mut error_response = None;
    loop {
        let mut header = [0u8; 5];
        stream.read_exact(&mut header).await.unwrap();
        let msg_type = header[0];
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        let mut payload = vec![0u8; len.saturating_sub(4)];
        if !payload.is_empty() {
            stream.read_exact(&mut payload).await.unwrap();
        }
        if msg_type == b'E' {
            error_response = Some(parse_error_fields(&payload));
        }
        if msg_type == b'Z' {
            break;
        }
    }
    error_response.unwrap_or_default()
}

/// ErrorResponse 载荷：field_type + value(NUL) 序列，以 NUL 结束。
fn parse_error_fields(payload: &[u8]) -> ErrorFields {
    let mut code = String::new();
    let mut message = String::new();
    let mut rest = payload;
    while let Some((&field, tail)) = rest.split_first() {
        if field == 0 {
            break;
        }
        let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
        let value = String::from_utf8_lossy(&tail[..end]).to_string();
        match field {
            b'C' => code = value,
            b'M' => message = value,
            _ => {}
        }
        rest = &tail[(end + 1).min(tail.len())..];
    }
    ErrorFields { code, message }
}

/// R3-S1：能力面拒绝（MERGE）→ 0A000，消息点名能力。
#[tokio::test]
async fn pg_capability_error_0a000() {
    let (server, addr, _temp_file) = start_test_server(15434).await;
    let shutdown = server.shutdown_token();
    tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let mut client = PgClient::connect(addr).await;
    let resp = client
        .query("MERGE INTO t USING t ON t.id = t.id WHEN MATCHED THEN UPDATE SET id = 1")
        .await;
    assert_eq!(resp.code, "0A000", "message: {}", resp.message);
    assert!(
        resp.message.contains("MERGE"),
        "能力拒绝消息必须点名能力: {}",
        resp.message
    );

    shutdown.cancel();
}

/// R3-S2：语法错误 → 42601。
#[tokio::test]
async fn pg_parse_error_42601() {
    let (server, addr, _temp_file) = start_test_server(15435).await;
    let shutdown = server.shutdown_token();
    tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let mut client = PgClient::connect(addr).await;
    let resp = client.query("SELECT (1 FROM t").await;
    assert_eq!(resp.code, "42601", "message: {}", resp.message);

    shutdown.cancel();
}

/// R3-S3：表不存在 → 42P01，列不存在 → 42703。
#[tokio::test]
async fn pg_name_resolution_errors() {
    let (server, addr, _temp_file) = start_test_server(15436).await;
    let shutdown = server.shutdown_token();
    tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let mut client = PgClient::connect(addr).await;
    let resp = client.query("SELECT * FROM nosuchtable").await;
    assert_eq!(resp.code, "42P01", "message: {}", resp.message);

    // 42703 见证取 JOIN ON 条件列解析路径（`PlanError::ColumnNotFound`）。
    client.query("CREATE TABLE t (id INT PRIMARY KEY)").await;
    client.query("CREATE TABLE r (a INT PRIMARY KEY)").await;
    let resp = client
        .query("SELECT t.id FROM t JOIN r ON t.nosuchcol = r.a")
        .await;
    assert_eq!(resp.code, "42703", "message: {}", resp.message);

    shutdown.cancel();
}

/// R6-S1 PG 面：纯列路径未知投影列 → 42703（`PlanError::ColumnNotFound`）。
#[tokio::test]
async fn pg_unknown_column_42703() {
    let (server, addr, _temp_file) = start_test_server(15439).await;
    let shutdown = server.shutdown_token();
    tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let mut client = PgClient::connect(addr).await;
    client
        .query("CREATE TABLE t (id INT PRIMARY KEY, name STRING)")
        .await;
    let resp = client.query("SELECT nosuchcol FROM t").await;
    assert_eq!(resp.code, "42703", "message: {}", resp.message);
    assert!(
        resp.message.contains("nosuchcol"),
        "message: {}",
        resp.message
    );

    shutdown.cancel();
}

/// R8-S1 PG 面：WHERE 谓词列缺失 SHALL 为 42703（SHALL NOT 42601）。
#[tokio::test]
async fn pg_where_column_missing_42703() {
    let (server, addr, _temp_file) = start_test_server(15440).await;
    let shutdown = server.shutdown_token();
    tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let mut client = PgClient::connect(addr).await;
    client.query("CREATE TABLE t (id INT PRIMARY KEY)").await;
    let resp = client.query("SELECT id FROM t WHERE nosuch = 1").await;
    assert_eq!(resp.code, "42703", "message: {}", resp.message);

    shutdown.cancel();
}
