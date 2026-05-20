use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::network::error::NetworkError;

/// Protocol abstraction trait
#[async_trait]
pub trait Protocol: Send + Sync {
    async fn parse_request(
        &mut self,
        stream: &mut TcpStream,
    ) -> Result<Option<Request>, NetworkError>;
    async fn write_response(
        &mut self,
        stream: &mut TcpStream,
        response: &Response,
    ) -> Result<(), NetworkError>;
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
    QueryResult { rows: Vec<Vec<serde_json::Value>> },
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

/// JSON protocol implementation
pub struct JsonProtocol {
    buffer: Vec<u8>,
}

impl JsonProtocol {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(4096),
        }
    }
}

impl Default for JsonProtocol {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Protocol for JsonProtocol {
    async fn parse_request(
        &mut self,
        stream: &mut TcpStream,
    ) -> Result<Option<Request>, NetworkError> {
        self.buffer.clear();

        // Read until newline
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte).await {
                Ok(0) => return Ok(None), // Connection closed
                Ok(_) => {
                    if byte[0] == b'\n' {
                        break;
                    }
                    self.buffer.push(byte[0]);
                }
                Err(e) => return Err(NetworkError::Io(e)),
            }
        }

        // Parse JSON
        let request: Request = serde_json::from_slice(&self.buffer)
            .map_err(|e| NetworkError::ProtocolParse(e.to_string()))?;

        Ok(Some(request))
    }

    async fn write_response(
        &mut self,
        stream: &mut TcpStream,
        response: &Response,
    ) -> Result<(), NetworkError> {
        let json =
            serde_json::to_vec(response).map_err(|e| NetworkError::ProtocolWrite(e.to_string()))?;

        stream.write_all(&json).await?;
        stream.write_all(&[b'\n']).await?;
        stream.flush().await?;

        Ok(())
    }
}
