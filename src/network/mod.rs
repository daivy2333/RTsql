//! Network layer - TCP server, protocol implementation
//!
//! M6: Implement tokio::net::TcpListener and connection handling

pub mod error;

pub use error::NetworkError;

// Task 3-8 will add these modules:
// pub mod protocol;
// pub mod connection;
// pub mod handler;
// pub mod server;