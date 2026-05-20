//! Transaction management - MVCC, concurrency control
//!
//! M3: Implement transaction ID allocation and MVCC snapshot read

mod error;
mod tx_id;

pub use error::{Result, TransactionError};
pub use tx_id::TransactionId;