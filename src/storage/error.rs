use thiserror::Error;
use tokio::task::JoinError;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Page size mismatch: expected {expected}, got {actual}")]
    PageSizeMismatch { expected: usize, actual: usize },

    #[error("Buffer pool full, no evictable page")]
    BufferPoolFull,

    #[error("Invalid page id: {0}")]
    InvalidPageId(u64),

    #[error("Invalid capacity: {0}, must be > 0")]
    InvalidCapacity(usize),

    #[error("Task join error: {0}")]
    JoinError(#[from] JoinError),

    #[error("Invalid page type: expected {expected:#x}, got {actual:#x}")]
    InvalidPageType { expected: u8, actual: u8 },

    #[error("Duplicate key")]
    DuplicateKey,

    #[error("Key not found")]
    KeyNotFound,

    #[error("Page full")]
    PageFull,
}

pub type Result<T> = std::result::Result<T, StorageError>;
