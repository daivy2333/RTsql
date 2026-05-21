//! WAL (Write-Ahead Logging) 模块
//!
//! 提供原子性保障和崩溃恢复能力

mod checkpoint;
mod reader;
mod record;
mod recovery;
mod writer;

pub use checkpoint::CheckpointManager;
pub use reader::WalReader;
pub use record::{WalError, WalRecord, WalRecordType};
pub use recovery::RecoveryManager;
pub use writer::WalWriter;
