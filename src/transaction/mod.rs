//! Transaction management - MVCC, concurrency control
//!
//! M3: Implement transaction ID allocation and MVCC snapshot read

mod error;
mod manager;
mod row_lock;
mod session;
mod snapshot;
mod tx_id;
mod version_chain;

/// Isolation level of a database (MS09 Iter000 D4).
///
/// Configured at open time via `Database::open_with_isolation` and fixed for
/// the lifetime of the database; there is no runtime switching.
///
/// - `RepeatableRead` (default): the historical semantics — statement
///   snapshots stay `None`, scans keep their no-snapshot visibility byte for
///   byte.
/// - `ReadCommitted`: every statement evaluates against a snapshot taken at
///   statement start, so only transactions already committed at that point
///   are visible (dirty reads excluded); a transaction still sees its own
///   uncommitted writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IsolationLevel {
    #[default]
    RepeatableRead,
    ReadCommitted,
}

pub use error::{Result, TransactionError};
pub use manager::{Transaction, TransactionManager, TransactionState};
pub use row_lock::RowLockTable;
pub use session::TransactionSession;
pub use snapshot::Snapshot;
pub use tx_id::TransactionId;
pub use version_chain::VersionHeader;
