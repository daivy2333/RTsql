//! Network layer - TCP server, protocol implementation
//!
//! M6: Implement tokio::net::TcpListener and connection handling

pub mod error;
pub mod protocol;
pub mod handler;
pub mod connection;

pub use error::NetworkError;
pub use protocol::{Protocol, JsonProtocol, Request, Response};
pub use handler::SqlHandler;
pub use connection::ConnectionHandler;

// Task 8 will add this module:
// pub mod server;