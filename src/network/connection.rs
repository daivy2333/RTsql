use crate::network::error::NetworkError;
use crate::network::handler::SqlHandler;
use crate::network::protocol::Protocol;
use tokio::net::TcpStream;

/// Connection handler, one coroutine per connection
pub struct ConnectionHandler<P: Protocol> {
    protocol: P,
    handler: SqlHandler,
}

impl<P: Protocol> ConnectionHandler<P> {
    pub fn new(protocol: P, handler: SqlHandler) -> Self {
        Self { protocol, handler }
    }

    /// Handle connection lifecycle
    pub async fn handle(&mut self, stream: TcpStream) -> Result<(), NetworkError> {
        let mut stream = stream;

        loop {
            // 1. Parse request
            let request = self.protocol.parse_request(&mut stream).await?;

            match request {
                Some(req) => {
                    // 2. Execute SQL
                    let response = self.handler.execute(req).await;

                    // 3. Write response
                    self.protocol.write_response(&mut stream, &response).await?;
                }
                None => {
                    // Connection closed
                    break;
                }
            }
        }

        Ok(())
    }
}
