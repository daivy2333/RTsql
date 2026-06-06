use thiserror::Error;
use tokio::task::JoinError;

use super::RowId;
use crate::transaction::TransactionError;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Page size mismatch: expected {expected}, got {actual}")]
    PageSizeMismatch { expected: usize, actual: usize },

    #[error("Buffer pool full, no evictable page")]
    BufferPoolFull,

    #[error("Buffer pool miss semaphore closed")]
    SemaphoreClosed,

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

    #[error("slot not found: {0:?}")]
    SlotNotFound(RowId),

    #[error("table not found: {0}")]
    TableNotFound(String),

    #[error("duplicate table: {0}")]
    DuplicateTable(String),

    #[error("column not found: {0}")]
    ColumnNotFound(String),

    #[error("table already exists: {0}")]
    TableAlreadyExists(String),

    #[error("constraint violation: {0}")]
    ConstraintViolation(String),

    #[error("invalid column type: {0}")]
    InvalidColumnType(String),

    #[error("execution error: {0}")]
    ExecutionError(String),

    #[error("transaction error: {0}")]
    Transaction(#[from] TransactionError),

    #[error("WAL error: {0}")]
    WalError(String),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

impl From<crate::wal::WalError> for StorageError {
    fn from(e: crate::wal::WalError) -> Self {
        StorageError::WalError(e.to_string())
    }
}
