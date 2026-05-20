use crate::network::connection::ConnectionHandler;
use crate::network::protocol::JsonProtocol;
use crate::network::handler::SqlHandler;
use crate::network::error::NetworkError;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use std::net::SocketAddr;

/// TCP Server
pub struct Server {
    addr: SocketAddr,
    shutdown: CancellationToken,
}

impl Server {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            shutdown: CancellationToken::new(),
        }
    }

    /// Get shutdown token (for external trigger)
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    /// Start server
    pub async fn run(self) -> Result<(), NetworkError> {
        let listener = TcpListener::bind(self.addr).await?;
        println!("Server listening on {}", self.addr);

        loop {
            tokio::select! {
                // Accept new connection
                result = listener.accept() => {
                    let (stream, peer_addr) = result?;

                    // Spawn coroutine to handle connection
                    let mut handler = ConnectionHandler::new(
                        JsonProtocol::new(),
                        SqlHandler::new(),
                    );

                    tokio::spawn(async move {
                        if let Err(e) = handler.handle(stream).await {
                            eprintln!("Connection error from {}: {}", peer_addr, e);
                        }
                    });
                }

                // Shutdown signal
                _ = self.shutdown.cancelled() => {
                    println!("Server shutting down");
                    break;
                }
            }
        }

        Ok(())
    }
}