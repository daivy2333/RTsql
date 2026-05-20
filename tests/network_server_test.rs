use rtsql::network::{JsonProtocol, Request, Response, Server};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

async fn start_test_server(port: u16) -> CancellationToken {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let server = Server::new(addr);
    let shutdown = server.shutdown_token();

    tokio::spawn(async move {
        server.run().await.unwrap();
    });

    // Wait for server to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    shutdown
}

async fn send_request(port: u16, request: &Request) -> Response {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect(addr).await.unwrap();

    // Send JSON request (with newline)
    let json = serde_json::to_string(request).unwrap();
    stream.write_all(json.as_bytes()).await.unwrap();
    stream.write_all(&[b'\n']).await.unwrap();

    // Read response
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read(&mut byte).await.unwrap();
        if byte[0] == b'\n' {
            break;
        }
        buffer.push(byte[0]);
    }

    let response: Response = serde_json::from_slice(&buffer).unwrap();
    response
}

#[tokio::test]
async fn test_server_ping_pong() {
    let shutdown = start_test_server(9001).await;

    let response = send_request(9001, &Request::Ping).await;
    assert!(matches!(response, Response::Pong));

    shutdown.cancel();
}

#[tokio::test]
async fn test_server_query_flow() {
    let shutdown = start_test_server(9002).await;

    let request = Request::Query {
        sql: "SELECT * FROM users".to_string(),
    };
    let response = send_request(9002, &request).await;

    match response {
        Response::QueryResult { row_ids } => {
            // M6 mock: return fixed RowId
            assert_eq!(row_ids, vec![(0, 1)]);
        }
        _ => panic!("Expected QueryResult"),
    }

    shutdown.cancel();
}

#[tokio::test]
async fn test_server_insert_flow() {
    let shutdown = start_test_server(9003).await;

    let request = Request::Insert {
        sql: "INSERT INTO users VALUES (1, 'Alice')".to_string(),
    };
    let response = send_request(9003, &request).await;

    match response {
        Response::AffectedRows { count } => {
            // M6 mock: return fixed AffectedRows
            assert_eq!(count, 1);
        }
        _ => panic!("Expected AffectedRows"),
    }

    shutdown.cancel();
}

#[tokio::test]
async fn test_server_multiple_requests() {
    let shutdown = start_test_server(9004).await;

    // Keep connection open, send multiple requests
    let addr = SocketAddr::from(([127, 0, 0, 1], 9004));
    let mut stream = TcpStream::connect(addr).await.unwrap();

    // Send Ping
    let json = serde_json::to_string(&Request::Ping).unwrap();
    stream.write_all(json.as_bytes()).await.unwrap();
    stream.write_all(&[b'\n']).await.unwrap();

    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read(&mut byte).await.unwrap();
        if byte[0] == b'\n' {
            break;
        }
        buffer.push(byte[0]);
    }
    let resp1: Response = serde_json::from_slice(&buffer).unwrap();
    assert!(matches!(resp1, Response::Pong));

    // Send Query
    buffer.clear();
    let json = serde_json::to_string(&Request::Query {
        sql: "SELECT 1".to_string(),
    })
    .unwrap();
    stream.write_all(json.as_bytes()).await.unwrap();
    stream.write_all(&[b'\n']).await.unwrap();

    loop {
        stream.read(&mut byte).await.unwrap();
        if byte[0] == b'\n' {
            break;
        }
        buffer.push(byte[0]);
    }
    let resp2: Response = serde_json::from_slice(&buffer).unwrap();
    match resp2 {
        Response::QueryResult { row_ids } => assert_eq!(row_ids, vec![(0, 1)]),
        _ => panic!("Expected QueryResult"),
    }

    shutdown.cancel();
}
