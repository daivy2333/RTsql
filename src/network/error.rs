use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Protocol parse error: {0}")]
    ProtocolParse(String),

    #[error("Protocol write error: {0}")]
    ProtocolWrite(String),

    #[error("SQL parse error: {0}")]
    SqlParse(String),

    #[error("Execution error: {0}")]
    Execution(String),
}
