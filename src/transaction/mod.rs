//! Transaction management - MVCC, concurrency control
//!
//! M3: Implement transaction ID allocation and MVCC snapshot read

mod error;
mod manager;
mod row_lock;
mod snapshot;
mod tx_id;
mod version_chain;

pub use error::{Result, TransactionError};
pub use manager::{Transaction, TransactionManager, TransactionState};
pub use row_lock::RowLockTable;
pub use snapshot::Snapshot;
pub use tx_id::TransactionId;
pub use version_chain::VersionHeader;
