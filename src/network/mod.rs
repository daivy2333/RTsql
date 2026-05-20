//! Network layer - TCP server, protocol implementation
//!
//! M6: Implement tokio::net::TcpListener and connection handling

pub mod error;
pub mod protocol;
pub mod handler;

pub use error::NetworkError;
pub use protocol::{Protocol, JsonProtocol, Request, Response};
pub use handler::SqlHandler;

// Task 7-8 will add these modules:
// pub mod connection;
// pub mod server;