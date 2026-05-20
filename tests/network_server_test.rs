// M8 切换到 PgProtocol (PostgreSQL protocol)，JsonProtocol 测试不再适用
// 暂时跳过所有测试，等待新的 pg_integration_test
// 参见 tests/e2e_test.rs 中的 PostgreSQL 协议集成测试

/*
use rtsql::database::Database;
use rtsql::network::{Request, Response, Server};
use rtsql::storage::ColumnType;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

async fn start_test_server(port: u16, database: Arc<Database>) -> CancellationToken {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let server = Server::new(addr, database);
    let shutdown = server.shutdown_token();

    tokio::spawn(async move {
        server.run().await.unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    shutdown
}

async fn send_request(port: u16, request: &Request) -> Response {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect(addr).await.unwrap();

    let json = serde_json::to_string(request).unwrap();
    stream.write_all(json.as_bytes()).await.unwrap();
    stream.write_all(&[b'\n']).await.unwrap();

    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await.unwrap();
        if byte[0] == b'\n' {
            break;
        }
        buffer.push(byte[0]);
    }

    serde_json::from_slice(&buffer).unwrap()
}

async fn open_temp_db() -> (Arc<Database>, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Arc::new(Database::open(&db_path).await.unwrap());
    (db, dir)
}

#[tokio::test]
async fn test_server_ping_pong() {
    let (db, _dir) = open_temp_db().await;
    let shutdown = start_test_server(9001, db).await;

    let response = send_request(9001, &Request::Ping).await;
    assert!(matches!(response, Response::Pong));

    shutdown.cancel();
}

#[tokio::test]
async fn test_server_query_flow() {
    let (db, _dir) = open_temp_db().await;
    db.create_table(
        "users",
        vec![
            ("id".to_string(), ColumnType::Int),
            ("name".to_string(), ColumnType::String(255)),
        ],
        "id",
    )
    .await
    .unwrap();

    let shutdown = start_test_server(9002, db).await;

    let insert = Request::Insert {
        sql: "INSERT INTO users (id, name) VALUES (1, 'Alice')".into(),
    };
    send_request(9002, &insert).await;

    let request = Request::Query {
        sql: "SELECT id, name FROM users WHERE id = 1".into(),
    };
    let response = send_request(9002, &request).await;

    match response {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1);
        }
        _ => panic!("Expected QueryResult"),
    }

    shutdown.cancel();
}

#[tokio::test]
async fn test_server_insert_flow() {
    let (db, _dir) = open_temp_db().await;
    db.create_table("users", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();

    let shutdown = start_test_server(9003, db).await;

    let request = Request::Insert {
        sql: "INSERT INTO users (id) VALUES (1)".into(),
    };
    let response = send_request(9003, &request).await;

    match response {
        Response::AffectedRows { count } => {
            assert_eq!(count, 1);
        }
        _ => panic!("Expected AffectedRows"),
    }

    shutdown.cancel();
}

#[tokio::test]
async fn test_server_multiple_requests() {
    let (db, _dir) = open_temp_db().await;
    let shutdown = start_test_server(9004, db).await;

    let addr = SocketAddr::from(([127, 0, 0, 1], 9004));
    let mut stream = TcpStream::connect(addr).await.unwrap();

    let json = serde_json::to_string(&Request::Ping).unwrap();
    stream.write_all(json.as_bytes()).await.unwrap();
    stream.write_all(&[b'\n']).await.unwrap();

    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await.unwrap();
        if byte[0] == b'\n' {
            break;
        }
        buffer.push(byte[0]);
    }
    let resp1: Response = serde_json::from_slice(&buffer).unwrap();
    assert!(matches!(resp1, Response::Pong));

    let json = serde_json::to_string(&Request::Ping).unwrap();
    stream.write_all(json.as_bytes()).await.unwrap();
    stream.write_all(&[b'\n']).await.unwrap();

    buffer.clear();
    loop {
        stream.read_exact(&mut byte).await.unwrap();
        if byte[0] == b'\n' {
            break;
        }
        buffer.push(byte[0]);
    }
    let resp2: Response = serde_json::from_slice(&buffer).unwrap();
    assert!(matches!(resp2, Response::Pong));

    shutdown.cancel();
}
*/
