// M8 切换到 PgProtocol (PostgreSQL protocol)，JsonProtocol 测试不再适用
// 暂时跳过所有测试，等待新的 pg_integration_test
// 参见 Task 13: pg_integration_test.rs

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

async fn start_server(port: u16, database: Arc<Database>) -> CancellationToken {
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

    let json = serde_json::to_vec(request).unwrap();
    stream.write_all(&json).await.unwrap();
    stream.write_all(b"\n").await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await.unwrap();
        if byte[0] == b'\n' {
            break;
        }
        buf.push(byte[0]);
    }
    serde_json::from_slice(&buf).unwrap()
}

async fn open_temp_db() -> (Arc<Database>, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let db = Arc::new(Database::open(&db_path).await.unwrap());
    (db, dir)
}

async fn setup_table(database: &Database, table: &str) {
    database
        .create_table(
            table,
            vec![
                ("id".to_string(), ColumnType::Int),
                ("name".to_string(), ColumnType::String(255)),
            ],
            "id",
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn e2e_insert_select_single_row() {
    let (db, _dir) = open_temp_db().await;
    setup_table(&db, "users").await;

    let shutdown = start_server(9101, db.clone()).await;

    let insert = Request::Insert {
        sql: "INSERT INTO users (id, name) VALUES (1, 'Alice')".into(),
    };
    let resp = send_request(9101, &insert).await;
    assert!(matches!(resp, Response::AffectedRows { count: 1 }));

    let query = Request::Query {
        sql: "SELECT id, name FROM users WHERE id = 1".into(),
    };
    let resp = send_request(9101, &query).await;
    if let Response::QueryResult { rows } = resp {
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 2);
        assert_eq!(rows[0][0], serde_json::Value::Number(1.into()));
        assert_eq!(rows[0][1], serde_json::Value::String("Alice".into()));
    } else {
        panic!("Expected QueryResult, got {:?}", resp);
    }

    shutdown.cancel();
}

#[tokio::test]
async fn e2e_insert_select_multiple_rows() {
    let (db, _dir) = open_temp_db().await;
    setup_table(&db, "users").await;

    let shutdown = start_server(9102, db.clone()).await;

    let insert = Request::Insert {
        sql: "INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol')".into(),
    };
    let resp = send_request(9102, &insert).await;
    assert!(matches!(resp, Response::AffectedRows { count: 3 }));

    let query = Request::Query {
        sql: "SELECT id, name FROM users WHERE id = 2".into(),
    };
    let resp = send_request(9102, &query).await;
    if let Response::QueryResult { rows } = resp {
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][1], serde_json::Value::String("Bob".into()));
    } else {
        panic!("Expected QueryResult, got {:?}", resp);
    }

    shutdown.cancel();
}

#[tokio::test]
async fn e2e_select_empty_table() {
    let (db, _dir) = open_temp_db().await;
    setup_table(&db, "empty_tbl").await;

    let shutdown = start_server(9103, db.clone()).await;

    let query = Request::Query {
        sql: "SELECT id, name FROM empty_tbl WHERE id = 999".into(),
    };
    let resp = send_request(9103, &query).await;
    if let Response::QueryResult { rows } = resp {
        assert_eq!(rows.len(), 0);
    } else {
        panic!("Expected QueryResult with empty rows, got {:?}", resp);
    }

    shutdown.cancel();
}

#[tokio::test]
async fn e2e_ping_pong() {
    let (db, _dir) = open_temp_db().await;

    let shutdown = start_server(9104, db.clone()).await;

    let resp = send_request(9104, &Request::Ping).await;
    assert!(matches!(resp, Response::Pong));

    shutdown.cancel();
}

#[tokio::test]
async fn e2e_invalid_sql_error() {
    let (db, _dir) = open_temp_db().await;

    let shutdown = start_server(9105, db.clone()).await;

    let query = Request::Query {
        sql: "GARBAGE INVALID SQL !!!".into(),
    };
    let resp = send_request(9105, &query).await;
    assert!(matches!(resp, Response::Error { .. }));

    shutdown.cancel();
}

#[tokio::test]
async fn e2e_insert_update_select() {
    let (db, _dir) = open_temp_db().await;
    setup_table(&db, "users").await;

    let shutdown = start_server(9106, db.clone()).await;

    let insert = Request::Insert {
        sql: "INSERT INTO users (id, name) VALUES (1, 'Alice')".into(),
    };
    let resp = send_request(9106, &insert).await;
    assert!(matches!(resp, Response::AffectedRows { count: 1 }));

    let update = Request::Update {
        sql: "UPDATE users SET name = 'AliceUpdated' WHERE id = 1".into(),
    };
    let resp = send_request(9106, &update).await;
    assert!(matches!(resp, Response::AffectedRows { count: 1 }));

    let query = Request::Query {
        sql: "SELECT id, name FROM users WHERE id = 1".into(),
    };
    let resp = send_request(9106, &query).await;
    if let Response::QueryResult { rows } = resp {
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][1], serde_json::Value::String("AliceUpdated".into()));
    } else {
        panic!("Expected QueryResult after update, got {:?}", resp);
    }

    shutdown.cancel();
}

#[tokio::test]
async fn e2e_insert_delete_select() {
    let (db, _dir) = open_temp_db().await;
    setup_table(&db, "users").await;

    let shutdown = start_server(9107, db.clone()).await;

    let insert = Request::Insert {
        sql: "INSERT INTO users (id, name) VALUES (1, 'Alice')".into(),
    };
    let resp = send_request(9107, &insert).await;
    assert!(matches!(resp, Response::AffectedRows { count: 1 }));

    let delete = Request::Delete {
        sql: "DELETE FROM users WHERE id = 1".into(),
    };
    let resp = send_request(9107, &delete).await;
    assert!(matches!(resp, Response::AffectedRows { count: 1 }));

    let query = Request::Query {
        sql: "SELECT id, name FROM users WHERE id = 1".into(),
    };
    let resp = send_request(9107, &query).await;
    if let Response::QueryResult { rows } = resp {
        assert_eq!(rows.len(), 0);
    } else {
        panic!(
            "Expected QueryResult with empty rows after delete, got {:?}",
            resp
        );
    }

    shutdown.cancel();
}
*/
