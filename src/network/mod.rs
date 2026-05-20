//! Network layer - TCP server, protocol implementation
//!
//! M6: Implement tokio::net::TcpListener and connection handling

pub mod connection;
pub mod error;
pub mod handler;
pub mod pg_messages; // M7: PostgreSQL message types
pub mod pg_protocol; // M7: PostgreSQL Simple Query Protocol
pub mod protocol;
pub mod server;

pub use connection::ConnectionHandler;
pub use error::NetworkError;
pub use handler::SqlHandler;
pub use pg_protocol::PgProtocol; // M7: PostgreSQL protocol implementation
pub use protocol::{JsonProtocol, Protocol, Request, Response};
pub use server::Server;
