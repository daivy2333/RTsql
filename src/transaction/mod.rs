//! Transaction management - MVCC, concurrency control
//!
//! M3: Implement transaction ID allocation and MVCC snapshot read

mod tx_id;

pub use tx_id::TransactionId;