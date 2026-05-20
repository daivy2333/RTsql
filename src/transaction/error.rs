use thiserror::Error;

#[derive(Error, Debug)]
pub enum TransactionError {
    #[error("Transaction {0} not found")]
    NotFound(u64),

    #[error("Transaction {0} already committed")]
    AlreadyCommitted(u64),

    #[error("Transaction {0} already aborted")]
    AlreadyAborted(u64),

    #[error("Lock conflict on row")]
    LockConflict,

    #[error("Version chain corrupted")]
    VersionChainCorrupted,
}

pub type Result<T> = std::result::Result<T, TransactionError>;