use crate::database::Database;
use crate::network::connection::ConnectionHandler;
use crate::network::error::NetworkError;
use crate::network::handler::SqlHandler;
use crate::network::protocol::JsonProtocol;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub struct Server {
    addr: SocketAddr,
    database: Arc<Database>,
    shutdown: CancellationToken,
}

impl Server {
    pub fn new(addr: SocketAddr, database: Arc<Database>) -> Self {
        Self {
            addr,
            database,
            shutdown: CancellationToken::new(),
        }
    }

    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    pub async fn run(self) -> Result<(), NetworkError> {
        let listener = TcpListener::bind(self.addr).await?;
        println!("Server listening on {}", self.addr);

        loop {
            tokio::select! {
                result = listener.accept() => {
                    let (stream, peer_addr) = result?;

                    let mut handler = ConnectionHandler::new(
                        JsonProtocol::new(),
                        SqlHandler::new(self.database.clone()),
                    );

                    tokio::spawn(async move {
                        if let Err(e) = handler.handle(stream).await {
                            eprintln!("Connection error from {}: {}", peer_addr, e);
                        }
                    });
                }

                _ = self.shutdown.cancelled() => {
                    println!("Server shutting down");
                    break;
                }
            }
        }

        Ok(())
    }
}
