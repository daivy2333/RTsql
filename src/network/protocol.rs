use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;

use crate::network::error::NetworkError;

/// Protocol abstraction trait
#[async_trait]
pub trait Protocol: Send + Sync {
    async fn parse_request(&mut self, stream: &mut TcpStream) -> Result<Option<Request>, NetworkError>;
    async fn write_response(&mut self, stream: &mut TcpStream, response: &Response) -> Result<(), NetworkError>;
}

/// Request types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Query { sql: String },
    Insert { sql: String },
    Update { sql: String },
    Delete { sql: String },
    Ping,
}

/// Response types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    QueryResult { row_ids: Vec<(u64, u16)> },
    AffectedRows { count: u64 },
    Error { message: String },
    Pong,
}

impl Request {
    pub fn sql(&self) -> Option<&str> {
        match self {
            Request::Query { sql } => Some(sql),
            Request::Insert { sql } => Some(sql),
            Request::Update { sql } => Some(sql),
            Request::Delete { sql } => Some(sql),
            Request::Ping => None,
        }
    }
}

// JsonProtocol implementation will be added in Task 4