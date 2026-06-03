use crate::database::Database;
use crate::network::connection::ConnectionHandler;
use crate::network::error::NetworkError;
use crate::network::handler::SqlHandler;
use crate::network::pg_protocol::PgProtocol;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

pub struct Server {
    addr: SocketAddr,
    database: Arc<Database>,
    shutdown: CancellationToken,
    connection_semaphore: Arc<Semaphore>,
}

impl Server {
    pub fn new(addr: SocketAddr, database: Arc<Database>, max_connections: usize) -> Self {
        Self {
            addr,
            database,
            shutdown: CancellationToken::new(),
            connection_semaphore: Arc::new(Semaphore::new(max_connections)),
        }
    }

    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    pub async fn run(self) -> Result<(), NetworkError> {
        let listener = TcpListener::bind(self.addr).await?;
        println!("Server listening on {}", self.addr);

        let conn_semaphore = self.connection_semaphore.clone();

        loop {
            tokio::select! {
                result = listener.accept() => {
                    let (stream, peer_addr) = result?;

                    let mut handler = ConnectionHandler::new(
                        PgProtocol::new(),
                        SqlHandler::new(self.database.clone()),
                    );

                    let sem = conn_semaphore.clone();

                    tokio::spawn(async move {
                        let _permit = sem.acquire_owned().await.expect("semaphore closed");
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
