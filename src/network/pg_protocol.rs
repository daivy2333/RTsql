//! PostgreSQL Simple Query Protocol implementation
//!
//! M7: PostgreSQL 3.0 Simple Query Protocol handler

use async_trait::async_trait;
use rand::Rng;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::network::error::NetworkError;
use crate::network::pg_messages;
use crate::network::protocol::{Protocol, Request};

/// Protocol state machine states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtocolState {
    Startup,
    Ready,
    Querying,
}

/// PostgreSQL protocol handler
pub struct PgProtocol {
    state: ProtocolState,
    process_id: u32,
    secret_key: u32,
    buffer: Vec<u8>,
}

impl PgProtocol {
    pub fn new() -> Self {
        let mut rng = rand::thread_rng();
        Self {
            state: ProtocolState::Startup,
            process_id: rng.gen::<u32>(),
            secret_key: rng.gen::<u32>(),
            buffer: Vec::with_capacity(8192),
        }
    }

    pub fn state(&self) -> &'static str {
        match self.state {
            ProtocolState::Startup => "Startup",
            ProtocolState::Ready => "Ready",
            ProtocolState::Querying => "Querying",
        }
    }

    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    pub fn secret_key(&self) -> u32 {
        self.secret_key
    }

    /// Read exact number of bytes
    async fn read_exact(&mut self, stream: &mut TcpStream, n: usize) -> Result<(), NetworkError> {
        self.buffer.clear();
        self.buffer.resize(n, 0);
        stream.read_exact(&mut self.buffer).await?;
        Ok(())
    }

    /// Handle StartupMessage in Startup state
    async fn handle_startup(
        &mut self,
        stream: &mut TcpStream,
    ) -> Result<Option<Request>, NetworkError> {
        // Read length (4 bytes)
        self.read_exact(stream, 4).await?;
        let length = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        // Read the rest of the message (length - 4 bytes)
        let rest_len = length.saturating_sub(4);
        if rest_len > 0 {
            self.read_exact(stream, rest_len).await?;
        }

        // Verify protocol version (first 4 bytes of payload)
        let version = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]);

        // PostgreSQL 3.0 = 196608 (protocol major=3, minor=0)
        if version != 196608 {
            return Err(NetworkError::ProtocolParse(format!(
                "Unsupported protocol version: {}",
                version
            )));
        }

        // Send startup response
        self.send_startup_response(stream).await?;

        // Transition to Ready state
        self.state = ProtocolState::Ready;

        // No query yet
        Ok(None)
    }

    /// Send startup response messages
    async fn send_startup_response(&mut self, stream: &mut TcpStream) -> Result<(), NetworkError> {
        // AuthenticationOk
        stream.write_all(&pg_messages::authentication_ok()).await?;

        // ParameterStatus messages (standard parameters)
        stream
            .write_all(&pg_messages::parameter_status("server_version", "14.0"))
            .await?;
        stream
            .write_all(&pg_messages::parameter_status("client_encoding", "UTF8"))
            .await?;
        stream
            .write_all(&pg_messages::parameter_status("server_encoding", "UTF8"))
            .await?;

        // BackendKeyData
        stream
            .write_all(&pg_messages::backend_key_data(
                self.process_id,
                self.secret_key,
            ))
            .await?;

        // ReadyForQuery (Idle)
        stream.write_all(&pg_messages::ready_for_query('I')).await?;

        stream.flush().await?;
        Ok(())
    }

    /// Handle Query message in Ready state
    async fn handle_query(
        &mut self,
        stream: &mut TcpStream,
    ) -> Result<Option<Request>, NetworkError> {
        // Read message type (already read 'Q')

        // Read length (4 bytes)
        self.read_exact(stream, 4).await?;
        let length = i32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        // Read SQL string (length - 4 bytes, includes NUL terminator)
        let sql_len = length.saturating_sub(4);
        if sql_len > 0 {
            self.read_exact(stream, sql_len).await?;
        }

        // Extract SQL (remove trailing NUL)
        let sql = if sql_len > 0 && self.buffer[sql_len - 1] == 0 {
            String::from_utf8_lossy(&self.buffer[..sql_len - 1]).to_string()
        } else {
            String::from_utf8_lossy(&self.buffer[..sql_len]).to_string()
        };

        // Transition to Querying state
        self.state = ProtocolState::Querying;

        Ok(Some(Request::Query { sql }))
    }

    /// Handle Terminate message in Ready state
    async fn handle_terminate(
        &mut self,
        stream: &mut TcpStream,
    ) -> Result<Option<Request>, NetworkError> {
        // Read length (4 bytes)
        self.read_exact(stream, 4).await?;
        // No payload for Terminate, just return None
        Ok(None)
    }
}

impl Default for PgProtocol {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Protocol for PgProtocol {
    async fn parse_request(
        &mut self,
        stream: &mut TcpStream,
    ) -> Result<Option<Request>, NetworkError> {
        match self.state {
            ProtocolState::Startup => self.handle_startup(stream).await,
            ProtocolState::Ready => {
                // Read message type (1 byte)
                self.read_exact(stream, 1).await?;
                let msg_type = self.buffer[0];

                match msg_type {
                    b'Q' => self.handle_query(stream).await,
                    b'X' => self.handle_terminate(stream).await,
                    _ => Err(NetworkError::ProtocolParse(format!(
                        "Unexpected message type '{}' in Ready state",
                        msg_type as char
                    ))),
                }
            }
            ProtocolState::Querying => {
                // During Querying state, we shouldn't receive new messages
                // The caller should use write_response to send results first
                Err(NetworkError::ProtocolParse(
                    "Cannot parse request during Querying state".to_string(),
                ))
            }
        }
    }

    async fn write_response(
        &mut self,
        _stream: &mut TcpStream,
        _response: &crate::network::protocol::Response,
    ) -> Result<(), NetworkError> {
        // TODO: Implement in Task 11
        todo!("write_response implementation in Task 11")
    }
}
